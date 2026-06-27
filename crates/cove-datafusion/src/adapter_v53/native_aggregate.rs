//! DataFusion 53.x native scalar aggregate execution.

use std::{any::Any, collections::BTreeMap, sync::Arc};

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, UInt64Array,
};
use arrow_schema::{DataType, SchemaRef};
use async_trait::async_trait;
use datafusion::{
    catalog::{Session, TableProvider},
    common::{stats::Precision, DataFusionError, Result, ScalarValue, Statistics},
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
        native_bool_i64_group_aggregate_scan, native_filecode_i64_group_aggregate_scan,
        native_i64_aggregate_scan, native_i64_i64_group_aggregate_scan, NativeI64AggregateScan,
    },
    metadata_aggregate::canonical_utf8,
    planner::ScanPlan,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeI64AggregateKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl NativeI64AggregateKind {
    fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeI64AggregateRequest {
    pub(crate) column_index: usize,
    pub(crate) kind: NativeI64AggregateKind,
}

#[derive(Debug)]
pub(crate) struct CoveNativeAggregateTableProvider {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
}

impl CoveNativeAggregateTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            requests,
            filter_plan,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeBoolI64GroupAggregateTableProvider {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
}

impl CoveNativeBoolI64GroupAggregateTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            requests,
            filter_plan,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeI64I64GroupAggregateTableProvider {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
}

impl CoveNativeI64I64GroupAggregateTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            requests,
            filter_plan,
        }
    }
}

