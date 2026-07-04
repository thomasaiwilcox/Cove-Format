use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::profile::{
    cove_map::{
        EmbeddedMapSection, MapAssociationBinding, MapIdentityRuleCatalog, MapRowSemanticRule,
        MapRowSemanticsCatalog,
    },
    cove_o::{
        CoveObjectSurface, ObjectTypeEntryV1, PropertyEntryV1, PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
        PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        PROPERTY_FLAG_ASSOCIATION_TYPE, PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
        PROPERTY_FLAG_ASSOCIATION_VALID_TO, PROPERTY_FLAG_EVIDENCE_REF,
        PROPERTY_FLAG_MAPPING_RULE_REF,
    },
};

use super::{
    helpers::object_kind,
    ident::{coveql_identifier, object_root},
};

pub(super) fn relationship_records_json(object_surface: Option<&CoveObjectSurface>) -> Vec<Value> {
    let Some(surface) = object_surface else {
        return Vec::new();
    };
    let mut relationships = BTreeMap::<String, Value>::new();
    for object_type in &surface.object_types {
        if matches!(object_kind(object_type), "association") {
            if let Some((id, relationship)) = object_catalog_relationship_json(object_type) {
                relationships.insert(id, relationship);
            }
        }
    }
    for (id, relationship) in map_relationship_records_json(&surface.embedded_map_sections) {
        relationships.insert(id, relationship);
    }
    relationships.into_values().collect()
}

pub(super) fn object_catalog_relationship_json(
    object_type: &ObjectTypeEntryV1,
) -> Option<(String, Value)> {
    let endpoint_properties = association_endpoint_properties_json(object_type);
    if endpoint_properties.is_empty() {
        return None;
    }
    let association_root = association_root(&object_type.type_name);
    let mut select_identifiers = endpoint_properties
        .values()
        .filter_map(Value::as_str)
        .take(4)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if select_identifiers.is_empty() {
        select_identifiers.push("*".to_string());
    }
    let id = format!(
        "object_catalog_association_{}",
        discovery_id_fragment(&object_type.type_name)
    );
    Some((
        id.clone(),
        json!({
            "id": id,
            "description": format!("Association surface for {} endpoint metadata.", object_type.type_name),
            "association_root": association_root,
            "cardinality_hint": "unknown",
            "authority": "object_catalog_flags",
            "endpoint_properties": endpoint_properties,
            "example": format!(
                "{association_root}.select({}).take(20)",
                select_identifiers.join(", ")
            )
        }),
    ))
}

fn association_endpoint_properties_json(
    object_type: &ObjectTypeEntryV1,
) -> serde_json::Map<String, Value> {
    let mut endpoints = serde_json::Map::new();
    for property in &object_type.properties {
        let Some(kind) = association_property_kind(property) else {
            continue;
        };
        endpoints.insert(
            kind.to_string(),
            json!(coveql_identifier(&property.property_name)),
        );
    }
    endpoints
}

fn association_property_kind(property: &PropertyEntryV1) -> Option<&'static str> {
    let flags = property.flags;
    if flags & PROPERTY_FLAG_ASSOCIATION_FROM_GOID != 0 {
        Some("from")
    } else if flags & PROPERTY_FLAG_ASSOCIATION_TO_GOID != 0 {
        Some("to")
    } else if flags & PROPERTY_FLAG_ASSOCIATION_TYPE != 0 {
        Some("type")
    } else if flags & PROPERTY_FLAG_ASSOCIATION_VALID_FROM != 0 {
        Some("valid_from")
    } else if flags & PROPERTY_FLAG_ASSOCIATION_VALID_TO != 0 {
        Some("valid_to")
    } else if flags & PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT != 0 {
        Some("observed_at")
    } else if flags & PROPERTY_FLAG_EVIDENCE_REF != 0 {
        Some("evidence_ref")
    } else if flags & PROPERTY_FLAG_MAPPING_RULE_REF != 0 {
        Some("mapping_rule_ref")
    } else {
        None
    }
}

pub(super) fn property_flag_labels(property: &PropertyEntryV1) -> Option<Value> {
    let mut labels = Vec::new();
    if property.flags & PROPERTY_FLAG_ASSOCIATION_FROM_GOID != 0 {
        labels.push("association_from_goid");
    }
    if property.flags & PROPERTY_FLAG_ASSOCIATION_TO_GOID != 0 {
        labels.push("association_to_goid");
    }
    if property.flags & PROPERTY_FLAG_ASSOCIATION_TYPE != 0 {
        labels.push("association_type");
    }
    if property.flags & PROPERTY_FLAG_ASSOCIATION_VALID_FROM != 0 {
        labels.push("association_valid_from");
    }
    if property.flags & PROPERTY_FLAG_ASSOCIATION_VALID_TO != 0 {
        labels.push("association_valid_to");
    }
    if property.flags & PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT != 0 {
        labels.push("association_observed_at");
    }
    if property.flags & PROPERTY_FLAG_EVIDENCE_REF != 0 {
        labels.push("evidence_ref");
    }
    if property.flags & PROPERTY_FLAG_MAPPING_RULE_REF != 0 {
        labels.push("mapping_rule_ref");
    }
    (!labels.is_empty()).then(|| json!(labels))
}

