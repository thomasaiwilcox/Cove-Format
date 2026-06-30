use super::*;

pub(super) fn expr_logical_type(expr: &ResolvedExpr) -> Option<&str> {
    match expr {
        ResolvedExpr::Path(path) => Some(path.logical_type.as_str()),
        ResolvedExpr::Literal(literal) => Some(literal.logical_type.as_str()),
        ResolvedExpr::FunctionCall { logical_type, .. }
        | ResolvedExpr::AggregateCall { logical_type, .. }
        | ResolvedExpr::Conditional { logical_type, .. } => Some(logical_type.as_str()),
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) | ResolvedExpr::TableExists(_) => {
            None
        }
    }
}

pub(super) fn common_output_logical_type(
    left: &ResolvedExpr,
    right: &ResolvedExpr,
) -> &'static str {
    match (expr_logical_type(left), expr_logical_type(right)) {
        (Some(left), Some(right)) => common_logical_type_names(left, right),
        _ => "unknown",
    }
}

pub(super) fn common_logical_type_names(left: &str, right: &str) -> &'static str {
    match (left, right) {
        ("null", "null") => "null",
        ("null", other) | (other, "null") => known_logical_type(other),
        (left, right) if left == right => known_logical_type(left),
        ("utf8" | "string", "utf8" | "string") => "utf8",
        (left, right) if is_decimal_type(left) || is_decimal_type(right) => "decimal128",
        (left, right) if is_numeric_type(left) && is_numeric_type(right) => {
            widest_numeric_type(left, right)
        }
        (left, right) if left == "unknown" || right == "unknown" => "unknown",
        _ => "incompatible",
    }
}

pub(super) fn coalesce_common_logical_type(args: &[ResolvedExpr]) -> Option<&'static str> {
    let mut current = "null";
    for arg in args {
        let arg_type = expr_logical_type(arg)?;
        let next = common_logical_type_names(current, arg_type);
        if next == "incompatible" {
            return None;
        }
        current = next;
    }
    Some(current)
}

pub(super) fn known_logical_type(logical_type: &str) -> &'static str {
    match logical_type {
        "null" => "null",
        "bool" => "bool",
        "utf8" => "utf8",
        "string" => "string",
        "int8" => "int8",
        "int16" => "int16",
        "int32" => "int32",
        "int64" => "int64",
        "uint8" => "uint8",
        "uint16" => "uint16",
        "uint32" => "uint32",
        "uint64" => "uint64",
        "float32" => "float32",
        "float64" => "float64",
        "decimal64" => "decimal64",
        "decimal128" => "decimal128",
        "timestamp_micros" => "timestamp_micros",
        "timestamp_nanos" => "timestamp_nanos",
        "date_days" => "date_days",
        _ => "unknown",
    }
}

pub(super) fn is_decimal_type(logical_type: &str) -> bool {
    matches!(logical_type, "decimal64" | "decimal128")
}

pub(super) fn is_numeric_type(logical_type: &str) -> bool {
    matches!(
        logical_type,
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
}

pub(super) fn widest_numeric_type(left: &str, right: &str) -> &'static str {
    if matches!(left, "float64" | "float32") || matches!(right, "float64" | "float32") {
        "float64"
    } else if matches!(left, "uint64") || matches!(right, "uint64") {
        "uint64"
    } else {
        "int64"
    }
}

pub(super) fn integer_literal_value(value: i128) -> ResolvedLiteralValue {
    if let Ok(value) = i64::try_from(value) {
        ResolvedLiteralValue::SignedInteger(value)
    } else if let Ok(value) = u64::try_from(value) {
        ResolvedLiteralValue::UnsignedInteger(value)
    } else {
        ResolvedLiteralValue::BigInteger(value.to_string())
    }
}

pub(super) fn decode_hex_bytes(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk).ok()?;
        out.push(u8::from_str_radix(text, 16).ok()?);
    }
    Some(out)
}

pub(super) fn parse_uuid_literal(value: &str) -> Option<([u8; 16], String)> {
    let mut compact = String::with_capacity(32);
    for (index, ch) in value.chars().enumerate() {
        if ch == '-' {
            if !matches!(index, 8 | 13 | 18 | 23) {
                return None;
            }
            continue;
        }
        if !ch.is_ascii_hexdigit() {
            return None;
        }
        compact.push(ch.to_ascii_lowercase());
    }
    if compact.len() != 32 {
        return None;
    }
    let decoded = decode_hex_bytes(&compact)?;
    if decoded.len() != 16 {
        return None;
    }
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&decoded);
    Some((bytes, compact))
}

