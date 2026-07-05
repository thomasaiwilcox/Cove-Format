use std::{fs, sync::Arc};

use cove::{
    explain_query_file, query_discovery_manifest_file, query_file, render_query_discovery_template,
    ExplainOptions, QueryOptions,
};
use cove_core::query_discovery::{
    query_discovery_validation_context_for_source, validate_query_discovery_manifest,
    MetadataDisclosureMode, QueryDiscoveryOptions, QueryDiscoveryValidationFlag,
    QueryDiscoveryValidationReport, QueryDiscoveryValidationStatus,
};
use coveql::{parse_resolve_and_plan_query, ExplainMode};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ListResourcesResult, ReadResourceRequestParams, ReadResourceResult,
        Resource, ResourceContents, ServerCapabilities, ServerInfo,
    },
    schemars::JsonSchema,
    tool, tool_handler, tool_router, ErrorData, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    config::{CoveMcpConfig, CoveMcpError, SourceRef},
    results::ResultStore,
};

#[derive(Clone)]
pub struct CoveMcpServer {
    config: Arc<CoveMcpConfig>,
    results: Arc<ResultStore>,
    tool_router: ToolRouter<Self>,
}

impl std::fmt::Debug for CoveMcpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoveMcpServer").finish_non_exhaustive()
    }
}

impl CoveMcpServer {
    pub fn new(config: Arc<CoveMcpConfig>) -> Self {
        let results = Arc::new(ResultStore::new(
            config.result_ttl,
            config.max_result_handles,
        ));
        Self::with_result_store(config, results)
    }

    pub fn with_result_store(config: Arc<CoveMcpConfig>, results: Arc<ResultStore>) -> Self {
        Self {
            config,
            results,
            tool_router: Self::tool_router(),
        }
    }

    fn query_discovery_options(&self, include_ai: bool) -> QueryDiscoveryOptions {
        let disclosure_mode = if self.config.developer_mode {
            MetadataDisclosureMode::Developer
        } else {
            MetadataDisclosureMode::Public
        };
        let label = match disclosure_mode {
            MetadataDisclosureMode::Public => "public",
            MetadataDisclosureMode::Developer => "developer",
        };
        QueryDiscoveryOptions {
            disclosure_mode,
            principal_class: Some(label.to_string()),
            audience: Some(label.to_string()),
            include_ai,
            include_developer_diagnostics: self.config.developer_mode,
            ..QueryDiscoveryOptions::default()
        }
    }

    fn resolve_source(&self, source: &SourceRef) -> Result<std::path::PathBuf, CoveMcpError> {
        self.config.resolve_source(source)
    }

    fn discovery_with_validation(
        &self,
        input: &SourceRef,
        include_ai: bool,
    ) -> Result<Value, CoveMcpError> {
        let path = self.resolve_source(input)?;
        let mut options = self.query_discovery_options(include_ai);
        options.source_name = Some(path.display().to_string());
        let manifest = query_discovery_manifest_file(&path, options.clone())
            .map_err(|error| CoveMcpError::Query(error.to_string()))?;
        let bytes = fs::read(&path)?;
        let context = query_discovery_validation_context_for_source(&bytes, &options)
            .map_err(|error| CoveMcpError::Query(error.to_string()))?;
        let validation = validate_query_discovery_manifest(&manifest, context);
        let diagnostics_handle = self.store_diagnostics(&validation);
        Ok(json!({
            "source": input,
            "manifest": manifest.value(),
            "validation": validation_report_json(&validation),
            "diagnostics_handle": diagnostics_handle,
        }))
    }
}

