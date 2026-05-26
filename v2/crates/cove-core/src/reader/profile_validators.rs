use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use crate::{
    array::{CoveArrayValue, EncodedArray},
    codec::CodecExtensionDescriptorV2,
    collation::CollationRegistry,
    compression,
    constants::{
        CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile, SectionKind,
        StorageClass, FEATURE_ENGINE_PROFILE, FEATURE_EXTENSION_REGISTRY, FEATURE_FILE_DICTIONARY,
        FEATURE_HARBOR_PROFILE, FEATURE_OBJECT_PROFILE, FEATURE_SEMANTIC_MAP,
    },
    dictionary::FileDictionaryView,
    digest::DigestManifest,
    domain::ColumnDomain,
    extensions::{ExtensionRegistry, ExtensionValidationContext},
    feature_binding::OperationKindV2,
    feature_scope::{FeatureTargetRefV2, FeatureUseRequestV2},
    footer::CoveSectionEntryV1,
    header::CoveHeaderV1,
    index::{
        aggregate::AggregateSynopsis, bloom::BloomFilterIndex, composite::CompositeIndex,
        exact_set::ExactSetIndex, inverted::InvertedMorselIndex, lookup::LookupIndex,
        topn::TopNSummary,
    },
    interop::lakehouse::LakehouseHints,
    kernel::KernelCapabilities,
    nested_schema::NestedSchemaSectionV1,
    page::{ColumnPageIndex, PAGE_FLAG_STATS_ONLY_CONSTANT},
    page_payload::{ColumnPagePayloadV1, PageBufferKind},
    page_validation::{
        materialize_stats_only_constant_page_payload, validate_column_page_payload,
        validate_column_page_wire, validate_stats_only_constant_page, PageValidationContext,
        StatsOnlyPageMaterializationContext,
    },
    profile::{
        cove_e::{
            CodeSpaceDescriptorV1, EngineMountPolicyV1, EngineProfileRegistry,
            ExecutionCodeDescriptorV1, ExecutionScopeDescriptorV1,
        },
        cove_h::HarborMountHintsV1,
        cove_map::{parse_embedded_section, validate_embedded_sections, EmbeddedMapSection},
        cove_o::{
            validate_self_contained, validate_temporal_property_page_elision_features,
            validate_temporal_property_stats_only_page, ObjectTypeCatalog, PropertyEntryV1,
            RecordKind, TemporalBloomIndex, TemporalPropertyColumn, TemporalSegmentData,
            TemporalSegmentIndex, TemporalSegmentIndexEntryV1, TrustManifest,
            PROPERTY_FLAG_BOOL_DECLARED_NUMERIC,
        },
    },
    redaction::RedactionManifest,
    segment::{
        TableColumnDirectoryEntryV1, TableSegmentIndex, TableSegmentPayloadV1,
        SEGMENT_COLUMN_FLAG_BOOL_DECLARED_NUMERIC,
    },
    table::{ColumnEntry, TableCatalog, TableEntry, COLUMN_FLAG_BOOL_DECLARED_NUMERIC},
    types::{
        numcode_as_date_days, numcode_as_decimal64, numcode_as_f32, numcode_as_f64, numcode_as_i16,
        numcode_as_i32, numcode_as_i64, numcode_as_i8, numcode_as_timestamp_micros,
        numcode_as_timestamp_nanos, numcode_as_u16, numcode_as_u32, numcode_as_u64, numcode_as_u8,
    },
    validity::ValidityBitmap,
    zone_stats::{ZoneStatsEntry, ZoneStatsSection},
    CoveError,
};

use super::{
    reports::{
        push_stage, IgnoredOptionalSection, OptionalPushdownPolicy, ValidationOptions,
        ValidationStage, ValidationStageReport, ValidationStageStatus,
    },
    shared_semantics::parse_validation_dictionary,
    ValidatedCoveFile,
};

pub(super) fn validate_shared_semantics(
    data: &[u8],
    validated: &ValidatedCoveFile,
    opts: &ValidationOptions,
    dict_entry_count: &mut Option<u32>,
    stages: &mut Vec<ValidationStageReport>,
    ignored_optional_sections: &mut Vec<IgnoredOptionalSection>,
) -> Result<(), CoveError> {
    let footer = &validated.footer;
    let mut checked = 0u32;
    let mut parsed_dict: Option<FileDictionaryView<'_>> = None;
    let mut dict_section_id: Option<u32> = None;
    let mut redaction_manifest_refs = BTreeSet::new();

    let has_dict_feature = validated.header.required_features & FEATURE_FILE_DICTIONARY != 0;
    if has_dict_feature {
        let index_entry = footer
            .sections
            .iter()
            .find(|s| s.section_kind == SectionKind::FileDictionaryIndex as u16);
        let payload_entry = footer
            .sections
            .iter()
            .find(|s| s.section_kind == SectionKind::FileDictionaryPayload as u16);

        match index_entry {
            None => {
                return Err(CoveError::BadSection(
                    "FEATURE_FILE_DICTIONARY set but FILE_DICTIONARY_INDEX section missing".into(),
                ));
            }
            Some(idx_entry) => {
                let index_bytes = compression::section_payload(data, idx_entry)?;
                let payload_bytes = match payload_entry {
                    Some(pay_entry) => compression::section_payload(data, pay_entry)?,
                    None => std::borrow::Cow::Borrowed(&[][..]),
                };
                let dict = FileDictionaryView::parse(index_bytes, payload_bytes)?;
                dict.validate_all()?;
                *dict_entry_count = Some(dict.len());
                dict_section_id = Some(idx_entry.section_id);
                parsed_dict = Some(dict);
                checked += 1 + u32::from(payload_entry.is_some());
            }
        }
    }

    let ext_registry_is_required =
        validated.header.required_features & FEATURE_EXTENSION_REGISTRY != 0;
    let ext_registry_is_optional =
        validated.header.optional_features & FEATURE_EXTENSION_REGISTRY != 0;
    if ext_registry_is_required || ext_registry_is_optional {
        let collation_count = footer
            .sections
            .iter()
            .find(|s| s.section_kind == SectionKind::CollationRegistry as u16)
            .map(|entry| {
                let bytes = compression::section_payload(data, entry)?;
                CollationRegistry::parse(&bytes).map(|registry| registry.entries.len())
            })
            .transpose()?;
        let ext_entry = footer
            .sections
            .iter()
            .find(|s| s.section_kind == SectionKind::ExtensionRegistry as u16);
        match (ext_registry_is_required, ext_entry) {
            (true, None) => {
                return Err(CoveError::BadSection(
                    "FEATURE_EXTENSION_REGISTRY set in required_features but \
                     EXTENSION_REGISTRY section missing"
                        .into(),
                ));
            }
            (_, Some(entry)) => {
                let ext_bytes = compression::section_payload(data, entry)?;
                let registry = ExtensionRegistry::parse(&ext_bytes)?;
                registry.validate_in_file(
                    data,
                    footer,
                    opts.allow_unknown_optional_extensions,
                    ExtensionValidationContext { collation_count },
                )?;
                checked += 1;
            }
            (false, None) => {}
        }
    }

    for entry in &footer.sections {
        let kind = SectionKind::from_u16(entry.section_kind).ok_or_else(|| {
            CoveError::BadSection(format!("unknown section_kind {}", entry.section_kind))
        })?;
        match kind {
            SectionKind::CollationRegistry => {
                let payload = compression::section_payload(data, entry)?;
                CollationRegistry::parse(&payload)?;
                checked += 1;
            }
            SectionKind::DigestManifest => {
                let payload = compression::section_payload(data, entry)?;
                DigestManifest::parse(&payload)?;
                checked += 1;
            }
            SectionKind::RedactionManifest => {
                let payload = compression::section_payload(data, entry)?;
                let manifest = RedactionManifest::parse(&payload)?;
                redaction_manifest_refs.extend(
                    manifest
                        .entries
                        .iter()
                        .map(|entry| (entry.section_id, entry.local_ref)),
                );
                checked += 1;
            }
            SectionKind::LakehouseHints => {
                let payload = compression::section_payload(data, entry)?;
                LakehouseHints::parse(&payload)?;
                checked += 1;
            }
            SectionKind::KernelCapabilities => {
                let payload = compression::section_payload(data, entry)?;
                KernelCapabilities::parse(&payload)?;
                checked += 1;
            }
            SectionKind::FileDictionaryIndex
            | SectionKind::FileDictionaryPayload
            | SectionKind::ArrowInteropHints
            | SectionKind::ExtensionRegistry
            | SectionKind::ProfileCapabilityMatrix
            | SectionKind::ExtendedFeatureSet
            | SectionKind::CodecExtensionRegistry
            | SectionKind::RuntimeCompatibilityHints
            | SectionKind::LayoutPlan
            | SectionKind::ScanSplitIndex
            | SectionKind::PageClusterDirectory
            | SectionKind::ZeroCopyBufferMap
            | SectionKind::FastMetadataIndex
            | SectionKind::CoverageProviderRegistry
            | SectionKind::CoverageSet
            | SectionKind::CoveragePlanCandidate
            | SectionKind::PredicateNormalForm
            | SectionKind::IndexOnlyCapability
            | SectionKind::SectionFeatureBinding
            | SectionKind::CoverageProofRecord
            | SectionKind::VendorExtension
            | SectionKind::TableCatalog
            | SectionKind::NestedSchema
            | SectionKind::TableSegmentIndex
            | SectionKind::TableSegmentData
            | SectionKind::ColumnDomain
            | SectionKind::ZoneStats
            | SectionKind::ExactSetIndex
            | SectionKind::BloomIndex
            | SectionKind::InvertedMorselIndex
            | SectionKind::LookupIndex
            | SectionKind::AggregateSynopsis
            | SectionKind::CompositeZoneIndex
            | SectionKind::TopNZoneSummary
            | SectionKind::EngineProfileRegistry
            | SectionKind::ExecutionCodeDescriptor
            | SectionKind::ExecutionScopeDescriptor
            | SectionKind::CodeSpaceDescriptor
            | SectionKind::EngineMountPolicy
            | SectionKind::ObjectTypeCatalog
            | SectionKind::TemporalSegmentIndex
            | SectionKind::TemporalSegmentData
            | SectionKind::TemporalBloomIndex
            | SectionKind::TrustManifest
            | SectionKind::HarborMountHints
            | SectionKind::MapSourceCatalog
            | SectionKind::MapFunctionRegistry
            | SectionKind::MapIdentityRuleCatalog
            | SectionKind::MapRowSemanticsCatalog
            | SectionKind::MapAssertionLog
            | SectionKind::MapIdentityEquivalenceIndex
            | SectionKind::MapEvidenceIndex
            | SectionKind::MapConversionReport
            | SectionKind::MapProjectionCatalog => {
                if is_optional_advisory_section(kind)
                    && opts.optional_pushdown_policy == OptionalPushdownPolicy::FailOpen
                {
                    let _ = optional_section_payload(
                        data,
                        entry,
                        opts.optional_pushdown_policy,
                        ignored_optional_sections,
                    )?;
                }
            }
        }
    }

    validate_redaction_manifest_links(
        parsed_dict.as_ref(),
        dict_section_id,
        &redaction_manifest_refs,
    )?;

    push_stage(
        stages,
        ValidationStage::SharedSemantic,
        ValidationStageStatus::Checked,
        checked,
    );
    Ok(())
}

