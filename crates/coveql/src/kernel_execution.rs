use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use cove_core::{
    array::{CoveArrayValue, EncodedArray},
    artifact::coveai::{
        ai_embedding, ai_vector_search, AiEmbeddingRequest, AiPayloadReader,
        AiVectorIndexSelection, AiVectorSearchPlan, AiVectorSearchResult, AiVectorSearchTargetKind,
        CoveAiAccessContext, CoveAiFile,
    },
    canonical::CanonicalValue,
    collation::CollationKind,
    compression,
    constants::{CoveLogicalType, CovePhysicalKind, SectionKind},
    mount::{
        mount_cove_file, ExecutionCodeRequest, ExecutionCodeResolver, ExecutionCodeValue,
        MountOptions, OutputRepresentation,
    },
    native::{
        compact_selection_bitmap, filter_bool_eq, filter_fixed_bytes_eq, filter_fixed_bytes_in,
        filter_local_u16_membership, filter_local_u32_membership, filter_local_u8_membership,
        filter_numcode_le_in_typed, filter_numcode_le_not_in_typed, filter_numcode_le_typed,
        filter_u32_le_in_sorted, filter_u32_le_not_in_sorted, filter_validity, filter_varbytes_eq,
        filter_varbytes_in, filter_varbytes_prefix, local_membership_u8, native_numcode_matches,
        native_object_temporal_batch_from_retained_segment,
        native_object_temporal_batch_from_segment, valid_rows_except, KernelStats, LaneRef,
        NativeCodeDomain, NativeKernelDispatch, NativeNumericLiteral, NativeNumericPredicateOp,
        NativeObjectTemporalBatch, ValidityRef,
    },
    page_payload::PageBufferKind,
    profile::cove_o::{
        read_object_kernel_surface_from_bytes_with_options,
        read_object_surface_from_bytes_with_options, read_retained_object_temporal_segments,
        reconstruct_object_states, CoveObjectKernelReadOptions, CoveObjectKernelSurface,
        CoveObjectReadOptions, CoveObjectReadWithPushdownOptions, CoveObjectReconstructionOptions,
        CoveObjectState, RetainedTemporalSegmentData, TemporalSegmentData,
    },
    reader::ValidationOptions,
    validity::ValidityBitmap,
    wire, CoveError,
};
use cove_coverage::{
    can_use_for_pruning, can_use_proof_for_pruning, CoverageGranularityV2, CoverageProofRecordV2,
    CoverageSetEntryV2, CoverageSetV2,
};
use cove_index::execution::{
    CoviAggregateKindV2, CoviCandidateSetV2, CoviIndexOnlyAnswerV2, CoviIndexOnlyRequestV2,
    CoviLookupKeyV2, CoviLookupRequestV2, CoviLookupTargetV2, CoviValidationContextV2,
    ValidatedCoviArtifactV2,
};
use cove_index::{CoviRowRangePostingV2, IndexCapabilityExactnessV2};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::CoveQlRetainedInput;
use crate::{
    association_opt::{
        association_row_matches_temporal, association_valid_at, current_row_dataset_file_ordinal,
        current_row_goid, AssociationEdgeTable,
    },
    evidence_opt::{materialized_evidence_rows_for_plan, EvidenceGrainIndex},
    execution::{
        apply_skip_take, check_time, combined_evidence_authority, enforce_result_budgets,
        evidence_object_rows_from_states, exec_error, exec_warning, execute_manifest_planned_query,
        execute_planned_query, finish_json_rows, finish_materialized_rows, manifest_member_plan,
        object_read_options, projection_root_execution_rows, reconstruction_options,
        resource_error, result_fingerprint, role_bound_states_at, sort_rows,
        validate_execution_grain, validate_execution_output_mode,
        validate_manifest_execution_members, validate_security_scope,
        zero_copy_owned_fallback_warning,
    },
    expr_eval::{
        eval_expr, literal_value, logical_value_key, parse_decimal_value, stable_value_key,
        value_ordering_typed, values_equal_typed, EvalContext, ExactDecimal,
    },
    kernel_metrics::{
        KernelCounters, KernelDecision, KernelDecisionKind, KernelExecutionMode,
        KernelExecutionOptions, KernelExecutionReport, KernelFallbackReason,
        OptimizationAuthorityReport,
    },
    kernel_plan::{
        aggregate_operator_name, compile_kernel_shape, diagnostic_kernel_shape_for_plan,
        native_association_root_scan_shape, native_bool_group_count_path,
        native_bool_group_count_shape, native_direct_aggregate_name, native_direct_aggregate_path,
        native_direct_aggregate_shape, native_direct_projection_shape,
        native_evidence_exists_direct_projection_candidate_shape, native_evidence_root_scan_shape,
        native_grouped_helper_aggregate_shape, native_helper_aggregate_shape,
        native_helper_exists_direct_projection_shape, native_projection_expr_is_exact,
        native_projection_root_scan_shape, native_role_bound_direct_projection_shape,
        native_temporal_direct_projection_shape, native_typed_order_path, native_typed_order_shape,
        row_root_predicate_is_direct_safe, CodedRepresentationClass, KernelShape,
    },
    kernel_predicate::{select_rows_with_base, KernelLiteral, KernelPredicate, SelectionBitmap},
    kernel_reconstruct::reconstruct_selected_object_states,
    materialized::{
        hex, ExecutionRow, MaterializedAssociationRow, MaterializedEvidenceRow,
        MaterializedObjectRow,
    },
    parse_resolve_plan_and_build_physical_plan, pushdown, AggregateDisclosurePolicy,
    AstAggregateName, AstCompareOp, AstNullOrdering, AstOrderDirection, BuildExecutionError,
    CoveQlAiOperation, CoveQlExecutionResult, ExecutedQuery, ExecutionAuthorityReport,
    ExecutionDiagnostic, ExecutionOptions, ExecutionRowCounts, ManifestDatasetMember,
    MetadataDisclosurePolicy, ParseOptions, PhysicalPlanOptions, PhysicalPlannedQuery, PlanOptions,
    ResolveOptions, ResolvedAiArgumentValue, ResolvedExpr, ResolvedLiteral, ResolvedLiteralValue,
    ResolvedPath, ResolvedPredicate, ResolvedRoot, ResolvedSystemField, TemporalMode,
    VisibilityPolicy,
};

#[derive(Debug, Clone)]
pub struct KernelExecutedQuery {
    pub physical: PhysicalPlannedQuery,
    pub executed: ExecutedQuery,
    pub kernel_report: KernelExecutionReport,
}

impl KernelExecutedQuery {
    pub fn explain_json(&self) -> Value {
        let mut value = self.executed.explain_json();
        let allow_protected = self
            .executed
            .planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            == MetadataDisclosurePolicy::AllowProtected;
        if let Some(execution) = value.get_mut("execution").and_then(Value::as_object_mut) {
            execution.insert(
                "kernel_report".into(),
                self.kernel_report.to_json(allow_protected),
            );
        }
        value
    }

    pub fn explain_text(&self) -> String {
        crate::render_explain_text(&self.explain_json())
    }
}

pub fn parse_resolve_plan_build_physical_and_execute_query(
    bytes: &[u8],
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    plan_options: PlanOptions,
    physical_options: PhysicalPlanOptions,
    execution_options: ExecutionOptions,
    kernel_options: KernelExecutionOptions,
    validation_options: ValidationOptions,
) -> Result<KernelExecutedQuery, BuildExecutionError> {
    let physical = parse_resolve_plan_and_build_physical_plan(
        bytes,
        text,
        parse_options,
        resolve_options,
        plan_options,
        physical_options,
        validation_options,
    )
    .map_err(|error| BuildExecutionError {
        diagnostics: error
            .diagnostics
            .into_iter()
            .map(|diagnostic| ExecutionDiagnostic {
                code: diagnostic.code,
                severity: diagnostic.severity,
                message: diagnostic.message,
                phase: diagnostic.phase,
                safe_details: diagnostic.safe_details,
                redacted: diagnostic.redacted,
            })
            .collect(),
        source: error.source,
    })?;
    execute_physical_planned_query(bytes, physical, execution_options, kernel_options)
}

pub fn parse_resolve_plan_build_physical_and_execute_query_retained(
    input: CoveQlRetainedInput,
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    plan_options: PlanOptions,
    physical_options: PhysicalPlanOptions,
    execution_options: ExecutionOptions,
    kernel_options: KernelExecutionOptions,
    validation_options: ValidationOptions,
) -> Result<KernelExecutedQuery, BuildExecutionError> {
    let physical = parse_resolve_plan_and_build_physical_plan(
        input.as_slice(),
        text,
        parse_options,
        resolve_options,
        plan_options,
        physical_options,
        validation_options,
    )
    .map_err(|error| BuildExecutionError {
        diagnostics: error
            .diagnostics
            .into_iter()
            .map(|diagnostic| ExecutionDiagnostic {
                code: diagnostic.code,
                severity: diagnostic.severity,
                message: diagnostic.message,
                phase: diagnostic.phase,
                safe_details: diagnostic.safe_details,
                redacted: diagnostic.redacted,
            })
            .collect(),
        source: error.source,
    })?;
    execute_physical_planned_query_retained(input, physical, execution_options, kernel_options)
}

pub fn execute_physical_planned_query(
    bytes: &[u8],
    physical: PhysicalPlannedQuery,
    execution_options: ExecutionOptions,
    kernel_options: KernelExecutionOptions,
) -> Result<KernelExecutedQuery, BuildExecutionError> {
    execute_physical_planned_query_inner(bytes, None, physical, execution_options, kernel_options)
}

fn execute_physical_planned_query_inner(
    bytes: &[u8],
    retained_input: Option<&CoveQlRetainedInput>,
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

    if let Some((executed, kernel_report)) =
        try_ai_semantic_search_executed_query(&physical, &execution_options, kernel_options.mode)?
    {
        return Ok(KernelExecutedQuery {
            physical,
            executed,
            kernel_report,
        });
    }
    if let Some((executed, kernel_report)) =
        try_ai_embedding_executed_query(&physical, &execution_options, kernel_options.mode)?
    {
        return Ok(KernelExecutedQuery {
            physical,
            executed,
            kernel_report,
        });
    }
    if let Some((executed, kernel_report)) = try_ai_descriptor_metadata_executed_query(
        bytes,
        &physical,
        &execution_options,
        kernel_options.mode,
    )? {
        return Ok(KernelExecutedQuery {
            physical,
            executed,
            kernel_report,
        });
    }

    match kernel_options.mode {
        KernelExecutionMode::ForceMaterialized => {
            let executed =
                execute_planned_query(bytes, physical.planned.clone(), execution_options)?;
            let mut kernel_report = KernelExecutionReport::disabled(
                KernelExecutionMode::ForceMaterialized,
                "kernel execution disabled by ForceMaterialized mode",
            );
            attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
            Ok(KernelExecutedQuery {
                physical,
                executed,
                kernel_report,
            })
        }
        KernelExecutionMode::Auto | KernelExecutionMode::CompareWithMaterialized => {
            if let Some(mut executed) =
                try_covi_empty_lookup_executed_query(&physical, &execution_options)?
            {
                let mut kernel_report = KernelExecutionReport::disabled(
                    kernel_options.mode,
                    "validated COVI/COVX empty lookup executed before object kernel selection",
                );
                kernel_report.optimization_authority =
                    OptimizationAuthorityReport::authoritative(
                        "exact COVI/COVX empty lookup proved no visible rows without materialized comparison",
                    );
                attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
                if kernel_options.mode == KernelExecutionMode::CompareWithMaterialized {
                    let materialized =
                        execute_planned_query(bytes, physical.planned.clone(), execution_options)?;
                    kernel_report.compared_with_materialized = true;
                    kernel_report.materialized_fingerprint =
                        Some(materialized.output_fingerprint.clone());
                    kernel_report.kernel_fingerprint = Some(executed.output_fingerprint.clone());
                    kernel_report.decisions.push(KernelDecision::new(
                        KernelDecisionKind::Compared,
                        "empty COVI/COVX lookup output fingerprint was compared with the materialized baseline",
                        json!({}),
                        false,
                    ));
                    if materialized.output_fingerprint != executed.output_fingerprint {
                        return Err(exec_error(
                            "E_COVI_EMPTY_LOOKUP_MISMATCH",
                            "empty COVI/COVX lookup output fingerprint does not match materialized baseline",
                            json!({}),
                        ));
                    }
                    executed.authority.compared_with_materialized = true;
                    executed.diagnostics.push(exec_warning(
                        "W_COVI_EMPTY_LOOKUP_COMPARE_MATCHED",
                        "empty COVI/COVX lookup output matched the materialized baseline fingerprint",
                        json!({}),
                    ));
                }
                return Ok(KernelExecutedQuery {
                    physical,
                    executed,
                    kernel_report,
                });
            }
            if let Some(mut executed) =
                try_index_only_executed_query(&physical, &execution_options)?
            {
                let mut kernel_report = KernelExecutionReport::disabled(
                    kernel_options.mode,
                    "validated index-only aggregate answer executed before object kernel selection",
                );
                kernel_report.optimization_authority =
                    OptimizationAuthorityReport::authoritative(
                        "exact COVI/COVX index-only aggregate answer produced output without materialized comparison",
                    );
                attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
                if kernel_options.mode == KernelExecutionMode::CompareWithMaterialized {
                    let materialized =
                        execute_planned_query(bytes, physical.planned.clone(), execution_options)?;
                    kernel_report.compared_with_materialized = true;
                    kernel_report.materialized_fingerprint =
                        Some(materialized.output_fingerprint.clone());
                    kernel_report.kernel_fingerprint = Some(executed.output_fingerprint.clone());
                    kernel_report.decisions.push(KernelDecision::new(
                        KernelDecisionKind::Compared,
                        "index-only output fingerprint was compared with the materialized baseline",
                        json!({}),
                        false,
                    ));
                    if materialized.output_fingerprint != executed.output_fingerprint {
                        return Err(exec_error(
                            "E_INDEX_ONLY_MISMATCH",
                            "index-only output fingerprint does not match materialized baseline",
                            json!({}),
                        ));
                    }
                    executed.authority.compared_with_materialized = true;
                    executed.diagnostics.push(exec_warning(
                        "W_INDEX_ONLY_COMPARE_MATCHED",
                        "index-only output matched the materialized baseline fingerprint",
                        json!({}),
                    ));
                }
                return Ok(KernelExecutedQuery {
                    physical,
                    executed,
                    kernel_report,
                });
            }
            let shape = match compile_kernel_shape(&physical) {
                Ok(shape) => shape,
                Err(reason) => {
                    let executed =
                        execute_planned_query(bytes, physical.planned.clone(), execution_options)?;
                    let mut kernel_report = KernelExecutionReport::fallback(
                        kernel_options.mode,
                        reason,
                        "no eligible Phase 7 kernel shape; materialized baseline executed",
                    );
                    kernel_report.decision.safe_details = json!({
                        "reason": format!("{reason:?}"),
                        "kernel_shape": diagnostic_kernel_shape_for_plan(&physical.planned, reason),
                        "residual_verification": true,
                        "fallback_boundary": "materialized",
                    });
                    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
                    return Ok(KernelExecutedQuery {
                        physical,
                        executed,
                        kernel_report,
                    });
                }
            };
            let (mut executed, mut kernel_report) = execute_kernel_object(
                bytes,
                retained_input,
                &physical,
                &execution_options,
                &kernel_options,
                shape,
            )?;
            if kernel_options.mode == KernelExecutionMode::CompareWithMaterialized {
                let materialized =
                    execute_planned_query(bytes, physical.planned.clone(), execution_options)?;
                kernel_report.compared_with_materialized = true;
                kernel_report.materialized_fingerprint =
                    Some(materialized.output_fingerprint.clone());
                kernel_report.kernel_fingerprint = Some(executed.output_fingerprint.clone());
                kernel_report.decisions.push(KernelDecision::new(
                    KernelDecisionKind::Compared,
                    "kernel output fingerprint was compared with the materialized baseline",
                    json!({}),
                    false,
                ));
                if materialized.output_fingerprint != executed.output_fingerprint {
                    return Err(exec_error(
                        "E_KERNEL_MISMATCH",
                        "kernel output fingerprint does not match materialized baseline",
                        json!({}),
                    ));
                }
                executed.authority.compared_with_materialized = true;
                executed.diagnostics.push(exec_warning(
                    "W_KERNEL_COMPARE_MATCHED",
                    "Phase 7 kernel output matched the materialized baseline fingerprint",
                    json!({}),
                ));
            }
            Ok(KernelExecutedQuery {
                physical,
                executed,
                kernel_report,
            })
        }
        KernelExecutionMode::ForceKernel => {
            let shape = compile_kernel_shape(&physical).map_err(|reason| {
                let kernel_shape = diagnostic_kernel_shape_for_plan(&physical.planned, reason);
                exec_error(
                    kernel_unsupported_diagnostic_code(
                        reason,
                        &kernel_shape,
                        "E_KERNEL_UNSUPPORTED",
                    ),
                    "ForceKernel mode rejected a query without an eligible Phase 7 kernel shape",
                    json!({
                        "reason": format!("{reason:?}"),
                        "kernel_shape": kernel_shape,
                        "residual_verification": true,
                        "fallback_boundary": "rejected",
                    }),
                )
            })?;
            let (executed, kernel_report) = execute_kernel_object(
                bytes,
                retained_input,
                &physical,
                &execution_options,
                &kernel_options,
                shape,
            )?;
            Ok(KernelExecutedQuery {
                physical,
                executed,
                kernel_report,
            })
        }
    }
}

pub fn execute_physical_planned_query_retained(
    input: CoveQlRetainedInput,
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

    if let Some((executed, kernel_report)) =
        try_ai_semantic_search_executed_query(&physical, &execution_options, kernel_options.mode)?
    {
        return Ok(KernelExecutedQuery {
            physical,
            executed,
            kernel_report,
        });
    }
    if let Some((executed, kernel_report)) =
        try_ai_embedding_executed_query(&physical, &execution_options, kernel_options.mode)?
    {
        return Ok(KernelExecutedQuery {
            physical,
            executed,
            kernel_report,
        });
    }
    if let Some((executed, kernel_report)) = try_ai_descriptor_metadata_executed_query(
        input.as_slice(),
        &physical,
        &execution_options,
        kernel_options.mode,
    )? {
        return Ok(KernelExecutedQuery {
            physical,
            executed,
            kernel_report,
        });
    }

    if kernel_options.mode != KernelExecutionMode::ForceMaterialized {
        if let Some(mut executed) = crate::zero_copy_arrow::try_execute_retained_zero_copy_arrow(
            &input,
            &physical,
            &execution_options,
        )? {
            let mut kernel_report = KernelExecutionReport::disabled(
                kernel_options.mode,
                "validated COVE-L retained zero-copy Arrow projection executed before object kernel selection",
            );
            kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
                "validated COVE-L zero-copy Arrow projection produced output without materialized comparison",
            );
            attach_phase8_plan_reports(&mut kernel_report, &physical.planned);
            kernel_report.decisions.push(KernelDecision::new(
                KernelDecisionKind::Applied,
                "retained zero-copy Arrow projection executed with COVE-L object-page authority",
                json!({ "execution": "zero_copy_arrow_projection" }),
                false,
            ));
            if kernel_options.mode == KernelExecutionMode::CompareWithMaterialized {
                let materialized = execute_planned_query(
                    input.as_slice(),
                    physical.planned.clone(),
                    execution_options.clone(),
                )?;
                kernel_report.compared_with_materialized = true;
                kernel_report.materialized_fingerprint =
                    Some(materialized.output_fingerprint.clone());
                kernel_report.kernel_fingerprint = Some(executed.output_fingerprint.clone());
                if materialized.output_fingerprint != executed.output_fingerprint {
                    return Err(exec_error(
                        "E_ZERO_COPY_MISMATCH",
                        "zero-copy Arrow output fingerprint does not match materialized baseline",
                        json!({}),
                    ));
                }
                executed.authority.compared_with_materialized = true;
            }
            return Ok(KernelExecutedQuery {
                physical,
                executed,
                kernel_report,
            });
        }
    }

    execute_physical_planned_query_inner(
        input.as_slice(),
        Some(&input),
        physical,
        execution_options,
        kernel_options,
    )
}

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

fn kernel_unsupported_diagnostic_code(
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
                        err,
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

struct AiDescriptorMetadataExecution {
    rows: Vec<Value>,
    input_rows: usize,
    warning_code: &'static str,
    warning_message: &'static str,
    authority_summary: &'static str,
    pushdown_summary: &'static str,
}

fn try_ai_descriptor_metadata_executed_query(
    source_bytes: &[u8],
    physical: &PhysicalPlannedQuery,
    execution_options: &ExecutionOptions,
    mode: KernelExecutionMode,
) -> Result<Option<(ExecutedQuery, KernelExecutionReport)>, BuildExecutionError> {
    let Some(operation) = physical
        .planned
        .resolved
        .method_chain
        .ai_operations
        .iter()
        .rev()
        .find(|operation| ai_descriptor_metadata_method(operation.method_name.as_str()))
    else {
        return Ok(None);
    };
    if mode == KernelExecutionMode::ForceMaterialized {
        return Err(exec_error(
            "E_AI_MATERIALIZED_UNSUPPORTED",
            "CoveQL-AI descriptor metadata operations require a validated COVE-AI sidecar; materialized COVE-O readback has no equivalent AI descriptor baseline",
            json!({ "method": operation.method_name }),
        ));
    }
    if !matches!(
        physical.planned.resolved.output_mode,
        crate::CoveQlOutputMode::JsonRows
    ) {
        return Err(exec_error(
            "E_AI_OUTPUT_UNSUPPORTED",
            "CoveQL-AI descriptor metadata operations currently return JSON rows",
            json!({ "output_mode": format!("{:?}", physical.planned.resolved.output_mode) }),
        ));
    }
    let include_payloads = ai_include_payloads_arg(operation)?;
    let generator_filter = if operation.method_name == "generatorAudit" {
        ai_generator_audit_filter(operation)?
    } else {
        AiGeneratorAuditFilter::default()
    };
    let Some(sidecar_bytes) = physical.sidecars.cove_ai_artifact_bytes.as_deref() else {
        return Err(exec_error(
            "E_AI_SIDECAR_REQUIRED",
            "CoveQL-AI descriptor metadata execution requires a supplied COVE-AI/COVE-VEC sidecar",
            json!({ "method": operation.method_name }),
        ));
    };
    let started = Instant::now();
    let sidecar = CoveAiFile::parse(sidecar_bytes).map_err(|error| {
        exec_error(
            "E_AI_SIDECAR_INVALID",
            format!("COVE-AI sidecar validation failed: {error}"),
            json!({ "method": operation.method_name }),
        )
    })?;
    let access_context = if include_payloads {
        CoveAiAccessContext::for_operation(operation.method_name.clone())
    } else {
        CoveAiAccessContext::descriptor_only(operation.method_name.clone())
    };
    let payload_reader = AiPayloadReader::new(sidecar_bytes, &sidecar, access_context);
    let chunk_source_context = ai_chunk_source_context_for_operation(
        operation.method_name.as_str(),
        include_payloads,
        source_bytes,
        &sidecar,
    );
    let execution = ai_descriptor_metadata_rows(
        operation.method_name.as_str(),
        &sidecar,
        include_payloads,
        &payload_reader,
        chunk_source_context.as_ref(),
        &generator_filter,
    );
    let result = CoveQlExecutionResult::JsonRows(execution.rows);
    let output_rows = match &result {
        CoveQlExecutionResult::JsonRows(rows) => rows.len(),
        _ => 0,
    };
    let row_counts = ExecutionRowCounts {
        input_rows: execution.input_rows,
        filtered_rows: output_rows,
        output_rows,
    };
    enforce_result_budgets(
        &result,
        &row_counts,
        &physical.planned,
        execution_options,
        started,
    )?;
    let output_fingerprint = result_fingerprint(&result)?;

    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        execution.warning_code,
        execution.warning_message,
        json!({
            "method": operation.method_name,
            "input_descriptor_rows": execution.input_rows,
            "output_rows": output_rows,
            "payload_access": ai_payload_access_label(&sidecar),
            "include_payloads": include_payloads,
            "payloads_policy_gated": true,
            "materialized_baseline_available": false,
        }),
    ));
    if mode == KernelExecutionMode::CompareWithMaterialized {
        diagnostics.push(exec_warning(
            "W_AI_NO_MATERIALIZED_BASELINE",
            "CoveQL-AI descriptor metadata execution has no materialized COVE-O baseline oracle; sidecar result was not fingerprint-compared",
            json!({ "method": operation.method_name }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(&physical.planned) {
        diagnostics.push(warning);
    }

    let mut kernel_report = KernelExecutionReport::applied(
        mode,
        KernelCounters {
            rows_scanned: execution.input_rows,
            rows_after_bitmap: execution.input_rows,
            rows_after_selection_vector: output_rows,
            output_rows,
            bytes_touched_estimate: sidecar_bytes.len(),
            dictionary_lookups_at_materialization: 0,
            ..KernelCounters::default()
        },
    );
    kernel_report.optimization_authority =
        OptimizationAuthorityReport::authoritative(execution.authority_summary);
    kernel_report.decision = KernelDecision::new(
        KernelDecisionKind::Applied,
        "CoveQL-AI descriptor metadata operation executed from a validated COVE-AI sidecar",
        json!({
            "method": operation.method_name,
            "input_descriptor_rows": execution.input_rows,
            "output_rows": output_rows,
            "payload_access": ai_payload_access_label(&sidecar),
            "include_payloads": include_payloads,
            "fallback_boundary": Value::Null,
            "materialized_baseline_available": false,
        }),
        false,
    );
    kernel_report.decisions = vec![kernel_report.decision.clone()];
    kernel_report.kernel_fingerprint = Some(output_fingerprint.clone());
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);

    let executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_applicable(
            &execution_options.pushdown,
            execution.pushdown_summary,
        ),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_kernel(execution.authority_summary, false),
    };
    Ok(Some((executed, kernel_report)))
}

fn ai_descriptor_metadata_method(method_name: &str) -> bool {
    matches!(
        method_name,
        "chunks"
            | "tokens"
            | "context"
            | "asPromptContext"
            | "trainingSamples"
            | "split"
            | "pack"
            | "multimodal"
            | "generatorAudit"
    )
}

fn ai_descriptor_metadata_rows(
    method_name: &str,
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
    chunk_source_context: Option<&AiChunkSourceContext>,
    generator_filter: &AiGeneratorAuditFilter,
) -> AiDescriptorMetadataExecution {
    match method_name {
        "chunks" => AiDescriptorMetadataExecution {
            rows: ai_chunk_projection_rows(
                sidecar,
                false,
                include_payloads,
                payload_reader,
                chunk_source_context,
            ),
            input_rows: sidecar.descriptor_tables.text_chunks.len(),
            warning_code: "W_AI_CHUNK_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .chunks() executed from validated COVE-CHUNK descriptor metadata",
            authority_summary: if include_payloads {
                "COVE-AI chunk descriptor metadata was validated; chunk text is reconstructed only from validated source values"
            } else {
                "COVE-AI chunk descriptor metadata was validated; chunk text payloads were withheld"
            },
            pushdown_summary:
                "CoveQL-AI chunk projection reads validated COVE-CHUNK sidecar descriptors rather than COVE-O row pages",
        },
        "tokens" => AiDescriptorMetadataExecution {
            rows: ai_token_projection_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.token_blocks.len()
                + sidecar.descriptor_tables.tokenized_spans.len()
                + sidecar.descriptor_tables.token_sequence_packs.len(),
            warning_code: "W_AI_TOKEN_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .tokens() executed from validated COVE-TOK descriptor metadata",
            authority_summary: if include_payloads {
                "COVE-AI token descriptor metadata was validated; token payload bytes are exposed only through AI payload leases"
            } else {
                "COVE-AI token descriptor metadata was validated; token payload bytes were withheld"
            },
            pushdown_summary:
                "CoveQL-AI token projection reads validated COVE-TOK sidecar descriptors rather than COVE-O row pages",
        },
        "context" | "asPromptContext" => AiDescriptorMetadataExecution {
            rows: ai_chunk_projection_rows(
                sidecar,
                true,
                include_payloads,
                payload_reader,
                chunk_source_context,
            ),
            input_rows: sidecar.descriptor_tables.text_chunks.len(),
            warning_code: "W_AI_RAG_CONTEXT_METADATA_EXECUTED",
            warning_message: if include_payloads {
                "CoveQL-AI RAG context executed from validated chunk descriptors with source-bound prompt text"
            } else {
                "CoveQL-AI RAG context executed from validated chunk descriptors with prompt text withheld"
            },
            authority_summary: if include_payloads {
                "COVE-AI RAG context descriptor metadata was validated; prompt text is reconstructed only from validated source values"
            } else {
                "COVE-AI RAG context descriptor metadata was validated; prompt text expansion was withheld by policy"
            },
            pushdown_summary:
                "CoveQL-AI context projection reads validated chunk sidecar descriptors rather than COVE-O row pages",
        },
        "split" => AiDescriptorMetadataExecution {
            rows: ai_training_split_rows(sidecar),
            input_rows: sidecar.descriptor_tables.dataset_splits.len(),
            warning_code: "W_AI_TRAINING_SPLIT_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .split() executed from validated COVE-TRAIN split descriptors",
            authority_summary:
                "COVE-AI training split descriptors were validated; sample payloads were withheld",
            pushdown_summary:
                "CoveQL-AI split projection reads validated COVE-TRAIN sidecar descriptors rather than COVE-O row pages",
        },
        "pack" => AiDescriptorMetadataExecution {
            rows: ai_training_pack_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.token_sequence_packs.len()
                + sidecar.descriptor_tables.multimodal_sequence_packs.len(),
            warning_code: "W_AI_TRAINING_PACK_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .pack() executed from validated token and multimodal pack descriptors",
            authority_summary:
                "COVE-AI training pack descriptors were validated; pack payloads were withheld",
            pushdown_summary:
                "CoveQL-AI pack projection reads validated COVE-AI sidecar descriptors rather than COVE-O row pages",
        },
        "multimodal" => AiDescriptorMetadataExecution {
            rows: ai_multimodal_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.multimodal_sequence_packs.len()
                + sidecar.descriptor_tables.multimodal_sequence_elements.len(),
            warning_code: "W_AI_MULTIMODAL_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .multimodal() executed from validated COVE-MMSEQ descriptor metadata",
            authority_summary:
                "COVE-AI multimodal sequence descriptors were validated; asset and tensor payloads were withheld",
            pushdown_summary:
                "CoveQL-AI multimodal projection reads validated COVE-MMSEQ sidecar descriptors rather than COVE-O row pages",
        },
        "generatorAudit" => AiDescriptorMetadataExecution {
            rows: ai_generator_audit_rows(
                sidecar,
                include_payloads,
                payload_reader,
                generator_filter,
            ),
            input_rows: sidecar.descriptor_tables.generator_provenance.len()
                + sidecar.descriptor_tables.model_actors.len()
                + sidecar.descriptor_tables.generation_decoding_profiles.len()
                + sidecar.descriptor_tables.human_reviews.len()
                + sidecar.descriptor_tables.training_labels.len()
                + sidecar.descriptor_tables.preference_pairs.len(),
            warning_code: "W_AI_GENERATOR_AUDIT_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .generatorAudit() executed from validated generator provenance descriptors",
            authority_summary:
                "COVE-AI generator provenance descriptors were validated; prompt/output payloads were withheld",
            pushdown_summary:
                "CoveQL-AI generator audit reads validated COVE-AI provenance descriptors rather than COVE-O row pages",
        },
        _ => AiDescriptorMetadataExecution {
            rows: ai_training_sample_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.training_samples.len(),
            warning_code: "W_AI_TRAINING_SAMPLE_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .trainingSamples() executed from validated COVE-TRAIN descriptor metadata",
            authority_summary:
                "COVE-AI training sample descriptors were validated; input/target payloads were withheld",
            pushdown_summary:
                "CoveQL-AI training sample projection reads validated COVE-TRAIN sidecar descriptors rather than COVE-O row pages",
        },
    }
}

struct AiChunkSourceContext {
    states: Vec<CoveObjectState>,
    read_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct AiGeneratorAuditFilter {
    model_namespace: AiStringRefFilter,
    model_name: AiStringRefFilter,
    model_version: AiStringRefFilter,
    provider: AiStringRefFilter,
    endpoint: AiStringRefFilter,
    decoding_profile_ref: Option<u32>,
    human_review_status: Option<AiHumanReviewStatusFilter>,
    reproducibility_class: Option<u8>,
}

impl AiGeneratorAuditFilter {
    fn is_empty(&self) -> bool {
        self.model_namespace.is_empty()
            && self.model_name.is_empty()
            && self.model_version.is_empty()
            && self.provider.is_empty()
            && self.endpoint.is_empty()
            && self.decoding_profile_ref.is_none()
            && self.human_review_status.is_none()
            && self.reproducibility_class.is_none()
    }
}

#[derive(Debug, Clone, Default)]
struct AiStringRefFilter {
    value: Option<String>,
    ref_id: Option<u32>,
}

impl AiStringRefFilter {
    fn is_empty(&self) -> bool {
        self.value.is_none() && self.ref_id.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiHumanReviewStatusFilter {
    Reviewed,
    Unreviewed,
}

struct AiChunkTextProjection {
    text: String,
    source_value_hash_status: &'static str,
    chunk_text_hash_status: &'static str,
}

fn ai_chunk_source_context_for_operation(
    method_name: &str,
    include_payloads: bool,
    source_bytes: &[u8],
    sidecar: &CoveAiFile,
) -> Option<AiChunkSourceContext> {
    if !include_payloads || !matches!(method_name, "chunks" | "context" | "asPromptContext") {
        return None;
    }

    let mut property_ids = sidecar
        .descriptor_tables
        .text_chunks
        .iter()
        .filter_map(|chunk| (chunk.property_id != 0).then_some(chunk.property_id))
        .collect::<Vec<_>>();
    property_ids.sort_unstable();
    property_ids.dedup();
    if property_ids.is_empty() {
        return Some(AiChunkSourceContext {
            states: Vec::new(),
            read_error: None,
        });
    }

    let read_options = CoveObjectReadOptions::requested_property_ids(property_ids);
    let surface = match read_object_surface_from_bytes_with_options(source_bytes, &read_options) {
        Ok(surface) => surface,
        Err(error) => {
            return Some(AiChunkSourceContext {
                states: Vec::new(),
                read_error: Some(format!("COVE-O source readback failed: {error}")),
            });
        }
    };
    let states =
        match reconstruct_object_states(&surface, &CoveObjectReconstructionOptions::default()) {
            Ok(states) => states,
            Err(error) => {
                return Some(AiChunkSourceContext {
                    states: Vec::new(),
                    read_error: Some(format!("COVE-O source reconstruction failed: {error}")),
                });
            }
        };
    Some(AiChunkSourceContext {
        states,
        read_error: None,
    })
}

fn ai_chunk_text_projection(
    chunk: &cove_core::artifact::coveai::TextChunkEntryV1,
    context: Option<&AiChunkSourceContext>,
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
) -> Result<AiChunkTextProjection, String> {
    let context = context.ok_or_else(|| "chunk text payloads were not requested".to_string())?;
    if let Some(error) = &context.read_error {
        return Err(error.clone());
    }
    if chunk.object_type_id == 0 || chunk.property_id == 0 {
        return Err(
            "chunk source binding does not identify a COVE-O object_type_id/property_id"
                .to_string(),
        );
    }

    let mut values = Vec::new();
    for state in &context.states {
        if state.object_type_id != chunk.object_type_id {
            continue;
        }
        if chunk.source_row_ref != 0 {
            let latest_row_index = u64::from(state.latest_row_index);
            let one_based_row_index = latest_row_index.saturating_add(1);
            if chunk.source_row_ref != latest_row_index
                && chunk.source_row_ref != one_based_row_index
            {
                continue;
            }
        }
        let Some(property) = state
            .properties
            .iter()
            .find(|property| property.property_id == chunk.property_id)
        else {
            continue;
        };
        if property.redacted {
            return Err("source property is redacted under the active COVE-O read policy".into());
        }
        let Some(text) = property.value.as_str() else {
            return Err("source property value is not UTF-8 text".into());
        };
        if !values.contains(&text) {
            values.push(text);
        }
    }

    let source_text = match values.as_slice() {
        [] => return Err("no matching source value was found for chunk binding".into()),
        [value] => *value,
        _ => return Err("chunk source binding resolved to multiple source values".into()),
    };
    let source_bytes = source_text.as_bytes();
    ai_verify_sha256_digest_ref(
        sidecar,
        payload_reader,
        chunk.source_value_hash_ref,
        source_bytes,
        "source_value_hash_ref",
    )?;

    let byte_start = usize::try_from(chunk.byte_start)
        .map_err(|_| "chunk byte_start exceeds platform usize".to_string())?;
    let byte_length = usize::try_from(chunk.byte_length)
        .map_err(|_| "chunk byte_length exceeds platform usize".to_string())?;
    let byte_end = byte_start
        .checked_add(byte_length)
        .ok_or_else(|| "chunk byte range overflows".to_string())?;
    if byte_end > source_bytes.len() {
        return Err("chunk byte range exceeds source text length".into());
    }
    if !source_text.is_char_boundary(byte_start) || !source_text.is_char_boundary(byte_end) {
        return Err("chunk byte range does not align to UTF-8 boundaries".into());
    }
    let chunk_text = &source_text[byte_start..byte_end];
    let scalar_start = u64::try_from(source_text[..byte_start].chars().count())
        .map_err(|_| "chunk unicode scalar start exceeds u64".to_string())?;
    let scalar_length = u64::try_from(chunk_text.chars().count())
        .map_err(|_| "chunk unicode scalar length exceeds u64".to_string())?;
    if chunk.unicode_scalar_start != scalar_start || chunk.unicode_scalar_length != scalar_length {
        return Err("chunk Unicode scalar offsets do not match source text span".into());
    }
    ai_verify_sha256_digest_ref(
        sidecar,
        payload_reader,
        chunk.chunk_text_hash_ref,
        chunk_text.as_bytes(),
        "chunk_text_hash_ref",
    )?;

    Ok(AiChunkTextProjection {
        text: chunk_text.to_string(),
        source_value_hash_status: "verified_sha256",
        chunk_text_hash_status: "verified_sha256",
    })
}

fn ai_verify_sha256_digest_ref(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    digest_ref: u32,
    expected_bytes: &[u8],
    label: &str,
) -> Result<(), String> {
    let digest = sidecar
        .descriptor_tables
        .digests
        .iter()
        .find(|digest| digest.digest_ref == digest_ref)
        .ok_or_else(|| format!("{label} {digest_ref} is missing from AI_DIGEST_TABLE"))?;
    if digest.digest_algorithm != 1 || digest.digest_len != 32 {
        return Err(format!(
            "{label} {digest_ref} uses unsupported digest algorithm/length"
        ));
    }
    let lease = payload_reader
        .lease_payload_ref(digest.digest_payload_ref)
        .map_err(|error| format!("{label} {digest_ref} digest payload withheld: {error}"))?;
    if lease.bytes.len() != 32 {
        return Err(format!(
            "{label} {digest_ref} digest payload length is not 32 bytes"
        ));
    }
    let actual = Sha256::digest(expected_bytes);
    if lease.bytes != actual.as_slice() {
        return Err(format!("{label} {digest_ref} digest mismatch"));
    }
    Ok(())
}

fn ai_payload_access_label(sidecar: &CoveAiFile) -> &'static str {
    match sidecar.payload_access {
        cove_core::artifact::coveai::AiPayloadAccessState::StructurallyAllowed => {
            "structurally_allowed"
        }
        cove_core::artifact::coveai::AiPayloadAccessState::PolicyBlockedMissingPrivacySummary => {
            "policy_blocked_missing_privacy_summary"
        }
    }
}

fn ai_payload_ref_projection(
    payload_ref: u32,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Value {
    if payload_ref == 0 {
        return json!({
            "payload_ref": 0,
            "payload_access": "not_declared",
            "payload": Value::Null,
        });
    }
    if !include_payloads {
        return json!({
            "payload_ref": payload_ref,
            "payload_access": "not_requested",
            "payload": Value::Null,
        });
    }
    match payload_reader.lease_payload_ref(payload_ref) {
        Ok(lease) => match std::str::from_utf8(lease.bytes) {
            Ok(text) => json!({
                "payload_ref": payload_ref,
                "payload_access": lease.disclosure.as_str(),
                "media_type_ref": lease.media_type_ref,
                "decoded_length": lease.decoded_length,
                "text": text,
            }),
            Err(_) => json!({
                "payload_ref": payload_ref,
                "payload_access": lease.disclosure.as_str(),
                "media_type_ref": lease.media_type_ref,
                "decoded_length": lease.decoded_length,
                "bytes_hex": hex(lease.bytes),
            }),
        },
        Err(error) => json!({
            "payload_ref": payload_ref,
            "payload_access": "withheld",
            "withholding_reason": error.to_string(),
            "payload": Value::Null,
        }),
    }
}

fn ai_token_block_payload_projection(
    block: &cove_core::artifact::coveai::TokenBlockHeaderV1,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Value {
    if !include_payloads {
        return json!({
            "payload_ref": block.payload_ref,
            "payload_access": "not_requested",
            "token_ids": Value::Null,
        });
    }
    let lease = match payload_reader.lease_payload_ref(block.payload_ref) {
        Ok(lease) => lease,
        Err(error) => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": error.to_string(),
                "token_ids": Value::Null,
            });
        }
    };
    let width = usize::from(block.token_id_width);
    let token_count = match usize::try_from(block.token_count) {
        Ok(value) => value,
        Err(_) => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": "token_count exceeds platform usize",
                "token_ids": Value::Null,
            });
        }
    };
    let offset = match usize::try_from(block.payload_offset) {
        Ok(value) => value,
        Err(_) => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": "payload_offset exceeds platform usize",
                "token_ids": Value::Null,
            });
        }
    };
    let length = if block.payload_length == 0 {
        match token_count.checked_mul(width) {
            Some(value) => value,
            None => {
                return json!({
                    "payload_ref": block.payload_ref,
                    "payload_access": "withheld",
                    "withholding_reason": "token payload length overflows",
                    "token_ids": Value::Null,
                });
            }
        }
    } else {
        match usize::try_from(block.payload_length) {
            Ok(value) => value,
            Err(_) => {
                return json!({
                    "payload_ref": block.payload_ref,
                    "payload_access": "withheld",
                    "withholding_reason": "payload_length exceeds platform usize",
                    "token_ids": Value::Null,
                });
            }
        }
    };
    let end = match offset.checked_add(length) {
        Some(value) => value,
        None => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": "token payload range overflows",
                "token_ids": Value::Null,
            });
        }
    };
    if end > lease.bytes.len() {
        return json!({
            "payload_ref": block.payload_ref,
            "payload_access": "withheld",
            "withholding_reason": "token payload range exceeds leased payload bytes",
            "token_ids": Value::Null,
        });
    }
    let token_bytes = &lease.bytes[offset..end];
    let mut token_ids = Vec::with_capacity(token_count);
    for chunk in token_bytes.chunks_exact(width) {
        let value = match block.token_id_width {
            1 => u64::from(chunk[0]),
            2 => u64::from(u16::from_le_bytes([chunk[0], chunk[1]])),
            4 => u64::from(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
            8 => u64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ]),
            _ => {
                return json!({
                    "payload_ref": block.payload_ref,
                    "payload_access": "withheld",
                    "withholding_reason": "unsupported token_id_width",
                    "token_ids": Value::Null,
                });
            }
        };
        token_ids.push(value);
    }
    json!({
        "payload_ref": block.payload_ref,
        "payload_access": lease.disclosure.as_str(),
        "token_id_width": block.token_id_width,
        "token_ids": token_ids,
        "byte_length": token_bytes.len(),
    })
}