#[async_trait]
impl TableProvider for CoveNativeI64I64GroupAggregateTableProvider {
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
                "COVE native i64 grouped aggregate provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeI64I64GroupAggregateExec::try_new(
            Arc::clone(&self.schema),
            Arc::clone(&self.state),
            self.group_column_index,
            self.requests.clone(),
            self.filter_plan.clone(),
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[async_trait]
impl TableProvider for CoveNativeBoolI64GroupAggregateTableProvider {
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
                "COVE native grouped aggregate provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeBoolI64GroupAggregateExec::try_new(
            Arc::clone(&self.schema),
            Arc::clone(&self.state),
            self.group_column_index,
            self.requests.clone(),
            self.filter_plan.clone(),
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeFileCodeI64GroupAggregateTableProvider {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
}

impl CoveNativeFileCodeI64GroupAggregateTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Self {
        Self {
            schema,
            state,
            group_column_index,
            requests,
            filter_plan,
        }
    }
}

#[async_trait]
impl TableProvider for CoveNativeFileCodeI64GroupAggregateTableProvider {
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
                "COVE native FileCode grouped aggregate provider does not accept pushed filters"
                    .into(),
            ));
        }
        CoveNativeFileCodeI64GroupAggregateExec::try_new(
            Arc::clone(&self.schema),
            Arc::clone(&self.state),
            self.group_column_index,
            self.requests.clone(),
            self.filter_plan.clone(),
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[async_trait]
impl TableProvider for CoveNativeAggregateTableProvider {
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
                "COVE native aggregate provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeAggregateExec::try_new(
            Arc::clone(&self.schema),
            Arc::clone(&self.state),
            self.requests.clone(),
            self.filter_plan.clone(),
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        statistics.num_rows = Precision::Exact(1);
        statistics.calculate_total_byte_size(self.schema.as_ref());
        Some(statistics)
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeBoolI64GroupAggregateExec {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    value_column_index: usize,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeBoolI64GroupAggregateExec {
    fn try_new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Result<Self> {
        let Some(first_request) = requests.first().copied() else {
            return Err(DataFusionError::Plan(
                "COVE native grouped aggregate exec requires at least one request".into(),
            ));
        };
        if requests
            .iter()
            .any(|request| request.column_index != first_request.column_index)
        {
            return Err(DataFusionError::Plan(
                "COVE native grouped aggregate exec currently requires one i64 value column".into(),
            ));
        }
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
            value_column_index: first_request.column_index,
            requests,
            filter_plan,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeBoolI64GroupAggregateExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                let requests = self
                    .requests
                    .iter()
                    .map(|request| format!("{}#{}", request.kind.name(), request.column_index))
                    .collect::<Vec<_>>()
                    .join(",");
                write!(
                    f,
                    "CoveNativeBoolI64GroupAggregateExec: group_column={}, requests=[{}], representation=group_key:boolean_dense,value:typed_numeric_i64, semantic_domain=key:cove.datafusion.native.bool,value:cove.datafusion.native.i64, kernel=shared_cove_core, null_policy=validity-bitmap, decode_boundary=none, fallback=page-decode-boundary, files={}, filters={}",
                    self.group_column_index,
                    requests,
                    self.state.file_count(),
                    self.filter_plan.filters.len()
                )
            }
            DisplayFormatType::TreeRender => write!(f, "CoveNativeBoolI64GroupAggregateExec"),
        }
    }
}

impl ExecutionPlan for CoveNativeBoolI64GroupAggregateExec {
    fn name(&self) -> &str {
        "CoveNativeBoolI64GroupAggregateExec"
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
                "CoveNativeBoolI64GroupAggregateExec is a leaf execution plan".into(),
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
                "CoveNativeBoolI64GroupAggregateExec has one partition, got partition {partition}"
            )));
        }
        let scan = native_bool_i64_group_aggregate_scan(
            &self.state,
            self.group_column_index,
            self.value_column_index,
            &self.filter_plan,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
        let batch = native_bool_i64_group_aggregate_batch(
            Arc::clone(&self.schema),
            &self.requests,
            scan.groups,
        )?;
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
                    "CoveNativeBoolI64GroupAggregateExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeI64I64GroupAggregateExec {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    value_column_index: usize,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeI64I64GroupAggregateExec {
    fn try_new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Result<Self> {
        let Some(first_request) = requests.first().copied() else {
            return Err(DataFusionError::Plan(
                "COVE native i64 grouped aggregate exec requires at least one request".into(),
            ));
        };
        if requests
            .iter()
            .any(|request| request.column_index != first_request.column_index)
        {
            return Err(DataFusionError::Plan(
                "COVE native i64 grouped aggregate exec currently requires one i64 value column"
                    .into(),
            ));
        }
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
            value_column_index: first_request.column_index,
            requests,
            filter_plan,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeI64I64GroupAggregateExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                let requests = self
                    .requests
                    .iter()
                    .map(|request| format!("{}#{}", request.kind.name(), request.column_index))
                    .collect::<Vec<_>>()
                    .join(",");
                write!(
                    f,
                    "CoveNativeI64I64GroupAggregateExec: group_column={}, requests=[{}], representation=group_key:typed_numeric_i64,value:typed_numeric_i64, semantic_domain=key:cove.datafusion.native.i64,value:cove.datafusion.native.i64, kernel=shared_cove_core, null_policy=validity-bitmap, decode_boundary=none, fallback=page-decode-boundary, files={}, filters={}",
                    self.group_column_index,
                    requests,
                    self.state.file_count(),
                    self.filter_plan.filters.len()
                )
            }
            DisplayFormatType::TreeRender => write!(f, "CoveNativeI64I64GroupAggregateExec"),
        }
    }
}

