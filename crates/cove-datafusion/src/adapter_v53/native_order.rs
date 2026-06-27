//! DataFusion 53.x native i64 order/top-N execution.

use std::{any::Any, sync::Arc};

use arrow_array::{ArrayRef, Int64Array, RecordBatch};
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
    adapter_v53::metrics::CoveFileMetrics, dataset_state::DatasetState,
    decode::native_i64_order_scan, planner::ScanPlan,
};

#[derive(Debug)]
pub(crate) struct CoveNativeI64OrderTableProvider {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    column_index: usize,
    filter_plan: ScanPlan,
    descending: bool,
    nulls_first: bool,
    fetch: Option<usize>,
}

impl CoveNativeI64OrderTableProvider {
    pub(crate) fn new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        column_index: usize,
        filter_plan: ScanPlan,
        descending: bool,
        nulls_first: bool,
        fetch: Option<usize>,
    ) -> Self {
        Self {
            schema,
            state,
            column_index,
            filter_plan,
            descending,
            nulls_first,
            fetch,
        }
    }
}

#[async_trait]
impl TableProvider for CoveNativeI64OrderTableProvider {
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
                "COVE native i64 order provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeI64OrderExec::try_new(
            Arc::clone(&self.schema),
            Arc::clone(&self.state),
            self.column_index,
            self.filter_plan.clone(),
            self.descending,
            self.nulls_first,
            self.fetch,
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(Statistics::new_unknown(self.schema.as_ref()))
    }
}

#[derive(Debug)]
pub(crate) struct CoveNativeI64OrderExec {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    column_index: usize,
    filter_plan: ScanPlan,
    descending: bool,
    nulls_first: bool,
    fetch: Option<usize>,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeI64OrderExec {
    fn try_new(
        schema: SchemaRef,
        state: Arc<DatasetState>,
        column_index: usize,
        filter_plan: ScanPlan,
        descending: bool,
        nulls_first: bool,
        fetch: Option<usize>,
    ) -> Result<Self> {
        if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
            return Err(DataFusionError::Plan(
                "native i64 order expects one Int64 output column".into(),
            ));
        }
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Final,
            Boundedness::Bounded,
        ));
        Ok(Self {
            schema,
            state,
            column_index,
            filter_plan,
            descending,
            nulls_first,
            fetch,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeI64OrderExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => write!(
                f,
                "{}: column={}, representation=typed_numeric_i64, semantic_domain=cove.datafusion.native.i64, kernel=shared_cove_core, null_policy=validity-bitmap, decode_boundary=none, fallback=none, files={}, filters={}, descending={}, nulls_first={}, fetch={:?}",
                self.name(),
                self.column_index,
                self.state.file_count(),
                self.filter_plan.filters.len(),
                self.descending,
                self.nulls_first,
                self.fetch
            ),
            DisplayFormatType::TreeRender => write!(f, "{}", self.name()),
        }
    }
}

impl ExecutionPlan for CoveNativeI64OrderExec {
    fn name(&self) -> &str {
        "CoveNativeI64OrderExec"
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
                "CoveNativeI64OrderExec is a leaf execution plan".into(),
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
                "CoveNativeI64OrderExec has one partition, got partition {partition}"
            )));
        }
        let scan = native_i64_order_scan(
            &self.state,
            self.column_index,
            &self.filter_plan,
            self.descending,
            self.nulls_first,
            self.fetch,
        )
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
        CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
        let batch = native_i64_order_batch(Arc::clone(&self.schema), scan.values)?;
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
                    "CoveNativeI64OrderExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(Statistics::new_unknown(self.schema.as_ref()))
    }
}

fn native_i64_order_batch(schema: SchemaRef, values: Vec<Option<i64>>) -> Result<RecordBatch> {
    RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values)) as ArrayRef])
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}
