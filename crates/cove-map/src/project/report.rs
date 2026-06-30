use cove_core::profile::cove_map::{
    MapProjectionCatalog, MapProjectionColumn, MapProjectionColumnLineage, MapProjectionEntry,
};
use serde_json::{json, Value};

pub(crate) fn projection_catalog_json_value(catalog: &MapProjectionCatalog) -> Value {
    json!({
        "mapping_id": catalog.mapping_id,
        "mapping_version": catalog.mapping_version,
        "projections": catalog.projections.iter().map(projection_entry_json_value).collect::<Vec<_>>()
    })
}

fn projection_entry_json_value(projection: &MapProjectionEntry) -> Value {
    let mut value = json!({
        "projection_id": projection.projection_id,
        "assertion_ids": projection.assertion_ids,
        "output_table": projection.output_table,
        "row_grain": projection.row_grain,
        "anchor": projection.anchor.as_ref().map(|anchor| json!({
            "object_type": anchor.object_type,
            "association_type": anchor.association_type,
        })),
        "temporal_mode": projection.temporal_mode,
        "columns": projection.columns.iter().map(projection_column_json_value).collect::<Vec<_>>(),
        "multi_value_policy": projection.multi_value_policy,
        "missing_policy": projection.missing_policy,
        "ordering": projection.ordering,
        "evidence_policy": projection.evidence_policy,
        "output_modes": projection.output_modes,
    });
    strip_null_json_fields(&mut value);
    value
}

fn projection_column_json_value(column: &MapProjectionColumn) -> Value {
    let mut value = json!({
        "name": column.name,
        "value": column.value,
        "logical_type": column.logical_type,
        "nested_shape": column.nested_shape,
        "conflict_policy": column.conflict_policy,
        "missing_policy": column.missing_policy,
        "lineage": column.lineage.as_ref().map(projection_column_lineage_json_value),
    });
    strip_null_json_fields(&mut value);
    value
}

fn projection_column_lineage_json_value(lineage: &MapProjectionColumnLineage) -> Value {
    json!({
        "source": lineage.source,
        "object_type_id": lineage.object_type_id,
        "object_type_name": lineage.object_type_name,
        "property_id": lineage.property_id,
        "property_name": lineage.property_name,
        "projection_table_id": lineage.projection_table_id,
        "projection_column_id": lineage.projection_column_id,
        "expression": lineage.expression,
        "transform": lineage.transform,
        "filter_pushdown": lineage.filter_pushdown,
    })
}

fn strip_null_json_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for value in object.values_mut() {
                strip_null_json_fields(value);
            }
            object.retain(|_, value| !value.is_null());
        }
        Value::Array(values) => {
            for value in values {
                strip_null_json_fields(value);
            }
        }
        _ => {}
    }
}