#[tool_router(router = tool_router)]
impl CoveMcpServer {
    #[tool(
        name = "cove_discover_query_surface",
        description = "Build a COVE-QD query discovery manifest and external validation report for a configured COVE source."
    )]
    async fn discover_query_surface(
        &self,
        request: Parameters<DiscoverRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        Ok(tool_result(self.discovery_with_validation(
            &request.source,
            request.include_ai.unwrap_or(false),
        )))
    }

    #[tool(
        name = "cove_validate_query_discovery_manifest",
        description = "Validate generated or embedded COVE-QD metadata against a configured COVE source."
    )]
    async fn validate_query_discovery_manifest_tool(
        &self,
        request: Parameters<ValidateManifestRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let path = self.map_source_error(self.resolve_source(&request.source))?;
        let mut options = self.query_discovery_options(false);
        options.source_name = Some(path.display().to_string());
        let bytes = fs::read(&path).map_err(to_invalid_params)?;
        let manifest = match &request.manifest {
            Some(value) => {
                let canonical = cove::canonical_query_discovery_json(value)
                    .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
                cove::QueryDiscoveryManifest::parse(&canonical)
                    .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?
            }
            None => query_discovery_manifest_file(&path, options.clone())
                .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?,
        };
        let context = query_discovery_validation_context_for_source(&bytes, &options)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let validation = validate_query_discovery_manifest(&manifest, context);
        let diagnostics_handle = self.store_diagnostics(&validation);
        Ok(CallToolResult::structured(json!({
            "source": request.source,
            "validation": validation_report_json(&validation),
            "diagnostics_handle": diagnostics_handle,
        })))
    }

    #[tool(
        name = "cove_list_query_templates",
        description = "List safe COVE-QD query templates for a configured COVE source."
    )]
    async fn list_query_templates(
        &self,
        request: Parameters<DiscoverRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let path = self.map_source_error(self.resolve_source(&request.source))?;
        let mut options = self.query_discovery_options(request.include_ai.unwrap_or(false));
        options.source_name = Some(path.display().to_string());
        let manifest = query_discovery_manifest_file(&path, options)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(CallToolResult::structured(json!({
            "source": request.source,
            "templates": manifest.value().get("templates").cloned().unwrap_or_else(|| json!([])),
            "resource_budgets": manifest.value().get("resource_budgets").cloned().unwrap_or_else(|| json!({})),
            "policy": manifest.value().get("policy").cloned().unwrap_or_else(|| json!({})),
        })))
    }

    #[tool(
        name = "cove_render_query_template",
        description = "Render CoveQL from a safe COVE-QD operator-chain template and typed parameters, then dry-run plan it."
    )]
    async fn render_query_template(
        &self,
        request: Parameters<RenderTemplateRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let path = self.map_source_error(self.resolve_source(&request.source))?;
        let mut options = self.query_discovery_options(false);
        options.source_name = Some(path.display().to_string());
        let manifest = query_discovery_manifest_file(&path, options)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let params = request
            .params
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        let query = render_query_discovery_template(&manifest, &request.template_id, &params)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let prepared = self.prepare_query(&query)?;
        let bytes = fs::read(&path).map_err(to_invalid_params)?;
        parse_resolve_and_plan_query(
            &bytes,
            &prepared,
            coveql::ParseOptions::default(),
            coveql::ResolveOptions::default(),
            coveql::PlanOptions::default(),
            cove::reader::ValidationOptions::default(),
        )
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(CallToolResult::structured(json!({
            "source": request.source,
            "template_id": request.template_id,
            "query": prepared,
            "query_validation": "planned_dry_run",
        })))
    }

    #[tool(
        name = "cove_validate_query",
        description = "Parse, resolve, and plan a CoveQL query without returning result rows."
    )]
    async fn validate_query(
        &self,
        request: Parameters<QueryRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let path = self.map_source_error(self.resolve_source(&request.source))?;
        let query = self.prepare_query(&request.query)?;
        let bytes = fs::read(&path).map_err(to_invalid_params)?;
        parse_resolve_and_plan_query(
            &bytes,
            &query,
            coveql::ParseOptions::default(),
            coveql::ResolveOptions::default(),
            coveql::PlanOptions::default(),
            cove::reader::ValidationOptions::default(),
        )
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(CallToolResult::structured(json!({
            "source": request.source,
            "query": query,
            "query_validation": "planned_dry_run",
        })))
    }

    #[tool(
        name = "cove_explain_query",
        description = "Run a bounded CoveQL explain query against a configured source."
    )]
    async fn explain_query(
        &self,
        request: Parameters<ExplainRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let path = self.map_source_error(self.resolve_source(&request.source))?;
        let mode = explain_mode(request.mode.as_deref(), self.config.developer_mode)?;
        let report = explain_query_file(
            &path,
            &request.query,
            ExplainOptions {
                query: QueryOptions {
                    take: request.take.or(Some(self.config.default_take)),
                    ..QueryOptions::default()
                },
                mode,
            },
        )
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(CallToolResult::structured(json!({
            "source": request.source,
            "explain": report.json,
            "text": report.text,
        })))
    }

    #[tool(
        name = "cove_query",
        description = "Execute a bounded CoveQL query and return the first page plus a result handle for additional pages."
    )]
    async fn query(&self, request: Parameters<QueryRequest>) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let path = self.map_source_error(self.resolve_source(&request.source))?;
        let take = request.take.unwrap_or(self.config.default_take);
        if take > self.config.max_take {
            return Err(ErrorData::invalid_params(
                format!(
                    "take {take} exceeds server max_take {}",
                    self.config.max_take
                ),
                None,
            ));
        }
        self.reject_over_limit_take(&request.query)?;
        let result = query_file(
            &path,
            &request.query,
            QueryOptions {
                take: Some(take),
                ..QueryOptions::default()
            },
        )
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let value = result
            .result_json()
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let rows = match value {
            Value::Array(rows) => rows,
            other => vec![other],
        };
        let handle = self.results.insert(
            rows,
            self.config.page_size,
            self.config.max_response_bytes,
            json!({
                "source": request.source,
                "query": request.query,
                "take": take,
            }),
        );
        let page = self
            .results
            .page(handle.as_str(), 0)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(CallToolResult::structured(page))
    }

    #[tool(
        name = "cove_fetch_result_page",
        description = "Fetch an additional page for a previous cove_query result handle."
    )]
    async fn fetch_result_page(
        &self,
        request: Parameters<FetchPageRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let page = self
            .results
            .page(&request.result_handle, request.offset)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(CallToolResult::structured(page))
    }

    #[tool(
        name = "cove_get_diagnostics",
        description = "Return diagnostics retained on a result page or validation response handle when available."
    )]
    async fn get_diagnostics(
        &self,
        request: Parameters<DiagnosticsRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = request.0;
        let page = self
            .results
            .page(&request.handle, 0)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(CallToolResult::structured(json!({
            "handle": request.handle,
            "diagnostics": page.pointer("/metadata/diagnostics").cloned().unwrap_or_else(|| json!([])),
        })))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CoveMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "cove-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Use COVE-QD discovery to generate CoveQL. CoveQL executes; discovery guides; canonical metadata and policy decide.",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resources = self
            .config
            .roots()
            .map(|root| {
                Resource::new(format!("cove-root://{}", root.id), root.id.clone())
                    .with_title(root.id.clone())
                    .with_description(format!("Configured COVE root at {}", root.display_name))
                    .with_mime_type("application/json")
            })
            .collect::<Vec<_>>();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let prefix = "cove-root://";
        let Some(root_id) = request.uri.strip_prefix(prefix) else {
            return Err(ErrorData::invalid_params("unsupported resource URI", None));
        };
        let root = self.config.allowed_root(root_id).ok_or_else(|| {
            ErrorData::invalid_params(format!("unknown configured root `{root_id}`"), None)
        })?;
        let text = serde_json::to_string_pretty(&json!({
            "root": root.id,
            "path": root.display_name,
            "note": "This resource describes an allowed root. Use MCP tools with root-relative paths; the server does not browse directories.",
        }))
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            text,
            request.uri,
        )
        .with_mime_type("application/json")]))
    }
}

