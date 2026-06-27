use std::collections::{BTreeMap, BTreeSet};

use cove_core::{
    artifact::covemap::CovemapFile,
    profile::cove_map::{
        MapCanonicalAnchor, MapIdentityRule, MapReviewedDecision, MapTypedIdentityReference,
    },
};
use serde_json::Value;

use crate::{
    hex_encode, join_key_tuple_from_rule_with_context, mapped_goid, mapping_context,
    object_types_from_mapping, row_digest, schema_fingerprint, JoinKeyEvaluation,
    ResolutionMetadata, SourceRow,
};

#[derive(Debug, Clone)]
pub(crate) struct PlannedIdentity {
    pub(crate) source_id: String,
    pub(crate) row_index: usize,
    pub(crate) row_digest: String,
    pub(crate) schema_fingerprint: String,
    pub(crate) source_row_identity: String,
    pub(crate) row_rule_id: String,
    pub(crate) identity_rule_id: String,
    pub(crate) object_type: String,
    pub(crate) join_key_sha256: String,
    pub(crate) identity_alias: String,
    pub(crate) equivalence_id: String,
    pub(crate) canonical_anchor: String,
    pub(crate) goid: [u8; 16],
    pub(crate) resolution_metadata: Vec<ResolutionMetadata>,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateMatch {
    pub(crate) source_id: String,
    pub(crate) row_index: usize,
    pub(crate) row_digest: String,
    pub(crate) schema_fingerprint: String,
    pub(crate) source_row_identity: String,
    pub(crate) row_rule_id: String,
    pub(crate) identity_rule_id: String,
    pub(crate) object_type: String,
    pub(crate) join_key_sha256: String,
    pub(crate) identity_alias: String,
    pub(crate) resolution_metadata: Vec<ResolutionMetadata>,
}

#[derive(Debug, Clone)]
pub(crate) struct IdentityPlan {
    pub(crate) canonical: Vec<PlannedIdentity>,
    pub(crate) candidates: Vec<CandidateMatch>,
}

pub(crate) fn plan_identities(
    file: &CovemapFile,
    rows: &[SourceRow],
) -> Result<IdentityPlan, String> {
    let context = mapping_context(file)?;
    let object_types = object_types_from_mapping(&context)?;
    let type_ids = object_types
        .iter()
        .map(|ty| (ty.type_name.clone(), ty.object_type_id))
        .collect::<BTreeMap<_, _>>();
    let mut keys = Vec::<IdentityKey>::new();
    let mut candidates = Vec::<CandidateMatch>::new();
    for row in rows {
        let matching_rules = context
            .row_rules
            .iter()
            .filter(|rule| rule.source_id == row.source_id)
            .collect::<Vec<_>>();
        if matching_rules.is_empty() {
            return Err(format!(
                "source '{}' has no declared row semantic rule",
                row.source_id
            ));
        }
        for row_rule in matching_rules {
            let identity_rule = context
                .identity_rules
                .get(&row_rule.identity_rule_id)
                .ok_or_else(|| {
                    format!(
                        "row rule '{}' references missing identity rule '{}'",
                        row_rule.rule_id, row_rule.identity_rule_id
                    )
                })?;
            let object_type_id = *type_ids
                .get(&identity_rule.object_type)
                .ok_or_else(|| format!("unknown object type '{}'", identity_rule.object_type))?;
            let evaluation = join_key_tuple_from_rule_with_context(
                identity_rule,
                row,
                object_type_id,
                Some(&context),
            )?;
            let source_row_identity = format!("{}:{}", row.source_id, row.row_index);
            let row_digest = row_digest(row);
            let schema_fingerprint = schema_fingerprint(row);
            let join_key_sha256 = crate::sha256_hex(&evaluation.tuple);
            if is_candidate_identity_rule(identity_rule) || !evaluation.materializes_identity {
                candidates.push(CandidateMatch {
                    source_id: row.source_id.clone(),
                    row_index: row.row_index,
                    row_digest,
                    schema_fingerprint,
                    source_row_identity,
                    row_rule_id: row_rule.rule_id.clone(),
                    identity_rule_id: identity_rule.rule_id.clone(),
                    object_type: identity_rule.object_type.clone(),
                    join_key_sha256: join_key_sha256.clone(),
                    identity_alias: format!("{}:{join_key_sha256}", identity_rule.rule_id),
                    resolution_metadata: evaluation.resolution_metadata,
                });
                continue;
            }
            let merge_class = merge_class(identity_rule, &evaluation);
            let source_order = context
                .source_order
                .get(&row.source_id)
                .copied()
                .unwrap_or(usize::MAX);
            let rule_order = context
                .identity_rule_order
                .get(&identity_rule.rule_id)
                .copied()
                .unwrap_or(usize::MAX);
            keys.push(IdentityKey {
                source_id: row.source_id.clone(),
                row_index: row.row_index,
                row_digest,
                schema_fingerprint,
                source_row_identity,
                row_rule_id: row_rule.rule_id.clone(),
                identity_rule_id: identity_rule.rule_id.clone(),
                object_type: identity_rule.object_type.clone(),
                object_type_id,
                class_rank: identity_class_rank(&identity_rule.confidence_class),
                rule_order,
                source_order,
                join_key_tuple: evaluation.tuple,
                join_key_sha256,
                merge_class,
                resolution_metadata: evaluation.resolution_metadata,
            });
        }
    }

    let mut uf = UnionFind::new(keys.len());
    let mut merge_groups = BTreeMap::<Vec<u8>, Vec<usize>>::new();
    for (index, key) in keys.iter().enumerate() {
        if let Some(group_key) = key.merge_group_key() {
            merge_groups.entry(group_key).or_default().push(index);
        }
    }
    for indexes in merge_groups.values() {
        if let Some((first, rest)) = indexes.split_first() {
            for index in rest {
                uf.union(*first, *index);
            }
        }
    }

    let reviewed_decisions = context
        .resolution_catalog
        .as_ref()
        .map(|catalog| catalog.reviewed_decisions.as_slice())
        .unwrap_or(&[]);
    let reviewed_plan =
        apply_reviewed_decisions(&context, &type_ids, reviewed_decisions, &mut uf, &keys)?;

    let mut components = BTreeMap::<usize, Vec<usize>>::new();
    for index in 0..keys.len() {
        components.entry(uf.find(index)).or_default().push(index);
    }
    validate_do_not_merge(&context.do_not_merge, &components, &keys)?;
    validate_reviewed_do_not_merge(&reviewed_plan.do_not_merge, &mut uf)?;

    let mut planned = Vec::with_capacity(keys.len());
    for indexes in components.values() {
        let anchor_index = indexes
            .iter()
            .copied()
            .min_by_key(|index| keys[*index].anchor_sort_key())
            .ok_or_else(|| "empty identity component".to_string())?;
        let anchor = &keys[anchor_index];
        let reviewed_anchor = reviewed_anchor_for_component(indexes, &reviewed_plan.same_object)?;
        let has_reviewed_merge = reviewed_plan
            .same_object
            .iter()
            .any(|edge| indexes.contains(&edge.left) && indexes.contains(&edge.right));
        let (goid_rule_id, goid_tuple, canonical_anchor, source_scope) =
            if let Some(reviewed_anchor) = reviewed_anchor {
                (
                    reviewed_anchor.identity_rule_id.clone(),
                    reviewed_anchor.join_key_tuple.clone(),
                    reviewed_anchor.identity_alias.clone(),
                    None,
                )
            } else {
                (
                    anchor.identity_rule_id.clone(),
                    anchor.join_key_tuple.clone(),
                    anchor.anchor_alias(),
                    if has_reviewed_merge {
                        None
                    } else {
                        anchor.goid_source_scope()
                    },
                )
            };
        let goid = mapped_goid(
            &file.header.mapping_id,
            file.mapping_version.as_bytes(),
            anchor.object_type_id,
            goid_rule_id.as_bytes(),
            &goid_tuple,
            source_scope.as_deref(),
        );
        let equivalence_id = format!("{}:{}", anchor.object_type, hex_encode(&goid));
        for index in indexes {
            let key = &keys[*index];
            planned.push(PlannedIdentity {
                source_id: key.source_id.clone(),
                row_index: key.row_index,
                row_digest: key.row_digest.clone(),
                schema_fingerprint: key.schema_fingerprint.clone(),
                source_row_identity: key.source_row_identity.clone(),
                row_rule_id: key.row_rule_id.clone(),
                identity_rule_id: key.identity_rule_id.clone(),
                object_type: key.object_type.clone(),
                join_key_sha256: key.join_key_sha256.clone(),
                identity_alias: key.anchor_alias(),
                equivalence_id: equivalence_id.clone(),
                canonical_anchor: canonical_anchor.clone(),
                goid,
                resolution_metadata: key.resolution_metadata.clone(),
            });
        }
    }
    planned.sort_by_key(|identity| {
        (
            identity.source_id.clone(),
            identity.row_index,
            identity.identity_rule_id.clone(),
            identity.goid,
        )
    });
    candidates.sort_by_key(|candidate| {
        (
            candidate.source_id.clone(),
            candidate.row_index,
            candidate.identity_rule_id.clone(),
            candidate.join_key_sha256.clone(),
        )
    });
    Ok(IdentityPlan {
        canonical: planned,
        candidates,
    })
}

fn is_candidate_identity_rule(rule: &MapIdentityRule) -> bool {
    rule.candidate_only || rule.confidence_class == "candidate"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentityMergeClass {
    MergeGlobal,
    MergeWithinSource,
    Singleton,
}

#[derive(Debug, Clone)]
struct IdentityKey {
    source_id: String,
    row_index: usize,
    row_digest: String,
    schema_fingerprint: String,
    source_row_identity: String,
    row_rule_id: String,
    identity_rule_id: String,
    object_type: String,
    object_type_id: u32,
    class_rank: u8,
    rule_order: usize,
    source_order: usize,
    join_key_tuple: Vec<u8>,
    join_key_sha256: String,
    merge_class: IdentityMergeClass,
    resolution_metadata: Vec<ResolutionMetadata>,
}

impl IdentityKey {
    fn merge_group_key(&self) -> Option<Vec<u8>> {
        if self.merge_class == IdentityMergeClass::Singleton {
            return None;
        }
        let mut out = Vec::new();
        crate::append_len_bytes(&mut out, self.object_type.as_bytes());
        crate::append_len_bytes(&mut out, self.identity_rule_id.as_bytes());
        if self.merge_class == IdentityMergeClass::MergeWithinSource {
            crate::append_len_bytes(&mut out, self.source_id.as_bytes());
        }
        crate::append_len_bytes(&mut out, &self.join_key_tuple);
        Some(out)
    }

    fn anchor_sort_key(&self) -> (u8, usize, usize, Vec<u8>, String) {
        (
            self.class_rank,
            self.rule_order,
            self.source_order,
            self.join_key_tuple.clone(),
            self.source_row_identity.clone(),
        )
    }

    fn goid_source_scope(&self) -> Option<String> {
        match self.merge_class {
            IdentityMergeClass::MergeGlobal => None,
            IdentityMergeClass::MergeWithinSource => Some(self.source_id.clone()),
            IdentityMergeClass::Singleton => Some(self.source_row_identity.clone()),
        }
    }

    fn anchor_alias(&self) -> String {
        format!("{}:{}", self.identity_rule_id, self.join_key_sha256)
    }

    fn aliases(&self) -> BTreeSet<String> {
        BTreeSet::from([
            self.source_row_identity.clone(),
            self.row_digest.clone(),
            self.anchor_alias(),
            format!("{}:{}", self.object_type, self.join_key_sha256),
        ])
    }

    fn resolver_family(&self) -> Vec<(String, String)> {
        let mut family = self
            .resolution_metadata
            .iter()
            .filter_map(|metadata| {
                Some((
                    metadata.resolver_id.clone(),
                    metadata.canonical_key.clone()?,
                ))
            })
            .collect::<Vec<_>>();
        family.sort();
        family.dedup();
        family
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReviewedAnchor {
    identity_rule_id: String,
    join_key_tuple: Vec<u8>,
    identity_alias: String,
}

#[derive(Debug, Clone)]
struct ReviewedSameObjectEdge {
    left: usize,
    right: usize,
    canonical_anchor: Option<ReviewedAnchor>,
}

#[derive(Debug, Clone)]
struct ReviewedDoNotMergeConstraint {
    left: Vec<usize>,
    right: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
struct ReviewedDecisionPlan {
    same_object: Vec<ReviewedSameObjectEdge>,
    do_not_merge: Vec<ReviewedDoNotMergeConstraint>,
}

fn apply_reviewed_decisions(
    context: &crate::MappingContext,
    type_ids: &BTreeMap<String, u32>,
    decisions: &[MapReviewedDecision],
    uf: &mut UnionFind,
    keys: &[IdentityKey],
) -> Result<ReviewedDecisionPlan, String> {
    let mut plan = ReviewedDecisionPlan::default();
    for decision in decisions {
        let left = resolve_typed_reference(context, &decision.left, keys)?;
        let right = resolve_typed_reference(context, &decision.right, keys)?;
        match decision.decision.as_str() {
            "same_object" => {
                let object_type =
                    validate_reviewed_same_object_allowed(context, keys, &left, &right)?;
                let requires_anchor = reviewed_decision_requires_anchor(keys, &left, &right);
                if requires_anchor && decision.canonical_anchor.is_none() {
                    return Err(format!(
                        "reviewed decision '{}' requires canonical_anchor",
                        decision.decision_id
                    ));
                }
                if let Some(anchor) = decision.canonical_anchor.as_ref() {
                    if anchor.object_type != object_type {
                        return Err(format!(
                            "reviewed canonical anchor object type '{}' does not match reviewed same-object object type '{}'",
                            anchor.object_type, object_type
                        ));
                    }
                }
                let canonical_anchor = decision
                    .canonical_anchor
                    .as_ref()
                    .map(|anchor| reviewed_anchor(context, type_ids, anchor))
                    .transpose()?;
                for &left_index in &left {
                    for &right_index in &right {
                        uf.union(left_index, right_index);
                        plan.same_object.push(ReviewedSameObjectEdge {
                            left: left_index,
                            right: right_index,
                            canonical_anchor: canonical_anchor.clone(),
                        });
                    }
                }
            }
            "do_not_merge" => {
                plan.do_not_merge
                    .push(ReviewedDoNotMergeConstraint { left, right });
            }
            other => {
                return Err(format!(
                    "reviewed decision '{}' has unsupported decision '{}'",
                    decision.decision_id, other
                ));
            }
        }
    }
    Ok(plan)
}

fn resolve_typed_reference(
    context: &crate::MappingContext,
    reference: &MapTypedIdentityReference,
    keys: &[IdentityKey],
) -> Result<Vec<usize>, String> {
    let mut matches = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        if key.object_type != reference.object_type {
            continue;
        }
        let matched = match reference.kind.as_str() {
            "identity_join_key" => {
                reference.identity_rule_id.as_deref() == Some(key.identity_rule_id.as_str())
                    && reference.join_key_sha256.as_deref() == Some(key.join_key_sha256.as_str())
            }
            "resolver_key" => key.resolution_metadata.iter().any(|metadata| {
                Some(metadata.resolver_id.as_str()) == reference.resolver_id.as_deref()
                    && metadata.canonical_key.as_deref() == reference.canonical_key.as_deref()
            }),
            "source_row" => {
                let Some(source_id) = reference.source_id.as_deref() else {
                    continue;
                };
                let Some(expected_source) = context.sources.get(source_id) else {
                    continue;
                };
                reference.identity_rule_id.as_deref() == Some(key.identity_rule_id.as_str())
                    && Some(key.source_id.as_str()) == reference.source_id.as_deref()
                    && Some(key.source_row_identity.as_str())
                        == reference.source_row_identity.as_deref()
                    && Some(key.schema_fingerprint.as_str())
                        == reference.schema_fingerprint.as_deref()
                    && expected_source.snapshot_digest.as_deref()
                        == reference.source_snapshot_digest.as_deref()
            }
            "row_digest" => {
                reference.row_digest.as_deref() == Some(key.row_digest.as_str())
                    && reference_matches_optional(
                        reference.identity_rule_id.as_deref(),
                        &key.identity_rule_id,
                    )
                    && reference_matches_optional(reference.source_id.as_deref(), &key.source_id)
                    && reference_matches_optional(
                        reference.source_row_identity.as_deref(),
                        &key.source_row_identity,
                    )
            }
            "identity_alias" => reference
                .identity_alias
                .as_deref()
                .is_some_and(|alias| key.aliases().contains(alias)),
            other => {
                return Err(format!(
                    "reviewed identity reference kind '{other}' is unsupported"
                ));
            }
        };
        if matched {
            matches.push(index);
        }
    }
    if matches.is_empty() {
        return Err(format!(
            "reviewed identity reference '{}' did not match any planned identity",
            reference.kind
        ));
    }
    if reference.kind == "row_digest" && matches.len() > 1 {
        return Err(format!(
            "reviewed row_digest reference matched {} planned identities; use source_row or identity_join_key for an unambiguous review decision",
            matches.len()
        ));
    }
    Ok(matches)
}

fn reference_matches_optional(expected: Option<&str>, actual: &str) -> bool {
    expected.map_or(true, |expected| expected == actual)
}

fn validate_reviewed_same_object_allowed<'a>(
    context: &crate::MappingContext,
    keys: &'a [IdentityKey],
    left: &[usize],
    right: &[usize],
) -> Result<&'a str, String> {
    let object_type = reviewed_same_object_object_type(keys, left, right)?;
    for index in left.iter().chain(right.iter()).copied() {
        let rule = context
            .identity_rules
            .get(&keys[index].identity_rule_id)
            .ok_or_else(|| {
                format!(
                    "reviewed decision references missing identity rule '{}'",
                    keys[index].identity_rule_id
                )
            })?;
        if !rule.allow_reviewed_equivalence {
            return Err(format!(
                "identity rule '{}' does not allow reviewed equivalence",
                rule.rule_id
            ));
        }
    }
    Ok(object_type)
}

fn reviewed_same_object_object_type<'a>(
    keys: &'a [IdentityKey],
    left: &[usize],
    right: &[usize],
) -> Result<&'a str, String> {
    let mut indexes = left.iter().chain(right.iter()).copied();
    let first = indexes
        .next()
        .ok_or_else(|| "reviewed same-object decision has no left or right matches".to_string())?;
    let object_type = keys[first].object_type.as_str();
    for index in indexes {
        if keys[index].object_type != object_type {
            return Err(format!(
                "reviewed same-object decision crosses object types '{}' and '{}'",
                object_type, keys[index].object_type
            ));
        }
    }
    Ok(object_type)
}