fn validate_redaction_manifest_links(
    dict: Option<&FileDictionaryView<'_>>,
    dict_section_id: Option<u32>,
    manifest_refs: &BTreeSet<(u32, u64)>,
) -> Result<(), CoveError> {
    let (Some(dict), Some(dict_section_id)) = (dict, dict_section_id) else {
        return Ok(());
    };

    let mut redacted_codes = BTreeSet::new();
    for file_code in 0..dict.len() {
        let entry = dict.get_entry(file_code)?;
        if matches!(
            StorageClass::from_u8(entry.storage_class),
            Some(StorageClass::Redacted)
        ) {
            redacted_codes.insert(u64::from(file_code));
        }
    }

    for file_code in &redacted_codes {
        if !manifest_refs.contains(&(dict_section_id, *file_code)) {
            return Err(CoveError::BadSchema(format!(
                "redacted FileCode {file_code} is missing a redaction manifest entry"
            )));
        }
    }

    for (_, file_code) in manifest_refs
        .iter()
        .filter(|(section_id, _)| *section_id == dict_section_id)
    {
        let file_code = u32::try_from(*file_code).map_err(|_| CoveError::ArithOverflow)?;
        let entry = dict.get_entry(file_code).map_err(|error| match error {
            CoveError::BadFileCode => CoveError::BadSchema(format!(
                "redaction manifest references out-of-range FileCode {file_code}"
            )),
            other => other,
        })?;
        if !matches!(
            StorageClass::from_u8(entry.storage_class),
            Some(StorageClass::Redacted)
        ) {
            return Err(CoveError::BadSchema(format!(
                "redaction manifest references non-redacted FileCode {file_code}"
            )));
        }
    }

    Ok(())
}

pub(super) fn validate_cove_t_semantics(
    data: &[u8],
    validated: &ValidatedCoveFile,
    opts: &ValidationOptions,
    stages: &mut Vec<ValidationStageReport>,
    ignored_optional_sections: &mut Vec<IgnoredOptionalSection>,
) -> Result<(), CoveError> {
    validate_cove_t_semantics_with_registered_page_scope(
        data,
        validated,
        opts,
        stages,
        ignored_optional_sections,
        RegisteredPageValidationScope::All,
    )
}

pub(super) fn validate_cove_t_semantics_with_registered_page_scope(
    data: &[u8],
    validated: &ValidatedCoveFile,
    opts: &ValidationOptions,
    stages: &mut Vec<ValidationStageReport>,
    ignored_optional_sections: &mut Vec<IgnoredOptionalSection>,
    registered_page_scope: RegisteredPageValidationScope<'_>,
) -> Result<(), CoveError> {
    let mut checked = 0u32;
    let mut catalogs = Vec::new();
    let mut nested_schemas = Vec::new();
    let mut segment_indexes = Vec::new();
    let mut segment_payloads = Vec::new();
    let mut zone_stats_entries = Vec::new();
    let mut codec_descriptors = Vec::new();
    let dictionary = parse_validation_dictionary(data, &validated.footer)?;

    for entry in &validated.footer.sections {
        let kind = SectionKind::from_u16(entry.section_kind).ok_or_else(|| {
            CoveError::BadSection(format!("unknown section_kind {}", entry.section_kind))
        })?;
        match kind {
            SectionKind::TableCatalog => {
                let payload = compression::section_payload(data, entry)?;
                catalogs.push((entry.section_id, TableCatalog::parse(&payload)?));
                checked += 1;
            }
            SectionKind::NestedSchema => {
                let payload = compression::section_payload(data, entry)?;
                nested_schemas.push((entry.section_id, NestedSchemaSectionV1::parse(&payload)?));
                checked += 1;
            }
            SectionKind::TableSegmentIndex => {
                let payload = compression::section_payload(data, entry)?;
                segment_indexes.push((entry.section_id, TableSegmentIndex::parse(&payload)?));
                checked += 1;
            }
            SectionKind::TableSegmentData => {
                let payload = compression::section_payload(data, entry)?;
                segment_payloads.push((
                    entry.section_id,
                    entry.offset,
                    TableSegmentPayloadV1::parse_with_feature_advertisement(
                        &payload,
                        validated.header.required_features,
                        validated.header.required_features | validated.header.optional_features,
                        entry.required_features | entry.optional_features,
                    )?,
                    payload.into_owned(),
                ));
                checked += 1;
            }
            SectionKind::ColumnDomain => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match ColumnDomain::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::ExactSetIndex => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match ExactSetIndex::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::BloomIndex => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match BloomFilterIndex::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::InvertedMorselIndex => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match InvertedMorselIndex::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::LookupIndex => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match LookupIndex::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::AggregateSynopsis => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match AggregateSynopsis::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::CompositeZoneIndex => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match CompositeIndex::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::TopNZoneSummary => {
                if let Some(payload) = optional_section_payload(
                    data,
                    entry,
                    opts.optional_pushdown_policy,
                    ignored_optional_sections,
                )? {
                    match TopNSummary::parse(&payload) {
                        Ok(_) => checked += 1,
                        Err(error) => {
                            optional_section_parse_error(
                                entry,
                                opts.optional_pushdown_policy,
                                ignored_optional_sections,
                                error,
                            )?;
                        }
                    }
                }
            }
            SectionKind::ZoneStats => {
                let payload = compression::section_payload(data, entry)?;
                zone_stats_entries.extend(ZoneStatsSection::parse(&payload)?.entries);
                checked += 1;
            }
            SectionKind::CodecExtensionRegistry => {
                let payload = compression::section_payload(data, entry)?;
                codec_descriptors.extend(CodecExtensionDescriptorV2::parse_many(&payload)?);
                checked += 1;
            }
            SectionKind::FileDictionaryIndex
            | SectionKind::FileDictionaryPayload
            | SectionKind::CollationRegistry
            | SectionKind::DigestManifest
            | SectionKind::RedactionManifest
            | SectionKind::ArrowInteropHints
            | SectionKind::LakehouseHints
            | SectionKind::ExtensionRegistry
            | SectionKind::ProfileCapabilityMatrix
            | SectionKind::ExtendedFeatureSet
            | SectionKind::LayoutPlan
            | SectionKind::ScanSplitIndex
            | SectionKind::PageClusterDirectory
            | SectionKind::ZeroCopyBufferMap
            | SectionKind::FastMetadataIndex
            | SectionKind::CoverageProviderRegistry
            | SectionKind::CoverageSet
            | SectionKind::CoveragePlanCandidate
            | SectionKind::PredicateNormalForm
            | SectionKind::IndexOnlyCapability
            | SectionKind::SectionFeatureBinding
            | SectionKind::CoverageProofRecord
            | SectionKind::KernelCapabilities
            | SectionKind::EngineProfileRegistry
            | SectionKind::ExecutionCodeDescriptor
            | SectionKind::ExecutionScopeDescriptor
            | SectionKind::CodeSpaceDescriptor
            | SectionKind::EngineMountPolicy
            | SectionKind::RuntimeCompatibilityHints
            | SectionKind::ObjectTypeCatalog
            | SectionKind::TemporalSegmentIndex
            | SectionKind::TemporalSegmentData
            | SectionKind::TemporalBloomIndex
            | SectionKind::TrustManifest
            | SectionKind::HarborMountHints
            | SectionKind::MapSourceCatalog
            | SectionKind::MapFunctionRegistry
            | SectionKind::MapIdentityRuleCatalog
            | SectionKind::MapRowSemanticsCatalog
            | SectionKind::MapAssertionLog
            | SectionKind::MapIdentityEquivalenceIndex
            | SectionKind::MapEvidenceIndex
            | SectionKind::MapConversionReport
            | SectionKind::MapProjectionCatalog
            | SectionKind::VendorExtension => {}
        }
    }
    validate_cove_t_cross_sections(
        &catalogs,
        &nested_schemas,
        &segment_indexes,
        &segment_payloads,
        CoveTCrossSectionRefs {
            dictionary: dictionary.as_ref(),
            zone_stats: &zone_stats_entries,
            codec_descriptors: &codec_descriptors,
            registered_page_scope,
        },
    )?;
    push_stage(
        stages,
        ValidationStage::CoveTable,
        ValidationStageStatus::Checked,
        checked,
    );
    Ok(())
}

fn optional_section_payload<'a>(
    data: &'a [u8],
    entry: &CoveSectionEntryV1,
    policy: OptionalPushdownPolicy,
    ignored_optional_sections: &mut Vec<IgnoredOptionalSection>,
) -> Result<Option<Cow<'a, [u8]>>, CoveError> {
    match compression::section_payload(data, entry) {
        Ok(payload) => Ok(Some(payload)),
        Err(error) => {
            optional_section_parse_error(entry, policy, ignored_optional_sections, error)?;
            Ok(None)
        }
    }
}

fn optional_section_parse_error(
    entry: &CoveSectionEntryV1,
    policy: OptionalPushdownPolicy,
    ignored_optional_sections: &mut Vec<IgnoredOptionalSection>,
    error: CoveError,
) -> Result<(), CoveError> {
    if policy == OptionalPushdownPolicy::FailOpen && is_optional_advisory_entry(entry) {
        if ignored_optional_sections
            .iter()
            .any(|ignored| ignored.section_id == entry.section_id)
        {
            return Ok(());
        }
        ignored_optional_sections.push(IgnoredOptionalSection {
            section_id: entry.section_id,
            section_kind: entry.section_kind,
            reason: error.to_string(),
        });
        return Ok(());
    }
    Err(error)
}

pub(super) fn is_optional_advisory_entry(entry: &CoveSectionEntryV1) -> bool {
    entry.required_features == 0
        && SectionKind::from_u16(entry.section_kind)
            .map(is_optional_advisory_section)
            .unwrap_or(false)
}

