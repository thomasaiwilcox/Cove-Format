use serde_json::{json, Value};

use crate::{
    mount::MountedTable,
    profile::{cove_map::MapProjectionEntry, cove_o::ObjectTypeEntryV1},
};

use super::{
    helpers::column_operations,
    ident::{coveql_identifier, object_root, projection_root, table_root},
};

pub(super) fn ai_capability_json(include_ai: bool, table_roots: &[String]) -> Value {
    let methods = if include_ai && !table_roots.is_empty() {
        vec![json!({
            "name": "similar",
            "profiles": ["table", "ai"],
            "capabilities": ["similarity"],
            "targets": table_roots,
            "requires_sidecar": true,
            "authority": "validated_exact_flat_vector_scan_or_advisory_candidate_rerank",
            "example": format!(
                "# profiles: table, ai\n{}.similar(fileCode: 1, k: 5)",
                table_roots[0]
            )
        })]
    } else {
        Vec::new()
    };
    json!({
        "profile_available": include_ai,
        "sidecars": [],
        "methods": methods
    })
}

pub(super) fn table_filter_select_take_template(tables: &[MountedTable]) -> Option<Value> {
    let eligible_tables = tables
        .iter()
        .filter(|table| table_has_template_columns(table));
    let eligible_tables = eligible_tables.collect::<Vec<_>>();
    if eligible_tables.is_empty() {
        return None;
    }
    let allowed_tables = eligible_tables
        .iter()
        .map(|table| coveql_identifier(&table.name))
        .collect::<Vec<_>>();
    let filterable_by_root = root_scoped_column_identifiers(&eligible_tables, "filter");
    let selectable_by_root = root_scoped_column_identifiers(&eligible_tables, "select");

    Some(json!({
        "id": "table_filter_select_take",
        "title": "Filter and select rows from a table",
        "intent": "Find rows matching a simple predicate and return selected columns.",
        "profiles": ["table"],
        "binding_mode": "typed_ast_fragments",
        "template_display": "table({table}).where({predicate}).select({columns}).take({limit})",
        "operator_chain": [
            {"method": "root", "kind": "table", "arg": "{table}"},
            {"method": "where", "arg": "{predicate}"},
            {"method": "select", "arg": "{columns}"},
            {"method": "take", "arg": "{limit}"}
        ],
        "parameters": [
            {
                "name": "table",
                "kind": "root_name",
                "allowed_query_identifiers": allowed_tables
            },
            {
                "name": "predicate",
                "kind": "predicate",
                "root": "table({table})",
                "allowed_query_identifiers_by_root": filterable_by_root,
                "allowed_operators": ["eq", "neq", "lt", "lte", "gt", "gte", "and", "or"],
                "allowed_literal_types": ["utf8", "int64", "uint64", "float64", "bool", "null"],
                "max_depth": 4
            },
            {
                "name": "columns",
                "kind": "select_list",
                "root": "table({table})",
                "allowed_query_identifiers_by_root": selectable_by_root,
                "max_items": 8
            },
            {
                "name": "limit",
                "kind": "uint",
                "default": 50,
                "max": 500
            }
        ],
        "expected_output": "rows",
        "template_validation": "operator_chain_validated",
        "safety": {
            "requires_take": true,
            "allows_cross_root_lookup": false,
            "allows_ai_payloads": false
        }
    }))
}

pub(super) fn object_select_take_template(object_types: &[ObjectTypeEntryV1]) -> Value {
    let allowed_objects = object_types
        .iter()
        .map(|object_type| coveql_identifier(&object_type.type_name))
        .collect::<Vec<_>>();
    let allowed_by_root = root_scoped_property_identifiers(object_types);

    json!({
        "id": "object_select_take",
        "title": "Select properties from objects",
        "intent": "Return selected properties from a bounded object root.",
        "profiles": ["object"],
        "binding_mode": "typed_ast_fragments",
        "template_display": "object({object_type}).select({properties}).take({limit})",
        "operator_chain": [
            {"method": "root", "kind": "object", "arg": "{object_type}"},
            {"method": "select", "arg": "{properties}"},
            {"method": "take", "arg": "{limit}"}
        ],
        "parameters": [
            {
                "name": "object_type",
                "kind": "root_name",
                "allowed_query_identifiers": allowed_objects
            },
            {
                "name": "properties",
                "kind": "select_list",
                "root": "object({object_type})",
                "allowed_query_identifiers_by_root": allowed_by_root,
                "max_items": 8
            },
            {
                "name": "limit",
                "kind": "uint",
                "default": 50,
                "max": 500
            }
        ],
        "expected_output": "object_rows",
        "template_validation": "operator_chain_validated",
        "safety": {
            "requires_take": true,
            "allows_cross_root_lookup": false,
            "allows_ai_payloads": false
        }
    })
}

