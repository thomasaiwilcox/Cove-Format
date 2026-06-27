//! DataFusion 53.x native row-count execution.

use std::{any::Any, sync::Arc};

use arrow_array::{ArrayRef, Int64Array, RecordBatch, UInt64Array};
use arrow_schema::{DataType, SchemaRef};
use async_trait::async_trait;
use datafusion::{
    catalog::{Session, TableProvider},
    common::{stats::Precision, DataFusionError, Result, Statistics},
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
    decode::native_row_count_scan, planner::ScanPlan,
};

#[derive(Debug)]
pub(crate) struct CoveNativeCountTableProvider {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    filter_plan: ScanPlan,
}

impl CoveNativeCountTableProvider {
    pub(crate) fn new(schema: SchemaRef, state: Arc<DatasetState>, filter_plan: ScanPlan) -> Self {
        Self {
            schema,
            state,
            filter_plan,
        }
    }
}

#[async_trait]
impl TableProvider for CoveNativeCountTableProvider {
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
                "COVE native count provider does not accept pushed filters".into(),
            ));
        }
        CoveNativeCountExec::try_new(
            Arc::clone(&self.schema),
            Arc::clone(&self.state),
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
pub(crate) struct CoveNativeCountExec {
    schema: SchemaRef,
    state: Arc<DatasetState>,
    filter_plan: ScanPlan,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveNativeCountExec {
    fn try_new(schema: SchemaRef, state: Arc<DatasetState>, filter_plan: ScanPlan) -> Result<Self> {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Ok(Self {
            schema,
            state,
            filter_plan,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for CoveNativeCountExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => write!(
                f,
                "CoveNativeCountExec: representation=rowset_count, semantic_domain=none, kernel=shared_cove_core, decode_boundary=none, fallback=none, files={}, filters={}",
                self.state.file_count(),
                self.filter_plan.filters.len()
            ),
            DisplayFormatType::TreeRender => write!(f, "CoveNativeCountExec"),
        }
    }
}

impl ExecutionPlan for CoveNativeCountExec {
    fn name(&self) -> &str {
        "CoveNativeCountExec"
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
                "CoveNativeCountExec is a leaf execution plan".into(),
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
                "CoveNativeCountExec has one partition, got partition {partition}"
            )));
        }
        let scan = native_row_count_scan(&self.state, &self.filter_plan)
            .map_err(crate::adapter_v53::cove_to_datafusion)?;
        CoveFileMetrics::new(&self.metrics, partition).record_decode(scan.stats);
        let batch = native_count_batch(Arc::clone(&self.schema), scan.count)?;
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
                    "CoveNativeCountExec has one partition, got partition {partition}"
                )));
            }
        }
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        statistics.num_rows = Precision::Exact(1);
        statistics.calculate_total_byte_size(self.schema.as_ref());
        Ok(statistics)
    }
}

fn native_count_batch(schema: SchemaRef, count: u64) -> Result<RecordBatch> {
    if schema.fields().len() != 1 {
        return Err(DataFusionError::Plan(
            "native row count expects one count column".into(),
        ));
    }
    let array = count_array(count, schema.field(0).data_type())?;
    RecordBatch::try_new(schema, vec![array])
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))
}

fn count_array(count: u64, data_type: &DataType) -> Result<ArrayRef> {
    match data_type {
        DataType::Int64 => Ok(Arc::new(Int64Array::from(vec![i64::try_from(count)
            .map_err(|_| DataFusionError::Plan("native COUNT result exceeds Int64".into()))?]))
            as ArrayRef),
        DataType::UInt64 => Ok(Arc::new(UInt64Array::from(vec![count])) as ArrayRef),
        other => Err(DataFusionError::Plan(format!(
            "unsupported native COUNT output type {other:?}"
        ))),
    }
}
