use crate::{
    ast::*, build_operation_context, parse_query, BranchContext, BranchSelector, CoveQlAiOperation,
    CoveQlDiagnostic, CoveQlExplainTarget, CoveQlOperationRequest, CoveQlOutputMode,
    CoveQlProfileId, CoveQlSelectedOperation, DiagnosticSeverity, ExplainMode,
    MetadataDisclosurePolicy, RedactionReference, RejectionKind, RejectionReport,
    ResourceUseEstimate, SecurityContext, TemporalContext, TemporalMode, TemporalRole,
    TombstoneContext, VisibilityReference,
};
use cove_core::{
    constants::CovePhysicalKind,
    profile::{
        cove_map::{EmbeddedMapSection, MapEvidenceIndex, MapProjectionCatalog},
        cove_o::{
            read_object_surface_from_bytes_with_options, CoveObjectReadOptions,
            CoveObjectRedactionReadPolicy, CoveObjectSurface, ObjectTypeEntryV1, PropertyEntryV1,
            OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_LINK_OBJECT,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
            PROPERTY_FLAG_ASSOCIATION_TYPE, PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
            PROPERTY_FLAG_ASSOCIATION_VALID_TO,
        },
    },
    reader::ValidationOptions,
    types::logical_type_name,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

mod ai;
mod diagnostics;
mod fingerprints;
mod functions;
mod graphs;
mod metadata_requests;
mod methods;
mod paths;
mod profiles;
mod roots;
mod tables;
mod temporal;

use ai::*;
use diagnostics::*;
use fingerprints::*;
use functions::*;
use graphs::*;
use metadata_requests::*;
use paths::*;
use profiles::*;
use roots::*;
use temporal::*;

pub fn parse_and_resolve_query(
    bytes: &[u8],
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    validation_options: ValidationOptions,
) -> Result<ResolvedQuery, BuildResolvedQueryError> {
    let parsed = parse_query(text, parse_options).map_err(BuildResolvedQueryError::from_parse)?;
    resolve_query(bytes, parsed, resolve_options, validation_options)
}

pub fn resolve_query(
    bytes: &[u8],
    parsed: ParsedQuery,
    resolve_options: ResolveOptions,
    validation_options: ValidationOptions,
) -> Result<ResolvedQuery, BuildResolvedQueryError> {
    validate_profile_shell(&parsed, &resolve_options)?;
    let mut settings = collect_chain_settings(&parsed, &resolve_options)?;
    let request = operation_request_for(&parsed, &settings, &resolve_options);
    let operation_context = build_operation_context(bytes, request, validation_options)
        .map_err(BuildResolvedQueryError::from_operation_context)?;

    let surface = match read_object_surface_from_bytes_with_options(
        bytes,
        &CoveObjectReadOptions {
            include_records: false,
            include_association_object_types: true,
            include_projection_catalog: true,
            include_function_registry: true,
            include_evidence_index: true,
            redaction_read_policy: CoveObjectRedactionReadPolicy::PreserveMarker,
            ..CoveObjectReadOptions::default()
        },
    ) {
        Ok(surface) => surface,
        Err(_error)
            if can_resolve_registered_table_without_object_surface(&parsed, &resolve_options) =>
        {
            empty_object_surface()
        }
        Err(error) => {
            return Err(BuildResolvedQueryError::single(diagnostic(
                "E_RESOLVE",
                format!("failed to read resolver metadata: {error}"),
                "resolve",
                &resolve_options.security,
            )))
        }
    };

    let mut resolver = Resolver::new(surface, resolve_options.clone());
    resolver.window_function_scope = parsed.methods.iter().any(|method| {
        matches!(
            &method.node,
            AstMethod::ProfileCall { name, .. } if name.name == "window"
        )
    });
    let mut root = resolver.resolve_root(&parsed.root.node)?;
    apply_root_alias(&mut root, parsed.root_alias.as_ref());
    settings.temporal = resolver.resolve_temporal_context(settings.temporal.clone(), &root)?;
    let method_chain = resolver.resolve_methods(&parsed, &root, &settings)?;
    let resolved_query_fingerprint = resolved_fingerprint(
        &root,
        &method_chain,
        &settings.output_mode,
        &settings.temporal,
        &settings.branch,
        &settings.tombstone,
        &parsed.profiles,
    );
    let diagnostics = operation_context
        .diagnostics
        .iter()
        .cloned()
        .chain(resolver.diagnostics)
        .collect();

    Ok(ResolvedQuery {
        parsed,
        operation_context,
        root,
        method_chain,
        output_mode: settings.output_mode,
        temporal: settings.temporal,
        branch: settings.branch,
        tombstone: settings.tombstone,
        visibility_reference: VisibilityReference {
            policy: resolve_options.security.visibility_policy.clone(),
        },
        redaction_reference: RedactionReference {
            policy: resolve_options.security.redaction_policy.clone(),
        },
        diagnostic_policy: DiagnosticPolicy {
            metadata_disclosure: resolve_options.security.metadata_disclosure_policy,
            explain_policy: resolve_options.security.explain_policy,
        },
        resource_use: settings.resource_use,
        resolved_query_fingerprint,
        diagnostics,
    })
}

fn can_resolve_registered_table_without_object_surface(
    parsed: &ParsedQuery,
    options: &ResolveOptions,
) -> bool {
    let AstRoot::Table(identifier) = &parsed.root.node else {
        return false;
    };
    options.table_authorities.contains_key(&identifier.name)
        || options.table_authorities.values().any(|authority| {
            authority.contract.table_name == identifier.name
                || authority.contract.table_id == identifier.name
        })
}

fn empty_object_surface() -> CoveObjectSurface {
    CoveObjectSurface {
        object_types: Vec::new(),
        records: Vec::new(),
        projection_catalog: None,
        evidence_index: None,
        embedded_function_ids: BTreeSet::new(),
        embedded_map_sections: Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub struct BuildResolvedQueryError {
    pub diagnostics: Vec<CoveQlDiagnostic>,
    pub rejections: Vec<RejectionReport>,
    pub source: Option<String>,
}

impl BuildResolvedQueryError {
    fn single(diagnostic: CoveQlDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            rejections: Vec::new(),
            source: None,
        }
    }

    fn from_parse(diagnostics: Vec<CoveQlDiagnostic>) -> Self {
        Self {
            diagnostics,
            rejections: Vec::new(),
            source: None,
        }
    }

    fn from_operation_context(error: crate::BuildOperationContextError) -> Self {
        Self {
            diagnostics: error.diagnostics,
            rejections: error.rejections,
            source: error.source,
        }
    }

    pub fn explain_json(&self) -> serde_json::Value {
        crate::explain::error_explain_json(
            "resolve",
            self.diagnostics
                .iter()
                .map(crate::CoveQlDiagnostic::to_json)
                .collect(),
            self.rejections
                .iter()
                .map(crate::RejectionReport::to_json)
                .collect(),
        )
    }
}

impl fmt::Display for BuildResolvedQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = self.diagnostics.first() {
            write!(f, "{}: {}", diagnostic.code, diagnostic.message)
        } else {
            write!(f, "CoveQL query resolution failed")
        }
    }
}

