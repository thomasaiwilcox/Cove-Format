//! Cove Format (COVE) v2.0 — COVE-MAP embedded-section reference schema.
//!
//! Spec §70 fixes the COVE-MAP validation boundary but leaves exact reusable
//! mapping-definition payload bodies to a companion schema specification or a
//! required extension. The reference implementation therefore validates
//! embedded `MAP_*` sections using a small JSON-backed schema that captures the
//! normative cross-reference rules from Spec §73.6.

use std::collections::BTreeMap;

use crate::{constants::SectionKind, CoveError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSourceEntry {
    pub source_id: String,
    pub schema_fingerprint: Option<String>,
    pub snapshot_digest: Option<String>,
    pub row_identity_rules: Vec<String>,
    pub replay_claimed: bool,
    pub source_priority: Option<i64>,
    pub sensitivity_label: Option<String>,
    pub sensitivity_rank: Option<i64>,
    pub access_policy_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSourceCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub governance_reconciliation_policy: String,
    pub sources: Vec<MapSourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapFunctionEntry {
    pub function_id: String,
    pub version: String,
    pub deterministic: bool,
    pub dependency: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapFunctionRegistry {
    pub mapping_id: String,
    pub mapping_version: String,
    pub functions: Vec<MapFunctionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapResolutionCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub normalization_pipelines: Vec<MapNormalizationPipeline>,
    pub resolvers: Vec<MapResolver>,
    pub match_rules: Vec<MapCandidateMatchRule>,
    pub reviewed_decisions: Vec<MapReviewedDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapNormalizationPipeline {
    pub pipeline_id: String,
    pub functions: Vec<MapNormalizationFunction>,
    pub tables: Vec<MapNormalizationTable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapNormalizationFunction {
    pub function_id: String,
    pub version: String,
    pub table_id: Option<String>,
    pub suffix_table_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapNormalizationTable {
    pub table_id: String,
    pub digest: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapResolver {
    pub resolver_id: String,
    pub kind: String,
    pub object_type: String,
    pub authority: String,
    pub confidence_class: String,
    pub normalization_pipeline_id: String,
    pub on_hit: String,
    pub on_miss: String,
    pub miss_confidence_class: Option<String>,
    pub ambiguous_policy: String,
    pub catalog_digest: String,
    pub pipeline_digest: String,
    pub resolver_digest: String,
    pub order_sensitive_catalog: bool,
    pub evidence_policy: String,
    pub alias_catalog: Option<MapAliasCatalog>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAliasCatalog {
    pub alias_catalog_id: String,
    pub entries: Vec<MapAliasEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAliasEntry {
    pub alias_entry_id: String,
    pub canonical_key: String,
    pub canonical_label: String,
    pub aliases: Vec<String>,
    pub ambiguous: bool,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub non_semantic_metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCandidateMatchRule {
    pub match_rule_id: String,
    pub object_type: String,
    pub inputs: Vec<MapCandidateMatchInput>,
    pub blocking: BTreeMap<String, serde_json::Value>,
    pub normalization_pipeline_id: String,
    pub scoring: BTreeMap<String, serde_json::Value>,
    pub limits: MapCandidateMatchLimits,
    pub output: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCandidateMatchInput {
    pub source_id: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCandidateMatchLimits {
    pub max_pairs_per_block: u64,
    pub max_pairs_total: u64,
    pub on_limit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapReviewedDecision {
    pub decision_id: String,
    pub decision: String,
    pub confidence_class: String,
    pub reviewed_by: String,
    pub reviewed_at: String,
    pub reason: Option<String>,
    pub left: MapTypedIdentityReference,
    pub right: MapTypedIdentityReference,
    pub canonical_anchor: Option<MapCanonicalAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapTypedIdentityReference {
    pub kind: String,
    pub object_type: String,
    pub identity_rule_id: Option<String>,
    pub resolver_id: Option<String>,
    pub canonical_key: Option<String>,
    pub join_key_sha256: Option<String>,
    pub source_id: Option<String>,
    pub source_row_identity: Option<String>,
    pub source_snapshot_digest: Option<String>,
    pub schema_fingerprint: Option<String>,
    pub row_digest: Option<String>,
    pub identity_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCanonicalAnchor {
    pub kind: String,
    pub object_type: String,
    pub identity_rule_id: String,
    pub components: Vec<MapCanonicalAnchorComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCanonicalAnchorComponent {
    pub role_id: String,
    pub logical_type: String,
    pub resolved_value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapResolutionOutcome {
    AliasHit,
    AliasMiss,
    AliasAmbiguous,
    ReviewedSameObject,
    ReviewedDoNotMerge,
    CandidateOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapEffectiveMergeAuthority {
    Authoritative,
    StrongDeterministic,
    WeakDeterministic,
    SourceScoped,
    CandidateOnly,
    ReviewedAuthoritative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapResolutionBinding {
    pub resolver_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapJoinKeyComponent {
    pub role_id: String,
    pub source_column: String,
    pub logical_type: String,
    pub canonicalization: String,
    pub null_policy: String,
    pub ordering: String,
    pub resolution: Option<MapResolutionBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapIdentityRule {
    pub rule_id: String,
    pub object_type: String,
    pub semantic_role: String,
    pub confidence_class: String,
    pub auto_merge: Option<bool>,
    pub candidate_only: bool,
    pub property_conflicts_declared: bool,
    pub allow_reviewed_equivalence: bool,
    pub function_ids: Vec<String>,
    pub join_keys: Vec<MapJoinKeyComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapDoNotMergeConstraint {
    pub left_identity: String,
    pub right_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapIdentityRuleCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub identity_rules: Vec<MapIdentityRule>,
    pub do_not_merge: Vec<MapDoNotMergeConstraint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOperationKind {
    Fact,
    Insert,
    Upsert,
    PatchProperty,
    ReplaceObjectState,
    CloseAssociation,
    ExpireAndCreate,
    TombstoneObject,
    TombstoneProperty,
    TombstoneAssociation,
    RedactEvidence,
    EvidenceOnly,
    Correction,
}

impl SourceOperationKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "Fact" => Some(Self::Fact),
            "Insert" => Some(Self::Insert),
            "Upsert" => Some(Self::Upsert),
            "PatchProperty" => Some(Self::PatchProperty),
            "ReplaceObjectState" => Some(Self::ReplaceObjectState),
            "CloseAssociation" => Some(Self::CloseAssociation),
            "ExpireAndCreate" => Some(Self::ExpireAndCreate),
            "TombstoneObject" => Some(Self::TombstoneObject),
            "TombstoneProperty" => Some(Self::TombstoneProperty),
            "TombstoneAssociation" => Some(Self::TombstoneAssociation),
            "RedactEvidence" => Some(Self::RedactEvidence),
            "EvidenceOnly" => Some(Self::EvidenceOnly),
            "Correction" => Some(Self::Correction),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "Fact",
            Self::Insert => "Insert",
            Self::Upsert => "Upsert",
            Self::PatchProperty => "PatchProperty",
            Self::ReplaceObjectState => "ReplaceObjectState",
            Self::CloseAssociation => "CloseAssociation",
            Self::ExpireAndCreate => "ExpireAndCreate",
            Self::TombstoneObject => "TombstoneObject",
            Self::TombstoneProperty => "TombstoneProperty",
            Self::TombstoneAssociation => "TombstoneAssociation",
            Self::RedactEvidence => "RedactEvidence",
            Self::EvidenceOnly => "EvidenceOnly",
            Self::Correction => "Correction",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRowSemanticRule {
    pub rule_id: String,
    pub source_id: String,
    pub identity_rule_id: String,
    pub row_semantics_kind: String,
    pub source_operation_kind: SourceOperationKind,
    pub assertion_kinds: Vec<String>,
    pub tombstone_target: Option<String>,
    pub record_kind: String,
    pub temporal_policy: String,
    pub conflict_policy: String,
    pub function_ids: Vec<String>,
    pub output_assertion_ids: Vec<String>,
    pub association_endpoints: Vec<String>,
    pub property_bindings: Vec<MapPropertyBinding>,
    pub association_bindings: Vec<MapAssociationBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapPropertyBinding {
    pub assertion_id: String,
    pub property_id: String,
    pub property_name: String,
    pub source_column: String,
    pub logical_type: String,
    pub physical_kind: String,
    pub value_expression: String,
    pub nullable: bool,
    pub missing_policy: String,
    pub conflict_policy: String,
    pub source_priority: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAssociationBinding {
    pub assertion_id: String,
    pub association_type: String,
    pub target_identity_rule_id: String,
    pub source_identity_rule_id: String,
    pub source_role: String,
    pub target_role: String,
    pub source_endpoint_expression: String,
    pub target_endpoint_expression: String,
    pub valid_from_expression: Option<String>,
    pub valid_to_expression: Option<String>,
    pub cardinality_policy: String,
    pub missing_policy: String,
    pub link_object_materialization: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRowSemanticsCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub rules: Vec<MapRowSemanticRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAssertionEntry {
    pub assertion_id: String,
    pub output_object_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAssertionLog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub assertions: Vec<MapAssertionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEquivalencePair {
    pub left_identity: String,
    pub right_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapIdentityEquivalenceIndex {
    pub mapping_id: String,
    pub mapping_version: String,
    pub equivalences: Vec<MapEquivalencePair>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEvidenceEntry {
    pub source_id: String,
    pub source_row_identity: String,
    pub rule_id: String,
    pub assertion_id: String,
    pub output_object_id: String,
    pub observed_schema_fingerprint: Option<String>,
    pub observed_snapshot_digest: Option<String>,
    pub operation_metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEvidenceIndex {
    pub mapping_id: String,
    pub mapping_version: String,
    pub entries: Vec<MapEvidenceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapObservedSourceState {
    pub source_id: String,
    pub schema_fingerprint: Option<String>,
    pub snapshot_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapConversionReport {
    pub mapping_id: String,
    pub mapping_version: String,
    pub sources: Vec<MapObservedSourceState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapProjectionEntry {
    pub projection_id: String,
    pub assertion_ids: Vec<String>,
    pub output_table: Option<String>,
    pub row_grain: Option<String>,
    pub anchor: Option<MapProjectionAnchor>,
    pub temporal_mode: Option<String>,
    pub columns: Vec<MapProjectionColumn>,
    pub multi_value_policy: Option<String>,
    pub missing_policy: String,
    pub ordering: Vec<String>,
    pub evidence_policy: String,
    pub output_modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapProjectionAnchor {
    pub object_type: Option<String>,
    pub association_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapProjectionColumn {
    pub name: String,
    pub value: String,
    pub logical_type: Option<String>,
    pub nested_shape: Option<String>,
    pub conflict_policy: String,
    pub missing_policy: String,
    pub lineage: Option<MapProjectionColumnLineage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapProjectionColumnLineage {
    pub source: String,
    pub object_type_id: u32,
    pub object_type_name: String,
    pub property_id: u32,
    pub property_name: String,
    pub projection_table_id: u32,
    pub projection_column_id: u32,
    pub expression: String,
    pub transform: String,
    pub filter_pushdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapProjectionCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub projections: Vec<MapProjectionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAiProfileCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub profiles: Vec<MapAiProfileV1>,
    pub slot_policies: Vec<MapAiSlotPolicyV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAiProfileV1 {
    pub profile_id: String,
    pub profile_name: Option<String>,
    pub active: bool,
    pub default_decision: String,
    pub default_granularity: String,
    pub default_role: String,
    pub default_sensitivity: String,
    pub slot_policy_ids: Vec<String>,
    pub template_ids: Vec<String>,
    pub composition_ids: Vec<String>,
    pub training_policy_ids: Vec<String>,
    pub flags: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAiSlotPolicyV1 {
    pub slot_policy_id: String,
    pub source_id: Option<String>,
    pub table_id: Option<u32>,
    pub column_id: Option<u32>,
    pub source_column: Option<String>,
    pub object_type: Option<String>,
    pub property_id: Option<String>,
    pub association_type: Option<String>,
    pub path: String,
    pub role: String,
    pub decision: String,
    pub granularity: String,
    pub sensitivity: String,
    pub vector_space_id: Option<String>,
    pub template_id: Option<String>,
    pub chunk_profile_id: Option<String>,
    pub tokenizer_profile_id: Option<String>,
    pub training_policy_id: Option<String>,
    pub composition_weight_ppm: Option<u32>,
    pub min_distinct_count: Option<u32>,
    pub max_distinct_count: Option<u32>,
    pub max_value_bytes: Option<u32>,
    pub evidence_policy_id: Option<String>,
    pub license_policy_id: Option<String>,
    pub redaction_policy_id: Option<String>,
    pub flags: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAiTemplateCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub templates: Vec<AiVectorTemplateV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiVectorTemplateV1 {
    pub template_id: String,
    pub template_kind: String,
    pub template_text: String,
    pub locale: Option<String>,
    pub deterministic: bool,
    pub template_fingerprint: Option<String>,
    pub flags: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAiTrainingPolicyCatalog {
    pub mapping_id: String,
    pub mapping_version: String,
    pub training_policies: Vec<MapAiTrainingPolicyV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapAiTrainingPolicyV1 {
    pub training_policy_id: String,
    pub sample_policy: String,
    pub label_policy: String,
    pub split_policy: String,
    pub weighting_policy: String,
    pub dedup_policy: String,
    pub quality_policy: String,
    pub flags: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmbeddedMapSection {
    SourceCatalog(MapSourceCatalog),
    FunctionRegistry(MapFunctionRegistry),
    ResolutionCatalog(MapResolutionCatalog),
    IdentityRuleCatalog(MapIdentityRuleCatalog),
    RowSemanticsCatalog(MapRowSemanticsCatalog),
    AssertionLog(MapAssertionLog),
    IdentityEquivalenceIndex(MapIdentityEquivalenceIndex),
    EvidenceIndex(MapEvidenceIndex),
    ConversionReport(MapConversionReport),
    ProjectionCatalog(MapProjectionCatalog),
    AiProfileCatalog(MapAiProfileCatalog),
    AiTemplateCatalog(MapAiTemplateCatalog),
    AiTrainingPolicyCatalog(MapAiTrainingPolicyCatalog),
}

mod embedded;

pub fn parse_embedded_section(
    kind: SectionKind,
    bytes: &[u8],
) -> Result<EmbeddedMapSection, CoveError> {
    embedded::parse_embedded_section(kind, bytes)
}

pub fn validate_embedded_sections(sections: &[EmbeddedMapSection]) -> Result<(), CoveError> {
    embedded::validate_embedded_sections(sections)
}

pub fn compact_evidence_index_bytes(index: &MapEvidenceIndex) -> Result<Vec<u8>, CoveError> {
    embedded::compact_evidence_index_bytes(index)
}

pub fn is_compact_evidence_index_bytes(bytes: &[u8]) -> bool {
    embedded::is_compact_evidence_index_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    fn parse_json(kind: SectionKind, value: Value) -> EmbeddedMapSection {
        parse_embedded_section(
            kind,
            &serde_json::to_vec_pretty(&payload(kind, value)).unwrap(),
        )
        .unwrap()
    }

    fn parse_json_result(kind: SectionKind, value: Value) -> Result<EmbeddedMapSection, CoveError> {
        parse_embedded_section(
            kind,
            &serde_json::to_vec_pretty(&payload(kind, value)).unwrap(),
        )
    }

    fn payload(kind: SectionKind, mut value: Value) -> Value {
        if let Value::Object(object) = &mut value {
            object.insert(
                "schema_id".to_string(),
                Value::String("org.coveformat.covemap.v2".to_string()),
            );
            object.insert(
                "section_id".to_string(),
                Value::Number((kind as u16).into()),
            );
        }
        value
    }

    #[test]
    fn evidence_index_parse_can_filter_operation_metadata_keys() {
        let bytes = serde_json::to_vec_pretty(&payload(
            SectionKind::MapEvidenceIndex,
            json!({
                "mapping_id": "demo",
                "mapping_version": "1",
                "entries": [{
                    "source_id": "crm",
                    "source_row_identity": "crm:0",
                    "rule_id": "row_rule",
                    "assertion_id": "assertion:1",
                    "output_object_id": "goid:1",
                    "source_operation_kind": "Upsert",
                    "operation_effect": "merged",
                    "association_type": "member_of",
                    "property_name": "name"
                }]
            }),
        ))
        .unwrap();
        let index = MapEvidenceIndex::parse_with_requested_operation_metadata_keys(
            &bytes,
            &[String::from("source_operation_kind")],
        )
        .unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(
            index.entries[0]
                .operation_metadata
                .get("source_operation_kind"),
            Some(&json!("Upsert"))
        );
        assert!(!index.entries[0]
            .operation_metadata
            .contains_key("operation_effect"));
        assert!(!index.entries[0]
            .operation_metadata
            .contains_key("association_type"));
        assert!(!index.entries[0]
            .operation_metadata
            .contains_key("property_name"));

        let index = MapEvidenceIndex::parse_with_requested_operation_metadata_keys(
            &bytes,
            &[String::from("association_type")],
        )
        .unwrap();
        assert_eq!(
            index.entries[0].operation_metadata.get("association_type"),
            Some(&json!("member_of"))
        );
        assert!(!index.entries[0]
            .operation_metadata
            .contains_key("source_operation_kind"));
    }

    #[test]
    fn compact_evidence_index_round_trips_and_filters_metadata() {
        let index = MapEvidenceIndex {
            mapping_id: "demo-map".into(),
            mapping_version: "2026.06".into(),
            entries: vec![
                MapEvidenceEntry {
                    source_id: "crm".into(),
                    source_row_identity: "crm:1".into(),
                    rule_id: "upsert_person".into(),
                    assertion_id: "assert:name".into(),
                    output_object_id: "goid:person:1".into(),
                    observed_schema_fingerprint: Some("schema:crm:v1".into()),
                    observed_snapshot_digest: Some("sha256:crm-snapshot".into()),
                    operation_metadata: BTreeMap::from([
                        ("source_operation_kind".into(), json!("Upsert")),
                        ("operation_effect".into(), json!("merged")),
                        ("property_name".into(), json!("name")),
                    ]),
                },
                MapEvidenceEntry {
                    source_id: "support".into(),
                    source_row_identity: "support:1".into(),
                    rule_id: "upsert_person".into(),
                    assertion_id: "assert:name".into(),
                    output_object_id: "goid:person:1".into(),
                    observed_schema_fingerprint: Some("schema:support:v1".into()),
                    observed_snapshot_digest: Some("sha256:support-snapshot".into()),
                    operation_metadata: BTreeMap::from([
                        ("source_operation_kind".into(), json!("Upsert")),
                        ("operation_effect".into(), json!("merged")),
                        ("property_name".into(), json!("name")),
                    ]),
                },
            ],
        };

        let compact = compact_evidence_index_bytes(&index).unwrap();
        assert!(is_compact_evidence_index_bytes(&compact));
        assert_eq!(MapEvidenceIndex::parse(&compact).unwrap(), index);

        let filtered = MapEvidenceIndex::parse_with_requested_operation_metadata_keys(
            &compact,
            &[String::from("source_operation_kind")],
        )
        .unwrap();
        assert_eq!(
            filtered.entries[0]
                .operation_metadata
                .get("source_operation_kind"),
            Some(&json!("Upsert"))
        );
        assert!(!filtered.entries[0]
            .operation_metadata
            .contains_key("operation_effect"));
        assert!(!filtered.entries[0]
            .operation_metadata
            .contains_key("property_name"));

        let mut corrupt = compact;
        *corrupt.last_mut().unwrap() ^= 0x80;
        assert_eq!(
            MapEvidenceIndex::parse(&corrupt),
            Err(CoveError::MapEvidenceInvalid)
        );
    }

    fn row_rule_with_operation(
        operation: &str,
        row_semantics_kind: &str,
        assertion_kinds: Vec<&str>,
        tombstone_target: Option<&str>,
        property_binding: bool,
        association_binding: bool,
    ) -> Value {
        let mut rule = json!({
            "rule_id": "upsert_customer",
            "source_id": "crm.customers",
            "identity_rule_id": "customer_identity",
            "row_semantics_kind": row_semantics_kind,
            "source_operation_kind": operation,
            "assertion_kinds": assertion_kinds,
            "function_ids": ["trim_lower"],
            "output_assertion_ids": ["assert_customer_name"],
            "association_endpoints": []
        });
        let object = rule.as_object_mut().unwrap();
        if let Some(target) = tombstone_target {
            object.insert("tombstone_target".into(), json!(target));
        }
        if property_binding {
            object.insert(
                "property_bindings".into(),
                json!([{
                    "assertion_id": "assert_customer_name",
                    "property_id": "name",
                    "property_name": "name",
                    "source_column": "name",
                    "logical_type": "utf8"
                }]),
            );
        }
        if association_binding {
            object.insert(
                "association_bindings".into(),
                json!([{
                    "assertion_id": "assert_customer_name",
                    "association_type": "member_of",
                    "target_identity_rule_id": "customer_identity"
                }]),
            );
        }
        rule
    }

    fn replace_row_rule(sections: &mut [EmbeddedMapSection], rule: Value) {
        sections[3] = parse_json(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "rules": [rule]
            }),
        );
    }

    fn valid_sections() -> Vec<EmbeddedMapSection> {
        vec![
            parse_json(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "sources": [{
                        "source_id": "crm.customers",
                        "schema_fingerprint": "schema-v1",
                        "snapshot_digest": "digest-v1",
                        "row_identity_rules": ["customer_id"],
                        "replay_claimed": true
                    }]
                }),
            ),
            parse_json(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "functions": [{
                        "function_id": "trim_lower",
                        "version": "1.0.0",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            parse_json(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "identity_rules": [{
                        "rule_id": "customer_identity",
                        "object_type": "Customer",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["trim_lower"],
                        "join_keys": [{
                            "role_id": "customer_id",
                            "source_column": "customer_id",
                            "logical_type": "utf8",
                            "canonicalization": "trim_lower",
                            "null_policy": "reject",
                            "ordering": "asc"
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            parse_json(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "rules": [{
                        "rule_id": "upsert_customer",
                        "source_id": "crm.customers",
                        "identity_rule_id": "customer_identity",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["trim_lower"],
                        "output_assertion_ids": ["assert_customer_name"],
                        "association_endpoints": []
                    }]
                }),
            ),
            parse_json(
                SectionKind::MapAssertionLog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "assertions": [{
                        "assertion_id": "assert_customer_name",
                        "output_object_id": "goid:customer:1"
                    }]
                }),
            ),
            parse_json(
                SectionKind::MapEvidenceIndex,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "entries": [{
                        "source_id": "crm.customers",
                        "source_row_identity": "customer_id=1",
                        "rule_id": "upsert_customer",
                        "assertion_id": "assert_customer_name",
                        "output_object_id": "goid:customer:1",
                        "observed_schema_fingerprint": "schema-v1",
                        "observed_snapshot_digest": "digest-v1"
                    }]
                }),
            ),
        ]
    }

    fn valid_ai_sections() -> Vec<EmbeddedMapSection> {
        let mut sections = valid_sections();
        replace_row_rule(
            &mut sections,
            row_rule_with_operation(
                "PatchProperty",
                "Object",
                vec!["object", "property", "evidence"],
                None,
                true,
                false,
            ),
        );
        sections.push(parse_json(
            SectionKind::MapAiTemplateCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "templates": [{
                    "template_id": "product_name_template",
                    "template_kind": "Vectorization",
                    "template_text": "Product name: {value}",
                    "deterministic": true,
                    "template_fingerprint": "sha256:66642c510c3b29a7bac97c2bcdc64aa9da4e67576cd388d092b06b80f21ddc10"
                }]
            }),
        ));
        sections.push(parse_json(
            SectionKind::MapAiTrainingPolicyCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "training_policies": [{
                    "training_policy_id": "sample_policy",
                    "sample_policy": "curated",
                    "label_policy": "none",
                    "split_policy": "deterministic",
                    "weighting_policy": "uniform",
                    "dedup_policy": "source_hash",
                    "quality_policy": "required"
                }]
            }),
        ));
        sections.push(parse_json(
            SectionKind::MapAiProfileCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "profiles": [{
                    "profile_id": "customer_ai_v1",
                    "profile_name": "Customer AI v1",
                    "active": true,
                    "default_decision": "Ignore",
                    "default_granularity": "SlotValue",
                    "default_role": "Unknown",
                    "default_sensitivity": "Internal",
                    "slot_policy_ids": ["customer_name_ai"],
                    "template_ids": ["product_name_template"],
                    "training_policy_ids": ["sample_policy"]
                }],
                "slot_policies": [{
                    "slot_policy_id": "customer_name_ai",
                    "source_id": "crm.customers",
                    "source_column": "name",
                    "object_type": "Customer",
                    "property_id": "name",
                    "path": "Customer.name",
                    "role": "Title",
                    "decision": "VectorizeSlotValues",
                    "granularity": "SlotValue",
                    "sensitivity": "Internal",
                    "template_id": "product_name_template",
                    "training_policy_id": "sample_policy",
                    "composition_weight_ppm": 500000
                }]
            }),
        ));
        sections
    }

    #[test]
    fn map_source_catalog_parse_rejects_missing_mapping_id() {
        assert_eq!(
            MapSourceCatalog::parse(
                &serde_json::to_vec_pretty(&payload(
                    SectionKind::MapSourceCatalog,
                    json!({
                        "mapping_version": "2026.05",
                        "sources": []
                    })
                ))
                .unwrap()
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_payload_rejects_duplicate_keys_before_value_collapse() {
        let bytes = br#"{"schema_id":"org.coveformat.covemap.v2","schema_id":"org.coveformat.covemap.v2","section_id":60,"mapping_id":"m","mapping_version":"v"}"#;
        assert_eq!(
            parse_embedded_section(SectionKind::MapSourceCatalog, bytes),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_payload_rejects_wrong_section_id() {
        let bytes = br#"{"schema_id":"org.coveformat.covemap.v2","section_id":61,"mapping_id":"customer-map","mapping_version":"2026.05"}"#;
        assert_eq!(
            parse_embedded_section(SectionKind::MapSourceCatalog, bytes),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_payload_rejects_unknown_nested_source_field() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "sources": [{
                        "source_id": "crm.customers",
                        "row_identity_rules": ["customer_id"],
                        "unexpected_source_field": true
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_payload_rejects_unknown_nested_projection_column_field() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "projections": [{
                        "projection_id": "customer_projection",
                        "output_table": "customers",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Customer"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [{
                            "name": "goid",
                            "value": "object.goid",
                            "unexpected_column_field": "bad"
                        }],
                        "output_modes": ["json"]
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_payload_accepts_object_extensions() {
        assert!(parse_json_result(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "extension": {"x.example": {"enabled": true}},
                "extensions": {"x.example.audit": {"mode": "strict"}},
                "sources": []
            }),
        )
        .is_ok());
    }

    #[test]
    fn map_payload_rejects_malformed_extension_containers() {
        for payload in [
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "extension": "bad",
                "sources": []
            }),
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "extensions": [],
                "sources": []
            }),
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "extensions": {"x.example": null},
                "sources": []
            }),
        ] {
            assert_eq!(
                parse_json_result(SectionKind::MapSourceCatalog, payload),
                Err(CoveError::MapInvalid)
            );
        }
    }

    #[test]
    fn projection_parse_accepts_nested_shape() {
        let section = parse_json_result(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "projections": [{
                    "projection_id": "customer_projection",
                    "output_table": "customers",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Customer"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [{
                        "name": "tags",
                        "value": "property.tags",
                        "logical_type": "list",
                        "nested_shape": {"type": "list", "item": {"logical_type": "utf8"}}
                    }],
                    "output_modes": ["json", "arrow"]
                }]
            }),
        )
        .unwrap();
        let EmbeddedMapSection::ProjectionCatalog(catalog) = section else {
            panic!("expected projection catalog");
        };
        assert!(catalog.projections[0].columns[0]
            .nested_shape
            .as_deref()
            .unwrap()
            .contains("\"list\""));
    }

    #[test]
    fn projection_parse_rejects_malformed_nested_shape() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "projections": [{
                        "projection_id": "customer_projection",
                        "output_table": "customers",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Customer"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [{
                            "name": "tags",
                            "value": "property.tags",
                            "logical_type": "list",
                            "nested_shape": []
                        }],
                        "output_modes": ["json", "arrow"]
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn row_semantics_parse_rejects_missing_assertion_kinds() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "rules": [{
                        "rule_id": "upsert_customer",
                        "source_id": "crm.customers",
                        "identity_rule_id": "customer_identity"
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn row_semantics_parse_rejects_unknown_row_kind() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "rules": [{
                        "rule_id": "bad_customer",
                        "source_id": "crm.customers",
                        "identity_rule_id": "customer_identity",
                        "row_semantics_kind": "MaybeObject",
                        "assertion_kinds": ["object"]
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn row_semantics_parse_rejects_unknown_source_operation_kind() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "rules": [{
                        "rule_id": "bad_customer",
                        "source_id": "crm.customers",
                        "identity_rule_id": "customer_identity",
                        "row_semantics_kind": "Object",
                        "source_operation_kind": "MaybePatch",
                        "assertion_kinds": ["object"]
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn row_semantics_parse_rejects_invalid_tombstone_target() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "rules": [{
                        "rule_id": "delete_customer",
                        "source_id": "crm.customers",
                        "identity_rule_id": "customer_identity",
                        "row_semantics_kind": "Tombstone",
                        "assertion_kinds": ["tombstone", "evidence"],
                        "tombstone_target": "foreign_key"
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn projection_parse_rejects_malformed_policy() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "projections": [{
                        "projection_id": "customer_projection",
                        "output_table": "customers",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Customer"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "maybe",
                        "columns": [{"name": "goid", "value": "object.goid"}],
                        "output_modes": ["json"]
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn embedded_map_validation_rejects_expanded_projection_without_policy() {
        let mut sections = valid_sections();
        sections.push(parse_json(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "projections": [{
                    "projection_id": "customer_projection",
                    "output_table": "customers",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Customer"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "columns": [{"name": "goid", "value": "object.goid"}],
                    "output_modes": ["json"]
                }]
            }),
        ));
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn embedded_map_validation_accepts_consistent_sections() {
        assert_eq!(validate_embedded_sections(&valid_sections()), Ok(()));
    }

    #[test]
    fn map_ai_validation_accepts_slot_policy_with_template_and_training_refs() {
        assert_eq!(validate_embedded_sections(&valid_ai_sections()), Ok(()));
    }

    #[test]
    fn map_ai_training_policy_parse_rejects_invalid_sample_policy() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapAiTrainingPolicyCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "training_policies": [{
                        "training_policy_id": "bad_sample_policy",
                        "sample_policy": "export_everything"
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_ai_training_policy_parse_rejects_invalid_dedup_policy() {
        assert_eq!(
            parse_json_result(
                SectionKind::MapAiTrainingPolicyCatalog,
                json!({
                    "mapping_id": "customer-map",
                    "mapping_version": "2026.05",
                    "training_policies": [{
                        "training_policy_id": "bad_dedup_policy",
                        "dedup_policy": "hope"
                    }]
                }),
            ),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_ai_validation_rejects_duplicate_slot_path_in_active_profile() {
        let mut sections = valid_sections();
        sections.push(parse_json(
            SectionKind::MapAiProfileCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "profiles": [{
                    "profile_id": "customer_ai_v1",
                    "active": true,
                    "slot_policy_ids": ["name_slot_a", "name_slot_b"]
                }],
                "slot_policies": [
                    {
                        "slot_policy_id": "name_slot_a",
                        "path": "Customer.name",
                        "role": "Title",
                        "decision": "VectorizeSlotValues",
                        "granularity": "SlotValue",
                        "sensitivity": "Internal"
                    },
                    {
                        "slot_policy_id": "name_slot_b",
                        "path": "Customer.name",
                        "role": "Title",
                        "decision": "Tokenize",
                        "granularity": "SlotValue",
                        "sensitivity": "Internal"
                    }
                ]
            }),
        ));
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_ai_validation_rejects_forbidden_slot_conflict_across_active_profiles() {
        let mut sections = valid_sections();
        sections.push(parse_json(
            SectionKind::MapAiProfileCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "profiles": [
                    {
                        "profile_id": "privacy_ai_v1",
                        "active": true,
                        "slot_policy_ids": ["private_note_forbidden"]
                    },
                    {
                        "profile_id": "search_ai_v1",
                        "active": true,
                        "slot_policy_ids": ["private_note_vector"]
                    }
                ],
                "slot_policies": [
                    {
                        "slot_policy_id": "private_note_forbidden",
                        "path": "Customer.private_note",
                        "role": "PolicyProtected",
                        "decision": "Forbidden",
                        "granularity": "SlotValue",
                        "sensitivity": "Forbidden"
                    },
                    {
                        "slot_policy_id": "private_note_vector",
                        "path": "Customer.private_note",
                        "role": "NaturalLanguageLong",
                        "decision": "VectorizeSlotValues",
                        "granularity": "SlotValue",
                        "sensitivity": "Sensitive"
                    }
                ]
            }),
        ));
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn map_ai_validation_rejects_referenced_template_without_fingerprint() {
        let mut sections = valid_sections();
        sections.push(parse_json(
            SectionKind::MapAiTemplateCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "templates": [{
                    "template_id": "missing_fingerprint_template",
                    "template_kind": "Vectorization",
                    "template_text": "Name: {value}",
                    "deterministic": true
                }]
            }),
        ));
        sections.push(parse_json(
            SectionKind::MapAiProfileCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "profiles": [{
                    "profile_id": "customer_ai_v1",
                    "active": true,
                    "slot_policy_ids": ["name_slot"],
                    "template_ids": ["missing_fingerprint_template"]
                }],
                "slot_policies": [{
                    "slot_policy_id": "name_slot",
                    "path": "Customer.name",
                    "role": "Title",
                    "decision": "VectorizeSlotValues",
                    "granularity": "SlotValue",
                    "sensitivity": "Internal",
                    "template_id": "missing_fingerprint_template"
                }]
            }),
        ));
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn embedded_map_validation_accepts_all_source_operation_kinds() {
        let cases = [
            (
                "Fact",
                "Object",
                vec!["object", "evidence"],
                None,
                false,
                false,
            ),
            (
                "Insert",
                "Object",
                vec!["object", "evidence"],
                None,
                false,
                false,
            ),
            (
                "Upsert",
                "Object",
                vec!["object", "evidence"],
                None,
                false,
                false,
            ),
            (
                "PatchProperty",
                "Object",
                vec!["object", "property", "evidence"],
                None,
                true,
                false,
            ),
            (
                "ReplaceObjectState",
                "Object",
                vec!["object", "evidence"],
                None,
                false,
                false,
            ),
            (
                "CloseAssociation",
                "Object",
                vec!["object", "association", "evidence"],
                None,
                false,
                true,
            ),
            (
                "ExpireAndCreate",
                "Object",
                vec!["object", "evidence"],
                None,
                false,
                false,
            ),
            (
                "TombstoneObject",
                "Tombstone",
                vec!["tombstone", "evidence"],
                Some("object"),
                false,
                false,
            ),
            (
                "TombstoneProperty",
                "Tombstone",
                vec!["tombstone", "evidence"],
                Some("property"),
                false,
                false,
            ),
            (
                "TombstoneAssociation",
                "Tombstone",
                vec!["tombstone", "evidence"],
                Some("association"),
                false,
                false,
            ),
            (
                "RedactEvidence",
                "EvidenceOnly",
                vec!["evidence"],
                None,
                false,
                false,
            ),
            (
                "EvidenceOnly",
                "EvidenceOnly",
                vec!["evidence"],
                None,
                false,
                false,
            ),
            (
                "Correction",
                "Object",
                vec!["object", "property", "evidence"],
                None,
                true,
                false,
            ),
        ];

        for (operation, row_kind, assertions, target, property, association) in cases {
            let mut sections = valid_sections();
            replace_row_rule(
                &mut sections,
                row_rule_with_operation(
                    operation,
                    row_kind,
                    assertions,
                    target,
                    property,
                    association,
                ),
            );
            assert_eq!(
                validate_embedded_sections(&sections),
                Ok(()),
                "operation {operation} should validate"
            );
        }
    }

    #[test]
    fn embedded_map_validation_rejects_malformed_operation_payloads() {
        let mut sections = valid_sections();
        replace_row_rule(
            &mut sections,
            row_rule_with_operation(
                "PatchProperty",
                "Object",
                vec!["object", "property", "evidence"],
                None,
                false,
                false,
            ),
        );
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapInvalid)
        );

        let mut sections = valid_sections();
        replace_row_rule(
            &mut sections,
            row_rule_with_operation(
                "CloseAssociation",
                "Object",
                vec!["object", "association", "evidence"],
                None,
                false,
                false,
            ),
        );
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapInvalid)
        );

        let mut sections = valid_sections();
        replace_row_rule(
            &mut sections,
            row_rule_with_operation(
                "EvidenceOnly",
                "Object",
                vec!["object", "evidence"],
                None,
                false,
                false,
            ),
        );
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn embedded_map_validation_rejects_undeclared_function_reference() {
        let mut sections = valid_sections();
        sections[1] = parse_json(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "functions": []
            }),
        );
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapFunctionUndeclared)
        );
    }

    #[test]
    fn embedded_map_validation_rejects_identity_conflict() {
        let mut sections = valid_sections();
        sections.push(parse_json(
            SectionKind::MapIdentityEquivalenceIndex,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "equivalences": [{
                    "left_identity": "customer:1",
                    "right_identity": "customer:2"
                }]
            }),
        ));
        sections[2] = parse_json(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "identity_rules": [{
                    "rule_id": "customer_identity",
                    "object_type": "Customer",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["trim_lower"],
                    "join_keys": [{
                        "role_id": "customer_id",
                        "source_column": "customer_id",
                        "logical_type": "utf8",
                        "canonicalization": "trim_lower",
                        "null_policy": "reject",
                        "ordering": "asc"
                    }]
                }],
                "do_not_merge": [{
                    "left_identity": "customer:1",
                    "right_identity": "customer:2"
                }]
            }),
        );
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapIdentityConflict)
        );
    }

    #[test]
    fn embedded_map_validation_rejects_stale_source_state() {
        let mut sections = valid_sections();
        sections.push(parse_json(
            SectionKind::MapConversionReport,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "sources": [{
                    "source_id": "crm.customers",
                    "schema_fingerprint": "schema-v2",
                    "snapshot_digest": "digest-v1"
                }]
            }),
        ));
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapSourceStale)
        );
    }

    #[test]
    fn embedded_map_validation_rejects_invalid_evidence_reference() {
        let mut sections = valid_sections();
        sections[5] = parse_json(
            SectionKind::MapEvidenceIndex,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "entries": [{
                    "source_id": "crm.customers",
                    "source_row_identity": "customer_id=1",
                    "rule_id": "upsert_customer",
                    "assertion_id": "assert_missing",
                    "output_object_id": "goid:customer:1"
                }]
            }),
        );
        assert_eq!(
            validate_embedded_sections(&sections),
            Err(CoveError::MapEvidenceInvalid)
        );
    }
}
