use std::cmp::Ordering;

use cove_core::{
    collation::CollationKind,
    profile::cove_o::{
        CoveObjectKernelPropertyLane, CoveObjectKernelPropertyValues, CoveObjectKernelSurface,
    },
};

use crate::{
    AstCompareOp, ResolvedExpr, ResolvedLiteral, ResolvedLiteralValue, ResolvedPath,
    ResolvedPredicate, ResolvedSystemField,
};

pub(crate) use cove_core::native::{SelectionBitmap, SelectionVector};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KernelPredicate {
    ComparePathLiteral {
        path: ResolvedPath,
        op: AstCompareOp,
        literal: KernelLiteral,
    },
    InList {
        path: ResolvedPath,
        literals: Vec<KernelLiteral>,
    },
    StartsWithPathLiteral {
        path: ResolvedPath,
        prefix: String,
    },
    LengthComparePathLiteral {
        path: ResolvedPath,
        op: AstCompareOp,
        literal: KernelLiteral,
    },
    StringFunctionComparePathLiteral {
        function_id: String,
        path: ResolvedPath,
        op: AstCompareOp,
        literal: String,
    },
    NullCheck {
        path: ResolvedPath,
        negated: bool,
    },
    BoolPath {
        path: ResolvedPath,
    },
    CoalesceBool {
        terms: Vec<KernelBoolTerm>,
    },
    Not(Box<KernelPredicate>),
    And(Vec<KernelPredicate>),
    Or(Vec<KernelPredicate>),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KernelBoolTerm {
    Path(ResolvedPath),
    Literal(Option<bool>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KernelLiteral {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KernelTruth {
    True,
    False,
    Unknown,
}

pub(crate) fn select_rows_with_base(
    surface: &CoveObjectKernelSurface,
    object_type_id: u32,
    predicates: &[KernelPredicate],
    base: Option<SelectionBitmap>,
) -> SelectionBitmap {
    let mut bitmap = base.unwrap_or_else(|| SelectionBitmap::all(surface.system.len()));
    let row_count = surface.system.len();
    bitmap.retain_set_bits(|row| {
        row < row_count && surface.system.object_type_ids[row] == object_type_id
    });
    for predicate in predicates {
        apply_predicate(surface, predicate, &mut bitmap);
    }
    bitmap
}

fn apply_predicate(
    surface: &CoveObjectKernelSurface,
    predicate: &KernelPredicate,
    bitmap: &mut SelectionBitmap,
) {
    let row_count = surface.system.len();
    bitmap.retain_set_bits(|row| {
        row < row_count && eval_predicate_truth(surface, row, predicate) == KernelTruth::True
    });
}

fn eval_predicate_truth(
    surface: &CoveObjectKernelSurface,
    row: usize,
    predicate: &KernelPredicate,
) -> KernelTruth {
    match predicate {
        KernelPredicate::ComparePathLiteral { path, op, literal } => {
            compare_path_literal_truth(surface, row, path, *op, literal)
        }
        KernelPredicate::InList { path, literals } => {
            in_list_path_literals_truth(surface, row, path, literals)
        }
        KernelPredicate::StartsWithPathLiteral { path, prefix } => {
            starts_with_path_literal_truth(surface, row, path, prefix)
        }
        KernelPredicate::LengthComparePathLiteral { path, op, literal } => {
            length_compare_path_literal_truth(surface, row, path, *op, literal)
        }
        KernelPredicate::StringFunctionComparePathLiteral {
            function_id,
            path,
            op,
            literal,
        } => string_function_compare_path_literal_truth(
            surface,
            row,
            function_id,
            path,
            *op,
            literal,
        ),
        KernelPredicate::NullCheck { path, negated } => {
            let is_null = path_value_is_null(surface, row, path);
            KernelTruth::from_bool(if *negated { !is_null } else { is_null })
        }
        KernelPredicate::BoolPath { path } => bool_path(surface, row, path)
            .map(KernelTruth::from_bool)
            .unwrap_or(KernelTruth::Unknown),
        KernelPredicate::CoalesceBool { terms } => {
            for term in terms {
                match term {
                    KernelBoolTerm::Path(path) => {
                        if let Some(value) = bool_path(surface, row, path) {
                            return KernelTruth::from_bool(value);
                        }
                    }
                    KernelBoolTerm::Literal(Some(value)) => {
                        return KernelTruth::from_bool(*value);
                    }
                    KernelBoolTerm::Literal(None) => {}
                }
            }
            KernelTruth::Unknown
        }
        KernelPredicate::Not(inner) => match eval_predicate_truth(surface, row, inner) {
            KernelTruth::True => KernelTruth::False,
            KernelTruth::False => KernelTruth::True,
            KernelTruth::Unknown => KernelTruth::Unknown,
        },
        KernelPredicate::And(parts) => {
            let mut saw_unknown = false;
            for part in parts {
                match eval_predicate_truth(surface, row, part) {
                    KernelTruth::True => {}
                    KernelTruth::False => return KernelTruth::False,
                    KernelTruth::Unknown => saw_unknown = true,
                }
            }
            if saw_unknown {
                KernelTruth::Unknown
            } else {
                KernelTruth::True
            }
        }
        KernelPredicate::Or(parts) => {
            let mut saw_unknown = false;
            for part in parts {
                match eval_predicate_truth(surface, row, part) {
                    KernelTruth::True => return KernelTruth::True,
                    KernelTruth::False => {}
                    KernelTruth::Unknown => saw_unknown = true,
                }
            }
            if saw_unknown {
                KernelTruth::Unknown
            } else {
                KernelTruth::False
            }
        }
    }
}

impl KernelTruth {
    fn from_bool(value: bool) -> Self {
        if value {
            Self::True
        } else {
            Self::False
        }
    }
}

fn compare_path_literal_truth(
    surface: &CoveObjectKernelSurface,
    row: usize,
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
) -> KernelTruth {
    if matches!(literal, KernelLiteral::Null) {
        return KernelTruth::Unknown;
    }
    let ordering = match path.system_field.as_ref() {
        Some(ResolvedSystemField::BranchKey) => {
            compare_u64(surface.system.branch_keys[row], literal)
        }
        Some(ResolvedSystemField::TimestampUs) => {
            compare_i64(surface.system.timestamp_us[row], literal)
        }
        Some(ResolvedSystemField::Csn) => compare_u64(surface.system.csn[row], literal),
        Some(ResolvedSystemField::Goid) => {
            return if matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) {
                KernelTruth::from_bool(compare_hex(&surface.system.goids[row], literal, op))
            } else {
                KernelTruth::Unknown
            };
        }
        _ => property_lane(surface, path).and_then(|lane| compare_lane(lane, row, literal)),
    };
    ordering
        .map(|ordering| KernelTruth::from_bool(compare_ordering(ordering, op)))
        .unwrap_or(KernelTruth::Unknown)
}

fn in_list_path_literals_truth(
    surface: &CoveObjectKernelSurface,
    row: usize,
    path: &ResolvedPath,
    literals: &[KernelLiteral],
) -> KernelTruth {
    if path_value_is_null(surface, row, path) {
        return KernelTruth::Unknown;
    }
    let mut saw_unknown = false;
    for literal in literals {
        match compare_path_literal_truth(surface, row, path, AstCompareOp::Eq, literal) {
            KernelTruth::True => return KernelTruth::True,
            KernelTruth::False => {}
            KernelTruth::Unknown => saw_unknown = true,
        }
    }
    if saw_unknown {
        KernelTruth::Unknown
    } else {
        KernelTruth::False
    }
}

fn starts_with_path_literal_truth(
    surface: &CoveObjectKernelSurface,
    row: usize,
    path: &ResolvedPath,
    prefix: &str,
) -> KernelTruth {
    let Some(lane) = property_lane(surface, path) else {
        return KernelTruth::Unknown;
    };
    let CoveObjectKernelPropertyValues::String(values) = &lane.values else {
        return KernelTruth::Unknown;
    };
    values
        .get(row)
        .and_then(|value| value.as_ref())
        .map(|value| KernelTruth::from_bool(value.starts_with(prefix)))
        .unwrap_or(KernelTruth::Unknown)
}

fn length_compare_path_literal_truth(
    surface: &CoveObjectKernelSurface,
    row: usize,
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
) -> KernelTruth {
    let Some(length) = string_path_char_len(surface, row, path) else {
        return KernelTruth::Unknown;
    };
    compare_u64(length, literal)
        .map(|ordering| KernelTruth::from_bool(compare_ordering(ordering, op)))
        .unwrap_or(KernelTruth::Unknown)
}

fn string_function_compare_path_literal_truth(
    surface: &CoveObjectKernelSurface,
    row: usize,
    function_id: &str,
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &str,
) -> KernelTruth {
    if !matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) {
        return KernelTruth::Unknown;
    }
    let Some(value) = string_path_function_value(surface, row, function_id, path) else {
        return KernelTruth::Unknown;
    };
    KernelTruth::from_bool(match op {
        AstCompareOp::Eq => value == literal,
        AstCompareOp::Ne => value != literal,
        _ => false,
    })
}

fn compare_hex(bytes: &[u8; 16], literal: &KernelLiteral, op: AstCompareOp) -> bool {
    let KernelLiteral::String(literal) = literal else {
        return false;
    };
    let actual = crate::hex_lower(bytes);
    match op {
        AstCompareOp::Eq => actual == *literal,
        AstCompareOp::Ne => actual != *literal,
        _ => false,
    }
}

fn compare_lane(
    lane: &CoveObjectKernelPropertyLane,
    row: usize,
    literal: &KernelLiteral,
) -> Option<Ordering> {
    match (&lane.values, literal) {
        (CoveObjectKernelPropertyValues::Bool(values), KernelLiteral::Bool(rhs)) => values
            .get(row)
            .and_then(|value| value.map(|lhs| lhs.cmp(rhs))),
        (CoveObjectKernelPropertyValues::I64(values), literal) => values
            .get(row)
            .and_then(|value| value.and_then(|lhs| compare_i64(lhs, literal))),
        (CoveObjectKernelPropertyValues::U64(values), literal) => values
            .get(row)
            .and_then(|value| value.and_then(|lhs| compare_u64(lhs, literal))),
        (CoveObjectKernelPropertyValues::F64(values), literal) => values
            .get(row)
            .and_then(|value| value.and_then(|lhs| compare_f64(lhs, literal))),
        (CoveObjectKernelPropertyValues::String(values), KernelLiteral::String(rhs)) => values
            .get(row)
            .and_then(|value| value.as_ref().map(|lhs| lhs.cmp(rhs))),
        (_, KernelLiteral::Null) => None,
        _ => None,
    }
}

fn compare_i64(lhs: i64, literal: &KernelLiteral) -> Option<Ordering> {
    match literal {
        KernelLiteral::I64(rhs) => Some(lhs.cmp(rhs)),
        KernelLiteral::U64(rhs) => i128::from(lhs).partial_cmp(&i128::from(*rhs)),
        KernelLiteral::F64(rhs) => (lhs as f64).partial_cmp(rhs),
        _ => None,
    }
}

fn compare_u64(lhs: u64, literal: &KernelLiteral) -> Option<Ordering> {
    match literal {
        KernelLiteral::I64(rhs) => i128::from(lhs).partial_cmp(&i128::from(*rhs)),
        KernelLiteral::U64(rhs) => Some(lhs.cmp(rhs)),
        KernelLiteral::F64(rhs) => (lhs as f64).partial_cmp(rhs),
        _ => None,
    }
}

fn compare_f64(lhs: f64, literal: &KernelLiteral) -> Option<Ordering> {
    match literal {
        KernelLiteral::I64(rhs) => lhs.partial_cmp(&(*rhs as f64)),
        KernelLiteral::U64(rhs) => lhs.partial_cmp(&(*rhs as f64)),
        KernelLiteral::F64(rhs) => lhs.partial_cmp(rhs),
        _ => None,
    }
}

fn compare_ordering(ordering: Ordering, op: AstCompareOp) -> bool {
    match op {
        AstCompareOp::Eq => ordering == Ordering::Equal,
        AstCompareOp::Ne => ordering != Ordering::Equal,
        AstCompareOp::Lt => ordering == Ordering::Less,
        AstCompareOp::Le => ordering != Ordering::Greater,
        AstCompareOp::Gt => ordering == Ordering::Greater,
        AstCompareOp::Ge => ordering != Ordering::Less,
    }
}

fn path_value_is_null(surface: &CoveObjectKernelSurface, row: usize, path: &ResolvedPath) -> bool {
    if path.system_field.is_some() {
        return false;
    }
    property_lane(surface, path)
        .map(|lane| match &lane.values {
            CoveObjectKernelPropertyValues::Bool(values) => {
                values.get(row).is_none_or(Option::is_none)
            }
            CoveObjectKernelPropertyValues::I64(values) => {
                values.get(row).is_none_or(Option::is_none)
            }
            CoveObjectKernelPropertyValues::U64(values) => {
                values.get(row).is_none_or(Option::is_none)
            }
            CoveObjectKernelPropertyValues::F64(values) => {
                values.get(row).is_none_or(Option::is_none)
            }
            CoveObjectKernelPropertyValues::String(values) => {
                values.get(row).is_none_or(Option::is_none)
            }
            CoveObjectKernelPropertyValues::Json(values) => {
                values.get(row).is_none_or(serde_json::Value::is_null)
            }
        })
        .unwrap_or(true)
}

fn bool_path(surface: &CoveObjectKernelSurface, row: usize, path: &ResolvedPath) -> Option<bool> {
    let lane = property_lane(surface, path)?;
    match &lane.values {
        CoveObjectKernelPropertyValues::Bool(values) => values.get(row).copied().flatten(),
        _ => None,
    }
}

fn string_path_char_len(
    surface: &CoveObjectKernelSurface,
    row: usize,
    path: &ResolvedPath,
) -> Option<u64> {
    let lane = property_lane(surface, path)?;
    match &lane.values {
        CoveObjectKernelPropertyValues::String(values) => values
            .get(row)
            .and_then(|value| value.as_ref())
            .map(|value| value.chars().count() as u64),
        _ => None,
    }
}

fn string_path_function_value(
    surface: &CoveObjectKernelSurface,
    row: usize,
    function_id: &str,
    path: &ResolvedPath,
) -> Option<String> {
    let lane = property_lane(surface, path)?;
    let CoveObjectKernelPropertyValues::String(values) = &lane.values else {
        return None;
    };
    let value = values.get(row)?.as_ref()?;
    match function_id {
        "lower" | "lowercase" => Some(value.to_lowercase()),
        "upper" | "uppercase" => Some(value.to_uppercase()),
        "trim" => Some(value.trim().to_string()),
        _ => None,
    }
}

fn property_lane<'a>(
    surface: &'a CoveObjectKernelSurface,
    path: &ResolvedPath,
) -> Option<&'a CoveObjectKernelPropertyLane> {
    let property_id = path.property_id?;
    let object_type_id = path.object_type_id?;
    surface
        .property_lanes
        .iter()
        .find(|lane| lane.object_type_id == object_type_id && lane.property_id == property_id)
}

