use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct KernelHelperPrefilterReport {
    pub(super) input_rows: usize,
    pub(super) output_rows: usize,
    pub(super) association_semi_join_terms: usize,
    pub(super) association_anti_join_terms: usize,
    pub(super) evidence_semi_join_terms: usize,
    pub(super) evidence_anti_join_terms: usize,
    pub(super) skipped_complex_terms: usize,
    pub(super) skipped_security_terms: usize,
}

impl KernelHelperPrefilterReport {
    pub(super) fn applied_terms(&self) -> usize {
        self.association_semi_join_terms
            + self.association_anti_join_terms
            + self.evidence_semi_join_terms
            + self.evidence_anti_join_terms
    }

    pub(super) fn skipped_terms(&self) -> usize {
        self.skipped_complex_terms + self.skipped_security_terms
    }
}

enum KernelHelperPrefilterTerm<'a> {
    Association {
        root: &'a crate::ResolvedAssociationRoot,
        negated: bool,
    },
    Evidence {
        root: &'a crate::ResolvedEvidenceRoot,
        negated: bool,
    },
}

pub(super) fn apply_kernel_helper_prefilters(
    rows: &mut Vec<ExecutionRow>,
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[crate::materialized::MaterializedEvidenceRow],
    planned: &crate::PlannedQuery,
) -> KernelHelperPrefilterReport {
    let mut terms = Vec::new();
    let mut report = KernelHelperPrefilterReport {
        input_rows: rows.len(),
        output_rows: rows.len(),
        ..KernelHelperPrefilterReport::default()
    };
    collect_kernel_helper_prefilter_terms(
        planned.resolved.method_chain.where_predicate.as_ref(),
        &mut terms,
        &mut report,
    );
    if terms.is_empty() {
        return report;
    }
    if !kernel_helper_prefilters_are_safe(planned) {
        report.skipped_security_terms += terms.len();
        return report;
    }

    let association_edges = AssociationEdgeTable::from_rows_for_plan(planned, associations);
    let evidence_index = EvidenceGrainIndex::from_rows(evidence_rows);
    for term in &terms {
        match term {
            KernelHelperPrefilterTerm::Association { negated: false, .. } => {
                report.association_semi_join_terms += 1;
            }
            KernelHelperPrefilterTerm::Association { negated: true, .. } => {
                report.association_anti_join_terms += 1;
            }
            KernelHelperPrefilterTerm::Evidence { negated: false, .. } => {
                report.evidence_semi_join_terms += 1;
            }
            KernelHelperPrefilterTerm::Evidence { negated: true, .. } => {
                report.evidence_anti_join_terms += 1;
            }
        }
    }
    rows.retain(|row| {
        terms.iter().all(|term| {
            let found = match term {
                KernelHelperPrefilterTerm::Association { root, .. } => current_row_goid(row)
                    .map(|goid| {
                        association_edges.exists_for_scoped(
                            current_row_dataset_file_ordinal(row),
                            goid,
                            root,
                        )
                    })
                    .unwrap_or(false),
                KernelHelperPrefilterTerm::Evidence { root, .. } => {
                    evidence_index.exists_for(row, root)
                }
            };
            match term {
                KernelHelperPrefilterTerm::Association { negated, .. }
                | KernelHelperPrefilterTerm::Evidence { negated, .. } => {
                    if *negated {
                        !found
                    } else {
                        found
                    }
                }
            }
        })
    });
    report.output_rows = rows.len();
    report
}

pub(super) fn kernel_helper_prefilters_are_safe(planned: &crate::PlannedQuery) -> bool {
    matches!(planned.resolved.root, ResolvedRoot::Object(_))
        && matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            VisibilityPolicy::AllRows
        )
}

fn collect_kernel_helper_prefilter_terms<'a>(
    predicate: Option<&'a ResolvedPredicate>,
    terms: &mut Vec<KernelHelperPrefilterTerm<'a>>,
    report: &mut KernelHelperPrefilterReport,
) {
    let Some(predicate) = predicate else {
        return;
    };
    if let Some(term) = kernel_helper_prefilter_term(predicate, false) {
        terms.push(term);
        return;
    }
    match predicate {
        ResolvedPredicate::And(parts) => {
            for part in parts {
                if let Some(term) = kernel_helper_prefilter_term(part, false) {
                    terms.push(term);
                } else if predicate_contains_helper_exists(part) {
                    report.skipped_complex_terms += 1;
                }
            }
        }
        _ if predicate_contains_helper_exists(predicate) => {
            report.skipped_complex_terms += 1;
        }
        _ => {}
    }
}

fn kernel_helper_prefilter_term<'a>(
    predicate: &'a ResolvedPredicate,
    negated: bool,
) -> Option<KernelHelperPrefilterTerm<'a>> {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::Association(root)) => {
            Some(KernelHelperPrefilterTerm::Association { root, negated })
        }
        ResolvedPredicate::Exists(ResolvedExpr::Evidence(root)) => {
            Some(KernelHelperPrefilterTerm::Evidence { root, negated })
        }
        ResolvedPredicate::Not(inner) => kernel_helper_prefilter_term(inner, !negated),
        _ => None,
    }
}

pub(super) fn predicate_contains_helper_exists(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::Association(_))
        | ResolvedPredicate::Exists(ResolvedExpr::Evidence(_)) => true,
        ResolvedPredicate::Not(inner) => predicate_contains_helper_exists(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_helper_exists)
        }
        ResolvedPredicate::Compare { left, right, .. } => {
            expr_contains_helper_exists(left) || expr_contains_helper_exists(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr)
        | ResolvedPredicate::Exists(expr) => expr_contains_helper_exists(expr),
    }
}

