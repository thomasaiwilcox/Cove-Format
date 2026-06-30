use super::*;

pub(super) fn execute_rows(
    bytes: &[u8],
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
    match &planned.resolved.root {
        ResolvedRoot::Projection(root) => {
            let (result, row_counts) =
                execute_projection_root(bytes, planned, options, started, &root.projection_id)?;
            let report = projection_readback_pushdown_report(planned, options, &row_counts);
            Ok((result, row_counts, report, None))
        }
        ResolvedRoot::Table(root) => {
            let (result, row_counts) = if table_relation_context_required(planned) {
                execute_table_relation_root(bytes, planned, options, started, root)?
            } else if matches!(
                &root.execution_authority,
                TableExecutionAuthority::DeterministicProjection { .. }
            ) {
                execute_projection_root(
                    bytes,
                    planned,
                    options,
                    started,
                    &root.projection.projection_id,
                )?
            } else {
                execute_table_relation_root(bytes, planned, options, started, root)?
            };
            let report = projection_readback_pushdown_report(planned, options, &row_counts);
            Ok((result, row_counts, report, None))
        }
        ResolvedRoot::Object(_)
        | ResolvedRoot::Association(_)
        | ResolvedRoot::Edge(_)
        | ResolvedRoot::Evidence(_) => execute_object_backed_root(bytes, planned, options, started),
        ResolvedRoot::Node(root) => {
            if planned.resolved.method_chain.traversals.is_empty()
                && planned.resolved.method_chain.graph_algorithms.is_empty()
            {
                execute_object_backed_root(bytes, planned, options, started)
            } else {
                let (result, row_counts) =
                    execute_graph_traverse_root(bytes, planned, options, started, root)?;
                Ok((
                    result,
                    row_counts,
                    PushdownReport::not_executed(&options.pushdown),
                    None,
                ))
            }
        }
    }
}

pub(super) fn execute_rows_on_object_surface(
    surface: &CoveObjectSurface,
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
    match &planned.resolved.root {
        ResolvedRoot::Object(_)
        | ResolvedRoot::Association(_)
        | ResolvedRoot::Edge(_)
        | ResolvedRoot::Evidence(_) => {
            execute_object_surface_root(surface, planned, options, started)
        }
        ResolvedRoot::Node(_) if planned.resolved.method_chain.traversals.is_empty()
            && planned.resolved.method_chain.graph_algorithms.is_empty() =>
        {
            execute_object_surface_root(surface, planned, options, started)
        }
        ResolvedRoot::Node(root) => {
            let (result, row_counts) =
                execute_graph_traverse_surface_root(surface, planned, options, started, root)?;
            Ok((
                result,
                row_counts,
                PushdownReport::not_applicable(
                    &options.pushdown,
                    "object surface execution reads a pre-composed COVE-O surface and cannot apply byte-level pushdown",
                ),
                None,
            ))
        }
        ResolvedRoot::Projection(_) | ResolvedRoot::Table(_) => Err(exec_error(
            "E_UNSUPPORTED_SURFACE_ROOT",
            "object surface execution does not yet support projection or table roots; use materialized snapshot execution",
            json!({}),
        )),
    }
}

pub(super) fn execute_object_backed_root(
    bytes: &[u8],
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
    let mut source = object_backed_row_source(bytes, planned, options, started)?;
    let evidence_authority = source.evidence_authority;
    let (result, row_counts) = finish_materialized_rows(
        source.rows,
        &source.associations,
        &source.evidence_rows,
        &source.object_rows,
        planned,
        options,
        started,
    )?;
    source.pushdown_report.counters.rows_after_candidate_retain = row_counts.input_rows;
    Ok((
        result,
        row_counts,
        source.pushdown_report,
        evidence_authority,
    ))
}

pub(super) fn execute_object_surface_root(
    surface: &CoveObjectSurface,
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
    let mut source = object_surface_row_source(
        surface,
        planned,
        options,
        started,
        PushdownReport::not_applicable(
            &options.pushdown,
            "object surface execution reads a pre-composed COVE-O surface and cannot apply byte-level pushdown",
        ),
    )?;
    let evidence_authority = source.evidence_authority;
    let (result, row_counts) = finish_materialized_rows(
        source.rows,
        &source.associations,
        &source.evidence_rows,
        &source.object_rows,
        planned,
        options,
        started,
    )?;
    source.pushdown_report.counters.rows_after_candidate_retain = row_counts.input_rows;
    Ok((
        result,
        row_counts,
        source.pushdown_report,
        evidence_authority,
    ))
}