pub(crate) fn compile_kernel_predicates(
    predicate: Option<&ResolvedPredicate>,
) -> Result<Vec<KernelPredicate>, String> {
    let Some(predicate) = predicate else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    compile_predicate(predicate, &mut out)?;
    Ok(out)
}

fn compile_predicate(
    predicate: &ResolvedPredicate,
    out: &mut Vec<KernelPredicate>,
) -> Result<(), String> {
    match predicate {
        ResolvedPredicate::And(parts) => {
            for part in parts {
                compile_predicate(part, out)?;
            }
            Ok(())
        }
        ResolvedPredicate::Compare { left, op, right } => {
            if let Some((function_id, path, op, literal)) =
                string_function_path_literal_compare(left, *op, right)
            {
                out.push(KernelPredicate::StringFunctionComparePathLiteral {
                    function_id: function_id.to_string(),
                    path: path.clone(),
                    op,
                    literal,
                });
                return Ok(());
            }
            if let Some((path, op, literal)) = length_path_literal_compare(left, *op, right) {
                out.push(KernelPredicate::LengthComparePathLiteral {
                    path: path.clone(),
                    op,
                    literal,
                });
                return Ok(());
            }
            if let Some(predicate) = bool_expr_literal_compare(left, *op, right) {
                out.push(predicate);
                return Ok(());
            }
            let (path, op, literal) = path_literal_compare(left, *op, right).ok_or_else(|| {
                "kernel compare predicates require path/literal operands".to_string()
            })?;
            if !path_is_kernel_safe(path, op) {
                return Err("predicate path is not safe for kernel evaluation".into());
            }
            out.push(KernelPredicate::ComparePathLiteral {
                path: path.clone(),
                op,
                literal,
            });
            Ok(())
        }
        ResolvedPredicate::InList { expr, values } => {
            let ResolvedExpr::Path(path) = expr else {
                return Err("kernel IN predicates require a direct path".into());
            };
            if !path_is_kernel_safe(path, AstCompareOp::Eq) {
                return Err("IN predicate path is not safe for kernel evaluation".into());
            }
            let literals = values
                .iter()
                .map(|value| {
                    if !path_literal_is_kernel_compatible(path, value) {
                        return Err(
                            "kernel IN predicate literal is incompatible with the path type"
                                .to_string(),
                        );
                    }
                    kernel_literal(value)
                        .ok_or_else(|| "kernel IN predicate literal is unsupported".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if literals.is_empty() {
                return Err("kernel IN predicates require at least one literal".into());
            }
            out.push(KernelPredicate::InList {
                path: path.clone(),
                literals,
            });
            Ok(())
        }
        ResolvedPredicate::BoolExpr(expr) => {
            out.push(bool_expr_predicate(expr)?);
            Ok(())
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let ResolvedExpr::Path(path) = expr else {
                return Err("kernel null checks require direct paths".into());
            };
            out.push(KernelPredicate::NullCheck {
                path: path.clone(),
                negated: *negated,
            });
            Ok(())
        }
        ResolvedPredicate::Not(inner) => {
            let inner = compile_predicate_expr(inner)?;
            out.push(KernelPredicate::Not(Box::new(inner)));
            Ok(())
        }
        ResolvedPredicate::Or(parts) => {
            let parts = parts
                .iter()
                .map(compile_predicate_expr)
                .collect::<Result<Vec<_>, _>>()?;
            out.push(KernelPredicate::Or(parts));
            Ok(())
        }
        ResolvedPredicate::Exists(_) => Err("predicate form remains materialized residual".into()),
    }
}

fn compile_predicate_expr(predicate: &ResolvedPredicate) -> Result<KernelPredicate, String> {
    let mut predicates = Vec::new();
    compile_predicate(predicate, &mut predicates)?;
    Ok(if predicates.len() == 1 {
        predicates.remove(0)
    } else {
        KernelPredicate::And(predicates)
    })
}

fn path_literal_compare<'a>(
    left: &'a ResolvedExpr,
    op: AstCompareOp,
    right: &'a ResolvedExpr,
) -> Option<(&'a ResolvedPath, AstCompareOp, KernelLiteral)> {
    if let (Some(path), ResolvedExpr::Literal(literal)) =
        (direct_or_identity_cast_path(left), right)
    {
        if !path_literal_is_kernel_compatible(path, literal) {
            return None;
        }
        return kernel_literal(literal).map(|literal| (path, op, literal));
    }
    if let (ResolvedExpr::Literal(literal), Some(path)) =
        (left, direct_or_identity_cast_path(right))
    {
        if !path_literal_is_kernel_compatible(path, literal) {
            return None;
        }
        return kernel_literal(literal).map(|literal| (path, invert_compare_op(op), literal));
    }
    None
}

fn direct_or_identity_cast_path(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    match expr {
        ResolvedExpr::Path(path) => Some(path),
        ResolvedExpr::FunctionCall {
            function_id,
            deterministic,
            args,
            ..
        } if function_id == "cast" && *deterministic => {
            let [ResolvedExpr::Path(path), ResolvedExpr::Literal(target)] = args.as_slice() else {
                return None;
            };
            let ResolvedLiteralValue::String(target) = &target.typed_value else {
                return None;
            };
            cast_target_is_identity(path, target).then_some(path)
        }
        _ => None,
    }
}

fn cast_target_is_identity(path: &ResolvedPath, target: &str) -> bool {
    normalized_cast_type(path.logical_type.as_str()) == normalized_cast_type(target)
}

fn normalized_cast_type(value: &str) -> &str {
    match value {
        "bool" | "boolean" => "bool",
        "string" | "utf8" => "utf8",
        other => other,
    }
}

fn path_literal_is_kernel_compatible(path: &ResolvedPath, literal: &ResolvedLiteral) -> bool {
    if matches!(literal.typed_value, ResolvedLiteralValue::Null) {
        return true;
    }
    if let Some(system) = &path.system_field {
        return match system {
            ResolvedSystemField::Goid => literal_is_string_like(literal),
            ResolvedSystemField::BranchKey | ResolvedSystemField::Csn => {
                literal_is_integer_like(literal)
            }
            ResolvedSystemField::TimestampUs => literal_is_temporal_or_integer_like(literal),
            _ => false,
        };
    }
    match normalized_path_type(path.logical_type.as_str()) {
        "bool" => matches!(literal.typed_value, ResolvedLiteralValue::Boolean(_)),
        "utf8" => literal_is_string_like(literal),
        "uuid" => literal_is_uuid_like(literal),
        "numeric" => literal_is_numeric_like(literal),
        _ => false,
    }
}

fn normalized_path_type(value: &str) -> &str {
    match value {
        "bool" | "boolean" => "bool",
        "utf8" | "string" | "json" => "utf8",
        "uuid" => "uuid",
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "decimal128" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => "numeric",
        other => other,
    }
}

fn literal_is_string_like(literal: &ResolvedLiteral) -> bool {
    matches!(
        literal.typed_value,
        ResolvedLiteralValue::String(_)
            | ResolvedLiteralValue::Uuid { .. }
            | ResolvedLiteralValue::Binary { .. }
    )
}

fn literal_is_uuid_like(literal: &ResolvedLiteral) -> bool {
    matches!(
        literal.typed_value,
        ResolvedLiteralValue::Uuid { .. } | ResolvedLiteralValue::String(_)
    )
}

fn literal_is_integer_like(literal: &ResolvedLiteral) -> bool {
    matches!(
        literal.typed_value,
        ResolvedLiteralValue::SignedInteger(_) | ResolvedLiteralValue::UnsignedInteger(_)
    )
}

fn literal_is_temporal_or_integer_like(literal: &ResolvedLiteral) -> bool {
    literal_is_integer_like(literal)
        || matches!(
            literal.typed_value,
            ResolvedLiteralValue::TimestampMicros { .. }
        )
}

fn literal_is_numeric_like(literal: &ResolvedLiteral) -> bool {
    literal_is_integer_like(literal)
        || matches!(
            literal.typed_value,
            ResolvedLiteralValue::Decimal { .. } | ResolvedLiteralValue::TimestampMicros { .. }
        )
}

fn length_path_literal_compare<'a>(
    left: &'a ResolvedExpr,
    op: AstCompareOp,
    right: &'a ResolvedExpr,
) -> Option<(&'a ResolvedPath, AstCompareOp, KernelLiteral)> {
    if let (Some(path), ResolvedExpr::Literal(literal)) = (length_path_arg(left), right) {
        return kernel_length_literal(literal).map(|literal| (path, op, literal));
    }
    if let (ResolvedExpr::Literal(literal), Some(path)) = (left, length_path_arg(right)) {
        return kernel_length_literal(literal)
            .map(|literal| (path, invert_compare_op(op), literal));
    }
    None
}

fn string_function_path_literal_compare<'a>(
    left: &'a ResolvedExpr,
    op: AstCompareOp,
    right: &'a ResolvedExpr,
) -> Option<(&'a str, &'a ResolvedPath, AstCompareOp, String)> {
    if !matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) {
        return None;
    }
    if let (Some((function_id, path)), ResolvedExpr::Literal(literal)) =
        (string_function_path_arg(left), right)
    {
        return kernel_string_literal(literal).map(|literal| (function_id, path, op, literal));
    }
    if let (ResolvedExpr::Literal(literal), Some((function_id, path))) =
        (left, string_function_path_arg(right))
    {
        return kernel_string_literal(literal)
            .map(|literal| (function_id, path, invert_compare_op(op), literal));
    }
    None
}

