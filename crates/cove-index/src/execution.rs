use std::{cmp::Ordering, collections::BTreeMap};

use cove_core::{
    canonical::validate_canonical_payload,
    constants::{CoveLogicalType, DigestAlgorithm, ValueTag},
    domain::INVALID_RANK,
    wire, CoveError,
};
use cove_coverage::CoverageProofStrengthV2;

use crate::{
    CoviAggregateAnswerBlockV2, CoviAggregateAnswerV2, CoviArtifactV2, CoviByteRangePostingV2,
    CoviComparatorKindV2, CoviDimensionalBucketPostingV2, CoviEntryBlockV2, CoviFileRefPostingV2,
    CoviIndexEntryV2, CoviIndexKindV2, CoviIndexRootV2, CoviIndexedTargetKindV2, CoviKeyBlockV2,
    CoviKeyEncodingKindV2, CoviMorselRefPostingV2, CoviObjectPathPostingV2, CoviPageRefPostingV2,
    CoviPostingRepresentationV2, CoviPostingsBlockV2, CoviReferencedFileV2, CoviRowRangePostingV2,
    CoviSectionKindV2, CoviSegmentRefPostingV2, CoviSnapshotValidityV2, IndexCapabilityExactnessV2,
    IndexCapabilityV2, IndexOnlyCapabilityUseContextV2, IndexOnlyCapabilityV2,
};

