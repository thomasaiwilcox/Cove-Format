use super::*;

pub(crate) fn execution_diagnostics_for_physical(
    physical: &PhysicalPlannedQuery,
) -> Vec<ExecutionDiagnostic> {
    let mut diagnostics = physical
        .planned
        .diagnostics
        .iter()
        .cloned()
        .map(ExecutionDiagnostic::from)
        .collect::<Vec<_>>();
    diagnostics.extend(
        physical
            .diagnostics
            .iter()
            .map(|diagnostic| ExecutionDiagnostic {
                code: diagnostic.code.clone(),
                severity: diagnostic.severity,
                message: diagnostic.message.clone(),
                phase: diagnostic.phase.clone(),
                safe_details: diagnostic.safe_details.clone(),
                redacted: diagnostic.redacted,
            }),
    );
    diagnostics
}

pub(super) fn execute_physical_explain_only(
    physical: PhysicalPlannedQuery,
    kernel_options: KernelExecutionOptions,
) -> Result<KernelExecutedQuery, BuildExecutionError> {
    let result = CoveQlExecutionResult::ExplainJson(physical.explain_json());
    let output_fingerprint = result_fingerprint(&result)?;
    let executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics: physical
            .diagnostics
            .iter()
            .map(|diagnostic| ExecutionDiagnostic {
                code: diagnostic.code.clone(),
                severity: diagnostic.severity,
                message: diagnostic.message.clone(),
                phase: diagnostic.phase.clone(),
                safe_details: diagnostic.safe_details.clone(),
                redacted: diagnostic.redacted,
            })
            .collect(),
        row_counts: ExecutionRowCounts::default(),
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_executed(
            &ExecutionOptions::default().pushdown,
        ),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::physical_plan_only(
            "physical explain output did not execute data rows",
        ),
    };
    let mut kernel_report = KernelExecutionReport::fallback(
        kernel_options.mode,
        KernelFallbackReason::ExplainOnly,
        "explain output does not execute Phase 7 kernels",
    );
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
    Ok(KernelExecutedQuery {
        physical,
        executed,
        kernel_report,
    })
}

pub(crate) fn attach_phase8_plan_reports(
    report: &mut KernelExecutionReport,
    planned: &crate::PlannedQuery,
) {
    report.association = crate::AssociationOptimizationReport::for_plan(planned, &[]);
    report.evidence = crate::EvidenceOptimizationReport::for_plan(planned, None);
    report.lineage = crate::LineageReuseReport::for_plan(planned);
}
