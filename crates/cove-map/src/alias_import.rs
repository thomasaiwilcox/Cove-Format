use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapPayloadEncodingV2, CovemapSection, CovemapSectionEntryV1,
    },
    checksum,
    constants::SectionKind,
    durable,
};
use serde_json::{json, Map, Value};

use crate::{parse_map, sha256_hex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AliasImportOptions {
    pub(crate) catalog_id: String,
    pub(crate) resolver_id: String,
}

#[derive(Debug, Clone)]
struct AliasImportRow {
    canonical_key: String,
    canonical_label: String,
    alias: String,
    authority: String,
    confidence_class: String,
    metadata: Option<Value>,
}

#[derive(Debug, Default)]
struct AliasImportGroup {
    canonical_label: Option<String>,
    rows: Vec<AliasImportRow>,
    aliases: BTreeSet<String>,
}

pub(crate) fn import_aliases_from_paths(
    map_path: &Path,
    aliases_path: &Path,
    output_path: &Path,
    options: &AliasImportOptions,
) -> Result<Value, String> {
    let file = parse_map(map_path)?;
    let aliases = std::fs::read(aliases_path)
        .map_err(|err| format!("cannot read {}: {err}", aliases_path.display()))?;
    let (updated, mut report) = import_aliases_from_csv_bytes(&file, &aliases, options)?;
    let bytes = updated
        .serialize()
        .map_err(|err| format!("cannot serialize imported COVE-MAP: {err}"))?;
    CovemapFile::parse_validated(&bytes)
        .map_err(|err| format!("import produced invalid COVE-MAP: {err}"))?;
    durable::durable_replace(output_path, &bytes)
        .map_err(|err| format!("cannot durably write {}: {err}", output_path.display()))?;
    if let Value::Object(object) = &mut report {
        object.insert(
            "output".to_string(),
            Value::String(output_path.display().to_string()),
        );
    }
    Ok(report)
}

pub(crate) fn import_aliases_from_csv_bytes(
    file: &CovemapFile,
    csv_bytes: &[u8],
    options: &AliasImportOptions,
) -> Result<(CovemapFile, Value), String> {
    if options.catalog_id.trim().is_empty() {
        return Err("aliases import requires a non-empty --catalog-id".into());
    }
    if options.resolver_id.trim().is_empty() {
        return Err("aliases import requires a non-empty --resolver-id".into());
    }

    let rows = parse_alias_csv(csv_bytes)?;
    let alias_catalog = alias_catalog_value(&options.catalog_id, &rows)?;
    let mut updated = file.clone();
    let section_index = updated
        .sections
        .iter()
        .position(|section| section.entry.section_id == SectionKind::MapResolutionCatalog as u32)
        .ok_or_else(|| "aliases import requires an existing MAP_RESOLUTION_CATALOG".to_string())?;

    let mut payload: Value = serde_json::from_slice(&updated.sections[section_index].payload)
        .map_err(|err| format!("invalid MAP_RESOLUTION_CATALOG JSON: {err}"))?;
    let (catalog_digest, resolver_digest, entry_count, alias_count) =
        update_resolution_catalog_payload(&mut payload, &options.resolver_id, alias_catalog)?;

    let payload_bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|err| format!("cannot encode MAP_RESOLUTION_CATALOG JSON: {err}"))?;
    updated.sections[section_index] =
        covemap_section_with_entry(&updated.sections[section_index], payload_bytes);

    let report = json!({
        "ok": true,
        "catalog_id": options.catalog_id,
        "resolver_id": options.resolver_id,
        "alias_entry_count": entry_count,
        "alias_count": alias_count,
        "catalog_digest": catalog_digest,
        "resolver_digest": resolver_digest
    });
    Ok((updated, report))
}