impl Error for BuildResolvedQueryError {}

#[derive(Debug, Clone)]
struct ChainSettings {
    selected_operation: CoveQlSelectedOperation,
    output_mode: CoveQlOutputMode,
    temporal: TemporalContext,
    branch: BranchContext,
    tombstone: TombstoneContext,
    resource_use: ResourceUseEstimate,
}

fn collect_chain_settings(
    parsed: &ParsedQuery,
    resolve_options: &ResolveOptions,
) -> Result<ChainSettings, BuildResolvedQueryError> {
    let mut temporal = TemporalContext::default();
    let mut branch = BranchContext::default();
    let mut tombstone = TombstoneContext::default();
    let mut seen_select = false;
    let mut seen_as_of = false;
    let mut seen_branch = false;
    let mut seen_tombstones = false;
    let mut seen_history = false;
    let mut seen_changes = false;
    let mut seen_order_by = false;
    let mut seen_take = false;
    let mut seen_skip = false;
    let mut seen_group_by = false;
    let mut seen_relationship_expansion = matches!(parsed.root.node, AstRoot::Path(_));
    let mut explain = None;

    for method in &parsed.methods {
        if explain.is_some() {
            return Err(conflict(
                "explain must be the final method in a CoveQL method chain",
                resolve_options,
            ));
        }
        match &method.node {
            AstMethod::Where(_) => {
                if seen_group_by {
                    return Err(conflict(
                        "where after groupBy requires a profile-declared having/post-aggregate filter",
                        resolve_options,
                    ));
                }
            }
            AstMethod::Select(_) => reject_duplicate(&mut seen_select, "select", resolve_options)?,
            AstMethod::AsOf(bound) => {
                reject_duplicate(&mut seen_as_of, "asOf", resolve_options)?;
                if seen_history || seen_changes {
                    return Err(conflict(
                        "asOf cannot be mixed with history or changes",
                        resolve_options,
                    ));
                }
                if seen_relationship_expansion {
                    return Err(conflict(
                        "asOf must appear in the temporal phase before lookup, traverse, or path expansion",
                        resolve_options,
                    ));
                }
                temporal = temporal_for_as_of(bound, resolve_options)?;
            }
            AstMethod::Branch(selector) => {
                reject_duplicate(&mut seen_branch, "branch", resolve_options)?;
                branch = branch_for_selector(selector, resolve_options)?;
            }
            AstMethod::IncludeTombstones(value) => {
                reject_duplicate(&mut seen_tombstones, "includeTombstones", resolve_options)?;
                tombstone.include_tombstones = *value;
            }
            AstMethod::History(mode) => {
                reject_duplicate(&mut seen_history, "history", resolve_options)?;
                if seen_as_of || seen_changes {
                    return Err(conflict(
                        "history cannot be mixed with asOf or changes",
                        resolve_options,
                    ));
                }
                if seen_relationship_expansion {
                    return Err(conflict(
                        "history must appear in the temporal phase before lookup, traverse, or path expansion",
                        resolve_options,
                    ));
                }
                temporal.mode = match mode {
                    AstHistoryMode::Records => TemporalMode::HistoryRecords,
                    AstHistoryMode::States => TemporalMode::HistoryStates,
                    AstHistoryMode::RecordsAndStates => TemporalMode::HistoryRecordsAndStates,
                };
            }
            AstMethod::Changes { from, to, mode } => {
                reject_duplicate(&mut seen_changes, "changes", resolve_options)?;
                if seen_as_of || seen_history {
                    return Err(conflict(
                        "changes cannot be mixed with asOf or history",
                        resolve_options,
                    ));
                }
                if seen_relationship_expansion {
                    return Err(conflict(
                        "changes must appear in the temporal phase before lookup, traverse, or path expansion",
                        resolve_options,
                    ));
                }
                if from.bound_kind() != to.bound_kind() {
                    return Err(conflict(
                        "changes bounds must use the same bound kind",
                        resolve_options,
                    ));
                }
                temporal.mode = match mode {
                    AstChangeMode::Records => TemporalMode::ChangesRecords,
                    AstChangeMode::StateTransitions => TemporalMode::ChangesStateTransitions,
                    AstChangeMode::PropertyDiffs => TemporalMode::ChangesPropertyDiffs,
                    AstChangeMode::FinalRows => TemporalMode::ChangesFinalObjects,
                };
            }
            AstMethod::OrderBy(_) => {
                reject_duplicate(&mut seen_order_by, "orderBy", resolve_options)?
            }
            AstMethod::Take(_) => reject_duplicate(&mut seen_take, "take", resolve_options)?,
            AstMethod::Skip(_) => reject_duplicate(&mut seen_skip, "skip", resolve_options)?,
            AstMethod::GroupBy(_) => {
                reject_duplicate(&mut seen_group_by, "groupBy", resolve_options)?
            }
            AstMethod::Explain(mode) => {
                if explain.replace(*mode).is_some() {
                    return Err(duplicate("duplicate explain method", resolve_options));
                }
            }
            AstMethod::ProfileCall { name, .. } if !is_coveql_profile_method(&name.name) => {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    format!(
                        "CoveQL profile method {} is not declared by the active profile contract for this root grain",
                        name.name
                    ),
                    resolve_options,
                ));
            }
            AstMethod::ProfileCall { name, .. } => {
                if seen_group_by {
                    return Err(conflict(
                        format!(
                            "{} cannot appear after groupBy unless its profile contract accepts aggregate grain",
                            name.name
                        ),
                        resolve_options,
                    ));
                }
                if profile_method_expands_relationships(&name.name) {
                    seen_relationship_expansion = true;
                }
            }
        }
    }

    let target = explain_target_for_root(&parsed.root.node);
    let default_output = output_mode_for_root(&parsed.root.node);
    let output_mode = if explain.is_some() {
        resolve_options
            .output_mode
            .clone()
            .unwrap_or(CoveQlOutputMode::ExplainJson)
    } else {
        resolve_options
            .output_mode
            .clone()
            .unwrap_or(default_output)
    };
    let ai_operation = selected_ai_operation_for_methods(&parsed.methods, explain);
    let selected_operation = if let Some(operation) = ai_operation {
        CoveQlSelectedOperation::Ai {
            root: target,
            operation,
        }
    } else if let Some(mode) = explain {
        CoveQlSelectedOperation::Explain { target, mode }
    } else if let CoveQlOutputMode::ArrowRecordBatch {
        zero_copy_requested,
    } = &output_mode
    {
        CoveQlSelectedOperation::ArrowExport {
            zero_copy_requested: *zero_copy_requested,
        }
    } else {
        selected_operation_for_root(&parsed.root.node)
    };

    let mut resource_use = parsed.resource_use.clone();
    resource_use.planning_time_ms = Some(0);
    resource_use.decode_bytes = Some(0);
    resource_use.range_requests = Some(0);

    Ok(ChainSettings {
        selected_operation,
        output_mode,
        temporal,
        branch,
        tombstone,
        resource_use,
    })
}

