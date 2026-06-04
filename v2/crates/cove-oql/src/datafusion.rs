use std::{any::Any, collections::BTreeMap, path::Path, sync::Arc};

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use arrow_schema::{DataType, SchemaRef};
use async_trait::async_trait;
use cove_datafusion::{
    adapter_v53::table_provider::CoveTableProvider, dataset_state::DatasetState,
    projection_provider, register,
};
use cove_map::{
    projected_record_batches_from_cove_o_bytes, ProjectionBatchOptions, ProjectionFilter,
    ProjectionFilterLiteral, ProjectionFilterOp,
};
use datafusion::datasource::MemTable;
use datafusion::logical_expr::Expr;
use datafusion::{
    catalog::{Session, TableProvider},
    common::{stats::Precision, DataFusionError, Result, Statistics},
    execution::{context::SessionContext, SendableRecordBatchStream, TaskContext},
    logical_expr::{TableProviderFilterPushDown, TableType},
    physical_expr::EquivalenceProperties,
    physical_plan::{
        execution_plan::{Boundedness, EmissionType},
        memory::MemoryStream,
        metrics::{Count, ExecutionPlanMetricsSet, MetricBuilder, MetricsSet},
        DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    build_physical_plan, execute_manifest_physical_planned_query,
    execute_manifest_planned_query_retained, execute_physical_planned_query,
    execute_planned_query_retained,
    predicate::{
        classify_predicate, classify_predicate_for_dataset, LogicalPredicateForm,
        PredicateProofState, RepresentationClass,
    },
    AstCompareOp, AstLiteral, CodeDomainId, CoveOqlExecutionResult, CoveOqlRetainedInput,
    CoveOqlRetainedManifestMember, ExecutionAuthoritySource, ExecutionOptions,
    KernelExecutionOptions, PhysicalPlanOptions, PlannedQuery, ResolvedExpr, ResolvedLiteral,
    ResolvedLiteralValue, ResolvedPath, ResolvedPathRootKind, ResolvedPredicate, ResolvedRoot,
    ResolvedSelectItem,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFusionOqlPushdownReport {
    pub report_version: String,
    pub projection_id: String,
    pub root_kind: String,
    pub root_id: Option<String>,
    pub supported_filter_count: usize,
    pub residual_filter_count: usize,
    pub received_filters: Vec<String>,
    pub filter_outcomes: Vec<DataFusionOqlFilterOutcome>,
    pub pushed_filters: Vec<String>,
    pub trusted_filters: Vec<String>,
    pub residual_filters: Vec<String>,
    pub rejected_filters: Vec<String>,
    pub lowered_oql_predicates: Vec<String>,
    pub proof_states: Vec<PredicateProofState>,
    pub decode_boundaries: Vec<String>,
    pub trusted: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFusionOqlFilterOutcome {
    pub received_filter: String,
    pub outcome: DataFusionOqlFilterOutcomeKind,
    pub lowered_oql_predicates: Vec<String>,
    pub proof_state: PredicateProofState,
    pub trusted: bool,
    pub diagnostic_code: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataFusionOqlFilterOutcomeKind {
    TrustedExact,
    PushedInexact,
    ResidualRejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFusionOqlProviderReport {
    pub report_version: String,
    pub provider_kind: String,
    pub root_kind: String,
    pub root_id: Option<String>,
    pub dataset_file_count: usize,
    pub planned_output_mode: crate::CoveOqlOutputMode,
    pub materialized_oql_before_registration: bool,
    pub residual_verification: bool,
    pub scan_residual_verification_required: bool,
    pub scan_filter_pushdown_supported: bool,
    pub scan_projection_pushdown_supported: bool,
    pub residual_filter_authority: String,
    pub oql_scan_authority_source: ExecutionAuthoritySource,
    pub oql_scan_materialized_fallback: bool,
    pub oql_scan_residual_required: bool,
    pub oql_scan_compared_with_materialized: bool,
    pub scan_execution_policy: String,
    pub limit_pushdown_policy: String,
    pub unhandled_residuals: Vec<String>,
    pub row_count: usize,
    pub batch_count: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFusionOqlScanNegotiationReport {
    pub report_version: String,
    pub provider_kind: String,
    pub root_kind: String,
    pub root_id: Option<String>,
    pub dataset_file_count: usize,
    pub received_projection_columns: Option<Vec<String>>,
    pub projection_pushdown_supported: bool,
    pub projection_pushed_to_oql: bool,
    pub pushed_projection_columns: Vec<String>,
    pub received_filters: Vec<String>,
    pub filter_outcomes: Vec<DataFusionOqlFilterOutcome>,
    pub pushed_filters: Vec<String>,
    pub trusted_filters: Vec<String>,
    pub residual_filters: Vec<String>,
    pub rejected_filters: Vec<String>,
    pub lowered_oql_predicates: Vec<String>,
    pub proof_states: Vec<PredicateProofState>,
    pub filters_trusted_exact: bool,
    pub received_limit: Option<usize>,
    pub limit_pushed_to_oql: bool,
    pub pushed_limit: Option<usize>,
    pub residual_filter_authority: String,
    pub scan_execution_policy: String,
    pub unhandled_residuals: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug)]
pub struct OqlTableProvider {
    bytes: Arc<Vec<u8>>,
    planned: PlannedQuery,
    options: ExecutionOptions,
    schema: SchemaRef,
    schema_probe_rows: usize,
    schema_probe_batches: usize,
    schema_probe_authority_source: ExecutionAuthoritySource,
    schema_probe_materialized_fallback: bool,
    schema_probe_residual_required: bool,
    schema_probe_compared_with_materialized: bool,
}

impl OqlTableProvider {
    pub fn try_new(
        bytes: Arc<Vec<u8>>,
        planned: PlannedQuery,
        options: ExecutionOptions,
    ) -> Result<Self> {
        validate_oql_table_provider_scope(&planned)?;
        let probe = execute_oql_arrow(bytes.as_slice(), &planned, &options)?;
        let schema = probe
            .batches
            .first()
            .map(|batch| batch.schema())
            .ok_or_else(|| {
                DataFusionError::Execution(
                    "OQL DataFusion provider could not infer an Arrow schema from planned output"
                        .into(),
                )
            })?;
        Ok(Self {
            bytes,
            planned,
            options,
            schema,
            schema_probe_rows: probe.row_count,
            schema_probe_batches: probe.batches.len(),
            schema_probe_authority_source: probe.authority_source,
            schema_probe_materialized_fallback: probe.materialized_fallback,
            schema_probe_residual_required: probe.residual_required,
            schema_probe_compared_with_materialized: probe.compared_with_materialized,
        })
    }

    pub fn report(&self) -> DataFusionOqlProviderReport {
        let scan_filter_pushdown_supported = can_apply_datafusion_scan_filters(&self.planned);
        let scan_projection_pushdown_supported =
            can_apply_datafusion_scan_projection(&self.planned);
        DataFusionOqlProviderReport {
            report_version: crate::DATAFUSION_OQL_REPORT_VERSION.into(),
            provider_kind: "oql_table_provider".into(),
            root_kind: resolved_root_name(&self.planned.resolved.root).into(),
            root_id: datafusion_pushdown_report_root_id(&self.planned.resolved.root),
            dataset_file_count: self.planned.resolved.operation_context.dataset.files.len(),
            planned_output_mode: self.planned.resolved.output_mode.clone(),
            materialized_oql_before_registration: false,
            residual_verification: true,
            scan_residual_verification_required: self.schema_probe_residual_required,
            scan_filter_pushdown_supported,
            scan_projection_pushdown_supported,
            residual_filter_authority:
                "DataFusion retains SQL filters as residuals unless Cove-OQL reports TrustedExact and the scan projection contract is unambiguous; inexact pushdown or multi-column row-root projection guards still require DataFusion residual verification".into(),
            oql_scan_authority_source: self.schema_probe_authority_source,
            oql_scan_materialized_fallback: self.schema_probe_materialized_fallback,
            oql_scan_residual_required: self.schema_probe_residual_required,
            oql_scan_compared_with_materialized: self.schema_probe_compared_with_materialized,
            scan_execution_policy: provider_scan_execution_policy(&self.planned, false).into(),
            limit_pushdown_policy: if scan_filter_pushdown_supported {
                "scan limit is pushed when no DataFusion filter is present or every pushed filter is proven trusted exact; scans with residual or inexact filters leave limit to DataFusion residual order".into()
            } else {
                "scan limit may be pushed for filterless scans; residual DataFusion filters or non-lowerable OQL operators keep limit outside Cove-OQL when they can affect row order/count".into()
            },
            unhandled_residuals: provider_residuals(&self.planned),
            row_count: self.schema_probe_rows,
            batch_count: self.schema_probe_batches,
            notes: vec![
                "planned OQL semantics execute inside the DataFusion provider scan path".into(),
                "direct projection-root filters can be lowered as TrustedExact when the COVE-MAP projection-column proof is exact; direct object/association/evidence filters can be lowered as TrustedExact only when OQL predicate proofs are exact, but multi-column row-root table scans report DataFusion Inexact support to preserve residual projection semantics".into(),
                "planned OQL physical execution is attempted inside the provider scan; materialized OQL remains the fallback authority for residual predicates, unproven temporal shapes, visibility, and redaction".into(),
            ],
        }
    }

    pub fn filter_pushdown_report(&self, filters: &[Expr]) -> Result<DataFusionOqlPushdownReport> {
        datafusion_pushdown_report_for_plan(&self.schema, filters, &self.planned)
    }

    pub fn scan_negotiation_report(
        &self,
        projection: Option<&[usize]>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<DataFusionOqlScanNegotiationReport> {
        datafusion_scan_negotiation_report(
            "oql_table_provider",
            &self.schema,
            &self.planned,
            projection,
            filters,
            limit,
            false,
        )
    }
}

#[derive(Debug)]
pub struct ManifestOqlTableProvider {
    members: Vec<CoveOqlRetainedManifestMember>,
    planned: PlannedQuery,
    options: ExecutionOptions,
    schema: SchemaRef,
    schema_probe_rows: usize,
    schema_probe_batches: usize,
    schema_probe_authority_source: ExecutionAuthoritySource,
    schema_probe_materialized_fallback: bool,
    schema_probe_residual_required: bool,
    schema_probe_compared_with_materialized: bool,
}

impl ManifestOqlTableProvider {
    pub fn try_new(
        members: Vec<CoveOqlRetainedManifestMember>,
        planned: PlannedQuery,
        options: ExecutionOptions,
    ) -> Result<Self> {
        validate_manifest_oql_table_provider_scope(&planned, &members)?;
        let probe = execute_manifest_oql_arrow(&members, &planned, &options)?;
        let schema = probe
            .batches
            .first()
            .map(|batch| batch.schema())
            .ok_or_else(|| {
                DataFusionError::Execution(
                    "manifest OQL DataFusion provider could not infer an Arrow schema from planned output"
                        .into(),
                )
            })?;
        Ok(Self {
            members,
            planned,
            options,
            schema,
            schema_probe_rows: probe.row_count,
            schema_probe_batches: probe.batches.len(),
            schema_probe_authority_source: probe.authority_source,
            schema_probe_materialized_fallback: probe.materialized_fallback,
            schema_probe_residual_required: probe.residual_required,
            schema_probe_compared_with_materialized: probe.compared_with_materialized,
        })
    }

    pub fn report(&self) -> DataFusionOqlProviderReport {
        let scan_filter_pushdown_supported = can_apply_datafusion_scan_filters(&self.planned);
        let scan_projection_pushdown_supported =
            can_apply_datafusion_scan_projection(&self.planned);
        DataFusionOqlProviderReport {
            report_version: crate::DATAFUSION_OQL_REPORT_VERSION.into(),
            provider_kind: "manifest_oql_table_provider".into(),
            root_kind: resolved_root_name(&self.planned.resolved.root).into(),
            root_id: datafusion_pushdown_report_root_id(&self.planned.resolved.root),
            dataset_file_count: self.planned.resolved.operation_context.dataset.files.len(),
            planned_output_mode: self.planned.resolved.output_mode.clone(),
            materialized_oql_before_registration: false,
            residual_verification: true,
            scan_residual_verification_required: self.schema_probe_residual_required,
            scan_filter_pushdown_supported,
            scan_projection_pushdown_supported,
            residual_filter_authority:
                "manifest DataFusion filters may be lowered into the manifest physical OQL kernel when an exact shape is proven, otherwise they execute through the materialized OQL oracle; inexact or rejected filters remain DataFusion residuals, and coded cross-file comparisons remain disabled without exact bridge proofs".into(),
            oql_scan_authority_source: self.schema_probe_authority_source,
            oql_scan_materialized_fallback: self.schema_probe_materialized_fallback,
            oql_scan_residual_required: self.schema_probe_residual_required,
            oql_scan_compared_with_materialized: self.schema_probe_compared_with_materialized,
            scan_execution_policy: provider_scan_execution_policy(&self.planned, true).into(),
            limit_pushdown_policy: if scan_filter_pushdown_supported {
                "scan limit is pushed inside manifest OQL only when no DataFusion filter is present or every pushed filter is proven trusted exact; scans with residual or inexact filters leave limit to DataFusion residual order".into()
            } else {
                "scan limit may be pushed inside manifest OQL for filterless scans; residual DataFusion filters or non-lowerable OQL operators keep limit outside manifest OQL when they can affect row order/count".into()
            },
            unhandled_residuals: provider_residuals(&self.planned),
            row_count: self.schema_probe_rows,
            batch_count: self.schema_probe_batches,
            notes: vec![
                "planned OQL semantics execute across validated COVM members inside the DataFusion provider scan path".into(),
                "materialized manifest OQL execution remains the fallback authority for cross-file ordering, visibility, redaction, association/evidence helper scope, and pagination when no exact manifest physical kernel path is selected".into(),
                "raw local codes from different files are never compared by this provider; coded pushdown requires exact manifest bridge proofs and a selected manifest physical kernel path".into(),
            ],
        }
    }

    pub fn filter_pushdown_report(&self, filters: &[Expr]) -> Result<DataFusionOqlPushdownReport> {
        datafusion_pushdown_report_for_plan(&self.schema, filters, &self.planned)
    }

    pub fn scan_negotiation_report(
        &self,
        projection: Option<&[usize]>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<DataFusionOqlScanNegotiationReport> {
        datafusion_scan_negotiation_report(
            "manifest_oql_table_provider",
            &self.schema,
            &self.planned,
            projection,
            filters,
            limit,
            true,
        )
    }
}

fn datafusion_scan_negotiation_report(
    provider_kind: &str,
    schema: &SchemaRef,
    planned: &PlannedQuery,
    projection: Option<&[usize]>,
    filters: &[Expr],
    limit: Option<usize>,
    manifest: bool,
) -> Result<DataFusionOqlScanNegotiationReport> {
    let filter_report = datafusion_pushdown_report_for_plan(schema, filters, planned)?;
    let scan_filters = classify_scan_projection_filters(schema, filters, planned)?;
    let projection_pushdown_supported = can_apply_datafusion_scan_projection(planned);
    let push_projection_to_oql =
        projection_pushdown_supported && (filters.is_empty() || filter_report.trusted);
    let received_projection_columns = projection
        .map(|projection| projection_column_names(schema, Some(projection)))
        .transpose()?;
    let pushed_projection_columns = if push_projection_to_oql {
        received_projection_columns.clone().unwrap_or_default()
    } else {
        Vec::new()
    };
    let projection_pushed_to_oql = push_projection_to_oql && !pushed_projection_columns.is_empty();
    let limit_pushed_to_oql = filters.is_empty() || filter_report.trusted;
    let mut unhandled_residuals = provider_residuals(planned);
    if !filter_report.residual_filters.is_empty() {
        unhandled_residuals.push(format!(
            "{} DataFusion filter(s) remain residual outside Cove-OQL pushdown",
            filter_report.residual_filters.len()
        ));
    }
    if !filter_report.rejected_filters.is_empty() {
        unhandled_residuals.push(format!(
            "{} DataFusion filter(s) were rejected for Cove-OQL pushdown",
            filter_report.rejected_filters.len()
        ));
    }
    if received_projection_columns.is_some() && !projection_pushed_to_oql {
        unhandled_residuals.push(
            "DataFusion scan projection remains outside Cove-OQL because filters are residual/inexact or the planned OQL shape cannot prove equivalent projection pushdown"
                .into(),
        );
    }
    if limit.is_some() && !limit_pushed_to_oql {
        unhandled_residuals.push(
            "DataFusion scan limit remains outside Cove-OQL because residual or inexact filters can affect row count/order"
                .into(),
        );
    }
    let residual_filter_authority = residual_filter_authority_label(&filter_report).into();
    Ok(DataFusionOqlScanNegotiationReport {
        report_version: crate::DATAFUSION_OQL_REPORT_VERSION.into(),
        provider_kind: provider_kind.into(),
        root_kind: resolved_root_name(&planned.resolved.root).into(),
        root_id: datafusion_pushdown_report_root_id(&planned.resolved.root),
        dataset_file_count: planned.resolved.operation_context.dataset.files.len(),
        received_projection_columns,
        projection_pushdown_supported,
        projection_pushed_to_oql,
        pushed_projection_columns,
        received_filters: filter_report.received_filters,
        filter_outcomes: filter_report.filter_outcomes,
        pushed_filters: filter_report.pushed_filters,
        trusted_filters: filter_report.trusted_filters,
        residual_filters: filter_report.residual_filters,
        rejected_filters: filter_report.rejected_filters,
        lowered_oql_predicates: filter_report.lowered_oql_predicates,
        proof_states: filter_report.proof_states,
        filters_trusted_exact: filter_report.trusted,
        received_limit: limit,
        limit_pushed_to_oql,
        pushed_limit: limit_pushed_to_oql.then_some(limit).flatten(),
        residual_filter_authority,
        scan_execution_policy: exec_scan_execution_policy(
            planned,
            manifest,
            projection_pushed_to_oql,
            !scan_filters.is_empty(),
        )
        .into(),
        unhandled_residuals,
        notes: vec![
            "scan negotiation report is computed with the same projection/filter/limit decisions used by the DataFusion TableProvider scan".into(),
            "projection and limit pushdown require either no DataFusion filters or filters proven TrustedExact under Cove-OQL predicate semantics".into(),
        ],
    })
}

fn validate_manifest_oql_table_provider_scope(
    planned: &PlannedQuery,
    members: &[CoveOqlRetainedManifestMember],
) -> Result<()> {
    if planned.resolved.output_mode != crate::CoveOqlOutputMode::DataFusionTableProvider {
        return Err(DataFusionError::Plan(
            "manifest OQL DataFusion provider requires DataFusionTableProvider output mode".into(),
        ));
    }
    let file_count = planned.resolved.operation_context.dataset.files.len();
    if file_count == 0 {
        return Err(DataFusionError::Plan(
            "manifest OQL DataFusion provider requires a resolved dataset scope".into(),
        ));
    }
    if file_count != members.len() {
        return Err(DataFusionError::Plan(format!(
            "manifest OQL DataFusion provider member count mismatch: resolved scope has {file_count}, provider received {}",
            members.len()
        )));
    }
    Ok(())
}

fn validate_oql_table_provider_scope(planned: &PlannedQuery) -> Result<()> {
    let file_count = planned.resolved.operation_context.dataset.files.len();
    if file_count <= 1 {
        return Ok(());
    }
    Err(DataFusionError::Plan(format!(
        "Cove-OQL OqlTableProvider accepts one retained COVE file; manifest-scoped plans with {file_count} members require manifest-member execution or a dedicated manifest DataFusion provider"
    )))
}

#[async_trait]
impl TableProvider for ManifestOqlTableProvider {
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
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let scan_filters = classify_scan_projection_filters(&self.schema, filters, &self.planned)?;
        let scan_report =
            datafusion_pushdown_report_for_plan(&self.schema, filters, &self.planned)?;
        let push_scan_projection_to_oql = can_apply_datafusion_scan_projection(&self.planned)
            && (filters.is_empty() || scan_report.trusted);
        let push_scan_limit_to_oql = filters.is_empty() || scan_report.trusted;
        let received_limit = limit;
        let limit = if push_scan_limit_to_oql { limit } else { None };
        ManifestOqlExec::try_new(
            self.members.clone(),
            self.planned.clone(),
            self.options.clone(),
            Arc::clone(&self.schema),
            projection.cloned(),
            push_scan_projection_to_oql,
            scan_filters,
            scan_report,
            self.schema_probe_authority_source,
            self.schema_probe_materialized_fallback,
            self.schema_probe_residual_required,
            self.schema_probe_compared_with_materialized,
            received_limit,
            limit,
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        filters
            .iter()
            .map(|filter| scan_filter_pushdown_support(&self.schema, filter, &self.planned))
            .collect()
    }

    fn statistics(&self) -> Option<Statistics> {
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        statistics.num_rows = Precision::Inexact(self.schema_probe_rows);
        statistics.calculate_total_byte_size(self.schema.as_ref());
        Some(statistics)
    }
}

#[async_trait]
impl TableProvider for OqlTableProvider {
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
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let scan_filters = classify_scan_projection_filters(&self.schema, filters, &self.planned)?;
        let scan_report =
            datafusion_pushdown_report_for_plan(&self.schema, filters, &self.planned)?;
        let push_scan_projection_to_oql = can_apply_datafusion_scan_projection(&self.planned)
            && (filters.is_empty() || scan_report.trusted);
        let push_scan_limit_to_oql = filters.is_empty() || scan_report.trusted;
        let received_limit = limit;
        let limit = if push_scan_limit_to_oql { limit } else { None };
        OqlExec::try_new(
            Arc::clone(&self.bytes),
            self.planned.clone(),
            self.options.clone(),
            Arc::clone(&self.schema),
            projection.cloned(),
            push_scan_projection_to_oql,
            scan_filters,
            scan_report,
            self.schema_probe_authority_source,
            self.schema_probe_materialized_fallback,
            self.schema_probe_residual_required,
            self.schema_probe_compared_with_materialized,
            received_limit,
            limit,
        )
        .map(|exec| Arc::new(exec) as Arc<dyn ExecutionPlan>)
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        filters
            .iter()
            .map(|filter| scan_filter_pushdown_support(&self.schema, filter, &self.planned))
            .collect()
    }

    fn statistics(&self) -> Option<Statistics> {
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        statistics.num_rows = Precision::Inexact(self.schema_probe_rows);
        statistics.calculate_total_byte_size(self.schema.as_ref());
        Some(statistics)
    }
}

#[derive(Debug)]
struct OqlExec {
    bytes: Arc<Vec<u8>>,
    planned: PlannedQuery,
    options: ExecutionOptions,
    base_schema: SchemaRef,
    schema: SchemaRef,
    projection: Option<Vec<usize>>,
    scan_projection_pushed_to_oql: bool,
    pushed_projection_columns: Vec<String>,
    scan_filters: Vec<ProjectionFilter>,
    scan_report: DataFusionOqlPushdownReport,
    schema_probe_authority_source: ExecutionAuthoritySource,
    schema_probe_materialized_fallback: bool,
    schema_probe_residual_required: bool,
    schema_probe_compared_with_materialized: bool,
    received_limit: Option<usize>,
    limit: Option<usize>,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl OqlExec {
    fn try_new(
        bytes: Arc<Vec<u8>>,
        planned: PlannedQuery,
        options: ExecutionOptions,
        base_schema: SchemaRef,
        projection: Option<Vec<usize>>,
        push_scan_projection_to_oql: bool,
        scan_filters: Vec<ProjectionFilter>,
        scan_report: DataFusionOqlPushdownReport,
        schema_probe_authority_source: ExecutionAuthoritySource,
        schema_probe_materialized_fallback: bool,
        schema_probe_residual_required: bool,
        schema_probe_compared_with_materialized: bool,
        received_limit: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Self> {
        let schema = projected_schema(&base_schema, projection.as_deref())?;
        let pushed_projection_columns = if push_scan_projection_to_oql {
            projection_column_names(&base_schema, projection.as_deref())?
        } else {
            Vec::new()
        };
        let scan_projection_pushed_to_oql = push_scan_projection_to_oql
            && projection.is_some()
            && !pushed_projection_columns.is_empty();
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Ok(Self {
            bytes,
            planned,
            options,
            base_schema,
            schema,
            projection,
            scan_projection_pushed_to_oql,
            pushed_projection_columns,
            scan_filters,
            scan_report,
            schema_probe_authority_source,
            schema_probe_materialized_fallback,
            schema_probe_residual_required,
            schema_probe_compared_with_materialized,
            received_limit,
            limit,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for OqlExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => write!(
                f,
                "OqlExec: root={}, projection={:?}, projection_pushed_to_oql={}, pushed_projection_columns={:?}, pushed_filters={}, trusted_filters={}, residual_filters={}, rejected_filters={}, received_filters={}, trusted={}, received_limit={:?}, limit_pushed_to_oql={}, limit={:?}, scan_execution_policy={}, residual_filter_authority={}, oql_scan_authority_probe={:?}, oql_scan_materialized_fallback={}, oql_scan_residual_required={}, oql_scan_compared_with_materialized={}",
                resolved_root_name(&self.planned.resolved.root),
                self.projection,
                self.scan_projection_pushed_to_oql,
                self.pushed_projection_columns,
                self.scan_report.pushed_filters.len(),
                self.scan_report.trusted_filters.len(),
                self.scan_report.residual_filters.len(),
                self.scan_report.rejected_filters.len(),
                self.scan_report.received_filters.len(),
                self.scan_report.trusted,
                self.received_limit,
                self.limit.is_some(),
                self.limit,
                exec_scan_execution_policy(
                    &self.planned,
                    false,
                    self.scan_projection_pushed_to_oql,
                    !self.scan_filters.is_empty()
                ),
                residual_filter_authority_label(&self.scan_report),
                self.schema_probe_authority_source,
                self.schema_probe_materialized_fallback,
                self.schema_probe_residual_required,
                self.schema_probe_compared_with_materialized
            ),
            DisplayFormatType::TreeRender => write!(f, "OqlExec"),
        }
    }
}

impl ExecutionPlan for OqlExec {
    fn name(&self) -> &str {
        "OqlExec"
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
                "OqlExec is a leaf execution plan".into(),
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
                "OqlExec has one partition, got partition {partition}"
            )));
        }
        let metrics = OqlExecMetrics::new(&self.metrics, partition);
        let pushed_projection = self
            .scan_projection_pushed_to_oql
            .then(|| self.projection.as_deref())
            .flatten();
        let arrow = execute_oql_arrow_with_scan_filters(
            self.bytes.as_slice(),
            &self.planned,
            &self.options,
            &self.base_schema,
            pushed_projection,
            &self.scan_filters,
            self.limit,
        )?;
        let project_after_oql = if self.scan_projection_pushed_to_oql {
            None
        } else {
            self.projection.as_deref()
        };
        let batches = project_and_limit_batches(arrow.batches, project_after_oql, self.limit)?;
        metrics.record(
            arrow.row_count,
            batches.iter().map(RecordBatch::num_rows).sum(),
        );
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
                    "OqlExec has one partition, got partition {partition}"
                )));
            }
        }
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        statistics.num_rows = Precision::Absent;
        statistics.calculate_total_byte_size(self.schema.as_ref());
        Ok(statistics)
    }
}