fn ai_chunk_projection_rows(
    sidecar: &CoveAiFile,
    context_mode: bool,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
    chunk_source_context: Option<&AiChunkSourceContext>,
) -> Vec<Value> {
    sidecar
        .descriptor_tables
        .text_chunks
        .iter()
        .map(|chunk| {
            let text_projection = if include_payloads {
                Some(ai_chunk_text_projection(
                    chunk,
                    chunk_source_context,
                    sidecar,
                    payload_reader,
                ))
            } else {
                None
            };
            let mut row = json!({
                "record_kind": if context_mode { "rag_context_chunk" } else { "text_chunk" },
                "chunk_id": chunk.chunk_id,
                "source_ref": chunk.source_ref,
                "table_id": chunk.table_id,
                "column_id": chunk.column_id,
                "object_type_id": chunk.object_type_id,
                "property_id": chunk.property_id,
                "association_type_id": chunk.association_type_id,
                "path_ref": chunk.path_ref,
                "source_row_ref": chunk.source_row_ref,
                "source_object_ref": chunk.source_object_ref,
                "source_value_hash_ref": chunk.source_value_hash_ref,
                "byte_start": chunk.byte_start,
                "byte_length": chunk.byte_length,
                "unicode_scalar_start": chunk.unicode_scalar_start,
                "unicode_scalar_length": chunk.unicode_scalar_length,
                "token_start": chunk.token_start,
                "token_count": chunk.token_count,
                "parent_chunk_id": chunk.parent_chunk_id,
                "previous_chunk_id": chunk.previous_chunk_id,
                "next_chunk_id": chunk.next_chunk_id,
                "chunk_text_hash_ref": chunk.chunk_text_hash_ref,
                "evidence_ref": chunk.evidence_ref,
                "policy_ref": chunk.policy_ref,
                "descriptor_validated": true,
                "exact": true,
                "result_authority": "ValidatedAiDescriptorMetadata",
                "text": Value::Null,
                "text_withheld": true,
                "include_payloads": include_payloads,
                "withholding_reason": "chunk_text_requires_source_value_reconstruction_from_validated_cove_source",
            });
            match text_projection {
                Some(Ok(projection)) => {
                    row["text"] = json!(projection.text);
                    row["text_withheld"] = json!(false);
                    row["withholding_reason"] = Value::Null;
                    row["payload_access"] = json!("validated_source_value_reconstruction");
                    row["source_value_hash_status"] = json!(projection.source_value_hash_status);
                    row["chunk_text_hash_status"] = json!(projection.chunk_text_hash_status);
                    row["result_authority"] = json!("ValidatedAiSourceValueReconstruction");
                }
                Some(Err(reason)) => {
                    row["withholding_reason"] = json!(reason);
                    row["payload_access"] = json!("withheld");
                }
                None => {
                    row["payload_access"] = json!("not_requested");
                }
            }
            if context_mode {
                if row["text_withheld"].as_bool() == Some(false) {
                    row["prompt_context"] = row["text"].clone();
                    row["neighbor_expansion"] =
                        json!("withheld_without_verified_neighbor_expansion_policy");
                    row["redaction_report"] = json!({
                        "text_withheld": false,
                        "neighbor_chunks_withheld": true,
                        "reason": "current chunk text passed source, hash, UTF-8, and redaction checks; neighboring context remains withheld unless separately validated",
                    });
                } else {
                    row["prompt_context"] = Value::Null;
                    row["neighbor_expansion"] = json!("requires_source_value_reconstruction");
                    row["redaction_report"] = json!({
                        "text_withheld": true,
                        "neighbor_chunks_withheld": true,
                        "reason": row["withholding_reason"].clone(),
                    });
                }
            }
            row
        })
        .collect()
}