pub(super) fn aggregate_logical_type(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count | AstAggregateName::DistinctCount => "uint64",
        AstAggregateName::Exists => "bool",
        AstAggregateName::Min
        | AstAggregateName::Max
        | AstAggregateName::Sum
        | AstAggregateName::Avg => "unknown",
    }
}

pub(super) fn aggregate_function_name(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count => "count",
        AstAggregateName::Exists => "exists",
        AstAggregateName::DistinctCount => "distinct_count",
        AstAggregateName::Sum => "sum",
        AstAggregateName::Avg => "avg",
        AstAggregateName::Min => "min",
        AstAggregateName::Max => "max",
    }
}

pub(super) fn builtin_function_execution_class(
    name: &str,
    args: &[ResolvedExpr],
) -> FunctionExecutionClass {
    match name {
        "isNull" | "isNotNull" => FunctionExecutionClass::CodedSafe,
        "coalesce" if coalesce_bool_args_are_coded_safe(args) => FunctionExecutionClass::CodedSafe,
        "cast" if cast_args_are_identity(args) => FunctionExecutionClass::CodedSafe,
        _ => FunctionExecutionClass::MaterializedOnly,
    }
}

pub(super) fn coalesce_bool_args_are_coded_safe(args: &[ResolvedExpr]) -> bool {
    !args.is_empty()
        && args.iter().all(|arg| {
            matches!(
                expr_logical_type(arg).map(normalized_function_type),
                Some("bool" | "null")
            )
        })
}

pub(super) fn cast_args_are_identity(args: &[ResolvedExpr]) -> bool {
    let Some(value_type) = args.first().and_then(expr_logical_type) else {
        return false;
    };
    let Some(target_type) = cast_target_logical_type(args) else {
        return false;
    };
    normalized_function_type(value_type) == normalized_function_type(target_type)
}

pub(super) fn normalized_function_type(value: &str) -> &str {
    match value {
        "bool" | "boolean" => "bool",
        "string" | "utf8" => "utf8",
        "null" => "null",
        other => other,
    }
}

pub(super) fn is_window_function_name(name: &str) -> bool {
    matches!(
        name,
        "row_number"
            | "rank"
            | "dense_rank"
            | "lag"
            | "lead"
            | "first_value"
            | "last_value"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "count"
    )
}

pub(super) fn window_aggregate_function_name(name: AstAggregateName) -> Option<&'static str> {
    match name {
        AstAggregateName::Count => Some("count"),
        AstAggregateName::Sum => Some("sum"),
        AstAggregateName::Avg => Some("avg"),
        AstAggregateName::Min => Some("min"),
        AstAggregateName::Max => Some("max"),
        AstAggregateName::Exists | AstAggregateName::DistinctCount => None,
    }
}

pub(super) fn function_logical_type(name: &str, args: &[ResolvedExpr]) -> String {
    match name {
        "startsWith" | "exists" | "isNull" | "isNotNull" => "bool".into(),
        "length" | "row_number" | "rank" | "dense_rank" | "count" => "uint64".into(),
        "sum" | "avg" => "float64".into(),
        "lag" | "lead" | "first_value" | "last_value" | "min" | "max" => args
            .first()
            .and_then(expr_logical_type)
            .unwrap_or("unknown")
            .into(),
        "cast" => cast_target_logical_type(args).unwrap_or("unknown").into(),
        "coalesce" => coalesce_common_logical_type(args)
            .unwrap_or("unknown")
            .into(),
        "identity" => args
            .first()
            .and_then(expr_logical_type)
            .unwrap_or("unknown")
            .into(),
        "trim" | "lower" | "lowercase" | "upper" | "uppercase" => "utf8".into(),
        _ => "unknown".into(),
    }
}

pub(super) fn function_physical_kind(name: &str, args: &[ResolvedExpr]) -> &'static str {
    match name {
        "startsWith" | "isNull" | "isNotNull" => "boolean",
        "length" | "row_number" | "rank" | "dense_rank" | "count" | "sum" | "avg" => "num_code",
        "lag" | "lead" | "first_value" | "last_value" | "min" | "max" => args
            .first()
            .and_then(expr_logical_type)
            .map(physical_kind_for_logical_name)
            .unwrap_or("var_bytes"),
        "coalesce" => coalesce_common_logical_type(args)
            .map(physical_kind_for_logical_name)
            .unwrap_or("var_bytes"),
        "cast" => cast_target_logical_type(args)
            .map(physical_kind_for_logical_name)
            .unwrap_or("var_bytes"),
        _ => "var_bytes",
    }
}