struct OqlExecMetrics {
    scans: Count,
    rows_from_oql: Count,
    rows_emitted: Count,
}

impl OqlExecMetrics {
    fn new(metrics: &ExecutionPlanMetricsSet, partition: usize) -> Self {
        Self {
            scans: MetricBuilder::new(metrics).counter("oql_scans", partition),
            rows_from_oql: MetricBuilder::new(metrics).counter("oql_rows_from_oql", partition),
            rows_emitted: MetricBuilder::new(metrics).counter("oql_rows_emitted", partition),
        }
    }

    fn record(&self, rows_from_oql: usize, rows_emitted: usize) {
        self.scans.add(1);
        self.rows_from_oql.add(rows_from_oql);
        self.rows_emitted.add(rows_emitted);
    }
}

#[derive(Debug)]
struct ManifestOqlExec {
    members: Vec<CoveOqlRetainedManifestMember>,
    planned: PlannedQuery,
    options: ExecutionOptions,
    base_schema: SchemaRef,
    schema: SchemaRef,
    projection: Option<Vec<usize>>,
    scan_projection_pushed_to_oql: bool,
    pushed_projection_columns: Vec<String>,
    scan_filters: Vec<ProjectionFilter>,
    scan_report: DataFusionOqlPushdownReport,
    schema_probe_authority_source: ExecutionAuthoritySource,
    schema_probe_materialized_fallback: bool,
    schema_probe_residual_required: bool,
    schema_probe_compared_with_materialized: bool,
    received_limit: Option<usize>,
    limit: Option<usize>,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}

