//! Shared semantic validation for embedded optional-profile payloads.

use cove_core::{
    compression,
    constants::{PrimaryProfile, SectionKind},
    feature_binding::OperationKindV2,
    feature_scope::FeatureUseRequestV2,
    footer::CoveSectionEntryV1,
    reader::{OptionalProfilePayloadValidator, OptionalPushdownPolicy, ValidationReport},
    segment::TableSegmentIndex,
    table::{TableCatalog, TableEntry},
    CoveError,
};
use cove_coverage::{
    CoveragePlanCandidateV2, CoverageProofRecordV2, CoverageProviderDescriptorV2, CoverageSetV2,
    PredicateNormalFormWithPayloadV2,
};
use cove_index::IndexOnlyCapabilityV2;
use cove_layout::{
    validate_fast_metadata_authority, validate_page_cluster_authority, FastMetadataIndexV2,
    LayoutPlanV2, PageClusterDirectoryV2, ScanSplitIndexV2, ValidatedLayoutPlanV2,
    ValidatedScanSplitIndexV2, ValidatedZeroCopyBufferMapV2, ZeroCopyBufferMapV2,
};
use cove_runtime::{validate_hints, RuntimeCompatibilityHintV2, RuntimeSession};

#[derive(Debug, Clone, Default)]
pub struct EmbeddedOptionalProfileValidator {
    runtime_session: RuntimeSession,
}

impl EmbeddedOptionalProfileValidator {
    pub fn new(runtime_session: RuntimeSession) -> Self {
        Self { runtime_session }
    }

    pub fn empty() -> Self {
        Self::new(RuntimeSession::empty())
    }

    pub fn default_builtins() -> Self {
        Self::new(RuntimeSession::default_builtins())
    }

    pub fn validate_embedded_optional_profile_sections(
        &self,
        data: &[u8],
        report: &ValidationReport,
        optional_pushdown_policy: OptionalPushdownPolicy,
        feature_use: Option<&FeatureUseRequestV2>,
        only_required_for_feature_use: bool,
    ) -> Result<(), CoveError> {
        validate_embedded_optional_profile_sections_with_runtime_session(
            data,
            report,
            optional_pushdown_policy,
            feature_use,
            only_required_for_feature_use,
            &self.runtime_session,
        )
    }
}

impl OptionalProfilePayloadValidator for EmbeddedOptionalProfileValidator {
    fn validate_optional_profile_sections(
        &self,
        data: &[u8],
        report: &ValidationReport,
        optional_pushdown_policy: OptionalPushdownPolicy,
        feature_use: Option<&FeatureUseRequestV2>,
        only_required_for_feature_use: bool,
    ) -> Result<(), CoveError> {
        self.validate_embedded_optional_profile_sections(
            data,
            report,
            optional_pushdown_policy,
            feature_use,
            only_required_for_feature_use,
        )
    }
}

pub fn validate_embedded_optional_profile_sections(
    data: &[u8],
    report: &ValidationReport,
    optional_pushdown_policy: OptionalPushdownPolicy,
    feature_use: Option<&FeatureUseRequestV2>,
    only_required_for_feature_use: bool,
) -> Result<(), CoveError> {
    validate_embedded_optional_profile_sections_with_runtime_session(
        data,
        report,
        optional_pushdown_policy,
        feature_use,
        only_required_for_feature_use,
        &RuntimeSession::empty(),
    )
}

