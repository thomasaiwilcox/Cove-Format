#[cfg(feature = "covi")]
use std::collections::BTreeSet;
use std::{
    any::Any,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use async_trait::async_trait;
#[cfg(feature = "covi")]
use cove_core::{
    checksum, constants::DigestAlgorithm, digest::compute_digest, postscript::CovePostscriptV1,
    types::logical_type_from_name,
};
#[cfg(feature = "covi")]
use cove_index::CoviArtifactV2;
#[cfg(feature = "covi")]
use cove_index::{
    build::projection_column_lookup_key_for_json_value,
    execution::{
        CoviLookupComparatorContextV2, CoviLookupKeyV2, CoviLookupOpV2, CoviLookupRequestV2,
        CoviLookupTargetV2, CoviValidationContextV2, ValidatedCoviArtifactV2,
    },
};
use cove_map::{
    projected_record_batches_from_cove_o_bytes_with_catalog, projection_arrow_schema,
    ProjectionBatchOptions, ProjectionCandidateRows, ProjectionDescriptor, ProjectionFilter,
};
#[cfg(feature = "covi")]
use cove_map::{projection_covi_filter_plan, ProjectionFilterLiteral, ProjectionFilterOp};
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
#[cfg(feature = "covi")]
use serde_json::Value as JsonValue;

use crate::{adapter_v53::metrics::CoveFileMetrics, range_reader::LocalFileRangeReader};

mod filters;
mod loading;

pub use self::filters::{
    classify_projection_filter, classify_projection_filters_report,
    ProjectionFilterClassificationReport,
};

use self::{
    filters::{
        classify_projection_filters, merged_projection_columns, project_batch_columns,
        projected_schema,
    },
    loading::{load_projection_bytes_via_ranges, InstrumentedProjectionRangeReader},
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
        let schema = projection_arrow_schema(&projection).map_err(|err| {
            DataFusionError::Execution(format!(
                "cannot derive Arrow schema for projection '{}' from metadata: {err}",
                projection.projection_id
            ))
        })?;
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
            row_count: None,
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
        let candidate_projection_rows = projection_covi_candidate_rows(
            &self.object_path,
            &self.projection,
            &self.pushed_filters,
            &metrics,
        );

        let reader = InstrumentedProjectionRangeReader::new(
            LocalFileRangeReader::new(&self.object_path),
            metrics.clone(),
        );
        let projection_options = ProjectionBatchOptions {
            max_rows: self.limit,
            output_columns: self.execution_columns.clone(),
            pushed_filters: self.pushed_filters.clone(),
            candidate_projection_rows,
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

#[cfg(feature = "covi")]
fn projection_covi_candidate_rows(
    object_path: &Path,
    projection: &ProjectionDescriptor,
    filters: &[ProjectionFilter],
    metrics: &CoveFileMetrics,
) -> Option<ProjectionCandidateRows> {
    let plan = projection_covi_filter_plan(projection, filters);
    metrics.exact_predicates.add(plan.lookups.len());
    metrics
        .residual_predicates
        .add(plan.unsupported_filters.len());
    let sidecar = load_projection_covi_sidecar(object_path, metrics)?;
    if plan.lookups.is_empty() {
        return None;
    }
    let mut selected: Option<BTreeSet<u64>> = None;
    for lookup in plan.lookups {
        let Some(request) = projection_covi_lookup_request(&lookup) else {
            metrics.lookup_index_misses.add(1);
            continue;
        };
        match sidecar.lookup(&request) {
            Ok(candidates) => {
                metrics.lookup_index_hits.add(1);
                let ordinals = projection_row_ordinals_from_ranges(&candidates.row_ranges);
                metrics.index_rows_selected.add(ordinals.len());
                selected = Some(match selected.take() {
                    Some(existing) => existing.intersection(&ordinals).copied().collect(),
                    None => ordinals,
                });
            }
            Err(_) => {
                metrics.lookup_index_misses.add(1);
                metrics.index_fallbacks.add(1);
            }
        }
    }
    selected.map(ProjectionCandidateRows::from_ordinals)
}

#[cfg(feature = "covi")]
fn load_projection_covi_sidecar(
    object_path: &Path,
    metrics: &CoveFileMetrics,
) -> Option<ValidatedCoviArtifactV2> {
    let Some(covi_path) = discover_projection_columns_covi_path(object_path) else {
        metrics.sidecar_index_fallbacks.add(1);
        return None;
    };
    let Ok(object_bytes) = std::fs::read(object_path) else {
        metrics.covi_sidecars_ignored.add(1);
        return None;
    };
    let Ok(covi_bytes) = std::fs::read(&covi_path) else {
        metrics.covi_sidecars_ignored.add(1);
        return None;
    };
    let Ok(postscript) = CovePostscriptV1::parse_from_tail(&object_bytes) else {
        metrics.covi_sidecars_ignored.add(1);
        return None;
    };
    let snapshot_id = {
        let mut snapshot_id = [0u8; 16];
        snapshot_id[0..4].copy_from_slice(&checksum::crc32c(&object_bytes).to_le_bytes());
        snapshot_id[4..8].copy_from_slice(&postscript.footer.crc32c.to_le_bytes());
        snapshot_id[8..16].copy_from_slice(&(object_bytes.len() as u64).to_le_bytes());
        snapshot_id
    };
    let digest = compute_digest(DigestAlgorithm::Sha256, &object_bytes).ok();
    let context = CoviValidationContextV2::for_file(
        cove_core::header::CoveHeaderV1::parse(
            &object_bytes[..object_bytes.len().min(cove_core::header::HEADER_SIZE)],
        )
        .map(|header| header.file_id)
        .unwrap_or([0u8; 16]),
        object_bytes.len() as u64,
        postscript.footer.crc32c,
    )
    .with_dataset_id(
        cove_core::header::CoveHeaderV1::parse(
            &object_bytes[..object_bytes.len().min(cove_core::header::HEADER_SIZE)],
        )
        .map(|header| header.file_id)
        .unwrap_or([0u8; 16]),
    )
    .with_snapshot_id(snapshot_id)
    .with_file_code_keys(true);
    let context = if let Some(digest) = digest {
        context.with_file_digest(DigestAlgorithm::Sha256, digest)
    } else {
        context
    };
    let parsed = CoviArtifactV2::parse(&covi_bytes).ok();
    match ValidatedCoviArtifactV2::parse_and_validate(&covi_bytes, context) {
        Ok(validated) => {
            metrics.covi_sidecars_loaded.add(1);
            if let Some(parsed) = parsed {
                metrics.inverted_index_hits.add(parsed.index_roots.len());
                metrics.morsels_considered.add(
                    parsed
                        .index_roots
                        .iter()
                        .map(|root| root.value_count as usize)
                        .sum::<usize>(),
                );
            }
            Some(validated)
        }
        Err(_) => {
            metrics.covi_sidecars_stale.add(1);
            metrics.sidecar_index_fallbacks.add(1);
            None
        }
    }
}

#[cfg(not(feature = "covi"))]
fn projection_covi_candidate_rows(
    _object_path: &Path,
    _projection: &ProjectionDescriptor,
    _filters: &[ProjectionFilter],
    metrics: &CoveFileMetrics,
) -> Option<ProjectionCandidateRows> {
    metrics.sidecar_index_fallbacks.add(1);
    None
}

#[cfg(feature = "covi")]
fn discover_projection_columns_covi_path(object_path: &Path) -> Option<PathBuf> {
    let bundle = object_path
        .parent()?
        .join("indexes")
        .join("projection_columns.covi");
    if bundle.is_file() {
        return Some(bundle);
    }
    let appended = PathBuf::from(format!("{}.projection_columns.covi", object_path.display()));
    if appended.is_file() {
        return Some(appended);
    }
    let sibling = object_path.with_extension("projection_columns.covi");
    if sibling.is_file() {
        return Some(sibling);
    }
    let appended = PathBuf::from(format!("{}.covi", object_path.display()));
    if appended.is_file() {
        return Some(appended);
    }
    let replaced = object_path.with_extension("covi");
    if replaced.is_file() {
        return Some(replaced);
    }
    let bundle = object_path
        .parent()?
        .join("indexes")
        .join("object_properties.covi");
    bundle.is_file().then_some(bundle)
}

#[cfg(feature = "covi")]
fn projection_covi_lookup_request(
    lookup: &cove_map::ProjectionCoviFilterLookup,
) -> Option<CoviLookupRequestV2> {
    let logical_type = logical_type_from_name(&lookup.logical_type).ok()?;
    let target = CoviLookupTargetV2::ProjectionColumn {
        table_id: lookup.projection_table_id,
        column_id: lookup.projection_column_id,
    };
    match &lookup.filter {
        ProjectionFilter::Compare { op, literal, .. } => {
            let key = projection_filter_lookup_key(literal, logical_type)?;
            match op {
                ProjectionFilterOp::Eq => Some(CoviLookupRequestV2::eq_target(target, key)),
                ProjectionFilterOp::Lt
                | ProjectionFilterOp::LtEq
                | ProjectionFilterOp::Gt
                | ProjectionFilterOp::GtEq => Some(projection_covi_range_request(
                    target,
                    logical_type,
                    *op,
                    key,
                )),
                ProjectionFilterOp::Ne => None,
            }
        }
        ProjectionFilter::InList { literals, .. } => {
            let keys = literals
                .iter()
                .map(|literal| projection_filter_lookup_key(literal, logical_type))
                .collect::<Option<Vec<_>>>()?;
            Some(CoviLookupRequestV2::membership_target(target, keys))
        }
        ProjectionFilter::IsNull { .. } => None,
    }
}

#[cfg(feature = "covi")]
fn projection_covi_range_request(
    target: CoviLookupTargetV2,
    logical_type: cove_core::constants::CoveLogicalType,
    op: ProjectionFilterOp,
    key: CoviLookupKeyV2,
) -> CoviLookupRequestV2 {
    let (lower_key, upper_key, lower_inclusive, upper_inclusive) = match op {
        ProjectionFilterOp::Lt => (
            projection_covi_min_key(logical_type),
            Some(key),
            true,
            false,
        ),
        ProjectionFilterOp::LtEq => (projection_covi_min_key(logical_type), Some(key), true, true),
        ProjectionFilterOp::Gt => (key, None, false, true),
        ProjectionFilterOp::GtEq => (key, None, true, true),
        ProjectionFilterOp::Eq | ProjectionFilterOp::Ne => unreachable!(),
    };
    CoviLookupRequestV2 {
        table_id: match target {
            CoviLookupTargetV2::ProjectionColumn { table_id, .. } => table_id,
            _ => u32::MAX,
        },
        column_id: match target {
            CoviLookupTargetV2::ProjectionColumn { column_id, .. } => column_id,
            _ => u32::MAX,
        },
        target,
        op: CoviLookupOpV2::Range {
            lower_inclusive,
            upper_inclusive,
        },
        lower_key,
        upper_key,
        membership_keys: Vec::new(),
        logical_type: Some(logical_type),
        comparator_context: CoviLookupComparatorContextV2::default(),
        require_exact: true,
    }
}

#[cfg(feature = "covi")]
fn projection_covi_min_key(logical_type: cove_core::constants::CoveLogicalType) -> CoviLookupKeyV2 {
    use cove_core::constants::CoveLogicalType;
    let code = match logical_type {
        CoveLogicalType::Int8 => i8::MIN as u8 as u64,
        CoveLogicalType::Int16 => i16::MIN as u16 as u64,
        CoveLogicalType::Int32 | CoveLogicalType::DateDays => i32::MIN as u32 as u64,
        CoveLogicalType::Int64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => i64::MIN as u64,
        CoveLogicalType::Float32 => f32::NEG_INFINITY.to_bits() as u64,
        CoveLogicalType::Float64 => f64::NEG_INFINITY.to_bits(),
        _ => 0,
    };
    CoviLookupKeyV2::NumCode(code)
}

#[cfg(feature = "covi")]
fn projection_filter_lookup_key(
    literal: &ProjectionFilterLiteral,
    logical_type: cove_core::constants::CoveLogicalType,
) -> Option<CoviLookupKeyV2> {
    let value = match literal {
        ProjectionFilterLiteral::Null => return None,
        ProjectionFilterLiteral::Boolean(value) => JsonValue::Bool(*value),
        ProjectionFilterLiteral::Int64(value) => JsonValue::Number((*value).into()),
        ProjectionFilterLiteral::UInt64(value) => JsonValue::Number((*value).into()),
        ProjectionFilterLiteral::Float64(value) => {
            JsonValue::Number(serde_json::Number::from_f64(*value)?)
        }
        ProjectionFilterLiteral::Utf8(value) => JsonValue::String(value.clone()),
    };
    projection_column_lookup_key_for_json_value(&value, logical_type)
        .ok()
        .flatten()
}

#[cfg(feature = "covi")]
fn projection_row_ordinals_from_ranges(
    ranges: &[cove_index::CoviRowRangePostingV2],
) -> BTreeSet<u64> {
    let mut ordinals = BTreeSet::new();
    for range in ranges {
        for offset in 0..range.row_count {
            ordinals.insert(range.row_start.saturating_add(offset));
        }
    }
    ordinals
}
