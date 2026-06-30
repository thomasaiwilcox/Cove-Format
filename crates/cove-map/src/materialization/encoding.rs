use std::collections::{BTreeMap, BTreeSet};

use cove_core::{
    artifact::covemap::CovemapFile,
    canonical::{CanonicalField, CanonicalValue},
    checksum,
    constants::{CompressionCodec, CoveLogicalType, CovePhysicalKind},
    dictionary::{FileDictionary, FileDictionaryEncoding, FileDictionaryKey},
    nested_schema::NestedSchemaNodeV1,
    page::{ColumnPageIndexEntryV1, COLUMN_PAGE_INDEX_ENTRY_LEN},
    page_payload::ColumnPagePayloadV1,
    profile::cove_o::{
        temporal_row_trust_payload, CoveRecordRefV1, ObjectTypeEntryV1, PropertyEntryV1,
        RecordKind, TemporalRowEntryV1, TemporalSegmentData, TemporalSegmentHeaderV1,
        TemporalSegmentIndex, TemporalSegmentIndexEntryV1, TrustManifest, TrustManifestEntryV1,
        TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
    },
    segment::{TableColumnDirectoryEntryV1, TABLE_COLUMN_DIRECTORY_ENTRY_LEN},
    trust_chain,
};
use serde_json::Value;

use super::{MaterializedModel, NestedShapeByProperty, ObjectRow, TemporalSegmentBuild};
use crate::{
    project,
    sections::embedded_sections,
    support::{
        encoding_for_physical, fixed_bytes_for_property, hex_decode_16, json_bool, json_f64,
        json_i128, json_i64, json_numcode, json_string, json_u64, json_uuid, physical_for_logical,
        stable_u32, var_bytes_for_property,
    },
};

