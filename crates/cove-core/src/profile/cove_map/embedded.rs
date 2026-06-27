//! Internal COVE-MAP embedded-section parsing and validation helpers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{checksum, constants::DigestAlgorithm, digest::compute_digest};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

use super::*;

const COMPACT_EVIDENCE_MAGIC: &[u8; 8] = b"COVMEV1\0";
const COMPACT_EVIDENCE_VERSION: u16 = 1;
const COMPACT_EVIDENCE_HEADER_LEN: usize = 16;
const NONE_INDEX: u32 = u32::MAX;

impl EmbeddedMapSection {
    fn mapping_id(&self) -> &str {
        match self {
            Self::SourceCatalog(section) => &section.mapping_id,
            Self::FunctionRegistry(section) => &section.mapping_id,
            Self::ResolutionCatalog(section) => &section.mapping_id,
            Self::IdentityRuleCatalog(section) => &section.mapping_id,
            Self::RowSemanticsCatalog(section) => &section.mapping_id,
            Self::AssertionLog(section) => &section.mapping_id,
            Self::IdentityEquivalenceIndex(section) => &section.mapping_id,
            Self::EvidenceIndex(section) => &section.mapping_id,
            Self::ConversionReport(section) => &section.mapping_id,
            Self::ProjectionCatalog(section) => &section.mapping_id,
        }
    }

    fn mapping_version(&self) -> &str {
        match self {
            Self::SourceCatalog(section) => &section.mapping_version,
            Self::FunctionRegistry(section) => &section.mapping_version,
            Self::ResolutionCatalog(section) => &section.mapping_version,
            Self::IdentityRuleCatalog(section) => &section.mapping_version,
            Self::RowSemanticsCatalog(section) => &section.mapping_version,
            Self::AssertionLog(section) => &section.mapping_version,
            Self::IdentityEquivalenceIndex(section) => &section.mapping_version,
            Self::EvidenceIndex(section) => &section.mapping_version,
            Self::ConversionReport(section) => &section.mapping_version,
            Self::ProjectionCatalog(section) => &section.mapping_version,
        }
    }
}

pub(super) fn parse_embedded_section(
    kind: SectionKind,
    bytes: &[u8],
) -> Result<EmbeddedMapSection, CoveError> {
    match kind {
        SectionKind::MapSourceCatalog => {
            MapSourceCatalog::parse(bytes).map(EmbeddedMapSection::SourceCatalog)
        }
        SectionKind::MapFunctionRegistry => {
            MapFunctionRegistry::parse(bytes).map(EmbeddedMapSection::FunctionRegistry)
        }
        SectionKind::MapResolutionCatalog => {
            MapResolutionCatalog::parse(bytes).map(EmbeddedMapSection::ResolutionCatalog)
        }
        SectionKind::MapIdentityRuleCatalog => {
            MapIdentityRuleCatalog::parse(bytes).map(EmbeddedMapSection::IdentityRuleCatalog)
        }
        SectionKind::MapRowSemanticsCatalog => {
            MapRowSemanticsCatalog::parse(bytes).map(EmbeddedMapSection::RowSemanticsCatalog)
        }
        SectionKind::MapAssertionLog => {
            MapAssertionLog::parse(bytes).map(EmbeddedMapSection::AssertionLog)
        }
        SectionKind::MapIdentityEquivalenceIndex => MapIdentityEquivalenceIndex::parse(bytes)
            .map(EmbeddedMapSection::IdentityEquivalenceIndex),
        SectionKind::MapEvidenceIndex => {
            MapEvidenceIndex::parse(bytes).map(EmbeddedMapSection::EvidenceIndex)
        }
        SectionKind::MapConversionReport => {
            MapConversionReport::parse(bytes).map(EmbeddedMapSection::ConversionReport)
        }
        SectionKind::MapProjectionCatalog => {
            MapProjectionCatalog::parse(bytes).map(EmbeddedMapSection::ProjectionCatalog)
        }
        _ => Err(CoveError::MapInvalid),
    }
}

pub(super) fn validate_embedded_sections(sections: &[EmbeddedMapSection]) -> Result<(), CoveError> {
    if sections.is_empty() {
        return Ok(());
    }

    let mapping_id = sections[0].mapping_id();
    let mapping_version = sections[0].mapping_version();
    for section in sections.iter().skip(1) {
        if section.mapping_id() != mapping_id || section.mapping_version() != mapping_version {
            return Err(CoveError::MapInvalid);
        }
    }

    let mut sources = BTreeMap::<String, MapSourceEntry>::new();
    let mut function_ids = BTreeSet::<String>::new();
    let mut function_versions = BTreeSet::<(String, String)>::new();
    let mut referenced_function_ids = BTreeSet::<String>::new();
    let mut referenced_function_versions = BTreeSet::<(String, String)>::new();
    let mut identity_rule_ids = BTreeSet::<String>::new();
    let mut resolver_object_types = BTreeMap::<String, String>::new();
    let mut referenced_resolvers = BTreeSet::<(String, String)>::new();
    let mut do_not_merge = BTreeSet::<(String, String)>::new();
    let mut row_rules = BTreeMap::<String, MapRowSemanticRule>::new();
    let mut assertion_ids = BTreeSet::<String>::new();
    let mut output_object_ids = BTreeSet::<String>::new();
    let mut equivalence_pairs = Vec::<(String, String)>::new();
    let mut evidence_entries = Vec::<MapEvidenceEntry>::new();
    let mut observed_sources = Vec::<MapObservedSourceState>::new();
    let mut projections = Vec::<MapProjectionEntry>::new();

    for section in sections {
        match section {
            EmbeddedMapSection::SourceCatalog(catalog) => {
                for source in &catalog.sources {
                    if sources
                        .insert(source.source_id.clone(), source.clone())
                        .is_some()
                    {
                        return Err(CoveError::MapInvalid);
                    }
                }
            }
            EmbeddedMapSection::FunctionRegistry(registry) => {
                for function in &registry.functions {
                    function_ids.insert(function.function_id.clone());
                    if !function_versions
                        .insert((function.function_id.clone(), function.version.clone()))
                    {
                        return Err(CoveError::MapInvalid);
                    }
                    if !function.deterministic
                        || matches!(
                            function.dependency.as_str(),
                            "random"
                                | "wall_clock"
                                | "locale_default"
                                | "network"
                                | "mutable_external"
                        )
                    {
                        return Err(CoveError::MapInvalid);
                    }
                }
            }
            EmbeddedMapSection::ResolutionCatalog(catalog) => {
                for pipeline in &catalog.normalization_pipelines {
                    for function in &pipeline.functions {
                        referenced_function_versions
                            .insert((function.function_id.clone(), function.version.clone()));
                    }
                }
                for resolver in &catalog.resolvers {
                    if resolver_object_types
                        .insert(resolver.resolver_id.clone(), resolver.object_type.clone())
                        .is_some()
                    {
                        return Err(CoveError::MapInvalid);
                    }
                }
                for rule in &catalog.match_rules {
                    if !catalog
                        .normalization_pipelines
                        .iter()
                        .any(|pipeline| pipeline.pipeline_id == rule.normalization_pipeline_id)
                    {
                        return Err(CoveError::MapInvalid);
                    }
                }
            }
            EmbeddedMapSection::IdentityRuleCatalog(catalog) => {
                for rule in &catalog.identity_rules {
                    if !identity_rule_ids.insert(rule.rule_id.clone()) {
                        return Err(CoveError::MapInvalid);
                    }
                    referenced_function_ids.extend(rule.function_ids.iter().cloned());
                    referenced_resolvers.extend(
                        rule.join_keys
                            .iter()
                            .filter_map(|component| component.resolution.as_ref())
                            .map(|resolution| {
                                (resolution.resolver_id.clone(), rule.object_type.clone())
                            }),
                    );
                }
                for constraint in &catalog.do_not_merge {
                    let pair =
                        normalize_pair(&constraint.left_identity, &constraint.right_identity)?;
                    do_not_merge.insert(pair);
                }
            }
            EmbeddedMapSection::RowSemanticsCatalog(catalog) => {
                for rule in &catalog.rules {
                    if row_rules
                        .insert(rule.rule_id.clone(), rule.clone())
                        .is_some()
                    {
                        return Err(CoveError::MapInvalid);
                    }
                    referenced_function_ids.extend(rule.function_ids.iter().cloned());
                }
            }
            EmbeddedMapSection::AssertionLog(log) => {
                for assertion in &log.assertions {
                    if !assertion_ids.insert(assertion.assertion_id.clone()) {
                        return Err(CoveError::MapInvalid);
                    }
                    if !output_object_ids.insert(assertion.output_object_id.clone()) {
                        return Err(CoveError::MapInvalid);
                    }
                }
            }
            EmbeddedMapSection::IdentityEquivalenceIndex(index) => {
                for pair in &index.equivalences {
                    equivalence_pairs
                        .push(normalize_pair(&pair.left_identity, &pair.right_identity)?);
                }
            }
            EmbeddedMapSection::EvidenceIndex(index) => {
                evidence_entries.extend(index.entries.iter().cloned());
            }
            EmbeddedMapSection::ConversionReport(report) => {
                observed_sources.extend(report.sources.iter().cloned());
            }
            EmbeddedMapSection::ProjectionCatalog(catalog) => {
                projections.extend(catalog.projections.iter().cloned());
            }
        }
    }

    for function_id in referenced_function_ids {
        if !function_ids.contains(&function_id) {
            return Err(CoveError::MapFunctionUndeclared);
        }
    }

    for function_version in referenced_function_versions {
        if !function_versions.contains(&function_version) {
            return Err(CoveError::MapFunctionUndeclared);
        }
    }

    for (resolver_id, object_type) in referenced_resolvers {
        match resolver_object_types.get(&resolver_id) {
            Some(resolver_object_type) if resolver_object_type == &object_type => {}
            _ => return Err(CoveError::MapInvalid),
        }
    }

    for rule in row_rules.values() {
        if !sources.contains_key(&rule.source_id)
            || !identity_rule_ids.contains(&rule.identity_rule_id)
        {
            return Err(CoveError::MapInvalid);
        }
        validate_row_semantic_rule_shape(rule)?;
        if !assertion_ids.is_empty()
            && rule
                .output_assertion_ids
                .iter()
                .any(|assertion_id| !assertion_ids.contains(assertion_id))
        {
            return Err(CoveError::MapInvalid);
        }
        if rule
            .association_endpoints
            .iter()
            .any(|identity_id| !identity_rule_ids.contains(identity_id))
        {
            return Err(CoveError::MapInvalid);
        }
        if !assertion_ids.is_empty()
            && rule
                .property_bindings
                .iter()
                .any(|binding| !assertion_ids.contains(&binding.assertion_id))
        {
            return Err(CoveError::MapInvalid);
        }
        if !assertion_ids.is_empty()
            && rule
                .association_bindings
                .iter()
                .any(|binding| !assertion_ids.contains(&binding.assertion_id))
        {
            return Err(CoveError::MapInvalid);
        }
        if rule.association_bindings.iter().any(|binding| {
            !identity_rule_ids.contains(&binding.target_identity_rule_id)
                || (!binding.source_identity_rule_id.is_empty()
                    && !identity_rule_ids.contains(&binding.source_identity_rule_id))
        }) {
            return Err(CoveError::MapInvalid);
        }
    }

    for pair in equivalence_pairs {
        if do_not_merge.contains(&pair) {
            return Err(CoveError::MapIdentityConflict);
        }
    }

    for source_state in observed_sources {
        let Some(source) = sources.get(&source_state.source_id) else {
            return Err(CoveError::MapSourceStale);
        };
        if source_state
            .schema_fingerprint
            .as_ref()
            .zip(source.schema_fingerprint.as_ref())
            .is_some_and(|(observed, expected)| observed != expected)
            || source_state
                .snapshot_digest
                .as_ref()
                .zip(source.snapshot_digest.as_ref())
                .is_some_and(|(observed, expected)| observed != expected)
        {
            return Err(CoveError::MapSourceStale);
        }
    }

    for evidence in evidence_entries {
        let Some(source) = sources.get(&evidence.source_id) else {
            return Err(CoveError::MapEvidenceInvalid);
        };
        if !row_rules.contains_key(&evidence.rule_id)
            || !assertion_ids.contains(&evidence.assertion_id)
            || !output_object_ids.contains(&evidence.output_object_id)
        {
            return Err(CoveError::MapEvidenceInvalid);
        }
        if evidence
            .observed_schema_fingerprint
            .as_ref()
            .zip(source.schema_fingerprint.as_ref())
            .is_some_and(|(observed, expected)| observed != expected)
            || evidence
                .observed_snapshot_digest
                .as_ref()
                .zip(source.snapshot_digest.as_ref())
                .is_some_and(|(observed, expected)| observed != expected)
        {
            return Err(CoveError::MapSourceStale);
        }
    }

    for projection in projections {
        if !assertion_ids.is_empty()
            && projection
                .assertion_ids
                .iter()
                .any(|assertion_id| !assertion_ids.contains(assertion_id))
        {
            return Err(CoveError::MapEvidenceInvalid);
        }
        let expanded = projection.output_table.is_some()
            || projection.row_grain.is_some()
            || projection.anchor.is_some()
            || !projection.columns.is_empty()
            || !projection.output_modes.is_empty();
        if expanded {
            if projection.output_table.is_none()
                || projection.row_grain.is_none()
                || projection.anchor.is_none()
                || projection.temporal_mode.is_none()
                || projection.multi_value_policy.is_none()
                || projection.columns.is_empty()
                || projection.output_modes.is_empty()
            {
                return Err(CoveError::MapInvalid);
            }
            let row_grain = projection
                .row_grain
                .as_deref()
                .ok_or(CoveError::MapInvalid)?;
            if !is_valid_projection_row_grain(row_grain) {
                return Err(CoveError::MapInvalid);
            }
            if !projection
                .temporal_mode
                .as_deref()
                .is_some_and(is_valid_temporal_mode)
            {
                return Err(CoveError::MapInvalid);
            }
            if !projection
                .multi_value_policy
                .as_deref()
                .is_some_and(is_valid_multi_value_policy)
            {
                return Err(CoveError::MapInvalid);
            }
            if projection
                .output_modes
                .iter()
                .any(|mode| !is_valid_projection_output_mode(mode))
            {
                return Err(CoveError::MapInvalid);
            }
            let anchor = projection.anchor.as_ref().ok_or(CoveError::MapInvalid)?;
            if anchor.object_type.is_some() == anchor.association_type.is_some() {
                return Err(CoveError::MapInvalid);
            }
            match row_grain {
                "one_row_per_object"
                | "one_row_per_property_version"
                | "one_row_per_event_object"
                | "one_row_per_object_as_of_time"
                    if anchor.object_type.is_none() =>
                {
                    return Err(CoveError::MapInvalid);
                }
                "one_row_per_association" | "one_row_per_link_object"
                    if anchor.association_type.is_none() =>
                {
                    return Err(CoveError::MapInvalid);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn validate_row_semantic_rule_shape(rule: &MapRowSemanticRule) -> Result<(), CoveError> {
    let has = |kind: &str| rule.assertion_kinds.iter().any(|value| value == kind);
    match rule.row_semantics_kind.as_str() {
        "Object" | "EventObject" | "LinkObject" => {
            if !has("object") {
                return Err(CoveError::MapInvalid);
            }
        }
        "AssociationOnly" => {
            if !has("association") || has("object") {
                return Err(CoveError::MapInvalid);
            }
        }
        "Composite" | "Dispatched" => {
            if rule.assertion_kinds.len() < 2 {
                return Err(CoveError::MapInvalid);
            }
        }
        "ProjectionOnly" => {
            if rule
                .assertion_kinds
                .iter()
                .any(|kind| !matches!(kind.as_str(), "projection" | "evidence" | "candidate_match"))
            {
                return Err(CoveError::MapInvalid);
            }
        }
        "EvidenceOnly" => {
            if rule
                .assertion_kinds
                .iter()
                .any(|kind| !matches!(kind.as_str(), "evidence" | "candidate_match" | "conflict"))
            {
                return Err(CoveError::MapInvalid);
            }
        }
        "Tombstone" => {
            if !has("tombstone") || rule.tombstone_target.is_none() {
                return Err(CoveError::MapInvalid);
            }
        }
        "KeyValueFragment" => {
            if !has("property") {
                return Err(CoveError::MapInvalid);
            }
        }
        _ => return Err(CoveError::MapInvalid),
    }
    match rule.source_operation_kind {
        SourceOperationKind::PatchProperty if rule.property_bindings.is_empty() => {
            return Err(CoveError::MapInvalid);
        }
        SourceOperationKind::CloseAssociation if rule.association_bindings.is_empty() => {
            return Err(CoveError::MapInvalid);
        }
        SourceOperationKind::TombstoneObject
        | SourceOperationKind::TombstoneProperty
        | SourceOperationKind::TombstoneAssociation
            if rule.tombstone_target.is_none() =>
        {
            return Err(CoveError::MapInvalid);
        }
        SourceOperationKind::EvidenceOnly if rule.row_semantics_kind != "EvidenceOnly" => {
            return Err(CoveError::MapInvalid);
        }
        _ => {}
    }
    Ok(())
}

impl MapSourceCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapSourceCatalog, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let governance_reconciliation_policy =
            optional_non_empty_str(object, "governance_reconciliation_policy")?
                .unwrap_or_else(|| "emit_effective_policy".to_string());
        if !matches!(
            governance_reconciliation_policy.as_str(),
            "emit_effective_policy" | "reject_on_mixed_sensitivity"
        ) {
            return Err(CoveError::MapInvalid);
        }
        let mut sources = Vec::new();
        if let Some(values) = optional_array(object, "sources")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &[
                        "source_id",
                        "source_kind",
                        "schema_fingerprint",
                        "snapshot_digest",
                        "row_identity_rules",
                        "replay_claimed",
                        "source_priority",
                        "sensitivity_label",
                        "sensitivity_rank",
                        "access_policy_ids",
                    ],
                )?;
                let row_identity_rules = string_list(entry, "row_identity_rules")?;
                if row_identity_rules.is_empty() {
                    return Err(CoveError::MapInvalid);
                }
                let schema_fingerprint = optional_non_empty_str(entry, "schema_fingerprint")?;
                let snapshot_digest = optional_non_empty_str(entry, "snapshot_digest")?;
                let replay_claimed = optional_bool(entry, "replay_claimed", false)?;
                if replay_claimed && (schema_fingerprint.is_none() || snapshot_digest.is_none()) {
                    return Err(CoveError::MapInvalid);
                }
                sources.push(MapSourceEntry {
                    source_id: required_non_empty_str(entry, "source_id")?,
                    schema_fingerprint,
                    snapshot_digest,
                    row_identity_rules,
                    replay_claimed,
                    source_priority: optional_i64(entry, "source_priority")?,
                    sensitivity_label: optional_non_empty_str(entry, "sensitivity_label")?,
                    sensitivity_rank: optional_i64(entry, "sensitivity_rank")?,
                    access_policy_ids: optional_string_list(entry, "access_policy_ids")?,
                });
            }
        }
        Ok(Self {
            mapping_id,
            mapping_version,
            governance_reconciliation_policy,
            sources,
        })
    }
}

