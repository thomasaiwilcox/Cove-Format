use super::*;

pub fn execute_manifest_physical_planned_query(
    members: &[ManifestDatasetMember<'_>],
    physical: PhysicalPlannedQuery,
    execution_options: ExecutionOptions,
    kernel_options: KernelExecutionOptions,
) -> Result<KernelExecutedQuery, BuildExecutionError> {
    if matches!(
        physical.planned.resolved.output_mode,
        crate::CoveQlOutputMode::ExplainJson
    ) {
        return execute_physical_explain_only(physical, kernel_options);
    }

    validate_security_scope(&physical.planned, &execution_options)?;
    validate_execution_output_mode(&physical.planned)?;
    let ordered_members = validate_manifest_execution_members(&physical.planned, members)?;

    if kernel_options.mode == KernelExecutionMode::ForceMaterialized {
        let executed =
            execute_manifest_planned_query(members, physical.planned.clone(), execution_options)?;
        let mut kernel_report = KernelExecutionReport::disabled(
            KernelExecutionMode::ForceMaterialized,
            "manifest physical kernel execution disabled by ForceMaterialized mode",
        );
        attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
        return Ok(KernelExecutedQuery {
            physical,
            executed,
            kernel_report,
        });
    }

    let shape = match compile_kernel_shape(&physical) {
        Ok(shape) => shape,
        Err(reason) if kernel_options.mode == KernelExecutionMode::ForceKernel => {
            let kernel_shape = diagnostic_kernel_shape_for_plan(&physical.planned, reason);
            return Err(exec_error(
                kernel_unsupported_diagnostic_code(
                    reason,
                    &kernel_shape,
                    "E_MANIFEST_KERNEL_UNSUPPORTED",
                ),
                "ForceKernel mode rejected a manifest query without an eligible physical kernel shape",
                json!({
                    "reason": format!("{reason:?}"),
                    "kernel_shape": kernel_shape,
                    "fallback_boundary": "manifest_materialized",
                }),
            ))
        }
        Err(reason) => {
            return manifest_kernel_fallback(
                members,
                physical,
                execution_options,
                kernel_options,
                reason,
                "manifest physical execution fell back because the query lacks an eligible kernel shape",
            )
        }
    };

    if let Some(reason) =
        manifest_native_unsupported_reason(&physical, &shape.public, &execution_options)
    {
        if kernel_options.mode == KernelExecutionMode::ForceKernel {
            return Err(exec_error(
                kernel_unsupported_diagnostic_code(
                    reason,
                    &shape.public,
                    "E_MANIFEST_KERNEL_UNSUPPORTED",
                ),
                "ForceKernel mode rejected a manifest query outside the exact native manifest kernel surface",
                json!({
                    "reason": format!("{reason:?}"),
                    "kernel_shape": shape.public,
                    "fallback_boundary": "manifest_materialized",
                }),
            ));
        }
        return manifest_kernel_fallback(
            members,
            physical,
            execution_options,
            kernel_options,
            reason,
            "manifest physical execution fell back because no exact manifest native kernel path was proven",
        );
    }

    let started = Instant::now();
    let source = manifest_kernel_row_source(
        &ordered_members,
        &physical.planned,
        &execution_options,
        &shape,
        started,
    )?;
    let Some(native_result) =
        try_manifest_native_result(&source, &physical.planned, &execution_options, started)?
    else {
        if kernel_options.mode == KernelExecutionMode::ForceKernel {
            return Err(exec_error(
                "E_MANIFEST_KERNEL_UNSUPPORTED",
                "ForceKernel mode could not produce an exact manifest native result",
                json!({
                    "kernel_shape": shape.public,
                    "fallback_boundary": "manifest_materialized",
                }),
            ));
        }
        return manifest_kernel_fallback(
            members,
            physical,
            execution_options,
            kernel_options,
            KernelFallbackReason::UnsupportedProjection,
            "manifest physical execution fell back because native result execution declined the merged row set",
        );
    };
    let result = native_result.result;
    let row_counts = native_result.row_counts;
    enforce_result_budgets(
        &result,
        &row_counts,
        &physical.planned,
        &execution_options,
        started,
    )?;
    let output_fingerprint = result_fingerprint(&result)?;

    let mut diagnostics = execution_diagnostics_for_physical(&physical);
    diagnostics.push(exec_warning(
        native_result.diagnostic_code,
        native_result.message,
        {
            let mut details = native_result.details.clone();
            details["file_count"] = json!(ordered_members.len());
            details["rows_scanned"] = json!(source.rows_scanned);
            details["rows_after_bitmap"] = json!(source.rows_after_bitmap);
            details["reconstructed_states"] = json!(source.reconstructed_states);
            details["residual_verification"] = json!(false);
            details["fallback_boundary"] = Value::Null;
            details
        },
    ));
    if native_result.diagnostic_code != "W_MANIFEST_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED" {
        diagnostics.push(exec_warning(
            "W_MANIFEST_KERNEL_NATIVE_EXECUTED",
            "manifest physical kernel executed member-local predicates, merged validated member rows, and produced exact native output",
            json!({
            "file_count": ordered_members.len(),
            "native_result": native_result.kind,
            "rows_scanned": source.rows_scanned,
            "rows_after_bitmap": source.rows_after_bitmap,
            "reconstructed_states": source.reconstructed_states,
            "residual_verification": false,
            "fallback_boundary": Value::Null,
            }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(&physical.planned) {
        diagnostics.push(warning);
    }

    let counters = KernelCounters {
        rows_scanned: source.rows_scanned,
        rows_after_bitmap: source.rows_after_bitmap,
        rows_after_selection_vector: source.rows_after_selection_vector,
        candidate_object_keys: source.candidate_object_keys,
        retained_record_chain_rows: source.retained_record_chain_rows,
        reconstructed_states: source.reconstructed_states,
        residual_rows_checked: 0,
        output_rows: row_counts.output_rows,
        bitmap_words: source.bitmap_words,
        selection_vector_len: source.rows_after_selection_vector,
        typed_predicate_rows: shape.predicates.len() * source.rows_scanned,
        dictionary_lookups_at_materialization: row_counts.output_rows,
        bytes_touched_estimate: source.rows_scanned * std::mem::size_of::<u64>() * 5,
        scratch_high_water_bytes: source.bitmap_words * std::mem::size_of::<u64>()
            + source.rows_after_selection_vector * std::mem::size_of::<u32>(),
        ..KernelCounters::default()
    };
    if counters.scratch_high_water_bytes > kernel_options.scratch_budget_bytes {
        return Err(resource_error(
            "manifest_kernel_scratch_budget_bytes",
            counters.scratch_high_water_bytes,
        ));
    }
    let mut kernel_report = KernelExecutionReport::applied(kernel_options.mode, counters);
    kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
        "manifest native kernel used validated member identities, exact code-domain bridge contracts, member-local predicate kernels, and global manifest row finalization",
    );
    kernel_report.decision = KernelDecision::new(
        KernelDecisionKind::Applied,
        "manifest native kernel executed without materialized residuals",
        json!({
            "kernel_shape": shape.public,
            "file_count": ordered_members.len(),
            "native_result": native_result.kind,
            "details": native_result.details,
            "residual_verification": false,
            "row_grain": "manifest_merged_reconstructed_visible_rows",
        }),
        false,
    );
    kernel_report.decisions = vec![kernel_report.decision.clone()];
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);

    let mut executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_applicable(
            &execution_options.pushdown,
            format!(
                "manifest physical kernel read {} validated member files and applied member-local predicate kernels before global native output",
                ordered_members.len()
            ),
        ),
        evidence_authority: combined_evidence_authority(&source.evidence_authorities),
        authority: ExecutionAuthorityReport::exact_kernel(
            native_result.authority_reason,
            false,
        ),
    };

    if kernel_options.mode == KernelExecutionMode::CompareWithMaterialized {
        let materialized =
            execute_manifest_planned_query(members, physical.planned.clone(), execution_options)?;
        kernel_report.compared_with_materialized = true;
        kernel_report.materialized_fingerprint = Some(materialized.output_fingerprint.clone());
        kernel_report.kernel_fingerprint = Some(executed.output_fingerprint.clone());
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Compared,
            "manifest native output fingerprint was compared with the materialized manifest baseline",
            json!({}),
            false,
        ));
        if materialized.output_fingerprint != executed.output_fingerprint {
            return Err(exec_error(
                "E_MANIFEST_KERNEL_MISMATCH",
                "manifest native output fingerprint does not match materialized baseline",
                json!({}),
            ));
        }
        executed.authority.compared_with_materialized = true;
        executed.diagnostics.push(exec_warning(
            "W_MANIFEST_KERNEL_COMPARE_MATCHED",
            "manifest native output matched the materialized baseline fingerprint",
            json!({}),
        ));
    }

    Ok(KernelExecutedQuery {
        physical,
        executed,
        kernel_report,
    })
}

