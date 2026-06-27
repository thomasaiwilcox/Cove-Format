//! DataFusion 53.x native key-only equi-join execution.

use std::{any::Any, collections::HashMap, sync::Arc};

use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, SchemaRef};
use async_trait::async_trait;
use cove_core::native::{
    anti_join_i64_eq_left_nulls_unmatched, inner_join_i64_eq, semi_join_i64_eq, NativeCodeDomain,
    SelectionBitmap,
};
use cove_core::native::{
    anti_join_u32_eq_left_nulls_unmatched, inner_join_u32_eq, semi_join_u32_eq,
};
use datafusion::{
    catalog::{Session, TableProvider},
    common::{DataFusionError, Result, Statistics},
    execution::{SendableRecordBatchStream, TaskContext},
    logical_expr::TableType,
    physical_expr::EquivalenceProperties,
    physical_plan::{
        execution_plan::{Boundedness, EmissionType},
        memory::MemoryStream,
        metrics::{ExecutionPlanMetricsSet, MetricsSet},
        DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
    },
};

use crate::{
    adapter_v53::metrics::CoveFileMetrics,
    dataset_state::DatasetState,
    decode::{
        native_filecode_values_scan, native_i64_values_scan, DecodeStats, NativeFileCodeDenseLane,
        NativeI64DenseLane,
    },
    metadata_aggregate::canonical_utf8,
    planner::ScanPlan,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeI64JoinKind {
    Inner,
    LeftSemi,
    LeftAnti,
}

impl NativeI64JoinKind {
    fn label(self) -> &'static str {
        match self {
            Self::Inner => "inner",
            Self::LeftSemi => "left_semi",
            Self::LeftAnti => "left_anti",
        }
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeI64JoinTableProvider {
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    left_state: Arc<DatasetState>,
    right_state: Arc<DatasetState>,
    left_column_index: usize,
    right_column_index: usize,
    left_filter_plan: ScanPlan,
    right_filter_plan: ScanPlan,
}

impl CoveNativeI64JoinTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        join_kind: NativeI64JoinKind,
        left_state: Arc<DatasetState>,
        right_state: Arc<DatasetState>,
        left_column_index: usize,
        right_column_index: usize,
        left_filter_plan: ScanPlan,
        right_filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            join_kind,
            left_state,
            right_state,
            left_column_index,
            right_column_index,
            left_filter_plan,
            right_filter_plan,
        }
    }
}

#[async_trait]
impl TableProvider for CoveNativeI64JoinTableProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Temporary
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        _projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        _limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !filters.is_empty() {
            return Err(DataFusionError::Internal(
                "COVE native i64 join provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeI64JoinExec::try_new(
            Arc::clone(&self.schema),
            self.join_kind,
            Arc::clone(&self.left_state),
            Arc::clone(&self.right_state),
            self.left_column_index,
            self.right_column_index,
            self.left_filter_plan.clone(),
            self.right_filter_plan.clone(),
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeFileCodeJoinTableProvider {
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    left_state: Arc<DatasetState>,
    right_state: Arc<DatasetState>,
    left_column_index: usize,
    right_column_index: usize,
    left_filter_plan: ScanPlan,
    right_filter_plan: ScanPlan,
}

impl CoveNativeFileCodeJoinTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        join_kind: NativeI64JoinKind,
        left_state: Arc<DatasetState>,
        right_state: Arc<DatasetState>,
        left_column_index: usize,
        right_column_index: usize,
        left_filter_plan: ScanPlan,
        right_filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            join_kind,
            left_state,
            right_state,
            left_column_index,
            right_column_index,
            left_filter_plan,
            right_filter_plan,
        }
    }
}