fn reviewed_decision_requires_anchor(
    keys: &[IdentityKey],
    left: &[usize],
    right: &[usize],
) -> bool {
    let mut identity_rules = BTreeSet::new();
    let mut resolver_families = BTreeSet::new();
    for index in left.iter().chain(right.iter()).copied() {
        identity_rules.insert(keys[index].identity_rule_id.clone());
        resolver_families.insert(keys[index].resolver_family());
    }
    identity_rules.len() > 1 || resolver_families.len() > 1
}

fn reviewed_anchor(
    context: &crate::MappingContext,
    type_ids: &BTreeMap<String, u32>,
    anchor: &MapCanonicalAnchor,
) -> Result<ReviewedAnchor, String> {
    if anchor.kind != "resolved_join_key" {
        return Err(format!(
            "reviewed canonical anchor kind '{}' is unsupported",
            anchor.kind
        ));
    }
    let rule = context
        .identity_rules
        .get(&anchor.identity_rule_id)
        .ok_or_else(|| {
            format!(
                "reviewed canonical anchor references missing identity rule '{}'",
                anchor.identity_rule_id
            )
        })?;
    if rule.object_type != anchor.object_type {
        return Err(format!(
            "reviewed canonical anchor object type '{}' does not match identity rule '{}'",
            anchor.object_type, anchor.identity_rule_id
        ));
    }
    validate_reviewed_anchor_shape(rule, anchor)?;
    let object_type_id = *type_ids
        .get(&anchor.object_type)
        .ok_or_else(|| format!("unknown object type '{}'", anchor.object_type))?;
    let join_key_tuple = reviewed_anchor_join_key_tuple(object_type_id, anchor)?;
    let identity_alias = format!(
        "{}:{}",
        anchor.identity_rule_id,
        crate::sha256_hex(&join_key_tuple)
    );
    Ok(ReviewedAnchor {
        identity_rule_id: anchor.identity_rule_id.clone(),
        join_key_tuple,
        identity_alias,
    })
}