impl MapFunctionRegistry {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapFunctionRegistry, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let mut functions = Vec::new();
        if let Some(values) = optional_array(object, "functions")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &["function_id", "version", "deterministic", "dependency"],
                )?;
                functions.push(MapFunctionEntry {
                    function_id: required_non_empty_str(entry, "function_id")?,
                    version: required_non_empty_str(entry, "version")?,
                    deterministic: required_bool(entry, "deterministic")?,
                    dependency: optional_non_empty_str(entry, "dependency")?
                        .unwrap_or_else(|| "pure".to_string()),
                });
            }
        }
        Ok(Self {
            mapping_id,
            mapping_version,
            functions,
        })
    }
}

impl MapResolutionCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapResolutionCatalog, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;

        let mut normalization_pipelines = Vec::new();
        let mut pipeline_ids = BTreeSet::new();
        let mut pipeline_digests = BTreeMap::<String, String>::new();
        for value in required_array(object, "normalization_pipelines")? {
            let pipeline = as_object(value)?;
            validate_keys(pipeline, &["pipeline_id", "functions", "tables"])?;
            let pipeline_id = required_non_empty_str(pipeline, "pipeline_id")?;
            if !pipeline_ids.insert(pipeline_id.clone()) {
                return Err(CoveError::MapInvalid);
            }

            let mut functions = Vec::new();
            for value in required_array(pipeline, "functions")? {
                let function = as_object(value)?;
                validate_keys(
                    function,
                    &["function_id", "version", "table_id", "suffix_table_digest"],
                )?;
                functions.push(MapNormalizationFunction {
                    function_id: required_non_empty_str(function, "function_id")?,
                    version: required_non_empty_str(function, "version")?,
                    table_id: optional_non_empty_str(function, "table_id")?,
                    suffix_table_digest: optional_non_empty_str(function, "suffix_table_digest")?,
                });
            }
            if functions.is_empty() {
                return Err(CoveError::MapInvalid);
            }

            let mut tables = Vec::new();
            if let Some(values) = optional_array(pipeline, "tables")? {
                let mut table_ids = BTreeSet::new();
                for value in values {
                    let table = as_object(value)?;
                    validate_keys(table, &["table_id", "digest", "values"])?;
                    let table_id = required_non_empty_str(table, "table_id")?;
                    if !table_ids.insert(table_id.clone()) {
                        return Err(CoveError::MapInvalid);
                    }
                    let digest = required_sha256_digest(table, "digest")?;
                    tables.push(MapNormalizationTable {
                        table_id,
                        digest,
                        values: optional_string_list(table, "values")?,
                    });
                }
            }
            for function in &functions {
                if let Some(table_id) = &function.table_id {
                    if !tables.iter().any(|table| &table.table_id == table_id) {
                        return Err(CoveError::MapInvalid);
                    }
                }
                if let Some(digest) = &function.suffix_table_digest {
                    validate_sha256_digest_string(digest)?;
                }
            }

            let digest_input = pipeline_digest_input(&pipeline_id, &functions, &tables)?;
            let pipeline_digest = sha256_digest_string(&canonical_json(&digest_input)?)?;
            pipeline_digests.insert(pipeline_id.clone(), pipeline_digest);
            normalization_pipelines.push(MapNormalizationPipeline {
                pipeline_id,
                functions,
                tables,
            });
        }

        let mut resolvers = Vec::new();
        let mut resolver_ids = BTreeSet::new();
        for value in required_array(object, "resolvers")? {
            let resolver = as_object(value)?;
            validate_keys(
                resolver,
                &[
                    "resolver_id",
                    "kind",
                    "object_type",
                    "authority",
                    "confidence_class",
                    "normalization_pipeline_id",
                    "on_hit",
                    "on_miss",
                    "miss_confidence_class",
                    "ambiguous_policy",
                    "catalog_digest",
                    "pipeline_digest",
                    "resolver_digest",
                    "order_sensitive_catalog",
                    "evidence_policy",
                    "alias_catalog",
                ],
            )?;
            let resolver_id = required_non_empty_str(resolver, "resolver_id")?;
            if !resolver_ids.insert(resolver_id.clone()) {
                return Err(CoveError::MapInvalid);
            }
            let kind = required_non_empty_str(resolver, "kind")?;
            if kind != "alias_catalog" {
                return Err(CoveError::MapInvalid);
            }
            let object_type = required_non_empty_str(resolver, "object_type")?;
            let authority = required_non_empty_str(resolver, "authority")?;
            let confidence_class = required_non_empty_str(resolver, "confidence_class")?;
            let normalization_pipeline_id =
                required_non_empty_str(resolver, "normalization_pipeline_id")?;
            let expected_pipeline_digest = pipeline_digests
                .get(&normalization_pipeline_id)
                .ok_or(CoveError::MapInvalid)?;
            let on_hit = required_non_empty_str(resolver, "on_hit")?;
            if on_hit != "canonical_key" {
                return Err(CoveError::MapInvalid);
            }
            let on_miss = required_non_empty_str(resolver, "on_miss")?;
            if !matches!(
                on_miss.as_str(),
                "reject" | "normalized_value" | "candidate_only" | "source_scoped"
            ) {
                return Err(CoveError::MapInvalid);
            }
            let miss_confidence_class = optional_non_empty_str(resolver, "miss_confidence_class")?;
            if on_miss == "normalized_value" {
                if !matches!(
                    miss_confidence_class.as_deref(),
                    Some("strong_deterministic" | "weak_deterministic")
                ) {
                    return Err(CoveError::MapInvalid);
                }
            } else if miss_confidence_class.as_deref() == Some("authoritative") {
                return Err(CoveError::MapInvalid);
            }
            let ambiguous_policy = optional_non_empty_str(resolver, "ambiguous_policy")?
                .unwrap_or_else(|| "reject_auto_merge".to_string());
            if !matches!(
                ambiguous_policy.as_str(),
                "reject_auto_merge" | "candidate_only" | "reject"
            ) {
                return Err(CoveError::MapInvalid);
            }
            let order_sensitive_catalog =
                optional_bool(resolver, "order_sensitive_catalog", false)?;
            let evidence_policy = optional_non_empty_str(resolver, "evidence_policy")?
                .unwrap_or_else(|| "retain_raw".to_string());
            if !matches!(evidence_policy.as_str(), "retain_raw" | "redact_raw") {
                return Err(CoveError::MapInvalid);
            }
            let catalog_digest = required_sha256_digest(resolver, "catalog_digest")?;
            let pipeline_digest = required_sha256_digest(resolver, "pipeline_digest")?;
            if &pipeline_digest != expected_pipeline_digest {
                return Err(CoveError::DigestMismatch);
            }
            let resolver_digest = required_sha256_digest(resolver, "resolver_digest")?;
            let alias_catalog_value = resolver.get("alias_catalog").ok_or(CoveError::MapInvalid)?;
            let alias_catalog_object = as_object(alias_catalog_value)?;
            let alias_catalog = parse_alias_catalog(
                alias_catalog_object,
                &ambiguous_policy,
                order_sensitive_catalog,
            )?;
            let expected_catalog_digest = sha256_digest_string(&canonical_json(
                &alias_catalog_digest_input(&alias_catalog, order_sensitive_catalog)?,
            )?)?;
            if catalog_digest != expected_catalog_digest {
                return Err(CoveError::DigestMismatch);
            }

            let expected_resolver_digest =
                sha256_digest_string(&canonical_json(&resolver_digest_input(
                    &resolver_id,
                    &kind,
                    &object_type,
                    &authority,
                    &confidence_class,
                    &normalization_pipeline_id,
                    &pipeline_digest,
                    &on_hit,
                    &on_miss,
                    miss_confidence_class.as_deref(),
                    &ambiguous_policy,
                    &catalog_digest,
                    &evidence_policy,
                )?)?)?;
            if resolver_digest != expected_resolver_digest {
                return Err(CoveError::DigestMismatch);
            }

            resolvers.push(MapResolver {
                resolver_id,
                kind,
                object_type,
                authority,
                confidence_class,
                normalization_pipeline_id,
                on_hit,
                on_miss,
                miss_confidence_class,
                ambiguous_policy,
                catalog_digest,
                pipeline_digest,
                resolver_digest,
                order_sensitive_catalog,
                evidence_policy,
                alias_catalog: Some(alias_catalog),
            });
        }

        let mut match_rules = Vec::new();
        let mut match_rule_ids = BTreeSet::new();
        for value in required_array(object, "match_rules")? {
            let rule = parse_candidate_match_rule(as_object(value)?)?;
            if !match_rule_ids.insert(rule.match_rule_id.clone()) {
                return Err(CoveError::MapInvalid);
            }
            if !pipeline_ids.contains(&rule.normalization_pipeline_id) {
                return Err(CoveError::MapInvalid);
            }
            match_rules.push(rule);
        }

        let mut reviewed_decisions = Vec::new();
        let mut reviewed_decision_ids = BTreeSet::new();
        for value in required_array(object, "reviewed_decisions")? {
            let decision = parse_reviewed_decision(as_object(value)?)?;
            if !reviewed_decision_ids.insert(decision.decision_id.clone()) {
                return Err(CoveError::MapInvalid);
            }
            reviewed_decisions.push(decision);
        }

        Ok(Self {
            mapping_id,
            mapping_version,
            normalization_pipelines,
            resolvers,
            match_rules,
            reviewed_decisions,
        })
    }
}

