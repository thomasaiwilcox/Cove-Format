use cove_cache::{CoverageCacheUseContextV2, CoverageCacheV2, ABSENT_REF};
use cove_core::{
    codec::CodecExtensionDescriptorV2,
    compression,
    constants::{DigestAlgorithm, SectionKind, FEATURE_CODEC_EXTENSION_REGISTRY},
    digest::compute_digest,
    footer::CoveFooter,
    mount::{mount_cove_file, MountOptions, OutputRepresentation},
    profile::cove_o::{ObjectTypeCatalog, TemporalSegmentData},
    reader::ValidationOptions,
    segment::{TableSegmentIndex, TableSegmentIndexEntryV1},
    table::{TableCatalog, TableEntry},
};
use cove_coverage::{
    can_use_for_pruning, can_use_proof_for_pruning, CoveragePlanCandidateV2, CoverageProofRecordV2,
    CoverageSetV2, COVERAGE_PLAN_FLAG_MAY_UNDER_INCLUDE, COVERAGE_PLAN_FLAG_PRUNING_CANDIDATE,
};
use cove_index::execution::{CoviValidationContextV2, ValidatedCoviArtifactV2};
use cove_layout::{
    LayoutPlanV2, ObjectZeroCopyColumnAuthority, ObjectZeroCopyPageAuthority,
    ObjectZeroCopySegmentAuthority, PageClusterDirectoryV2, ScanSplitIndexV2,
    ValidatedZeroCopyBufferMapV2, ZeroCopyBufferMapV2, ZeroCopyCompatibilityContext,
    ZeroCopyDictionarySemanticsV2, ZeroCopyLifetimeScopeV2, ZeroCopyNestedLayoutKindV2,
};
use cove_runtime::{RuntimeCompatibilityHintV2, RuntimeSession};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    physical_predicate::{default_execution_code_domain, PhysicalExecutionCodeDomainDescriptor},
    physical_sidecars::{PhysicalSidecarInputs, PhysicalSidecarValidation},
    CoveOqlOutputMode, PlannedQuery,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofValidationReport {
    pub coverage: Vec<PhysicalSidecarValidation>,
    pub accepted_proof_count: usize,
    pub ignored_proof_count: usize,
    pub accepted_strengths: Vec<String>,
    pub fallbacks: Vec<PhysicalFallbackReport>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCapabilityReport {
    pub covi: Vec<PhysicalSidecarValidation>,
    pub covx: Vec<PhysicalSidecarValidation>,
    pub lookup_candidates: usize,
    pub index_only_candidates: usize,
    pub exact_candidates: usize,
    pub fallbacks: Vec<PhysicalFallbackReport>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutRangePlan {
    pub layout: Vec<PhysicalSidecarValidation>,
    pub range_count: usize,
    pub coalesced_range_count: usize,
    pub page_cluster_count: usize,
    pub fallbacks: Vec<PhysicalFallbackReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeroCopyEligibilityReport {
    pub requested: bool,
    pub candidate: bool,
    pub permitted_by_security: bool,
    pub layout_candidate_validated: bool,
    pub compatible: bool,
    pub reason: String,
    pub fallbacks: Vec<PhysicalFallbackReport>,
}

impl Default for ZeroCopyEligibilityReport {
    fn default() -> Self {
        Self {
            requested: false,
            candidate: false,
            permitted_by_security: false,
            layout_candidate_validated: false,
            compatible: false,
            reason: "zero-copy not requested".into(),
            fallbacks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalFallbackReport {
    pub source: String,
    pub reason: String,
    pub required: bool,
    pub redacted: bool,
}

impl PhysicalFallbackReport {
    pub(crate) fn new(source: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            reason: reason.into(),
            required: false,
            redacted: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PhysicalMetadataReports {
    pub proofs: ProofValidationReport,
    pub index: IndexCapabilityReport,
    pub layout: LayoutRangePlan,
    pub runtime: Vec<PhysicalSidecarValidation>,
    pub cache: Vec<PhysicalSidecarValidation>,
    pub codec: Vec<PhysicalSidecarValidation>,
    pub zero_copy: ZeroCopyEligibilityReport,
    pub execution_code_domains: Vec<PhysicalExecutionCodeDomainDescriptor>,
    pub sidecar_validations: Vec<PhysicalSidecarValidation>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SidecarPlanningFlags {
    pub enable_coverage: bool,
    pub enable_covi: bool,
    pub enable_covx: bool,
    pub enable_layout: bool,
    pub enable_cache: bool,
    pub enable_execution_code: bool,
    pub enable_zero_copy_candidates: bool,
    pub allow_file_code_literal_candidates: bool,
}

pub(crate) fn validate_sidecars(
    bytes: &[u8],
    planned: &PlannedQuery,
    inputs: &PhysicalSidecarInputs,
    flags: SidecarPlanningFlags,
    validation_options: &ValidationOptions,
) -> PhysicalMetadataReports {
    let mut reports = PhysicalMetadataReports::default();
    reports.proofs = coverage_report(inputs, flags.enable_coverage);
    reports.index = index_report(planned, inputs, flags);
    reports.layout = layout_report(inputs, flags.enable_layout);
    reports.runtime = vec![runtime_report(bytes, validation_options)];
    reports.cache = vec![cache_report(bytes, planned, inputs, flags.enable_cache)];
    reports.codec = vec![codec_report(bytes, validation_options)];
    reports.zero_copy = zero_copy_report(bytes, planned, inputs, flags, validation_options);
    reports.execution_code_domains = execution_code_domains(
        bytes,
        planned,
        flags.enable_execution_code,
        validation_options,
    );
    reports.sidecar_validations = sidecar_validations(&reports);
    reports
}

fn coverage_report(inputs: &PhysicalSidecarInputs, enabled: bool) -> ProofValidationReport {
    let mut report = ProofValidationReport::default();
    if !enabled {
        report
            .coverage
            .push(PhysicalSidecarValidation::disabled("cove_coverage"));
        return report;
    }

    match &inputs.coverage_plan_candidate_bytes {
        Some(bytes) => match CoveragePlanCandidateV2::parse_many(bytes) {
            Ok(candidates) => {
                let accepted = candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.flags & COVERAGE_PLAN_FLAG_PRUNING_CANDIDATE != 0
                            && candidate.flags & COVERAGE_PLAN_FLAG_MAY_UNDER_INCLUDE == 0
                    })
                    .count();
                if accepted > 0 {
                    report.coverage.push(PhysicalSidecarValidation::trusted(
                        "coverage_plan_candidates",
                        accepted,
                        json!({ "candidate_count": candidates.len(), "accepted_count": accepted }),
                    ));
                    report.accepted_proof_count += accepted;
                } else {
                    report.coverage.push(PhysicalSidecarValidation::ignored(
                        "coverage_plan_candidates",
                        "coverage candidates were present but none carried a safe pruning contract",
                        json!({ "candidate_count": candidates.len() }),
                    ));
                    report.ignored_proof_count += candidates.len();
                }
            }
            Err(error) => {
                report.coverage.push(PhysicalSidecarValidation::ignored(
                    "coverage_plan_candidates",
                    format!("coverage candidate parse failed: {error}"),
                    json!({}),
                ));
                report.fallbacks.push(PhysicalFallbackReport::new(
                    "cove_coverage",
                    "coverage candidates ignored",
                ));
            }
        },
        None => report.coverage.push(PhysicalSidecarValidation::missing(
            "coverage_plan_candidates",
        )),
    }

    match &inputs.coverage_proof_record_bytes {
        Some(bytes) => match CoverageProofRecordV2::parse_many(bytes) {
            Ok(records) => {
                let mut accepted = 0usize;
                let mut ignored = 0usize;
                let mut strengths = Vec::new();
                for record in &records {
                    let bound_ok = inputs
                        .coverage_set_bytes
                        .as_deref()
                        .map(|set_bytes| record.validate_against_coverage_set_bytes(set_bytes))
                        .transpose()
                        .is_ok();
                    if can_use_proof_for_pruning(record) && bound_ok {
                        accepted += 1;
                        strengths.push(format!("{:?}", record.proof_strength));
                    } else {
                        ignored += 1;
                    }
                }
                report.accepted_proof_count += accepted;
                report.ignored_proof_count += ignored;
                report.accepted_strengths.extend(strengths);
                if accepted > 0 {
                    report.coverage.push(PhysicalSidecarValidation::trusted(
                        "coverage_proof_records",
                        accepted,
                        json!({ "record_count": records.len(), "accepted_count": accepted }),
                    ));
                } else {
                    report.coverage.push(PhysicalSidecarValidation::ignored(
                        "coverage_proof_records",
                        "coverage proofs were not exact or no-false-negative conservative",
                        json!({ "record_count": records.len() }),
                    ));
                }
            }
            Err(error) => {
                report.coverage.push(PhysicalSidecarValidation::ignored(
                    "coverage_proof_records",
                    format!("coverage proof parse failed: {error}"),
                    json!({}),
                ));
                report.fallbacks.push(PhysicalFallbackReport::new(
                    "cove_coverage",
                    "coverage proofs ignored",
                ));
            }
        },
        None => report
            .coverage
            .push(PhysicalSidecarValidation::missing("coverage_proof_records")),
    }

    match &inputs.coverage_set_bytes {
        Some(bytes) => match CoverageSetV2::parse(bytes) {
            Ok(set) if can_use_for_pruning(&set.header) => {
                report.coverage.push(PhysicalSidecarValidation::trusted(
                    "coverage_set",
                    set.entries.len(),
                    json!({ "entry_count": set.entries.len() }),
                ));
            }
            Ok(set) => {
                report.coverage.push(PhysicalSidecarValidation::ignored(
                    "coverage_set",
                    "coverage set may under-include or does not allow pruning",
                    json!({ "entry_count": set.entries.len() }),
                ));
            }
            Err(error) => {
                report.coverage.push(PhysicalSidecarValidation::ignored(
                    "coverage_set",
                    format!("coverage set parse failed: {error}"),
                    json!({}),
                ));
            }
        },
        None => report
            .coverage
            .push(PhysicalSidecarValidation::missing("coverage_set")),
    }

    report
}

fn index_report(
    planned: &PlannedQuery,
    inputs: &PhysicalSidecarInputs,
    flags: SidecarPlanningFlags,
) -> IndexCapabilityReport {
    let mut report = IndexCapabilityReport::default();
    if !flags.enable_covi {
        report
            .covi
            .push(PhysicalSidecarValidation::disabled("cove_i"));
    } else {
        validate_covi_like(
            "cove_i",
            inputs.covi_artifact_bytes.as_deref(),
            planned,
            flags.allow_file_code_literal_candidates,
            &mut report,
            true,
        );
    }
    if !flags.enable_covx {
        report
            .covx
            .push(PhysicalSidecarValidation::disabled("covx"));
    } else {
        validate_covi_like(
            "covx",
            inputs.covx_artifact_bytes.as_deref(),
            planned,
            flags.allow_file_code_literal_candidates,
            &mut report,
            false,
        );
    }
    report
}

fn validate_covi_like(
    name: &str,
    bytes: Option<&[u8]>,
    planned: &PlannedQuery,
    allow_file_code_keys: bool,
    report: &mut IndexCapabilityReport,
    covi: bool,
) {
    let Some(bytes) = bytes else {
        let missing = PhysicalSidecarValidation::missing(name);
        if covi {
            report.covi.push(missing);
        } else {
            report.covx.push(missing);
        }
        return;
    };
    let context = CoviValidationContextV2::for_file(
        planned.resolved.operation_context.file.file_id,
        planned.resolved.operation_context.file.file_len,
        planned.resolved.operation_context.file.footer_crc32c,
    )
    .with_file_code_keys(allow_file_code_keys);
    match ValidatedCoviArtifactV2::parse_and_validate(bytes, context) {
        Ok(validated) => {
            let artifact = validated.artifact();
            let lookup_candidates = artifact.index_roots.len();
            let index_only_candidates = artifact.index_only_capabilities.len();
            report.lookup_candidates += lookup_candidates;
            report.index_only_candidates += index_only_candidates;
            report.exact_candidates += artifact
                .capabilities
                .iter()
                .filter(|capability| {
                    format!("{:?}", capability.exactness).contains("Exact")
                        && capability.proof_strength.allows_pruning()
                })
                .count();
            let validation = PhysicalSidecarValidation::trusted(
                name,
                lookup_candidates + index_only_candidates,
                json!({
                    "index_root_count": lookup_candidates,
                    "index_only_capability_count": index_only_candidates,
                    "capability_count": artifact.capabilities.len(),
                }),
            );
            if covi {
                report.covi.push(validation);
            } else {
                report.covx.push(validation);
            }
        }
        Err(error) => {
            let reason = format!("{name} artifact validation failed: {error}");
            let validation = PhysicalSidecarValidation::ignored(name, reason.clone(), json!({}));
            if covi {
                report.covi.push(validation);
            } else {
                report.covx.push(validation);
            }
            report
                .fallbacks
                .push(PhysicalFallbackReport::new(name, reason));
        }
    }
}

fn layout_report(inputs: &PhysicalSidecarInputs, enabled: bool) -> LayoutRangePlan {
    let mut report = LayoutRangePlan::default();
    if !enabled {
        report
            .layout
            .push(PhysicalSidecarValidation::disabled("cove_l"));
        return report;
    }
    match &inputs.layout_plan_bytes {
        Some(bytes) => match LayoutPlanV2::parse(bytes) {
            Ok(plan) => {
                report.range_count += plan.nodes.len();
                report.layout.push(PhysicalSidecarValidation::trusted(
                    "layout_plan",
                    plan.nodes.len(),
                    json!({ "node_count": plan.nodes.len() }),
                ));
            }
            Err(error) => {
                let reason = format!("layout plan parse failed: {error}");
                report.layout.push(PhysicalSidecarValidation::ignored(
                    "layout_plan",
                    reason.clone(),
                    json!({}),
                ));
                report
                    .fallbacks
                    .push(PhysicalFallbackReport::new("cove_l", reason));
            }
        },
        None => report
            .layout
            .push(PhysicalSidecarValidation::missing("layout_plan")),
    }
    match &inputs.scan_split_index_bytes {
        Some(bytes) => match ScanSplitIndexV2::parse(bytes) {
            Ok(index) => {
                report.coalesced_range_count += index.entries.len();
                report.layout.push(PhysicalSidecarValidation::trusted(
                    "scan_split_index",
                    index.entries.len(),
                    json!({ "split_count": index.entries.len() }),
                ));
            }
            Err(error) => {
                report.layout.push(PhysicalSidecarValidation::ignored(
                    "scan_split_index",
                    format!("scan split index parse failed: {error}"),
                    json!({}),
                ));
            }
        },
        None => report
            .layout
            .push(PhysicalSidecarValidation::missing("scan_split_index")),
    }
    match &inputs.page_cluster_directory_bytes {
        Some(bytes) => match PageClusterDirectoryV2::parse(bytes) {
            Ok(directory) => {
                report.page_cluster_count += directory.entries.len();
                report.layout.push(PhysicalSidecarValidation::trusted(
                    "page_cluster_directory",
                    directory.entries.len(),
                    json!({ "cluster_count": directory.entries.len() }),
                ));
            }
            Err(error) => {
                report.layout.push(PhysicalSidecarValidation::ignored(
                    "page_cluster_directory",
                    format!("page cluster directory parse failed: {error}"),
                    json!({}),
                ));
            }
        },
        None => report
            .layout
            .push(PhysicalSidecarValidation::missing("page_cluster_directory")),
    }
    report
}

fn zero_copy_report(
    file_bytes: &[u8],
    planned: &PlannedQuery,
    inputs: &PhysicalSidecarInputs,
    flags: SidecarPlanningFlags,
    validation_options: &ValidationOptions,
) -> ZeroCopyEligibilityReport {
    let requested = matches!(
        planned.resolved.output_mode,
        CoveOqlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true
        }
    );
    if !flags.enable_zero_copy_candidates {
        return ZeroCopyEligibilityReport {
            requested,
            reason: "zero-copy candidate planning disabled by physical plan options".into(),
            fallbacks: vec![PhysicalFallbackReport::new(
                "cove_l",
                "zero-copy candidate planning disabled",
            )],
            ..ZeroCopyEligibilityReport::default()
        };
    }
    let permitted = planned
        .resolved
        .operation_context
        .security
        .zero_copy_permission;
    let Some(bytes) = inputs.zero_copy_buffer_map_bytes.as_deref() else {
        return ZeroCopyEligibilityReport {
            requested,
            permitted_by_security: permitted,
            reason: "zero-copy buffer map not supplied".into(),
            fallbacks: vec![PhysicalFallbackReport::new(
                "cove_l",
                "zero-copy buffer map missing; owned Arrow buffers remain required",
            )],
            ..ZeroCopyEligibilityReport::default()
        };
    };
    match ZeroCopyBufferMapV2::parse(bytes) {
        Ok(map) => match validate_zero_copy_map_for_file(file_bytes, planned, map, validation_options) {
            Ok((target_count, entry_count)) => ZeroCopyEligibilityReport {
                requested,
                candidate: requested && permitted,
                permitted_by_security: permitted,
                layout_candidate_validated: true,
                compatible: requested && permitted,
                reason: format!(
                    "zero-copy map authority validated with {target_count} targets and {entry_count} entries"
                ),
                fallbacks: if requested && permitted {
                    Vec::new()
                } else {
                    vec![PhysicalFallbackReport::new(
                        "cove_l",
                        "zero-copy map is validated but output was not both requested and permitted",
                    )]
                },
            },
            Err(reason) => ZeroCopyEligibilityReport {
                requested,
                permitted_by_security: permitted,
                layout_candidate_validated: false,
                compatible: false,
                reason: format!("zero-copy map authority validation failed: {reason}"),
                fallbacks: vec![PhysicalFallbackReport::new(
                    "cove_l",
                    "zero-copy map did not validate against table layout",
                )],
                ..ZeroCopyEligibilityReport::default()
            },
        },
        Err(error) => ZeroCopyEligibilityReport {
            requested,
            permitted_by_security: permitted,
            reason: format!("zero-copy buffer map parse failed: {error}"),
            fallbacks: vec![PhysicalFallbackReport::new(
                "cove_l",
                "invalid zero-copy buffer map ignored",
            )],
            ..ZeroCopyEligibilityReport::default()
        },
    }
}

fn validate_zero_copy_map_for_file(
    bytes: &[u8],
    planned: &PlannedQuery,
    map: ZeroCopyBufferMapV2,
    validation_options: &ValidationOptions,
) -> Result<(usize, usize), String> {
    let target_count = map.targets.len();
    let entry_count = map.entries.len();
    let mounted = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: validation_options.verify_digests,
            allow_unknown_optional_extensions: validation_options.allow_unknown_optional_extensions,
            covx: None,
            covm: None,
        },
        None,
    )
    .map_err(|error| error.to_string())?;
    if let Some(catalog) = mounted.table_catalog.as_ref() {
        let table = zero_copy_table_for_map(catalog, &map)
            .ok_or_else(|| "zero-copy map targets no table in the catalog".to_string())?;
        let segments = table_segments_for_file(bytes)?;
        ValidatedZeroCopyBufferMapV2::validate(map, table, &segments)
            .map_err(|error| error.to_string())?;
    } else {
        let (catalog, segments) = object_zero_copy_authority_for_file(bytes, &mounted.footer)
            .map_err(|error| error.to_string())?;
        let compatibility = ZeroCopyCompatibilityContext {
            active_visibility_overlay: matches!(
                planned
                    .resolved
                    .operation_context
                    .security
                    .visibility_policy,
                crate::VisibilityPolicy::ExternalOverlay(_)
            ),
            accepts_cove_null_bitmap_polarity: true,
            expected_dictionary_semantics: ZeroCopyDictionarySemanticsV2::NoDictionary,
            expected_nested_layout_kind: ZeroCopyNestedLayoutKindV2::NotNested,
            required_lifetime_scope: ZeroCopyLifetimeScopeV2::ReaderSession,
        };
        ValidatedZeroCopyBufferMapV2::validate_object_temporal(
            map,
            &catalog,
            &segments,
            &compatibility,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok((target_count, entry_count))
}

fn zero_copy_table_for_map<'a>(
    catalog: &'a TableCatalog,
    map: &ZeroCopyBufferMapV2,
) -> Option<&'a TableEntry> {
    let table_id = map.entries.first()?.table_id;
    if !map.entries.iter().all(|entry| entry.table_id == table_id) {
        return None;
    }
    catalog
        .tables
        .iter()
        .find(|table| table.table_id == table_id)
}

fn table_segments_for_file(bytes: &[u8]) -> Result<Vec<TableSegmentIndexEntryV1>, String> {
    let mounted = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            ..MountOptions::default()
        },
        None,
    )
    .map_err(|error| error.to_string())?;
    let mut segments = Vec::new();
    for entry in mounted
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::TableSegmentIndex as u16)
    {
        let payload =
            compression::section_payload(bytes, entry).map_err(|error| error.to_string())?;
        let index = TableSegmentIndex::parse(&payload).map_err(|error| error.to_string())?;
        segments.extend(index.entries);
    }
    if segments.is_empty() {
        return Err("zero-copy validation requires a table segment index".into());
    }
    Ok(segments)
}

fn object_zero_copy_authority_for_file(
    bytes: &[u8],
    footer: &CoveFooter,
) -> Result<(ObjectTypeCatalog, Vec<ObjectZeroCopySegmentAuthority>), cove_core::CoveError> {
    let mut catalog = None;
    let mut segments = Vec::new();
    for entry in &footer.sections {
        let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
            continue;
        };
        let payload = compression::section_payload(bytes, entry)?;
        match kind {
            SectionKind::ObjectTypeCatalog => {
                catalog = Some(ObjectTypeCatalog::parse(payload.as_ref())?);
            }
            SectionKind::TemporalSegmentData => {
                let segment = TemporalSegmentData::parse(payload.as_ref())?;
                segments.push(object_zero_copy_segment_authority(&segment));
            }
            _ => {}
        }
    }
    let catalog = catalog.ok_or_else(|| {
        cove_core::CoveError::BadSchema(
            "zero-copy COVE-O validation requires OBJECT_TYPE_CATALOG".into(),
        )
    })?;
    if segments.is_empty() {
        return Err(cove_core::CoveError::BadSchema(
            "zero-copy COVE-O validation requires TEMPORAL_SEGMENT_DATA".into(),
        ));
    }
    Ok((catalog, segments))
}

