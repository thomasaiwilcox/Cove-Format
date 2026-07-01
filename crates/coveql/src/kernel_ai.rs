use std::{collections::BTreeSet, time::Instant};

use cove_core::{
    artifact::coveai::{
        ai_embedding, ai_vector_search, AiEmbeddingRequest, AiPayloadReader,
        AiVectorIndexSelection, AiVectorSearchPlan, AiVectorSearchResult, AiVectorSearchTargetKind,
        CoveAiAccessContext, CoveAiFile,
    },
    profile::cove_o::{
        read_object_surface_from_bytes_with_options, reconstruct_object_states,
        CoveObjectReadOptions, CoveObjectReconstructionOptions, CoveObjectState,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    execution::{
        enforce_result_budgets, exec_error, exec_warning, result_fingerprint,
        zero_copy_owned_fallback_warning,
    },
    kernel_execution::{attach_phase8_plan_reports, execution_diagnostics_for_physical},
    kernel_metrics::{
        KernelCounters, KernelDecision, KernelDecisionKind, KernelExecutionMode,
        KernelExecutionReport, OptimizationAuthorityReport,
    },
    materialized::hex,
    pushdown, BuildExecutionError, CoveQlAiOperation, CoveQlExecutionResult, ExecutedQuery,
    ExecutionAuthorityReport, ExecutionOptions, ExecutionRowCounts, PhysicalPlannedQuery,
    ResolvedAiArgumentValue, ResolvedExpr, ResolvedLiteralValue,
};

struct AiDescriptorMetadataExecution {
    rows: Vec<Value>,
    input_rows: usize,
    warning_code: &'static str,
    warning_message: &'static str,
    authority_summary: &'static str,
    pushdown_summary: &'static str,
}

pub(super) fn try_ai_descriptor_metadata_executed_query(
    source_bytes: &[u8],
    physical: &PhysicalPlannedQuery,
    execution_options: &ExecutionOptions,
    mode: KernelExecutionMode,
) -> Result<Option<(ExecutedQuery, KernelExecutionReport)>, BuildExecutionError> {
    let Some(operation) = physical
        .planned
        .resolved
        .method_chain
        .ai_operations
        .iter()
        .rev()
        .find(|operation| ai_descriptor_metadata_method(operation.method_name.as_str()))
    else {
        return Ok(None);
    };
    if mode == KernelExecutionMode::ForceMaterialized {
        return Err(exec_error(
            "E_AI_MATERIALIZED_UNSUPPORTED",
            "CoveQL-AI descriptor metadata operations require a validated COVE-AI sidecar; materialized COVE-O readback has no equivalent AI descriptor baseline",
            json!({ "method": operation.method_name }),
        ));
    }
    if !matches!(
        physical.planned.resolved.output_mode,
        crate::CoveQlOutputMode::JsonRows
    ) {
        return Err(exec_error(
            "E_AI_OUTPUT_UNSUPPORTED",
            "CoveQL-AI descriptor metadata operations currently return JSON rows",
            json!({ "output_mode": format!("{:?}", physical.planned.resolved.output_mode) }),
        ));
    }
    let include_payloads = ai_include_payloads_arg(operation)?;
    let generator_filter = if operation.method_name == "generatorAudit" {
        ai_generator_audit_filter(operation)?
    } else {
        AiGeneratorAuditFilter::default()
    };
    let Some(sidecar_bytes) = physical.sidecars.cove_ai_artifact_bytes.as_deref() else {
        return Err(exec_error(
            "E_AI_SIDECAR_REQUIRED",
            "CoveQL-AI descriptor metadata execution requires a supplied COVE-AI/COVE-VEC sidecar",
            json!({ "method": operation.method_name }),
        ));
    };
    let started = Instant::now();
    let sidecar =
        CoveAiFile::parse_for_operation(sidecar_bytes, operation.operation.operation_kind())
            .map_err(|error| {
                exec_error(
                    "E_AI_SIDECAR_INVALID",
                    format!("COVE-AI sidecar validation failed: {error}"),
                    json!({ "method": operation.method_name }),
                )
            })?;
    let access_context = if include_payloads {
        CoveAiAccessContext::for_operation(operation.method_name.clone())
    } else {
        CoveAiAccessContext::descriptor_only(operation.method_name.clone())
    };
    let payload_reader = AiPayloadReader::new(sidecar_bytes, &sidecar, access_context);
    let chunk_source_context = ai_chunk_source_context_for_operation(
        operation.method_name.as_str(),
        include_payloads,
        source_bytes,
        &sidecar,
    );
    let execution = ai_descriptor_metadata_rows(
        operation.method_name.as_str(),
        &sidecar,
        include_payloads,
        &payload_reader,
        chunk_source_context.as_ref(),
        &generator_filter,
    );
    let result = CoveQlExecutionResult::JsonRows(execution.rows);
    let output_rows = match &result {
        CoveQlExecutionResult::JsonRows(rows) => rows.len(),
        _ => 0,
    };
    let row_counts = ExecutionRowCounts {
        input_rows: execution.input_rows,
        filtered_rows: output_rows,
        output_rows,
    };
    enforce_result_budgets(
        &result,
        &row_counts,
        &physical.planned,
        execution_options,
        started,
    )?;
    let output_fingerprint = result_fingerprint(&result)?;

    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        execution.warning_code,
        execution.warning_message,
        json!({
            "method": operation.method_name,
            "input_descriptor_rows": execution.input_rows,
            "output_rows": output_rows,
            "payload_access": ai_payload_access_label(&sidecar),
            "include_payloads": include_payloads,
            "payloads_policy_gated": true,
            "materialized_baseline_available": false,
        }),
    ));
    if mode == KernelExecutionMode::CompareWithMaterialized {
        diagnostics.push(exec_warning(
            "W_AI_NO_MATERIALIZED_BASELINE",
            "CoveQL-AI descriptor metadata execution has no materialized COVE-O baseline oracle; sidecar result was not fingerprint-compared",
            json!({ "method": operation.method_name }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(&physical.planned) {
        diagnostics.push(warning);
    }

    let mut kernel_report = KernelExecutionReport::applied(
        mode,
        KernelCounters {
            rows_scanned: execution.input_rows,
            rows_after_bitmap: execution.input_rows,
            rows_after_selection_vector: output_rows,
            output_rows,
            bytes_touched_estimate: sidecar_bytes.len(),
            dictionary_lookups_at_materialization: 0,
            ..KernelCounters::default()
        },
    );
    kernel_report.optimization_authority =
        OptimizationAuthorityReport::authoritative(execution.authority_summary);
    kernel_report.decision = KernelDecision::new(
        KernelDecisionKind::Applied,
        "CoveQL-AI descriptor metadata operation executed from a validated COVE-AI sidecar",
        json!({
            "method": operation.method_name,
            "input_descriptor_rows": execution.input_rows,
            "output_rows": output_rows,
            "payload_access": ai_payload_access_label(&sidecar),
            "include_payloads": include_payloads,
            "fallback_boundary": Value::Null,
            "materialized_baseline_available": false,
        }),
        false,
    );
    kernel_report.decisions = vec![kernel_report.decision.clone()];
    kernel_report.kernel_fingerprint = Some(output_fingerprint.clone());
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);

    let executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_applicable(
            &execution_options.pushdown,
            execution.pushdown_summary,
        ),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_kernel(execution.authority_summary, false),
    };
    Ok(Some((executed, kernel_report)))
}

fn ai_descriptor_metadata_method(method_name: &str) -> bool {
    matches!(
        method_name,
        "chunks"
            | "tokens"
            | "context"
            | "asPromptContext"
            | "trainingSamples"
            | "split"
            | "pack"
            | "multimodal"
            | "generatorAudit"
    )
}

fn ai_descriptor_metadata_rows(
    method_name: &str,
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
    chunk_source_context: Option<&AiChunkSourceContext>,
    generator_filter: &AiGeneratorAuditFilter,
) -> AiDescriptorMetadataExecution {
    match method_name {
        "chunks" => AiDescriptorMetadataExecution {
            rows: ai_chunk_projection_rows(
                sidecar,
                false,
                include_payloads,
                payload_reader,
                chunk_source_context,
            ),
            input_rows: sidecar.descriptor_tables.text_chunks.len(),
            warning_code: "W_AI_CHUNK_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .chunks() executed from validated COVE-CHUNK descriptor metadata",
            authority_summary: if include_payloads {
                "COVE-AI chunk descriptor metadata was validated; chunk text is reconstructed only from validated source values"
            } else {
                "COVE-AI chunk descriptor metadata was validated; chunk text payloads were withheld"
            },
            pushdown_summary:
                "CoveQL-AI chunk projection reads validated COVE-CHUNK sidecar descriptors rather than COVE-O row pages",
        },
        "tokens" => AiDescriptorMetadataExecution {
            rows: ai_token_projection_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.token_blocks.len()
                + sidecar.descriptor_tables.tokenized_spans.len()
                + sidecar.descriptor_tables.token_sequence_packs.len(),
            warning_code: "W_AI_TOKEN_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .tokens() executed from validated COVE-TOK descriptor metadata",
            authority_summary: if include_payloads {
                "COVE-AI token descriptor metadata was validated; token payload bytes are exposed only through AI payload leases"
            } else {
                "COVE-AI token descriptor metadata was validated; token payload bytes were withheld"
            },
            pushdown_summary:
                "CoveQL-AI token projection reads validated COVE-TOK sidecar descriptors rather than COVE-O row pages",
        },
        "context" | "asPromptContext" => AiDescriptorMetadataExecution {
            rows: ai_chunk_projection_rows(
                sidecar,
                true,
                include_payloads,
                payload_reader,
                chunk_source_context,
            ),
            input_rows: sidecar.descriptor_tables.text_chunks.len(),
            warning_code: "W_AI_RAG_CONTEXT_METADATA_EXECUTED",
            warning_message: if include_payloads {
                "CoveQL-AI RAG context executed from validated chunk descriptors with source-bound prompt text"
            } else {
                "CoveQL-AI RAG context executed from validated chunk descriptors with prompt text withheld"
            },
            authority_summary: if include_payloads {
                "COVE-AI RAG context descriptor metadata was validated; prompt text is reconstructed only from validated source values"
            } else {
                "COVE-AI RAG context descriptor metadata was validated; prompt text expansion was withheld by policy"
            },
            pushdown_summary:
                "CoveQL-AI context projection reads validated chunk sidecar descriptors rather than COVE-O row pages",
        },
        "split" => AiDescriptorMetadataExecution {
            rows: ai_training_split_rows(sidecar),
            input_rows: sidecar.descriptor_tables.dataset_splits.len(),
            warning_code: "W_AI_TRAINING_SPLIT_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .split() executed from validated COVE-TRAIN split descriptors",
            authority_summary:
                "COVE-AI training split descriptors were validated; sample payloads were withheld",
            pushdown_summary:
                "CoveQL-AI split projection reads validated COVE-TRAIN sidecar descriptors rather than COVE-O row pages",
        },
        "pack" => AiDescriptorMetadataExecution {
            rows: ai_training_pack_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.token_sequence_packs.len()
                + sidecar.descriptor_tables.multimodal_sequence_packs.len(),
            warning_code: "W_AI_TRAINING_PACK_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .pack() executed from validated token and multimodal pack descriptors",
            authority_summary:
                "COVE-AI training pack descriptors were validated; pack payloads were withheld",
            pushdown_summary:
                "CoveQL-AI pack projection reads validated COVE-AI sidecar descriptors rather than COVE-O row pages",
        },
        "multimodal" => AiDescriptorMetadataExecution {
            rows: ai_multimodal_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.multimodal_sequence_packs.len()
                + sidecar.descriptor_tables.multimodal_sequence_elements.len(),
            warning_code: "W_AI_MULTIMODAL_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .multimodal() executed from validated COVE-MMSEQ descriptor metadata",
            authority_summary:
                "COVE-AI multimodal sequence descriptors were validated; asset and tensor payloads were withheld",
            pushdown_summary:
                "CoveQL-AI multimodal projection reads validated COVE-MMSEQ sidecar descriptors rather than COVE-O row pages",
        },
        "generatorAudit" => AiDescriptorMetadataExecution {
            rows: ai_generator_audit_rows(
                sidecar,
                include_payloads,
                payload_reader,
                generator_filter,
            ),
            input_rows: sidecar.descriptor_tables.generator_provenance.len()
                + sidecar.descriptor_tables.model_actors.len()
                + sidecar.descriptor_tables.generation_decoding_profiles.len()
                + sidecar.descriptor_tables.human_reviews.len()
                + sidecar.descriptor_tables.training_labels.len()
                + sidecar.descriptor_tables.preference_pairs.len(),
            warning_code: "W_AI_GENERATOR_AUDIT_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .generatorAudit() executed from validated generator provenance descriptors",
            authority_summary:
                "COVE-AI generator provenance descriptors were validated; prompt/output payloads were withheld",
            pushdown_summary:
                "CoveQL-AI generator audit reads validated COVE-AI provenance descriptors rather than COVE-O row pages",
        },
        _ => AiDescriptorMetadataExecution {
            rows: ai_training_sample_rows(sidecar, include_payloads, payload_reader),
            input_rows: sidecar.descriptor_tables.training_samples.len(),
            warning_code: "W_AI_TRAINING_SAMPLE_METADATA_EXECUTED",
            warning_message:
                "CoveQL-AI .trainingSamples() executed from validated COVE-TRAIN descriptor metadata",
            authority_summary:
                "COVE-AI training sample descriptors were validated; input/target payloads were withheld",
            pushdown_summary:
                "CoveQL-AI training sample projection reads validated COVE-TRAIN sidecar descriptors rather than COVE-O row pages",
        },
    }
}

struct AiChunkSourceContext {
    states: Vec<CoveObjectState>,
    read_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct AiGeneratorAuditFilter {
    model_namespace: AiStringRefFilter,
    model_name: AiStringRefFilter,
    model_version: AiStringRefFilter,
    provider: AiStringRefFilter,
    endpoint: AiStringRefFilter,
    decoding_profile_ref: Option<u32>,
    human_review_status: Option<AiHumanReviewStatusFilter>,
    reproducibility_class: Option<u8>,
}

impl AiGeneratorAuditFilter {
    fn is_empty(&self) -> bool {
        self.model_namespace.is_empty()
            && self.model_name.is_empty()
            && self.model_version.is_empty()
            && self.provider.is_empty()
            && self.endpoint.is_empty()
            && self.decoding_profile_ref.is_none()
            && self.human_review_status.is_none()
            && self.reproducibility_class.is_none()
    }
}

#[derive(Debug, Clone, Default)]
struct AiStringRefFilter {
    value: Option<String>,
    ref_id: Option<u32>,
}

impl AiStringRefFilter {
    fn is_empty(&self) -> bool {
        self.value.is_none() && self.ref_id.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiHumanReviewStatusFilter {
    Reviewed,
    Unreviewed,
}

struct AiChunkTextProjection {
    text: String,
    source_value_hash_status: &'static str,
    chunk_text_hash_status: &'static str,
}

fn ai_chunk_source_context_for_operation(
    method_name: &str,
    include_payloads: bool,
    source_bytes: &[u8],
    sidecar: &CoveAiFile,
) -> Option<AiChunkSourceContext> {
    if !include_payloads || !matches!(method_name, "chunks" | "context" | "asPromptContext") {
        return None;
    }

    let mut property_ids = sidecar
        .descriptor_tables
        .text_chunks
        .iter()
        .filter_map(|chunk| (chunk.property_id != 0).then_some(chunk.property_id))
        .collect::<Vec<_>>();
    property_ids.sort_unstable();
    property_ids.dedup();
    if property_ids.is_empty() {
        return Some(AiChunkSourceContext {
            states: Vec::new(),
            read_error: None,
        });
    }

    let read_options = CoveObjectReadOptions::requested_property_ids(property_ids);
    let surface = match read_object_surface_from_bytes_with_options(source_bytes, &read_options) {
        Ok(surface) => surface,
        Err(error) => {
            return Some(AiChunkSourceContext {
                states: Vec::new(),
                read_error: Some(format!("COVE-O source readback failed: {error}")),
            });
        }
    };
    let states =
        match reconstruct_object_states(&surface, &CoveObjectReconstructionOptions::default()) {
            Ok(states) => states,
            Err(error) => {
                return Some(AiChunkSourceContext {
                    states: Vec::new(),
                    read_error: Some(format!("COVE-O source reconstruction failed: {error}")),
                });
            }
        };
    Some(AiChunkSourceContext {
        states,
        read_error: None,
    })
}

fn ai_chunk_text_projection(
    chunk: &cove_core::artifact::coveai::TextChunkEntryV1,
    context: Option<&AiChunkSourceContext>,
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
) -> Result<AiChunkTextProjection, String> {
    let context = context.ok_or_else(|| "chunk text payloads were not requested".to_string())?;
    if let Some(error) = &context.read_error {
        return Err(error.clone());
    }
    if chunk.object_type_id == 0 || chunk.property_id == 0 {
        return Err(
            "chunk source binding does not identify a COVE-O object_type_id/property_id"
                .to_string(),
        );
    }

    let mut values = Vec::new();
    for state in &context.states {
        if state.object_type_id != chunk.object_type_id {
            continue;
        }
        if chunk.source_row_ref != 0 {
            let latest_row_index = u64::from(state.latest_row_index);
            let one_based_row_index = latest_row_index.saturating_add(1);
            if chunk.source_row_ref != latest_row_index
                && chunk.source_row_ref != one_based_row_index
            {
                continue;
            }
        }
        let Some(property) = state
            .properties
            .iter()
            .find(|property| property.property_id == chunk.property_id)
        else {
            continue;
        };
        if property.redacted {
            return Err("source property is redacted under the active COVE-O read policy".into());
        }
        let Some(text) = property.value.as_str() else {
            return Err("source property value is not UTF-8 text".into());
        };
        if !values.contains(&text) {
            values.push(text);
        }
    }

    let source_text = match values.as_slice() {
        [] => return Err("no matching source value was found for chunk binding".into()),
        [value] => *value,
        _ => return Err("chunk source binding resolved to multiple source values".into()),
    };
    let source_bytes = source_text.as_bytes();
    ai_verify_sha256_digest_ref(
        sidecar,
        payload_reader,
        chunk.source_value_hash_ref,
        source_bytes,
        "source_value_hash_ref",
    )?;

    let byte_start = usize::try_from(chunk.byte_start)
        .map_err(|_| "chunk byte_start exceeds platform usize".to_string())?;
    let byte_length = usize::try_from(chunk.byte_length)
        .map_err(|_| "chunk byte_length exceeds platform usize".to_string())?;
    let byte_end = byte_start
        .checked_add(byte_length)
        .ok_or_else(|| "chunk byte range overflows".to_string())?;
    if byte_end > source_bytes.len() {
        return Err("chunk byte range exceeds source text length".into());
    }
    if !source_text.is_char_boundary(byte_start) || !source_text.is_char_boundary(byte_end) {
        return Err("chunk byte range does not align to UTF-8 boundaries".into());
    }
    let chunk_text = &source_text[byte_start..byte_end];
    let scalar_start = u64::try_from(source_text[..byte_start].chars().count())
        .map_err(|_| "chunk unicode scalar start exceeds u64".to_string())?;
    let scalar_length = u64::try_from(chunk_text.chars().count())
        .map_err(|_| "chunk unicode scalar length exceeds u64".to_string())?;
    if chunk.unicode_scalar_start != scalar_start || chunk.unicode_scalar_length != scalar_length {
        return Err("chunk Unicode scalar offsets do not match source text span".into());
    }
    ai_verify_sha256_digest_ref(
        sidecar,
        payload_reader,
        chunk.chunk_text_hash_ref,
        chunk_text.as_bytes(),
        "chunk_text_hash_ref",
    )?;

    Ok(AiChunkTextProjection {
        text: chunk_text.to_string(),
        source_value_hash_status: "verified_sha256",
        chunk_text_hash_status: "verified_sha256",
    })
}

fn ai_verify_sha256_digest_ref(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    digest_ref: u32,
    expected_bytes: &[u8],
    label: &str,
) -> Result<(), String> {
    let digest = sidecar
        .descriptor_tables
        .digests
        .iter()
        .find(|digest| digest.digest_ref == digest_ref)
        .ok_or_else(|| format!("{label} {digest_ref} is missing from AI_DIGEST_TABLE"))?;
    if digest.digest_algorithm != 1 || digest.digest_len != 32 {
        return Err(format!(
            "{label} {digest_ref} uses unsupported digest algorithm/length"
        ));
    }
    let lease = payload_reader
        .lease_payload_ref(digest.digest_payload_ref)
        .map_err(|error| format!("{label} {digest_ref} digest payload withheld: {error}"))?;
    if lease.bytes.len() != 32 {
        return Err(format!(
            "{label} {digest_ref} digest payload length is not 32 bytes"
        ));
    }
    let actual = Sha256::digest(expected_bytes);
    if lease.bytes != actual.as_slice() {
        return Err(format!("{label} {digest_ref} digest mismatch"));
    }
    Ok(())
}

fn ai_payload_access_label(sidecar: &CoveAiFile) -> &'static str {
    match sidecar.payload_access {
        cove_core::artifact::coveai::AiPayloadAccessState::StructurallyAllowed => {
            "structurally_allowed"
        }
        cove_core::artifact::coveai::AiPayloadAccessState::PolicyBlockedMissingPrivacySummary => {
            "policy_blocked_missing_privacy_summary"
        }
    }
}

fn ai_payload_ref_projection(
    payload_ref: u32,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Value {
    if payload_ref == 0 {
        return json!({
            "payload_ref": 0,
            "payload_access": "not_declared",
            "payload": Value::Null,
        });
    }
    if !include_payloads {
        return json!({
            "payload_ref": payload_ref,
            "payload_access": "not_requested",
            "payload": Value::Null,
        });
    }
    match payload_reader.lease_payload_ref(payload_ref) {
        Ok(lease) => match std::str::from_utf8(lease.bytes) {
            Ok(text) => json!({
                "payload_ref": payload_ref,
                "payload_access": lease.disclosure.as_str(),
                "media_type_ref": lease.media_type_ref,
                "decoded_length": lease.decoded_length,
                "text": text,
            }),
            Err(_) => json!({
                "payload_ref": payload_ref,
                "payload_access": lease.disclosure.as_str(),
                "media_type_ref": lease.media_type_ref,
                "decoded_length": lease.decoded_length,
                "bytes_hex": hex(lease.bytes),
            }),
        },
        Err(error) => json!({
            "payload_ref": payload_ref,
            "payload_access": "withheld",
            "withholding_reason": error.to_string(),
            "payload": Value::Null,
        }),
    }
}

fn ai_token_block_payload_projection(
    block: &cove_core::artifact::coveai::TokenBlockHeaderV1,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Value {
    if !include_payloads {
        return json!({
            "payload_ref": block.payload_ref,
            "payload_access": "not_requested",
            "token_ids": Value::Null,
        });
    }
    let lease = match payload_reader.lease_payload_ref(block.payload_ref) {
        Ok(lease) => lease,
        Err(error) => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": error.to_string(),
                "token_ids": Value::Null,
            });
        }
    };
    let width = usize::from(block.token_id_width);
    let token_count = match usize::try_from(block.token_count) {
        Ok(value) => value,
        Err(_) => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": "token_count exceeds platform usize",
                "token_ids": Value::Null,
            });
        }
    };
    let offset = match usize::try_from(block.payload_offset) {
        Ok(value) => value,
        Err(_) => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": "payload_offset exceeds platform usize",
                "token_ids": Value::Null,
            });
        }
    };
    let length = if block.payload_length == 0 {
        match token_count.checked_mul(width) {
            Some(value) => value,
            None => {
                return json!({
                    "payload_ref": block.payload_ref,
                    "payload_access": "withheld",
                    "withholding_reason": "token payload length overflows",
                    "token_ids": Value::Null,
                });
            }
        }
    } else {
        match usize::try_from(block.payload_length) {
            Ok(value) => value,
            Err(_) => {
                return json!({
                    "payload_ref": block.payload_ref,
                    "payload_access": "withheld",
                    "withholding_reason": "payload_length exceeds platform usize",
                    "token_ids": Value::Null,
                });
            }
        }
    };
    let end = match offset.checked_add(length) {
        Some(value) => value,
        None => {
            return json!({
                "payload_ref": block.payload_ref,
                "payload_access": "withheld",
                "withholding_reason": "token payload range overflows",
                "token_ids": Value::Null,
            });
        }
    };
    if end > lease.bytes.len() {
        return json!({
            "payload_ref": block.payload_ref,
            "payload_access": "withheld",
            "withholding_reason": "token payload range exceeds leased payload bytes",
            "token_ids": Value::Null,
        });
    }
    let token_bytes = &lease.bytes[offset..end];
    let mut token_ids = Vec::with_capacity(token_count);
    for chunk in token_bytes.chunks_exact(width) {
        let value = match block.token_id_width {
            1 => u64::from(chunk[0]),
            2 => u64::from(u16::from_le_bytes([chunk[0], chunk[1]])),
            4 => u64::from(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
            8 => u64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ]),
            _ => {
                return json!({
                    "payload_ref": block.payload_ref,
                    "payload_access": "withheld",
                    "withholding_reason": "unsupported token_id_width",
                    "token_ids": Value::Null,
                });
            }
        };
        token_ids.push(value);
    }
    json!({
        "payload_ref": block.payload_ref,
        "payload_access": lease.disclosure.as_str(),
        "token_id_width": block.token_id_width,
        "token_ids": token_ids,
        "byte_length": token_bytes.len(),
    })
}