pub fn validate_embedded_optional_profile_sections_with_runtime_session(
    data: &[u8],
    report: &ValidationReport,
    optional_pushdown_policy: OptionalPushdownPolicy,
    feature_use: Option<&FeatureUseRequestV2>,
    only_required_for_feature_use: bool,
    runtime_session: &RuntimeSession,
) -> Result<(), CoveError> {
    for entry in &report.validated.footer.sections {
        if only_required_for_feature_use && !section_is_required_for_feature_use(entry, feature_use)
        {
            continue;
        }
        let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
            continue;
        };
        let payload = match kind {
            SectionKind::LayoutPlan
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
            | SectionKind::RuntimeCompatibilityHints => {
                match compression::section_payload(data, entry) {
                    Ok(payload) => payload,
                    Err(_) if can_fail_open(entry, optional_pushdown_policy, feature_use) => {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            _ => continue,
        };
        let result = match kind {
            SectionKind::LayoutPlan => LayoutPlanV2::parse(&payload).map(|_| ()),
            SectionKind::ScanSplitIndex => ScanSplitIndexV2::parse(&payload).map(|_| ()),
            SectionKind::PageClusterDirectory => {
                PageClusterDirectoryV2::parse(&payload).map(|_| ())
            }
            SectionKind::ZeroCopyBufferMap => ZeroCopyBufferMapV2::parse(&payload).map(|_| ()),
            SectionKind::FastMetadataIndex => FastMetadataIndexV2::parse(&payload).map(|_| ()),
            SectionKind::CoverageProviderRegistry => {
                CoverageProviderDescriptorV2::parse_many(&payload).map(|_| ())
            }
            SectionKind::CoverageSet => CoverageSetV2::parse(&payload).map(|_| ()),
            SectionKind::CoveragePlanCandidate => {
                CoveragePlanCandidateV2::parse_many(&payload).map(|_| ())
            }
            SectionKind::PredicateNormalForm => {
                PredicateNormalFormWithPayloadV2::parse_many(&payload).map(|_| ())
            }
            SectionKind::CoverageProofRecord => {
                CoverageProofRecordV2::parse_many(&payload).map(|_| ())
            }
            SectionKind::IndexOnlyCapability => {
                IndexOnlyCapabilityV2::parse_many(&payload).map(|_| ())
            }
            SectionKind::RuntimeCompatibilityHints => {
                let hints = RuntimeCompatibilityHintV2::parse_many(&payload)?;
                validate_hints(&hints)?;
                if section_is_required_for_feature_use(entry, feature_use)
                    && !required_runtime_hints_supported(&hints, runtime_session)
                {
                    return Err(CoveError::RuntimeHintUnsupported);
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            if can_fail_open(entry, optional_pushdown_policy, feature_use) {
                continue;
            }
            return Err(error);
        }
    }
    validate_layout_sections_with_authority(
        data,
        report,
        optional_pushdown_policy,
        feature_use,
        only_required_for_feature_use,
    )?;
    Ok(())
}

fn validate_layout_sections_with_authority(
    data: &[u8],
    report: &ValidationReport,
    optional_pushdown_policy: OptionalPushdownPolicy,
    feature_use: Option<&FeatureUseRequestV2>,
    only_required_for_feature_use: bool,
) -> Result<(), CoveError> {
    let layout_entries = report
        .validated
        .footer
        .sections
        .iter()
        .filter(|entry| {
            SectionKind::from_u16(entry.section_kind)
                .map(is_layout_section)
                .unwrap_or(false)
        })
        .filter(|entry| {
            !only_required_for_feature_use
                || section_is_required_for_feature_use(entry, feature_use)
        })
        .collect::<Vec<_>>();
    if layout_entries.is_empty() {
        return Ok(());
    }

    let Some(authority) = load_layout_authority(data, report)? else {
        return first_non_fail_open_layout_error(
            &layout_entries,
            optional_pushdown_policy,
            feature_use,
            CoveError::BadLayoutPlan,
        );
    };

    let mut page_clusters = Vec::new();
    for entry in layout_entries
        .iter()
        .copied()
        .filter(|entry| entry.section_kind == SectionKind::PageClusterDirectory as u16)
    {
        let result = layout_payload(data, entry)
            .and_then(|payload| PageClusterDirectoryV2::parse(&payload))
            .and_then(|directory| {
                validate_against_any_table(&authority.catalog, |table| {
                    validate_page_cluster_authority(
                        &directory,
                        &report.validated.footer,
                        table,
                        &authority.segments.entries,
                    )
                })
                .map(|_| directory)
            });
        match layout_result(result, entry, optional_pushdown_policy, feature_use)? {
            Some(directory) => page_clusters.push(directory),
            None => continue,
        }
    }

    let mut scan_splits = Vec::new();
    for entry in layout_entries
        .iter()
        .copied()
        .filter(|entry| entry.section_kind == SectionKind::ScanSplitIndex as u16)
    {
        let result = layout_payload(data, entry)
            .and_then(|payload| ScanSplitIndexV2::parse(&payload))
            .and_then(|index| {
                validate_scan_split_for_any_authority(
                    index,
                    &authority.catalog,
                    &authority.segments.entries,
                    &page_clusters,
                )
            });
        match layout_result(result, entry, optional_pushdown_policy, feature_use)? {
            Some(index) => scan_splits.push(index),
            None => continue,
        }
    }

    for entry in layout_entries.iter().copied() {
        let result = match SectionKind::from_u16(entry.section_kind) {
            Some(SectionKind::LayoutPlan) => layout_payload(data, entry)
                .and_then(|payload| LayoutPlanV2::parse(&payload))
                .and_then(|plan| {
                    validate_layout_plan_for_any_authority(
                        plan,
                        &report.validated.footer,
                        &authority.catalog,
                        &authority.segments.entries,
                        &page_clusters,
                        &scan_splits,
                    )
                })
                .map(|_| ()),
            Some(SectionKind::ZeroCopyBufferMap) => layout_payload(data, entry)
                .and_then(|payload| ZeroCopyBufferMapV2::parse(&payload))
                .and_then(|map| {
                    validate_against_any_table(&authority.catalog, |table| {
                        ValidatedZeroCopyBufferMapV2::validate(
                            map.clone(),
                            table,
                            &authority.segments.entries,
                        )
                        .map(|_| ())
                    })
                }),
            Some(SectionKind::FastMetadataIndex) => layout_payload(data, entry)
                .and_then(|payload| FastMetadataIndexV2::parse(&payload))
                .and_then(|index| {
                    validate_against_any_table(&authority.catalog, |table| {
                        validate_fast_metadata_authority(
                            &index,
                            &report.validated.footer,
                            table,
                            &authority.segments.entries,
                        )
                    })
                }),
            Some(SectionKind::PageClusterDirectory | SectionKind::ScanSplitIndex) => Ok(()),
            _ => Ok(()),
        };
        layout_result(result, entry, optional_pushdown_policy, feature_use)?;
    }

    Ok(())
}

struct LayoutAuthority {
    catalog: TableCatalog,
    segments: TableSegmentIndex,
}

fn load_layout_authority(
    data: &[u8],
    report: &ValidationReport,
) -> Result<Option<LayoutAuthority>, CoveError> {
    let catalog_entries = report
        .validated
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::TableCatalog as u16)
        .collect::<Vec<_>>();
    let segment_entries = report
        .validated
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::TableSegmentIndex as u16)
        .collect::<Vec<_>>();
    if catalog_entries.is_empty() && segment_entries.is_empty() {
        return Ok(None);
    }
    if catalog_entries.len() != 1 || segment_entries.len() != 1 {
        return Err(CoveError::BadLayoutPlan);
    }
    let catalog = layout_payload(data, catalog_entries[0])
        .and_then(|payload| TableCatalog::parse(&payload))?;
    let segments = layout_payload(data, segment_entries[0])
        .and_then(|payload| TableSegmentIndex::parse(&payload))?;
    Ok(Some(LayoutAuthority { catalog, segments }))
}

fn layout_payload(data: &[u8], entry: &CoveSectionEntryV1) -> Result<Vec<u8>, CoveError> {
    compression::section_payload(data, entry).map(|payload| payload.into_owned())
}

fn first_non_fail_open_layout_error(
    entries: &[&CoveSectionEntryV1],
    optional_pushdown_policy: OptionalPushdownPolicy,
    feature_use: Option<&FeatureUseRequestV2>,
    error: CoveError,
) -> Result<(), CoveError> {
    if entries
        .iter()
        .all(|entry| can_fail_open(entry, optional_pushdown_policy, feature_use))
    {
        Ok(())
    } else {
        Err(error)
    }
}

fn required_runtime_hints_supported(
    hints: &[RuntimeCompatibilityHintV2],
    runtime_session: &RuntimeSession,
) -> bool {
    runtime_session.unsupported_required_hints(hints).is_empty()
}

fn layout_result<T>(
    result: Result<T, CoveError>,
    entry: &CoveSectionEntryV1,
    optional_pushdown_policy: OptionalPushdownPolicy,
    feature_use: Option<&FeatureUseRequestV2>,
) -> Result<Option<T>, CoveError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(_error) if can_fail_open(entry, optional_pushdown_policy, feature_use) => Ok(None),
        Err(error) => Err(error),
    }
}

