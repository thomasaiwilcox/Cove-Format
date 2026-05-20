//! Shared semantic validation for embedded optional-profile payloads.

use cove_core::{
    compression,
    constants::{PrimaryProfile, SectionKind},
    feature_binding::OperationKindV2,
    feature_scope::FeatureUseRequestV2,
    footer::CoveSectionEntryV1,
    reader::{OptionalProfilePayloadValidator, OptionalPushdownPolicy, ValidationReport},
    CoveError,
};
use cove_coverage::{
    CoveragePlanCandidateV2, CoverageProofRecordV2, CoverageProviderDescriptorV2, CoverageSetV2,
    PredicateNormalFormWithPayloadV2,
};
use cove_index::IndexOnlyCapabilityV2;
use cove_layout::{
    FastMetadataIndexV2, LayoutPlanV2, PageClusterDirectoryV2, ScanSplitIndexV2,
    ZeroCopyBufferMapV2,
};
use cove_runtime::{validate_hints, RuntimeCompatibilityHintV2};

#[derive(Debug, Clone, Copy, Default)]
pub struct EmbeddedOptionalProfileValidator;

impl OptionalProfilePayloadValidator for EmbeddedOptionalProfileValidator {
    fn validate_optional_profile_sections(
        &self,
        data: &[u8],
        report: &ValidationReport,
        optional_pushdown_policy: OptionalPushdownPolicy,
        feature_use: Option<&FeatureUseRequestV2>,
        only_required_for_feature_use: bool,
    ) -> Result<(), CoveError> {
        validate_embedded_optional_profile_sections(
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
                    && hints.iter().any(|hint| hint.required)
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
    Ok(())
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
        OperationKindV2::CoveragePlanning => is_coverage_section(kind),
        OperationKindV2::IndexOnlyAnswer => kind == SectionKind::IndexOnlyCapability,
        OperationKindV2::ZeroCopyExport => kind == SectionKind::ZeroCopyBufferMap,
        OperationKindV2::RuntimeAdapterSelection => kind == SectionKind::RuntimeCompatibilityHints,
        _ => false,
    }
}

fn profile_requires_section(profile: u8, kind: SectionKind) -> bool {
    match PrimaryProfile::from_u8(profile) {
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
