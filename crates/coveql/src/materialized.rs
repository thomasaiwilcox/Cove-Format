use std::collections::{BTreeMap, BTreeSet};

use cove_core::profile::cove_o::{
    CoveObjectPropertyValue, CoveObjectRecord, CoveObjectState, CoveObjectTombstoneStatus,
    RecordKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{ResolvedExpr, ResolvedPath, ResolvedSystemField};

pub(crate) const INTERNAL_PROJECTION_FIELD_PREFIX: &str = "__coveql_";

pub(crate) fn window_function_key(name: &str, args: &[ResolvedExpr]) -> String {
    let args = serde_json::to_vec(args).unwrap_or_else(|_| format!("{args:?}").into_bytes());
    let digest = Sha256::digest(&args);
    let args_hash = hex(&digest[..16]);
    format!("{INTERNAL_PROJECTION_FIELD_PREFIX}window:{name}:{args_hash}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum OutputGrain {
    #[default]
    LatestState,
    HistoryRecord,
    HistoryState,
    ChangeRecord,
    ChangeStateTransition,
    ChangePropertyDiff,
    FinalObject,
    AssociationState,
    ProjectionRow,
    EvidenceRow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializedChangeDiffKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterializedChangeDetail {
    pub property_id: u32,
    pub property_name: String,
    pub old_value: Value,
    pub new_value: Value,
    pub diff_kind: MaterializedChangeDiffKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterializedObjectRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_file_ordinal: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_file_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_file_id: Option<String>,
    #[serde(default)]
    pub output_grain: OutputGrain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<MaterializedChangeDetail>,
    pub object_type_id: u32,
    pub object_type_name: String,
    pub branch_key: u64,
    pub goid: String,
    pub record_id: String,
    pub timestamp_us: i64,
    pub csn: u64,
    pub record_kind: String,
    pub tombstone_status: String,
    pub properties: BTreeMap<String, Value>,
    pub property_ids: BTreeMap<u32, String>,
    #[serde(default, skip_serializing)]
    pub redacted_properties: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterializedAssociationRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_file_ordinal: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_file_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_file_id: Option<String>,
    #[serde(default = "default_association_output_grain")]
    pub output_grain: OutputGrain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<MaterializedChangeDetail>,
    pub object_type_id: u32,
    pub association_type: Option<String>,
    pub branch_key: u64,
    pub goid: String,
    pub record_id: String,
    pub source_goid: Option<String>,
    pub target_goid: Option<String>,
    pub timestamp_us: i64,
    pub csn: u64,
    pub record_kind: String,
    pub tombstone_status: String,
    pub properties: BTreeMap<String, Value>,
    pub property_ids: BTreeMap<u32, String>,
    #[serde(default, skip_serializing)]
    pub redacted_properties: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterializedEvidenceRow {
    pub fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterializedProjectionRow {
    pub projection_id: String,
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExecutionRow {
    Object(MaterializedObjectRow),
    Association(MaterializedAssociationRow),
    Evidence(MaterializedEvidenceRow),
    Projection(MaterializedProjectionRow),
}

impl MaterializedObjectRow {
    pub(crate) fn from_record(record: &CoveObjectRecord) -> Self {
        let (properties, property_ids, redacted_properties) =
            materialized_properties(&record.properties);
        Self {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: OutputGrain::HistoryRecord,
            change: None,
            object_type_id: record.object_type_id,
            object_type_name: record.object_type_name.clone(),
            branch_key: record.branch_key,
            goid: hex(&record.goid),
            record_id: hex(&record.record_id),
            timestamp_us: record.timestamp_us,
            csn: record.csn,
            record_kind: record_kind_name(record.record_kind).into(),
            tombstone_status: tombstone_name(tombstone_status_for_record(record.record_kind))
                .into(),
            properties,
            property_ids,
            redacted_properties,
        }
    }

    pub(crate) fn from_state(state: &CoveObjectState) -> Self {
        let (properties, property_ids, redacted_properties) =
            materialized_properties(&state.properties);
        Self {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: OutputGrain::LatestState,
            change: None,
            object_type_id: state.object_type_id,
            object_type_name: state.object_type_name.clone(),
            branch_key: state.branch_key,
            goid: hex(&state.goid),
            record_id: hex(&state.latest_record_id),
            timestamp_us: state.timestamp_us,
            csn: state.csn,
            record_kind: record_kind_name(state.record_kind).into(),
            tombstone_status: tombstone_name(state.tombstone_status).into(),
            properties,
            property_ids,
            redacted_properties,
        }
    }

    pub(crate) fn with_output_grain(mut self, output_grain: OutputGrain) -> Self {
        self.output_grain = output_grain;
        self
    }

    pub(crate) fn with_change(mut self, change: MaterializedChangeDetail) -> Self {
        self.change = Some(change);
        self.output_grain = OutputGrain::ChangePropertyDiff;
        self
    }

    pub(crate) fn with_dataset_member(
        mut self,
        ordinal: usize,
        source: &str,
        file_id: String,
    ) -> Self {
        self.dataset_file_ordinal = Some(ordinal);
        self.dataset_file_source = Some(source.to_string());
        self.dataset_file_id = Some(file_id);
        self
    }

    pub(crate) fn value_for_path(&self, path: &ResolvedPath) -> Value {
        if let Some(system_field) = &path.system_field {
            return self.system_value(system_field);
        }
        if let Some(property_id) = path.property_id {
            if let Some(name) = self.property_ids.get(&property_id) {
                return self.properties.get(name).cloned().unwrap_or(Value::Null);
            }
        }
        self.properties
            .get(&path.display_name)
            .cloned()
            .unwrap_or(Value::Null)
    }

    pub(crate) fn system_value(&self, field: &ResolvedSystemField) -> Value {
        match field {
            ResolvedSystemField::Goid => Value::String(self.goid.clone()),
            ResolvedSystemField::ObjectType => Value::String(self.object_type_name.clone()),
            ResolvedSystemField::BranchKey => json!(self.branch_key),
            ResolvedSystemField::TimestampUs => json!(self.timestamp_us),
            ResolvedSystemField::Csn => json!(self.csn),
            ResolvedSystemField::RecordKind => Value::String(self.record_kind.clone()),
            _ => Value::Null,
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        let mut value = json!({
            "output_grain": self.output_grain,
            "object_type_id": self.object_type_id,
            "object_type_name": self.object_type_name,
            "branch_key": self.branch_key,
            "goid": self.goid,
            "record_id": self.record_id,
            "timestamp_us": self.timestamp_us,
            "csn": self.csn,
            "record_kind": self.record_kind,
            "tombstone_status": self.tombstone_status,
            "properties": self.properties,
        });
        if let Some(ordinal) = self.dataset_file_ordinal {
            value["dataset_file_ordinal"] = json!(ordinal);
        }
        if let Some(source) = &self.dataset_file_source {
            value["dataset_file_source"] = json!(source);
        }
        if let Some(file_id) = &self.dataset_file_id {
            value["dataset_file_id"] = json!(file_id);
        }
        if let Some(change) = &self.change {
            value["change"] = json!(change);
        }
        value
    }
}

impl MaterializedAssociationRow {
    pub(crate) fn from_record(record: &CoveObjectRecord) -> Option<Self> {
        let association = record.association.as_ref()?;
        let (properties, property_ids, redacted_properties) =
            materialized_properties(&record.properties);
        Some(Self {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: OutputGrain::HistoryRecord,
            change: None,
            object_type_id: record.object_type_id,
            association_type: association.association_type.clone(),
            branch_key: record.branch_key,
            goid: hex(&record.goid),
            record_id: hex(&record.record_id),
            source_goid: association.source_goid.clone(),
            target_goid: association.target_goid.clone(),
            timestamp_us: record.timestamp_us,
            csn: record.csn,
            record_kind: record_kind_name(record.record_kind).into(),
            tombstone_status: tombstone_name(tombstone_status_for_record(record.record_kind))
                .into(),
            properties,
            property_ids,
            redacted_properties,
        })
    }

    pub(crate) fn from_state(state: &CoveObjectState) -> Option<Self> {
        let association = state.association.as_ref()?;
        let (properties, property_ids, redacted_properties) =
            materialized_properties(&state.properties);
        Some(Self {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: OutputGrain::AssociationState,
            change: None,
            object_type_id: state.object_type_id,
            association_type: association.association_type.clone(),
            branch_key: state.branch_key,
            goid: hex(&state.goid),
            record_id: hex(&state.latest_record_id),
            source_goid: association.source_goid.clone(),
            target_goid: association.target_goid.clone(),
            timestamp_us: state.timestamp_us,
            csn: state.csn,
            record_kind: record_kind_name(state.record_kind).into(),
            tombstone_status: tombstone_name(state.tombstone_status).into(),
            properties,
            property_ids,
            redacted_properties,
        })
    }

    pub(crate) fn with_output_grain(mut self, output_grain: OutputGrain) -> Self {
        self.output_grain = output_grain;
        self
    }

    pub(crate) fn with_change(mut self, change: MaterializedChangeDetail) -> Self {
        self.change = Some(change);
        self.output_grain = OutputGrain::ChangePropertyDiff;
        self
    }

    pub(crate) fn with_dataset_member(
        mut self,
        ordinal: usize,
        source: &str,
        file_id: String,
    ) -> Self {
        self.dataset_file_ordinal = Some(ordinal);
        self.dataset_file_source = Some(source.to_string());
        self.dataset_file_id = Some(file_id);
        self
    }

    pub(crate) fn value_for_path(&self, path: &ResolvedPath) -> Value {
        if let Some(system_field) = &path.system_field {
            return self.system_value(system_field);
        }
        if let Some(property_id) = path.property_id {
            if let Some(name) = self.property_ids.get(&property_id) {
                return self.properties.get(name).cloned().unwrap_or(Value::Null);
            }
        }
        self.properties
            .get(&path.display_name)
            .cloned()
            .unwrap_or(Value::Null)
    }

    pub(crate) fn system_value(&self, field: &ResolvedSystemField) -> Value {
        match field {
            ResolvedSystemField::Goid => Value::String(self.goid.clone()),
            ResolvedSystemField::ObjectType | ResolvedSystemField::AssociationType => self
                .association_type
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            ResolvedSystemField::BranchKey => json!(self.branch_key),
            ResolvedSystemField::TimestampUs => json!(self.timestamp_us),
            ResolvedSystemField::Csn => json!(self.csn),
            ResolvedSystemField::RecordKind => Value::String(self.record_kind.clone()),
            ResolvedSystemField::SourceGoid => self
                .source_goid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            ResolvedSystemField::TargetGoid => self
                .target_goid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        let mut value = json!({
            "output_grain": self.output_grain,
            "object_type_id": self.object_type_id,
            "association_type": self.association_type,
            "branch_key": self.branch_key,
            "goid": self.goid,
            "record_id": self.record_id,
            "source_goid": self.source_goid,
            "target_goid": self.target_goid,
            "timestamp_us": self.timestamp_us,
            "csn": self.csn,
            "record_kind": self.record_kind,
            "tombstone_status": self.tombstone_status,
            "properties": self.properties,
        });
        if let Some(ordinal) = self.dataset_file_ordinal {
            value["dataset_file_ordinal"] = json!(ordinal);
        }
        if let Some(source) = &self.dataset_file_source {
            value["dataset_file_source"] = json!(source);
        }
        if let Some(file_id) = &self.dataset_file_id {
            value["dataset_file_id"] = json!(file_id);
        }
        if let Some(change) = &self.change {
            value["change"] = json!(change);
        }
        value
    }
}

fn default_association_output_grain() -> OutputGrain {
    OutputGrain::AssociationState
}

impl MaterializedEvidenceRow {
    pub(crate) fn value_for_path(&self, path: &ResolvedPath) -> Value {
        if let Some(field) = &path.evidence_field_id {
            return self.fields.get(field).cloned().unwrap_or(Value::Null);
        }
        self.fields
            .get(&path.display_name)
            .cloned()
            .unwrap_or(Value::Null)
    }

    pub(crate) fn to_json(&self) -> Value {
        Value::Object(self.fields.clone().into_iter().collect())
    }
}

impl MaterializedProjectionRow {
    pub(crate) fn value_for_path(&self, path: &ResolvedPath) -> Value {
        if let Some(column) = &path.projection_column {
            if let Some(value) = self.values.get(column) {
                return value.clone();
            }
            if let Some((_, unqualified)) = column.rsplit_once('.') {
                return self.values.get(unqualified).cloned().unwrap_or(Value::Null);
            }
            return Value::Null;
        }
        self.values
            .get(&path.display_name)
            .cloned()
            .unwrap_or(Value::Null)
    }

    pub(crate) fn to_json(&self) -> Value {
        let values = self
            .values
            .iter()
            .filter(|(key, _)| !key.starts_with(INTERNAL_PROJECTION_FIELD_PREFIX))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        json!({
            "projection_id": self.projection_id,
            "values": values,
        })
    }
}

impl ExecutionRow {
    pub(crate) fn dataset_file_ordinal(&self) -> Option<usize> {
        match self {
            ExecutionRow::Object(row) => row.dataset_file_ordinal,
            ExecutionRow::Association(row) => row.dataset_file_ordinal,
            ExecutionRow::Evidence(row) => row
                .fields
                .get("dataset_file_ordinal")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok()),
            ExecutionRow::Projection(row) => row
                .values
                .get("dataset_file_ordinal")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok()),
        }
    }

    pub(crate) fn with_dataset_member(self, ordinal: usize, source: &str, file_id: String) -> Self {
        match self {
            ExecutionRow::Object(row) => {
                ExecutionRow::Object(row.with_dataset_member(ordinal, source, file_id))
            }
            ExecutionRow::Association(row) => {
                ExecutionRow::Association(row.with_dataset_member(ordinal, source, file_id))
            }
            ExecutionRow::Evidence(mut row) => {
                row.fields
                    .insert("dataset_file_ordinal".into(), json!(ordinal));
                row.fields.insert(
                    "dataset_file_source".into(),
                    Value::String(source.to_string()),
                );
                row.fields
                    .insert("dataset_file_id".into(), Value::String(file_id));
                ExecutionRow::Evidence(row)
            }
            ExecutionRow::Projection(mut row) => {
                row.values
                    .insert("dataset_file_ordinal".into(), json!(ordinal));
                row.values.insert(
                    "dataset_file_source".into(),
                    Value::String(source.to_string()),
                );
                row.values
                    .insert("dataset_file_id".into(), Value::String(file_id));
                ExecutionRow::Projection(row)
            }
        }
    }

    pub(crate) fn value_for_path(&self, path: &ResolvedPath) -> Value {
        match self {
            ExecutionRow::Object(row) => row.value_for_path(path),
            ExecutionRow::Association(row) => row.value_for_path(path),
            ExecutionRow::Evidence(row) => row.value_for_path(path),
            ExecutionRow::Projection(row) => row.value_for_path(path),
        }
    }

    pub(crate) fn window_value(&self, field: &str) -> Value {
        match self {
            ExecutionRow::Projection(row) => row.values.get(field).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        match self {
            ExecutionRow::Object(row) => row.to_json(),
            ExecutionRow::Association(row) => row.to_json(),
            ExecutionRow::Evidence(row) => row.to_json(),
            ExecutionRow::Projection(row) => row.to_json(),
        }
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(TABLE[(byte >> 4) as usize] as char);
        out.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    out
}

fn materialized_properties(
    properties: &[CoveObjectPropertyValue],
) -> (
    BTreeMap<String, Value>,
    BTreeMap<u32, String>,
    BTreeSet<String>,
) {
    let mut values = BTreeMap::new();
    let mut ids = BTreeMap::new();
    let mut redacted = BTreeSet::new();
    for property in properties {
        values.insert(property.property_name.clone(), property.value.clone());
        ids.insert(property.property_id, property.property_name.clone());
        if property.redacted {
            redacted.insert(property.property_name.clone());
        }
    }
    (values, ids, redacted)
}

fn record_kind_name(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Baseline => "baseline",
        RecordKind::Delta => "delta",
        RecordKind::Snapshot => "snapshot",
        RecordKind::Tombstone => "tombstone",
        RecordKind::ReservedLegacyMaterializedDelta => "reserved_legacy_materialized_delta",
        _ => "unknown",
    }
}

fn tombstone_name(status: CoveObjectTombstoneStatus) -> &'static str {
    match status {
        CoveObjectTombstoneStatus::Live => "live",
        CoveObjectTombstoneStatus::Tombstoned => "tombstoned",
    }
}

fn tombstone_status_for_record(kind: RecordKind) -> CoveObjectTombstoneStatus {
    if kind == RecordKind::Tombstone {
        CoveObjectTombstoneStatus::Tombstoned
    } else {
        CoveObjectTombstoneStatus::Live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_function_key_uses_stable_short_argument_hash() {
        let key = window_function_key("row_number", &[]);
        let Some(hash) = key.strip_prefix("__coveql_window:row_number:") else {
            panic!("unexpected window key: {key}");
        };
        assert_eq!(hash.len(), 32);
        assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert!(!key.contains('['));
        assert!(!key.contains('"'));
    }
}
