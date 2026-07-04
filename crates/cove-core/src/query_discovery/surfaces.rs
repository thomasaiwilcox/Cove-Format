use serde_json::{json, Value};

use crate::{
    mount::{MountedColumn, MountedTable},
    profile::{
        cove_map::{MapProjectionColumn, MapProjectionEntry},
        cove_o::{CoveObjectSurface, ObjectTypeEntryV1, PropertyEntryV1},
    },
};

use super::{
    helpers::{
        column_operations, disclosure_label, logical_type_name, object_kind, rounded_count_hint,
    },
    ident::{coveql_identifier, object_root, projection_root, table_root},
    relationships::property_flag_labels,
    QueryDiscoveryOptions,
};

pub(super) fn table_surface_json(table: &MountedTable) -> Value {
    let root = table_root(&table.name);
    json!({
        "name": table.name,
        "query_name": table.name,
        "query_identifier": coveql_identifier(&table.name),
        "display_name": table.name,
        "root": root,
        "table_id": table.table_id.to_string(),
        "row_count_hint": {
            "value": rounded_count_hint(table.row_count).to_string(),
            "precision": if table.row_count <= 1000 { "exact" } else { "rounded" }
        },
        "authority": "materialized_coveql_authority",
        "columns": table.columns.iter().map(column_surface_json).collect::<Vec<_>>()
    })
}

pub(super) fn profile_contract_versions_json(profiles: &[&str]) -> Value {
    let mut versions = serde_json::Map::new();
    for profile in profiles {
        versions.insert((*profile).to_string(), json!("0.1"));
    }
    Value::Object(versions)
}

fn column_surface_json(column: &MountedColumn) -> Value {
    json!({
        "name": column.name,
        "query_name": column.name,
        "query_identifier": coveql_identifier(&column.name),
        "display_name": column.name,
        "column_id": column.column_id.to_string(),
        "logical_type": logical_type_name(column.logical),
        "nullable": column.nullable,
        "operations": column_operations(column.logical)
    })
}

pub(super) fn object_surface_json(object_type: &ObjectTypeEntryV1, row_count: usize) -> Value {
    let root = object_root(&object_type.type_name);
    json!({
        "type_name": object_type.type_name,
        "query_name": object_type.type_name,
        "query_identifier": coveql_identifier(&object_type.type_name),
        "display_name": object_type.type_name,
        "root": root,
        "object_type_id": object_type.object_type_id.to_string(),
        "kind": object_kind(object_type),
        "row_count_hint": {
            "value": rounded_count_hint(row_count as u64).to_string(),
            "precision": if row_count <= 1000 { "exact" } else { "rounded" }
        },
        "properties": object_type
            .properties
            .iter()
            .map(property_surface_json)
            .collect::<Vec<_>>()
    })
}

fn property_surface_json(property: &PropertyEntryV1) -> Value {
    json!({
        "name": property.property_name,
        "query_name": property.property_name,
        "query_identifier": coveql_identifier(&property.property_name),
        "display_name": property.property_name,
        "property_id": property.property_id.to_string(),
        "logical_type": logical_type_name(property.logical_type),
        "nullable": property.nullable,
        "operations": column_operations(property.logical_type)
    })
}

pub(super) fn projection_surface_json(projection: &MapProjectionEntry) -> Value {
    json!({
        "projection_id": projection.projection_id,
        "query_name": projection.projection_id,
        "query_identifier": coveql_identifier(&projection.projection_id),
        "display_name": projection.projection_id,
        "root": projection_root(&projection.projection_id),
        "output_table": &projection.output_table,
        "row_grain": &projection.row_grain,
        "columns": projection.columns.iter().map(projection_column_surface_json).collect::<Vec<_>>()
    })
}

fn projection_column_surface_json(column: &MapProjectionColumn) -> Value {
    json!({
        "name": column.name,
        "query_name": column.name,
        "query_identifier": coveql_identifier(&column.name),
        "display_name": column.name,
        "logical_type": &column.logical_type,
        "nullable": true,
        "operations": ["select", "filter", "order", "group"]
    })
}

