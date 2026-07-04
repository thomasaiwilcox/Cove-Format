use std::collections::BTreeMap;

use serde_json::Value;

use crate::CoveError;

use super::{ident::coveql_identifier, model::QueryDiscoveryManifest, query_discovery_error};

pub fn render_query_discovery_template(
    manifest: &QueryDiscoveryManifest,
    template_id: &str,
    params: &[(String, String)],
) -> Result<String, CoveError> {
    if !manifest.automation_allowed() {
        return Err(query_discovery_error(
            "human-only best-effort query-discovery manifests must not guide template expansion",
        ));
    }
    let template = manifest
        .value()
        .get("templates")
        .and_then(Value::as_array)
        .and_then(|templates| {
            templates
                .iter()
                .find(|template| template.get("id").and_then(Value::as_str) == Some(template_id))
        })
        .ok_or_else(|| {
            query_discovery_error(format!("unknown query-discovery template `{template_id}`"))
        })?;
    if template.get("binding_mode").and_then(Value::as_str) != Some("typed_ast_fragments") {
        return Err(query_discovery_error(format!(
            "template `{template_id}` does not use typed AST fragment binding"
        )));
    }
    let root = template
        .get("operator_chain")
        .and_then(Value::as_array)
        .and_then(|chain| chain.first())
        .ok_or_else(|| {
            query_discovery_error(format!(
                "template `{template_id}` has no operator_chain root"
            ))
        })?;
    if root.get("method").and_then(Value::as_str) != Some("root") {
        return Err(query_discovery_error(format!(
            "template `{template_id}` operator_chain must start with a root method"
        )));
    }
    match root.get("kind").and_then(Value::as_str) {
        Some("table") => render_table_template(manifest.value(), template, params),
        Some("object") => render_object_template(manifest.value(), template, params),
        Some("projection") => render_projection_template(manifest.value(), template, params),
        Some(kind) => Err(query_discovery_error(format!(
            "template `{template_id}` uses unsupported root kind `{kind}`"
        ))),
        None => Err(query_discovery_error(format!(
            "template `{template_id}` root method is missing kind"
        ))),
    }
}

fn render_table_template(
    manifest: &Value,
    template: &Value,
    params: &[(String, String)],
) -> Result<String, CoveError> {
    require_operator_methods(template, &["root", "where", "select", "take"])?;
    let param_map = query_template_param_map(params)?;
    let table_param = template_parameter(template, "table")?;
    let table = select_root_identifier(
        table_param,
        param_map.get("table").map(String::as_str),
        "table",
    )?;
    let root = format!("table({table})");
    let allowed_columns = root_scoped_template_identifiers(template, "columns", &root)?;
    if allowed_columns.is_empty() {
        return Err(query_discovery_error(format!(
            "template table root `{root}` has no selectable columns"
        )));
    }
    let filterable_columns = root_scoped_template_identifiers(template, "predicate", &root)?;
    let columns = select_identifier_list(
        param_map.get("columns").map(String::as_str),
        &allowed_columns,
        3,
        "columns",
    )?;
    let predicates = params
        .iter()
        .filter_map(|(name, value)| {
            table_filter_param_name(name).map(|field_name| (field_name, value))
        })
        .map(|(field_name, value)| {
            let field = normalize_query_identifier(field_name);
            if !filterable_columns.contains(&field) {
                return Err(query_discovery_error(format!(
                    "template parameter `{field_name}` is not selectable/filterable for `{root}`"
                )));
            }
            let literal = render_literal_for_field(manifest, "tables", &root, &field, value)?;
            Ok(format!("{field} == {literal}"))
        })
        .collect::<Result<Vec<_>, CoveError>>()?;
    if predicates.is_empty() {
        return Err(query_discovery_error(
            "table_filter_select_take requires at least one field=value predicate parameter",
        ));
    }
    let limit = render_template_limit(template, param_map.get("limit").map(String::as_str))?;
    Ok(format!(
        "{root}.where({}).select({}).take({limit})",
        predicates.join(" && "),
        columns.join(", ")
    ))
}

fn render_object_template(
    manifest: &Value,
    template: &Value,
    params: &[(String, String)],
) -> Result<String, CoveError> {
    require_operator_methods(template, &["root", "select", "take"])?;
    let param_map = query_template_param_map(params)?;
    for (name, _) in params {
        if !matches!(name.as_str(), "object_type" | "properties" | "limit") {
            return Err(query_discovery_error(format!(
                "object_select_take does not accept parameter `{name}`"
            )));
        }
    }
    let object_param = template_parameter(template, "object_type")?;
    let object_type = select_root_identifier(
        object_param,
        param_map.get("object_type").map(String::as_str),
        "object_type",
    )?;
    let root = format!("object({object_type})");
    let allowed_properties = root_scoped_template_identifiers(template, "properties", &root)?;
    if allowed_properties.is_empty() {
        return Err(query_discovery_error(format!(
            "template object root `{root}` has no selectable properties"
        )));
    }
    let properties = select_identifier_list(
        param_map.get("properties").map(String::as_str),
        &allowed_properties,
        3,
        "properties",
    )?;
    for property in &properties {
        ensure_surface_field(manifest, "objects", &root, property)?;
    }
    let limit = render_template_limit(template, param_map.get("limit").map(String::as_str))?;
    Ok(format!(
        "{root}.select({}).take({limit})",
        properties.join(", ")
    ))
}