pub(super) fn expr_contains_helper_exists(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) => true,
        ResolvedExpr::FunctionCall { args, .. } => args.iter().any(expr_contains_helper_exists),
        ResolvedExpr::AggregateCall { arg, .. } => arg
            .as_deref()
            .map(expr_contains_helper_exists)
            .unwrap_or(false),
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_helper_exists(predicate)
                || expr_contains_helper_exists(then_expr)
                || expr_contains_helper_exists(else_expr)
        }
        _ => false,
    }
}

pub(super) fn execute_kernel_object(
    bytes: &[u8],
    retained_input: Option<&CoveQlRetainedInput>,
    physical: &PhysicalPlannedQuery,
    options: &ExecutionOptions,
    kernel_options: &KernelExecutionOptions,
    shape: crate::kernel_plan::CompiledKernelShape,
) -> Result<(ExecutedQuery, KernelExecutionReport), BuildExecutionError> {
    let started = Instant::now();
    let planned = &physical.planned;
    validate_security_scope(planned, options)?;
    validate_execution_grain(planned)?;

    let mut diagnostics = planned
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
    if shape.public.residual_verification_required {
        diagnostics.push(exec_warning(
            "W_KERNEL_BASELINE_AUTHORITY",
            "Phase 7 kernel execution retained residual materialized verification as the semantic authority",
            json!({
                "mode": format!("{:?}", kernel_options.mode),
                "residual_verification": true,
            }),
        ));
    } else {
        diagnostics.push(exec_warning(
            "W_KERNEL_EXACT_AUTHORITY",
            "Phase 7 kernel execution has exact optimized authority for this proven shape",
            json!({
                "mode": format!("{:?}", kernel_options.mode),
                "residual_verification": false,
            }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(planned) {
        diagnostics.push(warning);
    }

    if let Some(projection_id) = projection_backed_root_id(&planned.resolved.root) {
        let exact_projection_scan = crate::kernel_plan::native_projection_root_scan_shape(planned)
            && !shape.public.residual_verification_required;
        let (result, row_counts) = crate::execution::execute_projection_root(
            bytes,
            planned,
            options,
            started,
            projection_id,
        )?;
        enforce_result_budgets(&result, &row_counts, planned, options, started)?;
        let output_fingerprint = result_fingerprint(&result)?;
        if exact_projection_scan {
            diagnostics.push(exec_warning(
                "W_KERNEL_NATIVE_PROJECTION_READBACK_EXECUTED",
                "projection root executed through an exact COVE-MAP projection provider scan",
                json!({
                    "projection_id": projection_id,
                    "residual_verification": false,
                }),
            ));
        } else {
            diagnostics.push(exec_warning(
                "W_KERNEL_PROJECTION_MATERIALIZED_READBACK_EXECUTED",
                "projection root executed through COVE-MAP materialized readback inside the kernel wrapper",
                json!({
                    "projection_id": projection_id,
                    "fallback_boundary": "projection_materialized_readback",
                    "residual_verification": true,
                }),
            ));
        }
        let counters = KernelCounters {
            output_rows: row_counts.output_rows,
            residual_rows_checked: if exact_projection_scan {
                0
            } else {
                row_counts.input_rows
            },
            dictionary_lookups_at_materialization: if exact_projection_scan {
                0
            } else {
                row_counts.output_rows
            },
            ..KernelCounters::default()
        };
        let mut kernel_report = KernelExecutionReport::applied(kernel_options.mode, counters);
        kernel_report.optimization_authority = if exact_projection_scan {
            OptimizationAuthorityReport::authoritative(
                "projection root direct scan used exact COVE-MAP projection batches with selected columns and primitive pushed filters",
            )
        } else {
            OptimizationAuthorityReport::materialized_fallback(
                "projection roots use COVE-MAP materialized readback; no coded projection kernel has been proven for this shape",
            )
        };
        kernel_report.decision = KernelDecision::new(
            KernelDecisionKind::Applied,
            if exact_projection_scan {
                "projection root executed through exact COVE-MAP projection provider scan"
            } else {
                "projection root executed through explicit materialized readback boundary"
            },
            if exact_projection_scan {
                json!({
                    "kernel_shape": shape.public,
                    "projection_id": projection_id,
                    "residual_verification": false,
                    "pushed_filters": "primitive_projection_filters",
                    "pushed_columns": "selected_projection_columns",
                })
            } else {
                json!({
                    "kernel_shape": shape.public,
                    "projection_id": projection_id,
                    "residual_verification": true,
                    "fallback_boundary": "projection_materialized_readback",
                })
            },
            false,
        );
        kernel_report.decisions = vec![kernel_report.decision.clone()];
        attach_phase8_plan_reports(&mut kernel_report, planned);
        let authority = if exact_projection_scan {
            ExecutionAuthorityReport::exact_kernel(
                "projection root executed through exact COVE-MAP projection provider scan",
                false,
            )
        } else {
            let mut authority = ExecutionAuthorityReport::materialized_baseline(
                "projection root executed through COVE-MAP materialized readback inside the kernel wrapper",
            );
            authority.materialized_fallback = true;
            authority
        };
        let pushdown_report =
            crate::execution::projection_readback_pushdown_report(planned, options, &row_counts);
        let executed = ExecutedQuery {
            planned: planned.clone(),
            result,
            diagnostics,
            row_counts,
            output_fingerprint,
            pushdown_report,
            evidence_authority: None,
            authority,
        };
        return Ok((executed, kernel_report));
    }

    let mut pushdown_plan = pushdown::pushdown_read_plan(planned, &options.pushdown);
    let read = read_object_kernel_surface_from_bytes_with_options(
        bytes,
        &CoveObjectKernelReadOptions {
            read: CoveObjectReadWithPushdownOptions {
                read: object_read_options(planned),
                pushdown: pushdown_plan.read_options.clone(),
            },
        },
    )
    .map_err(|err| {
        exec_error(
            "E_KERNEL_READBACK",
            format!("COVE-O kernel readback failed: {err}"),
            json!({}),
        )
    })?;
    pushdown_plan
        .report
        .merge_core_report(read.pushdown_report.clone());

    let coverage_prune = try_coverage_row_range_prune(
        physical,
        &read.kernel_surface,
        shape.public.root_object_type_id,
    )?;
    let covi_prune = try_covi_row_range_prune(
        physical,
        &read.kernel_surface,
        shape.public.root_object_type_id,
    )?;
    let execution_code_prune = try_execution_code_filecode_prune(
        bytes,
        physical,
        &read.kernel_surface,
        shape.public.root_object_type_id,
        options,
    )?;
    let file_code_prune = if execution_code_prune.is_none() {
        try_file_code_literal_prune(
            bytes,
            physical,
            &read.kernel_surface,
            shape.public.root_object_type_id,
        )?
    } else {
        None
    };
    let native_scalar_prune = try_native_scalar_predicate_prune(
        bytes,
        retained_input,
        physical,
        &read.kernel_surface,
        shape.public.root_object_type_id,
        &shape.predicates,
    )?;
    let execution_code_predicates_exact = execution_code_prune.as_ref().is_some_and(|prune| {
        code_prune_covers_kernel_predicate_count(
            prune.predicate_count,
            prune.page_count,
            shape.predicates.len(),
        )
    });
    let file_code_predicates_exact = file_code_prune.as_ref().is_some_and(|prune| {
        code_prune_covers_kernel_predicate_count(
            prune.predicate_count,
            prune.page_count,
            shape.predicates.len(),
        )
    });
    if let Some(prune) = &coverage_prune {
        diagnostics.push(exec_warning(
            "W_COVERAGE_ROW_RANGE_PRUNE_EXECUTED",
            "validated COVE-COVERAGE row-range set narrowed kernel candidate rows before residual predicate evaluation",
            json!({
                "predicate_form_ref": prune.predicate_form_ref,
                "row_range_count": prune.row_range_count,
                "matched_rows": prune.matched_rows,
            }),
        ));
    }
    if let Some(prune) = &covi_prune {
        diagnostics.push(exec_warning(
            "W_COVI_ROW_RANGE_PRUNE_EXECUTED",
            "validated exact COVI/COVX row-range lookup narrowed kernel candidate rows before residual predicate evaluation",
            json!({
                "source": prune.source,
                "lookup_count": prune.lookup_count,
                "row_range_count": prune.row_range_count,
                "matched_rows": prune.matched_rows,
            }),
        ));
    }
    if let Some(prune) = &execution_code_prune {
        diagnostics.push(exec_warning(
            "W_EXECUTION_CODE_REMAP_EXECUTED",
            "validated COVE-E FileCode-to-execution-code remap narrowed kernel candidate rows before residual predicate evaluation",
            json!({
                "predicate_count": prune.predicate_count,
                "page_count": prune.page_count,
                "matched_rows": prune.matched_rows,
                "residual_verification": !execution_code_predicates_exact,
            }),
        ));
    }
    if let Some(prune) = &file_code_prune {
        diagnostics.push(exec_warning(
            "W_FILE_CODE_LITERAL_PRUNE_EXECUTED",
            "single-file FileCode literal predicate narrowed kernel candidate rows before final predicate evaluation",
            json!({
                "predicate_count": prune.predicate_count,
                "page_count": prune.page_count,
                "matched_rows": prune.matched_rows,
                "dataset_file_count": planned.resolved.operation_context.dataset.files.len(),
                "residual_verification": !file_code_predicates_exact,
            }),
        ));
    }
    let native_scalar_predicates_exact = native_scalar_prune.as_ref().is_some_and(|prune| {
        native_scalar_prune_covers_all_kernel_predicates(prune, &shape.predicates)
    });
    if let Some(prune) = &native_scalar_prune {
        diagnostics.push(exec_warning(
            "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED",
            "native scalar page lanes narrowed kernel candidate rows before final predicate evaluation",
            json!({
                "predicate_count": prune.predicate_count,
                "executed_predicate_count": prune.executed_predicate_count,
                "page_count": prune.page_count,
                "matched_rows": prune.matched_rows,
                "predicate_order": prune.predicate_order.clone(),
                "short_circuited": prune.short_circuited,
                "residual_verification": !native_scalar_predicates_exact,
                "predicate_kernels": prune.predicate_dispatch.kernels,
                "predicate_kernel_scalar": prune.predicate_dispatch.scalar,
                "predicate_kernel_avx2": prune.predicate_dispatch.avx2,
                "predicate_kernel_neon": prune.predicate_dispatch.neon,
                "bitmap_intersections": prune.bitmap_dispatch.intersections,
                "bitmap_intersection_scalar": prune.bitmap_dispatch.scalar,
                "bitmap_intersection_avx2": prune.bitmap_dispatch.avx2,
                "bitmap_intersection_neon": prune.bitmap_dispatch.neon,
                "retained_page_buffers": prune.retained_page_buffers,
            }),
        ));
    }
    let mut base_selection = coverage_prune.as_ref().map(|prune| prune.bitmap.clone());
    if let Some(prune) = &covi_prune {
        if let Some(base) = &mut base_selection {
            let _ = base.intersect_with_dispatch(&prune.bitmap, NativeKernelDispatch::Auto);
        } else {
            base_selection = Some(prune.bitmap.clone());
        }
    }
    if let Some(prune) = &file_code_prune {
        if let Some(base) = &mut base_selection {
            let _ = base.intersect_with_dispatch(&prune.bitmap, NativeKernelDispatch::Auto);
        } else {
            base_selection = Some(prune.bitmap.clone());
        }
    }
    if let Some(prune) = &execution_code_prune {
        if let Some(base) = &mut base_selection {
            let _ = base.intersect_with_dispatch(&prune.bitmap, NativeKernelDispatch::Auto);
        } else {
            base_selection = Some(prune.bitmap.clone());
        }
    }
    if let Some(prune) = &native_scalar_prune {
        if let Some(base) = &mut base_selection {
            let _ = base.intersect_with_dispatch(&prune.bitmap, NativeKernelDispatch::Auto);
        } else {
            base_selection = Some(prune.bitmap.clone());
        }
    }

    let selection_predicates = if native_scalar_predicates_exact
        || execution_code_predicates_exact
        || file_code_predicates_exact
    {
        &[][..]
    } else {
        shape.predicates.as_slice()
    };
    let selection = select_rows_with_base(
        &read.kernel_surface,
        shape.public.root_object_type_id,
        selection_predicates,
        base_selection,
    );
    let (selection_vector, selection_compaction_stats) =
        compact_selection_bitmap(&selection, NativeKernelDispatch::Auto).map_err(|error| {
            exec_error(
                "E_KERNEL_SELECTION_COMPACT",
                format!("native selection-vector compaction failed: {error}"),
                json!({}),
            )
        })?;
    let native_role_bound_projection = native_role_bound_direct_projection_shape(planned);
    let reconstruction = reconstruction_options(planned)?;
    let retained_dependency_type_ids = planned
        .dependencies
        .association_type_ids
        .iter()
        .copied()
        .collect();
    let (states, candidate_object_keys, retained_record_chain_rows) =
        if native_role_bound_projection {
            let Some(binding) = planned.resolved.temporal.role_binding.as_deref() else {
                return Err(exec_error(
                    "E_KERNEL_TEMPORAL_ROLE",
                    "native role-bound temporal projection requires a resolved timestamp binding",
                    json!({}),
                ));
            };
            let TemporalMode::AsOfTimestampMicros(timestamp_micros) =
                planned.resolved.temporal.mode
            else {
                return Err(exec_error(
                    "E_KERNEL_TEMPORAL_ROLE",
                    "native role-bound temporal projection requires an asOf timestamp bound",
                    json!({ "binding": binding }),
                ));
            };
            let states = role_bound_states_at(
                &read.materialized_surface,
                planned,
                binding,
                timestamp_micros,
            )?;
            let retained = states.len();
            (states, retained, retained)
        } else {
            reconstruct_selected_object_states(
                &read.materialized_surface,
                &selection_vector,
                &reconstruction,
                &retained_dependency_type_ids,
            )
            .map_err(|err| exec_error("E_KERNEL_RECONSTRUCT", err.to_string(), json!({})))?
        };

    let associations = states
        .iter()
        .filter_map(MaterializedAssociationRow::from_state)
        .collect::<Vec<_>>();
    let (evidence_execution_rows, evidence_report) = materialized_evidence_rows_for_plan(
        planned,
        read.materialized_surface.evidence_index.as_ref(),
    );
    let evidence_execution_rows = if evidence_execution_rows.is_empty()
        && matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
    {
        evidence_object_rows_from_states(&states)
    } else {
        evidence_execution_rows
    };
    if matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
        && read.materialized_surface.evidence_index.is_none()
        && evidence_execution_rows.is_empty()
    {
        return Err(exec_error(
            "E_EVIDENCE_CATALOG_REQUIRED",
            "evidence roots require embedded COVE-MAP evidence metadata or materialized COVE-O evidence objects in kernel execution",
            json!({}),
        ));
    }
    let evidence_rows = evidence_execution_rows
        .iter()
        .filter_map(|row| match row {
            ExecutionRow::Evidence(row) => Some(row.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut rows = match &planned.resolved.root {
        ResolvedRoot::Object(root) if native_temporal_direct_projection_shape(planned) => {
            crate::execution::temporal_object_rows(
                &read.materialized_surface,
                planned,
                root.object_type_id,
            )?
        }
        ResolvedRoot::Association(root) if native_temporal_direct_projection_shape(planned) => {
            crate::execution::temporal_association_rows(
                &read.materialized_surface,
                planned,
                root.object_type_id,
            )?
        }
        ResolvedRoot::Object(root) => states
            .iter()
            .filter(|state| state.object_type_id == root.object_type_id)
            .map(MaterializedObjectRow::from_state)
            .map(ExecutionRow::Object)
            .collect::<Vec<_>>(),
        ResolvedRoot::Association(root) => associations
            .iter()
            .filter(|association| association.object_type_id == root.object_type_id)
            .filter(|association| {
                association_row_matches_temporal(association, root, association_valid_at(planned))
            })
            .cloned()
            .map(ExecutionRow::Association)
            .collect::<Vec<_>>(),
        ResolvedRoot::Node(root) => states
            .iter()
            .filter(|state| state.object_type_id == root.object.object_type_id)
            .map(MaterializedObjectRow::from_state)
            .map(ExecutionRow::Object)
            .collect::<Vec<_>>(),
        ResolvedRoot::Edge(root) => associations
            .iter()
            .filter(|association| association.object_type_id == root.association.object_type_id)
            .filter(|association| {
                association_row_matches_temporal(
                    association,
                    &root.association,
                    association_valid_at(planned),
                )
            })
            .cloned()
            .map(ExecutionRow::Association)
            .collect::<Vec<_>>(),
        ResolvedRoot::Evidence(_) => evidence_execution_rows,
        ResolvedRoot::Table(_) => {
            unreachable!("table roots are handled before object kernel readback")
        }
        ResolvedRoot::Projection(_) => {
            unreachable!("projection roots are handled before object kernel readback")
        }
    };
    let evidence_helper_runtime_exact =
        native_evidence_exists_direct_projection_candidate_shape(planned)
            && evidence_report.existence_fast_path_exact;
    let helper_prefilter =
        apply_kernel_helper_prefilters(&mut rows, &associations, &evidence_rows, planned);
    let helper_prefilter_exact =
        native_helper_exists_direct_projection_shape(planned) || evidence_helper_runtime_exact;
    if helper_prefilter.applied_terms() > 0 {
        let helper_message = if helper_prefilter_exact
            && helper_prefilter.evidence_semi_join_terms > 0
        {
            "native evidence existence semi-join filtered reconstructed rows with exact authority"
        } else if helper_prefilter_exact {
            "native association existence semi-join filtered reconstructed rows with exact authority"
        } else {
            "native association/evidence existence prefilters narrowed reconstructed rows before materialized residual verification"
        };
        diagnostics.push(exec_warning(
            "W_KERNEL_HELPER_PREFILTER_EXECUTED",
            helper_message,
            json!({
                "input_rows": helper_prefilter.input_rows,
                "output_rows": helper_prefilter.output_rows,
                "association_semi_join_terms": helper_prefilter.association_semi_join_terms,
                "association_anti_join_terms": helper_prefilter.association_anti_join_terms,
                "evidence_semi_join_terms": helper_prefilter.evidence_semi_join_terms,
                "evidence_anti_join_terms": helper_prefilter.evidence_anti_join_terms,
                "evidence_fast_path_exact": evidence_helper_runtime_exact,
                "residual_verification": !helper_prefilter_exact,
            }),
        ));
    }
    if helper_prefilter.skipped_terms() > 0 {
        diagnostics.push(exec_warning(
            "W_KERNEL_HELPER_PREFILTER_SKIPPED",
            "association/evidence helper predicates remained materialized residuals because they lacked a native prefilter proof",
            json!({
                "skipped_complex_terms": helper_prefilter.skipped_complex_terms,
                "skipped_security_terms": helper_prefilter.skipped_security_terms,
                "residual_verification": true,
            }),
        ));
    }
    let native_root_scan = try_native_root_scan_result(&rows, planned, options, started)?;
    let native_direct_projection = if native_root_scan.is_none() {
        try_native_direct_projection_result(
            &rows,
            &associations,
            &evidence_rows,
            planned,
            options,
            started,
            helper_prefilter_exact,
        )?
    } else {
        None
    };
    let native_bool_group_count =
        if native_root_scan.is_none() && native_direct_projection.is_none() {
            try_native_bool_group_count_result(&rows, planned, options, started)?
        } else {
            None
        };
    let native_grouped_helper_aggregate = if native_root_scan.is_none()
        && native_direct_projection.is_none()
        && native_bool_group_count.is_none()
    {
        try_native_grouped_helper_aggregate_result(
            &rows,
            &associations,
            &evidence_rows,
            planned,
            options,
            started,
        )?
    } else {
        None
    };
    let native_helper_aggregate = if native_root_scan.is_none()
        && native_direct_projection.is_none()
        && native_bool_group_count.is_none()
        && native_grouped_helper_aggregate.is_none()
    {
        try_native_helper_aggregate_result(
            &rows,
            &associations,
            &evidence_rows,
            planned,
            options,
            started,
        )?
    } else {
        None
    };
    let native_direct_aggregate = if native_root_scan.is_none()
        && native_direct_projection.is_none()
        && native_bool_group_count.is_none()
        && native_grouped_helper_aggregate.is_none()
        && native_helper_aggregate.is_none()
    {
        try_native_direct_aggregate_result(&rows, planned, options, started)?
    } else {
        None
    };
    let native_typed_order = if native_root_scan.is_none()
        && native_direct_projection.is_none()
        && native_bool_group_count.is_none()
        && native_grouped_helper_aggregate.is_none()
        && native_helper_aggregate.is_none()
        && native_direct_aggregate.is_none()
    {
        try_native_typed_order_result(&rows, planned, options, started)?
    } else {
        None
    };
    let (
        result,
        row_counts,
        native_bool_group_count_report,
        native_grouped_helper_aggregate_report,
        native_helper_aggregate_report,
        native_direct_aggregate_report,
        native_typed_order_report,
        native_root_scan_report,
        native_direct_projection_report,
    ) = if let Some((result, row_counts, report)) = native_root_scan {
        let diagnostic_code = match report.root_kind {
            "association" => "W_KERNEL_NATIVE_ASSOCIATION_ROOT_SCAN_EXECUTED",
            "evidence" => "W_KERNEL_NATIVE_EVIDENCE_ROOT_SCAN_EXECUTED",
            _ => "W_KERNEL_NATIVE_ROOT_SCAN_EXECUTED",
        };
        diagnostics.push(exec_warning(
            diagnostic_code,
            "native direct root scan produced exact row output",
            json!({
                "root_kind": report.root_kind,
                "root_type": report.root_type,
                "rows_returned": report.rows_returned,
                "residual_verification": false,
            }),
        ));
        (
            result,
            row_counts,
            None,
            None,
            None,
            None,
            None,
            Some(report),
            None,
        )
    } else if let Some((result, row_counts, report)) = native_direct_projection {
        diagnostics.push(exec_warning(
            "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED",
            "native direct projection produced exact selected output",
            json!({
                "root_kind": report.root_kind,
                "column_count": report.column_count,
                "rows_projected": report.rows_projected,
                "residual_verification": false,
            }),
        ));
        (
            result,
            row_counts,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(report),
        )
    } else if let Some((result, row_counts, report)) = native_bool_group_count {
        let (diagnostic_code, diagnostic_message) = if report.aggregate == "count"
            && matches!(report.group_logical_type.as_str(), "bool" | "boolean")
        {
            (
                "W_KERNEL_NATIVE_BOOL_GROUP_COUNT_EXECUTED",
                "native bool group/count kernel produced exact grouped aggregate output",
            )
        } else {
            (
                "W_KERNEL_NATIVE_DIRECT_GROUP_AGGREGATE_EXECUTED",
                "native direct grouped aggregate kernel produced exact grouped aggregate output",
            )
        };
        diagnostics.push(exec_warning(
            diagnostic_code,
            diagnostic_message,
            json!({
                "aggregate": report.aggregate,
                "group_property": report.group_property,
                "logical_type": report.group_logical_type,
                "group_count": report.group_count,
                "rows_counted": report.rows_counted,
                "values_seen": report.values_seen,
                "group_strategy": report.group_strategy,
                "aggregate_strategy": report.aggregate_strategy,
            }),
        ));
        (
            result,
            row_counts,
            Some(report),
            None,
            None,
            None,
            None,
            None,
            None,
        )
    } else if let Some((result, row_counts, report)) = native_grouped_helper_aggregate {
        diagnostics.push(exec_warning(
            "W_KERNEL_NATIVE_GROUPED_HELPER_AGGREGATE_EXECUTED",
            "native grouped association/evidence helper aggregate kernel produced exact grouped aggregate output",
            json!({
                "aggregate": report.aggregate,
                "helper_kind": report.helper_kind,
                "group_property": report.group_property,
                "logical_type": report.group_logical_type,
                "group_count": report.group_count,
                "rows_counted": report.rows_counted,
                "helper_values_seen": report.helper_values_seen,
            }),
        ));
        (
            result,
            row_counts,
            None,
            Some(report),
            None,
            None,
            None,
            None,
            None,
        )
    } else if let Some((result, row_counts, report)) = native_helper_aggregate {
        diagnostics.push(exec_warning(
            "W_KERNEL_NATIVE_HELPER_AGGREGATE_EXECUTED",
            "native association/evidence helper aggregate kernel produced exact aggregate output",
            json!({
                "aggregate": report.aggregate,
                "helper_kind": report.helper_kind,
                "rows_counted": report.rows_counted,
                "helper_values_seen": report.helper_values_seen,
            }),
        ));
        (
            result,
            row_counts,
            None,
            None,
            Some(report),
            None,
            None,
            None,
            None,
        )
    } else if let Some((result, row_counts, report)) = native_direct_aggregate {
        diagnostics.push(exec_warning(
            "W_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED",
            "native direct aggregate kernel produced exact aggregate output",
            json!({
                "aggregate": report.aggregate,
                "counted_path": report.counted_path,
                "rows_counted": report.rows_counted,
                "values_seen": report.values_seen,
                "aggregate_strategy": report.aggregate_strategy,
            }),
        ));
        (
            result,
            row_counts,
            None,
            None,
            None,
            Some(report),
            None,
            None,
            None,
        )
    } else if let Some((result, row_counts, report)) = native_typed_order {
        diagnostics.push(exec_warning(
            "W_KERNEL_NATIVE_TYPED_ORDER_EXECUTED",
            "native typed order kernel produced exact sorted projection output",
            json!({
                "order_property": report.order_property,
                "logical_type": report.logical_type,
                "collation_id": report.collation_id,
                "sort_key_boundary": report.sort_key_boundary,
                "rows_sorted": report.rows_sorted,
                "output_rows": report.output_rows,
            }),
        ));
        (
            result,
            row_counts,
            None,
            None,
            None,
            None,
            Some(report),
            None,
            None,
        )
    } else {
        let (result, row_counts) = crate::execution::finish_materialized_rows(
            rows,
            &associations,
            &evidence_rows,
            &[],
            planned,
            options,
            started,
        )?;
        (result, row_counts, None, None, None, None, None, None, None)
    };
    enforce_result_budgets(&result, &row_counts, planned, options, started)?;
    let output_fingerprint = result_fingerprint(&result)?;
    let residual_verification_required = native_root_scan_report.is_none()
        && native_direct_projection_report.is_none()
        && native_bool_group_count_report.is_none()
        && native_grouped_helper_aggregate_report.is_none()
        && native_helper_aggregate_report.is_none()
        && native_direct_aggregate_report.is_none()
        && native_typed_order_report.is_none();

    let mut counters = KernelCounters {
        rows_scanned: read.kernel_surface.system.len(),
        rows_after_bitmap: selection.count_ones(),
        rows_after_selection_vector: selection_vector.len(),
        candidate_object_keys,
        retained_record_chain_rows,
        reconstructed_states: states.len(),
        residual_rows_checked: if residual_verification_required {
            row_counts.input_rows
        } else {
            0
        },
        output_rows: row_counts.output_rows,
        bitmap_words: selection.word_count(),
        selection_vector_len: selection_vector.len(),
        coded_predicate_rows: covi_prune.as_ref().map_or(0, |prune| prune.matched_rows)
            + execution_code_prune
                .as_ref()
                .map_or(0, |prune| prune.matched_rows)
            + file_code_prune
                .as_ref()
                .map_or(0, |prune| prune.matched_rows)
            + native_scalar_prune
                .as_ref()
                .map_or(0, |prune| prune.matched_rows),
        typed_predicate_rows: shape.predicates.len() * read.kernel_surface.system.len(),
        bytes_touched_estimate: read.kernel_surface.system.len() * std::mem::size_of::<u64>() * 5
            + selection_compaction_stats.bytes_touched_estimate,
        scratch_high_water_bytes: selection.word_count() * std::mem::size_of::<u64>()
            + selection_vector.len() * std::mem::size_of::<u32>(),
        ..KernelCounters::default()
    };
    if counters.scratch_high_water_bytes > kernel_options.scratch_budget_bytes {
        return Err(crate::execution::resource_error(
            "kernel_scratch_budget_bytes",
            counters.scratch_high_water_bytes,
        ));
    }
    counters.dictionary_lookups_at_materialization = row_counts.output_rows;

    let runtime_kernel_shape = runtime_kernel_shape_for_helper_prefilter_proof(
        &shape.public,
        planned,
        evidence_helper_runtime_exact,
    );
    let mut kernel_report = KernelExecutionReport::applied(kernel_options.mode, counters);
    kernel_report.decision.safe_details = json!({
        "kernel_shape": runtime_kernel_shape,
        "kernel_surface_source": "temporal_segment_direct",
        "materialized_surface_role": "reconstruction_oracle_and_fallback",
        "residual_verification": residual_verification_required,
    });
    if let Some(report) = &native_root_scan_report {
        kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
            "native direct association/evidence root scan produced exact row output after reconstructed state/evidence-grain proof",
        );
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native direct association/evidence root scan executed without materialized residuals",
            json!({
                "root_kind": report.root_kind,
                "root_type": report.root_type,
                "rows_returned": report.rows_returned,
                "residual_verification": false,
                "row_grain": if report.root_kind == "association" { "reconstructed_visible_association_states" } else { "visible_evidence_rows" },
            }),
            false,
        ));
    }
    if let Some(report) = &native_direct_projection_report {
        kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
            "native direct projection produced exact output after reconstructed row-grain proof",
        );
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native direct projection executed without materialized expression residuals",
            json!({
                "root_kind": report.root_kind,
                "column_count": report.column_count,
                "rows_projected": report.rows_projected,
                "residual_verification": false,
                "row_grain": "reconstructed_visible_rows",
            }),
            false,
        ));
    }
    if let Some(report) = &native_bool_group_count_report {
        kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
            "native direct grouped aggregate kernel produced exact output after reconstructed state-grain proof",
        );
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native direct grouped aggregate kernel executed without materialized aggregate residuals",
            json!({
                "aggregate": report.aggregate,
                "group_property": report.group_property,
                "logical_type": report.group_logical_type,
                "group_count": report.group_count,
                "rows_counted": report.rows_counted,
                "values_seen": report.values_seen,
                "group_strategy": report.group_strategy,
                "aggregate_strategy": report.aggregate_strategy,
                "residual_verification": false,
                "row_grain": "reconstructed_visible_object_states",
            }),
            false,
        ));
    }
    if let Some(report) = &native_grouped_helper_aggregate_report {
        kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
            "native grouped association/evidence helper aggregate kernel produced exact output after reconstructed state-grain proof",
        );
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native grouped association/evidence helper aggregate kernel executed without materialized aggregate residuals",
            json!({
                "aggregate": report.aggregate,
                "helper_kind": report.helper_kind,
                "group_property": report.group_property,
                "logical_type": report.group_logical_type,
                "group_count": report.group_count,
                "rows_counted": report.rows_counted,
                "helper_values_seen": report.helper_values_seen,
                "residual_verification": false,
                "row_grain": "groups_over_reconstructed_visible_object_states",
            }),
            false,
        ));
    }
    if let Some(report) = &native_helper_aggregate_report {
        kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
            "native association/evidence helper aggregate kernel produced exact output after reconstructed state-grain proof",
        );
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native association/evidence helper aggregate kernel executed without materialized aggregate residuals",
            json!({
                "aggregate": report.aggregate,
                "helper_kind": report.helper_kind,
                "rows_counted": report.rows_counted,
                "helper_values_seen": report.helper_values_seen,
                "residual_verification": false,
                "row_grain": "reconstructed_visible_object_states",
            }),
            false,
        ));
    }
    if let Some(report) = &native_direct_aggregate_report {
        kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
            "native direct aggregate kernel produced exact output after reconstructed state-grain proof",
        );
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native direct aggregate kernel executed without materialized aggregate residuals",
            json!({
                "aggregate": report.aggregate,
                "counted_path": report.counted_path,
                "rows_counted": report.rows_counted,
                "values_seen": report.values_seen,
                "aggregate_strategy": report.aggregate_strategy,
                "residual_verification": false,
                "row_grain": "reconstructed_visible_object_states",
            }),
            false,
        ));
    }
    if let Some(report) = &native_typed_order_report {
        kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
            "native typed order kernel produced exact output after typed value-order proof",
        );
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native typed order kernel executed without materialized sort residuals",
            json!({
                "order_property": report.order_property,
                "logical_type": report.logical_type,
                "collation_id": report.collation_id,
                "sort_key_boundary": report.sort_key_boundary,
                "rows_sorted": report.rows_sorted,
                "output_rows": report.output_rows,
                "residual_verification": false,
                "row_grain": "reconstructed_visible_object_states",
            }),
            false,
        ));
    }
    if let Some(prune) = &covi_prune {
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "validated exact COVI/COVX row ranges were used as a kernel prefilter",
            json!({
                "source": prune.source,
                "lookup_count": prune.lookup_count,
                "row_range_count": prune.row_range_count,
                "matched_rows": prune.matched_rows,
                "residual_verification": true,
            }),
            false,
        ));
    }
    if let Some(prune) = &coverage_prune {
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "validated COVE-COVERAGE row ranges were used as a kernel prefilter",
            json!({
                "predicate_form_ref": prune.predicate_form_ref,
                "row_range_count": prune.row_range_count,
                "matched_rows": prune.matched_rows,
                "residual_verification": true,
            }),
            false,
        ));
    }
    if let Some(prune) = &execution_code_prune {
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "validated COVE-E execution-code remap was used as a kernel prefilter",
            json!({
                "predicate_count": prune.predicate_count,
                "page_count": prune.page_count,
                "matched_rows": prune.matched_rows,
                "residual_verification": !execution_code_predicates_exact,
            }),
            false,
        ));
    }
    if let Some(prune) = &file_code_prune {
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "single-file FileCode literal predicate was used as a kernel prefilter",
            json!({
                "predicate_count": prune.predicate_count,
                "page_count": prune.page_count,
                "matched_rows": prune.matched_rows,
                "dataset_file_count": planned.resolved.operation_context.dataset.files.len(),
                "residual_verification": !file_code_predicates_exact,
                "bridge_required": false,
                "row_grain": "single_file_object_records",
            }),
            false,
        ));
    }
    if let Some(prune) = &native_scalar_prune {
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            "native scalar page-lane predicate was used as a kernel prefilter",
            json!({
                "predicate_count": prune.predicate_count,
                "executed_predicate_count": prune.executed_predicate_count,
                "page_count": prune.page_count,
                "matched_rows": prune.matched_rows,
                "predicate_order": prune.predicate_order.clone(),
                "short_circuited": prune.short_circuited,
                "residual_verification": !native_scalar_predicates_exact,
                "predicate_kernels": prune.predicate_dispatch.kernels,
                "predicate_kernel_scalar": prune.predicate_dispatch.scalar,
                "predicate_kernel_avx2": prune.predicate_dispatch.avx2,
                "predicate_kernel_neon": prune.predicate_dispatch.neon,
                "retained_page_buffers": prune.retained_page_buffers,
                "row_grain": "single_file_object_records",
            }),
            false,
        ));
    }
    if helper_prefilter.applied_terms() > 0 {
        let helper_decision_reason = if helper_prefilter_exact
            && helper_prefilter.evidence_semi_join_terms > 0
        {
            "native evidence existence semi-join executed with exact authority"
        } else if helper_prefilter_exact {
            "native association existence semi-join executed with exact authority"
        } else {
            "native association/evidence existence prefilters executed with materialized residual verification"
        };
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Applied,
            helper_decision_reason,
            json!({
                "input_rows": helper_prefilter.input_rows,
                "output_rows": helper_prefilter.output_rows,
                "association_semi_join_terms": helper_prefilter.association_semi_join_terms,
                "association_anti_join_terms": helper_prefilter.association_anti_join_terms,
                "evidence_semi_join_terms": helper_prefilter.evidence_semi_join_terms,
                "evidence_anti_join_terms": helper_prefilter.evidence_anti_join_terms,
                "evidence_fast_path_exact": evidence_helper_runtime_exact,
                "residual_verification": !helper_prefilter_exact,
                "row_grain": "reconstructed_visible_object_states",
            }),
            false,
        ));
    }
    if helper_prefilter.skipped_terms() > 0 {
        kernel_report.decisions.push(KernelDecision::new(
            KernelDecisionKind::Rejected,
            "association/evidence helper prefilter remained a materialized residual boundary",
            json!({
                "skipped_complex_terms": helper_prefilter.skipped_complex_terms,
                "skipped_security_terms": helper_prefilter.skipped_security_terms,
                "residual_verification": true,
            }),
            false,
        ));
    }
    kernel_report.association =
        crate::AssociationOptimizationReport::for_plan(planned, &associations);
    kernel_report.evidence = evidence_report;
    kernel_report.lineage = crate::LineageReuseReport::for_plan(planned);
    let executed = ExecutedQuery {
        planned: planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown_plan.report,
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_kernel(
            if native_bool_group_count_report.is_some() {
                "native direct grouped aggregate kernel produced exact grouped aggregate output"
            } else if native_root_scan_report.is_some() {
                "native direct association/evidence root scan produced exact row output"
            } else if native_direct_projection_report.is_some() {
                "native direct projection produced exact selected output"
            } else if native_grouped_helper_aggregate_report.is_some() {
                "native grouped association/evidence helper aggregate kernel produced exact grouped aggregate output"
            } else if native_helper_aggregate_report.is_some() {
                "native association/evidence helper aggregate kernel produced exact aggregate output"
            } else if native_direct_aggregate_report.is_some() {
                "native direct aggregate kernel produced exact aggregate output"
            } else if native_typed_order_report.is_some() {
                "native typed order kernel produced exact sorted projection output"
            } else {
                "kernel path produced output after guarded predicate selection"
            },
            native_root_scan_report.is_none()
                && native_direct_projection_report.is_none()
                && native_bool_group_count_report.is_none()
                && native_grouped_helper_aggregate_report.is_none()
                && native_helper_aggregate_report.is_none()
                && native_direct_aggregate_report.is_none()
                && native_typed_order_report.is_none()
                && shape.public.residual_verification_required,
        ),
    };
    Ok((executed, kernel_report))
}
