//! Spec §49 — Arrow interop helpers.
//!
//! COVE stores nulls as a *null* bitmap (bit set ⇒ null), Arrow stores them as
//! a *validity* bitmap (bit set ⇒ valid). This module owns the bit inversion
//! and byte-aligned conversion required to bridge the two.

mod config;
mod dictionary;
mod nested;
mod selection_utils;
mod validity;

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    ptr::{self, NonNull},
    sync::Arc,
};

use arrow_array::{
    builder::{BinaryBuilder, BinaryViewBuilder, StringBuilder, StringViewBuilder},
    types::{
        Float32Type, Float64Type, GenericBinaryType, GenericStringType, Int16Type, Int32Type,
        Int64Type, Int8Type, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
    },
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Date32Array, Decimal128Array,
    DictionaryArray, FixedSizeBinaryArray, FixedSizeListArray, Float32Array, Float64Array,
    GenericByteArray, Int16Array, Int32Array, Int64Array, Int8Array, ListArray, MapArray,
    RecordBatch, StringViewArray, StructArray, TimestampMicrosecondArray, TimestampNanosecondArray,
    UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_buffer::{
    ArrowNativeType, BooleanBuffer, Buffer, NullBuffer, OffsetBuffer, ScalarBuffer,
};
use arrow_data::{ByteView, MAX_INLINE_VIEW_LEN};
use arrow_schema::{DataType, Field, Fields, Schema, TimeUnit};

type ArrowViewParts = (ScalarBuffer<u128>, Vec<Buffer>, Option<NullBuffer>);
type ArrowOffsetParts = (OffsetBuffer<i32>, Buffer, Option<NullBuffer>);

use crate::{
    array::{CoveArrayValue, EncodedArray},
    constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind, StorageClass, ValueTag},
    dictionary::DictionaryValue,
    encoding::{
        bit_packed::{BitPacked, BitPackedPayload},
        constant::ConstantPayload,
        delta::{Delta, DeltaPayload},
        frame_of_reference::{ForPayload, FrameOfReference},
        local_codebook::{LocalCodebookPayload, LocalCodebookValues},
        nested::{ListLayoutPayload, MapLayoutPayload, StructLayoutPayload},
        patched_base::{PatchedBase, PatchedBasePayload},
        rle::{Rle, RlePayload},
        run_end::{RunEnd, RunEndPayload},
        sparse::{Sparse, SparsePayload},
        Encoding,
    },
    nested_schema::NestedSchemaNodeV1,
    page_payload::{PageBufferKind, PagePayloadTreeNode, RetainedColumnPagePayloadV1},
    validity::ValidityBitmap,
    wire, CoveError,
};

pub use config::*;
pub use dictionary::*;
use dictionary::{try_filecode_dictionary_array, try_filecode_dictionary_array_for_selection};
pub use nested::*;
use selection_utils::{count_bitset_rows, mask_selection_tail, selected_rows_are_all_rows};
pub use validity::{arrow_validity_to_cove_null, cove_null_to_arrow_validity};

#[derive(Clone)]
pub struct ArrowEncodedColumn<'name, 'array, 'data> {
    pub name: &'name str,
    pub array: &'array EncodedArray<'data>,
    pub data_owner: Option<ArrowBufferOwner>,
}

impl<'name, 'array, 'data> ArrowEncodedColumn<'name, 'array, 'data> {
    pub fn borrowed(name: &'name str, array: &'array EncodedArray<'data>) -> Self {
        Self {
            name,
            array,
            data_owner: None,
        }
    }

    pub fn with_data_owner(
        name: &'name str,
        array: &'array EncodedArray<'data>,
        data_owner: Option<ArrowBufferOwner>,
    ) -> Self {
        Self {
            name,
            array,
            data_owner,
        }
    }
}