fn parse_alias_catalog(
    object: &Map<String, Value>,
    ambiguous_policy: &str,
    order_sensitive_catalog: bool,
) -> Result<MapAliasCatalog, CoveError> {
    validate_keys(object, &["alias_catalog_id", "entries"])?;
    let alias_catalog_id = required_non_empty_str(object, "alias_catalog_id")?;
    let mut entries = Vec::new();
    let mut entry_ids = BTreeSet::new();
    let mut alias_targets = BTreeMap::<String, BTreeSet<String>>::new();
    let mut alias_ambiguity = BTreeMap::<String, bool>::new();
    for value in required_array(object, "entries")? {
        let entry = as_object(value)?;
        validate_keys(
            entry,
            &[
                "alias_entry_id",
                "canonical_key",
                "canonical_label",
                "aliases",
                "ambiguous",
                "metadata",
                "non_semantic_metadata",
            ],
        )?;
        let alias_entry_id = required_non_empty_str(entry, "alias_entry_id")?;
        if !entry_ids.insert(alias_entry_id.clone()) {
            return Err(CoveError::MapInvalid);
        }
        let canonical_key = required_non_empty_str(entry, "canonical_key")?;
        let canonical_label = required_non_empty_str(entry, "canonical_label")?;
        let aliases = string_list(entry, "aliases")?;
        if aliases.is_empty() {
            return Err(CoveError::MapInvalid);
        }
        let ambiguous = optional_bool(entry, "ambiguous", false)?;
        for alias in &aliases {
            alias_targets
                .entry(alias.clone())
                .or_default()
                .insert(canonical_key.clone());
            alias_ambiguity
                .entry(alias.clone())
                .and_modify(|existing| *existing &= ambiguous)
                .or_insert(ambiguous);
        }
        entries.push(MapAliasEntry {
            alias_entry_id,
            canonical_key,
            canonical_label,
            aliases,
            ambiguous,
            metadata: optional_value_object(entry, "metadata")?,
            non_semantic_metadata: optional_value_object(entry, "non_semantic_metadata")?,
        });
    }
    if !order_sensitive_catalog {
        entries.sort_by(|left, right| left.alias_entry_id.cmp(&right.alias_entry_id));
    }
    let resolver_marks_ambiguous = ambiguous_policy == "candidate_only";
    for (alias, targets) in alias_targets {
        if targets.len() > 1
            && !resolver_marks_ambiguous
            && !alias_ambiguity.get(&alias).copied().unwrap_or(false)
        {
            return Err(CoveError::MapInvalid);
        }
    }
    Ok(MapAliasCatalog {
        alias_catalog_id,
        entries,
    })
}

fn parse_candidate_match_rule(
    object: &Map<String, Value>,
) -> Result<MapCandidateMatchRule, CoveError> {
    validate_keys(
        object,
        &[
            "match_rule_id",
            "object_type",
            "inputs",
            "blocking",
            "normalization_pipeline_id",
            "scoring",
            "limits",
            "output",
        ],
    )?;
    let mut inputs = Vec::new();
    for value in required_array(object, "inputs")? {
        let input = as_object(value)?;
        validate_keys(input, &["source_id", "column"])?;
        inputs.push(MapCandidateMatchInput {
            source_id: required_non_empty_str(input, "source_id")?,
            column: required_non_empty_str(input, "column")?,
        });
    }
    if inputs.is_empty() {
        return Err(CoveError::MapInvalid);
    }

    let scoring = required_value_object(object, "scoring")?;
    if scoring.get("merge_behavior").and_then(Value::as_str) != Some("never") {
        return Err(CoveError::MapInvalid);
    }
    let limits_object = object
        .get("limits")
        .and_then(Value::as_object)
        .ok_or(CoveError::MapInvalid)?;
    validate_keys(
        limits_object,
        &["max_pairs_per_block", "max_pairs_total", "on_limit"],
    )?;
    let on_limit = required_non_empty_str(limits_object, "on_limit")?;
    if !matches!(
        on_limit.as_str(),
        "fail_closed" | "emit_diagnostic_and_truncate"
    ) {
        return Err(CoveError::MapInvalid);
    }

    Ok(MapCandidateMatchRule {
        match_rule_id: required_non_empty_str(object, "match_rule_id")?,
        object_type: required_non_empty_str(object, "object_type")?,
        inputs,
        blocking: required_value_object(object, "blocking")?,
        normalization_pipeline_id: required_non_empty_str(object, "normalization_pipeline_id")?,
        scoring,
        limits: MapCandidateMatchLimits {
            max_pairs_per_block: required_u64(limits_object, "max_pairs_per_block")?,
            max_pairs_total: required_u64(limits_object, "max_pairs_total")?,
            on_limit,
        },
        output: required_value_object(object, "output")?,
    })
}

fn parse_reviewed_decision(object: &Map<String, Value>) -> Result<MapReviewedDecision, CoveError> {
    validate_keys(
        object,
        &[
            "decision_id",
            "decision",
            "confidence_class",
            "reviewed_by",
            "reviewed_at",
            "reason",
            "left",
            "right",
            "canonical_anchor",
        ],
    )?;
    let decision = required_non_empty_str(object, "decision")?;
    if !matches!(decision.as_str(), "same_object" | "do_not_merge") {
        return Err(CoveError::MapInvalid);
    }
    Ok(MapReviewedDecision {
        decision_id: required_non_empty_str(object, "decision_id")?,
        decision,
        confidence_class: required_non_empty_str(object, "confidence_class")?,
        reviewed_by: required_non_empty_str(object, "reviewed_by")?,
        reviewed_at: required_non_empty_str(object, "reviewed_at")?,
        reason: optional_non_empty_str(object, "reason")?,
        left: parse_typed_identity_reference(
            object
                .get("left")
                .and_then(Value::as_object)
                .ok_or(CoveError::MapInvalid)?,
        )?,
        right: parse_typed_identity_reference(
            object
                .get("right")
                .and_then(Value::as_object)
                .ok_or(CoveError::MapInvalid)?,
        )?,
        canonical_anchor: match object.get("canonical_anchor") {
            Some(value) => Some(parse_canonical_anchor(as_object(value)?)?),
            None => None,
        },
    })
}

fn parse_typed_identity_reference(
    object: &Map<String, Value>,
) -> Result<MapTypedIdentityReference, CoveError> {
    validate_keys(
        object,
        &[
            "kind",
            "object_type",
            "identity_rule_id",
            "resolver_id",
            "canonical_key",
            "join_key_sha256",
            "source_id",
            "source_row_identity",
            "source_snapshot_digest",
            "schema_fingerprint",
            "row_digest",
            "identity_alias",
        ],
    )?;
    let kind = required_non_empty_str(object, "kind")?;
    let reference = MapTypedIdentityReference {
        kind: kind.clone(),
        object_type: required_non_empty_str(object, "object_type")?,
        identity_rule_id: optional_non_empty_str(object, "identity_rule_id")?,
        resolver_id: optional_non_empty_str(object, "resolver_id")?,
        canonical_key: optional_non_empty_str(object, "canonical_key")?,
        join_key_sha256: optional_non_empty_str(object, "join_key_sha256")?,
        source_id: optional_non_empty_str(object, "source_id")?,
        source_row_identity: optional_non_empty_str(object, "source_row_identity")?,
        source_snapshot_digest: optional_non_empty_str(object, "source_snapshot_digest")?,
        schema_fingerprint: optional_non_empty_str(object, "schema_fingerprint")?,
        row_digest: optional_non_empty_str(object, "row_digest")?,
        identity_alias: optional_non_empty_str(object, "identity_alias")?,
    };
    let valid = match kind.as_str() {
        "identity_join_key" => {
            reference.identity_rule_id.is_some() && reference.join_key_sha256.is_some()
        }
        "resolver_key" => reference.resolver_id.is_some() && reference.canonical_key.is_some(),
        "source_row" => {
            reference.identity_rule_id.is_some()
                && reference.source_id.is_some()
                && reference.source_row_identity.is_some()
                && reference.source_snapshot_digest.is_some()
                && reference.schema_fingerprint.is_some()
        }
        "row_digest" => reference.row_digest.is_some(),
        "identity_alias" => reference.identity_alias.is_some(),
        _ => false,
    };
    valid.then_some(reference).ok_or(CoveError::MapInvalid)
}

fn parse_canonical_anchor(object: &Map<String, Value>) -> Result<MapCanonicalAnchor, CoveError> {
    validate_keys(
        object,
        &["kind", "object_type", "identity_rule_id", "components"],
    )?;
    let mut components = Vec::new();
    for value in required_array(object, "components")? {
        let component = as_object(value)?;
        validate_keys(component, &["role_id", "logical_type", "resolved_value"])?;
        components.push(MapCanonicalAnchorComponent {
            role_id: required_non_empty_str(component, "role_id")?,
            logical_type: required_non_empty_str(component, "logical_type")?,
            resolved_value: required_non_empty_str(component, "resolved_value")?,
        });
    }
    if components.is_empty() {
        return Err(CoveError::MapInvalid);
    }
    Ok(MapCanonicalAnchor {
        kind: required_non_empty_str(object, "kind")?,
        object_type: required_non_empty_str(object, "object_type")?,
        identity_rule_id: required_non_empty_str(object, "identity_rule_id")?,
        components,
    })
}

impl MapIdentityRuleCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapIdentityRuleCatalog, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let mut identity_rules = Vec::new();
        if let Some(values) = optional_array(object, "identity_rules")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &[
                        "rule_id",
                        "object_type",
                        "semantic_role",
                        "confidence_class",
                        "auto_merge",
                        "candidate_only",
                        "property_conflicts_declared",
                        "allow_reviewed_equivalence",
                        "function_ids",
                        "join_keys",
                    ],
                )?;
                let mut join_keys = Vec::new();
                for join_key in required_array(entry, "join_keys")? {
                    let join_key = as_object(join_key)?;
                    validate_keys(
                        join_key,
                        &[
                            "role_id",
                            "source_column",
                            "logical_type",
                            "canonicalization",
                            "null_policy",
                            "ordering",
                            "resolution",
                        ],
                    )?;
                    let resolution = match join_key.get("resolution") {
                        Some(value) => {
                            let resolution = as_object(value)?;
                            validate_keys(resolution, &["resolver_id"])?;
                            let canonicalization =
                                required_non_empty_str(join_key, "canonicalization")?;
                            if !matches!(canonicalization.as_str(), "identity" | "none") {
                                return Err(CoveError::MapInvalid);
                            }
                            Some(MapResolutionBinding {
                                resolver_id: required_non_empty_str(resolution, "resolver_id")?,
                            })
                        }
                        None => None,
                    };
                    join_keys.push(MapJoinKeyComponent {
                        role_id: required_non_empty_str(join_key, "role_id")?,
                        source_column: required_non_empty_str(join_key, "source_column")?,
                        logical_type: required_non_empty_str(join_key, "logical_type")?,
                        canonicalization: required_non_empty_str(join_key, "canonicalization")?,
                        null_policy: required_non_empty_str(join_key, "null_policy")?,
                        ordering: required_non_empty_str(join_key, "ordering")?,
                        resolution,
                    });
                }
                if join_keys.is_empty() {
                    return Err(CoveError::MapInvalid);
                }
                identity_rules.push(MapIdentityRule {
                    rule_id: required_non_empty_str(entry, "rule_id")?,
                    object_type: required_non_empty_str(entry, "object_type")?,
                    semantic_role: required_non_empty_str(entry, "semantic_role")?,
                    confidence_class: required_non_empty_str(entry, "confidence_class")?,
                    auto_merge: optional_bool_value(entry, "auto_merge")?,
                    candidate_only: optional_bool(entry, "candidate_only", false)?,
                    property_conflicts_declared: required_bool(
                        entry,
                        "property_conflicts_declared",
                    )?,
                    allow_reviewed_equivalence: optional_bool(
                        entry,
                        "allow_reviewed_equivalence",
                        false,
                    )?,
                    function_ids: optional_string_list(entry, "function_ids")?,
                    join_keys,
                });
            }
        }
        let mut do_not_merge = Vec::new();
        if let Some(values) = optional_array(object, "do_not_merge")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(entry, &["left_identity", "right_identity"])?;
                do_not_merge.push(MapDoNotMergeConstraint {
                    left_identity: required_non_empty_str(entry, "left_identity")?,
                    right_identity: required_non_empty_str(entry, "right_identity")?,
                });
            }
        }
        Ok(Self {
            mapping_id,
            mapping_version,
            identity_rules,
            do_not_merge,
        })
    }
}

