use super::*;

pub(super) fn validate_file_dictionary_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    if bytes.len() < 4 {
        return Err(CoveError::BufferTooShort);
    }
    let index_len =
        usize::try_from(read_u32_le_checked(bytes, 0)?).map_err(|_| CoveError::ArithOverflow)?;
    let split = 4usize
        .checked_add(index_len)
        .ok_or(CoveError::ArithOverflow)?;
    if split > bytes.len() {
        return Err(CoveError::OffsetRange);
    }
    FileDictionary::parse(&bytes[4..split], &bytes[split..]).map(|_| ())
}

pub(super) fn validate_encoding_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CoveError::BadSection(format!("invalid encoding fixture json: {err}")))?;
    let encoding = value
        .get("encoding")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("encoding fixture missing encoding".into()))?;
    let payload = parse_fixture_byte_vector(value.get("payload"), "payload")?;
    let expected_values = parse_fixture_i64_vector(value.get("expect_values"), "expect_values")?;

    match encoding {
        "constant" => validate_encoding_payload::<Constant, _, _>(
            &payload,
            &expected_values,
            ConstantPayload::parse,
        ),
        "rle" => {
            validate_encoding_payload::<Rle, _, _>(&payload, &expected_values, RlePayload::parse)
        }
        "run_end" => validate_encoding_payload::<RunEnd, _, _>(
            &payload,
            &expected_values,
            RunEndPayload::parse,
        ),
        "plain_fixed" => validate_encoding_payload::<PlainFixed, _, _>(
            &payload,
            &expected_values,
            PlainFixedPayload::parse,
        ),
        "plain_varint" => validate_encoding_payload::<PlainVarint, _, _>(
            &payload,
            &expected_values,
            PlainVarintPayload::parse,
        ),
        "bit_packed" => validate_encoding_payload::<BitPacked, _, _>(
            &payload,
            &expected_values,
            BitPackedPayload::parse,
        ),
        "delta" => validate_encoding_payload::<Delta, _, _>(
            &payload,
            &expected_values,
            DeltaPayload::parse,
        ),
        "frame_of_reference" => validate_encoding_payload::<FrameOfReference, _, _>(
            &payload,
            &expected_values,
            ForPayload::parse,
        ),
        "local_codebook" => validate_encoding_payload::<LocalCodebook, _, _>(
            &payload,
            &expected_values,
            LocalCodebookPayload::parse,
        ),
        "patched_base" => validate_encoding_payload::<PatchedBase, _, _>(
            &payload,
            &expected_values,
            PatchedBasePayload::parse,
        ),
        "sparse" => validate_encoding_payload::<Sparse, _, _>(
            &payload,
            &expected_values,
            SparsePayload::parse,
        ),
        other => Err(CoveError::BadSection(format!(
            "unsupported encoding fixture kind {other}"
        ))),
    }
}

pub(super) fn validate_nested_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CoveError::BadSection(format!("invalid nested fixture json: {err}")))?;
    let layout = value
        .get("layout")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("nested fixture missing layout".into()))?;

    match layout {
        "list" => {
            let offsets = parse_fixture_u32_vector(value.get("offsets"), "offsets")?;
            let child_row_count = value
                .get("child_row_count")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection("nested list fixture missing child_row_count".into())
                })?;
            ListLayout { offsets }.validate_child_count(child_row_count)
        }
        "struct" => {
            let field_row_counts =
                parse_fixture_u64_vector(value.get("field_row_counts"), "field_row_counts")?;
            let parent_row_count = value
                .get("parent_row_count")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    CoveError::BadSection("nested struct fixture missing parent_row_count".into())
                })?;
            let parent_null_handling_declared = value
                .get("parent_null_handling_declared")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            StructLayout { field_row_counts }
                .validate_parent_row_count(parent_row_count, parent_null_handling_declared)
        }
        "map" => {
            let offsets = parse_fixture_u32_vector(value.get("offsets"), "offsets")?;
            let key_row_count = value
                .get("key_row_count")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection("nested map fixture missing key_row_count".into())
                })?;
            let value_row_count = value
                .get("value_row_count")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection("nested map fixture missing value_row_count".into())
                })?;
            let keys_are_scalar = value
                .get("keys_are_scalar")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let allow_duplicate_keys = value
                .get("allow_duplicate_keys")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let canonical_keys =
                parse_fixture_string_bytes(value.get("canonical_keys"), "canonical_keys")?;
            MapLayout {
                offsets,
                key_row_count,
                value_row_count,
                keys_are_scalar,
                allow_duplicate_keys,
                canonical_keys,
            }
            .validate()
        }
        other => Err(CoveError::BadSection(format!(
            "unsupported nested fixture layout {other}"
        ))),
    }
}