fn validate_against_any_table(
    catalog: &TableCatalog,
    mut validate: impl FnMut(&TableEntry) -> Result<(), CoveError>,
) -> Result<(), CoveError> {
    for table in &catalog.tables {
        if validate(table).is_ok() {
            return Ok(());
        }
    }
    Err(CoveError::BadLayoutPlan)
}

fn validate_scan_split_for_any_authority(
    index: ScanSplitIndexV2,
    catalog: &TableCatalog,
    segments: &[cove_core::segment::TableSegmentIndexEntryV1],
    page_clusters: &[PageClusterDirectoryV2],
) -> Result<ScanSplitIndexV2, CoveError> {
    for table in &catalog.tables {
        if ValidatedScanSplitIndexV2::validate(index.clone(), table, segments, None).is_ok() {
            return Ok(index);
        }
        for clusters in page_clusters {
            if ValidatedScanSplitIndexV2::validate(index.clone(), table, segments, Some(clusters))
                .is_ok()
            {
                return Ok(index);
            }
        }
    }
    Err(CoveError::BadLayoutPlan)
}

fn validate_layout_plan_for_any_authority(
    plan: LayoutPlanV2,
    footer: &cove_core::footer::CoveFooter,
    catalog: &TableCatalog,
    segments: &[cove_core::segment::TableSegmentIndexEntryV1],
    page_clusters: &[PageClusterDirectoryV2],
    scan_splits: &[ScanSplitIndexV2],
) -> Result<LayoutPlanV2, CoveError> {
    for table in &catalog.tables {
        if ValidatedLayoutPlanV2::validate(plan.clone(), footer, table, segments, None, None)
            .is_ok()
        {
            return Ok(plan);
        }
        for clusters in page_clusters.iter().map(Some).chain(std::iter::once(None)) {
            for splits in scan_splits.iter().map(Some).chain(std::iter::once(None)) {
                if ValidatedLayoutPlanV2::validate(
                    plan.clone(),
                    footer,
                    table,
                    segments,
                    clusters,
                    splits,
                )
                .is_ok()
                {
                    return Ok(plan);
                }
            }
        }
    }
    Err(CoveError::BadLayoutPlan)
}