fn manifest_kernel_fallback(
    members: &[ManifestDatasetMember<'_>],
    physical: PhysicalPlannedQuery,
    execution_options: ExecutionOptions,
    kernel_options: KernelExecutionOptions,
    reason: KernelFallbackReason,
    message: &'static str,
) -> Result<KernelExecutedQuery, BuildExecutionError> {
    let executed =
        execute_manifest_planned_query(members, physical.planned.clone(), execution_options)?;
    let mut kernel_report = KernelExecutionReport::fallback(kernel_options.mode, reason, message);
    kernel_report.decision.safe_details = json!({
        "reason": format!("{reason:?}"),
        "kernel_shape": diagnostic_kernel_shape_for_plan(&physical.planned, reason),
        "residual_verification": true,
        "fallback_boundary": "manifest_materialized",
        "manifest_file_count": physical.planned.resolved.operation_context.dataset.files.len(),
    });
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
    Ok(KernelExecutedQuery {
        physical,
        executed,
        kernel_report,
    })
}

fn manifest_native_unsupported_reason(
    physical: &PhysicalPlannedQuery,
    shape: &crate::kernel_plan::KernelShape,
    options: &ExecutionOptions,
) -> Option<KernelFallbackReason> {
    if options.visibility_overlay.is_some() {
        return Some(KernelFallbackReason::UnsupportedPredicate);
    }
    if shape.residual_verification_required {
        return Some(KernelFallbackReason::UnsupportedProjection);
    }
    if !manifest_native_shape_supported(&physical.planned) {
        return Some(KernelFallbackReason::UnsupportedProjection);
    }
    if matches!(shape.root_kind, crate::KernelRootKind::Projection)
        && !native_projection_root_scan_shape(&physical.planned)
    {
        return Some(KernelFallbackReason::NonObjectRoot);
    }
    None
}

pub(super) fn kernel_unsupported_diagnostic_code(
    reason: KernelFallbackReason,
    shape: &KernelShape,
    default_code: &'static str,
) -> &'static str {
    if reason == KernelFallbackReason::UnsafeCodedPredicate
        || shape.operator_contracts.iter().any(|contract| {
            contract.representation_class == CodedRepresentationClass::CrossSourceCodeBridge
                && contract.residual_required
        })
    {
        "E_UNSAFE_CODE_DOMAIN"
    } else {
        default_code
    }
}