fn ai_chunk_projection_rows(
    sidecar: &CoveAiFile,
    context_mode: bool,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
    chunk_source_context: Option<&AiChunkSourceContext>,
) -> Vec<Value> {
    sidecar
        .descriptor_tables
        .text_chunks
        .iter()
        .map(|chunk| {
            let text_projection = if include_payloads {
                Some(ai_chunk_text_projection(
                    chunk,
                    chunk_source_context,
                    sidecar,
                    payload_reader,
                ))
            } else {
                None
            };
            let mut row = json!({
                "record_kind": if context_mode { "rag_context_chunk" } else { "text_chunk" },
                "chunk_id": chunk.chunk_id,
                "source_ref": chunk.source_ref,
                "table_id": chunk.table_id,
                "column_id": chunk.column_id,
                "object_type_id": chunk.object_type_id,
                "property_id": chunk.property_id,
                "association_type_id": chunk.association_type_id,
                "path_ref": chunk.path_ref,
                "source_row_ref": chunk.source_row_ref,
                "source_object_ref": chunk.source_object_ref,
                "source_value_hash_ref": chunk.source_value_hash_ref,
                "byte_start": chunk.byte_start,
                "byte_length": chunk.byte_length,
                "unicode_scalar_start": chunk.unicode_scalar_start,
                "unicode_scalar_length": chunk.unicode_scalar_length,
                "token_start": chunk.token_start,
                "token_count": chunk.token_count,
                "parent_chunk_id": chunk.parent_chunk_id,
                "previous_chunk_id": chunk.previous_chunk_id,
                "next_chunk_id": chunk.next_chunk_id,
                "chunk_text_hash_ref": chunk.chunk_text_hash_ref,
                "evidence_ref": chunk.evidence_ref,
                "policy_ref": chunk.policy_ref,
                "descriptor_validated": true,
                "exact": true,
                "result_authority": "ValidatedAiDescriptorMetadata",
                "text": Value::Null,
                "text_withheld": true,
                "include_payloads": include_payloads,
                "withholding_reason": "chunk_text_requires_source_value_reconstruction_from_validated_cove_source",
            });
            match text_projection {
                Some(Ok(projection)) => {
                    row["text"] = json!(projection.text);
                    row["text_withheld"] = json!(false);
                    row["withholding_reason"] = Value::Null;
                    row["payload_access"] = json!("validated_source_value_reconstruction");
                    row["source_value_hash_status"] = json!(projection.source_value_hash_status);
                    row["chunk_text_hash_status"] = json!(projection.chunk_text_hash_status);
                    row["result_authority"] = json!("ValidatedAiSourceValueReconstruction");
                }
                Some(Err(reason)) => {
                    row["withholding_reason"] = json!(reason);
                    row["payload_access"] = json!("withheld");
                }
                None => {
                    row["payload_access"] = json!("not_requested");
                }
            }
            if context_mode {
                if row["text_withheld"].as_bool() == Some(false) {
                    row["prompt_context"] = row["text"].clone();
                    row["neighbor_expansion"] =
                        json!("withheld_without_verified_neighbor_expansion_policy");
                    row["redaction_report"] = json!({
                        "text_withheld": false,
                        "neighbor_chunks_withheld": true,
                        "reason": "current chunk text passed source, hash, UTF-8, and redaction checks; neighboring context remains withheld unless separately validated",
                    });
                } else {
                    row["prompt_context"] = Value::Null;
                    row["neighbor_expansion"] = json!("requires_source_value_reconstruction");
                    row["redaction_report"] = json!({
                        "text_withheld": true,
                        "neighbor_chunks_withheld": true,
                        "reason": row["withholding_reason"].clone(),
                    });
                }
            }
            row
        })
        .collect()
}

