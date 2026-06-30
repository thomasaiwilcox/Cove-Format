use super::*;

pub(super) fn row_matches_projection_filters(
    row: &Map<String, Value>,
    filters: &[ProjectionFilter],
) -> bool {
    filters.iter().all(|filter| filter.matches(row))
}

impl ProjectionFilter {
    pub(super) fn column_name(&self) -> &str {
        match self {
            Self::Compare { column, .. }
            | Self::InList { column, .. }
            | Self::IsNull { column, .. } => column,
        }
    }

    fn matches(&self, row: &Map<String, Value>) -> bool {
        let value = row.get(self.column_name()).unwrap_or(&Value::Null);
        projection_filter_matches_value(self, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_not_equal_filter_matches_non_null_unequal_values_only() {
        let filter = ProjectionFilter::Compare {
            column: "status".into(),
            op: ProjectionFilterOp::Ne,
            literal: ProjectionFilterLiteral::Utf8("closed".into()),
        };

        let mut open = Map::new();
        open.insert("status".into(), Value::String("open".into()));
        assert!(row_matches_projection_filters(
            &open,
            std::slice::from_ref(&filter)
        ));

        let mut closed = Map::new();
        closed.insert("status".into(), Value::String("closed".into()));
        assert!(!row_matches_projection_filters(
            &closed,
            std::slice::from_ref(&filter)
        ));

        let mut null = Map::new();
        null.insert("status".into(), Value::Null);
        assert!(!row_matches_projection_filters(
            &null,
            std::slice::from_ref(&filter)
        ));
    }
}

pub(super) fn projection_filter_matches_value(filter: &ProjectionFilter, value: &Value) -> bool {
    match filter {
        ProjectionFilter::Compare { op, literal, .. } => match op {
            ProjectionFilterOp::Eq => projection_filter_eq(value, literal),
            ProjectionFilterOp::Ne => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| !ordering.is_eq())
            }
            ProjectionFilterOp::Lt => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| ordering.is_lt())
            }
            ProjectionFilterOp::LtEq => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| !ordering.is_gt())
            }
            ProjectionFilterOp::Gt => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| ordering.is_gt())
            }
            ProjectionFilterOp::GtEq => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| !ordering.is_lt())
            }
        },
        ProjectionFilter::InList { literals, .. } => {
            !value.is_null()
                && literals
                    .iter()
                    .any(|literal| projection_filter_eq(value, literal))
        }
        ProjectionFilter::IsNull { negated, .. } => {
            let is_null = value.is_null();
            if *negated {
                !is_null
            } else {
                is_null
            }
        }
    }
}

pub(super) fn projection_filter_eq(value: &Value, literal: &ProjectionFilterLiteral) -> bool {
    projection_filter_cmp(value, literal).is_some_and(|ordering| ordering.is_eq())
}

pub(super) fn projection_filter_cmp(
    value: &Value,
    literal: &ProjectionFilterLiteral,
) -> Option<std::cmp::Ordering> {
    if value.is_null() {
        return None;
    }
    match literal {
        ProjectionFilterLiteral::Null => None,
        ProjectionFilterLiteral::Boolean(literal) => {
            value.as_bool().map(|value| value.cmp(literal))
        }
        ProjectionFilterLiteral::Int64(literal) => value
            .as_i64()
            .map(|value| value.cmp(literal))
            .or_else(|| {
                value
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
                    .map(|value| value.cmp(literal))
            })
            .or_else(|| {
                value
                    .as_f64()
                    .map(|value| value.total_cmp(&(*literal as f64)))
            }),
        ProjectionFilterLiteral::UInt64(literal) => value
            .as_u64()
            .map(|value| value.cmp(literal))
            .or_else(|| {
                value
                    .as_i64()
                    .and_then(|value| u64::try_from(value).ok())
                    .map(|value| value.cmp(literal))
            })
            .or_else(|| {
                value
                    .as_f64()
                    .map(|value| value.total_cmp(&(*literal as f64)))
            }),
        ProjectionFilterLiteral::Float64(literal) => {
            value.as_f64().map(|value| value.total_cmp(literal))
        }
        ProjectionFilterLiteral::Utf8(literal) => {
            value.as_str().map(|value| value.cmp(literal.as_str()))
        }
    }
}