impl ManifestOqlExec {
    fn try_new(
        members: Vec<CoveOqlRetainedManifestMember>,
        planned: PlannedQuery,
        options: ExecutionOptions,
        base_schema: SchemaRef,
        projection: Option<Vec<usize>>,
        push_scan_projection_to_oql: bool,
        scan_filters: Vec<ProjectionFilter>,
        scan_report: DataFusionOqlPushdownReport,
        schema_probe_authority_source: ExecutionAuthoritySource,
        schema_probe_materialized_fallback: bool,
        schema_probe_residual_required: bool,
        schema_probe_compared_with_materialized: bool,
        received_limit: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Self> {
        let schema = projected_schema(&base_schema, projection.as_deref())?;
        let pushed_projection_columns = if push_scan_projection_to_oql {
            projection_column_names(&base_schema, projection.as_deref())?
        } else {
            Vec::new()
        };
        let scan_projection_pushed_to_oql = push_scan_projection_to_oql
            && projection.is_some()
            && !pushed_projection_columns.is_empty();
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Ok(Self {
            members,
            planned,
            options,
            base_schema,
            schema,
            projection,
            scan_projection_pushed_to_oql,
            pushed_projection_columns,
            scan_filters,
            scan_report,
            schema_probe_authority_source,
            schema_probe_materialized_fallback,
            schema_probe_residual_required,
            schema_probe_compared_with_materialized,
            received_limit,
            limit,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        })
    }
}

impl DisplayAs for ManifestOqlExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => write!(
                f,
                "ManifestOqlExec: root={}, files={}, projection={:?}, projection_pushed_to_oql={}, pushed_projection_columns={:?}, pushed_filters={}, trusted_filters={}, residual_filters={}, rejected_filters={}, received_filters={}, trusted={}, received_limit={:?}, limit_pushed_to_oql={}, limit={:?}, scan_execution_policy={}, residual_filter_authority={}, oql_scan_authority_probe={:?}, oql_scan_materialized_fallback={}, oql_scan_residual_required={}, oql_scan_compared_with_materialized={}",
                resolved_root_name(&self.planned.resolved.root),
                self.members.len(),
                self.projection,
                self.scan_projection_pushed_to_oql,
                self.pushed_projection_columns,
                self.scan_report.pushed_filters.len(),
                self.scan_report.trusted_filters.len(),
                self.scan_report.residual_filters.len(),
                self.scan_report.rejected_filters.len(),
                self.scan_report.received_filters.len(),
                self.scan_report.trusted,
                self.received_limit,
                self.limit.is_some(),
                self.limit,
                exec_scan_execution_policy(
                    &self.planned,
                    true,
                    self.scan_projection_pushed_to_oql,
                    !self.scan_filters.is_empty()
                ),
                residual_filter_authority_label(&self.scan_report),
                self.schema_probe_authority_source,
                self.schema_probe_materialized_fallback,
                self.schema_probe_residual_required,
                self.schema_probe_compared_with_materialized
            ),
            DisplayFormatType::TreeRender => write!(f, "ManifestOqlExec"),
        }
    }
}

impl ExecutionPlan for ManifestOqlExec {
    fn name(&self) -> &str {
        "ManifestOqlExec"
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
                "ManifestOqlExec is a leaf execution plan".into(),
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
                "ManifestOqlExec has one partition, got partition {partition}"
            )));
        }
        let metrics = OqlExecMetrics::new(&self.metrics, partition);
        let pushed_projection = self
            .scan_projection_pushed_to_oql
            .then(|| self.projection.as_deref())
            .flatten();
        let arrow = execute_manifest_oql_arrow_with_scan_filters(
            &self.members,
            &self.planned,
            &self.options,
            &self.base_schema,
            pushed_projection,
            &self.scan_filters,
            self.limit,
        )?;
        let project_after_oql = if self.scan_projection_pushed_to_oql {
            None
        } else {
            self.projection.as_deref()
        };
        let batches = project_and_limit_batches(arrow.batches, project_after_oql, self.limit)?;
        metrics.record(
            arrow.row_count,
            batches.iter().map(RecordBatch::num_rows).sum(),
        );
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
                    "ManifestOqlExec has one partition, got partition {partition}"
                )));
            }
        }
        let mut statistics = Statistics::new_unknown(self.schema.as_ref());
        statistics.num_rows = Precision::Absent;
        statistics.calculate_total_byte_size(self.schema.as_ref());
        Ok(statistics)
    }
}

struct OqlArrowExecution {
    batches: Vec<RecordBatch>,
    row_count: usize,
    authority_source: ExecutionAuthoritySource,
    materialized_fallback: bool,
    residual_required: bool,
    compared_with_materialized: bool,
}