fn ai_token_projection_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    let mut rows = Vec::new();
    for block in &sidecar.descriptor_tables.token_blocks {
        rows.push(json!({
            "record_kind": "token_block",
            "token_block_id": block.token_block_id,
            "tokenizer_profile_id": block.tokenizer_profile_id,
            "token_count": block.token_count,
            "token_id_width": block.token_id_width,
            "compression_codec": block.compression_codec,
            "layout_kind": block.layout_kind,
            "payload_ref": block.payload_ref,
            "payload_offset": block.payload_offset,
            "payload_length": block.payload_length,
            "integrity_ref": block.integrity_ref,
            "descriptor_validated": true,
            "token_payload": ai_token_block_payload_projection(block, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for span in &sidecar.descriptor_tables.tokenized_spans {
        rows.push(json!({
            "record_kind": "tokenized_span",
            "tokenized_span_id": span.tokenized_span_id,
            "chunk_id": span.chunk_id,
            "tokenizer_profile_id": span.tokenizer_profile_id,
            "token_block_ref": span.token_block_ref,
            "token_offset": span.token_offset,
            "token_count": span.token_count,
            "byte_alignment_ref": span.byte_alignment_ref,
            "source_value_hash_ref": span.source_value_hash_ref,
            "descriptor_validated": true,
            "byte_alignment_payload": ai_payload_ref_projection(span.byte_alignment_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for pack in &sidecar.descriptor_tables.token_sequence_packs {
        rows.push(json!({
            "record_kind": "token_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "training_profile_ref": pack.training_profile_ref,
            "token_block_ref": pack.token_block_ref,
            "token_offset": pack.token_offset,
            "token_count": pack.token_count,
            "source_span_count": pack.source_span_count,
            "first_source_span_ref": pack.first_source_span_ref,
            "loss_mask_ref": pack.loss_mask_ref,
            "attention_mask_ref": pack.attention_mask_ref,
            "position_ids_ref": pack.position_ids_ref,
            "labels_ref": pack.labels_ref,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_ids": ai_payload_ref_projection(pack.position_ids_ref, include_payloads, payload_reader),
            "labels": ai_payload_ref_projection(pack.labels_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

fn ai_training_sample_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    sidecar
        .descriptor_tables
        .training_samples
        .iter()
        .map(|sample| {
            json!({
                "record_kind": "training_sample",
                "sample_id": sample.sample_id,
                "training_profile_id": sample.training_profile_id,
                "example_kind": sample.example_kind,
                "split_ref": sample.split_ref,
                "source_ref": sample.source_ref,
                "evidence_ref": sample.evidence_ref,
                "input_ref": sample.input_ref,
                "target_ref": sample.target_ref,
                "label_ref": sample.label_ref,
                "metadata_ref": sample.metadata_ref,
                "token_sequence_pack_ref": sample.token_sequence_pack_ref,
                "multimodal_sequence_pack_ref": sample.multimodal_sequence_pack_ref,
                "vector_ref": sample.vector_ref,
                "quality_score_ppm": sample.quality_score_ppm,
                "sample_weight_ppm": sample.sample_weight_ppm,
                "dedup_group_ref": sample.dedup_group_ref,
                "license_ref": sample.license_ref,
                "policy_ref": sample.policy_ref,
                "teacher_model_ref": sample.teacher_model_ref,
                "generator_provenance_ref": sample.generator_provenance_ref,
                "judge_generator_provenance_ref": sample.judge_generator_provenance_ref,
                "label_generator_provenance_ref": sample.label_generator_provenance_ref,
                "descriptor_validated": true,
                "input": ai_payload_ref_projection(sample.input_ref, include_payloads, payload_reader),
                "target": ai_payload_ref_projection(sample.target_ref, include_payloads, payload_reader),
                "metadata": ai_payload_ref_projection(sample.metadata_ref, include_payloads, payload_reader),
                "evidence": ai_payload_ref_projection(sample.evidence_ref, include_payloads, payload_reader),
                "payload_withheld": !include_payloads,
                "policy_withheld": !include_payloads,
                "exact": true,
                "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
            })
        })
        .collect()
}

fn ai_training_split_rows(sidecar: &CoveAiFile) -> Vec<Value> {
    sidecar
        .descriptor_tables
        .dataset_splits
        .iter()
        .map(|split| {
            let sample_count = sidecar
                .descriptor_tables
                .training_samples
                .iter()
                .filter(|sample| sample.split_ref == split.split_id)
                .count();
            json!({
                "record_kind": "dataset_split",
                "split_id": split.split_id,
                "split_name_ref": split.split_name_ref,
                "split_method": split.split_method,
                "source_snapshot_ref": split.source_snapshot_ref,
                "filter_policy_ref": split.filter_policy_ref,
                "seed": split.seed,
                "sample_count": sample_count,
                "first_sample_ref": split.first_sample_ref,
                "descriptor_validated": true,
                "payload_withheld": true,
                "withholding_reason": "descriptor_metadata_execution_does_not_expose_training_payloads",
                "exact": true,
                "result_authority": "ValidatedAiDescriptorMetadata",
            })
        })
        .collect()
}

fn ai_training_pack_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    let mut rows = Vec::new();
    for pack in &sidecar.descriptor_tables.token_sequence_packs {
        rows.push(json!({
            "record_kind": "token_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "training_profile_ref": pack.training_profile_ref,
            "token_block_ref": pack.token_block_ref,
            "token_offset": pack.token_offset,
            "token_count": pack.token_count,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_ids": ai_payload_ref_projection(pack.position_ids_ref, include_payloads, payload_reader),
            "labels": ai_payload_ref_projection(pack.labels_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for pack in &sidecar.descriptor_tables.multimodal_sequence_packs {
        rows.push(json!({
            "record_kind": "multimodal_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "training_profile_id": pack.training_profile_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "sequence_profile_ref": pack.sequence_profile_ref,
            "element_count": pack.element_count,
            "first_element_ref": pack.first_element_ref,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "label_ref": pack.label_ref,
            "evidence_ref": pack.evidence_ref,
            "generator_provenance_ref": pack.generator_provenance_ref,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_map": ai_payload_ref_projection(pack.position_map_ref, include_payloads, payload_reader),
            "label": ai_payload_ref_projection(pack.label_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(pack.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

fn ai_multimodal_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Vec<Value> {
    let mut rows = Vec::new();
    for pack in &sidecar.descriptor_tables.multimodal_sequence_packs {
        rows.push(json!({
            "record_kind": "multimodal_sequence_pack",
            "sequence_pack_id": pack.sequence_pack_id,
            "training_profile_id": pack.training_profile_id,
            "tokenizer_profile_id": pack.tokenizer_profile_id,
            "sequence_profile_ref": pack.sequence_profile_ref,
            "element_count": pack.element_count,
            "first_element_ref": pack.first_element_ref,
            "split_ref": pack.split_ref,
            "sample_weight_ppm": pack.sample_weight_ppm,
            "loss_mask_ref": pack.loss_mask_ref,
            "attention_mask_ref": pack.attention_mask_ref,
            "position_map_ref": pack.position_map_ref,
            "label_ref": pack.label_ref,
            "source_snapshot_ref": pack.source_snapshot_ref,
            "evidence_ref": pack.evidence_ref,
            "generator_provenance_ref": pack.generator_provenance_ref,
            "descriptor_validated": true,
            "loss_mask": ai_payload_ref_projection(pack.loss_mask_ref, include_payloads, payload_reader),
            "attention_mask": ai_payload_ref_projection(pack.attention_mask_ref, include_payloads, payload_reader),
            "position_map": ai_payload_ref_projection(pack.position_map_ref, include_payloads, payload_reader),
            "label": ai_payload_ref_projection(pack.label_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(pack.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for element in &sidecar.descriptor_tables.multimodal_sequence_elements {
        rows.push(json!({
            "record_kind": "multimodal_sequence_element",
            "element_id": element.element_id,
            "sequence_pack_id": element.sequence_pack_id,
            "ordinal": element.ordinal,
            "element_kind": element.element_kind,
            "modality": element.modality,
            "role": element.role,
            "tokenized_span_ref": element.tokenized_span_ref,
            "token_sequence_pack_ref": element.token_sequence_pack_ref,
            "asset_ref": element.asset_ref,
            "tensor_ref": element.tensor_ref,
            "vector_ref": element.vector_ref,
            "byte_start": element.byte_start,
            "byte_length": element.byte_length,
            "time_start_us": element.time_start_us,
            "time_duration_us": element.time_duration_us,
            "position_stream_ref": element.position_stream_ref,
            "evidence_ref": element.evidence_ref,
            "policy_ref": element.policy_ref,
            "descriptor_validated": true,
            "position_stream": ai_payload_ref_projection(element.position_stream_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(element.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

struct AiGeneratorFilterSelection {
    filtered: bool,
    provenance_ids: BTreeSet<u64>,
    actor_ids: BTreeSet<u32>,
    decoding_profile_ids: BTreeSet<u32>,
    human_review_ids: BTreeSet<u32>,
}

impl AiGeneratorFilterSelection {
    fn include_provenance(&self, id: u64) -> bool {
        !self.filtered || self.provenance_ids.contains(&id)
    }

    fn include_actor(&self, id: u32) -> bool {
        !self.filtered || self.actor_ids.contains(&id)
    }

    fn include_decoding_profile(&self, id: u32) -> bool {
        !self.filtered || self.decoding_profile_ids.contains(&id)
    }

    fn include_human_review(&self, id: u32) -> bool {
        !self.filtered || self.human_review_ids.contains(&id)
    }

    fn include_label(&self, generator_provenance_ref: u64, human_review_ref: u32) -> bool {
        !self.filtered
            || (generator_provenance_ref != 0
                && self.provenance_ids.contains(&generator_provenance_ref))
            || (human_review_ref != 0 && self.human_review_ids.contains(&human_review_ref))
    }

    fn include_preference_pair(
        &self,
        judge_generator_provenance_ref: u64,
        human_review_ref: u32,
    ) -> bool {
        !self.filtered
            || (judge_generator_provenance_ref != 0
                && self
                    .provenance_ids
                    .contains(&judge_generator_provenance_ref))
            || (human_review_ref != 0 && self.human_review_ids.contains(&human_review_ref))
    }
}

fn ai_generator_filter_selection(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    filter: &AiGeneratorAuditFilter,
) -> AiGeneratorFilterSelection {
    let mut selection = AiGeneratorFilterSelection {
        filtered: !filter.is_empty(),
        provenance_ids: BTreeSet::new(),
        actor_ids: BTreeSet::new(),
        decoding_profile_ids: BTreeSet::new(),
        human_review_ids: BTreeSet::new(),
    };
    if filter.is_empty() {
        return selection;
    }

    for provenance in &sidecar.descriptor_tables.generator_provenance {
        if !ai_generator_provenance_matches_filter(sidecar, payload_reader, provenance, filter) {
            continue;
        }
        selection
            .provenance_ids
            .insert(provenance.generator_provenance_id);
        if provenance.model_actor_ref != 0 {
            selection.actor_ids.insert(provenance.model_actor_ref);
        }
        if provenance.decoding_profile_ref != 0 {
            selection
                .decoding_profile_ids
                .insert(provenance.decoding_profile_ref);
        }
        if provenance.human_review_ref != 0 {
            selection
                .human_review_ids
                .insert(provenance.human_review_ref);
        }
    }
    selection
}

fn ai_generator_provenance_matches_filter(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    provenance: &cove_core::artifact::coveai::GeneratorProvenanceV1,
    filter: &AiGeneratorAuditFilter,
) -> bool {
    if let Some(class) = filter.reproducibility_class {
        if provenance.reproducibility_class != class {
            return false;
        }
    }
    if let Some(decoding_profile_ref) = filter.decoding_profile_ref {
        if provenance.decoding_profile_ref != decoding_profile_ref {
            return false;
        }
    }
    if let Some(status) = filter.human_review_status {
        let reviewed = provenance.human_review_ref != 0;
        if (status == AiHumanReviewStatusFilter::Reviewed && !reviewed)
            || (status == AiHumanReviewStatusFilter::Unreviewed && reviewed)
        {
            return false;
        }
    }

    if filter.model_namespace.is_empty()
        && filter.model_name.is_empty()
        && filter.model_version.is_empty()
        && filter.provider.is_empty()
        && filter.endpoint.is_empty()
    {
        return true;
    }
    let Some(actor) = sidecar
        .descriptor_tables
        .model_actors
        .iter()
        .find(|actor| actor.model_actor_id == provenance.model_actor_ref)
    else {
        return false;
    };
    ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.model_namespace_ref,
        &filter.model_namespace,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.model_name_ref,
        &filter.model_name,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.model_version_ref,
        &filter.model_version,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.provider_ref,
        &filter.provider,
    ) && ai_string_ref_matches(
        sidecar,
        payload_reader,
        actor.endpoint_ref,
        &filter.endpoint,
    )
}

fn ai_string_ref_matches(
    sidecar: &CoveAiFile,
    payload_reader: &AiPayloadReader<'_>,
    string_ref: u32,
    filter: &AiStringRefFilter,
) -> bool {
    if filter.is_empty() {
        return true;
    }
    if filter.ref_id.is_some_and(|ref_id| ref_id == string_ref) {
        return true;
    }
    let Some(expected) = filter.value.as_deref() else {
        return false;
    };
    let Some(entry) = sidecar
        .descriptor_tables
        .strings
        .iter()
        .find(|entry| entry.string_ref == string_ref)
    else {
        return false;
    };
    let Ok(lease) = payload_reader.lease_payload_ref(entry.payload_ref) else {
        return false;
    };
    std::str::from_utf8(lease.bytes).is_ok_and(|actual| actual == expected)
}

fn ai_generator_audit_rows(
    sidecar: &CoveAiFile,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
    filter: &AiGeneratorAuditFilter,
) -> Vec<Value> {
    let mut rows = Vec::new();
    let selection = ai_generator_filter_selection(sidecar, payload_reader, filter);
    for actor in &sidecar.descriptor_tables.model_actors {
        if !selection.include_actor(actor.model_actor_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "model_actor",
            "model_actor_id": actor.model_actor_id,
            "model_namespace_ref": actor.model_namespace_ref,
            "model_name_ref": actor.model_name_ref,
            "model_version_ref": actor.model_version_ref,
            "model_checkpoint_digest_ref": actor.model_checkpoint_digest_ref,
            "provider_ref": actor.provider_ref,
            "endpoint_ref": actor.endpoint_ref,
            "endpoint_version_ref": actor.endpoint_version_ref,
            "model_family_ref": actor.model_family_ref,
            "modality_mask": actor.modality_mask,
            "license_ref": actor.license_ref,
            "policy_ref": actor.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "payload_withheld": false,
            "exact": true,
            "result_authority": "ValidatedAiDescriptorMetadata",
        }));
    }
    for profile in &sidecar.descriptor_tables.generation_decoding_profiles {
        if !selection.include_decoding_profile(profile.decoding_profile_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "generation_decoding_profile",
            "decoding_profile_id": profile.decoding_profile_id,
            "temperature_micros": profile.temperature_micros,
            "top_p_micros": profile.top_p_micros,
            "top_k": profile.top_k,
            "seed": profile.seed,
            "max_output_tokens": profile.max_output_tokens,
            "stop_sequence_ref": profile.stop_sequence_ref,
            "safety_policy_ref": profile.safety_policy_ref,
            "deterministic_claim": profile.deterministic_claim,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "stop_sequence": ai_payload_ref_projection(profile.stop_sequence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for review in &sidecar.descriptor_tables.human_reviews {
        if !selection.include_human_review(review.human_review_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "human_review",
            "human_review_id": review.human_review_id,
            "review_kind": review.review_kind,
            "reviewer_role_ref": review.reviewer_role_ref,
            "review_time_us": review.review_time_us,
            "rating_ppm": review.rating_ppm,
            "notes_ref": review.notes_ref,
            "policy_ref": review.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "notes": ai_payload_ref_projection(review.notes_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for provenance in &sidecar.descriptor_tables.generator_provenance {
        if !selection.include_provenance(provenance.generator_provenance_id) {
            continue;
        }
        rows.push(json!({
            "record_kind": "generator_provenance",
            "generator_provenance_id": provenance.generator_provenance_id,
            "generator_kind": provenance.generator_kind,
            "model_actor_ref": provenance.model_actor_ref,
            "prompt_template_ref": provenance.prompt_template_ref,
            "decoding_profile_ref": provenance.decoding_profile_ref,
            "toolchain_ref": provenance.toolchain_ref,
            "source_input_ref": provenance.source_input_ref,
            "source_context_ref": provenance.source_context_ref,
            "source_sample_ref": provenance.source_sample_ref,
            "parent_generator_provenance_ref": provenance.parent_generator_provenance_ref,
            "generation_time_us": provenance.generation_time_us,
            "confidence_ppm": provenance.confidence_ppm,
            "human_review_ref": provenance.human_review_ref,
            "policy_ref": provenance.policy_ref,
            "reproducibility_class": provenance.reproducibility_class,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "prompt_template": ai_payload_ref_projection(provenance.prompt_template_ref, include_payloads, payload_reader),
            "toolchain": ai_payload_ref_projection(provenance.toolchain_ref, include_payloads, payload_reader),
            "source_input": ai_payload_ref_projection(provenance.source_input_ref, include_payloads, payload_reader),
            "source_context": ai_payload_ref_projection(provenance.source_context_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "external_audit_only_unless_deterministic_regeneration_proven": true,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for label in &sidecar.descriptor_tables.training_labels {
        if !selection.include_label(label.generator_provenance_ref, label.human_review_ref) {
            continue;
        }
        rows.push(json!({
            "record_kind": "training_label",
            "label_id": label.label_id,
            "label_kind": label.label_kind,
            "label_authority": label.label_authority,
            "label_payload_ref": label.label_payload_ref,
            "generator_provenance_ref": label.generator_provenance_ref,
            "human_review_ref": label.human_review_ref,
            "confidence_ppm": label.confidence_ppm,
            "evidence_ref": label.evidence_ref,
            "policy_ref": label.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "label_payload": ai_payload_ref_projection(label.label_payload_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(label.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    for pair in &sidecar.descriptor_tables.preference_pairs {
        if !selection
            .include_preference_pair(pair.judge_generator_provenance_ref, pair.human_review_ref)
        {
            continue;
        }
        rows.push(json!({
            "record_kind": "preference_pair",
            "preference_pair_id": pair.preference_pair_id,
            "prompt_ref": pair.prompt_ref,
            "chosen_ref": pair.chosen_ref,
            "rejected_ref": pair.rejected_ref,
            "judge_generator_provenance_ref": pair.judge_generator_provenance_ref,
            "human_review_ref": pair.human_review_ref,
            "preference_strength_ppm": pair.preference_strength_ppm,
            "confidence_ppm": pair.confidence_ppm,
            "evidence_ref": pair.evidence_ref,
            "policy_ref": pair.policy_ref,
            "descriptor_validated": true,
            "generator_filter_matched": selection.filtered,
            "prompt": ai_payload_ref_projection(pair.prompt_ref, include_payloads, payload_reader),
            "chosen": ai_payload_ref_projection(pair.chosen_ref, include_payloads, payload_reader),
            "rejected": ai_payload_ref_projection(pair.rejected_ref, include_payloads, payload_reader),
            "evidence": ai_payload_ref_projection(pair.evidence_ref, include_payloads, payload_reader),
            "payload_withheld": !include_payloads,
            "exact": true,
            "result_authority": if include_payloads { "ValidatedAiPayloadLease" } else { "ValidatedAiDescriptorMetadata" },
        }));
    }
    rows
}

pub(super) fn try_ai_embedding_executed_query(
    physical: &PhysicalPlannedQuery,
    execution_options: &ExecutionOptions,
    mode: KernelExecutionMode,
) -> Result<Option<(ExecutedQuery, KernelExecutionReport)>, BuildExecutionError> {
    let Some(operation) = physical
        .planned
        .resolved
        .method_chain
        .ai_operations
        .iter()
        .find(|operation| operation.operation == CoveQlAiOperation::Embedding)
    else {
        return Ok(None);
    };
    if operation.method_name != "embedding" {
        return Err(exec_error(
            "E_AI_OPERATION_UNSUPPORTED",
            ".embedding() supports fileCode or vectorRef over a validated COVE-AI sidecar",
            json!({
                "method": operation.method_name,
                "operation": format!("{:?}", operation.operation),
            }),
        ));
    }
    if mode == KernelExecutionMode::ForceMaterialized {
        return Err(exec_error(
            "E_AI_MATERIALIZED_UNSUPPORTED",
            "CoveQL-AI embedding lookup requires a validated COVE-AI sidecar; materialized COVE-O readback has no equivalent .embedding() baseline",
            json!({ "method": operation.method_name }),
        ));
    }
    if !matches!(
        physical.planned.resolved.output_mode,
        crate::CoveQlOutputMode::JsonRows
    ) {
        return Err(exec_error(
            "E_AI_OUTPUT_UNSUPPORTED",
            "CoveQL-AI embedding lookup currently returns JSON rows",
            json!({ "output_mode": format!("{:?}", physical.planned.resolved.output_mode) }),
        ));
    }
    let Some(sidecar_bytes) = physical.sidecars.cove_ai_artifact_bytes.as_deref() else {
        return Err(exec_error(
            "E_AI_SIDECAR_REQUIRED",
            "CoveQL-AI .embedding() requires a supplied COVE-AI/COVE-VEC sidecar",
            json!({ "method": operation.method_name }),
        ));
    };
    let embedding_request = ai_embedding_request_args(operation)?;
    let sidecar_summary =
        CoveAiFile::parse_for_operation(sidecar_bytes, operation.operation.operation_kind())
            .map_err(|error| {
                exec_error(
                    "E_AI_SIDECAR_INVALID",
                    format!("COVE-AI sidecar validation failed: {error}"),
                    json!({ "method": operation.method_name }),
                )
            })?;
    let binding_count = sidecar_summary
        .descriptor_tables
        .filecode_vector_bindings
        .len();
    let vector_space_count = sidecar_summary.descriptor_tables.vector_spaces.len();
    let started = Instant::now();
    let embedding = ai_embedding(sidecar_bytes, &embedding_request).map_err(|error| {
        exec_error(
            "E_AI_EMBEDDING_FAILED",
            format!("COVE-AI embedding lookup failed: {error}"),
            json!({
                "method": operation.method_name,
                "file_code": embedding_request.file_code,
                "vector_ref": embedding_request.vector_ref,
            }),
        )
    })?;
    let result = CoveQlExecutionResult::JsonRows(vec![json!({
        "target_kind": embedding.target_kind,
        "file_code": embedding.file_code,
        "vector_ref": embedding.vector_ref,
        "vector_space_id": embedding.vector_space_id,
        "dimension_count": embedding.dimension_count,
        "element_type": embedding.element_type,
        "embedding": embedding.values,
        "exact": true,
        "result_authority": embedding.result_authority,
    })]);
    let row_counts = ExecutionRowCounts {
        input_rows: binding_count,
        filtered_rows: 1,
        output_rows: 1,
    };
    enforce_result_budgets(
        &result,
        &row_counts,
        &physical.planned,
        execution_options,
        started,
    )?;
    let output_fingerprint = result_fingerprint(&result)?;

    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        "W_AI_FILECODE_EMBEDDING_EXECUTED",
        "CoveQL-AI .embedding() executed with validated COVE-AI vector lookup",
        json!({
            "file_code": embedding_request.file_code,
            "vector_ref": embedding_request.vector_ref,
            "dimension_count": embedding.dimension_count,
            "sidecar_vector_spaces": vector_space_count,
            "sidecar_filecode_bindings": binding_count,
            "result_authority": embedding.result_authority,
            "materialized_baseline_available": false,
        }),
    ));
    if mode == KernelExecutionMode::CompareWithMaterialized {
        diagnostics.push(exec_warning(
            "W_AI_NO_MATERIALIZED_BASELINE",
            "CoveQL-AI .embedding() has no materialized COVE-O baseline oracle; exact sidecar result was not fingerprint-compared",
            json!({ "file_code": embedding_request.file_code, "vector_ref": embedding_request.vector_ref }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(&physical.planned) {
        diagnostics.push(warning);
    }

    let mut kernel_report = KernelExecutionReport::applied(
        mode,
        KernelCounters {
            rows_scanned: binding_count,
            rows_after_bitmap: binding_count,
            rows_after_selection_vector: 1,
            output_rows: 1,
            bytes_touched_estimate: sidecar_bytes.len(),
            dictionary_lookups_at_materialization: 1,
            ..KernelCounters::default()
        },
    );
    kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
        "COVE-AI embedding lookup used validated COVE-VEC descriptor tables, payload integrity, privacy summary, and FileCode-local bindings",
    );
    kernel_report.decision = KernelDecision::new(
        KernelDecisionKind::Applied,
        "CoveQL-AI .embedding() FileCode vector lookup executed without materialized fallback",
        json!({
            "file_code": embedding_request.file_code,
            "vector_ref": embedding_request.vector_ref,
            "dimension_count": embedding.dimension_count,
            "sidecar_filecode_bindings": binding_count,
            "sidecar_vector_spaces": vector_space_count,
            "result_authority": embedding.result_authority,
            "fallback_boundary": Value::Null,
            "materialized_baseline_available": false,
        }),
        false,
    );
    kernel_report.decisions = vec![kernel_report.decision.clone()];
    kernel_report.kernel_fingerprint = Some(output_fingerprint.clone());
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);

    let executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_applicable(
            &execution_options.pushdown,
            "CoveQL-AI embedding lookup reads a validated COVE-VEC sidecar binding rather than COVE-O row pages",
        ),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_kernel(
            "CoveQL-AI FileCode embedding lookup returned a vector from a validated COVE-VEC sidecar",
            false,
        ),
    };
    Ok(Some((executed, kernel_report)))
}

pub(super) fn try_ai_semantic_search_executed_query(
    physical: &PhysicalPlannedQuery,
    execution_options: &ExecutionOptions,
    mode: KernelExecutionMode,
) -> Result<Option<(ExecutedQuery, KernelExecutionReport)>, BuildExecutionError> {
    let Some(operation) = physical
        .planned
        .resolved
        .method_chain
        .ai_operations
        .iter()
        .find(|operation| operation.operation == CoveQlAiOperation::SemanticSearch)
    else {
        return Ok(None);
    };
    let method_name = operation.method_name.as_str();
    if !matches!(method_name, "similar" | "hybrid" | "rerank") {
        return Err(exec_error(
            "E_AI_OPERATION_UNSUPPORTED",
            "CoveQL-AI semantic search physical execution supports .similar(), .hybrid(), and .rerank() with fileCode/vectorRef/k/target/index arguments",
            json!({
                "method": operation.method_name,
                "operation": format!("{:?}", operation.operation),
            }),
        ));
    }
    let exact_semantic_authority = method_name == "similar";
    let result_authority = if exact_semantic_authority {
        "ExactOptimizedKernel"
    } else {
        "RuntimeAdvisory"
    };
    if mode == KernelExecutionMode::ForceMaterialized {
        return Err(exec_error(
            "E_AI_MATERIALIZED_UNSUPPORTED",
            "CoveQL-AI semantic search requires a validated COVE-AI sidecar; materialized COVE-O readback has no equivalent AI semantic-search baseline",
            json!({ "method": operation.method_name }),
        ));
    }
    if !matches!(
        physical.planned.resolved.output_mode,
        crate::CoveQlOutputMode::JsonRows
    ) {
        return Err(exec_error(
            "E_AI_OUTPUT_UNSUPPORTED",
            "CoveQL-AI semantic search currently returns JSON rows",
            json!({ "output_mode": format!("{:?}", physical.planned.resolved.output_mode) }),
        ));
    }
    let Some(sidecar_bytes) = physical.sidecars.cove_ai_artifact_bytes.as_deref() else {
        return Err(exec_error(
            "E_AI_SIDECAR_REQUIRED",
            "CoveQL-AI semantic search requires a supplied COVE-AI/COVE-VEC sidecar",
            json!({ "method": operation.method_name }),
        ));
    };
    let search_plan = ai_vector_search_plan_args(operation)?;
    let sidecar_summary =
        CoveAiFile::parse_for_operation(sidecar_bytes, operation.operation.operation_kind())
            .map_err(|error| {
                exec_error(
                    "E_AI_SIDECAR_INVALID",
                    format!("COVE-AI sidecar validation failed: {error}"),
                    json!({ "method": operation.method_name }),
                )
            })?;
    let binding_count = sidecar_summary
        .descriptor_tables
        .filecode_vector_bindings
        .len()
        + sidecar_summary
            .descriptor_tables
            .chunk_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .object_state_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .training_sample_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .association_state_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .asset_vector_bindings
            .len()
        + sidecar_summary
            .descriptor_tables
            .multimodal_sequence_vector_bindings
            .len();
    let vector_space_count = sidecar_summary.descriptor_tables.vector_spaces.len();
    let started = Instant::now();
    let results = ai_vector_search(sidecar_bytes, &search_plan).map_err(|error| {
        exec_error(
            "E_AI_SEMANTIC_SEARCH_FAILED",
            format!("COVE-AI semantic search failed: {error}"),
            json!({
                "method": operation.method_name,
                "query_file_code": search_plan.query_file_code,
                "query_vector_ref": search_plan.query_vector_ref,
                "top_k": search_plan.top_k,
                "target": search_plan.target_kind.as_str(),
                "index": search_plan.index.as_str(),
            }),
        )
    })?;
    let rows = ai_semantic_search_rows(
        method_name,
        &search_plan,
        &results,
        exact_semantic_authority,
    );
    let vector_exact = results.iter().all(|result| result.exact);
    let fallback_used = results.iter().any(|result| result.fallback_used);
    let selected_index = results
        .first()
        .map(|result| result.selected_index.clone())
        .unwrap_or_else(|| search_plan.index.as_str().to_string());
    let selected_result_authority = results
        .first()
        .map(|result| result.result_authority.clone())
        .unwrap_or_else(|| result_authority.to_string());
    let result = CoveQlExecutionResult::JsonRows(rows);
    let row_counts = ExecutionRowCounts {
        input_rows: binding_count,
        filtered_rows: results.len(),
        output_rows: results.len(),
    };
    enforce_result_budgets(
        &result,
        &row_counts,
        &physical.planned,
        execution_options,
        started,
    )?;
    let output_fingerprint = result_fingerprint(&result)?;

    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        if exact_semantic_authority {
            "W_AI_EXACT_FLAT_VECTOR_SCAN_EXECUTED"
        } else {
            "W_AI_ADVISORY_VECTOR_SCAN_EXECUTED"
        },
        if exact_semantic_authority {
            "CoveQL-AI .similar() executed with validated COVE-AI vector search"
        } else {
            "CoveQL-AI semantic-search advisory method executed with validated vector candidates but without persisted hybrid/rerank authority"
        },
        json!({
            "method": method_name,
            "query_file_code": search_plan.query_file_code,
            "query_vector_ref": search_plan.query_vector_ref,
            "top_k": search_plan.top_k,
            "target": search_plan.target_kind.as_str(),
            "index": search_plan.index.as_str(),
            "result_count": results.len(),
            "sidecar_vector_spaces": vector_space_count,
            "sidecar_vector_bindings": binding_count,
            "selected_index": selected_index.clone(),
            "vector_exact": vector_exact,
            "fallback_used": fallback_used,
            "semantic_exact": exact_semantic_authority,
            "result_authority": selected_result_authority.clone(),
            "materialized_baseline_available": false,
        }),
    ));
    if mode == KernelExecutionMode::CompareWithMaterialized {
        diagnostics.push(exec_warning(
            "W_AI_NO_MATERIALIZED_BASELINE",
            "CoveQL-AI semantic search has no materialized COVE-O baseline oracle; sidecar result was not fingerprint-compared",
            json!({ "method": method_name, "query_file_code": search_plan.query_file_code, "query_vector_ref": search_plan.query_vector_ref }),
        ));
    }
    if let Some(warning) = zero_copy_owned_fallback_warning(&physical.planned) {
        diagnostics.push(warning);
    }

    let mut kernel_report = KernelExecutionReport::applied(
        mode,
        KernelCounters {
            rows_scanned: binding_count,
            rows_after_bitmap: binding_count,
            rows_after_selection_vector: results.len(),
            output_rows: results.len(),
            bytes_touched_estimate: sidecar_bytes.len(),
            dictionary_lookups_at_materialization: results.len(),
            ..KernelCounters::default()
        },
    );
    kernel_report.optimization_authority = OptimizationAuthorityReport::authoritative(
        "COVE-AI vector search used validated COVE-VEC descriptor tables, payload integrity, privacy summary, vector bindings, and exactness labels",
    );
    kernel_report.decision = KernelDecision::new(
        KernelDecisionKind::Applied,
        "CoveQL-AI semantic-search vector scan executed without materialized fallback",
        json!({
            "method": method_name,
            "query_file_code": search_plan.query_file_code,
            "query_vector_ref": search_plan.query_vector_ref,
            "top_k": search_plan.top_k,
            "target": search_plan.target_kind.as_str(),
            "index": search_plan.index.as_str(),
            "result_count": results.len(),
            "sidecar_vector_bindings": binding_count,
            "sidecar_vector_spaces": vector_space_count,
            "selected_index": selected_index,
            "vector_exact": vector_exact,
            "semantic_exact": exact_semantic_authority,
            "fallback_used": fallback_used,
            "result_authority": selected_result_authority,
            "fallback_boundary": Value::Null,
            "materialized_baseline_available": false,
        }),
        false,
    );
    kernel_report.decisions = vec![kernel_report.decision.clone()];
    kernel_report.kernel_fingerprint = Some(output_fingerprint.clone());
    attach_phase8_plan_reports(&mut kernel_report, &physical.planned);

    let executed = ExecutedQuery {
        planned: physical.planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_applicable(
            &execution_options.pushdown,
            "CoveQL-AI vector search reads validated COVE-VEC sidecar bindings rather than COVE-O row pages",
        ),
        evidence_authority: None,
        authority: if exact_semantic_authority && vector_exact {
            ExecutionAuthorityReport::exact_kernel(
                "CoveQL-AI vector search produced exact ranked results from a validated COVE-VEC sidecar",
                false,
            )
        } else {
            ExecutionAuthorityReport::physical_plan_only(
                "CoveQL-AI vector search produced advisory or approximate ranked results with per-result exactness labels",
            )
        },
    };
    Ok(Some((executed, kernel_report)))
}

fn ai_vector_search_plan_args(
    operation: &crate::ResolvedAiOperation,
) -> Result<AiVectorSearchPlan, BuildExecutionError> {
    let mut query_file_code = None;
    let mut query_vector_ref = None;
    let mut top_k = None;
    let mut target_kind = AiVectorSearchTargetKind::FileCode;
    let mut index = AiVectorIndexSelection::Auto;
    let mut unnamed_integer_index = 0usize;
    for arg in &operation.args {
        match arg.name.as_deref() {
            Some("fileCode" | "file_code" | "queryFileCode" | "query_file_code" | "query") => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search fileCode/queryFileCode must be an integer literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                query_file_code = Some(u32::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search fileCode argument exceeds u32",
                        json!({ "value": value }),
                    )
                })?);
            }
            Some("vectorRef" | "vector_ref" | "queryVectorRef" | "query_vector_ref") => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search vectorRef must be an integer literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                query_vector_ref = Some(value);
            }
            Some("k" | "topK" | "top_k" | "limit") => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search k/topK must be an integer literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                top_k = Some(usize::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search k argument exceeds usize",
                        json!({ "value": value }),
                    )
                })?);
            }
            Some("target" | "targetKind" | "target_kind") => {
                let value = ai_argument_string(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search target must be a string literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                target_kind = ai_vector_target_kind(&value)?;
            }
            Some("index" | "indexKind" | "index_kind") => {
                let value = ai_argument_string(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search index must be a string literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
                index = ai_vector_index_selection(&value)?;
            }
            None if unnamed_integer_index == 0 => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search first unnamed argument must be integer fileCode",
                        json!({}),
                    )
                })?;
                query_file_code = Some(u32::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search fileCode argument exceeds u32",
                        json!({ "value": value }),
                    )
                })?);
                unnamed_integer_index += 1;
            }
            None if unnamed_integer_index == 1 => {
                let value = ai_argument_u64(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI semantic search second unnamed argument must be integer k",
                        json!({}),
                    )
                })?;
                top_k = Some(usize::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        "CoveQL-AI semantic search k argument exceeds usize",
                        json!({ "value": value }),
                    )
                })?);
                unnamed_integer_index += 1;
            }
            Some(name) => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI semantic search supports fileCode/queryFileCode, vectorRef, k, target, and index arguments",
                    json!({ "argument": name }),
                ));
            }
            None => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI semantic search supports at most two unnamed integer arguments: fileCode, k",
                    json!({}),
                ));
            }
        }
    }
    if query_file_code.is_some() && query_vector_ref.is_some() {
        return Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI semantic search accepts either fileCode/queryFileCode or vectorRef, not both",
            json!({ "file_code": query_file_code, "vector_ref": query_vector_ref }),
        ));
    }
    if query_file_code.is_none() && query_vector_ref.is_none() {
        return Err(exec_error(
            "E_AI_ARGUMENT_REQUIRED",
            "CoveQL-AI semantic search requires fileCode/queryFileCode or vectorRef",
            json!({}),
        ));
    };
    Ok(AiVectorSearchPlan {
        query_file_code,
        query_vector_ref,
        query_values: None,
        top_k: top_k.unwrap_or(10),
        target_kind,
        index,
    })
}

fn ai_embedding_request_args(
    operation: &crate::ResolvedAiOperation,
) -> Result<AiEmbeddingRequest, BuildExecutionError> {
    let mut file_code = None;
    let mut vector_ref = None;
    for arg in &operation.args {
        let value = ai_argument_u64(&arg.value).ok_or_else(|| {
            exec_error(
                "E_AI_ARGUMENT_UNSUPPORTED",
                ".embedding() requires integer fileCode or vectorRef arguments",
                json!({ "argument": arg.name.clone() }),
            )
        })?;
        match arg.name.as_deref() {
            Some("fileCode" | "file_code" | "queryFileCode" | "query_file_code" | "query")
            | None
                if file_code.is_none() && vector_ref.is_none() =>
            {
                file_code = Some(u32::try_from(value).map_err(|_| {
                    exec_error(
                        "E_AI_ARGUMENT_RANGE",
                        ".embedding() fileCode argument exceeds u32",
                        json!({ "value": value }),
                    )
                })?);
            }
            Some("vectorRef" | "vector_ref") => {
                vector_ref = Some(value);
            }
            Some(name) => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    ".embedding() supports fileCode or vectorRef arguments",
                    json!({ "argument": name }),
                ));
            }
            None => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    ".embedding() supports one unnamed integer argument: fileCode",
                    json!({}),
                ));
            }
        }
    }
    if file_code.is_some() && vector_ref.is_some() {
        return Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            ".embedding() accepts either fileCode or vectorRef, not both",
            json!({ "file_code": file_code, "vector_ref": vector_ref }),
        ));
    }
    if file_code.is_none() && vector_ref.is_none() {
        return Err(exec_error(
            "E_AI_ARGUMENT_REQUIRED",
            ".embedding() requires fileCode or vectorRef",
            json!({}),
        ));
    }
    Ok(AiEmbeddingRequest {
        file_code,
        vector_ref,
    })
}

fn ai_vector_target_kind(value: &str) -> Result<AiVectorSearchTargetKind, BuildExecutionError> {
    match value {
        "all" => Ok(AiVectorSearchTargetKind::All),
        "fileCode" | "file_code" | "file-code" => Ok(AiVectorSearchTargetKind::FileCode),
        "chunk" | "chunks" => Ok(AiVectorSearchTargetKind::Chunk),
        "object" | "objectState" | "object_state" | "object-state" => {
            Ok(AiVectorSearchTargetKind::ObjectState)
        }
        "association"
        | "associationState"
        | "association_state"
        | "association-state"
        | "edge" => Ok(AiVectorSearchTargetKind::AssociationState),
        "sample" | "trainingSample" | "training_sample" | "training-sample" => {
            Ok(AiVectorSearchTargetKind::TrainingSample)
        }
        "asset" | "assets" => Ok(AiVectorSearchTargetKind::Asset),
        "multimodal"
        | "multimodalSequence"
        | "multimodal_sequence"
        | "multimodal-sequence"
        | "sequence" => Ok(AiVectorSearchTargetKind::MultimodalSequence),
        other => Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI semantic search target must be all, file_code, chunk, object_state, association_state, training_sample, asset, or multimodal_sequence",
            json!({ "target": other }),
        )),
    }
}

fn ai_vector_index_selection(value: &str) -> Result<AiVectorIndexSelection, BuildExecutionError> {
    match value {
        "auto" => Ok(AiVectorIndexSelection::Auto),
        "exact" | "exactFlat" | "exact_flat" | "exact-flat" => {
            Ok(AiVectorIndexSelection::ExactFlat)
        }
        "hnsw" => Ok(AiVectorIndexSelection::Hnsw),
        "ivf" | "ivfFlat" | "ivf_flat" | "ivf-flat" => Ok(AiVectorIndexSelection::IvfFlat),
        "ivfPq" | "ivf_pq" | "ivf-pq" => Ok(AiVectorIndexSelection::IvfPq),
        "diskann" | "diskAnn" | "disk_ann" | "disk-ann" => Ok(AiVectorIndexSelection::DiskAnn),
        "vamana" => Ok(AiVectorIndexSelection::Vamana),
        other => Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI semantic search index must be auto, exact_flat, hnsw, ivf_flat, ivf_pq, diskann, or vamana",
            json!({ "index": other }),
        )),
    }
}

fn ai_include_payloads_arg(
    operation: &crate::ResolvedAiOperation,
) -> Result<bool, BuildExecutionError> {
    let mut include_payloads = true;
    for arg in &operation.args {
        match arg.name.as_deref() {
            Some("includePayloads" | "include_payloads" | "payloads") => {
                include_payloads = ai_argument_bool(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI includePayloads argument must be a boolean literal",
                        json!({ "argument": arg.name.clone() }),
                    )
                })?;
            }
            Some(name) => {
                if operation.method_name == "generatorAudit" && ai_generator_filter_arg_name(name) {
                    continue;
                }
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI descriptor/runtime projection supports only includePayloads for this method",
                    json!({ "method": operation.method_name, "argument": name }),
                ));
            }
            None => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI descriptor/runtime projection requires named arguments",
                    json!({ "method": operation.method_name }),
                ));
            }
        }
    }
    Ok(include_payloads)
}

fn ai_generator_filter_arg_name(name: &str) -> bool {
    matches!(
        name,
        "modelNamespace"
            | "model_namespace"
            | "modelNamespaceRef"
            | "model_namespace_ref"
            | "modelName"
            | "model_name"
            | "modelNameRef"
            | "model_name_ref"
            | "modelVersion"
            | "model_version"
            | "modelVersionRef"
            | "model_version_ref"
            | "provider"
            | "providerRef"
            | "provider_ref"
            | "endpoint"
            | "endpointRef"
            | "endpoint_ref"
            | "decodingProfile"
            | "decoding_profile"
            | "decodingProfileRef"
            | "decoding_profile_ref"
            | "humanReviewStatus"
            | "human_review_status"
            | "reproducibilityClass"
            | "reproducibility_class"
    )
}

fn ai_generator_audit_filter(
    operation: &crate::ResolvedAiOperation,
) -> Result<AiGeneratorAuditFilter, BuildExecutionError> {
    let mut filter = AiGeneratorAuditFilter::default();
    for arg in &operation.args {
        let Some(name) = arg.name.as_deref() else {
            return Err(exec_error(
                "E_AI_ARGUMENT_UNSUPPORTED",
                "CoveQL-AI generatorAudit filter arguments must be named",
                json!({ "method": operation.method_name }),
            ));
        };
        match name {
            "includePayloads" | "include_payloads" | "payloads" => {}
            "modelNamespace" | "model_namespace" | "modelNamespaceRef" | "model_namespace_ref" => {
                filter.model_namespace = ai_string_ref_filter(name, &arg.value)?;
            }
            "modelName" | "model_name" | "modelNameRef" | "model_name_ref" => {
                filter.model_name = ai_string_ref_filter(name, &arg.value)?;
            }
            "modelVersion" | "model_version" | "modelVersionRef" | "model_version_ref" => {
                filter.model_version = ai_string_ref_filter(name, &arg.value)?;
            }
            "provider" | "providerRef" | "provider_ref" => {
                filter.provider = ai_string_ref_filter(name, &arg.value)?;
            }
            "endpoint" | "endpointRef" | "endpoint_ref" => {
                filter.endpoint = ai_string_ref_filter(name, &arg.value)?;
            }
            "decodingProfile"
            | "decoding_profile"
            | "decodingProfileRef"
            | "decoding_profile_ref" => {
                filter.decoding_profile_ref =
                    Some(ai_argument_u32(name, &arg.value, "decoding profile")?);
            }
            "humanReviewStatus" | "human_review_status" => {
                let value = ai_argument_string(&arg.value).ok_or_else(|| {
                    exec_error(
                        "E_AI_ARGUMENT_UNSUPPORTED",
                        "CoveQL-AI generatorAudit humanReviewStatus must be reviewed or unreviewed",
                        json!({ "argument": name }),
                    )
                })?;
                filter.human_review_status = Some(match value.as_str() {
                    "reviewed" | "humanReviewed" | "human_reviewed" => {
                        AiHumanReviewStatusFilter::Reviewed
                    }
                    "unreviewed" | "notReviewed" | "not_reviewed" => {
                        AiHumanReviewStatusFilter::Unreviewed
                    }
                    _ => {
                        return Err(exec_error(
                            "E_AI_ARGUMENT_UNSUPPORTED",
                            "CoveQL-AI generatorAudit humanReviewStatus must be reviewed or unreviewed",
                            json!({ "argument": name, "value": value }),
                        ));
                    }
                });
            }
            "reproducibilityClass" | "reproducibility_class" => {
                filter.reproducibility_class =
                    Some(ai_reproducibility_class_filter(name, &arg.value)?);
            }
            _ => {
                return Err(exec_error(
                    "E_AI_ARGUMENT_UNSUPPORTED",
                    "CoveQL-AI generatorAudit supports model namespace/name/version, provider, endpoint, decoding profile, human review status, reproducibility class, and includePayloads arguments",
                    json!({ "argument": name }),
                ));
            }
        }
    }
    Ok(filter)
}

fn ai_string_ref_filter(
    name: &str,
    value: &ResolvedAiArgumentValue,
) -> Result<AiStringRefFilter, BuildExecutionError> {
    if name.ends_with("Ref") || name.ends_with("_ref") {
        return Ok(AiStringRefFilter {
            value: None,
            ref_id: Some(ai_argument_u32(name, value, "string ref")?),
        });
    }
    if let Some(ref_id) = ai_argument_u64(value).and_then(|value| u32::try_from(value).ok()) {
        return Ok(AiStringRefFilter {
            value: None,
            ref_id: Some(ref_id),
        });
    }
    let text = ai_argument_string(value).ok_or_else(|| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI generatorAudit string filters must be string literals or integer string refs",
            json!({ "argument": name }),
        )
    })?;
    Ok(AiStringRefFilter {
        value: Some(text),
        ref_id: None,
    })
}

fn ai_argument_u32(
    name: &str,
    value: &ResolvedAiArgumentValue,
    label: &str,
) -> Result<u32, BuildExecutionError> {
    let value = ai_argument_u64(value).ok_or_else(|| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            format!("CoveQL-AI generatorAudit {label} argument must be an integer literal"),
            json!({ "argument": name }),
        )
    })?;
    u32::try_from(value).map_err(|_| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            format!("CoveQL-AI generatorAudit {label} argument exceeds u32"),
            json!({ "argument": name, "value": value }),
        )
    })
}