pub(super) fn reached_projection_limit(rows: &mut Vec<Value>, max_rows: Option<usize>) -> bool {
    let Some(max_rows) = max_rows else {
        return false;
    };
    if rows.len() > max_rows {
        rows.truncate(max_rows);
    }
    rows.len() >= max_rows
}

pub(super) fn projection_value(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    expression: &str,
) -> Result<Value, String> {
    match expression {
        "goid" | "object.goid" | "Object.goid" | "association.goid" => {
            return Ok(json!(hex_encode(&row.goid)));
        }
        "record.id" | "record.record_id" => return Ok(json!(hex_encode(&row.record_id))),
        "record.kind" => return Ok(json!(record_kind_name(row.record_kind))),
        "object.type_id" | "object_type_id" => return Ok(json!(row.object_type_id)),
        "temporal.timestamp_us" | "timestamp_us" => return Ok(json!(row.timestamp_us)),
        "temporal.csn" | "csn" => return Ok(json!(row.csn)),
        "temporal.branch_key" | "branch_key" => return Ok(json!(row.branch_key)),
        "object_type" | "object.type" | "Object.type" => return Ok(json!(row.object_type)),
        "association.source_goid" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                "source_goid",
            ))
        }
        "association.target_goid" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                "target_goid",
            ))
        }
        "association.association_type" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_TYPE,
                "association_type",
            ))
        }
        "association.mapping_rule_id" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_MAPPING_RULE_REF,
                "mapping_rule_id",
            ))
        }
        "association.source_evidence_id" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_EVIDENCE_REF,
                "source_evidence_id",
            ))
        }
        "association.source_role" => return Ok(projection_property_by_name(row, "source_role")),
        "association.target_role" => return Ok(projection_property_by_name(row, "target_role")),
        "association.valid_from" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
                "valid_from",
            ))
        }
        "association.valid_to" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_VALID_TO,
                "valid_to",
            ))
        }
        "association.observed_at" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT,
                "observed_at",
            ))
        }
        "association.cardinality_policy" => {
            return Ok(projection_property_by_name(row, "cardinality_policy"))
        }
        _ => {}
    }
    if let Some(literal) = literal_value(expression) {
        return Ok(literal);
    }
    if let Some(resolution) = parse_resolution_expression(expression) {
        return resolution_expression_value(model, row, resolution, expression);
    }
    if let Some(value) = conditional_expression(model, projection, row, expression)? {
        return Ok(value);
    }
    if let Some(inner) = expression
        .strip_prefix("count(association(")
        .and_then(|rest| rest.strip_suffix("))"))
    {
        let (association_type, endpoint_role) = parse_association_call_args(inner);
        let count = associated_rows(model, projection, row, association_type, endpoint_role)?.len();
        return Ok(json!(count));
    }
    if let Some((function, args)) = parse_function_call(expression) {
        return projection_function_value(model, projection, row, function, &args);
    }
    if let Some(traversal) = parse_association_traversal(expression) {
        let values = associated_rows(
            model,
            projection,
            row,
            traversal.association_type,
            traversal.endpoint_role,
        )?
        .into_iter()
        .map(|candidate| association_projection_value(&candidate, traversal.property_name))
        .collect::<Vec<_>>();
        return Ok(Value::Array(values));
    }
    let property_name = expression
        .rsplit('.')
        .next()
        .ok_or_else(|| format!("unsupported projection expression '{expression}'"))?;
    Ok(projection_property_by_name(row, property_name))
}

pub(super) fn projection_function_value(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
) -> Result<Value, String> {
    match function {
        "identity" => unary_arg(model, projection, row, function, args),
        "trim" => string_unary(model, projection, row, function, args, |value| {
            value.trim().to_string()
        }),
        "lower" | "lowercase" => string_unary(model, projection, row, function, args, |value| {
            value.to_ascii_lowercase()
        }),
        "upper" | "uppercase" => string_unary(model, projection, row, function, args, |value| {
            value.to_ascii_uppercase()
        }),
        "exists" => {
            let value = unary_arg(model, projection, row, function, args)?;
            Ok(json!(
                !value.is_null() && !matches!(&value, Value::Array(values) if values.is_empty())
            ))
        }
        "coalesce" => {
            for arg in args {
                let value = projection_value(model, projection, row, arg)?;
                if !value.is_null() {
                    return Ok(value);
                }
            }
            Ok(Value::Null)
        }
        "association" => {
            if args.len() != 1 {
                return Err("projection function 'association' expects one argument".into());
            }
            let (association_type, endpoint_role) = parse_association_call_args(&args[0]);
            Ok(Value::Array(
                associated_rows(model, projection, row, association_type, endpoint_role)?
                    .into_iter()
                    .map(|candidate| json!(hex_encode(&candidate.goid)))
                    .collect(),
            ))
        }
        "count" | "min" | "max" | "sum" | "avg" | "distinct_count" | "list" => {
            aggregate_function_value(model, projection, row, function, args)
        }
        other => Err(format!("unsupported projection function '{other}'")),
    }
}

