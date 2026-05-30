use super::*;

fn projection_logical_type_from_name(name: &str) -> Result<CoveLogicalType, String> {
    cove_core::types::logical_type_from_name(name)
        .map_err(|_| format!("unsupported nested_shape logical_type '{name}'"))
}

pub(super) fn encode_arrow_projection(table: &ProjectedTable) -> Result<Vec<u8>, String> {
    let batch = arrow_record_batch(table)?;
    let schema = batch.schema();
    let mut bytes = Vec::new();
    {
        let mut writer = FileWriter::try_new(&mut bytes, &schema)
            .map_err(|err| format!("cannot create Arrow IPC writer: {err}"))?;
        writer
            .write(&batch)
            .map_err(|err| format!("cannot write Arrow IPC batch: {err}"))?;
        writer
            .finish()
            .map_err(|err| format!("cannot finish Arrow IPC file: {err}"))?;
    }
    Ok(bytes)
}

pub(super) fn arrow_record_batch(table: &ProjectedTable) -> Result<RecordBatch, String> {
    let schema = arrow_schema_from_projected_columns(&table.columns)?;
    let arrays = table
        .columns
        .iter()
        .map(|column| encode_arrow_column(table, column))
        .collect::<Result<Vec<_>, _>>()?;
    let options = RecordBatchOptions::new().with_row_count(Some(table.rows.len()));
    RecordBatch::try_new_with_options(schema, arrays, &options)
        .map_err(|err| format!("cannot build Arrow record batch: {err}"))
}

pub(super) fn arrow_schema_from_projected_columns(
    columns: &[ProjectedColumn],
) -> Result<SchemaRef, String> {
    let fields = columns
        .iter()
        .map(|column| Ok::<Field, String>(Field::new(&column.name, arrow_data_type(column)?, true)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Arc::new(Schema::new(fields)))
}

fn arrow_data_type(column: &ProjectedColumn) -> Result<DataType, String> {
    match column.logical {
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            nested_arrow_data_type(column)
        }
        logical => arrow_data_type_for_logical(logical),
    }
}

fn arrow_data_type_for_logical(logical: CoveLogicalType) -> Result<DataType, String> {
    match logical {
        CoveLogicalType::Null | CoveLogicalType::Utf8 => Ok(DataType::Utf8),
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
        CoveLogicalType::Decimal64 => Ok(DataType::Decimal64(18, 0)),
        CoveLogicalType::Decimal128 => Ok(DataType::Decimal128(38, 0)),
        CoveLogicalType::DateDays => Ok(DataType::Date32),
        CoveLogicalType::TimestampMicros => Ok(DataType::Timestamp(TimeUnit::Microsecond, None)),
        CoveLogicalType::TimestampNanos => Ok(DataType::Timestamp(TimeUnit::Nanosecond, None)),
        CoveLogicalType::Binary | CoveLogicalType::Json => Ok(DataType::Binary),
        CoveLogicalType::Uuid => Ok(DataType::FixedSizeBinary(16)),
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => Err(format!(
            "nested logical type {logical:?} requires nested_shape"
        )),
        _ => Err(format!(
            "unknown projection logical type {logical:?} is not supported for Arrow output"
        )),
    }
}

fn nested_arrow_data_type(column: &ProjectedColumn) -> Result<DataType, String> {
    let shape = column
        .nested_shape
        .as_deref()
        .ok_or_else(|| format!("projection column '{}' requires nested_shape", column.name))?;
    let value: Value = serde_json::from_str(shape).map_err(|err| {
        format!(
            "projection column '{}' has invalid nested_shape JSON: {err}",
            column.name
        )
    })?;
    let data_type = nested_shape_data_type(&value)?;
    match (&column.logical, &data_type) {
        (CoveLogicalType::List, DataType::List(_))
        | (CoveLogicalType::Struct, DataType::Struct(_))
        | (CoveLogicalType::Map, DataType::Map(_, _)) => Ok(data_type),
        _ => Err(format!(
            "projection column '{}' nested_shape does not match logical type {:?}",
            column.name, column.logical
        )),
    }
}

fn nested_shape_data_type(value: &Value) -> Result<DataType, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "nested_shape must be a JSON object".to_string())?;
    let kind = object
        .get("type")
        .or_else(|| object.get("kind"))
        .and_then(Value::as_str)
        .ok_or_else(|| "nested_shape requires type".to_string())?;
    match kind {
        "list" => {
            let item = object
                .get("item")
                .or_else(|| object.get("element"))
                .ok_or_else(|| "list nested_shape requires item".to_string())?;
            let field = nested_shape_field("item", item, true)?;
            Ok(DataType::List(Arc::new(field)))
        }
        "struct" => {
            let fields = object
                .get("fields")
                .and_then(Value::as_array)
                .ok_or_else(|| "struct nested_shape requires fields array".to_string())?;
            let fields = fields
                .iter()
                .map(|field| {
                    let name = field
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "struct field nested_shape requires name".to_string())?;
                    nested_shape_field(name, field, true)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DataType::Struct(Fields::from(fields)))
        }
        "map" => {
            let key = object
                .get("key")
                .ok_or_else(|| "map nested_shape requires key".to_string())?;
            let value = object
                .get("value")
                .ok_or_else(|| "map nested_shape requires value".to_string())?;
            let key = nested_shape_field("key", key, false)?;
            let value = nested_shape_field("value", value, true)?;
            Ok(Field::new_map(
                "map",
                "entries",
                Arc::new(key),
                Arc::new(value),
                false,
                true,
            )
            .data_type()
            .clone())
        }
        other => Err(format!("unsupported nested_shape type '{other}'")),
    }
}

