use super::*;

pub fn execute_manifest_planned_query(
    members: &[ManifestDatasetMember<'_>],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let started = Instant::now();
    validate_security_scope(&planned, &options)?;
    validate_execution_output_mode(&planned)?;
    if matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::DataFusionTableProvider
    ) {
        return Err(exec_error(
            "E_UNSUPPORTED_OUTPUT",
            "manifest execution returns materialized CoveQL output; register an CoveQL DataFusion provider separately",
            json!({}),
        ));
    }
    let ordered_members = validate_manifest_execution_members(&planned, members)?;

    let mut diagnostics = planned
        .diagnostics
        .iter()
        .cloned()
        .map(ExecutionDiagnostic::from)
        .collect::<Vec<_>>();
    let (exact_bridge_count, inexact_bridge_count) =
        manifest_code_domain_bridge_counts(&planned.resolved.operation_context.dataset);
    let manifest_fallback_reason = if exact_bridge_count > 0 {
        "manifest member execution validated exact COVM code-domain bridge proofs, but this logical executor used the materialized CoveQL oracle across members because no manifest physical kernel path was selected"
    } else {
        "manifest member execution used the materialized CoveQL oracle across validated COVM members because cross-file coded acceleration requires exact bridge proofs and a manifest physical kernel path"
    };
    diagnostics.push(exec_warning(
        "W_MATERIALIZED_MANIFEST_BASELINE",
        manifest_fallback_reason,
        json!({
            "file_count": ordered_members.len(),
            "file_membership_fingerprint": planned.resolved.operation_context.dataset.file_membership_fingerprint.clone(),
            "exact_code_domain_bridge_count": exact_bridge_count,
            "inexact_code_domain_bridge_count": inexact_bridge_count,
            "fallback_boundary": if exact_bridge_count > 0 {
                "manifest_physical_kernel_not_selected"
            } else {
                "manifest_cross_file_bridge_not_exact"
            },
        }),
    ));
    if let Some(warning) = zero_copy_owned_fallback_warning(&planned) {
        diagnostics.push(warning);
    }

    let (result, row_counts, pushdown_report, evidence_authority) =
        match &planned.resolved.output_mode {
            CoveQlOutputMode::ExplainJson => {
                let explain = planned.explain_json();
                (
                    CoveQlExecutionResult::ExplainJson(explain),
                    ExecutionRowCounts::default(),
                    PushdownReport::not_executed(&options.pushdown),
                    None,
                )
            }
            CoveQlOutputMode::DataFusionTableProvider => unreachable!("handled above"),
            _ => execute_manifest_rows(&ordered_members, &planned, &options, started)?,
        };

    enforce_result_budgets(&result, &row_counts, &planned, &options, started)?;
    let output_fingerprint = result_fingerprint(&result)?;
    Ok(ExecutedQuery {
        planned,
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report,
        evidence_authority,
        authority: ExecutionAuthorityReport::materialized_baseline(
            "manifest materialized baseline execution produced the visible output",
        ),
    })
}

pub fn execute_manifest_planned_query_retained(
    members: &[CoveQlRetainedManifestMember],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let borrowed = members
        .iter()
        .map(CoveQlRetainedManifestMember::as_manifest_member)
        .collect::<Vec<_>>();
    execute_manifest_planned_query(&borrowed, planned, options)
}

pub(super) fn manifest_code_domain_bridge_counts(
    dataset: &crate::DatasetScopeContext,
) -> (usize, usize) {
    let exact = dataset
        .code_domain_bridges
        .iter()
        .filter(|bridge| bridge.exact)
        .count();
    let inexact = dataset.code_domain_bridges.len().saturating_sub(exact);
    (exact, inexact)
}

pub(crate) struct ManifestExecutionMemberRef<'a> {
    pub(crate) scope: DatasetFileIdentity,
    pub(crate) file: ValidatedFileIdentity,
    pub(crate) bytes: &'a [u8],
}