fn selected_operation_for_root(root: &AstRoot) -> CoveQlSelectedOperation {
    match root {
        AstRoot::Object(_) => CoveQlSelectedOperation::Object,
        AstRoot::Association(_) => CoveQlSelectedOperation::Association,
        AstRoot::Node(_) => CoveQlSelectedOperation::GraphNode,
        AstRoot::Edge(_) => CoveQlSelectedOperation::GraphEdge,
        AstRoot::Table(_) => CoveQlSelectedOperation::Table,
        AstRoot::Projection(_) => CoveQlSelectedOperation::Projection,
        AstRoot::Evidence(_) => CoveQlSelectedOperation::Evidence,
        AstRoot::Path(_) => CoveQlSelectedOperation::GraphNode,
    }
}

fn explain_target_for_root(root: &AstRoot) -> CoveQlExplainTarget {
    match root {
        AstRoot::Object(_) => CoveQlExplainTarget::Object,
        AstRoot::Association(_) => CoveQlExplainTarget::Association,
        AstRoot::Node(_) => CoveQlExplainTarget::GraphNode,
        AstRoot::Edge(_) => CoveQlExplainTarget::GraphEdge,
        AstRoot::Table(_) => CoveQlExplainTarget::Table,
        AstRoot::Projection(_) => CoveQlExplainTarget::Projection,
        AstRoot::Evidence(_) => CoveQlExplainTarget::Evidence,
        AstRoot::Path(_) => CoveQlExplainTarget::GraphNode,
    }
}