fn nested_shape_field(name: &str, value: &Value, default_nullable: bool) -> Result<Field, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "nested_shape field must be an object".to_string())?;
    let nullable = object
        .get("nullable")
        .and_then(Value::as_bool)
        .unwrap_or(default_nullable);
    let data_type = if object
        .get("type")
        .or_else(|| object.get("kind"))
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "list" | "struct" | "map"))
    {
        nested_shape_data_type(value)?
    } else {
        let logical = object
            .get("logical_type")
            .or_else(|| object.get("logical"))
            .or_else(|| object.get("type"))
            .and_then(Value::as_str)
            .ok_or_else(|| "nested_shape field requires logical_type".to_string())?;
        arrow_data_type_for_logical(projection_logical_type_from_name(logical)?)?
    };
    Ok(Field::new(name, data_type, nullable))
}

fn encode_arrow_column(
    table: &ProjectedTable,
    column: &ProjectedColumn,
) -> Result<ArrayRef, String> {
    let values = table
        .rows
        .iter()
        .map(|row| row.get(&column.name).unwrap_or(&Value::Null))
        .collect::<Vec<_>>();
    encode_arrow_values(&values, column)
}

pub(super) fn encode_arrow_values(
    values: &[&Value],
    column: &ProjectedColumn,
) -> Result<ArrayRef, String> {
    match column.logical {
        CoveLogicalType::Null | CoveLogicalType::Utf8 => Ok(Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| typed_string_value(value, column.logical))
                .collect::<Result<Vec<_>, _>>()?,
        )) as ArrayRef),
        CoveLogicalType::Bool => Ok(Arc::new(BooleanArray::from(
            values
                .iter()
                .map(|value| typed_bool_value(value))
                .collect::<Result<Vec<_>, _>>()?,
        )) as ArrayRef),
        CoveLogicalType::Int8 => primitive_array::<Int8Array, i8>(&values, column.logical),
        CoveLogicalType::Int16 => primitive_array::<Int16Array, i16>(&values, column.logical),
        CoveLogicalType::Int32 => primitive_array::<Int32Array, i32>(&values, column.logical),
        CoveLogicalType::Int64 => primitive_array::<Int64Array, i64>(&values, column.logical),
        CoveLogicalType::UInt8 => primitive_array::<UInt8Array, u8>(&values, column.logical),
        CoveLogicalType::UInt16 => primitive_array::<UInt16Array, u16>(&values, column.logical),
        CoveLogicalType::UInt32 => primitive_array::<UInt32Array, u32>(&values, column.logical),
        CoveLogicalType::UInt64 => primitive_array::<UInt64Array, u64>(&values, column.logical),
        CoveLogicalType::Float32 => Ok(Arc::new(Float32Array::from(
            values
                .iter()
                .map(|value| typed_f64_value(value).map(|value| value.map(|value| value as f32)))
                .collect::<Result<Vec<_>, _>>()?,
        )) as ArrayRef),
        CoveLogicalType::Float64 => Ok(Arc::new(Float64Array::from(
            values
                .iter()
                .map(|value| typed_f64_value(value))
                .collect::<Result<Vec<_>, _>>()?,
        )) as ArrayRef),
        CoveLogicalType::Decimal64 => Ok(Arc::new(
            Decimal64Array::from(
                values
                    .iter()
                    .map(|value| {
                        typed_i128_value(value, column.logical).and_then(|value| {
                            value
                                .map(|value| {
                                    i64::try_from(value).map_err(|_| {
                                        "decimal64 projection value is outside i64 range"
                                            .to_string()
                                    })
                                })
                                .transpose()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .with_precision_and_scale(18, 0)
            .map_err(|err| format!("cannot assign decimal64 precision/scale: {err}"))?,
        ) as ArrayRef),
        CoveLogicalType::Decimal128 => Ok(Arc::new(
            Decimal128Array::from(
                values
                    .iter()
                    .map(|value| typed_i128_value(value, column.logical))
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .with_precision_and_scale(38, 0)
            .map_err(|err| format!("cannot assign decimal128 precision/scale: {err}"))?,
        ) as ArrayRef),
        CoveLogicalType::DateDays => primitive_array::<Date32Array, i32>(&values, column.logical),
        CoveLogicalType::TimestampMicros => {
            primitive_array::<TimestampMicrosecondArray, i64>(&values, column.logical)
        }
        CoveLogicalType::TimestampNanos => {
            primitive_array::<TimestampNanosecondArray, i64>(&values, column.logical)
        }
        CoveLogicalType::Binary | CoveLogicalType::Json => {
            let owned = values
                .iter()
                .map(|value| typed_bytes_value(value, column.logical))
                .collect::<Result<Vec<_>, _>>()?;
            let borrowed = owned
                .iter()
                .map(|value| value.as_deref())
                .collect::<Vec<_>>();
            Ok(Arc::new(BinaryArray::from_opt_vec(borrowed)) as ArrayRef)
        }
        CoveLogicalType::Uuid => {
            let owned = values
                .iter()
                .map(|value| typed_uuid_value(value).map(|value| value.map(Vec::from)))
                .collect::<Result<Vec<_>, _>>()?;
            let borrowed = owned
                .iter()
                .map(|value| value.as_deref())
                .collect::<Vec<_>>();
            Ok(Arc::new(FixedSizeBinaryArray::from(borrowed)) as ArrayRef)
        }
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            encode_arrow_nested_column(column, &values)
        }
        _ => Err(format!(
            "unknown projection logical type {:?} is not supported for Arrow output",
            column.logical
        )),
    }
}

fn encode_arrow_nested_column(
    column: &ProjectedColumn,
    values: &[&Value],
) -> Result<ArrayRef, String> {
    let data_type = arrow_data_type(column)?;
    if values.is_empty() {
        return Ok(new_empty_array(&data_type));
    }
    let field = Arc::new(Field::new(&column.name, data_type, true));
    let mut json_lines = String::new();
    for value in values {
        json_lines.push_str(
            &serde_json::to_string(value)
                .map_err(|err| format!("cannot encode nested projection JSON value: {err}"))?,
        );
        json_lines.push('\n');
    }
    let mut reader = JsonReaderBuilder::new_with_field(field)
        .with_batch_size(values.len().max(1))
        .build(json_lines.as_bytes())
        .map_err(|err| format!("cannot build nested Arrow projection decoder: {err}"))?;
    match reader.next() {
        Some(Ok(batch)) => Ok(batch.column(0).clone()),
        Some(Err(err)) => Err(format!(
            "cannot encode nested Arrow projection column: {err}"
        )),
        None => Ok(new_empty_array(&arrow_data_type(column)?)),
    }
}

fn primitive_array<A, T>(values: &[&Value], logical: CoveLogicalType) -> Result<ArrayRef, String>
where
    A: From<Vec<Option<T>>> + Array + 'static,
    T: TryFrom<i128>,
{
    let converted = values
        .iter()
        .map(|value| {
            typed_i128_value(value, logical).and_then(|value| {
                value
                    .map(|value| {
                        T::try_from(value)
                            .map_err(|_| format!("projection value is outside {logical:?} range"))
                    })
                    .transpose()
            })
        })
        .collect::<Vec<_>>();
    let converted = converted.into_iter().collect::<Result<Vec<_>, _>>()?;
    Ok(Arc::new(A::from(converted)) as ArrayRef)
}

pub(super) fn encode_cove_t_projection(table: &ProjectedTable) -> Result<Vec<u8>, String> {
    let columns = table
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            Ok(ColumnEntry {
                column_id: (index + 1) as u32,
                name: column.name.clone(),
                logical: column.logical,
                physical: projection_physical_kind(column.logical)?,
                nullable: true,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let row_count =
        u32::try_from(table.rows.len()).map_err(|_| "too many projection rows".to_string())?;
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: table.mapping_id.clone(),
            name: table.output_table.clone(),
            row_count: table.rows.len() as u64,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns,
        }],
    };
    let mut segment = ScanSegment::new(1, 0, 0, row_count, table.columns.len() as u32);
    segment.morsel_row_count = row_count.max(1);
    for (index, column) in table.columns.iter().enumerate() {
        let physical = projection_physical_kind(column.logical)?;
        let (payload, null_count) = if matches!(
            physical,
            CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map
        ) {
            (
                projection_nested_payload(table, column, (index + 1) as u32)?,
                0,
            )
        } else {
            projection_physical_payload(table, column, physical)?
        };
        let page = ScanPageSpec::new(row_count, payload)
            .with_encoding_root(projection_encoding_kind(physical) as u32)
            .with_counts(row_count.saturating_sub(null_count), null_count);
        segment.set_column_pages((index + 1) as u32, vec![page]);
    }
    let mut writer = ScanProfileCoveWriter::new(catalog);
    let nested_entries = table
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| {
            matches!(
                column.logical,
                CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map
            )
        })
        .map(|(index, column)| {
            Ok(NestedSchemaEntryV1 {
                table_id: 1,
                column_id: (index + 1) as u32,
                root: nested_schema_node_for_column(column)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !nested_entries.is_empty() {
        writer
            .push_nested_schema(&NestedSchemaSectionV1::new(nested_entries))
            .map_err(|err| err.to_string())?;
    }
    writer.metadata_json = serde_json::to_vec(&json!({
        "projection_id": table.projection_id,
        "mapping_id": table.mapping_id,
        "mapping_version": table.mapping_version,
    }))
    .map_err(|err| err.to_string())?;
    writer.push_segment(segment);
    writer.write().map_err(|err| err.to_string())
}

fn projection_physical_kind(logical: CoveLogicalType) -> Result<CovePhysicalKind, String> {
    match logical {
        CoveLogicalType::Bool => Ok(CovePhysicalKind::Boolean),
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64
        | CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64
        | CoveLogicalType::Float32
        | CoveLogicalType::Float64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::DateDays
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => Ok(CovePhysicalKind::NumCode),
        CoveLogicalType::Decimal128 | CoveLogicalType::Uuid => Ok(CovePhysicalKind::FixedBytes),
        CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json => {
            Ok(CovePhysicalKind::VarBytes)
        }
        CoveLogicalType::List => Ok(CovePhysicalKind::List),
        CoveLogicalType::Struct => Ok(CovePhysicalKind::Struct),
        CoveLogicalType::Map => Ok(CovePhysicalKind::Map),
        CoveLogicalType::Null => Err(format!(
            "projection logical type {logical:?} is not supported for COVE-T output"
        )),
        _ => Err(format!("unknown projection logical type {logical:?}")),
    }
}

fn projection_encoding_kind(physical: CovePhysicalKind) -> CoveEncodingKind {
    cove_core::types::encoding_kind_for_physical(physical)
}

fn projection_physical_payload(
    table: &ProjectedTable,
    column: &ProjectedColumn,
    physical: CovePhysicalKind,
) -> Result<(Vec<u8>, u32), String> {
    let mut null_bitmap = vec![0u8; table.rows.len().div_ceil(8)];
    let mut values = Vec::new();
    let mut null_count = 0u32;
    for (row_index, row) in table.rows.iter().enumerate() {
        let value = row.get(&column.name).unwrap_or(&Value::Null);
        if value.is_null() {
            null_count = null_count
                .checked_add(1)
                .ok_or_else(|| "projection null count overflow".to_string())?;
            null_bitmap[row_index / 8] |= 1u8 << (row_index % 8);
        }
        append_projection_physical_value(column.logical, physical, value, &mut values)?;
    }
    let mut payload = Vec::new();
    if null_count != 0 {
        payload.extend_from_slice(&null_bitmap);
    }
    payload.extend_from_slice(&values);
    Ok((payload, null_count))
}

struct NestedPayloadBuild {
    logical_len: u32,
}

fn projection_nested_payload(
    table: &ProjectedTable,
    column: &ProjectedColumn,
    _column_id: u32,
) -> Result<Vec<u8>, String> {
    let schema = nested_schema_node_for_column(column)?;
    let values = table
        .rows
        .iter()
        .map(|row| row.get(&column.name).unwrap_or(&Value::Null).clone())
        .collect::<Vec<_>>();
    let mut nodes = Vec::new();
    let mut buffers = Vec::new();
    let mut next_node_id = 0u16;
    let root = build_nested_payload_node(
        &schema,
        &values,
        &mut next_node_id,
        &mut nodes,
        &mut buffers,
    )?;
    ColumnPagePayloadV1::build_tree(root.logical_len, 0, nodes, buffers)
        .map_err(|err| err.to_string())
}

fn build_nested_payload_node(
    schema: &NestedSchemaNodeV1,
    values: &[Value],
    next_node_id: &mut u16,
    nodes: &mut Vec<CoveEncodingNodeV1>,
    buffers: &mut Vec<(PageBufferKind, Vec<u8>)>,
) -> Result<NestedPayloadBuild, String> {
    let node_id = *next_node_id;
    *next_node_id = next_node_id
        .checked_add(1)
        .ok_or_else(|| "nested projection node id overflow".to_string())?;
    match schema.physical {
        CovePhysicalKind::List => {
            let child_schema = schema
                .children
                .first()
                .ok_or_else(|| "list nested_shape requires one child".to_string())?;
            let mut offsets = Vec::with_capacity(values.len() + 1);
            offsets.push(0u32);
            let mut child_values = Vec::new();
            for value in values {
                if value.is_null() {
                    offsets.push(child_values.len() as u32);
                    continue;
                }
                let array = value
                    .as_array()
                    .ok_or_else(|| "list projection value must be an array".to_string())?;
                child_values.extend(array.iter().cloned());
                offsets.push(child_values.len() as u32);
            }
            let layout = ListLayoutPayload {
                layout: ListLayout { offsets },
                child_row_count: child_values.len() as u32,
            };
            nodes.push(CoveEncodingNodeV1 {
                node_id,
                encoding_kind: CoveEncodingKind::Canonical,
                logical_type: schema.logical,
                physical_kind: schema.physical,
                flags: 0,
                logical_len: values.len() as u32,
                child_count: 1,
                buffer_count: 1,
                params_offset: 0,
                params_length: 0,
                stats_id: 0,
                reserved: 0,
            });
            buffers.push((PageBufferKind::ChildLayout, layout.encode()));
            build_nested_payload_node(child_schema, &child_values, next_node_id, nodes, buffers)?;
            Ok(NestedPayloadBuild {
                logical_len: values.len() as u32,
            })
        }
        CovePhysicalKind::Struct => {
            let row_count = values.len() as u64;
            let layout = StructLayoutPayload {
                layout: StructLayout {
                    field_row_counts: vec![row_count; schema.children.len()],
                },
                parent_null_handling_declared: true,
            };
            nodes.push(CoveEncodingNodeV1 {
                node_id,
                encoding_kind: CoveEncodingKind::Canonical,
                logical_type: schema.logical,
                physical_kind: schema.physical,
                flags: 0,
                logical_len: values.len() as u32,
                child_count: schema.children.len() as u16,
                buffer_count: 1,
                params_offset: 0,
                params_length: 0,
                stats_id: 0,
                reserved: 0,
            });
            buffers.push((PageBufferKind::ChildLayout, layout.encode()));
            for child in &schema.children {
                let child_values = values
                    .iter()
                    .map(|value| {
                        if value.is_null() {
                            Value::Null
                        } else {
                            value
                                .as_object()
                                .and_then(|object| object.get(&child.name))
                                .cloned()
                                .unwrap_or(Value::Null)
                        }
                    })
                    .collect::<Vec<_>>();
                build_nested_payload_node(child, &child_values, next_node_id, nodes, buffers)?;
            }
            Ok(NestedPayloadBuild {
                logical_len: values.len() as u32,
            })
        }
        CovePhysicalKind::Map => {
            if schema.children.len() != 2 {
                return Err("map nested_shape requires key and value children".into());
            }
            let mut offsets = Vec::with_capacity(values.len() + 1);
            let mut key_values = Vec::new();
            let mut value_values = Vec::new();
            let mut canonical_keys = Vec::new();
            offsets.push(0u32);
            for value in values {
                if value.is_null() {
                    offsets.push(key_values.len() as u32);
                    continue;
                }
                let object = value
                    .as_object()
                    .ok_or_else(|| "map projection value must be a JSON object".to_string())?;
                for (key, value) in object {
                    key_values.push(Value::String(key.clone()));
                    value_values.push(value.clone());
                    canonical_keys.push(key.as_bytes().to_vec());
                }
                offsets.push(key_values.len() as u32);
            }
            let layout = MapLayoutPayload {
                layout: MapLayout {
                    offsets,
                    key_row_count: key_values.len() as u32,
                    value_row_count: value_values.len() as u32,
                    keys_are_scalar: true,
                    allow_duplicate_keys: false,
                    canonical_keys,
                },
            };
            nodes.push(CoveEncodingNodeV1 {
                node_id,
                encoding_kind: CoveEncodingKind::Canonical,
                logical_type: schema.logical,
                physical_kind: schema.physical,
                flags: 0,
                logical_len: values.len() as u32,
                child_count: 2,
                buffer_count: 1,
                params_offset: 0,
                params_length: 0,
                stats_id: 0,
                reserved: 0,
            });
            buffers.push((PageBufferKind::ChildLayout, layout.encode()));
            build_nested_payload_node(
                &schema.children[0],
                &key_values,
                next_node_id,
                nodes,
                buffers,
            )?;
            build_nested_payload_node(
                &schema.children[1],
                &value_values,
                next_node_id,
                nodes,
                buffers,
            )?;
            Ok(NestedPayloadBuild {
                logical_len: values.len() as u32,
            })
        }
        _ => {
            let mut null_bitmap = vec![0u8; values.len().div_ceil(8)];
            let mut null_count = 0usize;
            let mut encoded_values = Vec::new();
            for (index, value) in values.iter().enumerate() {
                if value.is_null() {
                    null_count += 1;
                    null_bitmap[index / 8] |= 1u8 << (index % 8);
                }
                append_projection_physical_value(
                    schema.logical,
                    schema.physical,
                    value,
                    &mut encoded_values,
                )?;
            }
            let mut direct_buffers = Vec::new();
            if null_count != 0 {
                direct_buffers.push((PageBufferKind::NullBitmap, null_bitmap));
            }
            if !encoded_values.is_empty() {
                direct_buffers.push((PageBufferKind::Values, encoded_values));
            }
            nodes.push(CoveEncodingNodeV1 {
                node_id,
                encoding_kind: projection_encoding_kind(schema.physical),
                logical_type: schema.logical,
                physical_kind: schema.physical,
                flags: 0,
                logical_len: values.len() as u32,
                child_count: 0,
                buffer_count: direct_buffers.len() as u16,
                params_offset: 0,
                params_length: 0,
                stats_id: 0,
                reserved: 0,
            });
            buffers.extend(direct_buffers);
            Ok(NestedPayloadBuild {
                logical_len: values.len() as u32,
            })
        }
    }
}

fn nested_schema_node_for_column(column: &ProjectedColumn) -> Result<NestedSchemaNodeV1, String> {
    let shape = column
        .nested_shape
        .as_deref()
        .ok_or_else(|| format!("projection column '{}' requires nested_shape", column.name))?;
    let value: Value = serde_json::from_str(shape).map_err(|err| {
        format!(
            "projection column '{}' has invalid nested_shape JSON: {err}",
            column.name
        )
    })?;
    let mut root = nested_schema_node_from_shape(&column.name, &value, true)?;
    root.name = column.name.clone();
    root.logical = column.logical;
    root.physical = projection_physical_kind(column.logical)?;
    Ok(root)
}

pub(crate) fn nested_schema_node_from_shape(
    fallback_name: &str,
    value: &Value,
    default_nullable: bool,
) -> Result<NestedSchemaNodeV1, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "nested_shape node must be a JSON object".to_string())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(fallback_name)
        .to_string();
    let nullable = object
        .get("nullable")
        .and_then(Value::as_bool)
        .unwrap_or(default_nullable);
    let kind = object
        .get("type")
        .or_else(|| object.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("scalar");
    match kind {
        "list" => {
            let item = object
                .get("item")
                .or_else(|| object.get("element"))
                .ok_or_else(|| "list nested_shape requires item".to_string())?;
            Ok(NestedSchemaNodeV1 {
                name,
                logical: CoveLogicalType::List,
                physical: CovePhysicalKind::List,
                nullable,
                precision: 0,
                scale: 0,
                collation_id: 0,
                flags: 0,
                fixed_size_list_len: object
                    .get("fixed_size_list_len")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                children: vec![nested_schema_node_from_shape("item", item, true)?],
            })
        }
        "struct" => {
            let fields = object
                .get("fields")
                .and_then(Value::as_array)
                .ok_or_else(|| "struct nested_shape requires fields array".to_string())?;
            let children = fields
                .iter()
                .map(|field| {
                    let name = field
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "struct field nested_shape requires name".to_string())?;
                    nested_schema_node_from_shape(name, field, true)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(NestedSchemaNodeV1 {
                name,
                logical: CoveLogicalType::Struct,
                physical: CovePhysicalKind::Struct,
                nullable,
                precision: 0,
                scale: 0,
                collation_id: 0,
                flags: 0,
                fixed_size_list_len: 0,
                children,
            })
        }
        "map" => {
            let key = object
                .get("key")
                .ok_or_else(|| "map nested_shape requires key".to_string())?;
            let value = object
                .get("value")
                .ok_or_else(|| "map nested_shape requires value".to_string())?;
            Ok(NestedSchemaNodeV1 {
                name,
                logical: CoveLogicalType::Map,
                physical: CovePhysicalKind::Map,
                nullable,
                precision: 0,
                scale: 0,
                collation_id: 0,
                flags: 0,
                fixed_size_list_len: 0,
                children: vec![
                    nested_schema_node_from_shape("key", key, false)?,
                    nested_schema_node_from_shape("value", value, true)?,
                ],
            })
        }
        _ => {
            let logical = object
                .get("logical_type")
                .or_else(|| object.get("logical"))
                .or_else(|| object.get("type"))
                .and_then(Value::as_str)
                .ok_or_else(|| "scalar nested_shape requires logical_type".to_string())
                .and_then(projection_logical_type_from_name)?;
            Ok(NestedSchemaNodeV1::scalar(
                name,
                logical,
                projection_physical_kind(logical)?,
                nullable,
            ))
        }
    }
}

fn append_projection_physical_value(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    value: &Value,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    if value.is_null() {
        append_projection_null_placeholder(logical, physical, out)?;
        return Ok(());
    }
    match physical {
        CovePhysicalKind::Boolean => out.push(u8::from(typed_bool_value(value)?.unwrap_or(false))),
        CovePhysicalKind::NumCode => {
            out.extend_from_slice(&projection_numcode(logical, value)?.to_le_bytes())
        }
        CovePhysicalKind::FixedBytes => {
            out.extend_from_slice(&projection_fixed_bytes(logical, value)?)
        }
        CovePhysicalKind::VarBytes => {
            let bytes = typed_bytes_value(value, logical)?.unwrap_or_default();
            let len = u32::try_from(bytes.len())
                .map_err(|_| "projection value exceeds COVE-T VarBytes limit".to_string())?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        CovePhysicalKind::FileCode
        | CovePhysicalKind::List
        | CovePhysicalKind::Struct
        | CovePhysicalKind::Map => {
            return Err(format!(
                "projection physical kind {physical:?} is not supported for COVE-T output"
            ))
        }
        _ => return Err(format!("unknown projection physical kind {physical:?}")),
    }
    Ok(())
}

fn append_projection_null_placeholder(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    match physical {
        CovePhysicalKind::Boolean => out.push(0),
        CovePhysicalKind::NumCode => out.extend_from_slice(&0u64.to_le_bytes()),
        CovePhysicalKind::FixedBytes => {
            let width = match logical {
                CoveLogicalType::Decimal128 | CoveLogicalType::Uuid => 16,
                _ => {
                    return Err(format!(
                        "unsupported fixed-width projection logical type {logical:?}"
                    ))
                }
            };
            out.resize(out.len() + width, 0);
        }
        CovePhysicalKind::VarBytes => out.extend_from_slice(&0u32.to_le_bytes()),
        CovePhysicalKind::FileCode
        | CovePhysicalKind::List
        | CovePhysicalKind::Struct
        | CovePhysicalKind::Map => {
            return Err(format!(
                "projection physical kind {physical:?} is not supported for null placeholders"
            ))
        }
        _ => return Err(format!("unknown projection physical kind {physical:?}")),
    }
    Ok(())
}

fn projection_numcode(logical: CoveLogicalType, value: &Value) -> Result<u64, String> {
    Ok(match logical {
        CoveLogicalType::Int8 => i8::try_from(typed_i128_required(value, logical)?)
            .map_err(|_| "int8 projection value is out of range".to_string())?
            as u8 as u64,
        CoveLogicalType::Int16 => i16::try_from(typed_i128_required(value, logical)?)
            .map_err(|_| "int16 projection value is out of range".to_string())?
            as u16 as u64,
        CoveLogicalType::Int32 | CoveLogicalType::DateDays => {
            i32::try_from(typed_i128_required(value, logical)?)
                .map_err(|_| format!("{logical:?} projection value is out of range"))?
                as u32 as u64
        }
        CoveLogicalType::Int64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos
        | CoveLogicalType::Decimal64 => i64::try_from(typed_i128_required(value, logical)?)
            .map_err(|_| format!("{logical:?} projection value is out of range"))?
            as u64,
        CoveLogicalType::UInt8 => u8::try_from(typed_i128_required(value, logical)?)
            .map_err(|_| "uint8 projection value is out of range".to_string())?
            as u64,
        CoveLogicalType::UInt16 => u16::try_from(typed_i128_required(value, logical)?)
            .map_err(|_| "uint16 projection value is out of range".to_string())?
            as u64,
        CoveLogicalType::UInt32 => u32::try_from(typed_i128_required(value, logical)?)
            .map_err(|_| "uint32 projection value is out of range".to_string())?
            as u64,
        CoveLogicalType::UInt64 => u64::try_from(typed_i128_required(value, logical)?)
            .map_err(|_| "uint64 projection value is out of range".to_string())?,
        CoveLogicalType::Float32 => (typed_f64_value(value)?
            .ok_or_else(|| "float32 projection value is null".to_string())?
            as f32)
            .to_bits() as u64,
        CoveLogicalType::Float64 => typed_f64_value(value)?
            .ok_or_else(|| "float64 projection value is null".to_string())?
            .to_bits(),
        _ => {
            return Err(format!(
                "logical type {logical:?} is not NumCode-backed in projection output"
            ))
        }
    })
}

fn projection_fixed_bytes(logical: CoveLogicalType, value: &Value) -> Result<Vec<u8>, String> {
    match logical {
        CoveLogicalType::Decimal128 => {
            Ok(typed_i128_required(value, logical)?.to_le_bytes().to_vec())
        }
        CoveLogicalType::Uuid => typed_uuid_value(value)?
            .map(|value| value.to_vec())
            .ok_or_else(|| "uuid projection value is null".to_string()),
        _ => Err(format!(
            "logical type {logical:?} is not fixed-width projection output"
        )),
    }
}

fn typed_i128_required(value: &Value, logical: CoveLogicalType) -> Result<i128, String> {
    typed_i128_value(value, logical)?.ok_or_else(|| format!("{logical:?} projection value is null"))
}

fn typed_i128_value(value: &Value, logical: CoveLogicalType) -> Result<Option<i128>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let parsed = match value {
        Value::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(|value| value as i128))
            .ok_or_else(|| format!("{logical:?} projection value must be an integer"))?,
        Value::String(text) => text
            .parse::<i128>()
            .map_err(|_| format!("{logical:?} projection value must be an integer"))?,
        Value::Bool(value) if logical == CoveLogicalType::Bool => i128::from(*value),
        _ => {
            return Err(format!(
                "{logical:?} projection value has incompatible JSON type"
            ))
        }
    };
    Ok(Some(parsed))
}

fn typed_f64_value(value: &Value) -> Result<Option<f64>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let parsed = match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| "floating projection value is not finite JSON number".to_string())?,
        Value::String(text) => text
            .parse::<f64>()
            .map_err(|_| "floating projection value must be numeric".to_string())?,
        _ => return Err("floating projection value has incompatible JSON type".into()),
    };
    Ok(Some(parsed))
}

fn typed_bool_value(value: &Value) -> Result<Option<bool>, String> {
    if value.is_null() {
        return Ok(None);
    }
    match value {
        Value::Bool(value) => Ok(Some(*value)),
        Value::String(text) if text.eq_ignore_ascii_case("true") => Ok(Some(true)),
        Value::String(text) if text.eq_ignore_ascii_case("false") => Ok(Some(false)),
        _ => Err("bool projection value must be boolean or true/false string".into()),
    }
}

fn typed_string_value(value: &Value, logical: CoveLogicalType) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    match value {
        Value::String(value) => Ok(Some(value.clone())),
        Value::Bool(_) | Value::Number(_) if logical == CoveLogicalType::Utf8 => {
            Ok(json_value_to_output_string(value))
        }
        Value::Array(_) | Value::Object(_) if logical == CoveLogicalType::Utf8 => {
            Ok(Some(value.to_string()))
        }
        _ => Err(format!(
            "{logical:?} projection value cannot be encoded as string"
        )),
    }
}