fn render_projection_template(
    manifest: &Value,
    template: &Value,
    params: &[(String, String)],
) -> Result<String, CoveError> {
    require_operator_methods(template, &["root", "select", "take"])?;
    let param_map = query_template_param_map(params)?;
    for (name, _) in params {
        if !matches!(name.as_str(), "projection" | "columns" | "limit") {
            return Err(query_discovery_error(format!(
                "projection_select_take does not accept parameter `{name}`"
            )));
        }
    }
    let projection_param = template_parameter(template, "projection")?;
    let projection = select_root_identifier(
        projection_param,
        param_map.get("projection").map(String::as_str),
        "projection",
    )?;
    let root = format!("projection({projection})");
    let allowed_columns = root_scoped_template_identifiers(template, "columns", &root)?;
    if allowed_columns.is_empty() {
        return Err(query_discovery_error(format!(
            "template projection root `{root}` has no selectable columns"
        )));
    }
    let columns = select_identifier_list(
        param_map.get("columns").map(String::as_str),
        &allowed_columns,
        3,
        "columns",
    )?;
    for column in &columns {
        ensure_surface_field(manifest, "projections", &root, column)?;
    }
    let limit = render_template_limit(template, param_map.get("limit").map(String::as_str))?;
    Ok(format!(
        "{root}.select({}).take({limit})",
        columns.join(", ")
    ))
}

fn query_template_param_map(
    params: &[(String, String)],
) -> Result<BTreeMap<String, String>, CoveError> {
    let mut map = BTreeMap::new();
    for (name, value) in params {
        if name.trim().is_empty() {
            return Err(query_discovery_error(
                "template parameter name must not be empty",
            ));
        }
        if map.insert(name.clone(), value.clone()).is_some() {
            return Err(query_discovery_error(format!(
                "duplicate template parameter `{name}`"
            )));
        }
    }
    Ok(map)
}

fn require_operator_methods(template: &Value, expected: &[&str]) -> Result<(), CoveError> {
    let methods = template
        .get("operator_chain")
        .and_then(Value::as_array)
        .ok_or_else(|| query_discovery_error("template has no operator_chain"))?
        .iter()
        .map(|entry| {
            entry
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    for method in expected {
        if !methods.contains(method) {
            let template_id = template
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            return Err(query_discovery_error(format!(
                "template `{template_id}` operator_chain is missing `{method}`"
            )));
        }
    }
    Ok(())
}

fn template_parameter<'a>(template: &'a Value, name: &str) -> Result<&'a Value, CoveError> {
    template
        .get("parameters")
        .and_then(Value::as_array)
        .and_then(|parameters| {
            parameters
                .iter()
                .find(|parameter| parameter.get("name").and_then(Value::as_str) == Some(name))
        })
        .ok_or_else(|| query_discovery_error(format!("template is missing `{name}` parameter")))
}

fn select_root_identifier(
    parameter: &Value,
    requested: Option<&str>,
    parameter_name: &str,
) -> Result<String, CoveError> {
    let allowed = string_array_field(parameter, "allowed_query_identifiers")?;
    if let Some(requested) = requested {
        let identifier = normalize_query_identifier(requested);
        if allowed.contains(&identifier) {
            return Ok(identifier);
        }
        return Err(query_discovery_error(format!(
            "`{requested}` is not an allowed {parameter_name} query identifier"
        )));
    }
    if allowed.len() == 1 {
        return Ok(allowed[0].clone());
    }
    Err(query_discovery_error(format!(
        "template parameter `{parameter_name}` is required when more than one root is available"
    )))
}

fn root_scoped_template_identifiers(
    template: &Value,
    parameter_name: &str,
    root: &str,
) -> Result<Vec<String>, CoveError> {
    let parameter = template_parameter(template, parameter_name)?;
    parameter
        .get("allowed_query_identifiers_by_root")
        .and_then(Value::as_object)
        .and_then(|by_root| by_root.get(root))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            query_discovery_error(format!(
                "template parameter `{parameter_name}` has no identifiers for root `{root}`"
            ))
        })?
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                query_discovery_error(format!(
                    "template parameter `{parameter_name}` contains a non-string identifier"
                ))
            })
        })
        .collect()
}

fn select_identifier_list(
    requested: Option<&str>,
    allowed: &[String],
    default_count: usize,
    parameter_name: &str,
) -> Result<Vec<String>, CoveError> {
    let selected = if let Some(requested) = requested {
        requested
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(normalize_query_identifier)
            .collect::<Vec<_>>()
    } else {
        allowed
            .iter()
            .take(default_count)
            .cloned()
            .collect::<Vec<_>>()
    };
    if selected.is_empty() {
        return Err(query_discovery_error(format!(
            "template parameter `{parameter_name}` must not be empty"
        )));
    }
    for identifier in &selected {
        if !allowed.contains(identifier) {
            return Err(query_discovery_error(format!(
                "`{identifier}` is not allowed for template parameter `{parameter_name}`"
            )));
        }
    }
    Ok(selected)
}