impl MapRowSemanticsCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapRowSemanticsCatalog, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let mut rules = Vec::new();
        if let Some(values) = optional_array(object, "rules")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &[
                        "rule_id",
                        "source_id",
                        "identity_rule_id",
                        "row_semantics_kind",
                        "kind",
                        "source_operation_kind",
                        "operation_kind",
                        "assertion_kinds",
                        "tombstone_target",
                        "record_kind",
                        "temporal_policy",
                        "conflict_policy",
                        "function_ids",
                        "output_assertion_ids",
                        "association_endpoints",
                        "property_bindings",
                        "association_bindings",
                    ],
                )?;
                let property_bindings =
                    parse_property_bindings(optional_array(entry, "property_bindings")?)?;
                let association_bindings =
                    parse_association_bindings(optional_array(entry, "association_bindings")?)?;
                let row_semantics_kind = optional_non_empty_str(entry, "row_semantics_kind")?
                    .or_else(|| optional_non_empty_str(entry, "kind").ok().flatten())
                    .unwrap_or_else(|| "Object".to_string());
                validate_row_semantics_kind(&row_semantics_kind)?;
                let source_operation_kind =
                    parse_source_operation_kind(entry, &row_semantics_kind)?;
                let assertion_kinds = string_list(entry, "assertion_kinds")?;
                if assertion_kinds.is_empty() {
                    return Err(CoveError::MapInvalid);
                }
                for kind in &assertion_kinds {
                    validate_assertion_kind(kind)?;
                }
                let tombstone_target = optional_non_empty_str(entry, "tombstone_target")?;
                match row_semantics_kind.as_str() {
                    "Tombstone" => match tombstone_target.as_deref() {
                        Some(target) if is_valid_tombstone_target(target) => {}
                        _ => return Err(CoveError::MapInvalid),
                    },
                    _ if tombstone_target.is_some() => return Err(CoveError::MapInvalid),
                    _ => {}
                }
                rules.push(MapRowSemanticRule {
                    rule_id: required_non_empty_str(entry, "rule_id")?,
                    source_id: required_non_empty_str(entry, "source_id")?,
                    identity_rule_id: required_non_empty_str(entry, "identity_rule_id")?,
                    row_semantics_kind,
                    source_operation_kind,
                    assertion_kinds,
                    tombstone_target,
                    record_kind: optional_non_empty_str(entry, "record_kind")?
                        .unwrap_or_else(|| "Baseline".to_string()),
                    temporal_policy: optional_non_empty_str(entry, "temporal_policy")?
                        .unwrap_or_else(|| "latest_committed".to_string()),
                    conflict_policy: optional_non_empty_str(entry, "conflict_policy")?
                        .unwrap_or_else(|| "reject_conflict".to_string()),
                    function_ids: optional_string_list(entry, "function_ids")?,
                    output_assertion_ids: optional_string_list(entry, "output_assertion_ids")?,
                    association_endpoints: optional_string_list(entry, "association_endpoints")?,
                    property_bindings,
                    association_bindings,
                });
            }
        }
        Ok(Self {
            mapping_id,
            mapping_version,
            rules,
        })
    }
}

fn parse_property_bindings(
    values: Option<&Vec<Value>>,
) -> Result<Vec<MapPropertyBinding>, CoveError> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    values
        .iter()
        .map(|value| {
            let entry = as_object(value)?;
            validate_keys(
                entry,
                &[
                    "assertion_id",
                    "property_id",
                    "property_name",
                    "source_column",
                    "logical_type",
                    "physical_kind",
                    "value_expression",
                    "nullable",
                    "missing_policy",
                    "conflict_policy",
                    "source_priority",
                ],
            )?;
            Ok(MapPropertyBinding {
                assertion_id: required_non_empty_str(entry, "assertion_id")?,
                property_id: required_non_empty_str(entry, "property_id")?,
                property_name: required_non_empty_str(entry, "property_name")?,
                source_column: required_non_empty_str(entry, "source_column")?,
                logical_type: required_non_empty_str(entry, "logical_type")?,
                physical_kind: optional_non_empty_str(entry, "physical_kind")?
                    .unwrap_or_else(|| "auto".to_string()),
                value_expression: optional_non_empty_str(entry, "value_expression")?
                    .unwrap_or_else(|| {
                        entry
                            .get("source_column")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string()
                    }),
                nullable: optional_bool(entry, "nullable", true)?,
                missing_policy: optional_non_empty_str(entry, "missing_policy")?
                    .unwrap_or_else(|| "null".to_string()),
                conflict_policy: optional_non_empty_str(entry, "conflict_policy")?
                    .unwrap_or_else(|| "reject_conflict".to_string()),
                source_priority: optional_i64(entry, "source_priority")?,
            })
        })
        .collect()
}

fn parse_source_operation_kind(
    entry: &Map<String, Value>,
    row_semantics_kind: &str,
) -> Result<SourceOperationKind, CoveError> {
    let value = optional_non_empty_str(entry, "source_operation_kind")?.or_else(|| {
        optional_non_empty_str(entry, "operation_kind")
            .ok()
            .flatten()
    });
    let kind = match value {
        Some(value) => SourceOperationKind::parse(&value).ok_or(CoveError::MapInvalid)?,
        None => match row_semantics_kind {
            "EvidenceOnly" => SourceOperationKind::EvidenceOnly,
            "Tombstone" => SourceOperationKind::TombstoneObject,
            _ => SourceOperationKind::Fact,
        },
    };
    Ok(kind)
}

fn parse_association_bindings(
    values: Option<&Vec<Value>>,
) -> Result<Vec<MapAssociationBinding>, CoveError> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    values
        .iter()
        .map(|value| {
            let entry = as_object(value)?;
            validate_keys(
                entry,
                &[
                    "assertion_id",
                    "association_type",
                    "target_identity_rule_id",
                    "source_identity_rule_id",
                    "source_role",
                    "target_role",
                    "source_endpoint_expression",
                    "target_endpoint_expression",
                    "valid_from_expression",
                    "valid_to_expression",
                    "cardinality_policy",
                    "missing_policy",
                    "link_object_materialization",
                ],
            )?;
            Ok(MapAssociationBinding {
                assertion_id: required_non_empty_str(entry, "assertion_id")?,
                association_type: required_non_empty_str(entry, "association_type")?,
                target_identity_rule_id: required_non_empty_str(entry, "target_identity_rule_id")?,
                source_identity_rule_id: optional_non_empty_str(entry, "source_identity_rule_id")?
                    .unwrap_or_default(),
                source_role: optional_non_empty_str(entry, "source_role")?
                    .unwrap_or_else(|| "source".to_string()),
                target_role: optional_non_empty_str(entry, "target_role")?
                    .unwrap_or_else(|| "target".to_string()),
                source_endpoint_expression: optional_non_empty_str(
                    entry,
                    "source_endpoint_expression",
                )?
                .unwrap_or_else(|| "source.goid".to_string()),
                target_endpoint_expression: optional_non_empty_str(
                    entry,
                    "target_endpoint_expression",
                )?
                .unwrap_or_else(|| "target.goid".to_string()),
                valid_from_expression: optional_non_empty_str(entry, "valid_from_expression")?,
                valid_to_expression: optional_non_empty_str(entry, "valid_to_expression")?,
                cardinality_policy: optional_non_empty_str(entry, "cardinality_policy")?
                    .unwrap_or_else(|| "one".to_string()),
                missing_policy: optional_non_empty_str(entry, "missing_policy")?
                    .unwrap_or_else(|| "reject".to_string()),
                link_object_materialization: optional_non_empty_str(
                    entry,
                    "link_object_materialization",
                )?
                .unwrap_or_else(|| "required".to_string()),
            })
        })
        .collect()
}

impl MapAssertionLog {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapAssertionLog, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let mut assertions = Vec::new();
        if let Some(values) = optional_array(object, "assertions")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &[
                        "assertion_id",
                        "output_object_id",
                        "source_operation_kind",
                        "operation_effect",
                        "operation_target",
                        "correction_of",
                        "replacement_of",
                        "redaction_scope",
                    ],
                )?;
                assertions.push(MapAssertionEntry {
                    assertion_id: required_non_empty_str(entry, "assertion_id")?,
                    output_object_id: required_non_empty_str(entry, "output_object_id")?,
                });
            }
        }
        Ok(Self {
            mapping_id,
            mapping_version,
            assertions,
        })
    }
}

impl MapIdentityEquivalenceIndex {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapIdentityEquivalenceIndex, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let mut equivalences = Vec::new();
        if let Some(values) = optional_array(object, "equivalences")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(entry, &["left_identity", "right_identity"])?;
                equivalences.push(MapEquivalencePair {
                    left_identity: required_non_empty_str(entry, "left_identity")?,
                    right_identity: required_non_empty_str(entry, "right_identity")?,
                });
            }
        }
        validate_identity_components(object)?;
        Ok(Self {
            mapping_id,
            mapping_version,
            equivalences,
        })
    }
}