fn typed_bytes_value(value: &Value, logical: CoveLogicalType) -> Result<Option<Vec<u8>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    match logical {
        CoveLogicalType::Utf8 => Ok(typed_string_value(value, logical)?.map(String::into_bytes)),
        CoveLogicalType::Json => serde_json::to_vec(value)
            .map(Some)
            .map_err(|err| format!("cannot encode projection JSON value: {err}")),
        CoveLogicalType::Binary => match value {
            Value::String(text) => Ok(Some(text.as_bytes().to_vec())),
            Value::Array(values) => values
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or_else(|| "binary projection array values must be u8".to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            _ => Err("binary projection value must be string or u8 array".into()),
        },
        other => Err(format!("{other:?} projection value is not byte-backed")),
    }
}

fn typed_uuid_value(value: &Value) -> Result<Option<[u8; 16]>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .ok_or_else(|| "uuid projection value must be a 32-character hex string".to_string())?;
    hex_decode_16(text).map(Some)
}

pub(super) fn encode_sql_projection(tables: &[ProjectedTable]) -> Result<Vec<u8>, String> {
    let mut out = String::new();
    for table in tables {
        out.push_str(&format!(
            "CREATE TABLE {} (\n",
            quote_sql_identifier(&table.output_table)
        ));
        for (index, column) in table.columns.iter().enumerate() {
            let suffix = if index + 1 == table.columns.len() {
                "\n"
            } else {
                ",\n"
            };
            out.push_str(&format!(
                "  {} {}{}",
                quote_sql_identifier(&column.name),
                sql_type_name(column.logical)?,
                suffix
            ));
        }
        out.push_str(");\n");
        for row in &table.rows {
            let values = table
                .columns
                .iter()
                .map(|column| {
                    let value = row.get(&column.name).unwrap_or(&Value::Null);
                    sql_literal(value, column.logical)
                })
                .collect::<Result<Vec<_>, _>>()?;
            out.push_str(&format!(
                "INSERT INTO {} ({}) VALUES ({});\n",
                quote_sql_identifier(&table.output_table),
                table
                    .columns
                    .iter()
                    .map(|column| quote_sql_identifier(&column.name))
                    .collect::<Vec<_>>()
                    .join(", "),
                values.join(", ")
            ));
        }
    }
    Ok(out.into_bytes())
}

