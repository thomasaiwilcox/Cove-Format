use std::collections::{BTreeMap, BTreeSet};

use cove_core::profile::cove_map::{MapEvidenceEntry, MapEvidenceIndex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    association_opt::{current_row_dataset_file_ordinal, current_row_goid},
    materialized::{ExecutionRow, MaterializedEvidenceRow},
    AggregateDisclosurePolicy, AstEvidenceGrain, PlannedQuery, ResolvedEvidenceRoot,
    ResolvedEvidenceTarget, ResolvedExpr, ResolvedPredicate, ResolvedRoot,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceOptimizationReport {
    pub enabled: bool,
    pub index_reports: Vec<EvidenceGrainIndexReport>,
    pub target_index_kinds: Vec<EvidenceTargetIndexKind>,
    pub evidence_entry_count: usize,
    pub hidden_entry_filtering_applied: bool,
    pub existence_fast_path_candidates: usize,
    pub existence_fast_path_exact: bool,
    pub count_fast_path_candidates: usize,
    pub count_fast_path_exact: bool,
    pub filtered_by_target: bool,
    pub fallback_reasons: Vec<String>,
}

impl EvidenceOptimizationReport {
    pub fn for_plan(planned: &PlannedQuery, index: Option<&MapEvidenceIndex>) -> Self {
        let roots = evidence_roots(planned);
        if roots.is_empty() && planned.dependencies.evidence_fields.is_empty() {
            return Self::default();
        }
        let hidden_entry_filtering_applied = index.is_some_and(|index| {
            index
                .entries
                .iter()
                .any(|entry| evidence_entry_is_hidden(entry))
        });
        let visible_entries = index.map(|index| {
            index
                .entries
                .iter()
                .filter(|entry| !evidence_entry_is_hidden(entry))
                .collect::<Vec<_>>()
        });

        let mut report = Self {
            enabled: true,
            evidence_entry_count: visible_entries.as_ref().map_or(0, Vec::len),
            hidden_entry_filtering_applied,
            ..Self::default()
        };
        if index.is_none() {
            report
                .fallback_reasons
                .push("missing_evidence_index".into());
        }

        let mut grains = BTreeSet::new();
        for root in &roots {
            grains.insert(EvidenceGrainKind::from(root.grain));
            if root.target.is_some() {
                report.filtered_by_target = true;
            }
            if let Some(kind) = root.target.as_ref().map(EvidenceTargetIndexKind::from) {
                report.target_index_kinds.push(kind);
            }
        }
        report.target_index_kinds.sort_unstable();
        report.target_index_kinds.dedup();
        if grains.is_empty() && !planned.dependencies.evidence_fields.is_empty() {
            grains.insert(EvidenceGrainKind::Unknown);
            report
                .fallback_reasons
                .push("evidence_dependency_without_root".into());
        }

        for grain in grains {
            let (indexed, fallback) = visible_entries
                .as_ref()
                .map(|entries| {
                    entries
                        .iter()
                        .filter(|entry| entry_grain(entry) == grain)
                        .fold((0usize, 0usize), |(indexed, fallback), entry| {
                            if entry_grain(entry) == EvidenceGrainKind::Unknown {
                                (indexed, fallback + 1)
                            } else {
                                (indexed + 1, fallback)
                            }
                        })
                })
                .unwrap_or_default();
            report.index_reports.push(EvidenceGrainIndexReport {
                grain,
                candidate_entries: indexed + fallback,
                indexed_entries: indexed,
                fallback_entries: fallback,
                target_filtered: report.filtered_by_target,
                disclosure_limited: true,
            });
        }

        report.existence_fast_path_candidates =
            count_evidence_exists(planned.resolved.method_chain.where_predicate.as_ref());
        report.count_fast_path_candidates = count_evidence_counts(planned);
        let aggregate_policy_allows_exact = planned
            .resolved
            .operation_context
            .security
            .aggregate_disclosure_policy
            == AggregateDisclosurePolicy::AllowExact;
        report.existence_fast_path_exact = report.existence_fast_path_candidates > 0
            && report.fallback_reasons.is_empty()
            && !report
                .index_reports
                .iter()
                .any(|grain| grain.fallback_entries > 0)
            && aggregate_policy_allows_exact;
        report.count_fast_path_exact = report.count_fast_path_candidates > 0
            && report.fallback_reasons.is_empty()
            && !report
                .index_reports
                .iter()
                .any(|grain| grain.fallback_entries > 0)
            && aggregate_policy_allows_exact;
        if (report.existence_fast_path_candidates > 0 || report.count_fast_path_candidates > 0)
            && !aggregate_policy_allows_exact
        {
            report.fallback_reasons.push(
                "aggregate_disclosure_policy_requires_materialized_evidence_authority".into(),
            );
        }
        if report
            .index_reports
            .iter()
            .any(|grain| grain.grain == EvidenceGrainKind::Unknown)
        {
            report
                .fallback_reasons
                .push("unsupported_evidence_grain".into());
        }
        report
    }

    pub fn to_json(&self, allow_protected: bool) -> Value {
        if allow_protected {
            return serde_json::to_value(self).unwrap_or(Value::Null);
        }
        json!({
            "enabled": self.enabled,
            "grain_index_count": self.index_reports.len(),
            "grains": self.index_reports.iter().map(|report| report.redacted_json()).collect::<Vec<_>>(),
            "target_index_kinds": self.target_index_kinds,
            "evidence_entry_count": "<redacted>",
            "hidden_entry_filtering_applied": self.hidden_entry_filtering_applied,
            "existence_fast_path_candidates": self.existence_fast_path_candidates > 0,
            "existence_fast_path_exact": self.existence_fast_path_exact,
            "count_fast_path_candidates": self.count_fast_path_candidates > 0,
            "count_fast_path_exact": self.count_fast_path_exact,
            "filtered_by_target": self.filtered_by_target,
            "fallback_reasons": self.fallback_reasons,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceGrainIndexReport {
    pub grain: EvidenceGrainKind,
    pub candidate_entries: usize,
    pub indexed_entries: usize,
    pub fallback_entries: usize,
    pub target_filtered: bool,
    pub disclosure_limited: bool,
}

impl EvidenceGrainIndexReport {
    fn redacted_json(&self) -> Value {
        json!({
            "grain": self.grain,
            "candidate_entries_present": self.candidate_entries > 0,
            "indexed_entries_present": self.indexed_entries > 0,
            "fallback_entries_present": self.fallback_entries > 0,
            "target_filtered": self.target_filtered,
            "disclosure_limited": self.disclosure_limited,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrainKind {
    Object,
    Property,
    Association,
    Projection,
    Row,
    Source,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTargetIndexKind {
    CurrentObject,
    ObjectType,
    Property,
    AssociationType,
    Projection,
}

impl From<&ResolvedEvidenceTarget> for EvidenceTargetIndexKind {
    fn from(target: &ResolvedEvidenceTarget) -> Self {
        match target {
            ResolvedEvidenceTarget::CurrentRoot => Self::CurrentObject,
            ResolvedEvidenceTarget::ObjectType { .. } => Self::ObjectType,
            ResolvedEvidenceTarget::AssociationType { .. } => Self::AssociationType,
            ResolvedEvidenceTarget::Projection { .. } => Self::Projection,
            ResolvedEvidenceTarget::Property { .. } => Self::Property,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct EvidenceGrainIndex {
    rows: Vec<MaterializedEvidenceRow>,
    by_grain: BTreeMap<EvidenceGrainKind, Vec<usize>>,
    by_object: BTreeMap<(Option<usize>, String), Vec<usize>>,
    by_property: BTreeMap<String, Vec<usize>>,
    by_association: BTreeMap<String, Vec<usize>>,
    by_projection: BTreeMap<String, Vec<usize>>,
    by_source: BTreeMap<String, Vec<usize>>,
    fallback_rows: Vec<usize>,
}

impl EvidenceGrainIndex {
    pub(crate) fn from_rows(rows: &[MaterializedEvidenceRow]) -> Self {
        let rows = rows.to_vec();
        let mut index = Self {
            rows,
            by_grain: BTreeMap::new(),
            by_object: BTreeMap::new(),
            by_property: BTreeMap::new(),
            by_association: BTreeMap::new(),
            by_projection: BTreeMap::new(),
            by_source: BTreeMap::new(),
            fallback_rows: Vec::new(),
        };
        for row_index in 0..index.rows.len() {
            let row = &index.rows[row_index];
            if materialized_evidence_row_is_hidden(row) {
                continue;
            }
            let grain = materialized_row_grain(row);
            if grain == EvidenceGrainKind::Unknown {
                index.fallback_rows.push(row_index);
            } else {
                index.by_grain.entry(grain).or_default().push(row_index);
            }
            if let Some(value) = materialized_str(row, "output_object_id") {
                index
                    .by_object
                    .entry((materialized_file_ordinal(row), value.to_string()))
                    .or_default()
                    .push(row_index);
            }
            if let Some(value) = materialized_str(row, "property_name") {
                index
                    .by_property
                    .entry(value.to_string())
                    .or_default()
                    .push(row_index);
            }
            if let Some(value) = materialized_str(row, "association_type")
                .or_else(|| materialized_str(row, "type_name"))
            {
                index
                    .by_association
                    .entry(value.to_string())
                    .or_default()
                    .push(row_index);
            }
            if let Some(value) = materialized_str(row, "projection_id") {
                index
                    .by_projection
                    .entry(value.to_string())
                    .or_default()
                    .push(row_index);
            }
            if let Some(value) = materialized_str(row, "source_row_identity")
                .or_else(|| materialized_str(row, "source_id"))
            {
                index
                    .by_source
                    .entry(value.to_string())
                    .or_default()
                    .push(row_index);
            }
        }
        index
    }

    pub(crate) fn exists_for(
        &self,
        current_row: &ExecutionRow,
        root: &ResolvedEvidenceRoot,
    ) -> bool {
        self.count_for(current_row, root) > 0
    }

    pub(crate) fn count_for(
        &self,
        current_row: &ExecutionRow,
        root: &ResolvedEvidenceRoot,
    ) -> usize {
        self.candidate_rows(current_row, root)
            .filter(|row| materialized_evidence_row_matches(row, root, Some(current_row)))
            .count()
    }

    pub(crate) fn identities_for(
        &self,
        current_row: &ExecutionRow,
        root: &ResolvedEvidenceRoot,
    ) -> BTreeSet<String> {
        self.candidate_rows(current_row, root)
            .filter(|row| materialized_evidence_row_matches(row, root, Some(current_row)))
            .filter_map(evidence_identity)
            .collect()
    }

    fn candidate_rows<'a>(
        &'a self,
        current_row: &ExecutionRow,
        root: &ResolvedEvidenceRoot,
    ) -> Box<dyn Iterator<Item = &'a MaterializedEvidenceRow> + 'a> {
        let mut indexes = BTreeSet::<usize>::new();
        if let Some(target_indexes) = self.target_indexes(current_row, root) {
            indexes.extend(target_indexes.iter().copied());
        } else if let Some(grain_indexes) = self.by_grain.get(&EvidenceGrainKind::from(root.grain))
        {
            indexes.extend(grain_indexes.iter().copied());
        } else {
            indexes.extend(0..self.rows.len());
        }
        indexes.extend(self.fallback_rows.iter().copied());
        Box::new(indexes.into_iter().filter_map(|index| self.rows.get(index)))
    }

    fn target_indexes(
        &self,
        current_row: &ExecutionRow,
        root: &ResolvedEvidenceRoot,
    ) -> Option<&Vec<usize>> {
        match root.target.as_ref()? {
            ResolvedEvidenceTarget::CurrentRoot => current_row_goid(current_row).and_then(|goid| {
                self.by_object.get(&(
                    current_row_dataset_file_ordinal(current_row),
                    goid.to_string(),
                ))
            }),
            ResolvedEvidenceTarget::Projection { projection_id } => {
                self.by_projection.get(projection_id)
            }
            ResolvedEvidenceTarget::Property { property_name, .. } => {
                self.by_property.get(property_name)
            }
            ResolvedEvidenceTarget::AssociationType { type_name, .. } => {
                self.by_association.get(type_name)
            }
            ResolvedEvidenceTarget::ObjectType { .. } => {
                current_row_goid(current_row).and_then(|goid| {
                    self.by_object.get(&(
                        current_row_dataset_file_ordinal(current_row),
                        goid.to_string(),
                    ))
                })
            }
        }
    }
}

impl Default for EvidenceGrainKind {
    fn default() -> Self {
        Self::Unknown
    }
}

impl From<AstEvidenceGrain> for EvidenceGrainKind {
    fn from(grain: AstEvidenceGrain) -> Self {
        match grain {
            AstEvidenceGrain::Object => Self::Object,
            AstEvidenceGrain::Property => Self::Property,
            AstEvidenceGrain::Association => Self::Association,
            AstEvidenceGrain::Row => Self::Row,
            AstEvidenceGrain::Source => Self::Source,
        }
    }
}

pub(crate) fn materialized_evidence_rows_for_plan(
    planned: &PlannedQuery,
    index: Option<&MapEvidenceIndex>,
) -> (Vec<ExecutionRow>, EvidenceOptimizationReport) {
    let report = EvidenceOptimizationReport::for_plan(planned, index);
    let Some(index) = index else {
        return (Vec::new(), report);
    };
    let root = match &planned.resolved.root {
        ResolvedRoot::Evidence(root) => Some(root),
        _ => None,
    };
    let rows = index
        .entries
        .iter()
        .filter(|entry| root.map_or(true, |root| evidence_entry_matches_root(entry, root)))
        .map(evidence_entry_row)
        .map(ExecutionRow::Evidence)
        .collect();
    (rows, report)
}

pub(crate) fn evidence_entry_row(entry: &MapEvidenceEntry) -> MaterializedEvidenceRow {
    let mut fields = BTreeMap::new();
    fields.insert("source_id".into(), Value::String(entry.source_id.clone()));
    fields.insert(
        "source_row_identity".into(),
        Value::String(entry.source_row_identity.clone()),
    );
    fields.insert("rule_id".into(), Value::String(entry.rule_id.clone()));
    fields.insert(
        "assertion_id".into(),
        Value::String(entry.assertion_id.clone()),
    );
    fields.insert(
        "output_object_id".into(),
        Value::String(entry.output_object_id.clone()),
    );
    if let Some(value) = &entry.observed_schema_fingerprint {
        fields.insert(
            "observed_schema_fingerprint".into(),
            Value::String(value.clone()),
        );
    }
    if let Some(value) = &entry.observed_snapshot_digest {
        fields.insert(
            "observed_snapshot_digest".into(),
            Value::String(value.clone()),
        );
    }
    fields.extend(entry.operation_metadata.clone());
    MaterializedEvidenceRow { fields }
}

fn evidence_entry_matches_root(entry: &MapEvidenceEntry, root: &ResolvedEvidenceRoot) -> bool {
    if evidence_entry_is_hidden(entry) {
        return false;
    }
    let entry_grain = entry_grain(entry);
    let requested = EvidenceGrainKind::from(root.grain);
    if entry_grain != EvidenceGrainKind::Unknown && entry_grain != requested {
        return false;
    }
    target_matches(entry, root.target.as_ref())
}

fn materialized_evidence_row_matches(
    row: &MaterializedEvidenceRow,
    root: &ResolvedEvidenceRoot,
    current_row: Option<&ExecutionRow>,
) -> bool {
    if materialized_evidence_row_is_hidden(row) {
        return false;
    }
    let row_grain = materialized_row_grain(row);
    let requested = EvidenceGrainKind::from(root.grain);
    if row_grain != EvidenceGrainKind::Unknown && row_grain != requested {
        return false;
    }
    if !materialized_scope_matches(row, current_row) {
        return false;
    }
    materialized_target_matches(row, root.target.as_ref(), current_row)
}

fn materialized_target_matches(
    row: &MaterializedEvidenceRow,
    target: Option<&ResolvedEvidenceTarget>,
    current_row: Option<&ExecutionRow>,
) -> bool {
    match target {
        None => true,
        Some(ResolvedEvidenceTarget::CurrentRoot) => current_row
            .and_then(current_row_goid)
            .is_some_and(|goid| materialized_str(row, "output_object_id") == Some(goid)),
        Some(ResolvedEvidenceTarget::Projection { projection_id }) => {
            materialized_str(row, "projection_id").map_or(true, |value| value == projection_id)
        }
        Some(ResolvedEvidenceTarget::Property {
            object_type_id,
            property_id,
            property_name,
        }) => {
            if let Some(expected_object_type_id) = object_type_id {
                if materialized_u64(row, "object_type_id")
                    .is_some_and(|value| value != u64::from(*expected_object_type_id))
                {
                    return false;
                }
            }
            if materialized_u64(row, "property_id")
                .is_some_and(|value| value != u64::from(*property_id))
            {
                return false;
            }
            materialized_str(row, "property_name").map_or(true, |value| value == property_name)
        }
        Some(ResolvedEvidenceTarget::AssociationType {
            object_type_id,
            type_name,
        }) => {
            if materialized_u64(row, "object_type_id")
                .is_some_and(|value| value != u64::from(*object_type_id))
            {
                return false;
            }
            if let Some(value) = materialized_str(row, "association_type")
                .or_else(|| materialized_str(row, "type_name"))
            {
                return value == type_name;
            }
            let target = materialized_str(row, "operation_target")
                .or_else(|| materialized_str(row, "target_kind"));
            target.map_or(true, |value| value == "association" || value == type_name)
        }
        Some(ResolvedEvidenceTarget::ObjectType {
            object_type_id,
            type_name,
        }) => {
            if let Some(current_goid) = current_row.and_then(current_row_goid) {
                if materialized_str(row, "output_object_id")
                    .is_some_and(|value| value != current_goid)
                {
                    return false;
                }
            }
            if materialized_u64(row, "object_type_id")
                .is_some_and(|value| value != u64::from(*object_type_id))
            {
                return false;
            }
            if let Some(value) =
                materialized_str(row, "object_type").or_else(|| materialized_str(row, "type_name"))
            {
                return value == type_name;
            }
            let target = materialized_str(row, "operation_target")
                .or_else(|| materialized_str(row, "target_kind"));
            target.map_or(true, |value| value == "object" || value == type_name)
        }
    }
}

fn materialized_row_grain(row: &MaterializedEvidenceRow) -> EvidenceGrainKind {
    materialized_str(row, "operation_target")
        .or_else(|| materialized_str(row, "target_kind"))
        .or_else(|| materialized_str(row, "grain"))
        .map(grain_from_metadata)
        .unwrap_or(EvidenceGrainKind::Unknown)
}

fn materialized_str<'a>(row: &'a MaterializedEvidenceRow, key: &str) -> Option<&'a str> {
    row.fields.get(key).and_then(Value::as_str)
}

fn materialized_u64(row: &MaterializedEvidenceRow, key: &str) -> Option<u64> {
    row.fields.get(key).and_then(Value::as_u64)
}

fn materialized_file_ordinal(row: &MaterializedEvidenceRow) -> Option<usize> {
    row.fields
        .get("dataset_file_ordinal")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn materialized_scope_matches(
    row: &MaterializedEvidenceRow,
    current_row: Option<&ExecutionRow>,
) -> bool {
    match current_row.and_then(current_row_dataset_file_ordinal) {
        Some(current_ordinal) => materialized_file_ordinal(row) == Some(current_ordinal),
        None => materialized_file_ordinal(row).is_none(),
    }
}

fn evidence_identity(row: &MaterializedEvidenceRow) -> Option<String> {
    materialized_str(row, "assertion_id")
        .or_else(|| materialized_str(row, "source_row_identity"))
        .or_else(|| materialized_str(row, "source_id"))
        .map(|identity| match materialized_file_ordinal(row) {
            Some(ordinal) => format!("{ordinal}:{identity}"),
            None => identity.to_string(),
        })
}

fn materialized_evidence_row_is_hidden(row: &MaterializedEvidenceRow) -> bool {
    materialized_bool(row, "suppressed") || materialized_bool(row, "redacted")
}

fn materialized_bool(row: &MaterializedEvidenceRow, key: &str) -> bool {
    row.fields
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn evidence_entry_is_hidden(entry: &MapEvidenceEntry) -> bool {
    metadata_bool(entry, "suppressed") || metadata_bool(entry, "redacted")
}

fn target_matches(entry: &MapEvidenceEntry, target: Option<&ResolvedEvidenceTarget>) -> bool {
    match target {
        None | Some(ResolvedEvidenceTarget::CurrentRoot) => true,
        Some(ResolvedEvidenceTarget::Projection { projection_id }) => {
            metadata_str(entry, "projection_id")
                .map_or(true, |value| value == projection_id.as_str())
        }
        Some(ResolvedEvidenceTarget::Property {
            object_type_id,
            property_id,
            property_name,
        }) => {
            if let Some(expected_object_type_id) = object_type_id {
                if metadata_u64(entry, "object_type_id")
                    .is_some_and(|value| value != u64::from(*expected_object_type_id))
                {
                    return false;
                }
            }
            if metadata_u64(entry, "property_id")
                .is_some_and(|value| value != u64::from(*property_id))
            {
                return false;
            }
            metadata_str(entry, "property_name")
                .map_or(true, |value| value == property_name.as_str())
        }
        Some(ResolvedEvidenceTarget::AssociationType {
            object_type_id,
            type_name,
        }) => {
            if metadata_u64(entry, "object_type_id")
                .is_some_and(|value| value != u64::from(*object_type_id))
            {
                return false;
            }
            if let Some(value) =
                metadata_str(entry, "association_type").or_else(|| metadata_str(entry, "type_name"))
            {
                return value == type_name.as_str();
            }
            let target = metadata_str(entry, "operation_target")
                .or_else(|| metadata_str(entry, "target_kind"));
            target.map_or(true, |value| {
                value == "association" || value == type_name.as_str()
            })
        }
        Some(ResolvedEvidenceTarget::ObjectType {
            object_type_id,
            type_name,
        }) => {
            if metadata_u64(entry, "object_type_id")
                .is_some_and(|value| value != u64::from(*object_type_id))
            {
                return false;
            }
            if let Some(value) =
                metadata_str(entry, "object_type").or_else(|| metadata_str(entry, "type_name"))
            {
                return value == type_name.as_str();
            }
            let target = metadata_str(entry, "operation_target")
                .or_else(|| metadata_str(entry, "target_kind"));
            target.map_or(true, |value| {
                value == "object" || value == type_name.as_str()
            })
        }
    }
}

fn entry_grain(entry: &MapEvidenceEntry) -> EvidenceGrainKind {
    metadata_str(entry, "operation_target")
        .or_else(|| metadata_str(entry, "target_kind"))
        .or_else(|| metadata_str(entry, "grain"))
        .map(grain_from_metadata)
        .unwrap_or(EvidenceGrainKind::Unknown)
}

fn grain_from_metadata(value: &str) -> EvidenceGrainKind {
    match value {
        "object" | "object_type" => EvidenceGrainKind::Object,
        "property" | "object_property" => EvidenceGrainKind::Property,
        "association" | "link" => EvidenceGrainKind::Association,
        "projection" | "projection_row" => EvidenceGrainKind::Projection,
        "row" | "output_row" => EvidenceGrainKind::Row,
        "source" | "source_row" | "source_record" => EvidenceGrainKind::Source,
        _ => EvidenceGrainKind::Unknown,
    }
}

fn metadata_str<'a>(entry: &'a MapEvidenceEntry, key: &str) -> Option<&'a str> {
    entry.operation_metadata.get(key).and_then(Value::as_str)
}

fn metadata_u64(entry: &MapEvidenceEntry, key: &str) -> Option<u64> {
    entry.operation_metadata.get(key).and_then(Value::as_u64)
}

fn metadata_bool(entry: &MapEvidenceEntry, key: &str) -> bool {
    entry
        .operation_metadata
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn evidence_roots(planned: &PlannedQuery) -> Vec<&ResolvedEvidenceRoot> {
    let mut out = Vec::new();
    if let ResolvedRoot::Evidence(evidence) = &planned.resolved.root {
        out.push(evidence);
    }
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_evidence_predicates(predicate, &mut out);
    }
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            collect_evidence_exprs(&item.expr, &mut out);
        }
    }
    out
}

fn collect_evidence_predicates<'a>(
    predicate: &'a ResolvedPredicate,
    out: &mut Vec<&'a ResolvedEvidenceRoot>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_evidence_exprs(left, out);
            collect_evidence_exprs(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_evidence_exprs(expr, out),
        ResolvedPredicate::Not(inner) => collect_evidence_predicates(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_evidence_predicates(part, out);
            }
        }
    }
}

fn collect_evidence_exprs<'a>(expr: &'a ResolvedExpr, out: &mut Vec<&'a ResolvedEvidenceRoot>) {
    match expr {
        ResolvedExpr::Evidence(evidence) => out.push(evidence),
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_evidence_exprs(arg, out);
            }
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_evidence_exprs(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_evidence_predicates(predicate, out);
            collect_evidence_exprs(then_expr, out);
            collect_evidence_exprs(else_expr, out);
        }
        _ => {}
    }
}

fn count_evidence_exists(predicate: Option<&ResolvedPredicate>) -> usize {
    let Some(predicate) = predicate else {
        return 0;
    };
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::Evidence(_)) => 1,
        ResolvedPredicate::Not(inner) => count_evidence_exists(Some(inner)),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => parts
            .iter()
            .map(|part| count_evidence_exists(Some(part)))
            .sum(),
        _ => 0,
    }
}

fn count_evidence_counts(planned: &PlannedQuery) -> usize {
    planned
        .dependencies
        .aggregate_kinds
        .iter()
        .filter(|kind| kind.as_str() == "count" || kind.as_str() == "exists")
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materialized::MaterializedObjectRow;

    fn entry(target: Option<&str>, property_name: Option<&str>) -> MapEvidenceEntry {
        let mut operation_metadata = BTreeMap::new();
        if let Some(target) = target {
            operation_metadata.insert("operation_target".into(), json!(target));
        }
        if let Some(property_name) = property_name {
            operation_metadata.insert("property_name".into(), json!(property_name));
        }
        MapEvidenceEntry {
            source_id: "source".into(),
            source_row_identity: "row".into(),
            rule_id: "rule".into(),
            assertion_id: "assertion".into(),
            output_object_id: "object".into(),
            observed_schema_fingerprint: None,
            observed_snapshot_digest: None,
            operation_metadata,
        }
    }

    fn evidence_row(object_id: &str, target: &str, assertion: &str) -> MaterializedEvidenceRow {
        let mut fields = BTreeMap::new();
        fields.insert("output_object_id".into(), json!(object_id));
        fields.insert("operation_target".into(), json!(target));
        fields.insert("assertion_id".into(), json!(assertion));
        MaterializedEvidenceRow { fields }
    }

    fn object_row(goid: &str) -> ExecutionRow {
        ExecutionRow::Object(MaterializedObjectRow {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: crate::materialized::OutputGrain::LatestState,
            change: None,
            object_type_id: 1,
            object_type_name: "Person".into(),
            branch_key: 0,
            goid: goid.into(),
            record_id: "record".into(),
            timestamp_us: 0,
            csn: 0,
            record_kind: "baseline".into(),
            tombstone_status: "live".into(),
            properties: BTreeMap::new(),
            property_ids: BTreeMap::new(),
            redacted_properties: BTreeSet::new(),
        })
    }

    #[test]
    fn operation_target_metadata_classifies_grains() {
        assert_eq!(
            entry_grain(&entry(Some("object"), None)),
            EvidenceGrainKind::Object
        );
        assert_eq!(
            entry_grain(&entry(Some("association"), None)),
            EvidenceGrainKind::Association
        );
        assert_eq!(
            entry_grain(&entry(Some("source_row"), None)),
            EvidenceGrainKind::Source
        );
    }

    #[test]
    fn missing_metadata_is_kept_as_materialized_fallback() {
        let root = ResolvedEvidenceRoot {
            target: Some(ResolvedEvidenceTarget::Property {
                object_type_id: Some(1),
                property_id: 2,
                property_name: "active".into(),
            }),
            grain: AstEvidenceGrain::Property,
            mapping_id: None,
            mapping_version: None,
        };
        assert!(evidence_entry_matches_root(&entry(None, None), &root));
        assert!(evidence_entry_matches_root(
            &entry(Some("property"), Some("active")),
            &root
        ));
        assert!(!evidence_entry_matches_root(
            &entry(Some("object"), None),
            &root
        ));
        assert!(!evidence_entry_matches_root(
            &entry(Some("property"), Some("inactive")),
            &root
        ));
    }

    #[test]
    fn evidence_grain_index_filters_by_current_object_target() {
        let rows = vec![
            evidence_row("a", "object", "assertion-a"),
            evidence_row("b", "object", "assertion-b"),
        ];
        let index = EvidenceGrainIndex::from_rows(&rows);
        let root = ResolvedEvidenceRoot {
            target: Some(ResolvedEvidenceTarget::ObjectType {
                object_type_id: 1,
                type_name: "Person".into(),
            }),
            grain: AstEvidenceGrain::Object,
            mapping_id: None,
            mapping_version: None,
        };

        assert!(index.exists_for(&object_row("a"), &root));
        assert_eq!(index.count_for(&object_row("a"), &root), 1);
        assert_eq!(index.identities_for(&object_row("a"), &root).len(), 1);
        assert!(!index.exists_for(&object_row("missing"), &root));
    }

    #[test]
    fn evidence_grain_index_keeps_manifest_member_scopes_distinct() {
        let mut left = evidence_row("a", "object", "left-assertion");
        left.fields.insert("dataset_file_ordinal".into(), json!(0));
        let mut right = evidence_row("a", "object", "right-assertion");
        right.fields.insert("dataset_file_ordinal".into(), json!(1));
        let index = EvidenceGrainIndex::from_rows(&[left, right]);
        let root = ResolvedEvidenceRoot {
            target: Some(ResolvedEvidenceTarget::ObjectType {
                object_type_id: 1,
                type_name: "Person".into(),
            }),
            grain: AstEvidenceGrain::Object,
            mapping_id: None,
            mapping_version: None,
        };
        let left_object = object_row("a").with_dataset_member(0, "left.cove", "left-file".into());
        let right_object =
            object_row("a").with_dataset_member(1, "right.cove", "right-file".into());

        assert_eq!(index.count_for(&left_object, &root), 1);
        assert_eq!(index.count_for(&right_object, &root), 1);
        assert_eq!(index.count_for(&object_row("a"), &root), 0);
        assert_eq!(
            index.identities_for(&left_object, &root),
            ["0:left-assertion".to_string()].into_iter().collect()
        );
        assert_eq!(
            index.identities_for(&right_object, &root),
            ["1:right-assertion".to_string()].into_iter().collect()
        );
    }

    #[test]
    fn evidence_grain_index_uses_property_keys_and_retains_ambiguous_fallbacks() {
        let mut active = evidence_row("a", "property", "active-assertion");
        active
            .fields
            .insert("property_name".into(), json!("active"));
        let mut inactive = evidence_row("a", "property", "inactive-assertion");
        inactive
            .fields
            .insert("property_name".into(), json!("inactive"));
        let ambiguous = evidence_row("a", "unknown", "ambiguous-assertion");
        let index = EvidenceGrainIndex::from_rows(&[active, inactive, ambiguous]);
        let root = ResolvedEvidenceRoot {
            target: Some(ResolvedEvidenceTarget::Property {
                object_type_id: Some(1),
                property_id: 2,
                property_name: "active".into(),
            }),
            grain: AstEvidenceGrain::Property,
            mapping_id: None,
            mapping_version: None,
        };

        let identities = index.identities_for(&object_row("a"), &root);
        assert!(identities.contains("active-assertion"));
        assert!(identities.contains("ambiguous-assertion"));
        assert!(!identities.contains("inactive-assertion"));
    }
}