fn validate_encoding_payload<E, P, F>(
    payload: &[u8],
    expected_values: &[i64],
    parse: F,
) -> Result<(), CoveError>
where
    E: Encoding<Payload = P>,
    F: FnOnce(&[u8]) -> Result<P, CoveError>,
{
    let payload = parse(payload)?;
    assert_parity::<E>(&payload)?;
    let actual = E::canonical_decode(&payload)?;
    if actual != expected_values {
        return Err(CoveError::BadSection(format!(
            "encoding fixture mismatch: expected {:?}, got {:?}",
            expected_values, actual
        )));
    }
    Ok(())
}

pub(super) fn validate_encoded_array_decode_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|err| {
        CoveError::BadSection(format!("invalid encoded_array fixture json: {err}"))
    })?;
    let fixture = fixture_encoded_array(&value)?;
    let array = fixture.as_array();
    let expected = value
        .get("expect")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("encoded_array fixture missing expect".into()))?;
    if expected.len() as u64 != array.row_count {
        return Err(CoveError::BadSection(
            "encoded_array fixture expect length must match row_count".into(),
        ));
    }

    let actual = array
        .decode_all_rows()?
        .into_iter()
        .map(|value| array_value_to_json(array.logical, value))
        .collect::<Result<Vec<_>, _>>()?;
    if actual != *expected {
        return Err(CoveError::BadSection(format!(
            "encoded_array fixture mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

pub(super) fn validate_arrow_export_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|err| {
        CoveError::BadSection(format!("invalid arrow export fixture json: {err}"))
    })?;
    let fixture = fixture_encoded_array(&value)?;
    let array = fixture.as_array();
    let arrow = encoded_array_to_arrow(&array)?;
    let expected_type = value
        .get("expect_type")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("arrow export fixture missing expect_type".into()))?;
    let actual_type = format!("{:?}", arrow.data_type());
    if actual_type != expected_type {
        return Err(CoveError::BadSection(format!(
            "arrow export data type mismatch: expected {expected_type}, got {actual_type}"
        )));
    }
    let expected = value
        .get("expect")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("arrow export fixture missing expect".into()))?;
    let actual = arrow_array_to_json(expected_type, arrow.as_ref())?;
    if actual != *expected {
        return Err(CoveError::BadSection(format!(
            "arrow export fixture mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

pub(super) fn validate_arrow_view_materialization_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CoveError::BadSection(format!("invalid arrow view fixture json: {err}")))?;
    let fixture = fixture_encoded_array(&value)?;
    let array = fixture.as_array();
    let rows = parse_fixture_u32_vector(value.get("selection"), "selection")?;
    let options = ArrowExportOptions {
        varbytes_policy: ArrowVarBytesExportPolicy::View,
        ..ArrowExportOptions::default()
    };
    let arrow = encoded_array_to_arrow_with_row_selection_options(
        &array,
        ArrowRowSelection::Rows(&rows),
        options,
    )?
    .value;
    let expected_type = value
        .get("expect_type")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("arrow view fixture missing expect_type".into()))?;
    let actual_type = format!("{:?}", arrow.data_type());
    if actual_type != expected_type {
        return Err(CoveError::BadSection(format!(
            "arrow view data type mismatch: expected {expected_type}, got {actual_type}"
        )));
    }
    let expected = value
        .get("expect")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("arrow view fixture missing expect".into()))?;
    let actual = arrow_array_to_json(expected_type, arrow.as_ref())?;
    if actual != *expected {
        return Err(CoveError::BadSection(format!(
            "arrow view fixture mismatch: expected {expected:?}, got {actual:?}"
        )));
    }
    let backing = arrow_view_data_buffers(arrow.as_ref());
    for value in parse_fixture_string_vector(
        value.get("expect_absent_in_buffers"),
        "expect_absent_in_buffers",
    )? {
        let needle = value.as_bytes();
        if !needle.is_empty() && backing.windows(needle.len()).any(|window| window == needle) {
            return Err(CoveError::BadSection(format!(
                "arrow view fixture found excluded bytes in backing buffer: {value}"
            )));
        }
    }
    Ok(())
}

struct EncodedArrayFixture {
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    encoding: CoveEncodingKind,
    row_count: u64,
    payload: Vec<u8>,
}

impl EncodedArrayFixture {
    fn as_array(&self) -> EncodedArray<'_> {
        EncodedArray::new(
            self.logical,
            self.physical,
            self.row_count,
            self.encoding,
            None,
            &self.payload,
            None,
        )
    }
}