fn parse_alias_csv(csv_bytes: &[u8]) -> Result<Vec<AliasImportRow>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(csv_bytes);
    let headers = reader
        .headers()
        .map_err(|err| format!("invalid alias CSV header: {err}"))?
        .clone();
    let canonical_key = header_index(&headers, "canonical_key")?;
    let canonical_label = header_index(&headers, "canonical_label")?;
    let alias = header_index(&headers, "alias")?;
    let authority = header_index(&headers, "authority")?;
    let confidence_class = header_index(&headers, "confidence_class")?;
    let metadata_json = header_index(&headers, "metadata_json")?;

    let mut rows = Vec::new();
    for (row_index, record) in reader.records().enumerate() {
        let record =
            record.map_err(|err| format!("invalid alias CSV row {}: {err}", row_index + 2))?;
        let row = AliasImportRow {
            canonical_key: required_csv_field(&record, canonical_key, row_index, "canonical_key")?,
            canonical_label: required_csv_field(
                &record,
                canonical_label,
                row_index,
                "canonical_label",
            )?,
            alias: required_csv_field(&record, alias, row_index, "alias")?,
            authority: required_csv_field(&record, authority, row_index, "authority")?,
            confidence_class: required_csv_field(
                &record,
                confidence_class,
                row_index,
                "confidence_class",
            )?,
            metadata: optional_metadata_json(&record, metadata_json, row_index)?,
        };
        rows.push(row);
    }
    if rows.is_empty() {
        return Err("alias CSV must contain at least one alias row".into());
    }
    Ok(rows)
}

fn header_index(headers: &csv::StringRecord, name: &str) -> Result<usize, String> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| format!("alias CSV missing required column '{name}'"))
}

fn required_csv_field(
    record: &csv::StringRecord,
    index: usize,
    row_index: usize,
    field: &str,
) -> Result<String, String> {
    let value = record.get(index).unwrap_or("").trim();
    if value.is_empty() {
        return Err(format!(
            "alias CSV row {} has empty required field '{}'",
            row_index + 2,
            field
        ));
    }
    Ok(value.to_string())
}

fn optional_metadata_json(
    record: &csv::StringRecord,
    index: usize,
    row_index: usize,
) -> Result<Option<Value>, String> {
    let value = record.get(index).unwrap_or("").trim();
    if value.is_empty() {
        return Ok(None);
    }
    let parsed: Value = serde_json::from_str(value).map_err(|err| {
        format!(
            "alias CSV row {} metadata_json is not valid JSON: {err}",
            row_index + 2
        )
    })?;
    if !parsed.is_object() {
        return Err(format!(
            "alias CSV row {} metadata_json must be a JSON object",
            row_index + 2
        ));
    }
    Ok(Some(parsed))
}

fn alias_catalog_value(catalog_id: &str, rows: &[AliasImportRow]) -> Result<Value, String> {
    let mut groups = BTreeMap::<String, AliasImportGroup>::new();
    let mut alias_targets = BTreeMap::<String, String>::new();
    for row in rows {
        match alias_targets.insert(row.alias.clone(), row.canonical_key.clone()) {
            Some(existing) if existing != row.canonical_key => {
                return Err(format!(
                    "alias '{}' maps to both '{}' and '{}'; ambiguous imports require explicit catalog editing",
                    row.alias, existing, row.canonical_key
                ));
            }
            _ => {}
        }

        let group = groups.entry(row.canonical_key.clone()).or_default();
        if group
            .canonical_label
            .as_ref()
            .is_some_and(|label| label != &row.canonical_label)
        {
            return Err(format!(
                "canonical_key '{}' has multiple canonical_label values",
                row.canonical_key
            ));
        }
        group.canonical_label = Some(row.canonical_label.clone());
        if !group.aliases.insert(row.alias.clone()) {
            return Err(format!(
                "duplicate alias '{}' for canonical_key '{}'",
                row.alias, row.canonical_key
            ));
        }
        group.rows.push(row.clone());
    }

    let mut entries = Vec::with_capacity(groups.len());
    for (canonical_key, group) in groups {
        let mut aliases = group.aliases.into_iter().collect::<Vec<_>>();
        aliases.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        let canonical_label = group
            .canonical_label
            .ok_or_else(|| format!("canonical_key '{canonical_key}' has no label"))?;
        let mut entry = Map::new();
        entry.insert(
            "alias_entry_id".to_string(),
            Value::String(format!("alias:{}", sha256_hex(canonical_key.as_bytes()))),
        );
        entry.insert("canonical_key".to_string(), Value::String(canonical_key));
        entry.insert(
            "canonical_label".to_string(),
            Value::String(canonical_label),
        );
        entry.insert(
            "aliases".to_string(),
            Value::Array(aliases.into_iter().map(Value::String).collect()),
        );
        let metadata = import_metadata(&group.rows);
        if !metadata.is_empty() {
            entry.insert("metadata".to_string(), Value::Object(metadata));
        }
        entries.push(Value::Object(entry));
    }

    Ok(json!({
        "alias_catalog_id": catalog_id,
        "entries": entries
    }))
}

