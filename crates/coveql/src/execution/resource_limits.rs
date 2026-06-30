use super::*;

pub(super) fn require_evidence_catalog_or_objects(
    planned: &PlannedQuery,
    index: &Option<cove_core::profile::cove_map::MapEvidenceIndex>,
    rows: &[ExecutionRow],
) -> Result<(), BuildExecutionError> {
    if matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
        && index.is_none()
        && rows.is_empty()
    {
        return Err(exec_error(
            "E_EVIDENCE_CATALOG_REQUIRED",
            "evidence roots require embedded COVE-MAP evidence metadata or materialized COVE-O evidence objects",
            json!({}),
        ));
    }
    Ok(())
}

pub(crate) fn evidence_object_rows_from_states(states: &[CoveObjectState]) -> Vec<ExecutionRow> {
    states
        .iter()
        .filter(|state| state.object_type_flags & OBJECT_TYPE_FLAG_EVIDENCE_OBJECT != 0)
        .map(|state| {
            let mut fields = BTreeMap::new();
            fields.insert("evidence_id".into(), Value::String(hex(&state.goid)));
            fields.insert(
                "record_id".into(),
                Value::String(hex(&state.latest_record_id)),
            );
            fields.insert("object_type_id".into(), json!(state.object_type_id));
            fields.insert(
                "object_type_name".into(),
                Value::String(state.object_type_name.clone()),
            );
            fields.insert("branch_key".into(), json!(state.branch_key));
            fields.insert("timestamp_us".into(), json!(state.timestamp_us));
            fields.insert("csn".into(), json!(state.csn));
            fields.insert("grain".into(), Value::String("object".into()));
            for property in &state.properties {
                insert_evidence_property(&mut fields, property);
            }
            ExecutionRow::Evidence(MaterializedEvidenceRow { fields })
        })
        .collect()
}

pub(super) fn insert_evidence_property(
    fields: &mut BTreeMap<String, Value>,
    property: &CoveObjectPropertyValue,
) {
    fields.insert(property.property_name.clone(), property.value.clone());
    if property.flags & PROPERTY_FLAG_EVIDENCE_REF != 0 {
        fields.insert("source_evidence_id".into(), property.value.clone());
    }
    if property.flags & PROPERTY_FLAG_MAPPING_RULE_REF != 0 {
        fields.insert("rule_id".into(), property.value.clone());
    }
}

pub(crate) fn object_read_options(planned: &PlannedQuery) -> CoveObjectReadOptions {
    CoveObjectReadOptions {
        requested_property_ids: planned.dependencies.property_ids.iter().copied().collect(),
        requested_property_names: planned
            .dependencies
            .temporal_role_bindings
            .iter()
            .cloned()
            .collect(),
        requested_object_type_names: planned
            .dependencies
            .object_type_names
            .iter()
            .cloned()
            .collect(),
        requested_evidence_metadata_keys: planned
            .dependencies
            .evidence_fields
            .iter()
            .filter_map(|field| {
                field
                    .strip_prefix("operation_metadata:")
                    .map(str::to_string)
            })
            .collect(),
        include_projection_catalog: true,
        include_function_registry: true,
        include_association_object_types: !planned.dependencies.association_type_ids.is_empty()
            || matches!(planned.resolved.root, ResolvedRoot::Association(_)),
        include_records: true,
        include_evidence_index: matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
            || !planned.dependencies.evidence_fields.is_empty(),
        redaction_read_policy: CoveObjectRedactionReadPolicy::PreserveMarker,
    }
}