pub(super) fn unary_arg(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "projection function '{function}' expects one argument"
        ));
    }
    projection_value(model, projection, row, &args[0])
}

pub(super) fn string_unary(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
    op: impl FnOnce(&str) -> String,
) -> Result<Value, String> {
    let value = unary_arg(model, projection, row, function, args)?;
    Ok(value
        .as_str()
        .map(|text| json!(op(text)))
        .unwrap_or(Value::Null))
}

pub(super) fn aggregate_function_value(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "projection aggregate '{function}' expects one argument"
        ));
    }
    let values = if let Some(traversal) = parse_association_traversal(&args[0]) {
        associated_rows(
            model,
            projection,
            row,
            traversal.association_type,
            traversal.endpoint_role,
        )?
        .into_iter()
        .map(|candidate| association_projection_value(&candidate, traversal.property_name))
        .collect::<Vec<_>>()
    } else if let Some((association_function, association_args)) = parse_function_call(&args[0]) {
        if association_function == "association" && association_args.len() == 1 {
            let (association_type, endpoint_role) =
                parse_association_call_args(&association_args[0]);
            associated_rows(model, projection, row, association_type, endpoint_role)?
                .into_iter()
                .map(|candidate| json!(hex_encode(&candidate.goid)))
                .collect::<Vec<_>>()
        } else {
            vec![projection_value(model, projection, row, &args[0])?]
        }
    } else {
        vec![projection_value(model, projection, row, &args[0])?]
    };
    match function {
        "count" => Ok(json!(values
            .iter()
            .filter(|value| !value.is_null())
            .count())),
        "list" => Ok(Value::Array(values)),
        "distinct_count" => {
            let set = values
                .into_iter()
                .filter(|value| !value.is_null())
                .map(|value| value.to_string())
                .collect::<std::collections::BTreeSet<_>>();
            Ok(json!(set.len()))
        }
        "min" => Ok(min_max_json(values, true)),
        "max" => Ok(min_max_json(values, false)),
        "sum" => Ok(json!(values
            .into_iter()
            .filter_map(json_number_f64)
            .sum::<f64>())),
        "avg" => {
            let numbers = values
                .into_iter()
                .filter_map(json_number_f64)
                .collect::<Vec<_>>();
            if numbers.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(json!(numbers.iter().sum::<f64>() / numbers.len() as f64))
            }
        }
        other => Err(format!("unsupported projection aggregate '{other}'")),
    }
}

pub(super) fn conditional_expression(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    expression: &str,
) -> Result<Option<Value>, String> {
    let Some((function, args)) = parse_function_call(expression) else {
        return Ok(None);
    };
    if !matches!(function, "if" | "ifelse") {
        return Ok(None);
    }
    if args.len() != 3 {
        return Err(format!(
            "projection conditional '{function}' expects three arguments"
        ));
    }
    let condition = projection_condition(model, projection, row, &args[0])?;
    Ok(Some(projection_value(
        model,
        projection,
        row,
        if condition { &args[1] } else { &args[2] },
    )?))
}

pub(super) fn projection_condition(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    expression: &str,
) -> Result<bool, String> {
    for op in ["==", "!=", ">=", "<=", ">", "<"] {
        if let Some((left, right)) = expression.split_once(op) {
            let left = projection_value(model, projection, row, left.trim())?;
            let right = projection_value(model, projection, row, right.trim())?;
            return Ok(compare_json_values(&left, &right, op));
        }
    }
    Ok(json_truthy(&projection_value(
        model, projection, row, expression,
    )?))
}