fn ai_token_projection_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    let mut rows = Vec::new();
    for block in &sidecar.descriptor_tables.token_blocks {
        rows.push(json!({
            "record_kind": "token_block",
            "token_block_id": block.token_block_id,
            "tokenizer_profile_id": block.tokenizer_profile_id,
            "token_count": block.token_count,
            "token_id_width": block.token_id_width,
            "compression_codec": block.compression_codec,
            "layout_kind": block.layout_kind,
            "payload_ref": block.payload_ref,
            "payload_offset": block.payload_offset,
            "payload_length": block.payload_length,
            "integrity_ref": block.integrity_ref,
            "descriptor_validated": true,
            "token_payload": ai_token_block_payload_projection(block, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for span in &sidecar.descriptor_tables.tokenized_spans {
        rows.push(json!({
            "record_kind": "tokenized_span",
            "tokenized_span_id": span.tokenized_span_id,
            "chunk_id": span.chunk_id,
            "tokenizer_profile_id": span.tokenizer_profile_id,
            "token_block_ref": span.token_block_ref,
            "token_offset": span.token_offset,
            "token_count": span.token_count,
            "byte_alignment_ref": span.byte_alignment_ref,
            "source_value_hash_ref": span.source_value_hash_ref,
            "descriptor_validated": true,
            "byte_alignment_payload": ai_payload_ref_projection(span.byte_alignment_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for pack in &sidecar.descriptor_tables.token_sequence_packs {
        rows.push(json!({
            "record_kind": "token_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "training_profile_ref": pack.training_profile_ref,
            "token_block_ref": pack.token_block_ref,
            "token_offset": pack.token_offset,
            "token_count": pack.token_count,
            "source_span_count": pack.source_span_count,
            "first_source_span_ref": pack.first_source_span_ref,
            "loss_mask_ref": pack.loss_mask_ref,
            "attention_mask_ref": pack.attention_mask_ref,
            "position_ids_ref": pack.position_ids_ref,
            "labels_ref": pack.labels_ref,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_ids": ai_payload_ref_projection(pack.position_ids_ref, include_payloads, payload_reader),
            "labels": ai_payload_ref_projection(pack.labels_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

fn ai_training_sample_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    sidecar
        .descriptor_tables
        .training_samples
        .iter()
        .map(|sample| {
            json!({
                "record_kind": "training_sample",
                "sample_id": sample.sample_id,
                "training_profile_id": sample.training_profile_id,
                "example_kind": sample.example_kind,
                "split_ref": sample.split_ref,
                "source_ref": sample.source_ref,
                "evidence_ref": sample.evidence_ref,
                "input_ref": sample.input_ref,
                "target_ref": sample.target_ref,
                "label_ref": sample.label_ref,
                "metadata_ref": sample.metadata_ref,
                "token_sequence_pack_ref": sample.token_sequence_pack_ref,
                "multimodal_sequence_pack_ref": sample.multimodal_sequence_pack_ref,
                "vector_ref": sample.vector_ref,
                "quality_score_ppm": sample.quality_score_ppm,
                "sample_weight_ppm": sample.sample_weight_ppm,
                "dedup_group_ref": sample.dedup_group_ref,
                "license_ref": sample.license_ref,
                "policy_ref": sample.policy_ref,
                "teacher_model_ref": sample.teacher_model_ref,
                "generator_provenance_ref": sample.generator_provenance_ref,
                "judge_generator_provenance_ref": sample.judge_generator_provenance_ref,
                "label_generator_provenance_ref": sample.label_generator_provenance_ref,
                "descriptor_validated": true,
                "input": ai_payload_ref_projection(sample.input_ref, include_payloads, payload_reader),
                "target": ai_payload_ref_projection(sample.target_ref, include_payloads, payload_reader),
                "metadata": ai_payload_ref_projection(sample.metadata_ref, include_payloads, payload_reader),
                "evidence": ai_payload_ref_projection(sample.evidence_ref, include_payloads, payload_reader),
                "payload_withheld": !include_payloads,
                "policy_withheld": !include_payloads,
                "exact": true,
                "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
            })
        })
        .collect()
}

fn ai_training_split_rows(sidecar: &CoveAiFile) -> Vec<Value> {
    sidecar
        .descriptor_tables
        .dataset_splits
        .iter()
        .map(|split| {
            let sample_count = sidecar
                .descriptor_tables
                .training_samples
                .iter()
                .filter(|sample| sample.split_ref == split.split_id)
                .count();
            json!({
                "record_kind": "dataset_split",
                "split_id": split.split_id,
                "split_name_ref": split.split_name_ref,
                "split_method": split.split_method,
                "source_snapshot_ref": split.source_snapshot_ref,
                "filter_policy_ref": split.filter_policy_ref,
                "seed": split.seed,
                "sample_count": sample_count,
                "first_sample_ref": split.first_sample_ref,
                "descriptor_validated": true,
                "payload_withheld": true,
                "withholding_reason": "descriptor_metadata_execution_does_not_expose_training_payloads",
                "exact": true,
                "result_authority": "ValidatedAiDescriptorMetadata",
            })
        })
        .collect()
}

fn ai_training_pack_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    let mut rows = Vec::new();
    for pack in &sidecar.descriptor_tables.token_sequence_packs {
        rows.push(json!({
            "record_kind": "token_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "training_profile_ref": pack.training_profile_ref,
            "token_block_ref": pack.token_block_ref,
            "token_offset": pack.token_offset,
            "token_count": pack.token_count,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_ids": ai_payload_ref_projection(pack.position_ids_ref, include_payloads, payload_reader),
            "labels": ai_payload_ref_projection(pack.labels_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for pack in &sidecar.descriptor_tables.multimodal_sequence_packs {
        rows.push(json!({
            "record_kind": "multimodal_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "training_profile_id": pack.training_profile_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "sequence_profile_ref": pack.sequence_profile_ref,
            "element_count": pack.element_count,
            "first_element_ref": pack.first_element_ref,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "label_ref": pack.label_ref,
            "evidence_ref": pack.evidence_ref,
            "generator_provenance_ref": pack.generator_provenance_ref,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_map": ai_payload_ref_projection(pack.position_map_ref, include_payloads, payload_reader),
            "label": ai_payload_ref_projection(pack.label_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(pack.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

fn ai_multimodal_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    let mut rows = Vec::new();
    for pack in &sidecar.descriptor_tables.multimodal_sequence_packs {
        rows.push(json!({
            "record_kind": "multimodal_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "training_profile_id": pack.training_profile_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "sequence_profile_ref": pack.sequence_profile_ref,
            "element_count": pack.element_count,
            "first_element_ref": pack.first_element_ref,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "loss_mask_ref": pack.loss_mask_ref,
            "attention_mask_ref": pack.attention_mask_ref,
            "position_map_ref": pack.position_map_ref,
            "label_ref": pack.label_ref,
            "source_snapshot_ref": pack.source_snapshot_ref,
            "evidence_ref": pack.evidence_ref,
            "generator_provenance_ref": pack.generator_provenance_ref,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_map": ai_payload_ref_projection(pack.position_map_ref, include_payloads, payload_reader),
            "label": ai_payload_ref_projection(pack.label_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(pack.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for element in &sidecar.descriptor_tables.multimodal_sequence_elements {
        rows.push(json!({
            "record_kind": "multimodal_sequence_element",
            "element_id": element.element_id,
            "sequence_pack_id": element.sequence_pack_id,
            "ordinal": element.ordinal,
            "element_kind": element.element_kind,
            "modality": element.modality,
            "role": element.role,
            "tokenized_span_ref": element.tokenized_span_ref,
            "token_sequence_pack_ref": element.token_sequence_pack_ref,
            "asset_ref": element.asset_ref,
            "tensor_ref": element.tensor_ref,
            "vector_ref": element.vector_ref,
            "byte_start": element.byte_start,
            "byte_length": element.byte_length,
            "time_start_us": element.time_start_us,
            "time_duration_us": element.time_duration_us,
            "position_stream_ref": element.position_stream_ref,
            "evidence_ref": element.evidence_ref,
            "policy_ref": element.policy_ref,
            "descriptor_validated": true,
            "position_stream": ai_payload_ref_projection(element.position_stream_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(element.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

struct AiGeneratorFilterSelection {
    filtered: bool,
    provenance_ids: BTreeSet<u64>,
    actor_ids: BTreeSet<u32>,
    decoding_profile_ids: BTreeSet<u32>,
    human_review_ids: BTreeSet<u32>,
}

impl AiGeneratorFilterSelection {
    fn include_provenance(&self, id: u64) -> bool {
        !self.filtered || self.provenance_ids.contains(&id)
    }

    fn include_actor(&self, id: u32) -> bool {
        !self.filtered || self.actor_ids.contains(&id)
    }

    fn include_decoding_profile(&self, id: u32) -> bool {
        !self.filtered || self.decoding_profile_ids.contains(&id)
    }

    fn include_human_review(&self, id: u32) -> bool {
        !self.filtered || self.human_review_ids.contains(&id)
    }

    fn include_label(&self, generator_provenance_ref: u64, human_review_ref: u32) -> bool {
        !self.filtered
            || (generator_provenance_ref != 0
                && self.provenance_ids.contains(&generator_provenance_ref))
            || (human_review_ref != 0 && self.human_review_ids.contains(&human_review_ref))
    }

    fn include_preference_pair(
        &self,
        judge_generator_provenance_ref: u64,
        human_review_ref: u32,
    ) -> bool {
        !self.filtered
            || (judge_generator_provenance_ref != 0
                && self
                    .provenance_ids
                    .contains(&judge_generator_provenance_ref))
            || (human_review_ref != 0 && self.human_review_ids.contains(&human_review_ref))
    }
}

fn ai_generator_filter_selection(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    filter: &AiGeneratorAuditFilter,
) -> AiGeneratorFilterSelection {
    let mut selection = AiGeneratorFilterSelection {
        filtered: !filter.is_empty(),
        provenance_ids: BTreeSet::new(),
        actor_ids: BTreeSet::new(),
        decoding_profile_ids: BTreeSet::new(),
        human_review_ids: BTreeSet::new(),
    };
    if filter.is_empty() {
        return selection;
    }

    for provenance in &sidecar.descriptor_tables.generator_provenance {
        if !ai_generator_provenance_matches_filter(sidecar, payload_reader, provenance, filter) {
            continue;
        }
        selection
            .provenance_ids
            .insert(provenance.generator_provenance_id);
        if provenance.model_actor_ref != 0 {
            selection.actor_ids.insert(provenance.model_actor_ref);
        }
        if provenance.decoding_profile_ref != 0 {
            selection
                .decoding_profile_ids
                .insert(provenance.decoding_profile_ref);
        }
        if provenance.human_review_ref != 0 {
            selection
                .human_review_ids
                .insert(provenance.human_review_ref);
        }
    }
    selection
}

fn ai_generator_provenance_matches_filter(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    provenance: &cove_core::artifact::coveai::GeneratorProvenanceV1,
    filter: &AiGeneratorAuditFilter,
) -> bool {
    if let Some(class) = filter.reproducibility_class {
        if provenance.reproducibility_class != class {
            return false;
        }
    }
    if let Some(decoding_profile_ref) = filter.decoding_profile_ref {
        if provenance.decoding_profile_ref != decoding_profile_ref {
            return false;
        }
    }
    if let Some(status) = filter.human_review_status {
        let reviewed = provenance.human_review_ref != 0;
        if (status == AiHumanReviewStatusFilter::Reviewed && !reviewed)
            || (status == AiHumanReviewStatusFilter::Unreviewed && reviewed)
        {
            return false;
        }
    }

    if filter.model_namespace.is_empty()
        && filter.model_name.is_empty()
        && filter.model_version.is_empty()
        && filter.provider.is_empty()
        && filter.endpoint.is_empty()
    {
        return true;
    }
    let Some(actor) = sidecar
        .descriptor_tables
        .model_actors
        .iter()
        .find(|actor| actor.model_actor_id == provenance.model_actor_ref)
    else {
        return false;
    };
    ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.model_namespace_ref,
        &filter.model_namespace,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.model_name_ref,
        &filter.model_name,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.model_version_ref,
        &filter.model_version,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.provider_ref,
        &filter.provider,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.endpoint_ref,
        &filter.endpoint,
    )
}

fn ai_string_ref_matches(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    string_ref: u32,
    filter: &AiStringRefFilter,
) -> bool {
    if filter.is_empty() {
        return true;
    }
    if filter.ref_id.is_some_and(|ref_id| ref_id == string_ref) {
        return true;
    }
    let Some(expected) = filter.value.as_deref() else {
        return false;
    };
    let Some(entry) = sidecar
        .descriptor_tables
        .strings
        .iter()
        .find(|entry| entry.string_ref == string_ref)
    else {
        return false;
    };
    let Ok(lease) = payload_reader.lease_payload_ref(entry.payload_ref) else {
        return false;
    };
    std::str::from_utf8(lease.bytes).is_ok_and(|actual| actual == expected)
}

fn ai_generator_audit_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
    filter: &AiGeneratorAuditFilter,
) -> Vec<Value> {
    let mut rows = Vec::new();
    let selection = ai_generator_filter_selection(sidecar, payload_reader, filter);
    for actor in &sidecar.descriptor_tables.model_actors {
        if !selection.include_actor(actor.model_actor_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "model_actor",
            "model_actor_id": actor.model_actor_id,
            "model_namespace_ref": actor.model_namespace_ref,
            "model_name_ref": actor.model_name_ref,
            "model_version_ref": actor.model_version_ref,
            "model_checkpoint_digest_ref": actor.model_checkpoint_digest_ref,
            "provider_ref": actor.provider_ref,
            "endpoint_ref": actor.endpoint_ref,
            "endpoint_version_ref": actor.endpoint_version_ref,
            "model_family_ref": actor.model_family_ref,
            "modality_mask": actor.modality_mask,
            "license_ref": actor.license_ref,
            "policy_ref": actor.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "payload_withheld": false,
            "exact": true,
            "result_authority": "ValidatedAiDescriptorMetadata",
        }));
    }
    for profile in &sidecar.descriptor_tables.generation_decoding_profiles {
        if !selection.include_decoding_profile(profile.decoding_profile_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "generation_decoding_profile",
            "decoding_profile_id": profile.decoding_profile_id,
            "temperature_micros": profile.temperature_micros,
            "top_p_micros": profile.top_p_micros,
            "top_k": profile.top_k,
            "seed": profile.seed,
            "max_output_tokens": profile.max_output_tokens,
            "stop_sequence_ref": profile.stop_sequence_ref,
            "safety_policy_ref": profile.safety_policy_ref,
            "deterministic_claim": profile.deterministic_claim,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "stop_sequence": ai_payload_ref_projection(profile.stop_sequence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for review in &sidecar.descriptor_tables.human_reviews {
        if !selection.include_human_review(review.human_review_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "human_review",
            "human_review_id": review.human_review_id,
            "review_kind": review.review_kind,
            "reviewer_role_ref": review.reviewer_role_ref,
            "review_time_us": review.review_time_us,
            "rating_ppm": review.rating_ppm,
            "notes_ref": review.notes_ref,
            "policy_ref": review.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "notes": ai_payload_ref_projection(review.notes_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for provenance in &sidecar.descriptor_tables.generator_provenance {
        if !selection.include_provenance(provenance.generator_provenance_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "generator_provenance",
            "generator_provenance_id": provenance.generator_provenance_id,
            "generator_kind": provenance.generator_kind,
            "model_actor_ref": provenance.model_actor_ref,
            "prompt_template_ref": provenance.prompt_template_ref,
            "decoding_profile_ref": provenance.decoding_profile_ref,
            "toolchain_ref": provenance.toolchain_ref,
            "source_input_ref": provenance.source_input_ref,
            "source_context_ref": provenance.source_context_ref,
            "source_sample_ref": provenance.source_sample_ref,
            "parent_generator_provenance_ref": provenance.parent_generator_provenance_ref,
            "generation_time_us": provenance.generation_time_us,
            "confidence_ppm": provenance.confidence_ppm,
            "human_review_ref": provenance.human_review_ref,
            "policy_ref": provenance.policy_ref,
            "reproducibility_class": provenance.reproducibility_class,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "prompt_template": ai_payload_ref_projection(provenance.prompt_template_ref, include_payloads, payload_reader),
            "toolchain": ai_payload_ref_projection(provenance.toolchain_ref, include_payloads, payload_reader),
            "source_input": ai_payload_ref_projection(provenance.source_input_ref, include_payloads, payload_reader),
            "source_context": ai_payload_ref_projection(provenance.source_context_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "external_audit_only_unless_deterministic_regeneration_proven": true,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for label in &sidecar.descriptor_tables.training_labels {
        if !selection.include_label(label.generator_provenance_ref, label.human_review_ref) {
            continue;
        }
        rows.push(json!({
            "record_kind": "training_label",
            "label_id": label.label_id,
            "label_kind": label.label_kind,
            "label_authority": label.label_authority,
            "label_payload_ref": label.label_payload_ref,
            "generator_provenance_ref": label.generator_provenance_ref,
            "human_review_ref": label.human_review_ref,
            "confidence_ppm": label.confidence_ppm,
            "evidence_ref": label.evidence_ref,
            "policy_ref": label.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "label_payload": ai_payload_ref_projection(label.label_payload_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(label.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for pair in &sidecar.descriptor_tables.preference_pairs {
        if !selection
            .include_preference_pair(pair.judge_generator_provenance_ref, pair.human_review_ref)
        {
            continue;
        }
        rows.push(json!({
            "record_kind": "preference_pair",
            "preference_pair_id": pair.preference_pair_id,
            "prompt_ref": pair.prompt_ref,
            "chosen_ref": pair.chosen_ref,
            "rejected_ref": pair.rejected_ref,
            "judge_generator_provenance_ref": pair.judge_generator_provenance_ref,
            "human_review_ref": pair.human_review_ref,
            "preference_strength_ppm": pair.preference_strength_ppm,
            "confidence_ppm": pair.confidence_ppm,
            "evidence_ref": pair.evidence_ref,
            "policy_ref": pair.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "prompt": ai_payload_ref_projection(pair.prompt_ref, include_payloads, payload_reader),
            "chosen": ai_payload_ref_projection(pair.chosen_ref, include_payloads, payload_reader),
            "rejected": ai_payload_ref_projection(pair.rejected_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(pair.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

fn try_ai_embedding_executed_query(
    physical: &PhysicalPlannedQuery,
    execution_options: &ExecutionOptions,
    mode: KernelExecutionMode,
) -> Result<Option<(ExecutedQuery, KernelExecutionReport)>, BuildExecutionError> {
    let Some(operation) = physical
        .planned
        .resolved
        .method_chain
        .ai_operations
        .iter()
        .find(|operation| operation.operation == CoveQlAiOperation::Embedding)
    else {
        return Ok(None);
    };
    if operation.method_name != "embedding" {
        return Err(exec_error(
            "E_AI_OPERATION_UNSUPPORTED",
            ".embedding() supports fileCode or vectorRef over a validated COVE-AI sidecar",
            json!({
                "method": operation.method_name,
                "operation": format!("{:?}", operation.operation),
            }),
        ));
    }
    if mode == KernelExecutionMode::ForceMaterialized {
        return Err(exec_error(
            "E_AI_MATERIALIZED_UNSUPPORTED",
            "CoveQL-AI embedding lookup requires a validated COVE-AI sidecar; materialized COVE-O readback has no equivalent .embedding() baseline",
            json!({ "method": operation.method_name }),
        ));
    }
    if !matches!(
        physical.planned.resolved.output_mode,
        crate::CoveQlOutputMode::JsonRows
    ) {
        return Err(exec_error(
            "E_AI_OUTPUT_UNSUPPORTED",
            "CoveQL-AI embedding lookup currently returns JSON rows",
            json!({ "output_mode": format!("{:?}", physical.planned.resolved.output_mode) }),
        ));
    }
    let Some(sidecar_bytes) = physical.sidecars.cove_ai_artifact_bytes.as_deref() else {
        return Err(exec_error(
            "E_AI_SIDECAR_REQUIRED",
            "CoveQL-AI .embedding() requires a supplied COVE-AI/COVE-VEC sidecar",
            json!({ "method": operation.method_name }),
        ));
    };
    let embedding_request = ai_embedding_request_args(operation)?;
    let sidecar_summary = CoveAiFile::parse(sidecar_bytes).map_err(|error| {
        exec_error(
            "E_AI_SIDECAR_INVALID",
            format!("COVE-AI sidecar validation failed: {error}"),
            json!({ "method": operation.method_name }),
        )
    })?;
    let binding_count = sidecar_summary
        .descriptor_tables
        .filecode_vector_bindings
        .len();
    let vector_space_count = sidecar_summary.descriptor_tables.vector_spaces.len();
    let started = Instant::now();
    let embedding = ai_embedding(sidecar_bytes, &embedding_request).map_err(|error| {
        exec_error(
            "E_AI_EMBEDDING_FAILED",
            format!("COVE-AI embedding lookup failed: {error}"),
            json!({
                "method": operation.method_name,
                "file_code": embedding_request.file_code,
                "vector_ref": embedding_request.vector_ref,
            }),
        )
    })?;
    let result = CoveQlExecutionResult::JsonRows(vec![json!({
        "target_kind": embedding.target_kind,
        "file_code": embedding.file_code,
        "vector_ref": embedding.vector_ref,
        "vector_space_id": embedding.vector_space_id,
        "dimension_count": embedding.dimension_count,
        "element_type": embedding.element_type,
        "embedding": embedding.values,
        "exact": true,
        "result_authority": embedding.result_authority,
    })]);
    let row_counts = ExecutionRowCounts {
        input_rows: binding_count,
        filtered_rows: 1,
        output_rows: 1,
    };
    enforce_result_budgets(
        &result,
        &row_counts,
        &physical.planned,
        execution_options,
        started,
    )?;
    let output_fingerprint = result_fingerprint(&result)?;

    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        "W_AI_FILECODE_EMBEDDING_EXECUTED",
        "CoveQL-AI .embedding() executed with validated COVE-AI vector lookup",
        json!({
            "file_code": embedding_request.file_code,
            "vector_ref": embedding_request.vector_ref,
            "dimension_count": embedding.dimension_count,
            "sidecar_vector_spaces": vector_space_count,
            "sidecar_filecode_bindings": binding_count,
            "result_authority": embedding.result_authority,
            "materialized_baseline_available": false,
        }),
    ));
    if mode == KernelExecutionMode::CompareWithMaterialized {
        diagnostics.push(exec_warning(
            "W_AI_NO_MATERIALIZED_BASELINE",
            "CoveQL-AI .embedding() has no materialized COVE-O baseline oracle; exact sidecar result was not fingerprint-compared",
            json!({ "file_code": embedding_request.file_code, "vector_ref": embedding_request.vector_ref }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(&physical.planned) {
        diagnostics.push(warning);
    }

    let mut kernel_report = KernelExecutionReport::applied(
        mode,
        KernelCounters {
            rows_scanned: binding_count,
            rows_after_bitmap: binding_count,
            rows_after_selection_vector: 1,
            output_rows: 1,
            bytes_touched_estimate: sidecar_bytes.len(),
            dictionary_lookups_at_materialization: 1,
            ..KernelCounters::default()
        },
    );
    kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
        "COVE-AI embedding lookup used validated COVE-VEC descriptor tables, payload integrity, privacy summary, and FileCode-local bindings",
    );
    kernel_report.decision = KernelDecision::new(
        KernelDecisionKind::Applied,
        "CoveQL-AI .embedding() FileCode vector lookup executed without materialized fallback",
        json!({
            "file_code": embedding_request.file_code,
            "vector_ref": embedding_request.vector_ref,
            "dimension_count": embedding.dimension_count,
            "sidecar_filecode_bindings": binding_count,
            "sidecar_vector_spaces": vector_space_count,
            "result_authority": embedding.result_authority,
            "fallback_boundary": Value::Null,
            "materialized_baseline_available": false,
        }),
        false,
    );
    kernel_report.decisions = vec![kernel_report.decision.clone()];
    kernel_report.kernel_fingerprint = Some(output_fingerprint.clone());
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);

    let executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_applicable(
            &execution_options.pushdown,
            "CoveQL-AI embedding lookup reads a validated COVE-VEC sidecar binding rather than COVE-O row pages",
        ),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_kernel(
            "CoveQL-AI FileCode embedding lookup returned a vector from a validated COVE-VEC sidecar",
            false,
        ),
    };
    Ok(Some((executed, kernel_report)))
}

fn try_ai_semantic_search_executed_query(
    physical: &PhysicalPlannedQuery,
    execution_options: &ExecutionOptions,
    mode: KernelExecutionMode,
) -> Result<Option<(ExecutedQuery, KernelExecutionReport)>, BuildExecutionError> {
    let Some(operation) = physical
        .planned
        .resolved
        .method_chain
        .ai_operations
        .iter()
        .find(|operation| operation.operation == CoveQlAiOperation::SemanticSearch)
    else {
        return Ok(None);
    };
    let method_name = operation.method_name.as_str();
    if !matches!(method_name, "similar" | "hybrid" | "rerank") {
        return Err(exec_error(
            "E_AI_OPERATION_UNSUPPORTED",
            "CoveQL-AI semantic search physical execution supports .similar(), .hybrid(), and .rerank() with fileCode/vectorRef/k/target/index arguments",
            json!({
                "method": operation.method_name,
                "operation": format!("{:?}", operation.operation),
            }),
        ));
    }
    let exact_semantic_authority = method_name == "similar";
    let result_authority = if exact_semantic_authority {
        "ExactOptimizedKernel"
    } else {
        "RuntimeAdvisory"
    };
    if mode == KernelExecutionMode::ForceMaterialized {
        return Err(exec_error(
            "E_AI_MATERIALIZED_UNSUPPORTED",
            "CoveQL-AI semantic search requires a validated COVE-AI sidecar; materialized COVE-O readback has no equivalent AI semantic-search baseline",
            json!({ "method": operation.method_name }),
        ));
    }
    if !matches!(
        physical.planned.resolved.output_mode,
        crate::CoveQlOutputMode::JsonRows
    ) {
        return Err(exec_error(
            "E_AI_OUTPUT_UNSUPPORTED",
            "CoveQL-AI semantic search currently returns JSON rows",
            json!({ "output_mode": format!("{:?}", physical.planned.resolved.output_mode) }),
        ));
    }
    let Some(sidecar_bytes) = physical.sidecars.cove_ai_artifact_bytes.as_deref() else {
        return Err(exec_error(
            "E_AI_SIDECAR_REQUIRED",
            "CoveQL-AI semantic search requires a supplied COVE-AI/COVE-VEC sidecar",
            json!({ "method": operation.method_name }),
        ));
    };
    let search_plan = ai_vector_search_plan_args(operation)?;
    let sidecar_summary = CoveAiFile::parse(sidecar_bytes).map_err(|error| {
        exec_error(
            "E_AI_SIDECAR_INVALID",
            format!("COVE-AI sidecar validation failed: {error}"),
            json!({ "method": operation.method_name }),
        )
    })?;
    let binding_count = sidecar_summary
        .descriptor_tables
        .filecode_vector_bindings
        .len()
        + sidecar_summary
            .descriptor_tables
            .chunk_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .object_state_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .training_sample_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .association_state_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .asset_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .multimodal_sequence_vector_bindings
            .len();
    let vector_space_count = sidecar_summary.descriptor_tables.vector_spaces.len();
    let started = Instant::now();
    let results = ai_vector_search(sidecar_bytes, &search_plan).map_err(|error| {
        exec_error(
            "E_AI_SEMANTIC_SEARCH_FAILED",
            format!("COVE-AI semantic search failed: {error}"),
            json!({
                "method": operation.method_name,
                "query_file_code": search_plan.query_file_code,
                "query_vector_ref": search_plan.query_vector_ref,
                "top_k": search_plan.top_k,
                "target": search_plan.target_kind.as_str(),
                "index": search_plan.index.as_str(),
            }),
        )
    })?;
    let rows = ai_semantic_search_rows(
        method_name,
        &search_plan,
        &results,
        exact_semantic_authority,
    );
    let vector_exact = results.iter().all(|result| result.exact);
    let fallback_used = results.iter().any(|result| result.fallback_used);
    let selected_index = results
        .first()
        .map(|result| result.selected_index.clone())
        .unwrap_or_else(|| search_plan.index.as_str().to_string());
    let selected_result_authority = results
        .first()
        .map(|result| result.result_authority.clone())
        .unwrap_or_else(|| result_authority.to_string());
    let result = CoveQlExecutionResult::JsonRows(rows);
    let row_counts = ExecutionRowCounts {
        input_rows: binding_count,
        filtered_rows: results.len(),
        output_rows: results.len(),
    };
    enforce_result_budgets(
        &result,
        &row_counts,
        &physical.planned,
        execution_options,
        started,
    )?;
    let output_fingerprint = result_fingerprint(&result)?;

    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        if exact_semantic_authority {
            "W_AI_EXACT_FLAT_VECTOR_SCAN_EXECUTED"
        } else {
            "W_AI_ADVISORY_VECTOR_SCAN_EXECUTED"
        },
        if exact_semantic_authority {
            "CoveQL-AI .similar() executed with validated COVE-AI vector search"
        } else {
            "CoveQL-AI semantic-search advisory method executed with validated vector candidates but without persisted hybrid/rerank authority"
        },
        json!({
            "method": method_name,
            "query_file_code": search_plan.query_file_code,
            "query_vector_ref": search_plan.query_vector_ref,
            "top_k": search_plan.top_k,
            "target": search_plan.target_kind.as_str(),
            "index": search_plan.index.as_str(),
            "result_count": results.len(),
            "sidecar_vector_spaces": vector_space_count,
            "sidecar_vector_bindings": binding_count,
            "selected_index": selected_index.clone(),
            "vector_exact": vector_exact,
            "fallback_used": fallback_used,
            "semantic_exact": exact_semantic_authority,
            "result_authority": selected_result_authority.clone(),
            "materialized_baseline_available": false,
        }),
    ));
    if mode == KernelExecutionMode::CompareWithMaterialized {
        diagnostics.push(exec_warning(
            "W_AI_NO_MATERIALIZED_BASELINE",
            "CoveQL-AI semantic search has no materialized COVE-O baseline oracle; sidecar result was not fingerprint-compared",
            json!({ "method": method_name, "query_file_code": search_plan.query_file_code, "query_vector_ref": search_plan.query_vector_ref }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(&physical.planned) {
        diagnostics.push(warning);
    }

    let mut kernel_report = KernelExecutionReport::applied(
        mode,
        KernelCounters {
            rows_scanned: binding_count,
            rows_after_bitmap: binding_count,
            rows_after_selection_vector: results.len(),
            output_rows: results.len(),
            bytes_touched_estimate: sidecar_bytes.len(),
            dictionary_lookups_at_materialization: results.len(),
            ..KernelCounters::default()
        },
    );
    kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
        "COVE-AI vector search used validated COVE-VEC descriptor tables, payload integrity, privacy summary, vector bindings, and exactness labels",
    );
    kernel_report.decision = KernelDecision::new(
        KernelDecisionKind::Applied,
        "CoveQL-AI semantic-search vector scan executed without materialized fallback",
        json!({
            "method": method_name,
            "query_file_code": search_plan.query_file_code,
            "query_vector_ref": search_plan.query_vector_ref,
            "top_k": search_plan.top_k,
            "target": search_plan.target_kind.as_str(),
            "index": search_plan.index.as_str(),
            "result_count": results.len(),
            "sidecar_vector_bindings": binding_count,
            "sidecar_vector_spaces": vector_space_count,
            "selected_index": selected_index,
            "vector_exact": vector_exact,
            "semantic_exact": exact_semantic_authority,
            "fallback_used": fallback_used,
            "result_authority": selected_result_authority,
            "fallback_boundary": Value::Null,
            "materialized_baseline_available": false,
        }),
        false,
    );
    kernel_report.decisions = vec![kernel_report.decision.clone()];
    kernel_report.kernel_fingerprint = Some(output_fingerprint.clone());
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);

    let executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_applicable(
            &execution_options.pushdown,
            "CoveQL-AI vector search reads validated COVE-VEC sidecar bindings rather than COVE-O row pages",
        ),
        evidence_authority: None,
        authority: if exact_semantic_authority && vector_exact {
            ExecutionAuthorityReport::exact_kernel(
                "CoveQL-AI vector search produced exact ranked results from a validated COVE-VEC sidecar",
                false,
            )
        } else {
            ExecutionAuthorityReport::physical_plan_only(
                "CoveQL-AI vector search produced advisory or approximate ranked results with per-result exactness labels",
            )
        },
    };
    Ok(Some((executed, kernel_report)))
}

fn ai_vector_search_plan_args(
    operation: &crate::ResolvedAiOperation,
) -> Result<AiVectorSearchPlan, BuildExecutionError> {
    let mut query_file_code = None;
    let mut query_vector_ref = None;
    let mut top_k = None;
    let mut target_kind = AiVectorSearchTargetKind::FileCode;
    let mut index = AiVectorIndexSelection::Auto;
    let mut unnamed_integer_index = 0usize;
    for arg in &operation.args {
        match arg.name.as_deref() {
            Some("fileCode" | "file_code" | "queryFileCode" | "query_file_code" | "query") => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search fileCode/queryFileCode must be an integer literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                query_file_code = Some(u32::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search fileCode argument exceeds u32",
                        json!({ "value": value }),
                    )
                })?);
            }
            Some("vectorRef" | "vector_ref" | "queryVectorRef" | "query_vector_ref") => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search vectorRef must be an integer literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                query_vector_ref = Some(value);
            }
            Some("k" | "topK" | "top_k" | "limit") => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search k/topK must be an integer literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                top_k = Some(usize::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search k argument exceeds usize",
                        json!({ "value": value }),
                    )
                })?);
            }
            Some("target" | "targetKind" | "target_kind") => {
                let value = ai_argument_string(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search target must be a string literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                target_kind = ai_vector_target_kind(&value)?;
            }
            Some("index" | "indexKind" | "index_kind") => {
                let value = ai_argument_string(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search index must be a string literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                index = ai_vector_index_selection(&value)?;
            }
            None if unnamed_integer_index == 0 => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search first unnamed argument must be integer fileCode",
                        json!({}),
                    )
                })?;
                query_file_code = Some(u32::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search fileCode argument exceeds u32",
                        json!({ "value": value }),
                    )
                })?);
                unnamed_integer_index += 1;
            }
            None if unnamed_integer_index == 1 => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search second unnamed argument must be integer k",
                        json!({}),
                    )
                })?;
                top_k = Some(usize::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search k argument exceeds usize",
                        json!({ "value": value }),
                    )
                })?);
                unnamed_integer_index += 1;
            }
            Some(name) => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI semantic search supports fileCode/queryFileCode, vectorRef, k, target, and index arguments",
                    json!({ "argument": name }),
                ));
            }
            None => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI semantic search supports at most two unnamed integer arguments: fileCode, k",
                    json!({}),
                ));
            }
        }
    }
    if query_file_code.is_some() && query_vector_ref.is_some() {
        return Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI semantic search accepts either fileCode/queryFileCode or vectorRef, not both",
            json!({ "file_code": query_file_code, "vector_ref": query_vector_ref }),
        ));
    }
    if query_file_code.is_none() && query_vector_ref.is_none() {
        return Err(exec_error(
            "E_AI_ARGUMENT_REQUIRED",
            "CoveQL-AI semantic search requires fileCode/queryFileCode or vectorRef",
            json!({}),
        ));
    };
    Ok(AiVectorSearchPlan {
        query_file_code,
        query_vector_ref,
        query_values: None,
        top_k: top_k.unwrap_or(10),
        target_kind,
        index,
    })
}

fn ai_embedding_request_args(
    operation: &crate::ResolvedAiOperation,
) -> Result<AiEmbeddingRequest, BuildExecutionError> {
    let mut file_code = None;
    let mut vector_ref = None;
    for arg in &operation.args {
        let value = ai_argument_u64(&arg.value).ok_or_else(|| {
            exec_error(
                "E_AI_ARGUMENT_UNSUPPORTED",
                ".embedding() requires integer fileCode or vectorRef arguments",
                json!({ "argument": arg.name.clone() }),
            )
        })?;
        match arg.name.as_deref() {
            Some("fileCode" | "file_code" | "queryFileCode" | "query_file_code" | "query")
            | None
                if file_code.is_none() && vector_ref.is_none() =>
            {
                file_code = Some(u32::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        ".embedding() fileCode argument exceeds u32",
                        json!({ "value": value }),
                    )
                })?);
            }
            Some("vectorRef" | "vector_ref") => {
                vector_ref = Some(value);
            }
            Some(name) => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    ".embedding() supports fileCode or vectorRef arguments",
                    json!({ "argument": name }),
                ));
            }
            None => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    ".embedding() supports one unnamed integer argument: fileCode",
                    json!({}),
                ));
            }
        }
    }
    if file_code.is_some() && vector_ref.is_some() {
        return Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            ".embedding() accepts either fileCode or vectorRef, not both",
            json!({ "file_code": file_code, "vector_ref": vector_ref }),
        ));
    }
    if file_code.is_none() && vector_ref.is_none() {
        return Err(exec_error(
            "E_AI_ARGUMENT_REQUIRED",
            ".embedding() requires fileCode or vectorRef",
            json!({}),
        ));
    }
    Ok(AiEmbeddingRequest {
        file_code,
        vector_ref,
    })
}

fn ai_vector_target_kind(value: &str) -> Result<AiVectorSearchTargetKind, BuildExecutionError> {
    match value {
        "all" => Ok(AiVectorSearchTargetKind::All),
        "fileCode" | "file_code" | "file-code" => Ok(AiVectorSearchTargetKind::FileCode),
        "chunk" | "chunks" => Ok(AiVectorSearchTargetKind::Chunk),
        "object" | "objectState" | "object_state" | "object-state" => {
            Ok(AiVectorSearchTargetKind::ObjectState)
        }
        "association"
        | "associationState"
        | "association_state"
        | "association-state"
        | "edge" => Ok(AiVectorSearchTargetKind::AssociationState),
        "sample" | "trainingSample" | "training_sample" | "training-sample" => {
            Ok(AiVectorSearchTargetKind::TrainingSample)
        }
        "asset" | "assets" => Ok(AiVectorSearchTargetKind::Asset),
        "multimodal"
        | "multimodalSequence"
        | "multimodal_sequence"
        | "multimodal-sequence"
        | "sequence" => Ok(AiVectorSearchTargetKind::MultimodalSequence),
        other => Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI semantic search target must be all, file_code, chunk, object_state, association_state, training_sample, asset, or multimodal_sequence",
            json!({ "target": other }),
        )),
    }
}

fn ai_vector_index_selection(value: &str) -> Result<AiVectorIndexSelection, BuildExecutionError> {
    match value {
        "auto" => Ok(AiVectorIndexSelection::Auto),
        "exact" | "exactFlat" | "exact_flat" | "exact-flat" => {
            Ok(AiVectorIndexSelection::ExactFlat)
        }
        "hnsw" => Ok(AiVectorIndexSelection::Hnsw),
        "ivf" | "ivfFlat" | "ivf_flat" | "ivf-flat" => Ok(AiVectorIndexSelection::IvfFlat),
        "ivfPq" | "ivf_pq" | "ivf-pq" => Ok(AiVectorIndexSelection::IvfPq),
        "diskann" | "diskAnn" | "disk_ann" | "disk-ann" => Ok(AiVectorIndexSelection::DiskAnn),
        "vamana" => Ok(AiVectorIndexSelection::Vamana),
        other => Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI semantic search index must be auto, exact_flat, hnsw, ivf_flat, ivf_pq, diskann, or vamana",
            json!({ "index": other }),
        )),
    }
}

fn ai_include_payloads_arg(
    operation: &crate::ResolvedAiOperation,
) -> Result<bool, BuildExecutionError> {
    let mut include_payloads = true;
    for arg in &operation.args {
        match arg.name.as_deref() {
            Some("includePayloads" | "include_payloads" | "payloads") => {
                include_payloads = ai_argument_bool(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI includePayloads argument must be a boolean literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
            }
            Some(name) => {
                if operation.method_name == "generatorAudit" && ai_generator_filter_arg_name(name) {
                    continue;
                }
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI descriptor/runtime projection supports only includePayloads for this method",
                    json!({ "method": operation.method_name, "argument": name }),
                ));
            }
            None => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI descriptor/runtime projection requires named arguments",
                    json!({ "method": operation.method_name }),
                ));
            }
        }
    }
    Ok(include_payloads)
}

fn ai_generator_filter_arg_name(name: &str) -> bool {
    matches!(
        name,
        "modelNamespace"
            | "model_namespace"
            | "modelNamespaceRef"
            | "model_namespace_ref"
            | "modelName"
            | "model_name"
            | "modelNameRef"
            | "model_name_ref"
            | "modelVersion"
            | "model_version"
            | "modelVersionRef"
            | "model_version_ref"
            | "provider"
            | "providerRef"
            | "provider_ref"
            | "endpoint"
            | "endpointRef"
            | "endpoint_ref"
            | "decodingProfile"
            | "decoding_profile"
            | "decodingProfileRef"
            | "decoding_profile_ref"
            | "humanReviewStatus"
            | "human_review_status"
            | "reproducibilityClass"
            | "reproducibility_class"
    )
}

fn ai_generator_audit_filter(
    operation: &crate::ResolvedAiOperation,
) -> Result<AiGeneratorAuditFilter, BuildExecutionError> {
    let mut filter = AiGeneratorAuditFilter::default();
    for arg in &operation.args {
        let Some(name) = arg.name.as_deref() else {
            return Err(exec_error(
                "E_AI_ARGUMENT_UNSUPPORTED",
                "CoveQL-AI generatorAudit filter arguments must be named",
                json!({ "method": operation.method_name }),
            ));
        };
        match name {
            "includePayloads" | "include_payloads" | "payloads" => {}
            "modelNamespace" | "model_namespace" | "modelNamespaceRef" | "model_namespace_ref" => {
                filter.model_namespace = ai_string_ref_filter(name, &arg.value)?;
            }
            "modelName" | "model_name" | "modelNameRef" | "model_name_ref" => {
                filter.model_name = ai_string_ref_filter(name, &arg.value)?;
            }
            "modelVersion" | "model_version" | "modelVersionRef" | "model_version_ref" => {
                filter.model_version = ai_string_ref_filter(name, &arg.value)?;
            }
            "provider" | "providerRef" | "provider_ref" => {
                filter.provider = ai_string_ref_filter(name, &arg.value)?;
            }
            "endpoint" | "endpointRef" | "endpoint_ref" => {
                filter.endpoint = ai_string_ref_filter(name, &arg.value)?;
            }
            "decodingProfile"
            | "decoding_profile"
            | "decodingProfileRef"
            | "decoding_profile_ref" => {
                filter.decoding_profile_ref =
                    Some(ai_argument_u32(name, &arg.value, "decoding profile")?);
            }
            "humanReviewStatus" | "human_review_status" => {
                let value = ai_argument_string(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI generatorAudit humanReviewStatus must be reviewed or unreviewed",
                        json!({ "argument": name }),
                    )
                })?;
                filter.human_review_status = Some(match value.as_str() {
                    "reviewed" | "humanReviewed" | "human_reviewed" => {
                        AiHumanReviewStatusFilter::Reviewed
                    }
                    "unreviewed" | "notReviewed" | "not_reviewed" => {
                        AiHumanReviewStatusFilter::Unreviewed
                    }
                    _ => {
                        return Err(exec_error(
                            "E_AI_ARGUMENT_UNSUPPORTED",
                            "CoveQL-AI generatorAudit humanReviewStatus must be reviewed or unreviewed",
                            json!({ "argument": name, "value": value }),
                        ));
                    }
                });
            }
            "reproducibilityClass" | "reproducibility_class" => {
                filter.reproducibility_class =
                    Some(ai_reproducibility_class_filter(name, &arg.value)?);
            }
            _ => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI generatorAudit supports model namespace/name/version, provider, endpoint, decoding profile, human review status, reproducibility class, and includePayloads arguments",
                    json!({ "argument": name }),
                ));
            }
        }
    }
    Ok(filter)
}

fn ai_string_ref_filter(
    name: &str,
    value: &ResolvedAiArgumentValue,
) -> Result<AiStringRefFilter, BuildExecutionError> {
    if name.ends_with("Ref") || name.ends_with("_ref") {
        return Ok(AiStringRefFilter {
            value: None,
            ref_id: Some(ai_argument_u32(name, value, "string ref")?),
        });
    }
    if let Some(ref_id) = ai_argument_u64(value).and_then(|value| u32::try_from(value).ok()) {
        return Ok(AiStringRefFilter {
            value: None,
            ref_id: Some(ref_id),
        });
    }
    let text = ai_argument_string(value).ok_or_else(|| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI generatorAudit string filters must be string literals or integer string refs",
            json!({ "argument": name }),
        )
    })?;
    Ok(AiStringRefFilter {
        value: Some(text),
        ref_id: None,
    })
}

fn ai_argument_u32(
    name: &str,
    value: &ResolvedAiArgumentValue,
    label: &str,
) -> Result<u32, BuildExecutionError> {
    let value = ai_argument_u64(value).ok_or_else(|| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            format!("CoveQL-AI generatorAudit {label} argument must be an integer literal"),
            json!({ "argument": name }),
        )
    })?;
    u32::try_from(value).map_err(|_| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            format!("CoveQL-AI generatorAudit {label} argument exceeds u32"),
            json!({ "argument": name, "value": value }),
        )
    })
}

fn ai_reproducibility_class_filter(
    name: &str,
    value: &ResolvedAiArgumentValue,
) -> Result<u8, BuildExecutionError> {
    if let Some(value) = ai_argument_u64(value) {
        return u8::try_from(value).map_err(|_| {
            exec_error(
                "E_AI_ARGUMENT_UNSUPPORTED",
                "CoveQL-AI generatorAudit reproducibilityClass exceeds u8",
                json!({ "argument": name, "value": value }),
            )
        });
    }
    let text = ai_argument_string(value).ok_or_else(|| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI generatorAudit reproducibilityClass must be an integer or known class name",
            json!({ "argument": name }),
        )
    })?;
    match text.as_str() {
        "descriptiveOnly" | "descriptive_only" => Ok(0),
        "sourceSnapshotReproducible" | "source_snapshot_reproducible" => Ok(1),
        "preprocessingReproducible" | "preprocessing_reproducible" => Ok(2),
        "storedPayloadVerifiable" | "stored_payload_verifiable" => Ok(3),
        "canonicalRecomputeReproducible" | "canonical_recompute_reproducible" => Ok(4),
        "externalAuditOnly" | "external_audit_only" => Ok(5),
        _ => Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI generatorAudit reproducibilityClass is not recognized",
            json!({ "argument": name, "value": text }),
        )),
    }
}