fn bool_expr_literal_compare(
    left: &ResolvedExpr,
    op: AstCompareOp,
    right: &ResolvedExpr,
) -> Option<KernelPredicate> {
    if !matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) {
        return None;
    }
    let (expr, literal) = match (left, right) {
        (expr, ResolvedExpr::Literal(literal)) => (expr, literal),
        (ResolvedExpr::Literal(literal), expr) => (expr, literal),
        _ => return None,
    };
    let ResolvedLiteralValue::Boolean(value) = &literal.typed_value else {
        return None;
    };
    let predicate = bool_expr_predicate(expr).ok()?;
    let keep_true = matches!(
        (op, *value),
        (AstCompareOp::Eq, true) | (AstCompareOp::Ne, false)
    );
    Some(if keep_true {
        predicate
    } else {
        KernelPredicate::Not(Box::new(predicate))
    })
}

fn bool_expr_predicate(expr: &ResolvedExpr) -> Result<KernelPredicate, String> {
    match expr {
        ResolvedExpr::Path(path) => {
            if !matches!(path.logical_type.as_str(), "bool" | "boolean") {
                return Err("kernel boolean predicates require a boolean path".into());
            }
            Ok(KernelPredicate::BoolPath { path: path.clone() })
        }
        ResolvedExpr::FunctionCall {
            function_id,
            deterministic,
            args,
            ..
        } if *deterministic => match function_id.as_str() {
            "startsWith" => {
                let (path, prefix) = starts_with_path_literal_args(args)?;
                Ok(KernelPredicate::StartsWithPathLiteral {
                    path: path.clone(),
                    prefix,
                })
            }
            "isNull" | "isNotNull" => {
                let path = null_check_path_arg(args)?;
                Ok(KernelPredicate::NullCheck {
                    path: path.clone(),
                    negated: function_id == "isNotNull",
                })
            }
            "identity" => {
                let path = identity_bool_path_arg(args)?;
                Ok(KernelPredicate::BoolPath { path: path.clone() })
            }
            "cast" => {
                let path = identity_cast_bool_path_arg(args)?;
                Ok(KernelPredicate::BoolPath { path: path.clone() })
            }
            "coalesce" => {
                let terms = coalesce_bool_terms(args)?;
                Ok(KernelPredicate::CoalesceBool { terms })
            }
            _ => Err("predicate form remains materialized residual".into()),
        },
        _ => Err("predicate form remains materialized residual".into()),
    }
}