impl ExecutionPlan for CoveNativeI64I64GroupAggregateExec {
    fn name(&self) -> &str {
        "CoveNativeI64I64GroupAggregateExec"
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
                "CoveNativeI64I64GroupAggregateExec is a leaf execution plan".into(),
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
                "CoveNativeI64I64GroupAggregateExec has one partition, got partition {partition}"
            )));
        }
        let scan = native_i64_i64_group_aggregate_scan(
            &self.state,
            self.group_column_index,
            self.value_column_index,
            &self.filter_plan,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
        let batch = native_i64_i64_group_aggregate_batch(
            Arc::clone(&self.schema),
            &self.requests,
            scan.groups,
        )?;
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
                    "CoveNativeI64I64GroupAggregateExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeFileCodeI64GroupAggregateExec {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    group_column_index: usize,
    value_column_index: usize,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeFileCodeI64GroupAggregateExec {
    fn try_new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        group_column_index: usize,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Result<Self> {
        let Some(first_request) = requests.first().copied() else {
            return Err(DataFusionError::Plan(
                "COVE native FileCode grouped aggregate exec requires at least one request".into(),
            ));
        };
        if requests
            .iter()
            .any(|request| request.column_index != first_request.column_index)
        {
            return Err(DataFusionError::Plan(
                "COVE native FileCode grouped aggregate exec currently requires one i64 value column"
                    .into(),
            ));
        }
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
            value_column_index: first_request.column_index,
            requests,
            filter_plan,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeFileCodeI64GroupAggregateExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                let requests = self
                    .requests
                    .iter()
                    .map(|request| format!("{}#{}", request.kind.name(), request.column_index))
                    .collect::<Vec<_>>()
                    .join(",");
                write!(
                    f,
                    "CoveNativeFileCodeI64GroupAggregateExec: group_column={}, requests=[{}], representation=group_key:filecode_utf8,value:typed_numeric_i64, semantic_domain=key:file-local-dictionary-to-canonical-utf8,value:cove.datafusion.native.i64, kernel=shared_cove_core, null_policy=validity-bitmap, decode_boundary=group-label-output, fallback=page-decode-boundary, files={}, filters={}",
                    self.group_column_index,
                    requests,
                    self.state.file_count(),
                    self.filter_plan.filters.len()
                )
            }
            DisplayFormatType::TreeRender => {
                write!(f, "CoveNativeFileCodeI64GroupAggregateExec")
            }
        }
    }
}

impl ExecutionPlan for CoveNativeFileCodeI64GroupAggregateExec {
    fn name(&self) -> &str {
        "CoveNativeFileCodeI64GroupAggregateExec"
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
                "CoveNativeFileCodeI64GroupAggregateExec is a leaf execution plan".into(),
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
                "CoveNativeFileCodeI64GroupAggregateExec has one partition, got partition {partition}"
            )));
        }
        let scan = native_filecode_i64_group_aggregate_scan(
            &self.state,
            self.group_column_index,
            self.value_column_index,
            &self.filter_plan,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
        let batch = native_filecode_i64_group_aggregate_batch(
            Arc::clone(&self.schema),
            &self.requests,
            scan.groups,
        )?;
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
                    "CoveNativeFileCodeI64GroupAggregateExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeAggregateExec {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    requests: Vec<NativeI64AggregateRequest>,
    filter_plan: ScanPlan,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeAggregateExec {
    fn try_new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        requests: Vec<NativeI64AggregateRequest>,
        filter_plan: ScanPlan,
    ) -> Result<Self> {
        if requests.is_empty() {
            return Err(DataFusionError::Plan(
                "COVE native aggregate exec requires at least one request".into(),
            ));
        }
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Ok(Self {
            schema,
            state,
            requests,
            filter_plan,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeAggregateExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                let requests = self
                    .requests
                    .iter()
                    .map(|request| format!("{}#{}", request.kind.name(), request.column_index))
                    .collect::<Vec<_>>()
                    .join(",");
                write!(
                    f,
                    "CoveNativeAggregateExec: requests=[{}], representation=typed_numeric_i64, semantic_domain=cove.datafusion.native.i64, kernel=shared_cove_core, null_policy=validity-bitmap, decode_boundary=none, fallback=none, files={}, filters={}",
                    requests,
                    self.state.file_count(),
                    self.filter_plan.filters.len()
                )
            }
            DisplayFormatType::TreeRender => write!(f, "CoveNativeAggregateExec"),
        }
    }
}

