//! Public registration helpers and thin session glue.

use std::{collections::BTreeSet, path::Path, sync::Arc};

use datafusion::{
    common::Result, datasource::listing::ListingOptions, execution::context::SessionContext,
};

use crate::{
    adapter_v53::{
        cove_to_datafusion,
        file_format::{CoveFileFormat, CoveFormatFactory, CoveTableFactory},
        optimizer::install_cove_optimizer,
        table_provider::CoveTableProvider,
    },
    bootstrap::{
        bootstrap_local_file, bootstrap_local_file_async, bootstrap_local_file_with_options,
        bootstrap_local_file_with_options_async, bootstrap_overlay_snapshot_with_options,
        bootstrap_overlay_snapshot_with_options_async,
    },
    overlay::CoveOverlaySnapshot,
    projection_provider::CoveProjectionTableProvider,
};
use cove_map::{projection_descriptors_from_cove_o_path, ProjectionDescriptor};

#[cfg(feature = "covm")]
use crate::bootstrap::{
    bootstrap_covm_local_file_with_options, bootstrap_covm_local_file_with_options_async,
};

pub use crate::options::{
    CoveTableOptions, CoviDiscovery, CovmTrustPolicy, CovxDiscovery, ExecutionCodePolicy,
    FilterResidualPolicy, SidecarDigestPolicy,
};
pub use datafusion as df;

#[derive(Debug, Clone)]
pub struct RegisteredCoveProjection {
    pub table_name: String,
    pub projection_id: String,
    pub output_table: Option<String>,
    pub provider: Arc<CoveProjectionTableProvider>,
}