pub(super) fn associated_rows(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    association_type: &str,
    endpoint_role: Option<&str>,
) -> Result<Vec<ProjectionRow>, String> {
    let mut rows = model
        .rows_for_projection_for_aggregate()?
        .into_iter()
        .filter(|candidate| row_matches_association(candidate, association_type))
        .filter(|candidate| association_endpoint_matches(candidate, row, endpoint_role))
        .collect::<Vec<_>>();
    sort_projection_rows_by_ordering(&mut rows, projection, Some(row))?;
    Ok(rows)
}

pub(super) fn association_endpoint_matches(
    candidate: &ProjectionRow,
    anchor: &ProjectionRow,
    endpoint_role: Option<&str>,
) -> bool {
    let anchor_goid = json!(hex_encode(&anchor.goid));
    let source_goid = projection_property_by_flag_or_name(
        candidate,
        PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
        "source_goid",
    );
    let target_goid = projection_property_by_flag_or_name(
        candidate,
        PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        "target_goid",
    );
    let Some(role) = endpoint_role.map(str::trim).filter(|role| !role.is_empty()) else {
        return source_goid == anchor_goid;
    };
    match role {
        "source" | "from" => source_goid == anchor_goid,
        "target" | "to" => target_goid == anchor_goid,
        other => {
            (source_goid == anchor_goid
                && projection_property_by_name(candidate, "source_role").as_str() == Some(other))
                || (target_goid == anchor_goid
                    && projection_property_by_name(candidate, "target_role").as_str()
                        == Some(other))
        }
    }
}

pub(super) fn association_projection_value(row: &ProjectionRow, property_name: &str) -> Value {
    match property_name {
        "goid" => json!(hex_encode(&row.goid)),
        "source_goid" => projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
            "source_goid",
        ),
        "target_goid" => projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_TO_GOID,
            "target_goid",
        ),
        "association_type" => projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_TYPE,
            "association_type",
        ),
        other => projection_property_by_name(row, other),
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AssociationTraversal<'a> {
    pub(super) association_type: &'a str,
    pub(super) endpoint_role: Option<&'a str>,
    pub(super) property_name: &'a str,
}

pub(super) fn parse_association_traversal(expression: &str) -> Option<AssociationTraversal<'_>> {
    let expression = expression.trim();
    let rest = expression.strip_prefix("association(")?;
    let (association_type, rest) = rest.split_once(").")?;
    let (association_type, endpoint_role) = match association_type.split_once(',') {
        Some((association_type, endpoint_role)) => {
            (association_type.trim(), Some(endpoint_role.trim()))
        }
        None => (association_type.trim(), None),
    };
    (!association_type.is_empty() && !rest.trim().is_empty()).then_some(AssociationTraversal {
        association_type,
        endpoint_role: endpoint_role.filter(|role| !role.is_empty()),
        property_name: rest.trim(),
    })
}

pub(super) fn parse_association_call_args(input: &str) -> (&str, Option<&str>) {
    match input.split_once(',') {
        Some((association_type, endpoint_role)) => (
            association_type.trim(),
            Some(endpoint_role.trim()).filter(|role| !role.is_empty()),
        ),
        None => (input.trim(), None),
    }
}

pub(super) fn parse_function_call(expression: &str) -> Option<(&str, Vec<String>)> {
    let expression = expression.trim();
    let open = expression.find('(')?;
    if !expression.ends_with(')') {
        return None;
    }
    let function = expression[..open].trim();
    if function.is_empty()
        || !function
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    let inner = &expression[open + 1..expression.len() - 1];
    Some((function, split_args(inner)))
}