fn object_zero_copy_segment_authority(
    segment: &TemporalSegmentData,
) -> ObjectZeroCopySegmentAuthority {
    ObjectZeroCopySegmentAuthority {
        object_type_id: segment.header.object_type_id,
        segment_id: segment.header.segment_id,
        morsel_count: segment.header.morsel_count,
        columns: segment
            .property_columns
            .iter()
            .map(|column| ObjectZeroCopyColumnAuthority {
                property_id: column.directory.column_id,
                logical_type: column.directory.logical_type,
                physical_kind: column.directory.physical_kind,
                pages: column
                    .pages
                    .iter()
                    .enumerate()
                    .map(|(index, page)| ObjectZeroCopyPageAuthority {
                        page_ref: u32::try_from(index + 1).unwrap_or(u32::MAX),
                        morsel_id: page.index_entry.morsel_id,
                        row_count: page.index_entry.row_count,
                        flags: page.index_entry.flags,
                        has_payload: page.payload.is_some(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn execution_code_domains(
    bytes: &[u8],
    planned: &PlannedQuery,
    enabled: bool,
    validation_options: &ValidationOptions,
) -> Vec<PhysicalExecutionCodeDomainDescriptor> {
    if !enabled {
        return vec![default_execution_code_domain(
            planned,
            false,
            Some("execution-code candidate planning disabled".into()),
        )];
    }
    match mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: validation_options.verify_digests,
            allow_unknown_optional_extensions: validation_options.allow_unknown_optional_extensions,
            covx: None,
            covm: None,
        },
        None,
    ) {
        Ok(mounted)
            if !mounted.execution_descriptors.is_empty()
                || !mounted.code_spaces.is_empty()
                || !mounted.engine_mount_policies.is_empty() =>
        {
            vec![PhysicalExecutionCodeDomainDescriptor {
                engine_profile_id: mounted
                    .engine_profile_registries
                    .first()
                    .map(|_| "embedded_engine_profile".into()),
                code_space_id: mounted.code_spaces.first().map(|_| "embedded_code_space".into()),
                comparison_scope: mounted
                    .execution_descriptors
                    .first()
                    .map(|descriptor| format!("{:?}", descriptor.comparison_scope)),
                lifetime: mounted
                    .execution_descriptors
                    .first()
                    .map(|descriptor| format!("{:?}", descriptor.lifetime)),
                epoch: None,
                null_code_policy: mounted
                    .execution_descriptors
                    .first()
                    .map(|descriptor| format!("{:?}", descriptor.null_code_policy)),
                semantic_domain_id: None,
                security_scope: crate::physical_predicate::security_scope_descriptor(planned),
                validated: true,
                fallback_reason: Some(
                    "execution-code metadata was validated; runtime execution still requires a per-operation remap proof gate"
                        .into(),
                ),
            }]
        }
        Ok(_) => vec![default_execution_code_domain(
            planned,
            false,
            Some("no embedded COVE-E execution-code metadata found".into()),
        )],
        Err(error) => vec![default_execution_code_domain(
            planned,
            false,
            Some(format!("COVE-E metadata validation failed: {error}")),
        )],
    }
}

fn sidecar_validations(reports: &PhysicalMetadataReports) -> Vec<PhysicalSidecarValidation> {
    let mut out = Vec::new();
    out.extend(reports.proofs.coverage.clone());
    out.extend(reports.index.covi.clone());
    out.extend(reports.index.covx.clone());
    out.extend(reports.layout.layout.clone());
    out.extend(reports.runtime.clone());
    out.extend(reports.cache.clone());
    out.extend(reports.codec.clone());
    out
}

fn runtime_report(
    bytes: &[u8],
    validation_options: &ValidationOptions,
) -> PhysicalSidecarValidation {
    let mounted = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: validation_options.verify_digests,
            allow_unknown_optional_extensions: validation_options.allow_unknown_optional_extensions,
            covx: None,
            covm: None,
        },
        None,
    );
    let Ok(mounted) = mounted else {
        return PhysicalSidecarValidation::ignored(
            "cove_r",
            "runtime compatibility hints could not be derived because the selected file could not be mounted",
            json!({}),
        );
    };
    let runtime_entries = mounted
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::RuntimeCompatibilityHints as u16)
        .collect::<Vec<_>>();
    if runtime_entries.is_empty() {
        return PhysicalSidecarValidation::missing("cove_r");
    }

    let session = RuntimeSession::default_builtins();
    let mut hint_count = 0usize;
    let mut required_hint_count = 0usize;
    let mut unsupported_required_hint_count = 0usize;
    for entry in &runtime_entries {
        let payload = match compression::section_payload(bytes, entry) {
            Ok(payload) => payload,
            Err(error) => {
                return PhysicalSidecarValidation::ignored(
                    "cove_r",
                    format!("runtime compatibility hint payload could not be read: {error}"),
                    json!({ "runtime_hint_section_count": runtime_entries.len() }),
                );
            }
        };
        let hints = match RuntimeCompatibilityHintV2::parse_many(payload.as_ref()) {
            Ok(hints) => hints,
            Err(error) => {
                return PhysicalSidecarValidation::ignored(
                    "cove_r",
                    format!("runtime compatibility hint parse failed: {error}"),
                    json!({ "runtime_hint_section_count": runtime_entries.len() }),
                );
            }
        };
        required_hint_count += hints.iter().filter(|hint| hint.required).count();
        unsupported_required_hint_count += session.unsupported_required_hints(&hints).len();
        hint_count += hints.len();
    }

    let details = json!({
        "runtime_hint_section_count": runtime_entries.len(),
        "hint_count": hint_count,
        "required_hint_count": required_hint_count,
        "unsupported_required_hint_count": unsupported_required_hint_count,
        "authority": "advisory_runtime_capability_only",
    });
    if unsupported_required_hint_count > 0 {
        PhysicalSidecarValidation::ignored(
            "cove_r",
            "runtime compatibility hints include required capabilities unsupported by the default Cove-OQL runtime; hints cannot authorize optimized execution",
            details,
        )
    } else {
        PhysicalSidecarValidation::trusted("cove_r", hint_count, details)
    }
}

fn codec_report(bytes: &[u8], validation_options: &ValidationOptions) -> PhysicalSidecarValidation {
    let mounted = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: validation_options.verify_digests,
            allow_unknown_optional_extensions: validation_options.allow_unknown_optional_extensions,
            covx: None,
            covm: None,
        },
        None,
    );
    let Ok(mounted) = mounted else {
        return PhysicalSidecarValidation::ignored(
            "cove_cx",
            "codec compatibility could not be derived because the selected file could not be mounted",
            json!({}),
        );
    };
    let registry_entries = mounted
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::CodecExtensionRegistry as u16)
        .collect::<Vec<_>>();
    let feature_required = mounted.header.required_features & FEATURE_CODEC_EXTENSION_REGISTRY != 0;
    if registry_entries.is_empty() {
        return PhysicalSidecarValidation::trusted(
            "cove_cx",
            0,
            json!({
                "registered_codec_registry_present": false,
                "required_feature": feature_required,
                "compatibility": "core_codec_only",
            }),
        );
    }

    let mut descriptor_count = 0usize;
    for entry in &registry_entries {
        let payload = match compression::section_payload(bytes, entry) {
            Ok(payload) => payload,
            Err(error) => {
                return PhysicalSidecarValidation::ignored(
                    "cove_cx",
                    format!("codec registry section payload could not be read: {error}"),
                    json!({ "registry_section_count": registry_entries.len() }),
                );
            }
        };
        match CodecExtensionDescriptorV2::parse_many(payload.as_ref()) {
            Ok(descriptors) => descriptor_count += descriptors.len(),
            Err(error) => {
                return PhysicalSidecarValidation::ignored(
                    "cove_cx",
                    format!("codec registry descriptor parse failed: {error}"),
                    json!({ "registry_section_count": registry_entries.len() }),
                );
            }
        }
    }

    PhysicalSidecarValidation::trusted(
        "cove_cx",
        descriptor_count,
        json!({
            "registered_codec_registry_present": true,
            "required_feature": feature_required,
            "registry_section_count": registry_entries.len(),
            "descriptor_count": descriptor_count,
            "compatibility": "registered_codec_descriptors_parsed",
        }),
    )
}