fn fixture_encoded_array(value: &Value) -> Result<EncodedArrayFixture, CoveError> {
    let logical = parse_logical_type(
        value
            .get("logical")
            .and_then(Value::as_str)
            .ok_or_else(|| CoveError::BadSection("array fixture missing logical".into()))?,
    )?;
    let physical = parse_physical_kind(
        value
            .get("physical")
            .and_then(Value::as_str)
            .ok_or_else(|| CoveError::BadSection("array fixture missing physical".into()))?,
    )?;
    let encoding = parse_encoding_kind(
        value
            .get("encoding")
            .and_then(Value::as_str)
            .ok_or_else(|| CoveError::BadSection("array fixture missing encoding".into()))?,
    )?;
    let row_count = value
        .get("row_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| CoveError::BadSection("array fixture missing row_count".into()))?;
    let payload = value
        .get("payload")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("array fixture missing payload".into()))?;
    let payload = payload
        .iter()
        .map(|item| {
            item.as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| CoveError::BadSection("array fixture payload must be bytes".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EncodedArrayFixture {
        logical,
        physical,
        encoding,
        row_count,
        payload,
    })
}

fn array_value_to_json(
    logical: CoveLogicalType,
    value: CoveArrayValue<'_>,
) -> Result<Value, CoveError> {
    match value {
        CoveArrayValue::Null => Ok(Value::Null),
        CoveArrayValue::ValidityBit(value) | CoveArrayValue::Boolean(value) => Ok(json!(value)),
        CoveArrayValue::Int64(value) => Ok(json!(value)),
        CoveArrayValue::NumCode(value) | CoveArrayValue::Varint(value) => Ok(json!(value)),
        CoveArrayValue::FileCode(value) => Ok(json!(value)),
        CoveArrayValue::Bytes(bytes) => bytes_value_to_json(logical, bytes),
        CoveArrayValue::OwnedBytes(bytes) => bytes_value_to_json(logical, &bytes),
        CoveArrayValue::DictValue(_) => Err(CoveError::BadSection(
            "array decode conformance fixtures do not use dictionaries".into(),
        )),
        _ => Err(CoveError::BadSection(
            "array decode fixture produced a future value kind".into(),
        )),
    }
}

fn bytes_value_to_json(logical: CoveLogicalType, bytes: &[u8]) -> Result<Value, CoveError> {
    match logical {
        CoveLogicalType::Utf8 | CoveLogicalType::Json => Ok(json!(std::str::from_utf8(bytes)
            .map_err(|err| { CoveError::BadSection(format!("invalid UTF-8 value: {err}")) })?)),
        CoveLogicalType::Binary | CoveLogicalType::Uuid => Ok(json!(hex_encode(bytes))),
        _ => Ok(json!(hex_encode(bytes))),
    }
}

fn arrow_array_to_json(expected_type: &str, array: &dyn Array) -> Result<Vec<Value>, CoveError> {
    match expected_type {
        "Boolean" => {
            let values = downcast_arrow_array::<BooleanArray>(array, expected_type)?;
            Ok((0..values.len())
                .map(|row| {
                    if values.is_null(row) {
                        Value::Null
                    } else {
                        json!(values.value(row))
                    }
                })
                .collect())
        }
        "Int32" => {
            let values = downcast_arrow_array::<Int32Array>(array, expected_type)?;
            Ok((0..values.len())
                .map(|row| {
                    if values.is_null(row) {
                        Value::Null
                    } else {
                        json!(values.value(row))
                    }
                })
                .collect())
        }
        "UInt64" => {
            let values = downcast_arrow_array::<UInt64Array>(array, expected_type)?;
            Ok((0..values.len())
                .map(|row| {
                    if values.is_null(row) {
                        Value::Null
                    } else {
                        json!(values.value(row))
                    }
                })
                .collect())
        }
        "Utf8" => {
            let values = downcast_arrow_array::<StringArray>(array, expected_type)?;
            Ok((0..values.len())
                .map(|row| {
                    if values.is_null(row) {
                        Value::Null
                    } else {
                        json!(values.value(row))
                    }
                })
                .collect())
        }
        "Utf8View" => {
            let values = downcast_arrow_array::<StringViewArray>(array, expected_type)?;
            Ok((0..values.len())
                .map(|row| {
                    if values.is_null(row) {
                        Value::Null
                    } else {
                        json!(values.value(row))
                    }
                })
                .collect())
        }
        "Binary" => {
            let values = downcast_arrow_array::<BinaryArray>(array, expected_type)?;
            Ok((0..values.len())
                .map(|row| {
                    if values.is_null(row) {
                        Value::Null
                    } else {
                        json!(hex_encode(values.value(row)))
                    }
                })
                .collect())
        }
        "BinaryView" => {
            let values = downcast_arrow_array::<BinaryViewArray>(array, expected_type)?;
            Ok((0..values.len())
                .map(|row| {
                    if values.is_null(row) {
                        Value::Null
                    } else {
                        json!(hex_encode(values.value(row)))
                    }
                })
                .collect())
        }
        other => Err(CoveError::BadSection(format!(
            "unsupported arrow export fixture type {other}"
        ))),
    }
}

fn arrow_view_data_buffers(array: &dyn Array) -> Vec<u8> {
    array
        .to_data()
        .buffers()
        .iter()
        .skip(1)
        .flat_map(|buffer| buffer.as_slice().iter().copied())
        .collect()
}

fn downcast_arrow_array<'a, T: 'static>(
    array: &'a dyn Array,
    expected_type: &str,
) -> Result<&'a T, CoveError> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        CoveError::BadSection(format!(
            "arrow export fixture expected {expected_type} array"
        ))
    })
}