#[async_trait]
impl TableProvider for CoveNativeFileCodeJoinTableProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Temporary
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        _projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        _limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !filters.is_empty() {
            return Err(DataFusionError::Internal(
                "COVE native FileCode join provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeFileCodeJoinExec::try_new(
            Arc::clone(&self.schema),
            self.join_kind,
            Arc::clone(&self.left_state),
            Arc::clone(&self.right_state),
            self.left_column_index,
            self.right_column_index,
            self.left_filter_plan.clone(),
            self.right_filter_plan.clone(),
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeI64JoinExec {
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    left_state: Arc<DatasetState>,
    right_state: Arc<DatasetState>,
    left_column_index: usize,
    right_column_index: usize,
    left_filter_plan: ScanPlan,
    right_filter_plan: ScanPlan,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeI64JoinExec {
    fn try_new(
        schema: SchemaRef,
        join_kind: NativeI64JoinKind,
        left_state: Arc<DatasetState>,
        right_state: Arc<DatasetState>,
        left_column_index: usize,
        right_column_index: usize,
        left_filter_plan: ScanPlan,
        right_filter_plan: ScanPlan,
    ) -> Result<Self> {
        match join_kind {
            NativeI64JoinKind::Inner => {
                if schema.fields().len() != 2
                    || schema.field(0).data_type() != &DataType::Int64
                    || schema.field(1).data_type() != &DataType::Int64
                {
                    return Err(DataFusionError::Plan(
                        "native i64 inner join expects two Int64 output columns".into(),
                    ));
                }
            }
            NativeI64JoinKind::LeftSemi | NativeI64JoinKind::LeftAnti => {
                if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
                    return Err(DataFusionError::Plan(
                        "native i64 semi/anti join expects one Int64 output column".into(),
                    ));
                }
            }
        }
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Final,
            Boundedness::Bounded,
        ));
        Ok(Self {
            schema,
            join_kind,
            left_state,
            right_state,
            left_column_index,
            right_column_index,
            left_filter_plan,
            right_filter_plan,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeI64JoinExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => write!(
                f,
                "{}: kind={}, representation=typed_numeric_i64, semantic_domain=cove.datafusion.native.i64, kernel=shared_cove_core, null_policy=validity-bitmap-nulls-never-match, decode_boundary=none, fallback=none, left_column={}, right_column={}, left_files={}, right_files={}, left_filters={}, right_filters={}",
                self.name(),
                self.join_kind.label(),
                self.left_column_index,
                self.right_column_index,
                self.left_state.file_count(),
                self.right_state.file_count(),
                self.left_filter_plan.filters.len(),
                self.right_filter_plan.filters.len()
            ),
            DisplayFormatType::TreeRender => write!(f, "{}", self.name()),
        }
    }
}

impl ExecutionPlan for CoveNativeI64JoinExec {
    fn name(&self) -> &str {
        "CoveNativeI64JoinExec"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        Vec::new()
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !children.is_empty() {
            return Err(DataFusionError::Internal(
                "CoveNativeI64JoinExec is a leaf execution plan".into(),
            ));
        }
        Ok(self)
    }

    fn execute(
        &self,
        partition: usize,
        _context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(DataFusionError::Internal(format!(
                "CoveNativeI64JoinExec has one partition, got partition {partition}"
            )));
        }
        let left = native_i64_values_scan(
            &self.left_state,
            self.left_column_index,
            &self.left_filter_plan,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        let right = native_i64_values_scan(
            &self.right_state,
            self.right_column_index,
            &self.right_filter_plan,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        let mut stats = left.stats;
        stats.add_decode(right.stats);
        let batch = native_i64_join_batch(
            Arc::clone(&self.schema),
            self.join_kind,
            &left.lane,
            &right.lane,
            &mut stats,
        )?;
        CoveFileMetrics::new(&self.metrics, partition).record_decode(stats);
        Ok(Box::pin(MemoryStream::try_new(
            vec![batch],
            Arc::clone(&self.schema),
            None,
        )?))
    }

    fn metrics(&self) -> Option<MetricsSet> {
        Some(self.metrics.clone_inner())
    }

    fn partition_statistics(&self, partition: Option<usize>) -> Result<Statistics> {
        if let Some(partition) = partition {
            if partition != 0 {
                return Err(DataFusionError::Internal(format!(
                    "CoveNativeI64JoinExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeFileCodeJoinExec {
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    left_state: Arc<DatasetState>,
    right_state: Arc<DatasetState>,
    left_column_index: usize,
    right_column_index: usize,
    left_filter_plan: ScanPlan,
    right_filter_plan: ScanPlan,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeFileCodeJoinExec {
    fn try_new(
        schema: SchemaRef,
        join_kind: NativeI64JoinKind,
        left_state: Arc<DatasetState>,
        right_state: Arc<DatasetState>,
        left_column_index: usize,
        right_column_index: usize,
        left_filter_plan: ScanPlan,
        right_filter_plan: ScanPlan,
    ) -> Result<Self> {
        match join_kind {
            NativeI64JoinKind::Inner => {
                if schema.fields().len() != 2
                    || schema.field(0).data_type() != &DataType::Utf8
                    || schema.field(1).data_type() != &DataType::Utf8
                {
                    return Err(DataFusionError::Plan(
                        "native FileCode inner join expects two Utf8 output columns".into(),
                    ));
                }
            }
            NativeI64JoinKind::LeftSemi | NativeI64JoinKind::LeftAnti => {
                if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Utf8 {
                    return Err(DataFusionError::Plan(
                        "native FileCode semi/anti join expects one Utf8 output column".into(),
                    ));
                }
            }
        }
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Final,
            Boundedness::Bounded,
        ));
        Ok(Self {
            schema,
            join_kind,
            left_state,
            right_state,
            left_column_index,
            right_column_index,
            left_filter_plan,
            right_filter_plan,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeFileCodeJoinExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => write!(
                f,
                "{}: kind={}, representation=filecode_utf8_execution_code_u32, semantic_domain=file-local-dictionary-to-canonical-utf8, kernel=shared_cove_core, null_policy=validity-bitmap-nulls-never-match, decode_boundary=join-key-canonicalization, fallback=page-decode-boundary, left_column={}, right_column={}, left_files={}, right_files={}, left_filters={}, right_filters={}",
                self.name(),
                self.join_kind.label(),
                self.left_column_index,
                self.right_column_index,
                self.left_state.file_count(),
                self.right_state.file_count(),
                self.left_filter_plan.filters.len(),
                self.right_filter_plan.filters.len()
            ),
            DisplayFormatType::TreeRender => write!(f, "{}", self.name()),
        }
    }
}

impl ExecutionPlan for CoveNativeFileCodeJoinExec {
    fn name(&self) -> &str {
        "CoveNativeFileCodeJoinExec"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        Vec::new()
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !children.is_empty() {
            return Err(DataFusionError::Internal(
                "CoveNativeFileCodeJoinExec is a leaf execution plan".into(),
            ));
        }
        Ok(self)
    }

    fn execute(
        &self,
        partition: usize,
        _context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(DataFusionError::Internal(format!(
                "CoveNativeFileCodeJoinExec has one partition, got partition {partition}"
            )));
        }
        let left = native_filecode_values_scan(
            &self.left_state,
            self.left_column_index,
            &self.left_filter_plan,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        let right = native_filecode_values_scan(
            &self.right_state,
            self.right_column_index,
            &self.right_filter_plan,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        let mut stats = left.stats;
        stats.add_decode(right.stats);
        let batch = native_filecode_join_batch(
            Arc::clone(&self.schema),
            self.join_kind,
            &left.lane,
            &right.lane,
            &mut stats,
        )?;
        CoveFileMetrics::new(&self.metrics, partition).record_decode(stats);
        Ok(Box::pin(MemoryStream::try_new(
            vec![batch],
            Arc::clone(&self.schema),
            None,
        )?))
    }

    fn metrics(&self) -> Option<MetricsSet> {
        Some(self.metrics.clone_inner())
    }

    fn partition_statistics(&self, partition: Option<usize>) -> Result<Statistics> {
        if let Some(partition) = partition {
            if partition != 0 {
                return Err(DataFusionError::Internal(format!(
                    "CoveNativeFileCodeJoinExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(Statistics::new_unknown(self.schema.as_ref()))
    }
}

fn native_i64_join_batch(
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    left_lane: &NativeI64DenseLane,
    right_lane: &NativeI64DenseLane,
    stats: &mut DecodeStats,
) -> Result<RecordBatch> {
    let domain = native_i64_join_domain();

    match join_kind {
        NativeI64JoinKind::Inner => {
            let (pairs, kernel_stats) = inner_join_i64_eq(
                left_lane.values(),
                left_lane.validity(),
                &domain,
                right_lane.values(),
                right_lane.validity(),
                &domain,
                None,
            )
            .map_err(crate::adapter_v53::cove_to_datafusion)?;
            stats.record_native_join_kernel(kernel_stats);

            let mut left_out = Vec::with_capacity(pairs.left_rows.len());
            let mut right_out = Vec::with_capacity(pairs.right_rows.len());
            for (&left_row, &right_row) in pairs.left_rows.iter().zip(&pairs.right_rows) {
                let left_row = usize::try_from(left_row).map_err(|_| {
                    DataFusionError::Execution("native join left row id overflowed usize".into())
                })?;
                let right_row = usize::try_from(right_row).map_err(|_| {
                    DataFusionError::Execution("native join right row id overflowed usize".into())
                })?;
                let Some(left_value) = left_lane
                    .value_at(left_row)
                    .map_err(crate::adapter_v53::cove_to_datafusion)?
                else {
                    return Err(DataFusionError::Execution(
                        "native inner join emitted an invalid left row id".into(),
                    ));
                };
                let Some(right_value) = right_lane
                    .value_at(right_row)
                    .map_err(crate::adapter_v53::cove_to_datafusion)?
                else {
                    return Err(DataFusionError::Execution(
                        "native inner join emitted an invalid right row id".into(),
                    ));
                };
                left_out.push(Some(left_value));
                right_out.push(Some(right_value));
            }
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Int64Array::from(left_out)) as ArrayRef,
                    Arc::new(Int64Array::from(right_out)) as ArrayRef,
                ],
            )
        }
        NativeI64JoinKind::LeftSemi => {
            let (selected, kernel_stats) = semi_join_i64_eq(
                left_lane.values(),
                left_lane.validity(),
                &domain,
                right_lane.values(),
                right_lane.validity(),
                &domain,
                None,
            )
            .map_err(crate::adapter_v53::cove_to_datafusion)?;
            stats.record_native_join_kernel(kernel_stats);
            let out = selected_i64_values(left_lane, &selected)?;
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(out)) as ArrayRef])
        }
        NativeI64JoinKind::LeftAnti => {
            let (selected, kernel_stats) = anti_join_i64_eq_left_nulls_unmatched(
                left_lane.values(),
                left_lane.validity(),
                &domain,
                right_lane.values(),
                right_lane.validity(),
                &domain,
                None,
            )
            .map_err(crate::adapter_v53::cove_to_datafusion)?;
            stats.record_native_join_kernel(kernel_stats);
            let out = selected_i64_values(left_lane, &selected)?;
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(out)) as ArrayRef])
        }
    }
    .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_filecode_join_batch(
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    left_lane: &NativeFileCodeDenseLane,
    right_lane: &NativeFileCodeDenseLane,
    stats: &mut DecodeStats,
) -> Result<RecordBatch> {
    let (left_codes, right_codes) = remap_filecode_join_codes(left_lane, right_lane)?;
    let domain = native_filecode_join_domain();

    match join_kind {
        NativeI64JoinKind::Inner => {
            let (pairs, kernel_stats) = inner_join_u32_eq(
                &left_codes,
                left_lane.validity(),
                &domain,
                &right_codes,
                right_lane.validity(),
                &domain,
                None,
            )
            .map_err(crate::adapter_v53::cove_to_datafusion)?;
            stats.record_native_join_kernel(kernel_stats);

            let mut left_out = Vec::with_capacity(pairs.left_rows.len());
            let mut right_out = Vec::with_capacity(pairs.right_rows.len());
            for (&left_row, &right_row) in pairs.left_rows.iter().zip(&pairs.right_rows) {
                let left_row = usize::try_from(left_row).map_err(|_| {
                    DataFusionError::Execution(
                        "native FileCode join left row id overflowed usize".into(),
                    )
                })?;
                let right_row = usize::try_from(right_row).map_err(|_| {
                    DataFusionError::Execution(
                        "native FileCode join right row id overflowed usize".into(),
                    )
                })?;
                let Some(left_value) = left_lane
                    .value_at(left_row)
                    .map_err(crate::adapter_v53::cove_to_datafusion)?
                else {
                    return Err(DataFusionError::Execution(
                        "native FileCode inner join emitted an invalid left row id".into(),
                    ));
                };
                let Some(right_value) = right_lane
                    .value_at(right_row)
                    .map_err(crate::adapter_v53::cove_to_datafusion)?
                else {
                    return Err(DataFusionError::Execution(
                        "native FileCode inner join emitted an invalid right row id".into(),
                    ));
                };
                left_out.push(Some(
                    canonical_utf8(left_value).map_err(crate::adapter_v53::cove_to_datafusion)?,
                ));
                right_out.push(Some(
                    canonical_utf8(right_value).map_err(crate::adapter_v53::cove_to_datafusion)?,
                ));
            }
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(StringArray::from(left_out)) as ArrayRef,
                    Arc::new(StringArray::from(right_out)) as ArrayRef,
                ],
            )
        }
        NativeI64JoinKind::LeftSemi => {
            let (selected, kernel_stats) = semi_join_u32_eq(
                &left_codes,
                left_lane.validity(),
                &domain,
                &right_codes,
                right_lane.validity(),
                &domain,
                None,
            )
            .map_err(crate::adapter_v53::cove_to_datafusion)?;
            stats.record_native_join_kernel(kernel_stats);
            let out = selected_filecode_values(left_lane, &selected)?;
            RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(out)) as ArrayRef])
        }
        NativeI64JoinKind::LeftAnti => {
            let (selected, kernel_stats) = anti_join_u32_eq_left_nulls_unmatched(
                &left_codes,
                left_lane.validity(),
                &domain,
                &right_codes,
                right_lane.validity(),
                &domain,
                None,
            )
            .map_err(crate::adapter_v53::cove_to_datafusion)?;
            stats.record_native_join_kernel(kernel_stats);
            let out = selected_filecode_values(left_lane, &selected)?;
            RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(out)) as ArrayRef])
        }
    }
    .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn remap_filecode_join_codes(
    left_lane: &NativeFileCodeDenseLane,
    right_lane: &NativeFileCodeDenseLane,
) -> Result<(Vec<u32>, Vec<u32>)> {
    let mut shared = HashMap::<Vec<u8>, u32>::with_capacity(
        left_lane
            .canonical_values()
            .len()
            .saturating_add(right_lane.canonical_values().len()),
    );
    let mut left_remap = Vec::with_capacity(left_lane.canonical_values().len());
    for canonical in left_lane.canonical_values() {
        let code = intern_join_canonical(&mut shared, canonical)?;
        left_remap.push(code);
    }
    let mut right_remap = Vec::with_capacity(right_lane.canonical_values().len());
    for canonical in right_lane.canonical_values() {
        let code = intern_join_canonical(&mut shared, canonical)?;
        right_remap.push(code);
    }
    Ok((
        remap_filecode_row_codes(left_lane.codes(), &left_remap)?,
        remap_filecode_row_codes(right_lane.codes(), &right_remap)?,
    ))
}