pub(super) fn cast_target_logical_type(args: &[ResolvedExpr]) -> Option<&'static str> {
    let Some(ResolvedExpr::Literal(literal)) = args.get(1) else {
        return None;
    };
    let ResolvedLiteralValue::String(target) = &literal.typed_value else {
        return None;
    };
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

pub(super) fn canonicalize_null_check_predicate(expr: ResolvedExpr) -> ResolvedPredicate {
    match expr {
        ResolvedExpr::FunctionCall {
            function_id,
            mut args,
            ..
        } if matches!(function_id.as_str(), "isNull" | "isNotNull") && args.len() == 1 => {
            ResolvedPredicate::NullCheck {
                expr: args.remove(0),
                negated: function_id == "isNotNull",
            }
        }
        expr => ResolvedPredicate::BoolExpr(expr),
    }
}

pub(super) fn declares_unicode_or_collation_dependency(dependency: &str) -> bool {
    let dependency = dependency.to_ascii_lowercase();
    dependency.contains("unicode") || dependency.contains("collation")
}

pub(super) fn declares_exact_string_function_dependency(name: &str, dependency: &str) -> bool {
    let dependency = dependency.to_ascii_lowercase();
    if !declares_unicode_or_collation_dependency(&dependency) {
        return false;
    }
    match name {
        "startsWith" => {
            dependency.contains("startswith")
                && dependency.contains("exact")
                && (dependency.contains("accelerator") || dependency.contains("prefix"))
        }
        "length" => {
            dependency.contains("length")
                && dependency.contains("exact")
                && (dependency.contains("logical") || dependency.contains("encoded"))
        }
        _ => false,
    }
}

pub(super) fn declares_safe_cast_dependency(dependency: &str) -> bool {
    let dependency = dependency.to_ascii_lowercase();
    dependency.contains("safe-cast") || dependency.contains("safe_cast")
}

pub(super) fn timestamp_micros(
    value: &str,
    security: &SecurityContext,
) -> Result<(i64, String), BuildResolvedQueryError> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
        BuildResolvedQueryError::single(diagnostic(
            "E_LITERAL",
            "timestamp literals must be RFC3339 strings with an explicit offset",
            "resolve",
            security,
        ))
    })?;
    let nanos = timestamp.unix_timestamp_nanos();
    let micros = i64::try_from(nanos / 1_000).map_err(|_| {
        BuildResolvedQueryError::single(diagnostic(
            "E_LITERAL",
            "timestamp literal is outside supported microsecond range",
            "resolve",
            security,
        ))
    })?;
    let canonical = timestamp.format(&Rfc3339).map_err(|_| {
        BuildResolvedQueryError::single(diagnostic(
            "E_LITERAL",
            "failed to canonicalize timestamp literal",
            "resolve",
            security,
        ))
    })?;
    Ok((micros, canonical))
}