fn render_template_limit(template: &Value, requested: Option<&str>) -> Result<usize, CoveError> {
    let limit_parameter = template_parameter(template, "limit")?;
    let default = limit_parameter
        .get("default")
        .and_then(Value::as_u64)
        .unwrap_or(50) as usize;
    let max = limit_parameter
        .get("max")
        .and_then(Value::as_u64)
        .unwrap_or(500) as usize;
    let limit = match requested {
        Some(value) => value.parse::<usize>().map_err(|_| {
            query_discovery_error(format!(
                "template limit `{value}` is not a positive integer"
            ))
        })?,
        None => default,
    };
    if limit == 0 || limit > max {
        return Err(query_discovery_error(format!(
            "template limit must be between 1 and {max}"
        )));
    }
    Ok(limit)
}

fn render_literal_for_field(
    manifest: &Value,
    surface_kind: &str,
    root: &str,
    field_identifier: &str,
    raw: &str,
) -> Result<String, CoveError> {
    let field = surface_field(manifest, surface_kind, root, field_identifier)?;
    if raw == "null" {
        if field
            .get("nullable")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            return Ok("null".to_string());
        }
        return Err(query_discovery_error(format!(
            "`{field_identifier}` is not nullable"
        )));
    }
    let logical_type = field
        .get("logical_type")
        .and_then(Value::as_str)
        .unwrap_or("utf8");
    match logical_type {
        "bool" => match raw {
            "true" | "false" => Ok(raw.to_string()),
            _ => Err(query_discovery_error(format!(
                "`{field_identifier}` expects a bool literal"
            ))),
        },
        "int8" | "int16" | "int32" | "int64" => {
            raw.parse::<i64>().map(|_| raw.to_string()).map_err(|_| {
                query_discovery_error(format!("`{field_identifier}` expects an integer literal"))
            })
        }
        "uint8" | "uint16" | "uint32" | "uint64" => {
            raw.parse::<u64>().map(|_| raw.to_string()).map_err(|_| {
                query_discovery_error(format!(
                    "`{field_identifier}` expects an unsigned integer literal"
                ))
            })
        }
        "float32" | "float64" | "decimal64" | "decimal128" => {
            raw.parse::<f64>().map(|_| raw.to_string()).map_err(|_| {
                query_discovery_error(format!("`{field_identifier}` expects a numeric literal"))
            })
        }
        _ => serde_json::to_string(raw).map_err(|err| {
            query_discovery_error(format!("failed to render string literal: {err}"))
        }),
    }
}

fn ensure_surface_field(
    manifest: &Value,
    surface_kind: &str,
    root: &str,
    field_identifier: &str,
) -> Result<(), CoveError> {
    surface_field(manifest, surface_kind, root, field_identifier).map(|_| ())
}

fn surface_field<'a>(
    manifest: &'a Value,
    surface_kind: &str,
    root: &str,
    field_identifier: &str,
) -> Result<&'a Value, CoveError> {
    let field_array = match surface_kind {
        "tables" | "projections" => "columns",
        _ => "properties",
    };
    manifest
        .get("surfaces")
        .and_then(|surfaces| surfaces.get(surface_kind))
        .and_then(Value::as_array)
        .and_then(|surfaces| {
            surfaces
                .iter()
                .find(|surface| surface.get("root").and_then(Value::as_str) == Some(root))
        })
        .and_then(|surface| surface.get(field_array))
        .and_then(Value::as_array)
        .and_then(|fields| {
            fields.iter().find(|field| {
                field.get("query_identifier").and_then(Value::as_str) == Some(field_identifier)
            })
        })
        .ok_or_else(|| {
            query_discovery_error(format!(
                "manifest surface `{root}` does not expose field `{field_identifier}`"
            ))
        })
}

fn string_array_field(value: &Value, field: &str) -> Result<Vec<String>, CoveError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| query_discovery_error(format!("expected `{field}` string array")))?
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                query_discovery_error(format!("`{field}` contains a non-string value"))
            })
        })
        .collect()
}

fn normalize_query_identifier(raw: &str) -> String {
    let trimmed = raw.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('`') && trimmed.ends_with('`'))
    {
        trimmed.to_string()
    } else {
        coveql_identifier(trimmed)
    }
}

fn is_table_template_reserved_param(name: &str) -> bool {
    matches!(name, "table" | "columns" | "limit")
}

fn table_filter_param_name(name: &str) -> Option<&str> {
    if let Some(stripped) = name.strip_prefix("filter.") {
        return Some(stripped);
    }
    if let Some(stripped) = name.strip_prefix("where.") {
        return Some(stripped);
    }
    (!is_table_template_reserved_param(name)).then_some(name)
}