pub(crate) fn reconstruction_options(
    planned: &PlannedQuery,
) -> Result<CoveObjectReconstructionOptions, BuildExecutionError> {
    let temporal_cut = match planned.resolved.temporal.mode {
        TemporalMode::Latest => CoveObjectTemporalCut::LatestCommitted,
        TemporalMode::AsOfCsn(csn) => CoveObjectTemporalCut::Csn(csn),
        TemporalMode::AsOfTimestampMicros(timestamp) => {
            if planned.resolved.temporal.role == TemporalRole::AssociationValidTime {
                CoveObjectTemporalCut::LatestCommitted
            } else if matches!(
                planned.resolved.temporal.role,
                TemporalRole::ValidTime
                    | TemporalRole::ObservedTime
                    | TemporalRole::SourceEventTime
            ) {
                CoveObjectTemporalCut::TimestampUs(timestamp)
            } else {
                if planned.resolved.temporal.role != TemporalRole::CommitTime {
                    return Err(exec_error(
                        "E_UNSUPPORTED_TEMPORAL_ROLE",
                        "Phase 3 materialized readback supports commit-time timestamp cuts only",
                        json!({}),
                    ));
                }
                CoveObjectTemporalCut::TimestampUs(timestamp)
            }
        }
        TemporalMode::HistoryRecords
        | TemporalMode::HistoryStates
        | TemporalMode::HistoryRecordsAndStates
        | TemporalMode::ChangesRecords
        | TemporalMode::ChangesStateTransitions
        | TemporalMode::ChangesPropertyDiffs
        | TemporalMode::ChangesFinalObjects => CoveObjectTemporalCut::LatestCommitted,
    };
    let branch_key = match planned.resolved.branch.selector {
        crate::BranchSelector::Default | crate::BranchSelector::RejectAmbiguous => None,
        crate::BranchSelector::BranchKey(branch) => Some(branch),
    };
    Ok(CoveObjectReconstructionOptions {
        temporal_cut,
        branch_key,
        include_tombstones: planned.resolved.tombstone.include_tombstones,
    })
}

pub(crate) fn validate_security_scope(
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Result<(), BuildExecutionError> {
    if let VisibilityPolicy::ExternalOverlay(required_id) = &planned
        .resolved
        .operation_context
        .security
        .visibility_policy
    {
        let Some(overlay) = &options.visibility_overlay else {
            return Err(exec_error(
                "E_VISIBILITY_OVERLAY_UNAVAILABLE",
                "external visibility policy requires a matching execution overlay",
                json!({ "overlay_id": required_id }),
            ));
        };
        if &overlay.overlay_id != required_id {
            return Err(exec_error(
                "E_VISIBILITY_OVERLAY_MISMATCH",
                "external visibility overlay id does not match the resolved security policy",
                json!({ "required_overlay_id": required_id, "provided_overlay_id": overlay.overlay_id }),
            ));
        }
    }
    if zero_copy_arrow_requested(planned)
        && planned.resolved.operation_context.request.fallback_policy
            == FallbackPolicy::RejectOnFallback
    {
        let reason = zero_copy_owned_fallback_reason(planned);
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy Arrow output could not be proven by materialized execution; owned buffers would be required",
            json!({ "fallback_policy": "reject_on_fallback", "reason": reason }),
        ));
    }
    validate_association_evidence_disclosure(planned)?;
    Ok(())
}

pub(crate) fn zero_copy_owned_fallback_warning(
    planned: &PlannedQuery,
) -> Option<ExecutionDiagnostic> {
    if zero_copy_arrow_requested(planned) {
        let reason = zero_copy_owned_fallback_reason(planned);
        Some(exec_warning(
            "W_ZERO_COPY_MATERIALIZED_FALLBACK",
            "zero-copy Arrow output was requested, but CoveQL execution emitted owned materialized buffers",
            json!({ "fallback": "materialized_owned_arrow_buffers", "reason": reason }),
        ))
    } else {
        None
    }
}