fn reviewed_anchor_join_key_tuple(
    object_type_id: u32,
    anchor: &MapCanonicalAnchor,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.extend_from_slice(b"COVE-MAP-JOIN-KEY-V1");
    out.extend_from_slice(&object_type_id.to_le_bytes());
    crate::append_len_bytes(&mut out, anchor.identity_rule_id.as_bytes());
    out.extend_from_slice(&(anchor.components.len() as u32).to_le_bytes());
    for component in &anchor.components {
        let value =
            reviewed_anchor_component_value(&component.logical_type, &component.resolved_value)?;
        let encoded = crate::canonical_component_bytes(&component.logical_type, &value)?;
        crate::append_len_bytes(&mut out, component.role_id.as_bytes());
        crate::append_len_bytes(&mut out, component.logical_type.as_bytes());
        out.push(1);
        crate::append_len_bytes(&mut out, &encoded);
    }
    Ok(out)
}

fn validate_reviewed_anchor_shape(
    rule: &MapIdentityRule,
    anchor: &MapCanonicalAnchor,
) -> Result<(), String> {
    if anchor.components.len() != rule.join_keys.len() {
        return Err(format!(
            "reviewed canonical anchor for identity rule '{}' has {} components, expected {}",
            rule.rule_id,
            anchor.components.len(),
            rule.join_keys.len()
        ));
    }
    for (index, (anchor_component, rule_component)) in anchor
        .components
        .iter()
        .zip(rule.join_keys.iter())
        .enumerate()
    {
        if anchor_component.role_id != rule_component.role_id
            || anchor_component.logical_type != rule_component.logical_type
        {
            return Err(format!(
                "reviewed canonical anchor component {index} does not match identity rule '{}' join key shape",
                rule.rule_id
            ));
        }
    }
    Ok(())
}