fn execute_manifest_oql_arrow_with_scan_filters(
    members: &[CoveOqlRetainedManifestMember],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    base_schema: &SchemaRef,
    pushed_projection: Option<&[usize]>,
    scan_filters: &[ProjectionFilter],
    limit: Option<usize>,
) -> Result<OqlArrowExecution> {
    if (scan_filters.is_empty() && pushed_projection.is_none())
        || !can_apply_datafusion_scan_filters(planned)
    {
        let limited = planned_with_scan_limit(planned, limit);
        return execute_manifest_oql_arrow(members, &limited, options);
    }
    match &planned.resolved.root {
        ResolvedRoot::Projection(_) => {
            let mut planned =
                planned_with_projection_scan_filters(planned, base_schema, scan_filters)
                    .ok_or_else(|| {
                        DataFusionError::Execution(
                    "manifest OQL DataFusion provider could not lower projection scan filters"
                        .into(),
                )
                    })?;
            if let Some(projection) = pushed_projection {
                apply_projection_scan_projection(&mut planned, base_schema, projection)?;
            }
            apply_scan_limit(&mut planned, limit);
            execute_manifest_oql_arrow(members, &planned, options)
        }
        ResolvedRoot::Object(_) | ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_) => {
            let mut planned =
                planned_with_row_scan_filters(planned, scan_filters).ok_or_else(|| {
                    DataFusionError::Execution(
                        "manifest OQL DataFusion provider could not lower row-root scan filters"
                            .into(),
                    )
                })?;
            if let Some(projection) = pushed_projection {
                apply_row_scan_projection(&mut planned, base_schema, projection)?;
            }
            apply_scan_limit(&mut planned, limit);
            execute_manifest_oql_arrow(members, &planned, options)
        }
    }
}

