use std::{any::Any, path::PathBuf, sync::Arc};

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use async_trait::async_trait;
use cove_map::{
    projected_record_batches_from_cove_o_bytes_with_catalog, ProjectionBatchOptions,
    ProjectionDescriptor, ProjectionFilter,
};
use datafusion::{
    catalog::{Session, TableProvider},
    common::{stats::Precision, DataFusionError, Result, Statistics},
    execution::{SendableRecordBatchStream, TaskContext},
    logical_expr::{Expr, TableProviderFilterPushDown, TableType},
    physical_expr::EquivalenceProperties,
    physical_plan::{
        execution_plan::{Boundedness, EmissionType},
        memory::MemoryStream,
        metrics::{ExecutionPlanMetricsSet, MetricsSet},
        DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
    },
};

use crate::{adapter_v53::metrics::CoveFileMetrics, range_reader::LocalFileRangeReader};

mod filters;
mod loading;

use self::{
    filters::{
        classify_projection_filter, classify_projection_filters, merged_projection_columns,
        project_batch_columns, projected_schema,
    },
    loading::{
        decode_projection_arrow, load_projection_arrow, load_projection_bytes_via_ranges,
        InstrumentedProjectionRangeReader,
    },
};

#[derive(Debug, Clone)]
pub struct CoveProjectionTableProvider {
    object_path: PathBuf,
    object_len: u64,
    mapping_path: Option<PathBuf>,
    projection: ProjectionDescriptor,
    schema: SchemaRef,
    row_count: Option<usize>,
}

impl CoveProjectionTableProvider {
    pub fn try_new(
        object_path: PathBuf,
        mapping_path: Option<PathBuf>,
        projection: ProjectionDescriptor,
    ) -> Result<Self> {
        let bytes = load_projection_arrow(&object_path, mapping_path.as_deref(), &projection)?;
        let (schema, batches) = decode_projection_arrow(&bytes)?;
        let row_count = Some(batches.iter().map(RecordBatch::num_rows).sum());
        let object_len = std::fs::metadata(&object_path)
            .map_err(|err| {
                DataFusionError::Execution(format!(
                    "cannot stat mapped COVE-O {}: {err}",
                    object_path.display()
                ))
            })?
            .len();
        Ok(Self {
            object_path,
            object_len,
            mapping_path,
            projection,
            schema,
            row_count,
        })
    }

    pub fn projection(&self) -> &ProjectionDescriptor {
        &self.projection
    }

    fn table_statistics(&self) -> Statistics {
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        if let Some(row_count) = self.row_count {
            statistics.num_rows = Precision::Exact(row_count);
        }
        statistics.calculate_total_byte_size(self.schema.as_ref());
        statistics
    }
}

#[async_trait]
impl TableProvider for CoveProjectionTableProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let output_projection = projection.cloned();
        let output_columns = output_projection.as_ref().map(|projection| {
            projection
                .iter()
                .map(|&index| self.schema.field(index).name().to_string())
                .collect::<Vec<_>>()
        });
        let pushed_filters = classify_projection_filters(&self.schema, filters)?;
        let execution_columns =
            merged_projection_columns(output_columns.as_deref(), &pushed_filters);
        let output_schema = projected_schema(&self.schema, output_projection.as_deref())?;
        CoveProjectionExec::try_new(
            self.object_path.clone(),
            self.object_len,
            self.mapping_path.clone(),
            self.projection.clone(),
            output_schema,
            self.row_count,
            output_columns,
            execution_columns,
            pushed_filters,
            limit,
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        Ok(filters
            .iter()
            .map(|filter| {
                if classify_projection_filter(&self.schema, filter).is_some() {
                    TableProviderFilterPushDown::Exact
                } else {
                    TableProviderFilterPushDown::Unsupported
                }
            })
            .collect())
    }

    fn statistics(&self) -> Option<Statistics> {
        Some(self.table_statistics())
    }
}