fn quote_sql_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn sql_type_name(logical: CoveLogicalType) -> Result<&'static str, String> {
    match logical {
        CoveLogicalType::Bool => Ok("BOOLEAN"),
        CoveLogicalType::Int8 => Ok("TINYINT"),
        CoveLogicalType::Int16 => Ok("SMALLINT"),
        CoveLogicalType::Int32 | CoveLogicalType::DateDays => Ok("INTEGER"),
        CoveLogicalType::Int64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => Ok("BIGINT"),
        CoveLogicalType::UInt8 => Ok("UTINYINT"),
        CoveLogicalType::UInt16 => Ok("USMALLINT"),
        CoveLogicalType::UInt32 => Ok("UINTEGER"),
        CoveLogicalType::UInt64 => Ok("UBIGINT"),
        CoveLogicalType::Float32 => Ok("REAL"),
        CoveLogicalType::Float64 => Ok("DOUBLE"),
        CoveLogicalType::Decimal64 => Ok("DECIMAL(18,0)"),
        CoveLogicalType::Decimal128 => Ok("DECIMAL(38,0)"),
        CoveLogicalType::Utf8 | CoveLogicalType::Uuid => Ok("TEXT"),
        CoveLogicalType::Binary => Ok("BLOB"),
        CoveLogicalType::Json => Ok("JSON"),
        CoveLogicalType::Null
        | CoveLogicalType::List
        | CoveLogicalType::Struct
        | CoveLogicalType::Map => Err(format!(
            "projection logical type {logical:?} has no SQL output type"
        )),
        _ => Err(format!("unknown projection logical type {logical:?}")),
    }
}