fn length_path_arg(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    let ResolvedExpr::FunctionCall {
        function_id,
        deterministic,
        args,
        ..
    } = expr
    else {
        return None;
    };
    if function_id != "length" || !deterministic {
        return None;
    }
    let [ResolvedExpr::Path(path)] = args.as_slice() else {
        return None;
    };
    matches!(path.logical_type.as_str(), "utf8" | "string").then_some(path)
}

fn string_function_path_arg(expr: &ResolvedExpr) -> Option<(&str, &ResolvedPath)> {
    let ResolvedExpr::FunctionCall {
        function_id,
        deterministic,
        args,
        ..
    } = expr
    else {
        return None;
    };
    if !*deterministic
        || !matches!(
            function_id.as_str(),
            "lower" | "lowercase" | "upper" | "uppercase" | "trim"
        )
    {
        return None;
    }
    let [ResolvedExpr::Path(path)] = args.as_slice() else {
        return None;
    };
    matches!(path.logical_type.as_str(), "utf8" | "string").then_some((function_id.as_str(), path))
}

fn starts_with_path_literal_args(args: &[ResolvedExpr]) -> Result<(&ResolvedPath, String), String> {
    let [ResolvedExpr::Path(path), ResolvedExpr::Literal(prefix)] = args else {
        return Err("startsWith kernel predicate requires path and literal arguments".into());
    };
    if !matches!(path.logical_type.as_str(), "utf8" | "string" | "json") {
        return Err("startsWith kernel predicate requires a string path".into());
    }
    let ResolvedLiteralValue::String(prefix) = &prefix.typed_value else {
        return Err("startsWith kernel predicate requires a string literal prefix".into());
    };
    Ok((path, prefix.clone()))
}