fn ai_reproducibility_class_filter(
    name: &str,
    value: &ResolvedAiArgumentValue,
) -> Result<u8, BuildExecutionError> {
    if let Some(value) = ai_argument_u64(value) {
        return u8::try_from(value).map_err(|_| {
            exec_error(
                "E_AI_ARGUMENT_UNSUPPORTED",
                "CoveQL-AI generatorAudit reproducibilityClass exceeds u8",
                json!({ "argument": name, "value": value }),
            )
        });
    }
    let text = ai_argument_string(value).ok_or_else(|| {
        exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI generatorAudit reproducibilityClass must be an integer or known class name",
            json!({ "argument": name }),
        )
    })?;
    match text.as_str() {
        "descriptiveOnly" | "descriptive_only" => Ok(0),
        "sourceSnapshotReproducible" | "source_snapshot_reproducible" => Ok(1),
        "preprocessingReproducible" | "preprocessing_reproducible" => Ok(2),
        "storedPayloadVerifiable" | "stored_payload_verifiable" => Ok(3),
        "canonicalRecomputeReproducible" | "canonical_recompute_reproducible" => Ok(4),
        "externalAuditOnly" | "external_audit_only" => Ok(5),
        _ => Err(exec_error(
            "E_AI_ARGUMENT_UNSUPPORTED",
            "CoveQL-AI generatorAudit reproducibilityClass is not recognized",
            json!({ "argument": name, "value": text }),
        )),
    }
}