impl ExecutionPlan for CoveNativeAggregateExec {
    fn name(&self) -> &str {
        "CoveNativeAggregateExec"
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
                "CoveNativeAggregateExec is a leaf execution plan".into(),
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
                "CoveNativeAggregateExec has one partition, got partition {partition}"
            )));
        }
        let batch = native_aggregate_batch(
            Arc::clone(&self.schema),
            self.state.as_ref(),
            &self.requests,
            &self.filter_plan,
            &self.metrics,
            partition,
        )?;
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
                    "CoveNativeAggregateExec has one partition, got partition {partition}"
                )));
            }
        }
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        statistics.num_rows = Precision::Exact(1);
        statistics.calculate_total_byte_size(self.schema.as_ref());
        Ok(statistics)
    }
}

fn native_aggregate_batch(
    schema: SchemaRef,
    state: &DatasetState,
    requests: &[NativeI64AggregateRequest],
    filter_plan: &ScanPlan,
    metrics: &ExecutionPlanMetricsSet,
    partition: usize,
) -> Result<RecordBatch> {
    let mut scans = BTreeMap::<usize, NativeI64AggregateScan>::new();
    for request in requests {
        if !scans.contains_key(&request.column_index) {
            let scan = native_i64_aggregate_scan(state, request.column_index, filter_plan)
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
            CoveFileMetrics::new(metrics, partition).record_decode(scan.stats);
            scans.insert(request.column_index, scan);
        }
    }

    let mut arrays = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        let scan = scans.get(&request.column_index).ok_or_else(|| {
            DataFusionError::Internal(format!(
                "missing native aggregate scan for column {}",
                request.column_index
            ))
        })?;
        let field = schema.field(index);
        arrays.push(
            native_aggregate_scalar(request.kind, &scan.aggregate, field.data_type())?
                .to_array()?,
        );
    }
    RecordBatch::try_new(schema, arrays)
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_bool_i64_group_aggregate_batch(
    schema: SchemaRef,
    requests: &[NativeI64AggregateRequest],
    groups: cove_core::native::NativeDenseI64GroupAggregates,
) -> Result<RecordBatch> {
    if schema.fields().len() != requests.len() + 1
        || schema.field(0).data_type() != &DataType::Boolean
    {
        return Err(DataFusionError::Plan(
            "native bool/i64 grouped aggregate expects Boolean key followed by aggregate columns"
                .into(),
        ));
    }
    if groups.aggregates.len() != 2 || groups.row_counts.len() != 2 {
        return Err(DataFusionError::Plan(
            "native bool/i64 grouped aggregate received non-boolean dense groups".into(),
        ));
    }
    let mut keys = Vec::with_capacity(
        usize::from(groups.row_counts[0] != 0)
            + usize::from(groups.row_counts[1] != 0)
            + usize::from(groups.null_row_count != 0),
    );
    let mut aggregates = Vec::with_capacity(keys.capacity());
    if groups.row_counts[0] != 0 {
        keys.push(Some(false));
        aggregates.push(groups.aggregates[0].clone());
    }
    if groups.row_counts[1] != 0 {
        keys.push(Some(true));
        aggregates.push(groups.aggregates[1].clone());
    }
    if groups.null_row_count != 0 {
        keys.push(None);
        aggregates.push(groups.null_aggregate);
    }

    let mut arrays = Vec::with_capacity(requests.len() + 1);
    arrays.push(Arc::new(BooleanArray::from(keys)) as ArrayRef);
    for (index, request) in requests.iter().enumerate() {
        let field = schema.field(index + 1);
        arrays.push(native_grouped_i64_aggregate_array(
            request.kind,
            &aggregates,
            field.data_type(),
        )?);
    }
    RecordBatch::try_new(schema, arrays)
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_i64_i64_group_aggregate_batch(
    schema: SchemaRef,
    requests: &[NativeI64AggregateRequest],
    groups: cove_core::native::NativeI64I64HashGroupAggregates,
) -> Result<RecordBatch> {
    if schema.fields().len() != requests.len() + 1
        || schema.field(0).data_type() != &DataType::Int64
    {
        return Err(DataFusionError::Plan(
            "native i64/i64 grouped aggregate expects Int64 key followed by aggregate columns"
                .into(),
        ));
    }
    let mut values = groups
        .aggregates
        .into_iter()
        .map(|(key, aggregate)| (key, aggregate))
        .collect::<Vec<_>>();
    values.sort_unstable_by_key(|(key, _)| *key);

    let mut keys = Vec::with_capacity(values.len() + usize::from(groups.null_row_count != 0));
    let mut aggregates = Vec::with_capacity(keys.capacity());
    for (key, aggregate) in values {
        keys.push(Some(key));
        aggregates.push(aggregate);
    }
    if groups.null_row_count != 0 {
        keys.push(None);
        aggregates.push(groups.null_aggregate);
    }

    let mut arrays = Vec::with_capacity(requests.len() + 1);
    arrays.push(Arc::new(Int64Array::from(keys)) as ArrayRef);
    for (index, request) in requests.iter().enumerate() {
        let field = schema.field(index + 1);
        arrays.push(native_grouped_i64_aggregate_array(
            request.kind,
            &aggregates,
            field.data_type(),
        )?);
    }
    RecordBatch::try_new(schema, arrays)
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_filecode_i64_group_aggregate_batch(
    schema: SchemaRef,
    requests: &[NativeI64AggregateRequest],
    groups: crate::decode::NativeFileCodeI64GroupAggregates,
) -> Result<RecordBatch> {
    if schema.fields().len() != requests.len() + 1 || schema.field(0).data_type() != &DataType::Utf8
    {
        return Err(DataFusionError::Plan(
            "native FileCode/i64 grouped aggregate expects Utf8 key followed by aggregate columns"
                .into(),
        ));
    }
    let mut labels =
        Vec::with_capacity(groups.groups.len() + usize::from(groups.null_row_count != 0));
    let mut aggregates = Vec::with_capacity(labels.capacity());
    for (canonical_key, group) in groups.groups {
        labels.push(Some(
            canonical_utf8(&canonical_key).map_err(crate::adapter_v53::cove_to_datafusion)?,
        ));
        aggregates.push(group.aggregate);
    }
    if groups.null_row_count != 0 {
        labels.push(None);
        aggregates.push(groups.null_aggregate);
    }

    let mut arrays = Vec::with_capacity(requests.len() + 1);
    arrays.push(Arc::new(StringArray::from(
        labels
            .iter()
            .map(|label| label.as_ref().map(String::as_str))
            .collect::<Vec<_>>(),
    )) as ArrayRef);
    for (index, request) in requests.iter().enumerate() {
        let field = schema.field(index + 1);
        arrays.push(native_grouped_i64_aggregate_array(
            request.kind,
            &aggregates,
            field.data_type(),
        )?);
    }
    RecordBatch::try_new(schema, arrays)
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn native_grouped_i64_aggregate_array(
    kind: NativeI64AggregateKind,
    aggregates: &[cove_core::native::NativeI64Aggregates],
    data_type: &DataType,
) -> Result<ArrayRef> {
    match (kind, data_type) {
        (NativeI64AggregateKind::Count, DataType::Int64) => aggregates
            .iter()
            .map(|aggregate| {
                i64::try_from(aggregate.count).map(Some).map_err(|_| {
                    DataFusionError::Plan("native grouped i64 COUNT result exceeds Int64".into())
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| Arc::new(Int64Array::from(values)) as ArrayRef),
        (NativeI64AggregateKind::Count, DataType::UInt64) => Ok(Arc::new(UInt64Array::from(
            aggregates
                .iter()
                .map(|aggregate| Some(aggregate.count))
                .collect::<Vec<_>>(),
        )) as ArrayRef),
        (NativeI64AggregateKind::Sum, DataType::Int64) => aggregates
            .iter()
            .map(|aggregate| {
                if aggregate.count == 0 {
                    Ok(None)
                } else {
                    i64::try_from(aggregate.sum).map(Some).map_err(|_| {
                        DataFusionError::Plan("native grouped i64 SUM result exceeds Int64".into())
                    })
                }
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| Arc::new(Int64Array::from(values)) as ArrayRef),
        (NativeI64AggregateKind::Avg, DataType::Float64) => Ok(Arc::new(Float64Array::from(
            aggregates
                .iter()
                .map(|aggregate| {
                    if aggregate.count == 0 {
                        None
                    } else {
                        Some(aggregate.sum as f64 / aggregate.count as f64)
                    }
                })
                .collect::<Vec<_>>(),
        )) as ArrayRef),
        (NativeI64AggregateKind::Min, DataType::Int64) => Ok(Arc::new(Int64Array::from(
            aggregates
                .iter()
                .map(|aggregate| {
                    if aggregate.count == 0 {
                        None
                    } else {
                        aggregate.min
                    }
                })
                .collect::<Vec<_>>(),
        )) as ArrayRef),
        (NativeI64AggregateKind::Max, DataType::Int64) => Ok(Arc::new(Int64Array::from(
            aggregates
                .iter()
                .map(|aggregate| {
                    if aggregate.count == 0 {
                        None
                    } else {
                        aggregate.max
                    }
                })
                .collect::<Vec<_>>(),
        )) as ArrayRef),
        (kind, data_type) => Err(DataFusionError::Plan(format!(
            "unsupported native grouped i64 {} output type {data_type:?}",
            kind.name()
        ))),
    }
}

fn native_aggregate_scalar(
    kind: NativeI64AggregateKind,
    aggregate: &cove_core::native::NativeI64Aggregates,
    data_type: &DataType,
) -> Result<ScalarValue> {
    if aggregate.count == 0 {
        return native_aggregate_null(kind, data_type);
    }
    match (kind, data_type) {
        (NativeI64AggregateKind::Count, DataType::Int64) => Ok(ScalarValue::Int64(Some(
            i64::try_from(aggregate.count).map_err(|_| {
                DataFusionError::Plan("native i64 COUNT result exceeds Int64".into())
            })?,
        ))),
        (NativeI64AggregateKind::Count, DataType::UInt64) => {
            Ok(ScalarValue::UInt64(Some(aggregate.count)))
        }
        (NativeI64AggregateKind::Sum, DataType::Int64) => Ok(ScalarValue::Int64(Some(
            i64::try_from(aggregate.sum)
                .map_err(|_| DataFusionError::Plan("native i64 SUM result exceeds Int64".into()))?,
        ))),
        (NativeI64AggregateKind::Avg, DataType::Float64) => Ok(ScalarValue::Float64(Some(
            aggregate.sum as f64 / aggregate.count as f64,
        ))),
        (NativeI64AggregateKind::Min, DataType::Int64) => Ok(ScalarValue::Int64(aggregate.min)),
        (NativeI64AggregateKind::Max, DataType::Int64) => Ok(ScalarValue::Int64(aggregate.max)),
        (kind, data_type) => Err(DataFusionError::Plan(format!(
            "unsupported native i64 {} output type {data_type:?}",
            kind.name()
        ))),
    }
}

fn native_aggregate_null(
    kind: NativeI64AggregateKind,
    data_type: &DataType,
) -> Result<ScalarValue> {
    match (kind, data_type) {
        (NativeI64AggregateKind::Count, DataType::Int64) => Ok(ScalarValue::Int64(Some(0))),
        (NativeI64AggregateKind::Count, DataType::UInt64) => Ok(ScalarValue::UInt64(Some(0))),
        (NativeI64AggregateKind::Sum, DataType::Int64)
        | (NativeI64AggregateKind::Min, DataType::Int64)
        | (NativeI64AggregateKind::Max, DataType::Int64) => Ok(ScalarValue::Int64(None)),
        (NativeI64AggregateKind::Avg, DataType::Float64) => Ok(ScalarValue::Float64(None)),
        (kind, data_type) => Err(DataFusionError::Plan(format!(
            "unsupported native i64 {} null output type {data_type:?}",
            kind.name()
        ))),
    }
}