pub(crate) fn validate_manifest_execution_members<'a>(
    planned: &PlannedQuery,
    members: &'a [ManifestDatasetMember<'a>],
) -> Result<Vec<ManifestExecutionMemberRef<'a>>, BuildExecutionError> {
    let expected_files = &planned.resolved.operation_context.dataset.files;
    if expected_files.is_empty() {
        return Err(exec_error(
            "E_UNSUPPORTED_DATASET_SCOPE",
            "manifest execution requires a resolved dataset scope with at least one member file",
            json!({}),
        ));
    }
    if expected_files.len() != members.len() {
        return Err(exec_error(
            "E_DATASET_MEMBER_MISMATCH",
            "manifest execution member count does not match the resolved dataset scope",
            json!({
                "expected_file_count": expected_files.len(),
                "provided_file_count": members.len(),
            }),
        ));
    }

    let mut ordered_expected = expected_files.clone();
    ordered_expected.sort_by_key(|file| file.ordinal);
    let mut used = vec![false; members.len()];
    let mut ordered = Vec::with_capacity(ordered_expected.len());
    for expected in ordered_expected {
        let Some((member_index, member)) = members
            .iter()
            .enumerate()
            .find(|(_, member)| member.source == expected.source)
        else {
            return Err(exec_error(
                "E_DATASET_MEMBER_MISMATCH",
                "manifest execution is missing a member file required by the resolved dataset scope",
                json!({ "source": expected.source }),
            ));
        };
        if used[member_index] {
            return Err(exec_error(
                "E_DATASET_MEMBER_MISMATCH",
                "manifest execution received a duplicate member source",
                json!({ "source": expected.source }),
            ));
        }
        used[member_index] = true;
        let validated = validate_bytes(member.bytes).map_err(|err| {
            exec_error(
                "E_DATASET_MEMBER_INVALID",
                format!(
                    "manifest execution member {} failed COVE validation: {err}",
                    member.source
                ),
                json!({ "source": member.source }),
            )
        })?;
        let file = ValidatedFileIdentity::from(&validated);
        validate_manifest_member_identity(&expected, &file)?;
        ordered.push(ManifestExecutionMemberRef {
            scope: expected,
            file,
            bytes: member.bytes,
        });
    }
    if let Some(member) = members
        .iter()
        .enumerate()
        .find(|(index, _)| !used[*index])
        .map(|(_, member)| member)
    {
        return Err(exec_error(
            "E_DATASET_MEMBER_MISMATCH",
            "manifest execution received a member file not present in the resolved dataset scope",
            json!({ "source": member.source }),
        ));
    }
    Ok(ordered)
}

pub(super) fn validate_manifest_member_identity(
    expected: &DatasetFileIdentity,
    actual: &ValidatedFileIdentity,
) -> Result<(), BuildExecutionError> {
    if expected.file_id == actual.file_id
        && expected.file_len == actual.file_len
        && expected.footer_crc32c == actual.footer_crc32c
        && expected.primary_profile == actual.primary_profile
    {
        return Ok(());
    }
    Err(exec_error(
        "E_DATASET_MEMBER_STALE",
        "manifest execution member identity does not match the resolved dataset scope",
        json!({
            "source": expected.source,
            "expected": {
                "file_id": hex(&expected.file_id),
                "file_len": expected.file_len,
                "footer_crc32c": expected.footer_crc32c,
                "primary_profile": expected.primary_profile,
            },
            "actual": {
                "file_id": hex(&actual.file_id),
                "file_len": actual.file_len,
                "footer_crc32c": actual.footer_crc32c,
                "primary_profile": actual.primary_profile,
            },
        }),
    ))
}