fn parse_logical_type(value: &str) -> Result<CoveLogicalType, CoveError> {
    match value {
        "Bool" => Ok(CoveLogicalType::Bool),
        "Int32" => Ok(CoveLogicalType::Int32),
        "Int64" => Ok(CoveLogicalType::Int64),
        "UInt64" => Ok(CoveLogicalType::UInt64),
        "Utf8" => Ok(CoveLogicalType::Utf8),
        "Binary" => Ok(CoveLogicalType::Binary),
        other => Err(CoveError::BadSection(format!(
            "unsupported array fixture logical type {other}"
        ))),
    }
}

fn parse_physical_kind(value: &str) -> Result<CovePhysicalKind, CoveError> {
    match value {
        "Boolean" => Ok(CovePhysicalKind::Boolean),
        "FixedBytes" => Ok(CovePhysicalKind::FixedBytes),
        "NumCode" => Ok(CovePhysicalKind::NumCode),
        "VarBytes" => Ok(CovePhysicalKind::VarBytes),
        other => Err(CoveError::BadSection(format!(
            "unsupported array fixture physical kind {other}"
        ))),
    }
}

fn parse_encoding_kind(value: &str) -> Result<CoveEncodingKind, CoveError> {
    match value {
        "PlainFixed" => Ok(CoveEncodingKind::PlainFixed),
        "NumCode" => Ok(CoveEncodingKind::NumCode),
        "VarBytes" => Ok(CoveEncodingKind::VarBytes),
        "Rle" => Ok(CoveEncodingKind::Rle),
        "LocalCodebook" => Ok(CoveEncodingKind::LocalCodebook),
        other => Err(CoveError::BadSection(format!(
            "unsupported array fixture encoding kind {other}"
        ))),
    }
}

pub(super) fn validate_arrow_bitmap_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CoveError::BadSection(format!("invalid arrow fixture json: {err}")))?;
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("arrow fixture missing op".into()))?;
    let row_count = value
        .get("row_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| CoveError::BadSection("arrow fixture missing row_count".into()))?;
    let input = parse_fixture_byte_vector(value.get("input"), "input")?;
    let expected = parse_fixture_byte_vector(value.get("expect"), "expect")?;

    let actual = match op {
        "cove_to_arrow" => cove_null_to_arrow_validity(&input, row_count)?,
        "arrow_to_cove" => arrow_validity_to_cove_null(&input, row_count)?,
        other => {
            return Err(CoveError::BadSection(format!(
                "unsupported arrow fixture op {other}"
            )));
        }
    };

    if actual != expected {
        return Err(CoveError::BadSection(format!(
            "arrow fixture mismatch: expected {:?}, got {:?}",
            expected, actual
        )));
    }
    Ok(())
}