fn import_metadata(rows: &[AliasImportRow]) -> Map<String, Value> {
    let authorities = rows
        .iter()
        .map(|row| row.authority.clone())
        .collect::<BTreeSet<_>>();
    let confidence_classes = rows
        .iter()
        .map(|row| row.confidence_class.clone())
        .collect::<BTreeSet<_>>();
    let import_rows = rows
        .iter()
        .map(|row| {
            let mut value = Map::new();
            value.insert("alias".to_string(), Value::String(row.alias.clone()));
            value.insert(
                "authority".to_string(),
                Value::String(row.authority.clone()),
            );
            value.insert(
                "confidence_class".to_string(),
                Value::String(row.confidence_class.clone()),
            );
            if let Some(metadata) = &row.metadata {
                value.insert("metadata_json".to_string(), metadata.clone());
            }
            Value::Object(value)
        })
        .collect::<Vec<_>>();

    let mut metadata = Map::new();
    metadata.insert(
        "authorities".to_string(),
        Value::Array(authorities.into_iter().map(Value::String).collect()),
    );
    metadata.insert(
        "confidence_classes".to_string(),
        Value::Array(confidence_classes.into_iter().map(Value::String).collect()),
    );
    metadata.insert("import_rows".to_string(), Value::Array(import_rows));
    metadata
}

fn update_resolution_catalog_payload(
    payload: &mut Value,
    resolver_id: &str,
    alias_catalog: Value,
) -> Result<(String, String, usize, usize), String> {
    let root = payload
        .as_object()
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG payload must be a JSON object".to_string())?;
    let resolver_index = resolver_index(root, resolver_id)?;
    let resolver = required_array(root, "resolvers")?[resolver_index]
        .as_object()
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG resolver must be an object".to_string())?;
    if required_str(resolver, "kind")? != "alias_catalog" {
        return Err(format!(
            "resolver '{resolver_id}' is not an alias_catalog resolver"
        ));
    }
    let normalization_pipeline_id = required_str(resolver, "normalization_pipeline_id")?;
    let pipeline_digest = pipeline_digest(root, normalization_pipeline_id)?;
    let order_sensitive_catalog = optional_bool(resolver, "order_sensitive_catalog", false)?;
    let catalog_digest = digest_json(&alias_catalog_digest_input(
        &alias_catalog,
        order_sensitive_catalog,
    )?)?;
    let resolver_digest = resolver_digest(resolver, &pipeline_digest, &catalog_digest)?;
    let (entry_count, alias_count) = alias_catalog_counts(&alias_catalog)?;

    let root = payload
        .as_object_mut()
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG payload must be a JSON object".to_string())?;
    let resolvers = root
        .get_mut("resolvers")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG missing resolvers array".to_string())?;
    let resolver = resolvers[resolver_index]
        .as_object_mut()
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG resolver must be an object".to_string())?;
    resolver.insert("alias_catalog".to_string(), alias_catalog);
    resolver.insert(
        "pipeline_digest".to_string(),
        Value::String(pipeline_digest),
    );
    resolver.insert(
        "catalog_digest".to_string(),
        Value::String(catalog_digest.clone()),
    );
    resolver.insert(
        "resolver_digest".to_string(),
        Value::String(resolver_digest.clone()),
    );

    Ok((catalog_digest, resolver_digest, entry_count, alias_count))
}

fn resolver_index(root: &Map<String, Value>, resolver_id: &str) -> Result<usize, String> {
    required_array(root, "resolvers")?
        .iter()
        .position(|value| {
            value
                .as_object()
                .and_then(|object| object.get("resolver_id"))
                .and_then(Value::as_str)
                == Some(resolver_id)
        })
        .ok_or_else(|| format!("MAP_RESOLUTION_CATALOG has no resolver '{resolver_id}'"))
}

fn pipeline_digest(root: &Map<String, Value>, pipeline_id: &str) -> Result<String, String> {
    let pipeline = required_array(root, "normalization_pipelines")?
        .iter()
        .find(|value| {
            value
                .as_object()
                .and_then(|object| object.get("pipeline_id"))
                .and_then(Value::as_str)
                == Some(pipeline_id)
        })
        .ok_or_else(|| {
            format!("MAP_RESOLUTION_CATALOG has no normalization pipeline '{pipeline_id}'")
        })?;
    digest_json(&pipeline_digest_input(pipeline)?)
}