fn output_mode_for_root(root: &AstRoot) -> CoveQlOutputMode {
    match root {
        AstRoot::Object(_) => CoveQlOutputMode::ObjectRows,
        AstRoot::Association(_) => CoveQlOutputMode::AssociationRows,
        AstRoot::Table(_) => CoveQlOutputMode::JsonRows,
        AstRoot::Projection(_) => CoveQlOutputMode::ProjectionRows,
        AstRoot::Evidence(_) => CoveQlOutputMode::EvidenceRows,
        AstRoot::Node(_) | AstRoot::Edge(_) | AstRoot::Path(_) => CoveQlOutputMode::JsonRows,
    }
}

struct Resolver {
    surface: CoveObjectSurface,
    options: ResolveOptions,
    diagnostics: Vec<CoveQlDiagnostic>,
    lookup_scope: Vec<ResolvedTableRoot>,
    cte_table_scope: BTreeMap<String, ResolvedTableRoot>,
    graph_node_scope: Vec<ResolvedGraphNodeRoot>,
    graph_edge_scope: Vec<ResolvedGraphEdgeRoot>,
    window_function_scope: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssociationResolveUsage {
    RootScan,
    ObjectRelative,
    EvidenceTarget,
}

impl Resolver {
    fn new(surface: CoveObjectSurface, options: ResolveOptions) -> Self {
        Self {
            surface,
            options,
            diagnostics: Vec::new(),
            lookup_scope: Vec::new(),
            cte_table_scope: BTreeMap::new(),
            graph_node_scope: Vec::new(),
            graph_edge_scope: Vec::new(),
            window_function_scope: false,
        }
    }
}