pub(super) fn projection_select_take_template(projections: &[MapProjectionEntry]) -> Value {
    let allowed_projections = projections
        .iter()
        .map(|projection| coveql_identifier(&projection.projection_id))
        .collect::<Vec<_>>();
    let allowed_by_root = root_scoped_projection_column_identifiers(projections);

    json!({
        "id": "projection_select_take",
        "title": "Select columns from a projection",
        "intent": "Return selected columns from a bounded projection root.",
        "profiles": ["object"],
        "binding_mode": "typed_ast_fragments",
        "template_display": "projection({projection}).select({columns}).take({limit})",
        "operator_chain": [
            {"method": "root", "kind": "projection", "arg": "{projection}"},
            {"method": "select", "arg": "{columns}"},
            {"method": "take", "arg": "{limit}"}
        ],
        "parameters": [
            {
                "name": "projection",
                "kind": "root_name",
                "allowed_query_identifiers": allowed_projections
            },
            {
                "name": "columns",
                "kind": "select_list",
                "root": "projection({projection})",
                "allowed_query_identifiers_by_root": allowed_by_root,
                "max_items": 8
            },
            {
                "name": "limit",
                "kind": "uint",
                "default": 50,
                "max": 500
            }
        ],
        "expected_output": "projection_rows",
        "template_validation": "operator_chain_validated",
        "safety": {
            "requires_take": true,
            "allows_cross_root_lookup": false,
            "allows_ai_payloads": false
        }
    })
}

pub(super) fn table_examples(tables: &[MountedTable]) -> Vec<Value> {
    tables
        .iter()
        .take(3)
        .map(|table| {
            let columns = table
                .columns
                .iter()
                .take(3)
                .map(|column| coveql_identifier(&column.name))
                .collect::<Vec<_>>();
            let query = if columns.is_empty() {
                format!("{}.take(50)", table_root(&table.name))
            } else {
                format!(
                    "{}.select({}).take(50)",
                    table_root(&table.name),
                    columns.join(", ")
                )
            };
            json!({
                "question": format!("Show rows from {}.", table.name),
                "profiles": ["table"],
                "query": query,
                "query_validation": "not_validated",
                "why": "Uses the mapped table surface for bounded flat output.",
                "expected_output": "rows"
            })
        })
        .collect()
}

pub(super) fn object_examples(object_types: &[ObjectTypeEntryV1]) -> Vec<Value> {
    object_types
        .iter()
        .take(3)
        .map(|object_type| {
            let properties = object_type
                .properties
                .iter()
                .take(3)
                .map(|property| coveql_identifier(&property.property_name))
                .collect::<Vec<_>>();
            let query = if properties.is_empty() {
                format!("{}.take(50)", object_root(&object_type.type_name))
            } else {
                format!(
                    "{}.select({}).take(50)",
                    object_root(&object_type.type_name),
                    properties.join(", ")
                )
            };
            json!({
                "question": format!("Show {} objects.", object_type.type_name),
                "profiles": ["object"],
                "query": query,
                "query_validation": "not_validated",
                "why": "Uses the object root rather than guessing source table names.",
                "expected_output": "object_rows"
            })
        })
        .collect()
}

pub(super) fn projection_examples(projections: &[MapProjectionEntry]) -> Vec<Value> {
    projections
        .iter()
        .take(3)
        .map(|projection| {
            let columns = projection
                .columns
                .iter()
                .take(3)
                .map(|column| coveql_identifier(&column.name))
                .collect::<Vec<_>>();
            let query = if columns.is_empty() {
                format!("{}.take(50)", projection_root(&projection.projection_id))
            } else {
                format!(
                    "{}.select({}).take(50)",
                    projection_root(&projection.projection_id),
                    columns.join(", ")
                )
            };
            json!({
                "question": format!("Show rows from projection {}.", projection.projection_id),
                "profiles": ["object"],
                "query": query,
                "query_validation": "not_validated",
                "why": "Uses the projection root rather than reconstructing source joins.",
                "expected_output": "projection_rows"
            })
        })
        .collect()
}

pub(super) fn evidence_examples(evidence_surfaces: &[Value]) -> Vec<Value> {
    evidence_surfaces
        .iter()
        .take(3)
        .filter_map(|surface| surface.get("root").and_then(Value::as_str))
        .map(|root| {
            json!({
                "question": format!("Show evidence rows for {root}."),
                "profiles": ["object"],
                "query": format!("{root}.select(source_id, source_row_identity).take(20)"),
                "query_validation": "not_validated",
                "why": "Uses the evidence root rather than guessing source table names.",
                "expected_output": "evidence_rows"
            })
        })
        .collect()
}

fn table_has_template_columns(table: &MountedTable) -> bool {
    table
        .columns
        .iter()
        .any(|column| column_operations(column.logical).contains(&"select"))
        && table
            .columns
            .iter()
            .any(|column| column_operations(column.logical).contains(&"filter"))
}

fn root_scoped_column_identifiers(
    tables: &[&MountedTable],
    operation: &str,
) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for table in tables {
        map.insert(
            table_root(&table.name),
            Value::Array(
                table
                    .columns
                    .iter()
                    .filter(|column| column_operations(column.logical).contains(&operation))
                    .map(|column| Value::String(coveql_identifier(&column.name)))
                    .collect(),
            ),
        );
    }
    map
}

fn root_scoped_property_identifiers(
    object_types: &[ObjectTypeEntryV1],
) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for object_type in object_types {
        map.insert(
            object_root(&object_type.type_name),
            Value::Array(
                object_type
                    .properties
                    .iter()
                    .map(|property| Value::String(coveql_identifier(&property.property_name)))
                    .collect(),
            ),
        );
    }
    map
}

fn root_scoped_projection_column_identifiers(
    projections: &[MapProjectionEntry],
) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for projection in projections {
        map.insert(
            projection_root(&projection.projection_id),
            Value::Array(
                projection
                    .columns
                    .iter()
                    .map(|column| Value::String(coveql_identifier(&column.name)))
                    .collect(),
            ),
        );
    }
    map
}
