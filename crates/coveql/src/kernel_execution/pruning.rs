use super::*;

pub(super) fn try_covi_empty_lookup_executed_query(
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

pub(super) fn try_coverage_row_range_prune(
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

pub(super) fn physical_predicate_form_exists(
    physical: &PhysicalPlannedQuery,
    form_id: u32,
) -> bool {
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

pub(super) fn coverage_row_range_bitmap(
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

pub(super) fn coverage_row_range_contains_loaded_record(
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

pub(super) fn try_covi_row_range_prune(
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

pub(super) fn exact_covi_row_range_bitmap(
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

pub(super) fn covi_row_range_contains_loaded_record(
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

pub(super) fn try_execution_code_filecode_prune(
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

pub(super) fn try_file_code_literal_prune(
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

pub(super) fn execution_code_predicate_requests(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
) -> Result<Vec<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let mut out = Vec::new();
    collect_execution_code_predicate_requests(predicate, root_object_type_id, &mut out)?;
    Ok(out)
}

pub(super) fn collect_execution_code_predicate_requests(
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

pub(super) fn execution_code_compare_request(
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

pub(super) fn execution_code_in_list_request(
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

pub(super) fn execution_code_same_path_or_request(
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

pub(super) fn execution_code_or_atom_request(
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

pub(super) fn execution_code_bool_expr_request(
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

pub(super) fn execution_code_not_request(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
) -> Result<Option<ExecutionCodePredicateRequest>, BuildExecutionError> {
    let Some(mut request) = execution_code_or_atom_request(predicate, root_object_type_id)? else {
        return Ok(None);
    };
    request.match_mode = request.match_mode.inverted();
    Ok(Some(request))
}

pub(super) fn normalize_canonical_literals(canonical_literals: &mut Vec<ExecutionCodeLiteralKey>) {
    canonical_literals.sort();
    canonical_literals.dedup();
}

pub(super) fn filecode_property_path_parts(
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

pub(super) fn canonical_literal_key(
    logical_type: &str,
    literal: &ResolvedLiteral,
) -> Result<Option<ExecutionCodeLiteralKey>, BuildExecutionError> {
    let Some(value) = canonical_value_for_resolved_literal(logical_type, literal)? else {
        return Ok(None);
    };
    execution_code_literal_key_from_canonical(value, logical_type).map(Some)
}

pub(super) fn execution_code_literal_key_from_canonical(
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

pub(super) fn file_code_for_canonical(
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

pub(super) fn unsigned_execution_code(
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

pub(super) fn execution_code_bitmap_for_request(
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

pub(super) fn file_code_bitmap_for_request(
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

pub(super) fn set_file_code_prune_surface_row(
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

pub(super) fn logical_type_for_execution_code_request(
    logical_type: &str,
) -> Option<CoveLogicalType> {
    logical_type_to_cove(logical_type)
}

pub(super) fn temporal_segments_from_bytes(
    bytes: &[u8],
) -> Result<Vec<TemporalSegmentData>, BuildExecutionError> {
    temporal_segments_from_bytes_with_context(bytes, "E_EXECUTION_CODE_MAP", "COVE-E")
}

pub(super) fn temporal_segments_from_bytes_with_context(
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

pub(super) fn covi_first_empty_lookup(
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

pub(super) fn covi_empty_lookup_requests(
    predicate: &ResolvedPredicate,
    root_object_type_id: u32,
) -> Result<Vec<CoviLookupRequestV2>, BuildExecutionError> {
    let mut requests = Vec::new();
    collect_conjunctive_covi_empty_lookup_requests(predicate, root_object_type_id, &mut requests)?;
    Ok(requests)
}

pub(super) fn collect_conjunctive_covi_empty_lookup_requests(
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
                    let Some(target) = covi_lookup_target_for_path(path) else {
                        return Ok(());
                    };
                    out.push(CoviLookupRequestV2::membership_target(target, keys));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn covi_compare_eq_request(
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
    let Some(target) = covi_lookup_target_for_path(path) else {
        return Ok(None);
    };
    Ok(Some(CoviLookupRequestV2::eq_target(target, key)))
}

pub(super) fn covi_object_property_path(
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

pub(super) fn covi_lookup_target_for_path(path: &ResolvedPath) -> Option<CoviLookupTargetV2> {
    Some(CoviLookupTargetV2::ObjectProperty {
        object_type_id: path.object_type_id?,
        property_id: path.property_id?,
    })
}

pub(super) fn covi_canonical_lookup_key(
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

pub(super) fn canonical_value_for_resolved_literal<'a>(
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

pub(super) fn unsigned_literal_as_signed_canonical_value(
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

pub(super) fn signed_literal_canonical_value(
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
            let value32 = i32::try_from(value).map_err(|_| {
                exec_error(
                    "E_COVI_LOOKUP_UNSAFE",
                    "signed literal does not fit int32 index key domain",
                    json!({ "value": value }),
                )
            })?;
            if logical == CoveLogicalType::DateDays {
                Some(CanonicalValue::DateDays(value32))
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

pub(super) fn unsigned_literal_canonical_value(
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

pub(super) fn tagged_canonical_lookup_key(
    value: CanonicalValue<'_>,
) -> Result<Vec<u8>, BuildExecutionError> {
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

pub(super) fn logical_type_to_cove(logical_type: &str) -> Option<CoveLogicalType> {
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

pub(super) fn covi_lookup_request_key_count(request: &CoviLookupRequestV2) -> usize {
    1 + request.membership_keys.len()
}

pub(super) fn covi_lookup_target_json(target: &CoviLookupTargetV2) -> Value {
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
