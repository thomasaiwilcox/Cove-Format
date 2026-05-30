use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{DataType, SchemaRef};
use cove_map::{ProjectionFilter, ProjectionFilterLiteral, ProjectionFilterOp};
use datafusion::{
    common::{DataFusionError, Result, ScalarValue},
    logical_expr::{Expr, Operator},
};

pub(super) fn projected_schema(
    schema: &SchemaRef,
    projection: Option<&[usize]>,
) -> Result<SchemaRef> {
    match projection {
        Some(projection) => schema.project(projection).map(Arc::new).map_err(|err| {
            DataFusionError::Execution(format!(
                "cannot project Arrow schema for mapped COVE-O provider: {err}"
            ))
        }),
        None => Ok(Arc::clone(schema)),
    }
}

pub(super) fn project_batch_columns(
    batch: RecordBatch,
    output_columns: Option<&[String]>,
) -> Result<RecordBatch> {
    let Some(output_columns) = output_columns else {
        return Ok(batch);
    };
    let schema = batch.schema();
    let indices = output_columns
        .iter()
        .map(|column| {
            schema.index_of(column).map_err(|err| {
                DataFusionError::Execution(format!(
                    "cannot project mapped batch column '{column}': {err}"
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    batch.project(&indices).map_err(|err| {
        DataFusionError::Execution(format!("cannot project mapped batch output columns: {err}"))
    })
}

pub(super) fn merged_projection_columns(
    output_columns: Option<&[String]>,
    filters: &[ProjectionFilter],
) -> Option<Vec<String>> {
    let mut merged = output_columns?.to_vec();
    for filter in filters {
        let column = filter_column_name(filter);
        if !merged.iter().any(|existing| existing == column) {
            merged.push(column.to_string());
        }
    }
    Some(merged)
}

pub(super) fn classify_projection_filters(
    schema: &SchemaRef,
    filters: &[Expr],
) -> Result<Vec<ProjectionFilter>> {
    let mut pushed = Vec::new();
    for filter in filters {
        let Some(mut lowered) = classify_projection_filter(schema, filter) else {
            continue;
        };
        pushed.append(&mut lowered);
    }
    Ok(pushed)
}

pub(super) fn classify_projection_filter(
    schema: &SchemaRef,
    expr: &Expr,
) -> Option<Vec<ProjectionFilter>> {
    match expr {
        Expr::BinaryExpr(binary) if binary.op == Operator::And => {
            let mut filters = classify_projection_filter(schema, &binary.left)?;
            filters.extend(classify_projection_filter(schema, &binary.right)?);
            Some(filters)
        }
        Expr::BinaryExpr(binary) => {
            classify_projection_binary(schema, &binary.left, binary.op, &binary.right)
        }
        Expr::Between(between) if !between.negated => {
            let column = filter_column_expr(schema, &between.expr)?;
            let low = projection_filter_literal(&between.low)?;
            let high = projection_filter_literal(&between.high)?;
            Some(vec![
                ProjectionFilter::Compare {
                    column: column.clone(),
                    op: ProjectionFilterOp::GtEq,
                    literal: low,
                },
                ProjectionFilter::Compare {
                    column,
                    op: ProjectionFilterOp::LtEq,
                    literal: high,
                },
            ])
        }
        Expr::InList(in_list) if !in_list.negated => {
            let column = filter_column_expr(schema, &in_list.expr)?;
            let literals = in_list
                .list
                .iter()
                .map(projection_filter_literal)
                .collect::<Option<Vec<_>>>()?;
            Some(vec![ProjectionFilter::InList { column, literals }])
        }
        Expr::IsNull(input) => {
            let column = filter_column_expr(schema, input)?;
            Some(vec![ProjectionFilter::IsNull {
                column,
                negated: false,
            }])
        }
        Expr::IsNotNull(input) => {
            let column = filter_column_expr(schema, input)?;
            Some(vec![ProjectionFilter::IsNull {
                column,
                negated: true,
            }])
        }
        _ => None,
    }
}

fn filter_column_name(filter: &ProjectionFilter) -> &str {
    match filter {
        ProjectionFilter::Compare { column, .. }
        | ProjectionFilter::InList { column, .. }
        | ProjectionFilter::IsNull { column, .. } => column,
    }
}

fn classify_projection_binary(
    schema: &SchemaRef,
    left: &Expr,
    op: Operator,
    right: &Expr,
) -> Option<Vec<ProjectionFilter>> {
    let filter_op = projection_filter_op(op)?;
    if let (Some(column), Some(literal)) = (
        filter_column_expr(schema, left),
        projection_filter_literal(right),
    ) {
        return Some(vec![ProjectionFilter::Compare {
            column,
            op: filter_op,
            literal,
        }]);
    }
    if let (Some(literal), Some(column)) = (
        projection_filter_literal(left),
        filter_column_expr(schema, right),
    ) {
        return Some(vec![ProjectionFilter::Compare {
            column,
            op: invert_projection_filter_op(filter_op),
            literal,
        }]);
    }
    None
}

fn projection_filter_op(op: Operator) -> Option<ProjectionFilterOp> {
    match op {
        Operator::Eq => Some(ProjectionFilterOp::Eq),
        Operator::Lt => Some(ProjectionFilterOp::Lt),
        Operator::LtEq => Some(ProjectionFilterOp::LtEq),
        Operator::Gt => Some(ProjectionFilterOp::Gt),
        Operator::GtEq => Some(ProjectionFilterOp::GtEq),
        _ => None,
    }
}

fn invert_projection_filter_op(op: ProjectionFilterOp) -> ProjectionFilterOp {
    match op {
        ProjectionFilterOp::Eq => ProjectionFilterOp::Eq,
        ProjectionFilterOp::Lt => ProjectionFilterOp::Gt,
        ProjectionFilterOp::LtEq => ProjectionFilterOp::GtEq,
        ProjectionFilterOp::Gt => ProjectionFilterOp::Lt,
        ProjectionFilterOp::GtEq => ProjectionFilterOp::LtEq,
    }
}

fn filter_column_expr(schema: &SchemaRef, expr: &Expr) -> Option<String> {
    let Expr::Column(column) = expr else {
        return None;
    };
    let field = schema.field_with_name(&column.name).ok()?;
    projection_filterable_type(field.data_type()).then(|| column.name.clone())
}

fn projection_filterable_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Boolean
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Date32
            | DataType::Timestamp(_, _)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{Field, Schema};
    use datafusion::common::Column;

    fn schema_ref(fields: Vec<Field>) -> SchemaRef {
        Arc::new(Schema::new(fields))
    }

    #[test]
    fn float_columns_are_not_exact_projection_pushdown() {
        let schema = schema_ref(vec![Field::new("score", DataType::Float64, false)]);
        let filter = Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
            Box::new(Expr::Column(Column::from_name("score"))),
            Operator::Eq,
            Box::new(Expr::Literal(ScalarValue::Float64(Some(0.0)), None)),
        ));

        assert!(classify_projection_filter(&schema, &filter).is_none());
        assert!(classify_projection_filters(&schema, &[filter])
            .unwrap()
            .is_empty());
    }
}

fn projection_filter_literal(expr: &Expr) -> Option<ProjectionFilterLiteral> {
    let Expr::Literal(value, _) = expr else {
        return None;
    };
    match value {
        ScalarValue::Null => Some(ProjectionFilterLiteral::Null),
        ScalarValue::Boolean(Some(value)) => Some(ProjectionFilterLiteral::Boolean(*value)),
        ScalarValue::Int8(Some(value)) => Some(ProjectionFilterLiteral::Int64(i64::from(*value))),
        ScalarValue::Int16(Some(value)) => Some(ProjectionFilterLiteral::Int64(i64::from(*value))),
        ScalarValue::Int32(Some(value)) => Some(ProjectionFilterLiteral::Int64(i64::from(*value))),
        ScalarValue::Int64(Some(value)) => Some(ProjectionFilterLiteral::Int64(*value)),
        ScalarValue::UInt8(Some(value)) => Some(ProjectionFilterLiteral::UInt64(u64::from(*value))),
        ScalarValue::UInt16(Some(value)) => {
            Some(ProjectionFilterLiteral::UInt64(u64::from(*value)))
        }
        ScalarValue::UInt32(Some(value)) => {
            Some(ProjectionFilterLiteral::UInt64(u64::from(*value)))
        }
        ScalarValue::UInt64(Some(value)) => Some(ProjectionFilterLiteral::UInt64(*value)),
        ScalarValue::Float32(Some(value)) => {
            Some(ProjectionFilterLiteral::Float64(f64::from(*value)))
        }
        ScalarValue::Float64(Some(value)) => Some(ProjectionFilterLiteral::Float64(*value)),
        ScalarValue::Date32(Some(value)) => Some(ProjectionFilterLiteral::Int64(i64::from(*value))),
        ScalarValue::TimestampMicrosecond(Some(value), None) => {
            Some(ProjectionFilterLiteral::Int64(*value))
        }
        ScalarValue::TimestampNanosecond(Some(value), None) => {
            Some(ProjectionFilterLiteral::Int64(*value))
        }
        ScalarValue::Utf8(Some(value))
        | ScalarValue::Utf8View(Some(value))
        | ScalarValue::LargeUtf8(Some(value)) => Some(ProjectionFilterLiteral::Utf8(value.clone())),
        ScalarValue::Dictionary(_, value) => {
            projection_filter_literal(&Expr::Literal((**value).clone(), None))
        }
        _ => None,
    }
}
