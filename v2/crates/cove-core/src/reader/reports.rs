use crate::{feature_scope::FeatureUseRequestV2, CoveError};

/// Options controlling the depth of validation.
#[derive(Debug, Clone)]
pub struct ValidationOptions {
    /// When true, validates dictionary semantics (entry bounds, redaction).
    pub semantic: bool,
    /// When true, verifies section digests if a DigestManifest is present.
    pub verify_digests: bool,
    /// When true, unknown optional extension registry entries are allowed.
    pub allow_unknown_optional_extensions: bool,
    /// Controls whether optional pushdown/acceleration metadata may fail open.
    pub optional_pushdown_policy: OptionalPushdownPolicy,
}

/// Policy for optional pushdown/acceleration sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OptionalPushdownPolicy {
    /// Corrupt optional metadata rejects the file, suitable for audit tooling.
    Strict,
    /// Corrupt optional metadata is ignored so readers can scan safely.
    FailOpen,
}

/// Optional pushdown metadata ignored under [`OptionalPushdownPolicy::FailOpen`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredOptionalSection {
    pub section_id: u32,
    pub section_kind: u16,
    pub reason: String,
}

/// Coarse validation stages surfaced by [`ValidationReport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationStage {
    Bootstrap,
    Structural,
    SharedSemantic,
    DigestVerification,
    CoveTable,
    CoveObject,
    CoveEngine,
    CoveHarbor,
    CoveMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationStageStatus {
    Checked,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationStageReport {
    pub stage: ValidationStage,
    pub status: ValidationStageStatus,
    pub sections_checked: u32,
}

impl Default for ValidationOptions {
    fn default() -> Self {
        Self {
            semantic: false,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            optional_pushdown_policy: OptionalPushdownPolicy::Strict,
        }
    }
}

/// Result of [`validate_bytes_with_options`].
#[derive(Debug, Clone)]
pub struct ValidationReport {
    /// The structurally validated file.
    pub validated: super::ValidatedCoveFile,
    /// Whether semantic checks were performed.
    pub semantic_checked: bool,
    /// Number of dictionary entries, if the dictionary was parsed.
    pub dict_entry_count: Option<u32>,
    /// Per-stage validation outcomes.
    pub stages: Vec<ValidationStageReport>,
    /// Optional pushdown sections ignored under fail-open validation.
    pub ignored_optional_sections: Vec<IgnoredOptionalSection>,
}

/// Optional-profile payload validator used by feature-use validation.
///
/// `cove-core` can identify when requested feature use depends on embedded
/// optional profile sections, but it intentionally does not depend on the
/// layout, coverage, runtime, or COVE-I crates that parse those payloads.
pub trait OptionalProfilePayloadValidator {
    fn validate_optional_profile_sections(
        &self,
        data: &[u8],
        report: &ValidationReport,
        optional_pushdown_policy: OptionalPushdownPolicy,
        feature_use: Option<&FeatureUseRequestV2>,
        only_required_for_feature_use: bool,
    ) -> Result<(), CoveError>;
}

/// Validate a COVE file with configurable options.
///
/// Always performs structural validation (equivalent to [`super::validate_bytes`]).
/// When `opts.semantic` is true, additionally parses any file dictionary.
/// When `opts.verify_digests` is true, verifies any `DIGEST_MANIFEST` section
/// against section bytes.
pub fn validate_bytes_with_options(
    data: &[u8],
    opts: ValidationOptions,
) -> Result<ValidationReport, CoveError> {
    validate_bytes_with_options_inner(data, opts, None)
}

fn validate_bytes_with_options_inner(
    data: &[u8],
    opts: ValidationOptions,
    request: Option<&FeatureUseRequestV2>,
) -> Result<ValidationReport, CoveError> {
    let (validated, mut ignored_optional_sections) =
        super::validate_bytes_with_optional_pushdown_policy(data, opts.optional_pushdown_policy)?;
    let mut stages = vec![
        ValidationStageReport {
            stage: ValidationStage::Bootstrap,
            status: ValidationStageStatus::Checked,
            sections_checked: 0,
        },
        ValidationStageReport {
            stage: ValidationStage::Structural,
            status: ValidationStageStatus::Checked,
            sections_checked: validated.footer.sections.len() as u32,
        },
    ];

    if !opts.semantic {
        push_stage(
            &mut stages,
            ValidationStage::SharedSemantic,
            ValidationStageStatus::Skipped,
            0,
        );
        if opts.verify_digests {
            let checked = super::digest_verification::verify_digest_manifests(
                data,
                &validated.footer,
                &ignored_optional_sections,
            )?;
            push_stage(
                &mut stages,
                ValidationStage::DigestVerification,
                ValidationStageStatus::Checked,
                checked,
            );
        } else {
            push_stage(
                &mut stages,
                ValidationStage::DigestVerification,
                ValidationStageStatus::Skipped,
                0,
            );
        }
        if let Some(request) = request {
            push_requested_profile_stages(data, &validated, &mut stages, request)?;
        } else {
            push_skipped_profile_stages(&mut stages);
        }
        return Ok(ValidationReport {
            validated,
            semantic_checked: false,
            dict_entry_count: None,
            stages,
            ignored_optional_sections,
        });
    }

    let mut dict_entry_count: Option<u32> = None;
    super::profile_validators::validate_shared_semantics(
        data,
        &validated,
        &opts,
        &mut dict_entry_count,
        &mut stages,
        &mut ignored_optional_sections,
    )?;
    if opts.verify_digests {
        let checked = super::digest_verification::verify_digest_manifests(
            data,
            &validated.footer,
            &ignored_optional_sections,
        )?;
        push_stage(
            &mut stages,
            ValidationStage::DigestVerification,
            ValidationStageStatus::Checked,
            checked,
        );
    } else {
        push_stage(
            &mut stages,
            ValidationStage::DigestVerification,
            ValidationStageStatus::Skipped,
            0,
        );
    }
    super::profile_validators::validate_cove_t_semantics(
        data,
        &validated,
        &opts,
        &mut stages,
        &mut ignored_optional_sections,
    )?;
    super::profile_validators::validate_cove_o_semantics(data, &validated, &mut stages, request)?;
    super::profile_validators::validate_cove_e_semantics(data, &validated, &mut stages, request)?;
    super::profile_validators::validate_cove_h_semantics(data, &validated, &mut stages, request)?;
    super::profile_validators::validate_cove_map_semantics(data, &validated, &mut stages, request)?;

    Ok(ValidationReport {
        validated,
        semantic_checked: opts.semantic,
        dict_entry_count,
        stages,
        ignored_optional_sections,
    })
}

pub fn validate_bytes_for_feature_use(
    data: &[u8],
    opts: ValidationOptions,
    request: FeatureUseRequestV2,
) -> Result<ValidationReport, CoveError> {
    let optional_pushdown_policy = opts.optional_pushdown_policy;
    let report = validate_bytes_with_options_inner(data, opts, Some(&request))?;
    for ignored in &report.ignored_optional_sections {
        if super::profile_validators::ignored_section_required_for_feature_use(ignored, &request) {
            return Err(CoveError::ChecksumMismatch);
        }
    }
    let scope_table = super::feature_scope_table_for_feature_use(data, &report.validated)?;
    scope_table.reject_unknowns_for_request(&request)?;
    fail_closed_required_optional_profile_sections(&report, optional_pushdown_policy, &request)?;
    Ok(report)
}

pub fn validate_bytes_for_ordinary_table_scan(
    data: &[u8],
    opts: ValidationOptions,
    request: FeatureUseRequestV2,
) -> Result<ValidationReport, CoveError> {
    let optional_pushdown_policy = opts.optional_pushdown_policy;
    let (validated, mut ignored_optional_sections) =
        super::validate_bytes_with_optional_pushdown_policy(data, opts.optional_pushdown_policy)?;
    let mut stages = vec![
        ValidationStageReport {
            stage: ValidationStage::Bootstrap,
            status: ValidationStageStatus::Checked,
            sections_checked: 0,
        },
        ValidationStageReport {
            stage: ValidationStage::Structural,
            status: ValidationStageStatus::Checked,
            sections_checked: validated.footer.sections.len() as u32,
        },
    ];

    let mut dict_entry_count: Option<u32> = None;
    super::profile_validators::validate_shared_semantics(
        data,
        &validated,
        &opts,
        &mut dict_entry_count,
        &mut stages,
        &mut ignored_optional_sections,
    )?;
    if opts.verify_digests {
        let checked = super::digest_verification::verify_digest_manifests(
            data,
            &validated.footer,
            &ignored_optional_sections,
        )?;
        push_stage(
            &mut stages,
            ValidationStage::DigestVerification,
            ValidationStageStatus::Checked,
            checked,
        );
    } else {
        push_stage(
            &mut stages,
            ValidationStage::DigestVerification,
            ValidationStageStatus::Skipped,
            0,
        );
    }
    super::profile_validators::validate_cove_t_semantics_with_registered_page_scope(
        data,
        &validated,
        &opts,
        &mut stages,
        &mut ignored_optional_sections,
        super::profile_validators::RegisteredPageValidationScope::RequestedPages(&request),
    )?;
    super::profile_validators::validate_cove_o_semantics(
        data,
        &validated,
        &mut stages,
        Some(&request),
    )?;
    super::profile_validators::validate_cove_e_semantics(
        data,
        &validated,
        &mut stages,
        Some(&request),
    )?;
    super::profile_validators::validate_cove_h_semantics(
        data,
        &validated,
        &mut stages,
        Some(&request),
    )?;
    super::profile_validators::validate_cove_map_semantics(
        data,
        &validated,
        &mut stages,
        Some(&request),
    )?;

    let report = ValidationReport {
        validated,
        semantic_checked: true,
        dict_entry_count,
        stages,
        ignored_optional_sections,
    };
    for ignored in &report.ignored_optional_sections {
        if super::profile_validators::ignored_section_required_for_feature_use(ignored, &request) {
            return Err(CoveError::ChecksumMismatch);
        }
    }
    let scope_table = super::feature_scope_table_for(data, &report.validated)?;
    scope_table.reject_unknowns_for_request(&request)?;
    fail_closed_required_optional_profile_sections(&report, optional_pushdown_policy, &request)?;
    Ok(report)
}

pub fn validate_bytes_for_feature_use_with_optional_profile_validator<V>(
    data: &[u8],
    opts: ValidationOptions,
    request: FeatureUseRequestV2,
    validator: &V,
) -> Result<ValidationReport, CoveError>
where
    V: OptionalProfilePayloadValidator + ?Sized,
{
    let optional_pushdown_policy = opts.optional_pushdown_policy;
    let report = validate_bytes_with_options_inner(data, opts, Some(&request))?;
    for ignored in &report.ignored_optional_sections {
        if super::profile_validators::ignored_section_required_for_feature_use(ignored, &request) {
            return Err(CoveError::ChecksumMismatch);
        }
    }
    let scope_table = super::feature_scope_table_for_feature_use(data, &report.validated)?;
    scope_table.reject_unknowns_for_request(&request)?;
    validator.validate_optional_profile_sections(
        data,
        &report,
        optional_pushdown_policy,
        Some(&request),
        !report.semantic_checked,
    )?;
    Ok(report)
}

fn fail_closed_required_optional_profile_sections(
    report: &ValidationReport,
    _optional_pushdown_policy: OptionalPushdownPolicy,
    request: &FeatureUseRequestV2,
) -> Result<(), CoveError> {
    for entry in &report.validated.footer.sections {
        let Some(kind) = crate::constants::SectionKind::from_u16(entry.section_kind) else {
            continue;
        };
        if super::profile_validators::is_embedded_optional_profile_section(kind)
            && super::profile_validators::section_entry_required_for_feature_use(entry, request)
        {
            return Err(CoveError::UnsupportedEncoding(
                "requested optional profile payload requires a semantic validator".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn push_stage(
    stages: &mut Vec<ValidationStageReport>,
    stage: ValidationStage,
    status: ValidationStageStatus,
    sections_checked: u32,
) {
    stages.push(ValidationStageReport {
        stage,
        status,
        sections_checked,
    });
}

fn push_skipped_profile_stages(stages: &mut Vec<ValidationStageReport>) {
    for stage in [
        ValidationStage::CoveTable,
        ValidationStage::CoveObject,
        ValidationStage::CoveEngine,
        ValidationStage::CoveHarbor,
        ValidationStage::CoveMap,
    ] {
        push_stage(stages, stage, ValidationStageStatus::Skipped, 0);
    }
}

fn push_requested_profile_stages(
    data: &[u8],
    validated: &super::ValidatedCoveFile,
    stages: &mut Vec<ValidationStageReport>,
    request: &FeatureUseRequestV2,
) -> Result<(), CoveError> {
    push_stage(
        stages,
        ValidationStage::CoveTable,
        ValidationStageStatus::Skipped,
        0,
    );
    if super::profile_validators::request_requires_object_profile(request) {
        super::profile_validators::validate_cove_o_semantics(
            data,
            validated,
            stages,
            Some(request),
        )?;
    } else {
        push_stage(
            stages,
            ValidationStage::CoveObject,
            ValidationStageStatus::Skipped,
            0,
        );
    }
    if super::profile_validators::request_requires_engine_profile(request) {
        super::profile_validators::validate_cove_e_semantics(
            data,
            validated,
            stages,
            Some(request),
        )?;
    } else {
        push_stage(
            stages,
            ValidationStage::CoveEngine,
            ValidationStageStatus::Skipped,
            0,
        );
    }
    if super::profile_validators::request_requires_harbor_profile(request) {
        super::profile_validators::validate_cove_h_semantics(
            data,
            validated,
            stages,
            Some(request),
        )?;
    } else {
        push_stage(
            stages,
            ValidationStage::CoveHarbor,
            ValidationStageStatus::Skipped,
            0,
        );
    }
    if super::profile_validators::request_requires_map_profile(request) {
        super::profile_validators::validate_cove_map_semantics(
            data,
            validated,
            stages,
            Some(request),
        )?;
    } else {
        push_stage(
            stages,
            ValidationStage::CoveMap,
            ValidationStageStatus::Skipped,
            0,
        );
    }
    Ok(())
}