fn manifest_native_shape_supported(planned: &crate::PlannedQuery) -> bool {
    native_direct_projection_shape(planned)
        || native_helper_exists_direct_projection_shape(planned)
        || native_association_root_scan_shape(planned)
        || native_evidence_root_scan_shape(planned)
        || native_projection_root_scan_shape(planned)
        || native_role_bound_direct_projection_shape(planned)
        || native_bool_group_count_shape(planned)
        || native_grouped_helper_aggregate_shape(planned)
        || native_helper_aggregate_shape(planned)
        || native_direct_aggregate_shape(planned)
        || native_typed_order_shape(planned)
}

#[derive(Debug, Clone)]
struct ManifestNativeResult {
    result: CoveQlExecutionResult,
    row_counts: ExecutionRowCounts,
    kind: &'static str,
    diagnostic_code: &'static str,
    message: &'static str,
    authority_reason: &'static str,
    details: Value,
}

fn try_manifest_native_result(
    source: &ManifestKernelRowSource,
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<Option<ManifestNativeResult>, BuildExecutionError> {
    let mut rows = source.rows.clone();
    let helper_prefilter = apply_kernel_helper_prefilters(
        &mut rows,
        &source.associations,
        &source.evidence_rows,
        planned,
    );
    let helper_prefilter_exact = native_helper_exists_direct_projection_shape(planned);
    if helper_prefilter.applied_terms() > 0 && !helper_prefilter_exact {
        return Ok(None);
    }
    if helper_prefilter.skipped_terms() > 0 {
        return Ok(None);
    }

    if let Some((result, row_counts, report)) =
        try_native_root_scan_result(&rows, planned, options, started)?
    {
        let diagnostic_code = match report.root_kind {
            "association" => "W_MANIFEST_KERNEL_NATIVE_ASSOCIATION_ROOT_SCAN_EXECUTED",
            "evidence" => "W_MANIFEST_KERNEL_NATIVE_EVIDENCE_ROOT_SCAN_EXECUTED",
            _ => "W_MANIFEST_KERNEL_NATIVE_ROOT_SCAN_EXECUTED",
        };
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "root_scan",
            diagnostic_code,
            message: "manifest native direct root scan produced exact row output after global member merge",
            authority_reason: "manifest native direct root scan produced exact visible output",
            details: json!({
                "root_kind": report.root_kind,
                "root_type": report.root_type,
                "rows_returned": report.rows_returned,
            }),
        }));
    }

    if let Some((result, row_counts, report)) =
        try_native_projection_root_scan_result(&rows, planned, options, started)?
    {
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "projection_root_scan",
            diagnostic_code: "W_MANIFEST_KERNEL_NATIVE_PROJECTION_ROOT_SCAN_EXECUTED",
            message: "manifest native projection root scan produced exact provider output after global member merge",
            authority_reason: "manifest native projection root scan produced exact visible output",
            details: json!({
                "projection_id": report.projection_id,
                "rows_returned": report.rows_returned,
            }),
        }));
    }

    if let Some((result, row_counts, report)) = try_native_direct_projection_result(
        &rows,
        &source.associations,
        &source.evidence_rows,
        planned,
        options,
        started,
        helper_prefilter_exact,
    )? {
        let mut details = json!({
            "root_kind": report.root_kind,
            "column_count": report.column_count,
            "rows_projected": report.rows_projected,
        });
        if helper_prefilter.applied_terms() > 0 {
            details["helper_prefilter"] = json!({
                "input_rows": helper_prefilter.input_rows,
                "output_rows": helper_prefilter.output_rows,
                "association_semi_join_terms": helper_prefilter.association_semi_join_terms,
                "association_anti_join_terms": helper_prefilter.association_anti_join_terms,
                "evidence_semi_join_terms": helper_prefilter.evidence_semi_join_terms,
                "evidence_anti_join_terms": helper_prefilter.evidence_anti_join_terms,
            });
        }
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "direct_projection",
            diagnostic_code: "W_MANIFEST_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED",
            message: "manifest native direct projection produced exact selected output after global member merge",
            authority_reason: "manifest native direct projection produced exact visible output",
            details,
        }));
    }

    if let Some((result, row_counts, report)) =
        try_native_bool_group_count_result(&rows, planned, options, started)?
    {
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "grouped_direct_aggregate",
            diagnostic_code: "W_MANIFEST_KERNEL_NATIVE_DIRECT_GROUP_AGGREGATE_EXECUTED",
            message: "manifest native direct grouped aggregate produced exact output after global member merge",
            authority_reason: "manifest native direct grouped aggregate produced exact visible output",
            details: json!({
                "aggregate": report.aggregate,
                "group_property": report.group_property,
                "logical_type": report.group_logical_type,
                "group_count": report.group_count,
                "rows_counted": report.rows_counted,
                "values_seen": report.values_seen,
                "group_strategy": report.group_strategy,
                "aggregate_strategy": report.aggregate_strategy,
            }),
        }));
    }

    if let Some((result, row_counts, report)) = try_native_grouped_helper_aggregate_result(
        &rows,
        &source.associations,
        &source.evidence_rows,
        planned,
        options,
        started,
    )? {
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "grouped_helper_aggregate",
            diagnostic_code: "W_MANIFEST_KERNEL_NATIVE_GROUPED_HELPER_AGGREGATE_EXECUTED",
            message: "manifest native grouped helper aggregate produced exact output after global member merge",
            authority_reason: "manifest native grouped helper aggregate produced exact visible output",
            details: json!({
                "aggregate": report.aggregate,
                "helper_kind": report.helper_kind,
                "group_property": report.group_property,
                "logical_type": report.group_logical_type,
                "group_count": report.group_count,
                "rows_counted": report.rows_counted,
                "helper_values_seen": report.helper_values_seen,
            }),
        }));
    }

    if let Some((result, row_counts, report)) = try_native_helper_aggregate_result(
        &rows,
        &source.associations,
        &source.evidence_rows,
        planned,
        options,
        started,
    )? {
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "helper_aggregate",
            diagnostic_code: "W_MANIFEST_KERNEL_NATIVE_HELPER_AGGREGATE_EXECUTED",
            message:
                "manifest native helper aggregate produced exact output after global member merge",
            authority_reason: "manifest native helper aggregate produced exact visible output",
            details: json!({
                "aggregate": report.aggregate,
                "helper_kind": report.helper_kind,
                "rows_counted": report.rows_counted,
                "helper_values_seen": report.helper_values_seen,
            }),
        }));
    }

    if let Some((result, row_counts, report)) =
        try_native_direct_aggregate_result(&rows, planned, options, started)?
    {
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "direct_aggregate",
            diagnostic_code: "W_MANIFEST_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED",
            message:
                "manifest native direct aggregate produced exact output after global member merge",
            authority_reason: "manifest native direct aggregate produced exact visible output",
            details: json!({
                "aggregate": report.aggregate,
                "counted_path": report.counted_path,
                "rows_counted": report.rows_counted,
                "values_seen": report.values_seen,
                "aggregate_strategy": report.aggregate_strategy,
            }),
        }));
    }

    if let Some((result, row_counts, report)) =
        try_native_typed_order_result(&rows, planned, options, started)?
    {
        return Ok(Some(ManifestNativeResult {
            result,
            row_counts,
            kind: "typed_order",
            diagnostic_code: "W_MANIFEST_KERNEL_NATIVE_TYPED_ORDER_EXECUTED",
            message: "manifest native typed order produced exact sorted projection output after global member merge",
            authority_reason: "manifest native typed order produced exact visible output",
            details: json!({
                "order_property": report.order_property,
                "logical_type": report.logical_type,
                "rows_sorted": report.rows_sorted,
                "output_rows": report.output_rows,
            }),
        }));
    }

    Ok(None)
}

