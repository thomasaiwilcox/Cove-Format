use super::*;

pub(super) fn try_filecode_dictionary_array(
    array: &EncodedArray<'_>,
    options: ArrowExportOptions,
) -> Result<Option<ArrayRef>, CoveError> {
    if array.encoding != crate::constants::CoveEncodingKind::FileCode {
        return Ok(None);
    }
    let Some(dictionary) = array.dictionary else {
        return Ok(None);
    };
    if file_dictionary_entries_compatible_with_logical(array.logical, dictionary)? {
        let values = file_dictionary_values_to_arrow(array.logical, dictionary, options)?;
        encoded_filecode_array_to_arrow_dictionary_with_values(
            array,
            ArrowRowSelection::All,
            values,
        )
        .map(Some)
    } else {
        encoded_filecode_array_to_arrow_dictionary_remapped(array, ArrowRowSelection::All, options)
            .map(Some)
    }
}

pub(super) fn try_filecode_dictionary_array_for_selection(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    options: ArrowExportOptions,
) -> Result<Option<ArrayRef>, CoveError> {
    if array.encoding != crate::constants::CoveEncodingKind::FileCode {
        return Ok(None);
    }
    let Some(dictionary) = array.dictionary else {
        return Ok(None);
    };
    if file_dictionary_entries_compatible_with_logical(array.logical, dictionary)? {
        let values = file_dictionary_values_to_arrow(array.logical, dictionary, options)?;
        encoded_filecode_array_to_arrow_dictionary_with_values(array, selection, values).map(Some)
    } else {
        encoded_filecode_array_to_arrow_dictionary_remapped(array, selection, options).map(Some)
    }
}

/// Return the effective Arrow export options for FileCode dictionary values.
///
/// DataFusion and other engines have well-tested support for dictionary values
/// stored as standard Utf8/Binary arrays. Keep Arrow view arrays out of the
/// FileCode dictionary values path until each downstream operator family is
/// validated against Dictionary<UInt32, Utf8View/BinaryView>.
pub fn filecode_dictionary_value_export_options(
    mut options: ArrowExportOptions,
) -> ArrowExportOptions {
    options.dictionary_policy = ArrowDictionaryPolicy::DecodeValues;
    options.varbytes_policy = ArrowVarBytesExportPolicy::Standard;
    options
}