fn ai_argument_u64(value: &ResolvedAiArgumentValue) -> Option<u64> {
    let ResolvedAiArgumentValue::Expr(ResolvedExpr::Literal(literal)) = value else {
        return None;
    };
    match &literal.typed_value {
        ResolvedLiteralValue::UnsignedInteger(value) => Some(*value),
        ResolvedLiteralValue::SignedInteger(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn ai_argument_bool(value: &ResolvedAiArgumentValue) -> Option<bool> {
    let ResolvedAiArgumentValue::Expr(ResolvedExpr::Literal(literal)) = value else {
        return None;
    };
    match &literal.typed_value {
        ResolvedLiteralValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn ai_argument_string(value: &ResolvedAiArgumentValue) -> Option<String> {
    let ResolvedAiArgumentValue::Expr(ResolvedExpr::Literal(literal)) = value else {
        return None;
    };
    match &literal.typed_value {
        ResolvedLiteralValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn ai_semantic_search_rows(
    method_name: &str,
    search_plan: &AiVectorSearchPlan,
    results: &[AiVectorSearchResult],
    exact_semantic_authority: bool,
) -> Vec<Value> {
    results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            let exact = exact_semantic_authority && result.exact;
            let result_authority = if exact_semantic_authority {
                result.result_authority.as_str()
            } else {
                "RuntimeAdvisory"
            };
            json!({
                "method": method_name,
                "rank": index + 1,
                "query_file_code": search_plan.query_file_code,
                "query_vector_ref": search_plan.query_vector_ref,
                "target": search_plan.target_kind.as_str(),
                "requested_index": search_plan.index.as_str(),
                "target_kind": result.target_kind,
                "binding_id": result.binding_id,
                "file_code": result.file_code,
                "chunk_id": result.chunk_id,
                "object_type_id": result.object_type_id,
                "association_type_id": result.association_type_id,
                "sample_id": result.sample_id,
                "asset_ref": result.asset_ref,
                "multimodal_sequence_pack_id": result.multimodal_sequence_pack_id,
                "vector_ref": result.vector_ref,
                "vector_space_id": result.vector_space_id,
                "score": result.score,
                "exact": exact,
                "vector_exact": result.exact,
                "index_kind": result.selected_index,
                "fallback_used": result.fallback_used,
                "result_authority": result_authority,
                "advisory_reason": if exact_semantic_authority {
                    Value::Null
                } else {
                    json!("no_persisted_hybrid_or_rerank_authority")
                },
            })
        })
        .collect()
}

fn try_covi_empty_lookup_executed_query(
    physical: &PhysicalPlannedQuery,
    options: &ExecutionOptions,
) -> Result<Option<ExecutedQuery>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if !physical.allow_index_only_answers
        || physical.index_capability_report.lookup_candidates == 0
        || !security.index_only_answer_permission
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || security.aggregate_disclosure_policy == AggregateDisclosurePolicy::AllowMaterializedOnly
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || options.visibility_overlay.is_some()
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || !matches!(planned.resolved.temporal.mode, TemporalMode::Latest)
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || matches!(
            planned.resolved.output_mode,
            crate::CoveQlOutputMode::DataFusionTableProvider | crate::CoveQlOutputMode::ExplainJson
        )
    {
        return Ok(None);
    }
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Ok(None);
    };
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    let requests = covi_empty_lookup_requests(predicate, root.object_type_id)?;
    if requests.is_empty() {
        return Ok(None);
    }
    let Some((request, source)) = covi_first_empty_lookup(physical, &requests)? else {
        return Ok(None);
    };

    let started = Instant::now();
    validate_security_scope(planned, options)?;
    validate_execution_grain(planned)?;
    let (result, row_counts) =
        finish_materialized_rows(Vec::new(), &[], &[], &[], planned, options, started)?;
    enforce_result_budgets(&result, &row_counts, planned, options, started)?;
    let output_fingerprint = result_fingerprint(&result)?;
    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        "W_COVI_EMPTY_LOOKUP_EXECUTED",
        "validated COVI/COVX lookup proved the predicate candidate set is empty",
        json!({
            "source": source,
            "lookup_target": covi_lookup_target_json(&request.target),
            "lookup_key_count": covi_lookup_request_key_count(&request),
        }),
    ));
    Ok(Some(ExecutedQuery {
        planned: planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_executed(&options.pushdown),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_index_only(
            "validated exact COVI/COVX lookup proved an empty candidate set",
        ),
    }))
}

fn execution_diagnostics_for_physical(physical: &PhysicalPlannedQuery) -> Vec<ExecutionDiagnostic> {
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

#[derive(Debug, Clone)]
struct CoviRowRangePrune {
    bitmap: SelectionBitmap,
    source: &'static str,
    lookup_count: usize,
    row_range_count: usize,
    matched_rows: usize,
}

#[derive(Debug, Clone)]
struct CoverageRowRangePrune {
    bitmap: SelectionBitmap,
    predicate_form_ref: u32,
    row_range_count: usize,
    matched_rows: usize,
}

#[derive(Debug, Clone)]
struct ExecutionCodePrune {
    bitmap: SelectionBitmap,
    predicate_count: usize,
    page_count: usize,
    matched_rows: usize,
}

#[derive(Debug, Clone)]
struct FileCodeLiteralPrune {
    bitmap: SelectionBitmap,
    predicate_count: usize,
    page_count: usize,
    matched_rows: usize,
}

#[derive(Debug, Clone)]
struct NativeScalarPredicatePrune {
    bitmap: SelectionBitmap,
    predicate_count: usize,
    executed_predicate_count: usize,
    page_count: usize,
    matched_rows: usize,
    predicate_order: Vec<&'static str>,
    predicate_dispatch: NativeKernelDispatchCounts,
    bitmap_dispatch: NativeBitmapDispatchCounts,
    retained_page_buffers: bool,
    short_circuited: bool,
}

#[derive(Debug, Clone, Default)]
struct NativeKernelDispatchCounts {
    kernels: usize,
    scalar: usize,
    avx2: usize,
    neon: usize,
}

impl NativeKernelDispatchCounts {
    fn record(&mut self, dispatch: NativeKernelDispatch) {
        self.kernels += 1;
        match dispatch {
            NativeKernelDispatch::Scalar | NativeKernelDispatch::Auto => self.scalar += 1,
            NativeKernelDispatch::Avx2 => self.avx2 += 1,
            NativeKernelDispatch::Neon => self.neon += 1,
        }
    }

    fn merge(&mut self, other: NativeKernelDispatchCounts) {
        self.kernels += other.kernels;
        self.scalar += other.scalar;
        self.avx2 += other.avx2;
        self.neon += other.neon;
    }
}

#[derive(Debug, Clone, Default)]
struct NativeBitmapDispatchCounts {
    intersections: usize,
    scalar: usize,
    avx2: usize,
    neon: usize,
}

impl NativeBitmapDispatchCounts {
    fn record(&mut self, dispatch: NativeKernelDispatch) {
        self.intersections += 1;
        match dispatch {
            NativeKernelDispatch::Scalar | NativeKernelDispatch::Auto => self.scalar += 1,
            NativeKernelDispatch::Avx2 => self.avx2 += 1,
            NativeKernelDispatch::Neon => self.neon += 1,
        }
    }
}

#[derive(Debug, Clone)]
struct NativeScalarBitmapScan {
    bitmap: SelectionBitmap,
    page_count: usize,
    predicate_dispatch: NativeKernelDispatchCounts,
}

#[derive(Debug, Clone)]
struct NativeScalarBatchScan {
    page_count: usize,
    predicate_dispatch: NativeKernelDispatchCounts,
}

#[derive(Debug, Clone, Default)]
struct SurfaceRowLookup {
    segments: Vec<SurfaceRowLookupSegment>,
}

#[derive(Debug, Clone)]
struct SurfaceRowLookupSegment {
    segment_id: u32,
    rows: Vec<Option<usize>>,
}

impl SurfaceRowLookup {
    fn from_segment_rows(segment_rows: BTreeMap<u32, Vec<Option<usize>>>) -> Self {
        Self {
            segments: segment_rows
                .into_iter()
                .map(|(segment_id, rows)| SurfaceRowLookupSegment { segment_id, rows })
                .collect(),
        }
    }

    fn get(&self, segment_id: u32, row_index: u32) -> Option<usize> {
        let segment = self
            .segments
            .binary_search_by_key(&segment_id, |segment| segment.segment_id)
            .ok()
            .and_then(|index| self.segments.get(index))?;
        segment.rows.get(row_index as usize).and_then(|row| *row)
    }
}

#[derive(Debug, Clone)]
struct ExecutionCodePredicateRequest {
    object_type_id: u32,
    property_id: u32,
    logical_type: String,
    match_mode: ExecutionCodeMatchMode,
    canonical_literals: Vec<ExecutionCodeLiteralKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExecutionCodeLiteralKey {
    value_tag: u16,
    canonical: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionCodeMatchMode {
    Include,
    Exclude,
}

#[derive(Debug, Clone)]
enum NativeScalarPredicateRequest {
    SystemNumeric {
        object_type_id: u32,
        field: NativeSystemNumericField,
        op: NativeNumericPredicateOp,
        literal: NativeNumericLiteral,
    },
    SystemGoidEq {
        object_type_id: u32,
        value: [u8; 16],
    },
    NullCheck {
        object_type_id: u32,
        property_id: u32,
        want_valid: bool,
    },
    BoolEq {
        object_type_id: u32,
        property_id: u32,
        value: bool,
    },
    FixedBytesEq {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        value: Vec<u8>,
    },
    FixedBytesIn {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        values: Vec<u8>,
    },
    FixedBytesNotIn {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        values: Vec<u8>,
    },
    VarBytesEq {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        value: Vec<u8>,
    },
    VarBytesIn {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        values: Vec<Vec<u8>>,
    },
    VarBytesNotIn {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        values: Vec<Vec<u8>>,
    },
    VarBytesPrefix {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        prefix: Vec<u8>,
    },
    NumCode {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        op: NativeNumericPredicateOp,
        literal: NativeNumericLiteral,
    },
    NumCodeIn {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        literals: Vec<NativeNumericLiteral>,
    },
    NumCodeNotIn {
        object_type_id: u32,
        property_id: u32,
        logical_type: CoveLogicalType,
        literals: Vec<NativeNumericLiteral>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeSystemNumericField {
    BranchKey,
    Csn,
    TimestampUs,
}

impl ExecutionCodeMatchMode {
    fn inverted(self) -> Self {
        match self {
            Self::Include => Self::Exclude,
            Self::Exclude => Self::Include,
        }
    }
}

struct StaticExecutionCodeResolver<'a> {
    filecode_to_execution: &'a BTreeMap<u32, u64>,
}

#[derive(Debug, Clone)]
struct NativeBoolGroupCountReport {
    aggregate: &'static str,
    group_property: String,
    group_logical_type: String,
    group_count: usize,
    rows_counted: usize,
    values_seen: usize,
    group_strategy: &'static str,
    aggregate_strategy: &'static str,
}

#[derive(Debug, Clone)]
struct NativeDirectAggregateReport {
    aggregate: &'static str,
    counted_path: Option<String>,
    rows_counted: usize,
    values_seen: usize,
    aggregate_strategy: &'static str,
}

#[derive(Debug, Clone)]
struct NativeHelperAggregateReport {
    aggregate: &'static str,
    helper_kind: &'static str,
    rows_counted: usize,
    helper_values_seen: usize,
}

#[derive(Debug, Clone)]
struct NativeGroupedHelperAggregateReport {
    aggregate: &'static str,
    helper_kind: &'static str,
    group_property: String,
    group_logical_type: String,
    group_count: usize,
    rows_counted: usize,
    helper_values_seen: usize,
}

#[derive(Debug, Clone)]
struct NativeTypedOrderReport {
    order_property: String,
    logical_type: String,
    collation_id: Option<u16>,
    sort_key_boundary: Option<String>,
    rows_sorted: usize,
    output_rows: usize,
}

#[derive(Debug, Clone)]
struct NativeRootScanReport {
    root_kind: &'static str,
    root_type: String,
    rows_returned: usize,
}

#[derive(Debug, Clone)]
struct NativeProjectionRootScanReport {
    projection_id: String,
    rows_returned: usize,
}

#[derive(Debug, Clone)]
struct NativeDirectProjectionReport {
    root_kind: &'static str,
    column_count: usize,
    rows_projected: usize,
}

impl ExecutionCodeResolver for StaticExecutionCodeResolver<'_> {
    fn resolve(&self, request: ExecutionCodeRequest<'_>) -> Result<ExecutionCodeValue, CoveError> {
        self.filecode_to_execution
            .get(&request.file_code)
            .copied()
            .map(ExecutionCodeValue::Unsigned)
            .ok_or(CoveError::ExecutionCodeMap)
    }
}

fn try_coverage_row_range_prune(
    physical: &PhysicalPlannedQuery,
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
) -> Result<Option<CoverageRowRangePrune>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if physical.proof_validation_report.accepted_proof_count == 0
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.method_chain.where_predicate.is_none()
        || matches!(
            planned.resolved.output_mode,
            crate::CoveQlOutputMode::DataFusionTableProvider | crate::CoveQlOutputMode::ExplainJson
        )
    {
        return Ok(None);
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    if root.object_type_id != root_object_type_id {
        return Ok(None);
    }
    let Some(set_bytes) = physical.sidecars.coverage_set_bytes.as_deref() else {
        return Ok(None);
    };
    let Some(proof_bytes) = physical.sidecars.coverage_proof_record_bytes.as_deref() else {
        return Ok(None);
    };
    let set = CoverageSetV2::parse(set_bytes).map_err(|error| {
        exec_error(
            "E_COVERAGE_PRUNE_UNSAFE",
            format!("coverage set could not be parsed for execution pruning: {error}"),
            json!({}),
        )
    })?;
    if set.header.granularity != CoverageGranularityV2::RowRange
        || !can_use_for_pruning(&set.header)
        || !physical_predicate_form_exists(physical, set.header.predicate_form_ref)
    {
        return Ok(None);
    }
    let proof_records = CoverageProofRecordV2::parse_many(proof_bytes).map_err(|error| {
        exec_error(
            "E_COVERAGE_PRUNE_UNSAFE",
            format!("coverage proof records could not be parsed for execution pruning: {error}"),
            json!({}),
        )
    })?;
    let proof_ok = proof_records.iter().any(|record| {
        can_use_proof_for_pruning(record)
            && record
                .validate_against_coverage_set_bytes(set_bytes)
                .is_ok()
    });
    if !proof_ok {
        return Ok(None);
    }
    let bitmap = coverage_row_range_bitmap(surface, root.object_type_id, &set.entries)?;
    Ok(Some(CoverageRowRangePrune {
        matched_rows: bitmap.count_ones(),
        row_range_count: set.entries.len(),
        predicate_form_ref: set.header.predicate_form_ref,
        bitmap,
    }))
}

fn physical_predicate_form_exists(physical: &PhysicalPlannedQuery, form_id: u32) -> bool {
    let forms = &physical.physical_plan.predicate_normal_forms;
    forms
        .ast_forms
        .iter()
        .chain(forms.cnf_forms.iter())
        .chain(forms.interval_forms.iter())
        .chain(forms.encoded_forms.iter())
        .chain(forms.coverage_forms.iter())
        .chain(forms.residual_forms.iter())
        .any(|form| form.form_id == form_id)
}

fn coverage_row_range_bitmap(
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
    entries: &[CoverageSetEntryV2],
) -> Result<SelectionBitmap, BuildExecutionError> {
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    for entry in entries {
        if entry.target_kind != CoverageGranularityV2::RowRange
            || entry.file_ref != 0
            || !matches!(entry.table_id, 0 | u32::MAX)
            || entry.object_type_id != root_object_type_id
        {
            return Err(exec_error(
                "E_COVERAGE_PRUNE_UNSAFE",
                "coverage row-range entry did not match the loaded COVE-O object row domain",
                json!({
                    "target_kind": format!("{:?}", entry.target_kind),
                    "file_ref": entry.file_ref,
                    "table_id": entry.table_id,
                    "object_type_id": entry.object_type_id,
                    "root_object_type_id": root_object_type_id,
                }),
            ));
        }
    }
    for row in 0..surface.system.len() {
        if surface.system.object_type_ids[row] != root_object_type_id {
            continue;
        }
        if entries.iter().any(|entry| {
            coverage_row_range_contains_loaded_record(
                entry,
                surface.system.segment_ids[row],
                surface.system.row_indices[row],
            )
        }) {
            bitmap.set(row);
        }
    }
    Ok(bitmap)
}

fn coverage_row_range_contains_loaded_record(
    entry: &CoverageSetEntryV2,
    segment_id: u32,
    row_index: u32,
) -> bool {
    if entry.segment_id != segment_id {
        return false;
    }
    let row_index = u64::from(row_index);
    row_index >= entry.row_start && row_index < entry.row_start.saturating_add(entry.row_count)
}

fn try_covi_row_range_prune(
    physical: &PhysicalPlannedQuery,
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
) -> Result<Option<CoviRowRangePrune>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if physical.index_capability_report.lookup_candidates == 0
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || matches!(
            planned.resolved.output_mode,
            crate::CoveQlOutputMode::DataFusionTableProvider | crate::CoveQlOutputMode::ExplainJson
        )
    {
        return Ok(None);
    }
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Ok(None);
    };
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    if root.object_type_id != root_object_type_id {
        return Ok(None);
    }
    let requests = covi_empty_lookup_requests(predicate, root.object_type_id)?;
    if requests.is_empty() {
        return Ok(None);
    }

    let mut best: Option<CoviRowRangePrune> = None;
    for (source, bytes) in [
        ("cove_i", physical.sidecars.covi_artifact_bytes.as_deref()),
        ("covx", physical.sidecars.covx_artifact_bytes.as_deref()),
    ]
    .into_iter()
    .filter_map(|(source, bytes)| bytes.map(|bytes| (source, bytes)))
    {
        let context = CoviValidationContextV2::for_file(
            planned.resolved.operation_context.file.file_id,
            planned.resolved.operation_context.file.file_len,
            planned.resolved.operation_context.file.footer_crc32c,
        )
        .with_file_code_keys(physical.allow_file_code_literal_candidates);
        let Ok(artifact) = ValidatedCoviArtifactV2::parse_and_validate(bytes, context) else {
            continue;
        };
        let mut combined: Option<SelectionBitmap> = None;
        let mut lookup_count = 0usize;
        let mut row_range_count = 0usize;
        for request in &requests {
            let candidates = match artifact.lookup(request) {
                Ok(candidates) => candidates,
                Err(CoveError::IndexOnlyUnsafe) => {
                    return Err(exec_error(
                        "E_COVI_LOOKUP_UNSAFE",
                        "COVI/COVX lookup metadata matched the target but did not prove an exact safe candidate set",
                        json!({ "source": source }),
                    ))
                }
                Err(_) => continue,
            };
            let Some(bitmap) =
                exact_covi_row_range_bitmap(surface, root.object_type_id, &candidates)
            else {
                continue;
            };
            lookup_count += 1;
            row_range_count += candidates.row_ranges.len();
            if let Some(existing) = &mut combined {
                let _ = existing.intersect_with_dispatch(&bitmap, NativeKernelDispatch::Auto);
            } else {
                combined = Some(bitmap);
            }
        }
        let Some(bitmap) = combined else {
            continue;
        };
        let matched_rows = bitmap.count_ones();
        let candidate = CoviRowRangePrune {
            bitmap,
            source,
            lookup_count,
            row_range_count,
            matched_rows,
        };
        let replace = best
            .as_ref()
            .is_none_or(|current| candidate.matched_rows < current.matched_rows);
        if replace {
            best = Some(candidate);
        }
    }
    Ok(best)
}

fn exact_covi_row_range_bitmap(
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
    candidates: &CoviCandidateSetV2,
) -> Option<SelectionBitmap> {
    if candidates.exactness != IndexCapabilityExactnessV2::Exact
        || !candidates.proof_strength.allows_pruning()
        || candidates.row_ranges.is_empty()
        || !candidates.byte_ranges.is_empty()
        || !candidates.object_paths.is_empty()
        || !candidates.dimensional_buckets.is_empty()
        || !candidates.row_ordinal_set_refs.is_empty()
        || !candidates.file_refs.is_empty()
        || !candidates.segment_refs.is_empty()
        || !candidates.morsel_refs.is_empty()
        || !candidates.page_refs.is_empty()
    {
        return None;
    }
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    for row in 0..surface.system.len() {
        if surface.system.object_type_ids[row] != root_object_type_id {
            continue;
        }
        if candidates.row_ranges.iter().any(|range| {
            covi_row_range_contains_loaded_record(
                range,
                surface.system.segment_ids[row],
                surface.system.row_indices[row],
            )
        }) {
            bitmap.set(row);
        }
    }
    Some(bitmap)
}

fn covi_row_range_contains_loaded_record(
    range: &CoviRowRangePostingV2,
    segment_id: u32,
    row_index: u32,
) -> bool {
    if range.file_ref != 0 || range.table_id != u32::MAX || range.segment_id != segment_id {
        return false;
    }
    let row_index = u64::from(row_index);
    row_index >= range.row_start && row_index < range.row_start.saturating_add(range.row_count)
}

fn try_execution_code_filecode_prune(
    bytes: &[u8],
    physical: &PhysicalPlannedQuery,
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
    options: &ExecutionOptions,
) -> Result<Option<ExecutionCodePrune>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if !planned
        .resolved
        .operation_context
        .request
        .execution_code_mapping_requested
        || options.execution_code_filecode_map.is_empty()
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || matches!(
            planned.resolved.output_mode,
            crate::CoveQlOutputMode::DataFusionTableProvider | crate::CoveQlOutputMode::ExplainJson
        )
        || !physical
            .physical_plan
            .execution_code_domains
            .iter()
            .any(|domain| domain.validated)
    {
        return Ok(None);
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    if root.object_type_id != root_object_type_id {
        return Ok(None);
    }
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Ok(None);
    };
    let requests = execution_code_predicate_requests(predicate, root.object_type_id)?;
    if requests.is_empty() {
        return Ok(None);
    }

    let resolver = StaticExecutionCodeResolver {
        filecode_to_execution: &options.execution_code_filecode_map,
    };
    let mounted = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::MapToExecutionCode,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            covx: None,
            covm: None,
        },
        Some(&resolver),
    )
    .map_err(|error| {
        exec_error(
            "E_EXECUTION_CODE_MAP",
            format!("COVE-E execution-code remap could not be mounted: {error}"),
            json!({}),
        )
    })?;
    if mounted.execution_descriptors.is_empty()
        || mounted.code_spaces.is_empty()
        || mounted.engine_mount_policies.is_empty()
    {
        return Ok(None);
    }
    if !mounted.engine_mount_policies.iter().any(|policy| {
        policy.filecode_mapping_kind
            == cove_core::profile::cove_e::FileCodeMappingKind::MapToExecutionCode
    }) {
        return Ok(None);
    }
    let Some(dictionary) = mounted.dictionary.as_ref() else {
        return Ok(None);
    };
    let Some(execution_map) = mounted.execution_code_map.as_ref() else {
        return Ok(None);
    };

    let mut combined: Option<SelectionBitmap> = None;
    let mut total_pages = 0usize;
    for request in &requests {
        let mut target_codes = BTreeSet::new();
        for key in &request.canonical_literals {
            let Some(file_code) =
                file_code_for_canonical(dictionary, key, "E_EXECUTION_CODE_MAP", "COVE-E")?
            else {
                if request.match_mode == ExecutionCodeMatchMode::Include {
                    return Ok(Some(ExecutionCodePrune {
                        bitmap: SelectionBitmap::none(surface.system.len()),
                        predicate_count: requests.len(),
                        page_count: total_pages,
                        matched_rows: 0,
                    }));
                }
                continue;
            };
            let execution_code = unsigned_execution_code(execution_map, file_code)?;
            target_codes.insert(execution_code);
        }
        let (bitmap, page_count) = execution_code_bitmap_for_request(
            bytes,
            surface,
            root.object_type_id,
            request,
            &target_codes,
            execution_map,
        )?;
        if page_count == 0 {
            return Ok(None);
        }
        total_pages += page_count;
        if let Some(existing) = &mut combined {
            let _ = existing.intersect_with_dispatch(&bitmap, NativeKernelDispatch::Auto);
        } else {
            combined = Some(bitmap);
        }
    }
    let Some(bitmap) = combined else {
        return Ok(None);
    };
    Ok(Some(ExecutionCodePrune {
        matched_rows: bitmap.count_ones(),
        bitmap,
        predicate_count: requests.len(),
        page_count: total_pages,
    }))
}

fn try_file_code_literal_prune(
    bytes: &[u8],
    physical: &PhysicalPlannedQuery,
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
) -> Result<Option<FileCodeLiteralPrune>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if planned.resolved.operation_context.dataset.files.len() > 1
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || matches!(
            planned.resolved.output_mode,
            crate::CoveQlOutputMode::DataFusionTableProvider | crate::CoveQlOutputMode::ExplainJson
        )
    {
        return Ok(None);
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    if root.object_type_id != root_object_type_id {
        return Ok(None);
    }
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Ok(None);
    };
    let requests = execution_code_predicate_requests(predicate, root.object_type_id)?;
    if requests.is_empty() {
        return Ok(None);
    }

    let mounted = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            covx: None,
            covm: None,
        },
        None,
    )
    .map_err(|error| {
        exec_error(
            "E_FILE_CODE_LITERAL_PRUNE",
            format!("single-file FileCode literal prune could not mount dictionary: {error}"),
            json!({}),
        )
    })?;
    let Some(dictionary) = mounted.dictionary.as_ref() else {
        return Ok(None);
    };

    let mut combined: Option<SelectionBitmap> = None;
    let mut total_pages = 0usize;
    for request in &requests {
        let mut target_codes = BTreeSet::new();
        for key in &request.canonical_literals {
            let Some(file_code) = file_code_for_canonical(
                dictionary,
                key,
                "E_FILE_CODE_LITERAL_PRUNE",
                "single-file FileCode literal prune",
            )?
            else {
                if request.match_mode == ExecutionCodeMatchMode::Include {
                    return Ok(Some(FileCodeLiteralPrune {
                        bitmap: SelectionBitmap::none(surface.system.len()),
                        predicate_count: requests.len(),
                        page_count: total_pages,
                        matched_rows: 0,
                    }));
                }
                continue;
            };
            target_codes.insert(file_code);
        }
        let (bitmap, page_count) = file_code_bitmap_for_request(
            bytes,
            surface,
            root.object_type_id,
            request,
            &target_codes,
        )?;
        if page_count == 0 {
            return Ok(None);
        }
        total_pages += page_count;
        if let Some(existing) = &mut combined {
            let _ = existing.intersect_with_dispatch(&bitmap, NativeKernelDispatch::Auto);
        } else {
            combined = Some(bitmap);
        }
    }
    let Some(bitmap) = combined else {
        return Ok(None);
    };
    Ok(Some(FileCodeLiteralPrune {
        matched_rows: bitmap.count_ones(),
        bitmap,
        predicate_count: requests.len(),
        page_count: total_pages,
    }))
}

fn try_native_scalar_predicate_prune(
    bytes: &[u8],
    retained_input: Option<&CoveQlRetainedInput>,
    physical: &PhysicalPlannedQuery,
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
    predicates: &[KernelPredicate],
) -> Result<Option<NativeScalarPredicatePrune>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if planned.resolved.operation_context.dataset.files.len() > 1
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || matches!(
            planned.resolved.output_mode,
            crate::CoveQlOutputMode::DataFusionTableProvider | crate::CoveQlOutputMode::ExplainJson
        )
    {
        return Ok(None);
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    if root.object_type_id != root_object_type_id {
        return Ok(None);
    }

    let mut requests = predicates
        .iter()
        .filter_map(|predicate| native_scalar_request_for_predicate(predicate, root_object_type_id))
        .collect::<Vec<_>>();
    if requests.is_empty() {
        return Ok(None);
    }
    order_native_scalar_requests_by_cost(&mut requests);

    let loaded_rows = surface_row_lookup_for_object(surface, root_object_type_id);

    if let Some(input) = retained_input {
        match read_retained_object_temporal_segments(
            input.retained_bytes(),
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        ) {
            Ok(retained) => {
                return native_scalar_prune_from_retained_segments(
                    &retained.segments,
                    surface,
                    &loaded_rows,
                    &requests,
                );
            }
            Err(CoveError::UnsupportedEncoding(_)) => {}
            Err(_) => {}
        }
    }

    let segments = temporal_segments_from_bytes_with_context(
        bytes,
        "E_NATIVE_SCALAR_LITERAL_PRUNE",
        "native scalar literal prune",
    )?;
    native_scalar_prune_from_segments(&segments, surface, &loaded_rows, &requests)
}

fn native_scalar_request_for_predicate(
    predicate: &KernelPredicate,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    match predicate {
        KernelPredicate::BoolPath { path } => {
            native_bool_eq_request(path, true, root_object_type_id)
        }
        KernelPredicate::Not(inner) => match inner.as_ref() {
            KernelPredicate::BoolPath { path } => {
                native_bool_eq_request(path, false, root_object_type_id)
            }
            KernelPredicate::InList { path, literals } => {
                native_not_in_list_request(path, literals, root_object_type_id)
            }
            KernelPredicate::ComparePathLiteral { path, op, literal } => {
                native_negated_compare_request(path, *op, literal, root_object_type_id)
            }
            _ => None,
        },
        KernelPredicate::NullCheck { path, negated } => {
            native_null_check_request(path, *negated, root_object_type_id)
        }
        KernelPredicate::ComparePathLiteral { path, op, literal }
            if matches!(
                op,
                AstCompareOp::Eq
                    | AstCompareOp::Ne
                    | AstCompareOp::Lt
                    | AstCompareOp::Le
                    | AstCompareOp::Gt
                    | AstCompareOp::Ge
            ) =>
        {
            native_compare_request(path, *op, literal, root_object_type_id)
        }
        KernelPredicate::InList { path, literals } => {
            native_in_list_request(path, literals, root_object_type_id)
        }
        KernelPredicate::Or(parts) => native_or_request(parts, root_object_type_id),
        KernelPredicate::StartsWithPathLiteral { path, prefix } => {
            let (object_type_id, property_id, logical_type) =
                native_object_property_path(path, root_object_type_id)?;
            if !matches!(logical_type, CoveLogicalType::Utf8)
                || path.physical_kind.as_str() != "var_bytes"
            {
                return None;
            }
            Some(NativeScalarPredicateRequest::VarBytesPrefix {
                object_type_id,
                property_id,
                logical_type,
                prefix: prefix.as_bytes().to_vec(),
            })
        }
        _ => None,
    }
}

fn native_or_request(
    parts: &[KernelPredicate],
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let mut path: Option<&ResolvedPath> = None;
    let mut literals = Vec::with_capacity(parts.len());
    for part in parts {
        let KernelPredicate::ComparePathLiteral {
            path: part_path,
            op,
            literal,
        } = part
        else {
            return None;
        };
        if *op != AstCompareOp::Eq {
            return None;
        }
        if path.is_some_and(|path| !same_native_path(path, part_path)) {
            return None;
        }
        path = Some(part_path);
        literals.push(literal.clone());
    }
    native_in_list_request(path?, &literals, root_object_type_id)
}

fn native_negated_compare_request(
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let inverted = match op {
        AstCompareOp::Eq => return native_not_equal_request(path, literal, root_object_type_id),
        AstCompareOp::Ne => AstCompareOp::Eq,
        AstCompareOp::Lt => AstCompareOp::Ge,
        AstCompareOp::Le => AstCompareOp::Gt,
        AstCompareOp::Gt => AstCompareOp::Le,
        AstCompareOp::Ge => AstCompareOp::Lt,
    };
    native_compare_request(path, inverted, literal, root_object_type_id)
}

fn native_in_list_request(
    path: &ResolvedPath,
    literals: &[KernelLiteral],
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    match path.physical_kind.as_str() {
        "boolean" if logical_type == CoveLogicalType::Bool => {
            let (has_true, has_false) = native_bool_literals_for_kernel(literals)?;
            match (has_true, has_false) {
                (true, true) => Some(NativeScalarPredicateRequest::NullCheck {
                    object_type_id,
                    property_id,
                    want_valid: true,
                }),
                (true, false) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: true,
                }),
                (false, true) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: false,
                }),
                (false, false) => None,
            }
        }
        "num_code" => {
            let literals = literals
                .iter()
                .map(native_numeric_literal_for_kernel)
                .collect::<Option<Vec<_>>>()?;
            (!literals.is_empty()).then_some(NativeScalarPredicateRequest::NumCodeIn {
                object_type_id,
                property_id,
                logical_type,
                literals,
            })
        }
        "fixed_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_fixed_bytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::FixedBytesIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        "var_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_varbytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::VarBytesIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        _ => None,
    }
}

fn native_not_equal_request(
    path: &ResolvedPath,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    match (path.physical_kind.as_str(), logical_type, literal) {
        ("boolean", CoveLogicalType::Bool, KernelLiteral::Bool(value)) => {
            Some(NativeScalarPredicateRequest::BoolEq {
                object_type_id,
                property_id,
                value: !*value,
            })
        }
        ("num_code", _, _) => {
            let literal = native_numeric_literal_for_kernel(literal)?;
            Some(NativeScalarPredicateRequest::NumCodeNotIn {
                object_type_id,
                property_id,
                logical_type,
                literals: vec![literal],
            })
        }
        ("fixed_bytes", CoveLogicalType::Uuid, KernelLiteral::String(_)) => {
            let value = native_fixed_bytes_literal_for_kernel(logical_type, literal)?;
            Some(NativeScalarPredicateRequest::FixedBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values: value,
            })
        }
        ("var_bytes", CoveLogicalType::Utf8, KernelLiteral::String(_)) => {
            let value = native_varbytes_literal_for_kernel(logical_type, literal)?;
            Some(NativeScalarPredicateRequest::VarBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values: vec![value],
            })
        }
        _ => None,
    }
}

fn native_not_in_list_request(
    path: &ResolvedPath,
    literals: &[KernelLiteral],
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    if literals
        .iter()
        .any(|literal| matches!(literal, KernelLiteral::Null))
    {
        return None;
    }
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    match path.physical_kind.as_str() {
        "boolean" if logical_type == CoveLogicalType::Bool => {
            let (has_true, has_false) = native_bool_literals_for_kernel(literals)?;
            match (has_true, has_false) {
                (true, false) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: false,
                }),
                (false, true) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: true,
                }),
                _ => None,
            }
        }
        "num_code" => {
            let literals = literals
                .iter()
                .map(native_numeric_literal_for_kernel)
                .collect::<Option<Vec<_>>>()?;
            (!literals.is_empty()).then_some(NativeScalarPredicateRequest::NumCodeNotIn {
                object_type_id,
                property_id,
                logical_type,
                literals,
            })
        }
        "fixed_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_fixed_bytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::FixedBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        "var_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_varbytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::VarBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        _ => None,
    }
}

fn same_native_path(left: &ResolvedPath, right: &ResolvedPath) -> bool {
    left.root_kind == right.root_kind
        && left.object_type_id == right.object_type_id
        && left.property_id == right.property_id
        && left.system_field == right.system_field
        && left.logical_type == right.logical_type
        && left.physical_kind == right.physical_kind
}

