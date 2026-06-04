use std::{cmp::Ordering, collections::BTreeSet};

use cove_core::collation::{CollationKind, Ordering3};
use serde_json::{json, Number, Value};

use crate::{
    association_opt::{
        association_valid_at, current_row_dataset_file_ordinal, current_row_goid,
        AssociationEdgeTable,
    },
    evidence_opt::EvidenceGrainIndex,
    materialized::{ExecutionRow, MaterializedAssociationRow, MaterializedEvidenceRow},
    AstCompareOp, ResolvedAssociationRoot, ResolvedEvidenceRoot, ResolvedExpr, ResolvedLiteral,
    ResolvedLiteralValue, ResolvedPath, ResolvedPredicate,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvalError {
    pub message: String,
}

impl EvalError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    True,
    False,
    Unknown,
}

pub(crate) struct EvalContext<'a> {
    association_edges: AssociationEdgeTable,
    evidence_index: EvidenceGrainIndex,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> EvalContext<'a> {
    pub(crate) fn for_plan(
        associations: &'a [MaterializedAssociationRow],
        evidence_rows: &'a [MaterializedEvidenceRow],
        planned: &crate::PlannedQuery,
    ) -> Self {
        Self::with_association_valid_at(associations, evidence_rows, association_valid_at(planned))
    }

    fn with_association_valid_at(
        associations: &'a [MaterializedAssociationRow],
        evidence_rows: &'a [MaterializedEvidenceRow],
        association_valid_at: Option<i64>,
    ) -> Self {
        Self {
            association_edges: AssociationEdgeTable::from_rows_with_association_valid_at(
                associations,
                association_valid_at,
            ),
            evidence_index: EvidenceGrainIndex::from_rows(evidence_rows),
            _marker: std::marker::PhantomData,
        }
    }

    pub(crate) fn association_count_for(
        &self,
        row: &ExecutionRow,
        association: &ResolvedAssociationRoot,
    ) -> usize {
        current_row_goid(row)
            .map(|goid| {
                self.association_edges.count_for_scoped(
                    current_row_dataset_file_ordinal(row),
                    goid,
                    association,
                )
            })
            .unwrap_or(0)
    }

    pub(crate) fn association_endpoint_keys_for(
        &self,
        row: &ExecutionRow,
        association: &ResolvedAssociationRoot,
    ) -> BTreeSet<String> {
        current_row_goid(row)
            .map(|goid| {
                self.association_edges.opposite_endpoints_for_scoped(
                    current_row_dataset_file_ordinal(row),
                    goid,
                    association,
                )
            })
            .unwrap_or_default()
    }

    pub(crate) fn evidence_count_for(
        &self,
        row: &ExecutionRow,
        evidence: &ResolvedEvidenceRoot,
    ) -> usize {
        self.evidence_index.count_for(row, evidence)
    }

    pub(crate) fn evidence_identity_keys_for(
        &self,
        row: &ExecutionRow,
        evidence: &ResolvedEvidenceRoot,
    ) -> BTreeSet<String> {
        self.evidence_index.identities_for(row, evidence)
    }
}

pub(crate) fn eval_predicate(
    predicate: &ResolvedPredicate,
    row: &ExecutionRow,
    context: &EvalContext<'_>,
) -> Result<bool, EvalError> {
    Ok(matches!(
        eval_predicate_truth(predicate, row, context)?,
        Truth::True
    ))
}