impl CoveMcpServer {
    fn store_diagnostics(&self, validation: &QueryDiscoveryValidationReport) -> String {
        let diagnostics = validation_report_json(validation)["diagnostics"].clone();
        self.results
            .insert(
                Vec::new(),
                self.config.page_size,
                self.config.max_response_bytes,
                json!({ "diagnostics": diagnostics }),
            )
            .as_str()
            .to_string()
    }

    fn map_source_error<T>(&self, result: Result<T, CoveMcpError>) -> Result<T, ErrorData> {
        result.map_err(|error| ErrorData::invalid_params(error.to_string(), None))
    }

    fn prepare_query(&self, query: &str) -> Result<String, ErrorData> {
        self.reject_over_limit_take(query)?;
        cove::prepare_query_text(
            query,
            cove::PreparedQueryTextOptions {
                take: Some(self.config.default_take),
                explain: None,
            },
        )
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))
    }

    fn reject_over_limit_take(&self, query: &str) -> Result<(), ErrorData> {
        if let Some(take) = max_take_literal(query) {
            if take > self.config.max_take {
                return Err(ErrorData::invalid_params(
                    format!(
                        "query take {take} exceeds server max_take {}",
                        self.config.max_take
                    ),
                    None,
                ));
            }
        }
        Ok(())
    }
}