fn sql_literal(value: &Value, logical: CoveLogicalType) -> Result<String, String> {
    if value.is_null() {
        return Ok("NULL".into());
    }
    match logical {
        CoveLogicalType::Bool => Ok(if typed_bool_value(value)?.unwrap_or(false) {
            "TRUE".into()
        } else {
            "FALSE".into()
        }),
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64
        | CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::Decimal128
        | CoveLogicalType::DateDays
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => Ok(typed_i128_required(value, logical)?.to_string()),
        CoveLogicalType::Float32 | CoveLogicalType::Float64 => {
            let value =
                typed_f64_value(value)?.ok_or_else(|| "float SQL value is null".to_string())?;
            if value.is_finite() {
                Ok(value.to_string())
            } else {
                Err("non-finite float projection values cannot be emitted as SQL literals".into())
            }
        }
        CoveLogicalType::Utf8 | CoveLogicalType::Uuid => Ok(quote_sql_string(
            &typed_string_value(value, CoveLogicalType::Utf8)?.unwrap_or_default(),
        )),
        CoveLogicalType::Json => Ok(quote_sql_string(&value.to_string())),
        CoveLogicalType::Binary => {
            let bytes = typed_bytes_value(value, logical)?.unwrap_or_default();
            Ok(format!("X'{}'", hex_encode(&bytes)))
        }
        CoveLogicalType::Null
        | CoveLogicalType::List
        | CoveLogicalType::Struct
        | CoveLogicalType::Map => Err(format!(
            "projection logical type {logical:?} cannot be emitted as SQL"
        )),
        _ => Err(format!("unknown projection logical type {logical:?}")),
    }
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn json_value_to_output_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}

pub(super) fn json_value_to_output_cow(value: &Value) -> Option<Cow<'_, str>> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(Cow::Borrowed(value.as_str())),
        Value::Bool(value) => Some(Cow::Owned(value.to_string())),
        Value::Number(value) => Some(Cow::Owned(value.to_string())),
        Value::Array(_) | Value::Object(_) => Some(Cow::Owned(value.to_string())),
    }
}