fn reviewed_anchor_component_value(
    logical_type: &str,
    resolved_value: &str,
) -> Result<Value, String> {
    match logical_type {
        "bool" | "boolean" => resolved_value
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|_| "reviewed canonical bool anchor value must be true or false".to_string()),
        "int64" | "int" => resolved_value
            .parse::<i64>()
            .map(|value| Value::Number(value.into()))
            .map_err(|_| "reviewed canonical int anchor value must be base-10 int64".to_string()),
        "uint64" | "uint" => resolved_value
            .parse::<u64>()
            .map(|value| Value::Number(value.into()))
            .map_err(|_| "reviewed canonical uint anchor value must be base-10 uint64".to_string()),
        "float64" => resolved_value
            .parse::<f64>()
            .map(|value| serde_json::Number::from_f64(value).map(Value::Number))
            .map_err(|_| {
                "reviewed canonical float anchor value must be finite float64".to_string()
            })?
            .ok_or_else(|| {
                "reviewed canonical float anchor value must be finite float64".to_string()
            }),
        "utf8" | "string" | "binary" => Ok(Value::String(resolved_value.to_string())),
        other => Err(format!(
            "logical type '{other}' is not supported in reviewed canonical anchors"
        )),
    }
}

fn reviewed_anchor_for_component(
    indexes: &[usize],
    edges: &[ReviewedSameObjectEdge],
) -> Result<Option<ReviewedAnchor>, String> {
    let mut anchors = BTreeSet::new();
    for edge in edges {
        if indexes.contains(&edge.left) && indexes.contains(&edge.right) {
            if let Some(anchor) = &edge.canonical_anchor {
                anchors.insert(anchor.clone());
            }
        }
    }
    if anchors.len() > 1 {
        return Err("reviewed same-object component has conflicting canonical anchors".into());
    }
    Ok(anchors.into_iter().next())
}