impl Resolver {
    pub(super) fn resolve_predicate(
        &self,
        predicate: &Spanned<AstPredicate>,
        root: &ResolvedRoot,
    ) -> Result<ResolvedPredicate, BuildResolvedQueryError> {
        match &predicate.node {
            AstPredicate::Compare { left, op, right } => {
                let mut left = self.resolve_expr(left, root)?;
                let mut right = self.resolve_expr(right, root)?;
                self.coerce_compare_literals(&mut left, &mut right)?;
                Ok(ResolvedPredicate::Compare {
                    left,
                    op: *op,
                    right,
                })
            }
            AstPredicate::InList { expr, values } => {
                let expr = self.resolve_expr(expr, root)?;
                let target_type = expr_logical_type(&expr).map(str::to_owned);
                let values = values
                    .iter()
                    .map(|literal| {
                        let literal = self.resolve_literal(&literal.node)?;
                        if let Some(target_type) = target_type.as_deref() {
                            self.coerce_literal_to_type(literal, target_type)
                        } else {
                            Ok(literal)
                        }
                    })
                    .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?;
                Ok(ResolvedPredicate::InList { expr, values })
            }
            AstPredicate::NullCheck { expr, negated } => Ok(ResolvedPredicate::NullCheck {
                expr: self.resolve_expr(expr, root)?,
                negated: *negated,
            }),
            AstPredicate::Exists { target, args } => {
                if !args.is_empty() || matches!(target.node, AstExpr::RootBinding(_)) {
                    let expr = self.resolve_profile_exists(target, args, root)?;
                    self.validate_exists_disclosure(&expr, false)?;
                    return Ok(ResolvedPredicate::Exists(expr));
                }
                if let AstExpr::Relationship(relationship) = &target.node {
                    let expr = ResolvedExpr::Association(
                        self.resolve_relationship_association(relationship, root)?,
                    );
                    self.validate_exists_disclosure(&expr, false)?;
                    return Ok(ResolvedPredicate::Exists(expr));
                }
                let expr = self.resolve_expr(target, root)?;
                self.validate_exists_disclosure(&expr, false)?;
                Ok(ResolvedPredicate::Exists(expr))
            }
            AstPredicate::BoolExpr(expr) => {
                let expr = self.resolve_expr(expr, root)?;
                Ok(canonicalize_null_check_predicate(expr))
            }
            AstPredicate::Not(inner) => {
                let inner = self.resolve_predicate(inner, root)?;
                self.validate_predicate_disclosure(&inner, true)?;
                Ok(ResolvedPredicate::Not(Box::new(inner)))
            }
            AstPredicate::And(parts) => Ok(ResolvedPredicate::And(
                parts
                    .iter()
                    .map(|part| self.resolve_predicate(part, root))
                    .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
            )),
            AstPredicate::Or(parts) => Ok(ResolvedPredicate::Or(
                parts
                    .iter()
                    .map(|part| self.resolve_predicate(part, root))
                    .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
            )),
        }
    }