const ABSENT_U32: u32 = u32::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviFileDigestV2 {
    pub algorithm: DigestAlgorithm,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviRowRangeScopeV2 {
    pub file_ref: u32,
    pub table_id: u32,
    pub segment_id: u32,
    pub morsel_id: u32,
    pub row_start: u64,
    pub row_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviValidationContextV2 {
    pub file_id: [u8; 16],
    pub file_len: u64,
    pub footer_crc32c: u32,
    pub file_digest: Option<CoviFileDigestV2>,
    pub dataset_id: Option<[u8; 16]>,
    pub snapshot_id: Option<[u8; 16]>,
    pub schema_fingerprint_ref: Option<u32>,
    pub semantic_map_fingerprint_ref: Option<u32>,
    pub external_visibility_ref: Option<u32>,
    pub delta_chain_digest: Option<CoviFileDigestV2>,
    pub row_range_scopes: Vec<CoviRowRangeScopeV2>,
    pub now_us: Option<i64>,
    pub allow_file_code_keys: bool,
    pub require_exact: bool,
}

impl CoviValidationContextV2 {
    pub fn for_file(file_id: [u8; 16], file_len: u64, footer_crc32c: u32) -> Self {
        Self {
            file_id,
            file_len,
            footer_crc32c,
            file_digest: None,
            dataset_id: None,
            snapshot_id: None,
            schema_fingerprint_ref: None,
            semantic_map_fingerprint_ref: None,
            external_visibility_ref: None,
            delta_chain_digest: None,
            row_range_scopes: Vec::new(),
            now_us: None,
            allow_file_code_keys: false,
            require_exact: true,
        }
    }

    pub fn with_dataset_id(mut self, dataset_id: [u8; 16]) -> Self {
        self.dataset_id = Some(dataset_id);
        self
    }

    pub fn with_snapshot_id(mut self, snapshot_id: [u8; 16]) -> Self {
        self.snapshot_id = Some(snapshot_id);
        self
    }

    pub fn with_schema_fingerprint_ref(mut self, schema_fingerprint_ref: u32) -> Self {
        self.schema_fingerprint_ref = Some(schema_fingerprint_ref);
        self
    }

    pub fn with_semantic_map_fingerprint_ref(mut self, semantic_map_fingerprint_ref: u32) -> Self {
        self.semantic_map_fingerprint_ref = Some(semantic_map_fingerprint_ref);
        self
    }

    pub fn with_external_visibility_ref(mut self, external_visibility_ref: u32) -> Self {
        self.external_visibility_ref = Some(external_visibility_ref);
        self
    }

    pub fn with_delta_chain_digest(mut self, algorithm: DigestAlgorithm, bytes: Vec<u8>) -> Self {
        self.delta_chain_digest = Some(CoviFileDigestV2 { algorithm, bytes });
        self
    }

    pub fn with_row_range_scopes(mut self, row_range_scopes: Vec<CoviRowRangeScopeV2>) -> Self {
        self.row_range_scopes = row_range_scopes;
        self
    }

    pub fn with_file_digest(mut self, algorithm: DigestAlgorithm, bytes: Vec<u8>) -> Self {
        self.file_digest = Some(CoviFileDigestV2 { algorithm, bytes });
        self
    }

    pub fn with_now_us(mut self, now_us: i64) -> Self {
        self.now_us = Some(now_us);
        self
    }

    pub fn with_file_code_keys(mut self, allowed: bool) -> Self {
        self.allow_file_code_keys = allowed;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCoviArtifactV2 {
    artifact: CoviArtifactV2,
    host_file_ref: u32,
    roots: BTreeMap<u32, CoviIndexRootV2>,
    capabilities: BTreeMap<u32, IndexCapabilityV2>,
    index_only_capabilities: BTreeMap<(u32, u16), IndexOnlyCapabilityV2>,
    snapshot_validity: BTreeMap<u32, CoviSnapshotValidityV2>,
    active_visibility_overlay_ref: Option<u32>,
    key_blocks: BTreeMap<u32, CoviKeyBlockV2>,
    entry_blocks: BTreeMap<u32, CoviEntryBlockV2>,
    postings_blocks: BTreeMap<u32, CoviPostingsBlockV2>,
    aggregate_blocks: BTreeMap<u32, CoviAggregateAnswerBlockV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoviLookupOpV2 {
    Eq,
    Range {
        lower_inclusive: bool,
        upper_inclusive: bool,
    },
    Prefix,
    Membership,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoviLookupKeyV2 {
    CanonicalValueBytes(Vec<u8>),
    FileCode(u32),
    NumCode(u64),
    IntervalTuple(CoviIntervalKeyV2),
    CanonicalHash {
        hash64: u64,
        canonical_value_bytes: Vec<u8>,
    },
    CanonicalHash128 {
        hash128: [u8; 16],
        canonical_value_bytes: Vec<u8>,
    },
    FixedBytes(Vec<u8>),
    Utf8BytewisePrefix(Vec<u8>),
    DimensionalTuple(Vec<u8>),
    ObjectPathTuple(Vec<u8>),
}

impl CoviLookupKeyV2 {
    fn key_bytes(&self) -> Vec<u8> {
        match self {
            Self::CanonicalValueBytes(bytes) => bytes.clone(),
            Self::FileCode(code) => code.to_le_bytes().to_vec(),
            Self::NumCode(code) => code.to_le_bytes().to_vec(),
            Self::IntervalTuple(key) => key.key_bytes(),
            Self::CanonicalHash {
                canonical_value_bytes,
                ..
            } => canonical_value_bytes.clone(),
            Self::CanonicalHash128 {
                canonical_value_bytes,
                ..
            } => canonical_value_bytes.clone(),
            Self::FixedBytes(bytes)
            | Self::Utf8BytewisePrefix(bytes)
            | Self::DimensionalTuple(bytes)
            | Self::ObjectPathTuple(bytes) => bytes.clone(),
        }
    }

    fn hash64(&self) -> Option<u64> {
        match self {
            Self::CanonicalHash { hash64, .. } => Some(*hash64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviIntervalKeyV2 {
    pub lower: Option<Vec<u8>>,
    pub upper: Option<Vec<u8>>,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
}

impl CoviIntervalKeyV2 {
    pub fn new(
        lower: Option<Vec<u8>>,
        upper: Option<Vec<u8>>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Self {
        Self {
            lower,
            upper,
            lower_inclusive,
            upper_inclusive,
        }
    }

    fn key_bytes(&self) -> Vec<u8> {
        const UNBOUNDED: u32 = u32::MAX;

        let lower_len = self
            .lower
            .as_ref()
            .map(|bytes| u32::try_from(bytes.len()).unwrap_or(u32::MAX - 1))
            .unwrap_or(UNBOUNDED);
        let upper_len = self
            .upper
            .as_ref()
            .map(|bytes| u32::try_from(bytes.len()).unwrap_or(u32::MAX - 1))
            .unwrap_or(UNBOUNDED);
        let mut out = Vec::with_capacity(
            9 + self.lower.as_ref().map(Vec::len).unwrap_or(0)
                + self.upper.as_ref().map(Vec::len).unwrap_or(0),
        );
        out.extend_from_slice(&lower_len.to_le_bytes());
        if let Some(bytes) = &self.lower {
            out.extend_from_slice(bytes);
        }
        out.extend_from_slice(&upper_len.to_le_bytes());
        if let Some(bytes) = &self.upper {
            out.extend_from_slice(bytes);
        }
        let mut flags = 0u8;
        if self.lower_inclusive {
            flags |= 1;
        }
        if self.upper_inclusive {
            flags |= 1 << 1;
        }
        out.push(flags);
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoviIntervalEncodingV2 {
    CanonicalBoundsV1,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoviDomainRankContextV2 {
    pub file_code_to_rank: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoviLookupComparatorContextV2 {
    pub domain_rank: Option<CoviDomainRankContextV2>,
    pub interval_encoding: Option<CoviIntervalEncodingV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoviLookupTargetV2 {
    TableColumn {
        table_id: u32,
        column_id: u32,
    },
    ProjectionColumn {
        table_id: u32,
        column_id: u32,
    },
    ObjectProperty {
        object_type_id: u32,
        property_id: u32,
    },
    ObjectPath {
        object_type_id: u32,
        path_ref: u32,
    },
    SemanticDimension {
        semantic_dimension_ref: u32,
    },
    DimensionalTuple {
        semantic_dimension_ref: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviLookupRequestV2 {
    pub table_id: u32,
    pub column_id: u32,
    pub target: CoviLookupTargetV2,
    pub op: CoviLookupOpV2,
    pub lower_key: CoviLookupKeyV2,
    pub upper_key: Option<CoviLookupKeyV2>,
    pub membership_keys: Vec<CoviLookupKeyV2>,
    pub logical_type: Option<CoveLogicalType>,
    pub comparator_context: CoviLookupComparatorContextV2,
    pub require_exact: bool,
}

impl CoviLookupRequestV2 {
    pub fn eq(table_id: u32, column_id: u32, key: CoviLookupKeyV2) -> Self {
        Self {
            table_id,
            column_id,
            target: CoviLookupTargetV2::TableColumn {
                table_id,
                column_id,
            },
            op: CoviLookupOpV2::Eq,
            lower_key: key,
            upper_key: None,
            membership_keys: Vec::new(),
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        }
    }

    pub fn eq_target(target: CoviLookupTargetV2, key: CoviLookupKeyV2) -> Self {
        let (table_id, column_id) = match target {
            CoviLookupTargetV2::TableColumn {
                table_id,
                column_id,
            }
            | CoviLookupTargetV2::ProjectionColumn {
                table_id,
                column_id,
            } => (table_id, column_id),
            _ => (ABSENT_U32, ABSENT_U32),
        };
        Self {
            table_id,
            column_id,
            target,
            op: CoviLookupOpV2::Eq,
            lower_key: key,
            upper_key: None,
            membership_keys: Vec::new(),
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        }
    }

    pub fn membership(
        table_id: u32,
        column_id: u32,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        Self::membership_target(
            CoviLookupTargetV2::TableColumn {
                table_id,
                column_id,
            },
            keys,
        )
    }

    pub fn membership_target(
        target: CoviLookupTargetV2,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        let mut keys = keys.into_iter().collect::<Vec<_>>();
        let lower_key = keys
            .first()
            .cloned()
            .unwrap_or_else(|| CoviLookupKeyV2::CanonicalValueBytes(Vec::new()));
        if !keys.is_empty() {
            keys.remove(0);
        }
        let (table_id, column_id) = match target {
            CoviLookupTargetV2::TableColumn {
                table_id,
                column_id,
            }
            | CoviLookupTargetV2::ProjectionColumn {
                table_id,
                column_id,
            } => (table_id, column_id),
            _ => (ABSENT_U32, ABSENT_U32),
        };
        Self {
            table_id,
            column_id,
            target,
            op: CoviLookupOpV2::Membership,
            lower_key,
            upper_key: None,
            membership_keys: keys,
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        }
    }

    pub fn prefix(table_id: u32, column_id: u32, key: CoviLookupKeyV2) -> Self {
        Self {
            table_id,
            column_id,
            target: CoviLookupTargetV2::TableColumn {
                table_id,
                column_id,
            },
            op: CoviLookupOpV2::Prefix,
            lower_key: key,
            upper_key: None,
            membership_keys: Vec::new(),
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        }
    }

    pub fn range_numcode(
        table_id: u32,
        column_id: u32,
        logical_type: CoveLogicalType,
        lower_key: u64,
        upper_key: Option<u64>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Self {
        Self {
            table_id,
            column_id,
            target: CoviLookupTargetV2::TableColumn {
                table_id,
                column_id,
            },
            op: CoviLookupOpV2::Range {
                lower_inclusive,
                upper_inclusive,
            },
            lower_key: CoviLookupKeyV2::NumCode(lower_key),
            upper_key: upper_key.map(CoviLookupKeyV2::NumCode),
            membership_keys: Vec::new(),
            logical_type: Some(logical_type),
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        }
    }

    pub fn with_domain_rank_context(mut self, file_code_to_rank: Vec<u32>) -> Self {
        self.comparator_context.domain_rank = Some(CoviDomainRankContextV2 { file_code_to_rank });
        self
    }

    pub fn with_interval_encoding(mut self, interval_encoding: CoviIntervalEncodingV2) -> Self {
        self.comparator_context.interval_encoding = Some(interval_encoding);
        self
    }

    pub fn table_column_membership(
        table_id: u32,
        column_id: u32,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        Self::membership(table_id, column_id, keys)
    }

    pub fn projection_column_membership(
        table_id: u32,
        column_id: u32,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        Self::membership_target(
            CoviLookupTargetV2::ProjectionColumn {
                table_id,
                column_id,
            },
            keys,
        )
    }

    pub fn object_property_membership(
        object_type_id: u32,
        property_id: u32,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        Self::membership_target(
            CoviLookupTargetV2::ObjectProperty {
                object_type_id,
                property_id,
            },
            keys,
        )
    }

    pub fn object_path_membership(
        object_type_id: u32,
        path_ref: u32,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        Self::membership_target(
            CoviLookupTargetV2::ObjectPath {
                object_type_id,
                path_ref,
            },
            keys,
        )
    }

    pub fn semantic_dimension_membership(
        semantic_dimension_ref: u32,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        Self::membership_target(
            CoviLookupTargetV2::SemanticDimension {
                semantic_dimension_ref,
            },
            keys,
        )
    }

    pub fn dimensional_tuple_membership(
        semantic_dimension_ref: u32,
        keys: impl IntoIterator<Item = CoviLookupKeyV2>,
    ) -> Self {
        Self::membership_target(
            CoviLookupTargetV2::DimensionalTuple {
                semantic_dimension_ref,
            },
            keys,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviCandidateSetV2 {
    pub exactness: IndexCapabilityExactnessV2,
    pub proof_strength: CoverageProofStrengthV2,
    pub row_ranges: Vec<CoviRowRangePostingV2>,
    pub byte_ranges: Vec<CoviByteRangePostingV2>,
    pub object_paths: Vec<CoviObjectPathPostingV2>,
    pub dimensional_buckets: Vec<CoviDimensionalBucketPostingV2>,
    pub row_ordinal_set_refs: Vec<u32>,
    pub file_refs: Vec<CoviFileRefPostingV2>,
    pub segment_refs: Vec<CoviSegmentRefPostingV2>,
    pub morsel_refs: Vec<CoviMorselRefPostingV2>,
    pub page_refs: Vec<CoviPageRefPostingV2>,
}

impl CoviCandidateSetV2 {
    pub fn is_empty(&self) -> bool {
        self.row_ranges.is_empty()
            && self.byte_ranges.is_empty()
            && self.object_paths.is_empty()
            && self.dimensional_buckets.is_empty()
            && self.row_ordinal_set_refs.is_empty()
            && self.file_refs.is_empty()
            && self.segment_refs.is_empty()
            && self.morsel_refs.is_empty()
            && self.page_refs.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum CoviAggregateKindV2 {
    Count = 0,
    Min = 1,
    Max = 2,
    Exists = 3,
    DistinctCount = 4,
    Membership = 5,
    Sum = 6,
    Avg = 7,
}

impl CoviAggregateKindV2 {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::Count),
            1 => Some(Self::Min),
            2 => Some(Self::Max),
            3 => Some(Self::Exists),
            4 => Some(Self::DistinctCount),
            5 => Some(Self::Membership),
            6 => Some(Self::Sum),
            7 => Some(Self::Avg),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviIndexOnlyRequestV2 {
    pub table_id: u32,
    pub column_id: Option<u32>,
    pub aggregate_kind: CoviAggregateKindV2,
    pub predicate_form_ref: Option<u32>,
    pub require_exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviIndexOnlyAnswerV2 {
    pub aggregate_kind: CoviAggregateKindV2,
    pub row_count: u64,
    pub null_count: u64,
    pub non_null_count: u64,
    pub value: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoviMembershipAnswerV2 {
    pub exactness: IndexCapabilityExactnessV2,
    pub proof_strength: CoverageProofStrengthV2,
    pub requested_key_count: usize,
    pub present_key_count: usize,
    pub present_keys: Vec<Vec<u8>>,
}

impl ValidatedCoviArtifactV2 {
    pub fn parse_and_validate(
        bytes: &[u8],
        context: CoviValidationContextV2,
    ) -> Result<Self, CoveError> {
        let artifact = CoviArtifactV2::parse(bytes)?;
        Self::validate_with_artifact_bytes(artifact, Some(bytes), context)
    }

    pub fn validate(
        artifact: CoviArtifactV2,
        context: CoviValidationContextV2,
    ) -> Result<Self, CoveError> {
        Self::validate_with_artifact_bytes(artifact, None, context)
    }

    fn validate_with_artifact_bytes(
        artifact: CoviArtifactV2,
        artifact_bytes: Option<&[u8]>,
        context: CoviValidationContextV2,
    ) -> Result<Self, CoveError> {
        let host_file_ref = validate_referenced_file(&artifact, artifact_bytes, &context)?.file_ref;
        let snapshot_validity = artifact
            .snapshot_validity
            .iter()
            .map(|entry| (entry.snapshot_validity_ref, entry.clone()))
            .collect::<BTreeMap<_, _>>();
        let string_table = if context.delta_chain_digest.is_some() {
            let Some(bytes) = artifact_bytes else {
                return Err(CoveError::BadCovi);
            };
            if artifact.header.string_table_section_ref == ABSENT_U32 {
                return Err(CoveError::BadCovi);
            }
            Some(
                artifact
                    .section_payload_from_bytes(bytes, artifact.header.string_table_section_ref)?,
            )
        } else {
            None
        };
        for entry in snapshot_validity.values() {
            validate_snapshot(entry, &context, string_table.as_deref())?;
        }

        let mut roots = BTreeMap::new();
        for root in &artifact.index_roots {
            if roots.insert(root.index_root_id, root.clone()).is_some() {
                return Err(CoveError::BadCovi);
            }
            validate_snapshot_ref(root.snapshot_validity_ref, &snapshot_validity)?;
        }
        let mut capabilities = BTreeMap::new();
        for capability in &artifact.capabilities {
            if capabilities
                .insert(capability.index_root_id, capability.clone())
                .is_some()
            {
                return Err(CoveError::BadCovi);
            }
            if !roots.contains_key(&capability.index_root_id) {
                return Err(CoveError::BadCovi);
            }
            validate_snapshot_ref(capability.snapshot_validity_ref, &snapshot_validity)?;
        }
        let mut index_only_capabilities = BTreeMap::new();
        for index_only in &artifact.index_only_capabilities {
            let aggregate_kind = CoviAggregateKindV2::from_u16(index_only.aggregate_kind)
                .ok_or(CoveError::BadCovi)?;
            let capability = artifact
                .capabilities
                .iter()
                .find(|capability| capability.capability_id == index_only.capability_id)
                .ok_or(CoveError::BadCovi)?;
            if capability.supports_index_only == 0
                || !index_only_capability_supports_aggregate(capability, aggregate_kind)
                || index_only.null_semantics != capability.null_semantics
            {
                return Err(CoveError::BadCovi);
            }
            validate_snapshot_ref(index_only.snapshot_validity_ref, &snapshot_validity)?;
            if index_only_capabilities
                .insert(
                    (index_only.capability_id, index_only.aggregate_kind),
                    index_only.clone(),
                )
                .is_some()
            {
                return Err(CoveError::BadCovi);
            }
        }
        let active_visibility_overlay_ref = context
            .external_visibility_ref
            .filter(|overlay_ref| *overlay_ref != ABSENT_U32);

        let mut key_blocks = BTreeMap::new();
        let mut entry_blocks = BTreeMap::new();
        let mut postings_blocks = BTreeMap::new();
        let mut aggregate_blocks = BTreeMap::new();
        let mut key_index = 0usize;
        let mut entry_index = 0usize;
        let mut postings_index = 0usize;
        let mut aggregate_index = 0usize;
        for section in &artifact.sections {
            match section.section_kind {
                CoviSectionKindV2::KeyBlock => {
                    let block = artifact
                        .key_blocks
                        .get(key_index)
                        .ok_or(CoveError::BadCovi)?;
                    key_blocks.insert(section.section_id, block.clone());
                    key_index += 1;
                }
                CoviSectionKindV2::EntryBlock => {
                    let block = artifact
                        .entry_blocks
                        .get(entry_index)
                        .ok_or(CoveError::BadCovi)?;
                    entry_blocks.insert(section.section_id, block.clone());
                    entry_index += 1;
                }
                CoviSectionKindV2::PostingsBlock => {
                    let block = artifact
                        .postings_blocks
                        .get(postings_index)
                        .ok_or(CoveError::BadCovi)?;
                    postings_blocks.insert(section.section_id, block.clone());
                    postings_index += 1;
                }
                CoviSectionKindV2::AggregateAnswerBlock => {
                    let block = artifact
                        .aggregate_answer_blocks
                        .get(aggregate_index)
                        .ok_or(CoveError::BadCovi)?;
                    aggregate_blocks.insert(section.section_id, block.clone());
                    aggregate_index += 1;
                }
                _ => {}
            }
        }

        for root in roots.values() {
            if !key_blocks.contains_key(&root.key_block_section_id)
                || !entry_blocks.contains_key(&root.entry_block_section_id)
                || !postings_blocks.contains_key(&root.postings_block_section_id)
            {
                return Err(CoveError::BadCovi);
            }
            if root.aggregate_block_section_id != ABSENT_U32
                && !aggregate_blocks.contains_key(&root.aggregate_block_section_id)
            {
                return Err(CoveError::BadCovi);
            }
            if root.key_encoding_kind == CoviKeyEncodingKindV2::FileCode as u8
                && !context.allow_file_code_keys
            {
                return Err(CoveError::BadCovi);
            }
            validate_root_blocks(
                root,
                &context,
                &artifact.referenced_files,
                &snapshot_validity,
                &capabilities,
                &key_blocks,
                &entry_blocks,
                &postings_blocks,
                &aggregate_blocks,
            )?;
        }

        Ok(Self {
            artifact,
            host_file_ref,
            roots,
            capabilities,
            index_only_capabilities,
            snapshot_validity,
            active_visibility_overlay_ref,
            key_blocks,
            entry_blocks,
            postings_blocks,
            aggregate_blocks,
        })
    }

    pub fn artifact(&self) -> &CoviArtifactV2 {
        &self.artifact
    }

    pub fn lookup(&self, request: &CoviLookupRequestV2) -> Result<CoviCandidateSetV2, CoveError> {
        if matches!(request.op, CoviLookupOpV2::Membership)
            && request.membership_keys.is_empty()
            && matches!(&request.lower_key, CoviLookupKeyV2::CanonicalValueBytes(bytes) if bytes.is_empty())
        {
            return Err(CoveError::BadCovi);
        }
        let (root, capability) = self.lookup_root(request)?;

        let key_block = self
            .key_blocks
            .get(&root.key_block_section_id)
            .ok_or(CoveError::BadCovi)?;
        let entry_block = self
            .entry_blocks
            .get(&root.entry_block_section_id)
            .ok_or(CoveError::BadCovi)?;
        let postings_block = self
            .postings_blocks
            .get(&root.postings_block_section_id)
            .ok_or(CoveError::BadCovi)?;
        let lower = request.lower_key.key_bytes();
        let upper = request.upper_key.as_ref().map(CoviLookupKeyV2::key_bytes);
        let membership_keys = membership_key_bytes(request);
        let mut rows = Vec::new();
        let mut byte_ranges = Vec::new();
        let mut object_paths = Vec::new();
        let mut dimensional_buckets = Vec::new();
        let mut row_ordinal_set_refs = Vec::new();
        let mut file_refs = Vec::new();
        let mut segment_refs = Vec::new();
        let mut morsel_refs = Vec::new();
        let mut page_refs = Vec::new();
        for entry in &entry_block.entries {
            if entry.index_root_id != root.index_root_id {
                continue;
            }
            let key = key_bytes_for_entry(key_block, entry)?;
            if !entry_hash64_may_match_request(root, request, entry, key)? {
                continue;
            }
            if !key_matches(
                root,
                request,
                key,
                &lower,
                upper.as_deref(),
                &membership_keys,
            )? {
                continue;
            }
            let posting = postings_block
                .postings
                .get(entry.postings_ref as usize)
                .ok_or(CoveError::BadCovi)?;
            let payload = postings_block.posting_payload(posting)?;
            match posting.representation {
                CoviPostingRepresentationV2::RowRangeList => {
                    rows.extend(crate::parse_covi_row_range_postings(payload)?);
                }
                CoviPostingRepresentationV2::ByteRangeList => {
                    byte_ranges.extend(parse_fixed_payload(
                        payload,
                        CoviByteRangePostingV2::LEN,
                        CoviByteRangePostingV2::parse,
                    )?);
                }
                CoviPostingRepresentationV2::ObjectPathRefs => {
                    object_paths.extend(parse_fixed_payload(
                        payload,
                        CoviObjectPathPostingV2::LEN,
                        CoviObjectPathPostingV2::parse,
                    )?);
                }
                CoviPostingRepresentationV2::DimensionalBucketRefs => {
                    dimensional_buckets.extend(parse_fixed_payload(
                        payload,
                        CoviDimensionalBucketPostingV2::LEN,
                        CoviDimensionalBucketPostingV2::parse,
                    )?);
                }
                CoviPostingRepresentationV2::RowOrdinalBitmap
                | CoviPostingRepresentationV2::RowOrdinalDeltaVarint => {
                    row_ordinal_set_refs.extend(parse_u32_refs(payload)?);
                }
                CoviPostingRepresentationV2::SortedFileRefs => {
                    file_refs.extend(parse_fixed_payload(
                        payload,
                        CoviFileRefPostingV2::LEN,
                        CoviFileRefPostingV2::parse,
                    )?);
                }
                CoviPostingRepresentationV2::SortedSegmentRefs => {
                    segment_refs.extend(parse_fixed_payload(
                        payload,
                        CoviSegmentRefPostingV2::LEN,
                        CoviSegmentRefPostingV2::parse,
                    )?);
                }
                CoviPostingRepresentationV2::SortedMorselRefs => {
                    morsel_refs.extend(parse_fixed_payload(
                        payload,
                        CoviMorselRefPostingV2::LEN,
                        CoviMorselRefPostingV2::parse,
                    )?);
                }
                CoviPostingRepresentationV2::SortedPageRefs => {
                    page_refs.extend(parse_fixed_payload(
                        payload,
                        CoviPageRefPostingV2::LEN,
                        CoviPageRefPostingV2::parse,
                    )?);
                }
                CoviPostingRepresentationV2::CoverageSetRef
                | CoviPostingRepresentationV2::Extension => return Err(CoveError::BadCovi),
            }
        }
        normalize_row_ranges(&mut rows)?;
        byte_ranges.sort();
        byte_ranges.dedup();
        object_paths.sort();
        object_paths.dedup();
        dimensional_buckets.sort();
        dimensional_buckets.dedup();
        row_ordinal_set_refs.sort_unstable();
        row_ordinal_set_refs.dedup();
        file_refs.sort();
        file_refs.dedup();
        segment_refs.sort();
        segment_refs.dedup();
        morsel_refs.sort();
        morsel_refs.dedup();
        page_refs.sort();
        page_refs.dedup();
        Ok(CoviCandidateSetV2 {
            exactness: capability.exactness,
            proof_strength: capability.proof_strength,
            row_ranges: rows,
            byte_ranges,
            object_paths,
            dimensional_buckets,
            row_ordinal_set_refs,
            file_refs,
            segment_refs,
            morsel_refs,
            page_refs,
        })
    }

    pub fn exact_membership_answer(
        &self,
        request: &CoviLookupRequestV2,
    ) -> Result<CoviMembershipAnswerV2, CoveError> {
        if request.op != CoviLookupOpV2::Membership || !request.require_exact {
            return Err(CoveError::BadCovi);
        }
        let (root, capability) = self.lookup_root(request)?;
        let key_block = self
            .key_blocks
            .get(&root.key_block_section_id)
            .ok_or(CoveError::BadCovi)?;
        let entry_block = self
            .entry_blocks
            .get(&root.entry_block_section_id)
            .ok_or(CoveError::BadCovi)?;
        let requested = membership_key_bytes(request);
        if requested.is_empty() {
            return Err(CoveError::BadCovi);
        }
        let mut present: Vec<Vec<u8>> = Vec::new();
        for entry in &entry_block.entries {
            if entry.index_root_id != root.index_root_id {
                continue;
            }
            let key = key_bytes_for_entry(key_block, entry)?;
            if !entry_hash64_may_match_request(root, request, entry, key)? {
                continue;
            }
            let mut requested_match = false;
            for candidate in &requested {
                if key_equals(root, request, key, candidate)? {
                    requested_match = true;
                    break;
                }
            }
            if !requested_match {
                continue;
            }
            let answer_key = membership_answer_key_for_entry(root, key)?;
            let mut already_present = false;
            for existing in &present {
                if membership_answer_keys_equal(root, request, existing, &answer_key)? {
                    already_present = true;
                    break;
                }
            }
            if !already_present {
                present.push(answer_key);
            }
        }
        present.sort();
        Ok(CoviMembershipAnswerV2 {
            exactness: capability.exactness,
            proof_strength: capability.proof_strength,
            requested_key_count: requested.len(),
            present_key_count: present.len(),
            present_keys: present,
        })
    }

    pub fn index_only_answer(
        &self,
        request: &CoviIndexOnlyRequestV2,
    ) -> Result<Option<CoviIndexOnlyAnswerV2>, CoveError> {
        self.index_only_answer_for_target(
            CoviLookupTargetV2::TableColumn {
                table_id: request.table_id,
                column_id: request.column_id.unwrap_or(ABSENT_U32),
            },
            request,
        )
    }

    pub fn index_only_answer_for_target(
        &self,
        target: CoviLookupTargetV2,
        request: &CoviIndexOnlyRequestV2,
    ) -> Result<Option<CoviIndexOnlyAnswerV2>, CoveError> {
        let mut saw_matching_root = false;
        let mut saw_inexact_candidate = false;
        let mut saw_unsupported_aggregate_candidate = false;
        for root in self
            .roots
            .values()
            .filter(|root| root_matches_index_only_target(root, target, request.column_id))
        {
            saw_matching_root = true;
            let Some(capability) = self.capabilities.get(&root.index_root_id) else {
                continue;
            };
            if capability.supports_index_only == 0 {
                continue;
            }
            if request.require_exact
                && (capability.exactness != IndexCapabilityExactnessV2::Exact
                    || !capability.proof_strength.supports_exact_covi_use()
                    || !root_supports_exact_covi_use(root)?)
            {
                saw_inexact_candidate = true;
                continue;
            }
            if root.aggregate_block_section_id == ABSENT_U32 {
                continue;
            }
            let Some(block) = self.aggregate_blocks.get(&root.aggregate_block_section_id) else {
                continue;
            };
            let Some(answer) = block.answers.iter().find(|answer| {
                answer.index_root_id == root.index_root_id
                    && CoviAggregateKindV2::from_u16(answer.aggregate_kind)
                        == Some(request.aggregate_kind)
                    && request
                        .predicate_form_ref
                        .map(|predicate| answer.predicate_form_ref == predicate)
                        .unwrap_or(answer.predicate_form_ref == ABSENT_U32)
            }) else {
                continue;
            };
            if request.require_exact && answer.exactness != IndexCapabilityExactnessV2::Exact as u8
            {
                saw_inexact_candidate = true;
                continue;
            }
            if !index_only_capability_supports_aggregate(capability, request.aggregate_kind) {
                if request.require_exact {
                    saw_unsupported_aggregate_candidate = true;
                }
                continue;
            }
            if self
                .validate_index_only_capability_for_answer(capability, request, answer)
                .is_err()
            {
                if request.require_exact {
                    saw_unsupported_aggregate_candidate = true;
                }
                continue;
            }
            return Ok(Some(answer_to_public(answer, block)?));
        }
        if request.require_exact && saw_matching_root && saw_unsupported_aggregate_candidate {
            return Err(CoveError::IndexOnlyUnsafe);
        }
        if request.require_exact && saw_matching_root && saw_inexact_candidate {
            return Err(CoveError::IndexOnlyUnsafe);
        }
        Ok(None)
    }

    fn validate_index_only_capability_for_answer(
        &self,
        capability: &IndexCapabilityV2,
        request: &CoviIndexOnlyRequestV2,
        answer: &CoviAggregateAnswerV2,
    ) -> Result<(), CoveError> {
        let Some(index_only) = self
            .index_only_capabilities
            .get(&(capability.capability_id, request.aggregate_kind as u16))
        else {
            return Err(CoveError::BadCovi);
        };
        if request.predicate_form_ref.is_some() && index_only.predicate_supported == 0 {
            return Err(CoveError::BadCovi);
        }
        if index_only.null_semantics != capability.null_semantics
            || answer.null_semantics != capability.null_semantics
        {
            return Err(CoveError::BadCovi);
        }
        if request.require_exact && answer.exactness != IndexCapabilityExactnessV2::Exact as u8 {
            return Err(CoveError::IndexOnlyUnsafe);
        }
        index_only.validate_for_use_context(&IndexOnlyCapabilityUseContextV2 {
            selected_snapshot_validity_ref: answer.snapshot_validity_ref,
            active_visibility_overlay_ref: self.active_visibility_overlay_ref,
            require_exact: request.require_exact,
        })
    }

    fn lookup_root(
        &self,
        request: &CoviLookupRequestV2,
    ) -> Result<(&CoviIndexRootV2, &IndexCapabilityV2), CoveError> {
        for root in self
            .roots
            .values()
            .filter(|root| root_matches_target(root, request.target))
        {
            let Some(capability) = self.capabilities.get(&root.index_root_id) else {
                continue;
            };
            if !lookup_capability_supports_request(capability, request) {
                continue;
            }
            if request.require_exact && !root_supports_exact_covi_use(root)? {
                continue;
            }
            if !root_blocks_available(self, root) {
                continue;
            }
            if !root_key_encoding_matches_request(root, request) {
                continue;
            }
            if !root_comparator_matches_request(root, request) {
                continue;
            }
            return Ok((root, capability));
        }
        Err(CoveError::BadCovi)
    }
}

fn lookup_capability_supports_request(
    capability: &IndexCapabilityV2,
    request: &CoviLookupRequestV2,
) -> bool {
    if request.require_exact {
        if capability.exactness != IndexCapabilityExactnessV2::Exact {
            return false;
        }
        if !capability.proof_strength.supports_exact_covi_use() {
            return false;
        }
    }
    match request.op {
        CoviLookupOpV2::Eq => capability.supports_eq != 0,
        CoviLookupOpV2::Range { .. } => capability.supports_range != 0,
        CoviLookupOpV2::Prefix => capability.supports_prefix != 0,
        CoviLookupOpV2::Membership => capability.supports_membership != 0,
    }
}

trait CoviExactProofStrength {
    fn supports_exact_covi_use(self) -> bool;
}

impl CoviExactProofStrength for CoverageProofStrengthV2 {
    fn supports_exact_covi_use(self) -> bool {
        matches!(self, Self::ExactTight | Self::ExactConservative)
    }
}

fn root_supports_exact_covi_use(root: &CoviIndexRootV2) -> Result<bool, CoveError> {
    let proof_strength =
        CoverageProofStrengthV2::from_u8(root.proof_strength).ok_or(CoveError::BadCovi)?;
    Ok(proof_strength.supports_exact_covi_use())
}

fn index_only_capability_supports_aggregate(
    capability: &IndexCapabilityV2,
    aggregate_kind: CoviAggregateKindV2,
) -> bool {
    match aggregate_kind {
        CoviAggregateKindV2::Count | CoviAggregateKindV2::Exists => capability.supports_count != 0,
        CoviAggregateKindV2::Min => capability.supports_min != 0,
        CoviAggregateKindV2::Max => capability.supports_max != 0,
        CoviAggregateKindV2::Sum => capability.supports_sum != 0,
        CoviAggregateKindV2::Avg => capability.supports_sum != 0 && capability.supports_count != 0,
        CoviAggregateKindV2::DistinctCount => capability.supports_distinct_count != 0,
        CoviAggregateKindV2::Membership => capability.supports_membership != 0,
    }
}

fn root_blocks_available(artifact: &ValidatedCoviArtifactV2, root: &CoviIndexRootV2) -> bool {
    artifact.key_blocks.contains_key(&root.key_block_section_id)
        && artifact
            .entry_blocks
            .contains_key(&root.entry_block_section_id)
        && artifact
            .postings_blocks
            .contains_key(&root.postings_block_section_id)
}

fn root_key_encoding_matches_request(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
) -> bool {
    let Some(encoding) = CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind) else {
        return false;
    };
    request_key_encoding_matches(encoding, &request.lower_key)
        && request
            .upper_key
            .as_ref()
            .map(|key| request_key_encoding_matches(encoding, key))
            .unwrap_or(true)
        && request
            .membership_keys
            .iter()
            .all(|key| request_key_encoding_matches(encoding, key))
}

fn request_key_encoding_matches(encoding: CoviKeyEncodingKindV2, key: &CoviLookupKeyV2) -> bool {
    matches!(
        (encoding, key),
        (
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviLookupKeyV2::CanonicalValueBytes(_) | CoviLookupKeyV2::CanonicalHash { .. }
        ) | (
            CoviKeyEncodingKindV2::CanonicalHash64,
            CoviLookupKeyV2::CanonicalHash { .. }
        ) | (
            CoviKeyEncodingKindV2::CanonicalHash128,
            CoviLookupKeyV2::CanonicalHash128 { .. }
        ) | (
            CoviKeyEncodingKindV2::FixedBytes,
            CoviLookupKeyV2::FixedBytes(_)
        ) | (
            CoviKeyEncodingKindV2::Utf8BytewisePrefix,
            CoviLookupKeyV2::Utf8BytewisePrefix(_)
        ) | (
            CoviKeyEncodingKindV2::DimensionalTuple,
            CoviLookupKeyV2::DimensionalTuple(_)
        ) | (
            CoviKeyEncodingKindV2::ObjectPathTuple,
            CoviLookupKeyV2::ObjectPathTuple(_)
        ) | (
            CoviKeyEncodingKindV2::FileCode,
            CoviLookupKeyV2::FileCode(_)
        ) | (CoviKeyEncodingKindV2::NumCode, CoviLookupKeyV2::NumCode(_))
            | (
                CoviKeyEncodingKindV2::IntervalTuple,
                CoviLookupKeyV2::IntervalTuple(_)
            )
    )
}

fn root_comparator_matches_request(root: &CoviIndexRootV2, request: &CoviLookupRequestV2) -> bool {
    let Some(comparator) = CoviComparatorKindV2::from_u16(root.comparator_kind) else {
        return false;
    };
    match comparator {
        CoviComparatorKindV2::ExtensionRequired => false,
        CoviComparatorKindV2::IntervalOverlap => {
            request.comparator_context.interval_encoding.is_some()
                && matches!(request.lower_key, CoviLookupKeyV2::IntervalTuple(_))
        }
        CoviComparatorKindV2::DomainRankOrdering => {
            request.comparator_context.domain_rank.is_some()
        }
        CoviComparatorKindV2::NumCodeLogicalOrdering => {
            let root_logical = CoveLogicalType::from_u16(root.logical_type);
            match (request.logical_type, root_logical) {
                (Some(request_logical), Some(root_logical)) => request_logical == root_logical,
                (Some(_), None) | (None, Some(_)) => true,
                (None, None) => false,
            }
        }
        CoviComparatorKindV2::CanonicalOrdering
        | CoviComparatorKindV2::CanonicalEquality
        | CoviComparatorKindV2::Utf8BytewisePrefix
        | CoviComparatorKindV2::DimensionalTupleLexicographic
        | CoviComparatorKindV2::ObjectPathLexicographic => true,
    }
}

fn root_matches_target(root: &CoviIndexRootV2, target: CoviLookupTargetV2) -> bool {
    match target {
        CoviLookupTargetV2::TableColumn {
            table_id,
            column_id,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::TableColumn
                && root.table_id == table_id
                && root.column_id == column_id
        }
        CoviLookupTargetV2::ProjectionColumn {
            table_id,
            column_id,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::ProjectionColumn
                && root.table_id == table_id
                && root.column_id == column_id
        }
        CoviLookupTargetV2::ObjectProperty {
            object_type_id,
            property_id,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::ObjectProperty
                && root.object_type_id == object_type_id
                && root.property_id == property_id
        }
        CoviLookupTargetV2::ObjectPath {
            object_type_id,
            path_ref,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::ObjectPath
                && root.object_type_id == object_type_id
                && root.path_ref == path_ref
        }
        CoviLookupTargetV2::SemanticDimension {
            semantic_dimension_ref,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::SemanticDimension
                && root.semantic_dimension_ref == semantic_dimension_ref
        }
        CoviLookupTargetV2::DimensionalTuple {
            semantic_dimension_ref,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::DimensionalTuple
                && root.semantic_dimension_ref == semantic_dimension_ref
        }
    }
}

fn root_matches_index_only_target(
    root: &CoviIndexRootV2,
    target: CoviLookupTargetV2,
    request_column_id: Option<u32>,
) -> bool {
    match target {
        CoviLookupTargetV2::TableColumn { table_id, .. } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::TableColumn
                && root.table_id == table_id
                && request_column_id
                    .map(|column_id| root.column_id == column_id)
                    .unwrap_or(true)
        }
        CoviLookupTargetV2::ProjectionColumn { table_id, .. } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::ProjectionColumn
                && root.table_id == table_id
                && request_column_id
                    .map(|column_id| root.column_id == column_id)
                    .unwrap_or(true)
        }
        CoviLookupTargetV2::ObjectProperty {
            object_type_id,
            property_id,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::ObjectProperty
                && root.object_type_id == object_type_id
                && root.property_id == property_id
        }
        CoviLookupTargetV2::ObjectPath {
            object_type_id,
            path_ref,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::ObjectPath
                && root.object_type_id == object_type_id
                && root.path_ref == path_ref
        }
        CoviLookupTargetV2::SemanticDimension {
            semantic_dimension_ref,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::SemanticDimension
                && root.semantic_dimension_ref == semantic_dimension_ref
        }
        CoviLookupTargetV2::DimensionalTuple {
            semantic_dimension_ref,
        } => {
            root.indexed_target_kind == CoviIndexedTargetKindV2::DimensionalTuple
                && root.semantic_dimension_ref == semantic_dimension_ref
        }
    }
}

fn validate_referenced_file<'a>(
    artifact: &'a CoviArtifactV2,
    artifact_bytes: Option<&[u8]>,
    context: &CoviValidationContextV2,
) -> Result<&'a CoviReferencedFileV2, CoveError> {
    let file = artifact
        .referenced_files
        .iter()
        .find(|file| {
            file.file_id == context.file_id
                && file.file_len == context.file_len
                && file.footer_crc32c == context.footer_crc32c
        })
        .ok_or(CoveError::BadCovi)?;
    if let Some(expected) = &context.file_digest {
        let declared_algorithm =
            DigestAlgorithm::from_u16(file.digest_algorithm).ok_or(CoveError::BadCovi)?;
        if declared_algorithm == DigestAlgorithm::None {
            return Err(CoveError::DigestMismatch);
        }
        if declared_algorithm != expected.algorithm {
            return Err(CoveError::DigestMismatch);
        }
        let Some(bytes) = artifact_bytes else {
            return Err(CoveError::BadCovi);
        };
        if artifact.header.string_table_section_ref == ABSENT_U32 {
            return Err(CoveError::BadCovi);
        }
        let string_table =
            artifact.section_payload_from_bytes(bytes, artifact.header.string_table_section_ref)?;
        let start = usize::try_from(file.digest_offset).map_err(|_| CoveError::OffsetRange)?;
        let len = usize::from(file.digest_len);
        let end = start.checked_add(len).ok_or(CoveError::ArithOverflow)?;
        if end > string_table.len() {
            return Err(CoveError::OffsetRange);
        }
        if &string_table[start..end] != expected.bytes.as_slice() {
            return Err(CoveError::DigestMismatch);
        }
    }
    Ok(file)
}

fn validate_snapshot(
    snapshot: &CoviSnapshotValidityV2,
    context: &CoviValidationContextV2,
    string_table: Option<&[u8]>,
) -> Result<(), CoveError> {
    if let Some(dataset_id) = context.dataset_id {
        if snapshot.dataset_id != dataset_id {
            return Err(CoveError::BadCovi);
        }
    }
    if let Some(snapshot_id) = context.snapshot_id {
        if snapshot.snapshot_id != snapshot_id {
            return Err(CoveError::BadCovi);
        }
    }
    if let Some(schema) = context.schema_fingerprint_ref {
        if snapshot.schema_fingerprint_ref != schema {
            return Err(CoveError::BadCovi);
        }
    }
    if let Some(map) = context.semantic_map_fingerprint_ref {
        if snapshot.semantic_map_fingerprint_ref != map {
            return Err(CoveError::BadCovi);
        }
    }
    match context.external_visibility_ref {
        Some(visibility) => {
            if snapshot.external_visibility_ref != visibility {
                return Err(CoveError::BadCovi);
            }
        }
        None => {
            if snapshot.external_visibility_ref != ABSENT_U32 {
                return Err(CoveError::BadCovi);
            }
        }
    }
    let declared_chain_algorithm = DigestAlgorithm::from_u16(snapshot.delta_chain_digest_algorithm)
        .ok_or(CoveError::BadCovi)?;
    match &context.delta_chain_digest {
        Some(expected) => {
            if declared_chain_algorithm == DigestAlgorithm::None {
                return Err(CoveError::BadCovi);
            }
            if declared_chain_algorithm != expected.algorithm {
                return Err(CoveError::DigestMismatch);
            }
            if usize::from(snapshot.delta_chain_digest_len) != expected.bytes.len() {
                return Err(CoveError::DigestMismatch);
            }
            let string_table = string_table.ok_or(CoveError::BadCovi)?;
            let start = usize::try_from(snapshot.delta_chain_digest_offset)
                .map_err(|_| CoveError::OffsetRange)?;
            let end = start
                .checked_add(usize::from(snapshot.delta_chain_digest_len))
                .ok_or(CoveError::ArithOverflow)?;
            if end > string_table.len() {
                return Err(CoveError::OffsetRange);
            }
            if &string_table[start..end] != expected.bytes.as_slice() {
                return Err(CoveError::DigestMismatch);
            }
        }
        None => {
            if declared_chain_algorithm != DigestAlgorithm::None
                || snapshot.delta_chain_digest_len != 0
                || snapshot.delta_chain_digest_offset != 0
            {
                return Err(CoveError::BadCovi);
            }
        }
    }
    if let Some(now_us) = context.now_us {
        if now_us < snapshot.valid_from_us || now_us >= snapshot.valid_until_us {
            return Err(CoveError::BadCovi);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_root_blocks(
    root: &CoviIndexRootV2,
    context: &CoviValidationContextV2,
    referenced_files: &[CoviReferencedFileV2],
    snapshots: &BTreeMap<u32, CoviSnapshotValidityV2>,
    capabilities: &BTreeMap<u32, IndexCapabilityV2>,
    key_blocks: &BTreeMap<u32, CoviKeyBlockV2>,
    entry_blocks: &BTreeMap<u32, CoviEntryBlockV2>,
    postings_blocks: &BTreeMap<u32, CoviPostingsBlockV2>,
    aggregate_blocks: &BTreeMap<u32, CoviAggregateAnswerBlockV2>,
) -> Result<(), CoveError> {
    let key_block = key_blocks
        .get(&root.key_block_section_id)
        .ok_or(CoveError::BadCovi)?;
    let entry_block = entry_blocks
        .get(&root.entry_block_section_id)
        .ok_or(CoveError::BadCovi)?;
    let postings_block = postings_blocks
        .get(&root.postings_block_section_id)
        .ok_or(CoveError::BadCovi)?;
    let aggregate_block = if root.aggregate_block_section_id == ABSENT_U32 {
        None
    } else {
        Some(
            aggregate_blocks
                .get(&root.aggregate_block_section_id)
                .ok_or(CoveError::BadCovi)?,
        )
    };
    if key_block.header.index_root_id != root.index_root_id
        || entry_block.header.index_root_id != root.index_root_id
        || postings_block.header.index_root_id != root.index_root_id
        || key_block.header.encoding_kind as u8 != root.key_encoding_kind
        || key_block.header.comparator_kind as u16 != root.comparator_kind
        || key_block.header.key_count != entry_block.entries.len() as u64
        || entry_block.header.key_block_id != key_block.header.key_block_id
        || entry_block.header.postings_block_id != postings_block.header.postings_block_id
    {
        return Err(CoveError::BadCovi);
    }
    if let Some(aggregate_block) = aggregate_block {
        if aggregate_block.header.index_root_id != root.index_root_id
            || entry_block.header.aggregate_block_id != aggregate_block.header.aggregate_block_id
        {
            return Err(CoveError::BadCovi);
        }
    } else if entry_block.header.aggregate_block_id != ABSENT_U32 {
        return Err(CoveError::BadCovi);
    }
    let capability = capabilities
        .get(&root.index_root_id)
        .ok_or(CoveError::BadCovi)?;
    if root.capability_ref == ABSENT_U32 || capability.capability_id != root.capability_ref {
        return Err(CoveError::BadCovi);
    }

    validate_entries_for_root(
        root,
        key_block,
        entry_block,
        postings_block,
        aggregate_block,
    )?;
    validate_postings_for_root(root, context, referenced_files, postings_block)?;
    if let Some(block) = aggregate_block {
        validate_aggregate_block(root, snapshots, block)?;
    }
    Ok(())
}

fn validate_entries_for_root(
    root: &CoviIndexRootV2,
    key_block: &CoviKeyBlockV2,
    entry_block: &CoviEntryBlockV2,
    postings_block: &CoviPostingsBlockV2,
    aggregate_block: Option<&CoviAggregateAnswerBlockV2>,
) -> Result<(), CoveError> {
    let sorted = matches!(
        root.index_kind,
        CoviIndexKindV2::Sorted | CoviIndexKindV2::SparseSorted
    );
    let mut previous_key: Option<Vec<u8>> = None;
    let mut previous_entry: Option<&CoviIndexEntryV2> = None;
    for entry in &entry_block.entries {
        if entry.index_root_id != root.index_root_id
            || entry.key_kind as u8 != root.key_encoding_kind
            || entry.comparator_kind as u16 != root.comparator_kind
        {
            return Err(CoveError::BadCovi);
        }
        if entry.postings_ref != ABSENT_U32
            && entry.postings_ref as usize >= postings_block.postings.len()
        {
            return Err(CoveError::BadCovi);
        }
        if entry.aggregate_answer_ref != ABSENT_U32 {
            let Some(block) = aggregate_block else {
                return Err(CoveError::BadCovi);
            };
            if entry.aggregate_answer_ref as usize >= block.answers.len() {
                return Err(CoveError::BadCovi);
            }
        }
        if entry.coverage_set_ref != ABSENT_U32 && root.coverage_set_ref == ABSENT_U32 {
            return Err(CoveError::BadCovi);
        }
        let key = key_bytes_for_entry(key_block, entry)?.to_vec();
        validate_key_bytes_for_root(root, &key)?;
        if sorted {
            if let Some(previous) = previous_key.as_ref() {
                let ordering = compare_entry_key_order_for_validation(root, &key, previous)?;
                if ordering == Ordering::Less {
                    return Err(CoveError::BadCovi);
                }
                if ordering == Ordering::Equal {
                    let Some(previous_entry) = previous_entry else {
                        return Err(CoveError::BadCovi);
                    };
                    let chained = previous_entry.next_duplicate_ref == entry.entry_ref;
                    let shared_posting = previous_entry.postings_ref != ABSENT_U32
                        && previous_entry.postings_ref == entry.postings_ref;
                    if !chained && !shared_posting {
                        return Err(CoveError::BadCovi);
                    }
                }
            }
        }
        previous_key = Some(key);
        previous_entry = Some(entry);
    }
    Ok(())
}

fn validate_key_bytes_for_root(root: &CoviIndexRootV2, key: &[u8]) -> Result<(), CoveError> {
    let encoding =
        CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind).ok_or(CoveError::BadCovi)?;
    match encoding {
        CoviKeyEncodingKindV2::FileCode => {
            if key.len() == 4 {
                Ok(())
            } else {
                Err(CoveError::BadCovi)
            }
        }
        CoviKeyEncodingKindV2::NumCode => {
            let logical = CoveLogicalType::from_u16(root.logical_type).ok_or(CoveError::BadCovi)?;
            compare_numcode_range_key(logical, key, key).map(|_| ())
        }
        CoviKeyEncodingKindV2::CanonicalValueBytes | CoviKeyEncodingKindV2::CanonicalHash64 => {
            validate_canonical_key(key)
        }
        CoviKeyEncodingKindV2::CanonicalHash128 => {
            let (_, canonical) = split_hash128_key(key)?;
            validate_canonical_key(canonical)
        }
        CoviKeyEncodingKindV2::FixedBytes
        | CoviKeyEncodingKindV2::DimensionalTuple
        | CoviKeyEncodingKindV2::ObjectPathTuple => Ok(()),
        CoviKeyEncodingKindV2::Utf8BytewisePrefix => validate_utf8_key(key),
        CoviKeyEncodingKindV2::IntervalTuple => parse_interval_key(key).map(|_| ()),
        CoviKeyEncodingKindV2::Extension => Err(CoveError::BadCovi),
    }
}

fn compare_entry_key_order_for_validation(
    root: &CoviIndexRootV2,
    key: &[u8],
    previous: &[u8],
) -> Result<Ordering, CoveError> {
    match compare_key_bytes_for_order(
        root.comparator_kind,
        CoveLogicalType::from_u16(root.logical_type),
        &CoviLookupComparatorContextV2::default(),
        key,
        previous,
    ) {
        Ok(ordering) => Ok(ordering),
        Err(CoveError::BadCovi)
            if CoviComparatorKindV2::from_u16(root.comparator_kind)
                == Some(CoviComparatorKindV2::CanonicalOrdering) =>
        {
            Ok(key.cmp(previous))
        }
        Err(error) => Err(error),
    }
}

fn validate_postings_for_root(
    root: &CoviIndexRootV2,
    context: &CoviValidationContextV2,
    referenced_files: &[CoviReferencedFileV2],
    postings_block: &CoviPostingsBlockV2,
) -> Result<(), CoveError> {
    for posting in &postings_block.postings {
        if posting.index_root_id != root.index_root_id {
            return Err(CoveError::BadCovi);
        }
        let payload = postings_block.posting_payload(posting)?;
        match posting.representation {
            CoviPostingRepresentationV2::SortedFileRefs => {
                for chunk in payload.chunks_exact(crate::CoviFileRefPostingV2::LEN) {
                    let item = crate::CoviFileRefPostingV2::parse(chunk)?;
                    validate_file_ref(item.file_ref, referenced_files)?;
                }
            }
            CoviPostingRepresentationV2::SortedSegmentRefs => {
                for chunk in payload.chunks_exact(crate::CoviSegmentRefPostingV2::LEN) {
                    let item = crate::CoviSegmentRefPostingV2::parse(chunk)?;
                    validate_file_ref(item.file_ref, referenced_files)?;
                }
            }
            CoviPostingRepresentationV2::SortedPageRefs => {
                for chunk in payload.chunks_exact(crate::CoviPageRefPostingV2::LEN) {
                    let item = crate::CoviPageRefPostingV2::parse(chunk)?;
                    validate_file_ref(item.file_ref, referenced_files)?;
                }
            }
            CoviPostingRepresentationV2::SortedMorselRefs => {
                for chunk in payload.chunks_exact(crate::CoviMorselRefPostingV2::LEN) {
                    let item = crate::CoviMorselRefPostingV2::parse(chunk)?;
                    validate_file_ref(item.file_ref, referenced_files)?;
                }
            }
            CoviPostingRepresentationV2::RowRangeList => {
                for row in crate::parse_covi_row_range_postings(payload)? {
                    validate_file_ref(row.file_ref, referenced_files)?;
                    validate_row_range_scope(&row, context)?;
                }
            }
            CoviPostingRepresentationV2::ByteRangeList => {
                for chunk in payload.chunks_exact(crate::CoviByteRangePostingV2::LEN) {
                    let item = crate::CoviByteRangePostingV2::parse(chunk)?;
                    let file = validate_file_ref(item.file_ref, referenced_files)?;
                    let end = item
                        .offset
                        .checked_add(item.length)
                        .ok_or(CoveError::ArithOverflow)?;
                    if end > file.file_len {
                        return Err(CoveError::BadCovi);
                    }
                }
            }
            CoviPostingRepresentationV2::ObjectPathRefs => {
                for chunk in payload.chunks_exact(crate::CoviObjectPathPostingV2::LEN) {
                    let item = crate::CoviObjectPathPostingV2::parse(chunk)?;
                    validate_file_ref(item.file_ref, referenced_files)?;
                }
            }
            CoviPostingRepresentationV2::DimensionalBucketRefs => {
                for chunk in payload.chunks_exact(crate::CoviDimensionalBucketPostingV2::LEN) {
                    let item = crate::CoviDimensionalBucketPostingV2::parse(chunk)?;
                    validate_file_ref(item.file_ref, referenced_files)?;
                }
            }
            CoviPostingRepresentationV2::RowOrdinalBitmap
            | CoviPostingRepresentationV2::RowOrdinalDeltaVarint
            | CoviPostingRepresentationV2::CoverageSetRef
            | CoviPostingRepresentationV2::Extension => {}
        }
    }
    Ok(())
}

fn validate_file_ref(
    file_ref: u32,
    referenced_files: &[CoviReferencedFileV2],
) -> Result<&CoviReferencedFileV2, CoveError> {
    referenced_files
        .get(file_ref as usize)
        .filter(|file| file.file_ref == file_ref)
        .ok_or(CoveError::BadCovi)
}

fn validate_row_range_scope(
    row: &CoviRowRangePostingV2,
    context: &CoviValidationContextV2,
) -> Result<(), CoveError> {
    if context.row_range_scopes.is_empty() {
        return Ok(());
    }
    let scope = context
        .row_range_scopes
        .iter()
        .find(|scope| {
            scope.file_ref == row.file_ref
                && scope.table_id == row.table_id
                && scope.segment_id == row.segment_id
                && scope.morsel_id == row.morsel_id
        })
        .ok_or(CoveError::BadCovi)?;
    let row_end = row
        .row_start
        .checked_add(row.row_count)
        .ok_or(CoveError::ArithOverflow)?;
    let scope_end = scope
        .row_start
        .checked_add(scope.row_count)
        .ok_or(CoveError::ArithOverflow)?;
    if row.row_start < scope.row_start || row_end > scope_end {
        return Err(CoveError::BadCovi);
    }
    Ok(())
}

fn validate_aggregate_block(
    root: &CoviIndexRootV2,
    snapshots: &BTreeMap<u32, CoviSnapshotValidityV2>,
    block: &CoviAggregateAnswerBlockV2,
) -> Result<(), CoveError> {
    for (index, answer) in block.answers.iter().enumerate() {
        let aggregate_kind =
            CoviAggregateKindV2::from_u16(answer.aggregate_kind).ok_or(CoveError::BadCovi)?;
        if answer.aggregate_answer_ref as usize != index
            || answer.index_root_id != root.index_root_id
            || IndexCapabilityExactnessV2::from_u8(answer.exactness).is_none()
        {
            return Err(CoveError::BadCovi);
        }
        answer.validate_counts()?;
        validate_snapshot_ref(answer.snapshot_validity_ref, snapshots)?;
        validate_aggregate_answer_payload(root, aggregate_kind, answer, block)?;
    }
    Ok(())
}

fn validate_aggregate_answer_payload(
    root: &CoviIndexRootV2,
    aggregate_kind: CoviAggregateKindV2,
    answer: &CoviAggregateAnswerV2,
    block: &CoviAggregateAnswerBlockV2,
) -> Result<(), CoveError> {
    let value = aggregate_value_payload(answer, block)?;
    match aggregate_kind {
        CoviAggregateKindV2::Count | CoviAggregateKindV2::Exists => {
            if value.is_some() {
                return Err(CoveError::BadCovi);
            }
        }
        CoviAggregateKindV2::DistinctCount => {
            let Some(value) = value else {
                return Err(CoveError::BadCovi);
            };
            let bytes: [u8; 8] = value.try_into().map_err(|_| CoveError::BadCovi)?;
            let distinct_count = u64::from_le_bytes(bytes);
            if distinct_count > answer.non_null_count {
                return Err(CoveError::BadCovi);
            }
        }
        CoviAggregateKindV2::Min | CoviAggregateKindV2::Max => {
            let Some(value) = value else {
                if answer.non_null_count == 0 {
                    return Ok(());
                }
                return Err(CoveError::BadCovi);
            };
            if answer.non_null_count == 0 {
                return Err(CoveError::BadCovi);
            }
            let logical = CoveLogicalType::from_u16(root.logical_type).ok_or(CoveError::BadCovi)?;
            let value_tag = aggregate_answer_value_tag(logical).ok_or(CoveError::BadCovi)?;
            validate_canonical_payload(value_tag, value).map_err(|_| CoveError::BadCovi)?;
        }
        CoviAggregateKindV2::Sum | CoviAggregateKindV2::Avg => {
            let Some(value) = value else {
                if answer.non_null_count == 0 {
                    return Ok(());
                }
                return Err(CoveError::BadCovi);
            };
            if answer.non_null_count == 0 {
                return Err(CoveError::BadCovi);
            }
            let logical = CoveLogicalType::from_u16(root.logical_type).ok_or(CoveError::BadCovi)?;
            let value_tag = aggregate_sum_answer_value_tag(logical).ok_or(CoveError::BadCovi)?;
            validate_canonical_payload(value_tag, value).map_err(|_| CoveError::BadCovi)?;
        }
        CoviAggregateKindV2::Membership => {}
    }
    Ok(())
}

fn aggregate_answer_value_tag(logical: CoveLogicalType) -> Option<ValueTag> {
    match logical {
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => Some(ValueTag::Int64),
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => Some(ValueTag::UInt64),
        CoveLogicalType::Float32 => Some(ValueTag::Float32Bits),
        CoveLogicalType::Float64 => Some(ValueTag::Float64Bits),
        CoveLogicalType::Decimal64 => Some(ValueTag::Decimal64),
        CoveLogicalType::Decimal128 => Some(ValueTag::Decimal128),
        CoveLogicalType::DateDays => Some(ValueTag::DateDays),
        CoveLogicalType::TimestampMicros => Some(ValueTag::TimestampMicros),
        CoveLogicalType::TimestampNanos => Some(ValueTag::TimestampNanos),
        CoveLogicalType::Utf8 => Some(ValueTag::Utf8),
        CoveLogicalType::Binary => Some(ValueTag::Binary),
        CoveLogicalType::Uuid => Some(ValueTag::Uuid),
        CoveLogicalType::Json => Some(ValueTag::Json),
        _ => None,
    }
}

fn validate_snapshot_ref(
    snapshot_ref: u32,
    snapshots: &BTreeMap<u32, CoviSnapshotValidityV2>,
) -> Result<(), CoveError> {
    if snapshot_ref == ABSENT_U32 || !snapshots.contains_key(&snapshot_ref) {
        return Err(CoveError::BadCovi);
    }
    Ok(())
}

fn key_bytes_for_entry<'a>(
    key_block: &'a CoviKeyBlockV2,
    entry: &CoviIndexEntryV2,
) -> Result<&'a [u8], CoveError> {
    let start = usize::try_from(entry.key_offset).map_err(|_| CoveError::OffsetRange)?;
    let len = usize::try_from(entry.key_length).map_err(|_| CoveError::OffsetRange)?;
    let end = start.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > key_block.key_data.len() {
        return Err(CoveError::OffsetRange);
    }
    Ok(&key_block.key_data[start..end])
}

fn entry_hash64_may_match_request(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    entry: &CoviIndexEntryV2,
    _key: &[u8],
) -> Result<bool, CoveError> {
    let Some(encoding) = CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind) else {
        return Err(CoveError::BadCovi);
    };
    if !matches!(
        encoding,
        CoviKeyEncodingKindV2::CanonicalValueBytes | CoviKeyEncodingKindV2::CanonicalHash64
    ) {
        return Ok(true);
    }
    let mut saw_hash_request = false;
    for request_key in std::iter::once(&request.lower_key).chain(request.membership_keys.iter()) {
        let Some(hash) = request_key.hash64() else {
            continue;
        };
        saw_hash_request = true;
        if entry.key_hash64 == hash {
            return Ok(true);
        }
    }
    if encoding == CoviKeyEncodingKindV2::CanonicalHash64 && saw_hash_request {
        Ok(false)
    } else {
        Ok(true)
    }
}

fn membership_answer_key_for_entry(
    root: &CoviIndexRootV2,
    key: &[u8],
) -> Result<Vec<u8>, CoveError> {
    match CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind) {
        Some(CoviKeyEncodingKindV2::CanonicalHash128) => Ok(split_hash128_key(key)?.1.to_vec()),
        Some(_) => Ok(key.to_vec()),
        None => Err(CoveError::BadCovi),
    }
}

fn membership_answer_keys_equal(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    left: &[u8],
    right: &[u8],
) -> Result<bool, CoveError> {
    if CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind)
        == Some(CoviKeyEncodingKindV2::CanonicalHash128)
    {
        return Ok(left == right);
    }
    key_equals(root, request, left, right)
}

fn membership_key_bytes(request: &CoviLookupRequestV2) -> Vec<Vec<u8>> {
    let mut keys = Vec::with_capacity(1 + request.membership_keys.len());
    let lower = request.lower_key.key_bytes();
    if !matches!(request.op, CoviLookupOpV2::Membership)
        || !matches!(&request.lower_key, CoviLookupKeyV2::CanonicalValueBytes(bytes) if bytes.is_empty())
    {
        keys.push(lower);
    }
    keys.extend(
        request
            .membership_keys
            .iter()
            .map(CoviLookupKeyV2::key_bytes),
    );
    keys.sort();
    keys.dedup();
    keys
}

fn key_matches(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    key: &[u8],
    lower: &[u8],
    upper: Option<&[u8]>,
    membership_keys: &[Vec<u8>],
) -> Result<bool, CoveError> {
    match request.op {
        CoviLookupOpV2::Eq => key_equals(root, request, key, lower),
        CoviLookupOpV2::Membership => {
            for candidate in membership_keys {
                if key_equals(root, request, key, candidate)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        CoviLookupOpV2::Prefix => key_has_prefix(root, key, lower),
        CoviLookupOpV2::Range {
            lower_inclusive,
            upper_inclusive,
        } => {
            let comparator =
                CoviComparatorKindV2::from_u16(root.comparator_kind).ok_or(CoveError::BadCovi)?;
            if comparator == CoviComparatorKindV2::IntervalOverlap {
                return interval_key_overlaps_request(root, request, key, lower);
            }
            let lower_cmp = compare_range_key(root, request, key, lower)?;
            let lower_ok = if lower_inclusive {
                !matches!(lower_cmp, Ordering::Less)
            } else {
                matches!(lower_cmp, Ordering::Greater)
            };
            let upper_ok = match upper {
                Some(upper) => {
                    let upper_cmp = compare_range_key(root, request, key, upper)?;
                    if upper_inclusive {
                        !matches!(upper_cmp, Ordering::Greater)
                    } else {
                        matches!(upper_cmp, Ordering::Less)
                    }
                }
                None => true,
            };
            Ok(lower_ok && upper_ok)
        }
    }
}

fn key_equals(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    left: &[u8],
    right: &[u8],
) -> Result<bool, CoveError> {
    if let Some(result) = raw_encoding_key_equals(root, request, left, right)? {
        return Ok(result);
    }
    let comparator =
        CoviComparatorKindV2::from_u16(root.comparator_kind).ok_or(CoveError::BadCovi)?;
    match comparator {
        CoviComparatorKindV2::NumCodeLogicalOrdering
        | CoviComparatorKindV2::CanonicalOrdering
        | CoviComparatorKindV2::DomainRankOrdering
        | CoviComparatorKindV2::Utf8BytewisePrefix
        | CoviComparatorKindV2::DimensionalTupleLexicographic
        | CoviComparatorKindV2::ObjectPathLexicographic => {
            Ok(compare_range_key(root, request, left, right)? == Ordering::Equal)
        }
        CoviComparatorKindV2::CanonicalEquality => {
            validate_canonical_key(left)?;
            validate_canonical_key(right)?;
            Ok(left == right)
        }
        CoviComparatorKindV2::IntervalOverlap => interval_keys_overlap(root, request, left, right),
        CoviComparatorKindV2::ExtensionRequired => Err(CoveError::UnsupportedEncoding(
            "COVE-I extension comparator is not supported".into(),
        )),
    }
}

fn raw_encoding_key_equals(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    left: &[u8],
    right: &[u8],
) -> Result<Option<bool>, CoveError> {
    let Some(encoding) = CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind) else {
        return Err(CoveError::BadCovi);
    };
    match encoding {
        CoviKeyEncodingKindV2::FixedBytes
        | CoviKeyEncodingKindV2::DimensionalTuple
        | CoviKeyEncodingKindV2::ObjectPathTuple => Ok(Some(left == right)),
        CoviKeyEncodingKindV2::Utf8BytewisePrefix => {
            validate_utf8_key(left)?;
            validate_utf8_key(right)?;
            Ok(Some(left == right))
        }
        CoviKeyEncodingKindV2::CanonicalHash64 => {
            validate_canonical_key(left)?;
            validate_canonical_key(right)?;
            Ok(Some(left == right))
        }
        CoviKeyEncodingKindV2::CanonicalHash128 => {
            let Some(request_hash) = request_hash128_for_canonical(request, right) else {
                return Err(CoveError::BadCovi);
            };
            let (hash, canonical) = split_hash128_key(left)?;
            validate_canonical_key(canonical)?;
            validate_canonical_key(right)?;
            Ok(Some(hash == request_hash && canonical == right))
        }
        _ => Ok(None),
    }
}

fn request_hash128_for_canonical(
    request: &CoviLookupRequestV2,
    canonical: &[u8],
) -> Option<[u8; 16]> {
    std::iter::once(&request.lower_key)
        .chain(request.membership_keys.iter())
        .find_map(|key| match key {
            CoviLookupKeyV2::CanonicalHash128 {
                hash128,
                canonical_value_bytes,
            } if canonical_value_bytes == canonical => Some(*hash128),
            _ => None,
        })
}

fn key_has_prefix(root: &CoviIndexRootV2, key: &[u8], prefix: &[u8]) -> Result<bool, CoveError> {
    let encoding =
        CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind).ok_or(CoveError::BadCovi)?;
    if encoding != CoviKeyEncodingKindV2::Utf8BytewisePrefix {
        return Err(CoveError::BadCovi);
    }
    validate_utf8_key(key)?;
    validate_utf8_key(prefix)?;
    Ok(key.starts_with(prefix))
}

fn validate_utf8_key(bytes: &[u8]) -> Result<(), CoveError> {
    std::str::from_utf8(bytes)
        .map(|_| ())
        .map_err(|_| CoveError::BadCovi)
}

fn split_hash128_key(key: &[u8]) -> Result<([u8; 16], &[u8]), CoveError> {
    if key.len() <= 16 {
        return Err(CoveError::BadCovi);
    }
    let hash = key[..16].try_into().unwrap();
    Ok((hash, &key[16..]))
}

fn compare_range_key(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    left: &[u8],
    right: &[u8],
) -> Result<Ordering, CoveError> {
    if let Some(ordering) = raw_encoding_range_order(root, left, right)? {
        return Ok(ordering);
    }
    let root_logical = CoveLogicalType::from_u16(root.logical_type);
    if let (Some(request_logical), Some(root_logical)) = (request.logical_type, root_logical) {
        if request_logical != root_logical {
            return Err(CoveError::BadCovi);
        }
    }
    compare_key_bytes_primary_order(
        root.comparator_kind,
        request.logical_type.or(root_logical),
        &request.comparator_context,
        left,
        right,
    )
}

fn raw_encoding_range_order(
    root: &CoviIndexRootV2,
    left: &[u8],
    right: &[u8],
) -> Result<Option<Ordering>, CoveError> {
    let Some(encoding) = CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind) else {
        return Err(CoveError::BadCovi);
    };
    match encoding {
        CoviKeyEncodingKindV2::FixedBytes
        | CoviKeyEncodingKindV2::DimensionalTuple
        | CoviKeyEncodingKindV2::ObjectPathTuple => Ok(Some(left.cmp(right))),
        CoviKeyEncodingKindV2::Utf8BytewisePrefix => {
            validate_utf8_key(left)?;
            validate_utf8_key(right)?;
            Ok(Some(left.cmp(right)))
        }
        CoviKeyEncodingKindV2::CanonicalHash64 | CoviKeyEncodingKindV2::CanonicalHash128 => {
            Err(CoveError::BadCovi)
        }
        _ => Ok(None),
    }
}

pub(crate) fn compare_key_bytes_for_order(
    comparator_kind: u16,
    logical_type: Option<CoveLogicalType>,
    comparator_context: &CoviLookupComparatorContextV2,
    left: &[u8],
    right: &[u8],
) -> Result<Ordering, CoveError> {
    let ordering = compare_key_bytes_primary_order(
        comparator_kind,
        logical_type,
        comparator_context,
        left,
        right,
    )?;
    if ordering == Ordering::Equal {
        Ok(left.cmp(right))
    } else {
        Ok(ordering)
    }
}

fn compare_key_bytes_primary_order(
    comparator_kind: u16,
    logical_type: Option<CoveLogicalType>,
    comparator_context: &CoviLookupComparatorContextV2,
    left: &[u8],
    right: &[u8],
) -> Result<Ordering, CoveError> {
    let comparator = CoviComparatorKindV2::from_u16(comparator_kind).ok_or(CoveError::BadCovi)?;
    match comparator {
        CoviComparatorKindV2::NumCodeLogicalOrdering => {
            let logical = logical_type.ok_or(CoveError::BadCovi)?;
            compare_numcode_range_key(logical, left, right)
        }
        CoviComparatorKindV2::CanonicalOrdering => compare_canonical_ordering_key(left, right),
        CoviComparatorKindV2::DomainRankOrdering => {
            compare_domain_rank_key(comparator_context, left, right)
        }
        CoviComparatorKindV2::Utf8BytewisePrefix
        | CoviComparatorKindV2::DimensionalTupleLexicographic
        | CoviComparatorKindV2::ObjectPathLexicographic => Ok(left.cmp(right)),
        CoviComparatorKindV2::CanonicalEquality => {
            validate_canonical_key(left)?;
            validate_canonical_key(right)?;
            Ok(left.cmp(right))
        }
        CoviComparatorKindV2::IntervalOverlap => Err(CoveError::UnsupportedEncoding(
            "COVE-I interval-overlap comparator is not an ordering comparator".into(),
        )),
        CoviComparatorKindV2::ExtensionRequired => Err(CoveError::UnsupportedEncoding(
            "COVE-I extension comparator is not supported".into(),
        )),
    }
}

fn compare_numcode_range_key(
    logical: CoveLogicalType,
    left: &[u8],
    right: &[u8],
) -> Result<Ordering, CoveError> {
    if left.len() != 8 || right.len() != 8 {
        return Err(CoveError::BadCovi);
    }
    let left = u64::from_le_bytes(left.try_into().unwrap());
    let right = u64::from_le_bytes(right.try_into().unwrap());
    let ordering = match logical {
        CoveLogicalType::Int8 => {
            cove_core::types::numcode_as_i8(left).cmp(&cove_core::types::numcode_as_i8(right))
        }
        CoveLogicalType::Int16 => {
            cove_core::types::numcode_as_i16(left).cmp(&cove_core::types::numcode_as_i16(right))
        }
        CoveLogicalType::Int32 => {
            cove_core::types::numcode_as_i32(left).cmp(&cove_core::types::numcode_as_i32(right))
        }
        CoveLogicalType::Int64 => {
            cove_core::types::numcode_as_i64(left).cmp(&cove_core::types::numcode_as_i64(right))
        }
        CoveLogicalType::UInt8 => {
            cove_core::types::numcode_as_u8(left).cmp(&cove_core::types::numcode_as_u8(right))
        }
        CoveLogicalType::UInt16 => {
            cove_core::types::numcode_as_u16(left).cmp(&cove_core::types::numcode_as_u16(right))
        }
        CoveLogicalType::UInt32 => {
            cove_core::types::numcode_as_u32(left).cmp(&cove_core::types::numcode_as_u32(right))
        }
        CoveLogicalType::UInt64 => {
            cove_core::types::numcode_as_u64(left).cmp(&cove_core::types::numcode_as_u64(right))
        }
        CoveLogicalType::Decimal64 => cove_core::types::numcode_as_decimal64(left)
            .cmp(&cove_core::types::numcode_as_decimal64(right)),
        CoveLogicalType::DateDays => cove_core::types::numcode_as_date_days(left)
            .cmp(&cove_core::types::numcode_as_date_days(right)),
        CoveLogicalType::TimestampMicros => cove_core::types::numcode_as_timestamp_micros(left)
            .cmp(&cove_core::types::numcode_as_timestamp_micros(right)),
        CoveLogicalType::TimestampNanos => cove_core::types::numcode_as_timestamp_nanos(left)
            .cmp(&cove_core::types::numcode_as_timestamp_nanos(right)),
        CoveLogicalType::Float32 => compare_float_range_key(
            cove_core::types::numcode_as_f32(left) as f64,
            cove_core::types::numcode_as_f32(right) as f64,
        )?,
        CoveLogicalType::Float64 => compare_float_range_key(
            cove_core::types::numcode_as_f64(left),
            cove_core::types::numcode_as_f64(right),
        )?,
        _ => return Err(CoveError::BadCovi),
    };
    Ok(ordering)
}

fn compare_float_range_key(left: f64, right: f64) -> Result<Ordering, CoveError> {
    if left.is_nan() || right.is_nan() {
        return Err(CoveError::BadCovi);
    }
    left.partial_cmp(&right).ok_or(CoveError::BadCovi)
}

#[derive(Debug, Clone, PartialEq)]
enum CanonicalComparable<'a> {
    Null,
    Bool(bool),
    Signed(i128),
    Unsigned(u128),
    Float(f64),
    Bytes(&'a [u8]),
}

fn compare_canonical_ordering_key(left: &[u8], right: &[u8]) -> Result<Ordering, CoveError> {
    let (left_tag, left_value) = parse_canonical_comparable(left)?;
    let (right_tag, right_value) = parse_canonical_comparable(right)?;
    if left_tag != right_tag {
        return Ok((left_tag as u16).cmp(&(right_tag as u16)));
    }
    let ordering = match (left_value, right_value) {
        (CanonicalComparable::Null, CanonicalComparable::Null) => Ordering::Equal,
        (CanonicalComparable::Bool(left), CanonicalComparable::Bool(right)) => left.cmp(&right),
        (CanonicalComparable::Signed(left), CanonicalComparable::Signed(right)) => left.cmp(&right),
        (CanonicalComparable::Unsigned(left), CanonicalComparable::Unsigned(right)) => {
            left.cmp(&right)
        }
        (CanonicalComparable::Float(left), CanonicalComparable::Float(right)) => {
            compare_float_range_key(left, right)?
        }
        (CanonicalComparable::Bytes(left), CanonicalComparable::Bytes(right)) => left.cmp(right),
        _ => return Err(CoveError::BadCovi),
    };
    Ok(ordering)
}

fn validate_canonical_key(key: &[u8]) -> Result<(), CoveError> {
    let (tag, payload) = split_canonical_key(key)?;
    validate_canonical_payload(tag, payload).map_err(|_| CoveError::BadCovi)
}

fn parse_canonical_comparable(
    key: &[u8],
) -> Result<(ValueTag, CanonicalComparable<'_>), CoveError> {
    let (tag, payload) = split_canonical_key(key)?;
    validate_canonical_payload(tag, payload).map_err(|_| CoveError::BadCovi)?;
    let value = match tag {
        ValueTag::Null => CanonicalComparable::Null,
        ValueTag::BoolFalse => CanonicalComparable::Bool(false),
        ValueTag::BoolTrue => CanonicalComparable::Bool(true),
        ValueTag::Int64
        | ValueTag::Decimal64
        | ValueTag::TimestampMicros
        | ValueTag::TimestampNanos => {
            CanonicalComparable::Signed(i64::from_le_bytes(fixed_payload(payload)?) as i128)
        }
        ValueTag::UInt64 => {
            CanonicalComparable::Unsigned(u64::from_le_bytes(fixed_payload(payload)?) as u128)
        }
        ValueTag::Float32Bits => {
            let value = f32::from_bits(u32::from_le_bytes(fixed_payload(payload)?)) as f64;
            if value.is_nan() {
                return Err(CoveError::BadCovi);
            }
            CanonicalComparable::Float(value)
        }
        ValueTag::Float64Bits => {
            let value = f64::from_bits(u64::from_le_bytes(fixed_payload(payload)?));
            if value.is_nan() {
                return Err(CoveError::BadCovi);
            }
            CanonicalComparable::Float(value)
        }
        ValueTag::Decimal128 => {
            CanonicalComparable::Signed(i128::from_le_bytes(fixed_payload(payload)?))
        }
        ValueTag::DateDays => {
            CanonicalComparable::Signed(i32::from_le_bytes(fixed_payload(payload)?) as i128)
        }
        ValueTag::Utf8 | ValueTag::Binary | ValueTag::Uuid | ValueTag::Json => {
            CanonicalComparable::Bytes(payload)
        }
        ValueTag::List | ValueTag::Struct | ValueTag::Map => return Err(CoveError::BadCovi),
        _ => return Err(CoveError::BadCovi),
    };
    Ok((tag, value))
}

fn split_canonical_key(key: &[u8]) -> Result<(ValueTag, &[u8]), CoveError> {
    let (tag, tag_len) = wire::decode_u64_leb128(key).map_err(|_| CoveError::BadCovi)?;
    let tag = u16::try_from(tag).map_err(|_| CoveError::BadCovi)?;
    let tag = ValueTag::from_u16(tag).ok_or(CoveError::BadCovi)?;
    Ok((tag, &key[tag_len..]))
}

fn fixed_payload<const N: usize>(payload: &[u8]) -> Result<[u8; N], CoveError> {
    payload.try_into().map_err(|_| CoveError::BadCovi)
}

fn compare_domain_rank_key(
    comparator_context: &CoviLookupComparatorContextV2,
    left: &[u8],
    right: &[u8],
) -> Result<Ordering, CoveError> {
    let left_rank = domain_rank_for_key(comparator_context, left)?;
    let right_rank = domain_rank_for_key(comparator_context, right)?;
    Ok(left_rank.cmp(&right_rank))
}

fn domain_rank_for_key(
    comparator_context: &CoviLookupComparatorContextV2,
    key: &[u8],
) -> Result<u32, CoveError> {
    if key.len() != 4 {
        return Err(CoveError::BadCovi);
    }
    let context = comparator_context.domain_rank.as_ref().ok_or_else(|| {
        CoveError::UnsupportedEncoding("COVE-I DomainRankOrdering requires rank context".into())
    })?;
    let file_code = u32::from_le_bytes(key.try_into().unwrap());
    let rank = *context
        .file_code_to_rank
        .get(file_code as usize)
        .ok_or(CoveError::BadCovi)?;
    if rank == INVALID_RANK {
        return Err(CoveError::BadCovi);
    }
    Ok(rank)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedIntervalKey<'a> {
    lower: Option<&'a [u8]>,
    upper: Option<&'a [u8]>,
    lower_inclusive: bool,
    upper_inclusive: bool,
}

fn interval_key_overlaps_request(
    root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    key: &[u8],
    requested: &[u8],
) -> Result<bool, CoveError> {
    interval_keys_overlap(root, request, key, requested)
}

fn interval_keys_overlap(
    _root: &CoviIndexRootV2,
    request: &CoviLookupRequestV2,
    left: &[u8],
    right: &[u8],
) -> Result<bool, CoveError> {
    if request.comparator_context.interval_encoding
        != Some(CoviIntervalEncodingV2::CanonicalBoundsV1)
    {
        return Err(CoveError::UnsupportedEncoding(
            "COVE-I IntervalOverlap requires interval encoding context".into(),
        ));
    }
    let left = parse_interval_key(left)?;
    let right = parse_interval_key(right)?;
    intervals_overlap(&left, &right)
}

fn parse_interval_key(bytes: &[u8]) -> Result<ParsedIntervalKey<'_>, CoveError> {
    const UNBOUNDED: u32 = u32::MAX;

    let mut offset = 0usize;
    let lower_len = read_interval_len(bytes, &mut offset)?;
    let lower = if lower_len == UNBOUNDED {
        None
    } else {
        Some(read_interval_bound(bytes, &mut offset, lower_len)?)
    };
    let upper_len = read_interval_len(bytes, &mut offset)?;
    let upper = if upper_len == UNBOUNDED {
        None
    } else {
        Some(read_interval_bound(bytes, &mut offset, upper_len)?)
    };
    let flags = *bytes.get(offset).ok_or(CoveError::BufferTooShort)?;
    offset = offset.checked_add(1).ok_or(CoveError::ArithOverflow)?;
    if offset != bytes.len() || flags & !0b11 != 0 {
        return Err(CoveError::BadCovi);
    }
    if let (Some(lower), Some(upper)) = (lower, upper) {
        match compare_canonical_ordering_key(lower, upper)? {
            Ordering::Greater => return Err(CoveError::BadCovi),
            Ordering::Equal if flags & 0b11 != 0b11 => return Err(CoveError::BadCovi),
            _ => {}
        }
    }
    Ok(ParsedIntervalKey {
        lower,
        upper,
        lower_inclusive: flags & 1 != 0,
        upper_inclusive: flags & (1 << 1) != 0,
    })
}

fn read_interval_len(bytes: &[u8], offset: &mut usize) -> Result<u32, CoveError> {
    let end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let value = u32::from_le_bytes(bytes[*offset..end].try_into().unwrap());
    *offset = end;
    Ok(value)
}

fn read_interval_bound<'a>(
    bytes: &'a [u8],
    offset: &mut usize,
    len: u32,
) -> Result<&'a [u8], CoveError> {
    let len = usize::try_from(len).map_err(|_| CoveError::OffsetRange)?;
    let end = offset.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let bound = &bytes[*offset..end];
    validate_canonical_key(bound)?;
    *offset = end;
    Ok(bound)
}

fn intervals_overlap(
    left: &ParsedIntervalKey<'_>,
    right: &ParsedIntervalKey<'_>,
) -> Result<bool, CoveError> {
    if let (Some(left_upper), Some(right_lower)) = (left.upper, right.lower) {
        match compare_canonical_ordering_key(left_upper, right_lower)? {
            Ordering::Less => return Ok(false),
            Ordering::Equal if !(left.upper_inclusive && right.lower_inclusive) => {
                return Ok(false)
            }
            _ => {}
        }
    }
    if let (Some(right_upper), Some(left_lower)) = (right.upper, left.lower) {
        match compare_canonical_ordering_key(right_upper, left_lower)? {
            Ordering::Less => return Ok(false),
            Ordering::Equal if !(right.upper_inclusive && left.lower_inclusive) => {
                return Ok(false)
            }
            _ => {}
        }
    }
    Ok(true)
}

fn normalize_row_ranges(rows: &mut Vec<CoviRowRangePostingV2>) -> Result<(), CoveError> {
    rows.sort_by_key(|row| {
        (
            row.file_ref,
            row.table_id,
            row.segment_id,
            row.morsel_id,
            row.row_start,
        )
    });
    let mut out: Vec<CoviRowRangePostingV2> = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        if row.row_count == 0 {
            return Err(CoveError::BadCovi);
        }
        if let Some(last) = out.last_mut() {
            let same_scope = last.file_ref == row.file_ref
                && last.table_id == row.table_id
                && last.segment_id == row.segment_id
                && last.morsel_id == row.morsel_id;
            let last_end = last
                .row_start
                .checked_add(last.row_count)
                .ok_or(CoveError::ArithOverflow)?;
            if same_scope && row.row_start <= last_end {
                let row_end = row
                    .row_start
                    .checked_add(row.row_count)
                    .ok_or(CoveError::ArithOverflow)?;
                last.row_count = row_end
                    .checked_sub(last.row_start)
                    .ok_or(CoveError::ArithOverflow)?;
                continue;
            }
        }
        out.push(row);
    }
    *rows = out;
    Ok(())
}

fn parse_fixed_payload<T>(
    payload: &[u8],
    width: usize,
    parse: impl Fn(&[u8]) -> Result<T, CoveError>,
) -> Result<Vec<T>, CoveError> {
    if !payload.len().is_multiple_of(width) {
        return Err(CoveError::BadCovi);
    }
    payload.chunks_exact(width).map(parse).collect()
}

fn parse_u32_refs(payload: &[u8]) -> Result<Vec<u32>, CoveError> {
    if !payload.len().is_multiple_of(4) {
        return Err(CoveError::BadCovi);
    }
    Ok(payload
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn aggregate_sum_answer_value_tag(logical: CoveLogicalType) -> Option<ValueTag> {
    match logical {
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => Some(ValueTag::Int64),
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => Some(ValueTag::UInt64),
        CoveLogicalType::Float32 => Some(ValueTag::Float32Bits),
        CoveLogicalType::Float64 => Some(ValueTag::Float64Bits),
        CoveLogicalType::Decimal64 => Some(ValueTag::Decimal64),
        CoveLogicalType::Decimal128 => Some(ValueTag::Decimal128),
        _ => None,
    }
}

fn aggregate_value_payload<'a>(
    answer: &CoviAggregateAnswerV2,
    block: &'a CoviAggregateAnswerBlockV2,
) -> Result<Option<&'a [u8]>, CoveError> {
    if answer.value_ref == ABSENT_U32 {
        return Ok(None);
    }
    let start = usize::try_from(answer.value_ref).map_err(|_| CoveError::OffsetRange)?;
    if start > block.payload.len() {
        return Err(CoveError::OffsetRange);
    }
    let end = block
        .answers
        .iter()
        .filter_map(|candidate| {
            (candidate.value_ref != ABSENT_U32)
                .then_some(candidate.value_ref as usize)
                .filter(|offset| *offset > start)
        })
        .min()
        .unwrap_or(block.payload.len());
    Ok(Some(&block.payload[start..end]))
}

fn answer_to_public(
    answer: &CoviAggregateAnswerV2,
    block: &CoviAggregateAnswerBlockV2,
) -> Result<CoviIndexOnlyAnswerV2, CoveError> {
    let value = aggregate_value_payload(answer, block)?.map(<[u8]>::to_vec);
    Ok(CoviIndexOnlyAnswerV2 {
        aggregate_kind: CoviAggregateKindV2::from_u16(answer.aggregate_kind)
            .ok_or(CoveError::BadCovi)?,
        row_count: answer.row_count,
        null_count: answer.null_count,
        non_null_count: answer.non_null_count,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CoviAggregateAnswerBlockHeaderV2, CoviEntryBlockHeaderV2, CoviHeaderV2,
        CoviKeyBlockHeaderV2, CoviPostingsBlockHeaderV2, CoviPostingsHeaderV2, CoviPostscriptV2,
        COVI_HEADER_LEN,
    };

    fn root_with(
        logical_type: CoveLogicalType,
        key_encoding_kind: CoviKeyEncodingKindV2,
        comparator_kind: CoviComparatorKindV2,
    ) -> CoviIndexRootV2 {
        CoviIndexRootV2 {
            index_root_id: 1,
            indexed_target_kind: CoviIndexedTargetKindV2::TableColumn,
            index_kind: CoviIndexKindV2::Sorted,
            coverage_granularity: 0,
            proof_strength: 0,
            exactness: 0,
            flags: 0,
            table_id: 1,
            column_id: 2,
            object_type_id: ABSENT_U32,
            property_id: ABSENT_U32,
            path_ref: ABSENT_U32,
            semantic_dimension_ref: ABSENT_U32,
            logical_type: logical_type as u16,
            physical_kind: 0,
            key_encoding_kind: key_encoding_kind as u8,
            comparator_kind: comparator_kind as u16,
            collation_id: 0,
            null_semantics: 0,
            sort_order: 0,
            value_count: 0,
            distinct_count: 0,
            null_count: 0,
            min_key_ref: ABSENT_U32,
            max_key_ref: ABSENT_U32,
            key_block_section_id: ABSENT_U32,
            entry_block_section_id: ABSENT_U32,
            postings_block_section_id: ABSENT_U32,
            aggregate_block_section_id: ABSENT_U32,
            coverage_set_ref: ABSENT_U32,
            capability_ref: ABSENT_U32,
            snapshot_validity_ref: ABSENT_U32,
            checksum: 0,
        }
    }

    fn tagged_key(tag: ValueTag, payload: impl AsRef<[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        wire::append_u64_leb128(&mut out, tag as u64);
        out.extend_from_slice(payload.as_ref());
        out
    }

    fn canonical_i64(value: i64) -> Vec<u8> {
        tagged_key(ValueTag::Int64, value.to_le_bytes())
    }

    fn empty_postings_block(root_id: u32) -> CoviPostingsBlockV2 {
        CoviPostingsBlockV2 {
            header: CoviPostingsBlockHeaderV2 {
                magic: CoviPostingsBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviPostingsBlockHeaderV2::LEN as u16,
                postings_header_len: CoviPostingsHeaderV2::LEN as u16,
                postings_block_id: root_id,
                index_root_id: root_id,
                postings_count: 0,
                row_ordinal_set_count: 0,
                postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
                row_ordinal_headers_offset: 0,
                postings_payload_offset: 0,
                postings_payload_length: 0,
                flags: 0,
                checksum: 0,
            },
            postings: Vec::new(),
            row_ordinal_sets: Vec::new(),
            payload: Vec::new(),
        }
    }

    fn entry_blocks_for_keys(
        root: &CoviIndexRootV2,
        keys: Vec<Vec<u8>>,
    ) -> (CoviKeyBlockV2, CoviEntryBlockV2, CoviPostingsBlockV2) {
        let mut key_data = Vec::new();
        let mut entries = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            let entry_ref = index as u32;
            let key_offset = key_data.len() as u64;
            key_data.extend_from_slice(key);
            entries.push(CoviIndexEntryV2 {
                entry_ref,
                index_root_id: root.index_root_id,
                entry_id: u64::from(entry_ref),
                key_kind: CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind).unwrap(),
                comparator_kind: CoviComparatorKindV2::from_u16(root.comparator_kind).unwrap(),
                flags: 0,
                key_offset,
                key_length: key.len() as u32,
                key_hash64: 0,
                postings_ref: ABSENT_U32,
                coverage_set_ref: ABSENT_U32,
                aggregate_answer_ref: ABSENT_U32,
                next_duplicate_ref: ABSENT_U32,
                checksum: 0,
            });
        }
        let key_block = CoviKeyBlockV2 {
            header: CoviKeyBlockHeaderV2 {
                magic: CoviKeyBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviKeyBlockHeaderV2::LEN as u16,
                reserved0: 0,
                key_block_id: root.key_block_section_id,
                index_root_id: root.index_root_id,
                key_count: entries.len() as u64,
                encoding_kind: CoviKeyEncodingKindV2::from_u8(root.key_encoding_kind).unwrap(),
                comparator_kind: CoviComparatorKindV2::from_u16(root.comparator_kind).unwrap(),
                flags: 0,
                key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
                key_data_length: key_data.len() as u64,
                checksum: 0,
            },
            key_data,
        };
        let entry_block = CoviEntryBlockV2 {
            header: CoviEntryBlockHeaderV2 {
                magic: CoviEntryBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviEntryBlockHeaderV2::LEN as u16,
                entry_len: CoviIndexEntryV2::LEN as u16,
                entry_block_id: root.entry_block_section_id,
                index_root_id: root.index_root_id,
                entry_count: entries.len() as u32,
                key_block_id: root.key_block_section_id,
                postings_block_id: root.postings_block_section_id,
                aggregate_block_id: ABSENT_U32,
                entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
                entries_length: (entries.len() * CoviIndexEntryV2::LEN) as u64,
                flags: 0,
                checksum: 0,
            },
            entries,
        };
        (
            key_block,
            entry_block,
            empty_postings_block(root.index_root_id),
        )
    }

    fn capability(
        root_id: u32,
        supports_eq: u8,
        supports_range: u8,
        supports_membership: u8,
        exactness: IndexCapabilityExactnessV2,
    ) -> IndexCapabilityV2 {
        IndexCapabilityV2 {
            capability_id: root_id,
            index_root_id: root_id,
            flags: 0,
            supports_eq,
            supports_range,
            supports_membership,
            supports_prefix: 0,
            supports_contains: 0,
            supports_count: 0,
            supports_min: 0,
            supports_max: 0,
            supports_sum: 0,
            supports_distinct_count: 0,
            supports_join_coverage: 0,
            supports_index_only: 0,
            exactness,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            null_semantics: 0,
            reserved: 0,
            snapshot_validity_ref: 0,
            coverage_provider_ref: ABSENT_U32,
            checksum: 0,
        }
    }

    fn aggregate_count_block(
        root_id: u32,
        block_id: u32,
        exactness: IndexCapabilityExactnessV2,
        row_count: u64,
    ) -> CoviAggregateAnswerBlockV2 {
        CoviAggregateAnswerBlockV2 {
            header: CoviAggregateAnswerBlockHeaderV2 {
                magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
                aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
                aggregate_block_id: block_id,
                index_root_id: root_id,
                aggregate_answer_count: 1,
                aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
                aggregate_payload_offset: 0,
                aggregate_payload_length: 0,
                flags: 0,
                checksum: 0,
            },
            answers: vec![CoviAggregateAnswerV2 {
                aggregate_answer_ref: 0,
                index_root_id: root_id,
                aggregate_kind: CoviAggregateKindV2::Count as u16,
                exactness: exactness as u8,
                null_semantics: 0,
                flags: 0,
                row_count,
                null_count: 0,
                non_null_count: row_count,
                value_ref: ABSENT_U32,
                predicate_form_ref: ABSENT_U32,
                snapshot_validity_ref: 0,
                checksum: 0,
            }],
            payload: Vec::new(),
        }
    }

    fn aggregate_value_block(
        root_id: u32,
        aggregate_kind: CoviAggregateKindV2,
        row_count: u64,
        null_count: u64,
        non_null_count: u64,
        payload: Option<Vec<u8>>,
    ) -> CoviAggregateAnswerBlockV2 {
        let payload_len = payload.as_ref().map_or(0, Vec::len);
        CoviAggregateAnswerBlockV2 {
            header: CoviAggregateAnswerBlockHeaderV2 {
                magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
                aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
                aggregate_block_id: 13,
                index_root_id: root_id,
                aggregate_answer_count: 1,
                aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
                aggregate_payload_offset: 0,
                aggregate_payload_length: payload_len as u64,
                flags: 0,
                checksum: 0,
            },
            answers: vec![CoviAggregateAnswerV2 {
                aggregate_answer_ref: 0,
                index_root_id: root_id,
                aggregate_kind: aggregate_kind as u16,
                exactness: IndexCapabilityExactnessV2::Exact as u8,
                null_semantics: 0,
                flags: 0,
                row_count,
                null_count,
                non_null_count,
                value_ref: if payload.is_some() { 0 } else { ABSENT_U32 },
                predicate_form_ref: ABSENT_U32,
                snapshot_validity_ref: 0,
                checksum: 0,
            }],
            payload: payload.unwrap_or_default(),
        }
    }

    fn row_range_postings_block(
        root_id: u32,
        block_id: u32,
        row_start: u64,
        row_count: u64,
    ) -> CoviPostingsBlockV2 {
        let row = CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 0,
            morsel_id: 0,
            row_start,
            row_count,
            flags: 0,
            checksum: 0,
        }
        .serialize()
        .unwrap();
        CoviPostingsBlockV2 {
            header: CoviPostingsBlockHeaderV2 {
                magic: CoviPostingsBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviPostingsBlockHeaderV2::LEN as u16,
                postings_header_len: CoviPostingsHeaderV2::LEN as u16,
                postings_block_id: block_id,
                index_root_id: root_id,
                postings_count: 1,
                row_ordinal_set_count: 0,
                postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
                row_ordinal_headers_offset: 0,
                postings_payload_offset: 0,
                postings_payload_length: row.len() as u64,
                flags: 0,
                checksum: 0,
            },
            postings: vec![CoviPostingsHeaderV2 {
                postings_ref: 0,
                index_root_id: root_id,
                representation: CoviPostingRepresentationV2::RowRangeList,
                target_granularity: cove_coverage::CoverageGranularityV2::Morsel as u8,
                flags: 0,
                item_count: 1,
                payload_offset: 0,
                payload_length: row.len() as u64,
                coverage_set_ref: ABSENT_U32,
                checksum: 0,
            }],
            row_ordinal_sets: Vec::new(),
            payload: row.to_vec(),
        }
    }

    fn blank_artifact() -> CoviArtifactV2 {
        CoviArtifactV2 {
            postscript: CoviPostscriptV2 {
                required_features: 0,
                optional_features: 0,
                file_len: 0,
                header_offset: 0,
                header_length: COVI_HEADER_LEN as u64,
                checksum: 0,
            },
            header: CoviHeaderV2 {
                magic: *b"COVI",
                header_len: COVI_HEADER_LEN,
                version_major: 2,
                version_minor: 0,
                flags: 0,
                index_artifact_id: [0; 16],
                dataset_id: [0; 16],
                snapshot_id: [0; 16],
                section_count: 0,
                referenced_file_count: 0,
                snapshot_validity_count: 0,
                index_root_count: 0,
                capability_count: 0,
                section_directory_offset: 0,
                section_directory_length: 0,
                referenced_files_offset: 0,
                snapshot_validity_offset: 0,
                index_roots_offset: 0,
                capabilities_offset: 0,
                string_table_section_ref: ABSENT_U32,
                created_at_us: 0,
                reserved: [0; 24],
                checksum: 0,
            },
            sections: Vec::new(),
            referenced_files: Vec::new(),
            snapshot_validity: Vec::new(),
            index_roots: Vec::new(),
            capabilities: Vec::new(),
            index_only_capabilities: Vec::new(),
            key_blocks: Vec::new(),
            entry_blocks: Vec::new(),
            postings_blocks: Vec::new(),
            aggregate_answer_blocks: Vec::new(),
        }
    }

    fn validated_with_roots(
        roots: Vec<CoviIndexRootV2>,
        capabilities: Vec<IndexCapabilityV2>,
    ) -> ValidatedCoviArtifactV2 {
        let mut key_blocks = std::collections::BTreeMap::new();
        let mut entry_blocks = std::collections::BTreeMap::new();
        let mut postings_blocks = std::collections::BTreeMap::new();
        for root in &roots {
            let (key_block, entry_block, postings_block) = entry_blocks_for_keys(root, Vec::new());
            key_blocks.insert(root.key_block_section_id, key_block);
            entry_blocks.insert(root.entry_block_section_id, entry_block);
            postings_blocks.insert(root.postings_block_section_id, postings_block);
        }
        let index_only_capabilities = test_index_only_capabilities(&capabilities);
        ValidatedCoviArtifactV2 {
            artifact: blank_artifact(),
            host_file_ref: 0,
            roots: roots
                .into_iter()
                .map(|root| (root.index_root_id, root))
                .collect(),
            capabilities: capabilities
                .into_iter()
                .map(|capability| (capability.index_root_id, capability))
                .collect(),
            index_only_capabilities,
            snapshot_validity: std::collections::BTreeMap::new(),
            active_visibility_overlay_ref: None,
            key_blocks,
            entry_blocks,
            postings_blocks,
            aggregate_blocks: std::collections::BTreeMap::new(),
        }
    }

    fn test_index_only_capabilities(
        capabilities: &[IndexCapabilityV2],
    ) -> std::collections::BTreeMap<(u32, u16), IndexOnlyCapabilityV2> {
        let mut index_only_capabilities = std::collections::BTreeMap::new();
        for capability in capabilities {
            if capability.supports_index_only == 0 {
                continue;
            }
            for (enabled, aggregate_kind) in [
                (capability.supports_count, CoviAggregateKindV2::Count),
                (capability.supports_count, CoviAggregateKindV2::Exists),
                (capability.supports_min, CoviAggregateKindV2::Min),
                (capability.supports_max, CoviAggregateKindV2::Max),
                (
                    capability.supports_distinct_count,
                    CoviAggregateKindV2::DistinctCount,
                ),
                (capability.supports_sum, CoviAggregateKindV2::Sum),
                (capability.supports_sum, CoviAggregateKindV2::Avg),
            ] {
                if enabled == 0 {
                    continue;
                }
                let index_only = IndexOnlyCapabilityV2 {
                    capability_id: capability.capability_id,
                    aggregate_kind: aggregate_kind as u16,
                    predicate_supported: 0,
                    exactness: capability.exactness,
                    null_semantics: capability.null_semantics,
                    flags: 0,
                    snapshot_validity_ref: capability.snapshot_validity_ref,
                    required_visibility_overlay_ref: ABSENT_U32,
                    checksum: 0,
                };
                index_only_capabilities.insert(
                    (index_only.capability_id, index_only.aggregate_kind),
                    index_only,
                );
            }
        }
        index_only_capabilities
    }

    fn snapshot_with_external_visibility(external_visibility_ref: u32) -> CoviSnapshotValidityV2 {
        CoviSnapshotValidityV2 {
            snapshot_validity_ref: 0,
            dataset_id: [1; 16],
            snapshot_id: [2; 16],
            schema_fingerprint_ref: ABSENT_U32,
            semantic_map_fingerprint_ref: ABSENT_U32,
            external_visibility_ref,
            data_checksum_root_ref: ABSENT_U32,
            delta_chain_digest_algorithm: DigestAlgorithm::None as u16,
            delta_chain_digest_len: 0,
            delta_chain_digest_offset: 0,
            valid_from_us: 0,
            valid_until_us: 100,
            flags: 0,
            checksum: 0,
        }
    }

    fn referenced_file_with_digest_algorithm(
        digest_algorithm: DigestAlgorithm,
    ) -> CoviReferencedFileV2 {
        CoviReferencedFileV2 {
            file_ref: 0,
            flags: 0,
            file_id: [3; 16],
            file_len: 64,
            footer_crc32c: 0x1234,
            digest_algorithm: digest_algorithm as u16,
            digest_len: if digest_algorithm == DigestAlgorithm::None {
                0
            } else {
                32
            },
            digest_offset: 0,
            uri_ref: ABSENT_U32,
            schema_fingerprint_ref: ABSENT_U32,
            checksum: 0,
        }
    }

    #[test]
    fn membership_request_preserves_typed_target_and_keys() {
        let request = CoviLookupRequestV2::object_property_membership(
            7,
            9,
            [
                CoviLookupKeyV2::CanonicalValueBytes(b"alpha".to_vec()),
                CoviLookupKeyV2::CanonicalValueBytes(b"beta".to_vec()),
            ],
        );
        assert_eq!(
            request.target,
            CoviLookupTargetV2::ObjectProperty {
                object_type_id: 7,
                property_id: 9
            }
        );
        assert_eq!(request.op, CoviLookupOpV2::Membership);
        assert_eq!(
            membership_key_bytes(&request),
            vec![b"alpha".to_vec(), b"beta".to_vec()]
        );
    }

    #[test]
    fn membership_key_match_checks_any_requested_key() {
        let request = CoviLookupRequestV2::membership(
            1,
            2,
            [
                CoviLookupKeyV2::CanonicalValueBytes(b"a".to_vec()),
                CoviLookupKeyV2::CanonicalValueBytes(b"c".to_vec()),
            ],
        );
        let keys = membership_key_bytes(&request);
        let root = CoviIndexRootV2 {
            index_root_id: 1,
            indexed_target_kind: CoviIndexedTargetKindV2::TableColumn,
            index_kind: CoviIndexKindV2::Hash,
            coverage_granularity: 0,
            proof_strength: 0,
            exactness: 0,
            flags: 0,
            table_id: 1,
            column_id: 2,
            object_type_id: ABSENT_U32,
            property_id: ABSENT_U32,
            path_ref: ABSENT_U32,
            semantic_dimension_ref: ABSENT_U32,
            logical_type: CoveLogicalType::Utf8 as u16,
            physical_kind: 0,
            key_encoding_kind: CoviKeyEncodingKindV2::Utf8BytewisePrefix as u8,
            comparator_kind: CoviComparatorKindV2::Utf8BytewisePrefix as u16,
            collation_id: 0,
            null_semantics: 0,
            sort_order: 0,
            value_count: 0,
            distinct_count: 0,
            null_count: 0,
            min_key_ref: ABSENT_U32,
            max_key_ref: ABSENT_U32,
            key_block_section_id: ABSENT_U32,
            entry_block_section_id: ABSENT_U32,
            postings_block_section_id: ABSENT_U32,
            aggregate_block_section_id: ABSENT_U32,
            coverage_set_ref: ABSENT_U32,
            capability_ref: ABSENT_U32,
            snapshot_validity_ref: ABSENT_U32,
            checksum: 0,
        };
        assert!(key_matches(&root, &request, b"c", b"a", None, &keys).unwrap());
        assert!(!key_matches(&root, &request, b"b", b"a", None, &keys).unwrap());
    }

    #[test]
    fn fixed_and_tuple_key_encodings_use_raw_lexicographic_matching() {
        let fixed = root_with(
            CoveLogicalType::Binary,
            CoviKeyEncodingKindV2::FixedBytes,
            CoviComparatorKindV2::CanonicalEquality,
        );
        let fixed_request = CoviLookupRequestV2 {
            op: CoviLookupOpV2::Range {
                lower_inclusive: true,
                upper_inclusive: false,
            },
            lower_key: CoviLookupKeyV2::FixedBytes(b"ab".to_vec()),
            upper_key: Some(CoviLookupKeyV2::FixedBytes(b"ad".to_vec())),
            ..CoviLookupRequestV2::eq(1, 2, CoviLookupKeyV2::FixedBytes(b"ab".to_vec()))
        };
        let upper = fixed_request
            .upper_key
            .as_ref()
            .map(CoviLookupKeyV2::key_bytes);
        assert!(key_matches(&fixed, &fixed_request, b"ac", b"ab", upper.as_deref(), &[]).unwrap());

        let dimensional = root_with(
            CoveLogicalType::Binary,
            CoviKeyEncodingKindV2::DimensionalTuple,
            CoviComparatorKindV2::DimensionalTupleLexicographic,
        );
        let dim_request =
            CoviLookupRequestV2::eq(1, 2, CoviLookupKeyV2::DimensionalTuple(b"a\0b".to_vec()));
        assert!(key_matches(&dimensional, &dim_request, b"a\0b", b"a\0b", None, &[]).unwrap());

        let object_path = root_with(
            CoveLogicalType::Binary,
            CoviKeyEncodingKindV2::ObjectPathTuple,
            CoviComparatorKindV2::ObjectPathLexicographic,
        );
        let path_request =
            CoviLookupRequestV2::eq(1, 2, CoviLookupKeyV2::ObjectPathTuple(b"/a/b".to_vec()));
        assert!(key_matches(&object_path, &path_request, b"/a/b", b"/a/b", None, &[]).unwrap());
    }

    #[test]
    fn utf8_prefix_requests_require_prefix_capability_and_valid_utf8() {
        let mut cap = capability(1, 1, 0, 1, IndexCapabilityExactnessV2::Exact);
        let request =
            CoviLookupRequestV2::prefix(1, 2, CoviLookupKeyV2::Utf8BytewisePrefix(b"al".to_vec()));
        assert!(!lookup_capability_supports_request(&cap, &request));
        cap.supports_prefix = 1;
        assert!(lookup_capability_supports_request(&cap, &request));

        let root = root_with(
            CoveLogicalType::Utf8,
            CoviKeyEncodingKindV2::Utf8BytewisePrefix,
            CoviComparatorKindV2::Utf8BytewisePrefix,
        );
        assert!(key_matches(&root, &request, b"alpha", b"al", None, &[]).unwrap());
        assert!(!key_matches(&root, &request, b"beta", b"al", None, &[]).unwrap());
        assert!(matches!(
            key_matches(&root, &request, b"\xff", b"al", None, &[]),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn exact_lookup_requires_exact_proof_strength() {
        let mut cap = capability(1, 1, 0, 1, IndexCapabilityExactnessV2::Exact);
        let request = CoviLookupRequestV2::eq(
            1,
            2,
            CoviLookupKeyV2::CanonicalValueBytes(canonical_i64(42)),
        );

        cap.proof_strength = CoverageProofStrengthV2::ExactTight;
        assert!(lookup_capability_supports_request(&cap, &request));
        cap.proof_strength = CoverageProofStrengthV2::ExactConservative;
        assert!(lookup_capability_supports_request(&cap, &request));

        for proof_strength in [
            CoverageProofStrengthV2::ProbabilisticConservative,
            CoverageProofStrengthV2::AdvisoryOnly,
            CoverageProofStrengthV2::EngineLocal,
            CoverageProofStrengthV2::ApproximateMayUnderInclude,
        ] {
            cap.proof_strength = proof_strength;
            assert!(!lookup_capability_supports_request(&cap, &request));
        }

        let advisory_request = CoviLookupRequestV2 {
            require_exact: false,
            ..request
        };
        cap.proof_strength = CoverageProofStrengthV2::AdvisoryOnly;
        assert!(lookup_capability_supports_request(&cap, &advisory_request));
    }

    #[test]
    fn canonical_hash_encodings_verify_canonical_bytes_and_hash_payloads() {
        let canonical = canonical_i64(42);
        let hash64_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalHash64,
            CoviComparatorKindV2::CanonicalEquality,
        );
        let hash64_request = CoviLookupRequestV2::eq(
            1,
            2,
            CoviLookupKeyV2::CanonicalHash {
                hash64: 7,
                canonical_value_bytes: canonical.clone(),
            },
        );
        let mut entry = CoviIndexEntryV2 {
            entry_ref: 0,
            index_root_id: hash64_root.index_root_id,
            entry_id: 0,
            key_kind: CoviKeyEncodingKindV2::CanonicalHash64,
            comparator_kind: CoviComparatorKindV2::CanonicalEquality,
            flags: 0,
            key_offset: 0,
            key_length: canonical.len() as u32,
            key_hash64: 7,
            postings_ref: 0,
            coverage_set_ref: ABSENT_U32,
            aggregate_answer_ref: ABSENT_U32,
            next_duplicate_ref: ABSENT_U32,
            checksum: 0,
        };
        assert!(
            entry_hash64_may_match_request(&hash64_root, &hash64_request, &entry, &canonical)
                .unwrap()
        );
        entry.key_hash64 = 8;
        assert!(
            !entry_hash64_may_match_request(&hash64_root, &hash64_request, &entry, &canonical)
                .unwrap()
        );
        assert!(key_equals(&hash64_root, &hash64_request, &[], &canonical).is_err());

        let canonical_bytes_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalEquality,
        );
        assert!(entry_hash64_may_match_request(
            &canonical_bytes_root,
            &hash64_request,
            &entry,
            &canonical
        )
        .unwrap());

        let hash = [3u8; 16];
        let mut hash128_key = hash.to_vec();
        hash128_key.extend_from_slice(&canonical);
        let hash128_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalHash128,
            CoviComparatorKindV2::CanonicalEquality,
        );
        let hash128_request = CoviLookupRequestV2::eq(
            1,
            2,
            CoviLookupKeyV2::CanonicalHash128 {
                hash128: hash,
                canonical_value_bytes: canonical.clone(),
            },
        );
        assert!(key_matches(
            &hash128_root,
            &hash128_request,
            &hash128_key,
            &canonical,
            None,
            &[]
        )
        .unwrap());
        let mut wrong_hash_key = [4u8; 16].to_vec();
        wrong_hash_key.extend_from_slice(&canonical);
        assert!(!key_matches(
            &hash128_root,
            &hash128_request,
            &wrong_hash_key,
            &canonical,
            None,
            &[]
        )
        .unwrap());
        assert!(matches!(
            key_equals(&hash128_root, &hash128_request, &[0; 16], &canonical),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn canonical_value_bytes_lookup_treats_key_hash64_as_hint() {
        let canonical = canonical_i64(42);
        let mut root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalEquality,
        );
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        let mut artifact = validated_with_roots(
            vec![root.clone()],
            vec![capability(1, 1, 0, 1, IndexCapabilityExactnessV2::Exact)],
        );
        let (key_block, mut entry_block, _) = entry_blocks_for_keys(&root, vec![canonical.clone()]);
        entry_block.entries[0].key_hash64 = 0xdead_beef_dead_beef;
        entry_block.entries[0].postings_ref = 0;
        artifact
            .key_blocks
            .insert(root.key_block_section_id, key_block);
        artifact
            .entry_blocks
            .insert(root.entry_block_section_id, entry_block);
        artifact.postings_blocks.insert(
            root.postings_block_section_id,
            row_range_postings_block(root.index_root_id, root.postings_block_section_id, 9, 3),
        );

        let candidates = artifact
            .lookup(&CoviLookupRequestV2::eq(
                1,
                2,
                CoviLookupKeyV2::CanonicalHash {
                    hash64: 7,
                    canonical_value_bytes: canonical,
                },
            ))
            .unwrap();

        assert_eq!(candidates.row_ranges.len(), 1);
        assert_eq!(candidates.row_ranges[0].row_start, 9);
        assert_eq!(candidates.row_ranges[0].row_count, 3);
    }

    #[test]
    fn snapshot_validation_rejects_overlay_without_matching_context() {
        let snapshot = snapshot_with_external_visibility(7);
        let context = CoviValidationContextV2::for_file([3; 16], 64, 0x1234);
        assert!(matches!(
            validate_snapshot(&snapshot, &context, None),
            Err(CoveError::BadCovi)
        ));

        let matching_context = context.clone().with_external_visibility_ref(7);
        validate_snapshot(&snapshot, &matching_context, None).unwrap();

        let mismatched_context = context.with_external_visibility_ref(8);
        assert!(matches!(
            validate_snapshot(&snapshot, &mismatched_context, None),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn snapshot_validation_accepts_absent_overlay_without_context_overlay() {
        let snapshot = snapshot_with_external_visibility(ABSENT_U32);
        let context = CoviValidationContextV2::for_file([3; 16], 64, 0x1234);
        validate_snapshot(&snapshot, &context, None).unwrap();
    }

    #[test]
    fn snapshot_validation_binds_delta_chain_digest() {
        let mut snapshot = snapshot_with_external_visibility(ABSENT_U32);
        snapshot.delta_chain_digest_algorithm = DigestAlgorithm::Sha256 as u16;
        snapshot.delta_chain_digest_len = 32;
        snapshot.delta_chain_digest_offset = 4;
        let mut string_table = vec![0, 1, 2, 3];
        string_table.extend_from_slice(&[0xAA; 32]);
        let matching_context = CoviValidationContextV2::for_file([3; 16], 64, 0x1234)
            .with_delta_chain_digest(DigestAlgorithm::Sha256, vec![0xAA; 32]);
        validate_snapshot(&snapshot, &matching_context, Some(&string_table)).unwrap();

        let mismatched_context = CoviValidationContextV2::for_file([3; 16], 64, 0x1234)
            .with_delta_chain_digest(DigestAlgorithm::Sha256, vec![0xBB; 32]);
        assert!(matches!(
            validate_snapshot(&snapshot, &mismatched_context, Some(&string_table)),
            Err(CoveError::DigestMismatch)
        ));

        let base_only_context = CoviValidationContextV2::for_file([3; 16], 64, 0x1234);
        assert!(matches!(
            validate_snapshot(&snapshot, &base_only_context, None),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn row_range_scope_validation_rejects_out_of_bounds_candidate() {
        let row = CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 10,
            morsel_id: 3,
            row_start: 18,
            row_count: 4,
            flags: 0,
            checksum: 0,
        };
        let context =
            CoviValidationContextV2::for_file([3; 16], 64, 0x1234).with_row_range_scopes(vec![
                CoviRowRangeScopeV2 {
                    file_ref: 0,
                    table_id: 1,
                    segment_id: 10,
                    morsel_id: 3,
                    row_start: 10,
                    row_count: 10,
                },
            ]);
        assert!(matches!(
            validate_row_range_scope(&row, &context),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn row_range_scope_validation_accepts_bounded_candidate() {
        let row = CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 10,
            morsel_id: 3,
            row_start: 12,
            row_count: 4,
            flags: 0,
            checksum: 0,
        };
        let context =
            CoviValidationContextV2::for_file([3; 16], 64, 0x1234).with_row_range_scopes(vec![
                CoviRowRangeScopeV2 {
                    file_ref: 0,
                    table_id: 1,
                    segment_id: 10,
                    morsel_id: 3,
                    row_start: 10,
                    row_count: 10,
                },
            ]);
        validate_row_range_scope(&row, &context).unwrap();
    }

    #[test]
    fn aggregate_block_validation_rejects_inconsistent_count_answer() {
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let mut block = aggregate_count_block(
            root.index_root_id,
            13,
            IndexCapabilityExactnessV2::Exact,
            42,
        );
        block.answers[0].null_count = 2;
        block.answers[0].non_null_count = 41;
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        assert!(matches!(
            validate_aggregate_block(&root, &snapshots, &block),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn aggregate_block_validation_rejects_malformed_minmax_payload() {
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let block = CoviAggregateAnswerBlockV2 {
            header: CoviAggregateAnswerBlockHeaderV2 {
                magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
                aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
                aggregate_block_id: 13,
                index_root_id: root.index_root_id,
                aggregate_answer_count: 1,
                aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
                aggregate_payload_offset: 0,
                aggregate_payload_length: 2,
                flags: 0,
                checksum: 0,
            },
            answers: vec![CoviAggregateAnswerV2 {
                aggregate_answer_ref: 0,
                index_root_id: root.index_root_id,
                aggregate_kind: CoviAggregateKindV2::Min as u16,
                exactness: IndexCapabilityExactnessV2::Exact as u8,
                null_semantics: 0,
                flags: 0,
                row_count: 1,
                null_count: 0,
                non_null_count: 1,
                value_ref: 0,
                predicate_form_ref: ABSENT_U32,
                snapshot_validity_ref: 0,
                checksum: 0,
            }],
            payload: vec![1, 2],
        };
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        assert!(matches!(
            validate_aggregate_block(&root, &snapshots, &block),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn aggregate_block_validation_accepts_distinct_count_payload() {
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let block = CoviAggregateAnswerBlockV2 {
            header: CoviAggregateAnswerBlockHeaderV2 {
                magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
                aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
                aggregate_block_id: 13,
                index_root_id: root.index_root_id,
                aggregate_answer_count: 1,
                aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
                aggregate_payload_offset: 0,
                aggregate_payload_length: 8,
                flags: 0,
                checksum: 0,
            },
            answers: vec![CoviAggregateAnswerV2 {
                aggregate_answer_ref: 0,
                index_root_id: root.index_root_id,
                aggregate_kind: CoviAggregateKindV2::DistinctCount as u16,
                exactness: IndexCapabilityExactnessV2::Exact as u8,
                null_semantics: 0,
                flags: 0,
                row_count: 5,
                null_count: 2,
                non_null_count: 3,
                value_ref: 0,
                predicate_form_ref: ABSENT_U32,
                snapshot_validity_ref: 0,
                checksum: 0,
            }],
            payload: 2u64.to_le_bytes().to_vec(),
        };
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        validate_aggregate_block(&root, &snapshots, &block).unwrap();
    }

    #[test]
    fn aggregate_block_validation_accepts_sum_and_avg_payloads() {
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        for kind in [CoviAggregateKindV2::Sum, CoviAggregateKindV2::Avg] {
            let block = aggregate_value_block(
                root.index_root_id,
                kind,
                3,
                0,
                3,
                Some(6i64.to_le_bytes().to_vec()),
            );
            validate_aggregate_block(&root, &snapshots, &block).unwrap();
        }
    }

    #[test]
    fn aggregate_block_validation_rejects_malformed_sum_payload() {
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let block = aggregate_value_block(
            root.index_root_id,
            CoviAggregateKindV2::Sum,
            3,
            0,
            3,
            Some(vec![1, 2]),
        );
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        assert!(matches!(
            validate_aggregate_block(&root, &snapshots, &block),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn aggregate_block_validation_rejects_unsupported_sum_logical_type() {
        let root = root_with(
            CoveLogicalType::Utf8,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let block = aggregate_value_block(
            root.index_root_id,
            CoviAggregateKindV2::Sum,
            1,
            0,
            1,
            Some(b"x".to_vec()),
        );
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        assert!(matches!(
            validate_aggregate_block(&root, &snapshots, &block),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn aggregate_block_validation_accepts_all_null_sum_without_payload() {
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let block =
            aggregate_value_block(root.index_root_id, CoviAggregateKindV2::Sum, 2, 2, 0, None);
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        validate_aggregate_block(&root, &snapshots, &block).unwrap();
    }

    #[test]
    fn aggregate_block_validation_rejects_boolean_minmax_payload() {
        let root = root_with(
            CoveLogicalType::Bool,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let block = CoviAggregateAnswerBlockV2 {
            header: CoviAggregateAnswerBlockHeaderV2 {
                magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
                version_major: 2,
                version_minor: 0,
                header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
                aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
                aggregate_block_id: 13,
                index_root_id: root.index_root_id,
                aggregate_answer_count: 1,
                aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
                aggregate_payload_offset: 0,
                aggregate_payload_length: 0,
                flags: 0,
                checksum: 0,
            },
            answers: vec![CoviAggregateAnswerV2 {
                aggregate_answer_ref: 0,
                index_root_id: root.index_root_id,
                aggregate_kind: CoviAggregateKindV2::Min as u16,
                exactness: IndexCapabilityExactnessV2::Exact as u8,
                null_semantics: 0,
                flags: 0,
                row_count: 1,
                null_count: 0,
                non_null_count: 1,
                value_ref: 0,
                predicate_form_ref: ABSENT_U32,
                snapshot_validity_ref: 0,
                checksum: 0,
            }],
            payload: Vec::new(),
        };
        let snapshots =
            std::collections::BTreeMap::from([(0, snapshot_with_external_visibility(ABSENT_U32))]);
        assert!(matches!(
            validate_aggregate_block(&root, &snapshots, &block),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn digest_required_validation_rejects_referenced_file_without_digest() {
        let mut artifact = blank_artifact();
        artifact
            .referenced_files
            .push(referenced_file_with_digest_algorithm(DigestAlgorithm::None));
        let context = CoviValidationContextV2::for_file([3; 16], 64, 0x1234)
            .with_file_digest(DigestAlgorithm::Sha256, vec![0; 32]);
        assert!(matches!(
            validate_referenced_file(&artifact, None, &context),
            Err(CoveError::DigestMismatch)
        ));
    }

    #[test]
    fn numcode_equality_uses_float_logical_equality_for_signed_zero() {
        let request = CoviLookupRequestV2::eq(1, 2, CoviLookupKeyV2::NumCode(0.0f64.to_bits()));
        let root = root_with(
            CoveLogicalType::Float64,
            CoviKeyEncodingKindV2::NumCode,
            CoviComparatorKindV2::NumCodeLogicalOrdering,
        );
        let lower = request.lower_key.key_bytes();
        assert!(key_matches(
            &root,
            &request,
            &(-0.0f64).to_bits().to_le_bytes(),
            &lower,
            None,
            &[]
        )
        .unwrap());
    }

    #[test]
    fn canonical_ordering_range_uses_signed_value_order() {
        let request = CoviLookupRequestV2 {
            table_id: 1,
            column_id: 2,
            target: CoviLookupTargetV2::TableColumn {
                table_id: 1,
                column_id: 2,
            },
            op: CoviLookupOpV2::Range {
                lower_inclusive: true,
                upper_inclusive: true,
            },
            lower_key: CoviLookupKeyV2::CanonicalValueBytes(canonical_i64(-10)),
            upper_key: Some(CoviLookupKeyV2::CanonicalValueBytes(canonical_i64(0))),
            membership_keys: Vec::new(),
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        };
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        let lower = request.lower_key.key_bytes();
        let upper = request.upper_key.as_ref().map(CoviLookupKeyV2::key_bytes);
        assert!(key_matches(
            &root,
            &request,
            &canonical_i64(-5),
            &lower,
            upper.as_deref(),
            &[]
        )
        .unwrap());
        assert!(!key_matches(
            &root,
            &request,
            &canonical_i64(5),
            &lower,
            upper.as_deref(),
            &[]
        )
        .unwrap());
    }

    #[test]
    fn sorted_entry_validation_uses_comparator_order_with_byte_tie_breaker() {
        let mut root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        let comparator_sorted = vec![
            canonical_i64(-10),
            canonical_i64(-5),
            canonical_i64(0),
            canonical_i64(5),
        ];
        let (key_block, entry_block, postings_block) =
            entry_blocks_for_keys(&root, comparator_sorted);
        validate_entries_for_root(&root, &key_block, &entry_block, &postings_block, None).unwrap();

        let raw_sorted = vec![
            canonical_i64(0),
            canonical_i64(5),
            canonical_i64(-10),
            canonical_i64(-5),
        ];
        let (key_block, entry_block, postings_block) = entry_blocks_for_keys(&root, raw_sorted);
        assert!(matches!(
            validate_entries_for_root(&root, &key_block, &entry_block, &postings_block, None),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn sorted_entry_validation_rejects_unchained_byte_identical_duplicates() {
        let mut root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        let (key_block, entry_block, postings_block) =
            entry_blocks_for_keys(&root, vec![canonical_i64(7), canonical_i64(7)]);
        assert!(matches!(
            validate_entries_for_root(&root, &key_block, &entry_block, &postings_block, None),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn unsorted_entry_validation_rejects_malformed_canonical_keys() {
        let mut root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalEquality,
        );
        root.index_kind = CoviIndexKindV2::Hash;
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        let (key_block, entry_block, postings_block) =
            entry_blocks_for_keys(&root, vec![vec![0xfe]]);
        assert!(matches!(
            validate_entries_for_root(&root, &key_block, &entry_block, &postings_block, None),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn entry_validation_rejects_bad_fixed_width_key_shapes() {
        let mut root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::NumCode,
            CoviComparatorKindV2::NumCodeLogicalOrdering,
        );
        root.index_kind = CoviIndexKindV2::Hash;
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        let (key_block, entry_block, postings_block) =
            entry_blocks_for_keys(&root, vec![vec![1, 2, 3]]);
        assert!(matches!(
            validate_entries_for_root(&root, &key_block, &entry_block, &postings_block, None),
            Err(CoveError::BadCovi)
        ));

        let mut root = root_with(
            CoveLogicalType::UInt32,
            CoviKeyEncodingKindV2::FileCode,
            CoviComparatorKindV2::DomainRankOrdering,
        );
        root.index_kind = CoviIndexKindV2::Hash;
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        let (key_block, entry_block, postings_block) =
            entry_blocks_for_keys(&root, vec![vec![1, 2, 3]]);
        assert!(matches!(
            validate_entries_for_root(&root, &key_block, &entry_block, &postings_block, None),
            Err(CoveError::BadCovi)
        ));
    }

    #[test]
    fn index_only_sum_and_avg_use_sum_and_count_support_flags() {
        let mut capability = capability(1, 0, 0, 0, IndexCapabilityExactnessV2::Exact);
        assert!(!index_only_capability_supports_aggregate(
            &capability,
            CoviAggregateKindV2::Sum
        ));
        assert!(!index_only_capability_supports_aggregate(
            &capability,
            CoviAggregateKindV2::Avg
        ));

        capability.supports_sum = 1;
        assert!(index_only_capability_supports_aggregate(
            &capability,
            CoviAggregateKindV2::Sum
        ));
        assert!(!index_only_capability_supports_aggregate(
            &capability,
            CoviAggregateKindV2::Avg
        ));

        capability.supports_count = 1;
        assert!(index_only_capability_supports_aggregate(
            &capability,
            CoviAggregateKindV2::Avg
        ));
    }

    #[test]
    fn comparator_order_tie_breaks_by_canonical_bytes() {
        let positive_zero = tagged_key(ValueTag::Float64Bits, 0.0f64.to_bits().to_le_bytes());
        let negative_zero = tagged_key(ValueTag::Float64Bits, (-0.0f64).to_bits().to_le_bytes());
        let context = CoviLookupComparatorContextV2::default();
        assert_eq!(
            compare_key_bytes_for_order(
                CoviComparatorKindV2::CanonicalOrdering as u16,
                Some(CoveLogicalType::Float64),
                &context,
                &positive_zero,
                &negative_zero
            )
            .unwrap(),
            positive_zero.cmp(&negative_zero)
        );
    }

    #[test]
    fn lookup_root_skips_incompatible_first_root_for_later_range_root() {
        let mut eq_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        eq_root.index_root_id = 1;
        eq_root.key_block_section_id = 10;
        eq_root.entry_block_section_id = 11;
        eq_root.postings_block_section_id = 12;
        let mut range_root = eq_root.clone();
        range_root.index_root_id = 2;
        range_root.key_block_section_id = 20;
        range_root.entry_block_section_id = 21;
        range_root.postings_block_section_id = 22;
        let artifact = validated_with_roots(
            vec![eq_root, range_root],
            vec![
                capability(1, 1, 0, 1, IndexCapabilityExactnessV2::Exact),
                capability(2, 1, 1, 1, IndexCapabilityExactnessV2::Exact),
            ],
        );
        let request = CoviLookupRequestV2 {
            table_id: 1,
            column_id: 2,
            target: CoviLookupTargetV2::TableColumn {
                table_id: 1,
                column_id: 2,
            },
            op: CoviLookupOpV2::Range {
                lower_inclusive: true,
                upper_inclusive: true,
            },
            lower_key: CoviLookupKeyV2::CanonicalValueBytes(canonical_i64(-10)),
            upper_key: Some(CoviLookupKeyV2::CanonicalValueBytes(canonical_i64(10))),
            membership_keys: Vec::new(),
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        };
        let (root, _) = artifact.lookup_root(&request).unwrap();
        assert_eq!(root.index_root_id, 2);
    }

    #[test]
    fn lookup_root_skips_exact_root_with_non_exact_proof_for_later_valid_root() {
        let mut advisory_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        advisory_root.index_root_id = 1;
        advisory_root.key_block_section_id = 10;
        advisory_root.entry_block_section_id = 11;
        advisory_root.postings_block_section_id = 12;
        advisory_root.proof_strength = CoverageProofStrengthV2::AdvisoryOnly as u8;
        let mut valid_root = advisory_root.clone();
        valid_root.index_root_id = 2;
        valid_root.key_block_section_id = 20;
        valid_root.entry_block_section_id = 21;
        valid_root.postings_block_section_id = 22;
        valid_root.proof_strength = CoverageProofStrengthV2::ExactConservative as u8;

        let mut advisory_capability = capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        advisory_capability.proof_strength = CoverageProofStrengthV2::ExactConservative;
        let valid_capability = capability(2, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        let artifact = validated_with_roots(
            vec![advisory_root, valid_root],
            vec![advisory_capability, valid_capability],
        );
        let request = CoviLookupRequestV2::eq(
            1,
            2,
            CoviLookupKeyV2::CanonicalValueBytes(canonical_i64(42)),
        );

        let (root, _) = artifact.lookup_root(&request).unwrap();
        assert_eq!(root.index_root_id, 2);
    }

    #[test]
    fn lookup_root_skips_exact_capability_with_non_exact_proof_for_later_valid_root() {
        let mut advisory_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        advisory_root.index_root_id = 1;
        advisory_root.key_block_section_id = 10;
        advisory_root.entry_block_section_id = 11;
        advisory_root.postings_block_section_id = 12;
        let mut valid_root = advisory_root.clone();
        valid_root.index_root_id = 2;
        valid_root.key_block_section_id = 20;
        valid_root.entry_block_section_id = 21;
        valid_root.postings_block_section_id = 22;

        let mut advisory_capability = capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        advisory_capability.proof_strength = CoverageProofStrengthV2::AdvisoryOnly;
        let valid_capability = capability(2, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        let artifact = validated_with_roots(
            vec![advisory_root, valid_root],
            vec![advisory_capability, valid_capability],
        );
        let request = CoviLookupRequestV2::eq(
            1,
            2,
            CoviLookupKeyV2::CanonicalValueBytes(canonical_i64(42)),
        );

        let (root, _) = artifact.lookup_root(&request).unwrap();
        assert_eq!(root.index_root_id, 2);
    }

    #[test]
    fn index_only_answer_skips_approximate_root_for_exact_later_root() {
        let mut approximate_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        approximate_root.index_root_id = 1;
        approximate_root.key_block_section_id = 10;
        approximate_root.entry_block_section_id = 11;
        approximate_root.postings_block_section_id = 12;
        approximate_root.aggregate_block_section_id = 13;
        let mut exact_root = approximate_root.clone();
        exact_root.index_root_id = 2;
        exact_root.key_block_section_id = 20;
        exact_root.entry_block_section_id = 21;
        exact_root.postings_block_section_id = 22;
        exact_root.aggregate_block_section_id = 23;
        let mut approximate_capability =
            capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Approximate);
        approximate_capability.supports_index_only = 1;
        approximate_capability.supports_count = 1;
        let mut exact_capability = capability(2, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        exact_capability.supports_index_only = 1;
        exact_capability.supports_count = 1;
        let mut artifact = validated_with_roots(
            vec![approximate_root, exact_root],
            vec![approximate_capability, exact_capability],
        );
        artifact.aggregate_blocks.insert(
            13,
            aggregate_count_block(1, 13, IndexCapabilityExactnessV2::Approximate, 7),
        );
        artifact.aggregate_blocks.insert(
            23,
            aggregate_count_block(2, 23, IndexCapabilityExactnessV2::Exact, 42),
        );
        let answer = artifact
            .index_only_answer(&CoviIndexOnlyRequestV2 {
                table_id: 1,
                column_id: Some(2),
                aggregate_kind: CoviAggregateKindV2::Count,
                predicate_form_ref: None,
                require_exact: true,
            })
            .unwrap()
            .unwrap();
        assert_eq!(answer.row_count, 42);
    }

    #[test]
    fn index_only_answer_skips_non_exact_proof_for_exact_later_root() {
        let mut advisory_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        advisory_root.index_root_id = 1;
        advisory_root.key_block_section_id = 10;
        advisory_root.entry_block_section_id = 11;
        advisory_root.postings_block_section_id = 12;
        advisory_root.aggregate_block_section_id = 13;
        advisory_root.proof_strength = CoverageProofStrengthV2::AdvisoryOnly as u8;
        let mut valid_root = advisory_root.clone();
        valid_root.index_root_id = 2;
        valid_root.key_block_section_id = 20;
        valid_root.entry_block_section_id = 21;
        valid_root.postings_block_section_id = 22;
        valid_root.aggregate_block_section_id = 23;
        valid_root.proof_strength = CoverageProofStrengthV2::ExactConservative as u8;

        let mut advisory_capability = capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        advisory_capability.supports_index_only = 1;
        advisory_capability.supports_count = 1;
        let mut valid_capability = capability(2, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        valid_capability.supports_index_only = 1;
        valid_capability.supports_count = 1;

        let mut artifact = validated_with_roots(
            vec![advisory_root, valid_root],
            vec![advisory_capability, valid_capability],
        );
        artifact.aggregate_blocks.insert(
            13,
            aggregate_count_block(1, 13, IndexCapabilityExactnessV2::Exact, 7),
        );
        artifact.aggregate_blocks.insert(
            23,
            aggregate_count_block(2, 23, IndexCapabilityExactnessV2::Exact, 42),
        );

        let answer = artifact
            .index_only_answer(&CoviIndexOnlyRequestV2 {
                table_id: 1,
                column_id: Some(2),
                aggregate_kind: CoviAggregateKindV2::Count,
                predicate_form_ref: None,
                require_exact: true,
            })
            .unwrap()
            .unwrap();
        assert_eq!(answer.row_count, 42);
    }

    #[test]
    fn index_only_answer_matches_object_property_targets() {
        let mut root = root_with(
            CoveLogicalType::Bool,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        root.indexed_target_kind = CoviIndexedTargetKindV2::ObjectProperty;
        root.table_id = ABSENT_U32;
        root.column_id = ABSENT_U32;
        root.object_type_id = 7;
        root.property_id = 11;
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        root.aggregate_block_section_id = 13;
        let mut capability = capability(1, 1, 0, 0, IndexCapabilityExactnessV2::Exact);
        capability.supports_index_only = 1;
        capability.supports_count = 1;
        let mut artifact = validated_with_roots(vec![root], vec![capability]);
        artifact.aggregate_blocks.insert(
            13,
            aggregate_count_block(1, 13, IndexCapabilityExactnessV2::Exact, 5),
        );

        let answer = artifact
            .index_only_answer_for_target(
                CoviLookupTargetV2::ObjectProperty {
                    object_type_id: 7,
                    property_id: 11,
                },
                &CoviIndexOnlyRequestV2 {
                    table_id: ABSENT_U32,
                    column_id: None,
                    aggregate_kind: CoviAggregateKindV2::Count,
                    predicate_form_ref: None,
                    require_exact: true,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(answer.non_null_count, 5);
    }

    #[test]
    fn index_only_answer_rejects_only_non_exact_proof_for_exact_request() {
        let mut root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        root.aggregate_block_section_id = 13;
        let mut capability = capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        capability.supports_index_only = 1;
        capability.supports_count = 1;
        capability.proof_strength = CoverageProofStrengthV2::EngineLocal;
        let mut artifact = validated_with_roots(vec![root], vec![capability]);
        artifact.aggregate_blocks.insert(
            13,
            aggregate_count_block(1, 13, IndexCapabilityExactnessV2::Exact, 7),
        );

        assert!(matches!(
            artifact.index_only_answer(&CoviIndexOnlyRequestV2 {
                table_id: 1,
                column_id: Some(2),
                aggregate_kind: CoviAggregateKindV2::Count,
                predicate_form_ref: None,
                require_exact: true,
            }),
            Err(CoveError::IndexOnlyUnsafe)
        ));
    }

    #[test]
    fn index_only_answer_rejects_disabled_aggregate_support_flag() {
        let mut root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        root.aggregate_block_section_id = 13;
        let mut capability = capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        capability.supports_index_only = 1;
        capability.supports_count = 0;
        let mut artifact = validated_with_roots(vec![root], vec![capability]);
        artifact.aggregate_blocks.insert(
            13,
            aggregate_count_block(1, 13, IndexCapabilityExactnessV2::Exact, 7),
        );

        assert!(matches!(
            artifact.index_only_answer(&CoviIndexOnlyRequestV2 {
                table_id: 1,
                column_id: Some(2),
                aggregate_kind: CoviAggregateKindV2::Count,
                predicate_form_ref: None,
                require_exact: true,
            }),
            Err(CoveError::IndexOnlyUnsafe)
        ));
    }

    #[test]
    fn index_only_answer_skips_disabled_aggregate_root_for_later_valid_root() {
        let mut unsupported_root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::CanonicalValueBytes,
            CoviComparatorKindV2::CanonicalOrdering,
        );
        unsupported_root.index_root_id = 1;
        unsupported_root.key_block_section_id = 10;
        unsupported_root.entry_block_section_id = 11;
        unsupported_root.postings_block_section_id = 12;
        unsupported_root.aggregate_block_section_id = 13;
        let mut supported_root = unsupported_root.clone();
        supported_root.index_root_id = 2;
        supported_root.key_block_section_id = 20;
        supported_root.entry_block_section_id = 21;
        supported_root.postings_block_section_id = 22;
        supported_root.aggregate_block_section_id = 23;

        let mut unsupported_capability = capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        unsupported_capability.supports_index_only = 1;
        unsupported_capability.supports_count = 0;
        let mut supported_capability = capability(2, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        supported_capability.supports_index_only = 1;
        supported_capability.supports_count = 1;

        let mut artifact = validated_with_roots(
            vec![unsupported_root, supported_root],
            vec![unsupported_capability, supported_capability],
        );
        artifact.aggregate_blocks.insert(
            13,
            aggregate_count_block(1, 13, IndexCapabilityExactnessV2::Exact, 7),
        );
        artifact.aggregate_blocks.insert(
            23,
            aggregate_count_block(2, 23, IndexCapabilityExactnessV2::Exact, 42),
        );

        let answer = artifact
            .index_only_answer(&CoviIndexOnlyRequestV2 {
                table_id: 1,
                column_id: Some(2),
                aggregate_kind: CoviAggregateKindV2::Count,
                predicate_form_ref: None,
                require_exact: true,
            })
            .unwrap()
            .unwrap();
        assert_eq!(answer.row_count, 42);
    }

    #[test]
    fn exact_membership_uses_comparator_equality_for_signed_zero() {
        let mut root = root_with(
            CoveLogicalType::Float64,
            CoviKeyEncodingKindV2::NumCode,
            CoviComparatorKindV2::NumCodeLogicalOrdering,
        );
        root.key_block_section_id = 10;
        root.entry_block_section_id = 11;
        root.postings_block_section_id = 12;
        let mut capability = capability(1, 1, 1, 1, IndexCapabilityExactnessV2::Exact);
        capability.supports_membership = 1;
        let mut artifact = validated_with_roots(vec![root.clone()], vec![capability]);
        let (key_block, entry_block, postings_block) = entry_blocks_for_keys(
            &root,
            vec![
                (-0.0f64).to_bits().to_le_bytes().to_vec(),
                0.0f64.to_bits().to_le_bytes().to_vec(),
            ],
        );
        artifact
            .key_blocks
            .insert(root.key_block_section_id, key_block);
        artifact
            .entry_blocks
            .insert(root.entry_block_section_id, entry_block);
        artifact
            .postings_blocks
            .insert(root.postings_block_section_id, postings_block);

        let request = CoviLookupRequestV2 {
            table_id: 1,
            column_id: 2,
            target: CoviLookupTargetV2::TableColumn {
                table_id: 1,
                column_id: 2,
            },
            op: CoviLookupOpV2::Membership,
            lower_key: CoviLookupKeyV2::NumCode(0.0f64.to_bits()),
            upper_key: None,
            membership_keys: Vec::new(),
            logical_type: Some(CoveLogicalType::Float64),
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        };
        let answer = artifact.exact_membership_answer(&request).unwrap();
        assert_eq!(answer.requested_key_count, 1);
        assert_eq!(answer.present_key_count, 1);
    }

    #[test]
    fn domain_rank_ordering_requires_context_and_uses_rank_order() {
        let request = CoviLookupRequestV2 {
            table_id: 1,
            column_id: 2,
            target: CoviLookupTargetV2::TableColumn {
                table_id: 1,
                column_id: 2,
            },
            op: CoviLookupOpV2::Range {
                lower_inclusive: true,
                upper_inclusive: true,
            },
            lower_key: CoviLookupKeyV2::FileCode(10),
            upper_key: Some(CoviLookupKeyV2::FileCode(30)),
            membership_keys: Vec::new(),
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        };
        let root = root_with(
            CoveLogicalType::Utf8,
            CoviKeyEncodingKindV2::FileCode,
            CoviComparatorKindV2::DomainRankOrdering,
        );
        let lower = request.lower_key.key_bytes();
        let upper = request.upper_key.as_ref().map(CoviLookupKeyV2::key_bytes);
        assert!(matches!(
            key_matches(
                &root,
                &request,
                &20u32.to_le_bytes(),
                &lower,
                upper.as_deref(),
                &[]
            ),
            Err(CoveError::UnsupportedEncoding(_))
        ));

        let request = request.with_domain_rank_context({
            let mut ranks = vec![INVALID_RANK; 31];
            ranks[10] = 0;
            ranks[20] = 1;
            ranks[30] = 2;
            ranks
        });
        assert!(key_matches(
            &root,
            &request,
            &20u32.to_le_bytes(),
            &lower,
            upper.as_deref(),
            &[]
        )
        .unwrap());
    }

    #[test]
    fn interval_overlap_matches_overlapping_intervals_only() {
        let root = root_with(
            CoveLogicalType::Int64,
            CoviKeyEncodingKindV2::IntervalTuple,
            CoviComparatorKindV2::IntervalOverlap,
        );
        let request_interval = CoviLookupKeyV2::IntervalTuple(CoviIntervalKeyV2::new(
            Some(canonical_i64(10)),
            Some(canonical_i64(20)),
            true,
            false,
        ));
        let request = CoviLookupRequestV2 {
            table_id: 1,
            column_id: 2,
            target: CoviLookupTargetV2::TableColumn {
                table_id: 1,
                column_id: 2,
            },
            op: CoviLookupOpV2::Range {
                lower_inclusive: true,
                upper_inclusive: true,
            },
            lower_key: request_interval,
            upper_key: None,
            membership_keys: Vec::new(),
            logical_type: None,
            comparator_context: CoviLookupComparatorContextV2::default(),
            require_exact: true,
        }
        .with_interval_encoding(CoviIntervalEncodingV2::CanonicalBoundsV1);
        let lower = request.lower_key.key_bytes();
        let overlapping = CoviLookupKeyV2::IntervalTuple(CoviIntervalKeyV2::new(
            Some(canonical_i64(19)),
            Some(canonical_i64(30)),
            true,
            true,
        ))
        .key_bytes();
        let adjacent = CoviLookupKeyV2::IntervalTuple(CoviIntervalKeyV2::new(
            Some(canonical_i64(20)),
            Some(canonical_i64(30)),
            true,
            true,
        ))
        .key_bytes();
        assert!(key_matches(&root, &request, &overlapping, &lower, None, &[]).unwrap());
        assert!(!key_matches(&root, &request, &adjacent, &lower, None, &[]).unwrap());
    }

    #[test]
    fn numcode_range_key_match_uses_signed_logical_order() {
        let mut request = CoviLookupRequestV2::range_numcode(
            1,
            2,
            CoveLogicalType::Int64,
            (-10i64) as u64,
            Some(0),
            true,
            true,
        );
        let root = CoviIndexRootV2 {
            index_root_id: 1,
            indexed_target_kind: CoviIndexedTargetKindV2::TableColumn,
            index_kind: CoviIndexKindV2::Sorted,
            coverage_granularity: 0,
            proof_strength: 0,
            exactness: 0,
            flags: 0,
            table_id: 1,
            column_id: 2,
            object_type_id: ABSENT_U32,
            property_id: ABSENT_U32,
            path_ref: ABSENT_U32,
            semantic_dimension_ref: ABSENT_U32,
            logical_type: CoveLogicalType::Int64 as u16,
            physical_kind: 0,
            key_encoding_kind: CoviKeyEncodingKindV2::NumCode as u8,
            comparator_kind: CoviComparatorKindV2::NumCodeLogicalOrdering as u16,
            collation_id: 0,
            null_semantics: 0,
            sort_order: 0,
            value_count: 0,
            distinct_count: 0,
            null_count: 0,
            min_key_ref: ABSENT_U32,
            max_key_ref: ABSENT_U32,
            key_block_section_id: ABSENT_U32,
            entry_block_section_id: ABSENT_U32,
            postings_block_section_id: ABSENT_U32,
            aggregate_block_section_id: ABSENT_U32,
            coverage_set_ref: ABSENT_U32,
            capability_ref: ABSENT_U32,
            snapshot_validity_ref: ABSENT_U32,
            checksum: 0,
        };
        let lower = request.lower_key.key_bytes();
        let upper = request.upper_key.as_ref().map(CoviLookupKeyV2::key_bytes);
        let empty = Vec::new();

        assert!(key_matches(
            &root,
            &request,
            &((-5i64) as u64).to_le_bytes(),
            &lower,
            upper.as_deref(),
            &empty
        )
        .unwrap());
        assert!(!key_matches(
            &root,
            &request,
            &(5u64).to_le_bytes(),
            &lower,
            upper.as_deref(),
            &empty
        )
        .unwrap());

        request.logical_type = None;
        assert!(key_matches(
            &root,
            &request,
            &((-5i64) as u64).to_le_bytes(),
            &lower,
            upper.as_deref(),
            &empty
        )
        .unwrap());
    }
}