pub(super) fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote = None;
    let bytes = input.as_bytes();
    for (index, ch) in input.char_indices() {
        if let Some(active) = quote {
            if ch == active && bytes.get(index.wrapping_sub(1)) != Some(&b'\\') {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(input[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = input[start..].trim();
    if !tail.is_empty() || !input.is_empty() {
        args.push(tail.to_string());
    }
    args
}

pub(super) fn literal_value(expression: &str) -> Option<Value> {
    let expression = expression.trim();
    if expression == "null" {
        return Some(Value::Null);
    }
    if expression == "true" {
        return Some(Value::Bool(true));
    }
    if expression == "false" {
        return Some(Value::Bool(false));
    }
    if (expression.starts_with('"') && expression.ends_with('"'))
        || (expression.starts_with('\'') && expression.ends_with('\''))
    {
        return Some(Value::String(
            expression[1..expression.len() - 1].to_string(),
        ));
    }
    if let Ok(value) = expression.parse::<i64>() {
        return Some(json!(value));
    }
    if let Ok(value) = expression.parse::<f64>() {
        return Some(json!(value));
    }
    None
}

pub(super) fn min_max_json(values: Vec<Value>, min: bool) -> Value {
    values
        .into_iter()
        .filter(|value| !value.is_null())
        .min_by(|left, right| {
            let ordering = compare_json_order(left, right);
            if min {
                ordering
            } else {
                ordering.reverse()
            }
        })
        .unwrap_or(Value::Null)
}

pub(super) fn compare_json_order(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (
        json_number_f64(left.clone()),
        json_number_f64(right.clone()),
    ) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        _ => left.to_string().cmp(&right.to_string()),
    }
}

pub(super) fn compare_json_values(left: &Value, right: &Value, op: &str) -> bool {
    let ordering = compare_json_order(left, right);
    match op {
        "==" => left == right,
        "!=" => left != right,
        ">" => ordering.is_gt(),
        ">=" => !ordering.is_lt(),
        "<" => ordering.is_lt(),
        "<=" => !ordering.is_gt(),
        _ => false,
    }
}

pub(super) fn json_number_f64(value: Value) -> Option<f64> {
    value.as_f64()
}

pub(super) fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
    }
}

pub(super) fn record_kind_name(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Delta => "delta",
        RecordKind::Snapshot => "snapshot",
        RecordKind::ReservedLegacyMaterializedDelta => "reserved_legacy_materialized_delta",
        RecordKind::Baseline => "baseline",
        RecordKind::Tombstone => "tombstone",
        _ => "unknown",
    }
}

impl ProjectionModel {
    fn rows_for_projection_for_aggregate(&self) -> Result<Vec<ProjectionRow>, String> {
        Ok(self.reconstructed_rows.clone())
    }
}

#[cfg(test)]
pub(crate) fn property_by_name(row: &ObjectRow, property_name: &str) -> Value {
    row.properties
        .values()
        .find(|property| property.entry.property_name == property_name)
        .map(|property| property.value.clone())
        .unwrap_or(Value::Null)
}

pub(super) fn projection_property_by_name(row: &ProjectionRow, property_name: &str) -> Value {
    row.properties
        .iter()
        .find(|property| property.property_name == property_name)
        .map(|property| property.value.clone())
        .unwrap_or(Value::Null)
}

pub(super) fn projection_property_ref_by_name<'a>(
    row: &'a ProjectionRow,
    property_name: &str,
) -> Option<&'a Value> {
    row.properties
        .iter()
        .find(|property| property.property_name == property_name)
        .map(|property| &property.value)
}

pub(super) fn projection_property_by_flag_or_name(
    row: &ProjectionRow,
    flag: u32,
    property_name: &str,
) -> Value {
    row.properties
        .iter()
        .find(|property| property.flags & flag != 0)
        .map(|property| property.value.clone())
        .unwrap_or_else(|| projection_property_by_name(row, property_name))
}

pub(super) fn row_matches_association(row: &ProjectionRow, association_type: &str) -> bool {
    if row.object_type_flags & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT)
        != 0
    {
        let flagged = projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_TYPE,
            "association_type",
        );
        if flagged.as_str() == Some(association_type) {
            return true;
        }
        if row.object_type.strip_prefix("Association:") == Some(association_type) {
            return true;
        }
    }
    row.object_type == format!("Association:{association_type}")
}

pub(crate) fn run_fixture_path(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let fixture: Value = serde_json::from_slice(&bytes)
        .map_err(|err| format!("fixture {} is not valid JSON: {err}", path.display()))?;
    let map = PathBuf::from(required_str(&fixture, "mapping")?);
    let sources = fixture
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| "fixture.sources must be an array".to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| "fixture.sources entries must be strings".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let file = parse_map(&map)?;
    let rows = read_sources(&sources)?;
    if let Some(expected_rows) = fixture.get("expected_projected_rows") {
        let projected = project_rows(&file, &rows)?;
        if &projected["rows"] != expected_rows {
            return Err("fixture projected rows did not match".into());
        }
    }
    println!("{}", json!({"ok": true, "fixture": path}));
    Ok(())
}