pub(super) fn execute_manifest_rows(
    members: &[ManifestExecutionMemberRef<'_>],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    (
        CoveQlExecutionResult,
        ExecutionRowCounts,
        PushdownReport,
        Option<EvidenceAuthority>,
    ),
    BuildExecutionError,
> {
    if matches!(
        planned.resolved.root,
        ResolvedRoot::Node(_)
            | ResolvedRoot::Edge(_)
            | ResolvedRoot::Table(_)
            | ResolvedRoot::Projection(_)
            | ResolvedRoot::Evidence(_)
    ) && (planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some())
    {
        return Err(incompatible_execution_grain(
            planned,
            "history and changes output grains require object or association roots",
        ));
    }

    let mut rows = Vec::new();
    let mut associations = Vec::new();
    let mut evidence_rows = Vec::new();
    let mut object_rows = Vec::new();
    let mut evidence_authorities = Vec::new();
    for member in members {
        check_time(&options.resource_budget, started)?;
        let member_plan = manifest_member_plan(planned, member);
        let file_id = hex(&member.scope.file_id);
        match &member_plan.resolved.root {
            ResolvedRoot::Projection(root) => {
                rows.extend(
                    projection_root_execution_rows(
                        member.bytes,
                        &member_plan,
                        options,
                        started,
                        &root.projection_id,
                    )?
                    .into_iter()
                    .map(|row| {
                        row.with_dataset_member(
                            member.scope.ordinal,
                            &member.scope.source,
                            file_id.clone(),
                        )
                    }),
                );
            }
            ResolvedRoot::Table(root) => {
                rows.extend(
                    projection_root_execution_rows(
                        member.bytes,
                        &member_plan,
                        options,
                        started,
                        &root.projection.projection_id,
                    )?
                    .into_iter()
                    .map(|row| {
                        row.with_dataset_member(
                            member.scope.ordinal,
                            &member.scope.source,
                            file_id.clone(),
                        )
                    }),
                );
            }
            ResolvedRoot::Object(_)
            | ResolvedRoot::Association(_)
            | ResolvedRoot::Node(_)
            | ResolvedRoot::Edge(_)
            | ResolvedRoot::Evidence(_) => {
                let source =
                    object_backed_row_source(member.bytes, &member_plan, options, started)?;
                rows.extend(source.rows.into_iter().map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                }));
                associations.extend(source.associations.into_iter().map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                }));
                object_rows.extend(source.object_rows.into_iter().map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                }));
                evidence_rows.extend(source.evidence_rows.into_iter().map(|mut row| {
                    row.fields
                        .insert("dataset_file_ordinal".into(), json!(member.scope.ordinal));
                    row.fields.insert(
                        "dataset_file_source".into(),
                        Value::String(member.scope.source.clone()),
                    );
                    row.fields
                        .insert("dataset_file_id".into(), Value::String(file_id.clone()));
                    row
                }));
                if let Some(authority) = source.evidence_authority {
                    evidence_authorities.push(authority);
                }
            }
        }
    }

    let (result, row_counts) = finish_materialized_rows(
        rows,
        &associations,
        &evidence_rows,
        &object_rows,
        planned,
        options,
        started,
    )?;
    Ok((
        result,
        row_counts,
        manifest_materialized_pushdown_report(planned, options, members.len()),
        combined_evidence_authority(&evidence_authorities),
    ))
}

pub(crate) fn manifest_member_plan(
    planned: &PlannedQuery,
    member: &ManifestExecutionMemberRef<'_>,
) -> PlannedQuery {
    let mut member_plan = planned.clone();
    member_plan.resolved.operation_context.file = member.file.clone();
    member_plan.resolved.operation_context.dataset =
        crate::DatasetScopeContext::single_file_with_source(
            &member.file,
            &planned.resolved.operation_context.snapshot,
            &planned.resolved.operation_context.security,
            member.scope.source.clone(),
        );
    member_plan
}

pub(super) fn manifest_materialized_pushdown_report(
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    file_count: usize,
) -> PushdownReport {
    PushdownReport::not_applicable(
        &options.pushdown,
        format!(
            "manifest execution read {file_count} validated member files and applied global materialized CoveQL residual semantics for {} root",
            match planned.resolved.root {
                ResolvedRoot::Object(_) => "object",
                ResolvedRoot::Association(_) => "association",
                ResolvedRoot::Node(_) => "node",
                ResolvedRoot::Edge(_) => "edge",
                ResolvedRoot::Table(_) => "table",
                ResolvedRoot::Evidence(_) => "evidence",
                ResolvedRoot::Projection(_) => "projection",
            }
        ),
    )
}

pub(crate) fn combined_evidence_authority(
    authorities: &[EvidenceAuthority],
) -> Option<EvidenceAuthority> {
    let first = authorities.first().copied()?;
    if authorities.iter().all(|authority| *authority == first) {
        return Some(first);
    }
    Some(EvidenceAuthority::MaterializedEvidenceObjects)
}