pub(crate) fn temporal_segment_payload(
    segment_id: u32,
    object_type: &ObjectTypeEntryV1,
    rows: &[ObjectRow],
    nested_shapes: &NestedShapeByProperty,
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<Vec<u8>, String> {
    let row_count = u32::try_from(rows.len()).map_err(|_| "too many COVE-O rows".to_string())?;
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes_len = rows
        .len()
        .checked_mul(TEMPORAL_ROW_ENTRY_LEN)
        .ok_or_else(|| "temporal row directory length overflow".to_string())?;
    let column_directory_offset = row_directory_offset
        .checked_add(row_bytes_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let column_count = u32::try_from(object_type.properties.len())
        .map_err(|_| "too many COVE-O property columns".to_string())?;
    let column_dir_len = object_type
        .properties
        .len()
        .checked_mul(TABLE_COLUMN_DIRECTORY_ENTRY_LEN)
        .ok_or_else(|| "temporal column directory length overflow".to_string())?;
    let page_index_offset = column_directory_offset
        .checked_add(column_dir_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let total_page_index_len = object_type
        .properties
        .len()
        .checked_mul(COLUMN_PAGE_INDEX_ENTRY_LEN)
        .ok_or_else(|| "temporal page index length overflow".to_string())?;
    let data_offset = page_index_offset
        .checked_add(total_page_index_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let header = TemporalSegmentHeaderV1 {
        segment_id,
        object_type_id: object_type.object_type_id,
        time_range_start_us: 0,
        time_range_end_us: 0,
        csn_min: 0,
        csn_max: rows.len().saturating_sub(1) as u64,
        row_count,
        morsel_count: if row_count == 0 { 0 } else { 1 },
        morsel_row_count: if row_count == 0 { 0 } else { row_count },
        column_count,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    let prev_refs = temporal_prev_refs(segment_id, rows);
    for (index, row) in rows.iter().enumerate() {
        out.extend_from_slice(
            &TemporalRowEntryV1 {
                timestamp_us: 0,
                csn: index as u64,
                branch_key: 0,
                goid: row.goid,
                record_id: row.record_id,
                record_kind: row.record_kind,
                prev_ref: prev_refs[index],
            }
            .serialize(),
        );
    }
    let mut column_directory = Vec::new();
    let mut page_index_bytes = Vec::new();
    let mut page_payload_bytes = Vec::new();
    let mut next_page_index_offset = page_index_offset;
    let mut next_data_offset = data_offset;
    for property in &object_type.properties {
        let column_page_index_offset = next_page_index_offset;
        let column_data_offset = next_data_offset;
        let page_payload = build_property_page_payload(
            object_type.object_type_id,
            property,
            rows,
            nested_shapes,
            dictionary,
        )?;
        let page_length = page_payload.len() as u64;
        let page_checksum = checksum::crc32c(&page_payload);
        let null_count = rows
            .iter()
            .filter(|row| {
                row.properties
                    .get(&property.property_id)
                    .is_none_or(|value| value.value.is_null())
            })
            .count() as u32;
        let page = ColumnPageIndexEntryV1 {
            column_id: property.property_id,
            morsel_id: 0,
            row_count,
            non_null_count: row_count.saturating_sub(null_count),
            null_count,
            encoding_root: encoding_for_physical(property.physical_kind) as u32,
            page_offset: next_data_offset,
            page_length,
            uncompressed_length: page_length,
            stats_ref: 0,
            flags: CompressionCodec::None as u32,
            checksum: page_checksum,
        };
        page_index_bytes.extend_from_slice(&page.serialize());
        page_payload_bytes.extend_from_slice(&page_payload);
        next_page_index_offset = next_page_index_offset
            .checked_add(COLUMN_PAGE_INDEX_ENTRY_LEN as u64)
            .ok_or_else(|| "temporal page index offset overflow".to_string())?;
        next_data_offset = next_data_offset
            .checked_add(page_length)
            .ok_or_else(|| "temporal data offset overflow".to_string())?;
        column_directory.push(TableColumnDirectoryEntryV1 {
            column_id: property.property_id,
            logical_type: property.logical_type,
            physical_kind: property.physical_kind,
            flags: 0,
            page_index_offset: column_page_index_offset,
            page_index_length: COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
            data_offset: column_data_offset,
            data_length: next_data_offset - column_data_offset,
            stats_ref: 0,
            domain_ref: 0,
            checksum: 0,
        });
    }
    for entry in &column_directory {
        out.extend_from_slice(&entry.serialize());
    }
    out.extend_from_slice(&page_index_bytes);
    out.extend_from_slice(&page_payload_bytes);
    Ok(out)
}

fn build_property_page_payload(
    object_type_id: u32,
    property: &PropertyEntryV1,
    rows: &[ObjectRow],
    nested_shapes: &NestedShapeByProperty,
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<Vec<u8>, String> {
    let row_count = u32::try_from(rows.len()).map_err(|_| "too many rows".to_string())?;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut values = Vec::new();
    let mut null_count = 0usize;
    for (row_index, row) in rows.iter().enumerate() {
        let value = row
            .properties
            .get(&property.property_id)
            .map(|property| &property.value)
            .unwrap_or(&Value::Null);
        if value.is_null() {
            null_count += 1;
            null_bitmap[row_index / 8] |= 1u8 << (row_index % 8);
        }
        append_property_value_bytes(
            property,
            value,
            nested_shapes.get(&(object_type_id, property.property_id)),
            dictionary,
            &mut values,
        )?;
    }
    ColumnPagePayloadV1::build_single_node(
        row_count,
        encoding_for_physical(property.physical_kind),
        property.logical_type,
        property.physical_kind,
        (null_count != 0).then_some(null_bitmap),
        values,
    )
    .map_err(|err| err.to_string())
}

pub(crate) fn nested_shapes_for_model(
    file: &CovemapFile,
    materialized: &MaterializedModel,
) -> Result<NestedShapeByProperty, String> {
    let mut out = NestedShapeByProperty::new();
    let object_types_by_name = materialized
        .object_types
        .iter()
        .map(|object_type| (object_type.type_name.as_str(), object_type))
        .collect::<BTreeMap<_, _>>();
    for section in embedded_sections(file)? {
        let cove_core::profile::cove_map::EmbeddedMapSection::ProjectionCatalog(catalog) = section
        else {
            continue;
        };
        for projection in catalog.projections {
            let output_table = projection
                .output_table
                .as_deref()
                .unwrap_or(&projection.projection_id);
            let Some(object_type) = object_types_by_name.get(output_table) else {
                continue;
            };
            let properties_by_name = object_type
                .properties
                .iter()
                .map(|property| (property.property_name.as_str(), property))
                .collect::<BTreeMap<_, _>>();
            for column in projection.columns {
                let Some(shape) = column.nested_shape.as_deref() else {
                    continue;
                };
                let Some(property) = properties_by_name.get(column.name.as_str()) else {
                    continue;
                };
                let shape_value: Value = serde_json::from_str(shape).map_err(|err| {
                    format!(
                        "projection column '{}' has invalid nested_shape JSON: {err}",
                        column.name
                    )
                })?;
                let mut node =
                    project::nested_schema_node_from_shape(&column.name, &shape_value, true)?;
                node.name = column.name.clone();
                node.logical = property.logical_type;
                node.physical = physical_for_logical(property.logical_type);
                out.insert((object_type.object_type_id, property.property_id), node);
            }
        }
    }
    Ok(out)
}

pub(crate) fn file_dictionary_for_model(
    materialized: &MaterializedModel,
    nested_shapes: &NestedShapeByProperty,
) -> Result<Option<FileDictionaryEncoding>, String> {
    let mut keys = BTreeSet::<FileDictionaryKey>::new();
    let properties_by_type = materialized
        .object_types
        .iter()
        .flat_map(|object_type| {
            object_type
                .properties
                .iter()
                .map(move |property| ((object_type.object_type_id, property.property_id), property))
        })
        .collect::<BTreeMap<_, _>>();
    for row in &materialized.rows {
        for (property_id, property_value) in &row.properties {
            let Some(property) = properties_by_type.get(&(row.object_type_id, *property_id)) else {
                continue;
            };
            if property.physical_kind != CovePhysicalKind::FileCode
                || property_value.value.is_null()
            {
                continue;
            }
            keys.insert(file_dictionary_key_for_property(
                property.logical_type,
                &property_value.value,
                nested_shapes.get(&(row.object_type_id, *property_id)),
            )?);
        }
    }
    if keys.is_empty() {
        return Ok(None);
    }
    FileDictionaryEncoding::from_keys(keys)
        .map(Some)
        .map_err(|err| err.to_string())
}

pub(crate) fn file_dictionary_index_bytes(dictionary: &FileDictionary) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        cove_core::dictionary::DICT_HEADER_SIZE
            + dictionary.entries.len() * cove_core::dictionary::DICT_INDEX_ENTRY_SIZE,
    );
    out.extend_from_slice(&dictionary.header.serialize());
    for entry in &dictionary.entries {
        out.extend_from_slice(&entry.serialize());
    }
    out
}

pub(crate) fn file_dictionary_key_for_property(
    logical: CoveLogicalType,
    value: &Value,
    nested_shape: Option<&NestedSchemaNodeV1>,
) -> Result<FileDictionaryKey, String> {
    if logical == CoveLogicalType::Json {
        let text = serde_json::to_string(value).map_err(|err| err.to_string())?;
        let canonical = CanonicalValue::Json(&text);
        return Ok(FileDictionaryKey {
            value_tag: canonical.value_tag() as u16,
            canonical: canonical.encode().map_err(|err| err.to_string())?,
        });
    }
    let canonical = canonical_value_for_logical(logical, value, nested_shape)?;
    let value_tag = canonical.value_tag() as u16;
    let canonical = canonical.encode().map_err(|err| err.to_string())?;
    Ok(FileDictionaryKey {
        value_tag,
        canonical,
    })
}

fn canonical_value_for_logical<'a>(
    logical: CoveLogicalType,
    value: &'a Value,
    nested_shape: Option<&NestedSchemaNodeV1>,
) -> Result<CanonicalValue<'a>, String> {
    if value.is_null() {
        return Ok(CanonicalValue::Null);
    }
    match logical {
        CoveLogicalType::Null => Ok(CanonicalValue::Null),
        CoveLogicalType::Bool => Ok(CanonicalValue::Bool(json_bool(value)?)),
        CoveLogicalType::Int8 => Ok(CanonicalValue::Int {
            width: 1,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::Int16 => Ok(CanonicalValue::Int {
            width: 2,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::Int32 => Ok(CanonicalValue::Int {
            width: 4,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::Int64 => Ok(CanonicalValue::Int {
            width: 8,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::UInt8 => Ok(CanonicalValue::Uint {
            width: 1,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::UInt16 => Ok(CanonicalValue::Uint {
            width: 2,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::UInt32 => Ok(CanonicalValue::Uint {
            width: 4,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::UInt64 => Ok(CanonicalValue::Uint {
            width: 8,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::Float32 => Ok(CanonicalValue::Float32(json_f64(value)? as f32)),
        CoveLogicalType::Float64 => Ok(CanonicalValue::Float64(json_f64(value)?)),
        CoveLogicalType::Decimal64 => Ok(CanonicalValue::Decimal64(json_i64(value)?)),
        CoveLogicalType::Decimal128 => Ok(CanonicalValue::Decimal128(json_i128(value)?)),
        CoveLogicalType::DateDays => Ok(CanonicalValue::DateDays(
            json_i64(value)?
                .try_into()
                .map_err(|_| "date_days out of i32 range".to_string())?,
        )),
        CoveLogicalType::TimestampMicros => Ok(CanonicalValue::TimestampMicros(json_i64(value)?)),
        CoveLogicalType::TimestampNanos => Ok(CanonicalValue::TimestampNanos(json_i64(value)?)),
        CoveLogicalType::Utf8 => Ok(CanonicalValue::Utf8(json_string(value)?)),
        CoveLogicalType::Binary => Ok(CanonicalValue::Bytes(json_string(value)?.as_bytes())),
        CoveLogicalType::Uuid => Ok(CanonicalValue::Uuid(json_uuid(value)?)),
        CoveLogicalType::Json => unreachable!("JSON is handled before borrowing conversion"),
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            if let Some(shape) = nested_shape {
                canonical_value_for_nested_shape(shape, value)
            } else {
                match logical {
                    CoveLogicalType::List => canonical_list_value(value),
                    CoveLogicalType::Struct => canonical_struct_value(value),
                    CoveLogicalType::Map => canonical_map_value(value),
                    _ => unreachable!(),
                }
            }
        }
        _ => Err("unsupported future logical type for FileCode dictionary".into()),
    }
}

fn canonical_value_for_nested_shape<'a>(
    shape: &NestedSchemaNodeV1,
    value: &'a Value,
) -> Result<CanonicalValue<'a>, String> {
    if value.is_null() {
        return Ok(CanonicalValue::Null);
    }
    match shape.logical {
        CoveLogicalType::List => {
            let item_shape = shape
                .children
                .first()
                .ok_or_else(|| "list nested_shape requires one child".to_string())?;
            let items = value
                .as_array()
                .ok_or_else(|| "list property value must be an array".to_string())?
                .iter()
                .map(|item| canonical_value_for_nested_shape(item_shape, item))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CanonicalValue::List(items))
        }
        CoveLogicalType::Struct => {
            let object = value
                .as_object()
                .ok_or_else(|| "struct property value must be an object".to_string())?;
            let mut fields = Vec::with_capacity(shape.children.len());
            for (index, child) in shape.children.iter().enumerate() {
                let child_value = object.get(&child.name).unwrap_or(&Value::Null);
                fields.push(CanonicalField {
                    field_id: stable_u32(&child.name, index as u32 + 1) as u64,
                    value: canonical_value_for_nested_shape(child, child_value)?,
                });
            }
            Ok(CanonicalValue::Struct(fields))
        }
        CoveLogicalType::Map => {
            if shape.children.len() != 2 {
                return Err("map nested_shape requires key and value children".into());
            }
            let key_shape = &shape.children[0];
            let value_shape = &shape.children[1];
            let mut entries = Vec::new();
            match value {
                Value::Object(object) => {
                    for (key, value) in object {
                        entries.push((
                            canonical_map_object_key_for_shape(key_shape, key)?,
                            canonical_value_for_nested_shape(value_shape, value)?,
                        ));
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        let pair = item.as_array().ok_or_else(|| {
                            "map array entries must be [key, value] pairs".to_string()
                        })?;
                        if pair.len() != 2 {
                            return Err("map array entries must be [key, value] pairs".into());
                        }
                        entries.push((
                            canonical_value_for_nested_shape(key_shape, &pair[0])?,
                            canonical_value_for_nested_shape(value_shape, &pair[1])?,
                        ));
                    }
                }
                _ => return Err("map property value must be an object or pair array".into()),
            }
            Ok(CanonicalValue::Map(entries))
        }
        _ => canonical_value_for_logical(shape.logical, value, None),
    }
}

fn canonical_map_object_key_for_shape<'a>(
    shape: &NestedSchemaNodeV1,
    key: &'a str,
) -> Result<CanonicalValue<'a>, String> {
    match shape.logical {
        CoveLogicalType::Bool => match key {
            "true" => Ok(CanonicalValue::Bool(true)),
            "false" => Ok(CanonicalValue::Bool(false)),
            _ => Err("map object key is not a boolean".into()),
        },
        CoveLogicalType::Int8 => Ok(CanonicalValue::Int {
            width: 1,
            value: key
                .parse::<i8>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int8".to_string())?,
        }),
        CoveLogicalType::Int16 => Ok(CanonicalValue::Int {
            width: 2,
            value: key
                .parse::<i16>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int16".to_string())?,
        }),
        CoveLogicalType::Int32 => Ok(CanonicalValue::Int {
            width: 4,
            value: key
                .parse::<i32>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int32".to_string())?,
        }),
        CoveLogicalType::Int64 => Ok(CanonicalValue::Int {
            width: 8,
            value: key
                .parse::<i64>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int64".to_string())?,
        }),
        CoveLogicalType::UInt8 => Ok(CanonicalValue::Uint {
            width: 1,
            value: key
                .parse::<u8>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint8".to_string())?,
        }),
        CoveLogicalType::UInt16 => Ok(CanonicalValue::Uint {
            width: 2,
            value: key
                .parse::<u16>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint16".to_string())?,
        }),
        CoveLogicalType::UInt32 => Ok(CanonicalValue::Uint {
            width: 4,
            value: key
                .parse::<u32>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint32".to_string())?,
        }),
        CoveLogicalType::UInt64 => Ok(CanonicalValue::Uint {
            width: 8,
            value: key
                .parse::<u64>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint64".to_string())?,
        }),
        CoveLogicalType::Float32 => Ok(CanonicalValue::Float32(
            key.parse::<f32>()
                .map_err(|_| "map object key is not a float32".to_string())?,
        )),
        CoveLogicalType::Float64 => Ok(CanonicalValue::Float64(
            key.parse::<f64>()
                .map_err(|_| "map object key is not a float64".to_string())?,
        )),
        CoveLogicalType::Decimal64 => Ok(CanonicalValue::Decimal64(
            key.parse::<i64>()
                .map_err(|_| "map object key is not a decimal64".to_string())?,
        )),
        CoveLogicalType::Decimal128 => Ok(CanonicalValue::Decimal128(
            key.parse::<i128>()
                .map_err(|_| "map object key is not a decimal128".to_string())?,
        )),
        CoveLogicalType::DateDays => Ok(CanonicalValue::DateDays(
            key.parse::<i32>()
                .map_err(|_| "map object key is not a date_days".to_string())?,
        )),
        CoveLogicalType::TimestampMicros => Ok(CanonicalValue::TimestampMicros(
            key.parse::<i64>()
                .map_err(|_| "map object key is not a timestamp_micros".to_string())?,
        )),
        CoveLogicalType::TimestampNanos => Ok(CanonicalValue::TimestampNanos(
            key.parse::<i64>()
                .map_err(|_| "map object key is not a timestamp_nanos".to_string())?,
        )),
        CoveLogicalType::Utf8 => Ok(CanonicalValue::Utf8(key)),
        CoveLogicalType::Binary => Ok(CanonicalValue::Bytes(key.as_bytes())),
        CoveLogicalType::Json => Ok(CanonicalValue::Json(key)),
        CoveLogicalType::Uuid => Ok(CanonicalValue::Uuid(hex_decode_16(key)?)),
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            Err("map object keys cannot use nested logical types".into())
        }
        _ => Err("unsupported future map key logical type".into()),
    }
}

fn canonical_value_from_json<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    match value {
        Value::Null => Ok(CanonicalValue::Null),
        Value::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(CanonicalValue::Int {
                    width: 8,
                    value: i128::from(value),
                })
            } else if let Some(value) = number.as_u64() {
                Ok(CanonicalValue::Uint {
                    width: 8,
                    value: u128::from(value),
                })
            } else {
                Ok(CanonicalValue::Float64(
                    number
                        .as_f64()
                        .ok_or_else(|| "non-finite JSON number".to_string())?,
                ))
            }
        }
        Value::String(value) => Ok(CanonicalValue::Utf8(value)),
        Value::Array(_) => canonical_list_value(value),
        Value::Object(_) => canonical_struct_value(value),
    }
}

fn canonical_list_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "list property value must be an array".to_string())?
        .iter()
        .map(canonical_value_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CanonicalValue::List(items))
}

fn canonical_struct_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "struct property value must be an object".to_string())?;
    let mut fields = Vec::with_capacity(object.len());
    for (index, (name, value)) in object.iter().enumerate() {
        fields.push(CanonicalField {
            field_id: stable_u32(name, index as u32 + 1) as u64,
            value: canonical_value_from_json(value)?,
        });
    }
    fields.sort_by_key(|field| field.field_id);
    Ok(CanonicalValue::Struct(fields))
}

fn canonical_map_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    let mut entries = Vec::new();
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                entries.push((CanonicalValue::Utf8(key), canonical_value_from_json(value)?));
            }
        }
        Value::Array(items) => {
            for item in items {
                let pair = item
                    .as_array()
                    .ok_or_else(|| "map array entries must be [key, value] pairs".to_string())?;
                if pair.len() != 2 {
                    return Err("map array entries must be [key, value] pairs".into());
                }
                entries.push((
                    canonical_value_from_json(&pair[0])?,
                    canonical_value_from_json(&pair[1])?,
                ));
            }
        }
        _ => return Err("map property value must be an object or pair array".into()),
    }
    Ok(CanonicalValue::Map(entries))
}