fn tool_result(result: Result<Value, CoveMcpError>) -> CallToolResult {
    match result {
        Ok(value) => CallToolResult::structured(value),
        Err(error) => CallToolResult::structured_error(json!({
            "error": error.to_string(),
        })),
    }
}

fn to_invalid_params(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::invalid_params(error.to_string(), None)
}

fn validation_report_json(report: &QueryDiscoveryValidationReport) -> Value {
    json!({
        "validation_status": match report.validation_status {
            QueryDiscoveryValidationStatus::Valid => "valid",
            QueryDiscoveryValidationStatus::Stale => "stale",
            QueryDiscoveryValidationStatus::Invalid => "invalid",
        },
        "validation_flags": report.validation_flags.iter().map(|flag| match flag {
            QueryDiscoveryValidationFlag::PolicyFiltered => "policy_filtered",
            QueryDiscoveryValidationFlag::DiagnosticsWithheld => "diagnostics_withheld",
            QueryDiscoveryValidationFlag::ExamplesLimited => "examples_limited",
            QueryDiscoveryValidationFlag::AiLimited => "ai_limited",
        }).collect::<Vec<_>>(),
        "diagnostics": report.diagnostics.iter().map(|diagnostic| {
            json!({
                "code": diagnostic.code,
                "severity": format!("{:?}", diagnostic.severity).to_ascii_lowercase(),
                "message": diagnostic.message,
                "target_kind": diagnostic.target_kind,
                "target": diagnostic.target,
                "withheld": diagnostic.withheld,
            })
        }).collect::<Vec<_>>(),
    })
}

fn explain_mode(value: Option<&str>, developer_mode: bool) -> Result<ExplainMode, ErrorData> {
    match value.unwrap_or("public") {
        "public" => Ok(ExplainMode::Public),
        "proof" => Ok(ExplainMode::Proof),
        "coded" => Ok(ExplainMode::Coded),
        "ai" => Ok(ExplainMode::Ai),
        "developer" if developer_mode => Ok(ExplainMode::Developer),
        "forensic" if developer_mode => Ok(ExplainMode::Forensic),
        other => Err(ErrorData::invalid_params(
            format!("unsupported or disallowed explain mode `{other}`"),
            None,
        )),
    }
}

fn max_take_literal(query: &str) -> Option<usize> {
    let mut max = None;
    let mut rest = query;
    while let Some(index) = rest.find(".take(") {
        let after = &rest[index + ".take(".len()..];
        let digits = after
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if let Ok(value) = digits.parse::<usize>() {
            max = Some(max.map_or(value, |current: usize| current.max(value)));
        }
        rest = after;
    }
    max
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoverRequest {
    pub source: SourceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_ai: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidateManifestRequest {
    pub source: SourceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderTemplateRequest {
    pub source: SourceRef,
    pub template_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryRequest {
    pub source: SourceRef,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExplainRequest {
    pub source: SourceRef,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FetchPageRequest {
    pub result_handle: String,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiagnosticsRequest {
    pub handle: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_largest_take_literal() {
        assert_eq!(max_take_literal("table(t).take(10)"), Some(10));
        assert_eq!(max_take_literal("table(t).take(10).take(3)"), Some(10));
        assert_eq!(max_take_literal("table(t)"), None);
    }
}