pub(super) fn evidence_surface_json(object_type: &ObjectTypeEntryV1, row_count: usize) -> Value {
    json!({
        "root": format!("evidence({}, grain: object)", coveql_identifier(&object_type.type_name)),
        "grain": "object",
        "targets": [object_root(&object_type.type_name)],
        "columns": evidence_column_surface_json(),
        "row_count_hint": {
            "value": rounded_count_hint(row_count as u64).to_string(),
            "precision": if row_count <= 1000 { "exact" } else { "rounded" }
        }
    })
}

fn evidence_column_surface_json() -> Vec<Value> {
    [
        ("source_id", "utf8", false),
        ("source_row_identity", "utf8", false),
        ("rule_id", "utf8", false),
        ("assertion_id", "utf8", false),
        ("output_object_id", "utf8", false),
        ("observed_schema_fingerprint", "utf8", true),
        ("observed_snapshot_digest", "utf8", true),
    ]
    .into_iter()
    .map(|(name, logical_type, nullable)| {
        json!({
            "name": name,
            "query_name": name,
            "query_identifier": name,
            "display_name": name,
            "logical_type": logical_type,
            "nullable": nullable,
            "operations": ["select", "filter"]
        })
    })
    .collect()
}

pub(super) fn property_glossary_json(
    tables: &[MountedTable],
    object_surface: Option<&CoveObjectSurface>,
) -> Vec<Value> {
    let mut entries = Vec::new();
    for table in tables {
        let root = table_root(&table.name);
        for column in &table.columns {
            let identifier = coveql_identifier(&column.name);
            entries.push(property_glossary_entry_json(
                "table_column",
                &format!("{root}.{identifier}"),
                &column.name,
                logical_type_name(column.logical),
                column_operations(column.logical),
                "materialized_table_catalog",
            ));
        }
    }

    let Some(surface) = object_surface else {
        return entries;
    };
    for object_type in &surface.object_types {
        let root = object_root(&object_type.type_name);
        for property in &object_type.properties {
            let identifier = coveql_identifier(&property.property_name);
            let mut entry = property_glossary_entry_json(
                "object_property",
                &format!("{root}.{identifier}"),
                &property.property_name,
                logical_type_name(property.logical_type),
                column_operations(property.logical_type),
                "object_catalog",
            );
            if let Some(flags) = property_flag_labels(property) {
                entry["property_flags"] = flags;
            }
            entries.push(entry);
        }
    }
    if let Some(catalog) = &surface.projection_catalog {
        for projection in &catalog.projections {
            let root = projection_root(&projection.projection_id);
            for column in &projection.columns {
                let identifier = coveql_identifier(&column.name);
                let mut entry = property_glossary_entry_json(
                    "projection_column",
                    &format!("{root}.{identifier}"),
                    &column.name,
                    column.logical_type.as_deref().unwrap_or("unknown"),
                    vec!["select", "filter", "order", "group"],
                    "cove_map_projection_catalog",
                );
                if let Some(lineage) = &column.lineage {
                    entry["lineage"] = json!({
                        "source": lineage.source,
                        "object_type_id": lineage.object_type_id.to_string(),
                        "object_type_name": lineage.object_type_name,
                        "property_id": lineage.property_id.to_string(),
                        "property_name": lineage.property_name,
                        "projection_table_id": lineage.projection_table_id.to_string(),
                        "projection_column_id": lineage.projection_column_id.to_string(),
                        "expression": lineage.expression,
                        "transform": lineage.transform,
                        "filter_pushdown": lineage.filter_pushdown
                    });
                }
                entries.push(entry);
            }
        }
    }
    entries
}

fn property_glossary_entry_json(
    kind: &str,
    query_path: &str,
    display_name: &str,
    logical_type: &str,
    operations: Vec<&'static str>,
    authority: &str,
) -> Value {
    json!({
        "kind": kind,
        "path": query_path,
        "query_path": query_path,
        "query_identifier_path": query_path,
        "display_name": display_name,
        "description": format!("{display_name} exposed by {authority}."),
        "logical_type": logical_type,
        "safe_operations": operations,
        "sensitivity": "low",
        "value_examples": [],
        "policy_notes": [],
        "authority": authority
    })
}

