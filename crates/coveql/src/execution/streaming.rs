use super::*;

pub struct CoveQlResultStream {
    bytes: Vec<u8>,
    planned: PlannedQuery,
    options: ExecutionOptions,
    executed: Option<ExecutedQuery>,
    batches: Vec<CoveQlExecutionResult>,
    next_batch: usize,
    row_stream: Option<MaterializedRowStreamState>,
    blocking_reason: Option<String>,
    cancelled: bool,
}

impl CoveQlResultStream {
    pub fn executed(&self) -> Option<&ExecutedQuery> {
        self.executed.as_ref()
    }

    pub fn is_blocking(&self) -> bool {
        self.blocking_reason.is_some()
    }

    pub fn blocking_reason(&self) -> Option<&str> {
        self.blocking_reason.as_deref()
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.batches.clear();
    }

    pub fn next_batch(&mut self) -> Result<Option<CoveQlExecutionResult>, BuildExecutionError> {
        if self.blocking_reason.is_none() {
            return self.next_streaming_batch();
        }
        self.ensure_executed()?;
        let batch = self.batches.get(self.next_batch).cloned();
        if batch.is_some() {
            self.next_batch += 1;
        }
        Ok(batch)
    }

    pub fn finish(mut self) -> Result<ExecutedQuery, BuildExecutionError> {
        if self.blocking_reason.is_none() {
            self.finish_streaming()
        } else {
            self.ensure_executed()?;
            self.executed
                .take()
                .ok_or_else(|| exec_error("E_STREAM_CANCELLED", "stream was cancelled", json!({})))
        }
    }

    fn next_streaming_batch(
        &mut self,
    ) -> Result<Option<CoveQlExecutionResult>, BuildExecutionError> {
        if self.cancelled {
            return Err(exec_error(
                "E_STREAM_CANCELLED",
                "stream was cancelled before completion",
                json!({}),
            ));
        }
        if self.executed.is_some() {
            return Ok(None);
        }
        if self.row_stream.is_none() {
            self.row_stream = Some(MaterializedRowStreamState::new(
                &self.bytes,
                self.planned.clone(),
                self.options.clone(),
            )?);
        }
        let batch = self
            .row_stream
            .as_mut()
            .ok_or_else(|| exec_error("E_STREAM", "stream state was not initialized", json!({})))?
            .next_batch()?;
        Ok(batch)
    }

    fn finish_streaming(&mut self) -> Result<ExecutedQuery, BuildExecutionError> {
        if self.cancelled {
            return Err(exec_error(
                "E_STREAM_CANCELLED",
                "stream was cancelled before completion",
                json!({}),
            ));
        }
        if let Some(executed) = self.executed.take() {
            return Ok(executed);
        }
        if self.row_stream.is_none() {
            self.row_stream = Some(MaterializedRowStreamState::new(
                &self.bytes,
                self.planned.clone(),
                self.options.clone(),
            )?);
        }
        let state = self
            .row_stream
            .as_mut()
            .ok_or_else(|| exec_error("E_STREAM", "stream state was not initialized", json!({})))?;
        while state.next_batch()?.is_some() {}
        let executed = state.finish()?;
        self.executed = Some(executed.clone());
        Ok(executed)
    }

