use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::CoveError;

use super::{
    query_discovery_error,
    validate::{
        manifest_has_unknown_features_tolerated_for_human_inspection,
        reject_duplicate_json_object_keys, validate_manifest_value,
    },
};

pub const QUERY_DISCOVERY_SCHEMA_V1: &str = "cove.query_discovery.v1";
pub const QUERY_DISCOVERY_CANONICALIZATION_JCS: &str = "rfc8785-jcs";
pub const QUERY_DISCOVERY_ADVISORY_AUTHORITY: &str = "advisory_discovery_not_archive_truth";

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryManifest {
    raw_canonical_json: Vec<u8>,
    value: Value,
    automation_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDiscoveryOptions {
    pub source_name: Option<String>,
    pub disclosure_mode: MetadataDisclosureMode,
    pub policy_fingerprint: Option<String>,
    pub principal_class: Option<String>,
    pub audience: Option<String>,
    pub include_examples: bool,
    pub include_ai: bool,
    pub validate_examples: bool,
    pub include_developer_diagnostics: bool,
}

impl Default for QueryDiscoveryOptions {
    fn default() -> Self {
        Self {
            source_name: None,
            disclosure_mode: MetadataDisclosureMode::Public,
            policy_fingerprint: None,
            principal_class: Some("public".to_string()),
            audience: Some("public".to_string()),
            include_examples: true,
            include_ai: false,
            validate_examples: true,
            include_developer_diagnostics: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataDisclosureMode {
    Public,
    Developer,
}

impl QueryDiscoveryManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        Self::parse_with_options(bytes, &QueryDiscoveryParseOptions::default())
    }

    pub fn parse_with_options(
        bytes: &[u8],
        options: &QueryDiscoveryParseOptions,
    ) -> Result<Self, CoveError> {
        reject_duplicate_json_object_keys(bytes)?;
        let value: Value = serde_json::from_slice(bytes).map_err(|err| {
            query_discovery_error(format!("manifest is not valid UTF-8 JSON: {err}"))
        })?;
        validate_manifest_value(bytes, &value, options)?;
        let automation_allowed =
            !manifest_has_unknown_features_tolerated_for_human_inspection(&value, options)?;
        Ok(Self {
            raw_canonical_json: bytes.to_vec(),
            value,
            automation_allowed,
        })
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn raw_canonical_json(&self) -> &[u8] {
        &self.raw_canonical_json
    }

    pub fn automation_allowed(&self) -> bool {
        self.automation_allowed
    }

    pub fn schema(&self) -> &str {
        self.value
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub fn authority(&self) -> &str {
        self.value
            .get("authority")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub fn source_binding_record(&self) -> QueryDiscoverySourceBinding {
        source_binding_record(self.value.get("source_binding").unwrap_or(&Value::Null))
    }

    pub fn coveql_contract(&self) -> QueryDiscoveryCoveQlContract {
        coveql_contract_record(self.value.get("coveql").unwrap_or(&Value::Null))
    }

    pub fn surface_records(&self) -> Vec<QueryDiscoverySurface> {
        surface_records(self.value.get("surfaces").unwrap_or(&Value::Null))
    }

    pub fn alias_binding_records(&self) -> Vec<QueryDiscoveryAliasBinding> {
        value_array(self.value.get("alias_bindings"))
            .into_iter()
            .map(alias_binding_record)
            .collect()
    }

    pub fn relationship_records(&self) -> Vec<QueryDiscoveryRelationship> {
        value_array(self.value.get("relationships"))
            .into_iter()
            .map(relationship_record)
            .collect()
    }

    pub fn property_glossary_records(&self) -> Vec<QueryDiscoveryPropertyGlossaryEntry> {
        value_array(self.value.get("property_glossary"))
            .into_iter()
            .map(property_glossary_record)
            .collect()
    }

    pub fn template_records(&self) -> Vec<QueryTemplate> {
        value_array(self.value.get("templates"))
            .into_iter()
            .filter_map(query_template_record)
            .collect()
    }

    pub fn example_records(&self) -> Vec<QueryExample> {
        value_array(self.value.get("examples"))
            .into_iter()
            .filter_map(query_example_record)
            .collect()
    }

    pub fn ai_capability(&self) -> QueryAiCapability {
        ai_capability_record(self.value.get("ai").unwrap_or(&Value::Null))
    }

    pub fn policy_record(&self) -> QueryDiscoveryPolicy {
        policy_record(self.value.get("policy").unwrap_or(&Value::Null))
    }

    pub fn budget_record(&self) -> QueryDiscoveryBudget {
        budget_record(self.value.get("resource_budgets").unwrap_or(&Value::Null))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDiscoveryParseOptions {
    pub require_jcs: bool,
    pub allow_unknown_manifest_features_for_human_inspection: bool,
    pub known_manifest_features: BTreeSet<String>,
}

impl Default for QueryDiscoveryParseOptions {
    fn default() -> Self {
        Self {
            require_jcs: true,
            allow_unknown_manifest_features_for_human_inspection: false,
            known_manifest_features: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDiscoveryValidationStatus {
    Valid,
    Stale,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDiscoveryValidationFlag {
    PolicyFiltered,
    DiagnosticsWithheld,
    ExamplesLimited,
    AiLimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDiscoveryDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDiscoveryDiagnostic {
    pub code: String,
    pub severity: QueryDiscoveryDiagnosticSeverity,
    pub message: String,
    pub target_kind: Option<String>,
    pub target: Option<String>,
    pub withheld: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryDiscoveryValidationContext {
    pub strict_discovery: bool,
    pub source_binding_valid: Option<bool>,
    pub policy_scope_compatible: Option<bool>,
    pub expected_source_kind: Option<String>,
    pub expected_file_digest: Option<String>,
    pub expected_file_length: Option<String>,
    pub expected_header_crc32c: Option<String>,
    pub expected_footer_crc32c: Option<String>,
    pub expected_schema_fingerprint: Option<String>,
    pub expected_dictionary_crc32c: Option<String>,
    pub expected_covm_snapshot_digest: Option<String>,
    pub expected_member_file_digests: BTreeMap<String, String>,
    pub expected_policy_fingerprint: Option<String>,
    pub expected_principal_class: Option<String>,
    pub expected_audience: Option<String>,
    pub validation_flags: Vec<QueryDiscoveryValidationFlag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDiscoveryValidationReport {
    pub validation_status: QueryDiscoveryValidationStatus,
    pub validation_flags: Vec<QueryDiscoveryValidationFlag>,
    pub diagnostics: Vec<QueryDiscoveryDiagnostic>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoverySourceBinding {
    pub source_kind: String,
    pub source_uri_hint: Option<String>,
    pub file_digest: Option<String>,
    pub file_length: Option<String>,
    pub header_crc32c: Option<String>,
    pub footer_crc32c: Option<String>,
    pub schema_fingerprint: Option<String>,
    pub dictionary_crc32c: Option<String>,
    pub covm_snapshot_digest: Option<String>,
    pub members: Vec<QueryDiscoverySourceMember>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoverySourceMember {
    pub member_id: String,
    pub uri_hint: Option<String>,
    pub file_digest: Option<String>,
    pub file_length: Option<String>,
    pub footer_crc32c: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryCoveQlContract {
    pub language_version: Option<String>,
    pub core_version: Option<String>,
    pub profiles: Vec<String>,
    pub roots: Vec<String>,
    pub capabilities: Vec<String>,
    pub default_explain_mode: Option<String>,
    pub allowed_explain_modes: Vec<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoverySurface {
    pub surface_kind: String,
    pub root: Option<String>,
    pub query_name: Option<String>,
    pub query_identifier: Option<String>,
    pub display_name: Option<String>,
    pub fields: Vec<QueryDiscoveryIdentifier>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryIdentifier {
    pub name: Option<String>,
    pub query_name: Option<String>,
    pub query_identifier: Option<String>,
    pub display_name: Option<String>,
    pub logical_type: Option<String>,
    pub nullable: Option<bool>,
    pub operations: Vec<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryRelationship {
    pub id: Option<String>,
    pub from_root: Option<String>,
    pub to_root: Option<String>,
    pub association_root: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryPropertyGlossaryEntry {
    pub path: Option<String>,
    pub query_path: Option<String>,
    pub query_identifier_path: Option<String>,
    pub display_name: Option<String>,
    pub logical_type: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryTemplate {
    pub id: String,
    pub binding_mode: Option<String>,
    pub template_validation: QueryTemplateValidationStatus,
    pub operator_chain: Vec<QueryTemplateOperatorChain>,
    pub parameters: Vec<QueryTemplateParameterConstraint>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryTemplateValidationStatus {
    OperatorChainValidated,
    ParametersValidated,
    RepresentativePlannedDryRun,
    PlannedDryRun,
    NotValidatedPolicyLimited,
    NotValidatedBudgetLimited,
    NotValidated,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryTemplateParameterConstraint {
    pub name: String,
    pub kind: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryTemplateOperatorChain {
    pub method: String,
    pub kind: Option<String>,
    pub arg: Option<String>,
    pub profiles: Vec<String>,
    pub capabilities: Vec<String>,
    pub requires_sidecar: Option<bool>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryExample {
    pub question: Option<String>,
    pub query: String,
    pub query_validation: QueryExampleValidationStatus,
    pub profiles: Vec<String>,
    pub roots: Vec<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryExampleValidationStatus {
    ParsedAndResolved,
    ParsedOnly,
    PlannedDryRun,
    NotValidatedPolicyLimited,
    NotValidatedBudgetLimited,
    NotValidated,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryAiCapability {
    pub profile_available: bool,
    pub methods: Vec<Value>,
    pub sidecars: Vec<Value>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryPolicy {
    pub visibility_scope: Option<String>,
    pub redaction_scope: Option<String>,
    pub policy_fingerprint: Option<String>,
    pub principal_class: Option<String>,
    pub audience: Option<String>,
    pub metadata_disclosure: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryBudget {
    pub default_take: Option<u64>,
    pub max_take: Option<u64>,
    pub max_graph_depth: Option<u64>,
    pub max_graph_paths: Option<u64>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryDiscoveryAliasBinding {
    pub alias: Option<String>,
    pub query_identifier: Option<String>,
    pub target_kind: Option<String>,
    pub target_root: Option<String>,
    pub policy_scope: Option<String>,
    pub raw: Value,
}

pub fn parse_query_discovery_manifest(bytes: &[u8]) -> Result<QueryDiscoveryManifest, CoveError> {
    QueryDiscoveryManifest::parse(bytes)
}

fn source_binding_record(value: &Value) -> QueryDiscoverySourceBinding {
    QueryDiscoverySourceBinding {
        source_kind: string_field(value, "source_kind").unwrap_or_default(),
        source_uri_hint: string_field(value, "source_uri_hint"),
        file_digest: string_field(value, "file_digest"),
        file_length: string_field(value, "file_length"),
        header_crc32c: string_field(value, "header_crc32c"),
        footer_crc32c: string_field(value, "footer_crc32c"),
        schema_fingerprint: string_field(value, "schema_fingerprint"),
        dictionary_crc32c: string_field(value, "dictionary_crc32c"),
        covm_snapshot_digest: string_field(value, "covm_snapshot_digest"),
        members: value_array(value.get("members"))
            .into_iter()
            .map(|member| QueryDiscoverySourceMember {
                member_id: string_field(member, "member_id").unwrap_or_default(),
                uri_hint: string_field(member, "uri_hint"),
                file_digest: string_field(member, "file_digest"),
                file_length: string_field(member, "file_length"),
                footer_crc32c: string_field(member, "footer_crc32c"),
                raw: member.clone(),
            })
            .collect(),
        raw: value.clone(),
    }
}

fn coveql_contract_record(value: &Value) -> QueryDiscoveryCoveQlContract {
    QueryDiscoveryCoveQlContract {
        language_version: string_field(value, "language_version"),
        core_version: string_field(value, "core_version"),
        profiles: string_array_value(value.get("profiles")),
        roots: string_array_value(value.get("roots")),
        capabilities: string_array_value(value.get("capabilities")),
        default_explain_mode: string_field(value, "default_explain_mode"),
        allowed_explain_modes: string_array_value(value.get("allowed_explain_modes")),
        raw: value.clone(),
    }
}

fn surface_records(value: &Value) -> Vec<QueryDiscoverySurface> {
    let mut out = Vec::new();
    for (kind, field_name) in [
        ("table", "columns"),
        ("object", "properties"),
        ("projection", "columns"),
        ("evidence", "columns"),
    ] {
        let key = match kind {
            "table" => "tables",
            "object" => "objects",
            "projection" => "projections",
            _ => "evidence",
        };
        for surface in value_array(value.get(key)) {
            let fields = value_array(surface.get(field_name))
                .into_iter()
                .map(identifier_record)
                .collect();
            out.push(QueryDiscoverySurface {
                surface_kind: kind.to_string(),
                root: string_field(surface, "root"),
                query_name: string_field(surface, "query_name")
                    .or_else(|| string_field(surface, "query_type_name")),
                query_identifier: string_field(surface, "query_identifier"),
                display_name: string_field(surface, "display_name"),
                fields,
                raw: surface.clone(),
            });
        }
    }
    out
}

fn identifier_record(value: &Value) -> QueryDiscoveryIdentifier {
    QueryDiscoveryIdentifier {
        name: string_field(value, "name"),
        query_name: string_field(value, "query_name"),
        query_identifier: string_field(value, "query_identifier"),
        display_name: string_field(value, "display_name"),
        logical_type: string_field(value, "logical_type"),
        nullable: value.get("nullable").and_then(Value::as_bool),
        operations: string_array_value(value.get("operations")),
        raw: value.clone(),
    }
}

fn alias_binding_record(value: &Value) -> QueryDiscoveryAliasBinding {
    QueryDiscoveryAliasBinding {
        alias: string_field(value, "alias"),
        query_identifier: string_field(value, "query_identifier"),
        target_kind: string_field(value, "target_kind"),
        target_root: string_field(value, "target_root"),
        policy_scope: string_field(value, "policy_scope"),
        raw: value.clone(),
    }
}

fn relationship_record(value: &Value) -> QueryDiscoveryRelationship {
    QueryDiscoveryRelationship {
        id: string_field(value, "id"),
        from_root: string_field(value, "from_root"),
        to_root: string_field(value, "to_root"),
        association_root: string_field(value, "association_root"),
        raw: value.clone(),
    }
}

fn property_glossary_record(value: &Value) -> QueryDiscoveryPropertyGlossaryEntry {
    QueryDiscoveryPropertyGlossaryEntry {
        path: string_field(value, "path"),
        query_path: string_field(value, "query_path"),
        query_identifier_path: string_field(value, "query_identifier_path"),
        display_name: string_field(value, "display_name"),
        logical_type: string_field(value, "logical_type"),
        raw: value.clone(),
    }
}

fn query_template_record(value: &Value) -> Option<QueryTemplate> {
    let id = string_field(value, "id")?;
    Some(QueryTemplate {
        id,
        binding_mode: string_field(value, "binding_mode"),
        template_validation: template_validation_status(
            string_field(value, "template_validation").as_deref(),
        ),
        operator_chain: value_array(value.get("operator_chain"))
            .into_iter()
            .filter_map(operator_chain_record)
            .collect(),
        parameters: value_array(value.get("parameters"))
            .into_iter()
            .filter_map(template_parameter_record)
            .collect(),
        raw: value.clone(),
    })
}

fn template_parameter_record(value: &Value) -> Option<QueryTemplateParameterConstraint> {
    Some(QueryTemplateParameterConstraint {
        name: string_field(value, "name")?,
        kind: string_field(value, "kind"),
        raw: value.clone(),
    })
}

fn operator_chain_record(value: &Value) -> Option<QueryTemplateOperatorChain> {
    Some(QueryTemplateOperatorChain {
        method: string_field(value, "method")?,
        kind: string_field(value, "kind"),
        arg: string_field(value, "arg"),
        profiles: string_array_value(value.get("profiles")),
        capabilities: string_array_value(value.get("capabilities")),
        requires_sidecar: value.get("requires_sidecar").and_then(Value::as_bool),
        raw: value.clone(),
    })
}

fn query_example_record(value: &Value) -> Option<QueryExample> {
    Some(QueryExample {
        question: string_field(value, "question"),
        query: string_field(value, "query")?,
        query_validation: example_validation_status(
            string_field(value, "query_validation").as_deref(),
        ),
        profiles: string_array_value(value.get("profiles")),
        roots: string_array_value(value.get("roots")),
        raw: value.clone(),
    })
}

fn ai_capability_record(value: &Value) -> QueryAiCapability {
    QueryAiCapability {
        profile_available: value
            .get("profile_available")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        methods: value_array(value.get("methods"))
            .into_iter()
            .cloned()
            .collect(),
        sidecars: value_array(value.get("sidecars"))
            .into_iter()
            .cloned()
            .collect(),
        raw: value.clone(),
    }
}

fn policy_record(value: &Value) -> QueryDiscoveryPolicy {
    QueryDiscoveryPolicy {
        visibility_scope: string_field(value, "visibility_scope"),
        redaction_scope: string_field(value, "redaction_scope"),
        policy_fingerprint: string_field(value, "policy_fingerprint"),
        principal_class: string_field(value, "principal_class"),
        audience: string_field(value, "audience"),
        metadata_disclosure: string_field(value, "metadata_disclosure"),
        raw: value.clone(),
    }
}

fn budget_record(value: &Value) -> QueryDiscoveryBudget {
    QueryDiscoveryBudget {
        default_take: value.get("default_take").and_then(Value::as_u64),
        max_take: value.get("max_take").and_then(Value::as_u64),
        max_graph_depth: value.get("max_graph_depth").and_then(Value::as_u64),
        max_graph_paths: value.get("max_graph_paths").and_then(Value::as_u64),
        raw: value.clone(),
    }
}

fn template_validation_status(raw: Option<&str>) -> QueryTemplateValidationStatus {
    match raw {
        Some("operator_chain_validated") => QueryTemplateValidationStatus::OperatorChainValidated,
        Some("parameters_validated") => QueryTemplateValidationStatus::ParametersValidated,
        Some("representative_planned_dry_run") => {
            QueryTemplateValidationStatus::RepresentativePlannedDryRun
        }
        Some("planned_dry_run") => QueryTemplateValidationStatus::PlannedDryRun,
        Some("not_validated_policy_limited") => {
            QueryTemplateValidationStatus::NotValidatedPolicyLimited
        }
        Some("not_validated_budget_limited") => {
            QueryTemplateValidationStatus::NotValidatedBudgetLimited
        }
        Some("not_validated") | None => QueryTemplateValidationStatus::NotValidated,
        Some(other) => QueryTemplateValidationStatus::Unknown(other.to_string()),
    }
}

fn example_validation_status(raw: Option<&str>) -> QueryExampleValidationStatus {
    match raw {
        Some("parsed_and_resolved") => QueryExampleValidationStatus::ParsedAndResolved,
        Some("parsed_only") => QueryExampleValidationStatus::ParsedOnly,
        Some("planned_dry_run") => QueryExampleValidationStatus::PlannedDryRun,
        Some("not_validated_policy_limited") => {
            QueryExampleValidationStatus::NotValidatedPolicyLimited
        }
        Some("not_validated_budget_limited") => {
            QueryExampleValidationStatus::NotValidatedBudgetLimited
        }
        Some("not_validated") | None => QueryExampleValidationStatus::NotValidated,
        Some(other) => QueryExampleValidationStatus::Unknown(other.to_string()),
    }
}

fn value_array(value: Option<&Value>) -> Vec<&Value> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn string_array_value(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}