pub(super) fn object_backed_row_source(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<MaterializedRowSource, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let read_options = object_read_options(planned);
    let mut pushdown_plan = pushdown::pushdown_read_plan(planned, &options.pushdown);
    let read_result = read_object_surface_from_bytes_with_pushdown_options(
        bytes,
        &CoveObjectReadWithPushdownOptions {
            read: read_options,
            pushdown: pushdown_plan.read_options.clone(),
        },
    )
    .map_err(|err| {
        exec_error(
            "E_READBACK",
            format!("COVE-O materialized readback failed: {err}"),
            json!({}),
        )
    })?;
    pushdown_plan
        .report
        .merge_core_report(read_result.pushdown_report);
    let surface = read_result.surface;
    object_surface_row_source(&surface, planned, options, started, pushdown_plan.report)
}

pub(super) fn object_surface_row_source(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    pushdown_report: PushdownReport,
) -> Result<MaterializedRowSource, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let states = object_states_for_temporal_context(surface, planned)?;
    let object_rows = states
        .iter()
        .map(MaterializedObjectRow::from_state)
        .map(|row| row.with_output_grain(OutputGrain::LatestState))
        .collect::<Vec<_>>();
    let associations = states
        .iter()
        .filter_map(MaterializedAssociationRow::from_state)
        .collect::<Vec<_>>();
    let (evidence_execution_rows, _) =
        materialized_evidence_rows_for_plan(planned, surface.evidence_index.as_ref());
    let evidence_authority = evidence_authority_for_rows(
        planned,
        surface.evidence_index.is_some(),
        !evidence_execution_rows.is_empty(),
    );
    let evidence_execution_rows = if evidence_execution_rows.is_empty()
        && matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
    {
        evidence_object_rows_from_states(&states)
    } else {
        evidence_execution_rows
    };
    let evidence_authority = evidence_authority.or_else(|| {
        evidence_authority_for_rows(
            planned,
            surface.evidence_index.is_some(),
            !evidence_execution_rows.is_empty(),
        )
    });
    require_evidence_catalog_or_objects(
        planned,
        &surface.evidence_index,
        &evidence_execution_rows,
    )?;
    let evidence_rows = evidence_execution_rows
        .iter()
        .filter_map(|row| match row {
            ExecutionRow::Evidence(row) => Some(row.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let context_associations = filter_association_context_rows(&associations, planned, options);
    let context_evidence_rows = filter_evidence_context_rows(&evidence_rows, planned, options);
    let all_rows = match &planned.resolved.root {
        ResolvedRoot::Object(root)
            if planned.resolved.method_chain.history.is_some()
                || planned.resolved.method_chain.changes.is_some() =>
        {
            temporal_object_rows(surface, planned, root.object_type_id)?
        }
        ResolvedRoot::Association(root)
            if planned.resolved.method_chain.history.is_some()
                || planned.resolved.method_chain.changes.is_some() =>
        {
            temporal_association_rows(surface, planned, root.object_type_id)?
        }
        ResolvedRoot::Object(root) => object_rows
            .iter()
            .filter(|state| state.object_type_id == root.object_type_id)
            .cloned()
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
        ResolvedRoot::Node(root) => object_rows
            .iter()
            .filter(|state| state.object_type_id == root.object.object_type_id)
            .cloned()
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
            unreachable!("table roots are handled through projection readback")
        }
        ResolvedRoot::Projection(_) => unreachable!("projection handled separately"),
    };
    Ok(MaterializedRowSource {
        rows: all_rows,
        associations: context_associations,
        evidence_rows: context_evidence_rows,
        object_rows,
        pushdown_report,
        evidence_authority,
    })
}

pub(super) fn evidence_authority_for_rows(
    planned: &PlannedQuery,
    has_evidence_index: bool,
    has_evidence_rows: bool,
) -> Option<EvidenceAuthority> {
    if !matches!(planned.resolved.root, ResolvedRoot::Evidence(_)) {
        return None;
    }
    Some(if has_evidence_index {
        EvidenceAuthority::CoveMapMetadata
    } else if has_evidence_rows {
        EvidenceAuthority::MaterializedEvidenceObjects
    } else {
        EvidenceAuthority::Missing
    })
}
