use std::collections::BTreeMap;

use cove_core::{
    nested_schema::NestedSchemaNodeV1,
    profile::cove_o::{CoveObjectState, ObjectTypeEntryV1, PropertyEntryV1, RecordKind},
};
use serde_json::Value;

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
pub(crate) struct ReviewedDecisionReplayBinding {
    pub(crate) count: usize,
    pub(crate) digest: String,
}

#[derive(Debug, Clone)]
pub(crate) struct JoinKeyEvaluation {
    pub(crate) tuple: Vec<u8>,
    pub(crate) materializes_identity: bool,
    pub(crate) effective_confidence_class: Option<String>,
    pub(crate) resolution_metadata: Vec<ResolutionMetadata>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolutionMetadata {
    pub(crate) role_id: String,
    pub(crate) resolution_kind: String,
    pub(crate) resolver_id: String,
    pub(crate) resolver_digest: String,
    pub(crate) catalog_digest: String,
    pub(crate) pipeline_digest: String,
    pub(crate) normalization_pipeline_id: String,
    pub(crate) evidence_policy: String,
    pub(crate) redacted_resolution_evidence: bool,
    pub(crate) raw_observed_value: String,
    pub(crate) normalized_value: String,
    pub(crate) resolved_identity_value: Option<String>,
    pub(crate) canonical_key: Option<String>,
    pub(crate) canonical_label: Option<String>,
    pub(crate) alias_catalog_id: Option<String>,
    pub(crate) alias_entry_id: Option<String>,
    pub(crate) alias_hit: bool,
    pub(crate) alias_miss: bool,
    pub(crate) alias_ambiguous: bool,
    pub(crate) miss_policy: Option<String>,
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

pub(crate) type NestedShapeByProperty = BTreeMap<(u32, u32), NestedSchemaNodeV1>;