/// Page-local row selection for Arrow export.
///
/// INVARIANT: rows are COVE page ordinals. Bitsets must cover exactly the
/// source page length so dense predicate selections can cross the cove-arrow
/// boundary without first materialising a row-index vector.
#[derive(Debug, Clone, Copy)]
pub enum ArrowRowSelection<'a> {
    All,
    Rows(&'a [u32]),
    Bitset { words: &'a [u64], len: usize },
}

impl<'a> ArrowRowSelection<'a> {
    fn is_all_rows(self, row_count: u64) -> Result<bool, CoveError> {
        match self {
            Self::All => Ok(true),
            Self::Rows(rows) => Ok(selected_rows_are_all_rows(rows, row_count)),
            Self::Bitset { words, len } => {
                self.validate_for_row_count(row_count)?;
                Ok(count_bitset_rows(words, len)? == len)
            }
        }
    }

    fn selected_len(self, row_count: u64) -> Result<usize, CoveError> {
        match self {
            Self::All => usize::try_from(row_count).map_err(|_| CoveError::ArithOverflow),
            Self::Rows(rows) => Ok(rows.len()),
            Self::Bitset { words, len } => {
                self.validate_for_row_count(row_count)?;
                count_bitset_rows(words, len)
            }
        }
    }

    fn validate_for_row_count(self, row_count: u64) -> Result<(), CoveError> {
        match self {
            Self::All => usize::try_from(row_count)
                .map(|_| ())
                .map_err(|_| CoveError::ArithOverflow),
            Self::Rows(rows) => {
                for row in rows {
                    if u64::from(*row) >= row_count {
                        return Err(CoveError::OffsetRange);
                    }
                }
                Ok(())
            }
            Self::Bitset { words, len } => {
                if u64::try_from(len).map_err(|_| CoveError::ArithOverflow)? != row_count {
                    return Err(CoveError::OffsetRange);
                }
                let word_len = len.div_ceil(64);
                if words.len() < word_len {
                    return Err(CoveError::BufferTooShort);
                }
                Ok(())
            }
        }
    }

    fn for_each_row<F>(self, row_count: u64, mut visit: F) -> Result<(), CoveError>
    where
        F: FnMut(usize) -> Result<(), CoveError>,
    {
        match self {
            Self::All => {
                let row_count = usize::try_from(row_count).map_err(|_| CoveError::ArithOverflow)?;
                for row in 0..row_count {
                    visit(row)?;
                }
            }
            Self::Rows(rows) => {
                for row in rows {
                    if u64::from(*row) >= row_count {
                        return Err(CoveError::OffsetRange);
                    }
                    visit(usize::try_from(*row).map_err(|_| CoveError::ArithOverflow)?)?;
                }
            }
            Self::Bitset { words, len } => {
                self.validate_for_row_count(row_count)?;
                let word_len = len.div_ceil(64);
                for (word_index, raw_word) in words.iter().take(word_len).copied().enumerate() {
                    let mut word = if word_index + 1 == word_len {
                        mask_selection_tail(raw_word, len)
                    } else {
                        raw_word
                    };
                    while word != 0 {
                        let bit = word.trailing_zeros() as usize;
                        let row = word_index
                            .checked_mul(64)
                            .and_then(|base| base.checked_add(bit))
                            .ok_or(CoveError::ArithOverflow)?;
                        visit(row)?;
                        word &= word - 1;
                    }
                }
            }
        }
        Ok(())
    }

    fn to_rows(self, row_count: u64) -> Result<Vec<u32>, CoveError> {
        let mut rows = Vec::with_capacity(self.selected_len(row_count)?);
        self.for_each_row(row_count, |row| {
            rows.push(u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?);
            Ok(())
        })?;
        Ok(rows)
    }
}

/// Export one decoded COVE array view as an Arrow array.
pub fn encoded_array_to_arrow(array: &EncodedArray<'_>) -> Result<ArrayRef, CoveError> {
    let result = encoded_array_to_arrow_with_options(array, ArrowExportOptions::default())?;
    if result.report.has_lossy_or_unsupported() {
        return Err(CoveError::UnsupportedEncoding(format!(
            "Arrow export for {:?} requires explicit fidelity reporting",
            array.logical
        )));
    }
    Ok(result.value)
}

/// Export one scalar COVE array with explicit dictionary handling.
pub fn encoded_array_to_arrow_with_policy(
    array: &EncodedArray<'_>,
    dictionary_policy: ArrowDictionaryPolicy,
) -> Result<ArrayRef, CoveError> {
    let result = encoded_array_to_arrow_with_options(
        array,
        ArrowExportOptions {
            dictionary_policy,
            ..ArrowExportOptions::default()
        },
    )?;
    if result.report.has_lossy_or_unsupported() {
        return Err(CoveError::UnsupportedEncoding(format!(
            "Arrow export for {:?} requires explicit fidelity reporting",
            array.logical
        )));
    }
    Ok(result.value)
}

/// Export one scalar COVE array and return representation-fidelity diagnostics.
pub fn encoded_array_to_arrow_with_report(
    array: &EncodedArray<'_>,
) -> Result<ArrowExportResult<ArrayRef>, CoveError> {
    encoded_array_to_arrow_with_options(array, ArrowExportOptions::default())
}

/// Export one scalar COVE array with explicit Arrow export options and diagnostics.
pub fn encoded_array_to_arrow_with_options(
    array: &EncodedArray<'_>,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<ArrayRef>, CoveError> {
    encoded_array_to_arrow_with_options_and_owner(array, options, None)
}

fn encoded_array_to_arrow_with_options_and_owner(
    array: &EncodedArray<'_>,
    options: ArrowExportOptions,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<ArrowExportResult<ArrayRef>, CoveError> {
    let mut report = ArrowExportReport::default();
    if options.dictionary_policy == ArrowDictionaryPolicy::DictionaryKeys {
        if let Some(dictionary_array) = try_filecode_dictionary_array(array, options)? {
            report.push(
                None,
                array.logical,
                ArrowFidelitySeverity::Informational,
                "FileCode values exported as Arrow dictionary keys",
            );
            return Ok(ArrowExportResult {
                value: dictionary_array,
                report,
            });
        }
    }
    let arrow_type = arrow_data_type_with_report(array.logical, &options, &mut report)?;
    if let Some(array_ref) = try_direct_byte_array(
        array,
        &arrow_type,
        data_owner,
        options.string_validation_policy,
    )? {
        return Ok(ArrowExportResult {
            value: array_ref,
            report,
        });
    }
    if let Some(array_ref) = try_direct_primitive_array(array, &arrow_type, data_owner)? {
        return Ok(ArrowExportResult {
            value: array_ref,
            report,
        });
    }
    if let Some(array_ref) = try_direct_decoded_array(array, ArrowRowSelection::All, &arrow_type)? {
        return Ok(ArrowExportResult {
            value: array_ref,
            report,
        });
    }
    let values = array.decode_all_rows()?;
    let array_ref = values_to_arrow_array_with_data_type(array.logical, &values, arrow_type)?;
    Ok(ArrowExportResult {
        value: array_ref,
        report,
    })
}

/// Export selected rows from one scalar COVE array.
///
/// INVARIANT: `selected_rows` are page-local row ordinals. The function never
/// silently wraps or clamps an out-of-range ordinal because that would create
/// a wrong-row projection at the Arrow boundary.
pub fn encoded_array_to_arrow_selected_with_options(
    array: &EncodedArray<'_>,
    selected_rows: &[u32],
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<ArrayRef>, CoveError> {
    encoded_array_to_arrow_with_row_selection_options(
        array,
        ArrowRowSelection::Rows(selected_rows),
        options,
    )
}

/// Export rows from one scalar COVE array using a page-local row selection.
pub fn encoded_array_to_arrow_with_row_selection_options(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<ArrayRef>, CoveError> {
    encoded_array_to_arrow_with_row_selection_options_and_owner(array, selection, options, None)
}

pub fn encoded_array_to_arrow_with_row_selection_options_and_owner(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    options: ArrowExportOptions,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<ArrowExportResult<ArrayRef>, CoveError> {
    if selection.is_all_rows(array.row_count)? {
        return encoded_array_to_arrow_with_options_and_owner(array, options, data_owner);
    }
    let mut report = ArrowExportReport::default();
    if options.dictionary_policy == ArrowDictionaryPolicy::DictionaryKeys {
        if let Some(dictionary_array) =
            try_filecode_dictionary_array_for_selection(array, selection, options)?
        {
            report.push(
                None,
                array.logical,
                ArrowFidelitySeverity::Informational,
                "selected FileCode values exported as Arrow dictionary keys",
            );
            return Ok(ArrowExportResult {
                value: dictionary_array,
                report,
            });
        }
    }
    let arrow_type = arrow_data_type_with_report(array.logical, &options, &mut report)?;
    if let Some(array_ref) = try_direct_byte_array_for_selection(
        array,
        selection,
        &arrow_type,
        data_owner,
        options.string_validation_policy,
    )? {
        return Ok(ArrowExportResult {
            value: array_ref,
            report,
        });
    }
    if let Some(array_ref) =
        try_direct_primitive_array_for_selection(array, selection, &arrow_type)?
    {
        return Ok(ArrowExportResult {
            value: array_ref,
            report,
        });
    }
    if let Some(array_ref) = try_direct_decoded_array(array, selection, &arrow_type)? {
        return Ok(ArrowExportResult {
            value: array_ref,
            report,
        });
    }
    let prepared = array.prepare()?;
    let selected_rows = selection.to_rows(array.row_count)?;
    let values = prepared.decode_selected_rows(&selected_rows)?;
    let array_ref = values_to_arrow_array_with_data_type(array.logical, &values, arrow_type)?;
    Ok(ArrowExportResult {
        value: array_ref,
        report,
    })
}

/// Export named COVE array views as an Arrow [`RecordBatch`] using a page-local
/// row selection.
pub fn encoded_columns_to_record_batch_selected_with_options(
    columns: &[(&str, &EncodedArray<'_>)],
    selected_rows: &[u32],
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<RecordBatch>, CoveError> {
    let selection = ArrowRowSelection::Rows(selected_rows);
    let result = encoded_columns_to_arrow_arrays_with_options(columns, selection, options)?;
    let batch = record_batch_from_exported_arrays(columns, result.value, options)?;
    Ok(ArrowExportResult {
        value: batch,
        report: result.report,
    })
}

/// Export named COVE array views as Arrow arrays using a page-local row selection.
pub fn encoded_columns_to_arrow_arrays_with_options(
    columns: &[(&str, &EncodedArray<'_>)],
    selection: ArrowRowSelection<'_>,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<Vec<ArrayRef>>, CoveError> {
    let owned_columns = columns
        .iter()
        .map(|(name, array)| ArrowEncodedColumn::borrowed(name, array))
        .collect::<Vec<_>>();
    encoded_columns_to_arrow_arrays_with_owners_options(&owned_columns, selection, options)
}

/// Export named COVE array views as Arrow arrays, retaining optional backing
/// owners for direct Arrow View buffers.
pub fn encoded_columns_to_arrow_arrays_with_owners_options(
    columns: &[ArrowEncodedColumn<'_, '_, '_>],
    selection: ArrowRowSelection<'_>,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<Vec<ArrayRef>>, CoveError> {
    let mut arrays = Vec::with_capacity(columns.len());
    let mut report = ArrowExportReport::default();
    for column in columns {
        let result = encoded_array_to_arrow_with_row_selection_options_and_owner(
            column.array,
            selection,
            options,
            column.data_owner.as_ref(),
        )?;
        report.extend_with_field(column.name, result.report);
        arrays.push(result.value);
    }
    Ok(ArrowExportResult {
        value: arrays,
        report,
    })
}

/// Export owner-backed named COVE array views as an Arrow [`RecordBatch`].
pub fn encoded_columns_to_record_batch_with_owners_options(
    columns: &[ArrowEncodedColumn<'_, '_, '_>],
    selection: ArrowRowSelection<'_>,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<RecordBatch>, CoveError> {
    let result = encoded_columns_to_arrow_arrays_with_owners_options(columns, selection, options)?;
    let mut fields = Vec::with_capacity(columns.len());
    for (column, arrow_array) in columns.iter().zip(result.value.iter()) {
        fields.push(arrow_field_for_cove(
            column.name,
            arrow_array.data_type().clone(),
            column.array.validity.is_some() || column.array.logical == CoveLogicalType::Null,
            column.array.logical,
            options,
        ));
    }
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), result.value)
        .map_err(|err| CoveError::BadSection(format!("Arrow RecordBatch export failed: {err}")))?;
    Ok(ArrowExportResult {
        value: batch,
        report: result.report,
    })
}

/// Export named COVE array views as an Arrow [`RecordBatch`].
pub fn encoded_columns_to_record_batch(
    columns: &[(&str, &EncodedArray<'_>)],
) -> Result<RecordBatch, CoveError> {
    let result =
        encoded_columns_to_record_batch_with_options(columns, ArrowExportOptions::default())?;
    if result.report.has_lossy_or_unsupported() {
        return Err(CoveError::UnsupportedEncoding(
            "Arrow export requires explicit fidelity reporting".into(),
        ));
    }
    Ok(result.value)
}

/// Export named COVE array views as an Arrow [`RecordBatch`] with fidelity diagnostics.
pub fn encoded_columns_to_record_batch_with_report(
    columns: &[(&str, &EncodedArray<'_>)],
) -> Result<ArrowExportResult<RecordBatch>, CoveError> {
    encoded_columns_to_record_batch_with_options(columns, ArrowExportOptions::default())
}

/// Export named COVE array views with explicit Arrow export options and diagnostics.
pub fn encoded_columns_to_record_batch_with_options(
    columns: &[(&str, &EncodedArray<'_>)],
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<RecordBatch>, CoveError> {
    let result =
        encoded_columns_to_arrow_arrays_with_options(columns, ArrowRowSelection::All, options)?;
    let batch = record_batch_from_exported_arrays(columns, result.value, options)?;
    Ok(ArrowExportResult {
        value: batch,
        report: result.report,
    })
}

fn record_batch_from_exported_arrays(
    columns: &[(&str, &EncodedArray<'_>)],
    arrays: Vec<ArrayRef>,
    options: ArrowExportOptions,
) -> Result<RecordBatch, CoveError> {
    let mut fields = Vec::with_capacity(columns.len());
    for ((name, array), arrow_array) in columns.iter().zip(arrays.iter()) {
        fields.push(arrow_field_for_cove(
            name,
            arrow_array.data_type().clone(),
            array.validity.is_some() || array.logical == CoveLogicalType::Null,
            array.logical,
            options,
        ));
    }
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
        .map_err(|err| CoveError::BadSection(format!("Arrow RecordBatch: {err}")))
}

fn arrow_node_nullable(node: &ArrowExportNode<'_>) -> bool {
    match node {
        ArrowExportNode::Scalar { array, .. } => {
            array.validity.is_some() || array.logical == CoveLogicalType::Null
        }
        ArrowExportNode::List { validity, .. }
        | ArrowExportNode::Struct { validity, .. }
        | ArrowExportNode::Map { validity, .. } => validity.is_some(),
    }
}

fn arrow_field_for_cove(
    name: &str,
    data_type: DataType,
    nullable: bool,
    logical: CoveLogicalType,
    options: ArrowExportOptions,
) -> Field {
    let field = Field::new(name, data_type, nullable);
    let metadata = arrow_extension_metadata(logical, options);
    if metadata.is_empty() {
        field
    } else {
        field.with_metadata(metadata)
    }
}

fn arrow_extension_metadata(
    logical: CoveLogicalType,
    options: ArrowExportOptions,
) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    match logical {
        CoveLogicalType::Uuid if options.emit_uuid_extension_metadata => {
            metadata.insert("ARROW:extension:name".into(), "cove.uuid".into());
            metadata.insert(
                "ARROW:extension:metadata".into(),
                r#"{"storage":"fixed_size_binary[16]"}"#.into(),
            );
        }
        CoveLogicalType::Json if options.emit_json_extension_metadata => {
            metadata.insert("ARROW:extension:name".into(), "cove.json".into());
            let storage = if options.varbytes_policy == ArrowVarBytesExportPolicy::View {
                r#"{"storage":"utf8_view"}"#
            } else {
                r#"{"storage":"utf8"}"#
            };
            metadata.insert("ARROW:extension:metadata".into(), storage.into());
        }
        _ => {}
    }
    metadata
}

fn arrow_i32_offsets(offsets: &[u32]) -> Result<OffsetBuffer<i32>, CoveError> {
    let mut converted = Vec::with_capacity(offsets.len());
    for &offset in offsets {
        converted.push(i32::try_from(offset).map_err(|_| {
            CoveError::UnsupportedEncoding(
                "Arrow ListArray/MapArray export requires i32 offsets; chunk the column first"
                    .into(),
            )
        })?);
    }
    Ok(OffsetBuffer::new(ScalarBuffer::from(converted)))
}

#[inline]
fn bitpacked_len(len: usize) -> Result<usize, CoveError> {
    len.checked_add(7)
        .ok_or(CoveError::ArithOverflow)
        .map(|len| len / 8)
}

#[inline]
fn set_packed_bit(bytes: &mut [u8], index: usize) {
    bytes[index / 8] |= 1u8 << (index % 8);
}

struct ArrowValidityBuilder {
    bytes: Vec<u8>,
    len: usize,
    pos: usize,
    null_count: usize,
}

impl ArrowValidityBuilder {
    fn new(len: usize) -> Result<Self, CoveError> {
        Ok(Self {
            bytes: vec![0u8; bitpacked_len(len)?],
            len,
            pos: 0,
            null_count: 0,
        })
    }

    fn append(&mut self, is_valid: bool) {
        debug_assert!(self.pos < self.len);
        if is_valid {
            set_packed_bit(&mut self.bytes, self.pos);
        } else {
            self.null_count += 1;
        }
        self.pos += 1;
    }

    fn finish(self) -> Option<NullBuffer> {
        debug_assert_eq!(self.pos, self.len);
        if self.null_count == 0 {
            return None;
        }
        let buffer = BooleanBuffer::new(Buffer::from_vec(self.bytes), 0, self.len);
        // INVARIANT: `append` writes exactly one validity bit per logical row,
        // setting true only for valid rows and incrementing `null_count` for
        // every false bit.
        // SAFETY: the packed BooleanBuffer therefore contains exactly
        // `null_count` zero bits over its declared logical length.
        Some(unsafe { NullBuffer::new_unchecked(buffer, self.null_count) })
    }
}

fn trusted_i32_offset_buffer(offsets: Vec<i32>) -> OffsetBuffer<i32> {
    debug_assert!(!offsets.is_empty());
    debug_assert_eq!(offsets[0], 0);
    debug_assert!(offsets.windows(2).all(|pair| pair[0] <= pair[1]));
    // INVARIANT: callers append offsets from checked cumulative byte lengths.
    // They start at zero, never decrease, and each value has already fit in
    // i32 before being pushed.
    // SAFETY: these are exactly Arrow's OffsetBuffer invariants for i32
    // offsets.
    unsafe { OffsetBuffer::new_unchecked(ScalarBuffer::from(offsets)) }
}

fn trusted_binary_array(
    offsets: OffsetBuffer<i32>,
    values: Buffer,
    nulls: Option<NullBuffer>,
) -> BinaryArray {
    debug_assert!(offsets.last().copied().unwrap_or_default() as usize <= values.len());
    // INVARIANT: `BytePayloadPlan::materialize*` builds monotonic offsets with
    // the final offset equal to the values buffer length, and any null buffer is
    // produced for the same logical row count.
    // SAFETY: Binary arrays do not require UTF-8 validation; with the proven
    // offset/value/null invariants, `try_new` would not fail.
    unsafe { GenericByteArray::<GenericBinaryType<i32>>::new_unchecked(offsets, values, nulls) }
}

fn trusted_string_array(
    offsets: OffsetBuffer<i32>,
    values: Buffer,
    nulls: Option<NullBuffer>,
) -> GenericByteArray<GenericStringType<i32>> {
    debug_assert!(offsets.last().copied().unwrap_or_default() as usize <= values.len());
    // INVARIANT: callers either validate every non-null string row against the
    // same offset/value/null buffers immediately before construction or carry
    // an explicit page-level proof that every non-null row slice is UTF-8.
    // SAFETY: with monotonic i32 offsets, final offset within `values`, null
    // buffer length matching the offset count, and proven UTF-8 row slices,
    // `try_new` would not fail.
    unsafe { GenericByteArray::<GenericStringType<i32>>::new_unchecked(offsets, values, nulls) }
}

fn arrow_null_buffer(
    validity: Option<ValidityBitmap<'_>>,
    row_count: usize,
) -> Result<Option<NullBuffer>, CoveError> {
    let Some(validity) = validity else {
        return Ok(None);
    };
    let row_count_u64 = u64::try_from(row_count).map_err(|_| CoveError::ArithOverflow)?;
    validity.validate_len(row_count_u64)?;
    let mut validity_builder = ArrowValidityBuilder::new(row_count)?;
    for row in 0..row_count_u64 {
        let valid = validity.is_valid(row)?;
        validity_builder.append(valid);
    }
    Ok(validity_builder.finish())
}

#[inline]
fn read_u32_len_prefixed_range(bytes: &[u8], offset: usize) -> Result<(usize, usize), CoveError> {
    let Some(data_start) = offset.checked_add(4) else {
        return Err(CoveError::ArithOverflow);
    };
    if data_start > bytes.len() {
        return Err(CoveError::OffsetRange);
    }
    let len = wire::read_u32_le_checked(bytes, offset)? as usize;
    let Some(data_end) = data_start.checked_add(len) else {
        return Err(CoveError::ArithOverflow);
    };
    if data_end > bytes.len() {
        return Err(CoveError::OffsetRange);
    }
    Ok((data_start, data_end))
}

#[inline]
fn read_leb128_len_prefixed_range(
    bytes: &[u8],
    offset: usize,
) -> Result<(usize, usize), CoveError> {
    if offset > bytes.len() {
        return Err(CoveError::OffsetRange);
    }
    let (len, consumed) = wire::decode_u64_leb128(&bytes[offset..])?;
    let len = usize::try_from(len).map_err(|_| CoveError::ArithOverflow)?;
    let Some(data_start) = offset.checked_add(consumed) else {
        return Err(CoveError::ArithOverflow);
    };
    let Some(data_end) = data_start.checked_add(len) else {
        return Err(CoveError::ArithOverflow);
    };
    if data_end > bytes.len() {
        return Err(CoveError::OffsetRange);
    }
    Ok((data_start, data_end))
}

#[derive(Debug, Clone, Copy)]
enum BytePayloadLayout {
    U32LengthPrefixed,
    Leb128LengthPrefixed,
}

fn try_direct_byte_array(
    array: &EncodedArray<'_>,
    data_type: &DataType,
    data_owner: Option<&ArrowBufferOwner>,
    string_validation_policy: ArrowStringValidationPolicy,
) -> Result<Option<ArrayRef>, CoveError> {
    if array.physical != CovePhysicalKind::VarBytes {
        return Ok(None);
    }
    let layout = match array.encoding {
        CoveEncodingKind::VarBytes => BytePayloadLayout::U32LengthPrefixed,
        CoveEncodingKind::Canonical
            if matches!(
                array.logical,
                CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json
            ) =>
        {
            BytePayloadLayout::Leb128LengthPrefixed
        }
        _ => return Ok(None),
    };
    byte_array_from_payload_plan(
        array,
        ArrowRowSelection::All,
        layout,
        data_type,
        data_owner,
        string_validation_policy,
    )
}

fn try_direct_byte_array_for_selection(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
    data_owner: Option<&ArrowBufferOwner>,
    string_validation_policy: ArrowStringValidationPolicy,
) -> Result<Option<ArrayRef>, CoveError> {
    if array.physical != CovePhysicalKind::VarBytes {
        return Ok(None);
    }
    let layout = match array.encoding {
        CoveEncodingKind::VarBytes => BytePayloadLayout::U32LengthPrefixed,
        CoveEncodingKind::Canonical
            if matches!(
                array.logical,
                CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json
            ) =>
        {
            BytePayloadLayout::Leb128LengthPrefixed
        }
        _ => return Ok(None),
    };
    byte_array_from_payload_plan(
        array,
        selection,
        layout,
        data_type,
        data_owner,
        string_validation_policy,
    )
}

fn byte_array_from_payload_plan(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    layout: BytePayloadLayout,
    data_type: &DataType,
    data_owner: Option<&ArrowBufferOwner>,
    string_validation_policy: ArrowStringValidationPolicy,
) -> Result<Option<ArrayRef>, CoveError> {
    if !matches!(
        data_type,
        DataType::Utf8 | DataType::Binary | DataType::Utf8View | DataType::BinaryView
    ) {
        return Ok(None);
    }
    let plan = BytePayloadPlan { layout };
    let array_ref = match data_type {
        DataType::Utf8 => {
            let (offsets, values, nulls) =
                plan.materialize_utf8(array, selection, string_validation_policy)?;
            // INVARIANT: Strict mode validates all materialized values before
            // construction. TrustedPageProof is an explicit caller contract
            // that every non-null source row slice is valid UTF-8 at the same
            // row boundaries used to build `offsets`.
            Arc::new(trusted_string_array(offsets, values, nulls)) as ArrayRef
        }
        DataType::Binary => {
            let (offsets, values, nulls) = plan.materialize(array, selection)?;
            Arc::new(trusted_binary_array(offsets, values, nulls)) as ArrayRef
        }
        DataType::Utf8View => {
            let (views, buffers, nulls) = plan.materialize_view(array, selection, data_owner)?;
            Arc::new(
                StringViewArray::try_new(views, buffers, nulls).map_err(|err| {
                    CoveError::BadSection(format!("Arrow Utf8View export: {err}"))
                })?,
            ) as ArrayRef
        }
        DataType::BinaryView => {
            let (views, buffers, nulls) = plan.materialize_view(array, selection, data_owner)?;
            Arc::new(
                BinaryViewArray::try_new(views, buffers, nulls).map_err(|err| {
                    CoveError::BadSection(format!("Arrow BinaryView export: {err}"))
                })?,
            ) as ArrayRef
        }
        _ => unreachable!(),
    };
    Ok(Some(array_ref))
}

fn byte_view_backing_buffer(
    array: &EncodedArray<'_>,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<Buffer, CoveError> {
    if array.data.is_empty() {
        return Ok(Buffer::from_vec(Vec::<u8>::new()));
    }
    let Some(owner) = data_owner else {
        return Ok(Buffer::from_vec(array.data.to_vec()));
    };
    let ptr = NonNull::new(array.data.as_ptr() as *mut u8).ok_or(CoveError::BufferTooShort)?;
    // SAFETY: `data_owner` is supplied by the caller for the allocation that
    // contains `array.data`, and the Arrow Buffer retains that owner for at
    // least as long as any view array referencing this byte range.
    Ok(unsafe { Buffer::from_custom_allocation(ptr, array.data.len(), Arc::clone(owner)) })
}

fn inline_byte_view(bytes: &[u8]) -> Result<u128, CoveError> {
    let len = u32::try_from(bytes.len()).map_err(|_| CoveError::ArithOverflow)?;
    if len > MAX_INLINE_VIEW_LEN {
        return Err(CoveError::ArithOverflow);
    }
    let mut raw = [0u8; 16];
    raw[..4].copy_from_slice(&len.to_le_bytes());
    raw[4..4 + bytes.len()].copy_from_slice(bytes);
    Ok(u128::from_le_bytes(raw))
}

fn buffered_byte_view(data: &[u8], start: usize, end: usize) -> Result<u128, CoveError> {
    let len = end.checked_sub(start).ok_or(CoveError::PageCorrupt)?;
    let len = u32::try_from(len).map_err(|_| CoveError::ArithOverflow)?;
    let offset = u32::try_from(start).map_err(|_| CoveError::ArithOverflow)?;
    let prefix_end = start.checked_add(4).ok_or(CoveError::ArithOverflow)?;
    if prefix_end > end {
        return Err(CoveError::PageCorrupt);
    }
    Ok(ByteView::new(len, &data[start..prefix_end])
        .with_buffer_index(0)
        .with_offset(offset)
        .as_u128())
}

fn byte_view_for_range(data: &[u8], start: usize, end: usize) -> Result<u128, CoveError> {
    let len = end.checked_sub(start).ok_or(CoveError::PageCorrupt)?;
    if u32::try_from(len).map_err(|_| CoveError::ArithOverflow)? <= MAX_INLINE_VIEW_LEN {
        inline_byte_view(&data[start..end])
    } else {
        buffered_byte_view(data, start, end)
    }
}

fn validate_utf8_offsets_values(
    offsets: &OffsetBuffer<i32>,
    values: &Buffer,
) -> Result<(), CoveError> {
    validate_utf8_offsets_slice(offsets, values.as_slice())
}

fn validate_utf8_offsets_slice(offsets: &[i32], values: &[u8]) -> Result<(), CoveError> {
    for pair in offsets.windows(2) {
        let start = usize::try_from(pair[0]).map_err(|_| CoveError::OffsetRange)?;
        let end = usize::try_from(pair[1]).map_err(|_| CoveError::OffsetRange)?;
        if start > end || end > values.len() {
            return Err(CoveError::OffsetRange);
        }
        // If the concatenated values buffer is valid UTF-8, each row is valid
        // iff every non-empty row starts on a codepoint boundary. The only way
        // two adjacent rows can form a valid cross-row codepoint is when the
        // later row starts with a continuation byte.
        if start != 0 && start != end && (values[start] & 0b1100_0000) == 0b1000_0000 {
            return Err(CoveError::BadSection(
                "Arrow Utf8 export: row boundary splits a UTF-8 codepoint".into(),
            ));
        }
    }
    if values.is_ascii() {
        return Ok(());
    }
    validate_arrow_utf8(values)
}

const ASCII_HIGH_BIT_MASK_U64: u64 = 0x8080_8080_8080_8080;
const FIXED_U32_NO_NULLS_MAX_ROWS: usize = 16;
const FIXED_U32_NO_NULLS_MAX_DATA_BYTES: usize = 1024;

#[inline(always)]
fn validate_arrow_utf8(bytes: &[u8]) -> Result<(), CoveError> {
    simdutf8::basic::from_utf8(bytes)
        .map(|_| ())
        .map_err(|err| CoveError::BadSection(format!("Arrow Utf8 export: {err}")))
}

#[inline(always)]
unsafe fn read_u32_le_unaligned(src: *const u8) -> u32 {
    // SAFETY: callers prove that `src..src + 4` is in bounds for the backing
    // byte slice before invoking this helper. `read_unaligned` handles any byte
    // alignment accepted by COVE wire payloads.
    u32::from_le(unsafe { ptr::read_unaligned(src.cast::<u32>()) })
}

#[inline(always)]
unsafe fn copy_varbytes_value(src: *const u8, dst: *mut u8, len: usize) {
    if len <= 16 {
        // SAFETY: the caller proves both ranges are valid for `len` bytes and
        // non-overlapping. The small-copy helper only reads and writes within
        // those same ranges.
        unsafe {
            copy_small_varbytes_value(src, dst, len);
        }
    } else {
        // SAFETY: forwarded caller invariant.
        unsafe {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }
}

#[inline(always)]
unsafe fn copy_varbytes_value_ascii_mask(src: *const u8, dst: *mut u8, len: usize) -> u64 {
    if len <= 16 {
        // SAFETY: forwarded caller invariant.
        return unsafe { copy_small_varbytes_value_ascii_mask(src, dst, len) };
    }
    if len <= 64 {
        let mut mask = 0u64;
        let mut offset = 0usize;
        while offset + 8 <= len {
            // SAFETY: `offset + 8 <= len`, and callers prove both ranges are
            // valid for `len` bytes.
            let word = unsafe { ptr::read_unaligned(src.add(offset).cast::<u64>()) };
            // SAFETY: destination range is valid for the same initialized word.
            unsafe {
                ptr::write_unaligned(dst.add(offset).cast::<u64>(), word);
            }
            mask |= word & ASCII_HIGH_BIT_MASK_U64;
            offset += 8;
        }
        if offset < len {
            // SAFETY: tail lies inside the proven source/destination ranges.
            mask |= unsafe {
                copy_small_varbytes_value_ascii_mask(src.add(offset), dst.add(offset), len - offset)
            };
        }
        return mask;
    }

    // SAFETY: forwarded caller invariant. Large rows use the platform memcpy
    // for throughput, then scan the source with raw loads only on the strict
    // validation path.
    unsafe {
        ptr::copy_nonoverlapping(src, dst, len);
    }
    // SAFETY: source is valid for `len` bytes by caller invariant.
    unsafe { ascii_high_bit_mask(src, len) }
}

#[inline(always)]
unsafe fn ascii_high_bit_mask(src: *const u8, len: usize) -> u64 {
    let mut mask = 0u64;
    let mut offset = 0usize;
    while offset + 8 <= len {
        // SAFETY: `offset + 8 <= len`, and callers prove source validity.
        let word = unsafe { ptr::read_unaligned(src.add(offset).cast::<u64>()) };
        mask |= word & ASCII_HIGH_BIT_MASK_U64;
        offset += 8;
    }
    while offset < len {
        // SAFETY: tail byte lies inside the proven source range.
        mask |= (unsafe { src.add(offset).read() } as u64) & 0x80;
        offset += 1;
    }
    mask
}

#[inline(always)]
unsafe fn copy_small_varbytes_value(src: *const u8, dst: *mut u8, len: usize) {
    match len {
        0 => {}
        1 => {
            // SAFETY: caller proved one byte is readable and writable.
            unsafe {
                dst.write(src.read());
            }
        }
        2 => unsafe {
            ptr::write_unaligned(dst.cast::<u16>(), ptr::read_unaligned(src.cast::<u16>()));
        },
        3 => unsafe {
            ptr::write_unaligned(dst.cast::<u16>(), ptr::read_unaligned(src.cast::<u16>()));
            dst.add(2).write(src.add(2).read());
        },
        4 => unsafe {
            ptr::write_unaligned(dst.cast::<u32>(), ptr::read_unaligned(src.cast::<u32>()));
        },
        5..=7 => unsafe {
            ptr::write_unaligned(dst.cast::<u32>(), ptr::read_unaligned(src.cast::<u32>()));
            ptr::write_unaligned(
                dst.add(len - 4).cast::<u32>(),
                ptr::read_unaligned(src.add(len - 4).cast::<u32>()),
            );
        },
        8 => unsafe {
            ptr::write_unaligned(dst.cast::<u64>(), ptr::read_unaligned(src.cast::<u64>()));
        },
        9..=16 => unsafe {
            ptr::write_unaligned(dst.cast::<u64>(), ptr::read_unaligned(src.cast::<u64>()));
            ptr::write_unaligned(
                dst.add(len - 8).cast::<u64>(),
                ptr::read_unaligned(src.add(len - 8).cast::<u64>()),
            );
        },
        _ => unsafe {
            ptr::copy_nonoverlapping(src, dst, len);
        },
    }
}

#[inline(always)]
unsafe fn copy_small_varbytes_value_ascii_mask(src: *const u8, dst: *mut u8, len: usize) -> u64 {
    match len {
        0 => 0,
        1 => {
            // SAFETY: caller proved one byte is readable and writable.
            let byte = unsafe { src.read() };
            // SAFETY: destination byte is valid.
            unsafe {
                dst.write(byte);
            }
            (byte as u64) & 0x80
        }
        2 => unsafe {
            let word = ptr::read_unaligned(src.cast::<u16>());
            ptr::write_unaligned(dst.cast::<u16>(), word);
            (word as u64) & 0x8080
        },
        3 => unsafe {
            let first = ptr::read_unaligned(src.cast::<u16>());
            let last = src.add(2).read();
            ptr::write_unaligned(dst.cast::<u16>(), first);
            dst.add(2).write(last);
            ((first as u64) & 0x8080) | ((last as u64) & 0x80)
        },
        4 => unsafe {
            let word = ptr::read_unaligned(src.cast::<u32>());
            ptr::write_unaligned(dst.cast::<u32>(), word);
            (word as u64) & 0x8080_8080
        },
        5..=7 => unsafe {
            let first = ptr::read_unaligned(src.cast::<u32>());
            let last = ptr::read_unaligned(src.add(len - 4).cast::<u32>());
            ptr::write_unaligned(dst.cast::<u32>(), first);
            ptr::write_unaligned(dst.add(len - 4).cast::<u32>(), last);
            ((first as u64) | (last as u64)) & 0x8080_8080
        },
        8 => unsafe {
            let word = ptr::read_unaligned(src.cast::<u64>());
            ptr::write_unaligned(dst.cast::<u64>(), word);
            word & ASCII_HIGH_BIT_MASK_U64
        },
        9..=16 => unsafe {
            let first = ptr::read_unaligned(src.cast::<u64>());
            let last = ptr::read_unaligned(src.add(len - 8).cast::<u64>());
            ptr::write_unaligned(dst.cast::<u64>(), first);
            ptr::write_unaligned(dst.add(len - 8).cast::<u64>(), last);
            (first | last) & ASCII_HIGH_BIT_MASK_U64
        },
        _ => unsafe { copy_varbytes_value_ascii_mask(src, dst, len) },
    }
}

fn fixed_u32_no_nulls_len(data: &[u8], row_count: usize) -> Result<Option<usize>, CoveError> {
    if row_count == 0 {
        return Ok(None);
    }
    if data.len() < 4 {
        return Err(CoveError::OffsetRange);
    }
    // SAFETY: `data.len() >= 4` proves the first prefix is readable.
    let fixed_len = unsafe { read_u32_le_unaligned(data.as_ptr()) } as usize;
    let stride = fixed_len.checked_add(4).ok_or(CoveError::ArithOverflow)?;
    let expected_len = row_count
        .checked_mul(stride)
        .ok_or(CoveError::ArithOverflow)?;
    if expected_len != data.len() {
        return Ok(None);
    }
    for row in 1..row_count {
        let pos = row.checked_mul(stride).ok_or(CoveError::ArithOverflow)?;
        // SAFETY: `expected_len == data.len()` and `pos` is a stride boundary
        // for `row < row_count`, so `pos..pos + 4` is in-bounds.
        let len = unsafe { read_u32_le_unaligned(data.as_ptr().add(pos)) } as usize;
        if len != fixed_len {
            return Ok(None);
        }
    }
    Ok(Some(fixed_len))
}

#[inline(always)]
unsafe fn copy_fixed_varbytes_value(src: *const u8, dst: *mut u8, len: usize) {
    if len <= 16 {
        // SAFETY: forwarded caller invariant.
        unsafe {
            copy_small_varbytes_value(src, dst, len);
        }
    } else {
        // SAFETY: forwarded caller invariant.
        unsafe {
            ptr::copy_nonoverlapping(src, dst, len);
        }
    }
}

#[inline(always)]
unsafe fn copy_fixed_varbytes_value_ascii_mask(src: *const u8, dst: *mut u8, len: usize) -> u64 {
    if len <= 16 {
        // SAFETY: forwarded caller invariant.
        unsafe { copy_small_varbytes_value_ascii_mask(src, dst, len) }
    } else {
        // SAFETY: forwarded caller invariant.
        unsafe { copy_varbytes_value_ascii_mask(src, dst, len) }
    }
}

struct BytePayloadPlan {
    layout: BytePayloadLayout,
}

impl BytePayloadPlan {
    fn parse_ranges(&self, array: &EncodedArray<'_>) -> Result<Vec<(usize, usize)>, CoveError> {
        let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
        let mut ranges = Vec::with_capacity(row_count);
        let data = array.data;
        let mut pos = 0usize;
        for _ in 0..row_count {
            let (data_start, data_end) = match self.layout {
                BytePayloadLayout::U32LengthPrefixed => read_u32_len_prefixed_range(data, pos)?,
                BytePayloadLayout::Leb128LengthPrefixed => {
                    read_leb128_len_prefixed_range(data, pos)?
                }
            };
            ranges.push((data_start, data_end));
            pos = data_end;
        }
        if pos != data.len() {
            return Err(CoveError::PageCorrupt);
        }
        Ok(ranges)
    }

    fn materialize(
        &self,
        array: &EncodedArray<'_>,
        selection: ArrowRowSelection<'_>,
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        if matches!(selection, ArrowRowSelection::All) {
            return self.materialize_all_rows(array);
        }
        if let Some(result) = self.materialize_selected_forward_u32(array, selection)? {
            return Ok(result);
        }
        let ranges = self.parse_ranges(array)?;
        self.materialize_selected(array, selection, &ranges)
    }

    fn materialize_utf8(
        &self,
        array: &EncodedArray<'_>,
        selection: ArrowRowSelection<'_>,
        string_validation_policy: ArrowStringValidationPolicy,
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
        let has_nulls = match array.validity {
            Some(validity) => validity.null_count()? > 0,
            None => false,
        };
        if matches!(selection, ArrowRowSelection::All)
            && !has_nulls
            && matches!(self.layout, BytePayloadLayout::U32LengthPrefixed)
        {
            return match string_validation_policy {
                ArrowStringValidationPolicy::Strict
                | ArrowStringValidationPolicy::StrictOrCachedProof => {
                    self.materialize_all_u32_no_nulls_utf8_strict(array, row_count)
                }
                ArrowStringValidationPolicy::TrustedPageProof => {
                    self.materialize_all_u32_no_nulls(array, row_count)
                }
            };
        }
        let (offsets, values, nulls) = self.materialize(array, selection)?;
        if matches!(
            string_validation_policy,
            ArrowStringValidationPolicy::Strict | ArrowStringValidationPolicy::StrictOrCachedProof
        ) {
            validate_utf8_offsets_values(&offsets, &values)?;
        }
        Ok((offsets, values, nulls))
    }

    fn materialize_view(
        &self,
        array: &EncodedArray<'_>,
        selection: ArrowRowSelection<'_>,
        data_owner: Option<&ArrowBufferOwner>,
    ) -> Result<ArrowViewParts, CoveError> {
        if matches!(selection, ArrowRowSelection::All) {
            return self.materialize_view_all_rows(array, data_owner);
        }
        let ranges = self.parse_ranges(array)?;
        let selected_len = selection.selected_len(array.row_count)?;
        let has_nulls = match array.validity {
            Some(validity) => validity.null_count()? > 0,
            None => false,
        };
        let mut views = Vec::with_capacity(selected_len);
        let mut validity_builder = has_nulls
            .then(|| ArrowValidityBuilder::new(selected_len))
            .transpose()?;
        if data_owner.is_none() {
            return self.materialize_view_selected_owned(
                array,
                selection,
                &ranges,
                has_nulls,
                views,
                validity_builder,
            );
        }
        selection.for_each_row(array.row_count, |row| {
            let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
            let is_null = has_nulls && array.is_null(row_u64)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            if is_null {
                views.push(0u128);
                return Ok(());
            }
            let (start, end) = ranges[row];
            views.push(byte_view_for_range(array.data, start, end)?);
            Ok(())
        })?;

        let buffers = vec![byte_view_backing_buffer(array, data_owner)?];
        Ok((
            ScalarBuffer::from(views),
            buffers,
            validity_builder.and_then(ArrowValidityBuilder::finish),
        ))
    }

    fn materialize_view_selected_owned(
        &self,
        array: &EncodedArray<'_>,
        selection: ArrowRowSelection<'_>,
        ranges: &[(usize, usize)],
        has_nulls: bool,
        mut views: Vec<u128>,
        mut validity_builder: Option<ArrowValidityBuilder>,
    ) -> Result<ArrowViewParts, CoveError> {
        let mut values = Vec::with_capacity(array.data.len().min(views.capacity() * 16));
        selection.for_each_row(array.row_count, |row| {
            let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
            let is_null = has_nulls && array.is_null(row_u64)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            if is_null {
                views.push(0u128);
                return Ok(());
            }
            let (source_start, source_end) = ranges[row];
            let target_start = values.len();
            values.extend_from_slice(&array.data[source_start..source_end]);
            let target_end = values.len();
            views.push(byte_view_for_range(&values, target_start, target_end)?);
            Ok(())
        })?;

        Ok((
            ScalarBuffer::from(views),
            vec![Buffer::from_vec(values)],
            validity_builder.and_then(ArrowValidityBuilder::finish),
        ))
    }

    fn materialize_view_all_rows(
        &self,
        array: &EncodedArray<'_>,
        data_owner: Option<&ArrowBufferOwner>,
    ) -> Result<ArrowViewParts, CoveError> {
        let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
        let has_nulls = match array.validity {
            Some(validity) => validity.null_count()? > 0,
            None => false,
        };
        let mut views = Vec::with_capacity(row_count);
        let mut validity_builder = has_nulls
            .then(|| ArrowValidityBuilder::new(row_count))
            .transpose()?;

        let data = array.data;
        let mut pos = 0usize;
        for row in 0..array.row_count {
            let (data_start, data_end) = match self.layout {
                BytePayloadLayout::U32LengthPrefixed => read_u32_len_prefixed_range(data, pos)?,
                BytePayloadLayout::Leb128LengthPrefixed => {
                    read_leb128_len_prefixed_range(data, pos)?
                }
            };
            pos = data_end;

            let is_null = has_nulls && array.is_null(row)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            if is_null {
                views.push(0u128);
            } else {
                views.push(byte_view_for_range(data, data_start, data_end)?);
            }
        }
        if pos != data.len() {
            return Err(CoveError::PageCorrupt);
        }

        let buffers = vec![byte_view_backing_buffer(array, data_owner)?];
        Ok((
            ScalarBuffer::from(views),
            buffers,
            validity_builder.and_then(ArrowValidityBuilder::finish),
        ))
    }

    fn materialize_all_rows(
        &self,
        array: &EncodedArray<'_>,
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
        let has_nulls = match array.validity {
            Some(validity) => validity.null_count()? > 0,
            None => false,
        };
        if !has_nulls && matches!(self.layout, BytePayloadLayout::U32LengthPrefixed) {
            return self.materialize_all_u32_no_nulls(array, row_count);
        }
        let Some(offset_capacity) = row_count.checked_add(1) else {
            return Err(CoveError::ArithOverflow);
        };
        let mut offsets = Vec::<i32>::with_capacity(offset_capacity);
        let mut values = Vec::with_capacity(array.data.len());
        let mut validity_builder = has_nulls
            .then(|| ArrowValidityBuilder::new(row_count))
            .transpose()?;
        offsets.push(0i32);

        let data = array.data;
        let mut pos = 0usize;
        for row in 0..array.row_count {
            let (data_start, data_end) = match self.layout {
                BytePayloadLayout::U32LengthPrefixed => read_u32_len_prefixed_range(data, pos)?,
                BytePayloadLayout::Leb128LengthPrefixed => {
                    read_leb128_len_prefixed_range(data, pos)?
                }
            };
            pos = data_end;

            let is_null = has_nulls && array.is_null(row)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            if !is_null {
                values.extend_from_slice(&data[data_start..data_end]);
            }
            offsets.push(i32::try_from(values.len()).map_err(|_| CoveError::ArithOverflow)?);
        }
        if pos != data.len() {
            return Err(CoveError::PageCorrupt);
        }

        let offsets = trusted_i32_offset_buffer(offsets);
        let nulls = validity_builder.and_then(ArrowValidityBuilder::finish);
        Ok((offsets, Buffer::from_vec(values), nulls))
    }

    fn materialize_all_u32_no_nulls(
        &self,
        array: &EncodedArray<'_>,
        row_count: usize,
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        self.materialize_all_u32_no_nulls_impl::<false>(array, row_count)
    }

    fn materialize_all_u32_no_nulls_utf8_strict(
        &self,
        array: &EncodedArray<'_>,
        row_count: usize,
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        self.materialize_all_u32_no_nulls_impl::<true>(array, row_count)
    }

    fn materialize_all_u32_no_nulls_impl<const VALIDATE_UTF8: bool>(
        &self,
        array: &EncodedArray<'_>,
        row_count: usize,
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        let data = array.data;
        // The fixed-length detector needs its own prefix scan. Keep it narrow:
        // large pages are faster on the generic one-pass copy path.
        if row_count <= FIXED_U32_NO_NULLS_MAX_ROWS
            && data.len() <= FIXED_U32_NO_NULLS_MAX_DATA_BYTES
        {
            if let Some(fixed_len) = fixed_u32_no_nulls_len(data, row_count)? {
                return self.materialize_all_u32_no_nulls_fixed_impl::<VALIDATE_UTF8>(
                    data, row_count, fixed_len,
                );
            }
        }
        let Some(prefix_bytes) = row_count.checked_mul(4) else {
            return Err(CoveError::ArithOverflow);
        };
        if prefix_bytes > data.len() {
            return Err(CoveError::OffsetRange);
        }
        let value_len = data.len() - prefix_bytes;
        if value_len > i32::MAX as usize {
            return Err(CoveError::ArithOverflow);
        }

        let Some(offset_capacity) = row_count.checked_add(1) else {
            return Err(CoveError::ArithOverflow);
        };
        let mut offsets = Vec::<i32>::with_capacity(offset_capacity);
        let mut values = Vec::<u8>::with_capacity(value_len);
        let mut pos = 0usize;
        let mut write = 0usize;
        let mut saw_non_ascii = false;
        let offsets_ptr = offsets.as_mut_ptr();
        // INVARIANT: offset 0 is always initialized, and the vector length is
        // published only after all row offsets have been written.
        // SAFETY: `offsets` has capacity `row_count + 1`, so slot 0 is valid.
        unsafe {
            offsets_ptr.write(0i32);
        }
        for row in 0..row_count {
            if pos > data.len().saturating_sub(4) {
                return Err(CoveError::OffsetRange);
            }
            // INVARIANT: the branch above proves that `pos..pos + 4` is
            // in-bounds for `data`.
            // SAFETY: the length prefix pointer is valid for four bytes.
            let len = unsafe { read_u32_le_unaligned(data.as_ptr().add(pos)) } as usize;
            pos += 4;
            if len > data.len() - pos {
                return Err(CoveError::OffsetRange);
            }
            if len > value_len - write {
                return Err(CoveError::PageCorrupt);
            }
            let data_end = pos + len;
            let next_write = write + len;
            let src = data.as_ptr().wrapping_add(pos);
            let dst = values.as_mut_ptr().wrapping_add(write);
            // INVARIANT: bounds above prove source and destination ranges are
            // in-bounds and non-overlapping; destination length is published
            // only after every row has validated.
            // SAFETY: `values` has at least `value_len` capacity, source points
            // into immutable `data`, and both pointers are valid for `len`
            // bytes.
            if VALIDATE_UTF8 {
                saw_non_ascii |= unsafe { copy_varbytes_value_ascii_mask(src, dst, len) } != 0;
            } else {
                unsafe {
                    copy_varbytes_value(src, dst, len);
                }
            }
            pos = data_end;
            write = next_write;
            // INVARIANT: `value_len <= i32::MAX` was pre-proven and
            // `write <= value_len` is maintained by the checked length branch.
            // SAFETY: `row + 1 < offset_capacity`, so this raw write targets a
            // reserved offset slot.
            unsafe {
                offsets_ptr.add(row + 1).write(write as i32);
            }
        }
        if pos != data.len() || write != value_len {
            return Err(CoveError::PageCorrupt);
        }
        // INVARIANT: every byte in 0..value_len was initialized exactly once by
        // the checked copy loop above.
        // SAFETY: the vector has capacity `value_len`, and all elements in the
        // new initialized length have been written.
        unsafe {
            values.set_len(value_len);
        }
        // INVARIANT: slot 0 and one offset per row were initialized in order.
        // SAFETY: all elements in 0..offset_capacity have been written.
        unsafe {
            offsets.set_len(offset_capacity);
        }
        if VALIDATE_UTF8 && saw_non_ascii {
            validate_utf8_offsets_slice(&offsets, &values)?;
        }

        Ok((
            trusted_i32_offset_buffer(offsets),
            Buffer::from_vec(values),
            None,
        ))
    }

    fn materialize_all_u32_no_nulls_fixed_impl<const VALIDATE_UTF8: bool>(
        &self,
        data: &[u8],
        row_count: usize,
        fixed_len: usize,
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        let value_len = row_count
            .checked_mul(fixed_len)
            .ok_or(CoveError::ArithOverflow)?;
        if value_len > i32::MAX as usize {
            return Err(CoveError::ArithOverflow);
        }
        let Some(offset_capacity) = row_count.checked_add(1) else {
            return Err(CoveError::ArithOverflow);
        };
        let mut offsets = Vec::<i32>::with_capacity(offset_capacity);
        let mut values = Vec::<u8>::with_capacity(value_len);
        let mut saw_non_ascii = false;
        let offsets_ptr = offsets.as_mut_ptr();
        let values_ptr = values.as_mut_ptr();
        // INVARIANT: fixed-length U32 VarBytes pages have already been
        // pre-scanned for exact row count, equal prefixes, and total length.
        // Offsets are therefore an arithmetic progression by `fixed_len`.
        // SAFETY: `offsets` has `row_count + 1` capacity and every slot is
        // written exactly once before `set_len`.
        unsafe {
            offsets_ptr.write(0i32);
        }
        let stride = fixed_len.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        for row in 0..row_count {
            let data_start = row
                .checked_mul(stride)
                .and_then(|offset| offset.checked_add(4))
                .ok_or(CoveError::ArithOverflow)?;
            let write = row.checked_mul(fixed_len).ok_or(CoveError::ArithOverflow)?;
            let src = data.as_ptr().wrapping_add(data_start);
            let dst = values_ptr.wrapping_add(write);
            // INVARIANT: `fixed_u32_no_nulls_len` proved every source range is
            // in-bounds. `value_len` was computed as `row_count * fixed_len`,
            // so each destination range lies inside the reserved capacity.
            // SAFETY: source and destination ranges are valid for `fixed_len`
            // bytes and do not overlap.
            if VALIDATE_UTF8 {
                saw_non_ascii |=
                    unsafe { copy_fixed_varbytes_value_ascii_mask(src, dst, fixed_len) } != 0;
            } else {
                unsafe {
                    copy_fixed_varbytes_value(src, dst, fixed_len);
                }
            }
            let next = write
                .checked_add(fixed_len)
                .ok_or(CoveError::ArithOverflow)?;
            // SAFETY: `row + 1 < offset_capacity`, and `next <= value_len <= i32::MAX`.
            unsafe {
                offsets_ptr.add(row + 1).write(next as i32);
            }
        }
        // SAFETY: all destination bytes and offset slots were initialized by
        // the fixed-length copy loop above.
        unsafe {
            values.set_len(value_len);
            offsets.set_len(offset_capacity);
        }
        if VALIDATE_UTF8 && saw_non_ascii {
            validate_utf8_offsets_slice(&offsets, &values)?;
        }
        Ok((
            trusted_i32_offset_buffer(offsets),
            Buffer::from_vec(values),
            None,
        ))
    }

    fn materialize_selected(
        &self,
        array: &EncodedArray<'_>,
        selection: ArrowRowSelection<'_>,
        ranges: &[(usize, usize)],
    ) -> Result<(OffsetBuffer<i32>, Buffer, Option<NullBuffer>), CoveError> {
        let selected_len = selection.selected_len(array.row_count)?;
        let has_nulls = match array.validity {
            Some(validity) => validity.null_count()? > 0,
            None => false,
        };
        let mut value_len = 0usize;
        let mut any_null = false;
        selection.for_each_row(array.row_count, |row| {
            let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
            let is_null = has_nulls && array.is_null(row_u64)?;
            any_null |= is_null;
            if !is_null {
                let (start, end) = ranges[row];
                value_len = value_len
                    .checked_add(end.checked_sub(start).ok_or(CoveError::PageCorrupt)?)
                    .ok_or(CoveError::ArithOverflow)?;
            }
            Ok(())
        })?;
        i32::try_from(value_len).map_err(|_| CoveError::ArithOverflow)?;

        let Some(offset_capacity) = selected_len.checked_add(1) else {
            return Err(CoveError::ArithOverflow);
        };
        let mut offsets = Vec::with_capacity(offset_capacity);
        let mut values = Vec::<u8>::with_capacity(value_len);
        let mut validity_builder = any_null
            .then(|| ArrowValidityBuilder::new(selected_len))
            .transpose()?;
        let mut write = 0usize;
        offsets.push(0i32);
        selection.for_each_row(array.row_count, |row| {
            let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
            let is_null = has_nulls && array.is_null(row_u64)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            if !is_null {
                let (start, end) = ranges[row];
                let len = end.checked_sub(start).ok_or(CoveError::PageCorrupt)?;
                let next = write.checked_add(len).ok_or(CoveError::ArithOverflow)?;
                if next > value_len {
                    return Err(CoveError::PageCorrupt);
                }
                // INVARIANT: the prepass computed `value_len` from the same
                // selected non-null ranges, and every source range was parsed
                // and bounds-checked before this copy pass.
                // SAFETY: `values` has capacity `value_len`, source and
                // destination ranges are in-bounds and non-overlapping, and the
                // initialized length is set only after all copies finish.
                unsafe {
                    ptr::copy_nonoverlapping(
                        array.data.as_ptr().add(start),
                        values.as_mut_ptr().add(write),
                        len,
                    );
                }
                write = next;
            }
            offsets.push(i32::try_from(write).map_err(|_| CoveError::ArithOverflow)?);
            Ok(())
        })?;
        if write != value_len {
            return Err(CoveError::PageCorrupt);
        }
        // INVARIANT: selected copy loop initialized exactly the `write` prefix,
        // and `write == value_len` was proven by the prepass/debug assertion.
        // SAFETY: the vector capacity is `value_len` and all bytes in that
        // range have been written.
        unsafe {
            values.set_len(value_len);
        }

        let offsets = trusted_i32_offset_buffer(offsets);
        let nulls = validity_builder.and_then(ArrowValidityBuilder::finish);
        Ok((offsets, Buffer::from_vec(values), nulls))
    }

    fn materialize_selected_forward_u32(
        &self,
        array: &EncodedArray<'_>,
        selection: ArrowRowSelection<'_>,
    ) -> Result<Option<ArrowOffsetParts>, CoveError> {
        if !matches!(self.layout, BytePayloadLayout::U32LengthPrefixed) {
            return Ok(None);
        }
        let selected_len = selection.selected_len(array.row_count)?;
        if selected_len == 0 {
            return Ok(Some((
                trusted_i32_offset_buffer(vec![0]),
                Buffer::from_vec(Vec::<u8>::new()),
                None,
            )));
        }
        let last_selected = match selection {
            ArrowRowSelection::Rows(rows) => {
                if !rows.windows(2).all(|pair| pair[0] < pair[1]) {
                    return Ok(None);
                }
                let last = *rows.last().ok_or(CoveError::OffsetRange)? as usize;
                if u64::try_from(last).map_err(|_| CoveError::ArithOverflow)? >= array.row_count {
                    return Err(CoveError::OffsetRange);
                }
                last
            }
            ArrowRowSelection::Bitset { words, len } => {
                selection.validate_for_row_count(array.row_count)?;
                last_selected_bitset_row(words, len).ok_or(CoveError::OffsetRange)?
            }
            ArrowRowSelection::All => return Ok(None),
        };

        let has_nulls = array_has_nulls(array)?;
        let Some(offset_capacity) = selected_len.checked_add(1) else {
            return Err(CoveError::ArithOverflow);
        };
        let mut offsets = Vec::with_capacity(offset_capacity);
        let mut values = Vec::<u8>::with_capacity(array.data.len().min(selected_len * 16));
        let mut validity_builder = has_nulls
            .then(|| ArrowValidityBuilder::new(selected_len))
            .transpose()?;
        offsets.push(0);
        let mut pos = 0usize;
        let mut next_row_index = 0usize;
        for row in 0..=last_selected {
            let (data_start, data_end) = read_u32_len_prefixed_range(array.data, pos)?;
            pos = data_end;
            let selected = match selection {
                ArrowRowSelection::Rows(rows) => {
                    if rows
                        .get(next_row_index)
                        .map(|candidate| *candidate as usize == row)
                        .unwrap_or(false)
                    {
                        next_row_index += 1;
                        true
                    } else {
                        false
                    }
                }
                ArrowRowSelection::Bitset { words, .. } => bitset_row_selected(words, row),
                ArrowRowSelection::All => false,
            };
            if !selected {
                continue;
            }
            let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
            let is_null = has_nulls && array.is_null(row_u64)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            if !is_null {
                values.extend_from_slice(&array.data[data_start..data_end]);
            }
            offsets.push(i32::try_from(values.len()).map_err(|_| CoveError::ArithOverflow)?);
        }
        if last_selected + 1
            == usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?
            && pos != array.data.len()
        {
            return Err(CoveError::PageCorrupt);
        }
        if offsets.len() != offset_capacity {
            return Err(CoveError::PageCorrupt);
        }
        let offsets = trusted_i32_offset_buffer(offsets);
        let nulls = validity_builder.and_then(ArrowValidityBuilder::finish);
        Ok(Some((offsets, Buffer::from_vec(values), nulls)))
    }
}

fn bitset_row_selected(words: &[u64], row: usize) -> bool {
    words
        .get(row / 64)
        .map(|word| (*word & (1u64 << (row % 64))) != 0)
        .unwrap_or(false)
}

fn last_selected_bitset_row(words: &[u64], len: usize) -> Option<usize> {
    let word_len = len.div_ceil(64);
    for word_index in (0..word_len).rev() {
        let mut word = words.get(word_index).copied().unwrap_or(0);
        if word_index + 1 == word_len {
            word = mask_selection_tail(word, len);
        }
        if word != 0 {
            return Some(word_index * 64 + (63 - word.leading_zeros() as usize));
        }
    }
    None
}

fn try_direct_primitive_array(
    array: &EncodedArray<'_>,
    data_type: &DataType,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<Option<ArrayRef>, CoveError> {
    match array.encoding {
        CoveEncodingKind::NumCode if array.physical == CovePhysicalKind::NumCode => match data_type
        {
            DataType::Int64 => {
                if let Some(values) = retained_numcode_i64_values(array, data_owner)? {
                    let nulls = retained_array_nulls(array)?;
                    return Ok(Some(Arc::new(Int64Array::new(values, nulls)) as ArrayRef));
                }
                Ok(Some(Arc::new(numcode_i64_array(array)?) as ArrayRef))
            }
            DataType::UInt64 => {
                if let Some(values) = retained_numcode_u64_values(array, data_owner)? {
                    let nulls = retained_array_nulls(array)?;
                    return Ok(Some(Arc::new(UInt64Array::new(values, nulls)) as ArrayRef));
                }
                Ok(Some(Arc::new(numcode_u64_array(array)?) as ArrayRef))
            }
            DataType::Timestamp(TimeUnit::Microsecond, None) => {
                if let Some(values) = retained_numcode_i64_values(array, data_owner)? {
                    let nulls = retained_array_nulls(array)?;
                    return Ok(Some(
                        Arc::new(TimestampMicrosecondArray::new(values, nulls)) as ArrayRef,
                    ));
                }
                Ok(Some(
                    Arc::new(timestamp_micros_array(array, ArrowRowSelection::All)?) as ArrayRef,
                ))
            }
            DataType::Timestamp(TimeUnit::Nanosecond, None) => {
                if let Some(values) = retained_numcode_i64_values(array, data_owner)? {
                    let nulls = retained_array_nulls(array)?;
                    return Ok(Some(
                        Arc::new(TimestampNanosecondArray::new(values, nulls)) as ArrayRef
                    ));
                }
                Ok(Some(
                    Arc::new(timestamp_nanos_array(array, ArrowRowSelection::All)?) as ArrayRef,
                ))
            }
            _ => Ok(None),
        },
        CoveEncodingKind::PlainFixed
            if array.logical == CoveLogicalType::Bool && *data_type == DataType::Boolean =>
        {
            Ok(Some(Arc::new(plain_bool_array(array)?) as ArrayRef))
        }
        CoveEncodingKind::PlainFixed => {
            try_direct_plain_fixed_array(array, ArrowRowSelection::All, data_type, data_owner)
        }
        _ => Ok(None),
    }
}

fn try_direct_primitive_array_for_selection(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
) -> Result<Option<ArrayRef>, CoveError> {
    match array.encoding {
        CoveEncodingKind::NumCode if array.physical == CovePhysicalKind::NumCode => match data_type
        {
            DataType::Int64 => Ok(Some(
                Arc::new(numcode_i64_array_for_selection(array, selection)?) as ArrayRef,
            )),
            DataType::UInt64 => Ok(Some(
                Arc::new(numcode_u64_array_for_selection(array, selection)?) as ArrayRef,
            )),
            DataType::Timestamp(TimeUnit::Microsecond, None) => Ok(Some(Arc::new(
                timestamp_micros_array(array, selection)?,
            ) as ArrayRef)),
            DataType::Timestamp(TimeUnit::Nanosecond, None) => Ok(Some(Arc::new(
                timestamp_nanos_array(array, selection)?,
            ) as ArrayRef)),
            _ => Ok(None),
        },
        CoveEncodingKind::PlainFixed
            if array.logical == CoveLogicalType::Bool && *data_type == DataType::Boolean =>
        {
            Ok(Some(
                Arc::new(plain_bool_array_for_selection(array, selection)?) as ArrayRef,
            ))
        }
        CoveEncodingKind::PlainFixed => {
            try_direct_plain_fixed_array(array, selection, data_type, None)
        }
        _ => Ok(None),
    }
}

fn try_direct_plain_fixed_array(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<Option<ArrayRef>, CoveError> {
    if array.encoding != CoveEncodingKind::PlainFixed {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let width = crate::array::logical_type_fixed_width(array.logical).ok_or_else(|| {
        CoveError::UnsupportedEncoding(format!(
            "PlainFixed Arrow export requires fixed-width logical type, got {:?}",
            array.logical
        ))
    })?;
    let data = fixed_width_payload_prefix(array.data, row_count, width)?;
    match data_type {
        DataType::Int8 => Ok(Some(Arc::new(plain_fixed_native_array::<Int8Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<1>(bytes).map(i8::from_le_bytes),
        )?) as ArrayRef)),
        DataType::Int16 => Ok(Some(Arc::new(plain_fixed_native_array::<Int16Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<2>(bytes).map(i16::from_le_bytes),
        )?) as ArrayRef)),
        DataType::Int32 => Ok(Some(Arc::new(plain_fixed_native_array::<Int32Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<4>(bytes).map(i32::from_le_bytes),
        )?) as ArrayRef)),
        DataType::Date32 => {
            let (values, nulls) =
                collect_plain_fixed_native::<i32, _>(array, selection, data, width, |bytes| {
                    exact_bytes::<4>(bytes).map(i32::from_le_bytes)
                })?;
            Ok(Some(
                Arc::new(Date32Array::new(ScalarBuffer::from(values), nulls)) as ArrayRef,
            ))
        }
        DataType::Int64 => Ok(Some(Arc::new(plain_fixed_native_array::<Int64Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<8>(bytes).map(i64::from_le_bytes),
        )?) as ArrayRef)),
        DataType::UInt8 => Ok(Some(Arc::new(plain_fixed_native_array::<UInt8Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<1>(bytes).map(u8::from_le_bytes),
        )?) as ArrayRef)),
        DataType::UInt16 => Ok(Some(Arc::new(plain_fixed_native_array::<UInt16Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<2>(bytes).map(u16::from_le_bytes),
        )?) as ArrayRef)),
        DataType::UInt32 => Ok(Some(Arc::new(plain_fixed_native_array::<UInt32Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<4>(bytes).map(u32::from_le_bytes),
        )?) as ArrayRef)),
        DataType::UInt64 => Ok(Some(Arc::new(plain_fixed_native_array::<UInt64Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<8>(bytes).map(u64::from_le_bytes),
        )?) as ArrayRef)),
        DataType::Float32 => Ok(Some(Arc::new(plain_fixed_native_array::<Float32Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<4>(bytes).map(|raw| f32::from_bits(u32::from_le_bytes(raw))),
        )?) as ArrayRef)),
        DataType::Float64 => Ok(Some(Arc::new(plain_fixed_native_array::<Float64Type, _>(
            array,
            selection,
            data,
            width,
            |bytes| exact_bytes::<8>(bytes).map(|raw| f64::from_bits(u64::from_le_bytes(raw))),
        )?) as ArrayRef)),
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            let (values, nulls) =
                collect_plain_fixed_native::<i64, _>(array, selection, data, width, |bytes| {
                    exact_bytes::<8>(bytes).map(i64::from_le_bytes)
                })?;
            Ok(Some(Arc::new(TimestampMicrosecondArray::new(
                ScalarBuffer::from(values),
                nulls,
            )) as ArrayRef))
        }
        DataType::Timestamp(TimeUnit::Nanosecond, None) => {
            let (values, nulls) =
                collect_plain_fixed_native::<i64, _>(array, selection, data, width, |bytes| {
                    exact_bytes::<8>(bytes).map(i64::from_le_bytes)
                })?;
            Ok(Some(Arc::new(TimestampNanosecondArray::new(
                ScalarBuffer::from(values),
                nulls,
            )) as ArrayRef))
        }
        DataType::Decimal128(precision, scale) => {
            let (values, nulls) = collect_plain_fixed_decimal128(array, selection, data, width)?;
            let array = Decimal128Array::new(ScalarBuffer::from(values), nulls)
                .with_precision_and_scale(*precision, *scale)
                .map_err(|err| CoveError::BadSection(format!("Arrow Decimal128: {err}")))?;
            Ok(Some(Arc::new(array) as ArrayRef))
        }
        DataType::FixedSizeBinary(size) => {
            validate_fixed_size_binary_width(*size, width)?;
            if let Some(values) =
                retained_plain_fixed_binary_buffer(array, selection, data, width, data_owner)?
            {
                let nulls = retained_array_nulls(array)?;
                let array = FixedSizeBinaryArray::try_new(*size, values, nulls).map_err(|err| {
                    CoveError::BadSection(format!("Arrow FixedSizeBinary: {err}"))
                })?;
                return Ok(Some(Arc::new(array) as ArrayRef));
            }
            let (values, nulls) = collect_plain_fixed_bytes(array, selection, data, width)?;
            let array = FixedSizeBinaryArray::try_new(*size, Buffer::from_vec(values), nulls)
                .map_err(|err| CoveError::BadSection(format!("Arrow FixedSizeBinary: {err}")))?;
            Ok(Some(Arc::new(array) as ArrayRef))
        }
        _ => Ok(None),
    }
}

fn validate_fixed_size_binary_width(size: i32, width: usize) -> Result<(), CoveError> {
    let size = usize::try_from(size).map_err(|_| CoveError::PageCorrupt)?;
    if size != width {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

fn retained_plain_fixed_binary_buffer(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data: &[u8],
    width: usize,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<Option<Buffer>, CoveError> {
    let Some(owner) = data_owner else {
        return Ok(None);
    };
    if !selection.is_all_rows(array.row_count)? {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let byte_len = row_count
        .checked_mul(width)
        .ok_or(CoveError::ArithOverflow)?;
    if byte_len == 0 {
        return Ok(Some(Buffer::from_vec(Vec::<u8>::new())));
    }
    if data.len() < byte_len {
        return Err(CoveError::OffsetRange);
    }
    let Some(ptr) = NonNull::new(data.as_ptr() as *mut u8) else {
        return Err(CoveError::BufferTooShort);
    };
    // INVARIANT: `data_owner` owns the immutable retained COVE page allocation
    // containing `data`; Arrow clones that owner into the custom allocation, so
    // the byte-addressed fixed-size values buffer stays live for the array.
    Ok(Some(unsafe {
        Buffer::from_custom_allocation(ptr, byte_len, Arc::clone(owner))
    }))
}

fn plain_fixed_native_array<T, F>(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data: &[u8],
    width: usize,
    decode: F,
) -> Result<arrow_array::PrimitiveArray<T>, CoveError>
where
    T: arrow_array::types::ArrowPrimitiveType,
    T::Native: Default,
    F: Fn(&[u8]) -> Result<T::Native, CoveError>,
{
    let (values, nulls) =
        collect_plain_fixed_native::<T::Native, F>(array, selection, data, width, decode)?;
    Ok(arrow_array::PrimitiveArray::<T>::new(
        ScalarBuffer::from(values),
        nulls,
    ))
}

fn collect_plain_fixed_native<T, F>(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data: &[u8],
    width: usize,
    decode: F,
) -> Result<(Vec<T>, Option<NullBuffer>), CoveError>
where
    T: ArrowNativeType + Default,
    F: Fn(&[u8]) -> Result<T, CoveError>,
{
    let has_nulls = array_has_nulls(array)?;
    let selected_len = selection.selected_len(array.row_count)?;
    let mut values = Vec::with_capacity(selected_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    selection.for_each_row(array.row_count, |row| {
        let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        let is_null = has_nulls && array.is_null(row_u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if is_null {
            values.push(T::default());
            return Ok(());
        }
        let offset = row.checked_mul(width).ok_or(CoveError::ArithOverflow)?;
        let end = offset.checked_add(width).ok_or(CoveError::ArithOverflow)?;
        values.push(decode(
            data.get(offset..end).ok_or(CoveError::OffsetRange)?,
        )?);
        Ok(())
    })?;
    Ok((
        values,
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn collect_plain_fixed_decimal128(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data: &[u8],
    width: usize,
) -> Result<(Vec<i128>, Option<NullBuffer>), CoveError> {
    match array.logical {
        CoveLogicalType::Decimal64 => {
            collect_plain_fixed_native(array, selection, data, width, |bytes| {
                exact_bytes::<8>(bytes)
                    .map(i64::from_le_bytes)
                    .map(i128::from)
            })
        }
        CoveLogicalType::Decimal128 => {
            collect_plain_fixed_native(array, selection, data, width, |bytes| {
                exact_bytes::<16>(bytes).map(i128::from_le_bytes)
            })
        }
        _ => Err(CoveError::UnsupportedEncoding(format!(
            "Decimal128 Arrow export from {:?}",
            array.logical
        ))),
    }
}

fn collect_plain_fixed_bytes(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data: &[u8],
    width: usize,
) -> Result<(Vec<u8>, Option<NullBuffer>), CoveError> {
    let has_nulls = array_has_nulls(array)?;
    let selected_len = selection.selected_len(array.row_count)?;
    let value_len = selected_len
        .checked_mul(width)
        .ok_or(CoveError::ArithOverflow)?;
    let mut values = Vec::<u8>::with_capacity(value_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    selection.for_each_row(array.row_count, |row| {
        let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        let is_null = has_nulls && array.is_null(row_u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        let offset = row.checked_mul(width).ok_or(CoveError::ArithOverflow)?;
        let end = offset.checked_add(width).ok_or(CoveError::ArithOverflow)?;
        if is_null {
            values.resize(values.len() + width, 0);
        } else {
            values.extend_from_slice(data.get(offset..end).ok_or(CoveError::OffsetRange)?);
        }
        Ok(())
    })?;
    Ok((
        values,
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn try_direct_decoded_array(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
) -> Result<Option<ArrayRef>, CoveError> {
    match array.encoding {
        CoveEncodingKind::Constant => {
            let payload = ConstantPayload::parse(array.data)?;
            if payload.row_count != array.row_count {
                return Err(CoveError::PageCorrupt);
            }
            if array.physical == CovePhysicalKind::NumCode {
                return Ok(None);
            }
            direct_i64_values_to_arrow(array, selection, data_type, |row| {
                let _ = row;
                Ok(payload.value)
            })
        }
        CoveEncodingKind::PlainVarint => {
            let values = decode_plain_varint_u64_values(array)?;
            direct_u64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::Rle => {
            let payload = RlePayload::parse(array.data)?;
            let values = Rle::fast_decode(&payload)?;
            direct_i64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::RunEnd => {
            let payload = RunEndPayload::parse(array.data)?;
            let values = RunEnd::fast_decode(&payload)?;
            direct_i64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::BitPacked => {
            let payload = BitPackedPayload::parse(array.data)?;
            let values = BitPacked::fast_decode(&payload)?;
            direct_i64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::Delta => {
            let payload = DeltaPayload::parse(array.data)?;
            let values = Delta::fast_decode(&payload)?;
            direct_i64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::FrameOfReference => {
            let payload = ForPayload::parse(array.data)?;
            let values = FrameOfReference::fast_decode(&payload)?;
            direct_i64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::PatchedBase => {
            let payload = PatchedBasePayload::parse(array.data)?;
            let values = PatchedBase::fast_decode(&payload)?;
            direct_i64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::Sparse => {
            let payload = SparsePayload::parse(array.data)?;
            let values = Sparse::fast_decode(&payload)?;
            direct_i64_slice_to_arrow(array, &values, selection, data_type)
        }
        CoveEncodingKind::LocalCodebook => {
            try_direct_local_codebook_array(array, selection, data_type)
        }
        _ => Ok(None),
    }
}

fn decode_plain_varint_u64_values(array: &EncodedArray<'_>) -> Result<Vec<u64>, CoveError> {
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let mut values = Vec::with_capacity(row_count);
    let mut pos = 0usize;
    for _ in 0..row_count {
        if pos >= array.data.len() {
            return Err(CoveError::OffsetRange);
        }
        let (value, consumed) = wire::decode_u64_leb128(&array.data[pos..])?;
        pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
        values.push(value);
    }
    if pos != array.data.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok(values)
}

fn direct_i64_slice_to_arrow(
    array: &EncodedArray<'_>,
    values: &[i64],
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
) -> Result<Option<ArrayRef>, CoveError> {
    if values.len() != usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)? {
        return Err(CoveError::PageCorrupt);
    }
    direct_i64_values_to_arrow(array, selection, data_type, |row| {
        values.get(row).copied().ok_or(CoveError::PageCorrupt)
    })
}

fn direct_i64_values_to_arrow<F>(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
    value_at: F,
) -> Result<Option<ArrayRef>, CoveError>
where
    F: Fn(usize) -> Result<i64, CoveError>,
{
    match data_type {
        DataType::Boolean => {
            let (values, selected_len, nulls) =
                collect_i64_bool_values(array, selection, value_at)?;
            let values = BooleanBuffer::new(Buffer::from_vec(values), 0, selected_len);
            Ok(Some(Arc::new(BooleanArray::new(values, nulls)) as ArrayRef))
        }
        DataType::Int8 => Ok(Some(Arc::new(i64_values_primitive_array::<Int8Type, _, _>(
            array,
            selection,
            value_at,
            |value| i8::try_from(value).map_err(|_| CoveError::PageCorrupt),
        )?) as ArrayRef)),
        DataType::Int16 => Ok(Some(
            Arc::new(i64_values_primitive_array::<Int16Type, _, _>(
                array,
                selection,
                value_at,
                |value| i16::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::Int32 => Ok(Some(
            Arc::new(i64_values_primitive_array::<Int32Type, _, _>(
                array,
                selection,
                value_at,
                |value| i32::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::Date32 => {
            let (values, nulls) =
                collect_i64_values::<i32, _, _>(array, selection, value_at, |value| {
                    i32::try_from(value).map_err(|_| CoveError::PageCorrupt)
                })?;
            Ok(Some(
                Arc::new(Date32Array::new(ScalarBuffer::from(values), nulls)) as ArrayRef,
            ))
        }
        DataType::Int64 => Ok(Some(
            Arc::new(i64_values_primitive_array::<Int64Type, _, _>(
                array, selection, value_at, Ok,
            )?) as ArrayRef,
        )),
        DataType::UInt8 => Ok(Some(
            Arc::new(i64_values_primitive_array::<UInt8Type, _, _>(
                array,
                selection,
                value_at,
                |value| u8::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::UInt16 => Ok(Some(
            Arc::new(i64_values_primitive_array::<UInt16Type, _, _>(
                array,
                selection,
                value_at,
                |value| u16::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::UInt32 => Ok(Some(
            Arc::new(i64_values_primitive_array::<UInt32Type, _, _>(
                array,
                selection,
                value_at,
                |value| u32::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::UInt64 => Ok(Some(
            Arc::new(i64_values_primitive_array::<UInt64Type, _, _>(
                array,
                selection,
                value_at,
                |value| u64::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            let (values, nulls) = collect_i64_values::<i64, _, _>(array, selection, value_at, Ok)?;
            Ok(Some(Arc::new(TimestampMicrosecondArray::new(
                ScalarBuffer::from(values),
                nulls,
            )) as ArrayRef))
        }
        DataType::Timestamp(TimeUnit::Nanosecond, None) => {
            let (values, nulls) = collect_i64_values::<i64, _, _>(array, selection, value_at, Ok)?;
            Ok(Some(Arc::new(TimestampNanosecondArray::new(
                ScalarBuffer::from(values),
                nulls,
            )) as ArrayRef))
        }
        DataType::Decimal128(precision, scale) if array.logical == CoveLogicalType::Decimal64 => {
            let (values, nulls) =
                collect_i64_values::<i128, _, _>(array, selection, value_at, |value| {
                    Ok(i128::from(value))
                })?;
            let array = Decimal128Array::new(ScalarBuffer::from(values), nulls)
                .with_precision_and_scale(*precision, *scale)
                .map_err(|err| CoveError::BadSection(format!("Arrow Decimal128: {err}")))?;
            Ok(Some(Arc::new(array) as ArrayRef))
        }
        _ => Ok(None),
    }
}

fn i64_values_primitive_array<T, F, C>(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    value_at: F,
    cast: C,
) -> Result<arrow_array::PrimitiveArray<T>, CoveError>
where
    T: arrow_array::types::ArrowPrimitiveType,
    T::Native: Default,
    F: Fn(usize) -> Result<i64, CoveError>,
    C: Fn(i64) -> Result<T::Native, CoveError>,
{
    let (values, nulls) = collect_i64_values::<T::Native, F, C>(array, selection, value_at, cast)?;
    Ok(arrow_array::PrimitiveArray::<T>::new(
        ScalarBuffer::from(values),
        nulls,
    ))
}

fn collect_i64_values<T, F, C>(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    value_at: F,
    cast: C,
) -> Result<(Vec<T>, Option<NullBuffer>), CoveError>
where
    T: ArrowNativeType + Default,
    F: Fn(usize) -> Result<i64, CoveError>,
    C: Fn(i64) -> Result<T, CoveError>,
{
    let has_nulls = array_has_nulls(array)?;
    let selected_len = selection.selected_len(array.row_count)?;
    let mut values = Vec::with_capacity(selected_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    selection.for_each_row(array.row_count, |row| {
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if is_null {
            values.push(T::default());
        } else {
            values.push(cast(value_at(row)?)?);
        }
        Ok(())
    })?;
    Ok((
        values,
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn collect_i64_bool_values<F>(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    value_at: F,
) -> Result<(Vec<u8>, usize, Option<NullBuffer>), CoveError>
where
    F: Fn(usize) -> Result<i64, CoveError>,
{
    let has_nulls = array_has_nulls(array)?;
    let selected_len = selection.selected_len(array.row_count)?;
    let mut values = vec![0u8; bitpacked_len(selected_len)?];
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    let mut out_row = 0usize;
    selection.for_each_row(array.row_count, |row| {
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if !is_null {
            match value_at(row)? {
                0 => {}
                1 => set_packed_bit(&mut values, out_row),
                _ => return Err(CoveError::PageCorrupt),
            }
        }
        out_row += 1;
        Ok(())
    })?;
    Ok((
        values,
        selected_len,
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn try_direct_local_codebook_array(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
) -> Result<Option<ArrayRef>, CoveError> {
    let payload = LocalCodebookPayload::parse(array.data)?;
    match (&payload.values, data_type) {
        (LocalCodebookValues::FileCode(_), _) | (LocalCodebookValues::NumCode(_), _) => {
            let values = payload.decode_num_codes().or_else(|_| {
                payload
                    .decode_file_codes()
                    .map(|codes| codes.into_iter().map(u64::from).collect::<Vec<_>>())
            })?;
            if values.len()
                != usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?
            {
                return Err(CoveError::PageCorrupt);
            }
            direct_u64_slice_to_arrow(array, &values, selection, data_type)
        }
        (LocalCodebookValues::Boolean(_), DataType::Boolean) => {
            let values = payload.decode_booleans()?;
            if values.len()
                != usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?
            {
                return Err(CoveError::PageCorrupt);
            }
            let (packed, selected_len, nulls) =
                collect_bool_slice_values(array, &values, selection)?;
            let packed = BooleanBuffer::new(Buffer::from_vec(packed), 0, selected_len);
            Ok(Some(Arc::new(BooleanArray::new(packed, nulls)) as ArrayRef))
        }
        (LocalCodebookValues::VarBytes(_), DataType::Utf8 | DataType::Binary) => {
            let values = payload.decode_var_bytes()?;
            if values.len()
                != usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?
            {
                return Err(CoveError::PageCorrupt);
            }
            direct_bytes_vec_to_arrow(array, &values, selection, data_type)
        }
        _ => Ok(None),
    }
}

fn direct_u64_slice_to_arrow(
    array: &EncodedArray<'_>,
    values: &[u64],
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
) -> Result<Option<ArrayRef>, CoveError> {
    if values.len() != usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)? {
        return Err(CoveError::PageCorrupt);
    }
    match data_type {
        DataType::UInt8 => Ok(Some(
            Arc::new(u64_values_primitive_array::<UInt8Type, _, _>(
                array,
                selection,
                |row| values.get(row).copied().ok_or(CoveError::PageCorrupt),
                |value| u8::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::UInt16 => Ok(Some(
            Arc::new(u64_values_primitive_array::<UInt16Type, _, _>(
                array,
                selection,
                |row| values.get(row).copied().ok_or(CoveError::PageCorrupt),
                |value| u16::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::UInt32 => Ok(Some(
            Arc::new(u64_values_primitive_array::<UInt32Type, _, _>(
                array,
                selection,
                |row| values.get(row).copied().ok_or(CoveError::PageCorrupt),
                |value| u32::try_from(value).map_err(|_| CoveError::PageCorrupt),
            )?) as ArrayRef,
        )),
        DataType::UInt64 => Ok(Some(
            Arc::new(u64_values_primitive_array::<UInt64Type, _, _>(
                array,
                selection,
                |row| values.get(row).copied().ok_or(CoveError::PageCorrupt),
                Ok,
            )?) as ArrayRef,
        )),
        _ => direct_i64_values_to_arrow(array, selection, data_type, |row| {
            let value = values.get(row).copied().ok_or(CoveError::PageCorrupt)?;
            i64::try_from(value).map_err(|_| CoveError::PageCorrupt)
        }),
    }
}

fn u64_values_primitive_array<T, F, C>(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    value_at: F,
    cast: C,
) -> Result<arrow_array::PrimitiveArray<T>, CoveError>
where
    T: arrow_array::types::ArrowPrimitiveType,
    T::Native: Default,
    F: Fn(usize) -> Result<u64, CoveError>,
    C: Fn(u64) -> Result<T::Native, CoveError>,
{
    let has_nulls = array_has_nulls(array)?;
    let selected_len = selection.selected_len(array.row_count)?;
    let mut values = Vec::with_capacity(selected_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    selection.for_each_row(array.row_count, |row| {
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if is_null {
            values.push(T::Native::default());
        } else {
            values.push(cast(value_at(row)?)?);
        }
        Ok(())
    })?;
    Ok(arrow_array::PrimitiveArray::<T>::new(
        ScalarBuffer::from(values),
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn collect_bool_slice_values(
    array: &EncodedArray<'_>,
    values: &[bool],
    selection: ArrowRowSelection<'_>,
) -> Result<(Vec<u8>, usize, Option<NullBuffer>), CoveError> {
    let has_nulls = array_has_nulls(array)?;
    let selected_len = selection.selected_len(array.row_count)?;
    let mut packed = vec![0u8; bitpacked_len(selected_len)?];
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    let mut out_row = 0usize;
    selection.for_each_row(array.row_count, |row| {
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if !is_null && *values.get(row).ok_or(CoveError::PageCorrupt)? {
            set_packed_bit(&mut packed, out_row);
        }
        out_row += 1;
        Ok(())
    })?;
    Ok((
        packed,
        selected_len,
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn direct_bytes_vec_to_arrow(
    array: &EncodedArray<'_>,
    values: &[Vec<u8>],
    selection: ArrowRowSelection<'_>,
    data_type: &DataType,
) -> Result<Option<ArrayRef>, CoveError> {
    let has_nulls = array_has_nulls(array)?;
    match data_type {
        DataType::Utf8 => {
            let mut builder = StringBuilder::new();
            selection.for_each_row(array.row_count, |row| {
                let is_null = has_nulls && array.is_null(row as u64)?;
                if is_null {
                    builder.append_null();
                    return Ok(());
                }
                let bytes = values.get(row).ok_or(CoveError::PageCorrupt)?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|err| CoveError::BadSection(format!("Arrow Utf8 export: {err}")))?;
                builder.append_value(text);
                Ok(())
            })?;
            Ok(Some(Arc::new(builder.finish()) as ArrayRef))
        }
        DataType::Binary => {
            let mut builder = BinaryBuilder::new();
            selection.for_each_row(array.row_count, |row| {
                let is_null = has_nulls && array.is_null(row as u64)?;
                if is_null {
                    builder.append_null();
                } else {
                    builder.append_value(values.get(row).ok_or(CoveError::PageCorrupt)?);
                }
                Ok(())
            })?;
            Ok(Some(Arc::new(builder.finish()) as ArrayRef))
        }
        _ => Ok(None),
    }
}

fn fixed_width_payload_prefix(
    data: &[u8],
    row_count: usize,
    width: usize,
) -> Result<&[u8], CoveError> {
    let Some(required_len) = row_count.checked_mul(width) else {
        return Err(CoveError::ArithOverflow);
    };
    if data.len() < required_len {
        return Err(CoveError::OffsetRange);
    }
    Ok(&data[..required_len])
}

fn array_has_nulls(array: &EncodedArray<'_>) -> Result<bool, CoveError> {
    Ok(match array.validity {
        Some(validity) => validity.null_count()? > 0,
        None => false,
    })
}

#[inline]
fn read_numcode_u64(data: &[u8], row: usize) -> u64 {
    let offset = row * 8;
    // INVARIANT: callers validate `data` as an 8-byte fixed-width prefix for
    // the full row count and validate every selected row before reading.
    // SAFETY: `offset..offset + 8` is therefore in-bounds; unaligned loads are
    // explicitly allowed by `read_unaligned`.
    unsafe { u64::from_le(ptr::read_unaligned(data.as_ptr().add(offset) as *const u64)) }
}

fn retained_numcode_u64_values(
    array: &EncodedArray<'_>,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<Option<ScalarBuffer<u64>>, CoveError> {
    let Some(owner) = data_owner else {
        return Ok(None);
    };
    if !cfg!(target_endian = "little") {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let data = fixed_width_payload_prefix(array.data, row_count, 8)?;
    retained_numcode_scalar_buffer::<u64>(data, row_count, owner)
}

fn retained_numcode_i64_values(
    array: &EncodedArray<'_>,
    data_owner: Option<&ArrowBufferOwner>,
) -> Result<Option<ScalarBuffer<i64>>, CoveError> {
    let Some(owner) = data_owner else {
        return Ok(None);
    };
    if !cfg!(target_endian = "little") {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let data = fixed_width_payload_prefix(array.data, row_count, 8)?;
    for row in 0..row_count {
        checked_numcode_i64(read_numcode_u64(data, row))?;
    }
    retained_numcode_scalar_buffer::<i64>(data, row_count, owner)
}

fn retained_array_nulls(array: &EncodedArray<'_>) -> Result<Option<NullBuffer>, CoveError> {
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    arrow_null_buffer(array.validity, row_count)
}

fn retained_numcode_scalar_buffer<T: ArrowNativeType>(
    data: &[u8],
    row_count: usize,
    owner: &ArrowBufferOwner,
) -> Result<Option<ScalarBuffer<T>>, CoveError> {
    let Some(byte_len) = row_count.checked_mul(std::mem::size_of::<T>()) else {
        return Err(CoveError::ArithOverflow);
    };
    if byte_len == 0 {
        return Ok(Some(ScalarBuffer::from(Vec::<T>::new())));
    }
    if data.len() < byte_len {
        return Err(CoveError::OffsetRange);
    }
    let align = std::mem::align_of::<T>();
    if !(data.as_ptr() as usize).is_multiple_of(align) {
        return Ok(None);
    }
    let Some(ptr) = NonNull::new(data.as_ptr() as *mut u8) else {
        return Err(CoveError::BufferTooShort);
    };
    // INVARIANT: the returned Arrow buffer points into immutable retained COVE
    // page data. The `owner` is cloned into Arrow's custom allocation so the
    // backing bytes outlive every array using this buffer.
    // SAFETY: `data` was proven valid for `byte_len` bytes, the pointer is
    // non-null and aligned for `T`, and only little-endian fixed-width NumCode
    // payloads reach this helper.
    let buffer = unsafe { Buffer::from_custom_allocation(ptr, byte_len, Arc::clone(owner)) };
    Ok(Some(ScalarBuffer::new(buffer, 0, row_count)))
}

fn numcode_u64_array(array: &EncodedArray<'_>) -> Result<UInt64Array, CoveError> {
    let (values, nulls) = collect_numcode_u64_buffers(array, ArrowRowSelection::All)?;
    Ok(UInt64Array::new(ScalarBuffer::from(values), nulls))
}

fn numcode_u64_array_for_selection(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<UInt64Array, CoveError> {
    let (values, nulls) = collect_numcode_u64_buffers(array, selection)?;
    Ok(UInt64Array::new(ScalarBuffer::from(values), nulls))
}

fn numcode_i64_array(array: &EncodedArray<'_>) -> Result<Int64Array, CoveError> {
    let (values, nulls) = collect_numcode_i64_buffers(array, ArrowRowSelection::All)?;
    Ok(Int64Array::new(ScalarBuffer::from(values), nulls))
}

fn numcode_i64_array_for_selection(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<Int64Array, CoveError> {
    let (values, nulls) = collect_numcode_i64_buffers(array, selection)?;
    Ok(Int64Array::new(ScalarBuffer::from(values), nulls))
}

fn copy_numcode_bytes_to_vec<T: ArrowNativeType>(
    data: &[u8],
    row_count: usize,
    out: &mut Vec<T>,
) -> Result<(), CoveError> {
    let byte_len = row_count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(CoveError::ArithOverflow)?;
    if data.len() < byte_len {
        return Err(CoveError::OffsetRange);
    }
    // INVARIANT: NumCode uses little-endian fixed-width 8-byte payloads. This
    // helper is only called for no-null native Arrow buffers whose bytes are
    // identical to the checked COVE payload representation.
    // SAFETY: `out` has capacity for `row_count` native values. Copying through
    // `u8` pointers avoids source alignment requirements, and every destination
    // byte for the final vector length is initialized before `set_len`.
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), out.as_mut_ptr().cast::<u8>(), byte_len);
        out.set_len(row_count);
    }
    Ok(())
}

fn timestamp_micros_array(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<TimestampMicrosecondArray, CoveError> {
    let (values, nulls) = collect_numcode_i64_buffers(array, selection)?;
    Ok(TimestampMicrosecondArray::new(
        ScalarBuffer::from(values),
        nulls,
    ))
}

fn timestamp_nanos_array(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<TimestampNanosecondArray, CoveError> {
    let (values, nulls) = collect_numcode_i64_buffers(array, selection)?;
    Ok(TimestampNanosecondArray::new(
        ScalarBuffer::from(values),
        nulls,
    ))
}

fn collect_numcode_u64_buffers(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<(Vec<u64>, Option<NullBuffer>), CoveError> {
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let data = fixed_width_payload_prefix(array.data, row_count, 8)?;
    let has_nulls = array_has_nulls(array)?;
    match selection {
        ArrowRowSelection::All => collect_numcode_u64_all(array, data, row_count, has_nulls),
        ArrowRowSelection::Rows(rows) => {
            selection.validate_for_row_count(array.row_count)?;
            collect_numcode_u64_rows(array, data, rows, has_nulls)
        }
        ArrowRowSelection::Bitset { words, len } => {
            selection.validate_for_row_count(array.row_count)?;
            let selected_len = count_bitset_rows(words, len)?;
            collect_numcode_u64_bitset(array, data, words, len, selected_len, has_nulls)
        }
    }
}

fn collect_numcode_i64_buffers(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<(Vec<i64>, Option<NullBuffer>), CoveError> {
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let data = fixed_width_payload_prefix(array.data, row_count, 8)?;
    let has_nulls = array_has_nulls(array)?;
    match selection {
        ArrowRowSelection::All => collect_numcode_i64_all(array, data, row_count, has_nulls),
        ArrowRowSelection::Rows(rows) => {
            selection.validate_for_row_count(array.row_count)?;
            collect_numcode_i64_rows(array, data, rows, has_nulls)
        }
        ArrowRowSelection::Bitset { words, len } => {
            selection.validate_for_row_count(array.row_count)?;
            let selected_len = count_bitset_rows(words, len)?;
            collect_numcode_i64_bitset(array, data, words, len, selected_len, has_nulls)
        }
    }
}

fn collect_numcode_u64_all(
    array: &EncodedArray<'_>,
    data: &[u8],
    row_count: usize,
    has_nulls: bool,
) -> Result<(Vec<u64>, Option<NullBuffer>), CoveError> {
    let mut out = Vec::<u64>::with_capacity(row_count);
    if !has_nulls {
        copy_numcode_bytes_to_vec(data, row_count, &mut out)?;
        return Ok((out, None));
    }

    let mut validity_builder = ArrowValidityBuilder::new(row_count)?;
    for row in 0..row_count {
        let is_null = array.is_null(row as u64)?;
        validity_builder.append(!is_null);
        out.push(if is_null {
            0
        } else {
            read_numcode_u64(data, row)
        });
    }
    Ok((out, validity_builder.finish()))
}

fn collect_numcode_u64_rows(
    array: &EncodedArray<'_>,
    data: &[u8],
    rows: &[u32],
    has_nulls: bool,
) -> Result<(Vec<u64>, Option<NullBuffer>), CoveError> {
    let mut out = Vec::with_capacity(rows.len());
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(rows.len()))
        .transpose()?;
    for row in rows {
        let row = *row as usize;
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        out.push(if is_null {
            0
        } else {
            read_numcode_u64(data, row)
        });
    }
    Ok((out, validity_builder.and_then(ArrowValidityBuilder::finish)))
}

fn collect_numcode_u64_bitset(
    array: &EncodedArray<'_>,
    data: &[u8],
    words: &[u64],
    len: usize,
    selected_len: usize,
    has_nulls: bool,
) -> Result<(Vec<u64>, Option<NullBuffer>), CoveError> {
    let mut out = Vec::with_capacity(selected_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    let word_len = len.div_ceil(64);
    for (word_index, raw_word) in words.iter().take(word_len).copied().enumerate() {
        let mut word = if word_index + 1 == word_len {
            mask_selection_tail(raw_word, len)
        } else {
            raw_word
        };
        while word != 0 {
            let row = word_index * 64 + word.trailing_zeros() as usize;
            let is_null = has_nulls && array.is_null(row as u64)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            out.push(if is_null {
                0
            } else {
                read_numcode_u64(data, row)
            });
            word &= word - 1;
        }
    }
    Ok((out, validity_builder.and_then(ArrowValidityBuilder::finish)))
}

#[inline]
fn checked_numcode_i64(value: u64) -> Result<i64, CoveError> {
    if value > i64::MAX as u64 {
        return Err(CoveError::PageCorrupt);
    }
    Ok(value as i64)
}

fn collect_numcode_i64_all(
    array: &EncodedArray<'_>,
    data: &[u8],
    row_count: usize,
    has_nulls: bool,
) -> Result<(Vec<i64>, Option<NullBuffer>), CoveError> {
    let mut out = Vec::<i64>::with_capacity(row_count);
    if !has_nulls {
        for row in 0..row_count {
            checked_numcode_i64(read_numcode_u64(data, row))?;
        }
        copy_numcode_bytes_to_vec(data, row_count, &mut out)?;
        return Ok((out, None));
    }

    let mut validity_builder = ArrowValidityBuilder::new(row_count)?;
    for row in 0..row_count {
        let is_null = array.is_null(row as u64)?;
        validity_builder.append(!is_null);
        out.push(if is_null {
            0
        } else {
            checked_numcode_i64(read_numcode_u64(data, row))?
        });
    }
    Ok((out, validity_builder.finish()))
}

fn collect_numcode_i64_rows(
    array: &EncodedArray<'_>,
    data: &[u8],
    rows: &[u32],
    has_nulls: bool,
) -> Result<(Vec<i64>, Option<NullBuffer>), CoveError> {
    let mut out = Vec::with_capacity(rows.len());
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(rows.len()))
        .transpose()?;
    for row in rows {
        let row = *row as usize;
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        out.push(if is_null {
            0
        } else {
            checked_numcode_i64(read_numcode_u64(data, row))?
        });
    }
    Ok((out, validity_builder.and_then(ArrowValidityBuilder::finish)))
}

fn collect_numcode_i64_bitset(
    array: &EncodedArray<'_>,
    data: &[u8],
    words: &[u64],
    len: usize,
    selected_len: usize,
    has_nulls: bool,
) -> Result<(Vec<i64>, Option<NullBuffer>), CoveError> {
    let mut out = Vec::with_capacity(selected_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    let word_len = len.div_ceil(64);
    for (word_index, raw_word) in words.iter().take(word_len).copied().enumerate() {
        let mut word = if word_index + 1 == word_len {
            mask_selection_tail(raw_word, len)
        } else {
            raw_word
        };
        while word != 0 {
            let row = word_index * 64 + word.trailing_zeros() as usize;
            let is_null = has_nulls && array.is_null(row as u64)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            out.push(if is_null {
                0
            } else {
                checked_numcode_i64(read_numcode_u64(data, row))?
            });
            word &= word - 1;
        }
    }
    Ok((out, validity_builder.and_then(ArrowValidityBuilder::finish)))
}

fn plain_bool_array(array: &EncodedArray<'_>) -> Result<BooleanArray, CoveError> {
    plain_bool_array_for_selection(array, ArrowRowSelection::All)
}

fn plain_bool_array_for_selection(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<BooleanArray, CoveError> {
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let data = fixed_width_payload_prefix(array.data, row_count, 1)?;
    let has_nulls = array_has_nulls(array)?;
    let (values, selected_len, nulls) = match selection {
        ArrowRowSelection::All => collect_bool_all(array, data, row_count, has_nulls)?,
        ArrowRowSelection::Rows(rows) => {
            selection.validate_for_row_count(array.row_count)?;
            collect_bool_rows(array, data, rows, has_nulls)?
        }
        ArrowRowSelection::Bitset { words, len } => {
            selection.validate_for_row_count(array.row_count)?;
            let selected_len = count_bitset_rows(words, len)?;
            collect_bool_bitset(array, data, words, len, selected_len, has_nulls)?
        }
    };
    let values = BooleanBuffer::new(Buffer::from_vec(values), 0, selected_len);
    Ok(BooleanArray::new(values, nulls))
}

#[inline(always)]
fn checked_bool_byte(byte: u8) -> Result<bool, CoveError> {
    match byte {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CoveError::PageCorrupt),
    }
}

#[inline(always)]
fn pack_bool_chunk_8(chunk: &[u8]) -> Result<u8, CoveError> {
    debug_assert_eq!(chunk.len(), 8);
    let b0 = chunk[0];
    let b1 = chunk[1];
    let b2 = chunk[2];
    let b3 = chunk[3];
    let b4 = chunk[4];
    let b5 = chunk[5];
    let b6 = chunk[6];
    let b7 = chunk[7];
    if (b0 | b1 | b2 | b3 | b4 | b5 | b6 | b7) > 1 {
        return Err(CoveError::PageCorrupt);
    }
    Ok(b0 | (b1 << 1) | (b2 << 2) | (b3 << 3) | (b4 << 4) | (b5 << 5) | (b6 << 6) | (b7 << 7))
}

#[inline(always)]
fn pack_bool_chunk_16(chunk: &[u8]) -> Result<u16, CoveError> {
    debug_assert_eq!(chunk.len(), 16);
    let mut low = [0u8; 8];
    let mut high = [0u8; 8];
    low.copy_from_slice(&chunk[..8]);
    high.copy_from_slice(&chunk[8..16]);
    let packed_low = pack_bool_chunk_8(&low)?;
    let packed_high = pack_bool_chunk_8(&high)?;
    Ok(u16::from(packed_low) | (u16::from(packed_high) << 8))
}

fn collect_bool_all(
    array: &EncodedArray<'_>,
    data: &[u8],
    row_count: usize,
    has_nulls: bool,
) -> Result<(Vec<u8>, usize, Option<NullBuffer>), CoveError> {
    let mut values = vec![0u8; bitpacked_len(row_count)?];
    if !has_nulls {
        let mut chunks = data.chunks_exact(16);
        for (word_index, chunk) in chunks.by_ref().enumerate() {
            let packed = pack_bool_chunk_16(chunk)?.to_le_bytes();
            let offset = word_index * 2;
            values[offset] = packed[0];
            if offset + 1 < values.len() {
                values[offset + 1] = packed[1];
            }
        }
        let tail_start = row_count - chunks.remainder().len();
        for (bit, byte) in chunks.remainder().iter().copied().enumerate() {
            if checked_bool_byte(byte)? {
                set_packed_bit(&mut values, tail_start + bit);
            }
        }
        return Ok((values, row_count, None));
    }

    let mut validity_builder = ArrowValidityBuilder::new(row_count)?;
    for (row, byte) in data.iter().copied().enumerate() {
        let bit = checked_bool_byte(byte)?;
        let is_null = array.is_null(row as u64)?;
        validity_builder.append(!is_null);
        if !is_null && bit {
            set_packed_bit(&mut values, row);
        }
    }
    Ok((values, row_count, validity_builder.finish()))
}

fn collect_bool_rows(
    array: &EncodedArray<'_>,
    data: &[u8],
    rows: &[u32],
    has_nulls: bool,
) -> Result<(Vec<u8>, usize, Option<NullBuffer>), CoveError> {
    let mut values = vec![0u8; bitpacked_len(rows.len())?];
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(rows.len()))
        .transpose()?;
    for (out_row, row) in rows.iter().copied().enumerate() {
        let row = row as usize;
        let bit = checked_bool_byte(data[row])?;
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if !is_null && bit {
            set_packed_bit(&mut values, out_row);
        }
    }
    Ok((
        values,
        rows.len(),
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn collect_bool_bitset(
    array: &EncodedArray<'_>,
    data: &[u8],
    words: &[u64],
    len: usize,
    selected_len: usize,
    has_nulls: bool,
) -> Result<(Vec<u8>, usize, Option<NullBuffer>), CoveError> {
    let mut values = vec![0u8; bitpacked_len(selected_len)?];
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    let mut out_row = 0usize;
    let word_len = len.div_ceil(64);
    for (word_index, raw_word) in words.iter().take(word_len).copied().enumerate() {
        let mut word = if word_index + 1 == word_len {
            mask_selection_tail(raw_word, len)
        } else {
            raw_word
        };
        while word != 0 {
            let row = word_index * 64 + word.trailing_zeros() as usize;
            let bit = checked_bool_byte(data[row])?;
            let is_null = has_nulls && array.is_null(row as u64)?;
            if let Some(builder) = &mut validity_builder {
                builder.append(!is_null);
            }
            if !is_null && bit {
                set_packed_bit(&mut values, out_row);
            }
            out_row += 1;
            word &= word - 1;
        }
    }
    Ok((
        values,
        selected_len,
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

fn values_to_arrow_array_with_data_type(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
    data_type: DataType,
) -> Result<ArrayRef, CoveError> {
    Ok(match data_type {
        DataType::Boolean => Arc::new(BooleanArray::from(collect_bool(values)?)) as ArrayRef,
        DataType::Int8 => Arc::new(Int8Array::from(collect_i64(logical, values, |v| {
            i8::try_from(v).map_err(|_| CoveError::PageCorrupt)
        })?)) as ArrayRef,
        DataType::Int16 => Arc::new(Int16Array::from(collect_i64(logical, values, |v| {
            i16::try_from(v).map_err(|_| CoveError::PageCorrupt)
        })?)) as ArrayRef,
        DataType::Int32 => Arc::new(Int32Array::from(collect_i64(logical, values, |v| {
            i32::try_from(v).map_err(|_| CoveError::PageCorrupt)
        })?)) as ArrayRef,
        DataType::Int64 => {
            Arc::new(Int64Array::from(collect_i64(logical, values, Ok)?)) as ArrayRef
        }
        DataType::UInt8 => Arc::new(UInt8Array::from(collect_u64(logical, values, |v| {
            u8::try_from(v).map_err(|_| CoveError::PageCorrupt)
        })?)) as ArrayRef,
        DataType::UInt16 => Arc::new(UInt16Array::from(collect_u64(logical, values, |v| {
            u16::try_from(v).map_err(|_| CoveError::PageCorrupt)
        })?)) as ArrayRef,
        DataType::UInt32 => Arc::new(UInt32Array::from(collect_u64(logical, values, |v| {
            u32::try_from(v).map_err(|_| CoveError::PageCorrupt)
        })?)) as ArrayRef,
        DataType::UInt64 => {
            Arc::new(UInt64Array::from(collect_u64(logical, values, Ok)?)) as ArrayRef
        }
        DataType::Float32 => Arc::new(Float32Array::from(collect_f32(values)?)) as ArrayRef,
        DataType::Float64 => Arc::new(Float64Array::from(collect_f64(values)?)) as ArrayRef,
        DataType::Date32 => Arc::new(Date32Array::from(collect_i64(logical, values, |v| {
            i32::try_from(v).map_err(|_| CoveError::PageCorrupt)
        })?)) as ArrayRef,
        DataType::Timestamp(TimeUnit::Microsecond, None) => Arc::new(
            TimestampMicrosecondArray::from(collect_i64(logical, values, Ok)?),
        ) as ArrayRef,
        DataType::Timestamp(TimeUnit::Nanosecond, None) => Arc::new(TimestampNanosecondArray::from(
            collect_i64(logical, values, Ok)?,
        )) as ArrayRef,
        DataType::Utf8 => Arc::new(collect_utf8(logical, values)?) as ArrayRef,
        DataType::Utf8View => Arc::new(collect_utf8_view(logical, values)?) as ArrayRef,
        DataType::Binary => Arc::new(collect_binary(logical, values)?) as ArrayRef,
        DataType::BinaryView => Arc::new(collect_binary_view(logical, values)?) as ArrayRef,
        DataType::FixedSizeBinary(size) => {
            Arc::new(collect_fixed_size_binary(logical, values, size)?) as ArrayRef
        }
        DataType::Decimal128(precision, scale) => Arc::new(
            Decimal128Array::from(collect_i128(logical, values)?)
                .with_precision_and_scale(precision, scale)
                .map_err(|err| CoveError::BadSection(format!("Arrow Decimal128: {err}")))?,
        ) as ArrayRef,
        other => {
            return Err(CoveError::UnsupportedEncoding(format!(
                "Arrow export for {other:?}"
            )));
        }
    })
}

fn arrow_data_type(logical: CoveLogicalType) -> Result<DataType, CoveError> {
    match logical {
        CoveLogicalType::Bool => Ok(DataType::Boolean),
        CoveLogicalType::Int8 => Ok(DataType::Int8),
        CoveLogicalType::Int16 => Ok(DataType::Int16),
        CoveLogicalType::Int32 => Ok(DataType::Int32),
        CoveLogicalType::Int64 => Ok(DataType::Int64),
        CoveLogicalType::UInt8 => Ok(DataType::UInt8),
        CoveLogicalType::UInt16 => Ok(DataType::UInt16),
        CoveLogicalType::UInt32 => Ok(DataType::UInt32),
        CoveLogicalType::UInt64 => Ok(DataType::UInt64),
        CoveLogicalType::Float32 => Ok(DataType::Float32),
        CoveLogicalType::Float64 => Ok(DataType::Float64),
        CoveLogicalType::DateDays => Ok(DataType::Date32),
        CoveLogicalType::TimestampMicros => Ok(DataType::Timestamp(TimeUnit::Microsecond, None)),
        CoveLogicalType::TimestampNanos => Ok(DataType::Timestamp(TimeUnit::Nanosecond, None)),
        CoveLogicalType::Utf8 | CoveLogicalType::Json => Ok(DataType::Utf8),
        CoveLogicalType::Binary => Ok(DataType::Binary),
        CoveLogicalType::Uuid => Ok(DataType::FixedSizeBinary(16)),
        other => Err(CoveError::UnsupportedEncoding(format!(
            "Arrow export for {:?}",
            other
        ))),
    }
}

/// Return the Arrow data type used by the default decoded COVE export path.
///
/// This mirrors [`encoded_array_to_arrow`] without requiring callers to build
/// a synthetic array just to construct an Arrow schema.
pub fn decoded_arrow_data_type(logical: CoveLogicalType) -> Result<DataType, CoveError> {
    let result = arrow_data_type_for_export_options(logical, ArrowExportOptions::default())?;
    if result.report.has_lossy_or_unsupported() {
        return Err(CoveError::UnsupportedEncoding(format!(
            "Arrow export for {logical:?} requires explicit fidelity reporting"
        )));
    }
    Ok(result.value)
}

/// Return the Arrow data type for a COVE logical type under explicit export
/// options, including the same fidelity diagnostics as value export.
pub fn arrow_data_type_for_export_options(
    logical: CoveLogicalType,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<DataType>, CoveError> {
    let mut report = ArrowExportReport::default();
    let value = arrow_data_type_with_report(logical, &options, &mut report)?;
    Ok(ArrowExportResult { value, report })
}

/// Return the Arrow data type for a concrete COVE column under explicit export
/// options. This includes physical representation choices such as FileCode
/// dictionary-key output when a file dictionary is available.
pub fn arrow_data_type_for_column_export_options(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    has_file_dictionary: bool,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<DataType>, CoveError> {
    let dictionary_output = options.dictionary_policy == ArrowDictionaryPolicy::DictionaryKeys
        && physical == CovePhysicalKind::FileCode
        && has_file_dictionary;
    let value_options = if dictionary_output {
        filecode_dictionary_value_export_options(options)
    } else {
        options
    };
    let mut result = arrow_data_type_for_export_options(logical, value_options)?;
    if dictionary_output {
        result.report.push(
            None,
            logical,
            ArrowFidelitySeverity::Informational,
            "FileCode values exported as Arrow dictionary keys",
        );
        result.value = DataType::Dictionary(Box::new(DataType::UInt32), Box::new(result.value));
    }
    Ok(result)
}

pub fn arrow_data_type_for_nested_schema_node(
    node: &NestedSchemaNodeV1,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<DataType>, CoveError> {
    let mut report = ArrowExportReport::default();
    let value = arrow_data_type_for_nested_schema_node_with_report(node, options, &mut report)?;
    Ok(ArrowExportResult { value, report })
}

fn arrow_data_type_for_nested_schema_node_with_report(
    node: &NestedSchemaNodeV1,
    options: ArrowExportOptions,
    report: &mut ArrowExportReport,
) -> Result<DataType, CoveError> {
    match node.physical {
        CovePhysicalKind::List => {
            if node.children.len() != 1 {
                return Err(CoveError::BadSchema(
                    "NestedSchema List node must have exactly one child".into(),
                ));
            }
            let child = &node.children[0];
            let child_type =
                arrow_data_type_for_nested_schema_node_with_report(child, options, report)?;
            let field = Arc::new(Field::new(child.name.clone(), child_type, child.nullable));
            if node.fixed_size_list_len == 0 {
                Ok(DataType::List(field))
            } else {
                Ok(DataType::FixedSizeList(
                    field,
                    i32::try_from(node.fixed_size_list_len)
                        .map_err(|_| CoveError::ArithOverflow)?,
                ))
            }
        }
        CovePhysicalKind::Struct => {
            let mut fields = Vec::with_capacity(node.children.len());
            for child in &node.children {
                let child_type =
                    arrow_data_type_for_nested_schema_node_with_report(child, options, report)?;
                fields.push(Field::new(child.name.clone(), child_type, child.nullable));
            }
            Ok(DataType::Struct(Fields::from(fields)))
        }
        CovePhysicalKind::Map => {
            if node.children.len() != 2 {
                return Err(CoveError::BadSchema(
                    "NestedSchema Map node must have key and value children".into(),
                ));
            }
            let key = &node.children[0];
            let value = &node.children[1];
            let key_type =
                arrow_data_type_for_nested_schema_node_with_report(key, options, report)?;
            let value_type =
                arrow_data_type_for_nested_schema_node_with_report(value, options, report)?;
            let entries = Fields::from(vec![
                Field::new("key", key_type, false),
                Field::new("value", value_type, value.nullable),
            ]);
            Ok(DataType::Map(
                Arc::new(Field::new("entries", DataType::Struct(entries), false)),
                false,
            ))
        }
        _ => {
            let scalar_options = if matches!(
                node.logical,
                CoveLogicalType::Decimal64 | CoveLogicalType::Decimal128
            ) && node.precision != 0
            {
                ArrowExportOptions {
                    decimal: Some(ArrowDecimalContext {
                        precision: u8::try_from(node.precision)
                            .map_err(|_| CoveError::ArithOverflow)?,
                        scale: i8::try_from(node.scale).map_err(|_| CoveError::ArithOverflow)?,
                    }),
                    ..options
                }
            } else {
                options
            };
            arrow_data_type_with_report(node.logical, &scalar_options, report)
        }
    }
}

fn arrow_data_type_with_report(
    logical: CoveLogicalType,
    options: &ArrowExportOptions,
    report: &mut ArrowExportReport,
) -> Result<DataType, CoveError> {
    match logical {
        CoveLogicalType::Decimal64 => match options.decimal {
            Some(decimal) => Ok(DataType::Decimal128(decimal.precision, decimal.scale)),
            None => {
                report.push(
                    None,
                    logical,
                    ArrowFidelitySeverity::Lossy,
                    "Decimal64 exported as Int64 because no Arrow decimal precision/scale context was supplied",
                );
                Ok(DataType::Int64)
            }
        },
        CoveLogicalType::Decimal128 => match options.decimal {
            Some(decimal) => Ok(DataType::Decimal128(decimal.precision, decimal.scale)),
            None => {
                report.push(
                    None,
                    logical,
                    ArrowFidelitySeverity::Lossy,
                    "Decimal128 exported as FixedSizeBinary(16) because no Arrow decimal precision/scale context was supplied",
                );
                Ok(DataType::FixedSizeBinary(16))
            }
        },
        CoveLogicalType::Uuid => {
            if !options.emit_uuid_extension_metadata {
                report.push(
                    None,
                    logical,
                    ArrowFidelitySeverity::Informational,
                    "Uuid exported as FixedSizeBinary(16) without Arrow extension metadata",
                );
            }
            Ok(DataType::FixedSizeBinary(16))
        }
        CoveLogicalType::Json => {
            if !options.emit_json_extension_metadata {
                let storage = if options.varbytes_policy == ArrowVarBytesExportPolicy::View {
                    "Utf8View"
                } else {
                    "Utf8"
                };
                report.push(
                    None,
                    logical,
                    ArrowFidelitySeverity::Lossy,
                    format!("Json exported as {storage} without Arrow extension metadata"),
                );
            }
            if options.varbytes_policy == ArrowVarBytesExportPolicy::View {
                Ok(DataType::Utf8View)
            } else {
                Ok(DataType::Utf8)
            }
        }
        CoveLogicalType::Utf8 if options.varbytes_policy == ArrowVarBytesExportPolicy::View => {
            Ok(DataType::Utf8View)
        }
        CoveLogicalType::Binary if options.varbytes_policy == ArrowVarBytesExportPolicy::View => {
            Ok(DataType::BinaryView)
        }
        other => arrow_data_type(other),
    }
}

fn collect_bool(values: &[CoveArrayValue<'_>]) -> Result<Vec<Option<bool>>, CoveError> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(match value {
            CoveArrayValue::Null => None,
            CoveArrayValue::Boolean(value) | CoveArrayValue::ValidityBit(value) => Some(*value),
            CoveArrayValue::Bytes(bytes) if bytes.len() == 1 => match bytes[0] {
                0 => Some(false),
                1 => Some(true),
                _ => return Err(CoveError::PageCorrupt),
            },
            other => return Err(unexpected_value("Boolean", other)),
        });
    }
    Ok(out)
}

fn collect_i64<T, F>(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
    cast: F,
) -> Result<Vec<Option<T>>, CoveError>
where
    F: Fn(i64) -> Result<T, CoveError>,
{
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(match value {
            CoveArrayValue::Null => None,
            _ => Some(cast(value_to_i64(logical, value)?)?),
        });
    }
    Ok(out)
}

fn collect_u64<T, F>(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
    cast: F,
) -> Result<Vec<Option<T>>, CoveError>
where
    F: Fn(u64) -> Result<T, CoveError>,
{
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(match value {
            CoveArrayValue::Null => None,
            _ => Some(cast(value_to_u64(logical, value)?)?),
        });
    }
    Ok(out)
}

fn collect_f32(values: &[CoveArrayValue<'_>]) -> Result<Vec<Option<f32>>, CoveError> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(match value {
            CoveArrayValue::Null => None,
            _ => Some(value_to_f32(value)?),
        });
    }
    Ok(out)
}

fn collect_f64(values: &[CoveArrayValue<'_>]) -> Result<Vec<Option<f64>>, CoveError> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(match value {
            CoveArrayValue::Null => None,
            _ => Some(value_to_f64(value)?),
        });
    }
    Ok(out)
}

fn collect_utf8(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
) -> Result<arrow_array::StringArray, CoveError> {
    let mut builder = StringBuilder::new();
    for value in values {
        match value {
            CoveArrayValue::Null => builder.append_null(),
            _ => {
                let bytes = value_to_bytes(logical, value)?;
                let text = std::str::from_utf8(bytes.as_ref()).map_err(|_| {
                    CoveError::BadSection("Arrow Utf8 export requires valid UTF-8".into())
                })?;
                builder.append_value(text);
            }
        }
    }
    Ok(builder.finish())
}

fn collect_binary(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
) -> Result<BinaryArray, CoveError> {
    let mut builder = BinaryBuilder::new();
    for value in values {
        match value {
            CoveArrayValue::Null => builder.append_null(),
            _ => {
                let bytes = value_to_bytes(logical, value)?;
                builder.append_value(bytes.as_ref());
            }
        }
    }
    Ok(builder.finish())
}

fn collect_utf8_view(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
) -> Result<StringViewArray, CoveError> {
    let mut builder = StringViewBuilder::new();
    for value in values {
        match value {
            CoveArrayValue::Null => builder.append_null(),
            _ => {
                let bytes = value_to_bytes(logical, value)?;
                let text = std::str::from_utf8(bytes.as_ref()).map_err(|_| {
                    CoveError::BadSection("Arrow Utf8View export requires valid UTF-8".into())
                })?;
                builder.append_value(text);
            }
        }
    }
    Ok(builder.finish())
}

fn collect_binary_view(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
) -> Result<BinaryViewArray, CoveError> {
    let mut builder = BinaryViewBuilder::new();
    for value in values {
        match value {
            CoveArrayValue::Null => builder.append_null(),
            _ => {
                let bytes = value_to_bytes(logical, value)?;
                builder.append_value(bytes.as_ref());
            }
        }
    }
    Ok(builder.finish())
}

fn collect_fixed_size_binary(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
    size: i32,
) -> Result<FixedSizeBinaryArray, CoveError> {
    let mut out = Vec::with_capacity(values.len());
    let expected = usize::try_from(size).map_err(|_| CoveError::PageCorrupt)?;
    for value in values {
        match value {
            CoveArrayValue::Null => out.push(None),
            _ => {
                let bytes = value_to_bytes(logical, value)?;
                if bytes.len() != expected {
                    return Err(CoveError::PageCorrupt);
                }
                out.push(Some(bytes.into_owned()));
            }
        }
    }
    FixedSizeBinaryArray::try_from_sparse_iter_with_size(out.into_iter(), size)
        .map_err(|err| CoveError::BadSection(format!("Arrow FixedSizeBinary: {err}")))
}

fn collect_i128(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
) -> Result<Vec<Option<i128>>, CoveError> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(match value {
            CoveArrayValue::Null => None,
            _ => Some(value_to_i128(logical, value)?),
        });
    }
    Ok(out)
}

fn value_to_i64(logical: CoveLogicalType, value: &CoveArrayValue<'_>) -> Result<i64, CoveError> {
    match value {
        CoveArrayValue::Int64(value) => Ok(*value),
        CoveArrayValue::NumCode(value) => numcode_to_i64(logical, *value),
        CoveArrayValue::Varint(value) => i64::try_from(*value).map_err(|_| CoveError::PageCorrupt),
        CoveArrayValue::FileCode(value) => Ok(i64::from(*value)),
        CoveArrayValue::Bytes(bytes) => signed_from_bytes(logical, bytes),
        CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
            signed_from_bytes(logical, bytes)
        }
        other => Err(unexpected_value("signed integer", other)),
    }
}

fn numcode_to_i64(logical: CoveLogicalType, value: u64) -> Result<i64, CoveError> {
    match logical {
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::DateDays
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => Ok(i64::from_le_bytes(value.to_le_bytes())),
        _ => i64::try_from(value).map_err(|_| CoveError::PageCorrupt),
    }
}

fn value_to_i128(logical: CoveLogicalType, value: &CoveArrayValue<'_>) -> Result<i128, CoveError> {
    match logical {
        CoveLogicalType::Decimal64 => value_to_i64(logical, value).map(i128::from),
        CoveLogicalType::Decimal128 => {
            let bytes = plain_bytes(value)?;
            exact_bytes::<16>(bytes).map(i128::from_le_bytes)
        }
        _ => Err(unexpected_value("decimal", value)),
    }
}

fn value_to_u64(logical: CoveLogicalType, value: &CoveArrayValue<'_>) -> Result<u64, CoveError> {
    match value {
        CoveArrayValue::NumCode(value) | CoveArrayValue::Varint(value) => Ok(*value),
        CoveArrayValue::Int64(value) => u64::try_from(*value).map_err(|_| CoveError::PageCorrupt),
        CoveArrayValue::FileCode(value) => Ok(u64::from(*value)),
        CoveArrayValue::Bytes(bytes) => unsigned_from_bytes(logical, bytes),
        CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
            unsigned_from_bytes(logical, bytes)
        }
        other => Err(unexpected_value("unsigned integer", other)),
    }
}

fn value_to_f32(value: &CoveArrayValue<'_>) -> Result<f32, CoveError> {
    if let CoveArrayValue::NumCode(value) = value {
        let bits = u32::try_from(*value).map_err(|_| CoveError::PageCorrupt)?;
        return Ok(f32::from_bits(bits));
    }
    let bytes = plain_bytes(value)?;
    if bytes.len() != 4 {
        return Err(CoveError::PageCorrupt);
    }
    Ok(f32::from_bits(u32::from_le_bytes(exact_bytes(bytes)?)))
}

fn value_to_f64(value: &CoveArrayValue<'_>) -> Result<f64, CoveError> {
    if let CoveArrayValue::NumCode(value) = value {
        return Ok(f64::from_bits(*value));
    }
    let bytes = plain_bytes(value)?;
    if bytes.len() != 8 {
        return Err(CoveError::PageCorrupt);
    }
    Ok(f64::from_bits(u64::from_le_bytes(exact_bytes(bytes)?)))
}

fn value_to_bytes<'a>(
    logical: CoveLogicalType,
    value: &'a CoveArrayValue<'a>,
) -> Result<Cow<'a, [u8]>, CoveError> {
    match value {
        CoveArrayValue::Bytes(bytes) => Ok(Cow::Borrowed(bytes)),
        CoveArrayValue::OwnedBytes(bytes) => Ok(Cow::Borrowed(bytes)),
        CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
            canonical_payload_bytes(logical, bytes)
        }
        CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => {
            Err(CoveError::RedactionPolicy)
        }
        other => Err(unexpected_value("bytes", other)),
    }
}

fn plain_bytes<'a>(value: &'a CoveArrayValue<'a>) -> Result<&'a [u8], CoveError> {
    match value {
        CoveArrayValue::Bytes(bytes) => Ok(bytes),
        CoveArrayValue::OwnedBytes(bytes) => Ok(bytes),
        CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => Ok(bytes),
        CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => {
            Err(CoveError::RedactionPolicy)
        }
        other => Err(unexpected_value("plain bytes", other)),
    }
}

fn canonical_payload_bytes<'a>(
    logical: CoveLogicalType,
    bytes: &'a [u8],
) -> Result<Cow<'a, [u8]>, CoveError> {
    match logical {
        CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json => {
            let (len, consumed) = wire::decode_u64_leb128(bytes)?;
            let len = usize::try_from(len).map_err(|_| CoveError::ArithOverflow)?;
            let start = consumed;
            let end = start.checked_add(len).ok_or(CoveError::ArithOverflow)?;
            if end != bytes.len() {
                return Err(CoveError::PageCorrupt);
            }
            Ok(Cow::Borrowed(&bytes[start..end]))
        }
        _ => Ok(Cow::Borrowed(bytes)),
    }
}

fn signed_from_bytes(logical: CoveLogicalType, bytes: &[u8]) -> Result<i64, CoveError> {
    match logical {
        CoveLogicalType::Int8 => exact_bytes::<1>(bytes).map(|raw| i8::from_le_bytes(raw) as i64),
        CoveLogicalType::Int16 => exact_bytes::<2>(bytes).map(|raw| i16::from_le_bytes(raw) as i64),
        CoveLogicalType::Int32 | CoveLogicalType::DateDays => {
            exact_bytes::<4>(bytes).map(|raw| i32::from_le_bytes(raw) as i64)
        }
        CoveLogicalType::Int64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => exact_bytes::<8>(bytes).map(i64::from_le_bytes),
        _ => Err(CoveError::UnsupportedEncoding(format!(
            "signed Arrow export from {:?}",
            logical
        ))),
    }
}

fn unsigned_from_bytes(logical: CoveLogicalType, bytes: &[u8]) -> Result<u64, CoveError> {
    match logical {
        CoveLogicalType::UInt8 => exact_bytes::<1>(bytes).map(|raw| u8::from_le_bytes(raw) as u64),
        CoveLogicalType::UInt16 => {
            exact_bytes::<2>(bytes).map(|raw| u16::from_le_bytes(raw) as u64)
        }
        CoveLogicalType::UInt32 => {
            exact_bytes::<4>(bytes).map(|raw| u32::from_le_bytes(raw) as u64)
        }
        CoveLogicalType::UInt64 => exact_bytes::<8>(bytes).map(u64::from_le_bytes),
        _ => Err(CoveError::UnsupportedEncoding(format!(
            "unsigned Arrow export from {:?}",
            logical
        ))),
    }
}

fn exact_bytes<const N: usize>(bytes: &[u8]) -> Result<[u8; N], CoveError> {
    if bytes.len() != N {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn unexpected_value(expected: &str, value: &CoveArrayValue<'_>) -> CoveError {
    CoveError::UnsupportedEncoding(format!("cannot export {value:?} as Arrow {expected}"))
}

#[cfg(test)]
#[cfg(test)]
mod tests;