pub(super) fn ignored_section_required_for_feature_use(
    ignored: &IgnoredOptionalSection,
    request: &FeatureUseRequestV2,
) -> bool {
    if request.needed_section_ids.contains(&ignored.section_id) {
        return true;
    }
    if request
        .needed_page_refs
        .iter()
        .any(|target| target.section_id == ignored.section_id)
    {
        return true;
    }
    SectionKind::from_u16(ignored.section_kind)
        .map(|kind| section_kind_required_for_feature_use(kind, request))
        .unwrap_or(false)
}

pub(super) fn section_entry_required_for_feature_use(
    entry: &CoveSectionEntryV1,
    request: &FeatureUseRequestV2,
) -> bool {
    request.needed_section_ids.contains(&entry.section_id)
        || request
            .needed_page_refs
            .iter()
            .any(|target| target.section_id == entry.section_id)
        || SectionKind::from_u16(entry.section_kind)
            .map(|kind| section_kind_required_for_feature_use(kind, request))
            .unwrap_or(false)
}

pub(super) fn is_embedded_optional_profile_section(kind: SectionKind) -> bool {
    is_layout_section(kind)
        || is_coverage_section(kind)
        || matches!(
            kind,
            SectionKind::IndexOnlyCapability | SectionKind::RuntimeCompatibilityHints
        )
}

fn section_kind_required_for_feature_use(kind: SectionKind, request: &FeatureUseRequestV2) -> bool {
    request
        .requested_operation
        .map(|operation| operation_requires_section(operation, kind))
        .unwrap_or(false)
        || request
            .requested_profile
            .map(|profile| profile_requires_section(profile, kind))
            .unwrap_or(false)
}

fn operation_requires_section(operation: OperationKindV2, kind: SectionKind) -> bool {
    match operation {
        OperationKindV2::OrdinaryTableScan => is_table_scan_section(kind),
        OperationKindV2::CoveragePlanning => is_coverage_section(kind),
        OperationKindV2::IndexOnlyAnswer => kind == SectionKind::IndexOnlyCapability,
        OperationKindV2::ZeroCopyExport => kind == SectionKind::ZeroCopyBufferMap,
        OperationKindV2::RuntimeAdapterSelection => kind == SectionKind::RuntimeCompatibilityHints,
        OperationKindV2::EngineExecutionMapping => is_engine_execution_section(kind),
        OperationKindV2::ObjectReconstruction => is_object_section(kind),
        OperationKindV2::TrustVerification => kind == SectionKind::TrustManifest,
        OperationKindV2::RedactionPolicyEvaluation => kind == SectionKind::RedactionManifest,
        OperationKindV2::MappingReplay
        | OperationKindV2::MappingExplanation
        | OperationKindV2::ProjectionReadback => is_map_section(kind),
        OperationKindV2::HarborMount => is_harbor_section(kind),
        _ => false,
    }
}

fn profile_requires_section(profile: u8, kind: SectionKind) -> bool {
    match PrimaryProfile::from_u8(profile) {
        Some(PrimaryProfile::ObjectTemporal) => is_object_section(kind),
        Some(PrimaryProfile::EngineExecution) => is_engine_execution_section(kind),
        Some(PrimaryProfile::HarborExecution) => is_harbor_section(kind),
        Some(PrimaryProfile::SemanticMapping) => is_map_section(kind),
        Some(PrimaryProfile::LayoutPlanning) => is_layout_section(kind),
        Some(PrimaryProfile::RuntimeCompatibility) => {
            kind == SectionKind::RuntimeCompatibilityHints
        }
        Some(PrimaryProfile::CoverageMetadata) => is_coverage_section(kind),
        Some(PrimaryProfile::SecondaryIndex) => kind == SectionKind::IndexOnlyCapability,
        _ => false,
    }
}

fn is_optional_advisory_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::ColumnDomain
            | SectionKind::ExactSetIndex
            | SectionKind::BloomIndex
            | SectionKind::InvertedMorselIndex
            | SectionKind::LookupIndex
            | SectionKind::AggregateSynopsis
            | SectionKind::CompositeZoneIndex
            | SectionKind::TopNZoneSummary
            | SectionKind::LayoutPlan
            | SectionKind::ScanSplitIndex
            | SectionKind::PageClusterDirectory
            | SectionKind::ZeroCopyBufferMap
            | SectionKind::FastMetadataIndex
            | SectionKind::CoverageProviderRegistry
            | SectionKind::CoverageSet
            | SectionKind::CoveragePlanCandidate
            | SectionKind::PredicateNormalForm
            | SectionKind::CoverageProofRecord
            | SectionKind::IndexOnlyCapability
            | SectionKind::RuntimeCompatibilityHints
    )
}

fn is_layout_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::LayoutPlan
            | SectionKind::ScanSplitIndex
            | SectionKind::PageClusterDirectory
            | SectionKind::ZeroCopyBufferMap
            | SectionKind::FastMetadataIndex
    )
}

fn is_table_scan_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::TableCatalog
            | SectionKind::TableSegmentIndex
            | SectionKind::TableSegmentData
            | SectionKind::FileDictionaryIndex
            | SectionKind::FileDictionaryPayload
            | SectionKind::NestedSchema
            | SectionKind::ZoneStats
            | SectionKind::CodecExtensionRegistry
    )
}

fn is_coverage_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::CoverageProviderRegistry
            | SectionKind::CoverageSet
            | SectionKind::CoveragePlanCandidate
            | SectionKind::PredicateNormalForm
            | SectionKind::CoverageProofRecord
    )
}

fn is_engine_execution_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::EngineProfileRegistry
            | SectionKind::ExecutionCodeDescriptor
            | SectionKind::ExecutionScopeDescriptor
            | SectionKind::CodeSpaceDescriptor
            | SectionKind::EngineMountPolicy
    )
}

fn is_object_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::ObjectTypeCatalog
            | SectionKind::TemporalSegmentIndex
            | SectionKind::TemporalSegmentData
            | SectionKind::TemporalBloomIndex
            | SectionKind::TrustManifest
    )
}

fn is_harbor_section(kind: SectionKind) -> bool {
    kind == SectionKind::HarborMountHints
}

fn is_map_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::MapSourceCatalog
            | SectionKind::MapFunctionRegistry
            | SectionKind::MapIdentityRuleCatalog
            | SectionKind::MapRowSemanticsCatalog
            | SectionKind::MapAssertionLog
            | SectionKind::MapIdentityEquivalenceIndex
            | SectionKind::MapEvidenceIndex
            | SectionKind::MapConversionReport
            | SectionKind::MapProjectionCatalog
    )
}

#[derive(Clone, Copy)]
struct CoveTCrossSectionRefs<'a, 'data> {
    dictionary: Option<&'a FileDictionaryView<'data>>,
    zone_stats: &'a [ZoneStatsEntry],
    codec_descriptors: &'a [CodecExtensionDescriptorV2],
    registered_page_scope: RegisteredPageValidationScope<'a>,
}