impl MapEvidenceIndex {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        Self::parse_with_requested_operation_metadata_keys(bytes, &[])
    }

    pub fn parse_with_requested_operation_metadata_keys(
        bytes: &[u8],
        requested_keys: &[String],
    ) -> Result<Self, CoveError> {
        if is_compact_evidence_index_bytes(bytes) {
            return parse_compact_evidence_index(bytes, requested_keys);
        }
        let root = parse_root_for_section(SectionKind::MapEvidenceIndex, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let requested_keys = requested_keys
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut entries = Vec::new();
        if let Some(values) = optional_array(object, "entries")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &[
                        "source_id",
                        "source_row_identity",
                        "rule_id",
                        "assertion_id",
                        "output_object_id",
                        "observed_schema_fingerprint",
                        "observed_snapshot_digest",
                        "source_operation_kind",
                        "operation_effect",
                        "operation_target",
                        "property_id",
                        "property_name",
                        "suppressed",
                        "suppressed_reason",
                        "suppressed_value",
                        "redacted",
                        "redaction_scope",
                        "correction_of",
                        "closes_association",
                        "expires_previous",
                        "replacement_of",
                        "candidate",
                        "identity_rule_id",
                        "object_type",
                        "association_type",
                        "join_key_sha256",
                        "resolution_metadata",
                        "resolution_role_id",
                        "resolution_kind",
                        "resolver_id",
                        "resolver_digest",
                        "catalog_digest",
                        "pipeline_digest",
                        "normalization_pipeline_id",
                        "evidence_policy",
                        "redacted_resolution_evidence",
                        "raw_observed_value",
                        "normalized_value",
                        "resolved_identity_value",
                        "canonical_key",
                        "canonical_label",
                        "alias_catalog_id",
                        "alias_entry_id",
                        "alias_hit",
                        "alias_miss",
                        "alias_ambiguous",
                        "miss_policy",
                        "candidate_match_id",
                        "candidate_score",
                        "left_source_id",
                        "left_source_row_identity",
                        "left_raw_observed_value",
                        "left_normalized_value",
                        "left_row_digest",
                        "right_source_id",
                        "right_source_row_identity",
                        "right_raw_observed_value",
                        "right_normalized_value",
                        "right_row_digest",
                        "blocking_key",
                        "match_rule_id",
                        "review_decision_id",
                        "redacted_resolution_evidence",
                        "operation_metadata",
                    ],
                )?;
                let mut operation_metadata = entry
                    .iter()
                    .filter(|(key, _)| {
                        is_evidence_operation_metadata_key(key)
                            && (requested_keys.is_empty() || requested_keys.contains(key.as_str()))
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<BTreeMap<_, _>>();
                if let Some(metadata) = entry.get("operation_metadata") {
                    let metadata = as_object(metadata)?;
                    for (key, value) in metadata {
                        if !is_evidence_operation_metadata_key(key) {
                            return Err(CoveError::MapEvidenceInvalid);
                        }
                        if (requested_keys.is_empty() || requested_keys.contains(key.as_str()))
                            && operation_metadata
                                .insert(key.clone(), value.clone())
                                .is_some()
                        {
                            return Err(CoveError::MapEvidenceInvalid);
                        }
                    }
                }
                entries.push(MapEvidenceEntry {
                    source_id: required_non_empty_str(entry, "source_id")?,
                    source_row_identity: required_non_empty_str(entry, "source_row_identity")?,
                    rule_id: required_non_empty_str(entry, "rule_id")?,
                    assertion_id: required_non_empty_str(entry, "assertion_id")?,
                    output_object_id: required_non_empty_str(entry, "output_object_id")?,
                    observed_schema_fingerprint: optional_non_empty_str(
                        entry,
                        "observed_schema_fingerprint",
                    )?,
                    observed_snapshot_digest: optional_non_empty_str(
                        entry,
                        "observed_snapshot_digest",
                    )?,
                    operation_metadata,
                });
            }
        }
        Ok(Self {
            mapping_id,
            mapping_version,
            entries,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CompactMember {
    source_id: u32,
    source_row_identity: u32,
    rule_id: u32,
    assertion_id: u32,
    observed_schema_fingerprint: u32,
    observed_snapshot_digest: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CompactGroupKey {
    output_object_id: u32,
    operation_metadata: u32,
}

pub(super) fn is_compact_evidence_index_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= COMPACT_EVIDENCE_MAGIC.len()
        && &bytes[..COMPACT_EVIDENCE_MAGIC.len()] == COMPACT_EVIDENCE_MAGIC
}

pub(super) fn compact_evidence_index_bytes(index: &MapEvidenceIndex) -> Result<Vec<u8>, CoveError> {
    let mut strings = Vec::<String>::new();
    let mut string_ids = BTreeMap::<String, u32>::new();
    let mut intern = |value: &str| -> Result<u32, CoveError> {
        if let Some(existing) = string_ids.get(value) {
            return Ok(*existing);
        }
        let id = u32::try_from(strings.len()).map_err(|_| CoveError::MapEvidenceInvalid)?;
        strings.push(value.to_string());
        string_ids.insert(value.to_string(), id);
        Ok(id)
    };
    let mapping_id = intern(&index.mapping_id)?;
    let mapping_version = intern(&index.mapping_version)?;
    let mut groups = Vec::<(CompactGroupKey, Vec<CompactMember>)>::new();
    let mut group_ids = BTreeMap::<CompactGroupKey, u32>::new();
    let mut order = Vec::<(u32, u32)>::with_capacity(index.entries.len());
    for entry in &index.entries {
        let metadata_json = serde_json::to_string(&entry.operation_metadata)
            .map_err(|_| CoveError::MapEvidenceInvalid)?;
        let key = CompactGroupKey {
            output_object_id: intern(&entry.output_object_id)?,
            operation_metadata: intern(&metadata_json)?,
        };
        let group_id = if let Some(existing) = group_ids.get(&key) {
            *existing
        } else {
            let id = u32::try_from(groups.len()).map_err(|_| CoveError::MapEvidenceInvalid)?;
            groups.push((key.clone(), Vec::new()));
            group_ids.insert(key, id);
            id
        };
        let member = CompactMember {
            source_id: intern(&entry.source_id)?,
            source_row_identity: intern(&entry.source_row_identity)?,
            rule_id: intern(&entry.rule_id)?,
            assertion_id: intern(&entry.assertion_id)?,
            observed_schema_fingerprint: optional_string_index(
                &mut intern,
                entry.observed_schema_fingerprint.as_deref(),
            )?,
            observed_snapshot_digest: optional_string_index(
                &mut intern,
                entry.observed_snapshot_digest.as_deref(),
            )?,
        };
        let members = &mut groups
            .get_mut(usize::try_from(group_id).map_err(|_| CoveError::MapEvidenceInvalid)?)
            .ok_or(CoveError::MapEvidenceInvalid)?
            .1;
        let member_id = u32::try_from(members.len()).map_err(|_| CoveError::MapEvidenceInvalid)?;
        members.push(member);
        order.push((group_id, member_id));
    }

    let mut body = Vec::new();
    push_u32(&mut body, strings.len())?;
    for value in &strings {
        push_bytes(&mut body, value.as_bytes())?;
    }
    push_u32(&mut body, mapping_id)?;
    push_u32(&mut body, mapping_version)?;
    push_u32(&mut body, groups.len())?;
    for (key, members) in &groups {
        push_u32(&mut body, key.output_object_id)?;
        push_u32(&mut body, key.operation_metadata)?;
        push_u32(&mut body, members.len())?;
        for member in members {
            push_u32(&mut body, member.source_id)?;
            push_u32(&mut body, member.source_row_identity)?;
            push_u32(&mut body, member.rule_id)?;
            push_u32(&mut body, member.assertion_id)?;
            push_u32(&mut body, member.observed_schema_fingerprint)?;
            push_u32(&mut body, member.observed_snapshot_digest)?;
        }
    }
    push_u32(&mut body, order.len())?;
    for (group_id, member_id) in &order {
        push_u32(&mut body, *group_id)?;
        push_u32(&mut body, *member_id)?;
    }

    let mut bytes = Vec::with_capacity(COMPACT_EVIDENCE_HEADER_LEN + body.len());
    bytes.extend_from_slice(COMPACT_EVIDENCE_MAGIC);
    bytes.extend_from_slice(&COMPACT_EVIDENCE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&checksum::crc32c(&body).to_le_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

fn optional_string_index(
    intern: &mut impl FnMut(&str) -> Result<u32, CoveError>,
    value: Option<&str>,
) -> Result<u32, CoveError> {
    value.map(intern).unwrap_or(Ok(NONE_INDEX))
}

fn parse_compact_evidence_index(
    bytes: &[u8],
    requested_keys: &[String],
) -> Result<MapEvidenceIndex, CoveError> {
    if bytes.len() < COMPACT_EVIDENCE_HEADER_LEN {
        return Err(CoveError::MapEvidenceInvalid);
    }
    if &bytes[..COMPACT_EVIDENCE_MAGIC.len()] != COMPACT_EVIDENCE_MAGIC {
        return Err(CoveError::MapEvidenceInvalid);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != COMPACT_EVIDENCE_VERSION {
        return Err(CoveError::MapEvidenceInvalid);
    }
    let expected_crc = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let body = &bytes[COMPACT_EVIDENCE_HEADER_LEN..];
    if checksum::crc32c(body) != expected_crc {
        return Err(CoveError::MapEvidenceInvalid);
    }
    let mut cursor = CompactCursor {
        bytes: body,
        pos: 0,
    };
    let string_count = cursor.read_u32_as_usize()?;
    let mut strings = Vec::<String>::with_capacity(string_count);
    for _ in 0..string_count {
        let raw = cursor.read_len_bytes()?;
        strings.push(
            std::str::from_utf8(raw)
                .map_err(|_| CoveError::MapEvidenceInvalid)?
                .to_string(),
        );
    }
    let mapping_id = string_at(&strings, cursor.read_u32()?)?.to_string();
    let mapping_version = string_at(&strings, cursor.read_u32()?)?.to_string();
    let group_count = cursor.read_u32_as_usize()?;
    let mut groups = Vec::<(CompactGroupKey, Vec<CompactMember>)>::with_capacity(group_count);
    for _ in 0..group_count {
        let key = CompactGroupKey {
            output_object_id: cursor.read_u32()?,
            operation_metadata: cursor.read_u32()?,
        };
        string_at(&strings, key.output_object_id)?;
        string_at(&strings, key.operation_metadata)?;
        let member_count = cursor.read_u32_as_usize()?;
        let mut members = Vec::with_capacity(member_count);
        for _ in 0..member_count {
            let member = CompactMember {
                source_id: cursor.read_u32()?,
                source_row_identity: cursor.read_u32()?,
                rule_id: cursor.read_u32()?,
                assertion_id: cursor.read_u32()?,
                observed_schema_fingerprint: cursor.read_u32()?,
                observed_snapshot_digest: cursor.read_u32()?,
            };
            string_at(&strings, member.source_id)?;
            string_at(&strings, member.source_row_identity)?;
            string_at(&strings, member.rule_id)?;
            string_at(&strings, member.assertion_id)?;
            optional_string_at(&strings, member.observed_schema_fingerprint)?;
            optional_string_at(&strings, member.observed_snapshot_digest)?;
            members.push(member);
        }
        groups.push((key, members));
    }
    let requested_keys = requested_keys
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let order_count = cursor.read_u32_as_usize()?;
    let mut entries = Vec::with_capacity(order_count);
    for _ in 0..order_count {
        let group_id = cursor.read_u32_as_usize()?;
        let member_id = cursor.read_u32_as_usize()?;
        let (key, members) = groups.get(group_id).ok_or(CoveError::MapEvidenceInvalid)?;
        let member = members
            .get(member_id)
            .ok_or(CoveError::MapEvidenceInvalid)?;
        let metadata_json = string_at(&strings, key.operation_metadata)?;
        let metadata: BTreeMap<String, Value> =
            serde_json::from_str(metadata_json).map_err(|_| CoveError::MapEvidenceInvalid)?;
        if metadata
            .keys()
            .any(|key| !is_evidence_operation_metadata_key(key))
        {
            return Err(CoveError::MapEvidenceInvalid);
        }
        let operation_metadata = metadata
            .into_iter()
            .filter(|(key, _)| requested_keys.is_empty() || requested_keys.contains(key.as_str()))
            .collect();
        entries.push(MapEvidenceEntry {
            source_id: string_at(&strings, member.source_id)?.to_string(),
            source_row_identity: string_at(&strings, member.source_row_identity)?.to_string(),
            rule_id: string_at(&strings, member.rule_id)?.to_string(),
            assertion_id: string_at(&strings, member.assertion_id)?.to_string(),
            output_object_id: string_at(&strings, key.output_object_id)?.to_string(),
            observed_schema_fingerprint: optional_string_at(
                &strings,
                member.observed_schema_fingerprint,
            )?
            .map(str::to_string),
            observed_snapshot_digest: optional_string_at(
                &strings,
                member.observed_snapshot_digest,
            )?
            .map(str::to_string),
            operation_metadata,
        });
    }
    if cursor.pos != cursor.bytes.len() {
        return Err(CoveError::MapEvidenceInvalid);
    }
    Ok(MapEvidenceIndex {
        mapping_id,
        mapping_version,
        entries,
    })
}

struct CompactCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> CompactCursor<'a> {
    fn read_u32(&mut self) -> Result<u32, CoveError> {
        let end = self.pos.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or(CoveError::MapEvidenceInvalid)?;
        self.pos = end;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u32_as_usize(&mut self) -> Result<usize, CoveError> {
        usize::try_from(self.read_u32()?).map_err(|_| CoveError::MapEvidenceInvalid)
    }

    fn read_len_bytes(&mut self) -> Result<&'a [u8], CoveError> {
        let len = self.read_u32_as_usize()?;
        let end = self.pos.checked_add(len).ok_or(CoveError::ArithOverflow)?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or(CoveError::MapEvidenceInvalid)?;
        self.pos = end;
        Ok(bytes)
    }
}

fn push_u32(out: &mut Vec<u8>, value: impl TryInto<u32>) -> Result<(), CoveError> {
    let value = value
        .try_into()
        .map_err(|_| CoveError::MapEvidenceInvalid)?;
    out.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CoveError> {
    push_u32(out, bytes.len())?;
    out.extend_from_slice(bytes);
    Ok(())
}

fn string_at(strings: &[String], index: u32) -> Result<&str, CoveError> {
    strings
        .get(usize::try_from(index).map_err(|_| CoveError::MapEvidenceInvalid)?)
        .map(String::as_str)
        .ok_or(CoveError::MapEvidenceInvalid)
}

fn optional_string_at(strings: &[String], index: u32) -> Result<Option<&str>, CoveError> {
    if index == NONE_INDEX {
        Ok(None)
    } else {
        string_at(strings, index).map(Some)
    }
}

fn is_evidence_operation_metadata_key(key: &str) -> bool {
    matches!(
        key,
        "source_operation_kind"
            | "operation_effect"
            | "operation_target"
            | "property_id"
            | "property_name"
            | "suppressed"
            | "suppressed_reason"
            | "suppressed_value"
            | "redacted"
            | "redaction_scope"
            | "correction_of"
            | "closes_association"
            | "expires_previous"
            | "replacement_of"
            | "candidate"
            | "identity_rule_id"
            | "object_type"
            | "association_type"
            | "join_key_sha256"
            | "resolution_metadata"
            | "resolution_role_id"
            | "resolution_kind"
            | "resolver_id"
            | "resolver_digest"
            | "catalog_digest"
            | "pipeline_digest"
            | "normalization_pipeline_id"
            | "evidence_policy"
            | "redacted_resolution_evidence"
            | "raw_observed_value"
            | "normalized_value"
            | "resolved_identity_value"
            | "canonical_key"
            | "canonical_label"
            | "alias_catalog_id"
            | "alias_entry_id"
            | "alias_hit"
            | "alias_miss"
            | "alias_ambiguous"
            | "miss_policy"
            | "candidate_match_id"
            | "candidate_score"
            | "left_source_id"
            | "left_source_row_identity"
            | "left_raw_observed_value"
            | "left_normalized_value"
            | "left_row_digest"
            | "right_source_id"
            | "right_source_row_identity"
            | "right_raw_observed_value"
            | "right_normalized_value"
            | "right_row_digest"
            | "blocking_key"
            | "match_rule_id"
            | "review_decision_id"
    )
}

impl MapConversionReport {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapConversionReport, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let mut sources = Vec::new();
        if let Some(values) = optional_array(object, "sources")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &[
                        "source_id",
                        "source_kind",
                        "schema_fingerprint",
                        "snapshot_digest",
                    ],
                )?;
                sources.push(MapObservedSourceState {
                    source_id: required_non_empty_str(entry, "source_id")?,
                    schema_fingerprint: optional_non_empty_str(entry, "schema_fingerprint")?,
                    snapshot_digest: optional_non_empty_str(entry, "snapshot_digest")?,
                });
            }
        }
        validate_conversion_report_details(object)?;
        Ok(Self {
            mapping_id,
            mapping_version,
            sources,
        })
    }
}

impl MapProjectionCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let root = parse_root_for_section(SectionKind::MapProjectionCatalog, bytes)?;
        let object = as_object(&root)?;
        let (mapping_id, mapping_version) = parse_mapping_identity(object)?;
        let mut projections = Vec::new();
        if let Some(values) = optional_array(object, "projections")? {
            for value in values {
                let entry = as_object(value)?;
                validate_keys(
                    entry,
                    &[
                        "projection_id",
                        "assertion_ids",
                        "output_table",
                        "row_grain",
                        "anchor",
                        "temporal_mode",
                        "columns",
                        "multi_value_policy",
                        "missing_policy",
                        "ordering",
                        "evidence_policy",
                        "output_modes",
                    ],
                )?;
                projections.push(MapProjectionEntry {
                    projection_id: required_non_empty_str(entry, "projection_id")?,
                    assertion_ids: optional_string_list(entry, "assertion_ids")?,
                    output_table: optional_non_empty_str(entry, "output_table")?,
                    row_grain: {
                        let row_grain = optional_non_empty_str(entry, "row_grain")?;
                        if row_grain
                            .as_deref()
                            .is_some_and(|row_grain| !is_valid_projection_row_grain(row_grain))
                        {
                            return Err(CoveError::MapInvalid);
                        }
                        row_grain
                    },
                    anchor: parse_projection_anchor(entry)?,
                    temporal_mode: {
                        let mode = parse_temporal_mode(entry)?;
                        if mode
                            .as_deref()
                            .is_some_and(|mode| !is_valid_temporal_mode(mode))
                        {
                            return Err(CoveError::MapInvalid);
                        }
                        mode
                    },
                    columns: parse_projection_columns(optional_array(entry, "columns")?)?,
                    multi_value_policy: {
                        let policy = optional_non_empty_str(entry, "multi_value_policy")?;
                        if policy
                            .as_deref()
                            .is_some_and(|policy| !is_valid_multi_value_policy(policy))
                        {
                            return Err(CoveError::MapInvalid);
                        }
                        policy
                    },
                    missing_policy: optional_non_empty_str(entry, "missing_policy")?
                        .unwrap_or_else(|| "null".to_string()),
                    ordering: optional_string_list(entry, "ordering")?,
                    evidence_policy: optional_non_empty_str(entry, "evidence_policy")?
                        .unwrap_or_else(|| "omit".to_string()),
                    output_modes: {
                        let modes = optional_string_list(entry, "output_modes")?;
                        if modes
                            .iter()
                            .any(|mode| !is_valid_projection_output_mode(mode))
                        {
                            return Err(CoveError::MapInvalid);
                        }
                        modes
                    },
                });
            }
        }
        Ok(Self {
            mapping_id,
            mapping_version,
            projections,
        })
    }
}