pub(super) fn zero_copy_owned_fallback_reason(planned: &PlannedQuery) -> String {
    if !matches!(planned.resolved.root, ResolvedRoot::Object(_)) {
        return "zero-copy retained object-page export supports object roots only".into();
    }
    if planned.resolved.method_chain.where_predicate.is_some() {
        return "filters require materialized residual evaluation before Arrow export".into();
    }
    if planned.resolved.method_chain.order_by.is_some() {
        return "sorting requires materialized row ordering before Arrow export".into();
    }
    if planned.resolved.method_chain.skip.is_some() || planned.resolved.method_chain.take.is_some()
    {
        return "paging requires materialized row selection before Arrow export".into();
    }
    if planned.resolved.method_chain.history.is_some() {
        return "history output is not eligible for retained object-page zero-copy export".into();
    }
    if planned.resolved.method_chain.changes.is_some() {
        return "changes output is not eligible for retained object-page zero-copy export".into();
    }
    if planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .is_none_or(|select| {
            select.is_empty()
                || select
                    .iter()
                    .any(|item| !matches!(item.expr, ResolvedExpr::Path(_)))
        })
    {
        return "zero-copy retained object-page export requires explicit direct property projections"
            .into();
    }
    "borrowed materialized execution cannot prove retained COVE-L page ownership, nullable page polarity, or no-null NumCode page compatibility; use retained physical execution with a validated zero-copy buffer map".into()
}

pub(super) fn zero_copy_arrow_requested(planned: &PlannedQuery) -> bool {
    matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true
        }
    )
}

pub(super) fn validate_association_evidence_disclosure(
    planned: &PlannedQuery,
) -> Result<(), BuildExecutionError> {
    let allow_protected = planned
        .resolved
        .operation_context
        .security
        .metadata_disclosure_policy
        == MetadataDisclosurePolicy::AllowProtected;
    if allow_protected {
        return Ok(());
    }
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        validate_predicate_disclosure(predicate, false)?;
    }
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            validate_expr_disclosure(&item.expr)?;
        }
    }
    Ok(())
}

pub(super) fn validate_predicate_disclosure(
    predicate: &ResolvedPredicate,
    negated: bool,
) -> Result<(), BuildExecutionError> {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::Association(_)) if negated => Err(exec_error(
            "E_PROTECTED_ASSOCIATION_EXISTENCE",
            "association non-existence disclosure requires protected metadata disclosure permission",
            json!({}),
        )),
        ResolvedPredicate::Exists(ResolvedExpr::Evidence(_)) => Err(exec_error(
            "E_PROTECTED_EVIDENCE_EXISTENCE",
            "evidence existence disclosure requires protected metadata disclosure permission",
            json!({}),
        )),
        ResolvedPredicate::Not(inner) => validate_predicate_disclosure(inner, !negated),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                validate_predicate_disclosure(part, negated)?;
            }
            Ok(())
        }
        ResolvedPredicate::Compare { left, right, .. } => {
            validate_expr_disclosure(left)?;
            validate_expr_disclosure(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr) => validate_expr_disclosure(expr),
        _ => Ok(()),
    }
}

pub(super) fn validate_expr_disclosure(expr: &ResolvedExpr) -> Result<(), BuildExecutionError> {
    match expr {
        ResolvedExpr::AggregateCall {
            name,
            arg: Some(arg),
            ..
        } if matches!(
            name,
            AstAggregateName::Count | AstAggregateName::Exists | AstAggregateName::DistinctCount
        ) && matches!(arg.as_ref(), ResolvedExpr::Association(_)) =>
        {
            Err(exec_error(
                "E_PROTECTED_ASSOCIATION_EXISTENCE",
                "association count disclosure requires protected metadata disclosure permission",
                json!({}),
            ))
        }
        ResolvedExpr::AggregateCall {
            name,
            arg: Some(arg),
            ..
        } if matches!(
            name,
            AstAggregateName::Count | AstAggregateName::Exists | AstAggregateName::DistinctCount
        ) && matches!(arg.as_ref(), ResolvedExpr::Evidence(_)) =>
        {
            Err(exec_error(
                "E_PROTECTED_EVIDENCE_EXISTENCE",
                "evidence count disclosure requires protected metadata disclosure permission",
                json!({}),
            ))
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                validate_expr_disclosure(arg)?;
            }
            Ok(())
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                validate_expr_disclosure(arg)?;
            }
            Ok(())
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            validate_predicate_disclosure(predicate, false)?;
            validate_expr_disclosure(then_expr)?;
            validate_expr_disclosure(else_expr)
        }
        _ => Ok(()),
    }
}