pub(crate) fn cache_report(
    file_bytes: &[u8],
    planned: &PlannedQuery,
    inputs: &PhysicalSidecarInputs,
    enabled: bool,
) -> PhysicalSidecarValidation {
    if !enabled {
        return PhysicalSidecarValidation::disabled("cove_cache");
    }
    let Some(cache_bytes) = inputs.coverage_cache_bytes.as_deref() else {
        return PhysicalSidecarValidation::missing("cove_cache");
    };
    match CoverageCacheV2::parse(cache_bytes) {
        Ok(cache) => {
            let Some((dataset_id, snapshot_id)) = coverage_cache_ids(file_bytes, planned) else {
                return PhysicalSidecarValidation::ignored(
                    "cove_cache",
                    "coverage cache parsed, but selected dataset/snapshot cache binding could not be derived",
                    json!({ "entry_count": cache.entries.len() }),
                );
            };
            if cache
                .validate_header_for_use_context(dataset_id, snapshot_id, &[0])
                .is_err()
            {
                return PhysicalSidecarValidation::ignored(
                    "cove_cache",
                    "coverage cache dataset/snapshot header is stale for the selected file",
                    json!({ "entry_count": cache.entries.len() }),
                );
            }
            let usable_entries = cache
                .entries
                .iter()
                .filter(|entry| coverage_cache_entry_usable(entry, dataset_id, snapshot_id))
                .count();
            if usable_entries == 0 {
                PhysicalSidecarValidation::ignored(
                    "cove_cache",
                    "coverage cache contained no exact, no-under-include entries usable for this selected snapshot",
                    json!({ "entry_count": cache.entries.len() }),
                )
            } else {
                PhysicalSidecarValidation::trusted(
                    "cove_cache",
                    usable_entries,
                    json!({
                        "entry_count": cache.entries.len(),
                        "usable_entries": usable_entries,
                    }),
                )
            }
        }
        Err(error) => PhysicalSidecarValidation::ignored(
            "cove_cache",
            format!("coverage cache parse failed: {error}"),
            json!({}),
        ),
    }
}