fn eval_predicate_truth(
    predicate: &ResolvedPredicate,
    row: &ExecutionRow,
    context: &EvalContext<'_>,
) -> Result<Truth, EvalError> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            let left_value = eval_expr(left, row, context)?;
            let right_value = eval_expr(right, row, context)?;
            Ok(compare_values_typed(
                left,
                &left_value,
                *op,
                right,
                &right_value,
            ))
        }
        ResolvedPredicate::InList { expr, values } => {
            let expr_value = eval_expr(expr, row, context)?;
            if expr_value.is_null() {
                return Ok(Truth::Unknown);
            }
            let logical_type = expr_logical_type(expr);
            let mut saw_null = false;
            for literal in values {
                let literal = literal_value(literal)?;
                if literal.is_null() {
                    saw_null = true;
                    continue;
                }
                if values_equal_typed(&expr_value, &literal, logical_type) {
                    return Ok(Truth::True);
                }
            }
            Ok(if saw_null {
                Truth::Unknown
            } else {
                Truth::False
            })
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let value = eval_expr(expr, row, context)?;
            Ok(if value.is_null() ^ *negated {
                Truth::True
            } else {
                Truth::False
            })
        }
        ResolvedPredicate::Exists(expr) => exists_expr(expr, row, context),
        ResolvedPredicate::BoolExpr(expr) => bool_truth(&eval_expr(expr, row, context)?),
        ResolvedPredicate::Not(inner) => Ok(match eval_predicate_truth(inner, row, context)? {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            Truth::Unknown => Truth::Unknown,
        }),
        ResolvedPredicate::And(parts) => {
            let mut saw_unknown = false;
            for part in parts {
                match eval_predicate_truth(part, row, context)? {
                    Truth::True => {}
                    Truth::False => return Ok(Truth::False),
                    Truth::Unknown => saw_unknown = true,
                }
            }
            Ok(if saw_unknown {
                Truth::Unknown
            } else {
                Truth::True
            })
        }
        ResolvedPredicate::Or(parts) => {
            let mut saw_unknown = false;
            for part in parts {
                match eval_predicate_truth(part, row, context)? {
                    Truth::True => return Ok(Truth::True),
                    Truth::False => {}
                    Truth::Unknown => saw_unknown = true,
                }
            }
            Ok(if saw_unknown {
                Truth::Unknown
            } else {
                Truth::False
            })
        }
    }
}

pub(crate) fn eval_expr(
    expr: &ResolvedExpr,
    row: &ExecutionRow,
    context: &EvalContext<'_>,
) -> Result<Value, EvalError> {
    match expr {
        ResolvedExpr::Path(path) => Ok(row.value_for_path(path)),
        ResolvedExpr::Literal(literal) => literal_value(literal),
        ResolvedExpr::FunctionCall {
            function_id, args, ..
        } => eval_function(function_id, args, row, context),
        ResolvedExpr::AggregateCall { .. } => Err(EvalError::new(
            "aggregate expressions are evaluated by the aggregate executor",
        )),
        ResolvedExpr::Association(_) => Ok(Value::Null),
        ResolvedExpr::Evidence(_) => Ok(Value::Null),
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            if eval_predicate(predicate, row, context)? {
                eval_expr(then_expr, row, context)
            } else {
                eval_expr(else_expr, row, context)
            }
        }
    }
}

pub(crate) fn literal_value(literal: &ResolvedLiteral) -> Result<Value, EvalError> {
    match &literal.typed_value {
        ResolvedLiteralValue::Null => Ok(Value::Null),
        ResolvedLiteralValue::Boolean(value) => Ok(Value::Bool(*value)),
        ResolvedLiteralValue::String(value) => Ok(Value::String(value.clone())),
        ResolvedLiteralValue::SignedInteger(value) => Ok(Value::Number(Number::from(*value))),
        ResolvedLiteralValue::UnsignedInteger(value) => Ok(Value::Number(Number::from(*value))),
        ResolvedLiteralValue::BigInteger(_) => Err(EvalError::new(
            "integer literal is outside executable range",
        )),
        ResolvedLiteralValue::Decimal { canonical, .. } => Ok(Value::String(canonical.clone())),
        ResolvedLiteralValue::TimestampMicros { micros, .. } => {
            Ok(Value::Number(Number::from(*micros)))
        }
        ResolvedLiteralValue::Uuid { canonical_hex, .. } => {
            Ok(Value::String(canonical_hex.clone()))
        }
        ResolvedLiteralValue::Binary {
            canonical_hex,
            bytes,
        } => Ok(Value::String(
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .unwrap_or_else(|_| canonical_hex.clone()),
        )),
    }
}

