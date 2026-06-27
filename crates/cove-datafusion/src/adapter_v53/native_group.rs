//! DataFusion 53.x native group-count execution.

use std::{any::Any, sync::Arc};

use arrow_array::{ArrayRef, BooleanArray, Int64Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, SchemaRef};
use async_trait::async_trait;
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
        native_bool_group_count_scan, native_filecode_group_count_scan, native_i64_group_count_scan,
    },
    metadata_aggregate::canonical_utf8,
    planner::ScanPlan,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeI64GroupOutput {
    Count,
    Distinct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeGroupKeyKind {
    I64,
    Bool,
    FileCodeUtf8,
}

#[derive(Debug)]
pub(crate) struct CoveNativeGroupCountTableProvider {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    filter_plan: ScanPlan,
    key_kind: NativeGroupKeyKind,
    output: NativeI64GroupOutput,
}

impl CoveNativeGroupCountTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            filter_plan,
            key_kind: NativeGroupKeyKind::I64,
            output: NativeI64GroupOutput::Count,
        }
    }

    pub(crate) fn filecode_utf8_count(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            filter_plan,
            key_kind: NativeGroupKeyKind::FileCodeUtf8,
            output: NativeI64GroupOutput::Count,
        }
    }

    pub(crate) fn bool_count(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            filter_plan,
            key_kind: NativeGroupKeyKind::Bool,
            output: NativeI64GroupOutput::Count,
        }
    }

    pub(crate) fn bool_distinct(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            filter_plan,
            key_kind: NativeGroupKeyKind::Bool,
            output: NativeI64GroupOutput::Distinct,
        }
    }

    pub(crate) fn filecode_utf8_distinct(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            filter_plan,
            key_kind: NativeGroupKeyKind::FileCodeUtf8,
            output: NativeI64GroupOutput::Distinct,
        }
    }

    pub(crate) fn distinct(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            filter_plan,
            key_kind: NativeGroupKeyKind::I64,
            output: NativeI64GroupOutput::Distinct,
        }
    }
}

#[async_trait]
impl TableProvider for CoveNativeGroupCountTableProvider {
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
                "COVE native group provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeGroupCountExec::try_new(
            Arc::clone(&self.schema),
            Arc::clone(&self.state),
            self.group_column_index,
            self.filter_plan.clone(),
            self.key_kind,
            self.output,
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeGroupCountExec {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    filter_plan: ScanPlan,
    key_kind: NativeGroupKeyKind,
    output: NativeI64GroupOutput,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeGroupCountExec {
    fn try_new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        filter_plan: ScanPlan,
        key_kind: NativeGroupKeyKind,
        output: NativeI64GroupOutput,
    ) -> Result<Self> {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Ok(Self {
            schema,
            state,
            group_column_index,
            filter_plan,
            key_kind,
            output,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeGroupCountExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                let (representation, semantic_domain, decode_boundary, fallback) =
                    match self.key_kind {
                        NativeGroupKeyKind::I64 => (
                            "typed_numeric_i64",
                            "cove.datafusion.native.i64",
                            "none",
                            "none",
                        ),
                        NativeGroupKeyKind::Bool => (
                            "boolean_dense",
                            "cove.datafusion.native.bool",
                            "none",
                            "page-decode-boundary",
                        ),
                        NativeGroupKeyKind::FileCodeUtf8 => (
                            "filecode_utf8",
                            "file-local-dictionary-to-canonical-utf8",
                            "group-label-output",
                            "page-decode-boundary",
                        ),
                    };
                write!(
                    f,
                    "{}: group_column={}, representation={representation}, semantic_domain={semantic_domain}, kernel=shared_cove_core, null_policy=validity-bitmap, decode_boundary={decode_boundary}, fallback={fallback}, files={}, filters={}",
                    self.name(),
                    self.group_column_index,
                    self.state.file_count(),
                    self.filter_plan.filters.len()
                )
            }
            DisplayFormatType::TreeRender => write!(f, "{}", self.name()),
        }
    }
}

impl ExecutionPlan for CoveNativeGroupCountExec {
    fn name(&self) -> &str {
        match self.output {
            NativeI64GroupOutput::Count => "CoveNativeGroupCountExec",
            NativeI64GroupOutput::Distinct => "CoveNativeGroupDistinctExec",
        }
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
                "CoveNativeGroupCountExec is a leaf execution plan".into(),
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
                "CoveNativeGroupCountExec has one partition, got partition {partition}"
            )));
        }
        let batch = match (self.key_kind, self.output) {
            (NativeGroupKeyKind::I64, NativeI64GroupOutput::Count) => {
                let scan = native_i64_group_count_scan(
                    &self.state,
                    self.group_column_index,
                    &self.filter_plan,
                )
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
                CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
                native_group_count_batch(Arc::clone(&self.schema), scan.groups)?
            }
            (NativeGroupKeyKind::I64, NativeI64GroupOutput::Distinct) => {
                let scan = native_i64_group_count_scan(
                    &self.state,
                    self.group_column_index,
                    &self.filter_plan,
                )
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
                CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
                native_group_distinct_batch(Arc::clone(&self.schema), scan.groups)?
            }
            (NativeGroupKeyKind::Bool, NativeI64GroupOutput::Count) => {
                let scan = native_bool_group_count_scan(
                    &self.state,
                    self.group_column_index,
                    &self.filter_plan,
                )
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
                CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
                native_bool_group_count_batch(Arc::clone(&self.schema), scan.groups)?
            }
            (NativeGroupKeyKind::Bool, NativeI64GroupOutput::Distinct) => {
                let scan = native_bool_group_count_scan(
                    &self.state,
                    self.group_column_index,
                    &self.filter_plan,
                )
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
                CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
                native_bool_group_distinct_batch(Arc::clone(&self.schema), scan.groups)?
            }
            (NativeGroupKeyKind::FileCodeUtf8, NativeI64GroupOutput::Count) => {
                let scan = native_filecode_group_count_scan(
                    &self.state,
                    self.group_column_index,
                    &self.filter_plan,
                )
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
                CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
                native_filecode_group_count_batch(Arc::clone(&self.schema), scan.groups)?
            }
            (NativeGroupKeyKind::FileCodeUtf8, NativeI64GroupOutput::Distinct) => {
                let scan = native_filecode_group_count_scan(
                    &self.state,
                    self.group_column_index,
                    &self.filter_plan,
                )
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
                CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
                native_filecode_group_distinct_batch(Arc::clone(&self.schema), scan.groups)?
            }
        };
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
                    "CoveNativeGroupCountExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(Statistics::new_unknown(self.schema.as_ref()))
    }
}