pub(crate) fn validate_execution_grain(planned: &PlannedQuery) -> Result<(), BuildExecutionError> {
    validate_execution_output_mode(planned)?;
    if planned.resolved.operation_context.dataset.files.len() > 1
        && !matches!(
            planned.resolved.output_mode,
            CoveQlOutputMode::ExplainJson | CoveQlOutputMode::DataFusionTableProvider
        )
    {
        return Err(exec_error(
            "E_UNSUPPORTED_DATASET_SCOPE",
            "multi-file dataset scopes require manifest-member execution; the single-input CoveQL executor refuses to run a manifest-scoped plan against one file",
            json!({
                "dataset_id": planned.resolved.operation_context.dataset.dataset_id.clone(),
                "manifest_id": planned.resolved.operation_context.dataset.manifest_id.clone(),
                "file_count": planned.resolved.operation_context.dataset.files.len(),
                "file_membership_fingerprint": planned.resolved.operation_context.dataset.file_membership_fingerprint.clone(),
            }),
        ));
    }

    if matches!(
        planned.resolved.root,
        ResolvedRoot::Table(_) | ResolvedRoot::Projection(_) | ResolvedRoot::Evidence(_)
    ) && (planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some())
    {
        return Err(incompatible_execution_grain(
            planned,
            "history and changes output grains require object or association roots",
        ));
    }

    Ok(())
}

pub(crate) fn validate_execution_output_mode(
    planned: &PlannedQuery,
) -> Result<(), BuildExecutionError> {
    let output_valid = matches!(
        (&planned.resolved.root, &planned.resolved.output_mode),
        (_, CoveQlOutputMode::JsonRows)
            | (_, CoveQlOutputMode::ArrowRecordBatch { .. })
            | (_, CoveQlOutputMode::ExplainJson)
            | (ResolvedRoot::Object(_), CoveQlOutputMode::ObjectRows)
            | (
                ResolvedRoot::Association(_),
                CoveQlOutputMode::AssociationRows
            )
            | (ResolvedRoot::Evidence(_), CoveQlOutputMode::EvidenceRows)
            | (
                ResolvedRoot::Projection(_),
                CoveQlOutputMode::ProjectionRows
            )
            | (ResolvedRoot::Table(_), CoveQlOutputMode::ProjectionRows)
            | (_, CoveQlOutputMode::DataFusionTableProvider)
    );
    if !output_valid {
        return Err(incompatible_execution_grain(
            planned,
            "output mode is incompatible with the resolved root kind",
        ));
    }
    Ok(())
}

pub(super) fn incompatible_execution_grain(
    planned: &PlannedQuery,
    reason: &'static str,
) -> BuildExecutionError {
    exec_error(
        "E_EXECUTION_GRAIN",
        reason,
        json!({
            "root": execution_root_kind_name(&planned.resolved.root),
            "output_mode": planned.resolved.output_mode,
            "scan_grain": planned.logical_plan.context.scan_grain,
            "temporal_mode": planned.resolved.temporal.mode,
        }),
    )
}

pub(super) fn execution_root_kind_name(root: &ResolvedRoot) -> &'static str {
    match root {
        ResolvedRoot::Object(_) => "object",
        ResolvedRoot::Association(_) => "association",
        ResolvedRoot::Node(_) => "node",
        ResolvedRoot::Edge(_) => "edge",
        ResolvedRoot::Table(_) => "table",
        ResolvedRoot::Projection(_) => "projection",
        ResolvedRoot::Evidence(_) => "evidence",
    }
}

pub(crate) fn enforce_result_budgets(
    result: &CoveQlExecutionResult,
    row_counts: &ExecutionRowCounts,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<(), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    if planned.resolved.method_chain.take.is_none()
        && row_counts.output_rows > options.resource_budget.maximum_rows_without_explicit_take
    {
        return Err(resource_error(
            "maximum_rows_without_explicit_take",
            row_counts.output_rows,
        ));
    }
    let decode_bytes = result_decode_size(result)?;
    if decode_bytes > options.resource_budget.maximum_decode_bytes {
        return Err(resource_error("maximum_decode_bytes", decode_bytes));
    }
    let columns = max_output_columns(result)?;
    if columns > options.resource_budget.maximum_output_columns {
        return Err(resource_error("maximum_output_columns", columns));
    }
    Ok(())
}