fn eval_function(
    name: &str,
    args: &[ResolvedExpr],
    row: &ExecutionRow,
    context: &EvalContext<'_>,
) -> Result<Value, EvalError> {
    match name {
        "identity" => eval_required_arg(name, args, 0, row, context),
        "trim" => {
            let value = eval_required_arg(name, args, 0, row, context)?;
            Ok(match value.as_str() {
                Some(value) => Value::String(value.trim().to_string()),
                None if value.is_null() => Value::Null,
                _ => return Err(EvalError::new("trim expects a string argument")),
            })
        }
        "lower" | "lowercase" => {
            let value = eval_required_arg(name, args, 0, row, context)?;
            Ok(match value.as_str() {
                Some(value) => Value::String(value.to_lowercase()),
                None if value.is_null() => Value::Null,
                _ => return Err(EvalError::new("lower expects a string argument")),
            })
        }
        "upper" | "uppercase" => {
            let value = eval_required_arg(name, args, 0, row, context)?;
            Ok(match value.as_str() {
                Some(value) => Value::String(value.to_uppercase()),
                None if value.is_null() => Value::Null,
                _ => return Err(EvalError::new("upper expects a string argument")),
            })
        }
        "startsWith" => {
            let value = eval_required_arg(name, args, 0, row, context)?;
            let prefix = eval_required_arg(name, args, 1, row, context)?;
            Ok(match (value.as_str(), prefix.as_str()) {
                (Some(value), Some(prefix)) => Value::Bool(value.starts_with(prefix)),
                _ if value.is_null() || prefix.is_null() => Value::Null,
                _ => return Err(EvalError::new("startsWith expects string arguments")),
            })
        }
        "isNull" | "isNotNull" => {
            let value = eval_required_arg(name, args, 0, row, context)?;
            let is_null = value.is_null();
            Ok(Value::Bool(if name == "isNull" {
                is_null
            } else {
                !is_null
            }))
        }
        "length" => {
            let value = eval_required_arg(name, args, 0, row, context)?;
            Ok(match value {
                Value::String(value) => json!(value.chars().count() as u64),
                Value::Array(values) => json!(values.len() as u64),
                Value::Object(values) => json!(values.len() as u64),
                Value::Null => Value::Null,
                _ => return Err(EvalError::new("length expects string, array, or object")),
            })
        }
        "coalesce" => {
            for arg in args {
                let value = eval_expr(arg, row, context)?;
                if !value.is_null() {
                    return Ok(value);
                }
            }
            Ok(Value::Null)
        }
        "cast" => eval_cast(args, row, context),
        "exists" => {
            let value = eval_required_arg(name, args, 0, row, context)?;
            Ok(Value::Bool(
                !value.is_null() && !matches!(&value, Value::Array(values) if values.is_empty()),
            ))
        }
        other => Err(EvalError::new(format!(
            "deterministic function '{other}' has no Phase 3 executable body"
        ))),
    }
}

fn eval_cast(
    args: &[ResolvedExpr],
    row: &ExecutionRow,
    context: &EvalContext<'_>,
) -> Result<Value, EvalError> {
    let value = eval_required_arg("cast", args, 0, row, context)?;
    let target = eval_required_arg("cast", args, 1, row, context)?;
    let Some(target) = target.as_str().map(canonical_cast_target) else {
        return Err(EvalError::new("cast target must be a string literal"));
    };
    let Some(target) = target else {
        return Err(EvalError::new("cast target type is unsupported"));
    };
    if value.is_null() {
        return Ok(Value::Null);
    }
    match target {
        "bool" => cast_to_bool(value),
        "utf8" | "json" | "decimal128" | "uuid" | "binary" => {
            Ok(Value::String(json_to_string(value)))
        }
        "int64" | "timestamp_micros" => cast_to_i64(value).map(|value| json!(value)),
        "uint64" => cast_to_u64(value).map(|value| json!(value)),
        _ => Err(EvalError::new("cast target type is unsupported")),
    }
}

fn canonical_cast_target(target: &str) -> Option<&'static str> {
    match target.trim().to_ascii_lowercase().as_str() {
        "bool" | "boolean" => Some("bool"),
        "utf8" | "string" => Some("utf8"),
        "json" => Some("json"),
        "int" | "int64" => Some("int64"),
        "uint" | "uint64" => Some("uint64"),
        "decimal" | "decimal128" => Some("decimal128"),
        "timestamp" | "timestamp_micros" => Some("timestamp_micros"),
        "uuid" => Some("uuid"),
        "binary" => Some("binary"),
        _ => None,
    }
}

fn cast_to_bool(value: Value) -> Result<Value, EvalError> {
    match value {
        Value::Bool(value) => Ok(Value::Bool(value)),
        Value::String(value) if value.eq_ignore_ascii_case("true") => Ok(Value::Bool(true)),
        Value::String(value) if value.eq_ignore_ascii_case("false") => Ok(Value::Bool(false)),
        _ => Err(EvalError::new(
            "cast to bool expects bool or true/false string",
        )),
    }
}

