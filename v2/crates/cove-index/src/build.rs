use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use crate::{
    execution::{
        compare_key_bytes_for_order, CoviAggregateKindV2, CoviLookupComparatorContextV2,
        CoviLookupKeyV2,
    },
    CoviAggregateAnswerBlockHeaderV2, CoviAggregateAnswerBlockV2, CoviAggregateAnswerV2,
    CoviArtifactV2, CoviComparatorKindV2, CoviEntryBlockHeaderV2, CoviEntryBlockV2,
    CoviIndexEntryV2, CoviIndexKindV2, CoviIndexRootV2, CoviIndexedTargetKindV2,
    CoviKeyBlockHeaderV2, CoviKeyBlockV2, CoviKeyEncodingKindV2, CoviPostingRepresentationV2,
    CoviPostingsBlockHeaderV2, CoviPostingsBlockV2, CoviPostingsHeaderV2, CoviReferencedFileV2,
    CoviRowRangePostingV2, CoviSectionKindV2, CoviSectionPayloadV2, CoviSnapshotValidityV2,
    IndexCapabilityExactnessV2, IndexCapabilityV2, IndexOnlyCapabilityV2,
};
use cove_core::{
    array::{CoveArrayValue, EncodedArray},
    canonical::CanonicalValue,
    checksum,
    compression::{column_page_payload, section_payload},
    constants::{
        CoveLogicalType, CovePhysicalKind, DigestAlgorithm, SectionKind, ValueTag,
        FEATURE_INDEX_ONLY_CAPABILITY,
    },
    dictionary::DictionaryValue,
    digest::compute_digest,
    materialize_stats_only_constant_page_payload,
    mount::{mount_cove_file, MountOptions, MountedCoveFile, OutputRepresentation},
    page::{page_uses_payload_elision, ColumnPageIndex},
    page_payload::{ColumnPagePayloadV1, PageBufferKind},
    postscript::CovePostscriptV1,
    profile::cove_o::{
        read_object_surface_from_bytes_with_options, CoveObjectReadOptions, CoveObjectRecord,
        CoveObjectRedactionReadPolicy, ObjectTypeEntryV1, PropertyEntryV1,
    },
    reader::{validate_bytes_with_options, ValidationOptions},
    segment::TableSegmentPayloadV1,
    table::{ColumnEntry, TableEntry},
    types,
    validity::ValidityBitmap,
    wire, CoveError, StatsOnlyPageMaterializationContext,
};
use cove_coverage::{CoverageExactnessV2, CoverageGranularityV2, CoverageProofStrengthV2};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct CoviBuildOptions {
    pub target: CoviBuildTarget,
    pub table_id: Option<u32>,
    pub column_ids: Vec<u32>,
    pub all_columns: bool,
    pub include_index_only_counts: bool,
    pub include_index_only_min_max: bool,
    pub include_index_only_distinct_count: bool,
    pub include_index_only_exists: bool,
    pub include_index_only_sum_avg: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoviBuildTarget {
    #[default]
    TableColumn,
    ProjectionColumn,
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
pub struct CoviObjectPropertyBuildOptions {
    pub include_index_only_counts: bool,
    pub include_index_only_min_max: bool,
    pub include_index_only_distinct_count: bool,
    pub include_index_only_exists: bool,
    pub include_index_only_sum_avg: bool,
    pub include_object_paths: bool,
    pub include_association_endpoints: bool,
    pub include_evidence_lookup: bool,
    pub include_projection_fragments: bool,
}

#[derive(Debug, Clone)]
pub struct CoviProjectionColumnInput {
    pub table_id: u32,
    pub column_id: u32,
    pub logical_type: CoveLogicalType,
    pub physical_kind: CovePhysicalKind,
    pub values: Vec<Value>,
}

impl Default for CoviObjectPropertyBuildOptions {
    fn default() -> Self {
        Self {
            include_index_only_counts: true,
            include_index_only_min_max: true,
            include_index_only_distinct_count: true,
            include_index_only_exists: true,
            include_index_only_sum_avg: true,
            include_object_paths: true,
            include_association_endpoints: true,
            include_evidence_lookup: true,
            include_projection_fragments: true,
        }
    }
}

impl CoviObjectPropertyBuildOptions {
    fn as_table_options(&self) -> CoviBuildOptions {
        CoviBuildOptions {
            include_index_only_counts: self.include_index_only_counts,
            include_index_only_min_max: self.include_index_only_min_max,
            include_index_only_distinct_count: self.include_index_only_distinct_count,
            include_index_only_exists: self.include_index_only_exists,
            include_index_only_sum_avg: self.include_index_only_sum_avg,
            ..CoviBuildOptions::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct IndexedTargetMetadata {
    indexed_target_kind: CoviIndexedTargetKindV2,
    table_id: u32,
    column_id: u32,
    object_type_id: u32,
    property_id: u32,
    path_ref: u32,
    semantic_dimension_ref: u32,
}

type KeyPostingMap = BTreeMap<Vec<u8>, Vec<CoviRowRangePostingV2>>;
type OrderedKeyPostings = Vec<(Vec<u8>, Vec<CoviRowRangePostingV2>)>;

#[derive(Clone, Copy)]
struct BuildBlocksParams {
    root_id: u32,
    logical_type: CoveLogicalType,
    index_kind: CoviIndexKindV2,
    key_encoding_kind: CoviKeyEncodingKindV2,
    comparator_kind: CoviComparatorKindV2,
    supports_range: bool,
    null_count: u64,
    aggregate_block_id: Option<u32>,
}

#[derive(Clone, Copy)]
struct AggregateAnswerBlockParams<'a> {
    root_id: u32,
    row_count: u64,
    null_count: u64,
    distinct_count: u64,
    logical_type: CoveLogicalType,
    options: &'a CoviBuildOptions,
    include_min_max_answers: bool,
    include_sum_avg_answers: bool,
    min_key: Option<&'a [u8]>,
    max_key: Option<&'a [u8]>,
    sum_payload: Option<&'a [u8]>,
}

pub fn build_covi_from_cove_bytes(
    input: &[u8],
    options: &CoviBuildOptions,
) -> Result<Vec<u8>, CoveError> {
    if options.all_columns && !options.column_ids.is_empty() {
        return Err(CoveError::BadCovi);
    }
    let postscript = CovePostscriptV1::parse_from_tail(input)?;
    let mounted = mount_cove_file(
        input,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            ..MountOptions::default()
        },
        None,
    )?;
    let table_catalog = mounted
        .table_catalog
        .as_ref()
        .ok_or_else(|| CoveError::BadSchema("no table catalog found".into()))?;
    let table = match options.table_id {
        Some(table_id) => table_catalog
            .tables
            .iter()
            .find(|table| table.table_id == table_id)
            .ok_or_else(|| CoveError::BadSchema(format!("table_id {table_id} not found")))?,
        None => table_catalog
            .tables
            .first()
            .ok_or_else(|| CoveError::BadSchema("table catalog is empty".into()))?,
    };
    let columns = selected_columns(table, options)?;
    if columns.is_empty() {
        return Err(CoveError::BadSchema("no columns selected".into()));
    }

    let dataset_id = mounted.header.file_id;
    let snapshot_id = derived_snapshot_id(input, postscript.footer.crc32c);
    let file_digest = compute_digest(DigestAlgorithm::Sha256, input)?;
    let referenced_file = CoviReferencedFileV2 {
        file_ref: 0,
        flags: 0,
        file_id: mounted.header.file_id,
        file_len: input.len() as u64,
        footer_crc32c: postscript.footer.crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: file_digest
            .len()
            .try_into()
            .map_err(|_| CoveError::ArithOverflow)?,
        digest_offset: 0,
        uri_ref: u32::MAX,
        schema_fingerprint_ref: u32::MAX,
        checksum: 0,
    };
    let snapshot_validity = CoviSnapshotValidityV2 {
        snapshot_validity_ref: 0,
        dataset_id,
        snapshot_id,
        schema_fingerprint_ref: u32::MAX,
        semantic_map_fingerprint_ref: u32::MAX,
        external_visibility_ref: u32::MAX,
        data_checksum_root_ref: u32::MAX,
        valid_from_us: mounted.header.created_at_us,
        valid_until_us: i64::MAX,
        flags: 0,
        checksum: 0,
    };
    let mut roots = Vec::new();
    let mut capabilities = Vec::new();
    let mut index_only_capabilities = Vec::new();
    let mut section_payloads = vec![CoviSectionPayloadV2 {
        section_id: 1,
        section_kind: CoviSectionKindV2::StringTable,
        payload: file_digest,
        item_count: 1,
        required_features: 0,
        optional_features: 0,
    }];
    for (root_index, column) in columns.iter().enumerate() {
        let root_id = u32::try_from(root_index).map_err(|_| CoveError::ArithOverflow)?;
        let mut built_index = build_column_index(
            root_id,
            input,
            &mounted,
            table,
            column,
            options.include_index_only_counts.then_some(root_id),
        )?;
        let key_section_id = 2 + root_id * 4;
        let entry_section_id = key_section_id + 1;
        let postings_section_id = key_section_id + 2;
        let aggregate_section_id = key_section_id + 3;
        let include_min_max_answers = options.include_index_only_min_max
            && built_index.supports_range
            && aggregate_min_max_answer_supported(column.logical)
            && built_index.min_key.is_some()
            && built_index.max_key.is_some();
        let include_sum_avg_answers = options.include_index_only_sum_avg
            && aggregate_sum_answer_supported(column.logical)
            && (built_index.sum_payload.is_some() || built_index.null_count == table.row_count);
        let include_aggregate_block = options.include_index_only_counts
            || include_min_max_answers
            || options.include_index_only_distinct_count
            || options.include_index_only_exists
            || include_sum_avg_answers;
        let aggregate_block_section_id = if include_aggregate_block {
            aggregate_section_id
        } else {
            u32::MAX
        };
        if include_aggregate_block {
            built_index.entry_block =
                entry_block_with_aggregate_block_id(&built_index.entry_block, root_id)?;
            append_index_only_capabilities(
                &mut index_only_capabilities,
                root_id,
                options,
                include_min_max_answers,
                include_sum_avg_answers,
            );
        }
        let target = indexed_target_metadata(options.target, table.table_id, column.column_id);
        roots.push(CoviIndexRootV2 {
            index_root_id: root_id,
            indexed_target_kind: target.indexed_target_kind,
            index_kind: built_index.index_kind,
            coverage_granularity: CoverageGranularityV2::Morsel as u8,
            proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
            exactness: CoverageExactnessV2::Exact as u8,
            flags: 0,
            table_id: target.table_id,
            column_id: target.column_id,
            object_type_id: target.object_type_id,
            property_id: target.property_id,
            path_ref: target.path_ref,
            semantic_dimension_ref: target.semantic_dimension_ref,
            logical_type: column.logical as u16,
            physical_kind: column.physical as u8,
            key_encoding_kind: built_index.key_encoding_kind as u8,
            comparator_kind: built_index.comparator_kind as u16,
            collation_id: column.collation_id,
            null_semantics: 0,
            sort_order: column.sort_order.min(u16::from(u8::MAX)) as u8,
            value_count: table.row_count,
            distinct_count: built_index.distinct_count,
            null_count: built_index.null_count,
            min_key_ref: built_index.min_key_ref,
            max_key_ref: built_index.max_key_ref,
            key_block_section_id: key_section_id,
            entry_block_section_id: entry_section_id,
            postings_block_section_id: postings_section_id,
            aggregate_block_section_id,
            coverage_set_ref: u32::MAX,
            capability_ref: root_id,
            snapshot_validity_ref: 0,
            checksum: 0,
        });
        capabilities.push(IndexCapabilityV2 {
            capability_id: root_id,
            index_root_id: root_id,
            flags: 0,
            supports_eq: 1,
            supports_range: u8::from(built_index.supports_range),
            supports_membership: 1,
            supports_prefix: 0,
            supports_contains: 0,
            supports_count: u8::from(
                options.include_index_only_counts
                    || options.include_index_only_exists
                    || include_sum_avg_answers,
            ),
            supports_min: u8::from(include_min_max_answers),
            supports_max: u8::from(include_min_max_answers),
            supports_sum: u8::from(include_sum_avg_answers),
            supports_distinct_count: u8::from(options.include_index_only_distinct_count),
            supports_join_coverage: 0,
            supports_index_only: u8::from(include_aggregate_block),
            exactness: IndexCapabilityExactnessV2::Exact,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            null_semantics: 0,
            reserved: 0,
            snapshot_validity_ref: 0,
            coverage_provider_ref: u32::MAX,
            checksum: 0,
        });
        section_payloads.extend([
            CoviSectionPayloadV2 {
                section_id: key_section_id,
                section_kind: CoviSectionKindV2::KeyBlock,
                payload: built_index.key_block,
                item_count: built_index.distinct_count,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: entry_section_id,
                section_kind: CoviSectionKindV2::EntryBlock,
                payload: built_index.entry_block,
                item_count: built_index.distinct_count,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: postings_section_id,
                section_kind: CoviSectionKindV2::PostingsBlock,
                payload: built_index.postings_block,
                item_count: built_index.distinct_count,
                required_features: 0,
                optional_features: 0,
            },
        ]);
        if include_aggregate_block {
            let aggregate_block = aggregate_answer_block(AggregateAnswerBlockParams {
                root_id,
                row_count: table.row_count,
                null_count: built_index.null_count,
                distinct_count: built_index.distinct_count,
                logical_type: column.logical,
                options,
                include_min_max_answers,
                include_sum_avg_answers,
                min_key: built_index.min_key.as_deref(),
                max_key: built_index.max_key.as_deref(),
                sum_payload: built_index.sum_payload.as_deref(),
            })?;
            section_payloads.push(CoviSectionPayloadV2 {
                section_id: aggregate_section_id,
                section_kind: CoviSectionKindV2::AggregateAnswerBlock,
                payload: aggregate_block,
                item_count: aggregate_answer_count(
                    options,
                    include_min_max_answers,
                    include_sum_avg_answers,
                ) as u64,
                required_features: 0,
                optional_features: 0,
            });
        }
    }
    if !index_only_capabilities.is_empty() {
        let section_id = 2u32
            .checked_add(
                u32::try_from(columns.len())
                    .map_err(|_| CoveError::ArithOverflow)?
                    .checked_mul(4)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .ok_or(CoveError::ArithOverflow)?;
        section_payloads.push(CoviSectionPayloadV2 {
            section_id,
            section_kind: CoviSectionKindV2::IndexOnlyCapabilities,
            payload: index_only_capability_payload(&index_only_capabilities)?,
            item_count: index_only_capabilities.len() as u64,
            required_features: 0,
            optional_features: FEATURE_INDEX_ONLY_CAPABILITY,
        });
    }

    let bytes = CoviArtifactV2::serialize_with_sections(
        dataset_id,
        snapshot_id,
        &[referenced_file],
        &[snapshot_validity],
        &roots,
        &capabilities,
        &section_payloads,
    )?;
    CoviArtifactV2::parse(&bytes)?;
    Ok(bytes)
}

pub fn build_covi_from_cove_o_bytes(
    input: &[u8],
    options: &CoviObjectPropertyBuildOptions,
) -> Result<Vec<u8>, CoveError> {
    let postscript = CovePostscriptV1::parse_from_tail(input)?;
    let validation = validate_bytes_with_options(
        input,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )?;
    let surface = read_object_surface_from_bytes_with_options(
        input,
        &CoveObjectReadOptions {
            include_association_object_types: true,
            include_records: true,
            redaction_read_policy: CoveObjectRedactionReadPolicy::PreserveMarker,
            ..CoveObjectReadOptions::default()
        },
    )?;

    let dataset_id = validation.validated.header.file_id;
    let snapshot_id = derived_snapshot_id(input, postscript.footer.crc32c);
    let file_digest = compute_digest(DigestAlgorithm::Sha256, input)?;
    let referenced_file = CoviReferencedFileV2 {
        file_ref: 0,
        flags: 0,
        file_id: validation.validated.header.file_id,
        file_len: input.len() as u64,
        footer_crc32c: postscript.footer.crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: file_digest
            .len()
            .try_into()
            .map_err(|_| CoveError::ArithOverflow)?,
        digest_offset: 0,
        uri_ref: u32::MAX,
        schema_fingerprint_ref: u32::MAX,
        checksum: 0,
    };
    let snapshot_validity = CoviSnapshotValidityV2 {
        snapshot_validity_ref: 0,
        dataset_id,
        snapshot_id,
        schema_fingerprint_ref: u32::MAX,
        semantic_map_fingerprint_ref: u32::MAX,
        external_visibility_ref: u32::MAX,
        data_checksum_root_ref: u32::MAX,
        valid_from_us: validation.validated.header.created_at_us,
        valid_until_us: i64::MAX,
        flags: 0,
        checksum: 0,
    };

    let table_options = options.as_table_options();
    let mut roots = Vec::new();
    let mut capabilities = Vec::new();
    let mut index_only_capabilities = Vec::new();
    let mut section_payloads = vec![CoviSectionPayloadV2 {
        section_id: 1,
        section_kind: CoviSectionKindV2::StringTable,
        payload: file_digest,
        item_count: 1,
        required_features: 0,
        optional_features: 0,
    }];
    let mut root_id = 0u32;

    for object_type in &surface.object_types {
        let records = surface
            .records
            .iter()
            .filter(|record| record.object_type_id == object_type.object_type_id)
            .collect::<Vec<_>>();
        let value_count = u64::try_from(records.len()).map_err(|_| CoveError::ArithOverflow)?;
        let properties = object_properties_for_index(object_type, &records);
        for property in &properties {
            let current_root_id = root_id;
            root_id = root_id.checked_add(1).ok_or(CoveError::ArithOverflow)?;
            let key_section_id = 2 + current_root_id * 4;
            let entry_section_id = key_section_id + 1;
            let postings_section_id = key_section_id + 2;
            let aggregate_section_id = key_section_id + 3;
            let mut built_index = build_object_property_index(
                current_root_id,
                &records,
                property,
                options.include_index_only_counts.then_some(current_root_id),
            )?;
            let include_min_max_answers = options.include_index_only_min_max
                && built_index.supports_range
                && aggregate_min_max_answer_supported(property.logical_type)
                && built_index.min_key.is_some()
                && built_index.max_key.is_some();
            let include_sum_avg_answers = options.include_index_only_sum_avg
                && aggregate_sum_answer_supported(property.logical_type)
                && (built_index.sum_payload.is_some() || built_index.null_count == value_count);
            let include_aggregate_block = options.include_index_only_counts
                || include_min_max_answers
                || options.include_index_only_distinct_count
                || options.include_index_only_exists
                || include_sum_avg_answers;
            let aggregate_block_section_id = if include_aggregate_block {
                aggregate_section_id
            } else {
                u32::MAX
            };
            if include_aggregate_block {
                built_index.entry_block =
                    entry_block_with_aggregate_block_id(&built_index.entry_block, current_root_id)?;
                append_index_only_capabilities(
                    &mut index_only_capabilities,
                    current_root_id,
                    &table_options,
                    include_min_max_answers,
                    include_sum_avg_answers,
                );
            }

            roots.push(CoviIndexRootV2 {
                index_root_id: current_root_id,
                indexed_target_kind: CoviIndexedTargetKindV2::ObjectProperty,
                index_kind: built_index.index_kind,
                coverage_granularity: CoverageGranularityV2::RowRange as u8,
                proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
                exactness: CoverageExactnessV2::Exact as u8,
                flags: 0,
                table_id: u32::MAX,
                column_id: u32::MAX,
                object_type_id: object_type.object_type_id,
                property_id: property.property_id,
                path_ref: u32::MAX,
                semantic_dimension_ref: u32::MAX,
                logical_type: property.logical_type as u16,
                physical_kind: property.physical_kind as u8,
                key_encoding_kind: built_index.key_encoding_kind as u8,
                comparator_kind: built_index.comparator_kind as u16,
                collation_id: property.collation_id,
                null_semantics: 0,
                sort_order: 0,
                value_count,
                distinct_count: built_index.distinct_count,
                null_count: built_index.null_count,
                min_key_ref: built_index.min_key_ref,
                max_key_ref: built_index.max_key_ref,
                key_block_section_id: key_section_id,
                entry_block_section_id: entry_section_id,
                postings_block_section_id: postings_section_id,
                aggregate_block_section_id,
                coverage_set_ref: u32::MAX,
                capability_ref: current_root_id,
                snapshot_validity_ref: 0,
                checksum: 0,
            });
            capabilities.push(IndexCapabilityV2 {
                capability_id: current_root_id,
                index_root_id: current_root_id,
                flags: 0,
                supports_eq: 1,
                supports_range: u8::from(built_index.supports_range),
                supports_membership: 1,
                supports_prefix: 0,
                supports_contains: 0,
                supports_count: u8::from(
                    options.include_index_only_counts
                        || options.include_index_only_exists
                        || include_sum_avg_answers,
                ),
                supports_min: u8::from(include_min_max_answers),
                supports_max: u8::from(include_min_max_answers),
                supports_sum: u8::from(include_sum_avg_answers),
                supports_distinct_count: u8::from(options.include_index_only_distinct_count),
                supports_join_coverage: 0,
                supports_index_only: u8::from(include_aggregate_block),
                exactness: IndexCapabilityExactnessV2::Exact,
                proof_strength: CoverageProofStrengthV2::ExactConservative,
                null_semantics: 0,
                reserved: 0,
                snapshot_validity_ref: 0,
                coverage_provider_ref: u32::MAX,
                checksum: 0,
            });
            section_payloads.extend([
                CoviSectionPayloadV2 {
                    section_id: key_section_id,
                    section_kind: CoviSectionKindV2::KeyBlock,
                    payload: built_index.key_block,
                    item_count: built_index.distinct_count,
                    required_features: 0,
                    optional_features: 0,
                },
                CoviSectionPayloadV2 {
                    section_id: entry_section_id,
                    section_kind: CoviSectionKindV2::EntryBlock,
                    payload: built_index.entry_block,
                    item_count: built_index.distinct_count,
                    required_features: 0,
                    optional_features: 0,
                },
                CoviSectionPayloadV2 {
                    section_id: postings_section_id,
                    section_kind: CoviSectionKindV2::PostingsBlock,
                    payload: built_index.postings_block,
                    item_count: built_index.distinct_count,
                    required_features: 0,
                    optional_features: 0,
                },
            ]);
            if include_aggregate_block {
                let aggregate_block = aggregate_answer_block(AggregateAnswerBlockParams {
                    root_id: current_root_id,
                    row_count: value_count,
                    null_count: built_index.null_count,
                    distinct_count: built_index.distinct_count,
                    logical_type: property.logical_type,
                    options: &table_options,
                    include_min_max_answers,
                    include_sum_avg_answers,
                    min_key: built_index.min_key.as_deref(),
                    max_key: built_index.max_key.as_deref(),
                    sum_payload: built_index.sum_payload.as_deref(),
                })?;
                section_payloads.push(CoviSectionPayloadV2 {
                    section_id: aggregate_section_id,
                    section_kind: CoviSectionKindV2::AggregateAnswerBlock,
                    payload: aggregate_block,
                    item_count: aggregate_answer_count(
                        &table_options,
                        include_min_max_answers,
                        include_sum_avg_answers,
                    ) as u64,
                    required_features: 0,
                    optional_features: 0,
                });
            }
        }
        if options.include_object_paths {
            append_cove_o_auxiliary_index(
                &mut roots,
                &mut capabilities,
                &mut section_payloads,
                &mut root_id,
                CoviIndexedTargetKindV2::ObjectPath,
                object_type.object_type_id,
                u32::MAX,
                0,
                u32::MAX,
                &records,
                |record| Some(format!("{}/{}", object_type.type_name, hex16(&record.goid))),
            )?;
        }
        if options.include_projection_fragments {
            append_cove_o_auxiliary_index(
                &mut roots,
                &mut capabilities,
                &mut section_payloads,
                &mut root_id,
                CoviIndexedTargetKindV2::ProjectionColumn,
                object_type.object_type_id,
                u32::MAX,
                0,
                u32::MAX,
                &records,
                |record| Some(format!("{}:{}", object_type.type_name, record.segment_id)),
            )?;
        }
        if options.include_association_endpoints {
            append_cove_o_auxiliary_index(
                &mut roots,
                &mut capabilities,
                &mut section_payloads,
                &mut root_id,
                CoviIndexedTargetKindV2::AssociationEndpoint,
                object_type.object_type_id,
                u32::MAX,
                1,
                u32::MAX,
                &records,
                |record| {
                    record
                        .association
                        .as_ref()
                        .and_then(|association| association.source_goid.clone())
                },
            )?;
            append_cove_o_auxiliary_index(
                &mut roots,
                &mut capabilities,
                &mut section_payloads,
                &mut root_id,
                CoviIndexedTargetKindV2::AssociationEndpoint,
                object_type.object_type_id,
                u32::MAX,
                2,
                u32::MAX,
                &records,
                |record| {
                    record
                        .association
                        .as_ref()
                        .and_then(|association| association.target_goid.clone())
                },
            )?;
        }
        if options.include_evidence_lookup {
            append_cove_o_auxiliary_index(
                &mut roots,
                &mut capabilities,
                &mut section_payloads,
                &mut root_id,
                CoviIndexedTargetKindV2::AssociationEndpoint,
                object_type.object_type_id,
                u32::MAX,
                3,
                u32::MAX,
                &records,
                |record| {
                    record
                        .association
                        .as_ref()
                        .and_then(|association| association.evidence_ref.clone())
                },
            )?;
        }
    }

    if !index_only_capabilities.is_empty() {
        let section_id = 2u32
            .checked_add(root_id.checked_mul(4).ok_or(CoveError::ArithOverflow)?)
            .ok_or(CoveError::ArithOverflow)?;
        section_payloads.push(CoviSectionPayloadV2 {
            section_id,
            section_kind: CoviSectionKindV2::IndexOnlyCapabilities,
            payload: index_only_capability_payload(&index_only_capabilities)?,
            item_count: index_only_capabilities.len() as u64,
            required_features: 0,
            optional_features: FEATURE_INDEX_ONLY_CAPABILITY,
        });
    }

    let bytes = CoviArtifactV2::serialize_with_sections(
        dataset_id,
        snapshot_id,
        &[referenced_file],
        &[snapshot_validity],
        &roots,
        &capabilities,
        &section_payloads,
    )?;
    CoviArtifactV2::parse(&bytes)?;
    Ok(bytes)
}

pub fn build_projection_columns_covi_for_cove_o_bytes(
    input: &[u8],
    columns: &[CoviProjectionColumnInput],
) -> Result<Vec<u8>, CoveError> {
    let postscript = CovePostscriptV1::parse_from_tail(input)?;
    let validation = validate_bytes_with_options(
        input,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )?;

    let dataset_id = validation.validated.header.file_id;
    let snapshot_id = derived_snapshot_id(input, postscript.footer.crc32c);
    let file_digest = compute_digest(DigestAlgorithm::Sha256, input)?;
    let referenced_file = CoviReferencedFileV2 {
        file_ref: 0,
        flags: 0,
        file_id: validation.validated.header.file_id,
        file_len: input.len() as u64,
        footer_crc32c: postscript.footer.crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: file_digest
            .len()
            .try_into()
            .map_err(|_| CoveError::ArithOverflow)?,
        digest_offset: 0,
        uri_ref: u32::MAX,
        schema_fingerprint_ref: u32::MAX,
        checksum: 0,
    };
    let snapshot_validity = CoviSnapshotValidityV2 {
        snapshot_validity_ref: 0,
        dataset_id,
        snapshot_id,
        schema_fingerprint_ref: u32::MAX,
        semantic_map_fingerprint_ref: u32::MAX,
        external_visibility_ref: u32::MAX,
        data_checksum_root_ref: u32::MAX,
        valid_from_us: validation.validated.header.created_at_us,
        valid_until_us: i64::MAX,
        flags: 0,
        checksum: 0,
    };

    let mut roots = Vec::new();
    let mut capabilities = Vec::new();
    let mut section_payloads = vec![CoviSectionPayloadV2 {
        section_id: 1,
        section_kind: CoviSectionKindV2::StringTable,
        payload: file_digest,
        item_count: 1,
        required_features: 0,
        optional_features: 0,
    }];
    let mut root_id = 0u32;
    for column in columns {
        append_projection_column_index(
            &mut roots,
            &mut capabilities,
            &mut section_payloads,
            &mut root_id,
            column,
        )?;
    }

    let bytes = CoviArtifactV2::serialize_with_sections(
        dataset_id,
        snapshot_id,
        &[referenced_file],
        &[snapshot_validity],
        &roots,
        &capabilities,
        &section_payloads,
    )?;
    CoviArtifactV2::parse(&bytes)?;
    Ok(bytes)
}

pub fn projection_column_lookup_key_for_json_value(
    value: &Value,
    logical_type: CoveLogicalType,
) -> Result<Option<CoviLookupKeyV2>, CoveError> {
    match projection_column_key_mode(logical_type) {
        ProjectionColumnKeyMode::Canonical => key_for_json_property_value(value, logical_type)
            .map(|key| key.map(CoviLookupKeyV2::CanonicalValueBytes)),
        ProjectionColumnKeyMode::NumCode => projection_numcode_for_json_value(value, logical_type)
            .map(|key| key.map(CoviLookupKeyV2::NumCode)),
    }
}

#[allow(clippy::too_many_arguments)]
fn append_cove_o_auxiliary_index(
    roots: &mut Vec<CoviIndexRootV2>,
    capabilities: &mut Vec<IndexCapabilityV2>,
    section_payloads: &mut Vec<CoviSectionPayloadV2>,
    root_id: &mut u32,
    target_kind: CoviIndexedTargetKindV2,
    object_type_id: u32,
    property_id: u32,
    path_ref: u32,
    semantic_dimension_ref: u32,
    records: &[&CoveObjectRecord],
    key_fn: impl Fn(&CoveObjectRecord) -> Option<String>,
) -> Result<(), CoveError> {
    let current_root_id = *root_id;
    *root_id = root_id.checked_add(1).ok_or(CoveError::ArithOverflow)?;
    let key_section_id = 2 + current_root_id * 4;
    let entry_section_id = key_section_id + 1;
    let postings_section_id = key_section_id + 2;
    let mut keys = KeyPostingMap::new();
    let mut indexed_rows = 0u64;
    for record in records {
        let Some(key_text) = key_fn(record) else {
            continue;
        };
        indexed_rows = indexed_rows
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        let key = tagged_canonical(CanonicalValue::Utf8(&key_text))?;
        append_row_range(
            keys.entry(key).or_default(),
            CoviRowRangePostingV2 {
                file_ref: 0,
                table_id: u32::MAX,
                segment_id: record.segment_id,
                morsel_id: u32::MAX,
                row_start: u64::from(record.row_index),
                row_count: 1,
                flags: 0,
                checksum: 0,
            },
        )?;
    }
    let value_count = u64::try_from(records.len()).map_err(|_| CoveError::ArithOverflow)?;
    let null_count = value_count
        .checked_sub(indexed_rows)
        .ok_or(CoveError::ArithOverflow)?;
    let built_index = build_blocks_from_keys(
        keys,
        BuildBlocksParams {
            root_id: current_root_id,
            logical_type: CoveLogicalType::Utf8,
            index_kind: CoviIndexKindV2::Sorted,
            key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
            comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
            supports_range: true,
            null_count,
            aggregate_block_id: None,
        },
    )?;
    roots.push(CoviIndexRootV2 {
        index_root_id: current_root_id,
        indexed_target_kind: target_kind,
        index_kind: built_index.index_kind,
        coverage_granularity: CoverageGranularityV2::RowRange as u8,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: CoverageExactnessV2::Exact as u8,
        flags: 0,
        table_id: u32::MAX,
        column_id: u32::MAX,
        object_type_id,
        property_id,
        path_ref,
        semantic_dimension_ref,
        logical_type: CoveLogicalType::Utf8 as u16,
        physical_kind: CovePhysicalKind::VarBytes as u8,
        key_encoding_kind: built_index.key_encoding_kind as u8,
        comparator_kind: built_index.comparator_kind as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count,
        distinct_count: built_index.distinct_count,
        null_count: built_index.null_count,
        min_key_ref: built_index.min_key_ref,
        max_key_ref: built_index.max_key_ref,
        key_block_section_id: key_section_id,
        entry_block_section_id: entry_section_id,
        postings_block_section_id: postings_section_id,
        aggregate_block_section_id: u32::MAX,
        coverage_set_ref: u32::MAX,
        capability_ref: current_root_id,
        snapshot_validity_ref: 0,
        checksum: 0,
    });
    capabilities.push(IndexCapabilityV2 {
        capability_id: current_root_id,
        index_root_id: current_root_id,
        flags: 0,
        supports_eq: 1,
        supports_range: 1,
        supports_membership: 1,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: 0,
        supports_min: 0,
        supports_max: 0,
        supports_sum: 0,
        supports_distinct_count: 0,
        supports_join_coverage: 0,
        supports_index_only: 0,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    });
    section_payloads.extend([
        CoviSectionPayloadV2 {
            section_id: key_section_id,
            section_kind: CoviSectionKindV2::KeyBlock,
            payload: built_index.key_block,
            item_count: built_index.distinct_count,
            required_features: 0,
            optional_features: 0,
        },
        CoviSectionPayloadV2 {
            section_id: entry_section_id,
            section_kind: CoviSectionKindV2::EntryBlock,
            payload: built_index.entry_block,
            item_count: built_index.distinct_count,
            required_features: 0,
            optional_features: 0,
        },
        CoviSectionPayloadV2 {
            section_id: postings_section_id,
            section_kind: CoviSectionKindV2::PostingsBlock,
            payload: built_index.postings_block,
            item_count: built_index.distinct_count,
            required_features: 0,
            optional_features: 0,
        },
    ]);
    Ok(())
}

fn append_projection_column_index(
    roots: &mut Vec<CoviIndexRootV2>,
    capabilities: &mut Vec<IndexCapabilityV2>,
    section_payloads: &mut Vec<CoviSectionPayloadV2>,
    root_id: &mut u32,
    column: &CoviProjectionColumnInput,
) -> Result<(), CoveError> {
    let current_root_id = *root_id;
    *root_id = root_id.checked_add(1).ok_or(CoveError::ArithOverflow)?;
    let key_section_id = 2 + current_root_id * 4;
    let entry_section_id = key_section_id + 1;
    let postings_section_id = key_section_id + 2;
    let key_mode = projection_column_key_mode(column.logical_type);
    let mut keys = KeyPostingMap::new();
    let mut indexed_rows = 0u64;
    let mut has_nan = false;
    for (row_index, value) in column.values.iter().enumerate() {
        let Some(key) = projection_column_key(value, column.logical_type, key_mode)? else {
            continue;
        };
        if matches!(
            column.logical_type,
            CoveLogicalType::Float32 | CoveLogicalType::Float64
        ) {
            has_nan |= value.as_f64().is_some_and(f64::is_nan);
        }
        indexed_rows = indexed_rows
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        append_row_range(
            keys.entry(key).or_default(),
            CoviRowRangePostingV2 {
                file_ref: 0,
                table_id: column.table_id,
                segment_id: u32::MAX,
                morsel_id: u32::MAX,
                row_start: u64::try_from(row_index).map_err(|_| CoveError::ArithOverflow)?,
                row_count: 1,
                flags: 0,
                checksum: 0,
            },
        )?;
    }
    let value_count = u64::try_from(column.values.len()).map_err(|_| CoveError::ArithOverflow)?;
    let null_count = value_count
        .checked_sub(indexed_rows)
        .ok_or(CoveError::ArithOverflow)?;
    let supports_range = key_mode == ProjectionColumnKeyMode::NumCode && !has_nan;
    let built_index = build_blocks_from_keys(
        keys,
        BuildBlocksParams {
            root_id: current_root_id,
            logical_type: column.logical_type,
            index_kind: if supports_range {
                CoviIndexKindV2::Sorted
            } else {
                CoviIndexKindV2::Hash
            },
            key_encoding_kind: match key_mode {
                ProjectionColumnKeyMode::NumCode => CoviKeyEncodingKindV2::NumCode,
                ProjectionColumnKeyMode::Canonical => CoviKeyEncodingKindV2::CanonicalValueBytes,
            },
            comparator_kind: match key_mode {
                ProjectionColumnKeyMode::NumCode => CoviComparatorKindV2::NumCodeLogicalOrdering,
                ProjectionColumnKeyMode::Canonical => CoviComparatorKindV2::CanonicalEquality,
            },
            supports_range,
            null_count,
            aggregate_block_id: None,
        },
    )?;
    roots.push(CoviIndexRootV2 {
        index_root_id: current_root_id,
        indexed_target_kind: CoviIndexedTargetKindV2::ProjectionColumn,
        index_kind: built_index.index_kind,
        coverage_granularity: CoverageGranularityV2::RowRange as u8,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: CoverageExactnessV2::Exact as u8,
        flags: 0,
        table_id: column.table_id,
        column_id: column.column_id,
        object_type_id: u32::MAX,
        property_id: u32::MAX,
        path_ref: u32::MAX,
        semantic_dimension_ref: u32::MAX,
        logical_type: column.logical_type as u16,
        physical_kind: column.physical_kind as u8,
        key_encoding_kind: built_index.key_encoding_kind as u8,
        comparator_kind: built_index.comparator_kind as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count,
        distinct_count: built_index.distinct_count,
        null_count: built_index.null_count,
        min_key_ref: built_index.min_key_ref,
        max_key_ref: built_index.max_key_ref,
        key_block_section_id: key_section_id,
        entry_block_section_id: entry_section_id,
        postings_block_section_id: postings_section_id,
        aggregate_block_section_id: u32::MAX,
        coverage_set_ref: u32::MAX,
        capability_ref: current_root_id,
        snapshot_validity_ref: 0,
        checksum: 0,
    });
    capabilities.push(IndexCapabilityV2 {
        capability_id: current_root_id,
        index_root_id: current_root_id,
        flags: 0,
        supports_eq: 1,
        supports_range: u8::from(supports_range),
        supports_membership: 1,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: 0,
        supports_min: 0,
        supports_max: 0,
        supports_sum: 0,
        supports_distinct_count: 0,
        supports_join_coverage: 0,
        supports_index_only: 0,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    });
    section_payloads.extend([
        CoviSectionPayloadV2 {
            section_id: key_section_id,
            section_kind: CoviSectionKindV2::KeyBlock,
            payload: built_index.key_block,
            item_count: built_index.distinct_count,
            required_features: 0,
            optional_features: 0,
        },
        CoviSectionPayloadV2 {
            section_id: entry_section_id,
            section_kind: CoviSectionKindV2::EntryBlock,
            payload: built_index.entry_block,
            item_count: built_index.distinct_count,
            required_features: 0,
            optional_features: 0,
        },
        CoviSectionPayloadV2 {
            section_id: postings_section_id,
            section_kind: CoviSectionKindV2::PostingsBlock,
            payload: built_index.postings_block,
            item_count: built_index.distinct_count,
            required_features: 0,
            optional_features: 0,
        },
    ]);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectionColumnKeyMode {
    Canonical,
    NumCode,
}

fn projection_column_key_mode(logical_type: CoveLogicalType) -> ProjectionColumnKeyMode {
    match logical_type {
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64
        | CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64
        | CoveLogicalType::Float32
        | CoveLogicalType::Float64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::DateDays
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => ProjectionColumnKeyMode::NumCode,
        _ => ProjectionColumnKeyMode::Canonical,
    }
}

fn projection_column_key(
    value: &Value,
    logical_type: CoveLogicalType,
    mode: ProjectionColumnKeyMode,
) -> Result<Option<Vec<u8>>, CoveError> {
    match mode {
        ProjectionColumnKeyMode::Canonical => key_for_json_property_value(value, logical_type),
        ProjectionColumnKeyMode::NumCode => projection_numcode_for_json_value(value, logical_type)
            .map(|value| value.map(|code| code.to_le_bytes().to_vec())),
    }
}

fn projection_numcode_for_json_value(
    value: &Value,
    logical_type: CoveLogicalType,
) -> Result<Option<u64>, CoveError> {
    if value.is_null() {
        return Ok(None);
    }
    let key = match logical_type {
        CoveLogicalType::Int8 => json_i64(value)
            .and_then(|value| i8::try_from(value).ok())
            .map(|value| value as u8 as u64),
        CoveLogicalType::Int16 => json_i64(value)
            .and_then(|value| i16::try_from(value).ok())
            .map(|value| value as u16 as u64),
        CoveLogicalType::Int32 => json_i64(value)
            .and_then(|value| i32::try_from(value).ok())
            .map(|value| value as u32 as u64),
        CoveLogicalType::Int64 => json_i64(value).map(|value| value as u64),
        CoveLogicalType::UInt8 => json_u64(value)
            .and_then(|value| u8::try_from(value).ok())
            .map(u64::from),
        CoveLogicalType::UInt16 => json_u64(value)
            .and_then(|value| u16::try_from(value).ok())
            .map(u64::from),
        CoveLogicalType::UInt32 => json_u64(value)
            .and_then(|value| u32::try_from(value).ok())
            .map(u64::from),
        CoveLogicalType::UInt64 => json_u64(value),
        CoveLogicalType::Float32 => json_f64(value).map(|value| (value as f32).to_bits() as u64),
        CoveLogicalType::Float64 => json_f64(value).map(|value| value.to_bits()),
        CoveLogicalType::Decimal64 => json_i64(value).map(|value| value as u64),
        CoveLogicalType::DateDays => json_i64(value)
            .and_then(|value| i32::try_from(value).ok())
            .map(|value| value as u32 as u64),
        CoveLogicalType::TimestampMicros | CoveLogicalType::TimestampNanos => {
            json_i64(value).map(|value| value as u64)
        }
        _ => return Err(CoveError::BadCovi),
    };
    Ok(key)
}

fn append_index_only_capabilities(
    out: &mut Vec<IndexOnlyCapabilityV2>,
    capability_id: u32,
    options: &CoviBuildOptions,
    include_min_max_answers: bool,
    include_sum_avg_answers: bool,
) {
    if options.include_index_only_counts {
        out.push(index_only_capability(
            capability_id,
            CoviAggregateKindV2::Count,
        ));
    }
    if options.include_index_only_exists {
        out.push(index_only_capability(
            capability_id,
            CoviAggregateKindV2::Exists,
        ));
    }
    if include_min_max_answers {
        out.push(index_only_capability(
            capability_id,
            CoviAggregateKindV2::Min,
        ));
        out.push(index_only_capability(
            capability_id,
            CoviAggregateKindV2::Max,
        ));
    }
    if options.include_index_only_distinct_count {
        out.push(index_only_capability(
            capability_id,
            CoviAggregateKindV2::DistinctCount,
        ));
    }
    if include_sum_avg_answers {
        out.push(index_only_capability(
            capability_id,
            CoviAggregateKindV2::Sum,
        ));
        out.push(index_only_capability(
            capability_id,
            CoviAggregateKindV2::Avg,
        ));
    }
}

fn index_only_capability(
    capability_id: u32,
    aggregate_kind: CoviAggregateKindV2,
) -> IndexOnlyCapabilityV2 {
    IndexOnlyCapabilityV2 {
        capability_id,
        aggregate_kind: aggregate_kind as u16,
        predicate_supported: 0,
        exactness: IndexCapabilityExactnessV2::Exact,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref: 0,
        required_visibility_overlay_ref: u32::MAX,
        checksum: 0,
    }
}

fn index_only_capability_payload(records: &[IndexOnlyCapabilityV2]) -> Result<Vec<u8>, CoveError> {
    let mut payload = Vec::with_capacity(
        records
            .len()
            .checked_mul(IndexOnlyCapabilityV2::LEN)
            .ok_or(CoveError::ArithOverflow)?,
    );
    for record in records {
        payload.extend_from_slice(&record.serialize()?);
    }
    Ok(payload)
}

fn indexed_target_metadata(
    target: CoviBuildTarget,
    table_id: u32,
    column_id: u32,
) -> IndexedTargetMetadata {
    match target {
        CoviBuildTarget::TableColumn => IndexedTargetMetadata {
            indexed_target_kind: CoviIndexedTargetKindV2::TableColumn,
            table_id,
            column_id,
            object_type_id: u32::MAX,
            property_id: u32::MAX,
            path_ref: u32::MAX,
            semantic_dimension_ref: u32::MAX,
        },
        CoviBuildTarget::ProjectionColumn => IndexedTargetMetadata {
            indexed_target_kind: CoviIndexedTargetKindV2::ProjectionColumn,
            table_id,
            column_id,
            object_type_id: u32::MAX,
            property_id: u32::MAX,
            path_ref: u32::MAX,
            semantic_dimension_ref: u32::MAX,
        },
        CoviBuildTarget::ObjectProperty {
            object_type_id,
            property_id,
        } => IndexedTargetMetadata {
            indexed_target_kind: CoviIndexedTargetKindV2::ObjectProperty,
            table_id: u32::MAX,
            column_id: u32::MAX,
            object_type_id,
            property_id,
            path_ref: u32::MAX,
            semantic_dimension_ref: u32::MAX,
        },
        CoviBuildTarget::ObjectPath {
            object_type_id,
            path_ref,
        } => IndexedTargetMetadata {
            indexed_target_kind: CoviIndexedTargetKindV2::ObjectPath,
            table_id: u32::MAX,
            column_id: u32::MAX,
            object_type_id,
            property_id: u32::MAX,
            path_ref,
            semantic_dimension_ref: u32::MAX,
        },
        CoviBuildTarget::SemanticDimension {
            semantic_dimension_ref,
        } => IndexedTargetMetadata {
            indexed_target_kind: CoviIndexedTargetKindV2::SemanticDimension,
            table_id: u32::MAX,
            column_id: u32::MAX,
            object_type_id: u32::MAX,
            property_id: u32::MAX,
            path_ref: u32::MAX,
            semantic_dimension_ref,
        },
        CoviBuildTarget::DimensionalTuple {
            semantic_dimension_ref,
        } => IndexedTargetMetadata {
            indexed_target_kind: CoviIndexedTargetKindV2::DimensionalTuple,
            table_id: u32::MAX,
            column_id: u32::MAX,
            object_type_id: u32::MAX,
            property_id: u32::MAX,
            path_ref: u32::MAX,
            semantic_dimension_ref,
        },
    }
}

fn selected_columns<'a>(
    table: &'a TableEntry,
    options: &CoviBuildOptions,
) -> Result<Vec<&'a ColumnEntry>, CoveError> {
    if options.all_columns {
        return Ok(table.columns.iter().collect());
    }
    if options.column_ids.is_empty() {
        return table
            .columns
            .first()
            .map(|column| vec![column])
            .ok_or_else(|| CoveError::BadSchema("selected table has no columns".into()));
    }
    let mut columns = Vec::new();
    for column_id in &options.column_ids {
        let column = table
            .columns
            .iter()
            .find(|column| column.column_id == *column_id)
            .ok_or_else(|| {
                CoveError::BadSchema(format!(
                    "column_id {column_id} not found in table {}",
                    table.table_id
                ))
            })?;
        columns.push(column);
    }
    Ok(columns)
}

struct BuiltColumnIndex {
    key_block: Vec<u8>,
    entry_block: Vec<u8>,
    postings_block: Vec<u8>,
    index_kind: CoviIndexKindV2,
    key_encoding_kind: CoviKeyEncodingKindV2,
    comparator_kind: CoviComparatorKindV2,
    supports_range: bool,
    distinct_count: u64,
    null_count: u64,
    min_key_ref: u32,
    max_key_ref: u32,
    min_key: Option<Vec<u8>>,
    max_key: Option<Vec<u8>>,
    sum_payload: Option<Vec<u8>>,
}

fn build_column_index(
    root_id: u32,
    input: &[u8],
    mounted: &MountedCoveFile,
    table: &TableEntry,
    column: &ColumnEntry,
    aggregate_block_id: Option<u32>,
) -> Result<BuiltColumnIndex, CoveError> {
    let raw_filecode_mode =
        column.physical == CovePhysicalKind::FileCode && mounted.dictionary.is_none();
    let mut key_encoding_kind = if raw_filecode_mode {
        CoviKeyEncodingKindV2::FileCode
    } else {
        CoviKeyEncodingKindV2::CanonicalValueBytes
    };
    let mut comparator_kind = if raw_filecode_mode {
        CoviComparatorKindV2::DomainRankOrdering
    } else {
        CoviComparatorKindV2::CanonicalOrdering
    };
    let mut index_kind = if raw_filecode_mode {
        CoviIndexKindV2::Hash
    } else {
        CoviIndexKindV2::Sorted
    };
    let mut supports_range = !raw_filecode_mode;
    let mut keys: KeyPostingMap = BTreeMap::new();
    let mut null_count = 0u64;
    let mut rows_seen = 0u64;
    let zone_stats_entries = mounted
        .zone_stats
        .iter()
        .flat_map(|section| section.entries.iter().cloned())
        .collect::<Vec<_>>();

    for section in mounted
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::TableSegmentData as u16)
    {
        let segment_bytes = section_payload(input, section)?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            &segment_bytes,
            mounted.header.required_features,
        )?;
        if segment.header.table_id != table.table_id {
            continue;
        }
        let Some(column_dir) = segment
            .columns
            .iter()
            .find(|entry| entry.column_id == column.column_id)
        else {
            continue;
        };
        let page_index_start =
            usize::try_from(column_dir.page_index_offset).map_err(|_| CoveError::OffsetRange)?;
        let page_index_len =
            usize::try_from(column_dir.page_index_length).map_err(|_| CoveError::OffsetRange)?;
        let page_index_end = page_index_start
            .checked_add(page_index_len)
            .ok_or(CoveError::ArithOverflow)?;
        let page_index = ColumnPageIndex::parse(&segment_bytes[page_index_start..page_index_end])?;
        for page in page_index.entries {
            let morsel = segment.morsels.morsel_by_id(page.morsel_id)?;
            let decoded_page = if page_uses_payload_elision(page.flags) && page.page_length == 0 {
                if page.null_count == page.row_count {
                    null_count = null_count
                        .checked_add(u64::from(page.row_count))
                        .ok_or(CoveError::ArithOverflow)?;
                    rows_seen = rows_seen
                        .checked_add(u64::from(page.row_count))
                        .ok_or(CoveError::ArithOverflow)?;
                    continue;
                }
                Cow::Owned(
                    materialize_stats_only_constant_page_payload(
                        StatsOnlyPageMaterializationContext {
                            table_id: Some(table.table_id),
                            segment_id: Some(segment.header.segment_id),
                            column_id: column.column_id,
                            logical_type: column.logical,
                            physical_kind: column.physical,
                            dictionary_len: mounted
                                .dictionary
                                .as_ref()
                                .map(|dictionary| dictionary.len()),
                            zone_stats: &zone_stats_entries,
                        },
                        &page,
                    )
                    .map_err(|_| CoveError::BadCovi)?,
                )
            } else {
                let page_start =
                    usize::try_from(page.page_offset).map_err(|_| CoveError::OffsetRange)?;
                let page_len =
                    usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?;
                let page_end = page_start
                    .checked_add(page_len)
                    .ok_or(CoveError::ArithOverflow)?;
                let page_wire = &segment_bytes[page_start..page_end];
                column_page_payload(page_wire, &page)?
            };
            let payload = ColumnPagePayloadV1::parse(decoded_page.as_ref())?;
            let root = payload.root_node()?;
            let values = payload.buffer_bytes(PageBufferKind::Values)?.unwrap_or(&[]);
            let validity = payload
                .buffer_bytes(PageBufferKind::NullBitmap)?
                .map(|bytes| ValidityBitmap::new(bytes, u64::from(page.row_count)));
            let array = EncodedArray::new(
                root.logical_type,
                root.physical_kind,
                u64::from(page.row_count),
                root.encoding_kind,
                validity,
                values,
                None,
            );
            let decoded_rows = array.decode_all_rows()?;
            for (row_index, value) in decoded_rows.into_iter().enumerate() {
                rows_seen = rows_seen.checked_add(1).ok_or(CoveError::ArithOverflow)?;
                let Some(key) = key_for_value(
                    value,
                    column.logical,
                    mounted.dictionary.as_ref(),
                    raw_filecode_mode,
                )?
                else {
                    null_count = null_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
                    continue;
                };
                let row_start = segment
                    .header
                    .row_start
                    .checked_add(u64::from(morsel.first_row_in_segment))
                    .and_then(|start| start.checked_add(row_index as u64))
                    .ok_or(CoveError::ArithOverflow)?;
                append_row_range(
                    keys.entry(key).or_default(),
                    CoviRowRangePostingV2 {
                        file_ref: 0,
                        table_id: table.table_id,
                        segment_id: segment.header.segment_id,
                        morsel_id: page.morsel_id,
                        row_start,
                        row_count: 1,
                        flags: 0,
                        checksum: 0,
                    },
                )?;
            }
        }
    }

    if rows_seen != table.row_count {
        return Err(CoveError::BadCovi);
    }
    if canonical_float_keys_contain_nan(column.logical, &keys)? {
        key_encoding_kind = CoviKeyEncodingKindV2::CanonicalValueBytes;
        comparator_kind = CoviComparatorKindV2::CanonicalEquality;
        index_kind = CoviIndexKindV2::Hash;
        supports_range = false;
    }
    let sum_payload = index_only_sum_payload_from_keys(column.logical, &keys)?;
    let mut built = build_blocks_from_keys(
        keys,
        BuildBlocksParams {
            root_id,
            logical_type: column.logical,
            index_kind,
            key_encoding_kind,
            comparator_kind,
            supports_range,
            null_count,
            aggregate_block_id,
        },
    )?;
    built.sum_payload = sum_payload;
    Ok(built)
}

fn build_object_property_index(
    root_id: u32,
    records: &[&CoveObjectRecord],
    property: &PropertyEntryV1,
    aggregate_block_id: Option<u32>,
) -> Result<BuiltColumnIndex, CoveError> {
    let mut index_kind = CoviIndexKindV2::Sorted;
    let key_encoding_kind = CoviKeyEncodingKindV2::CanonicalValueBytes;
    let mut comparator_kind = CoviComparatorKindV2::CanonicalOrdering;
    let mut supports_range = !matches!(
        property.logical_type,
        CoveLogicalType::Null
            | CoveLogicalType::Bool
            | CoveLogicalType::List
            | CoveLogicalType::Struct
            | CoveLogicalType::Map
    );
    let mut keys: KeyPostingMap = BTreeMap::new();
    let mut rows_with_indexed_value = 0u64;

    for record in records {
        let mut record_keys = BTreeSet::new();
        for value in record
            .properties
            .iter()
            .filter(|value| value.property_id == property.property_id)
        {
            if value.redacted {
                continue;
            }
            if let Some(key) = key_for_json_property_value(&value.value, property.logical_type)? {
                record_keys.insert(key);
            }
        }
        if record_keys.is_empty() {
            continue;
        }
        rows_with_indexed_value = rows_with_indexed_value
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        for key in record_keys {
            append_row_range(
                keys.entry(key).or_default(),
                CoviRowRangePostingV2 {
                    file_ref: 0,
                    table_id: u32::MAX,
                    segment_id: record.segment_id,
                    morsel_id: u32::MAX,
                    row_start: u64::from(record.row_index),
                    row_count: 1,
                    flags: 0,
                    checksum: 0,
                },
            )?;
        }
    }

    if canonical_float_keys_contain_nan(property.logical_type, &keys)? {
        comparator_kind = CoviComparatorKindV2::CanonicalEquality;
        index_kind = CoviIndexKindV2::Hash;
        supports_range = false;
    }
    let value_count = u64::try_from(records.len()).map_err(|_| CoveError::ArithOverflow)?;
    let null_count = value_count
        .checked_sub(rows_with_indexed_value)
        .ok_or(CoveError::ArithOverflow)?;
    let sum_payload = index_only_sum_payload_from_keys(property.logical_type, &keys)?;
    let mut built = build_blocks_from_keys(
        keys,
        BuildBlocksParams {
            root_id,
            logical_type: property.logical_type,
            index_kind,
            key_encoding_kind,
            comparator_kind,
            supports_range,
            null_count,
            aggregate_block_id,
        },
    )?;
    built.sum_payload = sum_payload;
    Ok(built)
}

fn object_properties_for_index(
    object_type: &ObjectTypeEntryV1,
    records: &[&CoveObjectRecord],
) -> Vec<PropertyEntryV1> {
    let mut properties = object_type
        .properties
        .iter()
        .map(|property| (property.property_id, property.clone()))
        .collect::<BTreeMap<_, _>>();
    for record in records {
        for value in &record.properties {
            properties
                .entry(value.property_id)
                .or_insert_with(|| PropertyEntryV1 {
                    property_id: value.property_id,
                    property_name: value.property_name.clone(),
                    logical_type: value.logical_type,
                    physical_kind: value.physical_kind,
                    nullable: true,
                    collation_id: 0,
                    flags: value.flags,
                });
        }
    }
    properties.into_values().collect()
}

fn hex16(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(32);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn build_blocks_from_keys(
    keys: KeyPostingMap,
    params: BuildBlocksParams,
) -> Result<BuiltColumnIndex, CoveError> {
    let BuildBlocksParams {
        root_id,
        logical_type,
        index_kind,
        key_encoding_kind,
        comparator_kind,
        supports_range,
        null_count,
        aggregate_block_id,
    } = params;
    let distinct_count = u64::try_from(keys.len()).map_err(|_| CoveError::ArithOverflow)?;
    let keys = order_key_items(keys, logical_type, index_kind, comparator_kind)?;
    let min_key = supports_range
        .then(|| keys.first().map(|(key, _)| key.clone()))
        .flatten();
    let max_key = supports_range
        .then(|| keys.last().map(|(key, _)| key.clone()))
        .flatten();
    let mut key_data = Vec::new();
    let mut entries = Vec::new();
    let mut postings = Vec::new();
    let mut postings_payload = Vec::new();

    for (entry_index, (key, ranges)) in keys.iter().enumerate() {
        let ranges = normalize_row_ranges(ranges)?;
        let entry_ref = u32::try_from(entry_index).map_err(|_| CoveError::ArithOverflow)?;
        let key_offset = u64::try_from(key_data.len()).map_err(|_| CoveError::ArithOverflow)?;
        let key_length = u32::try_from(key.len()).map_err(|_| CoveError::ArithOverflow)?;
        key_data.extend_from_slice(key);

        let payload_offset = postings_payload.len();
        for range in &ranges {
            postings_payload.extend_from_slice(&range.serialize()?);
        }
        let payload_length = ranges
            .len()
            .checked_mul(CoviRowRangePostingV2::LEN)
            .ok_or(CoveError::ArithOverflow)?;
        postings.push(CoviPostingsHeaderV2 {
            postings_ref: entry_ref,
            index_root_id: root_id,
            representation: CoviPostingRepresentationV2::RowRangeList,
            target_granularity: CoverageGranularityV2::RowRange as u8,
            flags: 0,
            item_count: u64::try_from(ranges.len()).map_err(|_| CoveError::ArithOverflow)?,
            payload_offset: u64::try_from(payload_offset).map_err(|_| CoveError::ArithOverflow)?,
            payload_length: u64::try_from(payload_length).map_err(|_| CoveError::ArithOverflow)?,
            coverage_set_ref: u32::MAX,
            checksum: 0,
        });
        entries.push(CoviIndexEntryV2 {
            entry_ref,
            index_root_id: root_id,
            entry_id: u64::from(entry_ref),
            key_kind: key_encoding_kind,
            comparator_kind,
            flags: 0,
            key_offset,
            key_length,
            key_hash64: stable_hash64(key),
            postings_ref: entry_ref,
            coverage_set_ref: u32::MAX,
            aggregate_answer_ref: u32::MAX,
            next_duplicate_ref: u32::MAX,
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
            key_block_id: root_id,
            index_root_id: root_id,
            key_count: distinct_count,
            encoding_kind: key_encoding_kind,
            comparator_kind,
            flags: 0,
            key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
            key_data_length: key_data.len() as u64,
            checksum: 0,
        },
        key_data,
    }
    .serialize()?;
    let entry_block = CoviEntryBlockV2 {
        header: CoviEntryBlockHeaderV2 {
            magic: CoviEntryBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviEntryBlockHeaderV2::LEN as u16,
            entry_len: CoviIndexEntryV2::LEN as u16,
            entry_block_id: root_id,
            index_root_id: root_id,
            entry_count: entries.len() as u32,
            key_block_id: root_id,
            postings_block_id: root_id,
            aggregate_block_id: aggregate_block_id.unwrap_or(u32::MAX),
            entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
            entries_length: (entries.len() * CoviIndexEntryV2::LEN) as u64,
            flags: 0,
            checksum: 0,
        },
        entries,
    }
    .serialize()?;
    let postings_block = CoviPostingsBlockV2 {
        header: CoviPostingsBlockHeaderV2 {
            magic: CoviPostingsBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviPostingsBlockHeaderV2::LEN as u16,
            postings_header_len: CoviPostingsHeaderV2::LEN as u16,
            postings_block_id: root_id,
            index_root_id: root_id,
            postings_count: postings.len() as u32,
            row_ordinal_set_count: 0,
            postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
            row_ordinal_headers_offset: 0,
            postings_payload_offset: 0,
            postings_payload_length: postings_payload.len() as u64,
            flags: 0,
            checksum: 0,
        },
        postings,
        row_ordinal_sets: Vec::new(),
        payload: postings_payload,
    }
    .serialize()?;
    let max_key_ref = if distinct_count == 0 || !supports_range {
        u32::MAX
    } else {
        u32::try_from(distinct_count - 1).map_err(|_| CoveError::ArithOverflow)?
    };
    Ok(BuiltColumnIndex {
        key_block,
        entry_block,
        postings_block,
        index_kind,
        key_encoding_kind,
        comparator_kind,
        supports_range,
        distinct_count,
        null_count,
        min_key_ref: if distinct_count == 0 || !supports_range {
            u32::MAX
        } else {
            0
        },
        max_key_ref,
        min_key,
        max_key,
        sum_payload: None,
    })
}

fn canonical_float_keys_contain_nan(
    logical_type: CoveLogicalType,
    keys: &KeyPostingMap,
) -> Result<bool, CoveError> {
    if !matches!(
        logical_type,
        CoveLogicalType::Float32 | CoveLogicalType::Float64
    ) {
        return Ok(false);
    }
    for key in keys.keys() {
        if canonical_float_key_is_nan(logical_type, key)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn index_only_sum_payload_from_keys(
    logical_type: CoveLogicalType,
    keys: &KeyPostingMap,
) -> Result<Option<Vec<u8>>, CoveError> {
    if keys.is_empty() || !aggregate_sum_answer_supported(logical_type) {
        return Ok(None);
    }
    match logical_type {
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => index_only_signed_sum_payload(keys),
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => index_only_unsigned_sum_payload(keys),
        _ => Ok(None),
    }
}

fn index_only_signed_sum_payload(keys: &KeyPostingMap) -> Result<Option<Vec<u8>>, CoveError> {
    let mut total = 0i128;
    for (key, ranges) in keys {
        let value = i128::from(signed_i64_payload_from_key(key)?);
        let count = i128::from(row_count_for_ranges(ranges)?);
        let contribution = value.checked_mul(count).ok_or(CoveError::ArithOverflow)?;
        total = total
            .checked_add(contribution)
            .ok_or(CoveError::ArithOverflow)?;
    }
    let total = i64::try_from(total).map_err(|_| CoveError::ArithOverflow)?;
    Ok(Some(total.to_le_bytes().to_vec()))
}

fn index_only_unsigned_sum_payload(keys: &KeyPostingMap) -> Result<Option<Vec<u8>>, CoveError> {
    let mut total = 0u128;
    for (key, ranges) in keys {
        let value = u128::from(unsigned_u64_payload_from_key(key)?);
        let count = u128::from(row_count_for_ranges(ranges)?);
        let contribution = value.checked_mul(count).ok_or(CoveError::ArithOverflow)?;
        total = total
            .checked_add(contribution)
            .ok_or(CoveError::ArithOverflow)?;
    }
    let total = u64::try_from(total).map_err(|_| CoveError::ArithOverflow)?;
    Ok(Some(total.to_le_bytes().to_vec()))
}

fn row_count_for_ranges(ranges: &[CoviRowRangePostingV2]) -> Result<u64, CoveError> {
    ranges.iter().try_fold(0u64, |total, range| {
        total
            .checked_add(range.row_count)
            .ok_or(CoveError::ArithOverflow)
    })
}

fn signed_i64_payload_from_key(key: &[u8]) -> Result<i64, CoveError> {
    let (tag, payload) = tagged_key_payload(key)?;
    if tag != ValueTag::Int64 || payload.len() != 8 {
        return Err(CoveError::BadCovi);
    }
    Ok(i64::from_le_bytes(payload.try_into().unwrap()))
}

fn unsigned_u64_payload_from_key(key: &[u8]) -> Result<u64, CoveError> {
    let (tag, payload) = tagged_key_payload(key)?;
    if tag != ValueTag::UInt64 || payload.len() != 8 {
        return Err(CoveError::BadCovi);
    }
    Ok(u64::from_le_bytes(payload.try_into().unwrap()))
}

fn tagged_key_payload(key: &[u8]) -> Result<(ValueTag, &[u8]), CoveError> {
    let (tag, consumed) = wire::decode_u64_leb128(key)?;
    let tag = u16::try_from(tag)
        .ok()
        .and_then(ValueTag::from_u16)
        .ok_or(CoveError::BadCovi)?;
    Ok((tag, &key[consumed..]))
}

fn canonical_float_key_is_nan(
    logical_type: CoveLogicalType,
    key: &[u8],
) -> Result<bool, CoveError> {
    let (tag, consumed) = wire::decode_u64_leb128(key)?;
    let tag = u16::try_from(tag)
        .ok()
        .and_then(ValueTag::from_u16)
        .ok_or(CoveError::BadCovi)?;
    let payload = &key[consumed..];
    match (logical_type, tag, payload.len()) {
        (CoveLogicalType::Float32, ValueTag::Float32Bits, 4) => {
            let bits = u32::from_le_bytes(payload.try_into().unwrap());
            Ok(f32::from_bits(bits).is_nan())
        }
        (CoveLogicalType::Float64, ValueTag::Float64Bits, 8) => {
            let bits = u64::from_le_bytes(payload.try_into().unwrap());
            Ok(f64::from_bits(bits).is_nan())
        }
        _ => Err(CoveError::BadCovi),
    }
}

fn order_key_items(
    keys: KeyPostingMap,
    logical_type: CoveLogicalType,
    index_kind: CoviIndexKindV2,
    comparator_kind: CoviComparatorKindV2,
) -> Result<OrderedKeyPostings, CoveError> {
    let items = keys.into_iter().collect::<Vec<_>>();
    if !matches!(
        index_kind,
        CoviIndexKindV2::Sorted | CoviIndexKindV2::SparseSorted
    ) {
        return Ok(items);
    }
    let context = CoviLookupComparatorContextV2::default();
    let mut sorted = OrderedKeyPostings::with_capacity(items.len());
    'items: for item in items {
        for index in 0..sorted.len() {
            if compare_key_bytes_for_order(
                comparator_kind as u16,
                Some(logical_type),
                &context,
                &item.0,
                &sorted[index].0,
            )? == Ordering::Less
            {
                sorted.insert(index, item);
                continue 'items;
            }
        }
        sorted.push(item);
    }
    Ok(sorted)
}

fn aggregate_answer_block(params: AggregateAnswerBlockParams<'_>) -> Result<Vec<u8>, CoveError> {
    let AggregateAnswerBlockParams {
        root_id,
        row_count,
        null_count,
        distinct_count,
        logical_type,
        options,
        include_min_max_answers,
        include_sum_avg_answers,
        min_key,
        max_key,
        sum_payload,
    } = params;
    let non_null_count = row_count
        .checked_sub(null_count)
        .ok_or(CoveError::ArithOverflow)?;
    let mut answers = Vec::new();
    let mut payload = Vec::new();
    if options.include_index_only_counts {
        push_aggregate_answer(
            &mut answers,
            &mut payload,
            root_id,
            CoviAggregateKindV2::Count,
            row_count,
            null_count,
            non_null_count,
            None,
        )?;
    }
    if options.include_index_only_exists {
        push_aggregate_answer(
            &mut answers,
            &mut payload,
            root_id,
            CoviAggregateKindV2::Exists,
            row_count,
            null_count,
            non_null_count,
            None,
        )?;
    }
    if include_min_max_answers {
        let min_payload = min_key
            .map(|key| canonical_payload_for_min_max_answer(logical_type, key))
            .transpose()?;
        let max_payload = max_key
            .map(|key| canonical_payload_for_min_max_answer(logical_type, key))
            .transpose()?;
        push_aggregate_answer(
            &mut answers,
            &mut payload,
            root_id,
            CoviAggregateKindV2::Min,
            row_count,
            null_count,
            non_null_count,
            min_payload.as_deref(),
        )?;
        push_aggregate_answer(
            &mut answers,
            &mut payload,
            root_id,
            CoviAggregateKindV2::Max,
            row_count,
            null_count,
            non_null_count,
            max_payload.as_deref(),
        )?;
    }
    if options.include_index_only_distinct_count {
        let distinct_payload = distinct_count.to_le_bytes();
        push_aggregate_answer(
            &mut answers,
            &mut payload,
            root_id,
            CoviAggregateKindV2::DistinctCount,
            row_count,
            null_count,
            non_null_count,
            Some(&distinct_payload),
        )?;
    }
    if include_sum_avg_answers {
        push_aggregate_answer(
            &mut answers,
            &mut payload,
            root_id,
            CoviAggregateKindV2::Sum,
            row_count,
            null_count,
            non_null_count,
            sum_payload,
        )?;
        push_aggregate_answer(
            &mut answers,
            &mut payload,
            root_id,
            CoviAggregateKindV2::Avg,
            row_count,
            null_count,
            non_null_count,
            sum_payload,
        )?;
    }
    CoviAggregateAnswerBlockV2 {
        header: CoviAggregateAnswerBlockHeaderV2 {
            magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
            aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
            aggregate_block_id: root_id,
            index_root_id: root_id,
            aggregate_answer_count: answers.len() as u32,
            aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
            aggregate_payload_offset: 0,
            aggregate_payload_length: payload.len() as u64,
            flags: 0,
            checksum: 0,
        },
        answers,
        payload,
    }
    .serialize()
}

fn aggregate_answer_count(
    options: &CoviBuildOptions,
    include_min_max_answers: bool,
    include_sum_avg_answers: bool,
) -> usize {
    usize::from(options.include_index_only_counts)
        + usize::from(options.include_index_only_exists)
        + usize::from(include_min_max_answers) * 2
        + usize::from(options.include_index_only_distinct_count)
        + usize::from(include_sum_avg_answers) * 2
}

fn aggregate_min_max_answer_supported(logical_type: CoveLogicalType) -> bool {
    !matches!(
        logical_type,
        CoveLogicalType::Null
            | CoveLogicalType::Bool
            | CoveLogicalType::List
            | CoveLogicalType::Struct
            | CoveLogicalType::Map
    )
}

fn aggregate_sum_answer_supported(logical_type: CoveLogicalType) -> bool {
    matches!(
        logical_type,
        CoveLogicalType::Int8
            | CoveLogicalType::Int16
            | CoveLogicalType::Int32
            | CoveLogicalType::Int64
            | CoveLogicalType::UInt8
            | CoveLogicalType::UInt16
            | CoveLogicalType::UInt32
            | CoveLogicalType::UInt64
    )
}

fn entry_block_with_aggregate_block_id(
    entry_block: &[u8],
    aggregate_block_id: u32,
) -> Result<Vec<u8>, CoveError> {
    let mut block = CoviEntryBlockV2::parse(entry_block)?;
    block.header.aggregate_block_id = aggregate_block_id;
    block.serialize()
}

fn canonical_payload_for_min_max_answer(
    logical_type: CoveLogicalType,
    key: &[u8],
) -> Result<Vec<u8>, CoveError> {
    let (tag, consumed) = wire::decode_u64_leb128(key)?;
    let tag = u16::try_from(tag)
        .ok()
        .and_then(ValueTag::from_u16)
        .ok_or(CoveError::BadCovi)?;
    if !value_tag_matches_logical(logical_type, tag) {
        return Err(CoveError::BadCovi);
    }
    Ok(key[consumed..].to_vec())
}

fn value_tag_matches_logical(logical_type: CoveLogicalType, tag: ValueTag) -> bool {
    match logical_type {
        CoveLogicalType::Bool => matches!(tag, ValueTag::BoolFalse | ValueTag::BoolTrue),
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => tag == ValueTag::Int64,
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => tag == ValueTag::UInt64,
        CoveLogicalType::Float32 => tag == ValueTag::Float32Bits,
        CoveLogicalType::Float64 => tag == ValueTag::Float64Bits,
        CoveLogicalType::Decimal64 => tag == ValueTag::Decimal64,
        CoveLogicalType::Decimal128 => tag == ValueTag::Decimal128,
        CoveLogicalType::DateDays => tag == ValueTag::DateDays,
        CoveLogicalType::TimestampMicros => tag == ValueTag::TimestampMicros,
        CoveLogicalType::TimestampNanos => tag == ValueTag::TimestampNanos,
        CoveLogicalType::Utf8 => tag == ValueTag::Utf8,
        CoveLogicalType::Binary => tag == ValueTag::Binary,
        CoveLogicalType::Uuid => tag == ValueTag::Uuid,
        CoveLogicalType::Json => tag == ValueTag::Json,
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_aggregate_answer(
    answers: &mut Vec<CoviAggregateAnswerV2>,
    payload: &mut Vec<u8>,
    root_id: u32,
    aggregate_kind: CoviAggregateKindV2,
    row_count: u64,
    null_count: u64,
    non_null_count: u64,
    value: Option<&[u8]>,
) -> Result<(), CoveError> {
    let value_ref = if let Some(value) = value {
        let offset = u32::try_from(payload.len()).map_err(|_| CoveError::ArithOverflow)?;
        payload.extend_from_slice(value);
        offset
    } else {
        u32::MAX
    };
    answers.push(CoviAggregateAnswerV2 {
        aggregate_answer_ref: u32::try_from(answers.len()).map_err(|_| CoveError::ArithOverflow)?,
        index_root_id: root_id,
        aggregate_kind: aggregate_kind as u16,
        exactness: IndexCapabilityExactnessV2::Exact as u8,
        null_semantics: 0,
        flags: 0,
        row_count,
        null_count,
        non_null_count,
        value_ref,
        predicate_form_ref: u32::MAX,
        snapshot_validity_ref: 0,
        checksum: 0,
    });
    Ok(())
}

fn append_row_range(
    ranges: &mut Vec<CoviRowRangePostingV2>,
    next: CoviRowRangePostingV2,
) -> Result<(), CoveError> {
    if let Some(last) = ranges.last_mut() {
        let last_end = last
            .row_start
            .checked_add(last.row_count)
            .ok_or(CoveError::ArithOverflow)?;
        let same_scope = last.file_ref == next.file_ref
            && last.table_id == next.table_id
            && last.segment_id == next.segment_id
            && last.morsel_id == next.morsel_id;
        if same_scope {
            if last_end == next.row_start {
                last.row_count = last
                    .row_count
                    .checked_add(next.row_count)
                    .ok_or(CoveError::ArithOverflow)?;
                return Ok(());
            }
            if last_end > next.row_start {
                return Err(CoveError::BadCovi);
            }
        }
    }
    ranges.push(next);
    Ok(())
}

fn normalize_row_ranges(
    ranges: &[CoviRowRangePostingV2],
) -> Result<Vec<CoviRowRangePostingV2>, CoveError> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|range| {
        (
            range.file_ref,
            range.table_id,
            range.segment_id,
            range.morsel_id,
            range.row_start,
        )
    });
    let mut normalized = Vec::with_capacity(sorted.len());
    for range in sorted {
        append_row_range(&mut normalized, range)?;
    }
    Ok(normalized)
}

fn key_for_value(
    value: CoveArrayValue<'_>,
    logical: CoveLogicalType,
    dictionary: Option<&cove_core::dictionary::FileDictionary>,
    raw_filecode_mode: bool,
) -> Result<Option<Vec<u8>>, CoveError> {
    match value {
        CoveArrayValue::Null => Ok(None),
        CoveArrayValue::FileCode(code) if raw_filecode_mode => {
            Ok(Some(code.to_le_bytes().to_vec()))
        }
        CoveArrayValue::FileCode(code) => {
            let dictionary = dictionary.ok_or(CoveError::BadCovi)?;
            let entry = dictionary.get_entry(code)?;
            let bytes = match dictionary.decode_value(code)? {
                DictionaryValue::RawBytes(bytes) => bytes,
                DictionaryValue::RedactedPresent => return Err(CoveError::BadCovi),
                _ => return Err(CoveError::BadCovi),
            };
            Ok(Some(tagged_key(entry.value_tag, bytes)))
        }
        CoveArrayValue::NumCode(code) | CoveArrayValue::Varint(code) => {
            key_from_numcode(logical, code).map(Some)
        }
        CoveArrayValue::Int64(value) => key_from_i64(logical, value).map(Some),
        CoveArrayValue::Boolean(value) | CoveArrayValue::ValidityBit(value) => {
            tagged_canonical(CanonicalValue::Bool(value)).map(Some)
        }
        CoveArrayValue::Bytes(bytes) => key_from_bytes(logical, bytes).map(Some),
        CoveArrayValue::OwnedBytes(bytes) => key_from_bytes(logical, &bytes).map(Some),
        CoveArrayValue::DictValue(_) => Err(CoveError::BadCovi),
        _ => Err(CoveError::BadCovi),
    }
}

fn key_for_json_property_value(
    value: &Value,
    logical: CoveLogicalType,
) -> Result<Option<Vec<u8>>, CoveError> {
    if value.is_null() {
        return if logical == CoveLogicalType::Json {
            let json_text = value.to_string();
            tagged_canonical(CanonicalValue::Json(&json_text)).map(Some)
        } else {
            Ok(None)
        };
    }
    let key = match logical {
        CoveLogicalType::Null => None,
        CoveLogicalType::Bool => value
            .as_bool()
            .map(|value| tagged_canonical(CanonicalValue::Bool(value)))
            .transpose()?,
        CoveLogicalType::Int8 => json_i64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Int {
                    width: 1,
                    value: i128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::Int16 => json_i64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Int {
                    width: 2,
                    value: i128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::Int32 => json_i64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Int {
                    width: 4,
                    value: i128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::Int64 => json_i64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Int {
                    width: 8,
                    value: i128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::UInt8 => json_u64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Uint {
                    width: 1,
                    value: u128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::UInt16 => json_u64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Uint {
                    width: 2,
                    value: u128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::UInt32 => json_u64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Uint {
                    width: 4,
                    value: u128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::UInt64 => json_u64(value)
            .map(|value| {
                tagged_canonical(CanonicalValue::Uint {
                    width: 8,
                    value: u128::from(value),
                })
            })
            .transpose()?,
        CoveLogicalType::Float32 => json_f64(value)
            .map(|value| tagged_canonical(CanonicalValue::Float32(value as f32)))
            .transpose()?,
        CoveLogicalType::Float64 => json_f64(value)
            .map(|value| tagged_canonical(CanonicalValue::Float64(value)))
            .transpose()?,
        CoveLogicalType::Decimal64 => json_i64(value)
            .map(|value| tagged_canonical(CanonicalValue::Decimal64(value)))
            .transpose()?,
        CoveLogicalType::Decimal128 => json_i128(value)
            .map(|value| tagged_canonical(CanonicalValue::Decimal128(value)))
            .transpose()?,
        CoveLogicalType::DateDays => json_i64(value)
            .and_then(|value| i32::try_from(value).ok())
            .map(|value| tagged_canonical(CanonicalValue::DateDays(value)))
            .transpose()?,
        CoveLogicalType::TimestampMicros => json_i64(value)
            .map(|value| tagged_canonical(CanonicalValue::TimestampMicros(value)))
            .transpose()?,
        CoveLogicalType::TimestampNanos => json_i64(value)
            .map(|value| tagged_canonical(CanonicalValue::TimestampNanos(value)))
            .transpose()?,
        CoveLogicalType::Utf8 => value
            .as_str()
            .map(|value| tagged_canonical(CanonicalValue::Utf8(value)))
            .transpose()?,
        CoveLogicalType::Binary => json_binary_bytes(value)
            .map(|bytes| tagged_canonical(CanonicalValue::Bytes(&bytes)))
            .transpose()?,
        CoveLogicalType::Uuid => json_uuid_bytes(value)
            .map(|uuid| tagged_canonical(CanonicalValue::Uuid(uuid)))
            .transpose()?,
        CoveLogicalType::Json => {
            let json_text = value.to_string();
            Some(tagged_canonical(CanonicalValue::Json(&json_text))?)
        }
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => None,
        _ => None,
    };
    Ok(key)
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}

fn json_i128(value: &Value) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_str().and_then(|text| text.parse::<i128>().ok()))
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
}

fn json_f64(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| {
        value
            .as_str()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|value| value.is_finite() || value.is_nan())
    })
}

fn json_binary_bytes(value: &Value) -> Option<Vec<u8>> {
    if let Some(text) = value.as_str() {
        return Some(text.as_bytes().to_vec());
    }
    value.as_array().map(|items| {
        items
            .iter()
            .map(|item| item.as_u64().and_then(|value| u8::try_from(value).ok()))
            .collect::<Option<Vec<_>>>()
    })?
}

fn json_uuid_bytes(value: &Value) -> Option<[u8; 16]> {
    hex_uuid_bytes(value.as_str()?)
}

fn hex_uuid_bytes(value: &str) -> Option<[u8; 16]> {
    let mut bytes = [0u8; 16];
    let mut nibble_count = 0usize;
    let mut high = 0u8;
    for byte in value.bytes() {
        if byte == b'-' {
            continue;
        }
        let nibble = hex_nibble(byte)?;
        if (nibble_count & 1) == 0 {
            high = nibble << 4;
        } else {
            bytes[nibble_count / 2] = high | nibble;
        }
        nibble_count += 1;
        if nibble_count > 32 {
            return None;
        }
    }
    (nibble_count == 32).then_some(bytes)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn key_from_numcode(logical: CoveLogicalType, code: u64) -> Result<Vec<u8>, CoveError> {
    let value = match logical {
        CoveLogicalType::Bool => match code {
            0 => CanonicalValue::Bool(false),
            1 => CanonicalValue::Bool(true),
            _ => return Err(CoveError::BadCovi),
        },
        CoveLogicalType::Int8 => CanonicalValue::Int {
            width: 1,
            value: i128::from(types::numcode_as_i8(code)),
        },
        CoveLogicalType::Int16 => CanonicalValue::Int {
            width: 2,
            value: i128::from(types::numcode_as_i16(code)),
        },
        CoveLogicalType::Int32 => CanonicalValue::Int {
            width: 4,
            value: i128::from(types::numcode_as_i32(code)),
        },
        CoveLogicalType::Int64 => CanonicalValue::Int {
            width: 8,
            value: i128::from(types::numcode_as_i64(code)),
        },
        CoveLogicalType::UInt8 => CanonicalValue::Uint {
            width: 1,
            value: u128::from(types::numcode_as_u8(code)),
        },
        CoveLogicalType::UInt16 => CanonicalValue::Uint {
            width: 2,
            value: u128::from(types::numcode_as_u16(code)),
        },
        CoveLogicalType::UInt32 => CanonicalValue::Uint {
            width: 4,
            value: u128::from(types::numcode_as_u32(code)),
        },
        CoveLogicalType::UInt64 => CanonicalValue::Uint {
            width: 8,
            value: u128::from(types::numcode_as_u64(code)),
        },
        CoveLogicalType::Float32 => CanonicalValue::Float32(types::numcode_as_f32(code)),
        CoveLogicalType::Float64 => CanonicalValue::Float64(types::numcode_as_f64(code)),
        CoveLogicalType::Decimal64 => CanonicalValue::Decimal64(types::numcode_as_decimal64(code)),
        CoveLogicalType::DateDays => CanonicalValue::DateDays(types::numcode_as_date_days(code)),
        CoveLogicalType::TimestampMicros => {
            CanonicalValue::TimestampMicros(types::numcode_as_timestamp_micros(code))
        }
        CoveLogicalType::TimestampNanos => {
            CanonicalValue::TimestampNanos(types::numcode_as_timestamp_nanos(code))
        }
        _ => return Err(CoveError::BadCovi),
    };
    tagged_canonical(value)
}

fn key_from_i64(logical: CoveLogicalType, value: i64) -> Result<Vec<u8>, CoveError> {
    let canonical = match logical {
        CoveLogicalType::Int8 => CanonicalValue::Int {
            width: 1,
            value: i128::from(value),
        },
        CoveLogicalType::Int16 => CanonicalValue::Int {
            width: 2,
            value: i128::from(value),
        },
        CoveLogicalType::Int32 => CanonicalValue::Int {
            width: 4,
            value: i128::from(value),
        },
        CoveLogicalType::Int64 => CanonicalValue::Int {
            width: 8,
            value: i128::from(value),
        },
        CoveLogicalType::Decimal64 => CanonicalValue::Decimal64(value),
        CoveLogicalType::DateDays => {
            CanonicalValue::DateDays(i32::try_from(value).map_err(|_| CoveError::BadCovi)?)
        }
        CoveLogicalType::TimestampMicros => CanonicalValue::TimestampMicros(value),
        CoveLogicalType::TimestampNanos => CanonicalValue::TimestampNanos(value),
        _ => return Err(CoveError::BadCovi),
    };
    tagged_canonical(canonical)
}

fn key_from_bytes(logical: CoveLogicalType, bytes: &[u8]) -> Result<Vec<u8>, CoveError> {
    let canonical = match logical {
        CoveLogicalType::Bool => match bytes {
            [0] => CanonicalValue::Bool(false),
            [1] => CanonicalValue::Bool(true),
            _ => return Err(CoveError::BadCovi),
        },
        CoveLogicalType::Int8 => CanonicalValue::Int {
            width: 1,
            value: i128::from(i8::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::Int16 => CanonicalValue::Int {
            width: 2,
            value: i128::from(i16::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::Int32 => CanonicalValue::Int {
            width: 4,
            value: i128::from(i32::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::Int64 => CanonicalValue::Int {
            width: 8,
            value: i128::from(i64::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::UInt8 => CanonicalValue::Uint {
            width: 1,
            value: u128::from(u8::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::UInt16 => CanonicalValue::Uint {
            width: 2,
            value: u128::from(u16::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::UInt32 => CanonicalValue::Uint {
            width: 4,
            value: u128::from(u32::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::UInt64 => CanonicalValue::Uint {
            width: 8,
            value: u128::from(u64::from_le_bytes(fixed_array(bytes)?)),
        },
        CoveLogicalType::Float32 => {
            CanonicalValue::Float32(f32::from_bits(u32::from_le_bytes(fixed_array(bytes)?)))
        }
        CoveLogicalType::Float64 => {
            CanonicalValue::Float64(f64::from_bits(u64::from_le_bytes(fixed_array(bytes)?)))
        }
        CoveLogicalType::Decimal64 => {
            CanonicalValue::Decimal64(i64::from_le_bytes(fixed_array(bytes)?))
        }
        CoveLogicalType::DateDays => {
            CanonicalValue::DateDays(i32::from_le_bytes(fixed_array(bytes)?))
        }
        CoveLogicalType::TimestampMicros => {
            CanonicalValue::TimestampMicros(i64::from_le_bytes(fixed_array(bytes)?))
        }
        CoveLogicalType::TimestampNanos => {
            CanonicalValue::TimestampNanos(i64::from_le_bytes(fixed_array(bytes)?))
        }
        CoveLogicalType::Utf8 => {
            CanonicalValue::Utf8(std::str::from_utf8(bytes).map_err(|_| CoveError::BadCovi)?)
        }
        CoveLogicalType::Json => {
            CanonicalValue::Json(std::str::from_utf8(bytes).map_err(|_| CoveError::BadCovi)?)
        }
        CoveLogicalType::Binary => CanonicalValue::Bytes(bytes),
        CoveLogicalType::Uuid => {
            let uuid: [u8; 16] = bytes.try_into().map_err(|_| CoveError::BadCovi)?;
            CanonicalValue::Uuid(uuid)
        }
        CoveLogicalType::Decimal128 => {
            let decimal: [u8; 16] = bytes.try_into().map_err(|_| CoveError::BadCovi)?;
            CanonicalValue::Decimal128(i128::from_le_bytes(decimal))
        }
        _ => return Err(CoveError::BadCovi),
    };
    tagged_canonical(canonical)
}

fn fixed_array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], CoveError> {
    bytes.try_into().map_err(|_| CoveError::BadCovi)
}

fn tagged_canonical(value: CanonicalValue<'_>) -> Result<Vec<u8>, CoveError> {
    Ok(tagged_key(value.value_tag() as u16, value.encode()?))
}

fn tagged_key(value_tag: u16, payload: Vec<u8>) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + payload.len());
    cove_core::wire::append_u64_leb128(&mut key, u64::from(value_tag));
    key.extend_from_slice(&payload);
    key
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn derived_snapshot_id(bytes: &[u8], footer_crc32c: u32) -> [u8; 16] {
    let mut snapshot_id = [0u8; 16];
    snapshot_id[0..4].copy_from_slice(&checksum::crc32c(bytes).to_le_bytes());
    snapshot_id[4..8].copy_from_slice(&footer_crc32c.to_le_bytes());
    snapshot_id[8..16].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
    snapshot_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_core::{
        compression,
        constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind, SectionKind},
        page::{ColumnPageIndexEntryV1, COLUMN_PAGE_INDEX_ENTRY_LEN},
        reader,
        segment::{RowMorselEntryV1, TableSegmentHeaderV1, ROW_MORSEL_ENTRY_LEN},
        table::{ColumnEntry, TableCatalog, TableEntry},
        writer::{
            MinimalCoveWriter, ScanPageSpec, ScanProfileCoveWriter, ScanSegment, SectionPayload,
        },
    };
    use serde_json::{json, Value};

    fn row_range() -> CoviRowRangePostingV2 {
        CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 0,
            morsel_id: 0,
            row_start: 0,
            row_count: 1,
            flags: 0,
            checksum: 0,
        }
    }

    fn signed_key(value: i64) -> Vec<u8> {
        key_from_i64(CoveLogicalType::Int64, value).unwrap()
    }

    fn one_column_two_morsel_scan_file() -> Vec<u8> {
        let catalog = TableCatalog {
            flags: 0,
            tables: vec![TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "events".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![ColumnEntry {
                    column_id: 1,
                    name: "id".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                }],
            }],
        };
        let mut segment = ScanSegment::new(1, 0, 0, 2, 1);
        segment.morsel_row_count = 1;
        segment.set_column_pages(
            1,
            vec![
                ScanPageSpec::new(1, 1u64.to_le_bytes().to_vec())
                    .with_encoding_root(CoveEncodingKind::NumCode as u32),
                ScanPageSpec::new(1, 2u64.to_le_bytes().to_vec())
                    .with_encoding_root(CoveEncodingKind::NumCode as u32),
            ],
        );
        let mut writer = ScanProfileCoveWriter::new(catalog);
        writer.push_segment(segment);
        writer.write().unwrap()
    }

    fn bool_scan_file() -> Vec<u8> {
        let catalog = TableCatalog {
            flags: 0,
            tables: vec![TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "events".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![ColumnEntry {
                    column_id: 1,
                    name: "active".into(),
                    logical: CoveLogicalType::Bool,
                    physical: CovePhysicalKind::Boolean,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                }],
            }],
        };
        let mut writer = ScanProfileCoveWriter::new(catalog);
        writer.push_segment(ScanSegment::new(1, 0, 0, 2, 1));
        writer.write().unwrap()
    }

    fn rewrite_first_segment_data<F>(bytes: Vec<u8>, mut mutate: F) -> Vec<u8>
    where
        F: FnMut(&mut Vec<u8>),
    {
        let validated = reader::validate_bytes(&bytes).unwrap();
        let mut writer = MinimalCoveWriter::new();
        writer.created_at_us = validated.header.created_at_us;
        writer.file_id = validated.header.file_id;
        writer.producer_scope_id = validated.header.producer_scope_id;
        writer.producer_scope_kind = validated.header.producer_scope_kind;
        writer.primary_profile = validated.header.primary_profile;
        writer.required_features = validated.header.required_features;
        writer.optional_features = validated.header.optional_features;
        writer.metadata_json = validated.footer.metadata_json.clone();

        let mut mutated = false;
        for entry in &validated.footer.sections {
            let mut data = compression::section_payload(&bytes, entry)
                .unwrap()
                .into_owned();
            if !mutated && entry.section_kind == SectionKind::TableSegmentData as u16 {
                mutate(&mut data);
                mutated = true;
            }
            writer.sections.push(SectionPayload {
                section_kind: entry.section_kind,
                profile: entry.profile,
                flags: entry.flags,
                item_count: entry.item_count,
                row_count: entry.row_count,
                compression: entry.compression,
                alignment_log2: entry.alignment_log2,
                required_features: entry.required_features,
                optional_features: entry.optional_features,
                data,
            });
        }
        assert!(
            mutated,
            "fixture should contain a table segment data section"
        );
        writer.write().unwrap()
    }

    fn set_morsel_and_page_ids(segment_data: &mut [u8], ids: &[u32]) {
        let header = TableSegmentHeaderV1::parse(segment_data).unwrap();
        assert_eq!(header.morsel_count as usize, ids.len());
        let morsel_dir = header.morsel_directory_offset as usize;
        for (index, morsel_id) in ids.iter().copied().enumerate() {
            let offset = morsel_dir + index * ROW_MORSEL_ENTRY_LEN;
            let mut entry = RowMorselEntryV1::parse(&segment_data[offset..]).unwrap();
            entry.morsel_id = morsel_id;
            segment_data[offset..offset + ROW_MORSEL_ENTRY_LEN].copy_from_slice(&entry.serialize());
        }
        let page_index = header.page_index_offset as usize;
        for (index, morsel_id) in ids.iter().copied().enumerate() {
            let offset = page_index + index * COLUMN_PAGE_INDEX_ENTRY_LEN;
            let mut page = ColumnPageIndexEntryV1::parse(&segment_data[offset..]).unwrap();
            page.morsel_id = morsel_id;
            segment_data[offset..offset + COLUMN_PAGE_INDEX_ENTRY_LEN]
                .copy_from_slice(&page.serialize());
        }
    }

    fn all_row_range_postings(artifact: &CoviArtifactV2) -> Vec<CoviRowRangePostingV2> {
        artifact
            .postings_blocks
            .iter()
            .flat_map(|block| {
                block.postings.iter().flat_map(|posting| {
                    crate::parse_covi_row_range_postings(block.posting_payload(posting).unwrap())
                        .unwrap()
                })
            })
            .collect()
    }

    #[test]
    fn sorted_build_order_uses_declared_comparator_not_raw_bytes() {
        let mut keys = BTreeMap::new();
        for value in [-10, -5, 0, 5] {
            keys.insert(signed_key(value), vec![row_range()]);
        }
        let sorted = order_key_items(
            keys,
            CoveLogicalType::Int64,
            CoviIndexKindV2::Sorted,
            CoviComparatorKindV2::CanonicalOrdering,
        )
        .unwrap();
        let sorted_values = sorted
            .iter()
            .map(|(key, _)| i64::from_le_bytes(key[1..9].try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(sorted_values, vec![-10, -5, 0, 5]);
    }

    #[test]
    fn build_blocks_compute_min_max_after_comparator_sorting() {
        let mut keys = BTreeMap::new();
        for value in [-10, -5, 0, 5] {
            keys.insert(signed_key(value), vec![row_range()]);
        }
        let built = build_blocks_from_keys(
            keys,
            BuildBlocksParams {
                root_id: 1,
                logical_type: CoveLogicalType::Int64,
                index_kind: CoviIndexKindV2::Sorted,
                key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
                comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
                supports_range: true,
                null_count: 0,
                aggregate_block_id: None,
            },
        )
        .unwrap();
        assert_eq!(built.min_key_ref, 0);
        assert_eq!(built.max_key_ref, 3);
        assert_eq!(built.min_key.as_deref(), Some(signed_key(-10).as_slice()));
        assert_eq!(built.max_key.as_deref(), Some(signed_key(5).as_slice()));
    }

    #[test]
    fn index_only_min_max_answers_store_canonical_payload_not_full_key() {
        let min_key = signed_key(-10);
        let max_key = signed_key(5);
        let bytes = aggregate_answer_block(AggregateAnswerBlockParams {
            root_id: 1,
            row_count: 4,
            null_count: 0,
            distinct_count: 4,
            logical_type: CoveLogicalType::Int64,
            options: &CoviBuildOptions {
                include_index_only_min_max: true,
                ..CoviBuildOptions::default()
            },
            include_min_max_answers: true,
            include_sum_avg_answers: false,
            min_key: Some(&min_key),
            max_key: Some(&max_key),
            sum_payload: None,
        })
        .unwrap();
        let block = CoviAggregateAnswerBlockV2::parse(&bytes).unwrap();
        let min = block
            .answers
            .iter()
            .find(|answer| {
                CoviAggregateKindV2::from_u16(answer.aggregate_kind)
                    == Some(CoviAggregateKindV2::Min)
            })
            .unwrap();
        let max = block
            .answers
            .iter()
            .find(|answer| {
                CoviAggregateKindV2::from_u16(answer.aggregate_kind)
                    == Some(CoviAggregateKindV2::Max)
            })
            .unwrap();
        assert_eq!(min.value_ref, 0);
        assert_eq!(max.value_ref, 8);
        assert_eq!(&block.payload[0..8], &(-10i64).to_le_bytes());
        assert_eq!(&block.payload[8..16], &5i64.to_le_bytes());
    }

    #[test]
    fn index_only_distinct_count_answer_uses_payload_for_result_count() {
        let bytes = aggregate_answer_block(AggregateAnswerBlockParams {
            root_id: 1,
            row_count: 5,
            null_count: 2,
            distinct_count: 2,
            logical_type: CoveLogicalType::Int64,
            options: &CoviBuildOptions {
                include_index_only_distinct_count: true,
                ..CoviBuildOptions::default()
            },
            include_min_max_answers: false,
            include_sum_avg_answers: false,
            min_key: None,
            max_key: None,
            sum_payload: None,
        })
        .unwrap();
        let block = CoviAggregateAnswerBlockV2::parse(&bytes).unwrap();
        let answer = block
            .answers
            .iter()
            .find(|answer| {
                CoviAggregateKindV2::from_u16(answer.aggregate_kind)
                    == Some(CoviAggregateKindV2::DistinctCount)
            })
            .unwrap();

        assert_eq!(answer.row_count, 5);
        assert_eq!(answer.null_count, 2);
        assert_eq!(answer.non_null_count, 3);
        assert_eq!(answer.value_ref, 0);
        assert_eq!(block.payload, 2u64.to_le_bytes());
    }

    #[test]
    fn build_covi_emits_checked_integer_sum_and_avg_answers() {
        let built = build_covi_from_cove_bytes(
            &one_column_two_morsel_scan_file(),
            &CoviBuildOptions {
                all_columns: true,
                include_index_only_sum_avg: true,
                ..CoviBuildOptions::default()
            },
        )
        .unwrap();
        let artifact = CoviArtifactV2::parse(&built).unwrap();

        assert_eq!(artifact.capabilities[0].supports_sum, 1);
        assert_eq!(artifact.capabilities[0].supports_count, 1);
        assert_eq!(artifact.capabilities[0].supports_index_only, 1);
        assert!(artifact.index_only_capabilities.iter().any(|capability| {
            CoviAggregateKindV2::from_u16(capability.aggregate_kind)
                == Some(CoviAggregateKindV2::Sum)
        }));
        assert!(artifact.index_only_capabilities.iter().any(|capability| {
            CoviAggregateKindV2::from_u16(capability.aggregate_kind)
                == Some(CoviAggregateKindV2::Avg)
        }));

        let block = artifact.aggregate_answer_blocks.first().unwrap();
        assert_eq!(block.header.aggregate_answer_count, 2);
        let sum = block
            .answers
            .iter()
            .find(|answer| {
                CoviAggregateKindV2::from_u16(answer.aggregate_kind)
                    == Some(CoviAggregateKindV2::Sum)
            })
            .unwrap();
        let avg = block
            .answers
            .iter()
            .find(|answer| {
                CoviAggregateKindV2::from_u16(answer.aggregate_kind)
                    == Some(CoviAggregateKindV2::Avg)
            })
            .unwrap();
        assert_eq!(sum.row_count, 2);
        assert_eq!(sum.non_null_count, 2);
        assert_eq!(sum.value_ref, 0);
        assert_eq!(avg.value_ref, 8);
        assert_eq!(&block.payload[0..8], &3i64.to_le_bytes());
        assert_eq!(&block.payload[8..16], &3i64.to_le_bytes());
    }

    #[test]
    fn build_covi_does_not_emit_bool_sum_or_avg_answers() {
        let built = build_covi_from_cove_bytes(
            &bool_scan_file(),
            &CoviBuildOptions {
                all_columns: true,
                include_index_only_sum_avg: true,
                ..CoviBuildOptions::default()
            },
        )
        .unwrap();
        let artifact = CoviArtifactV2::parse(&built).unwrap();

        assert!(artifact.aggregate_answer_blocks.is_empty());
        assert_eq!(artifact.capabilities[0].supports_sum, 0);
        assert_eq!(artifact.capabilities[0].supports_index_only, 0);
    }

    #[test]
    fn build_covi_does_not_emit_ambiguous_bool_min_max_answers() {
        let built = build_covi_from_cove_bytes(
            &bool_scan_file(),
            &CoviBuildOptions {
                all_columns: true,
                include_index_only_min_max: true,
                ..CoviBuildOptions::default()
            },
        )
        .unwrap();
        let artifact = CoviArtifactV2::parse(&built).unwrap();

        assert!(artifact.aggregate_answer_blocks.is_empty());
        assert_eq!(artifact.capabilities[0].supports_min, 0);
        assert_eq!(artifact.capabilities[0].supports_max, 0);
        assert_eq!(artifact.capabilities[0].supports_index_only, 0);
    }

    #[test]
    fn raw_file_code_blocks_do_not_emit_range_min_max_refs() {
        let mut keys = BTreeMap::new();
        keys.insert(10u32.to_le_bytes().to_vec(), vec![row_range()]);
        keys.insert(20u32.to_le_bytes().to_vec(), vec![row_range()]);
        let built = build_blocks_from_keys(
            keys,
            BuildBlocksParams {
                root_id: 1,
                logical_type: CoveLogicalType::Utf8,
                index_kind: CoviIndexKindV2::Hash,
                key_encoding_kind: CoviKeyEncodingKindV2::FileCode,
                comparator_kind: CoviComparatorKindV2::DomainRankOrdering,
                supports_range: false,
                null_count: 0,
                aggregate_block_id: None,
            },
        )
        .unwrap();
        assert_eq!(built.index_kind, CoviIndexKindV2::Hash);
        assert_eq!(built.min_key_ref, u32::MAX);
        assert_eq!(built.max_key_ref, u32::MAX);
        assert!(built.min_key.is_none());
        assert!(built.max_key.is_none());
        assert!(!built.supports_range);
    }

    #[test]
    fn float_nan_keys_disable_range_min_max_refs() {
        let mut keys = BTreeMap::new();
        keys.insert(
            tagged_canonical(CanonicalValue::Float64(f64::NAN)).unwrap(),
            vec![row_range()],
        );
        keys.insert(
            tagged_canonical(CanonicalValue::Float64(1.0)).unwrap(),
            vec![row_range()],
        );
        assert!(canonical_float_keys_contain_nan(CoveLogicalType::Float64, &keys).unwrap());
        let built = build_blocks_from_keys(
            keys,
            BuildBlocksParams {
                root_id: 1,
                logical_type: CoveLogicalType::Float64,
                index_kind: CoviIndexKindV2::Hash,
                key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
                comparator_kind: CoviComparatorKindV2::CanonicalEquality,
                supports_range: false,
                null_count: 0,
                aggregate_block_id: None,
            },
        )
        .unwrap();
        assert_eq!(built.index_kind, CoviIndexKindV2::Hash);
        assert_eq!(
            built.comparator_kind,
            CoviComparatorKindV2::CanonicalEquality
        );
        assert_eq!(built.min_key_ref, u32::MAX);
        assert_eq!(built.max_key_ref, u32::MAX);
        assert!(built.min_key.is_none());
        assert!(built.max_key.is_none());
        assert!(!built.supports_range);
    }

    #[test]
    fn float_nan_detection_handles_float32_signaling_bits() {
        let mut keys = BTreeMap::new();
        keys.insert(
            tagged_canonical(CanonicalValue::Float32(f32::from_bits(0x7fc0_0001))).unwrap(),
            vec![row_range()],
        );
        keys.insert(
            tagged_canonical(CanonicalValue::Float32(f32::from_bits(0xffc0_0001))).unwrap(),
            vec![row_range()],
        );
        assert!(canonical_float_keys_contain_nan(CoveLogicalType::Float32, &keys).unwrap());
    }

    #[test]
    fn build_covi_accepts_stats_only_all_non_null_constant_pages() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../conformance/accept/cove_t_payload_elision_stats_only_all_non_null_valid.cove"
        ));

        let built = build_covi_from_cove_bytes(
            bytes,
            &CoviBuildOptions {
                all_columns: true,
                ..CoviBuildOptions::default()
            },
        )
        .unwrap();
        let artifact = CoviArtifactV2::parse(&built).unwrap();

        assert_eq!(artifact.index_roots.len(), 1);
        assert_eq!(artifact.index_roots[0].value_count, 6);
        assert_eq!(artifact.index_roots[0].null_count, 0);
    }

    #[test]
    fn build_covi_uses_sparse_morsel_ids_as_identifiers() {
        let bytes = rewrite_first_segment_data(one_column_two_morsel_scan_file(), |data| {
            set_morsel_and_page_ids(data, &[10, 20]);
        });

        let built = build_covi_from_cove_bytes(
            &bytes,
            &CoviBuildOptions {
                all_columns: true,
                ..CoviBuildOptions::default()
            },
        )
        .unwrap();
        let artifact = CoviArtifactV2::parse(&built).unwrap();
        let mut rows = all_row_range_postings(&artifact);
        rows.sort_by_key(|row| row.row_start);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].morsel_id, 10);
        assert_eq!(rows[0].row_start, 0);
        assert_eq!(rows[1].morsel_id, 20);
        assert_eq!(rows[1].row_start, 1);
    }

    #[test]
    fn build_covi_from_cove_o_emits_valid_covi_artifact() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../conformance/accept/cove_o_property_stats_only_all_non_null_valid.cove"
        ));

        let built = build_covi_from_cove_o_bytes(bytes, &CoviObjectPropertyBuildOptions::default())
            .unwrap();
        let artifact = CoviArtifactV2::parse(&built).unwrap();

        assert_eq!(artifact.referenced_files[0].file_len, bytes.len() as u64);

        let validated = validate_bytes_with_options(
            bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        let postscript = CovePostscriptV1::parse_from_tail(bytes).unwrap();
        crate::execution::ValidatedCoviArtifactV2::parse_and_validate(
            &built,
            crate::execution::CoviValidationContextV2::for_file(
                validated.validated.header.file_id,
                bytes.len() as u64,
                postscript.footer.crc32c,
            ),
        )
        .unwrap();
    }

    #[test]
    fn projection_column_covi_uses_projection_row_ordinals() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../conformance/accept/cove_o_property_stats_only_all_non_null_valid.cove"
        ));
        let built = build_projection_columns_covi_for_cove_o_bytes(
            bytes,
            &[CoviProjectionColumnInput {
                table_id: 7,
                column_id: 9,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                values: vec![json!("Ada"), json!("Grace"), json!("Ada")],
            }],
        )
        .unwrap();
        let artifact = CoviArtifactV2::parse(&built).unwrap();
        assert_eq!(artifact.index_roots.len(), 1);
        assert_eq!(
            artifact.index_roots[0].indexed_target_kind,
            CoviIndexedTargetKindV2::ProjectionColumn
        );
        assert_eq!(artifact.index_roots[0].table_id, 7);
        assert_eq!(artifact.index_roots[0].column_id, 9);

        let validated = validate_bytes_with_options(
            bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        let postscript = CovePostscriptV1::parse_from_tail(bytes).unwrap();
        let validated = crate::execution::ValidatedCoviArtifactV2::parse_and_validate(
            &built,
            crate::execution::CoviValidationContextV2::for_file(
                validated.validated.header.file_id,
                bytes.len() as u64,
                postscript.footer.crc32c,
            ),
        )
        .unwrap();
        let key = projection_column_lookup_key_for_json_value(&json!("Ada"), CoveLogicalType::Utf8)
            .unwrap()
            .unwrap();
        let candidates = validated
            .lookup(
                &crate::execution::CoviLookupRequestV2::projection_column_membership(7, 9, [key]),
            )
            .unwrap();
        let rows = candidates
            .row_ranges
            .iter()
            .flat_map(|range| range.row_start..range.row_start + range.row_count)
            .collect::<Vec<_>>();
        assert_eq!(rows, vec![0, 2]);
    }

    #[test]
    fn object_property_index_uses_cove_o_temporal_record_row_ranges() {
        let property = PropertyEntryV1 {
            property_id: 9,
            property_name: "name".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: true,
            collation_id: 0,
            flags: 0,
        };
        let mut first = object_record(0, json!("Ada"));
        let mut second = object_record(1, json!("Ada"));
        let mut third = object_record(2, Value::Null);
        first.properties[0].property_id = property.property_id;
        second.properties[0].property_id = property.property_id;
        third.properties[0].property_id = property.property_id;
        let records = vec![&first, &second, &third];

        let built = build_object_property_index(7, &records, &property, None).unwrap();
        let block = CoviPostingsBlockV2::parse(&built.postings_block).unwrap();
        let rows = block
            .postings
            .iter()
            .flat_map(|posting| {
                crate::parse_covi_row_range_postings(block.posting_payload(posting).unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();

        assert_eq!(built.distinct_count, 1);
        assert_eq!(built.null_count, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].table_id, u32::MAX);
        assert_eq!(rows[0].morsel_id, u32::MAX);
        assert_eq!(rows[0].segment_id, 3);
        assert_eq!(rows[0].row_start, 0);
        assert_eq!(rows[0].row_count, 2);
    }

    fn object_record(row_index: u32, value: Value) -> CoveObjectRecord {
        CoveObjectRecord {
            object_type_id: 1,
            object_type_name: "Person".into(),
            object_type_flags: 0,
            segment_id: 3,
            row_index,
            timestamp_us: 0,
            csn: u64::from(row_index),
            branch_key: 0,
            goid: [1; 16],
            record_id: [row_index as u8; 16],
            record_kind: cove_core::profile::cove_o::RecordKind::Baseline,
            prev_ref: None,
            properties: vec![cove_core::profile::cove_o::CoveObjectPropertyValue {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                flags: 0,
                value,
                redacted: false,
            }],
            association: None,
        }
    }
}