/// Build a DataFusion table provider for a local `.cove` file.
///
/// This synchronous convenience wrapper blocks the current thread.
pub fn cove_table_from_path(path: impl AsRef<Path>) -> Result<Arc<CoveTableProvider>> {
    let state = bootstrap_local_file(path).map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

/// Build a DataFusion table provider for a local `.cove` file.
pub async fn cove_table_from_path_async(path: impl AsRef<Path>) -> Result<Arc<CoveTableProvider>> {
    let state = bootstrap_local_file_async(path)
        .await
        .map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

/// Build a DataFusion table provider for a local `.cove` file with explicit
/// COVE table options.
///
/// This synchronous convenience wrapper blocks the current thread.
pub fn cove_table_from_path_with_options(
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    let state = bootstrap_local_file_with_options(path, options).map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

/// Build a DataFusion table provider for a local `.cove` file with explicit
/// COVE table options.
pub async fn cove_table_from_path_with_options_async(
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    let state = bootstrap_local_file_with_options_async(path, options)
        .await
        .map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

/// Build a DataFusion table provider for an overlay snapshot.
pub fn cove_table_from_overlay_snapshot(
    snapshot: CoveOverlaySnapshot,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    let state =
        bootstrap_overlay_snapshot_with_options(snapshot, options).map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

/// Build a DataFusion table provider for an overlay snapshot.
pub async fn cove_table_from_overlay_snapshot_async(
    snapshot: CoveOverlaySnapshot,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    let state = bootstrap_overlay_snapshot_with_options_async(snapshot, options)
        .await
        .map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

#[cfg(feature = "covm")]
pub fn cove_table_from_covm_path(path: impl AsRef<Path>) -> Result<Arc<CoveTableProvider>> {
    cove_table_from_covm_path_with_options(path, CoveTableOptions::default())
}

#[cfg(feature = "covm")]
pub async fn cove_table_from_covm_path_async(
    path: impl AsRef<Path>,
) -> Result<Arc<CoveTableProvider>> {
    cove_table_from_covm_path_with_options_async(path, CoveTableOptions::default()).await
}

#[cfg(feature = "covm")]
pub fn cove_table_from_covm_path_with_options(
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    let state =
        bootstrap_covm_local_file_with_options(path, options).map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

#[cfg(feature = "covm")]
pub async fn cove_table_from_covm_path_with_options_async(
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    let state = bootstrap_covm_local_file_with_options_async(path, options)
        .await
        .map_err(cove_to_datafusion)?;
    Ok(Arc::new(CoveTableProvider::new(state)))
}

/// Register a local `.cove` file as a DataFusion table.
///
/// This synchronous convenience wrapper blocks the current thread while it
/// builds the table provider.
pub fn register_cove_file(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_path(path)?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

/// Register a local `.cove` file as a DataFusion table.
pub async fn register_cove_file_async(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_path_async(path).await?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

/// Register a local `.cove` file as a DataFusion table with explicit COVE
/// table options.
///
/// This synchronous convenience wrapper blocks the current thread while it
/// builds the table provider.
pub fn register_cove_file_with_options(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_path_with_options(path, options)?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

/// Register a local `.cove` file as a DataFusion table with explicit COVE
/// table options.
pub async fn register_cove_file_with_options_async(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_path_with_options_async(path, options).await?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

pub fn register_cove_overlay_snapshot(
    ctx: &SessionContext,
    table_name: &str,
    snapshot: CoveOverlaySnapshot,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_overlay_snapshot(snapshot, options)?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

pub async fn register_cove_overlay_snapshot_async(
    ctx: &SessionContext,
    table_name: &str,
    snapshot: CoveOverlaySnapshot,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_overlay_snapshot_async(snapshot, options).await?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

/// Build a DataFusion table provider for a persisted COVE-O projection.
pub fn cove_o_projection_table_from_path(
    object_path: impl AsRef<Path>,
    mapping_path: Option<&Path>,
    projection_id: &str,
) -> Result<Arc<CoveProjectionTableProvider>> {
    let object_path = object_path.as_ref().to_path_buf();
    let mapping_path = mapping_path.map(Path::to_path_buf);
    let descriptor = projection_descriptor(&object_path, mapping_path.as_deref(), projection_id)?;
    Ok(Arc::new(CoveProjectionTableProvider::try_new(
        object_path,
        mapping_path,
        descriptor,
    )?))
}

/// Build a DataFusion table provider for a persisted COVE-O projection.
pub async fn cove_o_projection_table_from_path_async(
    object_path: impl AsRef<Path>,
    mapping_path: Option<&Path>,
    projection_id: &str,
) -> Result<Arc<CoveProjectionTableProvider>> {
    cove_o_projection_table_from_path(object_path, mapping_path, projection_id)
}

/// Register one persisted COVE-O projection as a DataFusion table.
pub fn register_cove_o_projection(
    ctx: &SessionContext,
    table_name: &str,
    object_path: impl AsRef<Path>,
    mapping_path: Option<&Path>,
    projection_id: &str,
) -> Result<Arc<CoveProjectionTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_o_projection_table_from_path(object_path, mapping_path, projection_id)?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

/// Register one persisted COVE-O projection as a DataFusion table.
pub async fn register_cove_o_projection_async(
    ctx: &SessionContext,
    table_name: &str,
    object_path: impl AsRef<Path>,
    mapping_path: Option<&Path>,
    projection_id: &str,
) -> Result<Arc<CoveProjectionTableProvider>> {
    register_cove_o_projection(ctx, table_name, object_path, mapping_path, projection_id)
}

/// Register all Arrow-capable persisted COVE-O projections as DataFusion tables.
pub fn register_cove_o_projections(
    ctx: &SessionContext,
    object_path: impl AsRef<Path>,
    mapping_path: Option<&Path>,
    prefix: Option<&str>,
) -> Result<Vec<RegisteredCoveProjection>> {
    install_cove_optimizer(ctx);
    let object_path = object_path.as_ref().to_path_buf();
    let mapping_path = mapping_path.map(Path::to_path_buf);
    let descriptors = supported_projection_descriptors(&object_path, mapping_path.as_deref())?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        let table_name = projection_table_name(prefix, &descriptor);
        if !seen.insert(table_name.clone()) {
            return Err(datafusion::common::DataFusionError::Execution(format!(
                "duplicate registered projection table name '{table_name}'"
            )));
        }
        let provider = Arc::new(CoveProjectionTableProvider::try_new(
            object_path.clone(),
            mapping_path.clone(),
            descriptor.clone(),
        )?);
        ctx.register_table(&table_name, provider.clone())?;
        out.push(RegisteredCoveProjection {
            table_name,
            projection_id: descriptor.projection_id.clone(),
            output_table: descriptor.output_table.clone(),
            provider,
        });
    }
    Ok(out)
}

/// Register all Arrow-capable persisted COVE-O projections as DataFusion tables.
pub async fn register_cove_o_projections_async(
    ctx: &SessionContext,
    object_path: impl AsRef<Path>,
    mapping_path: Option<&Path>,
    prefix: Option<&str>,
) -> Result<Vec<RegisteredCoveProjection>> {
    register_cove_o_projections(ctx, object_path, mapping_path, prefix)
}

#[cfg(feature = "covm")]
pub fn register_cove_covm(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_covm_path(path)?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

#[cfg(feature = "covm")]
pub async fn register_cove_covm_async(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_covm_path_async(path).await?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

#[cfg(feature = "covm")]
pub fn register_cove_covm_with_options(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_covm_path_with_options(path, options)?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

#[cfg(feature = "covm")]
pub async fn register_cove_covm_with_options_async(
    ctx: &SessionContext,
    table_name: &str,
    path: impl AsRef<Path>,
    options: CoveTableOptions,
) -> Result<Arc<CoveTableProvider>> {
    install_cove_optimizer(ctx);
    let provider = cove_table_from_covm_path_with_options_async(path, options).await?;
    ctx.register_table(table_name, provider.clone())?;
    Ok(provider)
}

fn projection_descriptor(
    object_path: &Path,
    mapping_path: Option<&Path>,
    projection_id: &str,
) -> Result<ProjectionDescriptor> {
    let descriptors =
        projection_descriptors_from_cove_o_path(object_path, mapping_path).map_err(|err| {
            datafusion::common::DataFusionError::Execution(format!(
                "cannot inspect projections for {}: {err}",
                object_path.display()
            ))
        })?;
    let descriptor = descriptors
        .into_iter()
        .find(|descriptor| descriptor.projection_id == projection_id)
        .ok_or_else(|| {
            datafusion::common::DataFusionError::Execution(format!(
                "projection_id '{projection_id}' was not found for {}",
                object_path.display()
            ))
        })?;
    ensure_projection_supports_arrow(object_path, &descriptor)?;
    Ok(descriptor)
}

fn supported_projection_descriptors(
    object_path: &Path,
    mapping_path: Option<&Path>,
) -> Result<Vec<ProjectionDescriptor>> {
    let descriptors =
        projection_descriptors_from_cove_o_path(object_path, mapping_path).map_err(|err| {
            datafusion::common::DataFusionError::Execution(format!(
                "cannot inspect projections for {}: {err}",
                object_path.display()
            ))
        })?;
    let supported = descriptors
        .into_iter()
        .filter(|descriptor| descriptor.output_modes.iter().any(|mode| mode == "arrow"))
        .collect::<Vec<_>>();
    if supported.is_empty() {
        return Err(datafusion::common::DataFusionError::Execution(format!(
            "{} exposes no projections declaring Arrow output mode",
            object_path.display()
        )));
    }
    Ok(supported)
}

fn ensure_projection_supports_arrow(
    object_path: &Path,
    descriptor: &ProjectionDescriptor,
) -> Result<()> {
    if descriptor.output_modes.iter().any(|mode| mode == "arrow") {
        Ok(())
    } else {
        Err(datafusion::common::DataFusionError::Execution(format!(
            "projection '{}' on {} does not declare Arrow output mode",
            descriptor.projection_id,
            object_path.display()
        )))
    }
}

fn projection_table_name(prefix: Option<&str>, descriptor: &ProjectionDescriptor) -> String {
    let base = descriptor
        .output_table
        .as_deref()
        .unwrap_or(&descriptor.projection_id);
    let normalized = normalize_identifier(base);
    match prefix {
        Some(prefix) if !prefix.is_empty() => {
            format!("{}__{}", normalize_identifier(prefix), normalized)
        }
        _ => normalized,
    }
}

fn normalize_identifier(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut prev_underscore = false;
    for ch in value.chars() {
        let normalized = if ch.is_ascii_alphanumeric() { ch } else { '_' };
        if normalized == '_' {
            if prev_underscore {
                continue;
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
        out.push(normalized.to_ascii_lowercase());
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "projection".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Build DataFusion listing options for `.cove` compatibility-mode datasets.
pub fn cove_listing_options(options: CoveTableOptions) -> ListingOptions {
    ListingOptions::new(Arc::new(CoveFileFormat::new(options))).with_file_extension("cove")
}

/// Register a directory, file, or object-store listing of `.cove` files through
/// DataFusion's file-format compatibility path.
pub async fn register_cove_listing_table(
    ctx: &SessionContext,
    table_name: &str,
    table_path: impl AsRef<str>,
) -> Result<()> {
    register_cove_listing_table_with_options(
        ctx,
        table_name,
        table_path,
        CoveTableOptions::default(),
    )
    .await
}

/// Register a `.cove` listing table with explicit COVE table options.
pub async fn register_cove_listing_table_with_options(
    ctx: &SessionContext,
    table_name: &str,
    table_path: impl AsRef<str>,
    options: CoveTableOptions,
) -> Result<()> {
    install_cove_optimizer(ctx);
    ctx.register_listing_table(
        table_name,
        table_path.as_ref(),
        cove_listing_options(options),
        None,
        None,
    )
    .await
}

/// Register COVE as a SQL external-table file format for this context.
///
/// After this call, DataFusion SQL can use `STORED AS COVE`.
pub fn register_cove_file_format(ctx: &SessionContext) -> Result<()> {
    install_cove_optimizer(ctx);
    let state_ref = ctx.state_ref();
    let mut state = state_ref.write();
    state.register_file_format(Arc::new(CoveFormatFactory), true)?;
    state
        .table_factories_mut()
        .insert("COVE".into(), Arc::new(CoveTableFactory::new()));
    Ok(())
}