fn cast_to_i64(value: Value) -> Result<i64, EvalError> {
    match value {
        Value::Number(value) => value
            .as_i64()
            .ok_or_else(|| EvalError::new("cast to int64 expects an integer in range")),
        Value::String(value) => value
            .parse::<i64>()
            .map_err(|_| EvalError::new("cast to int64 expects an integer string")),
        _ => Err(EvalError::new("cast to int64 expects number or string")),
    }
}

fn cast_to_u64(value: Value) -> Result<u64, EvalError> {
    match value {
        Value::Number(value) => value
            .as_u64()
            .ok_or_else(|| EvalError::new("cast to uint64 expects an unsigned integer in range")),
        Value::String(value) => value
            .parse::<u64>()
            .map_err(|_| EvalError::new("cast to uint64 expects an unsigned integer string")),
        _ => Err(EvalError::new("cast to uint64 expects number or string")),
    }
}

fn json_to_string(value: Value) -> String {
    match value {
        Value::String(value) => value,
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(&value)
            .expect("serde_json Value always serializes for cast to string"),
        Value::Null => String::new(),
    }
}

fn eval_required_arg(
    name: &str,
    args: &[ResolvedExpr],
    index: usize,
    row: &ExecutionRow,
    context: &EvalContext<'_>,
) -> Result<Value, EvalError> {
    let arg = args
        .get(index)
        .ok_or_else(|| EvalError::new(format!("{name} missing required argument {index}")))?;
    eval_expr(arg, row, context)
}

fn exists_expr(
    expr: &ResolvedExpr,
    row: &ExecutionRow,
    context: &EvalContext<'_>,
) -> Result<Truth, EvalError> {
    let found = match expr {
        ResolvedExpr::Association(association) => current_row_goid(row)
            .map(|goid| {
                context.association_edges.exists_for_scoped(
                    current_row_dataset_file_ordinal(row),
                    goid,
                    association,
                )
            })
            .unwrap_or(false),
        ResolvedExpr::Evidence(evidence) => context.evidence_index.exists_for(row, evidence),
        _ => false,
    };
    Ok(if found { Truth::True } else { Truth::False })
}

fn bool_truth(value: &Value) -> Result<Truth, EvalError> {
    match value {
        Value::Bool(true) => Ok(Truth::True),
        Value::Bool(false) => Ok(Truth::False),
        Value::Null => Ok(Truth::Unknown),
        _ => Err(EvalError::new(
            "boolean expression did not evaluate to bool",
        )),
    }
}

fn compare_values_typed(
    left_expr: &ResolvedExpr,
    left: &Value,
    op: AstCompareOp,
    right_expr: &ResolvedExpr,
    right: &Value,
) -> Truth {
    if left.is_null() || right.is_null() {
        return Truth::Unknown;
    }
    let type_hint = common_logical_type(left_expr, right_expr);
    let collation_id = expr_collation_id(left_expr).or_else(|| expr_collation_id(right_expr));
    let ordering = value_ordering_typed(left, right, type_hint.as_deref(), collation_id);
    match op {
        AstCompareOp::Eq => truth(values_equal_typed(left, right, type_hint.as_deref())),
        AstCompareOp::Ne => truth(!values_equal_typed(left, right, type_hint.as_deref())),
        AstCompareOp::Lt => ordering.map_or(Truth::Unknown, |ordering| truth(ordering.is_lt())),
        AstCompareOp::Le => ordering.map_or(Truth::Unknown, |ordering| {
            truth(ordering.is_lt() || ordering.is_eq())
        }),
        AstCompareOp::Gt => ordering.map_or(Truth::Unknown, |ordering| truth(ordering.is_gt())),
        AstCompareOp::Ge => ordering.map_or(Truth::Unknown, |ordering| {
            truth(ordering.is_gt() || ordering.is_eq())
        }),
    }
}

fn truth(value: bool) -> Truth {
    if value {
        Truth::True
    } else {
        Truth::False
    }
}