pub(super) fn validate_parquet_conversion_fixture(
    entry: &Entry,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let mut options = ParquetConversionOptions::default();
    if let Some(table_name) = entry.raw.get("table_name").and_then(Value::as_str) {
        options.table_name = table_name.to_string();
    }
    if let Some(namespace) = entry.raw.get("namespace").and_then(Value::as_str) {
        options.namespace = namespace.to_string();
    }
    if let Some(morsel_row_count) = entry.raw.get("morsel_row_count").and_then(Value::as_u64) {
        options.morsel_row_count = u32::try_from(morsel_row_count)
            .map_err(|_| CoveError::BadSection("invalid morsel_row_count".into()))?;
    }

    let result = convert_parquet_bytes(bytes, &options)?;
    let validation = reader::validate_bytes_with_options(
        &result.cove_bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )?;

    if let Some(expected_row_count) = entry.raw.get("expected_row_count").and_then(Value::as_u64) {
        if result.report.row_count != expected_row_count {
            return Err(CoveError::BadSection(format!(
                "parquet conversion row_count mismatch: expected {expected_row_count}, got {}",
                result.report.row_count
            )));
        }
    }

    let table_catalog_payload = section_payload_by_kind(
        &result.cove_bytes,
        &validation.validated,
        SectionKind::TableCatalog,
    )?;
    let table_catalog = TableCatalog::parse(table_catalog_payload.as_ref())?;
    let table = table_catalog
        .tables
        .first()
        .ok_or_else(|| CoveError::BadSection("converted parquet file is missing a table".into()))?;

    if table.name != options.table_name {
        return Err(CoveError::BadSection(format!(
            "parquet conversion table name mismatch: expected {}, got {}",
            options.table_name, table.name
        )));
    }
    if table.namespace != options.namespace {
        return Err(CoveError::BadSection(format!(
            "parquet conversion namespace mismatch: expected {}, got {}",
            options.namespace, table.namespace
        )));
    }

    let segment_payload = section_payload_by_kind(
        &result.cove_bytes,
        &validation.validated,
        SectionKind::TableSegmentData,
    )?;
    let segment = TableSegmentPayloadV1::parse(segment_payload.as_ref())?;

    let expected_columns = entry
        .raw
        .get("expected_columns")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("parquet conversion fixture missing expected_columns".into())
        })?;
    if expected_columns.len() != table.columns.len() {
        return Err(CoveError::BadSection(format!(
            "parquet conversion column_count mismatch: expected {}, got {}",
            expected_columns.len(),
            table.columns.len()
        )));
    }

    for (expected, column) in expected_columns.iter().zip(table.columns.iter()) {
        if let Some(expected_name) = expected.get("name").and_then(Value::as_str) {
            if column.name != expected_name {
                return Err(CoveError::BadSection(format!(
                    "parquet conversion column name mismatch: expected {expected_name}, got {}",
                    column.name
                )));
            }
        }
        if let Some(expected_logical) = expected.get("logical").and_then(Value::as_str) {
            if format!("{:?}", column.logical) != expected_logical {
                return Err(CoveError::BadSection(format!(
                    "parquet conversion logical mismatch for column {}: expected {expected_logical}, got {:?}",
                    column.name, column.logical
                )));
            }
        }
        if let Some(expected_physical) = expected.get("physical").and_then(Value::as_str) {
            if format!("{:?}", column.physical) != expected_physical {
                return Err(CoveError::BadSection(format!(
                    "parquet conversion physical mismatch for column {}: expected {expected_physical}, got {:?}",
                    column.name, column.physical
                )));
            }
        }
        let expected_values = expected
            .get("values")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "parquet conversion fixture missing values for column {}",
                    column.name
                ))
            })?;
        let actual_values =
            decode_segment_column_values(segment_payload.as_ref(), &segment, column)?;
        let actual_json = actual_values
            .into_iter()
            .map(|value| value.to_json_value())
            .collect::<Vec<_>>();
        if actual_json != *expected_values {
            return Err(CoveError::BadSection(format!(
                "parquet conversion values mismatch for column {}: expected {:?}, got {:?}",
                column.name, expected_values, actual_json
            )));
        }
    }

    Ok(())
}

