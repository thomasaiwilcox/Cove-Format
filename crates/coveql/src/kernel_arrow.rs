use std::{collections::BTreeSet, fmt, sync::Arc};

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, UInt64Array,
};
use arrow_schema::{ArrowError, Field, Schema};
use serde_json::Value;

use crate::{
    expr_eval::{eval_expr, expr_logical_type, EvalContext},
    materialized::ExecutionRow,
    PlannedQuery, ResolvedExpr, ResolvedSelectItem,
};

#[derive(Debug)]
pub(crate) enum KernelArrowError {
    ExpressionEvaluation(String),
    RecordBatch(ArrowError),
}

impl fmt::Display for KernelArrowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpressionEvaluation(message) => f.write_str(message),
            Self::RecordBatch(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for KernelArrowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ExpressionEvaluation(_) => None,
            Self::RecordBatch(err) => Some(err),
        }
    }
}

impl From<ArrowError> for KernelArrowError {
    fn from(err: ArrowError) -> Self {
        Self::RecordBatch(err)
    }
}

pub(crate) fn execution_rows_to_owned_record_batch(
    rows: &[ExecutionRow],
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<RecordBatch, KernelArrowError> {
    if let Some(select) = &planned.resolved.method_chain.select {
        return selected_execution_rows_to_owned_record_batch(rows, select, context);
    }
    let json_rows = rows.iter().map(ExecutionRow::to_json).collect::<Vec<_>>();
    json_rows_to_owned_record_batch_for_plan(&json_rows, planned)
}

fn selected_execution_rows_to_owned_record_batch(
    rows: &[ExecutionRow],
    select: &[ResolvedSelectItem],
    context: &EvalContext<'_>,
) -> Result<RecordBatch, KernelArrowError> {
    let mut fields = Vec::with_capacity(select.len());
    let mut arrays = Vec::with_capacity(select.len());
    for item in select {
        let name = item
            .alias
            .clone()
            .unwrap_or_else(|| output_name_for_expr(&item.expr));
        let array = if rows.is_empty() {
            empty_array_for_expr(&item.expr)
        } else {
            let values = rows
                .iter()
                .map(|row| {
                    eval_expr(&item.expr, row, context)
                        .map_err(|err| KernelArrowError::ExpressionEvaluation(err.message))
                })
                .collect::<Result<Vec<_>, _>>()?;
            values_to_array(&values)?
        };
        fields.push(Field::new(&name, array.data_type().clone(), true));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, arrays).map_err(Into::into)
}

pub(crate) fn json_rows_to_owned_record_batch_for_plan(
    rows: &[Value],
    planned: &PlannedQuery,
) -> Result<RecordBatch, KernelArrowError> {
    if rows.is_empty() {
        if let Some(select) = &planned.resolved.method_chain.select {
            return empty_selected_record_batch(select);
        }
    }
    json_rows_to_owned_record_batch(rows)
}

pub(crate) fn empty_selected_record_batch(
    select: &[ResolvedSelectItem],
) -> Result<RecordBatch, KernelArrowError> {
    let mut fields = Vec::with_capacity(select.len());
    let mut arrays = Vec::with_capacity(select.len());
    for item in select {
        let name = item
            .alias
            .clone()
            .unwrap_or_else(|| output_name_for_expr(&item.expr));
        let array = empty_array_for_expr(&item.expr);
        fields.push(Field::new(&name, array.data_type().clone(), true));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, arrays).map_err(Into::into)
}

fn empty_array_for_expr(expr: &ResolvedExpr) -> ArrayRef {
    match expr_logical_type(expr) {
        Some("bool" | "boolean") => Arc::new(BooleanArray::from(Vec::<Option<bool>>::new())),
        Some("uint8" | "uint16" | "uint32" | "uint64" | "usize" | "count" | "distinct_count") => {
            Arc::new(UInt64Array::from(Vec::<Option<u64>>::new()))
        }
        Some("float32" | "float64") => Arc::new(Float64Array::from(Vec::<Option<f64>>::new())),
        Some(
            "int8" | "int16" | "int32" | "int64" | "date_days" | "timestamp_micros"
            | "timestamp_nanos",
        ) => Arc::new(Int64Array::from(Vec::<Option<i64>>::new())),
        _ => Arc::new(StringArray::from(Vec::<Option<String>>::new())),
    }
}

pub(crate) fn json_rows_to_owned_record_batch(
    rows: &[Value],
) -> Result<RecordBatch, KernelArrowError> {
    let fields = ordered_fields(rows);
    let arrays = fields
        .iter()
        .map(|field| build_array(rows, field))
        .collect::<Result<Vec<_>, _>>()?;
    let schema = Arc::new(Schema::new(
        fields
            .iter()
            .zip(arrays.iter())
            .map(|(name, array)| Field::new(name, array.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    RecordBatch::try_new(schema, arrays).map_err(Into::into)
}

fn ordered_fields(rows: &[Value]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for row in rows {
        let Some(object) = row.as_object() else {
            if seen.insert("value".to_string()) {
                out.push("value".to_string());
            }
            continue;
        };
        for key in object.keys() {
            if seen.insert(key.clone()) {
                out.push(key.clone());
            }
        }
    }
    out
}

fn build_array(rows: &[Value], field: &str) -> Result<ArrayRef, KernelArrowError> {
    let values = rows
        .iter()
        .map(|row| {
            row.as_object()
                .and_then(|object| object.get(field))
                .unwrap_or(&Value::Null)
                .clone()
        })
        .collect::<Vec<_>>();

    values_to_array(&values)
}

fn values_to_array(values: &[Value]) -> Result<ArrayRef, KernelArrowError> {
    if values
        .iter()
        .all(|value| value.is_null() || value.as_bool().is_some())
    {
        return Ok(Arc::new(BooleanArray::from(
            values
                .iter()
                .map(|value| value.as_bool())
                .collect::<Vec<_>>(),
        )));
    }
    if values
        .iter()
        .all(|value| value.is_null() || value.as_i64().is_some())
    {
        return Ok(Arc::new(Int64Array::from(
            values
                .iter()
                .map(|value| value.as_i64())
                .collect::<Vec<_>>(),
        )));
    }
    if values
        .iter()
        .all(|value| value.is_null() || value.as_u64().is_some())
    {
        return Ok(Arc::new(UInt64Array::from(
            values
                .iter()
                .map(|value| value.as_u64())
                .collect::<Vec<_>>(),
        )));
    }
    if values
        .iter()
        .all(|value| value.is_null() || value.as_f64().is_some())
    {
        return Ok(Arc::new(Float64Array::from(
            values
                .iter()
                .map(|value| value.as_f64())
                .collect::<Vec<_>>(),
        )));
    }

    Ok(Arc::new(StringArray::from(
        values
            .iter()
            .map(|value| {
                if value.is_null() {
                    None
                } else if let Some(value) = value.as_str() {
                    Some(value.to_string())
                } else {
                    serde_json::to_string(value).ok()
                }
            })
            .collect::<Vec<_>>(),
    )))
}

fn output_name_for_expr(expr: &ResolvedExpr) -> String {
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::FunctionCall { function_id, .. } => function_id.clone(),
        ResolvedExpr::AggregateCall { name, .. } => format!("{name:?}").to_lowercase(),
        ResolvedExpr::Literal(_) => "literal".into(),
        ResolvedExpr::Association(association) => association.type_name.clone(),
        ResolvedExpr::Evidence(_) => "evidence".into(),
        ResolvedExpr::TableExists(_) => "exists".into(),
        ResolvedExpr::Conditional { .. } => "if".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::DataType;
    use serde_json::json;

    #[test]
    fn json_rows_build_typed_owned_arrow_columns() {
        let batch = json_rows_to_owned_record_batch(&[
            json!({ "id": 1_u64, "active": true, "name": "a" }),
            json!({ "id": 2_u64, "active": false, "name": "b" }),
        ])
        .unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
        assert_eq!(batch.schema().field(1).data_type(), &DataType::Boolean);
        assert_eq!(batch.schema().field(2).data_type(), &DataType::Utf8);
    }
}