pub(crate) fn values_equal_typed(left: &Value, right: &Value, logical_type: Option<&str>) -> bool {
    match (left, right) {
        _ if typed_or_mixed_numeric_values(left, right, logical_type) => {
            decimal_compare(left, right).is_some_and(|ordering| ordering.is_eq())
        }
        (Value::Number(left), Value::Number(right)) => number_to_f64(left)
            .zip(number_to_f64(right))
            .is_some_and(|(l, r)| l == r),
        _ => left == right,
    }
}

pub(crate) fn value_ordering_typed(
    left: &Value,
    right: &Value,
    logical_type: Option<&str>,
    collation_id: Option<u16>,
) -> Option<Ordering> {
    match (left, right) {
        _ if typed_or_mixed_numeric_values(left, right, logical_type) => {
            decimal_compare(left, right)
        }
        (Value::Number(left), Value::Number(right)) => {
            number_to_f64(left)?.partial_cmp(&number_to_f64(right)?)
        }
        (Value::String(left), Value::String(right)) => string_ordering(left, right, collation_id),
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

pub(crate) fn expr_logical_type(expr: &ResolvedExpr) -> Option<&str> {
    match expr {
        ResolvedExpr::Path(path) => Some(path.logical_type.as_str()),
        ResolvedExpr::Literal(literal) => Some(literal.logical_type.as_str()),
        ResolvedExpr::FunctionCall { logical_type, .. }
        | ResolvedExpr::AggregateCall { logical_type, .. }
        | ResolvedExpr::Conditional { logical_type, .. } => Some(logical_type.as_str()),
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) => None,
    }
}

pub(crate) fn expr_collation_id(expr: &ResolvedExpr) -> Option<u16> {
    match expr {
        ResolvedExpr::Path(ResolvedPath { collation_id, .. }) => *collation_id,
        _ => None,
    }
}

fn common_logical_type(left: &ResolvedExpr, right: &ResolvedExpr) -> Option<String> {
    let left = expr_logical_type(left);
    let right = expr_logical_type(right);
    match (left, right) {
        (Some(left), Some(right)) if left == right => Some(left.to_string()),
        (Some(left), _) if is_numeric_type(Some(left)) => Some(left.to_string()),
        (_, Some(right)) if is_numeric_type(Some(right)) => Some(right.to_string()),
        (Some(left), _) if left == "utf8" || left == "string" => Some(left.to_string()),
        (_, Some(right)) if right == "utf8" || right == "string" => Some(right.to_string()),
        _ => None,
    }
}

fn is_numeric_type(logical_type: Option<&str>) -> bool {
    matches!(
        logical_type,
        Some(
            "int8"
                | "int16"
                | "int32"
                | "int64"
                | "uint8"
                | "uint16"
                | "uint32"
                | "uint64"
                | "float32"
                | "float64"
                | "decimal64"
                | "decimal128"
                | "timestamp_micros"
                | "timestamp_nanos"
                | "date_days"
        )
    )
}

fn typed_or_mixed_numeric_values(left: &Value, right: &Value, logical_type: Option<&str>) -> bool {
    (is_numeric_type(logical_type) || left.is_number() || right.is_number())
        && parse_decimal_value(left).is_some()
        && parse_decimal_value(right).is_some()
}

fn string_ordering(left: &str, right: &str, collation_id: Option<u16>) -> Option<Ordering> {
    let id = collation_id.unwrap_or(CollationKind::Utf8Bytewise.id());
    let kind = if id == CollationKind::None.id() {
        CollationKind::Utf8Bytewise
    } else {
        CollationKind::from_id(id)?
    };
    if !kind.supports_ordering() {
        return None;
    }
    match kind.compare(left.as_bytes(), right.as_bytes()).ok()? {
        Ordering3::Less => Some(Ordering::Less),
        Ordering3::Equal => Some(Ordering::Equal),
        Ordering3::Greater => Some(Ordering::Greater),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactDecimal {
    value: i128,
    scale: u32,
}

impl ExactDecimal {
    pub(crate) fn to_json_sum(self) -> Result<Value, String> {
        decimal_to_json_value(self.value, self.scale)
    }

    pub(crate) fn checked_add(self, other: Self) -> Option<Self> {
        let scale = self.scale.max(other.scale);
        let left = rescale_decimal(self.value, self.scale, scale)?;
        let right = rescale_decimal(other.value, other.scale, scale)?;
        Some(Self {
            value: left.checked_add(right)?,
            scale,
        })
    }

    pub(crate) fn checked_div_u64(self, divisor: u64) -> Option<Self> {
        if divisor == 0 {
            return None;
        }
        let extra_scale = 6;
        let scaled = self.value.checked_mul(pow10(extra_scale)?)?;
        Some(
            Self {
                value: scaled / i128::from(divisor),
                scale: self.scale + extra_scale,
            }
            .normalized(),
        )
    }

    fn normalized(self) -> Self {
        let mut value = self.value;
        let mut scale = self.scale;
        while scale > 0 && value % 10 == 0 {
            value /= 10;
            scale -= 1;
        }
        Self { value, scale }
    }
}

pub(crate) fn parse_decimal_value(value: &Value) -> Option<ExactDecimal> {
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Some(ExactDecimal {
                    value: i128::from(value),
                    scale: 0,
                })
            } else if let Some(value) = number.as_u64() {
                Some(ExactDecimal {
                    value: i128::from(value),
                    scale: 0,
                })
            } else {
                let text = number.to_string();
                parse_decimal_str(&text)
            }
        }
        Value::String(text) => parse_decimal_str(text),
        _ => None,
    }
}