fn parse_projection_anchor(
    entry: &Map<String, Value>,
) -> Result<Option<MapProjectionAnchor>, CoveError> {
    let Some(anchor) = entry.get("anchor") else {
        return Ok(None);
    };
    let anchor = as_object(anchor)?;
    validate_keys(anchor, &["object_type", "association_type"])?;
    Ok(Some(MapProjectionAnchor {
        object_type: optional_non_empty_str(anchor, "object_type")?,
        association_type: optional_non_empty_str(anchor, "association_type")?,
    }))
}

fn parse_temporal_mode(entry: &Map<String, Value>) -> Result<Option<String>, CoveError> {
    match entry.get("temporal_mode") {
        None => Ok(None),
        Some(value) if value.is_string() => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Some(value.to_string()))
            .ok_or(CoveError::MapInvalid),
        Some(value) => {
            let mode = as_object(value)?;
            validate_keys(mode, &["as_of"])?;
            optional_non_empty_str(mode, "as_of")
        }
    }
}

fn parse_projection_columns(
    values: Option<&Vec<Value>>,
) -> Result<Vec<MapProjectionColumn>, CoveError> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    values
        .iter()
        .map(|value| {
            let entry = as_object(value)?;
            validate_keys(
                entry,
                &[
                    "name",
                    "value",
                    "logical_type",
                    "nested_shape",
                    "conflict_policy",
                    "missing_policy",
                    "lineage",
                ],
            )?;
            Ok(MapProjectionColumn {
                name: required_non_empty_str(entry, "name")?,
                value: required_non_empty_str(entry, "value")?,
                logical_type: optional_non_empty_str(entry, "logical_type")?,
                nested_shape: optional_nested_shape(entry, "nested_shape")?,
                conflict_policy: optional_non_empty_str(entry, "conflict_policy")?
                    .unwrap_or_else(|| "canonical_value".to_string()),
                missing_policy: optional_non_empty_str(entry, "missing_policy")?
                    .unwrap_or_else(|| "null".to_string()),
                lineage: parse_projection_column_lineage(entry)?,
            })
        })
        .collect()
}

fn parse_projection_column_lineage(
    entry: &Map<String, Value>,
) -> Result<Option<MapProjectionColumnLineage>, CoveError> {
    let Some(lineage) = entry.get("lineage") else {
        return Ok(None);
    };
    let lineage = as_object(lineage)?;
    validate_keys(
        lineage,
        &[
            "source",
            "object_type_id",
            "object_type_name",
            "property_id",
            "property_name",
            "projection_table_id",
            "projection_column_id",
            "expression",
            "transform",
            "filter_pushdown",
        ],
    )?;
    let source = required_non_empty_str(lineage, "source")?;
    let transform = required_non_empty_str(lineage, "transform")?;
    let filter_pushdown = required_non_empty_str(lineage, "filter_pushdown")?;
    if source != "object_property"
        || transform != "identity"
        || filter_pushdown != "projection_covi_prefilter"
    {
        return Err(CoveError::MapInvalid);
    }
    Ok(Some(MapProjectionColumnLineage {
        source,
        object_type_id: required_u32(lineage, "object_type_id")?,
        object_type_name: required_non_empty_str(lineage, "object_type_name")?,
        property_id: required_u32(lineage, "property_id")?,
        property_name: required_non_empty_str(lineage, "property_name")?,
        projection_table_id: required_u32(lineage, "projection_table_id")?,
        projection_column_id: required_u32(lineage, "projection_column_id")?,
        expression: required_non_empty_str(lineage, "expression")?,
        transform,
        filter_pushdown,
    }))
}

fn validate_row_semantics_kind(kind: &str) -> Result<(), CoveError> {
    match kind {
        "Object" | "EventObject" | "LinkObject" | "AssociationOnly" | "Composite"
        | "Dispatched" | "KeyValueFragment" | "ProjectionOnly" | "EvidenceOnly" | "Tombstone" => {
            Ok(())
        }
        _ => Err(CoveError::MapInvalid),
    }
}

fn validate_assertion_kind(kind: &str) -> Result<(), CoveError> {
    match kind {
        "object"
        | "property"
        | "association"
        | "temporal"
        | "identity_key"
        | "identity_equivalence"
        | "candidate_match"
        | "tombstone"
        | "evidence"
        | "conflict"
        | "projection" => Ok(()),
        _ => Err(CoveError::MapInvalid),
    }
}

fn is_valid_tombstone_target(target: &str) -> bool {
    matches!(
        target,
        "object" | "property" | "association" | "source_record" | "evidence"
    )
}

fn is_valid_temporal_mode(mode: &str) -> bool {
    matches!(
        mode,
        "latest_committed" | "full_history" | "valid_time" | "observed_time" | "commit_order"
    ) || mode
        .strip_prefix("as_of_timestamp_us:")
        .or_else(|| mode.strip_prefix("as_of_timestamp_us="))
        .or_else(|| mode.strip_prefix("timestamp_us:"))
        .or_else(|| mode.strip_prefix("timestamp_us="))
        .or_else(|| mode.strip_prefix("as_of_time:"))
        .or_else(|| mode.strip_prefix("as_of_time="))
        .is_some_and(|value| value.parse::<i64>().is_ok())
        || mode
            .strip_prefix("as_of_csn:")
            .or_else(|| mode.strip_prefix("as_of_csn="))
            .or_else(|| mode.strip_prefix("csn:"))
            .or_else(|| mode.strip_prefix("csn="))
            .is_some_and(|value| value.parse::<u64>().is_ok())
}

fn is_valid_multi_value_policy(policy: &str) -> bool {
    matches!(
        policy,
        "reject" | "explode" | "aggregate" | "first" | "last" | "list"
    )
}

fn is_valid_projection_row_grain(row_grain: &str) -> bool {
    matches!(
        row_grain,
        "one_row_per_object"
            | "one_row_per_association"
            | "one_row_per_link_object"
            | "one_row_per_property_version"
            | "one_row_per_event_object"
            | "one_row_per_object_as_of_time"
            | "one_row_per_evidence_assertion"
    )
}

fn is_valid_projection_output_mode(mode: &str) -> bool {
    matches!(mode, "json" | "cove-o" | "cove-t" | "arrow" | "sql")
}

const COVE_MAP_JSON_SCHEMA_ID: &str = "org.coveformat.covemap.v2";

#[derive(Debug)]
struct NoDuplicateValue(Value);

impl<'de> Deserialize<'de> for NoDuplicateValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateValueVisitor)
    }
}

struct NoDuplicateValueVisitor;

impl<'de> Visitor<'de> for NoDuplicateValueVisitor {
    type Value = NoDuplicateValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(NoDuplicateValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        NoDuplicateValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<NoDuplicateValue>()? {
            values.push(value.0);
        }
        Ok(NoDuplicateValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        let mut object = Map::new();
        while let Some((key, value)) = access.next_entry::<String, NoDuplicateValue>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate JSON key `{key}`")));
            }
            object.insert(key, value.0);
        }
        Ok(NoDuplicateValue(Value::Object(object)))
    }
}

fn parse_root_for_section(kind: SectionKind, bytes: &[u8]) -> Result<Value, CoveError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let root = NoDuplicateValue::deserialize(&mut deserializer)
        .map_err(|_| CoveError::MapInvalid)?
        .0;
    deserializer.end().map_err(|_| CoveError::MapInvalid)?;
    let object = as_object(&root)?;
    validate_payload_envelope(kind, object)?;
    Ok(root)
}

fn validate_payload_envelope(
    kind: SectionKind,
    object: &Map<String, Value>,
) -> Result<(), CoveError> {
    if object.get("schema_id").and_then(Value::as_str) != Some(COVE_MAP_JSON_SCHEMA_ID) {
        return Err(CoveError::MapInvalid);
    }
    if !section_id_matches(kind, object.get("section_id").ok_or(CoveError::MapInvalid)?) {
        return Err(CoveError::MapInvalid);
    }
    if !object.keys().all(|key| is_allowed_root_key(kind, key)) {
        return Err(CoveError::MapInvalid);
    }
    validate_extension_containers(object)?;
    Ok(())
}

fn section_id_matches(kind: SectionKind, value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_u64() == Some(kind as u16 as u64),
        Value::String(name) => {
            name == section_kind_schema_name(kind) || name == &format!("{kind:?}")
        }
        _ => false,
    }
}

fn section_kind_schema_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::MapSourceCatalog => "MAP_SOURCE_CATALOG",
        SectionKind::MapFunctionRegistry => "MAP_FUNCTION_REGISTRY",
        SectionKind::MapResolutionCatalog => "MAP_RESOLUTION_CATALOG",
        SectionKind::MapIdentityRuleCatalog => "MAP_IDENTITY_RULE_CATALOG",
        SectionKind::MapRowSemanticsCatalog => "MAP_ROW_SEMANTICS_CATALOG",
        SectionKind::MapAssertionLog => "MAP_ASSERTION_LOG",
        SectionKind::MapIdentityEquivalenceIndex => "MAP_IDENTITY_EQUIVALENCE_INDEX",
        SectionKind::MapEvidenceIndex => "MAP_EVIDENCE_INDEX",
        SectionKind::MapConversionReport => "MAP_CONVERSION_REPORT",
        SectionKind::MapProjectionCatalog => "MAP_PROJECTION_CATALOG",
        _ => "UNKNOWN",
    }
}