pub(crate) fn append_property_value_bytes(
    property: &PropertyEntryV1,
    value: &Value,
    nested_shape: Option<&NestedSchemaNodeV1>,
    dictionary: Option<&FileDictionaryEncoding>,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    if value.is_null() {
        append_null_placeholder(property, out)?;
        return Ok(());
    }
    match property.physical_kind {
        CovePhysicalKind::Boolean => out.push(if json_bool(value)? { 1 } else { 0 }),
        CovePhysicalKind::NumCode => out.extend_from_slice(&json_numcode(value)?.to_le_bytes()),
        CovePhysicalKind::FixedBytes => {
            let bytes = fixed_bytes_for_property(property, value)?;
            out.extend_from_slice(&bytes);
        }
        CovePhysicalKind::VarBytes => {
            let bytes = var_bytes_for_property(property, value)?;
            let len = u32::try_from(bytes.len())
                .map_err(|_| "property value is too large".to_string())?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        CovePhysicalKind::FileCode => {
            let dictionary = dictionary.ok_or_else(|| {
                "COVE-MAP writer needs a file dictionary for FileCode properties".to_string()
            })?;
            let key = file_dictionary_key_for_property(property.logical_type, value, nested_shape)?;
            let code = dictionary
                .file_code_for_key(&key)
                .map_err(|err| err.to_string())?;
            out.extend_from_slice(&code.to_le_bytes());
        }
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            return Err("COVE-MAP writer does not materialize nested properties yet".into())
        }
        _ => return Err("unsupported future COVE physical kind".into()),
    }
    Ok(())
}