fn intern_join_canonical(shared: &mut HashMap<Vec<u8>, u32>, canonical: &[u8]) -> Result<u32> {
    if let Some(code) = shared.get(canonical).copied() {
        return Ok(code);
    }
    let code = u32::try_from(shared.len()).map_err(|_| {
        DataFusionError::Execution("native FileCode join execution-code domain overflow".into())
    })?;
    shared.insert(canonical.to_vec(), code);
    Ok(code)
}

fn remap_filecode_row_codes(codes: &[u32], remap: &[u32]) -> Result<Vec<u32>> {
    let mut out = Vec::with_capacity(codes.len());
    for code in codes {
        let code = usize::try_from(*code).map_err(|_| {
            DataFusionError::Execution("native FileCode row code overflowed usize".into())
        })?;
        out.push(*remap.get(code).ok_or_else(|| {
            DataFusionError::Execution(
                "native FileCode row referenced an unknown execution code".into(),
            )
        })?);
    }
    Ok(out)
}

fn selected_i64_values(
    lane: &NativeI64DenseLane,
    selected: &SelectionBitmap,
) -> Result<Vec<Option<i64>>> {
    let rows = selected.to_selection_vector();
    let mut out = Vec::with_capacity(rows.rows().len());
    for &row in rows.rows() {
        let row = usize::try_from(row).map_err(|_| {
            DataFusionError::Execution("native join selected row id overflowed usize".into())
        })?;
        out.push(
            lane.value_at(row)
                .map_err(crate::adapter_v53::cove_to_datafusion)?,
        );
    }
    Ok(out)
}