fn native_bool_literals_for_kernel(literals: &[KernelLiteral]) -> Option<(bool, bool)> {
    let mut has_true = false;
    let mut has_false = false;
    for literal in literals {
        match literal {
            KernelLiteral::Bool(true) => has_true = true,
            KernelLiteral::Bool(false) => has_false = true,
            KernelLiteral::Null => {}
            _ => return None,
        }
    }
    Some((has_true, has_false))
}

fn native_compare_request(
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    if op == AstCompareOp::Ne {
        return native_not_equal_request(path, literal, root_object_type_id);
    }
    if let Some(request) = native_system_compare_request(path, op, literal, root_object_type_id) {
        return Some(request);
    }
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    if path.physical_kind.as_str() == "num_code" {
        let op = native_numeric_op_for_ast(op)?;
        let literal = native_numeric_literal_for_kernel(literal)?;
        return Some(NativeScalarPredicateRequest::NumCode {
            object_type_id,
            property_id,
            logical_type,
            op,
            literal,
        });
    }
    if op != AstCompareOp::Eq {
        return None;
    }
    match (path.physical_kind.as_str(), logical_type, literal) {
        ("boolean", CoveLogicalType::Bool, KernelLiteral::Bool(value)) => {
            Some(NativeScalarPredicateRequest::BoolEq {
                object_type_id,
                property_id,
                value: *value,
            })
        }
        ("fixed_bytes", CoveLogicalType::Uuid, KernelLiteral::String(value)) => {
            let value = decode_compact_hex_bytes(value).filter(|value| value.len() == 16)?;
            Some(NativeScalarPredicateRequest::FixedBytesEq {
                object_type_id,
                property_id,
                logical_type,
                value,
            })
        }
        ("var_bytes", CoveLogicalType::Utf8, KernelLiteral::String(value)) => {
            Some(NativeScalarPredicateRequest::VarBytesEq {
                object_type_id,
                property_id,
                logical_type,
                value: value.as_bytes().to_vec(),
            })
        }
        _ => None,
    }
}

fn native_system_compare_request(
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    if !matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
        || path.object_type_id != Some(root_object_type_id)
    {
        return None;
    }
    match path.system_field.as_ref()? {
        ResolvedSystemField::BranchKey => Some(NativeScalarPredicateRequest::SystemNumeric {
            object_type_id: root_object_type_id,
            field: NativeSystemNumericField::BranchKey,
            op: native_numeric_op_for_ast(op)?,
            literal: native_numeric_literal_for_kernel(literal)?,
        }),
        ResolvedSystemField::Csn => Some(NativeScalarPredicateRequest::SystemNumeric {
            object_type_id: root_object_type_id,
            field: NativeSystemNumericField::Csn,
            op: native_numeric_op_for_ast(op)?,
            literal: native_numeric_literal_for_kernel(literal)?,
        }),
        ResolvedSystemField::TimestampUs => Some(NativeScalarPredicateRequest::SystemNumeric {
            object_type_id: root_object_type_id,
            field: NativeSystemNumericField::TimestampUs,
            op: native_numeric_op_for_ast(op)?,
            literal: native_numeric_literal_for_kernel(literal)?,
        }),
        ResolvedSystemField::Goid if op == AstCompareOp::Eq => {
            let KernelLiteral::String(value) = literal else {
                return None;
            };
            let value: [u8; 16] = decode_compact_hex_bytes(value)?.try_into().ok()?;
            Some(NativeScalarPredicateRequest::SystemGoidEq {
                object_type_id: root_object_type_id,
                value,
            })
        }
        _ => None,
    }
}

fn native_numeric_op_for_ast(op: AstCompareOp) -> Option<NativeNumericPredicateOp> {
    match op {
        AstCompareOp::Eq => Some(NativeNumericPredicateOp::Eq),
        AstCompareOp::Lt => Some(NativeNumericPredicateOp::Lt),
        AstCompareOp::Le => Some(NativeNumericPredicateOp::LtEq),
        AstCompareOp::Gt => Some(NativeNumericPredicateOp::Gt),
        AstCompareOp::Ge => Some(NativeNumericPredicateOp::GtEq),
        AstCompareOp::Ne => None,
    }
}

fn native_numeric_literal_for_kernel(literal: &KernelLiteral) -> Option<NativeNumericLiteral> {
    match literal {
        KernelLiteral::I64(value) => Some(NativeNumericLiteral::Int64(*value)),
        KernelLiteral::U64(value) => Some(NativeNumericLiteral::UInt64(*value)),
        KernelLiteral::F64(value) => Some(NativeNumericLiteral::Float64(*value)),
        _ => None,
    }
}

fn native_fixed_bytes_literal_for_kernel(
    logical_type: CoveLogicalType,
    literal: &KernelLiteral,
) -> Option<Vec<u8>> {
    match (logical_type, literal) {
        (CoveLogicalType::Uuid, KernelLiteral::String(value)) => {
            decode_compact_hex_bytes(value).filter(|value| value.len() == 16)
        }
        _ => None,
    }
}

fn native_varbytes_literal_for_kernel(
    logical_type: CoveLogicalType,
    literal: &KernelLiteral,
) -> Option<Vec<u8>> {
    match (logical_type, literal) {
        (CoveLogicalType::Utf8, KernelLiteral::String(value)) => Some(value.as_bytes().to_vec()),
        _ => None,
    }
}

fn native_bool_eq_request(
    path: &ResolvedPath,
    value: bool,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    (path.physical_kind.as_str() == "boolean" && logical_type == CoveLogicalType::Bool).then_some(
        NativeScalarPredicateRequest::BoolEq {
            object_type_id,
            property_id,
            value,
        },
    )
}

fn native_null_check_request(
    path: &ResolvedPath,
    want_valid: bool,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, _) = native_object_property_path(path, root_object_type_id)?;
    Some(NativeScalarPredicateRequest::NullCheck {
        object_type_id,
        property_id,
        want_valid,
    })
}

fn native_object_property_path(
    path: &ResolvedPath,
    root_object_type_id: u32,
) -> Option<(u32, u32, CoveLogicalType)> {
    if !matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
        || path.system_field.is_some()
        || path.object_type_id != Some(root_object_type_id)
    {
        return None;
    }
    let object_type_id = path.object_type_id?;
    let property_id = path.property_id?;
    let logical_type = logical_type_to_cove(path.logical_type.as_str())?;
    Some((object_type_id, property_id, logical_type))
}