fn ai_argument_u64(value: &ResolvedAiArgumentValue) -> Option<u64> {
    let ResolvedAiArgumentValue::Expr(ResolvedExpr::Literal(literal)) = value else {
        return None;
    };
    match &literal.typed_value {
        ResolvedLiteralValue::UnsignedInteger(value) => Some(*value),
        ResolvedLiteralValue::SignedInteger(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn ai_argument_bool(value: &ResolvedAiArgumentValue) -> Option<bool> {
    let ResolvedAiArgumentValue::Expr(ResolvedExpr::Literal(literal)) = value else {
        return None;
    };
    match &literal.typed_value {
        ResolvedLiteralValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn ai_argument_string(value: &ResolvedAiArgumentValue) -> Option<String> {
    let ResolvedAiArgumentValue::Expr(ResolvedExpr::Literal(literal)) = value else {
        return None;
    };
    match &literal.typed_value {
        ResolvedLiteralValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn ai_semantic_search_rows(
    method_name: &str,
    search_plan: &AiVectorSearchPlan,
    results: &[AiVectorSearchResult],
    exact_semantic_authority: bool,
) -> Vec<Value> {
    results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            let exact = exact_semantic_authority && result.exact;
            let result_authority = if exact_semantic_authority {
                result.result_authority.as_str()
            } else {
                "RuntimeAdvisory"
            };
            json!({
                "method": method_name,
                "rank": index + 1,
                "query_file_code": search_plan.query_file_code,
                "query_vector_ref": search_plan.query_vector_ref,
                "target": search_plan.target_kind.as_str(),
                "requested_index": search_plan.index.as_str(),
                "target_kind": result.target_kind,
                "binding_id": result.binding_id,
                "file_code": result.file_code,
                "chunk_id": result.chunk_id,
                "object_type_id": result.object_type_id,
                "association_type_id": result.association_type_id,
                "sample_id": result.sample_id,
                "asset_ref": result.asset_ref,
                "multimodal_sequence_pack_id": result.multimodal_sequence_pack_id,
                "vector_ref": result.vector_ref,
                "vector_space_id": result.vector_space_id,
                "score": result.score,
                "exact": exact,
                "vector_exact": result.exact,
                "index_kind": result.selected_index,
                "fallback_used": result.fallback_used,
                "result_authority": result_authority,
                "advisory_reason": if exact_semantic_authority {
                    Value::Null
                } else {
                    json!("no_persisted_hybrid_or_rerank_authority")
                },
            })
        })
        .collect()
}