#[derive(Debug, Clone, Default)]
struct ManifestKernelRowSource {
    rows: Vec<ExecutionRow>,
    associations: Vec<MaterializedAssociationRow>,
    evidence_rows: Vec<MaterializedEvidenceRow>,
    evidence_authorities: Vec<crate::EvidenceAuthority>,
    rows_scanned: usize,
    rows_after_bitmap: usize,
    rows_after_selection_vector: usize,
    candidate_object_keys: usize,
    retained_record_chain_rows: usize,
    reconstructed_states: usize,
    bitmap_words: usize,
}

fn manifest_kernel_row_source(
    members: &[crate::execution::ManifestExecutionMemberRef<'_>],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    shape: &crate::kernel_plan::CompiledKernelShape,
    started: Instant,
) -> Result<ManifestKernelRowSource, BuildExecutionError> {
    let mut source = ManifestKernelRowSource::default();
    for member in members {
        check_time(&options.resource_budget, started)?;
        let member_plan = manifest_member_plan(planned, member);
        if let Some(projection_id) = projection_backed_root_id(&member_plan.resolved.root) {
            let file_id = hex(&member.scope.file_id);
            let rows = projection_root_execution_rows(
                member.bytes,
                &member_plan,
                options,
                started,
                projection_id,
            )?
            .into_iter()
            .map(|row| {
                row.with_dataset_member(member.scope.ordinal, &member.scope.source, file_id.clone())
            })
            .collect::<Vec<_>>();
            source.rows_scanned += rows.len();
            source.rows_after_bitmap += rows.len();
            source.rows_after_selection_vector += rows.len();
            source.reconstructed_states += rows.len();
            source.rows.extend(rows);
            continue;
        }
        let mut pushdown_plan = pushdown::pushdown_read_plan(&member_plan, &options.pushdown);
        let read = read_object_kernel_surface_from_bytes_with_options(
            member.bytes,
            &CoveObjectKernelReadOptions {
                read: CoveObjectReadWithPushdownOptions {
                    read: object_read_options(&member_plan),
                    pushdown: pushdown_plan.read_options.clone(),
                },
            },
        )
        .map_err(|err| {
            exec_error(
                "E_MANIFEST_KERNEL_READBACK",
                format!("manifest member COVE-O kernel readback failed: {err}"),
                json!({ "source": member.scope.source }),
            )
        })?;
        pushdown_plan
            .report
            .merge_core_report(read.pushdown_report.clone());

        let native_role_bound_projection = native_role_bound_direct_projection_shape(planned);
        let (
            states,
            candidate_object_keys,
            retained_record_chain_rows,
            selected_row_count,
            selection_vector_len,
            bitmap_words,
        ) = if native_role_bound_projection {
            let Some(binding) = member_plan.resolved.temporal.role_binding.as_deref() else {
                return Err(exec_error(
                    "E_MANIFEST_KERNEL_TEMPORAL_ROLE",
                    "manifest native role-bound temporal projection requires a resolved timestamp binding",
                    json!({ "source": member.scope.source }),
                ));
            };
            let TemporalMode::AsOfTimestampMicros(timestamp_micros) =
                member_plan.resolved.temporal.mode
            else {
                return Err(exec_error(
                    "E_MANIFEST_KERNEL_TEMPORAL_ROLE",
                    "manifest native role-bound temporal projection requires an asOf timestamp bound",
                    json!({ "source": member.scope.source, "binding": binding }),
                ));
            };
            let states = role_bound_states_at(
                &read.materialized_surface,
                &member_plan,
                binding,
                timestamp_micros,
            )?;
            let selected = states.len();
            (
                states,
                selected,
                selected,
                selected,
                selected,
                selected.div_ceil(64),
            )
        } else {
            let selection = select_rows_with_base(
                &read.kernel_surface,
                shape.public.root_object_type_id,
                &shape.predicates,
                None,
            );
            let (selection_vector, _compaction_stats) =
                compact_selection_bitmap(&selection, NativeKernelDispatch::Auto).map_err(
                    |error| {
                        exec_error(
                            "E_MANIFEST_KERNEL_SELECTION_COMPACT",
                            format!("manifest native selection-vector compaction failed: {error}"),
                            json!({ "source": member.scope.source }),
                        )
                    },
                )?;
            let reconstruction = reconstruction_options(&member_plan)?;
            let retained_dependency_type_ids = member_plan
                .dependencies
                .association_type_ids
                .iter()
                .copied()
                .collect();
            let (states, candidate_object_keys, retained_record_chain_rows) =
                reconstruct_selected_object_states(
                    &read.materialized_surface,
                    &selection_vector,
                    &reconstruction,
                    &retained_dependency_type_ids,
                )
                .map_err(|err| {
                    exec_error(
                        "E_MANIFEST_KERNEL_RECONSTRUCT",
                        err.to_string(),
                        json!({ "source": member.scope.source }),
                    )
                })?;
            (
                states,
                candidate_object_keys,
                retained_record_chain_rows,
                selection.count_ones(),
                selection_vector.len(),
                selection.word_count(),
            )
        };
        let associations = states
            .iter()
            .filter_map(MaterializedAssociationRow::from_state)
            .map(|row| {
                row.with_dataset_member(
                    member.scope.ordinal,
                    &member.scope.source,
                    hex(&member.scope.file_id),
                )
            })
            .collect::<Vec<_>>();
        let (evidence_execution_rows, _) = materialized_evidence_rows_for_plan(
            &member_plan,
            read.materialized_surface.evidence_index.as_ref(),
        );
        let evidence_execution_rows = if evidence_execution_rows.is_empty()
            && matches!(member_plan.resolved.root, ResolvedRoot::Evidence(_))
        {
            evidence_object_rows_from_states(&states)
        } else {
            evidence_execution_rows
        };
        if matches!(member_plan.resolved.root, ResolvedRoot::Evidence(_))
            && read.materialized_surface.evidence_index.is_none()
            && evidence_execution_rows.is_empty()
        {
            return Err(exec_error(
                "E_EVIDENCE_CATALOG_REQUIRED",
                "evidence roots require embedded COVE-MAP evidence metadata or materialized COVE-O evidence objects in manifest kernel execution",
                json!({ "source": member.scope.source }),
            ));
        }
        let evidence_rows = evidence_execution_rows
            .iter()
            .filter_map(|row| match row {
                ExecutionRow::Evidence(row) => Some(row.clone()),
                _ => None,
            })
            .map(|mut row| {
                row.fields
                    .insert("dataset_file_ordinal".into(), json!(member.scope.ordinal));
                row.fields.insert(
                    "dataset_file_source".into(),
                    Value::String(member.scope.source.clone()),
                );
                row.fields.insert(
                    "dataset_file_id".into(),
                    Value::String(hex(&member.scope.file_id)),
                );
                row
            })
            .collect::<Vec<_>>();
        if let Some(authority) = manifest_member_evidence_authority(
            &member_plan,
            read.materialized_surface.evidence_index.is_some(),
            !evidence_execution_rows.is_empty(),
        ) {
            source.evidence_authorities.push(authority);
        }
        let file_id = hex(&member.scope.file_id);
        let rows = match &member_plan.resolved.root {
            ResolvedRoot::Object(root) => states
                .iter()
                .filter(|state| state.object_type_id == root.object_type_id)
                .map(MaterializedObjectRow::from_state)
                .map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                })
                .map(ExecutionRow::Object)
                .collect::<Vec<_>>(),
            ResolvedRoot::Association(root) => associations
                .iter()
                .filter(|association| association.object_type_id == root.object_type_id)
                .filter(|association| {
                    association_row_matches_temporal(
                        association,
                        root,
                        association_valid_at(&member_plan),
                    )
                })
                .cloned()
                .map(ExecutionRow::Association)
                .collect::<Vec<_>>(),
            ResolvedRoot::Node(root) => states
                .iter()
                .filter(|state| state.object_type_id == root.object.object_type_id)
                .map(MaterializedObjectRow::from_state)
                .map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                })
                .map(ExecutionRow::Object)
                .collect::<Vec<_>>(),
            ResolvedRoot::Edge(root) => associations
                .iter()
                .filter(|association| association.object_type_id == root.association.object_type_id)
                .filter(|association| {
                    association_row_matches_temporal(
                        association,
                        &root.association,
                        association_valid_at(&member_plan),
                    )
                })
                .cloned()
                .map(ExecutionRow::Association)
                .collect::<Vec<_>>(),
            ResolvedRoot::Evidence(_) => evidence_execution_rows
                .into_iter()
                .map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                })
                .collect(),
            ResolvedRoot::Table(_) => return Err(exec_error(
                "E_MANIFEST_KERNEL_UNSUPPORTED",
                "table roots require projection-backed table execution and are not supported by manifest native direct-projection execution",
                json!({ "source": member.scope.source }),
            )),
            ResolvedRoot::Projection(_) => return Err(exec_error(
                "E_MANIFEST_KERNEL_UNSUPPORTED",
                "projection roots are not supported by manifest native direct-projection execution",
                json!({ "source": member.scope.source }),
            )),
        };

        source.rows_scanned += read.kernel_surface.system.len();
        source.rows_after_bitmap += selected_row_count;
        source.rows_after_selection_vector += selection_vector_len;
        source.candidate_object_keys += candidate_object_keys;
        source.retained_record_chain_rows += retained_record_chain_rows;
        source.reconstructed_states += states.len();
        source.bitmap_words += bitmap_words;
        source.rows.extend(rows);
        source.associations.extend(associations);
        source.evidence_rows.extend(evidence_rows);
    }
    Ok(source)
}

fn manifest_member_evidence_authority(
    planned: &crate::PlannedQuery,
    has_evidence_index: bool,
    has_evidence_rows: bool,
) -> Option<crate::EvidenceAuthority> {
    if !matches!(planned.resolved.root, ResolvedRoot::Evidence(_)) {
        return None;
    }
    Some(if has_evidence_index {
        crate::EvidenceAuthority::CoveMapMetadata
    } else if has_evidence_rows {
        crate::EvidenceAuthority::MaterializedEvidenceObjects
    } else {
        crate::EvidenceAuthority::Missing
    })
}
