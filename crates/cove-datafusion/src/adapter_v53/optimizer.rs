//! Logical optimizer hooks for COVE metadata-aware planning.

use std::{fmt::Debug, sync::Arc};

use arrow_array::{Int32Array, Int64Array, RecordBatch, StringArray, UInt32Array, UInt64Array};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datafusion::{
    common::{
        tree_node::Transformed, Column, DataFusionError, JoinConstraint, JoinType, NullEquality,
        Result, ScalarValue, TableReference,
    },
    datasource::{provider_as_source, source_as_provider},
    execution::context::SessionContext,
    logical_expr::{Distinct, Expr, Join, LogicalPlan, Projection, TableScan},
    optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule},
};

#[cfg(feature = "covi")]
use crate::metadata_aggregate::{
    exact_covi_unfiltered_distinct_counts, exact_covi_unfiltered_scalar_values,
};
use crate::{
    adapter_v53::{
        filter::classify_filter,
        metadata::CoveMetadataTableProvider,
        native_aggregate::{
            CoveNativeAggregateTableProvider, CoveNativeBoolI64GroupAggregateTableProvider,
            CoveNativeFileCodeI64GroupAggregateTableProvider,
            CoveNativeI64I64GroupAggregateTableProvider, NativeI64AggregateKind,
            NativeI64AggregateRequest,
        },
        native_count::CoveNativeCountTableProvider,
        native_group::CoveNativeGroupCountTableProvider,
        native_join::{
            CoveNativeFileCodeJoinTableProvider, CoveNativeI64JoinTableProvider, NativeI64JoinKind,
        },
        native_order::CoveNativeI64OrderTableProvider,
        table_provider::CoveTableProvider,
    },
    metadata_aggregate::{
        canonical_utf8, exact_filecode_filtered_count, exact_filecode_group_counts,
        exact_unfiltered_aggregate_synopses, exact_unfiltered_counts, MetadataAggregatePlan,
        MetadataAggregateValue, MetadataSynopsisAggregateKind,
    },
    planner::{plan_scan, CoveFilterUse, CovePredicate, ScanPlan, TopNScanHint},
};
use cove_core::constants::{CoveLogicalType, CovePhysicalKind};
#[cfg(feature = "covi")]
use cove_index::execution::CoviAggregateKindV2;

pub(crate) const COVE_METADATA_OPTIMIZER: &str = "cove_metadata_optimizer";

#[derive(Debug, Default)]
pub(crate) struct CoveMetadataOptimizerRule;

pub(crate) fn install_cove_optimizer(ctx: &SessionContext) {
    let already_installed = ctx
        .state()
        .optimizers()
        .iter()
        .any(|rule| rule.name() == COVE_METADATA_OPTIMIZER);
    if !already_installed {
        ctx.add_optimizer_rule(Arc::new(CoveMetadataOptimizerRule));
    }
}

impl OptimizerRule for CoveMetadataOptimizerRule {
    fn name(&self) -> &str {
        COVE_METADATA_OPTIMIZER
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        match plan {
            LogicalPlan::Aggregate(aggregate) => {
                if let Some(rewritten) = rewrite_exact_count_aggregate(&aggregate)? {
                    Ok(Transformed::yes(rewritten))
                } else {
                    Ok(Transformed::no(LogicalPlan::Aggregate(aggregate)))
                }
            }
            LogicalPlan::Sort(sort) => {
                if let Some(rewritten) = rewrite_topn_sort(&sort)? {
                    Ok(Transformed::yes(rewritten))
                } else {
                    Ok(Transformed::no(LogicalPlan::Sort(sort)))
                }
            }
            LogicalPlan::Distinct(distinct) => {
                if let Some(rewritten) = rewrite_native_i64_distinct_all(&distinct)? {
                    Ok(Transformed::yes(rewritten))
                } else if let Some(rewritten) = rewrite_native_bool_distinct_all(&distinct)? {
                    Ok(Transformed::yes(rewritten))
                } else if let Some(rewritten) = rewrite_native_filecode_distinct_all(&distinct)? {
                    Ok(Transformed::yes(rewritten))
                } else {
                    Ok(Transformed::no(LogicalPlan::Distinct(distinct)))
                }
            }
            LogicalPlan::Projection(projection) => {
                if let Some(rewritten) = rewrite_native_filecode_key_join_projection(&projection)? {
                    Ok(Transformed::yes(rewritten))
                } else if let Some(rewritten) = rewrite_native_i64_key_join_projection(&projection)?
                {
                    Ok(Transformed::yes(rewritten))
                } else {
                    Ok(Transformed::no(LogicalPlan::Projection(projection)))
                }
            }
            LogicalPlan::Join(join) => {
                if let Some(rewritten) = rewrite_native_filecode_key_semi_anti_join(&join)? {
                    Ok(Transformed::yes(rewritten))
                } else if let Some(rewritten) = rewrite_native_i64_key_semi_anti_join(&join)? {
                    Ok(Transformed::yes(rewritten))
                } else {
                    Ok(Transformed::no(LogicalPlan::Join(join)))
                }
            }
            other => Ok(Transformed::no(other)),
        }
    }
}