fn is_allowed_root_key(kind: SectionKind, key: &str) -> bool {
    matches!(
        key,
        "schema_id" | "section_id" | "mapping_id" | "mapping_version" | "extension" | "extensions"
    ) || match kind {
        SectionKind::MapSourceCatalog => {
            matches!(key, "governance_reconciliation_policy" | "sources")
        }
        SectionKind::MapFunctionRegistry => key == "functions",
        SectionKind::MapResolutionCatalog => matches!(
            key,
            "normalization_pipelines" | "resolvers" | "match_rules" | "reviewed_decisions"
        ),
        SectionKind::MapIdentityRuleCatalog => matches!(key, "identity_rules" | "do_not_merge"),
        SectionKind::MapRowSemanticsCatalog => key == "rules",
        SectionKind::MapAssertionLog => key == "assertions",
        SectionKind::MapIdentityEquivalenceIndex => matches!(key, "equivalences" | "components"),
        SectionKind::MapEvidenceIndex => key == "entries",
        SectionKind::MapConversionReport => matches!(
            key,
            "sources"
                | "source_count"
                | "row_count"
                | "object_count"
                | "association_count"
                | "property_value_count"
                | "candidate_match_count"
                | "resolver_hit_count"
                | "resolver_miss_count"
                | "ambiguous_alias_count"
                | "resolver_catalog_digests"
                | "reviewed_decision_count"
                | "reviewed_decision_catalog_digest"
                | "resolver_goid_impact"
                | "candidate_matches"
                | "generated_artifacts"
                | "unsupported"
                | "operation_counts"
                | "governance"
        ),
        SectionKind::MapProjectionCatalog => key == "projections",
        _ => false,
    }
}

fn validate_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), CoveError> {
    for key in object.keys() {
        if key == "extension" || key == "extensions" || allowed.iter().any(|allowed| allowed == key)
        {
            continue;
        }
        return Err(CoveError::MapInvalid);
    }
    validate_extension_containers(object)?;
    Ok(())
}

fn validate_extension_containers(object: &Map<String, Value>) -> Result<(), CoveError> {
    if let Some(extension) = object.get("extension") {
        if !extension.is_object() {
            return Err(CoveError::MapInvalid);
        }
    }
    if let Some(extensions) = object.get("extensions") {
        let extensions = extensions.as_object().ok_or(CoveError::MapInvalid)?;
        for (extension_id, payload) in extensions {
            if extension_id.trim().is_empty() || !payload.is_object() {
                return Err(CoveError::MapInvalid);
            }
        }
    }
    Ok(())
}

fn validate_identity_components(object: &Map<String, Value>) -> Result<(), CoveError> {
    let Some(components) = optional_array(object, "components")? else {
        return Ok(());
    };
    for value in components {
        let component = as_object(value)?;
        validate_keys(
            component,
            &["equivalence_id", "goid", "canonical_anchor", "members"],
        )?;
        if let Some(members) = optional_array(component, "members")? {
            for value in members {
                let member = as_object(value)?;
                validate_keys(
                    member,
                    &[
                        "source_id",
                        "row_index",
                        "source_row_identity",
                        "row_rule_id",
                        "identity_rule_id",
                        "identity_alias",
                        "object_type",
                        "join_key_sha256",
                        "row_digest",
                    ],
                )?;
            }
        }
    }
    Ok(())
}

fn validate_conversion_report_details(object: &Map<String, Value>) -> Result<(), CoveError> {
    if let Some(value) = object.get("reviewed_decision_count") {
        if value.as_u64().is_none() {
            return Err(CoveError::MapInvalid);
        }
    }
    if let Some(digest) = optional_non_empty_str(object, "reviewed_decision_catalog_digest")? {
        validate_sha256_digest_string(&digest)?;
    }
    if let Some(impacts) = optional_array(object, "resolver_goid_impact")? {
        for value in impacts {
            let impact = as_object(value)?;
            validate_keys(
                impact,
                &[
                    "resolver_id",
                    "normalization_pipeline_id",
                    "resolver_digest",
                    "catalog_digest",
                    "pipeline_digest",
                    "affected_goid_count",
                    "affected_goids",
                ],
            )?;
            required_non_empty_str(impact, "resolver_id")?;
            required_non_empty_str(impact, "normalization_pipeline_id")?;
            required_sha256_digest(impact, "resolver_digest")?;
            required_sha256_digest(impact, "catalog_digest")?;
            required_sha256_digest(impact, "pipeline_digest")?;
            let affected_goid_count = required_u64(impact, "affected_goid_count")?;
            let affected_goids = parse_string_values(required_array(impact, "affected_goids")?)?;
            if affected_goid_count != affected_goids.len() as u64 {
                return Err(CoveError::MapInvalid);
            }
        }
    }
    if let Some(matches) = optional_array(object, "candidate_matches")? {
        for value in matches {
            let candidate = as_object(value)?;
            validate_keys(
                candidate,
                &[
                    "candidate_match_id",
                    "source_id",
                    "source_row_identity",
                    "row_rule_id",
                    "identity_rule_id",
                    "object_type",
                    "join_key_sha256",
                    "match_rule_id",
                    "candidate_score",
                    "score_scale",
                    "blocking_key",
                    "left_source_id",
                    "left_source_row_identity",
                    "left_raw_observed_value",
                    "left_normalized_value",
                    "left_row_digest",
                    "right_source_id",
                    "right_source_row_identity",
                    "right_raw_observed_value",
                    "right_normalized_value",
                    "right_row_digest",
                ],
            )?;
        }
    }
    if let Some(artifacts) = optional_array(object, "generated_artifacts")? {
        parse_string_values(artifacts)?;
    }
    if let Some(unsupported) = optional_array(object, "unsupported")? {
        parse_string_values(unsupported)?;
    }
    if let Some(operation_counts) = object.get("operation_counts") {
        let operation_counts = as_object(operation_counts)?;
        for (key, value) in operation_counts {
            if SourceOperationKind::parse(key).is_none() || value.as_u64().is_none() {
                return Err(CoveError::MapInvalid);
            }
        }
    }
    if let Some(governance) = object.get("governance") {
        let governance = as_object(governance)?;
        validate_keys(
            governance,
            &[
                "reconciliation_policy",
                "sources",
                "effective_sensitivity_rank",
                "effective_sensitivity_labels",
                "access_policy_ids",
            ],
        )?;
        if let Some(sources) = optional_array(governance, "sources")? {
            for value in sources {
                let source = as_object(value)?;
                validate_keys(
                    source,
                    &[
                        "source_id",
                        "source_priority",
                        "sensitivity_label",
                        "sensitivity_rank",
                        "access_policy_ids",
                    ],
                )?;
            }
        }
    }
    Ok(())
}

fn parse_mapping_identity(object: &Map<String, Value>) -> Result<(String, String), CoveError> {
    Ok((
        required_non_empty_str(object, "mapping_id")?,
        required_non_empty_str(object, "mapping_version")?,
    ))
}

fn as_object(value: &Value) -> Result<&Map<String, Value>, CoveError> {
    value.as_object().ok_or(CoveError::MapInvalid)
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Vec<Value>, CoveError> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or(CoveError::MapInvalid)
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a Vec<Value>>, CoveError> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value.as_array().map(Some).ok_or(CoveError::MapInvalid),
    }
}

fn required_non_empty_str(object: &Map<String, Value>, key: &str) -> Result<String, CoveError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(CoveError::MapInvalid)
}

fn optional_non_empty_str(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, CoveError> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Some(value.to_string()))
            .ok_or(CoveError::MapInvalid),
    }
}

fn optional_nested_shape(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, CoveError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Err(CoveError::MapInvalid)
            } else {
                Ok(Some(value.to_string()))
            }
        }
        Some(Value::Object(_)) => object
            .get(key)
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| CoveError::MapInvalid),
        Some(_) => Err(CoveError::MapInvalid),
    }
}

fn optional_i64(object: &Map<String, Value>, key: &str) -> Result<Option<i64>, CoveError> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or(CoveError::MapInvalid),
    }
}

fn required_u32(object: &Map<String, Value>, key: &str) -> Result<u32, CoveError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(CoveError::MapInvalid)
}

fn required_u64(object: &Map<String, Value>, key: &str) -> Result<u64, CoveError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(CoveError::MapInvalid)
}

fn required_bool(object: &Map<String, Value>, key: &str) -> Result<bool, CoveError> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or(CoveError::MapInvalid)
}

fn optional_bool(object: &Map<String, Value>, key: &str, default: bool) -> Result<bool, CoveError> {
    match object.get(key) {
        None => Ok(default),
        Some(value) => value.as_bool().ok_or(CoveError::MapInvalid),
    }
}

fn optional_bool_value(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, CoveError> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or(CoveError::MapInvalid),
    }
}

fn string_list(object: &Map<String, Value>, key: &str) -> Result<Vec<String>, CoveError> {
    required_array(object, key).and_then(|values| parse_string_values(values))
}

fn optional_string_list(object: &Map<String, Value>, key: &str) -> Result<Vec<String>, CoveError> {
    match optional_array(object, key)? {
        Some(values) => parse_string_values(values),
        None => Ok(Vec::new()),
    }
}

fn required_value_object(
    object: &Map<String, Value>,
    key: &str,
) -> Result<BTreeMap<String, Value>, CoveError> {
    object
        .get(key)
        .and_then(Value::as_object)
        .map(value_object_to_btree)
        .ok_or(CoveError::MapInvalid)
}

fn optional_value_object(
    object: &Map<String, Value>,
    key: &str,
) -> Result<BTreeMap<String, Value>, CoveError> {
    match object.get(key) {
        None => Ok(BTreeMap::new()),
        Some(value) => value
            .as_object()
            .map(value_object_to_btree)
            .ok_or(CoveError::MapInvalid),
    }
}

fn value_object_to_btree(object: &Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn parse_string_values(values: &[Value]) -> Result<Vec<String>, CoveError> {
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or(CoveError::MapInvalid)
        })
        .collect()
}

fn normalize_pair(left: &str, right: &str) -> Result<(String, String), CoveError> {
    if left.is_empty() || right.is_empty() || left == right {
        return Err(CoveError::MapInvalid);
    }
    if left <= right {
        Ok((left.to_string(), right.to_string()))
    } else {
        Ok((right.to_string(), left.to_string()))
    }
}

fn required_sha256_digest(object: &Map<String, Value>, key: &str) -> Result<String, CoveError> {
    let digest = required_non_empty_str(object, key)?;
    validate_sha256_digest_string(&digest)?;
    Ok(digest)
}

fn validate_sha256_digest_string(value: &str) -> Result<(), CoveError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(CoveError::MapInvalid);
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CoveError::MapInvalid);
    }
    Ok(())
}

fn sha256_digest_string(bytes: &[u8]) -> Result<String, CoveError> {
    let digest = compute_digest(DigestAlgorithm::Sha256, bytes)?;
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    Ok(out)
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + nibble - 10) as char,
        _ => unreachable!(),
    }
}