fn native_scalar_prune_from_segments(
    segments: &[TemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    requests: &[NativeScalarPredicateRequest],
) -> Result<Option<NativeScalarPredicatePrune>, BuildExecutionError> {
    let mut combined: Option<SelectionBitmap> = None;
    let mut bitmap_dispatch = NativeBitmapDispatchCounts::default();
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    let mut total_pages = 0usize;
    let mut executed_predicate_count = 0usize;
    let mut short_circuited = false;
    for request in requests {
        if combined.as_ref().is_some_and(SelectionBitmap::all_zero) {
            short_circuited = true;
            break;
        }
        let Some(scan) = native_scalar_bitmap_for_request(segments, surface, loaded_rows, request)?
        else {
            return Ok(None);
        };
        executed_predicate_count += 1;
        total_pages += scan.page_count;
        predicate_dispatch.merge(scan.predicate_dispatch);
        if let Some(existing) = &mut combined {
            let dispatch =
                existing.intersect_with_dispatch(&scan.bitmap, NativeKernelDispatch::Auto);
            bitmap_dispatch.record(dispatch);
        } else {
            combined = Some(scan.bitmap);
        }
    }
    Ok(combined.map(|bitmap| NativeScalarPredicatePrune {
        matched_rows: bitmap.count_ones(),
        bitmap,
        predicate_count: requests.len(),
        executed_predicate_count,
        page_count: total_pages,
        predicate_order: requests
            .iter()
            .map(NativeScalarPredicateRequest::kind_name)
            .collect(),
        predicate_dispatch,
        bitmap_dispatch,
        retained_page_buffers: false,
        short_circuited,
    }))
}

fn native_scalar_prune_from_retained_segments(
    segments: &[RetainedTemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    requests: &[NativeScalarPredicateRequest],
) -> Result<Option<NativeScalarPredicatePrune>, BuildExecutionError> {
    let mut combined: Option<SelectionBitmap> = None;
    let mut bitmap_dispatch = NativeBitmapDispatchCounts::default();
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    let mut total_pages = 0usize;
    let mut executed_predicate_count = 0usize;
    let mut short_circuited = false;
    for request in requests {
        if combined.as_ref().is_some_and(SelectionBitmap::all_zero) {
            short_circuited = true;
            break;
        }
        let Some(scan) =
            native_scalar_bitmap_for_retained_request(segments, surface, loaded_rows, request)?
        else {
            return Ok(None);
        };
        executed_predicate_count += 1;
        total_pages += scan.page_count;
        predicate_dispatch.merge(scan.predicate_dispatch);
        if let Some(existing) = &mut combined {
            let dispatch =
                existing.intersect_with_dispatch(&scan.bitmap, NativeKernelDispatch::Auto);
            bitmap_dispatch.record(dispatch);
        } else {
            combined = Some(scan.bitmap);
        }
    }
    Ok(combined.map(|bitmap| NativeScalarPredicatePrune {
        matched_rows: bitmap.count_ones(),
        bitmap,
        predicate_count: requests.len(),
        executed_predicate_count,
        page_count: total_pages,
        predicate_order: requests
            .iter()
            .map(NativeScalarPredicateRequest::kind_name)
            .collect(),
        predicate_dispatch,
        bitmap_dispatch,
        retained_page_buffers: true,
        short_circuited,
    }))
}

fn native_scalar_bitmap_for_request(
    segments: &[TemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
) -> Result<Option<NativeScalarBitmapScan>, BuildExecutionError> {
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    let mut page_count = 0usize;
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    for segment in segments {
        if segment.header.object_type_id != request.object_type_id() {
            continue;
        }
        let batch = native_object_temporal_batch_from_segment(
            segment,
            NativeCodeDomain {
                object_type_id: Some(segment.header.object_type_id),
                ..NativeCodeDomain::default()
            },
        )
        .map_err(|error| {
            exec_error(
                "E_NATIVE_SCALAR_LITERAL_PRUNE",
                format!("native scalar literal prune page binding failed: {error}"),
                json!({ "segment_id": segment.header.segment_id }),
            )
        })?;
        let Some(batch_scan) =
            native_scalar_bitmap_for_batch(&mut bitmap, loaded_rows, request, &batch)?
        else {
            return Ok(None);
        };
        page_count += batch_scan.page_count;
        predicate_dispatch.merge(batch_scan.predicate_dispatch);
    }
    Ok(Some(NativeScalarBitmapScan {
        bitmap,
        page_count,
        predicate_dispatch,
    }))
}

fn native_scalar_bitmap_for_retained_request(
    segments: &[RetainedTemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
) -> Result<Option<NativeScalarBitmapScan>, BuildExecutionError> {
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    let mut page_count = 0usize;
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    for segment in segments {
        if segment.header.object_type_id != request.object_type_id() {
            continue;
        }
        let batch = native_object_temporal_batch_from_retained_segment(
            segment,
            NativeCodeDomain {
                object_type_id: Some(segment.header.object_type_id),
                ..NativeCodeDomain::default()
            },
        )
        .map_err(|error| {
            exec_error(
                "E_NATIVE_SCALAR_LITERAL_PRUNE",
                format!("retained native scalar literal prune page binding failed: {error}"),
                json!({ "segment_id": segment.header.segment_id }),
            )
        })?;
        let Some(batch_scan) =
            native_scalar_bitmap_for_batch(&mut bitmap, loaded_rows, request, &batch)?
        else {
            return Ok(None);
        };
        page_count += batch_scan.page_count;
        predicate_dispatch.merge(batch_scan.predicate_dispatch);
    }
    Ok(Some(NativeScalarBitmapScan {
        bitmap,
        page_count,
        predicate_dispatch,
    }))
}

fn native_selection_from_kernel(
    result: Result<(SelectionBitmap, KernelStats), CoveError>,
) -> Result<(SelectionBitmap, NativeKernelDispatch), CoveError> {
    result.map(|(selected, stats)| (selected, stats.dispatch))
}

fn native_selection_from_bitmap(
    selected: SelectionBitmap,
) -> (SelectionBitmap, NativeKernelDispatch) {
    (selected, NativeKernelDispatch::Scalar)
}

fn native_scalar_bitmap_for_batch(
    bitmap: &mut SelectionBitmap,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
    batch: &NativeObjectTemporalBatch<'_>,
) -> Result<Option<NativeScalarBatchScan>, BuildExecutionError> {
    if let Some(scanned_units) =
        native_scalar_bitmap_for_system_batch(bitmap, loaded_rows, request, batch)?
    {
        return Ok(Some(scanned_units));
    }
    let mut page_count = 0usize;
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    for page in &batch.property_pages {
        if page.property_id != request.property_id() {
            continue;
        }
        page_count += 1;
        let (selected, dispatch) = match (request, &page.lane) {
            (NativeScalarPredicateRequest::NullCheck { want_valid, .. }, lane) => {
                native_validity_selection_for_lane(lane, page.row_count, *want_valid).ok_or(
                    CoveError::UnsupportedEncoding(
                        "native null check requires a validity-backed lane".into(),
                    ),
                )
            }
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::Bool {
                    values,
                    row_count,
                    validity,
                    ..
                },
            ) => native_selection_from_kernel(filter_bool_eq(
                values, *row_count, *validity, *value, None,
            )),
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                },
            ) if *logical_type == CoveLogicalType::Bool
                && *physical_kind == CovePhysicalKind::Boolean =>
            {
                Ok(local_u8_selection_for_targets(
                    values,
                    *validity,
                    local_to_global,
                    &[u64::from(u8::from(*value))],
                    true,
                ))
            }
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                },
            ) if *logical_type == CoveLogicalType::Bool
                && *physical_kind == CovePhysicalKind::Boolean =>
            {
                Ok(local_u16_selection_for_targets(
                    values,
                    *validity,
                    local_to_global,
                    &[u64::from(u8::from(*value))],
                    true,
                ))
            }
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                },
            ) if *logical_type == CoveLogicalType::Bool
                && *physical_kind == CovePhysicalKind::Boolean =>
            {
                Ok(local_u32_selection_for_targets(
                    values,
                    *validity,
                    local_to_global,
                    &[u64::from(u8::from(*value))],
                    true,
                ))
            }
            (
                NativeScalarPredicateRequest::FixedBytesEq {
                    logical_type,
                    value,
                    ..
                },
                LaneRef::FixedBytes {
                    values,
                    width,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_fixed_bytes_eq(values, *row_count, *width, *validity, value, None),
            ),
            (
                NativeScalarPredicateRequest::FixedBytesIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::FixedBytes {
                    values,
                    width,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_fixed_bytes_in(values, *row_count, *width, *validity, needles, None),
            ),
            (
                NativeScalarPredicateRequest::FixedBytesNotIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::FixedBytes {
                    values,
                    width,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => filter_fixed_bytes_in(
                values, *row_count, *width, *validity, needles, None,
            )
            .map(|(matched, stats)| {
                (
                    valid_rows_except(*row_count, *validity, &matched),
                    stats.dispatch,
                )
            }),
            (
                NativeScalarPredicateRequest::VarBytesEq {
                    logical_type,
                    value,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_varbytes_eq(row_offsets, values, *validity, value, None),
            ),
            (
                NativeScalarPredicateRequest::VarBytesIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                let needles = needles.iter().map(Vec::as_slice).collect::<Vec<_>>();
                native_selection_from_kernel(filter_varbytes_in(
                    row_offsets,
                    values,
                    *validity,
                    &needles,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::VarBytesNotIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                let needles = needles.iter().map(Vec::as_slice).collect::<Vec<_>>();
                filter_varbytes_in(row_offsets, values, *validity, &needles, None).map(
                    |(matched, stats)| {
                        (
                            valid_rows_except(row_offsets.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                NativeScalarPredicateRequest::VarBytesPrefix {
                    logical_type,
                    prefix,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_varbytes_prefix(row_offsets, values, *validity, prefix, None),
            ),
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::NumCodeU64LeBytes {
                    bytes,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                native_selection_from_kernel(filter_numcode_le_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    *op,
                    *literal,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::NumCodeU64LeBytes {
                    bytes,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                native_selection_from_kernel(filter_numcode_le_in_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    literals,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::NumCodeU64LeBytes {
                    bytes,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                native_selection_from_kernel(filter_numcode_le_not_in_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    literals,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_membership(local_to_global, *logical_type, *op, *literal).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u8_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_membership(local_to_global, *logical_type, *op, *literal).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u16_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_membership(local_to_global, *logical_type, *op, *literal).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u32_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u8_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (matched, stats) =
                            filter_local_u8_membership(values, *validity, &membership, None);
                        (
                            valid_rows_except(values.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u16_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (matched, stats) =
                            filter_local_u16_membership(values, *validity, &membership, None);
                        (
                            valid_rows_except(values.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u32_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (matched, stats) =
                            filter_local_u32_membership(values, *validity, &membership, None);
                        (
                            valid_rows_except(values.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                _,
                LaneRef::DecodeBoundary {
                    reason: "all-null elided page",
                    ..
                },
            ) => Ok(native_selection_from_bitmap(SelectionBitmap::none(
                page.row_count,
            ))),
            _ => return Ok(None),
        }
        .map_err(|error| {
            exec_error(
                "E_NATIVE_SCALAR_LITERAL_PRUNE",
                format!("native scalar literal prune page scan failed: {error}"),
                json!({ "segment_id": batch.segment_id }),
            )
        })?;
        predicate_dispatch.record(dispatch);

        for local_row in selected.to_selection_vector().rows() {
            set_native_scalar_prune_surface_row(
                bitmap,
                loaded_rows,
                batch.segment_id,
                page.row_start,
                *local_row as usize,
            )?;
        }
    }
    if page_count == 0 {
        return Ok(None);
    }
    Ok(Some(NativeScalarBatchScan {
        page_count,
        predicate_dispatch,
    }))
}

fn native_scalar_bitmap_for_system_batch(
    bitmap: &mut SelectionBitmap,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
    batch: &NativeObjectTemporalBatch<'_>,
) -> Result<Option<NativeScalarBatchScan>, BuildExecutionError> {
    if !matches!(
        request,
        NativeScalarPredicateRequest::SystemNumeric { .. }
            | NativeScalarPredicateRequest::SystemGoidEq { .. }
    ) {
        return Ok(None);
    }
    for (row_index, row) in batch.rows.iter().enumerate() {
        let matched = match request {
            NativeScalarPredicateRequest::SystemNumeric {
                field, op, literal, ..
            } => native_system_numeric_row_matches(row, *field, *op, *literal)?,
            NativeScalarPredicateRequest::SystemGoidEq { value, .. } => row.goid == *value,
            _ => unreachable!(),
        };
        if matched {
            set_native_scalar_prune_surface_row(
                bitmap,
                loaded_rows,
                batch.segment_id,
                0,
                row_index,
            )?;
        }
    }
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    predicate_dispatch.record(NativeKernelDispatch::Scalar);
    Ok(Some(NativeScalarBatchScan {
        page_count: 1,
        predicate_dispatch,
    }))
}

fn native_system_numeric_row_matches(
    row: &cove_core::profile::cove_o::TemporalRowEntryV1,
    field: NativeSystemNumericField,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<bool, BuildExecutionError> {
    let (logical_type, raw_value) = match field {
        NativeSystemNumericField::BranchKey => (CoveLogicalType::UInt64, row.branch_key),
        NativeSystemNumericField::Csn => (CoveLogicalType::UInt64, row.csn),
        NativeSystemNumericField::TimestampUs => {
            (CoveLogicalType::TimestampMicros, row.timestamp_us as u64)
        }
    };
    native_numcode_matches(logical_type, raw_value, op, literal).map_err(|error| {
        exec_error(
            "E_NATIVE_SCALAR_LITERAL_PRUNE",
            format!("native system row predicate failed: {error}"),
            json!({ "logical_type": format!("{logical_type:?}") }),
        )
    })
}

fn local_numcode_membership(
    local_to_global: &[u64],
    logical_type: CoveLogicalType,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<Vec<bool>, CoveError> {
    local_to_global
        .iter()
        .copied()
        .map(|value| native_numcode_matches(logical_type, value, op, literal))
        .collect()
}

fn local_numcode_in_membership(
    local_to_global: &[u64],
    logical_type: CoveLogicalType,
    literals: &[NativeNumericLiteral],
) -> Result<Vec<bool>, CoveError> {
    local_to_global
        .iter()
        .copied()
        .map(|value| {
            for literal in literals {
                if native_numcode_matches(
                    logical_type,
                    value,
                    NativeNumericPredicateOp::Eq,
                    *literal,
                )? {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .collect()
}

fn local_u8_selection_for_targets(
    values: &[u8],
    validity: ValidityRef<'_>,
    local_to_global: &[u64],
    target_codes: &[u64],
    include: bool,
) -> (SelectionBitmap, NativeKernelDispatch) {
    let membership = local_membership_u8(local_to_global, target_codes);
    let (matched, stats) = filter_local_u8_membership(values, validity, &membership, None);
    let selected = if include {
        matched
    } else {
        valid_rows_except(values.len(), validity, &matched)
    };
    (selected, stats.dispatch)
}

fn local_u16_selection_for_targets(
    values: &[u16],
    validity: ValidityRef<'_>,
    local_to_global: &[u64],
    target_codes: &[u64],
    include: bool,
) -> (SelectionBitmap, NativeKernelDispatch) {
    let membership = local_membership_u8(local_to_global, target_codes);
    let (matched, stats) = filter_local_u16_membership(values, validity, &membership, None);
    let selected = if include {
        matched
    } else {
        valid_rows_except(values.len(), validity, &matched)
    };
    (selected, stats.dispatch)
}

fn local_u32_selection_for_targets(
    values: &[u32],
    validity: ValidityRef<'_>,
    local_to_global: &[u64],
    target_codes: &[u64],
    include: bool,
) -> (SelectionBitmap, NativeKernelDispatch) {
    let membership = local_membership_u8(local_to_global, target_codes);
    let (matched, stats) = filter_local_u32_membership(values, validity, &membership, None);
    let selected = if include {
        matched
    } else {
        valid_rows_except(values.len(), validity, &matched)
    };
    (selected, stats.dispatch)
}

fn native_validity_selection_for_lane(
    lane: &LaneRef<'_>,
    page_row_count: usize,
    want_valid: bool,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if lane.row_count() != page_row_count {
        return None;
    }
    let validity = lane.validity()?;
    let (selected, stats) = filter_validity(page_row_count, validity, want_valid, None);
    Some((selected, stats.dispatch))
}

impl NativeScalarPredicateRequest {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::SystemNumeric { .. } => "system_numeric",
            Self::SystemGoidEq { .. } => "system_goid_eq",
            Self::NullCheck { .. } => "null_check",
            Self::BoolEq { .. } => "bool_eq",
            Self::FixedBytesEq { .. } => "fixed_bytes_eq",
            Self::FixedBytesIn { .. } => "fixed_bytes_in",
            Self::FixedBytesNotIn { .. } => "fixed_bytes_not_in",
            Self::VarBytesEq { .. } => "varbytes_eq",
            Self::VarBytesIn { .. } => "varbytes_in",
            Self::VarBytesNotIn { .. } => "varbytes_not_in",
            Self::VarBytesPrefix { .. } => "varbytes_prefix",
            Self::NumCode { .. } => "numcode",
            Self::NumCodeIn { .. } => "numcode_in",
            Self::NumCodeNotIn { .. } => "numcode_not_in",
        }
    }

    fn object_type_id(&self) -> u32 {
        match self {
            Self::SystemNumeric { object_type_id, .. }
            | Self::SystemGoidEq { object_type_id, .. }
            | Self::NullCheck { object_type_id, .. }
            | Self::BoolEq { object_type_id, .. }
            | Self::FixedBytesEq { object_type_id, .. }
            | Self::FixedBytesIn { object_type_id, .. }
            | Self::FixedBytesNotIn { object_type_id, .. }
            | Self::VarBytesEq { object_type_id, .. }
            | Self::VarBytesIn { object_type_id, .. }
            | Self::VarBytesNotIn { object_type_id, .. }
            | Self::VarBytesPrefix { object_type_id, .. }
            | Self::NumCode { object_type_id, .. }
            | Self::NumCodeIn { object_type_id, .. }
            | Self::NumCodeNotIn { object_type_id, .. } => *object_type_id,
        }
    }

    fn property_id(&self) -> u32 {
        match self {
            Self::NullCheck { property_id, .. }
            | Self::BoolEq { property_id, .. }
            | Self::FixedBytesEq { property_id, .. }
            | Self::FixedBytesIn { property_id, .. }
            | Self::FixedBytesNotIn { property_id, .. }
            | Self::VarBytesEq { property_id, .. }
            | Self::VarBytesIn { property_id, .. }
            | Self::VarBytesNotIn { property_id, .. }
            | Self::VarBytesPrefix { property_id, .. }
            | Self::NumCode { property_id, .. }
            | Self::NumCodeIn { property_id, .. }
            | Self::NumCodeNotIn { property_id, .. } => *property_id,
            Self::SystemNumeric { .. } | Self::SystemGoidEq { .. } => {
                unreachable!("system native scalar predicates do not have property ids")
            }
        }
    }
}

fn native_scalar_prune_covers_all_kernel_predicates(
    prune: &NativeScalarPredicatePrune,
    predicates: &[KernelPredicate],
) -> bool {
    native_scalar_prune_covers_kernel_predicate_count(prune, predicates.len())
}

fn native_scalar_prune_covers_kernel_predicate_count(
    prune: &NativeScalarPredicatePrune,
    predicate_count: usize,
) -> bool {
    predicate_count > 0
        && prune.page_count > 0
        && prune.predicate_count == predicate_count
        && (prune.executed_predicate_count == prune.predicate_count || prune.short_circuited)
}

fn code_prune_covers_kernel_predicate_count(
    prune_predicate_count: usize,
    prune_page_count: usize,
    predicate_count: usize,
) -> bool {
    predicate_count > 0 && prune_page_count > 0 && prune_predicate_count == predicate_count
}

fn order_native_scalar_requests_by_cost(requests: &mut [NativeScalarPredicateRequest]) -> bool {
    let mut indexed = requests.iter().cloned().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(index, request)| (native_scalar_request_cost_key(request), *index));
    let reordered = indexed
        .iter()
        .enumerate()
        .any(|(slot, (original, _))| slot != *original);
    for (slot, (_, request)) in requests.iter_mut().zip(indexed) {
        *slot = request;
    }
    reordered
}

fn native_scalar_request_cost_key(request: &NativeScalarPredicateRequest) -> (u8, u8) {
    match request {
        NativeScalarPredicateRequest::SystemNumeric { .. } => (0, 0),
        NativeScalarPredicateRequest::SystemGoidEq { .. } => (0, 1),
        NativeScalarPredicateRequest::NullCheck { .. } => (1, 0),
        NativeScalarPredicateRequest::BoolEq { .. } => (2, 0),
        NativeScalarPredicateRequest::NumCode { .. } => (3, 0),
        NativeScalarPredicateRequest::NumCodeIn { .. } => (3, 1),
        NativeScalarPredicateRequest::NumCodeNotIn { .. } => (3, 2),
        NativeScalarPredicateRequest::FixedBytesEq { .. } => (4, 0),
        NativeScalarPredicateRequest::FixedBytesIn { .. } => (4, 1),
        NativeScalarPredicateRequest::FixedBytesNotIn { .. } => (4, 2),
        NativeScalarPredicateRequest::VarBytesEq { .. } => (5, 0),
        NativeScalarPredicateRequest::VarBytesIn { .. } => (5, 1),
        NativeScalarPredicateRequest::VarBytesNotIn { .. } => (5, 2),
        NativeScalarPredicateRequest::VarBytesPrefix { .. } => (5, 3),
    }
}

fn surface_row_lookup_for_object(
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
) -> SurfaceRowLookup {
    let mut segment_rows = BTreeMap::<u32, Vec<Option<usize>>>::new();
    for row in 0..surface.system.len() {
        if surface.system.object_type_ids[row] == root_object_type_id {
            let segment_id = surface.system.segment_ids[row];
            let row_index = surface.system.row_indices[row] as usize;
            let rows = segment_rows.entry(segment_id).or_default();
            if rows.len() <= row_index {
                rows.resize(row_index + 1, None);
            }
            rows[row_index] = Some(row);
        }
    }
    SurfaceRowLookup::from_segment_rows(segment_rows)
}

fn set_native_scalar_prune_surface_row(
    bitmap: &mut SelectionBitmap,
    loaded_rows: &SurfaceRowLookup,
    segment_id: u32,
    page_row_start: usize,
    local_row: usize,
) -> Result<(), BuildExecutionError> {
    let row_index = page_row_start.checked_add(local_row).ok_or_else(|| {
        exec_error(
            "E_NATIVE_SCALAR_LITERAL_PRUNE",
            "native scalar literal prune page row offset overflowed",
            json!({}),
        )
    })?;
    let row_index = u32::try_from(row_index).map_err(|_| {
        exec_error(
            "E_NATIVE_SCALAR_LITERAL_PRUNE",
            "native scalar literal prune page row index exceeded u32",
            json!({}),
        )
    })?;
    if let Some(surface_row) = loaded_rows.get(segment_id, row_index) {
        bitmap.set(surface_row);
    }
    Ok(())
}

fn decode_compact_hex_bytes(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk).ok()?;
        out.push(u8::from_str_radix(text, 16).ok()?);
    }
    Some(out)
}

fn execution_code_predicate_requests(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
) -> Result<Vec<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let mut out = Vec::new();
    collect_execution_code_predicate_requests(predicate, root_object_type_id, &mut out)?;
    Ok(out)
}

fn collect_execution_code_predicate_requests(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
    out: &mut Vec<ExecutionCodePredicateRequest>,
) -> Result<(), BuildExecutionError> {
    match predicate {
        ResolvedPredicate::Compare { left, right, op }
            if matches!(*op, AstCompareOp::Eq | AstCompareOp::Ne) =>
        {
            if let Some(request) =
                execution_code_compare_request(left, *op, right, root_object_type_id)?
            {
                out.push(request);
            } else if let Some(request) =
                execution_code_compare_request(right, *op, left, root_object_type_id)?
            {
                out.push(request);
            }
        }
        ResolvedPredicate::Compare { .. } => {}
        ResolvedPredicate::InList { expr, values } => {
            if let Some(request) =
                execution_code_in_list_request(expr, values, root_object_type_id)?
            {
                out.push(request);
            }
        }
        ResolvedPredicate::NullCheck { .. } | ResolvedPredicate::Exists(_) => {}
        ResolvedPredicate::BoolExpr(expr) => {
            if let Some(request) = execution_code_bool_expr_request(expr, root_object_type_id)? {
                out.push(request);
            }
        }
        ResolvedPredicate::Not(inner) => {
            if let Some(request) = execution_code_not_request(inner, root_object_type_id)? {
                out.push(request);
            }
        }
        ResolvedPredicate::Or(parts) => {
            if let Some(request) = execution_code_same_path_or_request(parts, root_object_type_id)?
            {
                out.push(request);
            }
        }
        ResolvedPredicate::And(parts) => {
            for part in parts {
                collect_execution_code_predicate_requests(part, root_object_type_id, out)?;
            }
        }
    }
    Ok(())
}

fn execution_code_compare_request(
    left: &ResolvedExpr,
    op: AstCompareOp,
    right: &ResolvedExpr,
    root_object_type_id: u32,
) -> Result<Option<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let match_mode = match op {
        AstCompareOp::Eq => ExecutionCodeMatchMode::Include,
        AstCompareOp::Ne => ExecutionCodeMatchMode::Exclude,
        _ => return Ok(None),
    };
    let ResolvedExpr::Path(path) = left else {
        return Ok(None);
    };
    let ResolvedExpr::Literal(literal) = right else {
        return Ok(None);
    };
    let Some((object_type_id, property_id, logical_type)) =
        filecode_property_path_parts(path, root_object_type_id)
    else {
        return Ok(None);
    };
    let Some(canonical) = canonical_literal_key(&logical_type, literal)? else {
        return Ok(None);
    };
    Ok(Some(ExecutionCodePredicateRequest {
        object_type_id,
        property_id,
        logical_type,
        match_mode,
        canonical_literals: vec![canonical],
    }))
}

fn execution_code_in_list_request(
    expr: &ResolvedExpr,
    values: &[ResolvedLiteral],
    root_object_type_id: u32,
) -> Result<Option<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let ResolvedExpr::Path(path) = expr else {
        return Ok(None);
    };
    let Some((object_type_id, property_id, logical_type)) =
        filecode_property_path_parts(path, root_object_type_id)
    else {
        return Ok(None);
    };
    let mut canonical_literals = Vec::new();
    for literal in values {
        if matches!(literal.typed_value, ResolvedLiteralValue::Null) {
            continue;
        }
        let Some(canonical) = canonical_literal_key(&logical_type, literal)? else {
            return Ok(None);
        };
        canonical_literals.push(canonical);
    }
    normalize_canonical_literals(&mut canonical_literals);
    Ok(
        (!canonical_literals.is_empty()).then_some(ExecutionCodePredicateRequest {
            object_type_id,
            property_id,
            logical_type,
            match_mode: ExecutionCodeMatchMode::Include,
            canonical_literals,
        }),
    )
}

fn execution_code_same_path_or_request(
    parts: &[ResolvedPredicate],
    root_object_type_id: u32,
) -> Result<Option<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let mut combined: Option<ExecutionCodePredicateRequest> = None;
    for part in parts {
        let Some(request) = execution_code_or_atom_request(part, root_object_type_id)? else {
            return Ok(None);
        };
        if let Some(existing) = &mut combined {
            if existing.object_type_id != request.object_type_id
                || existing.property_id != request.property_id
                || existing.logical_type != request.logical_type
                || existing.match_mode != ExecutionCodeMatchMode::Include
                || request.match_mode != ExecutionCodeMatchMode::Include
            {
                return Ok(None);
            }
            existing
                .canonical_literals
                .extend(request.canonical_literals);
            normalize_canonical_literals(&mut existing.canonical_literals);
        } else {
            combined = Some(request);
        }
    }
    Ok(combined.filter(|request| !request.canonical_literals.is_empty()))
}

fn execution_code_or_atom_request(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
) -> Result<Option<ExecutionCodePredicateRequest>, BuildExecutionError> {
    match predicate {
        ResolvedPredicate::Compare { left, right, op }
            if matches!(*op, AstCompareOp::Eq | AstCompareOp::Ne) =>
        {
            if let Some(request) =
                execution_code_compare_request(left, *op, right, root_object_type_id)?
            {
                Ok(Some(request))
            } else {
                execution_code_compare_request(right, *op, left, root_object_type_id)
            }
        }
        ResolvedPredicate::InList { expr, values } => {
            execution_code_in_list_request(expr, values, root_object_type_id)
        }
        ResolvedPredicate::BoolExpr(expr) => {
            execution_code_bool_expr_request(expr, root_object_type_id)
        }
        ResolvedPredicate::Or(parts) => {
            execution_code_same_path_or_request(parts, root_object_type_id)
        }
        _ => Ok(None),
    }
}

fn execution_code_bool_expr_request(
    expr: &ResolvedExpr,
    root_object_type_id: u32,
) -> Result<Option<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let ResolvedExpr::Path(path) = expr else {
        return Ok(None);
    };
    let Some((object_type_id, property_id, logical_type)) =
        filecode_property_path_parts(path, root_object_type_id)
    else {
        return Ok(None);
    };
    if !matches!(logical_type.as_str(), "bool" | "boolean") {
        return Ok(None);
    }
    Ok(Some(ExecutionCodePredicateRequest {
        object_type_id,
        property_id,
        logical_type,
        match_mode: ExecutionCodeMatchMode::Include,
        canonical_literals: vec![execution_code_literal_key_from_canonical(
            CanonicalValue::Bool(true),
            "bool",
        )?],
    }))
}

fn execution_code_not_request(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
) -> Result<Option<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let Some(mut request) = execution_code_or_atom_request(predicate, root_object_type_id)? else {
        return Ok(None);
    };
    request.match_mode = request.match_mode.inverted();
    Ok(Some(request))
}

fn normalize_canonical_literals(canonical_literals: &mut Vec<ExecutionCodeLiteralKey>) {
    canonical_literals.sort();
    canonical_literals.dedup();
}

fn filecode_property_path_parts(
    path: &ResolvedPath,
    root_object_type_id: u32,
) -> Option<(u32, u32, String)> {
    let object_type_id = path.object_type_id?;
    let property_id = path.property_id?;
    if object_type_id != root_object_type_id || path.physical_kind != "file_code" {
        return None;
    }
    Some((object_type_id, property_id, path.logical_type.clone()))
}

fn canonical_literal_key(
    logical_type: &str,
    literal: &ResolvedLiteral,
) -> Result<Option<ExecutionCodeLiteralKey>, BuildExecutionError> {
    let Some(value) = canonical_value_for_resolved_literal(logical_type, literal)? else {
        return Ok(None);
    };
    execution_code_literal_key_from_canonical(value, logical_type).map(Some)
}

fn execution_code_literal_key_from_canonical(
    value: CanonicalValue<'_>,
    logical_type: &str,
) -> Result<ExecutionCodeLiteralKey, BuildExecutionError> {
    let value_tag = value.value_tag() as u16;
    let canonical = value.encode().map_err(|error| {
        exec_error(
            "E_EXECUTION_CODE_MAP",
            format!("literal could not be converted to a canonical COVE-E FileCode key: {error}"),
            json!({ "logical_type": logical_type }),
        )
    })?;
    Ok(ExecutionCodeLiteralKey {
        value_tag,
        canonical,
    })
}

fn file_code_for_canonical(
    dictionary: &cove_core::dictionary::FileDictionary,
    key: &ExecutionCodeLiteralKey,
    error_code: &'static str,
    source_label: &'static str,
) -> Result<Option<u32>, BuildExecutionError> {
    for file_code in 0..dictionary.len() {
        let entry = dictionary.get_entry(file_code).map_err(|error| {
            exec_error(
                error_code,
                format!("{source_label} dictionary entry lookup failed: {error}"),
                json!({ "file_code": file_code }),
            )
        })?;
        match dictionary.decode_value(file_code) {
            Ok(cove_core::dictionary::DictionaryValue::RawBytes(bytes))
                if entry.value_tag == key.value_tag && bytes == key.canonical =>
            {
                return Ok(Some(file_code));
            }
            Ok(cove_core::dictionary::DictionaryValue::RawBytes(_)) => {}
            Ok(cove_core::dictionary::DictionaryValue::RedactedPresent) => {
                return Err(exec_error(
                    error_code,
                    format!("{source_label} cannot use redacted dictionary values"),
                    json!({}),
                ));
            }
            Ok(_) => {
                return Err(exec_error(
                    error_code,
                    format!("{source_label} dictionary returned an unsupported dictionary value"),
                    json!({}),
                ));
            }
            Err(error) => {
                return Err(exec_error(
                    error_code,
                    format!("{source_label} dictionary lookup failed: {error}"),
                    json!({}),
                ));
            }
        }
    }
    Ok(None)
}

fn unsigned_execution_code(
    execution_map: &cove_core::mount::ExecutionCodeMap,
    file_code: u32,
) -> Result<u64, BuildExecutionError> {
    let value = execution_map
        .filecode_to_execution
        .get(file_code as usize)
        .ok_or_else(|| {
            exec_error(
                "E_EXECUTION_CODE_MAP",
                "COVE-E execution-code map is missing a FileCode entry",
                json!({ "file_code": file_code }),
            )
        })?;
    match value {
        ExecutionCodeValue::Unsigned(value) => Ok(*value),
        ExecutionCodeValue::Signed(_) | ExecutionCodeValue::Bytes(_) => Err(exec_error(
            "E_EXECUTION_CODE_MAP",
            "CoveQL FileCode predicates require unsigned COVE-E execution codes",
            json!({ "file_code": file_code }),
        )),
        _ => Err(exec_error(
            "E_EXECUTION_CODE_MAP",
            "CoveQL FileCode predicates received an unsupported COVE-E execution code kind",
            json!({ "file_code": file_code }),
        )),
    }
}

fn execution_code_bitmap_for_request(
    bytes: &[u8],
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
    request: &ExecutionCodePredicateRequest,
    target_codes: &BTreeSet<u64>,
    execution_map: &cove_core::mount::ExecutionCodeMap,
) -> Result<(SelectionBitmap, usize), BuildExecutionError> {
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    let loaded_rows = surface_row_lookup_for_object(surface, root_object_type_id);
    let mut page_count = 0usize;
    for segment in temporal_segments_from_bytes(bytes)? {
        if segment.header.object_type_id != request.object_type_id {
            continue;
        }
        for column in &segment.property_columns {
            if column.directory.column_id != request.property_id
                || Some(column.directory.logical_type)
                    != logical_type_for_execution_code_request(&request.logical_type)
                || column.directory.physical_kind != CovePhysicalKind::FileCode
            {
                continue;
            }
            for page in &column.pages {
                page_count += 1;
                let page_row_start = page
                    .index_entry
                    .morsel_id
                    .checked_mul(segment.header.morsel_row_count)
                    .ok_or_else(|| {
                        exec_error(
                            "E_EXECUTION_CODE_MAP",
                            "COVE-E page row offset overflowed",
                            json!({}),
                        )
                    })?;
                let Some(payload) = &page.payload else {
                    if page.index_entry.null_count == page.index_entry.row_count {
                        continue;
                    }
                    return Err(exec_error(
                        "E_EXECUTION_CODE_MAP",
                        "COVE-E FileCode predicate cannot execute over non-null payload-elided pages",
                        json!({ "segment_id": segment.header.segment_id }),
                    ));
                };
                let root = payload.root_node().map_err(|error| {
                    exec_error(
                        "E_EXECUTION_CODE_MAP",
                        format!("COVE-E FileCode page root could not be read: {error}"),
                        json!({}),
                    )
                })?;
                let null_bitmap =
                    payload
                        .buffer_bytes(PageBufferKind::NullBitmap)
                        .map_err(|error| {
                            exec_error(
                                "E_EXECUTION_CODE_MAP",
                                format!(
                                    "COVE-E FileCode page null bitmap could not be read: {error}"
                                ),
                                json!({}),
                            )
                        })?;
                let validity = null_bitmap
                    .map(|bytes| ValidityBitmap::new(bytes, u64::from(page.index_entry.row_count)));
                let value_bytes = payload
                    .buffer_bytes(PageBufferKind::Values)
                    .map_err(|error| {
                        exec_error(
                            "E_EXECUTION_CODE_MAP",
                            format!(
                                "COVE-E FileCode page values buffer could not be read: {error}"
                            ),
                            json!({}),
                        )
                    })?
                    .unwrap_or(&[]);
                let array = EncodedArray::new(
                    logical_type_for_execution_code_request(&request.logical_type)
                        .unwrap_or(CoveLogicalType::Utf8),
                    CovePhysicalKind::FileCode,
                    u64::from(page.index_entry.row_count),
                    root.encoding_kind,
                    validity,
                    value_bytes,
                    None,
                );
                for local_row in 0..page.index_entry.row_count {
                    let Some(surface_row) =
                        loaded_rows.get(segment.header.segment_id, page_row_start + local_row)
                    else {
                        continue;
                    };
                    let value = array.decode_row(u64::from(local_row)).map_err(|error| {
                        exec_error(
                            "E_EXECUTION_CODE_MAP",
                            format!("COVE-E FileCode page decode failed: {error}"),
                            json!({}),
                        )
                    })?;
                    let CoveArrayValue::FileCode(file_code) = value else {
                        continue;
                    };
                    let execution_code = unsigned_execution_code(execution_map, file_code)?;
                    let contains = target_codes.contains(&execution_code);
                    if matches!(
                        (request.match_mode, contains),
                        (ExecutionCodeMatchMode::Include, true)
                            | (ExecutionCodeMatchMode::Exclude, false)
                    ) {
                        bitmap.set(surface_row);
                    }
                }
            }
        }
    }
    Ok((bitmap, page_count))
}

fn file_code_bitmap_for_request(
    bytes: &[u8],
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
    request: &ExecutionCodePredicateRequest,
    target_codes: &BTreeSet<u32>,
) -> Result<(SelectionBitmap, usize), BuildExecutionError> {
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    let loaded_rows = surface_row_lookup_for_object(surface, root_object_type_id);
    let mut page_count = 0usize;
    let target_codes_vec = target_codes.iter().copied().collect::<Vec<_>>();
    let target_codes_u64 = target_codes
        .iter()
        .copied()
        .map(u64::from)
        .collect::<Vec<_>>();
    let request_logical_type = logical_type_for_execution_code_request(&request.logical_type)
        .unwrap_or(CoveLogicalType::Utf8);
    for segment in temporal_segments_from_bytes_with_context(
        bytes,
        "E_FILE_CODE_LITERAL_PRUNE",
        "single-file FileCode literal prune",
    )? {
        if segment.header.object_type_id != request.object_type_id {
            continue;
        }
        let batch =
            native_object_temporal_batch_from_segment(&segment, NativeCodeDomain::default())
                .map_err(|error| {
                    exec_error(
                "E_FILE_CODE_LITERAL_PRUNE",
                format!("single-file FileCode literal prune native page binding failed: {error}"),
                json!({ "segment_id": segment.header.segment_id }),
            )
                })?;
        for page in &batch.property_pages {
            if page.property_id != request.property_id {
                continue;
            }
            match &page.lane {
                LaneRef::FileCodeU32LeBytes {
                    bytes,
                    row_count,
                    validity,
                    logical_type,
                    ..
                } if *logical_type == request_logical_type => {
                    page_count += 1;
                    match request.match_mode {
                        ExecutionCodeMatchMode::Include => {
                            let (selected, _) = filter_u32_le_in_sorted(
                                bytes,
                                *row_count,
                                *validity,
                                &target_codes_vec,
                                None,
                            )
                            .map_err(|error| {
                                exec_error(
                                    "E_FILE_CODE_LITERAL_PRUNE",
                                    format!(
                                        "single-file FileCode literal prune native page scan failed: {error}"
                                    ),
                                    json!({ "segment_id": segment.header.segment_id }),
                                )
                            })?;
                            for local_row in selected.to_selection_vector().rows() {
                                set_file_code_prune_surface_row(
                                    &mut bitmap,
                                    &loaded_rows,
                                    batch.segment_id,
                                    page.row_start,
                                    *local_row as usize,
                                )?;
                            }
                        }
                        ExecutionCodeMatchMode::Exclude => {
                            let (selected, _) = filter_u32_le_not_in_sorted(
                                bytes,
                                *row_count,
                                *validity,
                                &target_codes_vec,
                                None,
                            )
                            .map_err(|error| {
                                exec_error(
                                    "E_FILE_CODE_LITERAL_PRUNE",
                                    format!(
                                        "single-file FileCode literal prune native page scan failed: {error}"
                                    ),
                                    json!({ "segment_id": segment.header.segment_id }),
                                )
                            })?;
                            for local_row in selected.to_selection_vector().rows() {
                                set_file_code_prune_surface_row(
                                    &mut bitmap,
                                    &loaded_rows,
                                    batch.segment_id,
                                    page.row_start,
                                    *local_row as usize,
                                )?;
                            }
                        }
                    }
                }
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                } if *logical_type == request_logical_type
                    && *physical_kind == CovePhysicalKind::FileCode =>
                {
                    page_count += 1;
                    let selected = local_u8_selection_for_targets(
                        values,
                        *validity,
                        local_to_global,
                        &target_codes_u64,
                        request.match_mode == ExecutionCodeMatchMode::Include,
                    )
                    .0;
                    for local_row in selected.to_selection_vector().rows() {
                        set_file_code_prune_surface_row(
                            &mut bitmap,
                            &loaded_rows,
                            batch.segment_id,
                            page.row_start,
                            *local_row as usize,
                        )?;
                    }
                }
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                } if *logical_type == request_logical_type
                    && *physical_kind == CovePhysicalKind::FileCode =>
                {
                    page_count += 1;
                    let selected = local_u16_selection_for_targets(
                        values,
                        *validity,
                        local_to_global,
                        &target_codes_u64,
                        request.match_mode == ExecutionCodeMatchMode::Include,
                    )
                    .0;
                    for local_row in selected.to_selection_vector().rows() {
                        set_file_code_prune_surface_row(
                            &mut bitmap,
                            &loaded_rows,
                            batch.segment_id,
                            page.row_start,
                            *local_row as usize,
                        )?;
                    }
                }
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                } if *logical_type == request_logical_type
                    && *physical_kind == CovePhysicalKind::FileCode =>
                {
                    page_count += 1;
                    let selected = local_u32_selection_for_targets(
                        values,
                        *validity,
                        local_to_global,
                        &target_codes_u64,
                        request.match_mode == ExecutionCodeMatchMode::Include,
                    )
                    .0;
                    for local_row in selected.to_selection_vector().rows() {
                        set_file_code_prune_surface_row(
                            &mut bitmap,
                            &loaded_rows,
                            batch.segment_id,
                            page.row_start,
                            *local_row as usize,
                        )?;
                    }
                }
                LaneRef::DecodeBoundary {
                    logical_type,
                    physical_kind,
                    reason,
                    ..
                } if *logical_type == request_logical_type
                    && *physical_kind == CovePhysicalKind::FileCode =>
                {
                    page_count += 1;
                    if *reason == "all-null elided page" {
                        continue;
                    }
                    return Err(exec_error(
                        "E_FILE_CODE_LITERAL_PRUNE",
                        format!(
                            "single-file FileCode literal prune cannot execute over decode-boundary page: {reason}"
                        ),
                        json!({ "segment_id": segment.header.segment_id }),
                    ));
                }
                _ => {}
            }
        }
    }
    Ok((bitmap, page_count))
}

fn set_file_code_prune_surface_row(
    bitmap: &mut SelectionBitmap,
    loaded_rows: &SurfaceRowLookup,
    segment_id: u32,
    page_row_start: usize,
    local_row: usize,
) -> Result<(), BuildExecutionError> {
    let row_index = page_row_start.checked_add(local_row).ok_or_else(|| {
        exec_error(
            "E_FILE_CODE_LITERAL_PRUNE",
            "single-file FileCode literal prune page row offset overflowed",
            json!({}),
        )
    })?;
    let row_index = u32::try_from(row_index).map_err(|_| {
        exec_error(
            "E_FILE_CODE_LITERAL_PRUNE",
            "single-file FileCode literal prune page row index exceeded u32",
            json!({}),
        )
    })?;
    if let Some(surface_row) = loaded_rows.get(segment_id, row_index) {
        bitmap.set(surface_row);
    }
    Ok(())
}

fn logical_type_for_execution_code_request(logical_type: &str) -> Option<CoveLogicalType> {
    logical_type_to_cove(logical_type)
}

fn temporal_segments_from_bytes(
    bytes: &[u8],
) -> Result<Vec<TemporalSegmentData>, BuildExecutionError> {
    temporal_segments_from_bytes_with_context(bytes, "E_EXECUTION_CODE_MAP", "COVE-E")
}

fn temporal_segments_from_bytes_with_context(
    bytes: &[u8],
    error_code: &'static str,
    source_label: &'static str,
) -> Result<Vec<TemporalSegmentData>, BuildExecutionError> {
    let validation = cove_core::reader::validate_bytes_with_options(
        bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .map_err(|error| {
        exec_error(
            error_code,
            format!("{source_label} segment validation failed: {error}"),
            json!({}),
        )
    })?;
    let mut segments = Vec::new();
    for section in validation
        .validated
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::TemporalSegmentData as u16)
    {
        let payload = compression::section_payload(bytes, section).map_err(|error| {
            exec_error(
                error_code,
                format!("{source_label} temporal segment payload read failed: {error}"),
                json!({}),
            )
        })?;
        let segment = TemporalSegmentData::parse_with_required_features(
            payload.as_ref(),
            validation.validated.header.required_features,
        )
        .map_err(|error| {
            exec_error(
                error_code,
                format!("{source_label} temporal segment parse failed: {error}"),
                json!({}),
            )
        })?;
        segments.push(segment);
    }
    Ok(segments)
}

fn covi_first_empty_lookup(
    physical: &PhysicalPlannedQuery,
    requests: &[CoviLookupRequestV2],
) -> Result<Option<(CoviLookupRequestV2, &'static str)>, BuildExecutionError> {
    for (source, bytes) in [
        ("cove_i", physical.sidecars.covi_artifact_bytes.as_deref()),
        ("covx", physical.sidecars.covx_artifact_bytes.as_deref()),
    ]
    .into_iter()
    .filter_map(|(source, bytes)| bytes.map(|bytes| (source, bytes)))
    {
        let context = CoviValidationContextV2::for_file(
            physical.planned.resolved.operation_context.file.file_id,
            physical.planned.resolved.operation_context.file.file_len,
            physical
                .planned
                .resolved
                .operation_context
                .file
                .footer_crc32c,
        )
        .with_file_code_keys(physical.allow_file_code_literal_candidates);
        let Ok(artifact) = ValidatedCoviArtifactV2::parse_and_validate(bytes, context) else {
            continue;
        };
        for request in requests {
            match artifact.lookup(request) {
                Ok(candidates)
                    if candidates.exactness == IndexCapabilityExactnessV2::Exact
                        && candidates.proof_strength.allows_pruning()
                        && candidates.is_empty() =>
                {
                    return Ok(Some((request.clone(), source)));
                }
                Ok(_) => {}
                Err(CoveError::IndexOnlyUnsafe) => {
                    return Err(exec_error(
                        "E_COVI_LOOKUP_UNSAFE",
                        "COVI/COVX lookup metadata matched the target but did not prove an exact safe candidate set",
                        json!({ "source": source }),
                    ))
                }
                Err(_) => {}
            }
        }
    }
    Ok(None)
}

fn covi_empty_lookup_requests(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
) -> Result<Vec<CoviLookupRequestV2>, BuildExecutionError> {
    let mut requests = Vec::new();
    collect_conjunctive_covi_empty_lookup_requests(predicate, root_object_type_id, &mut requests)?;
    Ok(requests)
}

fn collect_conjunctive_covi_empty_lookup_requests(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
    out: &mut Vec<CoviLookupRequestV2>,
) -> Result<(), BuildExecutionError> {
    match predicate {
        ResolvedPredicate::And(parts) => {
            for part in parts {
                collect_conjunctive_covi_empty_lookup_requests(part, root_object_type_id, out)?;
            }
        }
        ResolvedPredicate::Compare { left, op, right } if *op == AstCompareOp::Eq => {
            if let Some(request) = covi_compare_eq_request(left, right, root_object_type_id)? {
                out.push(request);
            } else if let Some(request) = covi_compare_eq_request(right, left, root_object_type_id)?
            {
                out.push(request);
            }
        }
        ResolvedPredicate::InList { expr, values } => {
            if let Some(path) = covi_object_property_path(expr, root_object_type_id) {
                let mut keys = Vec::new();
                for literal in values {
                    if matches!(literal.typed_value, ResolvedLiteralValue::Null) {
                        continue;
                    }
                    let Some(key) = covi_canonical_lookup_key(path, literal)? else {
                        return Ok(());
                    };
                    keys.push(key);
                }
                if !keys.is_empty() {
                    out.push(CoviLookupRequestV2::membership_target(
                        covi_lookup_target_for_path(path),
                        keys,
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn covi_compare_eq_request(
    left: &ResolvedExpr,
    right: &ResolvedExpr,
    root_object_type_id: u32,
) -> Result<Option<CoviLookupRequestV2>, BuildExecutionError> {
    let Some(path) = covi_object_property_path(left, root_object_type_id) else {
        return Ok(None);
    };
    let ResolvedExpr::Literal(literal) = right else {
        return Ok(None);
    };
    let Some(key) = covi_canonical_lookup_key(path, literal)? else {
        return Ok(None);
    };
    Ok(Some(CoviLookupRequestV2::eq_target(
        covi_lookup_target_for_path(path),
        key,
    )))
}

fn covi_object_property_path(
    expr: &ResolvedExpr,
    root_object_type_id: u32,
) -> Option<&ResolvedPath> {
    let ResolvedExpr::Path(path) = expr else {
        return None;
    };
    if !matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
        || path.system_field.is_some()
        || path.object_type_id != Some(root_object_type_id)
        || path.property_id.is_none()
    {
        return None;
    }
    Some(path)
}

fn covi_lookup_target_for_path(path: &ResolvedPath) -> CoviLookupTargetV2 {
    CoviLookupTargetV2::ObjectProperty {
        object_type_id: path.object_type_id.expect("checked object_type_id"),
        property_id: path.property_id.expect("checked property_id"),
    }
}

fn covi_canonical_lookup_key(
    path: &ResolvedPath,
    literal: &ResolvedLiteral,
) -> Result<Option<CoviLookupKeyV2>, BuildExecutionError> {
    let Some(canonical) = canonical_value_for_resolved_literal(&path.logical_type, literal)? else {
        return Ok(None);
    };
    Ok(Some(CoviLookupKeyV2::CanonicalValueBytes(
        tagged_canonical_lookup_key(canonical)?,
    )))
}

fn canonical_value_for_resolved_literal<'a>(
    logical_type: &str,
    literal: &'a ResolvedLiteral,
) -> Result<Option<CanonicalValue<'a>>, BuildExecutionError> {
    Ok(
        match (&literal.typed_value, logical_type_to_cove(logical_type)) {
            (ResolvedLiteralValue::Null, _) => return Ok(None),
            (ResolvedLiteralValue::Boolean(value), Some(CoveLogicalType::Bool)) => {
                Some(CanonicalValue::Bool(*value))
            }
            (ResolvedLiteralValue::String(value), Some(CoveLogicalType::Utf8)) => {
                Some(CanonicalValue::Utf8(value))
            }
            (ResolvedLiteralValue::String(value), Some(CoveLogicalType::Json)) => {
                Some(CanonicalValue::Json(value))
            }
            (ResolvedLiteralValue::SignedInteger(value), Some(logical)) => {
                signed_literal_canonical_value(logical, *value)?
            }
            (
                ResolvedLiteralValue::UnsignedInteger(value),
                Some(
                    logical @ (CoveLogicalType::Int8
                    | CoveLogicalType::Int16
                    | CoveLogicalType::Int32
                    | CoveLogicalType::Int64
                    | CoveLogicalType::Decimal64
                    | CoveLogicalType::DateDays
                    | CoveLogicalType::TimestampMicros
                    | CoveLogicalType::TimestampNanos),
                ),
            ) => unsigned_literal_as_signed_canonical_value(logical, *value)?,
            (ResolvedLiteralValue::UnsignedInteger(value), Some(logical)) => {
                unsigned_literal_canonical_value(logical, *value)?
            }
            (
                ResolvedLiteralValue::TimestampMicros { micros, .. },
                Some(CoveLogicalType::TimestampMicros),
            ) => Some(CanonicalValue::TimestampMicros(*micros)),
            (ResolvedLiteralValue::Uuid { bytes, .. }, Some(CoveLogicalType::Uuid)) => {
                Some(CanonicalValue::Uuid(*bytes))
            }
            (ResolvedLiteralValue::Binary { bytes, .. }, Some(CoveLogicalType::Binary)) => {
                Some(CanonicalValue::Bytes(bytes.as_slice()))
            }
            _ => None,
        },
    )
}

fn unsigned_literal_as_signed_canonical_value(
    logical: CoveLogicalType,
    value: u64,
) -> Result<Option<CanonicalValue<'static>>, BuildExecutionError> {
    let signed = i64::try_from(value).map_err(|_| {
        exec_error(
            "E_LITERAL_TYPE_DOMAIN",
            "unsigned integer literal does not fit signed canonical value domain",
            json!({ "value": value }),
        )
    })?;
    signed_literal_canonical_value(logical, signed)
}

fn signed_literal_canonical_value(
    logical: CoveLogicalType,
    value: i64,
) -> Result<Option<CanonicalValue<'static>>, BuildExecutionError> {
    Ok(match logical {
        CoveLogicalType::Int8 => {
            let _ = i8::try_from(value).map_err(|_| {
                exec_error(
                    "E_COVI_LOOKUP_UNSAFE",
                    "signed literal does not fit int8 index key domain",
                    json!({ "value": value }),
                )
            })?;
            Some(CanonicalValue::Int {
                width: 1,
                value: i128::from(value),
            })
        }
        CoveLogicalType::Int16 => {
            let _ = i16::try_from(value).map_err(|_| {
                exec_error(
                    "E_COVI_LOOKUP_UNSAFE",
                    "signed literal does not fit int16 index key domain",
                    json!({ "value": value }),
                )
            })?;
            Some(CanonicalValue::Int {
                width: 2,
                value: i128::from(value),
            })
        }
        CoveLogicalType::Int32 | CoveLogicalType::DateDays => {
            let _ = i32::try_from(value).map_err(|_| {
                exec_error(
                    "E_COVI_LOOKUP_UNSAFE",
                    "signed literal does not fit int32 index key domain",
                    json!({ "value": value }),
                )
            })?;
            if logical == CoveLogicalType::DateDays {
                Some(CanonicalValue::DateDays(value as i32))
            } else {
                Some(CanonicalValue::Int {
                    width: 4,
                    value: i128::from(value),
                })
            }
        }
        CoveLogicalType::Int64 => Some(CanonicalValue::Int {
            width: 8,
            value: i128::from(value),
        }),
        CoveLogicalType::Decimal64 => Some(CanonicalValue::Decimal64(value)),
        CoveLogicalType::TimestampMicros => Some(CanonicalValue::TimestampMicros(value)),
        CoveLogicalType::TimestampNanos => Some(CanonicalValue::TimestampNanos(value)),
        _ => None,
    })
}

fn unsigned_literal_canonical_value(
    logical: CoveLogicalType,
    value: u64,
) -> Result<Option<CanonicalValue<'static>>, BuildExecutionError> {
    Ok(match logical {
        CoveLogicalType::UInt8 => {
            let _ = u8::try_from(value).map_err(|_| {
                exec_error(
                    "E_COVI_LOOKUP_UNSAFE",
                    "unsigned literal does not fit uint8 index key domain",
                    json!({ "value": value }),
                )
            })?;
            Some(CanonicalValue::Uint {
                width: 1,
                value: u128::from(value),
            })
        }
        CoveLogicalType::UInt16 => {
            let _ = u16::try_from(value).map_err(|_| {
                exec_error(
                    "E_COVI_LOOKUP_UNSAFE",
                    "unsigned literal does not fit uint16 index key domain",
                    json!({ "value": value }),
                )
            })?;
            Some(CanonicalValue::Uint {
                width: 2,
                value: u128::from(value),
            })
        }
        CoveLogicalType::UInt32 => {
            let _ = u32::try_from(value).map_err(|_| {
                exec_error(
                    "E_COVI_LOOKUP_UNSAFE",
                    "unsigned literal does not fit uint32 index key domain",
                    json!({ "value": value }),
                )
            })?;
            Some(CanonicalValue::Uint {
                width: 4,
                value: u128::from(value),
            })
        }
        CoveLogicalType::UInt64 => Some(CanonicalValue::Uint {
            width: 8,
            value: u128::from(value),
        }),
        _ => None,
    })
}

fn tagged_canonical_lookup_key(value: CanonicalValue<'_>) -> Result<Vec<u8>, BuildExecutionError> {
    let mut key = Vec::new();
    wire::append_u64_leb128(&mut key, value.value_tag() as u64);
    key.extend_from_slice(&value.encode().map_err(|error| {
        exec_error(
            "E_COVI_LOOKUP_UNSAFE",
            format!("literal could not be converted to a canonical COVI lookup key: {error}"),
            json!({}),
        )
    })?);
    Ok(key)
}

fn logical_type_to_cove(logical_type: &str) -> Option<CoveLogicalType> {
    match logical_type {
        "null" => Some(CoveLogicalType::Null),
        "bool" | "boolean" => Some(CoveLogicalType::Bool),
        "int8" => Some(CoveLogicalType::Int8),
        "int16" => Some(CoveLogicalType::Int16),
        "int32" => Some(CoveLogicalType::Int32),
        "int64" => Some(CoveLogicalType::Int64),
        "uint8" => Some(CoveLogicalType::UInt8),
        "uint16" => Some(CoveLogicalType::UInt16),
        "uint32" => Some(CoveLogicalType::UInt32),
        "uint64" => Some(CoveLogicalType::UInt64),
        "decimal64" => Some(CoveLogicalType::Decimal64),
        "date_days" => Some(CoveLogicalType::DateDays),
        "timestamp_micros" => Some(CoveLogicalType::TimestampMicros),
        "timestamp_nanos" => Some(CoveLogicalType::TimestampNanos),
        "utf8" | "string" => Some(CoveLogicalType::Utf8),
        "binary" => Some(CoveLogicalType::Binary),
        "uuid" => Some(CoveLogicalType::Uuid),
        "json" => Some(CoveLogicalType::Json),
        _ => None,
    }
}

fn covi_lookup_request_key_count(request: &CoviLookupRequestV2) -> usize {
    1 + request.membership_keys.len()
}

fn covi_lookup_target_json(target: &CoviLookupTargetV2) -> Value {
    match target {
        CoviLookupTargetV2::TableColumn {
            table_id,
            column_id,
        } => json!({ "kind": "table_column", "table_id": table_id, "column_id": column_id }),
        CoviLookupTargetV2::ProjectionColumn {
            table_id,
            column_id,
        } => json!({ "kind": "projection_column", "table_id": table_id, "column_id": column_id }),
        CoviLookupTargetV2::ObjectProperty {
            object_type_id,
            property_id,
        } => json!({
            "kind": "object_property",
            "object_type_id": object_type_id,
            "property_id": property_id,
        }),
        CoviLookupTargetV2::ObjectPath {
            object_type_id,
            path_ref,
        } => json!({
            "kind": "object_path",
            "object_type_id": object_type_id,
            "path_ref": path_ref,
        }),
        CoviLookupTargetV2::SemanticDimension {
            semantic_dimension_ref,
        } => {
            json!({ "kind": "semantic_dimension", "semantic_dimension_ref": semantic_dimension_ref })
        }
        CoviLookupTargetV2::DimensionalTuple {
            semantic_dimension_ref,
        } => json!({
            "kind": "dimensional_tuple",
            "semantic_dimension_ref": semantic_dimension_ref,
        }),
    }
}

fn try_index_only_executed_query(
    physical: &PhysicalPlannedQuery,
    options: &ExecutionOptions,
) -> Result<Option<ExecutedQuery>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if !physical.allow_index_only_answers
        || !security.index_only_answer_permission
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || security.aggregate_disclosure_policy == AggregateDisclosurePolicy::AllowMaterializedOnly
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || options.visibility_overlay.is_some()
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || !matches!(planned.resolved.temporal.mode, TemporalMode::Latest)
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
    {
        return Ok(None);
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let [item] = select.as_slice() else {
        return Ok(None);
    };
    let ResolvedExpr::AggregateCall {
        name, arg, star, ..
    } = &item.expr
    else {
        return Ok(None);
    };
    let Some(path) = index_only_aggregate_path(*name, arg.as_deref(), *star) else {
        return Ok(None);
    };
    let (Some(object_type_id), Some(property_id)) = (path.object_type_id, path.property_id) else {
        return Ok(None);
    };
    if object_type_id != root.object_type_id {
        return Ok(None);
    }
    let Some(aggregate_kind) = covi_index_only_aggregate_kind(*name) else {
        return Ok(None);
    };
    let Some(answer) =
        index_only_answer_from_sidecars(physical, object_type_id, property_id, aggregate_kind)?
    else {
        return Ok(None);
    };
    let exact = index_only_json_value(*name, &path.logical_type, &answer)?;
    let disclosed = disclose_index_only_aggregate(exact, answer.row_count, planned)?;
    let mut object = serde_json::Map::new();
    object.insert(
        item.alias
            .clone()
            .unwrap_or_else(|| aggregate_output_name(*name).into()),
        disclosed,
    );
    let result = CoveQlExecutionResult::JsonRows(vec![Value::Object(object)]);
    let output_fingerprint = result_fingerprint(&result)?;
    let input_rows = usize::try_from(answer.row_count).map_err(|_| {
        exec_error(
            "E_RESOURCE_LIMIT",
            "index-only row count does not fit the execution row counter",
            json!({ "limit": "usize_row_count" }),
        )
    })?;
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
    diagnostics.push(exec_warning(
        "W_INDEX_ONLY_ANSWER_EXECUTED",
        "validated COVI/COVX index-only aggregate answer was used as execution output",
        json!({
            "aggregate": aggregate_output_name(*name),
            "object_type_id": object_type_id,
            "property_id": property_id,
        }),
    ));
    Ok(Some(ExecutedQuery {
        planned: planned.clone(),
        result,
        diagnostics,
        row_counts: ExecutionRowCounts {
            input_rows,
            filtered_rows: input_rows,
            output_rows: 1,
        },
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_executed(&options.pushdown),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_index_only(
            "validated exact COVI/COVX index-only aggregate answer produced the output",
        ),
    }))
}

fn index_only_aggregate_path<'a>(
    name: AstAggregateName,
    arg: Option<&'a ResolvedExpr>,
    star: bool,
) -> Option<&'a ResolvedPath> {
    if star {
        return None;
    }
    match (name, arg) {
        (
            AstAggregateName::Count
            | AstAggregateName::Exists
            | AstAggregateName::DistinctCount
            | AstAggregateName::Min
            | AstAggregateName::Max,
            Some(ResolvedExpr::Path(path)),
        ) => Some(path),
        (AstAggregateName::Sum | AstAggregateName::Avg, Some(ResolvedExpr::Path(path)))
            if index_only_sum_avg_path_is_supported(path) =>
        {
            Some(path)
        }
        _ => None,
    }
}

fn index_only_sum_avg_path_is_supported(path: &ResolvedPath) -> bool {
    matches!(
        path.logical_type.as_str(),
        "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
    )
}

fn covi_index_only_aggregate_kind(name: AstAggregateName) -> Option<CoviAggregateKindV2> {
    match name {
        AstAggregateName::Count => Some(CoviAggregateKindV2::Count),
        AstAggregateName::Exists => Some(CoviAggregateKindV2::Exists),
        AstAggregateName::DistinctCount => Some(CoviAggregateKindV2::DistinctCount),
        AstAggregateName::Min => Some(CoviAggregateKindV2::Min),
        AstAggregateName::Max => Some(CoviAggregateKindV2::Max),
        AstAggregateName::Sum => Some(CoviAggregateKindV2::Sum),
        AstAggregateName::Avg => Some(CoviAggregateKindV2::Avg),
    }
}

fn index_only_answer_from_sidecars(
    physical: &PhysicalPlannedQuery,
    object_type_id: u32,
    property_id: u32,
    aggregate_kind: CoviAggregateKindV2,
) -> Result<Option<CoviIndexOnlyAnswerV2>, BuildExecutionError> {
    let request = CoviIndexOnlyRequestV2 {
        table_id: u32::MAX,
        column_id: None,
        aggregate_kind,
        predicate_form_ref: None,
        require_exact: true,
    };
    let target = CoviLookupTargetV2::ObjectProperty {
        object_type_id,
        property_id,
    };
    for bytes in [
        physical.sidecars.covi_artifact_bytes.as_deref(),
        physical.sidecars.covx_artifact_bytes.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let context = CoviValidationContextV2::for_file(
            physical.planned.resolved.operation_context.file.file_id,
            physical.planned.resolved.operation_context.file.file_len,
            physical
                .planned
                .resolved
                .operation_context
                .file
                .footer_crc32c,
        )
        .with_file_code_keys(physical.allow_file_code_literal_candidates);
        let Ok(artifact) = ValidatedCoviArtifactV2::parse_and_validate(bytes, context) else {
            continue;
        };
        match artifact.index_only_answer_for_target(target, &request) {
            Ok(Some(answer)) => return Ok(Some(answer)),
            Ok(None) => {}
            Err(CoveError::IndexOnlyUnsafe) => {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "index-only metadata matched the target but did not prove an exact safe answer",
                    json!({
                        "object_type_id": object_type_id,
                        "property_id": property_id,
                        "aggregate": format!("{aggregate_kind:?}"),
                    }),
                ))
            }
            Err(_) => {}
        }
    }
    Ok(None)
}

fn index_only_json_value(
    name: AstAggregateName,
    logical_type: &str,
    answer: &CoviIndexOnlyAnswerV2,
) -> Result<Value, BuildExecutionError> {
    Ok(match name {
        AstAggregateName::Count => json!(answer.non_null_count),
        AstAggregateName::Exists => Value::Bool(answer.non_null_count > 0),
        AstAggregateName::DistinctCount => {
            let Some(value) = &answer.value else {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "distinctCount index-only answer did not include an exact value payload",
                    json!({}),
                ));
            };
            let bytes: [u8; 8] = value.as_slice().try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "distinctCount index-only value payload was not a u64",
                    json!({ "payload_len": value.len() }),
                )
            })?;
            json!(u64::from_le_bytes(bytes))
        }
        AstAggregateName::Min | AstAggregateName::Max => match &answer.value {
            Some(value) => decode_index_only_min_max_value(logical_type, value)?,
            None => Value::Null,
        },
        AstAggregateName::Sum | AstAggregateName::Avg => {
            index_only_sum_avg_json_value(name, logical_type, answer)?
        }
    })
}

fn index_only_sum_avg_json_value(
    name: AstAggregateName,
    logical_type: &str,
    answer: &CoviIndexOnlyAnswerV2,
) -> Result<Value, BuildExecutionError> {
    if answer.non_null_count == 0 {
        return Ok(Value::Null);
    }
    let Some(value) = &answer.value else {
        return Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "sum/avg index-only answer did not include an exact sum payload",
            json!({ "aggregate": aggregate_output_name(name), "logical_type": logical_type }),
        ));
    };
    let sum = decode_index_only_numeric_sum_value(logical_type, value)?;
    match name {
        AstAggregateName::Sum => Ok(sum),
        AstAggregateName::Avg => {
            if matches!(logical_type, "float32" | "float64") {
                let Some(sum) = sum.as_f64() else {
                    return Err(exec_error(
                        "E_INDEX_ONLY_UNSAFE",
                        "float avg index-only sum payload was not a JSON number",
                        json!({ "logical_type": logical_type }),
                    ));
                };
                Ok(json!(sum / answer.non_null_count as f64))
            } else {
                parse_decimal_value(&sum)
                    .ok_or_else(|| {
                        exec_error(
                            "E_INDEX_ONLY_UNSAFE",
                            "exact avg index-only sum payload was not a decimal value",
                            json!({ "logical_type": logical_type }),
                        )
                    })?
                    .checked_div_u64(answer.non_null_count)
                    .ok_or_else(|| {
                        exec_error(
                            "E_INDEX_ONLY_UNSAFE",
                            "avg index-only sum payload overflowed decimal division",
                            json!({ "logical_type": logical_type }),
                        )
                    })?
                    .to_json_sum()
                    .map_err(|message| exec_error("E_INDEX_ONLY_UNSAFE", message, json!({})))
            }
        }
        _ => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "index-only numeric aggregate received a nonnumeric aggregate",
            json!({ "aggregate": aggregate_output_name(name) }),
        )),
    }
}

fn decode_index_only_numeric_sum_value(
    logical_type: &str,
    payload: &[u8],
) -> Result<Value, BuildExecutionError> {
    match logical_type {
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" => {
            decode_index_only_min_max_value(logical_type, payload)
        }
        _ => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "logical type is not supported for CoveQL index-only sum/avg output",
            json!({ "logical_type": logical_type }),
        )),
    }
}

