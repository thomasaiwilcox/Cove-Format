use super::*;

pub(super) fn reject_duplicate(
    seen: &mut bool,
    method: &'static str,
    resolve_options: &ResolveOptions,
) -> Result<(), BuildResolvedQueryError> {
    if *seen {
        Err(duplicate(
            format!("duplicate or conflicting {method} method"),
            resolve_options,
        ))
    } else {
        *seen = true;
        Ok(())
    }
}

pub(super) fn duplicate(
    message: impl Into<String>,
    resolve_options: &ResolveOptions,
) -> BuildResolvedQueryError {
    BuildResolvedQueryError {
        diagnostics: vec![diagnostic(
            "E_DUPLICATE_METHOD",
            message,
            "resolve",
            &resolve_options.security,
        )],
        rejections: vec![RejectionReport {
            kind: RejectionKind::FeatureValidation,
            reason: "duplicate method in method chain".into(),
        }],
        source: None,
    }
}

pub(super) fn conflict(
    message: impl Into<String>,
    resolve_options: &ResolveOptions,
) -> BuildResolvedQueryError {
    BuildResolvedQueryError {
        diagnostics: vec![diagnostic(
            "E_METHOD_CONFLICT",
            message,
            "resolve",
            &resolve_options.security,
        )],
        rejections: vec![RejectionReport {
            kind: RejectionKind::FeatureValidation,
            reason: "method chain conflict".into(),
        }],
        source: None,
    }
}

pub(super) fn warning(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    security: &SecurityContext,
) -> CoveQlDiagnostic {
    let mut diagnostic = diagnostic(code, message, phase, security);
    diagnostic.severity = DiagnosticSeverity::Warning;
    diagnostic
}

pub(super) fn diagnostic(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    security: &SecurityContext,
) -> CoveQlDiagnostic {
    CoveQlDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: phase.into(),
        safe_details: json!({}),
        redacted: security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected,
    }
}