fn append_null_placeholder(property: &PropertyEntryV1, out: &mut Vec<u8>) -> Result<(), String> {
    match property.physical_kind {
        CovePhysicalKind::Boolean => out.push(0),
        CovePhysicalKind::NumCode => out.extend_from_slice(&0u64.to_le_bytes()),
        CovePhysicalKind::FixedBytes => {
            let width = match property.logical_type {
                CoveLogicalType::Uuid | CoveLogicalType::Decimal128 => 16,
                CoveLogicalType::Decimal64 => 8,
                _ => return Err("unsupported fixed-width null placeholder".into()),
            };
            out.resize(out.len() + width, 0);
        }
        CovePhysicalKind::VarBytes => out.extend_from_slice(&0u32.to_le_bytes()),
        CovePhysicalKind::FileCode => out.extend_from_slice(&0u32.to_le_bytes()),
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            return Err("nested null placeholders are not supported".into())
        }
        _ => return Err("unsupported future COVE physical kind".into()),
    }
    Ok(())
}

pub(crate) fn temporal_segment_index(
    segments: &[TemporalSegmentBuild],
) -> Result<TemporalSegmentIndex, String> {
    let mut entries = Vec::with_capacity(segments.len());
    for segment in segments {
        let min_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .min()
            .unwrap_or([0; 16]);
        let max_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .max()
            .unwrap_or([0; 16]);
        let (delta_count, snapshot_count, baseline_count, tombstone_count) =
            row_kind_counts(&segment.rows);
        entries.push(TemporalSegmentIndexEntryV1 {
            segment_id: segment.segment_id,
            object_type_id: segment.object_type_id,
            time_range_start_us: 0,
            time_range_end_us: 0,
            csn_min: 0,
            csn_max: segment.rows.len().saturating_sub(1) as u64,
            row_count: u32::try_from(segment.rows.len())
                .map_err(|_| "too many COVE-O rows".to_string())?,
            delta_count,
            snapshot_count,
            baseline_count,
            tombstone_count,
            min_goid,
            max_goid,
            offset: 0,
            length: segment.payload.len() as u64,
            checksum: 0,
        });
    }
    Ok(TemporalSegmentIndex { flags: 0, entries })
}