fn decode_index_only_min_max_value(
    logical_type: &str,
    payload: &[u8],
) -> Result<Value, BuildExecutionError> {
    match logical_type {
        "int8" | "int16" | "int32" | "int64" | "decimal64" | "timestamp_micros"
        | "timestamp_nanos" => {
            let bytes: [u8; 8] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "signed index-only min/max payload was not 8 bytes",
                    json!({ "logical_type": logical_type, "payload_len": payload.len() }),
                )
            })?;
            Ok(json!(i64::from_le_bytes(bytes)))
        }
        "uint8" | "uint16" | "uint32" | "uint64" => {
            let bytes: [u8; 8] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "unsigned index-only min/max payload was not 8 bytes",
                    json!({ "logical_type": logical_type, "payload_len": payload.len() }),
                )
            })?;
            Ok(json!(u64::from_le_bytes(bytes)))
        }
        "float32" => {
            let bytes: [u8; 4] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "float32 index-only min/max payload was not 4 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            let value = f32::from_bits(u32::from_le_bytes(bytes));
            if !value.is_finite() {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "non-finite float32 index-only min/max payload cannot be represented as JSON",
                    json!({}),
                ));
            }
            Ok(json!(value))
        }
        "float64" => {
            let bytes: [u8; 8] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "float64 index-only min/max payload was not 8 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            let value = f64::from_bits(u64::from_le_bytes(bytes));
            if !value.is_finite() {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "non-finite float64 index-only min/max payload cannot be represented as JSON",
                    json!({}),
                ));
            }
            Ok(json!(value))
        }
        "date_days" => {
            let bytes: [u8; 4] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "date_days index-only min/max payload was not 4 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            Ok(json!(i32::from_le_bytes(bytes)))
        }
        "utf8" | "json" => {
            let (len, consumed) = wire::decode_u64_leb128(payload).map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload had an invalid length prefix",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            let len = usize::try_from(len).map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload length overflowed",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            let end = consumed.checked_add(len).ok_or_else(|| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload length overflowed",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            if end != payload.len() {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload length did not match payload",
                    json!({ "logical_type": logical_type }),
                ));
            }
            let text = std::str::from_utf8(&payload[consumed..end]).map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload was not valid UTF-8",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            if logical_type == "json" {
                serde_json::from_str(text).map_err(|_| {
                    exec_error(
                        "E_INDEX_ONLY_UNSAFE",
                        "json index-only min/max payload was not valid JSON",
                        json!({}),
                    )
                })
            } else {
                Ok(Value::String(text.into()))
            }
        }
        "uuid" => {
            let bytes: [u8; 16] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "uuid index-only min/max payload was not 16 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            Ok(Value::String(crate::materialized::hex(&bytes)))
        }
        "binary" | "decimal128" | "bool" | "null" | "list" | "struct" | "map" => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "logical type is not supported for CoveQL index-only min/max output",
            json!({ "logical_type": logical_type }),
        )),
        _ => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "unknown logical type for CoveQL index-only min/max output",
            json!({ "logical_type": logical_type }),
        )),
    }
}

fn disclose_index_only_aggregate(
    exact: Value,
    row_count: u64,
    planned: &crate::PlannedQuery,
) -> Result<Value, BuildExecutionError> {
    match planned
        .resolved
        .operation_context
        .security
        .aggregate_disclosure_policy
    {
        AggregateDisclosurePolicy::AllowExact => Ok(exact),
        AggregateDisclosurePolicy::AllowMaterializedOnly => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "index-only answers are forbidden by aggregate materialized-only disclosure policy",
            json!({}),
        )),
        AggregateDisclosurePolicy::AllowThresholded => {
            let threshold = planned
                .resolved
                .operation_context
                .security
                .aggregate_disclosure_threshold
                .unwrap_or(1);
            if row_count >= threshold {
                Ok(exact)
            } else {
                Ok(aggregate_policy_marker(
                    "thresholded",
                    "suppressed",
                    Some(threshold),
                ))
            }
        }
        AggregateDisclosurePolicy::AllowRedacted => {
            Ok(aggregate_policy_marker("redacted", "redacted", None))
        }
        AggregateDisclosurePolicy::Reject => Err(exec_error(
            "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
            "aggregate disclosure is rejected by the active security context",
            json!({}),
        )),
    }
}

fn aggregate_policy_marker(policy: &str, status: &str, threshold: Option<u64>) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("policy".into(), Value::String(policy.into()));
    object.insert("status".into(), Value::String(status.into()));
    if let Some(threshold) = threshold {
        object.insert("threshold".into(), json!(threshold));
    }
    Value::Object(object)
}

fn aggregate_output_name(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count => "count",
        AstAggregateName::Min => "min",
        AstAggregateName::Max => "max",
        AstAggregateName::Sum => "sum",
        AstAggregateName::Avg => "avg",
        AstAggregateName::Exists => "exists",
        AstAggregateName::DistinctCount => "distinct_count",
    }
}

fn try_native_bool_group_count_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeBoolGroupCountReport,
    )>,
    BuildExecutionError,
> {
    if !native_bool_group_count_shape(planned) {
        return Ok(None);
    }
    let Some(group_path) = native_bool_group_count_path(planned) else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    if let Some(result) = try_native_dense_bool_group_star_aggregate_result(
        rows, group_path, select, planned, options, started,
    )? {
        return Ok(Some(result));
    }
    let mut groups = BTreeMap::<String, (Value, Vec<usize>)>::new();
    for (row_index, row) in rows.iter().enumerate() {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, group_path) {
            return Ok(None);
        }
        let value = row.value_for_path(group_path);
        if !native_direct_group_count_value_is_supported(&value, group_path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native direct grouped aggregate kernel received a value outside its proven typed lane",
                json!({
                    "group_property": group_path.display_name,
                    "logical_type": group_path.logical_type,
                    "value": value,
                }),
            ));
        }
        let key = serde_json::to_string(&[value.clone()]).map_err(|err| {
            exec_error(
                "E_KERNEL_AGGREGATE",
                format!("native direct group key could not be serialized: {err}"),
                json!({}),
            )
        })?;
        groups
            .entry(key)
            .and_modify(|(_, row_indices)| row_indices.push(row_index))
            .or_insert((value, vec![row_index]));
    }
    if groups.len() > options.resource_budget.maximum_groups {
        return Err(resource_error("maximum_groups", groups.len()));
    }
    let mut json_rows = Vec::with_capacity(groups.len());
    let mut aggregate_name = "count";
    let mut aggregate_strategy = "none";
    let mut values_seen = 0usize;
    for (_key, (group_value, row_indices)) in groups {
        let mut object = serde_json::Map::new();
        for item in select {
            let key = native_bool_group_count_output_name(&item.expr, item.alias.as_deref());
            match &item.expr {
                ResolvedExpr::Path(_) => {
                    object.insert(key, group_value.clone());
                }
                ResolvedExpr::AggregateCall {
                    name, arg, star, ..
                } => {
                    aggregate_name = aggregate_operator_name(*name);
                    let aggregate_eval = native_grouped_direct_aggregate_eval(
                        *name,
                        *star,
                        arg.as_deref(),
                        &row_indices,
                        rows,
                    )?;
                    aggregate_strategy = aggregate_eval.strategy;
                    values_seen += aggregate_eval.values_seen;
                    object.insert(key, aggregate_eval.value);
                }
                _ => return Ok(None),
            }
        }
        json_rows.push(Value::Object(object));
    }
    let group_count = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeBoolGroupCountReport {
            aggregate: aggregate_name,
            group_property: group_path.display_name.clone(),
            group_logical_type: group_path.logical_type.clone(),
            group_count,
            rows_counted: rows.len(),
            values_seen,
            group_strategy: "row_index_single_pass",
            aggregate_strategy,
        },
    )))
}

fn try_native_dense_bool_group_star_aggregate_result(
    rows: &[ExecutionRow],
    group_path: &ResolvedPath,
    select: &[crate::ResolvedSelectItem],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeBoolGroupCountReport,
    )>,
    BuildExecutionError,
> {
    if !matches!(group_path.logical_type.as_str(), "bool" | "boolean") {
        return Ok(None);
    }
    let Some(aggregate_name) = select.iter().find_map(|item| match &item.expr {
        ResolvedExpr::AggregateCall {
            name,
            star: true,
            arg: None,
            ..
        } if matches!(name, AstAggregateName::Count | AstAggregateName::Exists) => Some(*name),
        _ => None,
    }) else {
        return Ok(None);
    };

    let mut false_count = 0usize;
    let mut null_count = 0usize;
    let mut true_count = 0usize;
    for row in rows {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, group_path) {
            return Ok(None);
        }
        match row.value_for_path(group_path) {
            Value::Bool(false) => false_count += 1,
            Value::Bool(true) => true_count += 1,
            Value::Null => null_count += 1,
            value => return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native dense bool group aggregate received a value outside its proven typed lane",
                json!({
                    "group_property": group_path.display_name,
                    "logical_type": group_path.logical_type,
                    "value": value,
                }),
            )),
        }
    }

    let mut json_rows = Vec::with_capacity(3);
    for (group_value, group_rows) in [
        (Value::Bool(false), false_count),
        (Value::Null, null_count),
        (Value::Bool(true), true_count),
    ] {
        if group_rows == 0 {
            continue;
        }
        let mut object = serde_json::Map::new();
        for item in select {
            let key = native_bool_group_count_output_name(&item.expr, item.alias.as_deref());
            match &item.expr {
                ResolvedExpr::Path(_) => {
                    object.insert(key, group_value.clone());
                }
                ResolvedExpr::AggregateCall {
                    name,
                    star: true,
                    arg: None,
                    ..
                } if *name == aggregate_name => {
                    let value = match aggregate_name {
                        AstAggregateName::Count => json!(group_rows as u64),
                        AstAggregateName::Exists => Value::Bool(group_rows > 0),
                        _ => unreachable!("dense bool group star aggregate was prefiltered"),
                    };
                    object.insert(key, value);
                }
                _ => return Ok(None),
            }
        }
        json_rows.push(Value::Object(object));
    }

    let group_count = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeBoolGroupCountReport {
            aggregate: aggregate_operator_name(aggregate_name),
            group_property: group_path.display_name.clone(),
            group_logical_type: group_path.logical_type.clone(),
            group_count,
            rows_counted: rows.len(),
            values_seen: rows.len(),
            group_strategy: "dense_bool_star",
            aggregate_strategy: "row_count",
        },
    )))
}

fn try_native_grouped_helper_aggregate_result(
    rows: &[ExecutionRow],
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[crate::materialized::MaterializedEvidenceRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeGroupedHelperAggregateReport,
    )>,
    BuildExecutionError,
> {
    if !native_grouped_helper_aggregate_shape(planned) {
        return Ok(None);
    }
    let Some(group_path) = native_bool_group_count_path(planned) else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let mut groups = BTreeMap::<String, (Value, Vec<usize>)>::new();
    for (row_index, row) in rows.iter().enumerate() {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, group_path) {
            return Ok(None);
        }
        let value = row.value_for_path(group_path);
        if !native_direct_group_count_value_is_supported(&value, group_path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped helper aggregate kernel received a value outside its proven typed group lane",
                json!({
                    "group_property": group_path.display_name,
                    "logical_type": group_path.logical_type,
                    "value": value,
                }),
            ));
        }
        let key = serde_json::to_string(&[value.clone()]).map_err(|err| {
            exec_error(
                "E_KERNEL_AGGREGATE",
                format!("native grouped helper key could not be serialized: {err}"),
                json!({}),
            )
        })?;
        groups
            .entry(key)
            .and_modify(|(_, row_indices)| row_indices.push(row_index))
            .or_insert((value, vec![row_index]));
    }
    if groups.len() > options.resource_budget.maximum_groups {
        return Err(resource_error("maximum_groups", groups.len()));
    }

    let association_edges = AssociationEdgeTable::from_rows_for_plan(planned, associations);
    let evidence_index = EvidenceGrainIndex::from_rows(evidence_rows);
    let mut json_rows = Vec::with_capacity(groups.len());
    let mut aggregate_name = "count";
    let mut helper_kind = "helper";
    let mut values_seen = 0usize;
    for (_key, (group_value, row_indices)) in groups {
        let mut object = serde_json::Map::new();
        for item in select {
            let key = native_bool_group_count_output_name(&item.expr, item.alias.as_deref());
            match &item.expr {
                ResolvedExpr::Path(_) => {
                    object.insert(key, group_value.clone());
                }
                ResolvedExpr::AggregateCall {
                    name,
                    arg: Some(arg),
                    star: false,
                    ..
                } => {
                    aggregate_name = aggregate_operator_name(*name);
                    let Some((aggregate_value, group_helper_kind, group_values_seen)) =
                        native_helper_aggregate_value_for_indices(
                            *name,
                            arg,
                            &row_indices,
                            rows,
                            &association_edges,
                            &evidence_index,
                        )?
                    else {
                        return Ok(None);
                    };
                    helper_kind = group_helper_kind;
                    values_seen += group_values_seen;
                    object.insert(key, aggregate_value);
                }
                _ => return Ok(None),
            }
        }
        json_rows.push(Value::Object(object));
    }
    let group_count = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeGroupedHelperAggregateReport {
            aggregate: aggregate_name,
            helper_kind,
            group_property: group_path.display_name.clone(),
            group_logical_type: group_path.logical_type.clone(),
            group_count,
            rows_counted: rows.len(),
            helper_values_seen: values_seen,
        },
    )))
}

fn object_path_is_redacted(row: &MaterializedObjectRow, path: &ResolvedPath) -> bool {
    if row.redacted_properties.contains(&path.display_name) {
        return true;
    }
    path.property_id
        .and_then(|property_id| row.property_ids.get(&property_id))
        .is_some_and(|name| row.redacted_properties.contains(name))
}

fn native_bool_group_count_output_name(expr: &ResolvedExpr, alias: Option<&str>) -> String {
    if let Some(alias) = alias {
        return alias.into();
    }
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::AggregateCall { name, .. } => aggregate_output_name(*name).into(),
        _ => "expr".into(),
    }
}

fn native_direct_group_count_value_is_supported(value: &Value, path: &ResolvedPath) -> bool {
    if value.is_null() {
        return true;
    }
    if path.physical_kind == "file_code" {
        return match path.logical_type.as_str() {
            "bool" | "boolean" => value.as_bool().is_some(),
            "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
            | "float32" | "float64" | "decimal64" | "decimal128" | "date_days"
            | "timestamp_micros" | "timestamp_nanos" => parse_decimal_value(value).is_some(),
            "utf8" | "string" | "uuid" | "binary" => value.as_str().is_some(),
            _ => false,
        };
    }
    match path.logical_type.as_str() {
        "bool" | "boolean" => value.as_bool().is_some(),
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "decimal128" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => parse_decimal_value(value).is_some(),
        _ => false,
    }
}

fn native_grouped_direct_aggregate_eval(
    aggregate: AstAggregateName,
    star: bool,
    arg: Option<&ResolvedExpr>,
    row_indices: &[usize],
    rows: &[ExecutionRow],
) -> Result<NativeDirectAggregateEval, BuildExecutionError> {
    if star {
        let value = match aggregate {
            AstAggregateName::Count => json!(row_indices.len() as u64),
            AstAggregateName::Exists => Value::Bool(!row_indices.is_empty()),
            _ => {
                return Err(exec_error(
                    "E_KERNEL_AGGREGATE",
                    "native grouped aggregate received an unsupported star aggregate",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                ))
            }
        };
        return Ok(NativeDirectAggregateEval {
            value,
            values_seen: row_indices.len(),
            strategy: "row_count",
        });
    }
    let Some(ResolvedExpr::Path(path)) = arg else {
        return Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native grouped aggregate requires a direct path argument",
            json!({ "aggregate": aggregate_operator_name(aggregate) }),
        ));
    };
    let mut state = NativeDirectAggregateState::new();
    for row_index in row_indices {
        let Some(row) = rows.get(*row_index) else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped aggregate row index was outside the reconstructed row set",
                json!({ "row_index": row_index }),
            ));
        };
        let ExecutionRow::Object(row) = row else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped aggregate expected object rows",
                json!({}),
            ));
        };
        if object_path_is_redacted(row, path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped aggregate cannot evaluate a redacted path",
                json!({ "path": path.display_name }),
            ));
        }
        let Some(value) = object_property_value_for_path(row, path) else {
            continue;
        };
        state.accumulate_value(aggregate, path, value)?;
    }
    state.finish(aggregate)
}

fn try_native_direct_aggregate_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeDirectAggregateReport,
    )>,
    BuildExecutionError,
