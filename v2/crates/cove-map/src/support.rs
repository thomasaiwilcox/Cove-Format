use std::collections::{BTreeMap, BTreeSet};

pub(crate) use cove_core::utility::hex_encode;
use cove_core::{
    artifact::covemap::CovemapFile,
    constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind},
    profile::cove_o::{PropertyEntryV1, RecordKind},
    types::{encoding_kind_for_physical, logical_type_from_name as parse_logical_type_name},
    utility::hex_decode_exact,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{sections::section_kind, SourceRow};

pub(crate) fn logical_type_from_name(name: &str) -> Result<CoveLogicalType, String> {
    parse_logical_type_name(name).map_err(|_| format!("unsupported COVE-MAP logical type '{name}'"))
}

pub(crate) fn physical_for_logical(logical: CoveLogicalType) -> CovePhysicalKind {
    match logical {
        CoveLogicalType::Bool => CovePhysicalKind::Boolean,
        CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json => {
            CovePhysicalKind::VarBytes
        }
        CoveLogicalType::Uuid | CoveLogicalType::Decimal128 | CoveLogicalType::Decimal64 => {
            CovePhysicalKind::FixedBytes
        }
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            CovePhysicalKind::FileCode
        }
        _ => CovePhysicalKind::NumCode,
    }
}

pub(crate) fn record_kind_from_name(name: &str) -> Result<RecordKind, String> {
    match name {
        "delta" | "Delta" => Ok(RecordKind::Delta),
        "snapshot" | "Snapshot" => Ok(RecordKind::Snapshot),
        "baseline" | "Baseline" | "upsert" | "Upsert" => Ok(RecordKind::Baseline),
        "tombstone" | "Tombstone" => Ok(RecordKind::Tombstone),
        other => Err(format!("unsupported COVE-O record kind '{other}'")),
    }
}

pub(crate) fn encoding_for_physical(physical: CovePhysicalKind) -> CoveEncodingKind {
    encoding_kind_for_physical(physical)
}

pub(crate) fn json_bool(value: &Value) -> Result<bool, String> {
    match value {
        Value::Bool(value) => Ok(*value),
        Value::String(text) if text.eq_ignore_ascii_case("true") => Ok(true),
        Value::String(text) if text.eq_ignore_ascii_case("false") => Ok(false),
        _ => Err("property value is not a bool".into()),
    }
}

pub(crate) fn json_numcode(value: &Value) -> Result<u64, String> {
    match value {
        Value::Bool(value) => Ok(u64::from(*value)),
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|value| u64::try_from(value).ok()))
            .ok_or_else(|| "numeric property value is outside supported NumCode range".to_string()),
        Value::String(text) => text
            .parse::<u64>()
            .map_err(|_| format!("'{text}' is not a supported NumCode value")),
        _ => Err("property value is not numeric".into()),
    }
}

pub(crate) fn fixed_bytes_for_property(
    property: &PropertyEntryV1,
    value: &Value,
) -> Result<Vec<u8>, String> {
    match property.logical_type {
        CoveLogicalType::Uuid => {
            let text = value
                .as_str()
                .ok_or_else(|| "uuid property values must be hex strings".to_string())?;
            Ok(hex_decode_16(text)?.to_vec())
        }
        CoveLogicalType::Decimal128 => {
            let int = value
                .as_i64()
                .map(i128::from)
                .or_else(|| value.as_str().and_then(|text| text.parse::<i128>().ok()))
                .ok_or_else(|| "decimal128 property value must be an integer".to_string())?;
            Ok(int.to_le_bytes().to_vec())
        }
        CoveLogicalType::Decimal64 => {
            let int = value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
                .ok_or_else(|| "decimal64 property value must be an integer".to_string())?;
            Ok(int.to_le_bytes().to_vec())
        }
        other => Err(format!("unsupported fixed-bytes logical type '{other:?}'")),
    }
}

