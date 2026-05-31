use crate::{
    constants::{
        PrimaryProfile, SectionKind, FEATURE_AGGREGATE_SYNOPSES, FEATURE_BLOOM_FILTERS,
        FEATURE_COLUMN_DOMAINS, FEATURE_COMPOSITE_ZONES, FEATURE_COVERAGE_METADATA,
        FEATURE_EXACT_SETS, FEATURE_FAST_METADATA_INDEX, FEATURE_INDEX_ONLY_CAPABILITY,
        FEATURE_INVERTED_INDEXES, FEATURE_LAYOUT_PLAN, FEATURE_LOOKUP_INDEXES,
        FEATURE_PAGE_CLUSTER_DIRECTORY, FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        FEATURE_SCAN_SPLIT_INDEX, FEATURE_TOPN_SUMMARIES, FEATURE_ZERO_COPY_BUFFER_MAP,
    },
    feature_binding::OperationKindV2,
    feature_scope::FeatureUseRequestV2,
    footer::CoveSectionEntryV1,
    header::CoveHeaderV1,
    CoveError,
};

use super::reports::IgnoredOptionalSection;

pub(super) fn validate_section_profile(section_kind: u16, profile: u8) -> Result<(), CoveError> {
    let section = SectionKind::from_u16(section_kind)
        .ok_or_else(|| CoveError::BadSection(format!("unknown section_kind {section_kind}")))?;
    let allowed = allowed_profiles_for_section(section);
    if !allowed.contains(&profile) {
        return Err(CoveError::BadSection(format!(
            "section_kind {section_kind} must use one of profiles {allowed:?}, got {profile}"
        )));
    }
    Ok(())
}

pub(super) fn is_optional_advisory_entry(
    header: &CoveHeaderV1,
    entry: &CoveSectionEntryV1,
) -> bool {
    if entry.required_features != 0 {
        return false;
    }
    let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
        return false;
    };
    if !is_optional_advisory_section(kind) {
        return false;
    }
    let owning_feature = section_owning_feature_bit(kind);
    owning_feature == 0 || header.required_features & owning_feature == 0
}

pub(super) fn is_optional_advisory_section(kind: SectionKind) -> bool {
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

fn allowed_profiles_for_section(section: SectionKind) -> &'static [u8] {
    match section {
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
        | SectionKind::FastMetadataIndex
        | SectionKind::SectionFeatureBinding
        | SectionKind::VendorExtension => &[0],
        SectionKind::RuntimeCompatibilityHints => &[9],
        SectionKind::TableCatalog
        | SectionKind::NestedSchema
        | SectionKind::TableSegmentIndex
        | SectionKind::TableSegmentData
        | SectionKind::ColumnDomain
        | SectionKind::ZoneStats => &[2],
        SectionKind::ExactSetIndex
        | SectionKind::BloomIndex
        | SectionKind::InvertedMorselIndex
        | SectionKind::KernelCapabilities => &[2, 3],
        SectionKind::LookupIndex
        | SectionKind::AggregateSynopsis
        | SectionKind::CompositeZoneIndex
        | SectionKind::TopNZoneSummary => &[3],
        SectionKind::CodecExtensionRegistry => &[7],
        SectionKind::LayoutPlan
        | SectionKind::ScanSplitIndex
        | SectionKind::PageClusterDirectory => &[8],
        SectionKind::ZeroCopyBufferMap => &[0, 8],
        SectionKind::CoverageProviderRegistry
        | SectionKind::CoverageSet
        | SectionKind::CoveragePlanCandidate
        | SectionKind::PredicateNormalForm
        | SectionKind::CoverageProofRecord => &[10],
        SectionKind::IndexOnlyCapability => &[3, 11],
        SectionKind::EngineProfileRegistry
        | SectionKind::ExecutionCodeDescriptor
        | SectionKind::ExecutionScopeDescriptor
        | SectionKind::CodeSpaceDescriptor
        | SectionKind::EngineMountPolicy => &[4],
        SectionKind::ObjectTypeCatalog
        | SectionKind::TemporalSegmentIndex
        | SectionKind::TemporalSegmentData
        | SectionKind::TemporalBloomIndex
        | SectionKind::TrustManifest => &[1],
        SectionKind::HarborMountHints => &[5],
        SectionKind::MapSourceCatalog
        | SectionKind::MapFunctionRegistry
        | SectionKind::MapIdentityRuleCatalog
        | SectionKind::MapRowSemanticsCatalog
        | SectionKind::MapAssertionLog
        | SectionKind::MapIdentityEquivalenceIndex
        | SectionKind::MapEvidenceIndex
        | SectionKind::MapConversionReport
        | SectionKind::MapProjectionCatalog => &[6],
    }
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

fn section_owning_feature_bit(kind: SectionKind) -> u64 {
    match kind {
        SectionKind::ColumnDomain => FEATURE_COLUMN_DOMAINS,
        SectionKind::ExactSetIndex => FEATURE_EXACT_SETS,
        SectionKind::BloomIndex => FEATURE_BLOOM_FILTERS,
        SectionKind::InvertedMorselIndex => FEATURE_INVERTED_INDEXES,
        SectionKind::LookupIndex => FEATURE_LOOKUP_INDEXES,
        SectionKind::AggregateSynopsis => FEATURE_AGGREGATE_SYNOPSES,
        SectionKind::CompositeZoneIndex => FEATURE_COMPOSITE_ZONES,
        SectionKind::TopNZoneSummary => FEATURE_TOPN_SUMMARIES,
        SectionKind::LayoutPlan => FEATURE_LAYOUT_PLAN,
        SectionKind::ScanSplitIndex => FEATURE_SCAN_SPLIT_INDEX,
        SectionKind::PageClusterDirectory => FEATURE_PAGE_CLUSTER_DIRECTORY,
        SectionKind::ZeroCopyBufferMap => FEATURE_ZERO_COPY_BUFFER_MAP,
        SectionKind::FastMetadataIndex => FEATURE_FAST_METADATA_INDEX,
        SectionKind::CoverageProviderRegistry
        | SectionKind::CoverageSet
        | SectionKind::CoveragePlanCandidate
        | SectionKind::PredicateNormalForm
        | SectionKind::CoverageProofRecord => FEATURE_COVERAGE_METADATA,
        SectionKind::IndexOnlyCapability => FEATURE_INDEX_ONLY_CAPABILITY,
        SectionKind::RuntimeCompatibilityHints => FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        _ => 0,
    }
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
}