> {
    if !native_direct_aggregate_shape(planned) {
        return Ok(None);
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let [item] = select.as_slice() else {
        return Ok(None);
    };
    let ResolvedExpr::AggregateCall { star, .. } = &item.expr else {
        return Ok(None);
    };
    let Some(aggregate) = native_direct_aggregate_name(planned) else {
        return Ok(None);
    };
    let counted_path = native_direct_aggregate_path(planned);
    let aggregate_eval = native_direct_aggregate_eval(aggregate, *star, counted_path, rows)?;
    let mut object = serde_json::Map::new();
    object.insert(
        native_bool_group_count_output_name(&item.expr, item.alias.as_deref()),
        aggregate_eval.value,
    );
    let (result, row_counts) = finish_json_rows(
        vec![Value::Object(object)],
        rows.len(),
        rows.len(),
        planned,
        options,
        started,
    )?;
    Ok(Some((
        result,
        row_counts,
        NativeDirectAggregateReport {
            aggregate: aggregate_operator_name(aggregate),
            counted_path: counted_path.map(|path| path.display_name.clone()),
            rows_counted: rows.len(),
            values_seen: aggregate_eval.values_seen,
            aggregate_strategy: aggregate_eval.strategy,
        },
    )))
}

fn try_native_helper_aggregate_result(
    rows: &[ExecutionRow],
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[crate::materialized::MaterializedEvidenceRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeHelperAggregateReport,
    )>,
    BuildExecutionError,
> {
    if !native_helper_aggregate_shape(planned) {
        return Ok(None);
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let [item] = select.as_slice() else {
        return Ok(None);
    };
    let ResolvedExpr::AggregateCall {
        name,
        arg: Some(arg),
        star: false,
        ..
    } = &item.expr
    else {
        return Ok(None);
    };

    let (aggregate_value, helper_kind, helper_values_seen) = match arg.as_ref() {
        ResolvedExpr::Association(association) => {
            let edge_table = AssociationEdgeTable::from_rows_for_plan(planned, associations);
            let mut matched_edges = 0u64;
            let mut distinct_endpoints = BTreeSet::new();
            for row in rows {
                let Some(goid) = current_row_goid(row) else {
                    return Ok(None);
                };
                let file_ordinal = current_row_dataset_file_ordinal(row);
                match name {
                    AstAggregateName::Count => {
                        matched_edges +=
                            edge_table.count_for_scoped(file_ordinal, goid, association) as u64;
                    }
                    AstAggregateName::Exists => {
                        if edge_table.exists_for_scoped(file_ordinal, goid, association) {
                            matched_edges += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_endpoints.extend(edge_table.opposite_endpoints_for_scoped(
                            file_ordinal,
                            goid,
                            association,
                        ));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match name {
                AstAggregateName::Count => json!(matched_edges),
                AstAggregateName::Exists => Value::Bool(matched_edges > 0),
                AstAggregateName::DistinctCount => json!(distinct_endpoints.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match name {
                AstAggregateName::DistinctCount => distinct_endpoints.len(),
                _ => usize::try_from(matched_edges).unwrap_or(usize::MAX),
            };
            (value, "association", seen)
        }
        ResolvedExpr::Evidence(evidence) => {
            let evidence_index = EvidenceGrainIndex::from_rows(evidence_rows);
            let mut matched_entries = 0u64;
            let mut distinct_identities = BTreeSet::new();
            for row in rows {
                match name {
                    AstAggregateName::Count => {
                        matched_entries += evidence_index.count_for(row, evidence) as u64;
                    }
                    AstAggregateName::Exists => {
                        if evidence_index.exists_for(row, evidence) {
                            matched_entries += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_identities.extend(evidence_index.identities_for(row, evidence));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match name {
                AstAggregateName::Count => json!(matched_entries),
                AstAggregateName::Exists => Value::Bool(matched_entries > 0),
                AstAggregateName::DistinctCount => json!(distinct_identities.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match name {
                AstAggregateName::DistinctCount => distinct_identities.len(),
                _ => usize::try_from(matched_entries).unwrap_or(usize::MAX),
            };
            (value, "evidence", seen)
        }
        _ => return Ok(None),
    };

    let mut object = serde_json::Map::new();
    object.insert(
        native_bool_group_count_output_name(&item.expr, item.alias.as_deref()),
        aggregate_value,
    );
    let (result, row_counts) = finish_json_rows(
        vec![Value::Object(object)],
        rows.len(),
        rows.len(),
        planned,
        options,
        started,
    )?;
    Ok(Some((
        result,
        row_counts,
        NativeHelperAggregateReport {
            aggregate: aggregate_operator_name(*name),
            helper_kind,
            rows_counted: rows.len(),
            helper_values_seen,
        },
    )))
}

fn try_native_root_scan_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    _options: &ExecutionOptions,
    _started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeRootScanReport,
    )>,
    BuildExecutionError,
> {
    if native_association_root_scan_shape(planned) {
        let ResolvedRoot::Association(root) = &planned.resolved.root else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let ExecutionRow::Association(row) = row else {
                return Ok(None);
            };
            if planned.resolved.operation_context.security.redaction_policy
                == crate::RedactionPolicy::RefuseProtectedValues
                && !row.redacted_properties.is_empty()
            {
                return Err(exec_error(
                    "E_SECURITY_DISCLOSURE_FORBIDDEN",
                    "redacted association values are present but the active redaction policy refuses protected values",
                    json!({}),
                ));
            }
            out.push(row.clone());
        }
        let row_counts = ExecutionRowCounts {
            input_rows: rows.len(),
            filtered_rows: rows.len(),
            output_rows: out.len(),
        };
        return Ok(Some((
            CoveQlExecutionResult::AssociationRows(out),
            row_counts,
            NativeRootScanReport {
                root_kind: "association",
                root_type: root.type_name.clone(),
                rows_returned: rows.len(),
            },
        )));
    }
    if native_evidence_root_scan_shape(planned) {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let ExecutionRow::Evidence(row) = row else {
                return Ok(None);
            };
            out.push(row.clone());
        }
        let row_counts = ExecutionRowCounts {
            input_rows: rows.len(),
            filtered_rows: rows.len(),
            output_rows: out.len(),
        };
        return Ok(Some((
            CoveQlExecutionResult::EvidenceRows(out),
            row_counts,
            NativeRootScanReport {
                root_kind: "evidence",
                root_type: "evidence".into(),
                rows_returned: rows.len(),
            },
        )));
    }
    Ok(None)
}

fn try_native_projection_root_scan_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeProjectionRootScanReport,
    )>,
    BuildExecutionError,
> {
    if !native_projection_root_scan_shape(planned) {
        return Ok(None);
    }
    let Some(projection_id) = projection_backed_root_id(&planned.resolved.root) else {
        return Ok(None);
    };
    if !rows
        .iter()
        .all(|row| matches!(row, ExecutionRow::Projection(_)))
    {
        return Ok(None);
    }
    let (result, row_counts) =
        finish_materialized_rows(rows.to_vec(), &[], &[], &[], planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeProjectionRootScanReport {
            projection_id: projection_id.to_string(),
            rows_returned: rows.len(),
        },
    )))
}

fn try_native_direct_projection_result(
    rows: &[ExecutionRow],
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[crate::materialized::MaterializedEvidenceRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    runtime_exact_helper_projection: bool,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeDirectProjectionReport,
    )>,
    BuildExecutionError,
> {
    if !(native_direct_projection_shape(planned)
        || native_helper_exists_direct_projection_shape(planned)
        || native_role_bound_direct_projection_shape(planned)
        || native_temporal_direct_projection_shape(planned)
        || runtime_exact_helper_projection)
    {
        return Ok(None);
    }
    for row in rows {
        if planned.resolved.operation_context.security.redaction_policy
            == crate::RedactionPolicy::RefuseProtectedValues
            && execution_row_has_redacted_values_for_native_projection(row)
        {
            return Err(exec_error(
                "E_SECURITY_DISCLOSURE_FORBIDDEN",
                "redacted values are present but the active redaction policy refuses protected values",
                json!({}),
            ));
        }
    }

    let context = EvalContext::for_plan(associations, evidence_rows, planned);
    let filtered_rows = native_direct_projection_filter_rows(rows, planned)?;
    let filtered_row_count = filtered_rows.len();
    let mut sorted_rows = filtered_rows;
    sort_rows(&mut sorted_rows, planned, &context)?;
    let projected_rows = apply_skip_take(sorted_rows, planned);

    let json_rows = if let Some(select) = &planned.resolved.method_chain.select {
        let mut out = Vec::with_capacity(projected_rows.len());
        for row in &projected_rows {
            let mut object = serde_json::Map::new();
            for item in select {
                let key = item
                    .alias
                    .clone()
                    .unwrap_or_else(|| native_direct_projection_output_name(&item.expr));
                let value = match &item.expr {
                    ResolvedExpr::Path(path) => row.value_for_path(path),
                    ResolvedExpr::Literal(literal) => literal_value(literal).map_err(|err| {
                        exec_error(
                            "E_EXPRESSION",
                            format!(
                                "native direct projection literal evaluation failed: {}",
                                err.message
                            ),
                            json!({ "column": key }),
                        )
                    })?,
                    _ if native_projection_expr_is_exact(&item.expr) => {
                        eval_expr(&item.expr, row, &context).map_err(|err| {
                            exec_error(
                                "E_EXPRESSION",
                                format!(
                                    "native direct projection expression evaluation failed: {}",
                                    err.message
                                ),
                                json!({ "column": key }),
                            )
                        })?
                    }
                    _ => return Ok(None),
                };
                object.insert(key, value);
            }
            out.push(Value::Object(object));
        }
        out
    } else {
        projected_rows.iter().map(ExecutionRow::to_json).collect()
    };
    let column_count = planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(0, Vec::len);
    let rows_projected = json_rows.len();
    let (result, row_counts) = finish_json_rows(
        json_rows,
        rows.len(),
        filtered_row_count,
        planned,
        options,
        started,
    )?;
    Ok(Some((
        result,
        row_counts,
        NativeDirectProjectionReport {
            root_kind: native_root_kind_name(&planned.resolved.root),
            column_count,
            rows_projected,
        },
    )))
}

fn native_direct_projection_filter_rows(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let Some(predicate) = planned.resolved.method_chain.where_predicate.as_ref() else {
        return Ok(rows.to_vec());
    };
    if !matches!(
        planned.resolved.root,
        ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_)
    ) {
        return Ok(rows.to_vec());
    }
    if !row_root_predicate_is_direct_safe(planned, predicate) {
        return Ok(rows.to_vec());
    }
    rows.iter()
        .filter_map(
            |row| match native_row_root_predicate_matches(row, predicate) {
                Ok(true) => Some(Ok(row.clone())),
                Ok(false) => None,
                Err(err) => Some(Err(err)),
            },
        )
        .collect()
}

fn native_row_root_predicate_matches(
    row: &ExecutionRow,
    predicate: &ResolvedPredicate,
) -> Result<bool, BuildExecutionError> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            if let Some((path, literal)) = native_row_root_path_literal_parts(left, right) {
                return native_row_root_compare(row, path, literal, *op, false);
            }
            if let Some((path, literal)) = native_row_root_path_literal_parts(right, left) {
                return native_row_root_compare(row, path, literal, *op, true);
            }
            Ok(false)
        }
        ResolvedPredicate::InList { expr, values } => {
            let ResolvedExpr::Path(path) = expr else {
                return Ok(false);
            };
            let value = row.value_for_path(path);
            if value.is_null() {
                return Ok(false);
            }
            for literal in values {
                let literal = literal_value(literal).map_err(|err| {
                    exec_error(
                        "E_EXPRESSION",
                        format!(
                            "native row-root IN predicate literal evaluation failed: {}",
                            err.message
                        ),
                        json!({ "path": path.display_name }),
                    )
                })?;
                if !literal.is_null()
                    && native_row_root_values_compare(&value, &literal, AstCompareOp::Eq, path)
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let ResolvedExpr::Path(path) = expr else {
                return Ok(false);
            };
            Ok(row.value_for_path(path).is_null() ^ *negated)
        }
        ResolvedPredicate::BoolExpr(expr) => {
            let ResolvedExpr::Path(path) = expr else {
                return Ok(false);
            };
            Ok(row.value_for_path(path).as_bool().unwrap_or(false))
        }
        ResolvedPredicate::And(parts) => {
            for part in parts {
                if !native_row_root_predicate_matches(row, part)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        ResolvedPredicate::Or(parts) => {
            for part in parts {
                if native_row_root_predicate_matches(row, part)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ResolvedPredicate::Not(_) | ResolvedPredicate::Exists(_) => Ok(false),
    }
}

fn runtime_kernel_shape_for_helper_prefilter_proof(
    shape: &KernelShape,
    planned: &crate::PlannedQuery,
    evidence_helper_runtime_exact: bool,
) -> KernelShape {
    if !evidence_helper_runtime_exact
        || !native_evidence_exists_direct_projection_candidate_shape(planned)
    {
        return shape.clone();
    }
    let mut runtime_shape = shape.clone();
    for contract in &mut runtime_shape.operator_contracts {
        if contract.operator == "predicate_exists:evidence" {
            contract.exact = true;
            contract.residual_required = false;
            contract.reason =
                "evidence existence semi-join is the full predicate and the loaded COVE-MAP evidence index proved an exact visible target fast path".into();
            contract.proof_obligation =
                "evidence semi-join is exact because protected metadata disclosure and exact aggregate disclosure are enabled, the helper predicate is positive and owns the whole WHERE clause, hidden evidence entries were filtered from the loaded evidence index, target grain fallback rows are absent, and rows are already reconstructed visible object states".into();
            contract.residual_reason = None;
            contract.fallback_boundary = None;
        }
    }
    if runtime_shape
        .operator_contracts
        .iter()
        .all(|contract| !contract.residual_required)
    {
        runtime_shape.residual_verification_required = false;
        runtime_shape.reason =
            "runtime COVE-MAP evidence index proof promoted helper existence direct projection to exact native authority".into();
    }
    runtime_shape
}

fn native_row_root_path_literal_parts<'a>(
    path_expr: &'a ResolvedExpr,
    literal_expr: &'a ResolvedExpr,
) -> Option<(&'a ResolvedPath, &'a ResolvedLiteral)> {
    let ResolvedExpr::Path(path) = path_expr else {
        return None;
    };
    let ResolvedExpr::Literal(literal) = literal_expr else {
        return None;
    };
    Some((path, literal))
}

fn native_row_root_compare(
    row: &ExecutionRow,
    path: &ResolvedPath,
    literal: &ResolvedLiteral,
    op: AstCompareOp,
    reversed: bool,
) -> Result<bool, BuildExecutionError> {
    let value = row.value_for_path(path);
    let literal = literal_value(literal).map_err(|err| {
        exec_error(
            "E_EXPRESSION",
            format!(
                "native row-root comparison literal evaluation failed: {}",
                err.message
            ),
            json!({ "path": path.display_name }),
        )
    })?;
    if value.is_null() || literal.is_null() {
        return Ok(false);
    }
    let op = if reversed {
        match op {
            AstCompareOp::Lt => AstCompareOp::Gt,
            AstCompareOp::Le => AstCompareOp::Ge,
            AstCompareOp::Gt => AstCompareOp::Lt,
            AstCompareOp::Ge => AstCompareOp::Le,
            other => other,
        }
    } else {
        op
    };
    Ok(native_row_root_values_compare(&value, &literal, op, path))
}

fn native_row_root_values_compare(
    left: &Value,
    right: &Value,
    op: AstCompareOp,
    path: &ResolvedPath,
) -> bool {
    match op {
        AstCompareOp::Eq => values_equal_typed(left, right, Some(path.logical_type.as_str())),
        AstCompareOp::Ne => !values_equal_typed(left, right, Some(path.logical_type.as_str())),
        AstCompareOp::Lt | AstCompareOp::Le | AstCompareOp::Gt | AstCompareOp::Ge => {
            let Some(ordering) = value_ordering_typed(
                left,
                right,
                Some(path.logical_type.as_str()),
                path.collation_id,
            ) else {
                return false;
            };
            match op {
                AstCompareOp::Lt => ordering.is_lt(),
                AstCompareOp::Le => ordering.is_le(),
                AstCompareOp::Gt => ordering.is_gt(),
                AstCompareOp::Ge => ordering.is_ge(),
                AstCompareOp::Eq | AstCompareOp::Ne => unreachable!(),
            }
        }
    }
}

fn execution_row_has_redacted_values_for_native_projection(row: &ExecutionRow) -> bool {
    match row {
        ExecutionRow::Object(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Association(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Evidence(_) | ExecutionRow::Projection(_) => false,
    }
}

fn native_direct_projection_output_name(expr: &ResolvedExpr) -> String {
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::Literal(_) => "literal".into(),
        ResolvedExpr::FunctionCall { function_id, .. } => function_id.clone(),
        _ => "expr".into(),
    }
}

fn native_root_kind_name(root: &ResolvedRoot) -> &'static str {
    match root {
        ResolvedRoot::Object(_) => "object",
        ResolvedRoot::Association(_) => "association",
        ResolvedRoot::Node(_) => "node",
        ResolvedRoot::Edge(_) => "edge",
        ResolvedRoot::Table(_) => "table",
        ResolvedRoot::Evidence(_) => "evidence",
        ResolvedRoot::Projection(_) => "projection",
    }
}

fn projection_backed_root_id(root: &ResolvedRoot) -> Option<&str> {
    match root {
        ResolvedRoot::Projection(root) => Some(root.projection_id.as_str()),
        ResolvedRoot::Table(root) => Some(root.projection.projection_id.as_str()),
        _ => None,
    }
}

fn native_helper_aggregate_value_for_indices(
    aggregate: AstAggregateName,
    arg: &ResolvedExpr,
    row_indices: &[usize],
    rows: &[ExecutionRow],
    association_edges: &AssociationEdgeTable,
    evidence_index: &EvidenceGrainIndex,
) -> Result<Option<(Value, &'static str, usize)>, BuildExecutionError> {
    match arg {
        ResolvedExpr::Association(association) => {
            let mut matched_edges = 0u64;
            let mut distinct_endpoints = BTreeSet::new();
            for row_index in row_indices {
                let Some(row) = rows.get(*row_index) else {
                    return Err(exec_error(
                        "E_KERNEL_AGGREGATE",
                        "native helper aggregate row index was outside the reconstructed row set",
                        json!({ "row_index": row_index }),
                    ));
                };
                let Some(goid) = current_row_goid(row) else {
                    return Ok(None);
                };
                let file_ordinal = current_row_dataset_file_ordinal(row);
                match aggregate {
                    AstAggregateName::Count => {
                        matched_edges +=
                            association_edges.count_for_scoped(file_ordinal, goid, association)
                                as u64;
                    }
                    AstAggregateName::Exists => {
                        if association_edges.exists_for_scoped(file_ordinal, goid, association) {
                            matched_edges += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_endpoints.extend(association_edges.opposite_endpoints_for_scoped(
                            file_ordinal,
                            goid,
                            association,
                        ));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match aggregate {
                AstAggregateName::Count => json!(matched_edges),
                AstAggregateName::Exists => Value::Bool(matched_edges > 0),
                AstAggregateName::DistinctCount => json!(distinct_endpoints.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match aggregate {
                AstAggregateName::DistinctCount => distinct_endpoints.len(),
                _ => usize::try_from(matched_edges).unwrap_or(usize::MAX),
            };
            Ok(Some((value, "association", seen)))
        }
        ResolvedExpr::Evidence(evidence) => {
            let mut matched_entries = 0u64;
            let mut distinct_identities = BTreeSet::new();
            for row_index in row_indices {
                let Some(row) = rows.get(*row_index) else {
                    return Err(exec_error(
                        "E_KERNEL_AGGREGATE",
                        "native helper aggregate row index was outside the reconstructed row set",
                        json!({ "row_index": row_index }),
                    ));
                };
                match aggregate {
                    AstAggregateName::Count => {
                        matched_entries += evidence_index.count_for(row, evidence) as u64;
                    }
                    AstAggregateName::Exists => {
                        if evidence_index.exists_for(row, evidence) {
                            matched_entries += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_identities.extend(evidence_index.identities_for(row, evidence));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match aggregate {
                AstAggregateName::Count => json!(matched_entries),
                AstAggregateName::Exists => Value::Bool(matched_entries > 0),
                AstAggregateName::DistinctCount => json!(distinct_identities.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match aggregate {
                AstAggregateName::DistinctCount => distinct_identities.len(),
                _ => usize::try_from(matched_entries).unwrap_or(usize::MAX),
            };
            Ok(Some((value, "evidence", seen)))
        }
        _ => Ok(None),
    }
}

#[derive(Debug)]
struct NativeDirectAggregateEval {
    value: Value,
    values_seen: usize,
    strategy: &'static str,
}

#[derive(Debug)]
struct NativeDirectAggregateState<'a> {
    values_seen: usize,
    distinct: BTreeSet<String>,
    selected: Option<&'a Value>,
    exact_sum: Option<ExactDecimal>,
    float_sum: f64,
    saw_float: bool,
}

impl<'a> NativeDirectAggregateState<'a> {
    fn new() -> Self {
        Self {
            values_seen: 0,
            distinct: BTreeSet::new(),
            selected: None,
            exact_sum: None,
            float_sum: 0.0,
            saw_float: false,
        }
    }

    fn accumulate_value(
        &mut self,
        aggregate: AstAggregateName,
        path: &ResolvedPath,
        value: &'a Value,
    ) -> Result<(), BuildExecutionError> {
        if value.is_null() {
            return Ok(());
        }
        self.values_seen += 1;

        match aggregate {
            AstAggregateName::Count | AstAggregateName::Exists => {}
            AstAggregateName::DistinctCount => {
                self.distinct
                    .insert(logical_value_key(value, Some(path.logical_type.as_str())));
            }
            AstAggregateName::Min | AstAggregateName::Max => {
                let replace = match self.selected {
                    None => true,
                    Some(current) => {
                        value_ordering_typed(
                            value,
                            current,
                            Some(path.logical_type.as_str()),
                            path.collation_id,
                        )
                        .ok_or_else(|| {
                            exec_error(
                                "E_KERNEL_AGGREGATE",
                                "native min/max aggregate could not prove typed value ordering",
                                json!({
                                    "aggregate": aggregate_operator_name(aggregate),
                                    "logical_type": path.logical_type,
                                }),
                            )
                        })?
                        .is_lt()
                            == matches!(aggregate, AstAggregateName::Min)
                    }
                };
                if replace {
                    self.selected = Some(value);
                }
            }
            AstAggregateName::Sum | AstAggregateName::Avg => {
                accumulate_native_direct_numeric_value(
                    aggregate,
                    path,
                    value,
                    &mut self.exact_sum,
                    &mut self.float_sum,
                    &mut self.saw_float,
                )?;
            }
        }
        Ok(())
    }

    fn finish(
        self,
        aggregate: AstAggregateName,
    ) -> Result<NativeDirectAggregateEval, BuildExecutionError> {
        let value = match aggregate {
            AstAggregateName::Count => json!(self.values_seen as u64),
            AstAggregateName::Exists => Value::Bool(self.values_seen > 0),
            AstAggregateName::DistinctCount => json!(self.distinct.len() as u64),
            AstAggregateName::Min | AstAggregateName::Max => {
                self.selected.cloned().unwrap_or(Value::Null)
            }
            AstAggregateName::Sum | AstAggregateName::Avg => {
                finish_native_direct_numeric_aggregate(
                    aggregate,
                    self.exact_sum,
                    self.float_sum,
                    self.saw_float,
                    self.values_seen,
                )?
            }
        };
        let strategy = match aggregate {
            AstAggregateName::Sum | AstAggregateName::Avg => "single_pass_typed_numeric",
            AstAggregateName::Min | AstAggregateName::Max => "single_pass_typed_order",
            AstAggregateName::Count
            | AstAggregateName::Exists
            | AstAggregateName::DistinctCount => "single_pass_value_ref",
        };
        Ok(NativeDirectAggregateEval {
            value,
            values_seen: self.values_seen,
            strategy,
        })
    }
}

fn native_direct_aggregate_eval(
    aggregate: AstAggregateName,
    star: bool,
    path: Option<&ResolvedPath>,
    rows: &[ExecutionRow],
) -> Result<NativeDirectAggregateEval, BuildExecutionError> {
    if star {
        let value = match aggregate {
            AstAggregateName::Count => json!(rows.len() as u64),
            AstAggregateName::Exists => Value::Bool(!rows.is_empty()),
            _ => {
                return Err(exec_error(
                    "E_KERNEL_AGGREGATE",
                    "native direct aggregate received an unsupported star aggregate",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                ))
            }
        };
        return Ok(NativeDirectAggregateEval {
            value,
            values_seen: rows.len(),
            strategy: "row_count",
        });
    }

    let Some(path) = path else {
        return Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native direct aggregate requires a direct path argument",
            json!({ "aggregate": aggregate_operator_name(aggregate) }),
        ));
    };

    let mut state = NativeDirectAggregateState::new();
    for row in rows {
        let ExecutionRow::Object(row) = row else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native direct aggregate expected object rows",
                json!({}),
            ));
        };
        if object_path_is_redacted(row, path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native direct aggregate cannot evaluate a redacted path",
                json!({ "path": path.display_name }),
            ));
        }
        let Some(value) = object_property_value_for_path(row, path) else {
            continue;
        };
        state.accumulate_value(aggregate, path, value)?;
    }
    state.finish(aggregate)
}

fn object_property_value_for_path<'a>(
    row: &'a MaterializedObjectRow,
    path: &ResolvedPath,
) -> Option<&'a Value> {
    let property_id = path.property_id?;
    if let Some(name) = row.property_ids.get(&property_id) {
        return row.properties.get(name);
    }
    row.properties.get(&path.display_name)
}

fn accumulate_native_direct_numeric_value(
    aggregate: AstAggregateName,
    path: &ResolvedPath,
    value: &Value,
    exact_sum: &mut Option<ExactDecimal>,
    float_sum: &mut f64,
    saw_float: &mut bool,
) -> Result<(), BuildExecutionError> {
    let numeric_is_float = matches!(path.logical_type.as_str(), "float32" | "float64")
        || value
            .as_number()
            .is_some_and(|number| number.as_i64().is_none() && number.as_u64().is_none());
    if numeric_is_float {
        let Some(number) = value.as_f64() else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native sum/avg aggregate requires numeric materialized values",
                json!({
                    "aggregate": aggregate_operator_name(aggregate),
                    "logical_type": path.logical_type,
                }),
            ));
        };
        *saw_float = true;
        *float_sum += number;
        return Ok(());
    }
    let Some(decimal) = parse_decimal_value(value) else {
        return Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native sum/avg aggregate requires numeric materialized values",
            json!({
                "aggregate": aggregate_operator_name(aggregate),
                "logical_type": path.logical_type,
            }),
        ));
    };
    *exact_sum = Some(match exact_sum.take() {
        Some(sum) => sum.checked_add(decimal).ok_or_else(|| {
            exec_error(
                "E_KERNEL_AGGREGATE",
                "native sum/avg aggregate overflowed decimal accumulator",
                json!({
                    "aggregate": aggregate_operator_name(aggregate),
                    "logical_type": path.logical_type,
                }),
            )
        })?,
        None => decimal,
    });
    Ok(())
}

fn finish_native_direct_numeric_aggregate(
    aggregate: AstAggregateName,
    exact_sum: Option<ExactDecimal>,
    float_sum: f64,
    saw_float: bool,
    count: usize,
) -> Result<Value, BuildExecutionError> {
    if count == 0 {
        return Ok(Value::Null);
    }
    match aggregate {
        AstAggregateName::Sum if saw_float => Ok(json!(float_sum)),
        AstAggregateName::Avg if saw_float => Ok(json!(float_sum / count as f64)),
        AstAggregateName::Sum => exact_sum
            .ok_or_else(|| {
                exec_error(
                    "E_KERNEL_AGGREGATE",
                    "empty native numeric aggregate accumulator",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                )
            })?
            .to_json_sum()
            .map_err(|message| exec_error("E_KERNEL_AGGREGATE", message, json!({}))),
        AstAggregateName::Avg => exact_sum
            .ok_or_else(|| {
                exec_error(
                    "E_KERNEL_AGGREGATE",
                    "empty native numeric aggregate accumulator",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                )
            })?
            .checked_div_u64(count as u64)
            .ok_or_else(|| {
                exec_error(
                    "E_KERNEL_AGGREGATE",
                    "native avg aggregate overflowed decimal accumulator",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                )
            })?
            .to_json_sum()
            .map_err(|message| exec_error("E_KERNEL_AGGREGATE", message, json!({}))),
        _ => Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native numeric aggregate received a nonnumeric aggregate",
            json!({ "aggregate": aggregate_operator_name(aggregate) }),
        )),
    }
}

fn try_native_typed_order_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeTypedOrderReport,
    )>,
    BuildExecutionError,
> {
    if !native_typed_order_shape(planned) {
        return Ok(None);
    }
    let Some(order_path) = native_typed_order_path(planned) else {
        return Ok(None);
    };
    let Some(order) = &planned.resolved.method_chain.order_by else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let mut ordered = Vec::<&MaterializedObjectRow>::with_capacity(rows.len());
    for row in rows {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, order_path) {
            return Ok(None);
        }
        let value = row.value_for_path(order_path);
        if !native_typed_order_value_is_supported(&value, order_path.logical_type.as_str()) {
            return Ok(None);
        }
        if select.iter().any(|item| {
            matches!(
                &item.expr,
                ResolvedExpr::Path(path) if object_path_is_redacted(row, path)
            )
        }) {
            return Ok(None);
        }
        ordered.push(row);
    }
    ordered.sort_by(|left, right| {
        let left_value = left.value_for_path(order_path);
        let right_value = right.value_for_path(order_path);
        compare_typed_sort_values(
            &left_value,
            &right_value,
            order.direction,
            order.nulls,
            order_path.logical_type.as_str(),
            order_path.collation_id,
        )
        .then_with(|| stable_value_key(&left.to_json()).cmp(&stable_value_key(&right.to_json())))
    });
    let rows_sorted = ordered.len();
    let skip = planned.resolved.method_chain.skip.unwrap_or(0) as usize;
    let take = planned
        .resolved
        .method_chain
        .take
        .map(|value| value as usize)
        .unwrap_or(usize::MAX);
    let mut json_rows = Vec::new();
    for row in ordered.into_iter().skip(skip).take(take) {
        let mut object = serde_json::Map::new();
        for item in select {
            let ResolvedExpr::Path(path) = &item.expr else {
                return Ok(None);
            };
            object.insert(
                native_bool_group_count_output_name(&item.expr, item.alias.as_deref()),
                row.value_for_path(path),
            );
        }
        json_rows.push(Value::Object(object));
    }
    let output_rows = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeTypedOrderReport {
            order_property: order_path.display_name.clone(),
            logical_type: order_path.logical_type.clone(),
            collation_id: order_path.collation_id,
            sort_key_boundary: native_typed_order_sort_key_boundary(order_path),
            rows_sorted,
            output_rows,
        },
    )))
}

fn native_typed_order_value_is_supported(value: &Value, logical_type: &str) -> bool {
    value.is_null()
        || match logical_type {
            "bool" | "boolean" => value.as_bool().is_some(),
            "utf8" | "string" => value.as_str().is_some(),
            "int8" | "int16" | "int32" | "int64" | "decimal64" | "date_days"
            | "timestamp_micros" | "timestamp_nanos" => value.as_i64().is_some(),
            "uint8" | "uint16" | "uint32" | "uint64" => value.as_u64().is_some(),
            "float32" | "float64" => value.as_f64().is_some(),
            _ => false,
        }
}

fn native_typed_order_sort_key_boundary(path: &ResolvedPath) -> Option<String> {
    (path.physical_kind == "file_code").then(|| match path.collation_id {
        Some(id) if id == CollationKind::Utf8Bytewise.id() => {
            "decoded_filecode_sort_key_under_declared_collation".to_string()
        }
        _ => "decoded_filecode_sort_key_under_default_collation".to_string(),
    })
}

fn compare_typed_sort_values(
    left: &Value,
    right: &Value,
    direction: AstOrderDirection,
    nulls: AstNullOrdering,
    logical_type: &str,
    collation_id: Option<u16>,
) -> Ordering {
    let nulls = match nulls {
        AstNullOrdering::Default => match direction {
            AstOrderDirection::Asc => AstNullOrdering::NullsLast,
            AstOrderDirection::Desc => AstNullOrdering::NullsFirst,
        },
        explicit => explicit,
    };
    match (left.is_null(), right.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => match nulls {
            AstNullOrdering::NullsFirst => Ordering::Less,
            AstNullOrdering::NullsLast | AstNullOrdering::Default => Ordering::Greater,
        },
        (false, true) => match nulls {
            AstNullOrdering::NullsFirst => Ordering::Greater,
            AstNullOrdering::NullsLast | AstNullOrdering::Default => Ordering::Less,
        },
        (false, false) => {
            let ordering = value_ordering_typed(left, right, Some(logical_type), collation_id)
                .unwrap_or(Ordering::Equal);
            match direction {
                AstOrderDirection::Asc => ordering,
                AstOrderDirection::Desc => ordering.reverse(),
            }
        }
    }
}

fn execute_physical_explain_only(
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

fn attach_phase8_plan_reports(report: &mut KernelExecutionReport, planned: &crate::PlannedQuery) {
    report.association = crate::AssociationOptimizationReport::for_plan(planned, &[]);
    report.evidence = crate::EvidenceOptimizationReport::for_plan(planned, None);
    report.lineage = crate::LineageReuseReport::for_plan(planned);
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct KernelHelperPrefilterReport {
    input_rows: usize,
    output_rows: usize,
    association_semi_join_terms: usize,
    association_anti_join_terms: usize,
    evidence_semi_join_terms: usize,
    evidence_anti_join_terms: usize,
    skipped_complex_terms: usize,
    skipped_security_terms: usize,
}

impl KernelHelperPrefilterReport {
    fn applied_terms(&self) -> usize {
        self.association_semi_join_terms
            + self.association_anti_join_terms
            + self.evidence_semi_join_terms
            + self.evidence_anti_join_terms
    }

    fn skipped_terms(&self) -> usize {
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

fn apply_kernel_helper_prefilters(
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

fn kernel_helper_prefilters_are_safe(planned: &crate::PlannedQuery) -> bool {
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

fn predicate_contains_helper_exists(predicate: &ResolvedPredicate) -> bool {
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

fn expr_contains_helper_exists(expr: &ResolvedExpr) -> bool {
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

fn execute_kernel_object(
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
            .map_err(|err| exec_error("E_KERNEL_RECONSTRUCT", err, json!({})))?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_only_min_max_decoder_handles_exact_scalar_payloads() {
        assert_eq!(
            decode_index_only_min_max_value("int64", &42i64.to_le_bytes()).unwrap(),
            json!(42)
        );
        assert_eq!(
            decode_index_only_min_max_value("uint64", &42u64.to_le_bytes()).unwrap(),
            json!(42)
        );
        assert_eq!(
            decode_index_only_min_max_value("date_days", &12i32.to_le_bytes()).unwrap(),
            json!(12)
        );
        let mut utf8 = Vec::new();
        wire::append_u64_leb128(&mut utf8, 3);
        utf8.extend_from_slice(b"abc");
        assert_eq!(
            decode_index_only_min_max_value("utf8", &utf8).unwrap(),
            json!("abc")
        );
    }

    #[test]
    fn index_only_min_max_decoder_rejects_unsupported_payloads() {
        assert!(decode_index_only_min_max_value("int64", &[1, 2, 3]).is_err());
        assert!(decode_index_only_min_max_value("binary", &[1, 2, 3]).is_err());
        let mut bad_utf8 = Vec::new();
        wire::append_u64_leb128(&mut bad_utf8, 2);
        bad_utf8.extend_from_slice(&[0xff, 0xff]);
        assert!(decode_index_only_min_max_value("utf8", &bad_utf8).is_err());
    }

    #[test]
    fn native_scalar_exact_predicate_proof_requires_pages_or_empty_short_circuit() {
        let mut prune = NativeScalarPredicatePrune {
            bitmap: SelectionBitmap::none(0),
            predicate_count: 1,
            executed_predicate_count: 1,
            page_count: 0,
            matched_rows: 0,
            predicate_order: vec!["bool_eq"],
            predicate_dispatch: NativeKernelDispatchCounts::default(),
            bitmap_dispatch: NativeBitmapDispatchCounts::default(),
            retained_page_buffers: false,
            short_circuited: false,
        };

        assert!(!native_scalar_prune_covers_kernel_predicate_count(
            &prune, 1
        ));

        prune.page_count = 1;
        assert!(native_scalar_prune_covers_kernel_predicate_count(&prune, 1));

        prune.predicate_count = 2;
        prune.executed_predicate_count = 1;
        prune.short_circuited = true;
        assert!(native_scalar_prune_covers_kernel_predicate_count(&prune, 2));

        assert!(!code_prune_covers_kernel_predicate_count(1, 0, 1));
        assert!(!code_prune_covers_kernel_predicate_count(1, 1, 2));
        assert!(code_prune_covers_kernel_predicate_count(2, 1, 2));
    }
}