pub(super) fn alias_bindings_json(
    tables: &[MountedTable],
    object_surface: Option<&CoveObjectSurface>,
    options: &QueryDiscoveryOptions,
) -> Vec<Value> {
    let mut bindings = Vec::new();
    for table in tables {
        push_alias_binding(
            &mut bindings,
            &table.name,
            &coveql_identifier(&table.name),
            "table",
            &table_root(&table.name),
            options,
        );
        let root = table_root(&table.name);
        for column in &table.columns {
            push_field_alias_binding(
                &mut bindings,
                &column.name,
                &coveql_identifier(&column.name),
                "column",
                &root,
                options,
            );
        }
    }

    let Some(surface) = object_surface else {
        return bindings;
    };
    for object_type in &surface.object_types {
        let root = object_root(&object_type.type_name);
        push_alias_binding(
            &mut bindings,
            &object_type.type_name,
            &coveql_identifier(&object_type.type_name),
            "object",
            &root,
            options,
        );
        for property in &object_type.properties {
            push_field_alias_binding(
                &mut bindings,
                &property.property_name,
                &coveql_identifier(&property.property_name),
                "property",
                &root,
                options,
            );
        }
    }
    if let Some(catalog) = &surface.projection_catalog {
        for projection in &catalog.projections {
            let root = projection_root(&projection.projection_id);
            push_alias_binding(
                &mut bindings,
                &projection.projection_id,
                &coveql_identifier(&projection.projection_id),
                "projection",
                &root,
                options,
            );
            for column in &projection.columns {
                push_field_alias_binding(
                    &mut bindings,
                    &column.name,
                    &coveql_identifier(&column.name),
                    "projection_column",
                    &root,
                    options,
                );
            }
        }
    }
    bindings
}

fn push_alias_binding(
    bindings: &mut Vec<Value>,
    alias: &str,
    query_identifier: &str,
    target_kind: &str,
    target_root: &str,
    options: &QueryDiscoveryOptions,
) {
    if alias == query_identifier {
        return;
    }
    bindings.push(json!({
        "alias": alias,
        "query_identifier": query_identifier,
        "target_kind": target_kind,
        "target_root": target_root,
        "policy_scope": disclosure_label(options.disclosure_mode)
    }));
}

fn push_field_alias_binding(
    bindings: &mut Vec<Value>,
    alias: &str,
    query_identifier: &str,
    target_kind: &str,
    target_root: &str,
    options: &QueryDiscoveryOptions,
) {
    if alias == query_identifier {
        return;
    }
    bindings.push(json!({
        "alias": alias,
        "query_identifier": query_identifier,
        "target_kind": target_kind,
        "target_root": target_root,
        "target_path": format!("{target_root}.{query_identifier}"),
        "policy_scope": disclosure_label(options.disclosure_mode)
    }));
}

pub(super) fn root_index_json(
    tables: &[MountedTable],
    object_surface: Option<&CoveObjectSurface>,
    include_evidence: bool,
) -> Vec<String> {
    let mut roots = tables
        .iter()
        .map(|table| table_root(&table.name))
        .collect::<Vec<_>>();
    if let Some(surface) = object_surface {
        roots.extend(
            surface
                .object_types
                .iter()
                .map(|object_type| object_root(&object_type.type_name)),
        );
        if let Some(catalog) = &surface.projection_catalog {
            roots.extend(
                catalog
                    .projections
                    .iter()
                    .map(|projection| projection_root(&projection.projection_id)),
            );
        }
        if include_evidence {
            roots.extend(surface.object_types.iter().map(|object_type| {
                format!(
                    "evidence({}, grain: object)",
                    coveql_identifier(&object_type.type_name)
                )
            }));
        }
    }
    roots
}