    fn ensure_executed(&mut self) -> Result<(), BuildExecutionError> {
        if self.cancelled {
            return Err(exec_error(
                "E_STREAM_CANCELLED",
                "stream was cancelled before completion",
                json!({}),
            ));
        }
        if self.executed.is_some() {
            return Ok(());
        }
        let mut executed =
            execute_planned_query(&self.bytes, self.planned.clone(), self.options.clone())?;
        if let Some(reason) = &self.blocking_reason {
            executed.diagnostics.push(exec_warning(
                "W_STREAM_BLOCKING_PLAN",
                format!("streaming plan is blocking: {reason}"),
                json!({ "blocking_reason": reason }),
            ));
        }
        let batch_size = if self.blocking_reason.is_some() {
            usize::MAX
        } else {
            self.options.batch_size.unwrap_or(usize::MAX).max(1)
        };
        self.batches = result_batches(&executed.result, batch_size);
        self.executed = Some(executed);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct MaterializedRowStreamState {
    planned: PlannedQuery,
    options: ExecutionOptions,
    started: Instant,
    source: MaterializedRowSource,
    next_input: usize,
    input_rows: usize,
    filtered_rows: usize,
    output_rows: usize,
    json_output: Vec<Value>,
    object_output: Vec<MaterializedObjectRow>,
    association_output: Vec<MaterializedAssociationRow>,
    evidence_output: Vec<MaterializedEvidenceRow>,
    finished: bool,
}

impl MaterializedRowStreamState {
    fn new(
        bytes: &[u8],
        planned: PlannedQuery,
        options: ExecutionOptions,
    ) -> Result<Self, BuildExecutionError> {
        let started = Instant::now();
        validate_security_scope(&planned, &options)?;
        validate_execution_grain(&planned)?;
        let mut source = object_backed_row_source(bytes, &planned, &options, started)?;
        source.object_rows = filter_object_context_rows(&source.object_rows, &planned, &options);
        let context = EvalContext::for_plan_with_objects(
            &source.associations,
            &source.evidence_rows,
            &source.object_rows,
            &planned,
        );
        sort_rows(&mut source.rows, &planned, &context)?;
        Ok(Self {
            planned,
            options,
            started,
            source,
            next_input: 0,
            input_rows: 0,
            filtered_rows: 0,
            output_rows: 0,
            json_output: Vec::new(),
            object_output: Vec::new(),
            association_output: Vec::new(),
            evidence_output: Vec::new(),
            finished: false,
        })
    }

    fn next_batch(&mut self) -> Result<Option<CoveQlExecutionResult>, BuildExecutionError> {
        if self.finished {
            return Ok(None);
        }
        let batch_size = self.options.batch_size.unwrap_or(usize::MAX).max(1);
        let take = self
            .planned
            .resolved
            .method_chain
            .take
            .and_then(|take| usize::try_from(take).ok());
        let mut json_batch = Vec::new();
        let mut object_batch = Vec::new();
        let mut association_batch = Vec::new();
        let mut evidence_batch = Vec::new();
        while self.next_input < self.source.rows.len()
            && batch_len(
                &json_batch,
                &object_batch,
                &association_batch,
                &evidence_batch,
            ) < batch_size
        {
            if take.is_some_and(|take| self.output_rows >= take) {
                self.finished = true;
                break;
            }
            let row = self.source.rows[self.next_input].clone();
            self.next_input += 1;
            if !stream_row_visible(&row, &self.planned, &self.options) {
                continue;
            }
            self.input_rows += 1;
            let context = EvalContext::for_plan_with_objects(
                &self.source.associations,
                &self.source.evidence_rows,
                &self.source.object_rows,
                &self.planned,
            );
            if !predicate_matches(&row, &self.planned, &context)? {
                continue;
            }
            self.filtered_rows += 1;
            let emitted = match (&self.planned.resolved.output_mode, row) {
                (CoveQlOutputMode::JsonRows, row) => {
                    let value =
                        select_json_rows(std::slice::from_ref(&row), &self.planned, &context)?
                            .into_iter()
                            .next()
                            .unwrap_or(Value::Null);
                    json_batch.push(value.clone());
                    self.json_output.push(value);
                    true
                }
                (CoveQlOutputMode::ObjectRows, ExecutionRow::Object(row)) => {
                    object_batch.push(row.clone());
                    self.object_output.push(row);
                    true
                }
                (CoveQlOutputMode::AssociationRows, ExecutionRow::Association(row)) => {
                    association_batch.push(row.clone());
                    self.association_output.push(row);
                    true
                }
                (CoveQlOutputMode::EvidenceRows, ExecutionRow::Evidence(row)) => {
                    evidence_batch.push(row.clone());
                    self.evidence_output.push(row);
                    true
                }
                _ => false,
            };
            if emitted {
                self.output_rows += 1;
            }
            check_time(&self.options.resource_budget, self.started)?;
        }
        if self.next_input >= self.source.rows.len() {
            self.finished = true;
        }
        if batch_len(
            &json_batch,
            &object_batch,
            &association_batch,
            &evidence_batch,
        ) == 0
        {
            return Ok(None);
        }
        Ok(Some(match &self.planned.resolved.output_mode {
            CoveQlOutputMode::JsonRows => CoveQlExecutionResult::JsonRows(json_batch),
            CoveQlOutputMode::ObjectRows => CoveQlExecutionResult::ObjectRows(object_batch),
            CoveQlOutputMode::AssociationRows => {
                CoveQlExecutionResult::AssociationRows(association_batch)
            }
            CoveQlOutputMode::EvidenceRows => CoveQlExecutionResult::EvidenceRows(evidence_batch),
            _ => unreachable!("non-streamable output modes are blocked before streaming"),
        }))
    }

    fn finish(&mut self) -> Result<ExecutedQuery, BuildExecutionError> {
        while self.next_batch()?.is_some() {}
        let result = match &self.planned.resolved.output_mode {
            CoveQlOutputMode::JsonRows => CoveQlExecutionResult::JsonRows(self.json_output.clone()),
            CoveQlOutputMode::ObjectRows => {
                CoveQlExecutionResult::ObjectRows(self.object_output.clone())
            }
            CoveQlOutputMode::AssociationRows => {
                CoveQlExecutionResult::AssociationRows(self.association_output.clone())
            }
            CoveQlOutputMode::EvidenceRows => {
                CoveQlExecutionResult::EvidenceRows(self.evidence_output.clone())
            }
            _ => unreachable!("non-streamable output modes are blocked before streaming"),
        };
        let row_counts = ExecutionRowCounts {
            input_rows: self.input_rows,
            filtered_rows: self.filtered_rows,
            output_rows: self.output_rows,
        };
        enforce_result_budgets(
            &result,
            &row_counts,
            &self.planned,
            &self.options,
            self.started,
        )?;
        let output_fingerprint = result_fingerprint(&result)?;
        let mut diagnostics = self
            .planned
            .diagnostics
            .iter()
            .cloned()
            .map(ExecutionDiagnostic::from)
            .collect::<Vec<_>>();
        diagnostics.push(exec_warning(
            "W_STREAM_BATCHED_EXECUTION",
            "streamable plan executed through materialized row-source batches; final summary was deferred until finish",
            json!({ "batch_size": self.options.batch_size }),
        ));
        self.source
            .pushdown_report
            .counters
            .rows_after_candidate_retain = row_counts.input_rows;
        Ok(ExecutedQuery {
            planned: self.planned.clone(),
            result,
            diagnostics,
            row_counts,
            output_fingerprint,
            pushdown_report: self.source.pushdown_report.clone(),
            evidence_authority: self.source.evidence_authority,
            authority: ExecutionAuthorityReport::materialized_baseline(
                "streamed materialized row-source execution produced the visible output",
            ),
        })
    }
}

pub(super) fn batch_len(
    json_batch: &[Value],
    object_batch: &[MaterializedObjectRow],
    association_batch: &[MaterializedAssociationRow],
    evidence_batch: &[MaterializedEvidenceRow],
) -> usize {
    json_batch.len() + object_batch.len() + association_batch.len() + evidence_batch.len()
}

pub(super) fn stream_row_visible(
    row: &ExecutionRow,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> bool {
    if !matches!(
        planned
            .resolved
            .operation_context
            .security
            .visibility_policy,
        VisibilityPolicy::ExternalOverlay(_)
    ) {
        return true;
    }
    options
        .visibility_overlay
        .as_ref()
        .is_some_and(|overlay| row_visible_in_overlay(row, overlay))
}

impl Iterator for CoveQlResultStream {
    type Item = Result<CoveQlExecutionResult, BuildExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_batch().transpose()
    }
}

pub fn execute_planned_query_stream(
    bytes: &[u8],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<CoveQlResultStream, BuildExecutionError> {
    Ok(CoveQlResultStream {
        bytes: bytes.to_vec(),
        blocking_reason: stream_blocking_reason(&planned),
        planned,
        options,
        executed: None,
        batches: Vec::new(),
        next_batch: 0,
        row_stream: None,
        cancelled: false,
    })
}

pub(super) fn stream_blocking_reason(planned: &PlannedQuery) -> Option<String> {
    if matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch { .. }
            | CoveQlOutputMode::ExplainJson
            | CoveQlOutputMode::DataFusionTableProvider
            | CoveQlOutputMode::ProjectionRows
    ) {
        return Some("output mode requires whole-result materialization".into());
    }
    if grouped_or_aggregate(planned) {
        return Some("aggregate execution requires complete input".into());
    }
    if planned.resolved.method_chain.order_by.is_some() {
        return Some("explicit orderBy requires full materialized sort".into());
    }
    if planned.resolved.method_chain.skip.is_some() {
        return Some("skip requires a stable global prefix".into());
    }
    if matches!(planned.resolved.root, ResolvedRoot::Projection(_)) {
        return Some("projection readback is materialized before streaming".into());
    }
    None
}

pub(super) fn result_batches(
    result: &CoveQlExecutionResult,
    batch_size: usize,
) -> Vec<CoveQlExecutionResult> {
    match result {
        CoveQlExecutionResult::ObjectRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::ObjectRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::AssociationRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::AssociationRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::EvidenceRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::EvidenceRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::ProjectionRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::ProjectionRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::ArrowRecordBatches(batches) => batches
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::ArrowRecordBatches(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::JsonRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::JsonRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::ExplainJson(value) => {
            vec![CoveQlExecutionResult::ExplainJson(value.clone())]
        }
    }
}