pub(crate) fn check_time(
    budget: &ResourceBudgetPolicy,
    started: Instant,
) -> Result<(), BuildExecutionError> {
    if started.elapsed().as_millis() as u64 > budget.maximum_execution_time_ms {
        return Err(resource_error(
            "maximum_execution_time_ms",
            started.elapsed().as_millis() as usize,
        ));
    }
    Ok(())
}

pub(crate) fn result_fingerprint(
    result: &CoveQlExecutionResult,
) -> Result<String, BuildExecutionError> {
    if let CoveQlExecutionResult::ArrowRecordBatches(batches) = result {
        return Ok(arrow_record_batches_fingerprint(batches));
    }
    let value = result_json(result)?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|err| exec_error("E_OUTPUT", err.to_string(), json!({})))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(super) fn result_decode_size(
    result: &CoveQlExecutionResult,
) -> Result<usize, BuildExecutionError> {
    if let CoveQlExecutionResult::ArrowRecordBatches(batches) = result {
        return Ok(arrow_record_batches_memory_size(batches));
    }
    serde_json::to_vec(&result_json(result)?)
        .map(|bytes| bytes.len())
        .map_err(|err| exec_error("E_OUTPUT", err.to_string(), json!({})))
}

pub(super) fn arrow_record_batches_memory_size(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::get_array_memory_size).sum()
}

pub(super) fn arrow_record_batches_fingerprint(batches: &[RecordBatch]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"coveql-arrow-record-batches-v1");
    hash_usize(&mut hasher, batches.len());
    for batch in batches {
        hash_usize(&mut hasher, batch.num_rows());
        hash_usize(&mut hasher, batch.num_columns());
        hash_arrow_schema(&mut hasher, batch.schema().as_ref());
        for column in batch.columns() {
            hash_arrow_array_data(&mut hasher, &column.to_data());
        }
    }
    format!("{:x}", hasher.finalize())
}

pub(super) fn hash_arrow_schema(hasher: &mut Sha256, schema: &arrow_schema::Schema) {
    hash_usize(hasher, schema.fields().len());
    for field in schema.fields() {
        hash_str(hasher, field.name());
        hash_str(hasher, &format!("{:?}", field.data_type()));
        hasher.update([u8::from(field.is_nullable())]);
        let mut metadata = field.metadata().iter().collect::<Vec<_>>();
        metadata.sort_by(|left, right| left.0.cmp(right.0));
        hash_usize(hasher, metadata.len());
        for (key, value) in metadata {
            hash_str(hasher, key);
            hash_str(hasher, value);
        }
    }
}

pub(super) fn hash_arrow_array_data(hasher: &mut Sha256, data: &ArrayData) {
    hash_str(hasher, &format!("{:?}", data.data_type()));
    hash_usize(hasher, data.len());
    hash_usize(hasher, data.offset());
    if let Some(nulls) = data.nulls() {
        hasher.update([1]);
        hash_usize(hasher, nulls.len());
        hash_usize(hasher, nulls.offset());
        hash_usize(hasher, nulls.null_count());
        hash_usize(hasher, nulls.validity().len());
        hasher.update(nulls.validity());
    } else {
        hasher.update([0]);
    }
    hash_usize(hasher, data.buffers().len());
    for buffer in data.buffers() {
        hash_usize(hasher, buffer.len());
        hasher.update(buffer.as_slice());
    }
    hash_usize(hasher, data.child_data().len());
    for child in data.child_data() {
        hash_arrow_array_data(hasher, child);
    }
}

pub(super) fn hash_str(hasher: &mut Sha256, value: &str) {
    hash_usize(hasher, value.len());
    hasher.update(value.as_bytes());
}

pub(super) fn hash_usize(hasher: &mut Sha256, value: usize) {
    hasher.update((value as u64).to_le_bytes());
}