/// Return true when the whole file dictionary can be used as the Arrow
/// dictionary values array for this logical type without key remapping.
pub fn file_dictionary_entries_compatible_with_logical(
    logical: CoveLogicalType,
    dictionary: &crate::dictionary::FileDictionary,
) -> Result<bool, CoveError> {
    for entry in &dictionary.entries {
        if !file_dictionary_entry_compatible_with_logical(logical, entry)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Build an Arrow dictionary array for a FileCode page using prebuilt
/// dictionary values.
///
/// This lets query engines cache the dictionary values for an immutable file
/// and materialise only the per-page key buffer on repeated scans.
pub fn encoded_filecode_array_to_arrow_dictionary_with_values(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    values: ArrayRef,
) -> Result<ArrayRef, CoveError> {
    if array.encoding != crate::constants::CoveEncodingKind::FileCode {
        return Err(CoveError::UnsupportedEncoding(
            "Arrow dictionary export requires FileCode encoding".into(),
        ));
    }
    let keys = filecode_key_array(array, selection)?;
    DictionaryArray::<UInt32Type>::try_new(keys, values)
        .map(|array| Arc::new(array) as ArrayRef)
        .map_err(|err| CoveError::BadSection(format!("Arrow DictionaryArray: {err}")))
}

/// Build an Arrow dictionary array for a FileCode page by remapping only the
/// selected, non-null FileCodes into a dense Arrow dictionary.
///
/// This is the safe path for files whose dictionary contains entries for
/// multiple logical domains. It decodes only entries referenced by this column
/// and selection, so unrelated binary/utf8/redacted entries cannot affect the
/// projected column.
pub fn encoded_filecode_array_to_arrow_dictionary_remapped(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
    options: ArrowExportOptions,
) -> Result<ArrayRef, CoveError> {
    if array.encoding != crate::constants::CoveEncodingKind::FileCode {
        return Err(CoveError::UnsupportedEncoding(
            "Arrow dictionary remap requires FileCode encoding".into(),
        ));
    }
    let Some(dictionary) = array.dictionary else {
        return Err(CoveError::UnsupportedEncoding(
            "Arrow dictionary remap requires a file dictionary".into(),
        ));
    };
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let data = fixed_width_payload_prefix(array.data, row_count, 4)?;
    let has_nulls = array_has_nulls(array)?;
    let mut dense_by_code = BTreeMap::<u32, u32>::new();
    selection.for_each_row(array.row_count, |row| {
        let is_null = has_nulls && array.is_null(row as u64)?;
        if !is_null {
            let code = wire::read_u32_le_checked(
                data,
                row.checked_mul(4).ok_or(CoveError::ArithOverflow)?,
            )?;
            dense_by_code.entry(code).or_insert(0);
        }
        Ok(())
    })?;
    for (dense, value) in dense_by_code.values_mut().enumerate() {
        *value = u32::try_from(dense).map_err(|_| CoveError::ArithOverflow)?;
    }
    let file_codes = dense_by_code.keys().copied().collect::<Vec<_>>();
    let values =
        file_dictionary_values_to_arrow_for_codes(array.logical, dictionary, &file_codes, options)?;

    let selected_len = selection.selected_len(array.row_count)?;
    let mut keys = Vec::<u32>::with_capacity(selected_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    selection.for_each_row(array.row_count, |row| {
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if is_null {
            keys.push(0);
        } else {
            let code = wire::read_u32_le_checked(
                data,
                row.checked_mul(4).ok_or(CoveError::ArithOverflow)?,
            )?;
            let dense = dense_by_code
                .get(&code)
                .copied()
                .ok_or(CoveError::BadFileCode)?;
            keys.push(dense);
        }
        Ok(())
    })?;

    let keys = UInt32Array::new(
        ScalarBuffer::from(keys),
        validity_builder.and_then(ArrowValidityBuilder::finish),
    );
    DictionaryArray::<UInt32Type>::try_new(keys, values)
        .map(|array| Arc::new(array) as ArrayRef)
        .map_err(|err| CoveError::BadSection(format!("Arrow DictionaryArray: {err}")))
}

fn filecode_key_array(
    array: &EncodedArray<'_>,
    selection: ArrowRowSelection<'_>,
) -> Result<UInt32Array, CoveError> {
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    let data = fixed_width_payload_prefix(array.data, row_count, 4)?;
    let has_nulls = array_has_nulls(array)?;
    let selected_len = selection.selected_len(array.row_count)?;
    let mut keys = Vec::<u32>::with_capacity(selected_len);
    let mut validity_builder = has_nulls
        .then(|| ArrowValidityBuilder::new(selected_len))
        .transpose()?;
    selection.for_each_row(array.row_count, |row| {
        let is_null = has_nulls && array.is_null(row as u64)?;
        if let Some(builder) = &mut validity_builder {
            builder.append(!is_null);
        }
        if is_null {
            keys.push(0);
        } else {
            keys.push(wire::read_u32_le_checked(
                data,
                row.checked_mul(4).ok_or(CoveError::ArithOverflow)?,
            )?);
        }
        Ok(())
    })?;
    Ok(UInt32Array::new(
        ScalarBuffer::from(keys),
        validity_builder.and_then(ArrowValidityBuilder::finish),
    ))
}

/// Decode COVE file dictionary entries into the Arrow dictionary values array.
pub fn file_dictionary_values_to_arrow(
    logical: CoveLogicalType,
    dictionary: &crate::dictionary::FileDictionary,
    options: ArrowExportOptions,
) -> Result<ArrayRef, CoveError> {
    if !file_dictionary_entries_compatible_with_logical(logical, dictionary)? {
        return Err(CoveError::UnsupportedEncoding(format!(
            "file dictionary contains entries incompatible with {logical:?}"
        )));
    }
    let codes = (0..dictionary.len()).collect::<Vec<_>>();
    file_dictionary_values_to_arrow_for_codes(logical, dictionary, &codes, options)
}

/// Decode selected COVE file dictionary entries into an Arrow dictionary values
/// array. `codes` are file-local FileCodes in the desired Arrow values order.
pub fn file_dictionary_values_to_arrow_for_codes(
    logical: CoveLogicalType,
    dictionary: &crate::dictionary::FileDictionary,
    codes: &[u32],
    options: ArrowExportOptions,
) -> Result<ArrayRef, CoveError> {
    let mut values = Vec::with_capacity(codes.len());
    for code in codes {
        let entry = dictionary.get_entry(*code)?;
        if !file_dictionary_entry_tag_compatible_with_logical(logical, entry)? {
            return Err(CoveError::UnsupportedEncoding(format!(
                "FileCode({code}) is not compatible with {logical:?}"
            )));
        }
        values.push(CoveArrayValue::DictValue(dictionary.decode_value(*code)?));
    }
    dictionary_values_to_arrow_array(logical, &values, options)
}

fn dictionary_values_to_arrow_array(
    logical: CoveLogicalType,
    values: &[CoveArrayValue<'_>],
    options: ArrowExportOptions,
) -> Result<ArrayRef, CoveError> {
    let options = filecode_dictionary_value_export_options(options);
    let mut report = ArrowExportReport::default();
    match arrow_data_type_with_report(logical, &options, &mut report)? {
        DataType::Boolean => Ok(Arc::new(BooleanArray::from(collect_bool(values)?))),
        DataType::Int8 => Ok(Arc::new(Int8Array::from(collect_i64(
            logical,
            values,
            |v| i8::try_from(v).map_err(|_| CoveError::PageCorrupt),
        )?))),
        DataType::Int16 => Ok(Arc::new(Int16Array::from(collect_i64(
            logical,
            values,
            |v| i16::try_from(v).map_err(|_| CoveError::PageCorrupt),
        )?))),
        DataType::Int32 => Ok(Arc::new(Int32Array::from(collect_i64(
            logical,
            values,
            |v| i32::try_from(v).map_err(|_| CoveError::PageCorrupt),
        )?))),
        DataType::Date32 => Ok(Arc::new(Date32Array::from(collect_i64(
            logical,
            values,
            |v| i32::try_from(v).map_err(|_| CoveError::PageCorrupt),
        )?))),
        DataType::Int64 => Ok(Arc::new(Int64Array::from(collect_i64(
            logical, values, Ok,
        )?))),
        DataType::UInt8 => Ok(Arc::new(UInt8Array::from(collect_u64(
            logical,
            values,
            |v| u8::try_from(v).map_err(|_| CoveError::PageCorrupt),
        )?))),
        DataType::UInt16 => Ok(Arc::new(UInt16Array::from(collect_u64(
            logical,
            values,
            |v| u16::try_from(v).map_err(|_| CoveError::PageCorrupt),
        )?))),
        DataType::UInt32 => Ok(Arc::new(UInt32Array::from(collect_u64(
            logical,
            values,
            |v| u32::try_from(v).map_err(|_| CoveError::PageCorrupt),
        )?))),
        DataType::UInt64 => Ok(Arc::new(UInt64Array::from(collect_u64(
            logical, values, Ok,
        )?))),
        DataType::Float32 => Ok(Arc::new(Float32Array::from(collect_f32(values)?))),
        DataType::Float64 => Ok(Arc::new(Float64Array::from(collect_f64(values)?))),
        DataType::Timestamp(TimeUnit::Microsecond, None) => Ok(Arc::new(
            TimestampMicrosecondArray::from(collect_i64(logical, values, Ok)?),
        )),
        DataType::Timestamp(TimeUnit::Nanosecond, None) => Ok(Arc::new(
            TimestampNanosecondArray::from(collect_i64(logical, values, Ok)?),
        )),
        DataType::Utf8 => Ok(Arc::new(collect_utf8(logical, values)?)),
        DataType::Utf8View => Ok(Arc::new(collect_utf8_view(logical, values)?)),
        DataType::Binary => Ok(Arc::new(collect_binary(logical, values)?)),
        DataType::BinaryView => Ok(Arc::new(collect_binary_view(logical, values)?)),
        DataType::FixedSizeBinary(size) => {
            Ok(Arc::new(collect_fixed_size_binary(logical, values, size)?))
        }
        other => Err(CoveError::UnsupportedEncoding(format!(
            "Arrow dictionary export for {other:?}"
        ))),
    }
}

fn file_dictionary_entry_compatible_with_logical(
    logical: CoveLogicalType,
    entry: &crate::dictionary::FileDictionaryIndexEntryV1,
) -> Result<bool, CoveError> {
    let Some(storage_class) = StorageClass::from_u8(entry.storage_class) else {
        return Err(CoveError::BadSection(format!(
            "unknown dictionary storage_class {}",
            entry.storage_class
        )));
    };
    if storage_class == StorageClass::Redacted {
        return Ok(false);
    }
    file_dictionary_entry_tag_compatible_with_logical(logical, entry)
}

fn file_dictionary_entry_tag_compatible_with_logical(
    logical: CoveLogicalType,
    entry: &crate::dictionary::FileDictionaryIndexEntryV1,
) -> Result<bool, CoveError> {
    let Some(tag) = ValueTag::from_u16(entry.value_tag) else {
        return Err(CoveError::BadSection(format!(
            "unknown dictionary value_tag {}",
            entry.value_tag
        )));
    };
    Ok(match logical {
        CoveLogicalType::Null => tag == ValueTag::Null,
        CoveLogicalType::Bool => matches!(tag, ValueTag::BoolFalse | ValueTag::BoolTrue),
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => tag == ValueTag::Int64,
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => tag == ValueTag::UInt64,
        CoveLogicalType::Float32 => tag == ValueTag::Float32Bits,
        CoveLogicalType::Float64 => tag == ValueTag::Float64Bits,
        CoveLogicalType::Decimal64 => tag == ValueTag::Decimal64,
        CoveLogicalType::Decimal128 => tag == ValueTag::Decimal128,
        CoveLogicalType::DateDays => tag == ValueTag::DateDays,
        CoveLogicalType::TimestampMicros => tag == ValueTag::TimestampMicros,
        CoveLogicalType::TimestampNanos => tag == ValueTag::TimestampNanos,
        CoveLogicalType::Utf8 => tag == ValueTag::Utf8,
        CoveLogicalType::Binary => tag == ValueTag::Binary,
        CoveLogicalType::Uuid => tag == ValueTag::Uuid,
        CoveLogicalType::Json => tag == ValueTag::Json,
        CoveLogicalType::List => tag == ValueTag::List,
        CoveLogicalType::Struct => tag == ValueTag::Struct,
        CoveLogicalType::Map => tag == ValueTag::Map,
        _ => false,
    })
}