fn null_check_path_arg(args: &[ResolvedExpr]) -> Result<&ResolvedPath, String> {
    let [ResolvedExpr::Path(path)] = args else {
        return Err("null-check kernel predicate requires one direct path argument".into());
    };
    Ok(path)
}

fn identity_bool_path_arg(args: &[ResolvedExpr]) -> Result<&ResolvedPath, String> {
    let [ResolvedExpr::Path(path)] = args else {
        return Err("identity kernel predicate requires one direct path argument".into());
    };
    if !matches!(path.logical_type.as_str(), "bool" | "boolean") {
        return Err("identity kernel predicate requires a boolean path".into());
    }
    Ok(path)
}

fn identity_cast_bool_path_arg(args: &[ResolvedExpr]) -> Result<&ResolvedPath, String> {
    let [ResolvedExpr::Path(path), ResolvedExpr::Literal(target)] = args else {
        return Err(
            "identity cast kernel predicate requires path and target type arguments".into(),
        );
    };
    let ResolvedLiteralValue::String(target) = &target.typed_value else {
        return Err("identity cast kernel predicate requires a string target type".into());
    };
    if !matches!(path.logical_type.as_str(), "bool" | "boolean") {
        return Err("identity cast boolean predicate requires a boolean path".into());
    }
    cast_target_is_identity(path, target)
        .then_some(path)
        .ok_or_else(|| "identity cast kernel predicate target must match the path type".into())
}

