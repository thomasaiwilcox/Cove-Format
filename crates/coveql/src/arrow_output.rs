use std::{collections::BTreeMap, fmt};

use arrow_array::{
    Array, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Decimal64Array,
    FixedSizeBinaryArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, RecordBatch, StringArray, TimestampMicrosecondArray, TimestampNanosecondArray,
    UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use serde_json::{json, Value};

use crate::materialized::{hex, ExecutionRow, MaterializedProjectionRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArrowOutputError {
    UnsupportedColumnType { data_type: String },
}

impl fmt::Display for ArrowOutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedColumnType { data_type } => {
                write!(f, "unsupported Arrow output column type {data_type}")
            }
        }
    }
}

impl std::error::Error for ArrowOutputError {}

pub(crate) fn execution_rows_to_json(rows: &[ExecutionRow]) -> Vec<Value> {
    rows.iter().map(ExecutionRow::to_json).collect()
}

pub(crate) fn record_batches_to_projection_rows(
    projection_id: &str,
    batches: &[RecordBatch],
) -> Result<Vec<MaterializedProjectionRow>, ArrowOutputError> {
    let mut out = Vec::new();
    for batch in batches {
        for row_index in 0..batch.num_rows() {
            let mut values = BTreeMap::new();
            for (column_index, field) in batch.schema().fields().iter().enumerate() {
                let value = array_value(batch.column(column_index).as_ref(), row_index)?;
                values.insert(field.name().clone(), value);
            }
            out.push(MaterializedProjectionRow {
                projection_id: projection_id.into(),
                values,
            });
        }
    }
    Ok(out)
}

pub(crate) fn record_batches_to_json_rows(
    batches: &[RecordBatch],
) -> Result<Vec<Value>, ArrowOutputError> {
    let mut out = Vec::new();
    for batch in batches {
        for row_index in 0..batch.num_rows() {
            let mut row = serde_json::Map::new();
            for (column_index, field) in batch.schema().fields().iter().enumerate() {
                row.insert(
                    field.name().clone(),
                    array_value(batch.column(column_index).as_ref(), row_index)?,
                );
            }
            out.push(Value::Object(row));
        }
    }
    Ok(out)
}

fn array_value(array: &dyn Array, row: usize) -> Result<Value, ArrowOutputError> {
    if array.is_null(row) {
        return Ok(Value::Null);
    }
    macro_rules! primitive {
        ($ty:ty) => {
            if let Some(array) = array.as_any().downcast_ref::<$ty>() {
                return Ok(json!(array.value(row)));
            }
        };
    }
    primitive!(BooleanArray);
    primitive!(Int8Array);
    primitive!(Int16Array);
    primitive!(Int32Array);
    primitive!(Int64Array);
    primitive!(UInt8Array);
    primitive!(UInt16Array);
    primitive!(UInt32Array);
    primitive!(UInt64Array);
    primitive!(Float32Array);
    primitive!(Float64Array);
    primitive!(Date32Array);
    primitive!(TimestampMicrosecondArray);
    primitive!(TimestampNanosecondArray);
    primitive!(Decimal64Array);
    primitive!(Decimal128Array);
    if let Some(array) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(Value::String(array.value(row).to_string()));
    }
    if let Some(array) = array.as_any().downcast_ref::<BinaryArray>() {
        return Ok(Value::String(hex(array.value(row))));
    }
    if let Some(array) = array.as_any().downcast_ref::<FixedSizeBinaryArray>() {
        return Ok(Value::String(hex(array.value(row))));
    }
    Err(ArrowOutputError::UnsupportedColumnType {
        data_type: format!("{:?}", array.data_type()),
    })
}