fn validate_reviewed_do_not_merge(
    constraints: &[ReviewedDoNotMergeConstraint],
    uf: &mut UnionFind,
) -> Result<(), String> {
    for constraint in constraints {
        for &left in &constraint.left {
            for &right in &constraint.right {
                if uf.find(left) == uf.find(right) {
                    return Err(
                        "identity resolution violates reviewed do-not-merge constraint".into(),
                    );
                }
            }
        }
    }
    Ok(())
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    fn find(&mut self, index: usize) -> usize {
        let parent = self.parent[index];
        if parent == index {
            index
        } else {
            let root = self.find(parent);
            self.parent[index] = root;
            root
        }
    }

    fn union(&mut self, left: usize, right: usize) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root != right_root {
            let (keep, replace) = if left_root <= right_root {
                (left_root, right_root)
            } else {
                (right_root, left_root)
            };
            self.parent[replace] = keep;
        }
    }
}

fn merge_class(rule: &MapIdentityRule, evaluation: &JoinKeyEvaluation) -> IdentityMergeClass {
    let confidence = evaluation
        .effective_confidence_class
        .as_deref()
        .unwrap_or(rule.confidence_class.as_str());
    match confidence {
        "authoritative" => {
            if rule.auto_merge.unwrap_or(true) {
                IdentityMergeClass::MergeGlobal
            } else {
                IdentityMergeClass::Singleton
            }
        }
        "strong_deterministic" => {
            if rule.auto_merge.unwrap_or(false) {
                IdentityMergeClass::MergeGlobal
            } else {
                IdentityMergeClass::Singleton
            }
        }
        "source_scoped" => IdentityMergeClass::MergeWithinSource,
        _ => IdentityMergeClass::Singleton,
    }
}

fn identity_class_rank(class: &str) -> u8 {
    match class {
        "authoritative" => 0,
        "strong_deterministic" => 1,
        "source_scoped" => 2,
        "weak_deterministic" => 3,
        _ => 4,
    }
}

fn validate_do_not_merge(
    constraints: &[(String, String)],
    components: &BTreeMap<usize, Vec<usize>>,
    keys: &[IdentityKey],
) -> Result<(), String> {
    for indexes in components.values() {
        let aliases = indexes
            .iter()
            .flat_map(|index| keys[*index].aliases())
            .collect::<BTreeSet<_>>();
        for (left, right) in constraints {
            if aliases.contains(left) && aliases.contains(right) {
                return Err(format!(
                    "identity resolution violates do-not-merge constraint '{left}' <-> '{right}'"
                ));
            }
        }
    }
    Ok(())
}