fn coalesce_bool_terms(args: &[ResolvedExpr]) -> Result<Vec<KernelBoolTerm>, String> {
    if args.is_empty() {
        return Err("coalesce kernel predicate requires at least one argument".into());
    }
    args.iter()
        .map(|arg| match arg {
            ResolvedExpr::Path(path)
                if matches!(path.logical_type.as_str(), "bool" | "boolean") =>
            {
                Ok(KernelBoolTerm::Path(path.clone()))
            }
            ResolvedExpr::Literal(literal) => match &literal.typed_value {
                ResolvedLiteralValue::Boolean(value) => Ok(KernelBoolTerm::Literal(Some(*value))),
                ResolvedLiteralValue::Null => Ok(KernelBoolTerm::Literal(None)),
                _ => Err("coalesce kernel predicate supports only boolean/null literals".into()),
            },
            _ => Err("coalesce kernel predicate supports only boolean paths and literals".into()),
        })
        .collect()
}

fn invert_compare_op(op: AstCompareOp) -> AstCompareOp {
    match op {
        AstCompareOp::Eq => AstCompareOp::Eq,
        AstCompareOp::Ne => AstCompareOp::Ne,
        AstCompareOp::Lt => AstCompareOp::Gt,
        AstCompareOp::Le => AstCompareOp::Ge,
        AstCompareOp::Gt => AstCompareOp::Lt,
        AstCompareOp::Ge => AstCompareOp::Le,
    }
}

