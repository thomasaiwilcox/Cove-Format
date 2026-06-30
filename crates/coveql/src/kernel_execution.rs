use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use cove_core::{
    array::{CoveArrayValue, EncodedArray},
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
        read_object_kernel_surface_from_bytes_with_options, read_retained_object_temporal_segments,
        CoveObjectKernelReadOptions, CoveObjectKernelSurface, CoveObjectReadWithPushdownOptions,
        RetainedTemporalSegmentData, TemporalSegmentData,
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
    CoviCandidateSetV2, CoviLookupKeyV2, CoviLookupRequestV2, CoviLookupTargetV2,
    CoviValidationContextV2, ValidatedCoviArtifactV2,
};
use cove_index::{CoviRowRangePostingV2, IndexCapabilityExactnessV2};
use serde_json::{json, Value};

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
    kernel_ai::{
        try_ai_descriptor_metadata_executed_query, try_ai_embedding_executed_query,
        try_ai_semantic_search_executed_query,
    },
    kernel_index::try_index_only_executed_query,
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
    CoveQlExecutionResult, ExecutedQuery, ExecutionAuthorityReport, ExecutionDiagnostic,
    ExecutionOptions, ExecutionRowCounts, ManifestDatasetMember, MetadataDisclosurePolicy,
    ParseOptions, PhysicalPlanOptions, PhysicalPlannedQuery, PlanOptions, ResolveOptions,
    ResolvedExpr, ResolvedLiteral, ResolvedLiteralValue, ResolvedPath, ResolvedPredicate,
    ResolvedRoot, ResolvedSystemField, TemporalMode, VisibilityPolicy,
};

mod association_evidence;
mod manifest;
mod native_aggregate;
mod native_order;
mod native_projection;
mod native_scalar;
mod pruning;
mod reports;

use association_evidence::*;
pub use manifest::execute_manifest_physical_planned_query;
use manifest::kernel_unsupported_diagnostic_code;
pub(crate) use native_aggregate::aggregate_output_name;
use native_aggregate::*;
use native_order::*;
use native_projection::*;
use native_scalar::*;
use pruning::*;
use reports::*;
pub(crate) use reports::{attach_phase8_plan_reports, execution_diagnostics_for_physical};

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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
