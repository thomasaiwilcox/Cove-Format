use std::collections::BTreeMap;

use cove_core::{
    nested_schema::NestedSchemaNodeV1,
    profile::cove_o::{CoveObjectState, ObjectTypeEntryV1, PropertyEntryV1, RecordKind},
};
use serde_json::Value;

mod encoding;

pub(crate) use encoding::{
    append_property_value_bytes, file_dictionary_for_model, file_dictionary_index_bytes,
    file_dictionary_key_for_property, nested_shapes_for_model, temporal_segment_index,
    temporal_segment_payload, trust_manifest,
};

#[derive(Debug, Clone)]
pub(crate) struct ObjectRow {
    pub(crate) goid: [u8; 16],
    pub(crate) record_id: [u8; 16],
    pub(crate) object_type_id: u32,
    pub(crate) object_type: String,
    pub(crate) source_id: String,
    pub(crate) source_row_index: usize,
    pub(crate) record_kind: RecordKind,
    pub(crate) properties: BTreeMap<u32, MaterializedProperty>,
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedProperty {
    pub(crate) entry: PropertyEntryV1,
    pub(crate) value: Value,
    pub(crate) assertion_id: String,
    pub(crate) source_id: String,
    pub(crate) source_row_index: usize,
    pub(crate) source_priority: i64,
    pub(crate) source_order: usize,
    pub(crate) conflict_policy: String,
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedModel {
    pub(crate) object_types: Vec<ObjectTypeEntryV1>,
    pub(crate) rows: Vec<ObjectRow>,
    pub(crate) assertions: Vec<Value>,
    pub(crate) assertion_log: Value,
    pub(crate) identity_equivalence_index: Value,
    pub(crate) evidence_entries: Vec<Value>,
    pub(crate) evidence_index: Value,
    pub(crate) conversion_report: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct TemporalSegmentBuild {
    pub(crate) segment_id: u32,
    pub(crate) object_type_id: u32,
    pub(crate) rows: Vec<ObjectRow>,
    pub(crate) payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReconstructedTemporalSegmentBuild {
    pub(crate) segment_id: u32,
    pub(crate) object_type_id: u32,
    pub(crate) rows: Vec<CoveObjectState>,
    pub(crate) payload: Vec<u8>,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectCheckpointTemporalSection {
    pub object_type_id: u32,
    pub row_count: u64,
    pub payload: Vec<u8>,
}

pub(crate) type NestedShapeByProperty = BTreeMap<(u32, u32), NestedSchemaNodeV1>;