pub(super) fn map_relationship_records_json(
    sections: &[EmbeddedMapSection],
) -> Vec<(String, Value)> {
    let identities = identity_rule_object_types(sections);
    let mut relationships = Vec::new();
    for section in sections {
        let EmbeddedMapSection::RowSemanticsCatalog(catalog) = section else {
            continue;
        };
        for rule in &catalog.rules {
            for binding in &rule.association_bindings {
                let id = map_relationship_id(rule, binding);
                relationships.push((
                    id.clone(),
                    map_relationship_json(catalog, rule, binding, &identities, &id),
                ));
            }
        }
    }
    relationships
}

fn identity_rule_object_types(sections: &[EmbeddedMapSection]) -> BTreeMap<String, String> {
    let mut identities = BTreeMap::new();
    for section in sections {
        if let EmbeddedMapSection::IdentityRuleCatalog(MapIdentityRuleCatalog {
            identity_rules,
            ..
        }) = section
        {
            for rule in identity_rules {
                identities.insert(rule.rule_id.clone(), rule.object_type.clone());
            }
        }
    }
    identities
}

fn map_relationship_json(
    catalog: &MapRowSemanticsCatalog,
    rule: &MapRowSemanticRule,
    binding: &MapAssociationBinding,
    identities: &BTreeMap<String, String>,
    id: &str,
) -> Value {
    let source_identity_rule_id = if binding.source_identity_rule_id.is_empty() {
        &rule.identity_rule_id
    } else {
        &binding.source_identity_rule_id
    };
    let source_object_type = identities.get(source_identity_rule_id);
    let target_object_type = identities.get(&binding.target_identity_rule_id);
    let association_root_text = association_root(&binding.association_type);
    let mut object = serde_json::Map::new();
    object.insert("id".to_string(), json!(id));
    object.insert(
        "description".to_string(),
        json!(format!(
            "COVE-MAP association binding {} from rule {}.",
            binding.association_type, rule.rule_id
        )),
    );
    if let Some(source_object_type) = source_object_type {
        object.insert(
            "from_root".to_string(),
            json!(object_root(source_object_type)),
        );
    }
    if let Some(target_object_type) = target_object_type {
        object.insert(
            "to_root".to_string(),
            json!(object_root(target_object_type)),
        );
    }
    object.insert("association_root".to_string(), json!(association_root_text));
    object.insert(
        "cardinality_hint".to_string(),
        json!(binding.cardinality_policy),
    );
    object.insert("authority".to_string(), json!("cove_map_row_semantics"));
    object.insert("mapping_id".to_string(), json!(catalog.mapping_id));
    object.insert(
        "mapping_version".to_string(),
        json!(catalog.mapping_version),
    );
    object.insert("rule_id".to_string(), json!(rule.rule_id));
    object.insert("assertion_id".to_string(), json!(binding.assertion_id));
    object.insert(
        "source_identity_rule_id".to_string(),
        json!(source_identity_rule_id),
    );
    object.insert(
        "target_identity_rule_id".to_string(),
        json!(binding.target_identity_rule_id),
    );
    object.insert("source_role".to_string(), json!(binding.source_role));
    object.insert("target_role".to_string(), json!(binding.target_role));
    object.insert(
        "source_endpoint_expression".to_string(),
        json!(binding.source_endpoint_expression),
    );
    object.insert(
        "target_endpoint_expression".to_string(),
        json!(binding.target_endpoint_expression),
    );
    if let Some(valid_from) = &binding.valid_from_expression {
        object.insert("valid_from_expression".to_string(), json!(valid_from));
    }
    if let Some(valid_to) = &binding.valid_to_expression {
        object.insert("valid_to_expression".to_string(), json!(valid_to));
    }
    let example = if let Some(source) = source_object_type {
        format!(
            "{}.where(exists({})).take(20)",
            object_root(source),
            association_root(&binding.association_type)
        )
    } else {
        format!("{}.take(20)", association_root(&binding.association_type))
    };
    object.insert("example".to_string(), json!(example));
    Value::Object(object)
}

fn map_relationship_id(rule: &MapRowSemanticRule, binding: &MapAssociationBinding) -> String {
    format!(
        "map_association_{}_{}_{}",
        discovery_id_fragment(&rule.rule_id),
        discovery_id_fragment(&binding.assertion_id),
        discovery_id_fragment(&binding.association_type)
    )
}

fn association_root(type_name: &str) -> String {
    format!("association({})", coveql_identifier(type_name))
}

fn discovery_id_fragment(value: &str) -> String {
    let mut fragment = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            fragment.push(ch.to_ascii_lowercase());
        } else if !fragment.ends_with('_') {
            fragment.push('_');
        }
    }
    let fragment = fragment.trim_matches('_');
    if fragment.is_empty() {
        "unnamed".to_string()
    } else {
        fragment.to_string()
    }
}