pub(crate) fn result_json(result: &CoveQlExecutionResult) -> Result<Value, BuildExecutionError> {
    Ok(match result {
        CoveQlExecutionResult::ObjectRows(rows) => {
            Value::Array(rows.iter().map(MaterializedObjectRow::to_json).collect())
        }
        CoveQlExecutionResult::AssociationRows(rows) => Value::Array(
            rows.iter()
                .map(MaterializedAssociationRow::to_json)
                .collect(),
        ),
        CoveQlExecutionResult::EvidenceRows(rows) => {
            Value::Array(rows.iter().map(MaterializedEvidenceRow::to_json).collect())
        }
        CoveQlExecutionResult::ProjectionRows(rows) => Value::Array(
            rows.iter()
                .map(MaterializedProjectionRow::to_json)
                .collect(),
        ),
        CoveQlExecutionResult::ArrowRecordBatches(batches) => Value::Array(
            record_batches_to_json_rows(batches)
                .map_err(|err| exec_error("E_ARROW_OUTPUT", err.to_string(), json!({})))?,
        ),
        CoveQlExecutionResult::JsonRows(rows) => Value::Array(rows.clone()),
        CoveQlExecutionResult::ExplainJson(value) => value.clone(),
    })
}

pub(super) fn max_output_columns(
    result: &CoveQlExecutionResult,
) -> Result<usize, BuildExecutionError> {
    let rows = match result {
        CoveQlExecutionResult::ArrowRecordBatches(batches) => {
            return Ok(batches
                .iter()
                .map(|batch| batch.num_columns())
                .max()
                .unwrap_or_default())
        }
        _ => result_json(result)?,
    };
    Ok(rows
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_object().map(|object| object.len()).unwrap_or(1))
                .max()
                .unwrap_or_default()
        })
        .unwrap_or_default())
}

pub(super) fn output_name_for_expr(expr: &ResolvedExpr) -> String {
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::FunctionCall { function_id, .. } => function_id.clone(),
        ResolvedExpr::AggregateCall { name, .. } => aggregate_name(*name).into(),
        ResolvedExpr::Literal(_) => "literal".into(),
        ResolvedExpr::Association(association) => association.type_name.clone(),
        ResolvedExpr::Evidence(_) => "evidence".into(),
        ResolvedExpr::TableExists(_) => "exists".into(),
        ResolvedExpr::Conditional { .. } => "if".into(),
    }
}

pub(super) fn aggregate_name(name: AstAggregateName) -> &'static str {
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

pub(super) fn grouped_or_aggregate(planned: &PlannedQuery) -> bool {
    planned.resolved.method_chain.group_by.is_some()
        || planned
            .resolved
            .method_chain
            .select
            .as_ref()
            .is_some_and(|select| select.iter().any(|item| contains_aggregate(&item.expr)))
}

pub(super) fn contains_aggregate(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::AggregateCall { .. } => true,
        ResolvedExpr::FunctionCall { args, .. } => args.iter().any(contains_aggregate),
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_aggregate(predicate)
                || contains_aggregate(then_expr)
                || contains_aggregate(else_expr)
        }
        _ => false,
    }
}

pub(super) fn predicate_contains_aggregate(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            contains_aggregate(left) || contains_aggregate(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => contains_aggregate(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_aggregate(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_aggregate)
        }
    }
}

pub(crate) fn resource_error(limit: &str, actual: usize) -> BuildExecutionError {
    exec_error(
        "E_RESOURCE_BUDGET_EXCEEDED",
        format!("{limit} budget exceeded during execution"),
        json!({ "limit": limit, "actual": actual }),
    )
}

pub(crate) fn exec_error(
    code: &str,
    message: impl Into<String>,
    safe_details: Value,
) -> BuildExecutionError {
    BuildExecutionError::single(ExecutionDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: "execute".into(),
        safe_details,
        redacted: false,
    })
}

pub(crate) fn exec_warning(
    code: &str,
    message: impl Into<String>,
    safe_details: Value,
) -> ExecutionDiagnostic {
    ExecutionDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Warning,
        message: message.into(),
        phase: "execute".into(),
        safe_details,
        redacted: false,
    }
}