fn kernel_literal(literal: &ResolvedLiteral) -> Option<KernelLiteral> {
    match &literal.typed_value {
        ResolvedLiteralValue::Null => Some(KernelLiteral::Null),
        ResolvedLiteralValue::Boolean(value) => Some(KernelLiteral::Bool(*value)),
        ResolvedLiteralValue::String(value) => Some(KernelLiteral::String(value.clone())),
        ResolvedLiteralValue::SignedInteger(value) => Some(KernelLiteral::I64(*value)),
        ResolvedLiteralValue::UnsignedInteger(value) => Some(KernelLiteral::U64(*value)),
        ResolvedLiteralValue::BigInteger(_) => None,
        ResolvedLiteralValue::Decimal { canonical, .. } => {
            canonical.parse::<f64>().ok().map(KernelLiteral::F64)
        }
        ResolvedLiteralValue::TimestampMicros { micros, .. } => Some(KernelLiteral::I64(*micros)),
        ResolvedLiteralValue::Uuid { canonical_hex, .. } => {
            Some(KernelLiteral::String(canonical_hex.clone()))
        }
        ResolvedLiteralValue::Binary {
            canonical_hex,
            bytes,
        } => Some(KernelLiteral::String(
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .unwrap_or_else(|_| canonical_hex.clone()),
        )),
    }
}