#[derive(Debug)]
struct CoveProjectionExec {
    object_path: PathBuf,
    object_len: u64,
    mapping_path: Option<PathBuf>,
    projection: ProjectionDescriptor,
    schema: SchemaRef,
    row_count: Option<usize>,
    output_columns: Option<Vec<String>>,
    execution_columns: Option<Vec<String>>,
    pushed_filters: Vec<ProjectionFilter>,
    limit: Option<usize>,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl CoveProjectionExec {
    fn try_new(
        object_path: PathBuf,
        object_len: u64,
        mapping_path: Option<PathBuf>,
        projection: ProjectionDescriptor,
        schema: SchemaRef,
        row_count: Option<usize>,
        output_columns: Option<Vec<String>>,
        execution_columns: Option<Vec<String>>,
        pushed_filters: Vec<ProjectionFilter>,
        limit: Option<usize>,
    ) -> Result<Self> {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Ok(Self {
            object_path,
            object_len,
            mapping_path,
            projection,
            schema,
            row_count,
            output_columns,
            execution_columns,
            pushed_filters,
            limit,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }

    fn partition_stats(&self) -> Statistics {
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        if let Some(row_count) = self.row_count {
            let row_count = self.limit.map_or(row_count, |limit| row_count.min(limit));
            statistics.num_rows = Precision::Exact(row_count);
        }
        statistics.calculate_total_byte_size(self.schema.as_ref());
        statistics
    }
}

impl DisplayAs for CoveProjectionExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => write!(
                f,
                "CoveProjectionExec: projection={}, object={}, limit={:?}",
                self.projection.projection_id,
                self.object_path.display(),
                self.limit
            ),
            DisplayFormatType::TreeRender => write!(f, "CoveProjectionExec"),
        }
    }
}

impl ExecutionPlan for CoveProjectionExec {
    fn name(&self) -> &str {
        "CoveProjectionExec"
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
                "CoveProjectionExec is a leaf execution plan".into(),
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
                "CoveProjectionExec has one partition, got partition {partition}"
            )));
        }
        let metrics = CoveFileMetrics::new(&self.metrics, partition);
        metrics.files_opened.add(1);
        metrics.files_considered.add(1);
        metrics.scan_tasks.add(1);
        metrics.scan_partitions.add(1);
        metrics.range_plan_sparse.add(1);

        let reader = InstrumentedProjectionRangeReader::new(
            LocalFileRangeReader::new(&self.object_path),
            metrics.clone(),
        );
        let projection_options = ProjectionBatchOptions {
            max_rows: self.limit,
            output_columns: self.execution_columns.clone(),
            pushed_filters: self.pushed_filters.clone(),
            batch_size: None,
        };
        let loaded = load_projection_bytes_via_ranges(
            &reader,
            self.object_len,
            self.mapping_path.as_deref(),
            &self.projection.projection_id,
            &projection_options,
        )
        .map_err(|err| {
            DataFusionError::Execution(format!(
                "cannot range-read mapped COVE-O {}: {err}",
                self.object_path.display()
            ))
        })?;
        let batches = projected_record_batches_from_cove_o_bytes_with_catalog(
            &loaded.bytes,
            self.mapping_path.as_deref(),
            &loaded.projection_catalog,
            &self.projection.projection_id,
            &projection_options,
        )
        .map_err(|err| {
            DataFusionError::Execution(format!(
                "cannot load Arrow projection '{}' from {}: {err}",
                self.projection.projection_id,
                self.object_path.display()
            ))
        })?;
        let batches = batches
            .into_iter()
            .map(|batch| project_batch_columns(batch, self.output_columns.as_deref()))
            .collect::<Result<Vec<_>>>()?;
        let emitted_rows = batches.iter().map(RecordBatch::num_rows).sum::<usize>();
        metrics.rows_materialized.add(emitted_rows);
        metrics.rows_selected.add(emitted_rows);
        Ok(Box::pin(MemoryStream::try_new(
            batches,
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
                    "CoveProjectionExec has one partition, got partition {partition}"
                )));
            }
        }
        Ok(self.partition_stats())
    }
}