fn pipeline_digest_input(
    pipeline_id: &str,
    functions: &[MapNormalizationFunction],
    tables: &[MapNormalizationTable],
) -> Result<Value, CoveError> {
    let function_values = functions
        .iter()
        .map(|function| {
            let mut object = Map::new();
            object.insert(
                "function_id".into(),
                Value::String(function.function_id.clone()),
            );
            object.insert("version".into(), Value::String(function.version.clone()));
            if let Some(table_id) = &function.table_id {
                object.insert("table_id".into(), Value::String(table_id.clone()));
            }
            if let Some(digest) = &function.suffix_table_digest {
                object.insert("suffix_table_digest".into(), Value::String(digest.clone()));
            }
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    let mut table_values = tables
        .iter()
        .map(|table| {
            let mut object = Map::new();
            object.insert("table_id".into(), Value::String(table.table_id.clone()));
            object.insert("digest".into(), Value::String(table.digest.clone()));
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    table_values.sort_by(|left, right| {
        left.get("table_id")
            .and_then(Value::as_str)
            .cmp(&right.get("table_id").and_then(Value::as_str))
    });
    Ok(json_object([
        ("pipeline_id", Value::String(pipeline_id.to_string())),
        ("functions", Value::Array(function_values)),
        ("tables", Value::Array(table_values)),
    ]))
}

fn alias_catalog_digest_input(
    catalog: &MapAliasCatalog,
    order_sensitive_catalog: bool,
) -> Result<Value, CoveError> {
    let mut entries = catalog.entries.clone();
    if !order_sensitive_catalog {
        entries.sort_by(|left, right| left.alias_entry_id.cmp(&right.alias_entry_id));
    }
    let entry_values = entries
        .iter()
        .map(|entry| {
            let mut aliases = entry.aliases.clone();
            if !order_sensitive_catalog {
                aliases.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            }
            let mut object = Map::new();
            object.insert(
                "alias_entry_id".into(),
                Value::String(entry.alias_entry_id.clone()),
            );
            object.insert(
                "canonical_key".into(),
                Value::String(entry.canonical_key.clone()),
            );
            object.insert(
                "canonical_label".into(),
                Value::String(entry.canonical_label.clone()),
            );
            object.insert(
                "aliases".into(),
                Value::Array(aliases.into_iter().map(Value::String).collect()),
            );
            if entry.ambiguous {
                object.insert("ambiguous".into(), Value::Bool(true));
            }
            if !entry.metadata.is_empty() {
                object.insert(
                    "metadata".into(),
                    Value::Object(btree_to_json_map(&entry.metadata)),
                );
            }
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    Ok(json_object([
        (
            "alias_catalog_id",
            Value::String(catalog.alias_catalog_id.clone()),
        ),
        ("entries", Value::Array(entry_values)),
    ]))
}

#[allow(clippy::too_many_arguments)]
fn resolver_digest_input(
    resolver_id: &str,
    kind: &str,
    object_type: &str,
    authority: &str,
    confidence_class: &str,
    normalization_pipeline_id: &str,
    pipeline_digest: &str,
    on_hit: &str,
    on_miss: &str,
    miss_confidence_class: Option<&str>,
    ambiguous_policy: &str,
    catalog_digest: &str,
    evidence_policy: &str,
) -> Result<Value, CoveError> {
    Ok(json_object([
        ("resolver_id", Value::String(resolver_id.to_string())),
        ("kind", Value::String(kind.to_string())),
        ("object_type", Value::String(object_type.to_string())),
        ("authority", Value::String(authority.to_string())),
        (
            "confidence_class",
            Value::String(confidence_class.to_string()),
        ),
        (
            "normalization_pipeline_id",
            Value::String(normalization_pipeline_id.to_string()),
        ),
        (
            "pipeline_digest",
            Value::String(pipeline_digest.to_string()),
        ),
        ("on_hit", Value::String(on_hit.to_string())),
        ("on_miss", Value::String(on_miss.to_string())),
        (
            "miss_confidence_class",
            miss_confidence_class
                .map(|value| Value::String(value.to_string()))
                .unwrap_or(Value::Null),
        ),
        (
            "ambiguous_policy",
            Value::String(ambiguous_policy.to_string()),
        ),
        ("catalog_digest", Value::String(catalog_digest.to_string())),
        (
            "evidence_policy",
            Value::String(evidence_policy.to_string()),
        ),
    ]))
}

fn json_object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut object = Map::new();
    for (key, value) in entries {
        object.insert(key.to_string(), value);
    }
    Value::Object(object)
}

fn btree_to_json_map(values: &BTreeMap<String, Value>) -> Map<String, Value> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn canonical_json(value: &Value) -> Result<Vec<u8>, CoveError> {
    let mut out = Vec::new();
    write_canonical_json(value, &mut out)?;
    Ok(out)
}

fn write_canonical_json(value: &Value, out: &mut Vec<u8>) -> Result<(), CoveError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(value) => {
            let encoded = serde_json::to_string(value).map_err(|_| CoveError::MapInvalid)?;
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
                let encoded_key = serde_json::to_string(key).map_err(|_| CoveError::MapInvalid)?;
                out.extend_from_slice(encoded_key.as_bytes());
                out.push(b':');
                let value = object.get(*key).ok_or(CoveError::MapInvalid)?;
                write_canonical_json(value, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(kind: SectionKind, mut value: Value) -> Vec<u8> {
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
        serde_json::to_vec_pretty(&value).unwrap()
    }

    fn resolution_catalog_payload(aliases: Vec<&str>) -> Value {
        let functions = vec![MapNormalizationFunction {
            function_id: "identity".into(),
            version: "1".into(),
            table_id: None,
            suffix_table_digest: None,
        }];
        let tables = Vec::new();
        let pipeline_digest = sha256_digest_string(
            &canonical_json(
                &pipeline_digest_input("company_name.v1", &functions, &tables).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

        let alias_catalog = MapAliasCatalog {
            alias_catalog_id: "company_aliases".into(),
            entries: vec![MapAliasEntry {
                alias_entry_id: "company:tesco".into(),
                canonical_key: "uk-company:tesco".into(),
                canonical_label: "Tesco".into(),
                aliases: aliases.into_iter().map(str::to_string).collect(),
                ambiguous: false,
                metadata: BTreeMap::new(),
                non_semantic_metadata: BTreeMap::new(),
            }],
        };
        let catalog_digest = sha256_digest_string(
            &canonical_json(&alias_catalog_digest_input(&alias_catalog, false).unwrap()).unwrap(),
        )
        .unwrap();
        let resolver_digest = sha256_digest_string(
            &canonical_json(
                &resolver_digest_input(
                    "uk_company_name_resolver",
                    "alias_catalog",
                    "Company",
                    "curated",
                    "authoritative",
                    "company_name.v1",
                    &pipeline_digest,
                    "canonical_key",
                    "candidate_only",
                    None,
                    "reject_auto_merge",
                    &catalog_digest,
                    "retain_raw",
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

        json!({
            "mapping_id": "company-map",
            "mapping_version": "2026.06",
            "normalization_pipelines": [{
                "pipeline_id": "company_name.v1",
                "functions": [{
                    "function_id": "identity",
                    "version": "1"
                }],
                "tables": []
            }],
            "resolvers": [{
                "resolver_id": "uk_company_name_resolver",
                "kind": "alias_catalog",
                "object_type": "Company",
                "authority": "curated",
                "confidence_class": "authoritative",
                "normalization_pipeline_id": "company_name.v1",
                "on_hit": "canonical_key",
                "on_miss": "candidate_only",
                "ambiguous_policy": "reject_auto_merge",
                "catalog_digest": catalog_digest,
                "pipeline_digest": pipeline_digest,
                "resolver_digest": resolver_digest,
                "alias_catalog": {
                    "alias_catalog_id": "company_aliases",
                    "entries": [{
                        "alias_entry_id": "company:tesco",
                        "canonical_key": "uk-company:tesco",
                        "canonical_label": "Tesco",
                        "aliases": alias_catalog.entries[0].aliases
                    }]
                }
            }],
            "match_rules": [],
            "reviewed_decisions": []
        })
    }

    #[test]
    fn resolution_catalog_parse_accepts_alias_catalog_with_verified_digests() {
        let catalog = MapResolutionCatalog::parse(&payload(
            SectionKind::MapResolutionCatalog,
            resolution_catalog_payload(vec!["Tesco", "Tesco PLC", "tesco supermarket"]),
        ))
        .unwrap();

        assert_eq!(
            catalog.normalization_pipelines[0].pipeline_id,
            "company_name.v1"
        );
        assert_eq!(catalog.resolvers[0].resolver_id, "uk_company_name_resolver");
        assert_eq!(
            catalog.resolvers[0]
                .alias_catalog
                .as_ref()
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn resolution_catalog_digest_is_stable_for_alias_order_changes() {
        MapResolutionCatalog::parse(&payload(
            SectionKind::MapResolutionCatalog,
            resolution_catalog_payload(vec!["Tesco", "Tesco PLC", "tesco supermarket"]),
        ))
        .unwrap();
        MapResolutionCatalog::parse(&payload(
            SectionKind::MapResolutionCatalog,
            resolution_catalog_payload(vec!["tesco supermarket", "Tesco PLC", "Tesco"]),
        ))
        .unwrap();
    }

    #[test]
    fn resolution_catalog_rejects_pipeline_digest_mismatch() {
        let mut value = resolution_catalog_payload(vec!["Tesco"]);
        value["normalization_pipelines"][0]["functions"][0]["version"] = json!("2");
        assert_eq!(
            MapResolutionCatalog::parse(&payload(SectionKind::MapResolutionCatalog, value)),
            Err(CoveError::DigestMismatch)
        );
    }

    #[test]
    fn embedded_validation_checks_resolution_pipeline_function_versions() {
        let functions = EmbeddedMapSection::FunctionRegistry(
            MapFunctionRegistry::parse(&payload(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "2026.06",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ))
            .unwrap(),
        );
        let resolution = EmbeddedMapSection::ResolutionCatalog(
            MapResolutionCatalog::parse(&payload(
                SectionKind::MapResolutionCatalog,
                resolution_catalog_payload(vec!["Tesco"]),
            ))
            .unwrap(),
        );
        validate_embedded_sections(&[functions.clone(), resolution.clone()]).unwrap();

        let wrong_functions = EmbeddedMapSection::FunctionRegistry(
            MapFunctionRegistry::parse(&payload(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "2026.06",
                    "functions": [{
                        "function_id": "identity",
                        "version": "2",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ))
            .unwrap(),
        );
        assert_eq!(
            validate_embedded_sections(&[wrong_functions, resolution]),
            Err(CoveError::MapFunctionUndeclared)
        );
    }

    #[test]
    fn embedded_validation_rejects_identity_rule_referencing_missing_resolver() {
        let functions = EmbeddedMapSection::FunctionRegistry(
            MapFunctionRegistry::parse(&payload(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "2026.06",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ))
            .unwrap(),
        );
        let identity = EmbeddedMapSection::IdentityRuleCatalog(
            MapIdentityRuleCatalog::parse(&payload(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "2026.06",
                    "identity_rules": [{
                        "rule_id": "company_by_resolved_name",
                        "object_type": "Company",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "company",
                            "source_column": "company_name",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared",
                            "resolution": {
                                "resolver_id": "uk_company_name_resolver"
                            }
                        }]
                    }],
                    "do_not_merge": []
                }),
            ))
            .unwrap(),
        );

        assert_eq!(
            validate_embedded_sections(&[functions, identity]),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn embedded_validation_rejects_resolver_object_type_mismatch() {
        let functions = EmbeddedMapSection::FunctionRegistry(
            MapFunctionRegistry::parse(&payload(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "2026.06",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ))
            .unwrap(),
        );
        let resolution = EmbeddedMapSection::ResolutionCatalog(
            MapResolutionCatalog::parse(&payload(
                SectionKind::MapResolutionCatalog,
                resolution_catalog_payload(vec!["Tesco"]),
            ))
            .unwrap(),
        );
        let identity = EmbeddedMapSection::IdentityRuleCatalog(
            MapIdentityRuleCatalog::parse(&payload(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "2026.06",
                    "identity_rules": [{
                        "rule_id": "person_by_resolved_company",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "company",
                            "source_column": "company_name",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared",
                            "resolution": {
                                "resolver_id": "uk_company_name_resolver"
                            }
                        }]
                    }],
                    "do_not_merge": []
                }),
            ))
            .unwrap(),
        );

        assert_eq!(
            validate_embedded_sections(&[functions, resolution, identity]),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn identity_rule_rejects_double_normalized_resolver_join_key() {
        let value = json!({
            "mapping_id": "company-map",
            "mapping_version": "2026.06",
            "identity_rules": [{
                "rule_id": "company_by_resolved_name",
                "object_type": "Company",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": ["identity"],
                "join_keys": [{
                    "role_id": "company",
                    "source_column": "company_name",
                    "logical_type": "utf8",
                    "canonicalization": "trim",
                    "null_policy": "reject",
                    "ordering": "declared",
                    "resolution": {
                        "resolver_id": "uk_company_name_resolver"
                    }
                }],
                "allow_reviewed_equivalence": true
            }],
            "do_not_merge": []
        });

        assert_eq!(
            MapIdentityRuleCatalog::parse(&payload(SectionKind::MapIdentityRuleCatalog, value)),
            Err(CoveError::MapInvalid)
        );
    }

    #[test]
    fn evidence_index_accepts_resolution_operation_metadata_keys() {
        let value = json!({
            "mapping_id": "company-map",
            "mapping_version": "2026.06",
            "entries": [{
                "source_id": "supplier_master",
                "source_row_identity": "supplier_master:1",
                "rule_id": "upsert_company",
                "assertion_id": "company_name",
                "output_object_id": "goid:company:1",
                "operation_metadata": {
                    "resolver_id": "uk_company_name_resolver",
                    "resolver_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "raw_observed_value": "Tesco PLC",
                    "normalized_value": "tesco",
                    "canonical_key": "uk-company:tesco",
                    "alias_hit": true,
                    "left_source_id": "supplier_master",
                    "right_source_id": "crm_accounts",
                    "candidate_score": 1000000
                }
            }]
        });

        let index =
            MapEvidenceIndex::parse(&payload(SectionKind::MapEvidenceIndex, value)).unwrap();
        assert_eq!(
            index.entries[0].operation_metadata["canonical_key"],
            json!("uk-company:tesco")
        );
    }

    #[test]
    fn conversion_report_accepts_review_decision_and_resolver_impact_fields() {
        let digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let value = json!({
            "mapping_id": "company-map",
            "mapping_version": "2026.06",
            "sources": [{
                "source_id": "crm_accounts",
                "source_kind": "table",
                "schema_fingerprint": digest,
                "snapshot_digest": digest
            }],
            "source_count": 1,
            "row_count": 1,
            "object_count": 1,
            "association_count": 0,
            "property_value_count": 1,
            "candidate_match_count": 0,
            "resolver_hit_count": 1,
            "resolver_miss_count": 0,
            "ambiguous_alias_count": 0,
            "reviewed_decision_count": 1,
            "reviewed_decision_catalog_digest": digest,
            "resolver_goid_impact": [{
                "resolver_id": "customer_resolver",
                "normalization_pipeline_id": "customer_name.v1",
                "resolver_digest": digest,
                "catalog_digest": digest,
                "pipeline_digest": digest,
                "affected_goid_count": 2,
                "affected_goids": ["goid:customer:1", "goid:customer:2"]
            }]
        });

        let report =
            MapConversionReport::parse(&payload(SectionKind::MapConversionReport, value)).unwrap();
        assert_eq!(report.sources[0].source_id, "crm_accounts");
    }
}