fn native_group_count_batch(
    schema: SchemaRef,
    groups: cove_core::native::NativeI64HashGroupCounts,
) -> Result<RecordBatch> {
    if schema.fields().len() != 2 || schema.field(0).data_type() != &DataType::Int64 {
        return Err(DataFusionError::Plan(
            "native i64 group count expects Int64 group key and one count column".into(),
        ));
    }
    let mut values = groups.counts.into_iter().collect::<Vec<_>>();
    values.sort_unstable_by_key(|(value, _)| *value);
    let mut keys = values
        .iter()
        .map(|(value, _)| Some(*value))
        .collect::<Vec<_>>();
    let mut counts = values.iter().map(|(_, count)| *count).collect::<Vec<_>>();
    if groups.null_count != 0 {
        keys.push(None);
        counts.push(groups.null_count);
    }
    let count_array = count_array_for_values(&counts, schema.field(1).data_type())?;
    RecordBatch::try_new(
        schema,
        vec![Arc::new(Int64Array::from(keys)) as ArrayRef, count_array],
    )
    .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_filecode_group_count_batch(
    schema: SchemaRef,
    groups: crate::decode::NativeFileCodeGroupCounts,
) -> Result<RecordBatch> {
    if schema.fields().len() != 2 || schema.field(0).data_type() != &DataType::Utf8 {
        return Err(DataFusionError::Plan(
            "native FileCode group count expects Utf8 group key and one count column".into(),
        ));
    }
    let mut values = groups
        .counts
        .into_iter()
        .map(|(canonical_value, count)| {
            canonical_utf8(&canonical_value)
                .map(|label| (label, count))
                .map_err(crate::adapter_v53::cove_to_datafusion)
        })
        .collect::<Result<Vec<_>>>()?;
    values.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut labels = values
        .iter()
        .map(|(label, _)| Some(label.as_str()))
        .collect::<Vec<_>>();
    let mut counts = values.iter().map(|(_, count)| *count).collect::<Vec<_>>();
    if groups.null_count != 0 {
        labels.push(None);
        counts.push(groups.null_count);
    }
    let count_array = count_array_for_values(&counts, schema.field(1).data_type())?;
    RecordBatch::try_new(
        schema,
        vec![Arc::new(StringArray::from(labels)) as ArrayRef, count_array],
    )
    .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_bool_group_count_batch(
    schema: SchemaRef,
    groups: cove_core::native::NativeDenseGroupCounts,
) -> Result<RecordBatch> {
    if schema.fields().len() != 2 || schema.field(0).data_type() != &DataType::Boolean {
        return Err(DataFusionError::Plan(
            "native bool group count expects Boolean group key and one count column".into(),
        ));
    }
    if groups.counts.len() != 2 {
        return Err(DataFusionError::Plan(
            "native bool group count received non-boolean dense groups".into(),
        ));
    }
    let mut keys = Vec::with_capacity(
        usize::from(groups.counts[0] != 0)
            + usize::from(groups.counts[1] != 0)
            + usize::from(groups.null_count != 0),
    );
    let mut counts = Vec::with_capacity(keys.capacity());
    if groups.counts[0] != 0 {
        keys.push(Some(false));
        counts.push(groups.counts[0]);
    }
    if groups.counts[1] != 0 {
        keys.push(Some(true));
        counts.push(groups.counts[1]);
    }
    if groups.null_count != 0 {
        keys.push(None);
        counts.push(groups.null_count);
    }
    let count_array = count_array_for_values(&counts, schema.field(1).data_type())?;
    RecordBatch::try_new(
        schema,
        vec![Arc::new(BooleanArray::from(keys)) as ArrayRef, count_array],
    )
    .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_filecode_group_distinct_batch(
    schema: SchemaRef,
    groups: crate::decode::NativeFileCodeGroupCounts,
) -> Result<RecordBatch> {
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Utf8 {
        return Err(DataFusionError::Plan(
            "native FileCode distinct group expects one Utf8 group key column".into(),
        ));
    }
    let mut labels = groups
        .counts
        .into_keys()
        .map(|canonical_value| {
            canonical_utf8(&canonical_value)
                .map(Some)
                .map_err(crate::adapter_v53::cove_to_datafusion)
        })
        .collect::<Result<Vec<_>>>()?;
    labels.sort_unstable();
    if groups.null_count != 0 {
        labels.push(None);
    }
    RecordBatch::try_new(
        schema,
        vec![Arc::new(StringArray::from(labels)) as ArrayRef],
    )
    .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_bool_group_distinct_batch(
    schema: SchemaRef,
    groups: cove_core::native::NativeDenseGroupCounts,
) -> Result<RecordBatch> {
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Boolean {
        return Err(DataFusionError::Plan(
            "native bool distinct group expects one Boolean group key column".into(),
        ));
    }
    if groups.counts.len() != 2 {
        return Err(DataFusionError::Plan(
            "native bool distinct group received non-boolean dense groups".into(),
        ));
    }
    let mut keys = Vec::with_capacity(
        usize::from(groups.counts[0] != 0)
            + usize::from(groups.counts[1] != 0)
            + usize::from(groups.null_count != 0),
    );
    if groups.counts[0] != 0 {
        keys.push(Some(false));
    }
    if groups.counts[1] != 0 {
        keys.push(Some(true));
    }
    if groups.null_count != 0 {
        keys.push(None);
    }
    RecordBatch::try_new(schema, vec![Arc::new(BooleanArray::from(keys)) as ArrayRef])
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_group_distinct_batch(
    schema: SchemaRef,
    groups: cove_core::native::NativeI64HashGroupCounts,
) -> Result<RecordBatch> {
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
        return Err(DataFusionError::Plan(
            "native i64 distinct group expects one Int64 group key column".into(),
        ));
    }
    let mut keys = groups.counts.into_keys().map(Some).collect::<Vec<_>>();
    keys.sort_unstable();
    if groups.null_count != 0 {
        keys.push(None);
    }
    RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(keys)) as ArrayRef])
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn count_array_for_values(counts: &[u64], data_type: &DataType) -> Result<ArrayRef> {
    match data_type {
        DataType::Int64 => counts
            .iter()
            .map(|count| {
                i64::try_from(*count).map_err(|_| {
                    DataFusionError::Plan("native group COUNT result exceeds Int64".into())
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| Arc::new(Int64Array::from(values)) as ArrayRef),
        DataType::UInt64 => Ok(Arc::new(UInt64Array::from(counts.to_vec())) as ArrayRef),
        other => Err(DataFusionError::Plan(format!(
            "unsupported native group COUNT output type {other:?}"
        ))),
    }
}