fn execute_manifest_oql_arrow(
    members: &[CoveOqlRetainedManifestMember],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Result<OqlArrowExecution> {
    let mut arrow_planned = planned.clone();
    arrow_planned.resolved.output_mode = crate::CoveOqlOutputMode::ArrowRecordBatch {
        zero_copy_requested: false,
    };
    arrow_planned.logical_plan.context.output_mode = crate::CoveOqlOutputMode::ArrowRecordBatch {
        zero_copy_requested: false,
    };
    let executed = execute_manifest_oql_arrow_physical_or_materialized(
        members,
        arrow_planned,
        options.clone(),
    )?;
    let authority = executed.authority.clone();
    let CoveOqlExecutionResult::ArrowRecordBatches(batches) = executed.result else {
        return Err(DataFusionError::Execution(
            "manifest OQL DataFusion provider requires ArrowRecordBatch output".into(),
        ));
    };
    Ok(OqlArrowExecution {
        row_count: batches.iter().map(RecordBatch::num_rows).sum(),
        batches,
        authority_source: authority.source,
        materialized_fallback: authority.materialized_fallback
            || authority.source == ExecutionAuthoritySource::MaterializedBaseline,
        residual_required: authority.residual_required,
        compared_with_materialized: authority.compared_with_materialized,
    })
}

fn execute_manifest_oql_arrow_physical_or_materialized(
    members: &[CoveOqlRetainedManifestMember],
    arrow_planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<crate::ExecutedQuery> {
    if let Some(first_member) = members.first() {
        if let Ok(physical) = build_physical_plan(
            first_member.as_slice(),
            arrow_planned.clone(),
            PhysicalPlanOptions::default(),
            cove_core::reader::ValidationOptions::default(),
        ) {
            let borrowed = members
                .iter()
                .map(CoveOqlRetainedManifestMember::as_manifest_member)
                .collect::<Vec<_>>();
            let kernel = execute_manifest_physical_planned_query(
                &borrowed,
                physical,
                options.clone(),
                KernelExecutionOptions::default(),
            )
            .map_err(|err| DataFusionError::Execution(err.to_string()))?;
            return Ok(kernel.executed);
        }
    }
    execute_manifest_planned_query_retained(members, arrow_planned, options)
        .map_err(|err| DataFusionError::Execution(err.to_string()))
}

fn execute_oql_arrow(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Result<OqlArrowExecution> {
    let mut arrow_planned = planned.clone();
    arrow_planned.resolved.output_mode = crate::CoveOqlOutputMode::ArrowRecordBatch {
        zero_copy_requested: false,
    };
    arrow_planned.logical_plan.context.output_mode = crate::CoveOqlOutputMode::ArrowRecordBatch {
        zero_copy_requested: false,
    };
    let executed =
        execute_oql_arrow_physical_or_materialized(bytes, arrow_planned, options.clone())?;
    let authority = executed.authority.clone();
    let CoveOqlExecutionResult::ArrowRecordBatches(batches) = executed.result else {
        return Err(DataFusionError::Execution(
            "OQL DataFusion provider requires ArrowRecordBatch output".into(),
        ));
    };
    Ok(OqlArrowExecution {
        row_count: batches.iter().map(RecordBatch::num_rows).sum(),
        batches,
        authority_source: authority.source,
        materialized_fallback: authority.materialized_fallback
            || authority.source == ExecutionAuthoritySource::MaterializedBaseline,
        residual_required: authority.residual_required,
        compared_with_materialized: authority.compared_with_materialized,
    })
}

fn execute_oql_arrow_physical_or_materialized(
    bytes: &[u8],
    arrow_planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<crate::ExecutedQuery> {
    if let Ok(physical) = build_physical_plan(
        bytes,
        arrow_planned.clone(),
        PhysicalPlanOptions::default(),
        cove_core::reader::ValidationOptions::default(),
    ) {
        let kernel = execute_physical_planned_query(
            bytes,
            physical,
            options.clone(),
            KernelExecutionOptions::default(),
        )
        .map_err(|err| DataFusionError::Execution(err.to_string()))?;
        return Ok(kernel.executed);
    }
    execute_planned_query_retained(
        CoveOqlRetainedInput::from_vec(bytes.to_vec()),
        arrow_planned,
        options,
    )
    .map_err(|err| DataFusionError::Execution(err.to_string()))
}

fn execute_oql_arrow_with_scan_filters(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    base_schema: &SchemaRef,
    pushed_projection: Option<&[usize]>,
    scan_filters: &[ProjectionFilter],
    limit: Option<usize>,
) -> Result<OqlArrowExecution> {
    if (scan_filters.is_empty() && pushed_projection.is_none())
        || !can_apply_datafusion_scan_filters(planned)
    {
        let limited = planned_with_scan_limit(planned, limit);
        return execute_oql_arrow(bytes, &limited, options);
    }
    match &planned.resolved.root {
        ResolvedRoot::Projection(root) => {
            let output_columns = if let Some(projection) = pushed_projection {
                projection_column_names(base_schema, Some(projection))?
            } else {
                base_schema
                    .fields()
                    .iter()
                    .map(|field| field.name().clone())
                    .collect()
            };
            let projection_options = ProjectionBatchOptions {
                max_rows: scan_limit_for_projection_readback(planned, limit),
                output_columns: Some(output_columns),
                pushed_filters: scan_filters.to_vec(),
                batch_size: options.batch_size,
            };
            let batches = projected_record_batches_from_cove_o_bytes(
                bytes,
                options.mapping_path.as_deref(),
                &root.projection_id,
                &projection_options,
            )
            .map_err(|err| {
                DataFusionError::Execution(format!(
                    "OQL DataFusion provider projection-filter readback failed: {err}"
                ))
            })?;
            Ok(OqlArrowExecution {
                row_count: batches.iter().map(RecordBatch::num_rows).sum(),
                batches,
                authority_source: ExecutionAuthoritySource::DataFusionProvider,
                materialized_fallback: false,
                residual_required: false,
                compared_with_materialized: false,
            })
        }
        ResolvedRoot::Object(_) | ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_) => {
            let mut planned =
                planned_with_row_scan_filters(planned, scan_filters).ok_or_else(|| {
                    DataFusionError::Execution(
                        "OQL DataFusion provider could not lower row-root scan filters".into(),
                    )
                })?;
            if let Some(projection) = pushed_projection {
                apply_row_scan_projection(&mut planned, base_schema, projection)?;
            }
            apply_scan_limit(&mut planned, limit);
            execute_oql_arrow(bytes, &planned, options)
        }
    }
}

fn planned_with_scan_limit(planned: &PlannedQuery, limit: Option<usize>) -> PlannedQuery {
    let mut planned = planned.clone();
    apply_scan_limit(&mut planned, limit);
    planned
}

fn apply_scan_limit(planned: &mut PlannedQuery, limit: Option<usize>) {
    let Some(limit) = limit else {
        return;
    };
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    planned.resolved.method_chain.take = Some(match planned.resolved.method_chain.take {
        Some(existing) => existing.min(limit),
        None => limit,
    });
}

fn scan_limit_for_projection_readback(
    planned: &PlannedQuery,
    limit: Option<usize>,
) -> Option<usize> {
    if planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
    {
        return None;
    }
    limit
}

fn can_apply_datafusion_scan_filters(planned: &PlannedQuery) -> bool {
    can_apply_datafusion_projection_filters(planned) || can_apply_datafusion_row_filters(planned)
}

fn can_apply_datafusion_scan_projection(planned: &PlannedQuery) -> bool {
    can_apply_datafusion_projection_filters(planned) || can_apply_datafusion_row_filters(planned)
}

fn can_apply_datafusion_projection_filters(planned: &PlannedQuery) -> bool {
    matches!(planned.resolved.root, ResolvedRoot::Projection(_))
        && projection_select_is_direct_unaliased_columns(planned)
        && planned.resolved.method_chain.where_predicate.is_none()
        && planned.resolved.method_chain.group_by.is_none()
        && planned.resolved.method_chain.order_by.is_none()
        && planned.resolved.method_chain.skip.is_none()
        && planned.resolved.method_chain.take.is_none()
        && planned.resolved.method_chain.history.is_none()
        && planned.resolved.method_chain.changes.is_none()
}

fn projection_select_is_direct_unaliased_columns(planned: &PlannedQuery) -> bool {
    let Some(select) = &planned.resolved.method_chain.select else {
        return true;
    };
    select.iter().all(|item| {
        let ResolvedExpr::Path(path) = &item.expr else {
            return false;
        };
        if path.root_kind != ResolvedPathRootKind::Projection {
            return false;
        }
        let Some(column) = &path.projection_column else {
            return false;
        };
        let alias_matches_column = match item.alias.as_deref() {
            Some(alias) => alias == column,
            None => true,
        };
        alias_matches_column && path.display_name == *column
    })
}

fn can_apply_datafusion_row_filters(planned: &PlannedQuery) -> bool {
    let Some(root_kind) = path_root_kind_for_resolved_root(&planned.resolved.root) else {
        return false;
    };
    !matches!(planned.resolved.root, ResolvedRoot::Projection(_))
        && planned.resolved.method_chain.where_predicate.is_none()
        && planned.resolved.method_chain.group_by.is_none()
        && planned.resolved.method_chain.order_by.is_none()
        && planned.resolved.method_chain.skip.is_none()
        && planned.resolved.method_chain.take.is_none()
        && planned.resolved.method_chain.history.is_none()
        && planned.resolved.method_chain.changes.is_none()
        && planned
            .resolved
            .method_chain
            .select
            .as_ref()
            .is_some_and(|select| {
                select.iter().all(|item| {
                    matches!(
                        &item.expr,
                        ResolvedExpr::Path(path) if path.root_kind == root_kind
                    )
                })
            })
}

fn classify_scan_projection_filters(
    schema: &SchemaRef,
    filters: &[Expr],
    planned: &PlannedQuery,
) -> Result<Vec<ProjectionFilter>> {
    if filters.is_empty() || !can_apply_datafusion_scan_filters(planned) {
        return Ok(Vec::new());
    }
    let report = projection_provider::classify_projection_filters_report(schema, filters)?;
    if can_apply_datafusion_row_filters(planned) {
        Ok(report
            .pushed_filters
            .into_iter()
            .filter(|filter| row_predicate_from_scan_filter(planned, filter).is_some())
            .collect())
    } else {
        Ok(report.pushed_filters)
    }
}

fn scan_filter_pushdown_support(
    schema: &SchemaRef,
    filter: &Expr,
    planned: &PlannedQuery,
) -> Result<TableProviderFilterPushDown> {
    if !can_apply_datafusion_scan_filters(planned) {
        return Ok(TableProviderFilterPushDown::Unsupported);
    }
    let report =
        datafusion_pushdown_report_for_plan(schema, std::slice::from_ref(filter), planned)?;
    if report.filter_outcomes.is_empty()
        || report.filter_outcomes.iter().any(|outcome| {
            matches!(
                outcome.outcome,
                DataFusionOqlFilterOutcomeKind::ResidualRejected
            )
        })
    {
        return Ok(TableProviderFilterPushDown::Unsupported);
    }
    if report.filter_outcomes.iter().all(|outcome| {
        matches!(
            outcome.outcome,
            DataFusionOqlFilterOutcomeKind::TrustedExact
        )
    }) {
        if row_root_exact_filter_needs_datafusion_projection_guard(schema, planned) {
            Ok(TableProviderFilterPushDown::Inexact)
        } else {
            Ok(TableProviderFilterPushDown::Exact)
        }
    } else {
        Ok(TableProviderFilterPushDown::Inexact)
    }
}

fn row_root_exact_filter_needs_datafusion_projection_guard(
    schema: &SchemaRef,
    planned: &PlannedQuery,
) -> bool {
    matches!(
        planned.resolved.root,
        ResolvedRoot::Object(_) | ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_)
    ) && schema.fields().len() > 1
}

fn planned_with_row_scan_filters(
    planned: &PlannedQuery,
    filters: &[ProjectionFilter],
) -> Option<PlannedQuery> {
    if filters.is_empty() || !can_apply_datafusion_row_filters(planned) {
        return Some(planned.clone());
    }
    let predicates = filters
        .iter()
        .map(|filter| row_predicate_from_scan_filter(planned, filter))
        .collect::<Option<Vec<_>>>()?;
    let mut planned = planned.clone();
    planned.resolved.method_chain.where_predicate = Some(if predicates.len() == 1 {
        predicates.into_iter().next().expect("checked len")
    } else {
        ResolvedPredicate::And(predicates)
    });
    Some(planned)
}

fn planned_with_projection_scan_filters(
    planned: &PlannedQuery,
    schema: &SchemaRef,
    filters: &[ProjectionFilter],
) -> Option<PlannedQuery> {
    if filters.is_empty() || !can_apply_datafusion_projection_filters(planned) {
        return Some(planned.clone());
    }
    let predicates = filters
        .iter()
        .map(|filter| projection_predicate_from_scan_filter(schema, filter))
        .collect::<Vec<_>>();
    let mut planned = planned.clone();
    planned.resolved.method_chain.where_predicate = Some(if predicates.len() == 1 {
        predicates.into_iter().next().expect("checked len")
    } else {
        ResolvedPredicate::And(predicates)
    });
    Some(planned)
}

fn projection_predicate_from_scan_filter(
    schema: &SchemaRef,
    filter: &ProjectionFilter,
) -> ResolvedPredicate {
    match filter {
        ProjectionFilter::Compare {
            column,
            op,
            literal,
        } => ResolvedPredicate::Compare {
            left: ResolvedExpr::Path(projection_resolved_path(column, schema)),
            op: projection_ast_op(*op),
            right: ResolvedExpr::Literal(projection_resolved_literal(literal)),
        },
        ProjectionFilter::InList { column, literals } => ResolvedPredicate::InList {
            expr: ResolvedExpr::Path(projection_resolved_path(column, schema)),
            values: literals
                .iter()
                .map(projection_resolved_literal)
                .collect::<Vec<_>>(),
        },
        ProjectionFilter::IsNull { column, negated } => ResolvedPredicate::NullCheck {
            expr: ResolvedExpr::Path(projection_resolved_path(column, schema)),
            negated: *negated,
        },
    }
}

fn apply_projection_scan_projection(
    planned: &mut PlannedQuery,
    base_schema: &SchemaRef,
    projection: &[usize],
) -> Result<()> {
    if projection.is_empty() || !matches!(planned.resolved.root, ResolvedRoot::Projection(_)) {
        return Ok(());
    }
    let requested_columns = projection_column_names(base_schema, Some(projection))?;
    let mut by_output_name = BTreeMap::new();
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            let output_name = select_item_output_name(item);
            if by_output_name
                .insert(output_name.clone(), item.clone())
                .is_some()
            {
                return Err(DataFusionError::Plan(format!(
                    "manifest OQL DataFusion provider cannot push ambiguous duplicate projection column {output_name:?}"
                )));
            }
        }
    }
    let narrowed = requested_columns
        .iter()
        .map(|column| {
            by_output_name.get(column).cloned().map_or_else(
                || {
                    Ok(ResolvedSelectItem {
                        alias: None,
                        expr: ResolvedExpr::Path(projection_resolved_path(column, base_schema)),
                    })
                },
                Ok,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    planned.resolved.method_chain.select = Some(narrowed);
    Ok(())
}

fn apply_row_scan_projection(
    planned: &mut PlannedQuery,
    base_schema: &SchemaRef,
    projection: &[usize],
) -> Result<()> {
    if projection.is_empty()
        || !matches!(
            planned.resolved.root,
            ResolvedRoot::Object(_) | ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_)
        )
    {
        return Ok(());
    }
    let requested_columns = projection_column_names(base_schema, Some(projection))?;
    let select = planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .ok_or_else(|| {
            DataFusionError::Execution(
                "OQL DataFusion provider cannot push row projection without selected row fields"
                    .into(),
            )
        })?;
    let mut by_output_name = BTreeMap::new();
    for item in select {
        let output_name = select_item_output_name(item);
        if by_output_name
            .insert(output_name.clone(), item.clone())
            .is_some()
        {
            return Err(DataFusionError::Plan(format!(
                "OQL DataFusion provider cannot push ambiguous duplicate output column {output_name:?}"
            )));
        }
    }
    let narrowed = requested_columns
        .iter()
        .map(|column| {
            by_output_name.get(column).cloned().ok_or_else(|| {
                DataFusionError::Plan(format!(
                    "OQL DataFusion provider cannot map projected column {column:?} to a selected row field"
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    planned.resolved.method_chain.select = Some(narrowed);
    Ok(())
}

fn select_item_output_name(item: &ResolvedSelectItem) -> String {
    item.alias
        .clone()
        .unwrap_or_else(|| resolved_expr_output_name(&item.expr))
}

fn resolved_expr_output_name(expr: &ResolvedExpr) -> String {
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::FunctionCall { function_id, .. } => function_id.clone(),
        ResolvedExpr::AggregateCall { name, .. } => format!("{name:?}").to_lowercase(),
        ResolvedExpr::Literal(_) => "literal".into(),
        ResolvedExpr::Association(association) => association.type_name.clone(),
        ResolvedExpr::Evidence(_) => "evidence".into(),
        ResolvedExpr::Conditional { .. } => "if".into(),
    }
}

fn row_predicate_from_scan_filter(
    planned: &PlannedQuery,
    filter: &ProjectionFilter,
) -> Option<ResolvedPredicate> {
    match filter {
        ProjectionFilter::Compare {
            column,
            op,
            literal,
        } => Some(ResolvedPredicate::Compare {
            left: ResolvedExpr::Path(row_resolved_path_for_column(planned, column)?),
            op: projection_ast_op(*op),
            right: ResolvedExpr::Literal(projection_resolved_literal(literal)),
        }),
        ProjectionFilter::InList { column, literals } => Some(ResolvedPredicate::InList {
            expr: ResolvedExpr::Path(row_resolved_path_for_column(planned, column)?),
            values: literals
                .iter()
                .map(projection_resolved_literal)
                .collect::<Vec<_>>(),
        }),
        ProjectionFilter::IsNull { column, negated } => Some(ResolvedPredicate::NullCheck {
            expr: ResolvedExpr::Path(row_resolved_path_for_column(planned, column)?),
            negated: *negated,
        }),
    }
}

fn row_resolved_path_for_column(planned: &PlannedQuery, column: &str) -> Option<ResolvedPath> {
    let root_kind = path_root_kind_for_resolved_root(&planned.resolved.root)?;
    let select = planned.resolved.method_chain.select.as_ref()?;
    select.iter().find_map(|item| {
        let ResolvedExpr::Path(path) = &item.expr else {
            return None;
        };
        if path.root_kind != root_kind {
            return None;
        }
        let output_name = item.alias.as_deref().unwrap_or(&path.display_name);
        (output_name == column).then(|| path.clone())
    })
}

fn path_root_kind_for_resolved_root(root: &ResolvedRoot) -> Option<ResolvedPathRootKind> {
    match root {
        ResolvedRoot::Object(_) => Some(ResolvedPathRootKind::Object),
        ResolvedRoot::Association(_) => Some(ResolvedPathRootKind::Association),
        ResolvedRoot::Evidence(_) => Some(ResolvedPathRootKind::Evidence),
        ResolvedRoot::Projection(_) => None,
    }
}

fn projected_schema(schema: &SchemaRef, projection: Option<&[usize]>) -> Result<SchemaRef> {
    let Some(projection) = projection else {
        return Ok(Arc::clone(schema));
    };
    let fields = projection
        .iter()
        .map(|index| {
            schema.fields().get(*index).cloned().ok_or_else(|| {
                DataFusionError::Internal(format!("projection index {index} is out of bounds"))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Arc::new(Schema::new(fields)))
}

fn projection_column_names(
    schema: &SchemaRef,
    projection: Option<&[usize]>,
) -> Result<Vec<String>> {
    let Some(projection) = projection else {
        return Ok(schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect());
    };
    projection
        .iter()
        .map(|index| {
            schema
                .fields()
                .get(*index)
                .map(|field| field.name().clone())
                .ok_or_else(|| {
                    DataFusionError::Internal(format!("projection index {index} is out of bounds"))
                })
        })
        .collect()
}

fn project_and_limit_batches(
    batches: Vec<RecordBatch>,
    projection: Option<&[usize]>,
    limit: Option<usize>,
) -> Result<Vec<RecordBatch>> {
    let mut remaining = limit.unwrap_or(usize::MAX);
    let mut out = Vec::new();
    for batch in batches {
        if remaining == 0 {
            break;
        }
        let projected = if let Some(projection) = projection {
            batch.project(projection).map_err(DataFusionError::from)?
        } else {
            batch
        };
        let limited = if projected.num_rows() > remaining {
            projected.slice(0, remaining)
        } else {
            projected
        };
        remaining = remaining.saturating_sub(limited.num_rows());
        out.push(limited);
    }
    Ok(out)
}

pub fn datafusion_oql_provider_for_plan(
    bytes: Arc<Vec<u8>>,
    planned: &PlannedQuery,
    options: ExecutionOptions,
) -> Result<Arc<OqlTableProvider>> {
    OqlTableProvider::try_new(bytes, planned.clone(), options).map(Arc::new)
}

pub fn datafusion_manifest_oql_provider_for_plan(
    members: Vec<CoveOqlRetainedManifestMember>,
    planned: &PlannedQuery,
    options: ExecutionOptions,
) -> Result<Arc<ManifestOqlTableProvider>> {
    ManifestOqlTableProvider::try_new(members, planned.clone(), options).map(Arc::new)
}

pub fn register_datafusion_oql_provider_for_plan(
    ctx: &SessionContext,
    table_name: &str,
    bytes: Arc<Vec<u8>>,
    planned: &PlannedQuery,
    options: ExecutionOptions,
) -> Result<DataFusionOqlProviderReport> {
    let provider = datafusion_oql_provider_for_plan(bytes, planned, options)?;
    let report = provider.report();
    ctx.register_table(table_name, provider as Arc<dyn TableProvider>)?;
    Ok(report)
}

pub fn register_datafusion_manifest_oql_provider_for_plan(
    ctx: &SessionContext,
    table_name: &str,
    members: Vec<CoveOqlRetainedManifestMember>,
    planned: &PlannedQuery,
    options: ExecutionOptions,
) -> Result<DataFusionOqlProviderReport> {
    let provider = datafusion_manifest_oql_provider_for_plan(members, planned, options)?;
    let report = provider.report();
    ctx.register_table(table_name, provider as Arc<dyn TableProvider>)?;
    Ok(report)
}

pub fn register_datafusion_oql_memtable_for_plan(
    ctx: &SessionContext,
    table_name: &str,
    bytes: Vec<u8>,
    planned: &PlannedQuery,
    options: ExecutionOptions,
) -> Result<DataFusionOqlProviderReport> {
    let mut arrow_planned = planned.clone();
    arrow_planned.resolved.output_mode = crate::CoveOqlOutputMode::ArrowRecordBatch {
        zero_copy_requested: false,
    };
    arrow_planned.logical_plan.context.output_mode = crate::CoveOqlOutputMode::ArrowRecordBatch {
        zero_copy_requested: false,
    };
    let executed = execute_planned_query_retained(
        CoveOqlRetainedInput::from_vec(bytes),
        arrow_planned,
        options,
    )
    .map_err(|err| datafusion::common::DataFusionError::Execution(err.to_string()))?;
    let authority = executed.authority.clone();
    let CoveOqlExecutionResult::ArrowRecordBatches(batches) = executed.result else {
        return Err(datafusion::common::DataFusionError::Execution(
            "OQL DataFusion MemTable registration requires ArrowRecordBatch output".into(),
        ));
    };
    let schema = batches.first().map(|batch| batch.schema()).ok_or_else(|| {
        datafusion::common::DataFusionError::Execution(
            "OQL DataFusion MemTable registration produced no Arrow batches".into(),
        )
    })?;
    let row_count = batches.iter().map(|batch| batch.num_rows()).sum();
    let batch_count = batches.len();
    let provider = MemTable::try_new(schema, vec![batches])?;
    ctx.register_table(table_name, Arc::new(provider) as Arc<dyn TableProvider>)?;
    Ok(DataFusionOqlProviderReport {
        report_version: crate::DATAFUSION_OQL_REPORT_VERSION.into(),
        provider_kind: "oql_memtable".into(),
        root_kind: resolved_root_name(&planned.resolved.root).into(),
        root_id: datafusion_pushdown_report_root_id(&planned.resolved.root),
        dataset_file_count: planned.resolved.operation_context.dataset.files.len(),
        planned_output_mode: planned.resolved.output_mode.clone(),
        materialized_oql_before_registration: true,
        residual_verification: true,
        scan_residual_verification_required: authority.residual_required,
        scan_filter_pushdown_supported: false,
        scan_projection_pushdown_supported: false,
        residual_filter_authority:
            "DataFusion evaluates filters against the materialized OQL Arrow MemTable".into(),
        oql_scan_authority_source: authority.source,
        oql_scan_materialized_fallback: authority.materialized_fallback
            || authority.source == ExecutionAuthoritySource::MaterializedBaseline,
        oql_scan_residual_required: authority.residual_required,
        oql_scan_compared_with_materialized: authority.compared_with_materialized,
        scan_execution_policy: "materialized_arrow_memtable_before_datafusion".into(),
        limit_pushdown_policy:
            "DataFusion applies limit against the materialized OQL Arrow MemTable".into(),
        unhandled_residuals: provider_residuals(planned),
        row_count,
        batch_count,
        notes: vec![
            "planned OQL semantics were executed before DataFusion registration; DataFusion scans a materialized Arrow MemTable".into(),
            "materialized OQL execution remains the semantic authority for residual predicates, temporal handling, visibility, and redaction".into(),
        ],
    })
}

pub fn datafusion_dataset_provider_for_plan(
    dataset: Arc<DatasetState>,
    planned: &PlannedQuery,
) -> Result<Arc<CoveTableProvider>> {
    validate_dataset_datafusion_plan(planned)?;
    Ok(Arc::new(CoveTableProvider::new(dataset)))
}

fn resolved_root_name(root: &ResolvedRoot) -> &'static str {
    match root {
        ResolvedRoot::Object(_) => "object",
        ResolvedRoot::Association(_) => "association",
        ResolvedRoot::Projection(_) => "projection",
        ResolvedRoot::Evidence(_) => "evidence",
    }
}

fn provider_scan_execution_policy(planned: &PlannedQuery, manifest: bool) -> &'static str {
    if manifest {
        if can_apply_datafusion_scan_filters(planned)
            || can_apply_datafusion_scan_projection(planned)
        {
            "manifest_oql_physical_or_materialized_scan"
        } else {
            "manifest_planned_oql_scan"
        }
    } else if matches!(planned.resolved.root, ResolvedRoot::Projection(_))
        && (can_apply_datafusion_projection_filters(planned)
            || can_apply_datafusion_scan_projection(planned))
    {
        "datafusion_projection_readback_fast_path_when_negotiated"
    } else if can_apply_datafusion_scan_filters(planned)
        || can_apply_datafusion_scan_projection(planned)
    {
        "oql_physical_or_materialized_scan"
    } else {
        "planned_oql_scan"
    }
}

fn exec_scan_execution_policy(
    planned: &PlannedQuery,
    manifest: bool,
    projection_pushed_to_oql: bool,
    filters_pushed_to_oql: bool,
) -> &'static str {
    if manifest {
        if projection_pushed_to_oql || filters_pushed_to_oql {
            "manifest_oql_physical_or_materialized_scan"
        } else {
            "manifest_planned_oql_scan"
        }
    } else if matches!(planned.resolved.root, ResolvedRoot::Projection(_))
        && (projection_pushed_to_oql || filters_pushed_to_oql)
    {
        "datafusion_projection_readback_fast_path"
    } else if projection_pushed_to_oql || filters_pushed_to_oql {
        "oql_physical_or_materialized_scan"
    } else {
        "planned_oql_scan"
    }
}

fn residual_filter_authority_label(report: &DataFusionOqlPushdownReport) -> &'static str {
    if report.received_filters.is_empty() {
        "no_datafusion_filters"
    } else if report.trusted
        && !report.trusted_filters.is_empty()
        && report.residual_filters.is_empty()
        && report.rejected_filters.is_empty()
    {
        "trusted_exact_oql_pushdown"
    } else {
        "datafusion_residual_verification"
    }
}

pub fn register_datafusion_dataset_for_plan(
    ctx: &SessionContext,
    table_name: &str,
    dataset: Arc<DatasetState>,
    planned: &PlannedQuery,
) -> Result<Arc<CoveTableProvider>> {
    let provider = datafusion_dataset_provider_for_plan(dataset, planned)?;
    ctx.register_table(table_name, provider.clone() as Arc<dyn TableProvider>)?;
    Ok(provider)
}

pub fn register_datafusion_projection_for_plan(
    ctx: &SessionContext,
    table_name: &str,
    object_path: &Path,
    mapping_path: Option<&Path>,
    planned: &PlannedQuery,
) -> Result<()> {
    let ResolvedRoot::Projection(root) = &planned.resolved.root else {
        return Err(datafusion::common::DataFusionError::Execution(
            "Cove-OQL DataFusion registration requires a projection-root plan".into(),
        ));
    };
    register::register_cove_o_projection(
        ctx,
        table_name,
        object_path,
        mapping_path,
        &root.projection_id,
    )
    .map(|_| ())
}

fn validate_dataset_datafusion_plan(planned: &PlannedQuery) -> Result<()> {
    if planned.resolved.output_mode != crate::CoveOqlOutputMode::DataFusionTableProvider {
        return Err(datafusion::common::DataFusionError::Plan(
            "Cove-OQL dataset DataFusion provider requires DataFusionTableProvider output mode"
                .into(),
        ));
    }
    if !matches!(planned.resolved.root, ResolvedRoot::Object(_)) {
        return Err(datafusion::common::DataFusionError::Plan(
            "Cove-OQL dataset DataFusion provider currently supports object-root table exposure only"
                .into(),
        ));
    }
    let chain = &planned.resolved.method_chain;
    if chain.where_predicate.is_some()
        || chain.group_by.is_some()
        || chain.order_by.is_some()
        || chain.skip.is_some()
        || chain.take.is_some()
        || chain.history.is_some()
        || chain.changes.is_some()
    {
        return Err(datafusion::common::DataFusionError::Plan(
            "Cove-OQL dataset DataFusion provider only exposes ungated object-root scans until OQL residual execution is attached"
                .into(),
        ));
    }
    Ok(())
}

pub fn datafusion_pushdown_report_for_plan(
    schema: &SchemaRef,
    filters: &[Expr],
    planned: &PlannedQuery,
) -> Result<DataFusionOqlPushdownReport> {
    if matches!(planned.resolved.root, ResolvedRoot::Projection(_))
        && can_apply_datafusion_projection_filters(planned)
    {
        return datafusion_projection_pushdown_report_for_plan(schema, filters, planned);
    }
    if matches!(
        planned.resolved.root,
        ResolvedRoot::Object(_) | ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_)
    ) && can_apply_datafusion_row_filters(planned)
    {
        return datafusion_row_pushdown_report_for_plan(schema, filters, planned);
    }
    Ok(datafusion_residual_pushdown_report(filters, planned))
}

pub fn datafusion_projection_pushdown_report_for_plan(
    schema: &SchemaRef,
    filters: &[Expr],
    planned: &PlannedQuery,
) -> Result<DataFusionOqlPushdownReport> {
    let ResolvedRoot::Projection(root) = &planned.resolved.root else {
        return Err(datafusion::common::DataFusionError::Execution(
            "Cove-OQL DataFusion pushdown reporting requires a projection-root plan".into(),
        ));
    };
    let report = projection_provider::classify_projection_filters_report(schema, filters)?;
    let lowered_forms = report
        .pushed_filters
        .iter()
        .map(|filter| projection_filter_logical_form(filter, schema, &planned.resolved.root))
        .collect::<Vec<_>>();
    let lowered_oql_predicates = report
        .pushed_filters
        .iter()
        .map(projection_filter_oql_summary)
        .collect::<Vec<_>>();
    let proof_states = lowered_forms
        .iter()
        .map(|form| form.representation.proof_state)
        .collect::<Vec<_>>();
    let received_filters = filters
        .iter()
        .map(|filter| format!("{filter:?}"))
        .collect::<Vec<_>>();
    let filter_outcomes = projection_filter_outcomes(schema, filters, &planned.resolved.root);
    let pushed_filters = report
        .pushed_filters
        .iter()
        .map(|filter| format!("{filter:?}"))
        .collect::<Vec<_>>();
    let trusted_filters = pushed_filters
        .iter()
        .zip(proof_states.iter())
        .filter_map(|(filter, proof_state)| {
            (*proof_state == PredicateProofState::ProvenExact).then(|| filter.clone())
        })
        .collect::<Vec<_>>();
    let residual_filters = residual_filters_from_outcomes(&filter_outcomes);
    let rejected_filters = rejected_filters_from_outcomes(&filter_outcomes);
    let mut decode_boundaries = rejected_filters.clone();
    decode_boundaries.extend(lowered_forms.iter().filter_map(|form| {
        if form.representation.representation == RepresentationClass::DecodeBoundary
            || form.representation.proof_state != PredicateProofState::ProvenExact
        {
            Some(
                form.residual_reason
                    .clone()
                    .unwrap_or_else(|| form.representation.reason.clone()),
            )
        } else {
            None
        }
    }));
    let all_supported = report.all_supported();
    let trusted = all_supported
        && proof_states
            .iter()
            .all(|state| *state == PredicateProofState::ProvenExact);
    Ok(DataFusionOqlPushdownReport {
        report_version: crate::DATAFUSION_OQL_REPORT_VERSION.into(),
        projection_id: root.projection_id.clone(),
        root_kind: resolved_root_name(&planned.resolved.root).into(),
        root_id: datafusion_pushdown_report_root_id(&planned.resolved.root),
        supported_filter_count: report.pushed_filters.len(),
        residual_filter_count: residual_filters.len(),
        received_filters,
        filter_outcomes,
        pushed_filters,
        trusted_filters,
        residual_filters,
        rejected_filters,
        lowered_oql_predicates,
        proof_states,
        decode_boundaries,
        trusted,
        notes: if trusted {
            vec!["all DataFusion filters lowered to proven-exact Cove-OQL predicates".into()]
        } else if all_supported {
            vec![
                "DataFusion filters lowered to Cove-OQL projection predicates, but no proof made them trusted exact; residual verification remains required".into(),
            ]
        } else {
            vec![
                "unsupported DataFusion expressions remain residual and must be evaluated by DataFusion".into(),
            ]
        },
    })
}

pub fn datafusion_object_pushdown_report_for_plan(
    schema: &SchemaRef,
    filters: &[Expr],
    planned: &PlannedQuery,
) -> Result<DataFusionOqlPushdownReport> {
    datafusion_row_pushdown_report_for_plan(schema, filters, planned)
}

pub fn datafusion_row_pushdown_report_for_plan(
    schema: &SchemaRef,
    filters: &[Expr],
    planned: &PlannedQuery,
) -> Result<DataFusionOqlPushdownReport> {
    let report = projection_provider::classify_projection_filters_report(schema, filters)?;
    let received_filters = filters
        .iter()
        .map(|filter| format!("{filter:?}"))
        .collect::<Vec<_>>();
    let filter_outcomes = row_filter_outcomes(schema, filters, planned);
    let mut pushed_filters = Vec::new();
    let mut trusted_filters = Vec::new();
    let mut lowered_oql_predicates = Vec::new();
    let mut proof_states = Vec::new();
    let mut decode_boundaries = report.residual_filters.clone();
    for filter in &report.pushed_filters {
        if let Some(predicate) = row_predicate_from_scan_filter(planned, filter) {
            let form = classify_predicate_for_dataset(
                &predicate,
                &planned.resolved.root,
                "datafusion_row_pushdown",
                &planned.resolved.operation_context.dataset,
            );
            let filter_text = format!("{filter:?}");
            pushed_filters.push(filter_text.clone());
            let proof_state = form.representation.proof_state;
            if proof_state == PredicateProofState::ProvenExact {
                trusted_filters.push(filter_text);
            }
            lowered_oql_predicates.push(scan_filter_oql_summary(
                resolved_root_name(&planned.resolved.root),
                filter,
            ));
            proof_states.push(proof_state);
            if form.representation.representation == RepresentationClass::DecodeBoundary
                || proof_state != PredicateProofState::ProvenExact
            {
                decode_boundaries.push(
                    form.residual_reason
                        .unwrap_or_else(|| form.representation.reason.clone()),
                );
            }
        } else {
            let residual = format!("{filter:?}");
            decode_boundaries.push(format!(
                "DataFusion filter {residual} does not reference a direct selected {} path",
                resolved_root_name(&planned.resolved.root)
            ));
            proof_states.push(PredicateProofState::DecodeRequired);
        }
    }
    let residual_filters = residual_filters_from_outcomes(&filter_outcomes);
    let rejected_filters = rejected_filters_from_outcomes(&filter_outcomes);
    let trusted = !pushed_filters.is_empty()
        && residual_filters.is_empty()
        && proof_states
            .iter()
            .all(|state| *state == PredicateProofState::ProvenExact);
    let notes = if trusted {
        vec![
            format!(
                "{}-root DataFusion filters lowered to exact Cove-OQL predicates over direct selected paths; DataFusion still retains residual verification",
                resolved_root_name(&planned.resolved.root)
            ),
        ]
    } else if residual_filters.is_empty() && !pushed_filters.is_empty() {
        vec![
            format!(
                "{}-root DataFusion filters lowered to Cove-OQL predicates, but proof contracts require DataFusion residual verification",
                resolved_root_name(&planned.resolved.root)
            ),
        ]
    } else {
        vec![
            format!(
                "only direct selected {}-path DataFusion filters can be lowered; unsupported filters remain DataFusion residuals",
                resolved_root_name(&planned.resolved.root)
            ),
        ]
    };
    Ok(DataFusionOqlPushdownReport {
        report_version: crate::DATAFUSION_OQL_REPORT_VERSION.into(),
        projection_id: String::new(),
        root_kind: resolved_root_name(&planned.resolved.root).into(),
        root_id: datafusion_pushdown_report_root_id(&planned.resolved.root),
        supported_filter_count: pushed_filters.len(),
        residual_filter_count: residual_filters.len(),
        received_filters,
        filter_outcomes,
        pushed_filters,
        trusted_filters,
        residual_filters: residual_filters.clone(),
        rejected_filters,
        lowered_oql_predicates,
        proof_states,
        decode_boundaries,
        trusted,
        notes,
    })
}

fn residual_filters_from_outcomes(outcomes: &[DataFusionOqlFilterOutcome]) -> Vec<String> {
    outcomes
        .iter()
        .filter(|outcome| !outcome.trusted)
        .map(|outcome| outcome.received_filter.clone())
        .collect()
}

fn rejected_filters_from_outcomes(outcomes: &[DataFusionOqlFilterOutcome]) -> Vec<String> {
    outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.outcome,
                DataFusionOqlFilterOutcomeKind::ResidualRejected
            )
        })
        .map(|outcome| outcome.received_filter.clone())
        .collect()
}

fn datafusion_residual_pushdown_report(
    filters: &[Expr],
    planned: &PlannedQuery,
) -> DataFusionOqlPushdownReport {
    let residual_filters = filters
        .iter()
        .map(|filter| format!("{filter:?}"))
        .collect::<Vec<_>>();
    DataFusionOqlPushdownReport {
        report_version: crate::DATAFUSION_OQL_REPORT_VERSION.into(),
        projection_id: match &planned.resolved.root {
            ResolvedRoot::Projection(root) => root.projection_id.clone(),
            _ => String::new(),
        },
        root_kind: resolved_root_name(&planned.resolved.root).into(),
        root_id: datafusion_pushdown_report_root_id(&planned.resolved.root),
        supported_filter_count: 0,
        residual_filter_count: filters.len(),
        received_filters: residual_filters.clone(),
        filter_outcomes: residual_filters
            .iter()
            .map(|filter| DataFusionOqlFilterOutcome {
                received_filter: filter.clone(),
                outcome: DataFusionOqlFilterOutcomeKind::ResidualRejected,
                lowered_oql_predicates: Vec::new(),
                proof_state: PredicateProofState::DecodeRequired,
                trusted: false,
                diagnostic_code: Some(
                    crate::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE.into(),
                ),
                reason: format!(
                    "plan shape for {} root does not allow scan-time filter lowering",
                    resolved_root_name(&planned.resolved.root)
                ),
            })
            .collect(),
        pushed_filters: Vec::new(),
        trusted_filters: Vec::new(),
        residual_filters: residual_filters.clone(),
        rejected_filters: residual_filters.clone(),
        lowered_oql_predicates: Vec::new(),
        proof_states: vec![PredicateProofState::DecodeRequired; filters.len()],
        decode_boundaries: if residual_filters.is_empty() {
            Vec::new()
        } else {
            vec![format!(
                "{} DataFusion filter(s) remain residual outside Cove-OQL pushdown for {} root",
                residual_filters.len(),
                resolved_root_name(&planned.resolved.root)
            )]
        },
        trusted: false,
        notes: vec![
            "DataFusion filters were not translated into Cove-OQL predicate forms for this plan; DataFusion must evaluate them as residual filters".into(),
        ],
    }
}

fn datafusion_pushdown_report_root_id(root: &ResolvedRoot) -> Option<String> {
    match root {
        ResolvedRoot::Object(root) => Some(root.type_name.clone()),
        ResolvedRoot::Association(root) => Some(root.type_name.clone()),
        ResolvedRoot::Projection(root) => Some(root.projection_id.clone()),
        ResolvedRoot::Evidence(_) => None,
    }
}

fn projection_filter_outcomes(
    schema: &SchemaRef,
    filters: &[Expr],
    root: &ResolvedRoot,
) -> Vec<DataFusionOqlFilterOutcome> {
    filters
        .iter()
        .map(|filter| {
            let received_filter = format!("{filter:?}");
            let Some(lowered_filters) = projection_provider::classify_projection_filter(schema, filter)
            else {
                return DataFusionOqlFilterOutcome {
                    received_filter,
                    outcome: DataFusionOqlFilterOutcomeKind::ResidualRejected,
                    lowered_oql_predicates: Vec::new(),
                    proof_state: PredicateProofState::DecodeRequired,
                    trusted: false,
                    diagnostic_code: Some(
                        crate::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE.into(),
                    ),
                    reason:
                        "DataFusion expression is not equivalent to a supported Cove-OQL scan predicate"
                            .into(),
                };
            };
            let proof_state = combined_projection_proof_state(schema, root, &lowered_filters);
            let trusted = proof_state == PredicateProofState::ProvenExact;
            DataFusionOqlFilterOutcome {
                received_filter,
                outcome: if trusted {
                    DataFusionOqlFilterOutcomeKind::TrustedExact
                } else {
                    DataFusionOqlFilterOutcomeKind::PushedInexact
                },
                lowered_oql_predicates: lowered_filters
                    .iter()
                    .map(projection_filter_oql_summary)
                    .collect(),
                proof_state,
                trusted,
                diagnostic_code: None,
                reason: if trusted {
                    "DataFusion filter lowered to a proven-exact Cove-OQL predicate".into()
                } else {
                    "DataFusion filter lowered to Cove-OQL scan predicates, but residual verification remains required".into()
                },
            }
        })
        .collect()
}

fn row_filter_outcomes(
    schema: &SchemaRef,
    filters: &[Expr],
    planned: &PlannedQuery,
) -> Vec<DataFusionOqlFilterOutcome> {
    filters
        .iter()
        .map(|filter| {
            let received_filter = format!("{filter:?}");
            let Some(lowered_filters) = projection_provider::classify_projection_filter(schema, filter)
            else {
                return DataFusionOqlFilterOutcome {
                    received_filter,
                    outcome: DataFusionOqlFilterOutcomeKind::ResidualRejected,
                    lowered_oql_predicates: Vec::new(),
                    proof_state: PredicateProofState::DecodeRequired,
                    trusted: false,
                    diagnostic_code: Some(
                        crate::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE.into(),
                    ),
                    reason:
                        "DataFusion expression is not equivalent to a supported Cove-OQL row predicate"
                            .into(),
                };
            };
            let lowered_oql_predicates = lowered_filters
                .iter()
                .filter(|filter| row_predicate_from_scan_filter(planned, filter).is_some())
                .map(|filter| {
                    scan_filter_oql_summary(resolved_root_name(&planned.resolved.root), filter)
                })
                .collect::<Vec<_>>();
            let proof_state = combined_row_proof_state(planned, &lowered_filters);
            let all_row_filters =
                !lowered_filters.is_empty() && lowered_oql_predicates.len() == lowered_filters.len();
            let trusted = all_row_filters && proof_state == PredicateProofState::ProvenExact;
            let outcome = if trusted {
                DataFusionOqlFilterOutcomeKind::TrustedExact
            } else if all_row_filters {
                DataFusionOqlFilterOutcomeKind::PushedInexact
            } else {
                DataFusionOqlFilterOutcomeKind::ResidualRejected
            };
            DataFusionOqlFilterOutcome {
                received_filter,
                outcome,
                lowered_oql_predicates,
                proof_state,
                trusted,
                diagnostic_code: if matches!(
                    outcome,
                    DataFusionOqlFilterOutcomeKind::ResidualRejected
                ) {
                    Some(crate::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE.into())
                } else {
                    None
                },
                reason: if trusted {
                    format!(
                        "DataFusion filter lowered to exact Cove-OQL {}-root predicates over direct selected paths",
                        resolved_root_name(&planned.resolved.root)
                    )
                } else if all_row_filters {
                    format!(
                        "DataFusion filter lowered to Cove-OQL {}-root predicates, but residual verification remains required by the predicate proof contract",
                        resolved_root_name(&planned.resolved.root)
                    )
                } else {
                    format!(
                        "DataFusion filter does not reference only direct selected {} paths and remains residual",
                        resolved_root_name(&planned.resolved.root)
                    )
                },
            }
        })
        .collect()
}

fn combined_row_proof_state(
    planned: &PlannedQuery,
    filters: &[ProjectionFilter],
) -> PredicateProofState {
    let mut saw_decode_required = false;
    let mut saw_candidate = false;
    for filter in filters {
        let Some(predicate) = row_predicate_from_scan_filter(planned, filter) else {
            saw_decode_required = true;
            continue;
        };
        match classify_predicate_for_dataset(
            &predicate,
            &planned.resolved.root,
            "datafusion_row_pushdown",
            &planned.resolved.operation_context.dataset,
        )
        .representation
        .proof_state
        {
            PredicateProofState::ProvenExact => {}
            PredicateProofState::CandidateNeedsResidual => saw_candidate = true,
            PredicateProofState::DecodeRequired => saw_decode_required = true,
        }
    }
    if saw_decode_required {
        PredicateProofState::DecodeRequired
    } else if saw_candidate {
        PredicateProofState::CandidateNeedsResidual
    } else {
        PredicateProofState::ProvenExact
    }
}

fn combined_projection_proof_state(
    schema: &SchemaRef,
    root: &ResolvedRoot,
    filters: &[ProjectionFilter],
) -> PredicateProofState {
    let mut saw_decode_required = false;
    let mut saw_candidate = false;
    for filter in filters {
        match projection_filter_logical_form(filter, schema, root)
            .representation
            .proof_state
        {
            PredicateProofState::ProvenExact => {}
            PredicateProofState::CandidateNeedsResidual => saw_candidate = true,
            PredicateProofState::DecodeRequired => saw_decode_required = true,
        }
    }
    if saw_decode_required {
        PredicateProofState::DecodeRequired
    } else if saw_candidate {
        PredicateProofState::CandidateNeedsResidual
    } else {
        PredicateProofState::ProvenExact
    }
}

fn provider_residuals(planned: &PlannedQuery) -> Vec<String> {
    let chain = &planned.resolved.method_chain;
    let mut residuals = Vec::new();
    if chain.where_predicate.is_some() {
        residuals
            .push("planned Cove-OQL where predicate executes before DataFusion scan output".into());
    }
    if chain.group_by.is_some() {
        residuals
            .push("planned Cove-OQL grouping executes inside materialized OQL semantics".into());
    }
    if chain.order_by.is_some() {
        residuals
            .push("planned Cove-OQL ordering executes inside materialized OQL semantics".into());
    }
    if chain.skip.is_some() || chain.take.is_some() {
        residuals
            .push("planned Cove-OQL pagination executes inside materialized OQL semantics".into());
    }
    if chain.history.is_some() || chain.changes.is_some() {
        residuals.push(
            "temporal history/changes semantics execute before DataFusion scan output".into(),
        );
    }
    residuals
}

fn projection_filter_logical_form(
    filter: &ProjectionFilter,
    schema: &SchemaRef,
    root: &ResolvedRoot,
) -> LogicalPredicateForm {
    let predicate = match filter {
        ProjectionFilter::Compare {
            column,
            op,
            literal,
        } => ResolvedPredicate::Compare {
            left: ResolvedExpr::Path(projection_resolved_path(column, schema)),
            op: projection_ast_op(*op),
            right: ResolvedExpr::Literal(projection_resolved_literal(literal)),
        },
        ProjectionFilter::InList { column, literals } => ResolvedPredicate::InList {
            expr: ResolvedExpr::Path(projection_resolved_path(column, schema)),
            values: literals
                .iter()
                .map(projection_resolved_literal)
                .collect::<Vec<_>>(),
        },
        ProjectionFilter::IsNull { column, negated } => ResolvedPredicate::NullCheck {
            expr: ResolvedExpr::Path(projection_resolved_path(column, schema)),
            negated: *negated,
        },
    };
    classify_predicate(&predicate, root, "datafusion_pushdown")
}

fn projection_resolved_path(column: &str, schema: &SchemaRef) -> ResolvedPath {
    let (logical_type, nullable) = schema
        .field_with_name(column)
        .map(|field| {
            (
                logical_type_for_arrow(field.data_type()).to_string(),
                field.is_nullable(),
            )
        })
        .unwrap_or_else(|_| ("utf8".into(), true));
    ResolvedPath {
        display_name: column.into(),
        root_kind: ResolvedPathRootKind::Projection,
        object_type_id: None,
        property_id: None,
        association_type_id: None,
        evidence_field_id: None,
        projection_id: None,
        projection_column: Some(column.into()),
        system_field: None,
        logical_type: logical_type.clone(),
        physical_kind: physical_kind_for_logical_name(&logical_type).into(),
        collation_id: None,
        nullable,
        null_policy: if nullable { "allow" } else { "reject" }.into(),
        temporal_role: None,
        code_domain_id: CodeDomainId::Placeholder {
            root: "projection".into(),
            object_type_id: None,
            property_id: None,
            projection_id: None,
            field: Some(column.into()),
        },
    }
}

fn projection_resolved_literal(literal: &ProjectionFilterLiteral) -> ResolvedLiteral {
    match literal {
        ProjectionFilterLiteral::Null => ResolvedLiteral {
            literal: AstLiteral::Null,
            logical_type: "null".into(),
            canonical: "null".into(),
            typed_value: ResolvedLiteralValue::Null,
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Boolean(value) => ResolvedLiteral {
            literal: AstLiteral::Boolean(*value),
            logical_type: "bool".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::Boolean(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Int64(value) => ResolvedLiteral {
            literal: AstLiteral::Integer(value.to_string()),
            logical_type: "int64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::SignedInteger(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::UInt64(value) => ResolvedLiteral {
            literal: AstLiteral::Integer(value.to_string()),
            logical_type: "uint64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::UnsignedInteger(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Float64(value) => ResolvedLiteral {
            literal: AstLiteral::Decimal(value.to_string()),
            logical_type: "float64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::Decimal {
                canonical: value.to_string(),
                precision: 0,
                scale: 0,
            },
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Utf8(value) => ResolvedLiteral {
            literal: AstLiteral::String(value.clone()),
            logical_type: "utf8".into(),
            canonical: value.clone(),
            typed_value: ResolvedLiteralValue::String(value.clone()),
            precision: None,
            scale: None,
        },
    }
}

fn projection_ast_op(op: ProjectionFilterOp) -> AstCompareOp {
    match op {
        ProjectionFilterOp::Eq => AstCompareOp::Eq,
        ProjectionFilterOp::Ne => AstCompareOp::Ne,
        ProjectionFilterOp::Lt => AstCompareOp::Lt,
        ProjectionFilterOp::LtEq => AstCompareOp::Le,
        ProjectionFilterOp::Gt => AstCompareOp::Gt,
        ProjectionFilterOp::GtEq => AstCompareOp::Ge,
    }
}

fn logical_type_for_arrow(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::Boolean => "bool",
        DataType::Int8 => "int8",
        DataType::Int16 => "int16",
        DataType::Int32 => "int32",
        DataType::Int64 => "int64",
        DataType::UInt8 => "uint8",
        DataType::UInt16 => "uint16",
        DataType::UInt32 => "uint32",
        DataType::UInt64 => "uint64",
        DataType::Float32 => "float32",
        DataType::Float64 => "float64",
        DataType::Date32 => "date_days",
        DataType::Timestamp(_, _) => "timestamp_micros",
        _ => "utf8",
    }
}

fn physical_kind_for_logical_name(logical: &str) -> &'static str {
    match logical {
        "bool" | "boolean" => "boolean",
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => "num_code",
        "uuid" | "decimal128" => "fixed_bytes",
        "list" => "list",
        "struct" => "struct",
        "map" => "map",
        _ => "var_bytes",
    }
}

fn projection_filter_oql_summary(filter: &ProjectionFilter) -> String {
    scan_filter_oql_summary("projection", filter)
}

fn scan_filter_oql_summary(root: &str, filter: &ProjectionFilter) -> String {
    match filter {
        ProjectionFilter::Compare {
            column,
            op,
            literal,
        } => format!("{root}.{column} {} {:?}", projection_op_name(*op), literal),
        ProjectionFilter::InList { column, literals } => {
            format!("{root}.{column} in [{} literals]", literals.len())
        }
        ProjectionFilter::IsNull { column, negated } => {
            if *negated {
                format!("{root}.{column} is not null")
            } else {
                format!("{root}.{column} is null")
            }
        }
    }
}

fn projection_op_name(op: ProjectionFilterOp) -> &'static str {
    match op {
        ProjectionFilterOp::Eq => "=",
        ProjectionFilterOp::Ne => "!=",
        ProjectionFilterOp::Lt => "<",
        ProjectionFilterOp::LtEq => "<=",
        ProjectionFilterOp::Gt => ">",
        ProjectionFilterOp::GtEq => ">=",
    }
}
