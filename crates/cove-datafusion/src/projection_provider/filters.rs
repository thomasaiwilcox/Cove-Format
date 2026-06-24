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

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionFilterClassificationReport {
    pub pushed_filters: Vec<ProjectionFilter>,
    pub residual_filters: Vec<String>,
}

impl ProjectionFilterClassificationReport {
    pub fn all_supported(&self) -> bool {
        self.residual_filters.is_empty()
    }
}

pub fn classify_projection_filters_report(
    schema: &SchemaRef,
    filters: &[Expr],
) -> Result<ProjectionFilterClassificationReport> {
    let mut pushed_filters = Vec::new();
    let mut residual_filters = Vec::new();
    for filter in filters {
        if let Some(mut lowered) = classify_projection_filter(schema, filter) {
            pushed_filters.append(&mut lowered);
        } else {
            residual_filters.push(filter.to_string());
        }
    }
    Ok(ProjectionFilterClassificationReport {
        pushed_filters,
        residual_filters,
    })
}

pub(super) fn classify_projection_filters(
    schema: &SchemaRef,
    filters: &[Expr],
) -> Result<Vec<ProjectionFilter>> {
    Ok(classify_projection_filters_report(schema, filters)?.pushed_filters)
}

pub fn classify_projection_filter(
    schema: &SchemaRef,
    expr: &Expr,
) -> Option<Vec<ProjectionFilter>> {
    match expr {
        Expr::Column(_) => {
            let column = filter_bool_column_expr(schema, expr)?;
            Some(vec![ProjectionFilter::Compare {
                column,
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(true),
            }])
        }
        Expr::Not(input) => classify_projection_not(schema, input),
        Expr::IsTrue(input) => {
            let column = filter_bool_column_expr(schema, input)?;
            Some(vec![ProjectionFilter::Compare {
                column,
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(true),
            }])
        }
        Expr::IsFalse(input) => {
            let column = filter_bool_column_expr(schema, input)?;
            Some(vec![ProjectionFilter::Compare {
                column,
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(false),
            }])
        }
        Expr::BinaryExpr(binary) if binary.op == Operator::And => {
            let mut filters = classify_projection_filter(schema, &binary.left)?;
            filters.extend(classify_projection_filter(schema, &binary.right)?);
            Some(filters)
        }
        Expr::BinaryExpr(binary) if binary.op == Operator::Or => {
            classify_same_column_equality_or(schema, &binary.left, &binary.right)
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
            classify_projection_in_list(schema, &in_list.expr, &in_list.list, false)
        }
        Expr::InList(in_list) => {
            classify_projection_in_list(schema, &in_list.expr, &in_list.list, true)
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

fn classify_projection_not(schema: &SchemaRef, input: &Expr) -> Option<Vec<ProjectionFilter>> {
    if let Some(column) = filter_bool_column_expr(schema, input) {
        return Some(vec![ProjectionFilter::Compare {
            column,
            op: ProjectionFilterOp::Eq,
            literal: ProjectionFilterLiteral::Boolean(false),
        }]);
    }
    match input {
        Expr::Not(inner) => classify_projection_filter(schema, inner),
        Expr::BinaryExpr(binary) => classify_projection_binary_with_op(
            schema,
            &binary.left,
            negated_projection_filter_op(binary.op)?,
            &binary.right,
        ),
        Expr::InList(in_list) => {
            classify_projection_in_list(schema, &in_list.expr, &in_list.list, !in_list.negated)
        }
        Expr::IsNull(input) => {
            let column = filter_column_expr(schema, input)?;
            Some(vec![ProjectionFilter::IsNull {
                column,
                negated: true,
            }])
        }
        Expr::IsNotNull(input) => {
            let column = filter_column_expr(schema, input)?;
            Some(vec![ProjectionFilter::IsNull {
                column,
                negated: false,
            }])
        }
        _ => None,
    }
}

fn classify_projection_in_list(
    schema: &SchemaRef,
    expr: &Expr,
    list: &[Expr],
    negated: bool,
) -> Option<Vec<ProjectionFilter>> {
    let column = filter_column_expr(schema, expr)?;
    let literals = list
        .iter()
        .map(projection_filter_literal)
        .collect::<Option<Vec<_>>>()?;
    if !negated {
        return Some(vec![ProjectionFilter::InList { column, literals }]);
    }
    if literals
        .iter()
        .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
    {
        return None;
    }
    Some(
        literals
            .into_iter()
            .map(|literal| ProjectionFilter::Compare {
                column: column.clone(),
                op: ProjectionFilterOp::Ne,
                literal,
            })
            .collect(),
    )
}

fn classify_same_column_equality_or(
    schema: &SchemaRef,
    left: &Expr,
    right: &Expr,
) -> Option<Vec<ProjectionFilter>> {
    let left = single_equality_set_filter(schema, left)?;
    let right = single_equality_set_filter(schema, right)?;
    if left.column != right.column {
        return None;
    }
    let mut literals = left.literals;
    for literal in right.literals {
        if !literals.contains(&literal) {
            literals.push(literal);
        }
    }
    Some(vec![ProjectionFilter::InList {
        column: left.column,
        literals,
    }])
}

struct EqualitySetFilter {
    column: String,
    literals: Vec<ProjectionFilterLiteral>,
}

fn single_equality_set_filter(schema: &SchemaRef, expr: &Expr) -> Option<EqualitySetFilter> {
    let mut filters = classify_projection_filter(schema, expr)?;
    if filters.len() != 1 {
        return None;
    }
    match filters.pop()? {
        ProjectionFilter::Compare {
            column,
            op: ProjectionFilterOp::Eq,
            literal,
        } => Some(EqualitySetFilter {
            column,
            literals: vec![literal],
        }),
        ProjectionFilter::InList { column, literals } if !literals.is_empty() => {
            Some(EqualitySetFilter { column, literals })
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
    classify_projection_binary_with_op(schema, left, filter_op, right)
}

fn classify_projection_binary_with_op(
    schema: &SchemaRef,
    left: &Expr,
    filter_op: ProjectionFilterOp,
    right: &Expr,
) -> Option<Vec<ProjectionFilter>> {
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
        Operator::NotEq => Some(ProjectionFilterOp::Ne),
        Operator::Lt => Some(ProjectionFilterOp::Lt),
        Operator::LtEq => Some(ProjectionFilterOp::LtEq),
        Operator::Gt => Some(ProjectionFilterOp::Gt),
        Operator::GtEq => Some(ProjectionFilterOp::GtEq),
        _ => None,
    }
}

fn negated_projection_filter_op(op: Operator) -> Option<ProjectionFilterOp> {
    match op {
        Operator::Eq => Some(ProjectionFilterOp::Ne),
        Operator::NotEq => Some(ProjectionFilterOp::Eq),
        Operator::Lt => Some(ProjectionFilterOp::GtEq),
        Operator::LtEq => Some(ProjectionFilterOp::Gt),
        Operator::Gt => Some(ProjectionFilterOp::LtEq),
        Operator::GtEq => Some(ProjectionFilterOp::Lt),
        _ => None,
    }
}

fn invert_projection_filter_op(op: ProjectionFilterOp) -> ProjectionFilterOp {
    match op {
        ProjectionFilterOp::Eq => ProjectionFilterOp::Eq,
        ProjectionFilterOp::Ne => ProjectionFilterOp::Ne,
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

fn filter_bool_column_expr(schema: &SchemaRef, expr: &Expr) -> Option<String> {
    let Expr::Column(column) = expr else {
        return None;
    };
    let field = schema.field_with_name(&column.name).ok()?;
    matches!(field.data_type(), DataType::Boolean).then(|| column.name.clone())
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
    use datafusion::{common::Column, logical_expr::expr::InList};

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

    #[test]
    fn bare_boolean_columns_lower_to_true_comparison() {
        let schema = schema_ref(vec![Field::new("active", DataType::Boolean, true)]);
        let filter = Expr::Column(Column::from_name("active"));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![ProjectionFilter::Compare {
                column: "active".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(true),
            }])
        );
    }

    #[test]
    fn negated_boolean_columns_lower_to_false_comparison() {
        let schema = schema_ref(vec![Field::new("active", DataType::Boolean, true)]);
        let filter = Expr::Not(Box::new(Expr::Column(Column::from_name("active"))));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![ProjectionFilter::Compare {
                column: "active".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(false),
            }])
        );
    }

    #[test]
    fn not_of_equality_lowers_to_ne_comparison() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::Not(Box::new(Expr::BinaryExpr(
            datafusion::logical_expr::BinaryExpr::new(
                Box::new(Expr::Column(Column::from_name("status"))),
                Operator::Eq,
                Box::new(Expr::Literal(
                    ScalarValue::Utf8(Some("closed".into())),
                    None,
                )),
            ),
        )));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![ProjectionFilter::Compare {
                column: "status".into(),
                op: ProjectionFilterOp::Ne,
                literal: ProjectionFilterLiteral::Utf8("closed".into()),
            }])
        );
    }

    #[test]
    fn not_of_in_list_lowers_to_ne_conjunction_without_nulls() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::Not(Box::new(Expr::InList(InList::new(
            Box::new(Expr::Column(Column::from_name("status"))),
            vec![
                Expr::Literal(ScalarValue::Utf8(Some("closed".into())), None),
                Expr::Literal(ScalarValue::Utf8(Some("pending".into())), None),
            ],
            false,
        ))));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![
                ProjectionFilter::Compare {
                    column: "status".into(),
                    op: ProjectionFilterOp::Ne,
                    literal: ProjectionFilterLiteral::Utf8("closed".into()),
                },
                ProjectionFilter::Compare {
                    column: "status".into(),
                    op: ProjectionFilterOp::Ne,
                    literal: ProjectionFilterLiteral::Utf8("pending".into()),
                },
            ])
        );
    }

    #[test]
    fn not_of_in_list_with_null_remains_residual() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::Not(Box::new(Expr::InList(InList::new(
            Box::new(Expr::Column(Column::from_name("status"))),
            vec![
                Expr::Literal(ScalarValue::Utf8(Some("closed".into())), None),
                Expr::Literal(ScalarValue::Null, None),
            ],
            false,
        ))));

        assert!(classify_projection_filter(&schema, &filter).is_none());
    }

    #[test]
    fn not_of_is_null_lowers_to_is_not_null() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::Not(Box::new(Expr::IsNull(Box::new(Expr::Column(
            Column::from_name("status"),
        )))));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![ProjectionFilter::IsNull {
                column: "status".into(),
                negated: true,
            }])
        );
    }

    #[test]
    fn not_equal_lowers_to_ne_comparison() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
            Box::new(Expr::Column(Column::from_name("status"))),
            Operator::NotEq,
            Box::new(Expr::Literal(
                ScalarValue::Utf8(Some("closed".into())),
                None,
            )),
        ));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![ProjectionFilter::Compare {
                column: "status".into(),
                op: ProjectionFilterOp::Ne,
                literal: ProjectionFilterLiteral::Utf8("closed".into()),
            }])
        );
    }

    #[test]
    fn negated_in_list_lowers_to_ne_conjunction() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::InList(InList::new(
            Box::new(Expr::Column(Column::from_name("status"))),
            vec![
                Expr::Literal(ScalarValue::Utf8(Some("closed".into())), None),
                Expr::Literal(ScalarValue::Utf8(Some("pending".into())), None),
            ],
            true,
        ));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![
                ProjectionFilter::Compare {
                    column: "status".into(),
                    op: ProjectionFilterOp::Ne,
                    literal: ProjectionFilterLiteral::Utf8("closed".into()),
                },
                ProjectionFilter::Compare {
                    column: "status".into(),
                    op: ProjectionFilterOp::Ne,
                    literal: ProjectionFilterLiteral::Utf8("pending".into()),
                },
            ])
        );
    }

    #[test]
    fn negated_in_list_with_null_remains_residual() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::InList(InList::new(
            Box::new(Expr::Column(Column::from_name("status"))),
            vec![
                Expr::Literal(ScalarValue::Utf8(Some("closed".into())), None),
                Expr::Literal(ScalarValue::Null, None),
            ],
            true,
        ));

        assert!(classify_projection_filter(&schema, &filter).is_none());
    }

    #[test]
    fn same_column_or_equalities_lower_to_in_list() {
        let schema = schema_ref(vec![Field::new("status", DataType::Utf8, true)]);
        let filter = Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
            Box::new(Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
                Box::new(Expr::Column(Column::from_name("status"))),
                Operator::Eq,
                Box::new(Expr::Literal(ScalarValue::Utf8(Some("open".into())), None)),
            ))),
            Operator::Or,
            Box::new(Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
                Box::new(Expr::Column(Column::from_name("status"))),
                Operator::Eq,
                Box::new(Expr::Literal(
                    ScalarValue::Utf8(Some("closed".into())),
                    None,
                )),
            ))),
        ));

        assert_eq!(
            classify_projection_filter(&schema, &filter),
            Some(vec![ProjectionFilter::InList {
                column: "status".into(),
                literals: vec![
                    ProjectionFilterLiteral::Utf8("open".into()),
                    ProjectionFilterLiteral::Utf8("closed".into())
                ],
            }])
        );
    }

    #[test]
    fn cross_column_or_equalities_remain_residual() {
        let schema = schema_ref(vec![
            Field::new("left", DataType::Boolean, true),
            Field::new("right", DataType::Boolean, true),
        ]);
        let filter = Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
            Box::new(Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
                Box::new(Expr::Column(Column::from_name("left"))),
                Operator::Eq,
                Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
            ))),
            Operator::Or,
            Box::new(Expr::BinaryExpr(datafusion::logical_expr::BinaryExpr::new(
                Box::new(Expr::Column(Column::from_name("right"))),
                Operator::Eq,
                Box::new(Expr::Literal(ScalarValue::Boolean(Some(false)), None)),
            ))),
        ));

        assert!(classify_projection_filter(&schema, &filter).is_none());
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