fn validate_cove_t_cross_sections(
    catalogs: &[(u32, TableCatalog)],
    nested_schemas: &[(u32, NestedSchemaSectionV1)],
    segment_indexes: &[(u32, TableSegmentIndex)],
    segment_payloads: &[(u32, u64, TableSegmentPayloadV1, Vec<u8>)],
    refs: CoveTCrossSectionRefs<'_, '_>,
) -> Result<(), CoveError> {
    if catalogs.is_empty() && segment_indexes.is_empty() && segment_payloads.is_empty() {
        return Ok(());
    }
    if catalogs.len() != 1 {
        return Err(CoveError::BadSchema(
            "COVE-T validation requires exactly one TableCatalog section".into(),
        ));
    }
    let catalog = &catalogs[0].1;
    if nested_schemas.len() > 1 {
        return Err(CoveError::BadSchema(
            "COVE-T validation supports at most one NestedSchema section".into(),
        ));
    }
    let nested_schema = nested_schemas
        .first()
        .map(|(_section_id, nested_schema)| nested_schema);
    if let Some(nested_schema) = nested_schema {
        nested_schema.validate_for_catalog(catalog)?;
    } else if catalog.tables.iter().any(|table| {
        table
            .columns
            .iter()
            .any(crate::nested_schema::column_uses_nested_schema)
    }) {
        return Err(CoveError::BadSchema(
            "native nested COVE-T columns require a NestedSchema section".into(),
        ));
    }
    if segment_indexes.is_empty() && segment_payloads.is_empty() {
        if catalogs[0]
            .1
            .tables
            .iter()
            .all(|table| table.row_count == 0)
        {
            return Ok(());
        }
        return Err(CoveError::SegmentCorrupt);
    }
    if segment_indexes.len() != 1 {
        return Err(CoveError::SegmentCorrupt);
    }
    let segment_index = &segment_indexes[0].1;
    let tables = catalog
        .tables
        .iter()
        .map(|table| (table.table_id, table))
        .collect::<BTreeMap<_, _>>();
    let mut payloads_by_key = BTreeMap::new();
    for (section_id, file_offset, payload, bytes) in segment_payloads {
        if payloads_by_key
            .insert(
                (payload.header.table_id, payload.header.segment_id),
                (*section_id, *file_offset, payload, bytes),
            )
            .is_some()
        {
            return Err(CoveError::SegmentCorrupt);
        }
    }
    let mut rows_by_table = BTreeMap::<u32, u64>::new();
    for entry in &segment_index.entries {
        let table = tables.get(&entry.table_id).ok_or_else(|| {
            CoveError::BadSchema(format!(
                "segment index references unknown table_id {}",
                entry.table_id
            ))
        })?;
        if entry.column_count != table.columns.len() as u32 {
            return Err(CoveError::SegmentCorrupt);
        }
        let Some((section_id, file_offset, payload, bytes)) =
            payloads_by_key.get(&(entry.table_id, entry.segment_id))
        else {
            return Err(CoveError::SegmentCorrupt);
        };
        if *file_offset != entry.offset
            || payload.header.row_start != entry.row_start
            || payload.header.row_count != entry.row_count
            || payload.header.morsel_count != entry.morsel_count
            || payload.header.morsel_row_count != entry.morsel_row_count
            || payload.header.column_count != entry.column_count
        {
            return Err(CoveError::SegmentCorrupt);
        }
        if entry.length != bytes.len() as u64 {
            return Err(CoveError::SegmentCorrupt);
        }
        *rows_by_table.entry(entry.table_id).or_default() += u64::from(entry.row_count);
        validate_segment_against_catalog(
            table,
            *section_id,
            payload,
            bytes,
            SegmentValidationRefs {
                dictionary: refs.dictionary,
                zone_stats: refs.zone_stats,
                codec_descriptors: refs.codec_descriptors,
                nested_schema,
                registered_page_scope: refs.registered_page_scope,
            },
        )?;
    }
    for table in &catalog.tables {
        if rows_by_table.get(&table.table_id).copied().unwrap_or(0) != table.row_count {
            return Err(CoveError::SegmentCorrupt);
        }
        validate_declared_primary_sort_order(
            table,
            segment_index,
            &payloads_by_key,
            refs.zone_stats,
        )?;
    }
    if payloads_by_key.len() != segment_index.entries.len() {
        return Err(CoveError::SegmentCorrupt);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum SortValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Bytes(Vec<u8>),
}

impl SortValue {
    fn cmp_total(&self, other: &Self) -> Result<std::cmp::Ordering, CoveError> {
        use std::cmp::Ordering;
        match (self, other) {
            (Self::Null, Self::Null) => Ok(Ordering::Equal),
            (Self::Null, _) => Ok(Ordering::Less),
            (_, Self::Null) => Ok(Ordering::Greater),
            (Self::Bool(a), Self::Bool(b)) => Ok(a.cmp(b)),
            (Self::I64(a), Self::I64(b)) => Ok(a.cmp(b)),
            (Self::U64(a), Self::U64(b)) => Ok(a.cmp(b)),
            (Self::F32(a), Self::F32(b)) => Ok(a.total_cmp(b)),
            (Self::F64(a), Self::F64(b)) => Ok(a.total_cmp(b)),
            (Self::Bytes(a), Self::Bytes(b)) => Ok(a.cmp(b)),
            _ => Err(CoveError::BadSchema(
                "sort key value kinds do not match declared column type".into(),
            )),
        }
    }
}

type SegmentPayloadByKey<'a> =
    BTreeMap<(u32, u32), (u32, u64, &'a TableSegmentPayloadV1, &'a Vec<u8>)>;

fn validate_declared_primary_sort_order(
    table: &TableEntry,
    segment_index: &TableSegmentIndex,
    payloads_by_key: &SegmentPayloadByKey<'_>,
    zone_stats: &[ZoneStatsEntry],
) -> Result<(), CoveError> {
    if table.primary_sort_key_count == 0 {
        return Ok(());
    }
    let mut sort_columns = table
        .columns
        .iter()
        .filter(|column| column.sort_order != 0)
        .collect::<Vec<_>>();
    sort_columns.sort_by_key(|column| column.sort_order);
    if sort_columns.len() != usize::from(table.primary_sort_key_count) {
        return Err(CoveError::BadSchema(
            "primary sort key count does not match column sort declarations".into(),
        ));
    }
    if let Some(column) = sort_columns
        .iter()
        .find(|column| !can_validate_table_sort_column(column))
    {
        return Err(unsupported_sort_claim(column));
    }

    let mut segment_entries = segment_index
        .entries
        .iter()
        .filter(|entry| entry.table_id == table.table_id)
        .collect::<Vec<_>>();
    segment_entries.sort_by_key(|entry| entry.row_start);

    let mut previous_key = None::<Vec<SortValue>>;
    for segment_entry in segment_entries {
        let (_, _, segment, segment_bytes) = payloads_by_key
            .get(&(segment_entry.table_id, segment_entry.segment_id))
            .ok_or(CoveError::SegmentCorrupt)?;
        let mut page_sets = Vec::with_capacity(sort_columns.len());
        for column in &sort_columns {
            let pages = load_sort_column_pages(column, segment, segment_bytes, zone_stats)?;
            page_sets.push(pages);
        }
        let mut morsels = segment.morsels.entries.iter().collect::<Vec<_>>();
        morsels.sort_by_key(|morsel| morsel.first_row_in_segment);
        for morsel in morsels {
            for local_row in 0..morsel.row_count {
                let mut key = Vec::with_capacity(sort_columns.len());
                for (column_index, _column) in sort_columns.iter().enumerate() {
                    let (page, values) = page_sets[column_index]
                        .iter()
                        .find(|(page, _)| page.morsel_id == morsel.morsel_id)
                        .ok_or(CoveError::PageCorrupt)?;
                    if local_row >= page.row_count {
                        return Err(CoveError::PageCorrupt);
                    }
                    key.push(
                        values
                            .get(usize::try_from(local_row).map_err(|_| CoveError::ArithOverflow)?)
                            .ok_or(CoveError::PageCorrupt)?
                            .clone(),
                    );
                }
                if let Some(previous) = &previous_key {
                    if compare_sort_keys(previous, &key)? == std::cmp::Ordering::Greater {
                        return Err(CoveError::BadSchema(
                            "declared primary sort order does not match row data".into(),
                        ));
                    }
                }
                previous_key = Some(key);
            }
        }
    }
    Ok(())
}

fn unsupported_sort_claim(column: &ColumnEntry) -> CoveError {
    CoveError::UnsupportedEncoding(format!(
        "declared primary sort key column {} uses unsupported validation semantics",
        column.column_id
    ))
}

fn can_validate_table_sort_column(column: &ColumnEntry) -> bool {
    if column.collation_id != 0 {
        return false;
    }
    !matches!(
        column.physical,
        CovePhysicalKind::FileCode
            | CovePhysicalKind::List
            | CovePhysicalKind::Struct
            | CovePhysicalKind::Map
    )
}

fn load_sort_column_pages(
    column: &ColumnEntry,
    segment: &TableSegmentPayloadV1,
    segment_bytes: &[u8],
    zone_stats: &[ZoneStatsEntry],
) -> Result<Vec<(crate::page::ColumnPageIndexEntryV1, Vec<SortValue>)>, CoveError> {
    let column_dir = segment
        .columns
        .iter()
        .find(|candidate| candidate.column_id == column.column_id)
        .ok_or(CoveError::SegmentCorrupt)?;
    let page_index_start =
        usize::try_from(column_dir.page_index_offset).map_err(|_| CoveError::OffsetRange)?;
    let page_index_end = usize::try_from(
        column_dir
            .page_index_offset
            .checked_add(column_dir.page_index_length)
            .ok_or(CoveError::ArithOverflow)?,
    )
    .map_err(|_| CoveError::OffsetRange)?;
    let page_index = ColumnPageIndex::parse(&segment_bytes[page_index_start..page_index_end])?;
    let mut pages = Vec::with_capacity(page_index.entries.len());
    for page in page_index.entries {
        let payload_owner = if page.flags & PAGE_FLAG_STATS_ONLY_CONSTANT != 0 {
            materialize_stats_only_constant_page_payload(
                StatsOnlyPageMaterializationContext {
                    table_id: Some(segment.header.table_id),
                    segment_id: Some(segment.header.segment_id),
                    column_id: column.column_id,
                    logical_type: column.logical,
                    physical_kind: column.physical,
                    dictionary_len: None,
                    zone_stats,
                },
                &page,
            )?
        } else {
            let start = usize::try_from(page.page_offset).map_err(|_| CoveError::OffsetRange)?;
            let end = usize::try_from(
                page.page_offset
                    .checked_add(page.page_length)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .map_err(|_| CoveError::OffsetRange)?;
            let page_wire = &segment_bytes[start..end];
            compression::column_page_payload(page_wire, &page)?.into_owned()
        };
        let payload = ColumnPagePayloadV1::parse(&payload_owner)?;
        let root = payload.root_node()?;
        if root.encoding_kind == CoveEncodingKind::RegisteredEncoding {
            return Err(unsupported_sort_claim(column));
        }
        let null_bitmap = payload.buffer_bytes(PageBufferKind::NullBitmap)?;
        let validity =
            null_bitmap.map(|bytes| ValidityBitmap::new(bytes, u64::from(page.row_count)));
        let values = payload.buffer_bytes(PageBufferKind::Values)?.unwrap_or(&[]);
        let array = EncodedArray::new(
            column.logical,
            column.physical,
            u64::from(page.row_count),
            root.encoding_kind,
            validity,
            values,
            None,
        );
        let prepared = array.prepare()?;
        let values = (0..page.row_count)
            .map(|row| {
                sort_value_from_array(
                    column.logical,
                    column.physical,
                    prepared.decode_row(u64::from(row))?,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        pages.push((page, values));
    }
    Ok(pages)
}

fn compare_sort_keys(
    left: &[SortValue],
    right: &[SortValue],
) -> Result<std::cmp::Ordering, CoveError> {
    for (left, right) in left.iter().zip(right) {
        let ordering = left.cmp_total(right)?;
        if ordering != std::cmp::Ordering::Equal {
            return Ok(ordering);
        }
    }
    Ok(std::cmp::Ordering::Equal)
}

fn sort_value_from_array(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    value: CoveArrayValue<'_>,
) -> Result<SortValue, CoveError> {
    match value {
        CoveArrayValue::Null => Ok(SortValue::Null),
        CoveArrayValue::Boolean(value) | CoveArrayValue::ValidityBit(value) => {
            Ok(SortValue::Bool(value))
        }
        CoveArrayValue::NumCode(value) | CoveArrayValue::Varint(value) => {
            sort_value_from_numcode(logical, value)
        }
        CoveArrayValue::Int64(value) => Ok(SortValue::I64(value)),
        CoveArrayValue::Bytes(bytes) => sort_value_from_bytes(logical, physical, bytes),
        CoveArrayValue::OwnedBytes(bytes) => sort_value_from_bytes(logical, physical, &bytes),
        CoveArrayValue::FileCode(_) | CoveArrayValue::DictValue(_) => Err(CoveError::BadSchema(
            "declared sort keys over FileCode require domain-aware validation".into(),
        )),
    }
}

fn sort_value_from_numcode(logical: CoveLogicalType, value: u64) -> Result<SortValue, CoveError> {
    match logical {
        CoveLogicalType::Bool => match value {
            0 => Ok(SortValue::Bool(false)),
            1 => Ok(SortValue::Bool(true)),
            _ => Err(CoveError::PageCorrupt),
        },
        CoveLogicalType::Int8 => Ok(SortValue::I64(i64::from(numcode_as_i8(value)))),
        CoveLogicalType::Int16 => Ok(SortValue::I64(i64::from(numcode_as_i16(value)))),
        CoveLogicalType::Int32 => Ok(SortValue::I64(i64::from(numcode_as_i32(value)))),
        CoveLogicalType::Int64 => Ok(SortValue::I64(numcode_as_i64(value))),
        CoveLogicalType::UInt8 => Ok(SortValue::U64(u64::from(numcode_as_u8(value)))),
        CoveLogicalType::UInt16 => Ok(SortValue::U64(u64::from(numcode_as_u16(value)))),
        CoveLogicalType::UInt32 => Ok(SortValue::U64(u64::from(numcode_as_u32(value)))),
        CoveLogicalType::UInt64 => Ok(SortValue::U64(numcode_as_u64(value))),
        CoveLogicalType::Float32 => Ok(SortValue::F32(numcode_as_f32(value))),
        CoveLogicalType::Float64 => Ok(SortValue::F64(numcode_as_f64(value))),
        CoveLogicalType::Decimal64 => Ok(SortValue::I64(numcode_as_decimal64(value))),
        CoveLogicalType::DateDays => Ok(SortValue::I64(i64::from(numcode_as_date_days(value)))),
        CoveLogicalType::TimestampMicros => Ok(SortValue::I64(numcode_as_timestamp_micros(value))),
        CoveLogicalType::TimestampNanos => Ok(SortValue::I64(numcode_as_timestamp_nanos(value))),
        _ => Err(CoveError::BadSchema(
            "declared sort key logical type is not NumCode-comparable".into(),
        )),
    }
}

fn sort_value_from_bytes(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    bytes: &[u8],
) -> Result<SortValue, CoveError> {
    match physical {
        CovePhysicalKind::Boolean => match bytes {
            [0] => Ok(SortValue::Bool(false)),
            [1] => Ok(SortValue::Bool(true)),
            _ => Err(CoveError::PageCorrupt),
        },
        CovePhysicalKind::FixedBytes | CovePhysicalKind::VarBytes => {
            if logical == CoveLogicalType::Utf8 {
                std::str::from_utf8(bytes).map_err(|_| CoveError::PageCorrupt)?;
            }
            if logical == CoveLogicalType::Json {
                serde_json::from_slice::<serde_json::Value>(bytes)
                    .map_err(|_| CoveError::PageCorrupt)?;
            }
            Ok(SortValue::Bytes(bytes.to_vec()))
        }
        _ => Err(CoveError::BadSchema(
            "declared sort key physical kind is not directly byte-comparable".into(),
        )),
    }
}

#[derive(Clone, Copy)]
pub(super) enum RegisteredPageValidationScope<'a> {
    All,
    RequestedPages(&'a FeatureUseRequestV2),
}

impl RegisteredPageValidationScope<'_> {
    fn should_materialize_registered_page(
        &self,
        section_id: u32,
        column_id: u32,
        morsel_id: u32,
    ) -> bool {
        match self {
            Self::All => true,
            Self::RequestedPages(request) => {
                request
                    .needed_page_refs
                    .contains(&FeatureTargetRefV2::cove_t_column_page(
                        section_id, column_id, morsel_id,
                    ))
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SegmentValidationRefs<'a, 'data> {
    dictionary: Option<&'a FileDictionaryView<'data>>,
    zone_stats: &'a [ZoneStatsEntry],
    codec_descriptors: &'a [CodecExtensionDescriptorV2],
    nested_schema: Option<&'a NestedSchemaSectionV1>,
    registered_page_scope: RegisteredPageValidationScope<'a>,
}

fn validate_segment_against_catalog(
    table: &TableEntry,
    segment_section_id: u32,
    segment: &TableSegmentPayloadV1,
    segment_bytes: &[u8],
    refs: SegmentValidationRefs<'_, '_>,
) -> Result<(), CoveError> {
    if segment.header.table_id != table.table_id {
        return Err(CoveError::SegmentCorrupt);
    }
    let columns = table
        .columns
        .iter()
        .map(|column| (column.column_id, column))
        .collect::<BTreeMap<_, _>>();
    if segment.columns.len() != table.columns.len() {
        return Err(CoveError::SegmentCorrupt);
    }
    for column_dir in &segment.columns {
        let column = columns.get(&column_dir.column_id).ok_or_else(|| {
            CoveError::BadSchema(format!(
                "segment references unknown column_id {}",
                column_dir.column_id
            ))
        })?;
        if column_dir.logical_type != column.logical || column_dir.physical_kind != column.physical
        {
            return Err(CoveError::PageCorrupt);
        }
        if (column.flags & COLUMN_FLAG_BOOL_DECLARED_NUMERIC != 0)
            != (column_dir.flags & SEGMENT_COLUMN_FLAG_BOOL_DECLARED_NUMERIC != 0)
        {
            return Err(CoveError::PageCorrupt);
        }
        validate_column_pages_against_catalog(
            column,
            column_dir,
            segment_section_id,
            segment,
            segment_bytes,
            refs,
        )?;
    }
    Ok(())
}

fn validate_column_pages_against_catalog(
    column: &ColumnEntry,
    column_dir: &TableColumnDirectoryEntryV1,
    segment_section_id: u32,
    segment: &TableSegmentPayloadV1,
    segment_bytes: &[u8],
    refs: SegmentValidationRefs<'_, '_>,
) -> Result<(), CoveError> {
    if column.physical == CovePhysicalKind::FileCode && refs.dictionary.is_none() {
        return Err(CoveError::BadFileCode);
    }
    let page_index_start =
        usize::try_from(column_dir.page_index_offset).map_err(|_| CoveError::OffsetRange)?;
    let page_index_end = usize::try_from(
        column_dir
            .page_index_offset
            .checked_add(column_dir.page_index_length)
            .ok_or(CoveError::ArithOverflow)?,
    )
    .map_err(|_| CoveError::OffsetRange)?;
    let page_index = ColumnPageIndex::parse(&segment_bytes[page_index_start..page_index_end])?;
    segment.morsels.validate_page_index_coverage(&page_index)?;
    for page in &page_index.entries {
        if !column.nullable && page.null_count != 0 {
            return Err(CoveError::BadSchema(format!(
                "non-nullable column {} has page null_count {}",
                column.column_id, page.null_count
            )));
        }
        let context = PageValidationContext {
            table_id: Some(segment.header.table_id),
            segment_id: Some(segment.header.segment_id),
            column_id: column.column_id,
            logical_type: column.logical,
            physical_kind: column.physical,
            dictionary: refs.dictionary,
            zone_stats: Some(refs.zone_stats),
            codec_descriptors: refs.codec_descriptors,
            nested_schema: refs
                .nested_schema
                .and_then(|schema| schema.entry(segment.header.table_id, column.column_id))
                .map(|entry| &entry.root),
        };
        if page.page_length == 0 {
            validate_stats_only_constant_page(&context, page)?;
            continue;
        }
        let start = usize::try_from(page.page_offset).map_err(|_| CoveError::OffsetRange)?;
        let end = usize::try_from(
            page.page_offset
                .checked_add(page.page_length)
                .ok_or(CoveError::ArithOverflow)?,
        )
        .map_err(|_| CoveError::OffsetRange)?;
        let page_wire = &segment_bytes[start..end];
        if page.encoding_root == crate::constants::CoveEncodingKind::RegisteredEncoding as u32
            && !refs
                .registered_page_scope
                .should_materialize_registered_page(
                    segment_section_id,
                    column.column_id,
                    page.morsel_id,
                )
        {
            crate::segment::validate_registered_page_wire_without_descriptor(
                column_dir, page, page_wire,
            )?;
        } else {
            validate_column_page_wire(&context, page, page_wire)?;
        }
    }
    Ok(())
}

pub(super) fn validate_cove_o_semantics(
    data: &[u8],
    validated: &ValidatedCoveFile,
    stages: &mut Vec<ValidationStageReport>,
    request: Option<&FeatureUseRequestV2>,
) -> Result<(), CoveError> {
    let mut checked = 0u32;
    let mut object_catalogs = Vec::new();
    let mut temporal_indexes = Vec::new();
    let mut temporal_segment_payloads = Vec::new();
    let mut trust_manifests = Vec::new();
    let mut zone_stats_entries = Vec::new();
    let mut codec_descriptors = Vec::new();
    let dictionary = parse_validation_dictionary(data, &validated.footer)?;
    for entry in &validated.footer.sections {
        let kind = SectionKind::from_u16(entry.section_kind).ok_or_else(|| {
            CoveError::BadSection(format!("unknown section_kind {}", entry.section_kind))
        })?;
        let result = match kind {
            SectionKind::ObjectTypeCatalog => {
                let payload = compression::section_payload(data, entry)?;
                ObjectTypeCatalog::parse(&payload).map(|catalog| {
                    object_catalogs.push(catalog);
                })
            }
            SectionKind::TemporalSegmentIndex => {
                let payload = compression::section_payload(data, entry)?;
                TemporalSegmentIndex::parse(&payload).map(|index| {
                    temporal_indexes.push(index);
                })
            }
            SectionKind::TemporalSegmentData => {
                let payload = compression::section_payload(data, entry)?;
                temporal_segment_payloads.push((
                    entry.offset,
                    payload.into_owned(),
                    entry.required_features | entry.optional_features,
                ));
                Ok(())
            }
            SectionKind::TemporalBloomIndex => {
                let payload = compression::section_payload(data, entry)?;
                TemporalBloomIndex::parse(&payload).map(|_| ())
            }
            SectionKind::TrustManifest => {
                let payload = compression::section_payload(data, entry)?;
                TrustManifest::parse(&payload).map(|manifest| {
                    trust_manifests.push(manifest);
                })
            }
            SectionKind::ZoneStats => {
                let payload = compression::section_payload(data, entry)?;
                ZoneStatsSection::parse(&payload).map(|section| {
                    zone_stats_entries.extend(section.entries);
                })
            }
            SectionKind::CodecExtensionRegistry => {
                let payload = compression::section_payload(data, entry)?;
                CodecExtensionDescriptorV2::parse_many(&payload).map(|descriptors| {
                    codec_descriptors.extend(descriptors);
                })
            }
            _ => continue,
        };
        checked += 1;
        if let Err(err) = result {
            if profile_error_is_fatal(&validated.header, entry, FEATURE_OBJECT_PROFILE)
                || request
                    .map(request_requires_object_profile)
                    .unwrap_or(false)
            {
                return Err(err);
            }
        }
    }
    let mut temporal_segments = Vec::with_capacity(temporal_segment_payloads.len());
    for (section_offset, payload, section_features) in temporal_segment_payloads {
        let result = TemporalSegmentData::parse_with_feature_advertisement_and_codec_descriptors(
            &payload,
            validated.header.required_features,
            validated.header.required_features | validated.header.optional_features,
            section_features,
            &codec_descriptors,
        );
        let segment = match result {
            Ok(segment) => segment,
            Err(err) if cove_o_profile_required(validated, request) => return Err(err),
            Err(_) => continue,
        };
        temporal_segments.push((section_offset, payload, segment));
    }
    if let Err(err) = validate_cove_o_cross_sections(
        &object_catalogs,
        &temporal_indexes,
        &temporal_segments,
        &trust_manifests,
        CoveOCrossSectionRefs {
            dictionary: dictionary.as_ref(),
            zone_stats: &zone_stats_entries,
            codec_descriptors: &codec_descriptors,
            required_features: validated.header.required_features,
        },
    ) {
        if cove_o_profile_required(validated, request) {
            return Err(err);
        }
    }
    push_stage(
        stages,
        ValidationStage::CoveObject,
        ValidationStageStatus::Checked,
        checked,
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct CoveOCrossSectionRefs<'a, 'data> {
    dictionary: Option<&'a FileDictionaryView<'data>>,
    zone_stats: &'a [ZoneStatsEntry],
    codec_descriptors: &'a [CodecExtensionDescriptorV2],
    required_features: u64,
}

fn validate_cove_o_cross_sections(
    catalogs: &[ObjectTypeCatalog],
    indexes: &[TemporalSegmentIndex],
    segments: &[(u64, Vec<u8>, TemporalSegmentData)],
    trust_manifests: &[TrustManifest],
    refs: CoveOCrossSectionRefs<'_, '_>,
) -> Result<(), CoveError> {
    if catalogs.is_empty()
        && indexes.is_empty()
        && segments.is_empty()
        && trust_manifests.is_empty()
    {
        return Ok(());
    }
    if catalogs.len() != 1 {
        return Err(CoveError::BadSchema(
            "COVE-O validation requires exactly one ObjectTypeCatalog section".into(),
        ));
    }
    if !segments.is_empty() && indexes.len() != 1 {
        return Err(CoveError::SegmentCorrupt);
    }
    if segments.is_empty() {
        if indexes.iter().all(|index| index.entries.is_empty()) {
            return Ok(());
        }
        return Err(CoveError::SegmentCorrupt);
    }

    let catalog = &catalogs[0];
    let object_types = catalog
        .types
        .iter()
        .map(|ty| (ty.object_type_id, ty))
        .collect::<BTreeMap<_, _>>();
    let index = &indexes[0];
    let index_entries = index
        .entries
        .iter()
        .map(|entry| ((entry.object_type_id, entry.segment_id), entry))
        .collect::<BTreeMap<_, _>>();
    if index_entries.len() != index.entries.len() {
        return Err(CoveError::SegmentCorrupt);
    }

    let segment_refs = segments
        .iter()
        .map(|(_, _, segment)| segment)
        .collect::<Vec<_>>();
    validate_temporal_segment_ids_file_unique(&segment_refs)?;
    let segment_values = segments
        .iter()
        .map(|(_, _, segment)| segment.clone())
        .collect::<Vec<_>>();
    let file_local_record_ids = segment_refs
        .iter()
        .flat_map(|segment| {
            (0..segment.rows.len())
                .map(move |row_index| ((segment.header.segment_id as u64) << 32) | row_index as u64)
        })
        .collect::<Vec<_>>();
    let file_prev_refs = segment_refs
        .iter()
        .flat_map(|segment| {
            segment.rows.iter().map(|row| {
                row.prev_ref.map(|prev_ref| {
                    ((prev_ref.segment_id as u64) << 32) | prev_ref.row_index as u64
                })
            })
        })
        .collect::<Vec<_>>();
    validate_self_contained(&file_prev_refs, &file_local_record_ids)?;
    validate_temporal_chains(&segment_refs)?;

    let mut payloads_by_key = BTreeMap::new();
    for (section_offset, bytes, segment) in segments {
        let object_type = object_types
            .get(&segment.header.object_type_id)
            .ok_or_else(|| {
                CoveError::BadSchema(format!(
                    "temporal segment references unknown object_type_id {}",
                    segment.header.object_type_id
                ))
            })?;
        let key = (segment.header.object_type_id, segment.header.segment_id);
        if payloads_by_key
            .insert(key, (*section_offset, bytes, segment))
            .is_some()
        {
            return Err(CoveError::SegmentCorrupt);
        }
        let index_entry = index_entries.get(&key).ok_or(CoveError::SegmentCorrupt)?;
        validate_temporal_segment_against_index(index_entry, *section_offset, bytes, segment)?;
        validate_temporal_property_columns(
            object_type,
            segment,
            refs.dictionary,
            refs.zone_stats,
            refs.codec_descriptors,
            refs.required_features,
        )?;
    }
    if payloads_by_key.len() != index.entries.len() {
        return Err(CoveError::SegmentCorrupt);
    }

    for manifest in trust_manifests {
        validate_trust_manifest_references(manifest, &segment_refs)?;
        manifest.verify_against_with_dictionary(
            &segment_values,
            refs.dictionary,
            refs.zone_stats,
        )?;
    }
    Ok(())
}

fn validate_temporal_segment_ids_file_unique(
    segments: &[&TemporalSegmentData],
) -> Result<(), CoveError> {
    let mut seen = BTreeSet::new();
    for segment in segments {
        if !seen.insert(segment.header.segment_id) {
            return Err(CoveError::RefInvalid);
        }
    }
    Ok(())
}

fn validate_temporal_segment_against_index(
    index: &TemporalSegmentIndexEntryV1,
    section_offset: u64,
    bytes: &[u8],
    segment: &TemporalSegmentData,
) -> Result<(), CoveError> {
    if index.segment_id != segment.header.segment_id
        || index.object_type_id != segment.header.object_type_id
        || index.time_range_start_us != segment.header.time_range_start_us
        || index.time_range_end_us != segment.header.time_range_end_us
        || index.csn_min != segment.header.csn_min
        || index.csn_max != segment.header.csn_max
        || index.row_count != segment.header.row_count
        || index.length != bytes.len() as u64
    {
        return Err(CoveError::SegmentCorrupt);
    }
    if index.offset != 0 && index.offset != section_offset {
        return Err(CoveError::SegmentCorrupt);
    }
    let counts = temporal_record_kind_counts(segment);
    if index.delta_count != counts.0
        || index.snapshot_count != counts.1
        || index.baseline_count != counts.2
        || index.tombstone_count != counts.3
    {
        return Err(CoveError::SegmentCorrupt);
    }
    if !segment.rows.is_empty() {
        let min_goid = segment.rows.iter().map(|row| row.goid).min().unwrap();
        let max_goid = segment.rows.iter().map(|row| row.goid).max().unwrap();
        if index.min_goid != min_goid || index.max_goid != max_goid {
            return Err(CoveError::SegmentCorrupt);
        }
    }
    Ok(())
}

fn temporal_record_kind_counts(segment: &TemporalSegmentData) -> (u32, u32, u32, u32) {
    let mut delta = 0u32;
    let mut snapshot = 0u32;
    let mut baseline = 0u32;
    let mut tombstone = 0u32;
    for row in &segment.rows {
        match row.record_kind {
            RecordKind::Delta => delta += 1,
            RecordKind::Snapshot => snapshot += 1,
            RecordKind::Baseline => baseline += 1,
            RecordKind::Tombstone => tombstone += 1,
            RecordKind::ReservedLegacyMaterializedDelta => {}
        }
    }
    (delta, snapshot, baseline, tombstone)
}

fn validate_temporal_property_columns(
    object_type: &crate::profile::cove_o::ObjectTypeEntryV1,
    segment: &TemporalSegmentData,
    dictionary: Option<&FileDictionaryView<'_>>,
    zone_stats: &[ZoneStatsEntry],
    codec_descriptors: &[CodecExtensionDescriptorV2],
    required_features: u64,
) -> Result<(), CoveError> {
    let properties = object_type
        .properties
        .iter()
        .map(|property| (property.property_id, property))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for column in &segment.property_columns {
        if !seen.insert(column.directory.column_id) {
            return Err(CoveError::BadSchema(format!(
                "duplicate temporal property column_id {}",
                column.directory.column_id
            )));
        }
        let property = properties.get(&column.directory.column_id).ok_or_else(|| {
            CoveError::BadSchema(format!(
                "temporal segment references unknown property_id {} for object_type_id {}",
                column.directory.column_id, object_type.object_type_id
            ))
        })?;
        if column.directory.logical_type != property.logical_type
            || column.directory.physical_kind != property.physical_kind
        {
            return Err(CoveError::PageCorrupt);
        }
        if (property.flags & PROPERTY_FLAG_BOOL_DECLARED_NUMERIC != 0)
            != (column.directory.flags & SEGMENT_COLUMN_FLAG_BOOL_DECLARED_NUMERIC != 0)
        {
            return Err(CoveError::PageCorrupt);
        }
        validate_temporal_property_pages(
            property,
            segment,
            column,
            dictionary,
            zone_stats,
            codec_descriptors,
            required_features,
        )?;
    }
    if segment.header.column_count as usize != segment.property_columns.len() {
        return Err(CoveError::SegmentCorrupt);
    }
    Ok(())
}

fn validate_temporal_property_pages(
    property: &PropertyEntryV1,
    segment: &TemporalSegmentData,
    column: &TemporalPropertyColumn,
    dictionary: Option<&FileDictionaryView<'_>>,
    zone_stats: &[ZoneStatsEntry],
    codec_descriptors: &[CodecExtensionDescriptorV2],
    required_features: u64,
) -> Result<(), CoveError> {
    if property.physical_kind == CovePhysicalKind::FileCode && dictionary.is_none() {
        return Err(CoveError::BadFileCode);
    }
    if column.page_index.entries.len() != expected_temporal_morsel_count(segment)? {
        return Err(CoveError::PageCorrupt);
    }
    let mut seen_morsels = BTreeSet::new();
    let mut rows_seen = 0u64;
    for page in &column.pages {
        if !seen_morsels.insert(page.index_entry.morsel_id) {
            return Err(CoveError::PageCorrupt);
        }
        let expected_rows = temporal_morsel_row_count(segment, page.index_entry.morsel_id)?;
        if page.index_entry.row_count != expected_rows {
            return Err(CoveError::PageCorrupt);
        }
        if !property.nullable && page.index_entry.null_count != 0 {
            return Err(CoveError::BadSchema(format!(
                "non-nullable property {} has page null_count {}",
                property.property_id, page.index_entry.null_count
            )));
        }
        validate_temporal_property_page_elision_features(
            &page.index_entry,
            Some(required_features),
        )?;
        rows_seen = rows_seen
            .checked_add(u64::from(page.index_entry.row_count))
            .ok_or(CoveError::ArithOverflow)?;
        let context = PageValidationContext {
            table_id: None,
            segment_id: Some(segment.header.segment_id),
            column_id: property.property_id,
            logical_type: property.logical_type,
            physical_kind: property.physical_kind,
            dictionary,
            zone_stats: Some(zone_stats),
            codec_descriptors,
            nested_schema: None,
        };
        if let Some(payload) = &page.payload {
            validate_column_page_payload(&context, &page.index_entry, payload)?;
        } else {
            validate_temporal_property_stats_only_page(&context, &page.index_entry)?;
        }
    }
    if rows_seen != u64::from(segment.header.row_count) {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

fn expected_temporal_morsel_count(segment: &TemporalSegmentData) -> Result<usize, CoveError> {
    if segment.header.row_count == 0 {
        return Ok(0);
    }
    if segment.header.morsel_count == 0 || segment.header.morsel_row_count == 0 {
        return Err(CoveError::SegmentCorrupt);
    }
    Ok(segment.header.morsel_count as usize)
}

fn temporal_morsel_row_count(
    segment: &TemporalSegmentData,
    morsel_id: u32,
) -> Result<u32, CoveError> {
    if morsel_id >= segment.header.morsel_count {
        return Err(CoveError::SegmentCorrupt);
    }
    let first_row = morsel_id
        .checked_mul(segment.header.morsel_row_count)
        .ok_or(CoveError::ArithOverflow)?;
    if first_row >= segment.header.row_count {
        return Err(CoveError::SegmentCorrupt);
    }
    let remaining = segment.header.row_count - first_row;
    Ok(remaining.min(segment.header.morsel_row_count))
}

fn validate_temporal_chains(segments: &[&TemporalSegmentData]) -> Result<(), CoveError> {
    let rows = segments
        .iter()
        .flat_map(|segment| {
            segment
                .rows
                .iter()
                .enumerate()
                .map(move |(row_index, row)| {
                    (
                        (segment.header.segment_id, row_index as u32),
                        (segment.header.object_type_id, row),
                    )
                })
        })
        .collect::<BTreeMap<_, _>>();
    for ((segment_id, row_index), (object_type_id, row)) in &rows {
        validate_prev_ref_target_kind((*object_type_id, row), row.prev_ref, &rows)?;
        if matches!(row.record_kind, RecordKind::Delta | RecordKind::Tombstone) {
            let mut seen = BTreeSet::new();
            let mut current = Some((*segment_id, *row_index));
            let mut anchored = false;
            while let Some(key) = current {
                if !seen.insert(key) {
                    return Err(CoveError::RefInvalid);
                }
                let (_, current_row) = rows.get(&key).ok_or(CoveError::NotSelfContained)?;
                if matches!(
                    current_row.record_kind,
                    RecordKind::Baseline | RecordKind::Snapshot
                ) {
                    anchored = true;
                    break;
                }
                if current_row.prev_ref.is_none() {
                    anchored = true;
                    break;
                }
                current = current_row
                    .prev_ref
                    .map(|prev_ref| (prev_ref.segment_id, prev_ref.row_index));
            }
            if !anchored && row.prev_ref.is_some() {
                return Err(CoveError::NotSelfContained);
            }
        }
    }
    Ok(())
}

fn validate_prev_ref_target_kind(
    current: (u32, &crate::profile::cove_o::TemporalRowEntryV1),
    prev_ref: Option<crate::profile::cove_o::CoveRecordRefV1>,
    rows: &BTreeMap<(u32, u32), (u32, &crate::profile::cove_o::TemporalRowEntryV1)>,
) -> Result<(), CoveError> {
    let Some(prev_ref) = prev_ref else {
        return Ok(());
    };
    let (current_object_type_id, current_row) = current;
    let (target_object_type_id, target) = rows
        .get(&(prev_ref.segment_id, prev_ref.row_index))
        .ok_or(CoveError::NotSelfContained)?;
    if prev_ref.target_kind != target_kind_for_record_kind(target.record_kind) {
        return Err(CoveError::RefInvalid);
    }
    if *target_object_type_id != current_object_type_id
        || target.branch_key != current_row.branch_key
        || target.goid != current_row.goid
        || target.row_key().cmp_lex(&current_row.row_key()) != std::cmp::Ordering::Less
    {
        return Err(CoveError::RefInvalid);
    }
    Ok(())
}

fn target_kind_for_record_kind(kind: RecordKind) -> u8 {
    match kind {
        RecordKind::Snapshot | RecordKind::Baseline => 1,
        RecordKind::Delta | RecordKind::Tombstone | RecordKind::ReservedLegacyMaterializedDelta => {
            0
        }
    }
}

fn validate_trust_manifest_references(
    manifest: &TrustManifest,
    segments: &[&TemporalSegmentData],
) -> Result<(), CoveError> {
    let row_counts = segments
        .iter()
        .map(|segment| (segment.header.segment_id, segment.rows.len()))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for entry in &manifest.entries {
        if !seen.insert((entry.segment_id, entry.row_index)) {
            return Err(CoveError::RefInvalid);
        }
        let row_count = row_counts
            .get(&entry.segment_id)
            .ok_or(CoveError::RefInvalid)?;
        if entry.row_index as usize >= *row_count {
            return Err(CoveError::RefInvalid);
        }
    }
    Ok(())
}

pub(super) fn validate_cove_e_semantics(
    data: &[u8],
    validated: &ValidatedCoveFile,
    stages: &mut Vec<ValidationStageReport>,
    request: Option<&FeatureUseRequestV2>,
) -> Result<(), CoveError> {
    let mut checked = 0u32;
    let mut registries = Vec::new();
    let mut execution_descriptors = Vec::new();
    let mut scope_descriptors = Vec::new();
    let mut code_space_descriptors = Vec::new();
    let mut mount_policies = Vec::new();
    for entry in &validated.footer.sections {
        let kind = SectionKind::from_u16(entry.section_kind).ok_or_else(|| {
            CoveError::BadSection(format!("unknown section_kind {}", entry.section_kind))
        })?;
        let result = match kind {
            SectionKind::EngineProfileRegistry => {
                let payload = compression::section_payload(data, entry)?;
                EngineProfileRegistry::parse(&payload).map(|registry| {
                    registries.push(registry);
                })
            }
            SectionKind::ExecutionCodeDescriptor => {
                let payload = compression::section_payload(data, entry)?;
                ExecutionCodeDescriptorV1::parse(&payload).map(|descriptor| {
                    execution_descriptors.push(descriptor);
                })
            }
            SectionKind::ExecutionScopeDescriptor => {
                let payload = compression::section_payload(data, entry)?;
                ExecutionScopeDescriptorV1::parse(&payload).map(|descriptor| {
                    scope_descriptors.push(descriptor);
                })
            }
            SectionKind::CodeSpaceDescriptor => {
                let payload = compression::section_payload(data, entry)?;
                CodeSpaceDescriptorV1::parse(&payload).map(|descriptor| {
                    code_space_descriptors.push(descriptor);
                })
            }
            SectionKind::EngineMountPolicy => {
                let payload = compression::section_payload(data, entry)?;
                EngineMountPolicyV1::parse(&payload).map(|policy| {
                    mount_policies.push(policy);
                })
            }
            _ => continue,
        };
        checked += 1;
        if let Err(err) = result {
            if profile_error_is_fatal(&validated.header, entry, FEATURE_ENGINE_PROFILE)
                || request
                    .map(request_requires_engine_profile)
                    .unwrap_or(false)
            {
                return Err(err);
            }
        }
    }
    if let Err(err) = validate_cove_e_cross_references(
        &registries,
        &execution_descriptors,
        &scope_descriptors,
        &code_space_descriptors,
        &mount_policies,
    ) {
        if cove_e_profile_required(validated, request) {
            return Err(err);
        }
    }
    push_stage(
        stages,
        ValidationStage::CoveEngine,
        ValidationStageStatus::Checked,
        checked,
    );
    Ok(())
}

fn validate_cove_e_cross_references(
    registries: &[EngineProfileRegistry],
    execution_descriptors: &[ExecutionCodeDescriptorV1],
    scope_descriptors: &[ExecutionScopeDescriptorV1],
    code_space_descriptors: &[CodeSpaceDescriptorV1],
    mount_policies: &[EngineMountPolicyV1],
) -> Result<(), CoveError> {
    use std::collections::HashSet;

    let mut execution_ids = HashSet::new();
    for descriptor in execution_descriptors {
        if !execution_ids.insert(descriptor.descriptor_id) {
            return Err(CoveError::BadEngineProfile);
        }
    }

    let mut scope_ids = HashSet::new();
    for descriptor in scope_descriptors {
        if !scope_ids.insert(descriptor.scope_id) {
            return Err(CoveError::BadEngineProfile);
        }
    }

    let mut code_space_ids = HashSet::new();
    for descriptor in code_space_descriptors {
        if !code_space_ids.insert(descriptor.code_space_id) {
            return Err(CoveError::BadEngineProfile);
        }
    }

    let mut policy_ids = HashSet::new();
    for policy in mount_policies {
        if !policy_ids.insert(policy.policy_id) {
            return Err(CoveError::BadEngineProfile);
        }
    }

    for registry in registries {
        for profile in &registry.profiles {
            if profile.execution_descriptor_ref != 0
                && !execution_ids.contains(&profile.execution_descriptor_ref)
            {
                return Err(CoveError::BadEngineProfile);
            }
            if profile.mount_policy_ref != 0 && !policy_ids.contains(&profile.mount_policy_ref) {
                return Err(CoveError::BadEngineProfile);
            }
        }
    }

    for descriptor in execution_descriptors {
        if descriptor.scope_ref != 0 && !scope_ids.contains(&descriptor.scope_ref) {
            return Err(CoveError::BadEngineProfile);
        }
        if descriptor.code_space_ref != 0 && !code_space_ids.contains(&descriptor.code_space_ref) {
            return Err(CoveError::BadEngineProfile);
        }
    }

    for policy in mount_policies {
        if policy.code_space_ref != 0 && !code_space_ids.contains(&policy.code_space_ref) {
            return Err(CoveError::BadEngineProfile);
        }
    }

    Ok(())
}

pub(super) fn validate_cove_h_semantics(
    data: &[u8],
    validated: &ValidatedCoveFile,
    stages: &mut Vec<ValidationStageReport>,
    request: Option<&FeatureUseRequestV2>,
) -> Result<(), CoveError> {
    let mut checked = 0u32;
    for entry in &validated.footer.sections {
        let kind = SectionKind::from_u16(entry.section_kind).ok_or_else(|| {
            CoveError::BadSection(format!("unknown section_kind {}", entry.section_kind))
        })?;
        let result = match kind {
            SectionKind::HarborMountHints => {
                let payload = compression::section_payload(data, entry)?;
                HarborMountHintsV1::parse(&payload).map(|_| ())
            }
            _ => continue,
        };
        checked += 1;
        if let Err(err) = result {
            if profile_error_is_fatal(&validated.header, entry, FEATURE_HARBOR_PROFILE)
                || request
                    .map(request_requires_harbor_profile)
                    .unwrap_or(false)
            {
                return Err(err);
            }
        }
    }
    push_stage(
        stages,
        ValidationStage::CoveHarbor,
        ValidationStageStatus::Checked,
        checked,
    );
    Ok(())
}

pub(super) fn validate_cove_map_semantics(
    data: &[u8],
    validated: &ValidatedCoveFile,
    stages: &mut Vec<ValidationStageReport>,
    request: Option<&FeatureUseRequestV2>,
) -> Result<(), CoveError> {
    let mut checked = 0u32;
    let mut map_sections = Vec::<EmbeddedMapSection>::new();
    for entry in &validated.footer.sections {
        let kind = SectionKind::from_u16(entry.section_kind).ok_or_else(|| {
            CoveError::BadSection(format!("unknown section_kind {}", entry.section_kind))
        })?;
        let result = match kind {
            SectionKind::MapSourceCatalog
            | SectionKind::MapFunctionRegistry
            | SectionKind::MapIdentityRuleCatalog
            | SectionKind::MapRowSemanticsCatalog
            | SectionKind::MapAssertionLog
            | SectionKind::MapIdentityEquivalenceIndex
            | SectionKind::MapEvidenceIndex
            | SectionKind::MapConversionReport
            | SectionKind::MapProjectionCatalog => {
                let payload = compression::section_payload(data, entry)?;
                parse_embedded_section(kind, &payload).map(|section| {
                    map_sections.push(section);
                })
            }
            _ => continue,
        };
        checked += 1;
        if let Err(err) = result {
            if profile_error_is_fatal(&validated.header, entry, FEATURE_SEMANTIC_MAP)
                || request.map(request_requires_map_profile).unwrap_or(false)
            {
                return Err(err);
            }
        }
    }
    if let Err(err) = validate_embedded_sections(&map_sections) {
        let map_required = validated.header.required_features & FEATURE_SEMANTIC_MAP != 0
            || validated
                .footer
                .sections
                .iter()
                .any(|entry| entry.required_features & FEATURE_SEMANTIC_MAP != 0);
        if map_required || request.map(request_requires_map_profile).unwrap_or(false) {
            return Err(err);
        }
    }
    push_stage(
        stages,
        ValidationStage::CoveMap,
        ValidationStageStatus::Checked,
        checked,
    );
    Ok(())
}

fn profile_error_is_fatal(header: &CoveHeaderV1, entry: &CoveSectionEntryV1, feature: u64) -> bool {
    header.required_features & feature != 0 || entry.required_features & feature != 0
}

fn cove_o_profile_required(
    validated: &ValidatedCoveFile,
    request: Option<&FeatureUseRequestV2>,
) -> bool {
    validated.header.required_features & FEATURE_OBJECT_PROFILE != 0
        || validated
            .footer
            .sections
            .iter()
            .any(|entry| entry.required_features & FEATURE_OBJECT_PROFILE != 0)
        || request
            .map(request_requires_object_profile)
            .unwrap_or(false)
}

fn cove_e_profile_required(
    validated: &ValidatedCoveFile,
    request: Option<&FeatureUseRequestV2>,
) -> bool {
    validated.header.required_features & FEATURE_ENGINE_PROFILE != 0
        || validated
            .footer
            .sections
            .iter()
            .any(|entry| entry.required_features & FEATURE_ENGINE_PROFILE != 0)
        || request
            .map(request_requires_engine_profile)
            .unwrap_or(false)
}

pub(super) fn request_requires_object_profile(request: &FeatureUseRequestV2) -> bool {
    request.requested_profile == Some(PrimaryProfile::ObjectTemporal as u8)
        || matches!(
            request.requested_operation,
            Some(OperationKindV2::ObjectReconstruction)
        )
}

pub(super) fn request_requires_engine_profile(request: &FeatureUseRequestV2) -> bool {
    request.requested_profile == Some(PrimaryProfile::EngineExecution as u8)
        || matches!(
            request.requested_operation,
            Some(OperationKindV2::EngineExecutionMapping)
        )
}

pub(super) fn request_requires_harbor_profile(request: &FeatureUseRequestV2) -> bool {
    request.requested_profile == Some(PrimaryProfile::HarborExecution as u8)
        || matches!(
            request.requested_operation,
            Some(OperationKindV2::HarborMount)
        )
}

pub(super) fn request_requires_map_profile(request: &FeatureUseRequestV2) -> bool {
    request.requested_profile == Some(PrimaryProfile::SemanticMapping as u8)
        || matches!(
            request.requested_operation,
            Some(
                OperationKindV2::MappingReplay
                    | OperationKindV2::MappingExplanation
                    | OperationKindV2::ProjectionReadback
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_table_scan_operation_requires_all_cove_t_scan_sections() {
        for kind in [
            SectionKind::TableCatalog,
            SectionKind::TableSegmentIndex,
            SectionKind::TableSegmentData,
            SectionKind::FileDictionaryIndex,
            SectionKind::FileDictionaryPayload,
            SectionKind::NestedSchema,
            SectionKind::ZoneStats,
            SectionKind::CodecExtensionRegistry,
        ] {
            assert!(
                operation_requires_section(OperationKindV2::OrdinaryTableScan, kind),
                "{kind:?} should be required for ordinary table scans"
            );
        }
    }

    #[test]
    fn engine_execution_operation_requires_cove_e_sections() {
        for kind in [
            SectionKind::EngineProfileRegistry,
            SectionKind::ExecutionCodeDescriptor,
            SectionKind::ExecutionScopeDescriptor,
            SectionKind::CodeSpaceDescriptor,
            SectionKind::EngineMountPolicy,
        ] {
            assert!(
                operation_requires_section(OperationKindV2::EngineExecutionMapping, kind),
                "{kind:?} should be required for engine execution mapping"
            );
        }
    }

    #[test]
    fn policy_operations_require_policy_sections() {
        assert!(operation_requires_section(
            OperationKindV2::TrustVerification,
            SectionKind::TrustManifest
        ));
        assert!(operation_requires_section(
            OperationKindV2::RedactionPolicyEvaluation,
            SectionKind::RedactionManifest
        ));
    }

    #[test]
    fn table_sort_validation_only_claims_supported_default_semantics() {
        let mut column = sort_test_column(CoveLogicalType::Int64, CovePhysicalKind::NumCode);
        assert!(can_validate_table_sort_column(&column));

        column.collation_id = 1;
        assert!(!can_validate_table_sort_column(&column));

        column.collation_id = 0;
        column.physical = CovePhysicalKind::FileCode;
        assert!(!can_validate_table_sort_column(&column));

        column.physical = CovePhysicalKind::List;
        assert!(!can_validate_table_sort_column(&column));
    }

    #[test]
    fn declared_primary_sort_rejects_unverifiable_column_semantics() {
        let mut column = sort_test_column(CoveLogicalType::Utf8, CovePhysicalKind::VarBytes);
        column.collation_id = 1;
        let table = sort_test_table(column);
        let segment_index = TableSegmentIndex {
            flags: 0,
            entries: Vec::new(),
        };
        let payloads = std::collections::BTreeMap::new();
        assert!(matches!(
            validate_declared_primary_sort_order(&table, &segment_index, &payloads, &[]),
            Err(CoveError::UnsupportedEncoding(_))
        ));
    }

    fn sort_test_table(column: ColumnEntry) -> TableEntry {
        TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "sorted".into(),
            row_count: 0,
            primary_sort_key_count: 1,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column],
        }
    }

    fn sort_test_column(logical: CoveLogicalType, physical: CovePhysicalKind) -> ColumnEntry {
        ColumnEntry {
            column_id: 1,
            name: "sort_key".into(),
            logical,
            physical,
            nullable: true,
            sort_order: 1,
            collation_id: 0,
            precision: 0,
            scale: 0,
            flags: 0,
        }
    }

    #[test]
    fn temporal_prev_ref_must_stay_on_same_object_chain() {
        let mut first = temporal_segment_for_chain(1, 0, [1; 16], RecordKind::Baseline, None);
        let second = temporal_segment_for_chain(
            2,
            1,
            [2; 16],
            RecordKind::Delta,
            Some(crate::profile::cove_o::CoveRecordRefV1 {
                segment_id: 1,
                row_index: 0,
                target_kind: 1,
            }),
        );
        assert_eq!(
            validate_temporal_chains(&[&first, &second]),
            Err(CoveError::RefInvalid)
        );

        first.rows[0].goid = [2; 16];
        assert!(validate_temporal_chains(&[&first, &second]).is_ok());
    }

    #[test]
    fn temporal_segment_ids_must_be_file_unique_across_object_types() {
        let first = temporal_segment_for_chain(1, 0, [1; 16], RecordKind::Baseline, None);
        let mut second = temporal_segment_for_chain(1, 1, [2; 16], RecordKind::Baseline, None);
        second.header.object_type_id = 2;
        assert_eq!(
            validate_temporal_segment_ids_file_unique(&[&first, &second]),
            Err(CoveError::RefInvalid)
        );
    }

    fn temporal_segment_for_chain(
        segment_id: u32,
        csn: u64,
        goid: [u8; 16],
        record_kind: RecordKind,
        prev_ref: Option<crate::profile::cove_o::CoveRecordRefV1>,
    ) -> TemporalSegmentData {
        TemporalSegmentData {
            header: crate::profile::cove_o::TemporalSegmentHeaderV1 {
                segment_id,
                object_type_id: 1,
                time_range_start_us: csn as i64,
                time_range_end_us: csn as i64,
                csn_min: csn,
                csn_max: csn,
                row_count: 1,
                morsel_count: 1,
                morsel_row_count: 1,
                column_count: 0,
                row_directory_offset: 0,
                column_directory_offset: 0,
                page_index_offset: 0,
                data_offset: 0,
                flags: 0,
                checksum: 0,
            },
            rows: vec![crate::profile::cove_o::TemporalRowEntryV1 {
                timestamp_us: csn as i64,
                csn,
                branch_key: 0,
                goid,
                record_id: [csn as u8; 16],
                record_kind,
                prev_ref,
            }],
            property_columns: Vec::new(),
        }
    }
}