fn selected_filecode_values(
    lane: &NativeFileCodeDenseLane,
    selected: &SelectionBitmap,
) -> Result<Vec<Option<String>>> {
    let rows = selected.to_selection_vector();
    let mut out = Vec::with_capacity(rows.rows().len());
    for &row in rows.rows() {
        let row = usize::try_from(row).map_err(|_| {
            DataFusionError::Execution(
                "native FileCode join selected row id overflowed usize".into(),
            )
        })?;
        out.push(
            match lane
                .value_at(row)
                .map_err(crate::adapter_v53::cove_to_datafusion)?
            {
                Some(value) => {
                    Some(canonical_utf8(value).map_err(crate::adapter_v53::cove_to_datafusion)?)
                }
                None => None,
            },
        );
    }
    Ok(out)
}

fn native_i64_join_domain() -> NativeCodeDomain {
    NativeCodeDomain {
        semantic_domain_id: Some("cove.datafusion.native.i64".into()),
        null_policy: Some("validity-bitmap-nulls-never-match".into()),
        ..NativeCodeDomain::default()
    }
}

fn native_filecode_join_domain() -> NativeCodeDomain {
    NativeCodeDomain {
        semantic_domain_id: Some("cove.datafusion.native.filecode.canonical_utf8".into()),
        null_policy: Some("validity-bitmap-nulls-never-match".into()),
        dictionary_id: Some("execution-local-remap".into()),
        ..NativeCodeDomain::default()
    }
}