fn can_fail_open(
    entry: &CoveSectionEntryV1,
    optional_pushdown_policy: OptionalPushdownPolicy,
    feature_use: Option<&FeatureUseRequestV2>,
) -> bool {
    optional_pushdown_policy == OptionalPushdownPolicy::FailOpen
        && entry.required_features == 0
        && !section_is_required_for_feature_use(entry, feature_use)
}

fn section_is_required_for_feature_use(
    entry: &CoveSectionEntryV1,
    feature_use: Option<&FeatureUseRequestV2>,
) -> bool {
    let Some(request) = feature_use else {
        return false;
    };
    if request.needed_section_ids.contains(&entry.section_id)
        || request
            .needed_page_refs
            .iter()
            .any(|target| target.section_id == entry.section_id)
    {
        return true;
    }
    let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
        return false;
    };
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
    use cove_runtime::RuntimeHintKindV2;

    #[test]
    fn supported_required_runtime_hints_are_accepted() {
        let hints = vec![RuntimeCompatibilityHintV2 {
            hint_id: 1,
            hint_kind: RuntimeHintKindV2::EngineAdapter,
            required: true,
            flags: 0,
            namespace: "org.cove".into(),
            name: "datafusion".into(),
            version_major: 1,
            version_minor: 0,
            payload_ref: u32::MAX,
            checksum: 0,
        }];

        assert!(required_runtime_hints_supported(
            &hints,
            &RuntimeSession::default_builtins()
        ));
    }

    #[test]
    fn required_runtime_hints_use_the_supplied_session() {
        let hints = vec![RuntimeCompatibilityHintV2 {
            hint_id: 1,
            hint_kind: RuntimeHintKindV2::EngineAdapter,
            required: true,
            flags: 0,
            namespace: "org.cove".into(),
            name: "datafusion".into(),
            version_major: 1,
            version_minor: 0,
            payload_ref: u32::MAX,
            checksum: 0,
        }];
        let mut custom = RuntimeSession::empty();
        assert!(!required_runtime_hints_supported(&hints, &custom));
        custom
            .engine_profiles
            .register("org.cove", "datafusion", 1, 0)
            .unwrap();
        assert!(required_runtime_hints_supported(&hints, &custom));
    }

    #[test]
    fn unsupported_required_runtime_hints_are_rejected() {
        let hints = vec![RuntimeCompatibilityHintV2 {
            hint_id: 1,
            hint_kind: RuntimeHintKindV2::EngineAdapter,
            required: true,
            flags: 0,
            namespace: "example.invalid".into(),
            name: "not-registered".into(),
            version_major: 1,
            version_minor: 0,
            payload_ref: u32::MAX,
            checksum: 0,
        }];

        assert!(!required_runtime_hints_supported(
            &hints,
            &RuntimeSession::default_builtins()
        ));
    }

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
