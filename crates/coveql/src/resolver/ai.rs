use super::*;

pub(super) fn selected_ai_operation_for_methods(
    methods: &[Spanned<AstMethod>],
    explain: Option<ExplainMode>,
) -> Option<CoveQlAiOperation> {
    if matches!(explain, Some(ExplainMode::Ai)) {
        return Some(CoveQlAiOperation::Inspect);
    }
    methods
        .iter()
        .filter_map(|method| match &method.node {
            AstMethod::ProfileCall { name, .. } => ai_operation_for_method(&name.name),
            _ => None,
        })
        .max_by_key(|operation| ai_operation_priority(*operation))
}

pub(super) fn ai_operation_for_method(name: &str) -> Option<CoveQlAiOperation> {
    match name {
        "chunks" => Some(CoveQlAiOperation::ChunkProjection),
        "tokens" => Some(CoveQlAiOperation::TokenProjection),
        "embedding" => Some(CoveQlAiOperation::Embedding),
        "similar" | "hybrid" | "rerank" => Some(CoveQlAiOperation::SemanticSearch),
        "context" | "asPromptContext" => Some(CoveQlAiOperation::RagContext),
        "trainingSamples" | "split" | "pack" => Some(CoveQlAiOperation::TrainingSampleExport),
        "multimodal" => Some(CoveQlAiOperation::MultimodalSequenceRead),
        "generatorAudit" => Some(CoveQlAiOperation::GeneratorAudit),
        _ => None,
    }
}

pub(super) fn ai_operation_priority(operation: CoveQlAiOperation) -> u8 {
    match operation {
        CoveQlAiOperation::Inspect => 0,
        CoveQlAiOperation::ChunkProjection => 1,
        CoveQlAiOperation::TokenProjection => 2,
        CoveQlAiOperation::Embedding => 3,
        CoveQlAiOperation::SemanticSearch => 4,
        CoveQlAiOperation::RagContext => 5,
        CoveQlAiOperation::TrainingSampleExport => 6,
        CoveQlAiOperation::MultimodalSequenceRead => 7,
        CoveQlAiOperation::GeneratorAudit => 8,
    }
}

pub(super) fn ai_authority(operation: CoveQlAiOperation) -> &'static str {
    match operation {
        CoveQlAiOperation::Embedding => {
            "sidecar_vector_or_deterministic_runtime_embedding; runtime_float_composition_is_advisory"
        }
        CoveQlAiOperation::SemanticSearch => {
            "validated_exact_flat_vector_scan_or_advisory_candidate_rerank"
        }
        CoveQlAiOperation::RagContext => "validated_chunk_source_binding_with_policy_scoped_context",
        CoveQlAiOperation::TrainingSampleExport => {
            "validated_training_policy_with_policy_withheld_diagnostics"
        }
        CoveQlAiOperation::MultimodalSequenceRead => {
            "validated_multimodal_sequence_asset_tensor_policy"
        }
        CoveQlAiOperation::ChunkProjection => "validated_chunk_profile_and_source_hash_freshness",
        CoveQlAiOperation::TokenProjection => {
            "validated_tokenizer_profile_token_payload_and_source_hash_freshness"
        }
        CoveQlAiOperation::GeneratorAudit => "validated_generator_provenance_and_review_records",
        CoveQlAiOperation::Inspect => "validated_ai_descriptor_summary",
    }
}

pub(super) fn ai_policy_scope(root: &ResolvedRoot) -> &'static str {
    match root {
        ResolvedRoot::Object(_) => "object_state_visibility_redaction",
        ResolvedRoot::Association(_) => "association_state_visibility_redaction",
        ResolvedRoot::Projection(_) => "projection_row_visibility_redaction",
        ResolvedRoot::Evidence(_) => "evidence_row_visibility_redaction",
        ResolvedRoot::Table(_) => "table_row_visibility_redaction",
        ResolvedRoot::Node(_) => "graph_node_visibility_redaction",
        ResolvedRoot::Edge(_) => "graph_edge_visibility_redaction",
    }
}

impl Resolver {
    pub(super) fn resolve_ai_operation(
        &self,
        name: &str,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedAiOperation, BuildResolvedQueryError> {
        let Some(operation) = ai_operation_for_method(name) else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!("unsupported CoveQL/AI method {name}"),
                &self.options,
            ));
        };
        let mut resolved_args = Vec::with_capacity(args.len());
        for arg in args {
            let value = match &arg.value {
                AstProfileArgumentValue::Expr(expr) => {
                    ResolvedAiArgumentValue::Expr(self.resolve_expr(expr, root)?)
                }
                AstProfileArgumentValue::Predicate(predicate) => {
                    ResolvedAiArgumentValue::Predicate(self.resolve_predicate(predicate, root)?)
                }
            };
            resolved_args.push(ResolvedAiArgument {
                name: arg.name.as_ref().map(|name| name.name.clone()),
                value,
            });
        }
        Ok(ResolvedAiOperation {
            operation,
            method_name: name.into(),
            args: resolved_args,
            sidecar_required: true,
            authority: ai_authority(operation).into(),
            policy_scope: ai_policy_scope(root).into(),
        })
    }
}