fn rewrite_exact_count_aggregate(
    aggregate: &datafusion::logical_expr::Aggregate,
) -> Result<Option<LogicalPlan>> {
    let Some((scan, filters)) = aggregate_scan_and_filters(aggregate.input.as_ref()) else {
        return Ok(None);
    };
    let Some(provider) = cove_provider_from_scan(scan)? else {
        return Ok(None);
    };
    if let Some(plan) = metadata_aggregate_plan(aggregate, scan, &filters, provider.as_ref())? {
        let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
        let Some(batch) = record_batch_for_metadata_plan(&plan, Arc::clone(&schema))? else {
            return Ok(None);
        };
        let proof = plan.proof().clone();
        let table = CoveMetadataTableProvider::new(Arc::clone(&schema), batch, proof);
        let scan = TableScan::try_new(
            scan.table_name.clone(),
            provider_as_source(Arc::new(table)),
            None,
            Vec::new(),
            None,
        )?;
        return Ok(Some(LogicalPlan::TableScan(scan)));
    }
    if let Some(rewritten) =
        rewrite_native_count_star(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_i64_aggregate(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_i64_i64_group_aggregate(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_bool_i64_group_aggregate(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_filecode_i64_group_aggregate(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_i64_group_count(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_bool_group_count(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_filecode_group_count(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_i64_distinct_group(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    if let Some(rewritten) =
        rewrite_native_bool_distinct_group(aggregate, scan, &filters, provider.as_ref())?
    {
        return Ok(Some(rewritten));
    }
    rewrite_native_filecode_distinct_group(aggregate, scan, &filters, provider.as_ref())
}

fn metadata_aggregate_plan(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<MetadataAggregatePlan>> {
    if aggregate.group_expr.is_empty() {
        if filters.is_empty() {
            let synopsis_requests = aggregate
                .aggr_expr
                .iter()
                .map(|expr| synopsis_aggregate_request(expr, provider))
                .collect::<Option<Vec<_>>>();
            if let Some(synopsis_requests) = synopsis_requests {
                if !synopsis_requests.is_empty() {
                    if let Some(plan) =
                        exact_unfiltered_aggregate_synopses(provider.state(), &synopsis_requests)
                            .map_err(crate::adapter_v53::cove_to_datafusion)?
                    {
                        return Ok(Some(plan));
                    }
                }
            }
        }

        #[cfg(feature = "covi")]
        if filters.is_empty() {
            let distinct_requests = aggregate
                .aggr_expr
                .iter()
                .map(|expr| count_distinct_column_index(expr, provider))
                .collect::<Option<Vec<_>>>();
            if let Some(distinct_requests) = distinct_requests {
                if !distinct_requests.is_empty() {
                    return exact_covi_unfiltered_distinct_counts(
                        provider.state(),
                        &distinct_requests,
                    )
                    .map_err(crate::adapter_v53::cove_to_datafusion);
                }
            }

            let scalar_value_requests = aggregate
                .aggr_expr
                .iter()
                .map(|expr| index_only_scalar_value_request(expr, provider))
                .collect::<Option<Vec<_>>>();
            if let Some(scalar_value_requests) = scalar_value_requests {
                if !scalar_value_requests.is_empty() {
                    return exact_covi_unfiltered_scalar_values(
                        provider.state(),
                        &scalar_value_requests,
                    )
                    .map_err(crate::adapter_v53::cove_to_datafusion);
                }
            }
        }

        let mut count_columns = Vec::with_capacity(aggregate.aggr_expr.len());
        for expr in &aggregate.aggr_expr {
            let Some(column_index) = count_column_index(expr, provider) else {
                return Ok(None);
            };
            count_columns.push(column_index);
        }
        if filters.is_empty() {
            if count_columns.iter().all(Option::is_none)
                && scan
                    .projection
                    .as_ref()
                    .map(|projection| !projection.is_empty())
                    .unwrap_or(false)
            {
                return Ok(None);
            }
            if count_columns.iter().all(Option::is_none)
                && provider
                    .state()
                    .table()
                    .columns
                    .iter()
                    .any(|column| column.physical == CovePhysicalKind::FileCode)
            {
                return Ok(None);
            }
            return exact_unfiltered_counts(provider.state(), &count_columns)
                .map_err(crate::adapter_v53::cove_to_datafusion);
        }
        if count_columns.len() == 1 && count_columns[0].is_none() && filters.len() == 1 {
            let Some((column_index, canonical_values)) = filecode_filter(provider, &filters[0])
            else {
                return Ok(None);
            };
            return exact_filecode_filtered_count(
                provider.state(),
                column_index,
                &canonical_values,
            )
            .map_err(crate::adapter_v53::cove_to_datafusion);
        }
        return Ok(None);
    }

    if aggregate.group_expr.len() == 1
        && aggregate.aggr_expr.len() == 1
        && filters.is_empty()
        && matches!(
            count_column_index(&aggregate.aggr_expr[0], provider),
            Some(None)
        )
    {
        let Expr::Column(column) = &aggregate.group_expr[0] else {
            return Ok(None);
        };
        let Some(column_index) = provider
            .state()
            .table()
            .columns
            .iter()
            .position(|candidate| candidate.name == column.name)
        else {
            return Ok(None);
        };
        return exact_filecode_group_counts(provider.state(), column_index)
            .map_err(crate::adapter_v53::cove_to_datafusion);
    }
    Ok(None)
}

fn aggregate_scan_and_filters(input: &LogicalPlan) -> Option<(&TableScan, Vec<Expr>)> {
    match input {
        LogicalPlan::TableScan(scan) => Some((scan, dedup_filters(scan.filters.clone()))),
        LogicalPlan::Projection(projection) if projection.expr.is_empty() => {
            aggregate_scan_and_filters(projection.input.as_ref())
        }
        LogicalPlan::Filter(filter) => {
            let (scan, mut filters) = aggregate_scan_and_filters(filter.input.as_ref())?;
            filters.push(filter.predicate.clone());
            Some((scan, dedup_filters(filters)))
        }
        _ => None,
    }
}

fn dedup_filters(filters: Vec<Expr>) -> Vec<Expr> {
    let mut out = Vec::new();
    for filter in filters {
        if !out
            .iter()
            .any(|existing: &Expr| existing.to_string() == filter.to_string())
        {
            out.push(filter);
        }
    }
    out
}

fn filecode_filter(provider: &CoveTableProvider, expr: &Expr) -> Option<(usize, Vec<Vec<u8>>)> {
    let filter = classify_filter(provider.state(), expr);
    match filter.predicate {
        Some(CovePredicate::FileCodeIn {
            column_index,
            canonical_keys,
            ..
        }) if !canonical_keys.is_empty() => Some((column_index, canonical_keys)),
        _ => None,
    }
}

fn native_exact_filter_plan(
    provider: &CoveTableProvider,
    filters: &[Expr],
) -> Result<Option<ScanPlan>> {
    let lowered = filters
        .iter()
        .map(|filter| classify_filter(provider.state(), filter))
        .collect::<Vec<_>>();
    let projection = Vec::new();
    let plan = plan_scan(provider.state(), Some(&projection), lowered)
        .map_err(crate::adapter_v53::cove_to_datafusion)?;
    if plan.scan_program.inexact_filters != 0 {
        return Ok(None);
    }
    if plan.filters.iter().any(|filter| {
        filter.use_kind != CoveFilterUse::FullRowPredicateExact || filter.predicate.is_none()
    }) {
        return Ok(None);
    }
    Ok(Some(plan))
}

fn rewrite_native_count_star(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if !aggregate.group_expr.is_empty() || aggregate.aggr_expr.len() != 1 || filters.is_empty() {
        return Ok(None);
    }
    if !matches!(
        count_column_index(&aggregate.aggr_expr[0], provider),
        Some(None)
    ) {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != 1
        || !matches!(
            schema.field(0).data_type(),
            DataType::Int64 | DataType::UInt64
        )
    {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let table = CoveNativeCountTableProvider::new(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = schema
        .fields()
        .iter()
        .map(|field| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                field.name().clone(),
            ))
            .alias(field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_i64_aggregate(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if !aggregate.group_expr.is_empty() {
        return Ok(None);
    }
    let Some(requests) = native_i64_aggregate_requests(aggregate, provider) else {
        return Ok(None);
    };
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    for (index, request) in requests.iter().enumerate() {
        let data_type = schema.field(index).data_type();
        let supported = match request.kind {
            NativeI64AggregateKind::Count => {
                matches!(data_type, DataType::Int64 | DataType::UInt64)
            }
            NativeI64AggregateKind::Avg => data_type == &DataType::Float64,
            NativeI64AggregateKind::Sum
            | NativeI64AggregateKind::Min
            | NativeI64AggregateKind::Max => data_type == &DataType::Int64,
        };
        if !supported {
            return Ok(None);
        }
    }
    let table = CoveNativeAggregateTableProvider::new(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        requests,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = schema
        .fields()
        .iter()
        .map(|field| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                field.name().clone(),
            ))
            .alias(field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_i64_i64_group_aggregate(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || aggregate.aggr_expr.is_empty() {
        return Ok(None);
    }
    let Expr::Column(group_column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(group_column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == group_column.name)
    else {
        return Ok(None);
    };
    let cove_group_column = &provider.state().table().columns[group_column_index];
    if cove_group_column.logical != CoveLogicalType::Int64
        || cove_group_column.physical != CovePhysicalKind::NumCode
    {
        return Ok(None);
    }
    let Some(requests) = native_i64_aggregate_requests(aggregate, provider) else {
        return Ok(None);
    };
    let Some(first_request) = requests.first() else {
        return Ok(None);
    };
    if requests
        .iter()
        .any(|request| request.column_index != first_request.column_index)
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != requests.len() + 1
        || schema.field(0).data_type() != &DataType::Int64
    {
        return Ok(None);
    }
    for (index, request) in requests.iter().enumerate() {
        let data_type = schema.field(index + 1).data_type();
        let supported = match request.kind {
            NativeI64AggregateKind::Count => {
                matches!(data_type, DataType::Int64 | DataType::UInt64)
            }
            NativeI64AggregateKind::Avg => data_type == &DataType::Float64,
            NativeI64AggregateKind::Sum
            | NativeI64AggregateKind::Min
            | NativeI64AggregateKind::Max => data_type == &DataType::Int64,
        };
        if !supported {
            return Ok(None);
        }
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let provider_fields = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            Field::new(
                format!("__cove_native_i64_group_agg_{index}"),
                field.data_type().clone(),
                field.is_nullable(),
            )
        })
        .collect::<Vec<_>>();
    let provider_schema: SchemaRef = Arc::new(Schema::new(provider_fields));
    let table = CoveNativeI64I64GroupAggregateTableProvider::new(
        Arc::clone(&provider_schema),
        Arc::clone(provider.state()),
        group_column_index,
        requests,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = provider_schema
        .fields()
        .iter()
        .zip(schema.fields().iter())
        .map(|(provider_field, output_field)| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                provider_field.name().clone(),
            ))
            .alias(output_field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_bool_i64_group_aggregate(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || aggregate.aggr_expr.is_empty() {
        return Ok(None);
    }
    let Expr::Column(group_column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(group_column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == group_column.name)
    else {
        return Ok(None);
    };
    let cove_group_column = &provider.state().table().columns[group_column_index];
    if cove_group_column.logical != CoveLogicalType::Bool
        || cove_group_column.physical != CovePhysicalKind::Boolean
    {
        return Ok(None);
    }
    let Some(requests) = native_i64_aggregate_requests(aggregate, provider) else {
        return Ok(None);
    };
    let Some(first_request) = requests.first() else {
        return Ok(None);
    };
    if requests
        .iter()
        .any(|request| request.column_index != first_request.column_index)
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != requests.len() + 1
        || schema.field(0).data_type() != &DataType::Boolean
    {
        return Ok(None);
    }
    for (index, request) in requests.iter().enumerate() {
        let data_type = schema.field(index + 1).data_type();
        let supported = match request.kind {
            NativeI64AggregateKind::Count => {
                matches!(data_type, DataType::Int64 | DataType::UInt64)
            }
            NativeI64AggregateKind::Avg => data_type == &DataType::Float64,
            NativeI64AggregateKind::Sum
            | NativeI64AggregateKind::Min
            | NativeI64AggregateKind::Max => data_type == &DataType::Int64,
        };
        if !supported {
            return Ok(None);
        }
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let provider_fields = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            Field::new(
                format!("__cove_native_group_agg_{index}"),
                field.data_type().clone(),
                field.is_nullable(),
            )
        })
        .collect::<Vec<_>>();
    let provider_schema: SchemaRef = Arc::new(Schema::new(provider_fields));
    let table = CoveNativeBoolI64GroupAggregateTableProvider::new(
        Arc::clone(&provider_schema),
        Arc::clone(provider.state()),
        group_column_index,
        requests,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = provider_schema
        .fields()
        .iter()
        .zip(schema.fields().iter())
        .map(|(provider_field, output_field)| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                provider_field.name().clone(),
            ))
            .alias(output_field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_filecode_i64_group_aggregate(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || aggregate.aggr_expr.is_empty() {
        return Ok(None);
    }
    let Expr::Column(group_column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(group_column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == group_column.name)
    else {
        return Ok(None);
    };
    let cove_group_column = &provider.state().table().columns[group_column_index];
    if cove_group_column.logical != CoveLogicalType::Utf8
        || cove_group_column.physical != CovePhysicalKind::FileCode
    {
        return Ok(None);
    }
    if provider
        .state()
        .files()
        .iter()
        .any(|file| file.has_redaction())
    {
        return Ok(None);
    }
    let Some(requests) = native_i64_aggregate_requests(aggregate, provider) else {
        return Ok(None);
    };
    let Some(first_request) = requests.first() else {
        return Ok(None);
    };
    if requests
        .iter()
        .any(|request| request.column_index != first_request.column_index)
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != requests.len() + 1 || schema.field(0).data_type() != &DataType::Utf8
    {
        return Ok(None);
    }
    for (index, request) in requests.iter().enumerate() {
        let data_type = schema.field(index + 1).data_type();
        let supported = match request.kind {
            NativeI64AggregateKind::Count => {
                matches!(data_type, DataType::Int64 | DataType::UInt64)
            }
            NativeI64AggregateKind::Avg => data_type == &DataType::Float64,
            NativeI64AggregateKind::Sum
            | NativeI64AggregateKind::Min
            | NativeI64AggregateKind::Max => data_type == &DataType::Int64,
        };
        if !supported {
            return Ok(None);
        }
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let provider_fields = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            Field::new(
                format!("__cove_native_filecode_group_agg_{index}"),
                field.data_type().clone(),
                field.is_nullable(),
            )
        })
        .collect::<Vec<_>>();
    let provider_schema: SchemaRef = Arc::new(Schema::new(provider_fields));
    let table = CoveNativeFileCodeI64GroupAggregateTableProvider::new(
        Arc::clone(&provider_schema),
        Arc::clone(provider.state()),
        group_column_index,
        requests,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = provider_schema
        .fields()
        .iter()
        .zip(schema.fields().iter())
        .map(|(provider_field, output_field)| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                provider_field.name().clone(),
            ))
            .alias(output_field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_i64_group_count(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || aggregate.aggr_expr.len() != 1 {
        return Ok(None);
    }
    if !matches!(
        count_column_index(&aggregate.aggr_expr[0], provider),
        Some(None)
    ) {
        return Ok(None);
    }
    let Expr::Column(column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
    else {
        return Ok(None);
    };
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical != CoveLogicalType::Int64
        || cove_column.physical != CovePhysicalKind::NumCode
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != 2 || schema.field(0).data_type() != &DataType::Int64 {
        return Ok(None);
    }
    if !matches!(
        schema.field(1).data_type(),
        DataType::Int64 | DataType::UInt64
    ) {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::new(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = schema
        .fields()
        .iter()
        .map(|field| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                field.name().clone(),
            ))
            .alias(field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_bool_group_count(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || aggregate.aggr_expr.len() != 1 {
        return Ok(None);
    }
    if !matches!(
        count_column_index(&aggregate.aggr_expr[0], provider),
        Some(None)
    ) {
        return Ok(None);
    }
    let Expr::Column(column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
    else {
        return Ok(None);
    };
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical != CoveLogicalType::Bool
        || cove_column.physical != CovePhysicalKind::Boolean
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != 2 || schema.field(0).data_type() != &DataType::Boolean {
        return Ok(None);
    }
    if !matches!(
        schema.field(1).data_type(),
        DataType::Int64 | DataType::UInt64
    ) {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::bool_count(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = schema
        .fields()
        .iter()
        .map(|field| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                field.name().clone(),
            ))
            .alias(field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_filecode_group_count(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || aggregate.aggr_expr.len() != 1 {
        return Ok(None);
    }
    if !matches!(
        count_column_index(&aggregate.aggr_expr[0], provider),
        Some(None)
    ) {
        return Ok(None);
    }
    let Expr::Column(column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
    else {
        return Ok(None);
    };
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical != CoveLogicalType::Utf8
        || cove_column.physical != CovePhysicalKind::FileCode
    {
        return Ok(None);
    }
    if provider
        .state()
        .files()
        .iter()
        .any(|file| file.has_redaction())
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != 2 || schema.field(0).data_type() != &DataType::Utf8 {
        return Ok(None);
    }
    if !matches!(
        schema.field(1).data_type(),
        DataType::Int64 | DataType::UInt64
    ) {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::filecode_utf8_count(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    let projection_exprs = schema
        .fields()
        .iter()
        .map(|field| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                field.name().clone(),
            ))
            .alias(field.name().clone())
        })
        .collect::<Vec<_>>();
    let projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        aggregate.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(projection)))
}

fn rewrite_native_i64_distinct_group(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || !aggregate.aggr_expr.is_empty() {
        return Ok(None);
    }
    let Expr::Column(column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
    else {
        return Ok(None);
    };
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical != CoveLogicalType::Int64
        || cove_column.physical != CovePhysicalKind::NumCode
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::distinct(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    Ok(Some(LogicalPlan::TableScan(rewritten_scan)))
}

fn rewrite_native_bool_distinct_group(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || !aggregate.aggr_expr.is_empty() {
        return Ok(None);
    }
    let Expr::Column(column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
    else {
        return Ok(None);
    };
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical != CoveLogicalType::Bool
        || cove_column.physical != CovePhysicalKind::Boolean
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Boolean {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::bool_distinct(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    Ok(Some(LogicalPlan::TableScan(rewritten_scan)))
}

fn rewrite_native_filecode_distinct_group(
    aggregate: &datafusion::logical_expr::Aggregate,
    scan: &TableScan,
    filters: &[Expr],
    provider: &CoveTableProvider,
) -> Result<Option<LogicalPlan>> {
    if aggregate.group_expr.len() != 1 || !aggregate.aggr_expr.is_empty() {
        return Ok(None);
    }
    let Expr::Column(column) = &aggregate.group_expr[0] else {
        return Ok(None);
    };
    let Some(column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
    else {
        return Ok(None);
    };
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical != CoveLogicalType::Utf8
        || cove_column.physical != CovePhysicalKind::FileCode
    {
        return Ok(None);
    }
    if provider
        .state()
        .files()
        .iter()
        .any(|file| file.has_redaction())
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(aggregate.schema.as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Utf8 {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider, filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::filecode_utf8_distinct(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    Ok(Some(LogicalPlan::TableScan(rewritten_scan)))
}

enum DistinctColumnRef<'a> {
    Named(&'a str),
    ProjectedIndex(usize),
}

fn rewrite_native_i64_distinct_all(distinct: &Distinct) -> Result<Option<LogicalPlan>> {
    let Distinct::All(input) = distinct else {
        return Ok(None);
    };
    let Some((scan, filters, column_ref)) = distinct_all_scan_and_filters(input.as_ref()) else {
        return Ok(None);
    };
    let Some(provider) = cove_provider_from_scan(scan)? else {
        return Ok(None);
    };
    let column_index = match column_ref {
        DistinctColumnRef::Named(name) => {
            let Some(index) = provider
                .state()
                .table()
                .columns
                .iter()
                .position(|candidate| candidate.name == name)
            else {
                return Ok(None);
            };
            index
        }
        DistinctColumnRef::ProjectedIndex(index) => index,
    };
    let Some(cove_column) = provider.state().table().columns.get(column_index) else {
        return Ok(None);
    };
    if cove_column.logical != CoveLogicalType::Int64
        || cove_column.physical != CovePhysicalKind::NumCode
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(input.schema().as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider.as_ref(), &filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::distinct(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    Ok(Some(LogicalPlan::TableScan(rewritten_scan)))
}

fn rewrite_native_bool_distinct_all(distinct: &Distinct) -> Result<Option<LogicalPlan>> {
    let Distinct::All(input) = distinct else {
        return Ok(None);
    };
    let Some((scan, filters, column_ref)) = distinct_all_scan_and_filters(input.as_ref()) else {
        return Ok(None);
    };
    let Some(provider) = cove_provider_from_scan(scan)? else {
        return Ok(None);
    };
    let column_index = match column_ref {
        DistinctColumnRef::Named(name) => {
            let Some(index) = provider
                .state()
                .table()
                .columns
                .iter()
                .position(|candidate| candidate.name == name)
            else {
                return Ok(None);
            };
            index
        }
        DistinctColumnRef::ProjectedIndex(index) => index,
    };
    let Some(cove_column) = provider.state().table().columns.get(column_index) else {
        return Ok(None);
    };
    if cove_column.logical != CoveLogicalType::Bool
        || cove_column.physical != CovePhysicalKind::Boolean
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(input.schema().as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Boolean {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider.as_ref(), &filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::bool_distinct(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    Ok(Some(LogicalPlan::TableScan(rewritten_scan)))
}

fn rewrite_native_filecode_distinct_all(distinct: &Distinct) -> Result<Option<LogicalPlan>> {
    let Distinct::All(input) = distinct else {
        return Ok(None);
    };
    let Some((scan, filters, column_ref)) = distinct_all_scan_and_filters(input.as_ref()) else {
        return Ok(None);
    };
    let Some(provider) = cove_provider_from_scan(scan)? else {
        return Ok(None);
    };
    let column_index = match column_ref {
        DistinctColumnRef::Named(name) => {
            let Some(index) = provider
                .state()
                .table()
                .columns
                .iter()
                .position(|candidate| candidate.name == name)
            else {
                return Ok(None);
            };
            index
        }
        DistinctColumnRef::ProjectedIndex(index) => index,
    };
    let Some(cove_column) = provider.state().table().columns.get(column_index) else {
        return Ok(None);
    };
    if cove_column.logical != CoveLogicalType::Utf8
        || cove_column.physical != CovePhysicalKind::FileCode
    {
        return Ok(None);
    }
    if provider
        .state()
        .files()
        .iter()
        .any(|file| file.has_redaction())
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(input.schema().as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Utf8 {
        return Ok(None);
    }
    let Some(filter_plan) = native_exact_filter_plan(provider.as_ref(), &filters)? else {
        return Ok(None);
    };
    let table = CoveNativeGroupCountTableProvider::filecode_utf8_distinct(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    Ok(Some(LogicalPlan::TableScan(rewritten_scan)))
}

fn rewrite_native_i64_key_join_projection(projection: &Projection) -> Result<Option<LogicalPlan>> {
    let LogicalPlan::Join(join) = projection.input.as_ref() else {
        return Ok(None);
    };
    let Some(join_kind) = native_i64_join_kind(join) else {
        return Ok(None);
    };
    let [(left_join_expr, right_join_expr)] = join.on.as_slice() else {
        return Ok(None);
    };
    let Expr::Column(left_join_column) = left_join_expr else {
        return Ok(None);
    };
    let Expr::Column(right_join_column) = right_join_expr else {
        return Ok(None);
    };
    let schema: SchemaRef = Arc::new(projection.schema.as_arrow().clone());
    match join_kind {
        NativeI64JoinKind::Inner => {
            let [left_projected_expr, right_projected_expr] = projection.expr.as_slice() else {
                return Ok(None);
            };
            let Some(left_projected_column) = projected_column(left_projected_expr) else {
                return Ok(None);
            };
            let Some(right_projected_column) = projected_column(right_projected_expr) else {
                return Ok(None);
            };
            if left_projected_column != left_join_column
                || right_projected_column != right_join_column
            {
                return Ok(None);
            }
            if schema.fields().len() != 2
                || schema.field(0).data_type() != &DataType::Int64
                || schema.field(1).data_type() != &DataType::Int64
                || schema.field(0).name() == schema.field(1).name()
            {
                return Ok(None);
            }
        }
        NativeI64JoinKind::LeftSemi | NativeI64JoinKind::LeftAnti => {
            let [left_projected_expr] = projection.expr.as_slice() else {
                return Ok(None);
            };
            let Some(left_projected_column) = projected_column(left_projected_expr) else {
                return Ok(None);
            };
            if left_projected_column != left_join_column {
                return Ok(None);
            }
            if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
                return Ok(None);
            }
        }
    }
    let Some(scan) = native_i64_key_join_scan(join, Arc::clone(&schema), join_kind, None)? else {
        return Ok(None);
    };
    let projection_exprs = schema
        .fields()
        .iter()
        .map(|field| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                field.name().clone(),
            ))
            .alias(field.name().clone())
        })
        .collect::<Vec<_>>();
    let rewritten_projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(scan)),
        projection.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(rewritten_projection)))
}

fn rewrite_native_filecode_key_join_projection(
    projection: &Projection,
) -> Result<Option<LogicalPlan>> {
    let LogicalPlan::Join(join) = projection.input.as_ref() else {
        return Ok(None);
    };
    let Some(join_kind) = native_i64_join_kind(join) else {
        return Ok(None);
    };
    let [(left_join_expr, right_join_expr)] = join.on.as_slice() else {
        return Ok(None);
    };
    let Expr::Column(left_join_column) = left_join_expr else {
        return Ok(None);
    };
    let Expr::Column(right_join_column) = right_join_expr else {
        return Ok(None);
    };
    let schema: SchemaRef = Arc::new(projection.schema.as_arrow().clone());
    match join_kind {
        NativeI64JoinKind::Inner => {
            let [left_projected_expr, right_projected_expr] = projection.expr.as_slice() else {
                return Ok(None);
            };
            let Some(left_projected_column) = projected_column(left_projected_expr) else {
                return Ok(None);
            };
            let Some(right_projected_column) = projected_column(right_projected_expr) else {
                return Ok(None);
            };
            if left_projected_column != left_join_column
                || right_projected_column != right_join_column
            {
                return Ok(None);
            }
            if schema.fields().len() != 2
                || schema.field(0).data_type() != &DataType::Utf8
                || schema.field(1).data_type() != &DataType::Utf8
                || schema.field(0).name() == schema.field(1).name()
            {
                return Ok(None);
            }
        }
        NativeI64JoinKind::LeftSemi | NativeI64JoinKind::LeftAnti => {
            let [left_projected_expr] = projection.expr.as_slice() else {
                return Ok(None);
            };
            let Some(left_projected_column) = projected_column(left_projected_expr) else {
                return Ok(None);
            };
            if left_projected_column != left_join_column {
                return Ok(None);
            }
            if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Utf8 {
                return Ok(None);
            }
        }
    }
    let Some(scan) = native_filecode_key_join_scan(join, Arc::clone(&schema), join_kind, None)?
    else {
        return Ok(None);
    };
    let projection_exprs = schema
        .fields()
        .iter()
        .map(|field| {
            Expr::Column(Column::new(
                Some(scan.table_name.clone()),
                field.name().clone(),
            ))
            .alias(field.name().clone())
        })
        .collect::<Vec<_>>();
    let rewritten_projection = Projection::try_new_with_schema(
        projection_exprs,
        Arc::new(LogicalPlan::TableScan(scan)),
        projection.schema.clone(),
    )?;
    Ok(Some(LogicalPlan::Projection(rewritten_projection)))
}

fn rewrite_native_i64_key_semi_anti_join(join: &Join) -> Result<Option<LogicalPlan>> {
    let Some(join_kind @ (NativeI64JoinKind::LeftSemi | NativeI64JoinKind::LeftAnti)) =
        native_i64_join_kind(join)
    else {
        return Ok(None);
    };
    let schema: SchemaRef = Arc::new(join.schema.as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
        return Ok(None);
    }
    let table_name = join.schema.qualified_field(0).0.cloned();
    let Some(scan) = native_i64_key_join_scan(join, schema, join_kind, table_name)? else {
        return Ok(None);
    };
    Ok(Some(LogicalPlan::TableScan(scan)))
}

fn rewrite_native_filecode_key_semi_anti_join(join: &Join) -> Result<Option<LogicalPlan>> {
    let Some(join_kind @ (NativeI64JoinKind::LeftSemi | NativeI64JoinKind::LeftAnti)) =
        native_i64_join_kind(join)
    else {
        return Ok(None);
    };
    let schema: SchemaRef = Arc::new(join.schema.as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Utf8 {
        return Ok(None);
    }
    let table_name = join.schema.qualified_field(0).0.cloned();
    let Some(scan) = native_filecode_key_join_scan(join, schema, join_kind, table_name)? else {
        return Ok(None);
    };
    Ok(Some(LogicalPlan::TableScan(scan)))
}

fn native_i64_key_join_scan(
    join: &Join,
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    table_name: Option<TableReference>,
) -> Result<Option<TableScan>> {
    if native_i64_join_kind(join) != Some(join_kind)
        || join.join_constraint != JoinConstraint::On
        || join.null_equality != NullEquality::NullEqualsNothing
        || join.null_aware
        || join.filter.is_some()
    {
        return Ok(None);
    }
    let [(left_expr, right_expr)] = join.on.as_slice() else {
        return Ok(None);
    };
    let Expr::Column(left_column) = left_expr else {
        return Ok(None);
    };
    let Expr::Column(right_column) = right_expr else {
        return Ok(None);
    };
    let Some((left_scan, left_filters)) = join_input_scan_and_filters(join.left.as_ref()) else {
        return Ok(None);
    };
    let Some((right_scan, right_filters)) = join_input_scan_and_filters(join.right.as_ref()) else {
        return Ok(None);
    };
    if left_scan.fetch.is_some() || right_scan.fetch.is_some() {
        return Ok(None);
    }
    let Some(left_provider) = cove_provider_from_scan(left_scan)? else {
        return Ok(None);
    };
    let Some(right_provider) = cove_provider_from_scan(right_scan)? else {
        return Ok(None);
    };
    let Some(left_column_index) = native_i64_join_column_index(left_provider.as_ref(), left_column)
    else {
        return Ok(None);
    };
    let Some(right_column_index) =
        native_i64_join_column_index(right_provider.as_ref(), right_column)
    else {
        return Ok(None);
    };
    if !scan_projects_only_column(left_scan, left_provider.as_ref(), left_column_index)
        || !scan_projects_only_column(right_scan, right_provider.as_ref(), right_column_index)
    {
        return Ok(None);
    }
    let Some(left_filter_plan) = native_exact_filter_plan(left_provider.as_ref(), &left_filters)?
    else {
        return Ok(None);
    };
    let Some(right_filter_plan) =
        native_exact_filter_plan(right_provider.as_ref(), &right_filters)?
    else {
        return Ok(None);
    };

    let table = CoveNativeI64JoinTableProvider::new(
        schema,
        join_kind,
        Arc::clone(left_provider.state()),
        Arc::clone(right_provider.state()),
        left_column_index,
        right_column_index,
        left_filter_plan,
        right_filter_plan,
    );
    let table_name = table_name.unwrap_or_else(|| left_scan.table_name.clone());
    let source = provider_as_source(Arc::new(table));
    let rewritten_scan = TableScan::try_new(table_name, source, None, Vec::new(), None)?;
    Ok(Some(rewritten_scan))
}

fn native_filecode_key_join_scan(
    join: &Join,
    schema: SchemaRef,
    join_kind: NativeI64JoinKind,
    table_name: Option<TableReference>,
) -> Result<Option<TableScan>> {
    if native_i64_join_kind(join) != Some(join_kind)
        || join.join_constraint != JoinConstraint::On
        || join.null_equality != NullEquality::NullEqualsNothing
        || join.null_aware
        || join.filter.is_some()
    {
        return Ok(None);
    }
    let [(left_expr, right_expr)] = join.on.as_slice() else {
        return Ok(None);
    };
    let Expr::Column(left_column) = left_expr else {
        return Ok(None);
    };
    let Expr::Column(right_column) = right_expr else {
        return Ok(None);
    };
    let Some((left_scan, left_filters)) = join_input_scan_and_filters(join.left.as_ref()) else {
        return Ok(None);
    };
    let Some((right_scan, right_filters)) = join_input_scan_and_filters(join.right.as_ref()) else {
        return Ok(None);
    };
    if left_scan.fetch.is_some() || right_scan.fetch.is_some() {
        return Ok(None);
    }
    let Some(left_provider) = cove_provider_from_scan(left_scan)? else {
        return Ok(None);
    };
    let Some(right_provider) = cove_provider_from_scan(right_scan)? else {
        return Ok(None);
    };
    if left_provider
        .state()
        .files()
        .iter()
        .any(|file| file.has_redaction())
        || right_provider
            .state()
            .files()
            .iter()
            .any(|file| file.has_redaction())
    {
        return Ok(None);
    }
    let Some(left_column_index) =
        native_filecode_join_column_index(left_provider.as_ref(), left_column)
    else {
        return Ok(None);
    };
    let Some(right_column_index) =
        native_filecode_join_column_index(right_provider.as_ref(), right_column)
    else {
        return Ok(None);
    };
    if !scan_projects_only_column(left_scan, left_provider.as_ref(), left_column_index)
        || !scan_projects_only_column(right_scan, right_provider.as_ref(), right_column_index)
    {
        return Ok(None);
    }
    let Some(left_filter_plan) = native_exact_filter_plan(left_provider.as_ref(), &left_filters)?
    else {
        return Ok(None);
    };
    let Some(right_filter_plan) =
        native_exact_filter_plan(right_provider.as_ref(), &right_filters)?
    else {
        return Ok(None);
    };

    let table = CoveNativeFileCodeJoinTableProvider::new(
        schema,
        join_kind,
        Arc::clone(left_provider.state()),
        Arc::clone(right_provider.state()),
        left_column_index,
        right_column_index,
        left_filter_plan,
        right_filter_plan,
    );
    let table_name = table_name.unwrap_or_else(|| left_scan.table_name.clone());
    let source = provider_as_source(Arc::new(table));
    let rewritten_scan = TableScan::try_new(table_name, source, None, Vec::new(), None)?;
    Ok(Some(rewritten_scan))
}

fn native_i64_join_kind(join: &Join) -> Option<NativeI64JoinKind> {
    match join.join_type {
        JoinType::Inner => Some(NativeI64JoinKind::Inner),
        JoinType::LeftSemi => Some(NativeI64JoinKind::LeftSemi),
        JoinType::LeftAnti => Some(NativeI64JoinKind::LeftAnti),
        _ => None,
    }
}

fn join_input_scan_and_filters(input: &LogicalPlan) -> Option<(&TableScan, Vec<Expr>)> {
    match input {
        LogicalPlan::TableScan(scan) => Some((scan, dedup_filters(scan.filters.clone()))),
        LogicalPlan::SubqueryAlias(alias) => join_input_scan_and_filters(alias.input.as_ref()),
        LogicalPlan::Projection(projection)
            if projection
                .expr
                .iter()
                .all(|expr| projected_column(expr).is_some()) =>
        {
            join_input_scan_and_filters(projection.input.as_ref())
        }
        LogicalPlan::Filter(filter) => {
            let (scan, mut filters) = join_input_scan_and_filters(filter.input.as_ref())?;
            filters.push(filter.predicate.clone());
            Some((scan, dedup_filters(filters)))
        }
        _ => None,
    }
}

fn native_filecode_join_column_index(
    provider: &CoveTableProvider,
    column: &Column,
) -> Option<usize> {
    let column_index = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)?;
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical == CoveLogicalType::Utf8
        && cove_column.physical == CovePhysicalKind::FileCode
    {
        Some(column_index)
    } else {
        None
    }
}

fn native_i64_join_column_index(provider: &CoveTableProvider, column: &Column) -> Option<usize> {
    let column_index = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)?;
    let cove_column = &provider.state().table().columns[column_index];
    if cove_column.logical == CoveLogicalType::Int64
        && cove_column.physical == CovePhysicalKind::NumCode
    {
        Some(column_index)
    } else {
        None
    }
}

fn distinct_all_scan_and_filters(
    input: &LogicalPlan,
) -> Option<(&TableScan, Vec<Expr>, DistinctColumnRef<'_>)> {
    match input {
        LogicalPlan::Projection(projection) if projection.expr.len() == 1 => {
            let column = projected_column(&projection.expr[0])?;
            let (scan, filters) = aggregate_scan_and_filters(projection.input.as_ref())?;
            Some((
                scan,
                filters,
                DistinctColumnRef::Named(column.name.as_str()),
            ))
        }
        LogicalPlan::TableScan(scan) => {
            let projection = scan.projection.as_ref()?;
            let [column_index] = projection.as_slice() else {
                return None;
            };
            Some((
                scan,
                dedup_filters(scan.filters.clone()),
                DistinctColumnRef::ProjectedIndex(*column_index),
            ))
        }
        _ => None,
    }
}

fn projected_column(expr: &Expr) -> Option<&Column> {
    match expr {
        Expr::Column(column) => Some(column),
        Expr::Alias(alias) => match alias.expr.as_ref() {
            Expr::Column(column) => Some(column),
            _ => None,
        },
        _ => None,
    }
}

#[allow(deprecated)]
fn native_i64_aggregate_requests(
    aggregate: &datafusion::logical_expr::Aggregate,
    provider: &CoveTableProvider,
) -> Option<Vec<NativeI64AggregateRequest>> {
    let mut requests = Vec::with_capacity(aggregate.aggr_expr.len());
    for expr in &aggregate.aggr_expr {
        let Expr::AggregateFunction(func) = expr else {
            return None;
        };
        if func.params.distinct || func.params.filter.is_some() || !func.params.order_by.is_empty()
        {
            return None;
        }
        let kind = if func.func.name().eq_ignore_ascii_case("count") {
            NativeI64AggregateKind::Count
        } else if func.func.name().eq_ignore_ascii_case("sum") {
            NativeI64AggregateKind::Sum
        } else if func.func.name().eq_ignore_ascii_case("avg") {
            NativeI64AggregateKind::Avg
        } else if func.func.name().eq_ignore_ascii_case("min") {
            NativeI64AggregateKind::Min
        } else if func.func.name().eq_ignore_ascii_case("max") {
            NativeI64AggregateKind::Max
        } else {
            return None;
        };
        let [arg] = func.params.args.as_slice() else {
            return None;
        };
        let column = native_aggregate_column_arg(arg)?;
        let column_index = provider
            .state()
            .table()
            .columns
            .iter()
            .position(|candidate| candidate.name == column.name)?;
        let cove_column = &provider.state().table().columns[column_index];
        if cove_column.logical != CoveLogicalType::Int64
            || cove_column.physical != CovePhysicalKind::NumCode
        {
            return None;
        }
        requests.push(NativeI64AggregateRequest { column_index, kind });
    }
    if requests.is_empty() {
        None
    } else {
        Some(requests)
    }
}

fn native_aggregate_column_arg(expr: &Expr) -> Option<&datafusion::common::Column> {
    match expr {
        Expr::Column(column) => Some(column),
        Expr::Cast(cast) => native_aggregate_column_arg(cast.expr.as_ref()),
        _ => None,
    }
}

#[allow(deprecated)]
fn synopsis_aggregate_request(
    expr: &Expr,
    provider: &CoveTableProvider,
) -> Option<(usize, MetadataSynopsisAggregateKind)> {
    let Expr::AggregateFunction(func) = expr else {
        return None;
    };
    if func.params.distinct || func.params.filter.is_some() || !func.params.order_by.is_empty() {
        return None;
    }
    let aggregate_kind = if func.func.name().eq_ignore_ascii_case("min") {
        MetadataSynopsisAggregateKind::Min
    } else if func.func.name().eq_ignore_ascii_case("max") {
        MetadataSynopsisAggregateKind::Max
    } else if func.func.name().eq_ignore_ascii_case("sum") {
        MetadataSynopsisAggregateKind::Sum
    } else if func.func.name().eq_ignore_ascii_case("avg") {
        MetadataSynopsisAggregateKind::Avg
    } else {
        return None;
    };
    let [Expr::Column(column)] = func.params.args.as_slice() else {
        return None;
    };
    provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
        .map(|column_index| (column_index, aggregate_kind))
}

#[cfg(feature = "covi")]
#[allow(deprecated)]
fn index_only_scalar_value_request(
    expr: &Expr,
    provider: &CoveTableProvider,
) -> Option<(usize, CoviAggregateKindV2)> {
    let Expr::AggregateFunction(func) = expr else {
        return None;
    };
    if func.params.distinct || func.params.filter.is_some() || !func.params.order_by.is_empty() {
        return None;
    }
    let aggregate_kind = if func.func.name().eq_ignore_ascii_case("min") {
        CoviAggregateKindV2::Min
    } else if func.func.name().eq_ignore_ascii_case("max") {
        CoviAggregateKindV2::Max
    } else if func.func.name().eq_ignore_ascii_case("sum") {
        CoviAggregateKindV2::Sum
    } else if func.func.name().eq_ignore_ascii_case("avg") {
        CoviAggregateKindV2::Avg
    } else {
        return None;
    };
    let [arg] = func.params.args.as_slice() else {
        return None;
    };
    let column = aggregate_column_arg(arg)?;
    provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
        .map(|column_index| (column_index, aggregate_kind))
}

#[cfg(feature = "covi")]
fn aggregate_column_arg(expr: &Expr) -> Option<&datafusion::common::Column> {
    match expr {
        Expr::Column(column) => Some(column),
        Expr::Cast(cast) => aggregate_column_arg(cast.expr.as_ref()),
        _ => None,
    }
}

fn record_batch_for_metadata_plan(
    plan: &MetadataAggregatePlan,
    schema: SchemaRef,
) -> Result<Option<RecordBatch>> {
    let arrays = match plan {
        MetadataAggregatePlan::ScalarCounts { counts, .. } => {
            let mut arrays = Vec::with_capacity(counts.len());
            for (index, count) in counts.iter().enumerate() {
                let field = schema.field(index);
                arrays.push(count_array_for_type(*count, field.data_type())?);
            }
            arrays
        }
        MetadataAggregatePlan::ScalarValues { values, .. } => {
            let mut arrays = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                let field = schema.field(index);
                arrays.push(canonical_value_array(value, field.data_type())?);
            }
            arrays
        }
        MetadataAggregatePlan::FileCodeGroupCounts { groups, .. } => {
            if schema.fields().len() != 2 || schema.field(0).data_type() != &DataType::Utf8 {
                return Ok(None);
            }
            let labels = groups
                .iter()
                .map(|group| canonical_utf8(&group.canonical_value))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(crate::adapter_v53::cove_to_datafusion)?;
            vec![
                Arc::new(StringArray::from(labels)) as arrow_array::ArrayRef,
                count_array_for_values(
                    &groups.iter().map(|group| group.count).collect::<Vec<_>>(),
                    schema.field(1).data_type(),
                )?,
            ]
        }
    };
    let batch = RecordBatch::try_new(schema, arrays)
        .map_err(|err| DataFusionError::ArrowError(Box::new(err), None))?;
    Ok(Some(batch))
}

fn rewrite_topn_sort(sort: &datafusion::logical_expr::Sort) -> Result<Option<LogicalPlan>> {
    if sort.expr.len() != 1 {
        return Ok(None);
    }
    let Expr::Column(column) = &sort.expr[0].expr else {
        return Ok(None);
    };
    let LogicalPlan::TableScan(scan) = sort.input.as_ref() else {
        return Ok(None);
    };
    let Some(provider) = cove_provider_from_scan(scan)? else {
        return Ok(None);
    };
    let Some(column_index) = provider
        .state()
        .table()
        .columns
        .iter()
        .position(|candidate| candidate.name == column.name)
    else {
        return Ok(None);
    };
    if let Some(rewritten) =
        rewrite_native_i64_order_sort(sort, scan, provider.as_ref(), column_index)?
    {
        return Ok(Some(rewritten));
    }
    let Some(fetch) = sort.fetch else {
        return Ok(None);
    };
    let hint = TopNScanHint {
        column_index,
        descending: !sort.expr[0].asc,
        fetch,
    };
    if provider.topn_hint() == Some(hint) {
        return Ok(None);
    }
    let hinted_provider = Arc::new(provider.with_topn_hint(hint));
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(hinted_provider),
        scan.projection.clone(),
        scan.filters.clone(),
        scan.fetch,
    )?;
    Ok(Some(LogicalPlan::Sort(datafusion::logical_expr::Sort {
        expr: sort.expr.clone(),
        input: Arc::new(LogicalPlan::TableScan(rewritten_scan)),
        fetch: sort.fetch,
    })))
}

fn rewrite_native_i64_order_sort(
    sort: &datafusion::logical_expr::Sort,
    scan: &TableScan,
    provider: &CoveTableProvider,
    column_index: usize,
) -> Result<Option<LogicalPlan>> {
    if scan.fetch.is_some() || !scan_projects_only_column(scan, provider, column_index) {
        return Ok(None);
    }
    let Some(cove_column) = provider.state().table().columns.get(column_index) else {
        return Ok(None);
    };
    if cove_column.logical != CoveLogicalType::Int64
        || cove_column.physical != CovePhysicalKind::NumCode
    {
        return Ok(None);
    }
    let schema: SchemaRef = Arc::new(sort.input.schema().as_arrow().clone());
    if schema.fields().len() != 1 || schema.field(0).data_type() != &DataType::Int64 {
        return Ok(None);
    }
    let filters = dedup_filters(scan.filters.clone());
    let Some(filter_plan) = native_exact_filter_plan(provider, &filters)? else {
        return Ok(None);
    };
    let table = CoveNativeI64OrderTableProvider::new(
        Arc::clone(&schema),
        Arc::clone(provider.state()),
        column_index,
        filter_plan,
        !sort.expr[0].asc,
        sort.expr[0].nulls_first,
        sort.fetch,
    );
    let rewritten_scan = TableScan::try_new(
        scan.table_name.clone(),
        provider_as_source(Arc::new(table)),
        None,
        Vec::new(),
        None,
    )?;
    Ok(Some(LogicalPlan::TableScan(rewritten_scan)))
}

fn scan_projects_only_column(
    scan: &TableScan,
    provider: &CoveTableProvider,
    column_index: usize,
) -> bool {
    match scan.projection.as_deref() {
        Some([projected]) => *projected == column_index,
        Some(_) => false,
        None => provider.state().table().columns.len() == 1 && column_index == 0,
    }
}

fn cove_provider_from_scan(scan: &TableScan) -> Result<Option<Arc<CoveTableProvider>>> {
    let provider = source_as_provider(&scan.source)?;
    let Some(cove) = provider.as_any().downcast_ref::<CoveTableProvider>() else {
        return Ok(None);
    };
    Ok(Some(Arc::new(cove.clone())))
}

#[allow(deprecated)]
fn count_column_index(expr: &Expr, provider: &CoveTableProvider) -> Option<Option<usize>> {
    let Expr::AggregateFunction(func) = expr else {
        return None;
    };
    if !func.func.name().eq_ignore_ascii_case("count")
        || func.params.distinct
        || func.params.filter.is_some()
        || !func.params.order_by.is_empty()
    {
        return None;
    }
    match func.params.args.as_slice() {
        [] => Some(None),
        [Expr::Wildcard { .. }] => Some(None),
        [Expr::Literal(value, _)] if !value.is_null() => Some(None),
        [Expr::Column(column)] => provider
            .state()
            .table()
            .columns
            .iter()
            .position(|candidate| candidate.name == column.name)
            .map(Some),
        _ => None,
    }
}

#[cfg(feature = "covi")]
fn count_distinct_column_index(expr: &Expr, provider: &CoveTableProvider) -> Option<usize> {
    let Expr::AggregateFunction(func) = expr else {
        return None;
    };
    if !func.func.name().eq_ignore_ascii_case("count")
        || !func.params.distinct
        || func.params.filter.is_some()
        || !func.params.order_by.is_empty()
    {
        return None;
    }
    match func.params.args.as_slice() {
        [Expr::Column(column)] => provider
            .state()
            .table()
            .columns
            .iter()
            .position(|candidate| candidate.name == column.name),
        _ => None,
    }
}

fn count_array_for_type(count: u64, data_type: &DataType) -> Result<arrow_array::ArrayRef> {
    let scalar =
        match data_type {
            DataType::Int64 => ScalarValue::Int64(Some(i64::try_from(count).map_err(|_| {
                DataFusionError::Plan("metadata COUNT result exceeds Int64".into())
            })?)),
            DataType::UInt64 => ScalarValue::UInt64(Some(count)),
            DataType::Int32 => ScalarValue::Int32(Some(i32::try_from(count).map_err(|_| {
                DataFusionError::Plan("metadata COUNT result exceeds Int32".into())
            })?)),
            DataType::UInt32 => ScalarValue::UInt32(Some(u32::try_from(count).map_err(|_| {
                DataFusionError::Plan("metadata COUNT result exceeds UInt32".into())
            })?)),
            other => {
                return Err(DataFusionError::Plan(format!(
                    "unsupported metadata COUNT output type {other:?}"
                )));
            }
        };
    scalar.to_array()
}

fn canonical_value_array(
    value: &MetadataAggregateValue,
    data_type: &DataType,
) -> Result<arrow_array::ArrayRef> {
    canonical_scalar_value(value, data_type)?.to_array()
}

fn canonical_scalar_value(
    value: &MetadataAggregateValue,
    data_type: &DataType,
) -> Result<ScalarValue> {
    let Some(bytes) = value.canonical_value.as_deref() else {
        return null_scalar_for_type(data_type);
    };
    match (value.logical, data_type) {
        (
            CoveLogicalType::Int8
            | CoveLogicalType::Int16
            | CoveLogicalType::Int32
            | CoveLogicalType::Int64,
            DataType::Int64,
        ) => Ok(ScalarValue::Int64(Some(i64::from_le_bytes(fixed_bytes(
            bytes,
        )?)))),
        (CoveLogicalType::Int32, DataType::Int32) => Ok(ScalarValue::Int32(Some(
            i32::try_from(i64::from_le_bytes(fixed_bytes(bytes)?))
                .map_err(|_| DataFusionError::Plan("COVI min/max value exceeds Int32".into()))?,
        ))),
        (
            CoveLogicalType::UInt8
            | CoveLogicalType::UInt16
            | CoveLogicalType::UInt32
            | CoveLogicalType::UInt64,
            DataType::UInt64,
        ) => Ok(ScalarValue::UInt64(Some(u64::from_le_bytes(fixed_bytes(
            bytes,
        )?)))),
        (CoveLogicalType::UInt32, DataType::UInt32) => Ok(ScalarValue::UInt32(Some(
            u32::try_from(u64::from_le_bytes(fixed_bytes(bytes)?))
                .map_err(|_| DataFusionError::Plan("COVI min/max value exceeds UInt32".into()))?,
        ))),
        (CoveLogicalType::Float32, DataType::Float32) => Ok(ScalarValue::Float32(Some(
            f32::from_bits(u32::from_le_bytes(fixed_bytes(bytes)?)),
        ))),
        (CoveLogicalType::Float64, DataType::Float64) => Ok(ScalarValue::Float64(Some(
            f64::from_bits(u64::from_le_bytes(fixed_bytes(bytes)?)),
        ))),
        (CoveLogicalType::Decimal128, DataType::Decimal128(precision, scale)) => {
            Ok(ScalarValue::Decimal128(
                Some(i128::from_le_bytes(fixed_bytes(bytes)?)),
                *precision,
                *scale,
            ))
        }
        (CoveLogicalType::DateDays, DataType::Date32) => Ok(ScalarValue::Date32(Some(
            i32::from_le_bytes(fixed_bytes(bytes)?),
        ))),
        (CoveLogicalType::TimestampMicros, DataType::Timestamp(TimeUnit::Microsecond, tz)) => {
            Ok(ScalarValue::TimestampMicrosecond(
                Some(i64::from_le_bytes(fixed_bytes(bytes)?)),
                tz.clone(),
            ))
        }
        (CoveLogicalType::TimestampNanos, DataType::Timestamp(TimeUnit::Nanosecond, tz)) => {
            Ok(ScalarValue::TimestampNanosecond(
                Some(i64::from_le_bytes(fixed_bytes(bytes)?)),
                tz.clone(),
            ))
        }
        (CoveLogicalType::Utf8, DataType::Utf8) => Ok(ScalarValue::Utf8(Some(
            canonical_utf8(bytes).map_err(crate::adapter_v53::cove_to_datafusion)?,
        ))),
        (CoveLogicalType::Utf8, DataType::LargeUtf8) => Ok(ScalarValue::LargeUtf8(Some(
            canonical_utf8(bytes).map_err(crate::adapter_v53::cove_to_datafusion)?,
        ))),
        other => Err(DataFusionError::Plan(format!(
            "unsupported COVI min/max output type {other:?}"
        ))),
    }
}

fn null_scalar_for_type(data_type: &DataType) -> Result<ScalarValue> {
    match data_type {
        DataType::Int32 => Ok(ScalarValue::Int32(None)),
        DataType::Int64 => Ok(ScalarValue::Int64(None)),
        DataType::UInt32 => Ok(ScalarValue::UInt32(None)),
        DataType::UInt64 => Ok(ScalarValue::UInt64(None)),
        DataType::Float32 => Ok(ScalarValue::Float32(None)),
        DataType::Float64 => Ok(ScalarValue::Float64(None)),
        DataType::Decimal128(precision, scale) => {
            Ok(ScalarValue::Decimal128(None, *precision, *scale))
        }
        DataType::Date32 => Ok(ScalarValue::Date32(None)),
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            Ok(ScalarValue::TimestampMicrosecond(None, tz.clone()))
        }
        DataType::Timestamp(TimeUnit::Nanosecond, tz) => {
            Ok(ScalarValue::TimestampNanosecond(None, tz.clone()))
        }
        DataType::Utf8 => Ok(ScalarValue::Utf8(None)),
        DataType::LargeUtf8 => Ok(ScalarValue::LargeUtf8(None)),
        other => Err(DataFusionError::Plan(format!(
            "unsupported COVI null min/max output type {other:?}"
        ))),
    }
}

fn fixed_bytes<const N: usize>(bytes: &[u8]) -> Result<[u8; N]> {
    bytes.try_into().map_err(|_| {
        DataFusionError::Plan(format!(
            "COVI min/max canonical value has {} bytes, expected {N}",
            bytes.len()
        ))
    })
}

fn count_array_for_values(counts: &[u64], data_type: &DataType) -> Result<arrow_array::ArrayRef> {
    match data_type {
        DataType::Int64 => counts
            .iter()
            .map(|count| {
                i64::try_from(*count).map_err(|_| {
                    DataFusionError::Plan("metadata COUNT result exceeds Int64".into())
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| Arc::new(Int64Array::from(values)) as arrow_array::ArrayRef),
        DataType::UInt64 => {
            Ok(Arc::new(UInt64Array::from(counts.to_vec())) as arrow_array::ArrayRef)
        }
        DataType::Int32 => counts
            .iter()
            .map(|count| {
                i32::try_from(*count).map_err(|_| {
                    DataFusionError::Plan("metadata COUNT result exceeds Int32".into())
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| Arc::new(Int32Array::from(values)) as arrow_array::ArrayRef),
        DataType::UInt32 => counts
            .iter()
            .map(|count| {
                u32::try_from(*count).map_err(|_| {
                    DataFusionError::Plan("metadata COUNT result exceeds UInt32".into())
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| Arc::new(UInt32Array::from(values)) as arrow_array::ArrayRef),
        other => Err(DataFusionError::Plan(format!(
            "unsupported metadata COUNT output type {other:?}"
        ))),
    }
}