    pub(super) fn resolve_profile_exists(
        &self,
        target: &Spanned<AstExpr>,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedExpr, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Table(_)) {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "profile-aware exists(root, on: ...) requires a declared bridge from the current root grain to CoveQL/Table",
                &self.options,
            ));
        }
        let AstExpr::RootBinding(binding) = &target.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "profile-aware exists requires a table(...) root binding target",
                &self.options,
            ));
        };
        let mut right_root = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut right_root, binding.alias.as_ref());
        let ResolvedRoot::Table(right) = right_root else {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "profile-aware exists target requires a declared bridge to a CoveQL/Table surface",
                &self.options,
            ));
        };
        let mut on = None;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (Some("on"), AstProfileArgumentValue::Predicate(predicate)) if on.is_none() => {
                    on = Some(predicate);
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported exists argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "profile-aware exists requires one named on: predicate",
                        &self.options,
                    ));
                }
            }
        }
        let on = on.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "profile-aware exists requires an on: predicate",
                &self.options,
            )
        })?;
        let scoped = self.clone_for_lookup_scope(right.clone());
        let on = scoped.resolve_predicate(on, root)?;
        scoped.validate_table_lookup_predicate(&on)?;
        Ok(ResolvedExpr::TableExists(ResolvedTableExists {
            right,
            on: Box::new(on),
        }))
    }

    pub(super) fn clone_for_lookup_scope(&self, right: ResolvedTableRoot) -> Self {
        let mut resolver = Self {
            surface: self.surface.clone(),
            options: self.options.clone(),
            diagnostics: Vec::new(),
            lookup_scope: self.lookup_scope.clone(),
            cte_table_scope: self.cte_table_scope.clone(),
            graph_node_scope: self.graph_node_scope.clone(),
            graph_edge_scope: self.graph_edge_scope.clone(),
            window_function_scope: self.window_function_scope,
        };
        resolver.lookup_scope.push(right);
        resolver
    }

    pub(super) fn resolve_expr(
        &self,
        expr: &Spanned<AstExpr>,
        root: &ResolvedRoot,
    ) -> Result<ResolvedExpr, BuildResolvedQueryError> {
        match &expr.node {
            AstExpr::Path(path) => Ok(ResolvedExpr::Path(self.resolve_path(path, root)?)),
            AstExpr::Literal(literal) => Ok(ResolvedExpr::Literal(self.resolve_literal(literal)?)),
            AstExpr::FunctionCall { name, args } => self.resolve_function(name, args, root),
            AstExpr::AggregateCall { name, arg, star } => {
                if self.window_function_scope {
                    let function_id = window_aggregate_function_name(*name).ok_or_else(|| {
                        self.function_contract_error(
                            "aggregate is not a supported CoveQL window function",
                            aggregate_function_name(*name),
                        )
                    })?;
                    let mut resolved_args = Vec::new();
                    if let Some(arg) = arg.as_deref() {
                        resolved_args.push(self.resolve_expr(arg, root)?);
                    } else if !star && *name != AstAggregateName::Count {
                        return Err(self.function_contract_error(
                            "window aggregate requires an argument",
                            function_id,
                        ));
                    }
                    let contract = self.resolve_function_contract(function_id, &resolved_args)?;
                    return Ok(ResolvedExpr::FunctionCall {
                        function_id: function_id.into(),
                        deterministic: contract.deterministic,
                        logical_type: function_logical_type(function_id, &resolved_args),
                        physical_kind: function_physical_kind(function_id, &resolved_args).into(),
                        contract,
                        args: resolved_args,
                    });
                }
                let arg = arg
                    .as_deref()
                    .map(|arg| self.resolve_expr(arg, root).map(Box::new))
                    .transpose()?;
                if let Some(arg) = arg.as_deref() {
                    self.validate_aggregate_disclosure(*name, arg)?;
                }
                Ok(ResolvedExpr::AggregateCall {
                    name: *name,
                    arg,
                    star: *star,
                    logical_type: aggregate_logical_type(*name).into(),
                    aggregate_disclosure: format!(
                        "{:?}",
                        self.options.security.aggregate_disclosure_policy
                    )
                    .to_ascii_lowercase(),
                })
            }
            AstExpr::Association(association) => {
                Ok(ResolvedExpr::Association(self.resolve_association_root(
                    association,
                    Some(root),
                    AssociationResolveUsage::ObjectRelative,
                )?))
            }
            AstExpr::Evidence(evidence) => Ok(ResolvedExpr::Evidence(
                self.resolve_evidence_root(evidence, Some(root))?,
            )),
            AstExpr::Relationship(relationship) => Ok(ResolvedExpr::Association(
                self.resolve_relationship_association(relationship, root)?,
            )),
            AstExpr::RootBinding(_) => Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE",
                "profile root expressions are valid only where a profile method, evidence target, or exists expression declares root-binding semantics",
                &self.options,
            )),
            AstExpr::Conditional {
                predicate,
                then_expr,
                else_expr,
            } => {
                let predicate = self.resolve_predicate(predicate, root)?;
                let then_expr = self.resolve_expr(then_expr, root)?;
                let else_expr = self.resolve_expr(else_expr, root)?;
                let logical_type = common_output_logical_type(&then_expr, &else_expr).into();
                Ok(ResolvedExpr::Conditional {
                    predicate: Box::new(predicate),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                    logical_type,
                })
            }
        }
    }

    pub(super) fn coerce_compare_literals(
        &self,
        left: &mut ResolvedExpr,
        right: &mut ResolvedExpr,
    ) -> Result<(), BuildResolvedQueryError> {
        let left_type = expr_logical_type(left).map(str::to_owned);
        let right_type = expr_logical_type(right).map(str::to_owned);
        if let (ResolvedExpr::Literal(literal), Some(target_type)) = (left, right_type.as_deref()) {
            *literal = self.coerce_literal_to_type(literal.clone(), target_type)?;
        }
        if let (Some(target_type), ResolvedExpr::Literal(literal)) = (left_type.as_deref(), right) {
            *literal = self.coerce_literal_to_type(literal.clone(), target_type)?;
        }
        Ok(())
    }

    pub(super) fn coerce_literal_to_type(
        &self,
        literal: ResolvedLiteral,
        target_type: &str,
    ) -> Result<ResolvedLiteral, BuildResolvedQueryError> {
        match target_type {
            "timestamp_micros" => {
                let AstLiteral::String(value) = &literal.literal else {
                    return Ok(literal);
                };
                let (micros, canonical_rfc3339) = timestamp_micros(value, &self.options.security)?;
                Ok(ResolvedLiteral {
                    literal: AstLiteral::Timestamp(value.clone()),
                    logical_type: "timestamp_micros".into(),
                    canonical: format!("{canonical_rfc3339}:{micros}"),
                    typed_value: ResolvedLiteralValue::TimestampMicros {
                        micros,
                        canonical_rfc3339,
                    },
                    precision: None,
                    scale: None,
                })
            }
            "uuid" => match &literal.literal {
                AstLiteral::String(value) | AstLiteral::Uuid(value) => {
                    let Some((bytes, canonical_hex)) = parse_uuid_literal(value) else {
                        return Err(BuildResolvedQueryError::single(diagnostic(
                            "E_LITERAL",
                            "malformed UUID literal",
                            "resolve",
                            &self.options.security,
                        )));
                    };
                    Ok(ResolvedLiteral {
                        literal: AstLiteral::Uuid(value.clone()),
                        logical_type: "uuid".into(),
                        canonical: canonical_hex.clone(),
                        typed_value: ResolvedLiteralValue::Uuid {
                            canonical_hex,
                            bytes,
                        },
                        precision: None,
                        scale: None,
                    })
                }
                _ => Ok(literal),
            },
            "binary" => match &literal.literal {
                AstLiteral::String(value) => {
                    let bytes = value.as_bytes().to_vec();
                    let canonical_hex = crate::hex_lower(&bytes);
                    Ok(ResolvedLiteral {
                        literal: AstLiteral::Binary(canonical_hex.clone()),
                        logical_type: "binary".into(),
                        canonical: canonical_hex.clone(),
                        typed_value: ResolvedLiteralValue::Binary {
                            canonical_hex,
                            bytes,
                        },
                        precision: None,
                        scale: None,
                    })
                }
                AstLiteral::Binary(value) => {
                    let Some(bytes) = decode_hex_bytes(value) else {
                        return Err(BuildResolvedQueryError::single(diagnostic(
                            "E_LITERAL",
                            "malformed binary literal",
                            "resolve",
                            &self.options.security,
                        )));
                    };
                    let canonical_hex = value.to_ascii_lowercase();
                    Ok(ResolvedLiteral {
                        literal: AstLiteral::Binary(canonical_hex.clone()),
                        logical_type: "binary".into(),
                        canonical: canonical_hex.clone(),
                        typed_value: ResolvedLiteralValue::Binary {
                            canonical_hex,
                            bytes,
                        },
                        precision: None,
                        scale: None,
                    })
                }
                _ => Ok(literal),
            },
            _ => Ok(literal),
        }
    }

    pub(super) fn resolve_function(
        &self,
        name: &AstIdentifier,
        args: &[Spanned<AstExpr>],
        root: &ResolvedRoot,
    ) -> Result<ResolvedExpr, BuildResolvedQueryError> {
        let resolved_args = args
            .iter()
            .map(|arg| self.resolve_expr(arg, root))
            .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?;
        let contract = self.resolve_function_contract(&name.name, &resolved_args)?;
        Ok(ResolvedExpr::FunctionCall {
            function_id: name.name.clone(),
            deterministic: contract.deterministic,
            logical_type: function_logical_type(&name.name, &resolved_args),
            physical_kind: function_physical_kind(&name.name, &resolved_args).into(),
            contract,
            args: resolved_args,
        })
    }

    pub(super) fn resolve_function_contract(
        &self,
        name: &str,
        args: &[ResolvedExpr],
    ) -> Result<ResolvedFunctionContract, BuildResolvedQueryError> {
        self.validate_builtin_function_signature(name, args)?;
        match name {
            "coalesce" | "isNull" | "isNotNull" => {
                let execution_class = builtin_function_execution_class(name, args);
                Ok(ResolvedFunctionContract {
                    function_id: name.into(),
                    version: "coveql-0.1".into(),
                    deterministic: true,
                    dependency: "pure".into(),
                    execution_class,
                    unicode_or_collation_contract: None,
                })
            }
            "startsWith" | "length" => {
                if let Some(function) = self.deterministic_function_entry(name) {
                    if declares_exact_string_function_dependency(name, &function.dependency) {
                        return Ok(ResolvedFunctionContract {
                            function_id: function.function_id.clone(),
                            version: function.version.clone(),
                            deterministic: true,
                            dependency: function.dependency.clone(),
                            execution_class: FunctionExecutionClass::CodedSafe,
                            unicode_or_collation_contract: Some(function.dependency.clone()),
                        });
                    }
                }
                Ok(ResolvedFunctionContract {
                    function_id: name.into(),
                    version: "coveql-0.1".into(),
                    deterministic: true,
                    dependency: "materialized-string-built-in".into(),
                    execution_class: FunctionExecutionClass::MaterializedOnly,
                    unicode_or_collation_contract: None,
                })
            }
            "cast" => {
                let execution_class = builtin_function_execution_class(name, args);
                if execution_class == FunctionExecutionClass::CodedSafe {
                    return Ok(ResolvedFunctionContract {
                        function_id: name.into(),
                        version: "coveql-0.1".into(),
                        deterministic: true,
                        dependency: "cove-map-safe-cast:coded-identity".into(),
                        execution_class,
                        unicode_or_collation_contract: None,
                    });
                }
                let Some(function) = self.deterministic_function_entry(name) else {
                    return Err(self.function_contract_error(
                        "non-identity cast requires deterministic COVE-MAP safe-cast metadata",
                        name,
                    ));
                };
                if !declares_safe_cast_dependency(&function.dependency) {
                    return Err(self.function_contract_error(
                        "non-identity cast requires COVE-MAP safe-cast dependency metadata",
                        name,
                    ));
                }
                Ok(ResolvedFunctionContract {
                    function_id: function.function_id.clone(),
                    version: function.version.clone(),
                    deterministic: true,
                    dependency: function.dependency.clone(),
                    execution_class,
                    unicode_or_collation_contract: None,
                })
            }
            "identity" | "trim" | "lower" | "lowercase" | "upper" | "uppercase" => {
                let Some(function) = self.deterministic_function_entry(name) else {
                    return Err(self.function_contract_error(
                        "function requires deterministic COVE-MAP metadata",
                        name,
                    ));
                };
                if matches!(name, "trim" | "lower" | "lowercase" | "upper" | "uppercase")
                    && !declares_unicode_or_collation_dependency(&function.dependency)
                {
                    return Err(self.function_contract_error(
                        "function requires Unicode/collation version metadata in the function dependency",
                        name,
                    ));
                }
                Ok(ResolvedFunctionContract {
                    function_id: function.function_id.clone(),
                    version: function.version.clone(),
                    deterministic: true,
                    dependency: function.dependency.clone(),
                    execution_class: FunctionExecutionClass::CodedSafe,
                    unicode_or_collation_contract: Some(function.dependency.clone()),
                })
            }
            "exists" => Ok(ResolvedFunctionContract {
                function_id: name.into(),
                version: "coveql-0.1".into(),
                deterministic: true,
                dependency: "pure".into(),
                execution_class: FunctionExecutionClass::MaterializedOnly,
                unicode_or_collation_contract: None,
            }),
            name if is_window_function_name(name) => {
                if !self.window_function_scope {
                    return Err(self.function_contract_error(
                        "window function requires a window(...) method",
                        name,
                    ));
                }
                Ok(ResolvedFunctionContract {
                    function_id: name.into(),
                    version: "coveql-relational-window-1".into(),
                    deterministic: true,
                    dependency: "materialized-window-frame".into(),
                    execution_class: FunctionExecutionClass::MaterializedOnly,
                    unicode_or_collation_contract: None,
                })
            }
            _ if self.deterministic_function_entry(name).is_some() => Err(self
                .function_contract_error(
                    "registered deterministic function has no CoveQL executable body",
                    name,
                )),
            _ => Err(self.unknown(
                "E_UNSUPPORTED_FUNCTION",
                "function is not registered as deterministic COVE-MAP metadata",
                Some(name),
            )),
        }
    }

    pub(super) fn validate_builtin_function_signature(
        &self,
        name: &str,
        args: &[ResolvedExpr],
    ) -> Result<(), BuildResolvedQueryError> {
        let arity_ok = match name {
            "coalesce" => !args.is_empty(),
            "identity" | "trim" | "lower" | "lowercase" | "upper" | "uppercase" | "length"
            | "exists" | "isNull" | "isNotNull" => args.len() == 1,
            "row_number" | "rank" | "dense_rank" => args.is_empty(),
            "lag" | "lead" => args.len() == 1 || args.len() == 2,
            "first_value" | "last_value" | "sum" | "avg" | "min" | "max" => args.len() == 1,
            "count" => args.len() <= 1,
            "startsWith" | "cast" => args.len() == 2,
            _ => true,
        };
        if !arity_ok {
            return Err(self.function_contract_error("invalid function arity", name));
        }
        let string_arg = |arg: &ResolvedExpr| {
            matches!(
                expr_logical_type(arg),
                Some("utf8" | "string" | "json" | "unknown")
            )
        };
        let types_ok = match name {
            "coalesce" => coalesce_common_logical_type(args).is_some(),
            "trim" | "lower" | "lowercase" | "upper" | "uppercase" => {
                args.first().is_some_and(string_arg)
            }
            "startsWith" => args.iter().all(string_arg),
            "cast" => cast_target_logical_type(args).is_some(),
            _ => true,
        };
        if !types_ok {
            return Err(self.function_contract_error(
                "function argument types do not satisfy the deterministic function contract",
                name,
            ));
        }
        Ok(())
    }

    pub(super) fn deterministic_function_entry(
        &self,
        name: &str,
    ) -> Option<&cove_core::profile::cove_map::MapFunctionEntry> {
        self.surface
            .embedded_map_sections
            .iter()
            .find_map(|section| match section {
                EmbeddedMapSection::FunctionRegistry(registry) => registry
                    .functions
                    .iter()
                    .find(|function| function.function_id == name && function.deterministic),
                _ => None,
            })
    }

    pub(super) fn function_contract_error(
        &self,
        message: &'static str,
        name: &str,
    ) -> BuildResolvedQueryError {
        let mut diagnostic = diagnostic(
            "E_FUNCTION_CONTRACT",
            message,
            "resolve",
            &self.options.security,
        );
        if self.options.security.metadata_disclosure_policy
            == MetadataDisclosurePolicy::AllowProtected
        {
            diagnostic.safe_details = json!({ "function_id": name });
            diagnostic.redacted = false;
        }
        BuildResolvedQueryError {
            diagnostics: vec![diagnostic],
            rejections: vec![RejectionReport {
                kind: RejectionKind::FeatureValidation,
                reason: message.into(),
            }],
            source: None,
        }
    }

    pub(super) fn validate_exists_disclosure(
        &self,
        expr: &ResolvedExpr,
        negated: bool,
    ) -> Result<(), BuildResolvedQueryError> {
        match expr {
            ResolvedExpr::Association(_) if negated && !self.protected_metadata_allowed() => {
                Err(self.disclosure_error(
                    "E_PROTECTED_ASSOCIATION_EXISTENCE",
                    "association non-existence disclosure requires protected metadata disclosure permission",
                ))
            }
            ResolvedExpr::Evidence(_) if !self.protected_metadata_allowed() => {
                Err(self.disclosure_error(
                    "E_PROTECTED_EVIDENCE_EXISTENCE",
                    "evidence existence disclosure requires protected metadata disclosure permission",
                ))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn validate_predicate_disclosure(
        &self,
        predicate: &ResolvedPredicate,
        negated: bool,
    ) -> Result<(), BuildResolvedQueryError> {
        match predicate {
            ResolvedPredicate::Exists(expr) => self.validate_exists_disclosure(expr, negated),
            ResolvedPredicate::Not(inner) => self.validate_predicate_disclosure(inner, !negated),
            ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
                for part in parts {
                    self.validate_predicate_disclosure(part, negated)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(super) fn validate_aggregate_disclosure(
        &self,
        name: AstAggregateName,
        arg: &ResolvedExpr,
    ) -> Result<(), BuildResolvedQueryError> {
        if !matches!(
            name,
            AstAggregateName::Count | AstAggregateName::Exists | AstAggregateName::DistinctCount
        ) {
            return Ok(());
        }
        match self.options.security.aggregate_disclosure_policy {
            crate::AggregateDisclosurePolicy::AllowExact
            | crate::AggregateDisclosurePolicy::AllowMaterializedOnly
            | crate::AggregateDisclosurePolicy::AllowThresholded
            | crate::AggregateDisclosurePolicy::AllowRedacted => {}
            crate::AggregateDisclosurePolicy::Reject => {
                return Err(self.disclosure_error(
                    "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
                    "association/evidence aggregate disclosure is forbidden by the active policy",
                ))
            }
        }
        if !self.protected_metadata_allowed() {
            if matches!(arg, ResolvedExpr::Association(_)) {
                return Err(self.disclosure_error(
                    "E_PROTECTED_ASSOCIATION_EXISTENCE",
                    "association count disclosure requires protected metadata disclosure permission",
                ));
            }
            if matches!(arg, ResolvedExpr::Evidence(_)) {
                return Err(self.disclosure_error(
                    "E_PROTECTED_EVIDENCE_EXISTENCE",
                    "evidence count disclosure requires protected metadata disclosure permission",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn protected_metadata_allowed(&self) -> bool {
        self.options.security.metadata_disclosure_policy == MetadataDisclosurePolicy::AllowProtected
    }

    pub(super) fn disclosure_error(
        &self,
        code: &'static str,
        message: &'static str,
    ) -> BuildResolvedQueryError {
        BuildResolvedQueryError {
            diagnostics: vec![diagnostic(code, message, "resolve", &self.options.security)],
            rejections: vec![RejectionReport {
                kind: RejectionKind::SecurityPolicy,
                reason: message.into(),
            }],
            source: None,
        }
    }
}
