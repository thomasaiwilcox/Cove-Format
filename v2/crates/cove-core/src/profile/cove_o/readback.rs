use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Number, Value};
use sha2::{Digest, Sha256};

use crate::{
    array::{CoveArrayValue, EncodedArray},
    codec::CodecExtensionDescriptorV2,
    compression,
    constants::{CompressionCodec, CoveLogicalType, CovePhysicalKind, SectionKind, ValueTag},
    dictionary::{DictionaryValue, FileDictionary},
    page::{PAGE_FLAG_ALL_NULL, PAGE_FLAG_STATS_ONLY_CONSTANT},
    page_payload::PageBufferKind,
    profile::{
        cove_map::{
            EmbeddedMapSection, MapEvidenceIndex, MapFunctionRegistry, MapProjectionCatalog,
        },
        cove_o::{
            CoveRecordRefV1, ObjectTypeCatalog, ObjectTypeEntryV1, PropertyEntryV1, RecordKind,
            RetainedTemporalSegmentData, TemporalBloomIndex, TemporalPropertyColumn,
            TemporalSegmentData, TemporalSegmentHeaderV1, OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            OBJECT_TYPE_FLAG_LINK_OBJECT, PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
            PROPERTY_FLAG_ASSOCIATION_TO_GOID, PROPERTY_FLAG_ASSOCIATION_TYPE,
            PROPERTY_FLAG_EVIDENCE_REF, PROPERTY_FLAG_MAPPING_RULE_REF,
        },
    },
    reader::{validate_bytes_with_options, ValidationOptions},
    retained_bytes::RetainedBytes,
    types::logical_type_from_name as parse_logical_type_name,
    utility::hex_encode,
    validity::ValidityBitmap,
    wire,
    zone_stats::{StatKind, StatScalar, ZoneStatFlags, ZoneStatsEntry, ZoneStatsSection},
    CoveError,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectSurface {
    pub object_types: Vec<ObjectTypeEntryV1>,
    pub records: Vec<CoveObjectRecord>,
    pub projection_catalog: Option<MapProjectionCatalog>,
    pub evidence_index: Option<MapEvidenceIndex>,
    pub embedded_function_ids: BTreeSet<String>,
    pub embedded_map_sections: Vec<EmbeddedMapSection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectRecord {
    pub object_type_id: u32,
    pub object_type_name: String,
    pub object_type_flags: u32,
    pub segment_id: u32,
    pub row_index: u32,
    pub timestamp_us: i64,
    pub csn: u64,
    pub branch_key: u64,
    pub goid: [u8; 16],
    pub record_id: [u8; 16],
    pub record_kind: RecordKind,
    pub prev_ref: Option<CoveRecordRefV1>,
    pub properties: Vec<CoveObjectPropertyValue>,
    pub association: Option<CoveAssociationMetadata>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectPropertyValue {
    pub property_id: u32,
    pub property_name: String,
    pub logical_type: CoveLogicalType,
    pub physical_kind: CovePhysicalKind,
    pub flags: u32,
    pub value: Value,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAssociationMetadata {
    pub association_type: Option<String>,
    pub source_goid: Option<String>,
    pub target_goid: Option<String>,
    pub evidence_ref: Option<String>,
    pub mapping_rule_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectReadOptions {
    pub requested_property_ids: Vec<u32>,
    pub requested_property_names: Vec<String>,
    pub requested_object_type_names: Vec<String>,
    pub requested_evidence_metadata_keys: Vec<String>,
    pub include_projection_catalog: bool,
    pub include_function_registry: bool,
    pub include_association_object_types: bool,
    pub include_records: bool,
    pub include_evidence_index: bool,
    pub redaction_read_policy: CoveObjectRedactionReadPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoveObjectRedactionReadPolicy {
    Refuse,
    PreserveMarker,
}

impl Default for CoveObjectRedactionReadPolicy {
    fn default() -> Self {
        Self::Refuse
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DecodedPropertyValue {
    value: Value,
    redacted: bool,
}

impl DecodedPropertyValue {
    fn plain(value: Value) -> Self {
        Self {
            value,
            redacted: false,
        }
    }

    fn redacted_marker() -> Self {
        Self {
            value: redacted_value_marker(),
            redacted: true,
        }
    }
}

fn redacted_value_marker() -> Value {
    json!({
        "policy": "redacted",
        "status": "redacted",
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectReadPushdownOptions {
    pub enabled: bool,
    pub temporal_cut: Option<CoveObjectTemporalCut>,
    pub branch_key: Option<u64>,
    pub candidate_goids: Vec<[u8; 16]>,
    pub include_tombstones: Option<bool>,
    pub association_endpoint_candidates: Vec<CoveObjectAssociationEndpointCandidate>,
    pub property_candidates: Vec<CoveObjectPropertyPredicateCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoveObjectPropertyPredicateOp {
    Eq,
    Ne,
    Lt,
    LtEq,
    Gt,
    GtEq,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoveObjectPropertyPredicateLiteral {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64Bits(u64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectPropertyPredicateCandidate {
    pub object_type_id: u32,
    pub property_id: u32,
    pub logical_type: CoveLogicalType,
    pub physical_kind: CovePhysicalKind,
    pub collation_id: Option<u16>,
    pub null_policy: Option<String>,
    pub op: CoveObjectPropertyPredicateOp,
    pub literal: Option<CoveObjectPropertyPredicateLiteral>,
    pub proof_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectAssociationEndpointCandidate {
    pub association_type_id: u32,
    pub direction: Option<String>,
    pub endpoint_role: String,
    pub branch_key: Option<u64>,
    pub temporal_cut: Option<CoveObjectTemporalCut>,
    pub candidate_goid: Option<[u8; 16]>,
    pub include_tombstones: Option<bool>,
}

impl Default for CoveObjectReadPushdownOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            temporal_cut: None,
            branch_key: None,
            candidate_goids: Vec::new(),
            include_tombstones: None,
            association_endpoint_candidates: Vec::new(),
            property_candidates: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectReadWithPushdownOptions {
    pub read: CoveObjectReadOptions,
    pub pushdown: CoveObjectReadPushdownOptions,
}

impl Default for CoveObjectReadWithPushdownOptions {
    fn default() -> Self {
        Self {
            read: CoveObjectReadOptions::default(),
            pushdown: CoveObjectReadPushdownOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoveObjectReadPushdownReport {
    pub enabled: bool,
    pub segments_seen: usize,
    pub segments_skipped: usize,
    pub rows_seen: usize,
    pub rows_candidates: usize,
    pub rows_skipped_by_property_candidates: usize,
    pub property_columns_requested: usize,
    pub decisions: Vec<CoveObjectReadPushdownDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectReadPushdownDecision {
    pub kind: String,
    pub outcome: String,
    pub reason: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectReadResult {
    pub surface: CoveObjectSurface,
    pub pushdown_report: CoveObjectReadPushdownReport,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectRetainedTemporalReadResult {
    pub catalog: ObjectTypeCatalog,
    pub segments: Vec<RetainedTemporalSegmentData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectKernelReadOptions {
    pub read: CoveObjectReadWithPushdownOptions,
}

impl Default for CoveObjectKernelReadOptions {
    fn default() -> Self {
        Self {
            read: CoveObjectReadWithPushdownOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectKernelReadResult {
    pub kernel_surface: CoveObjectKernelSurface,
    pub materialized_surface: CoveObjectSurface,
    pub pushdown_report: CoveObjectReadPushdownReport,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectKernelSurface {
    pub object_types: Vec<ObjectTypeEntryV1>,
    pub system: CoveObjectKernelSystemLanes,
    pub property_lanes: Vec<CoveObjectKernelPropertyLane>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoveObjectKernelSystemLanes {
    pub object_type_ids: Vec<u32>,
    pub segment_ids: Vec<u32>,
    pub row_indices: Vec<u32>,
    pub timestamp_us: Vec<i64>,
    pub csn: Vec<u64>,
    pub branch_keys: Vec<u64>,
    pub goids: Vec<[u8; 16]>,
    pub record_ids: Vec<[u8; 16]>,
    pub record_kinds: Vec<RecordKind>,
    pub prev_refs: Vec<Option<CoveRecordRefV1>>,
}

impl CoveObjectKernelSystemLanes {
    pub fn len(&self) -> usize {
        self.object_type_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.object_type_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectKernelPropertyLane {
    pub object_type_id: u32,
    pub property_id: u32,
    pub property_name: String,
    pub logical_type: CoveLogicalType,
    pub physical_kind: CovePhysicalKind,
    pub flags: u32,
    pub values: CoveObjectKernelPropertyValues,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoveObjectKernelPropertyValues {
    Bool(Vec<Option<bool>>),
    I64(Vec<Option<i64>>),
    U64(Vec<Option<u64>>),
    F64(Vec<Option<f64>>),
    String(Vec<Option<String>>),
    Json(Vec<Value>),
}

impl CoveObjectKernelPropertyValues {
    pub fn len(&self) -> usize {
        match self {
            Self::Bool(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::U64(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::String(values) => values.len(),
            Self::Json(values) => values.len(),
        }
    }
}

impl Default for CoveObjectReadOptions {
    fn default() -> Self {
        Self {
            requested_property_ids: Vec::new(),
            requested_property_names: Vec::new(),
            requested_object_type_names: Vec::new(),
            requested_evidence_metadata_keys: Vec::new(),
            include_projection_catalog: true,
            include_function_registry: true,
            include_association_object_types: false,
            include_records: true,
            include_evidence_index: true,
            redaction_read_policy: CoveObjectRedactionReadPolicy::Refuse,
        }
    }
}

impl CoveObjectReadOptions {
    pub fn all_properties() -> Self {
        Self::default()
    }

    pub fn requested_property_ids(property_ids: impl IntoIterator<Item = u32>) -> Self {
        Self {
            requested_property_ids: property_ids.into_iter().collect(),
            requested_property_names: Vec::new(),
            requested_object_type_names: Vec::new(),
            requested_evidence_metadata_keys: Vec::new(),
            include_projection_catalog: true,
            include_function_registry: true,
            include_association_object_types: false,
            include_records: true,
            include_evidence_index: true,
            redaction_read_policy: CoveObjectRedactionReadPolicy::Refuse,
        }
    }

    pub fn requested_property_names(
        property_names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            requested_property_ids: Vec::new(),
            requested_property_names: property_names.into_iter().map(Into::into).collect(),
            requested_object_type_names: Vec::new(),
            requested_evidence_metadata_keys: Vec::new(),
            include_projection_catalog: true,
            include_function_registry: true,
            include_association_object_types: false,
            include_records: true,
            include_evidence_index: true,
            redaction_read_policy: CoveObjectRedactionReadPolicy::Refuse,
        }
    }

    pub fn requested_object_type_names(
        object_type_names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            requested_property_ids: Vec::new(),
            requested_property_names: Vec::new(),
            requested_object_type_names: object_type_names.into_iter().map(Into::into).collect(),
            requested_evidence_metadata_keys: Vec::new(),
            include_projection_catalog: true,
            include_function_registry: true,
            include_association_object_types: false,
            include_records: true,
            include_evidence_index: true,
            redaction_read_policy: CoveObjectRedactionReadPolicy::Refuse,
        }
    }

    fn requests_property(&self, property: &PropertyEntryV1) -> bool {
        if self.requested_property_ids.is_empty() && self.requested_property_names.is_empty() {
            return true;
        }
        self.requested_property_ids.contains(&property.property_id)
            || self
                .requested_property_names
                .iter()
                .any(|name| name == &property.property_name)
    }

    fn requests_object_type(&self, object_type: &ObjectTypeEntryV1) -> bool {
        if self
            .requested_object_type_names
            .iter()
            .any(|name| name == &object_type.type_name)
        {
            return true;
        }
        if self.include_association_object_types && object_type_is_association_like(object_type) {
            return true;
        }
        self.requested_object_type_names.is_empty() && !self.include_association_object_types
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoveObjectTemporalCut {
    LatestCommitted,
    TimestampUs(i64),
    Csn(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectReconstructionOptions {
    pub temporal_cut: CoveObjectTemporalCut,
    pub branch_key: Option<u64>,
    pub include_tombstones: bool,
}

impl Default for CoveObjectReconstructionOptions {
    fn default() -> Self {
        Self {
            temporal_cut: CoveObjectTemporalCut::LatestCommitted,
            branch_key: None,
            include_tombstones: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoveObjectTombstoneStatus {
    Live,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoveObjectState {
    pub object_type_id: u32,
    pub object_type_name: String,
    pub object_type_flags: u32,
    pub branch_key: u64,
    pub goid: [u8; 16],
    pub latest_record_id: [u8; 16],
    pub latest_segment_id: u32,
    pub latest_row_index: u32,
    pub timestamp_us: i64,
    pub csn: u64,
    pub record_kind: RecordKind,
    pub tombstone_status: CoveObjectTombstoneStatus,
    pub properties: Vec<CoveObjectPropertyValue>,
    pub association: Option<CoveAssociationMetadata>,
}

pub fn read_object_surface_from_bytes(bytes: &[u8]) -> Result<CoveObjectSurface, CoveError> {
    read_object_surface_from_bytes_with_options(bytes, &CoveObjectReadOptions::default())
}

pub fn read_object_surface_from_bytes_with_options(
    bytes: &[u8],
    options: &CoveObjectReadOptions,
) -> Result<CoveObjectSurface, CoveError> {
    Ok(read_object_surface_from_bytes_with_pushdown_options(
        bytes,
        &CoveObjectReadWithPushdownOptions {
            read: options.clone(),
            pushdown: CoveObjectReadPushdownOptions::default(),
        },
    )?
    .surface)
}

pub fn read_retained_object_temporal_segments(
    data: impl Into<RetainedBytes>,
    validation_options: ValidationOptions,
) -> Result<CoveObjectRetainedTemporalReadResult, CoveError> {
    let data = data.into();
    let report = validate_bytes_with_options(data.as_slice(), validation_options)?;
    let mut catalog = None;
    let mut temporal_segment_entries = Vec::new();
    let mut codec_descriptors = Vec::<CodecExtensionDescriptorV2>::new();

    for entry in &report.validated.footer.sections {
        let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
            continue;
        };
        match kind {
            SectionKind::ObjectTypeCatalog => {
                let payload = compression::section_payload(data.as_slice(), entry)?;
                catalog = Some(ObjectTypeCatalog::parse(payload.as_ref())?);
            }
            SectionKind::TemporalSegmentData => {
                temporal_segment_entries.push(entry.clone());
            }
            SectionKind::CodecExtensionRegistry => {
                let payload = compression::section_payload(data.as_slice(), entry)?;
                codec_descriptors.extend(CodecExtensionDescriptorV2::parse_many(payload.as_ref())?);
            }
            _ => {}
        }
    }

    let catalog = catalog.ok_or_else(|| {
        CoveError::BadSchema("retained COVE-O readback requires OBJECT_TYPE_CATALOG".into())
    })?;
    let mut segments = Vec::with_capacity(temporal_segment_entries.len());
    for entry in temporal_segment_entries {
        let codec = CompressionCodec::from_u8(entry.compression)
            .ok_or_else(|| CoveError::BadSection("unknown temporal segment codec".into()))?;
        if codec != CompressionCodec::None || entry.length != entry.uncompressed_length {
            return Err(CoveError::UnsupportedEncoding(
                "retained COVE-O zero-copy requires uncompressed temporal segment sections".into(),
            ));
        }
        let offset = usize::try_from(entry.offset).map_err(|_| CoveError::OffsetRange)?;
        let length = usize::try_from(entry.length).map_err(|_| CoveError::OffsetRange)?;
        let section = data.slice(offset, length)?;
        segments.push(
            RetainedTemporalSegmentData::parse_after_semantic_validation_with_codec_descriptors(
                section,
                report.validated.header.required_features,
                &codec_descriptors,
            )?,
        );
    }

    Ok(CoveObjectRetainedTemporalReadResult { catalog, segments })
}

pub fn read_object_surface_from_bytes_with_pushdown_options(
    bytes: &[u8],
    options: &CoveObjectReadWithPushdownOptions,
) -> Result<CoveObjectReadResult, CoveError> {
    let read_options = &options.read;
    let pushdown_options = &options.pushdown;
    let mut pushdown_report = CoveObjectReadPushdownReport {
        enabled: pushdown_options.enabled,
        ..CoveObjectReadPushdownReport::default()
    };
    let report = validate_bytes_with_options(
        bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )?;

    let mut catalog = None;
    let mut projection_catalog = None;
    let mut evidence_index = None;
    let mut embedded_function_ids = BTreeSet::new();
    let mut embedded_map_sections = Vec::new();
    let mut dictionary_index = None::<Vec<u8>>;
    let mut dictionary_payload = None::<Vec<u8>>;
    let mut zone_stats = Vec::<ZoneStatsEntry>::new();
    let mut temporal_segment_entries = Vec::new();
    let mut temporal_bloom_index = None::<TemporalBloomIndex>;
    let mut codec_descriptors = Vec::<CodecExtensionDescriptorV2>::new();

    for entry in &report.validated.footer.sections {
        let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
            continue;
        };
        let payload = compression::section_payload(bytes, entry)?;
        match kind {
            SectionKind::ObjectTypeCatalog => {
                catalog = Some(ObjectTypeCatalog::parse(payload.as_ref())?);
            }
            SectionKind::TemporalSegmentData => {
                temporal_segment_entries.push(entry.clone());
            }
            SectionKind::TemporalBloomIndex => {
                temporal_bloom_index = Some(TemporalBloomIndex::parse(payload.as_ref())?);
            }
            SectionKind::FileDictionaryIndex => {
                dictionary_index = Some(payload.as_ref().to_vec());
            }
            SectionKind::FileDictionaryPayload => {
                dictionary_payload = Some(payload.as_ref().to_vec());
            }
            SectionKind::ZoneStats => {
                zone_stats.extend(ZoneStatsSection::parse(payload.as_ref())?.entries);
            }
            SectionKind::CodecExtensionRegistry => {
                codec_descriptors.extend(CodecExtensionDescriptorV2::parse_many(payload.as_ref())?);
            }
            SectionKind::MapProjectionCatalog => {
                if read_options.include_projection_catalog {
                    let catalog = MapProjectionCatalog::parse(payload.as_ref())?;
                    projection_catalog = Some(catalog.clone());
                    embedded_map_sections.push(EmbeddedMapSection::ProjectionCatalog(catalog));
                }
            }
            SectionKind::MapFunctionRegistry => {
                if read_options.include_function_registry {
                    let registry = MapFunctionRegistry::parse(payload.as_ref())?;
                    embedded_function_ids.extend(
                        registry
                            .functions
                            .iter()
                            .map(|function| function.function_id.clone()),
                    );
                    embedded_map_sections.push(EmbeddedMapSection::FunctionRegistry(registry));
                }
            }
            SectionKind::MapEvidenceIndex => {
                if read_options.include_evidence_index {
                    let index = MapEvidenceIndex::parse_with_requested_operation_metadata_keys(
                        payload.as_ref(),
                        &read_options.requested_evidence_metadata_keys,
                    )?;
                    evidence_index = Some(index.clone());
                    embedded_map_sections.push(EmbeddedMapSection::EvidenceIndex(index));
                }
            }
            SectionKind::MapSourceCatalog
            | SectionKind::MapIdentityRuleCatalog
            | SectionKind::MapRowSemanticsCatalog
            | SectionKind::MapAssertionLog
            | SectionKind::MapIdentityEquivalenceIndex
            | SectionKind::MapConversionReport => {}
            _ => {}
        }
    }
    let dictionary = match dictionary_index {
        Some(index) => Some(FileDictionary::parse(
            &index,
            dictionary_payload.as_deref().unwrap_or(&[]),
        )?),
        None => None,
    };

    let catalog = catalog.ok_or_else(|| {
        CoveError::BadSchema("COVE-O readback requires OBJECT_TYPE_CATALOG".into())
    })?;
    let object_types_by_id = catalog
        .types
        .iter()
        .map(|ty| (ty.object_type_id, ty))
        .collect::<BTreeMap<_, _>>();
    let mut records = Vec::new();
    record_pushdown_fallbacks(pushdown_options, &mut pushdown_report);
    if read_options.include_records {
        for entry in temporal_segment_entries {
            let payload = compression::section_payload(bytes, &entry)?;
            let header = TemporalSegmentHeaderV1::parse(payload.as_ref())?;
            let object_type = object_types_by_id
                .get(&header.object_type_id)
                .copied()
                .ok_or_else(|| {
                    CoveError::BadSchema(format!(
                        "temporal segment references missing object_type_id {}",
                        header.object_type_id
                    ))
                })?;
            if !read_options.requests_object_type(object_type) {
                continue;
            }
            pushdown_report.segments_seen += 1;
            if pushdown_options.enabled
                && segment_excluded_by_pushdown(&header, pushdown_options, &mut pushdown_report)
            {
                pushdown_report.segments_skipped += 1;
                continue;
            }
            if pushdown_options.enabled
                && segment_excluded_by_temporal_bloom(
                    &header,
                    temporal_bloom_index.as_ref(),
                    pushdown_options,
                    &mut pushdown_report,
                )
            {
                pushdown_report.segments_skipped += 1;
                continue;
            }
            let segment =
                TemporalSegmentData::parse_after_semantic_validation_with_codec_descriptors(
                    payload.as_ref(),
                    report.validated.header.required_features,
                    &codec_descriptors,
                )?;
            records.extend(records_from_segment(
                &segment,
                object_type,
                dictionary.as_ref(),
                &zone_stats,
                read_options,
                pushdown_options,
                &mut pushdown_report,
            )?);
        }
        if let Some(catalog) = &projection_catalog {
            apply_projection_nested_shapes(&mut records, catalog)?;
        }
    }

    Ok(CoveObjectReadResult {
        surface: CoveObjectSurface {
            object_types: catalog.types,
            records,
            projection_catalog,
            evidence_index,
            embedded_function_ids,
            embedded_map_sections,
        },
        pushdown_report,
    })
}

pub fn read_object_kernel_surface_from_bytes_with_options(
    bytes: &[u8],
    options: &CoveObjectKernelReadOptions,
) -> Result<CoveObjectKernelReadResult, CoveError> {
    let read = read_object_surface_from_bytes_with_pushdown_options(bytes, &options.read)?;
    let kernel_surface = kernel_surface_from_materialized(&read.surface)?;
    Ok(CoveObjectKernelReadResult {
        kernel_surface,
        materialized_surface: read.surface,
        pushdown_report: read.pushdown_report,
    })
}

fn kernel_surface_from_materialized(
    surface: &CoveObjectSurface,
) -> Result<CoveObjectKernelSurface, CoveError> {
    let mut system = CoveObjectKernelSystemLanes::default();
    for record in &surface.records {
        system.object_type_ids.push(record.object_type_id);
        system.segment_ids.push(record.segment_id);
        system.row_indices.push(record.row_index);
        system.timestamp_us.push(record.timestamp_us);
        system.csn.push(record.csn);
        system.branch_keys.push(record.branch_key);
        system.goids.push(record.goid);
        system.record_ids.push(record.record_id);
        system.record_kinds.push(record.record_kind);
        system.prev_refs.push(record.prev_ref);
    }

    let mut lane_keys =
        BTreeMap::<(u32, u32), (String, CoveLogicalType, CovePhysicalKind, u32)>::new();
    for record in &surface.records {
        for property in &record.properties {
            lane_keys
                .entry((record.object_type_id, property.property_id))
                .or_insert_with(|| {
                    (
                        property.property_name.clone(),
                        property.logical_type,
                        property.physical_kind,
                        property.flags,
                    )
                });
        }
    }

    let mut property_lanes = Vec::with_capacity(lane_keys.len());
    for ((object_type_id, property_id), (property_name, logical_type, physical_kind, flags)) in
        lane_keys
    {
        let values = surface
            .records
            .iter()
            .map(|record| {
                if record.object_type_id != object_type_id {
                    return Value::Null;
                }
                record
                    .properties
                    .iter()
                    .find(|property| property.property_id == property_id)
                    .map(|property| property.value.clone())
                    .unwrap_or(Value::Null)
            })
            .collect::<Vec<_>>();
        property_lanes.push(CoveObjectKernelPropertyLane {
            object_type_id,
            property_id,
            property_name,
            logical_type,
            physical_kind,
            flags,
            values: kernel_property_values(values),
        });
    }

    Ok(CoveObjectKernelSurface {
        object_types: surface.object_types.clone(),
        system,
        property_lanes,
    })
}

fn kernel_property_values(values: Vec<Value>) -> CoveObjectKernelPropertyValues {
    if values
        .iter()
        .all(|value| value.is_null() || value.as_bool().is_some())
    {
        return CoveObjectKernelPropertyValues::Bool(
            values.into_iter().map(|value| value.as_bool()).collect(),
        );
    }
    if values
        .iter()
        .all(|value| value.is_null() || value.as_i64().is_some())
    {
        return CoveObjectKernelPropertyValues::I64(
            values.into_iter().map(|value| value.as_i64()).collect(),
        );
    }
    if values
        .iter()
        .all(|value| value.is_null() || value.as_u64().is_some())
    {
        return CoveObjectKernelPropertyValues::U64(
            values.into_iter().map(|value| value.as_u64()).collect(),
        );
    }
    if values
        .iter()
        .all(|value| value.is_null() || value.as_f64().is_some())
    {
        return CoveObjectKernelPropertyValues::F64(
            values.into_iter().map(|value| value.as_f64()).collect(),
        );
    }
    if values
        .iter()
        .all(|value| value.is_null() || value.as_str().is_some())
    {
        return CoveObjectKernelPropertyValues::String(
            values
                .into_iter()
                .map(|value| value.as_str().map(str::to_string))
                .collect(),
        );
    }
    CoveObjectKernelPropertyValues::Json(values)
}

fn segment_excluded_by_pushdown(
    header: &TemporalSegmentHeaderV1,
    pushdown: &CoveObjectReadPushdownOptions,
    report: &mut CoveObjectReadPushdownReport,
) -> bool {
    match pushdown.temporal_cut {
        Some(CoveObjectTemporalCut::Csn(csn)) if header.csn_min > csn => {
            report.decisions.push(CoveObjectReadPushdownDecision {
                kind: "temporal_segment_prune".into(),
                outcome: "applied".into(),
                reason: "segment csn_min is after the requested asOf cut".into(),
                redacted: true,
            });
            true
        }
        Some(CoveObjectTemporalCut::TimestampUs(timestamp_us))
            if header.time_range_start_us > timestamp_us =>
        {
            report.decisions.push(CoveObjectReadPushdownDecision {
                kind: "temporal_segment_prune".into(),
                outcome: "applied".into(),
                reason: "segment time_range_start_us is after the requested asOf cut".into(),
                redacted: true,
            });
            true
        }
        _ => false,
    }
}

fn segment_excluded_by_temporal_bloom(
    header: &TemporalSegmentHeaderV1,
    bloom: Option<&TemporalBloomIndex>,
    pushdown: &CoveObjectReadPushdownOptions,
    report: &mut CoveObjectReadPushdownReport,
) -> bool {
    let Some(CoveObjectTemporalCut::TimestampUs(timestamp_us)) = pushdown.temporal_cut else {
        return false;
    };
    let Some(bloom) = bloom else {
        report.decisions.push(CoveObjectReadPushdownDecision {
            kind: "temporal_bloom_ignored".into(),
            outcome: "residual".into(),
            reason: "temporal bloom index is absent; segment header and row residual checks remain authoritative".into(),
            redacted: true,
        });
        return false;
    };
    let entries = bloom
        .entries
        .iter()
        .filter(|entry| entry.segment_id == header.segment_id)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return false;
    }
    if entries.iter().any(|entry| {
        entry.time_bucket_start_us <= timestamp_us && timestamp_us <= entry.time_bucket_end_us
    }) {
        return false;
    }
    report.decisions.push(CoveObjectReadPushdownDecision {
        kind: "temporal_bloom_prune".into(),
        outcome: "applied".into(),
        reason: "temporal bloom buckets prove this segment has no rows in the requested timestamp bucket".into(),
        redacted: true,
    });
    true
}

fn row_matches_pushdown(
    row: &crate::profile::cove_o::TemporalRowEntryV1,
    object_type: &ObjectTypeEntryV1,
    pushdown: &CoveObjectReadPushdownOptions,
) -> bool {
    if !pushdown.enabled {
        return true;
    }
    if let Some(branch_key) = pushdown.branch_key {
        if row.branch_key != branch_key {
            return false;
        }
    }
    if let Some(cut) = pushdown.temporal_cut {
        match cut {
            CoveObjectTemporalCut::LatestCommitted => {}
            CoveObjectTemporalCut::TimestampUs(timestamp_us) if row.timestamp_us > timestamp_us => {
                return false;
            }
            CoveObjectTemporalCut::Csn(csn) if row.csn > csn => return false,
            _ => {}
        }
    }
    let endpoint_candidates_apply = object_type_is_association_like(object_type)
        && pushdown
            .association_endpoint_candidates
            .iter()
            .any(|candidate| {
                candidate.association_type_id == object_type.object_type_id
                    && candidate.candidate_goid.is_some()
            });
    if !endpoint_candidates_apply
        && !pushdown.candidate_goids.is_empty()
        && !pushdown.candidate_goids.contains(&row.goid)
    {
        return false;
    }
    true
}

fn record_pushdown_fallbacks(
    pushdown: &CoveObjectReadPushdownOptions,
    report: &mut CoveObjectReadPushdownReport,
) {
    if pushdown.include_tombstones == Some(false) {
        report.decisions.push(CoveObjectReadPushdownDecision {
            kind: "tombstone_candidate".into(),
            outcome: "residual".into(),
            reason: "tombstone rows are retained before reconstruction to preserve record-chain correctness".into(),
            redacted: false,
        });
    }
    if !pushdown.association_endpoint_candidates.is_empty() {
        let applied = pushdown
            .association_endpoint_candidates
            .iter()
            .any(|candidate| candidate.candidate_goid.is_some());
        report.decisions.push(CoveObjectReadPushdownDecision {
            kind: "association_endpoint_candidate".into(),
            outcome: if applied { "applied" } else { "residual" }.into(),
            reason: if applied {
                "association endpoint candidates narrowed segment-local association keys; materialized endpoint verification remains authoritative".into()
            } else {
                "association endpoint candidates can narrow scheduling candidates only when a concrete GOID is available; materialized endpoint verification remains authoritative".into()
            },
            redacted: true,
        });
    }
    if !pushdown.property_candidates.is_empty() {
        let applied = pushdown
            .property_candidates
            .iter()
            .any(|candidate| candidate.proof_state == "proven_exact");
        report.decisions.push(CoveObjectReadPushdownDecision {
            kind: "property_predicate_candidate".into(),
            outcome: if applied { "applied" } else { "residual" }.into(),
            reason: if applied {
                "proven property predicate candidates narrowed segment-local rows; materialized residual verification remains authoritative".into()
            } else {
                "property predicate candidates were not proven exact and remain residual".into()
            },
            redacted: true,
        });
    }
}

fn property_columns_requested(
    segment: &TemporalSegmentData,
    object_type: &ObjectTypeEntryV1,
    options: &CoveObjectReadOptions,
) -> Result<usize, CoveError> {
    let properties_by_id = object_type
        .properties
        .iter()
        .map(|property| (property.property_id, property))
        .collect::<BTreeMap<_, _>>();
    let mut count = 0usize;
    for column in &segment.property_columns {
        let property = properties_by_id
            .get(&column.directory.column_id)
            .copied()
            .ok_or_else(|| {
                CoveError::BadSchema(format!(
                    "temporal property column references missing property_id {}",
                    column.directory.column_id
                ))
            })?;
        if options.requests_property(property) {
            count += 1;
        }
    }
    Ok(count)
}

fn object_type_is_association_like(object_type: &ObjectTypeEntryV1) -> bool {
    object_type.flags & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT) != 0
        || object_type.type_name.starts_with("Association:")
}

fn records_from_segment(
    segment: &TemporalSegmentData,
    object_type: &ObjectTypeEntryV1,
    dictionary: Option<&FileDictionary>,
    zone_stats: &[ZoneStatsEntry],
    options: &CoveObjectReadOptions,
    pushdown: &CoveObjectReadPushdownOptions,
    report: &mut CoveObjectReadPushdownReport,
) -> Result<Vec<CoveObjectRecord>, CoveError> {
    report.rows_seen += segment.rows.len();
    report.property_columns_requested += property_columns_requested(segment, object_type, options)?;
    let mut values_by_row = vec![Vec::new(); segment.rows.len()];
    let properties_by_id = object_type
        .properties
        .iter()
        .map(|property| (property.property_id, property))
        .collect::<BTreeMap<_, _>>();

    for column in &segment.property_columns {
        let property = properties_by_id
            .get(&column.directory.column_id)
            .copied()
            .ok_or_else(|| {
                CoveError::BadSchema(format!(
                    "temporal property column references missing property_id {}",
                    column.directory.column_id
                ))
            })?;
        if !options.requests_property(property) {
            continue;
        }
        let values = decode_property_column(
            segment,
            property,
            column,
            dictionary,
            zone_stats,
            options.redaction_read_policy,
        )?;
        for (row_values, value) in values_by_row.iter_mut().zip(values) {
            row_values.push(CoveObjectPropertyValue {
                property_id: property.property_id,
                property_name: property.property_name.clone(),
                logical_type: property.logical_type,
                physical_kind: property.physical_kind,
                flags: property.flags,
                value: value.value,
                redacted: value.redacted,
            });
        }
    }

    let endpoint_candidate_record_goids =
        endpoint_candidate_record_goids(object_type, &values_by_row, &segment.rows, pushdown);
    let mut records = Vec::with_capacity(segment.rows.len());
    for (row_index, row) in segment.rows.iter().enumerate() {
        if !row_matches_pushdown(row, object_type, pushdown) {
            continue;
        }
        if let Some(candidate_goids) = &endpoint_candidate_record_goids {
            if !candidate_goids.contains(&row.goid) {
                continue;
            }
        }
        if !property_candidates_match(
            object_type.object_type_id,
            &values_by_row[row_index],
            pushdown,
        ) {
            report.rows_skipped_by_property_candidates += 1;
            continue;
        }
        report.rows_candidates += 1;
        let properties = std::mem::take(&mut values_by_row[row_index]);
        let association = association_metadata(object_type, &properties);
        records.push(CoveObjectRecord {
            object_type_id: object_type.object_type_id,
            object_type_name: object_type.type_name.clone(),
            object_type_flags: object_type.flags,
            segment_id: segment.header.segment_id,
            row_index: row_index as u32,
            timestamp_us: row.timestamp_us,
            csn: row.csn,
            branch_key: row.branch_key,
            goid: row.goid,
            record_id: row.record_id,
            record_kind: row.record_kind,
            prev_ref: row.prev_ref,
            properties,
            association,
        });
    }
    Ok(records)
}

fn property_candidates_match(
    object_type_id: u32,
    values: &[CoveObjectPropertyValue],
    pushdown: &CoveObjectReadPushdownOptions,
) -> bool {
    for candidate in pushdown
        .property_candidates
        .iter()
        .filter(|candidate| candidate.object_type_id == object_type_id)
    {
        if candidate.proof_state != "proven_exact" {
            continue;
        }
        let value = values
            .iter()
            .find(|value| value.property_id == candidate.property_id)
            .map(|value| &value.value)
            .unwrap_or(&Value::Null);
        if !property_candidate_value_matches(value, candidate) {
            return false;
        }
    }
    true
}

fn property_candidate_value_matches(
    value: &Value,
    candidate: &CoveObjectPropertyPredicateCandidate,
) -> bool {
    match candidate.op {
        CoveObjectPropertyPredicateOp::IsNull => value.is_null(),
        CoveObjectPropertyPredicateOp::IsNotNull => !value.is_null(),
        CoveObjectPropertyPredicateOp::Eq
        | CoveObjectPropertyPredicateOp::Ne
        | CoveObjectPropertyPredicateOp::Lt
        | CoveObjectPropertyPredicateOp::LtEq
        | CoveObjectPropertyPredicateOp::Gt
        | CoveObjectPropertyPredicateOp::GtEq => {
            let Some(literal) = candidate.literal.as_ref() else {
                return true;
            };
            let Some(ordering) = compare_property_value_literal(value, literal) else {
                return true;
            };
            match candidate.op {
                CoveObjectPropertyPredicateOp::Eq => ordering == std::cmp::Ordering::Equal,
                CoveObjectPropertyPredicateOp::Ne => ordering != std::cmp::Ordering::Equal,
                CoveObjectPropertyPredicateOp::Lt => ordering == std::cmp::Ordering::Less,
                CoveObjectPropertyPredicateOp::LtEq => {
                    matches!(
                        ordering,
                        std::cmp::Ordering::Less | std::cmp::Ordering::Equal
                    )
                }
                CoveObjectPropertyPredicateOp::Gt => ordering == std::cmp::Ordering::Greater,
                CoveObjectPropertyPredicateOp::GtEq => {
                    matches!(
                        ordering,
                        std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                    )
                }
                CoveObjectPropertyPredicateOp::IsNull
                | CoveObjectPropertyPredicateOp::IsNotNull => true,
            }
        }
    }
}

fn compare_property_value_literal(
    value: &Value,
    literal: &CoveObjectPropertyPredicateLiteral,
) -> Option<std::cmp::Ordering> {
    match literal {
        CoveObjectPropertyPredicateLiteral::Null => {
            value.is_null().then_some(std::cmp::Ordering::Equal)
        }
        CoveObjectPropertyPredicateLiteral::Bool(expected) => {
            value.as_bool().map(|actual| actual.cmp(expected))
        }
        CoveObjectPropertyPredicateLiteral::I64(expected) => {
            value.as_i64().map(|actual| actual.cmp(expected))
        }
        CoveObjectPropertyPredicateLiteral::U64(expected) => {
            value.as_u64().map(|actual| actual.cmp(expected))
        }
        CoveObjectPropertyPredicateLiteral::F64Bits(expected_bits) => {
            let expected = f64::from_bits(*expected_bits);
            if expected.is_nan() {
                return None;
            }
            value
                .as_f64()
                .filter(|actual| !actual.is_nan())
                .and_then(|actual| actual.partial_cmp(&expected))
        }
        CoveObjectPropertyPredicateLiteral::String(expected) => {
            value.as_str().map(|actual| actual.cmp(expected.as_str()))
        }
    }
}

fn endpoint_candidate_record_goids(
    object_type: &ObjectTypeEntryV1,
    values_by_row: &[Vec<CoveObjectPropertyValue>],
    rows: &[crate::profile::cove_o::TemporalRowEntryV1],
    pushdown: &CoveObjectReadPushdownOptions,
) -> Option<BTreeSet<[u8; 16]>> {
    if !object_type_is_association_like(object_type) {
        return None;
    }
    let candidates = pushdown
        .association_endpoint_candidates
        .iter()
        .filter(|candidate| {
            candidate.association_type_id == object_type.object_type_id
                && candidate.candidate_goid.is_some()
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }

    let mut retained = BTreeSet::new();
    for (row, properties) in rows.iter().zip(values_by_row) {
        let association = association_metadata(object_type, properties);
        match association_endpoint_candidate_match(association.as_ref(), &candidates) {
            EndpointCandidateMatch::Match | EndpointCandidateMatch::Inconclusive => {
                retained.insert(row.goid);
            }
            EndpointCandidateMatch::Miss => {}
        }
    }
    Some(retained)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointCandidateMatch {
    Match,
    Miss,
    Inconclusive,
}

fn association_endpoint_candidate_match(
    association: Option<&CoveAssociationMetadata>,
    candidates: &[&CoveObjectAssociationEndpointCandidate],
) -> EndpointCandidateMatch {
    let Some(association) = association else {
        return EndpointCandidateMatch::Inconclusive;
    };
    let Some(source) = association.source_goid.as_deref() else {
        return EndpointCandidateMatch::Inconclusive;
    };
    let Some(target) = association.target_goid.as_deref() else {
        return EndpointCandidateMatch::Inconclusive;
    };
    for candidate in candidates {
        let Some(candidate_goid) = candidate.candidate_goid else {
            return EndpointCandidateMatch::Inconclusive;
        };
        let candidate_goid = hex_encode(&candidate_goid);
        let matched = match candidate.endpoint_role.as_str() {
            "source" => source == candidate_goid,
            "target" => target == candidate_goid,
            "either" => source == candidate_goid || target == candidate_goid,
            _ => return EndpointCandidateMatch::Inconclusive,
        };
        if matched {
            return EndpointCandidateMatch::Match;
        }
    }
    EndpointCandidateMatch::Miss
}

#[derive(Debug, Clone)]
enum ProjectionNestedShape {
    Scalar(CoveLogicalType),
    List(Box<ProjectionNestedShape>),
    Struct(Vec<ProjectionNestedField>),
    Map {
        key: Box<ProjectionNestedShape>,
        value: Box<ProjectionNestedShape>,
    },
}

#[derive(Debug, Clone)]
struct ProjectionNestedField {
    field_id: u64,
    name: String,
    shape: ProjectionNestedShape,
}

fn apply_projection_nested_shapes(
    records: &mut [CoveObjectRecord],
    catalog: &MapProjectionCatalog,
) -> Result<(), CoveError> {
    let lookup = projection_nested_shape_lookup(catalog)?;
    if lookup.is_empty() {
        return Ok(());
    }
    for record in records {
        for property in &mut record.properties {
            let Some(shape) = lookup.get(&(
                record.object_type_name.clone(),
                property.property_name.clone(),
            )) else {
                continue;
            };
            property.value = restore_nested_projection_value(&property.value, shape)?;
        }
    }
    Ok(())
}

fn projection_nested_shape_lookup(
    catalog: &MapProjectionCatalog,
) -> Result<BTreeMap<(String, String), ProjectionNestedShape>, CoveError> {
    let mut lookup = BTreeMap::new();
    for projection in &catalog.projections {
        let output_table = projection
            .output_table
            .as_deref()
            .unwrap_or(&projection.projection_id);
        for column in &projection.columns {
            let Some(shape) = column.nested_shape.as_deref() else {
                continue;
            };
            let shape = parse_projection_nested_shape(column.logical_type.as_deref(), shape)?;
            lookup.insert((output_table.to_string(), column.name.clone()), shape);
        }
    }
    Ok(lookup)
}

fn parse_projection_nested_shape(
    logical_type: Option<&str>,
    shape: &str,
) -> Result<ProjectionNestedShape, CoveError> {
    let value: Value = serde_json::from_str(shape)
        .map_err(|_| CoveError::BadSchema("projection nested_shape must be valid JSON".into()))?;
    let mut shape = parse_projection_nested_shape_value(&value)?;
    if let Some(logical_type) = logical_type {
        let expected = projection_logical_type_from_name(logical_type)?;
        shape = ensure_projection_shape_logical(shape, expected)?;
    }
    Ok(shape)
}

fn ensure_projection_shape_logical(
    shape: ProjectionNestedShape,
    expected: CoveLogicalType,
) -> Result<ProjectionNestedShape, CoveError> {
    let matches = matches!(
        (&shape, expected),
        (ProjectionNestedShape::List(_), CoveLogicalType::List)
            | (ProjectionNestedShape::Struct(_), CoveLogicalType::Struct)
            | (ProjectionNestedShape::Map { .. }, CoveLogicalType::Map)
    );
    if matches {
        Ok(shape)
    } else {
        Err(CoveError::BadSchema(
            "projection nested_shape does not match logical_type".into(),
        ))
    }
}

fn parse_projection_nested_shape_value(value: &Value) -> Result<ProjectionNestedShape, CoveError> {
    let object = value
        .as_object()
        .ok_or_else(|| CoveError::BadSchema("nested_shape must be an object".into()))?;
    let kind = object
        .get("type")
        .or_else(|| object.get("kind"))
        .or_else(|| object.get("logical_type"))
        .or_else(|| object.get("logical"))
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSchema("nested_shape requires type".into()))?;
    match kind {
        "list" => {
            let item = object
                .get("item")
                .or_else(|| object.get("element"))
                .ok_or_else(|| CoveError::BadSchema("list nested_shape requires item".into()))?;
            Ok(ProjectionNestedShape::List(Box::new(
                parse_projection_nested_shape_value(item)?,
            )))
        }
        "struct" => {
            let fields = object
                .get("fields")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    CoveError::BadSchema("struct nested_shape requires fields array".into())
                })?;
            let mut out = Vec::with_capacity(fields.len());
            for (index, field) in fields.iter().enumerate() {
                let field_object = field.as_object().ok_or_else(|| {
                    CoveError::BadSchema("struct nested_shape field must be an object".into())
                })?;
                let name = field_object
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CoveError::BadSchema("struct nested_shape field requires name".into())
                    })?;
                out.push(ProjectionNestedField {
                    field_id: stable_projection_field_id(name, index as u32 + 1) as u64,
                    name: name.to_string(),
                    shape: parse_projection_nested_shape_value(field)?,
                });
            }
            Ok(ProjectionNestedShape::Struct(out))
        }
        "map" => {
            let key = object
                .get("key")
                .ok_or_else(|| CoveError::BadSchema("map nested_shape requires key".into()))?;
            let value = object
                .get("value")
                .ok_or_else(|| CoveError::BadSchema("map nested_shape requires value".into()))?;
            Ok(ProjectionNestedShape::Map {
                key: Box::new(parse_projection_nested_shape_value(key)?),
                value: Box::new(parse_projection_nested_shape_value(value)?),
            })
        }
        _ => Ok(ProjectionNestedShape::Scalar(
            projection_logical_type_from_name(kind)?,
        )),
    }
}

fn restore_nested_projection_value(
    value: &Value,
    shape: &ProjectionNestedShape,
) -> Result<Value, CoveError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    match shape {
        ProjectionNestedShape::Scalar(logical) => {
            let _ = logical;
            Ok(value.clone())
        }
        ProjectionNestedShape::List(item_shape) => {
            let items = value.as_array().ok_or(CoveError::BadFileCode)?;
            Ok(Value::Array(
                items
                    .iter()
                    .map(|item| restore_nested_projection_value(item, item_shape))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        ProjectionNestedShape::Struct(fields) => {
            let object = value.as_object().ok_or(CoveError::BadFileCode)?;
            let mut out = serde_json::Map::new();
            for field in fields {
                let raw = object
                    .get(&field.field_id.to_string())
                    .unwrap_or(&Value::Null);
                out.insert(
                    field.name.clone(),
                    restore_nested_projection_value(raw, &field.shape)?,
                );
            }
            Ok(Value::Object(out))
        }
        ProjectionNestedShape::Map { key, value: item } => {
            let entries = value.as_array().ok_or(CoveError::BadFileCode)?;
            if matches!(
                key.as_ref(),
                ProjectionNestedShape::Scalar(CoveLogicalType::Utf8)
            ) {
                let mut out = serde_json::Map::new();
                for entry in entries {
                    let pair = entry.as_array().ok_or(CoveError::BadFileCode)?;
                    if pair.len() != 2 {
                        return Err(CoveError::BadFileCode);
                    }
                    let Some(key) = pair[0].as_str() else {
                        return Err(CoveError::BadFileCode);
                    };
                    out.insert(
                        key.to_string(),
                        restore_nested_projection_value(&pair[1], item)?,
                    );
                }
                Ok(Value::Object(out))
            } else {
                Ok(Value::Array(
                    entries
                        .iter()
                        .map(|entry| {
                            let pair = entry.as_array().ok_or(CoveError::BadFileCode)?;
                            if pair.len() != 2 {
                                return Err(CoveError::BadFileCode);
                            }
                            Ok(Value::Array(vec![
                                restore_nested_projection_value(&pair[0], key)?,
                                restore_nested_projection_value(&pair[1], item)?,
                            ]))
                        })
                        .collect::<Result<Vec<_>, CoveError>>()?,
                ))
            }
        }
    }
}

fn projection_logical_type_from_name(name: &str) -> Result<CoveLogicalType, CoveError> {
    parse_logical_type_name(name).map_err(|_| {
        CoveError::BadSchema(format!("unsupported nested_shape logical_type '{name}'"))
    })
}

fn stable_projection_field_id(text: &str, fallback: u32) -> u32 {
    let digest = Sha256::digest(text.as_bytes());
    let value = u32::from_le_bytes(digest[..4].try_into().unwrap());
    if value == 0 {
        fallback
    } else {
        value
    }
}

pub fn reconstruct_object_states(
    surface: &CoveObjectSurface,
    options: &CoveObjectReconstructionOptions,
) -> Result<Vec<CoveObjectState>, CoveError> {
    validate_prev_refs(&surface.records)?;
    let mut grouped = BTreeMap::<(u32, u64, [u8; 16]), Vec<&CoveObjectRecord>>::new();
    for record in &surface.records {
        if options
            .branch_key
            .is_some_and(|branch_key| record.branch_key != branch_key)
        {
            continue;
        }
        if !record_visible_at_cut(record, options.temporal_cut) {
            continue;
        }
        grouped
            .entry((record.object_type_id, record.branch_key, record.goid))
            .or_default()
            .push(record);
    }

    let mut states = Vec::with_capacity(grouped.len());
    for ((_object_type_id, _branch_key, _goid), mut records) in grouped {
        records.sort_by_key(|record| record_sort_key(record));
        let mut current: Option<CoveObjectState> = None;
        for record in records {
            validate_record_chain_step(record, current.as_ref())?;
            match record.record_kind {
                RecordKind::Baseline | RecordKind::Snapshot => {
                    current = Some(state_from_full_record(record));
                }
                RecordKind::Delta => {
                    if let Some(state) = current.as_mut() {
                        apply_delta_record(state, record);
                    } else {
                        current = Some(state_from_full_record(record));
                    }
                }
                RecordKind::Tombstone => {
                    if let Some(state) = current.as_mut() {
                        state.latest_record_id = record.record_id;
                        state.latest_segment_id = record.segment_id;
                        state.latest_row_index = record.row_index;
                        state.timestamp_us = record.timestamp_us;
                        state.csn = record.csn;
                        state.record_kind = record.record_kind;
                        state.tombstone_status = CoveObjectTombstoneStatus::Tombstoned;
                    } else {
                        let mut state = state_from_full_record(record);
                        state.tombstone_status = CoveObjectTombstoneStatus::Tombstoned;
                        current = Some(state);
                    }
                }
                RecordKind::ReservedLegacyMaterializedDelta => {
                    return Err(CoveError::BadSchema(
                        "reserved legacy materialized delta cannot be reconstructed".into(),
                    ))
                }
            }
        }
        if let Some(state) = current {
            if options.include_tombstones
                || state.tombstone_status == CoveObjectTombstoneStatus::Live
            {
                states.push(state);
            }
        }
    }
    states.sort_by_key(|state| {
        (
            state.object_type_id,
            state.branch_key,
            state.goid,
            state.timestamp_us,
            state.csn,
        )
    });
    Ok(states)
}

fn validate_prev_refs(records: &[CoveObjectRecord]) -> Result<(), CoveError> {
    let by_ref = records
        .iter()
        .map(|record| ((record.segment_id, record.row_index), record))
        .collect::<BTreeMap<_, _>>();
    for record in records {
        let Some(prev_ref) = record.prev_ref else {
            continue;
        };
        if prev_ref.target_kind > 1 {
            return Err(CoveError::RefInvalid);
        }
        let Some(prev) = by_ref
            .get(&(prev_ref.segment_id, prev_ref.row_index))
            .copied()
        else {
            return Err(CoveError::RefInvalid);
        };
        if prev.object_type_id != record.object_type_id
            || prev.branch_key != record.branch_key
            || prev.goid != record.goid
            || record_sort_key(prev) >= record_sort_key(record)
        {
            return Err(CoveError::RefInvalid);
        }
    }
    Ok(())
}

fn validate_record_chain_step(
    record: &CoveObjectRecord,
    current: Option<&CoveObjectState>,
) -> Result<(), CoveError> {
    if let Some(prev_ref) = record.prev_ref {
        let Some(current) = current else {
            return Err(CoveError::RefInvalid);
        };
        if current.latest_segment_id != prev_ref.segment_id
            || current.latest_row_index != prev_ref.row_index
        {
            return Err(CoveError::RefInvalid);
        }
    }
    Ok(())
}

fn state_from_full_record(record: &CoveObjectRecord) -> CoveObjectState {
    let tombstone_status = if record.record_kind == RecordKind::Tombstone {
        CoveObjectTombstoneStatus::Tombstoned
    } else {
        CoveObjectTombstoneStatus::Live
    };
    CoveObjectState {
        object_type_id: record.object_type_id,
        object_type_name: record.object_type_name.clone(),
        object_type_flags: record.object_type_flags,
        branch_key: record.branch_key,
        goid: record.goid,
        latest_record_id: record.record_id,
        latest_segment_id: record.segment_id,
        latest_row_index: record.row_index,
        timestamp_us: record.timestamp_us,
        csn: record.csn,
        record_kind: record.record_kind,
        tombstone_status,
        properties: record.properties.clone(),
        association: record.association.clone(),
    }
}

fn apply_delta_record(state: &mut CoveObjectState, record: &CoveObjectRecord) {
    state.latest_record_id = record.record_id;
    state.latest_segment_id = record.segment_id;
    state.latest_row_index = record.row_index;
    state.timestamp_us = record.timestamp_us;
    state.csn = record.csn;
    state.record_kind = record.record_kind;
    state.tombstone_status = CoveObjectTombstoneStatus::Live;
    for property in &record.properties {
        match state
            .properties
            .iter_mut()
            .find(|existing| existing.property_id == property.property_id)
        {
            Some(existing) => *existing = property.clone(),
            None => state.properties.push(property.clone()),
        }
    }
    state.association = association_metadata_from_state(state);
}

fn association_metadata_from_state(state: &CoveObjectState) -> Option<CoveAssociationMetadata> {
    let object_type = ObjectTypeEntryV1 {
        object_type_id: state.object_type_id,
        flags: state.object_type_flags,
        type_name: state.object_type_name.clone(),
        properties: Vec::new(),
    };
    association_metadata(&object_type, &state.properties)
}

fn record_visible_at_cut(record: &CoveObjectRecord, cut: CoveObjectTemporalCut) -> bool {
    match cut {
        CoveObjectTemporalCut::LatestCommitted => true,
        CoveObjectTemporalCut::TimestampUs(timestamp_us) => record.timestamp_us <= timestamp_us,
        CoveObjectTemporalCut::Csn(csn) => record.csn <= csn,
    }
}

fn record_sort_key(record: &CoveObjectRecord) -> (i64, u64, u32, u32, [u8; 16]) {
    (
        record.timestamp_us,
        record.csn,
        record.segment_id,
        record.row_index,
        record.record_id,
    )
}

fn decode_property_column(
    segment: &TemporalSegmentData,
    property: &PropertyEntryV1,
    column: &TemporalPropertyColumn,
    dictionary: Option<&FileDictionary>,
    zone_stats: &[ZoneStatsEntry],
    redaction_policy: CoveObjectRedactionReadPolicy,
) -> Result<Vec<DecodedPropertyValue>, CoveError> {
    let mut values = vec![DecodedPropertyValue::plain(Value::Null); segment.rows.len()];
    for page in &column.pages {
        let page_row_count = page.index_entry.row_count as usize;
        let row_start = (page.index_entry.morsel_id as usize)
            .checked_mul(segment.header.morsel_row_count as usize)
            .ok_or(CoveError::ArithOverflow)?;
        let row_end = row_start
            .checked_add(page_row_count)
            .ok_or(CoveError::ArithOverflow)?;
        if row_end > values.len() {
            return Err(CoveError::PageCorrupt);
        }

        let Some(payload) = &page.payload else {
            if page.index_entry.non_null_count == 0
                || page.index_entry.null_count == page.index_entry.row_count
                || page.index_entry.flags & PAGE_FLAG_ALL_NULL != 0
            {
                continue;
            }
            if page.index_entry.flags & PAGE_FLAG_STATS_ONLY_CONSTANT != 0 {
                let value = stats_only_constant_value(
                    segment,
                    property,
                    &page.index_entry,
                    dictionary,
                    zone_stats,
                    redaction_policy,
                )?;
                for row in &mut values[row_start..row_end] {
                    *row = value.clone();
                }
                continue;
            }
            return Err(CoveError::PageCorrupt);
        };

        if payload.header.row_count != page.index_entry.row_count {
            return Err(CoveError::PageCorrupt);
        }
        let root = payload.root_node()?;
        if root.logical_type != property.logical_type
            || root.physical_kind != property.physical_kind
        {
            return Err(CoveError::PageCorrupt);
        }
        let null_bitmap = payload.buffer_bytes(PageBufferKind::NullBitmap)?;
        let validity = null_bitmap
            .map(|bytes| ValidityBitmap::new(bytes, u64::from(page.index_entry.row_count)));
        if let Some(validity) = validity {
            validity.validate_len(u64::from(page.index_entry.row_count))?;
        }
        let value_bytes = payload.buffer_bytes(PageBufferKind::Values)?.unwrap_or(&[]);
        let array = EncodedArray::new(
            property.logical_type,
            property.physical_kind,
            u64::from(page.index_entry.row_count),
            root.encoding_kind,
            validity,
            value_bytes,
            None,
        );
        let prepared = array.prepare()?;
        for local_row in 0..page.index_entry.row_count {
            values[row_start + local_row as usize] = decode_property_value(
                property,
                prepared.decode_row(u64::from(local_row))?,
                dictionary,
                redaction_policy,
            )?;
        }
    }
    Ok(values)
}

fn decode_property_value(
    property: &PropertyEntryV1,
    value: CoveArrayValue<'_>,
    dictionary: Option<&FileDictionary>,
    redaction_policy: CoveObjectRedactionReadPolicy,
) -> Result<DecodedPropertyValue, CoveError> {
    match value {
        CoveArrayValue::Null => Ok(DecodedPropertyValue::plain(Value::Null)),
        CoveArrayValue::Boolean(value) | CoveArrayValue::ValidityBit(value) => {
            Ok(DecodedPropertyValue::plain(Value::Bool(value)))
        }
        CoveArrayValue::NumCode(value) | CoveArrayValue::Varint(value) => {
            Ok(DecodedPropertyValue::plain(json!(value)))
        }
        CoveArrayValue::Int64(value) => Ok(DecodedPropertyValue::plain(Value::Number(
            Number::from(value),
        ))),
        CoveArrayValue::Bytes(bytes) => {
            decode_bytes_value(property, bytes).map(DecodedPropertyValue::plain)
        }
        CoveArrayValue::OwnedBytes(bytes) => {
            decode_bytes_value(property, &bytes).map(DecodedPropertyValue::plain)
        }
        CoveArrayValue::FileCode(code) => {
            decode_file_code_value(property, code, dictionary, redaction_policy)
        }
        CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
            decode_canonical_dictionary_bytes(property, property.logical_type, &bytes)
                .map(DecodedPropertyValue::plain)
        }
        CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => {
            redacted_property_value(redaction_policy)
        }
    }
}

fn decode_file_code_value(
    property: &PropertyEntryV1,
    code: u32,
    dictionary: Option<&FileDictionary>,
    redaction_policy: CoveObjectRedactionReadPolicy,
) -> Result<DecodedPropertyValue, CoveError> {
    let dictionary = dictionary.ok_or_else(|| {
        CoveError::UnsupportedEncoding("FileCode property requires FILE_DICTIONARY sections".into())
    })?;
    let entry = dictionary.get_entry(code)?;
    let value_tag = ValueTag::from_u16(entry.value_tag).ok_or(CoveError::BadFileCode)?;
    match dictionary.decode_value(code)? {
        DictionaryValue::RawBytes(bytes) => {
            decode_canonical_value_tag(property, value_tag, &bytes).map(DecodedPropertyValue::plain)
        }
        DictionaryValue::RedactedPresent => redacted_property_value(redaction_policy),
    }
}

fn redacted_property_value(
    redaction_policy: CoveObjectRedactionReadPolicy,
) -> Result<DecodedPropertyValue, CoveError> {
    match redaction_policy {
        CoveObjectRedactionReadPolicy::Refuse => Err(CoveError::UnsupportedEncoding(
            "COVE-O readback refuses to expose redacted FileCode payload bytes".into(),
        )),
        CoveObjectRedactionReadPolicy::PreserveMarker => {
            Ok(DecodedPropertyValue::redacted_marker())
        }
    }
}

fn stats_only_constant_value(
    segment: &TemporalSegmentData,
    property: &PropertyEntryV1,
    page: &crate::page::ColumnPageIndexEntryV1,
    dictionary: Option<&FileDictionary>,
    zone_stats: &[ZoneStatsEntry],
    redaction_policy: CoveObjectRedactionReadPolicy,
) -> Result<DecodedPropertyValue, CoveError> {
    if page.non_null_count == 0
        || page.null_count == page.row_count
        || page.flags & PAGE_FLAG_ALL_NULL != 0
    {
        return Ok(DecodedPropertyValue::plain(Value::Null));
    }
    let stats_ref = usize::try_from(page.stats_ref).map_err(|_| CoveError::ArithOverflow)?;
    let entry = zone_stats.get(stats_ref).ok_or_else(|| {
        CoveError::UnsupportedEncoding(
            "COVE-O readback needs exact untruncated zone stats for this stats-only property"
                .into(),
        )
    })?;
    if entry.segment_id != segment.header.segment_id
        || entry.morsel_id != page.morsel_id
        || entry.column_id != property.property_id
        || entry.stats.row_count != u64::from(page.row_count)
        || entry.stats.null_count != 0
        || entry.non_null_count != page.row_count
        || page.null_count != 0
        || page.non_null_count != page.row_count
        || !entry.stats.flags.contains(ZoneStatFlags::CONSTANT)
        || !entry.stats.flags.contains(ZoneStatFlags::HAS_MIN_MAX)
        || entry.stats.flags.contains(ZoneStatFlags::MINMAX_TRUNCATED)
    {
        return Err(CoveError::UnsupportedEncoding(
            "COVE-O readback cannot prove a canonical stats-only constant for this property".into(),
        ));
    }
    let (Some(min), Some(max)) = (&entry.stats.min, &entry.stats.max) else {
        return Err(CoveError::UnsupportedEncoding(
            "COVE-O readback cannot prove a canonical stats-only constant for this property".into(),
        ));
    };
    if min.truncated || max.truncated || min != max {
        return Err(CoveError::UnsupportedEncoding(
            "COVE-O readback refuses ambiguous or truncated stats-only property values".into(),
        ));
    }
    decode_stat_scalar_value(property, min, dictionary, redaction_policy)
}

fn decode_stat_scalar_value(
    property: &PropertyEntryV1,
    scalar: &StatScalar,
    dictionary: Option<&FileDictionary>,
    redaction_policy: CoveObjectRedactionReadPolicy,
) -> Result<DecodedPropertyValue, CoveError> {
    match scalar.kind {
        StatKind::Int64 | StatKind::TimestampMicros | StatKind::TimestampNanos => {
            if scalar.bytes.len() != 8 {
                return Err(CoveError::BadStats);
            }
            Ok(DecodedPropertyValue::plain(json!(i64::from_le_bytes(
                scalar.bytes[..8].try_into().unwrap()
            ))))
        }
        StatKind::UInt64 => {
            if scalar.bytes.len() != 8 {
                return Err(CoveError::BadStats);
            }
            let value = u64::from_le_bytes(scalar.bytes[..8].try_into().unwrap());
            if property.physical_kind == CovePhysicalKind::Boolean {
                return match value {
                    0 => Ok(DecodedPropertyValue::plain(Value::Bool(false))),
                    1 => Ok(DecodedPropertyValue::plain(Value::Bool(true))),
                    _ => Err(CoveError::BadStats),
                };
            }
            if property.physical_kind == CovePhysicalKind::FileCode {
                let code = u32::try_from(value).map_err(|_| CoveError::BadFileCode)?;
                return decode_file_code_value(property, code, dictionary, redaction_policy);
            }
            Ok(DecodedPropertyValue::plain(json!(value)))
        }
        StatKind::Float64Bits => {
            if scalar.bytes.len() != 8 {
                return Err(CoveError::BadStats);
            }
            let value = f64::from_bits(u64::from_le_bytes(scalar.bytes[..8].try_into().unwrap()));
            Number::from_f64(value)
                .map(Value::Number)
                .map(DecodedPropertyValue::plain)
                .ok_or(CoveError::BadStats)
        }
        StatKind::Decimal128 => {
            if scalar.bytes.len() != 16 {
                return Err(CoveError::BadStats);
            }
            Ok(DecodedPropertyValue::plain(Value::String(
                i128::from_le_bytes(scalar.bytes[..16].try_into().unwrap()).to_string(),
            )))
        }
        StatKind::DateDays => {
            if scalar.bytes.len() != 4 {
                return Err(CoveError::BadStats);
            }
            Ok(DecodedPropertyValue::plain(json!(i32::from_le_bytes(
                scalar.bytes[..4].try_into().unwrap()
            ))))
        }
        StatKind::FixedBytes => {
            decode_bytes_value(property, &scalar.bytes).map(DecodedPropertyValue::plain)
        }
        StatKind::None => Ok(DecodedPropertyValue::plain(Value::Null)),
    }
}

fn decode_bytes_value(property: &PropertyEntryV1, bytes: &[u8]) -> Result<Value, CoveError> {
    match property.physical_kind {
        CovePhysicalKind::Boolean => {
            if bytes.len() != 1 {
                return Err(CoveError::PageCorrupt);
            }
            match bytes[0] {
                0 => Ok(Value::Bool(false)),
                1 => Ok(Value::Bool(true)),
                _ => Err(CoveError::PageCorrupt),
            }
        }
        CovePhysicalKind::FixedBytes => match property.logical_type {
            CoveLogicalType::Uuid => {
                if bytes.len() != 16 {
                    return Err(CoveError::PageCorrupt);
                }
                Ok(Value::String(hex_encode(bytes)))
            }
            CoveLogicalType::Decimal64 => {
                if bytes.len() != 8 {
                    return Err(CoveError::PageCorrupt);
                }
                let value = i64::from_le_bytes(bytes.try_into().unwrap());
                Ok(Value::String(value.to_string()))
            }
            CoveLogicalType::Decimal128 => {
                if bytes.len() != 16 {
                    return Err(CoveError::PageCorrupt);
                }
                let value = i128::from_le_bytes(bytes.try_into().unwrap());
                Ok(Value::String(value.to_string()))
            }
            _ => Ok(Value::String(hex_encode(bytes))),
        },
        CovePhysicalKind::VarBytes => match property.logical_type {
            CoveLogicalType::Utf8 => String::from_utf8(bytes.to_vec())
                .map(Value::String)
                .map_err(|_| CoveError::PageCorrupt),
            CoveLogicalType::Json => {
                serde_json::from_slice(bytes).map_err(|_| CoveError::PageCorrupt)
            }
            CoveLogicalType::Binary => match std::str::from_utf8(bytes) {
                Ok(text) => Ok(Value::String(text.to_string())),
                Err(_) => Ok(Value::String(hex_encode(bytes))),
            },
            _ => Ok(Value::String(hex_encode(bytes))),
        },
        _ => Err(CoveError::UnsupportedEncoding(format!(
            "COVE-O readback cannot decode bytes for physical kind {:?}",
            property.physical_kind
        ))),
    }
}

fn decode_canonical_dictionary_bytes(
    property: &PropertyEntryV1,
    logical_type: CoveLogicalType,
    bytes: &[u8],
) -> Result<Value, CoveError> {
    let value_tag = match logical_type {
        CoveLogicalType::Null => ValueTag::Null,
        CoveLogicalType::Bool => {
            return match bytes {
                [] => Ok(Value::Bool(false)),
                [0] => Ok(Value::Bool(false)),
                [1] => Ok(Value::Bool(true)),
                _ => Err(CoveError::BadFileCode),
            }
        }
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => ValueTag::Int64,
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => ValueTag::UInt64,
        CoveLogicalType::Float32 => ValueTag::Float32Bits,
        CoveLogicalType::Float64 => ValueTag::Float64Bits,
        CoveLogicalType::Decimal64 => ValueTag::Decimal64,
        CoveLogicalType::Decimal128 => ValueTag::Decimal128,
        CoveLogicalType::DateDays => ValueTag::DateDays,
        CoveLogicalType::TimestampMicros => ValueTag::TimestampMicros,
        CoveLogicalType::TimestampNanos => ValueTag::TimestampNanos,
        CoveLogicalType::Utf8 => ValueTag::Utf8,
        CoveLogicalType::Binary => ValueTag::Binary,
        CoveLogicalType::Uuid => ValueTag::Uuid,
        CoveLogicalType::Json => ValueTag::Json,
        CoveLogicalType::List => ValueTag::List,
        CoveLogicalType::Struct => ValueTag::Struct,
        CoveLogicalType::Map => ValueTag::Map,
    };
    decode_canonical_value_tag(property, value_tag, bytes)
}

fn decode_canonical_value_tag(
    property: &PropertyEntryV1,
    value_tag: ValueTag,
    bytes: &[u8],
) -> Result<Value, CoveError> {
    match value_tag {
        ValueTag::Null => Ok(Value::Null),
        ValueTag::BoolFalse => Ok(Value::Bool(false)),
        ValueTag::BoolTrue => Ok(Value::Bool(true)),
        ValueTag::Int64 | ValueTag::TimestampMicros | ValueTag::TimestampNanos => {
            if bytes.len() != 8 {
                return Err(CoveError::BadFileCode);
            }
            Ok(json!(i64::from_le_bytes(bytes.try_into().unwrap())))
        }
        ValueTag::UInt64 => {
            if bytes.len() != 8 {
                return Err(CoveError::BadFileCode);
            }
            Ok(json!(u64::from_le_bytes(bytes.try_into().unwrap())))
        }
        ValueTag::Float32Bits => {
            if bytes.len() != 4 {
                return Err(CoveError::BadFileCode);
            }
            Number::from_f64(f32::from_bits(u32::from_le_bytes(bytes.try_into().unwrap())) as f64)
                .map(Value::Number)
                .ok_or(CoveError::BadFileCode)
        }
        ValueTag::Float64Bits => {
            if bytes.len() != 8 {
                return Err(CoveError::BadFileCode);
            }
            Number::from_f64(f64::from_bits(u64::from_le_bytes(
                bytes.try_into().unwrap(),
            )))
            .map(Value::Number)
            .ok_or(CoveError::BadFileCode)
        }
        ValueTag::Decimal64 => {
            if bytes.len() != 8 {
                return Err(CoveError::BadFileCode);
            }
            Ok(Value::String(
                i64::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            ))
        }
        ValueTag::Decimal128 => {
            if bytes.len() != 16 {
                return Err(CoveError::BadFileCode);
            }
            Ok(Value::String(
                i128::from_le_bytes(bytes.try_into().unwrap()).to_string(),
            ))
        }
        ValueTag::DateDays => {
            if bytes.len() != 4 {
                return Err(CoveError::BadFileCode);
            }
            Ok(json!(i32::from_le_bytes(bytes.try_into().unwrap())))
        }
        ValueTag::Uuid => {
            if bytes.len() != 16 {
                return Err(CoveError::BadFileCode);
            }
            Ok(Value::String(hex_encode(bytes)))
        }
        ValueTag::Utf8 => {
            let payload = decode_canonical_length_prefixed(bytes)?;
            std::str::from_utf8(payload)
                .map(|value| Value::String(value.to_string()))
                .map_err(|_| CoveError::BadFileCode)
        }
        ValueTag::Binary => {
            let (payload, consumed) = decode_canonical_length_prefixed_consumed(bytes)?;
            if consumed != bytes.len() {
                return Err(CoveError::BadFileCode);
            }
            decode_canonical_binary_value(property, payload)
        }
        ValueTag::Json => {
            let payload = decode_canonical_length_prefixed(bytes)?;
            serde_json::from_slice(payload).map_err(|_| CoveError::BadFileCode)
        }
        ValueTag::List | ValueTag::Struct | ValueTag::Map => {
            let (value, consumed) = decode_canonical_payload_value(property, value_tag, bytes)?;
            if consumed != bytes.len() {
                return Err(CoveError::BadFileCode);
            }
            Ok(value)
        }
    }
}

fn decode_canonical_length_prefixed(bytes: &[u8]) -> Result<&[u8], CoveError> {
    let (payload, consumed) = decode_canonical_length_prefixed_consumed(bytes)?;
    if consumed != bytes.len() {
        return Err(CoveError::BadFileCode);
    }
    Ok(payload)
}

fn decode_canonical_length_prefixed_consumed(bytes: &[u8]) -> Result<(&[u8], usize), CoveError> {
    let (len, consumed) = wire::decode_u64_leb128(bytes)?;
    let len = usize::try_from(len).map_err(|_| CoveError::ArithOverflow)?;
    let end = consumed.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BadFileCode);
    }
    Ok((&bytes[consumed..end], end))
}

fn decode_canonical_tagged_value(
    property: &PropertyEntryV1,
    bytes: &[u8],
) -> Result<(ValueTag, Value, usize), CoveError> {
    let (raw_tag, tag_len) = wire::decode_u64_leb128(bytes)?;
    let raw_tag = u16::try_from(raw_tag).map_err(|_| CoveError::BadFileCode)?;
    let value_tag = ValueTag::from_u16(raw_tag).ok_or(CoveError::BadFileCode)?;
    let (value, payload_len) =
        decode_canonical_payload_value(property, value_tag, &bytes[tag_len..])?;
    let consumed = tag_len
        .checked_add(payload_len)
        .ok_or(CoveError::ArithOverflow)?;
    Ok((value_tag, value, consumed))
}

fn decode_canonical_payload_value(
    property: &PropertyEntryV1,
    value_tag: ValueTag,
    bytes: &[u8],
) -> Result<(Value, usize), CoveError> {
    match value_tag {
        ValueTag::Null => Ok((Value::Null, 0)),
        ValueTag::BoolFalse => Ok((Value::Bool(false), 0)),
        ValueTag::BoolTrue => Ok((Value::Bool(true), 0)),
        ValueTag::Int64 | ValueTag::TimestampMicros | ValueTag::TimestampNanos => {
            let payload = fixed_canonical_payload(bytes, 8)?;
            Ok((json!(i64::from_le_bytes(payload.try_into().unwrap())), 8))
        }
        ValueTag::UInt64 => {
            let payload = fixed_canonical_payload(bytes, 8)?;
            Ok((json!(u64::from_le_bytes(payload.try_into().unwrap())), 8))
        }
        ValueTag::Float32Bits => {
            let payload = fixed_canonical_payload(bytes, 4)?;
            let value = f32::from_bits(u32::from_le_bytes(payload.try_into().unwrap())) as f64;
            Number::from_f64(value)
                .map(|value| (Value::Number(value), 4))
                .ok_or(CoveError::BadFileCode)
        }
        ValueTag::Float64Bits => {
            let payload = fixed_canonical_payload(bytes, 8)?;
            let value = f64::from_bits(u64::from_le_bytes(payload.try_into().unwrap()));
            Number::from_f64(value)
                .map(|value| (Value::Number(value), 8))
                .ok_or(CoveError::BadFileCode)
        }
        ValueTag::Decimal64 => {
            let payload = fixed_canonical_payload(bytes, 8)?;
            Ok((
                Value::String(i64::from_le_bytes(payload.try_into().unwrap()).to_string()),
                8,
            ))
        }
        ValueTag::Decimal128 => {
            let payload = fixed_canonical_payload(bytes, 16)?;
            Ok((
                Value::String(i128::from_le_bytes(payload.try_into().unwrap()).to_string()),
                16,
            ))
        }
        ValueTag::DateDays => {
            let payload = fixed_canonical_payload(bytes, 4)?;
            Ok((json!(i32::from_le_bytes(payload.try_into().unwrap())), 4))
        }
        ValueTag::Uuid => {
            let payload = fixed_canonical_payload(bytes, 16)?;
            Ok((Value::String(hex_encode(payload)), 16))
        }
        ValueTag::Utf8 => {
            let (payload, consumed) = decode_canonical_length_prefixed_consumed(bytes)?;
            let value = std::str::from_utf8(payload)
                .map(|value| Value::String(value.to_string()))
                .map_err(|_| CoveError::BadFileCode)?;
            Ok((value, consumed))
        }
        ValueTag::Binary => {
            let (payload, consumed) = decode_canonical_length_prefixed_consumed(bytes)?;
            Ok((decode_canonical_binary_value(property, payload)?, consumed))
        }
        ValueTag::Json => {
            let (payload, consumed) = decode_canonical_length_prefixed_consumed(bytes)?;
            Ok((
                serde_json::from_slice(payload).map_err(|_| CoveError::BadFileCode)?,
                consumed,
            ))
        }
        ValueTag::List => {
            let (element_count, mut pos) = wire::decode_u64_leb128(bytes)?;
            let mut elements = Vec::with_capacity(
                usize::try_from(element_count).map_err(|_| CoveError::ArithOverflow)?,
            );
            for _ in 0..element_count {
                let (_, value, consumed) = decode_canonical_tagged_value(property, &bytes[pos..])?;
                pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
                elements.push(value);
            }
            Ok((Value::Array(elements), pos))
        }
        ValueTag::Struct => {
            let (field_count, mut pos) = wire::decode_u64_leb128(bytes)?;
            let mut previous_field_id = None;
            let mut object = serde_json::Map::new();
            for _ in 0..field_count {
                let (field_id, consumed) = wire::decode_u64_leb128(&bytes[pos..])?;
                pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
                if previous_field_id.is_some_and(|previous| field_id <= previous) {
                    return Err(CoveError::BadFileCode);
                }
                previous_field_id = Some(field_id);
                let (_, value, consumed) = decode_canonical_tagged_value(property, &bytes[pos..])?;
                pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
                object.insert(field_id.to_string(), value);
            }
            Ok((Value::Object(object), pos))
        }
        ValueTag::Map => {
            let (pair_count, mut pos) = wire::decode_u64_leb128(bytes)?;
            let mut previous_key = None::<Vec<u8>>;
            let mut entries = Vec::with_capacity(
                usize::try_from(pair_count).map_err(|_| CoveError::ArithOverflow)?,
            );
            for _ in 0..pair_count {
                let key_start = pos;
                let (key_tag, key, consumed) =
                    decode_canonical_tagged_value(property, &bytes[pos..])?;
                if matches!(key_tag, ValueTag::List | ValueTag::Struct | ValueTag::Map) {
                    return Err(CoveError::BadFileCode);
                }
                pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
                let key_bytes = bytes[key_start..pos].to_vec();
                if let Some(previous) = &previous_key {
                    if key_bytes <= *previous {
                        return Err(CoveError::BadFileCode);
                    }
                }
                previous_key = Some(key_bytes);
                let (_, value, consumed) = decode_canonical_tagged_value(property, &bytes[pos..])?;
                pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
                entries.push(Value::Array(vec![key, value]));
            }
            Ok((Value::Array(entries), pos))
        }
    }
}

fn fixed_canonical_payload(bytes: &[u8], width: usize) -> Result<&[u8], CoveError> {
    if bytes.len() < width {
        return Err(CoveError::BadFileCode);
    }
    Ok(&bytes[..width])
}

fn decode_canonical_binary_value(
    property: &PropertyEntryV1,
    bytes: &[u8],
) -> Result<Value, CoveError> {
    if property.physical_kind == CovePhysicalKind::VarBytes {
        return decode_bytes_value(property, bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(Value::String(text.to_string())),
        Err(_) => Ok(Value::String(hex_encode(bytes))),
    }
}

fn association_metadata(
    object_type: &ObjectTypeEntryV1,
    properties: &[CoveObjectPropertyValue],
) -> Option<CoveAssociationMetadata> {
    let is_association = object_type.flags
        & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT)
        != 0
        || object_type.type_name.starts_with("Association:");
    if !is_association {
        return None;
    }
    Some(CoveAssociationMetadata {
        association_type: property_string_by_flag(properties, PROPERTY_FLAG_ASSOCIATION_TYPE)
            .or_else(|| {
                object_type
                    .type_name
                    .strip_prefix("Association:")
                    .map(str::to_string)
            }),
        source_goid: property_string_by_flag(properties, PROPERTY_FLAG_ASSOCIATION_FROM_GOID),
        target_goid: property_string_by_flag(properties, PROPERTY_FLAG_ASSOCIATION_TO_GOID),
        evidence_ref: property_string_by_flag(properties, PROPERTY_FLAG_EVIDENCE_REF),
        mapping_rule_ref: property_string_by_flag(properties, PROPERTY_FLAG_MAPPING_RULE_REF),
    })
}

fn property_string_by_flag(properties: &[CoveObjectPropertyValue], flag: u32) -> Option<String> {
    properties
        .iter()
        .find(|property| property.flags & flag != 0)
        .and_then(|property| json_value_to_string(&property.value))
}

fn json_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{CanonicalField, CanonicalValue};
    use crate::{
        constants::StorageClass,
        dictionary::{FileDictionaryHeaderV1, FileDictionaryIndexEntryV1},
    };

    fn property(id: u32, name: &str, value: Value) -> CoveObjectPropertyValue {
        CoveObjectPropertyValue {
            property_id: id,
            property_name: name.into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            flags: 0,
            value,
            redacted: false,
        }
    }

    fn property_entry(logical_type: CoveLogicalType) -> PropertyEntryV1 {
        PropertyEntryV1 {
            property_id: 1,
            property_name: "nested".into(),
            logical_type,
            physical_kind: CovePhysicalKind::FileCode,
            nullable: true,
            collation_id: 0,
            flags: 0,
        }
    }

    fn tagged(tag: ValueTag, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        wire::append_u64_leb128(&mut out, tag as u64);
        out.extend_from_slice(payload);
        out
    }

    fn length_prefixed(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        wire::append_u64_leb128(&mut out, payload.len() as u64);
        out.extend_from_slice(payload);
        out
    }

    fn redacted_dictionary() -> FileDictionary {
        let entry = FileDictionaryIndexEntryV1 {
            value_tag: ValueTag::Utf8 as u16,
            storage_class: StorageClass::Redacted as u8,
            flags: 0,
            inline_len: 0,
            reserved0: [0; 3],
            inline_data: [0; 16],
            payload_offset: 0,
            payload_length: 0,
            canonical_hash64: 0,
            reserved1: 0,
        };
        let header = FileDictionaryHeaderV1 {
            entry_count: 1,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        };
        let mut index = Vec::new();
        index.extend_from_slice(&header.serialize());
        index.extend_from_slice(&entry.serialize());
        FileDictionary::parse(&index, &[]).unwrap()
    }

    #[test]
    fn redacted_filecode_read_policy_refuses_by_default() {
        let property = property_entry(CoveLogicalType::Utf8);
        let dictionary = redacted_dictionary();
        let err = decode_file_code_value(
            &property,
            0,
            Some(&dictionary),
            CoveObjectRedactionReadPolicy::Refuse,
        )
        .unwrap_err();

        assert!(format!("{err}").contains("redacted FileCode payload bytes"));
    }

    #[test]
    fn redacted_filecode_read_policy_preserves_safe_marker() {
        let property = property_entry(CoveLogicalType::Utf8);
        let dictionary = redacted_dictionary();
        let decoded = decode_file_code_value(
            &property,
            0,
            Some(&dictionary),
            CoveObjectRedactionReadPolicy::PreserveMarker,
        )
        .unwrap();

        assert!(decoded.redacted);
        assert_eq!(
            decoded.value,
            json!({"policy": "redacted", "status": "redacted"})
        );
    }

    fn record(
        row_index: u32,
        csn: u64,
        kind: RecordKind,
        prev_ref: Option<CoveRecordRefV1>,
        properties: Vec<CoveObjectPropertyValue>,
    ) -> CoveObjectRecord {
        CoveObjectRecord {
            object_type_id: 7,
            object_type_name: "Person".into(),
            object_type_flags: 0,
            segment_id: 1,
            row_index,
            timestamp_us: csn as i64,
            csn,
            branch_key: 0,
            goid: [0x11; 16],
            record_id: [row_index as u8; 16],
            record_kind: kind,
            prev_ref,
            properties,
            association: None,
        }
    }

    fn surface(records: Vec<CoveObjectRecord>) -> CoveObjectSurface {
        CoveObjectSurface {
            object_types: Vec::new(),
            records,
            projection_catalog: None,
            evidence_index: None,
            embedded_function_ids: BTreeSet::new(),
            embedded_map_sections: Vec::new(),
        }
    }

    #[test]
    fn reconstructs_baseline_delta_and_tombstone() {
        let baseline = record(
            0,
            1,
            RecordKind::Baseline,
            None,
            vec![property(1, "name", json!("Ada"))],
        );
        let delta = record(
            1,
            2,
            RecordKind::Delta,
            Some(CoveRecordRefV1 {
                segment_id: 1,
                row_index: 0,
                target_kind: 0,
            }),
            vec![property(2, "city", json!("London"))],
        );
        let states = reconstruct_object_states(
            &surface(vec![baseline.clone(), delta.clone()]),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].properties.len(), 2);
        assert_eq!(states[0].latest_row_index, 1);

        let tombstone = record(
            2,
            3,
            RecordKind::Tombstone,
            Some(CoveRecordRefV1 {
                segment_id: 1,
                row_index: 1,
                target_kind: 0,
            }),
            Vec::new(),
        );
        let live_states = reconstruct_object_states(
            &surface(vec![baseline, delta, tombstone]),
            &Default::default(),
        )
        .unwrap();
        assert!(live_states.is_empty());
    }

    #[test]
    fn rejects_invalid_prev_ref_chain() {
        let delta = record(
            1,
            2,
            RecordKind::Delta,
            Some(CoveRecordRefV1 {
                segment_id: 1,
                row_index: 99,
                target_kind: 0,
            }),
            vec![property(1, "name", json!("Ada"))],
        );
        assert!(matches!(
            reconstruct_object_states(&surface(vec![delta]), &Default::default()),
            Err(CoveError::RefInvalid)
        ));
    }

    #[test]
    fn readback_pushdown_entrypoint_reports_temporal_segment_pruning() {
        let bytes = include_bytes!("../../../../../conformance/accept/cove_o_temporal_valid.cove");
        let result = read_object_surface_from_bytes_with_pushdown_options(
            bytes,
            &CoveObjectReadWithPushdownOptions {
                read: CoveObjectReadOptions::requested_object_type_names(["Thing"]),
                pushdown: CoveObjectReadPushdownOptions {
                    enabled: true,
                    temporal_cut: Some(CoveObjectTemporalCut::Csn(0)),
                    ..CoveObjectReadPushdownOptions::default()
                },
            },
        )
        .unwrap();

        assert!(result.pushdown_report.enabled);
        assert!(result.pushdown_report.segments_seen >= 1);
        assert!(result.pushdown_report.segments_skipped >= 1);
        assert!(result.surface.records.is_empty());
    }

    #[test]
    fn materializes_nested_canonical_dictionary_values() {
        let property = property_entry(CoveLogicalType::List);
        let bytes = CanonicalValue::List(vec![
            CanonicalValue::Utf8("Ada"),
            CanonicalValue::Struct(vec![CanonicalField {
                field_id: 7,
                value: CanonicalValue::Bool(true),
            }]),
            CanonicalValue::Map(vec![
                (
                    CanonicalValue::Utf8("a"),
                    CanonicalValue::Int { width: 8, value: 1 },
                ),
                (CanonicalValue::Utf8("b"), CanonicalValue::Utf8("two")),
            ]),
        ])
        .encode()
        .unwrap();
        let value = decode_canonical_value_tag(&property, ValueTag::List, &bytes).unwrap();
        assert_eq!(
            value,
            json!([
                "Ada",
                {"7": true},
                [["a", 1], ["b", "two"]]
            ])
        );
    }

    #[test]
    fn rejects_malformed_canonical_nested_payloads() {
        let property = property_entry(CoveLogicalType::Map);
        let key = tagged(ValueTag::Utf8, &length_prefixed(b"k"));
        let first_value = tagged(ValueTag::Utf8, &length_prefixed(b"v1"));
        let second_value = tagged(ValueTag::Utf8, &length_prefixed(b"v2"));
        let mut bad = Vec::new();
        wire::append_u64_leb128(&mut bad, 2);
        bad.extend_from_slice(&key);
        bad.extend_from_slice(&first_value);
        bad.extend_from_slice(&key);
        bad.extend_from_slice(&second_value);

        assert_eq!(
            decode_canonical_value_tag(&property, ValueTag::Map, &bad),
            Err(CoveError::BadFileCode)
        );
    }
}