fn pipeline_digest_input(pipeline: &Value) -> Result<Value, String> {
    let object = pipeline
        .as_object()
        .ok_or_else(|| "normalization pipeline must be an object".to_string())?;
    let pipeline_id = required_str(object, "pipeline_id")?;
    let functions = required_array(object, "functions")?
        .iter()
        .map(|value| {
            let function = value
                .as_object()
                .ok_or_else(|| "normalization function must be an object".to_string())?;
            let mut out = Map::new();
            out.insert(
                "function_id".to_string(),
                Value::String(required_str(function, "function_id")?.to_string()),
            );
            out.insert(
                "version".to_string(),
                Value::String(required_str(function, "version")?.to_string()),
            );
            if let Some(table_id) = optional_str(function, "table_id")? {
                out.insert("table_id".to_string(), Value::String(table_id.to_string()));
            }
            if let Some(digest) = optional_str(function, "suffix_table_digest")? {
                out.insert(
                    "suffix_table_digest".to_string(),
                    Value::String(digest.to_string()),
                );
            }
            Ok(Value::Object(out))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let table_values = optional_array(object, "tables")?;
    let table_values = table_values.map(Vec::as_slice).unwrap_or(&[]);
    let mut tables = table_values
        .iter()
        .map(|value| {
            let table = value
                .as_object()
                .ok_or_else(|| "normalization table must be an object".to_string())?;
            let mut out = Map::new();
            out.insert(
                "table_id".to_string(),
                Value::String(required_str(table, "table_id")?.to_string()),
            );
            out.insert(
                "digest".to_string(),
                Value::String(required_str(table, "digest")?.to_string()),
            );
            Ok(Value::Object(out))
        })
        .collect::<Result<Vec<_>, String>>()?;
    tables.sort_by(|left, right| {
        left.get("table_id")
            .and_then(Value::as_str)
            .cmp(&right.get("table_id").and_then(Value::as_str))
    });

    Ok(json!({
        "pipeline_id": pipeline_id,
        "functions": functions,
        "tables": tables
    }))
}

fn alias_catalog_digest_input(
    alias_catalog: &Value,
    order_sensitive_catalog: bool,
) -> Result<Value, String> {
    let catalog = alias_catalog
        .as_object()
        .ok_or_else(|| "alias_catalog must be an object".to_string())?;
    let alias_catalog_id = required_str(catalog, "alias_catalog_id")?;
    let mut entries = required_array(catalog, "entries")?.to_vec();
    if !order_sensitive_catalog {
        entries.sort_by(|left, right| {
            left.get("alias_entry_id")
                .and_then(Value::as_str)
                .cmp(&right.get("alias_entry_id").and_then(Value::as_str))
        });
    }

    let mut entry_values = Vec::with_capacity(entries.len());
    for entry_value in entries {
        let entry = entry_value
            .as_object()
            .ok_or_else(|| "alias_catalog entry must be an object".to_string())?;
        let mut aliases = required_array(entry, "aliases")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "alias value must be a string".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        if !order_sensitive_catalog {
            aliases.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        }

        let mut out = Map::new();
        out.insert(
            "alias_entry_id".to_string(),
            Value::String(required_str(entry, "alias_entry_id")?.to_string()),
        );
        out.insert(
            "canonical_key".to_string(),
            Value::String(required_str(entry, "canonical_key")?.to_string()),
        );
        out.insert(
            "canonical_label".to_string(),
            Value::String(required_str(entry, "canonical_label")?.to_string()),
        );
        out.insert(
            "aliases".to_string(),
            Value::Array(aliases.into_iter().map(Value::String).collect()),
        );
        if optional_bool(entry, "ambiguous", false)? {
            out.insert("ambiguous".to_string(), Value::Bool(true));
        }
        if let Some(metadata) = entry.get("metadata").and_then(Value::as_object) {
            if !metadata.is_empty() {
                out.insert("metadata".to_string(), Value::Object(metadata.clone()));
            }
        }
        entry_values.push(Value::Object(out));
    }

    Ok(json!({
        "alias_catalog_id": alias_catalog_id,
        "entries": entry_values
    }))
}

fn resolver_digest(
    resolver: &Map<String, Value>,
    pipeline_digest: &str,
    catalog_digest: &str,
) -> Result<String, String> {
    let input = json!({
        "resolver_id": required_str(resolver, "resolver_id")?,
        "kind": required_str(resolver, "kind")?,
        "object_type": required_str(resolver, "object_type")?,
        "authority": required_str(resolver, "authority")?,
        "confidence_class": required_str(resolver, "confidence_class")?,
        "normalization_pipeline_id": required_str(resolver, "normalization_pipeline_id")?,
        "pipeline_digest": pipeline_digest,
        "on_hit": required_str(resolver, "on_hit")?,
        "on_miss": required_str(resolver, "on_miss")?,
        "miss_confidence_class": optional_str(resolver, "miss_confidence_class")?,
        "ambiguous_policy": optional_str(resolver, "ambiguous_policy")?.unwrap_or("reject_auto_merge"),
        "catalog_digest": catalog_digest,
        "evidence_policy": optional_str(resolver, "evidence_policy")?.unwrap_or("retain_raw")
    });
    digest_json(&input)
}

fn alias_catalog_counts(alias_catalog: &Value) -> Result<(usize, usize), String> {
    let catalog = alias_catalog
        .as_object()
        .ok_or_else(|| "alias_catalog must be an object".to_string())?;
    let entries = required_array(catalog, "entries")?;
    let mut alias_count = 0usize;
    for entry in entries {
        let entry = entry
            .as_object()
            .ok_or_else(|| "alias_catalog entry must be an object".to_string())?;
        alias_count = alias_count
            .checked_add(required_array(entry, "aliases")?.len())
            .ok_or_else(|| "alias count overflow".to_string())?;
    }
    Ok((entries.len(), alias_count))
}

fn covemap_section_with_entry(existing: &CovemapSection, payload: Vec<u8>) -> CovemapSection {
    CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: SectionKind::MapResolutionCatalog as u32,
            offset: existing.entry.offset,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: existing.entry.compression,
            payload_encoding: if existing.entry.payload_encoding == 0 {
                CovemapPayloadEncodingV2::CoveMapJsonV2 as u8
            } else {
                existing.entry.payload_encoding
            },
            required: existing.entry.required,
            reserved: 0,
            checksum: checksum::crc32c(&payload),
        },
        payload,
    }
}