pub(crate) fn var_bytes_for_property(
    property: &PropertyEntryV1,
    value: &Value,
) -> Result<Vec<u8>, String> {
    match property.logical_type {
        CoveLogicalType::Utf8 => value
            .as_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| "utf8 property value must be a string".to_string()),
        CoveLogicalType::Json => serde_json::to_vec(value).map_err(|err| err.to_string()),
        CoveLogicalType::Binary => value
            .as_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| "binary property value must be encoded as a string".to_string()),
        other => Err(format!("unsupported var-bytes logical type '{other:?}'")),
    }
}

pub(crate) fn stable_u32(text: &str, fallback: u32) -> u32 {
    let digest = Sha256::digest(text.as_bytes());
    let value = u32::from_le_bytes(digest[..4].try_into().unwrap());
    if value == 0 {
        fallback
    } else {
        value
    }
}

pub(crate) fn section_set(file: &CovemapFile) -> BTreeSet<String> {
    file.sections
        .iter()
        .map(|section| section_kind(section.entry.section_id))
        .collect()
}

pub(crate) fn object_to_btree(object: &Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub(crate) fn row_digest(row: &SourceRow) -> String {
    sha256_hex(canonical_row_json(&row.values).as_bytes())
}

pub(crate) fn schema_fingerprint(row: &SourceRow) -> String {
    let schema = row
        .values
        .iter()
        .map(|(key, value)| format!("{key}:{}", json_logical_type_name(value)))
        .collect::<Vec<_>>()
        .join("|");
    sha256_hex(schema.as_bytes())
}

fn canonical_row_json(values: &BTreeMap<String, Value>) -> String {
    serde_json::to_string(values).expect("BTreeMap JSON serialization cannot fail")
}

pub(crate) fn json_logical_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_i64() => "int64",
        Value::Number(number) if number.is_u64() => "uint64",
        Value::Number(_) => "float64",
        Value::String(_) => "utf8",
        Value::Array(_) => "list",
        Value::Object(_) => "struct",
    }
}

pub(crate) fn json_i64(value: &Value) -> Result<i64, String> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| "JSON number is not an i64".to_string()),
        Value::String(text) => text
            .parse::<i64>()
            .map_err(|_| format!("'{text}' is not an i64")),
        _ => Err("join key value is not an i64".into()),
    }
}

pub(crate) fn json_u64(value: &Value) -> Result<u64, String> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| "JSON number is not a u64".to_string()),
        Value::String(text) => text
            .parse::<u64>()
            .map_err(|_| format!("'{text}' is not a u64")),
        _ => Err("join key value is not a u64".into()),
    }
}

pub(crate) fn json_f64(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| "JSON number is not a finite f64".to_string()),
        Value::String(text) => text
            .parse::<f64>()
            .map_err(|_| format!("'{text}' is not an f64")),
        _ => Err("join key value is not an f64".into()),
    }
}

pub(crate) fn json_i128(value: &Value) -> Result<i128, String> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(|value| value as i128))
            .ok_or_else(|| "JSON number is not an i128-compatible integer".to_string()),
        Value::String(text) => text
            .parse::<i128>()
            .map_err(|_| format!("'{text}' is not an i128")),
        _ => Err("value is not an i128".into()),
    }
}

pub(crate) fn json_string(value: &Value) -> Result<&str, String> {
    value
        .as_str()
        .ok_or_else(|| "value must be a string".to_string())
}

pub(crate) fn json_uuid(value: &Value) -> Result<[u8; 16], String> {
    hex_decode_16(json_string(value)?)
}

pub(crate) fn append_len_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

pub(crate) fn sha256_array(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn first_16(bytes: &[u8; 32]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&bytes[..16]);
    out
}

pub(crate) fn hex_decode_16(text: &str) -> Result<[u8; 16], String> {
    let text = text.trim();
    if text.len() != 32 {
        return Err("uuid hex string must contain 32 hex characters".into());
    }
    hex_decode_exact::<16>(text).map_err(|_| "invalid hex character".into())
}

pub(crate) fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("fixture.{key} must be a string"))
}