fn row_kind_counts(rows: &[ObjectRow]) -> (u32, u32, u32, u32) {
    let mut delta = 0;
    let mut snapshot = 0;
    let mut baseline = 0;
    let mut tombstone = 0;
    for row in rows {
        match row.record_kind {
            RecordKind::Delta => delta += 1,
            RecordKind::Snapshot => snapshot += 1,
            RecordKind::Baseline => baseline += 1,
            RecordKind::Tombstone => tombstone += 1,
            RecordKind::ReservedLegacyMaterializedDelta => {}
            _ => {}
        }
    }
    (delta, snapshot, baseline, tombstone)
}

fn temporal_prev_refs(segment_id: u32, rows: &[ObjectRow]) -> Vec<Option<CoveRecordRefV1>> {
    let mut latest_by_goid = BTreeMap::<[u8; 16], u32>::new();
    let mut refs = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let prev_ref = if matches!(
            row.record_kind,
            RecordKind::Delta | RecordKind::Snapshot | RecordKind::Tombstone
        ) {
            latest_by_goid
                .get(&row.goid)
                .copied()
                .map(|row_index| CoveRecordRefV1 {
                    segment_id,
                    row_index,
                    target_kind: 0,
                })
        } else {
            None
        };
        refs.push(prev_ref);
        latest_by_goid.insert(row.goid, index as u32);
    }
    refs
}

pub(crate) fn trust_manifest(
    segments: &[TemporalSegmentBuild],
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<TrustManifest, String> {
    let mut previous = [0u8; 32];
    let mut entries = Vec::new();
    for segment in segments {
        let parsed_segment =
            TemporalSegmentData::parse(&segment.payload).map_err(|err| err.to_string())?;
        let dictionary = dictionary.map(|encoding| &encoding.dictionary);
        for index in 0..parsed_segment.rows.len() {
            let payload =
                temporal_row_trust_payload(&parsed_segment, index as u32, dictionary, &[])
                    .map_err(|err| err.to_string())?;
            let expected_hash =
                trust_chain::chain(&previous, &payload).map_err(|err| err.to_string())?;
            entries.push(TrustManifestEntryV1 {
                segment_id: segment.segment_id,
                row_index: index as u32,
                expected_hash,
            });
            previous = expected_hash;
        }
    }
    Ok(TrustManifest { entries })
}