pub(crate) fn digest_json(value: &Value) -> Result<String, String> {
    Ok(format!("sha256:{}", sha256_hex(&canonical_json(value)?)))
}

fn canonical_json(value: &Value) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    write_canonical_json(value, &mut out)?;
    Ok(out)
}

fn write_canonical_json(value: &Value, out: &mut Vec<u8>) -> Result<(), String> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(value) => {
            let encoded = serde_json::to_string(value)
                .map_err(|err| format!("cannot encode canonical JSON string: {err}"))?;
            out.extend_from_slice(encoded.as_bytes());
        }
        Value::Array(values) => {
            out.push(b'[');
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                write_canonical_json(value, out)?;
            }
            out.push(b']');
        }
        Value::Object(object) => {
            out.push(b'{');
            let mut keys = object
                .keys()
                .filter(|key| key.as_str() != "non_semantic_metadata")
                .collect::<Vec<_>>();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (idx, key) in keys.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                let encoded_key = serde_json::to_string(key)
                    .map_err(|err| format!("cannot encode canonical JSON key: {err}"))?;
                out.extend_from_slice(encoded_key.as_bytes());
                out.push(b':');
                let value = object
                    .get(*key)
                    .ok_or_else(|| "canonical JSON object key disappeared".to_string())?;
                write_canonical_json(value, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn required_array<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Vec<Value>, String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("expected '{key}' array"))
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a Vec<Value>>, String> {
    object
        .get(key)
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| format!("expected '{key}' array"))
        })
        .transpose()
}

fn required_str<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("expected non-empty string field '{key}'"))
}

fn optional_str<'a>(object: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        Some(Value::String(_)) => Err(format!("expected non-empty string field '{key}'")),
        Some(_) => Err(format!("expected string field '{key}'")),
    }
}

fn optional_bool(object: &Map<String, Value>, key: &str, default: bool) -> Result<bool, String> {
    match object.get(key) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("expected bool field '{key}'")),
    }
}