fn kernel_length_literal(literal: &ResolvedLiteral) -> Option<KernelLiteral> {
    match kernel_literal(literal)? {
        KernelLiteral::I64(value) => Some(KernelLiteral::I64(value)),
        KernelLiteral::U64(value) => Some(KernelLiteral::U64(value)),
        KernelLiteral::F64(value) => Some(KernelLiteral::F64(value)),
        _ => None,
    }
}

fn kernel_string_literal(literal: &ResolvedLiteral) -> Option<String> {
    match &literal.typed_value {
        ResolvedLiteralValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn path_is_kernel_safe(path: &ResolvedPath, op: AstCompareOp) -> bool {
    if let Some(system) = &path.system_field {
        return matches!(
            system,
            ResolvedSystemField::Goid
                | ResolvedSystemField::BranchKey
                | ResolvedSystemField::TimestampUs
                | ResolvedSystemField::Csn
        ) && matches!(
            op,
            AstCompareOp::Eq
                | AstCompareOp::Ne
                | AstCompareOp::Lt
                | AstCompareOp::Le
                | AstCompareOp::Gt
                | AstCompareOp::Ge
        );
    }
    if path.physical_kind == "file_code" {
        return matches!(op, AstCompareOp::Eq | AstCompareOp::Ne)
            || (matches!(
                op,
                AstCompareOp::Lt | AstCompareOp::Le | AstCompareOp::Gt | AstCompareOp::Ge
            ) && path_has_utf8_ordering_collation_contract(path));
    }
    if path.physical_kind == "fixed_bytes" {
        return matches!(path.logical_type.as_str(), "uuid")
            && matches!(op, AstCompareOp::Eq | AstCompareOp::Ne);
    }
    matches!(
        path.logical_type.as_str(),
        "bool"
            | "boolean"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "date_days"
            | "timestamp_micros"
            | "timestamp_nanos"
            | "utf8"
            | "string"
    )
}

fn path_has_utf8_ordering_collation_contract(path: &ResolvedPath) -> bool {
    matches!(path.logical_type.as_str(), "utf8" | "string")
        && match path.collation_id {
            None => true,
            Some(id) if id == CollationKind::None.id() => true,
            Some(id) => {
                CollationKind::from_id(id).is_some_and(|kind| kind == CollationKind::Utf8Bytewise)
            }
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_to_selection_vector_preserves_set_bits() {
        let mut bitmap = SelectionBitmap::all(130);
        bitmap.clear(0);
        bitmap.clear(64);
        bitmap.clear(129);
        let vector = bitmap.to_selection_vector();
        assert_eq!(bitmap.count_ones(), 127);
        assert_eq!(bitmap.word_count(), 3);
        assert_eq!(vector.len(), 127);
        assert!(!vector.rows().contains(&0));
        assert!(!vector.rows().contains(&64));
        assert!(!vector.rows().contains(&129));
    }
}