fn section_payload_by_kind<'a>(
    data: &'a [u8],
    validated: &'a reader::ValidatedCoveFile,
    kind: SectionKind,
) -> Result<Cow<'a, [u8]>, CoveError> {
    let entry = validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == kind as u16)
        .ok_or_else(|| CoveError::BadSection(format!("missing section {kind:?}")))?;
    section_payload(data, entry)
}

fn decode_segment_column_values(
    segment_bytes: &[u8],
    segment: &TableSegmentPayloadV1,
    column: &cove_core::table::ColumnEntry,
) -> Result<Vec<ParquetScalarValue>, CoveError> {
    let column_directory = segment
        .columns
        .iter()
        .find(|entry| entry.column_id == column.column_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "missing segment column directory for column {}",
                column.column_id
            ))
        })?;
    let page_index = ColumnPageIndex::parse(
        &segment_bytes[column_directory.page_index_offset as usize
            ..(column_directory.page_index_offset + column_directory.page_index_length) as usize],
    )?;
    let mut out = Vec::new();
    for page in &page_index.entries {
        let page_wire = &segment_bytes
            [page.page_offset as usize..(page.page_offset + page.page_length) as usize];
        let payload = column_page_payload(page_wire, page)?;
        let page_payload = ColumnPagePayloadV1::parse(payload.as_ref())?;
        let root = page_payload.root_node()?;
        if root.logical_type != column.logical
            || root.physical_kind != column.physical
            || root.logical_len != page.row_count
        {
            return Err(CoveError::PageCorrupt);
        }
        let values = page_payload
            .buffer_bytes(PageBufferKind::Values)?
            .unwrap_or(&[]);
        let decode_payload = match page_payload.buffer_bytes(PageBufferKind::NullBitmap)? {
            Some(nulls) => {
                let mut bytes = Vec::with_capacity(nulls.len() + values.len());
                bytes.extend_from_slice(nulls);
                bytes.extend_from_slice(values);
                Cow::Owned(bytes)
            }
            None => Cow::Borrowed(values),
        };
        out.extend(decode_materialized_page_values_with_nulls(
            column,
            page.row_count,
            page.null_count,
            decode_payload.as_ref(),
        )?);
    }
    Ok(out)
}

pub(super) fn parse_fixture_byte_vector(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<u8>, CoveError> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field} byte array")))?;
    items
        .iter()
        .map(|item| {
            item.as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "fixture field {field} must contain only byte values"
                    ))
                })
        })
        .collect()
}

pub(super) fn parse_optional_fixture_byte_vector(
    value: &Value,
    field: &str,
) -> Result<Option<Vec<u8>>, CoveError> {
    match value.get(field) {
        Some(Value::Null) | None => Ok(None),
        Some(bytes) => parse_fixture_byte_vector(Some(bytes), field).map(Some),
    }
}

pub(super) fn parse_fixture_i64_vector(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<i64>, CoveError> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field} i64 array")))?;
    items
        .iter()
        .map(|item| {
            item.as_i64().ok_or_else(|| {
                CoveError::BadSection(format!(
                    "fixture field {field} must contain only i64 values"
                ))
            })
        })
        .collect()
}

pub(super) fn parse_fixture_u32_vector(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<u32>, CoveError> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field} u32 array")))?;
    items
        .iter()
        .map(|item| {
            item.as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "fixture field {field} must contain only u32 values"
                    ))
                })
        })
        .collect()
}

pub(super) fn parse_fixture_u64_vector(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<u64>, CoveError> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field} u64 array")))?;
    items
        .iter()
        .map(|item| {
            item.as_u64().ok_or_else(|| {
                CoveError::BadSection(format!(
                    "fixture field {field} must contain only u64 values"
                ))
            })
        })
        .collect()
}

pub(super) fn parse_fixture_string_bytes(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<Vec<u8>>, CoveError> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field} string array")))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(|item| item.as_bytes().to_vec())
                .ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "fixture field {field} must contain only string values"
                    ))
                })
        })
        .collect()
}