fn decimal_compare(left: &Value, right: &Value) -> Option<Ordering> {
    let left = parse_decimal_value(left)?;
    let right = parse_decimal_value(right)?;
    let scale = left.scale.max(right.scale);
    Some(
        rescale_decimal(left.value, left.scale, scale)?.cmp(&rescale_decimal(
            right.value,
            right.scale,
            scale,
        )?),
    )
}

fn parse_decimal_str(text: &str) -> Option<ExactDecimal> {
    if text.is_empty() {
        return None;
    }
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    if unsigned.is_empty() {
        return None;
    }
    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or("");
    let fractional = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return None;
    }
    if whole.is_empty() && fractional.is_empty() {
        return None;
    }
    if !whole.chars().all(|ch| ch.is_ascii_digit())
        || !fractional.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fractional}");
    let mut value = digits.parse::<i128>().ok()?;
    if negative {
        value = value.checked_neg()?;
    }
    Some(
        ExactDecimal {
            value,
            scale: fractional.len() as u32,
        }
        .normalized(),
    )
}

fn rescale_decimal(value: i128, from_scale: u32, to_scale: u32) -> Option<i128> {
    if to_scale < from_scale {
        return Some(value / pow10(from_scale - to_scale)?);
    }
    value.checked_mul(pow10(to_scale - from_scale)?)
}

fn decimal_to_json_value(value: i128, scale: u32) -> Result<Value, String> {
    if scale == 0 {
        if let Ok(value) = i64::try_from(value) {
            return Ok(json!(value));
        }
        if let Ok(value) = u64::try_from(value) {
            return Ok(json!(value));
        }
    }
    let negative = value < 0;
    let mut digits = value.abs().to_string();
    let scale = scale as usize;
    if scale > 0 {
        if digits.len() <= scale {
            digits = format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits);
        }
        let split = digits.len() - scale;
        digits.insert(split, '.');
        while digits.ends_with('0') {
            digits.pop();
        }
        if digits.ends_with('.') {
            digits.pop();
        }
    }
    if negative {
        digits.insert(0, '-');
    }
    Ok(Value::String(digits))
}

fn pow10(power: u32) -> Option<i128> {
    let mut out = 1i128;
    for _ in 0..power {
        out = out.checked_mul(10)?;
    }
    Some(out)
}

pub(crate) fn stable_value_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

pub(crate) fn logical_value_key(value: &Value, logical_type: Option<&str>) -> String {
    if (is_numeric_type(logical_type) || value.is_number()) && parse_decimal_value(value).is_some()
    {
        let decimal = parse_decimal_value(value)
            .expect("checked above")
            .normalized();
        return format!("number:{}:{}", decimal.scale, decimal.value);
    }
    stable_value_key(value)
}

pub(crate) fn distinct_count(
    values: impl Iterator<Item = Value>,
    logical_type: Option<&str>,
) -> u64 {
    values
        .map(|value| logical_value_key(&value, logical_type))
        .collect::<BTreeSet<_>>()
        .len() as u64
}

pub(crate) fn number_to_f64(number: &Number) -> Option<f64> {
    number
        .as_f64()
        .or_else(|| number.as_i64().map(|value| value as f64))
        .or_else(|| number.as_u64().map(|value| value as f64))
}