fn coverage_cache_ids(bytes: &[u8], planned: &PlannedQuery) -> Option<([u8; 16], [u8; 16])> {
    let file = &planned.resolved.operation_context.file;
    let file_digest = compute_digest(DigestAlgorithm::Sha256, bytes).ok()?;
    let mut seed = Vec::with_capacity(28 + file_digest.len());
    seed.extend_from_slice(&file.file_id);
    seed.extend_from_slice(&file.file_len.to_le_bytes());
    seed.extend_from_slice(&file.footer_crc32c.to_le_bytes());
    seed.extend_from_slice(&file_digest);
    let digest = compute_digest(DigestAlgorithm::Sha256, &seed).ok()?;
    let mut snapshot_id = [0u8; 16];
    snapshot_id.copy_from_slice(digest.get(..16)?);
    Some((file.file_id, snapshot_id))
}

fn coverage_cache_entry_usable(
    entry: &cove_cache::CoverageCacheEntryV2,
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
) -> bool {
    if entry.interval_normal_form_ref != ABSENT_REF {
        return false;
    }
    let context = CoverageCacheUseContextV2 {
        dataset_id,
        snapshot_id,
        predicate_normal_form_ref: entry.predicate_normal_form_ref,
        interval_normal_form_ref: entry.interval_normal_form_ref,
        coverage_set_ref: entry.coverage_set_ref,
        coverage_granularity: entry.coverage_granularity,
        proof_strength: entry.proof_strength,
        exactness: entry.exactness,
        selected_snapshot_ref: if entry.valid_until_snapshot_ref == ABSENT_REF {
            None
        } else {
            Some(entry.valid_until_snapshot_ref)
        },
        compatible_producer_engine_refs: &[0],
    };
    entry.validate_for_use_context(&context).is_ok()
}
