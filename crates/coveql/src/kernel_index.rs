use cove_core::{wire, CoveError};
use cove_index::execution::{
    CoviAggregateKindV2, CoviIndexOnlyAnswerV2, CoviIndexOnlyRequestV2, CoviLookupTargetV2,
    CoviValidationContextV2, ValidatedCoviArtifactV2,
};
use serde_json::{json, Value};

use crate::{
    execution::{exec_error, exec_warning, result_fingerprint},
    expr_eval::parse_decimal_value,
    kernel_execution::{aggregate_output_name, execution_diagnostics_for_physical},
    pushdown, AggregateDisclosurePolicy, AstAggregateName, BuildExecutionError,
    CoveQlExecutionResult, ExecutedQuery, ExecutionAuthorityReport, ExecutionOptions,
    ExecutionRowCounts, MetadataDisclosurePolicy, PhysicalPlannedQuery, ResolvedExpr, ResolvedPath,
    ResolvedRoot, TemporalMode, VisibilityPolicy,
};

pub(super) fn try_index_only_executed_query(
    physical: &PhysicalPlannedQuery,
    options: &ExecutionOptions,
) -> Result<Option<ExecutedQuery>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if !physical.allow_index_only_answers
        || physical.index_capability_report.index_only_candidates == 0
        || !security.index_only_answer_permission
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || security.aggregate_disclosure_policy == AggregateDisclosurePolicy::AllowMaterializedOnly
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || options.visibility_overlay.is_some()
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || !matches!(planned.resolved.temporal.mode, TemporalMode::Latest)
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
    {
        return Ok(None);
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let [item] = select.as_slice() else {
        return Ok(None);
    };
    let ResolvedExpr::AggregateCall {
        name, arg, star, ..
    } = &item.expr
    else {
        return Ok(None);
    };
    let Some(path) = index_only_aggregate_path(*name, arg.as_deref(), *star) else {
        return Ok(None);
    };
    let (Some(object_type_id), Some(property_id)) = (path.object_type_id, path.property_id) else {
        return Ok(None);
    };
    if object_type_id != root.object_type_id {
        return Ok(None);
    }
    let aggregate_kind = covi_index_only_aggregate_kind(*name);
    let Some(answer) =
        index_only_answer_from_sidecars(physical, object_type_id, property_id, aggregate_kind)?
    else {
        return Ok(None);
    };
    let exact = index_only_json_value(*name, &path.logical_type, &answer)?;
    let disclosed = disclose_index_only_aggregate(exact, answer.row_count, planned)?;
    let mut object = serde_json::Map::new();
    object.insert(
        item.alias
            .clone()
            .unwrap_or_else(|| aggregate_output_name(*name).into()),
        disclosed,
    );
    let result = CoveQlExecutionResult::JsonRows(vec![Value::Object(object)]);
    let output_fingerprint = result_fingerprint(&result)?;
    let input_rows = usize::try_from(answer.row_count).map_err(|_| {
        exec_error(
            "E_RESOURCE_LIMIT",
            "index-only row count does not fit the execution row counter",
            json!({ "limit": "usize_row_count" }),
        )
    })?;
    let mut diagnostics = execution_diagnostics_for_physical(physical);
    diagnostics.push(exec_warning(
        "W_INDEX_ONLY_ANSWER_EXECUTED",
        "validated COVI/COVX index-only aggregate answer was used as execution output",
        json!({
            "aggregate": aggregate_output_name(*name),
            "object_type_id": object_type_id,
            "property_id": property_id,
        }),
    ));
    Ok(Some(ExecutedQuery {
        planned: planned.clone(),
        result,
        diagnostics,
        row_counts: ExecutionRowCounts {
            input_rows,
            filtered_rows: input_rows,
            output_rows: 1,
        },
        output_fingerprint,
        pushdown_report: pushdown::PushdownReport::not_executed(&options.pushdown),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_index_only(
            "validated exact COVI/COVX index-only aggregate answer produced the output",
        ),
    }))
}

fn index_only_aggregate_path(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    star: bool,
) -> Option<&ResolvedPath> {
    if star {
        return None;
    }
    match (name, arg) {
        (
            AstAggregateName::Count
            | AstAggregateName::Exists
            | AstAggregateName::DistinctCount
            | AstAggregateName::Min
            | AstAggregateName::Max,
            Some(ResolvedExpr::Path(path)),
        ) => Some(path),
        (AstAggregateName::Sum | AstAggregateName::Avg, Some(ResolvedExpr::Path(path)))
            if index_only_sum_avg_path_is_supported(path) =>
        {
            Some(path)
        }
        _ => None,
    }
}

fn index_only_sum_avg_path_is_supported(path: &ResolvedPath) -> bool {
    matches!(
        path.logical_type.as_str(),
        "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
    )
}

fn covi_index_only_aggregate_kind(name: AstAggregateName) -> CoviAggregateKindV2 {
    match name {
        AstAggregateName::Count => CoviAggregateKindV2::Count,
        AstAggregateName::Exists => CoviAggregateKindV2::Exists,
        AstAggregateName::DistinctCount => CoviAggregateKindV2::DistinctCount,
        AstAggregateName::Min => CoviAggregateKindV2::Min,
        AstAggregateName::Max => CoviAggregateKindV2::Max,
        AstAggregateName::Sum => CoviAggregateKindV2::Sum,
        AstAggregateName::Avg => CoviAggregateKindV2::Avg,
    }
}

fn index_only_answer_from_sidecars(
    physical: &PhysicalPlannedQuery,
    object_type_id: u32,
    property_id: u32,
    aggregate_kind: CoviAggregateKindV2,
) -> Result<Option<CoviIndexOnlyAnswerV2>, BuildExecutionError> {
    let request = CoviIndexOnlyRequestV2 {
        table_id: u32::MAX,
        column_id: None,
        aggregate_kind,
        predicate_form_ref: None,
        require_exact: true,
    };
    let target = CoviLookupTargetV2::ObjectProperty {
        object_type_id,
        property_id,
    };
    for bytes in [
        physical.sidecars.covi_artifact_bytes.as_deref(),
        physical.sidecars.covx_artifact_bytes.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let context = CoviValidationContextV2::for_file(
            physical.planned.resolved.operation_context.file.file_id,
            physical.planned.resolved.operation_context.file.file_len,
            physical
                .planned
                .resolved
                .operation_context
                .file
                .footer_crc32c,
        )
        .with_file_code_keys(physical.allow_file_code_literal_candidates);
        let Ok(artifact) = ValidatedCoviArtifactV2::parse_and_validate(bytes, context) else {
            continue;
        };
        match artifact.index_only_answer_for_target(target, &request) {
            Ok(Some(answer)) => return Ok(Some(answer)),
            Ok(None) => {}
            Err(CoveError::IndexOnlyUnsafe) => {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "index-only metadata matched the target but did not prove an exact safe answer",
                    json!({
                        "object_type_id": object_type_id,
                        "property_id": property_id,
                        "aggregate": format!("{aggregate_kind:?}"),
                    }),
                ))
            }
            Err(_) => {}
        }
    }
    Ok(None)
}

fn index_only_json_value(
    name: AstAggregateName,
    logical_type: &str,
    answer: &CoviIndexOnlyAnswerV2,
) -> Result<Value, BuildExecutionError> {
    Ok(match name {
        AstAggregateName::Count => json!(answer.non_null_count),
        AstAggregateName::Exists => Value::Bool(answer.non_null_count > 0),
        AstAggregateName::DistinctCount => {
            let Some(value) = &answer.value else {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "distinctCount index-only answer did not include an exact value payload",
                    json!({}),
                ));
            };
            let bytes: [u8; 8] = value.as_slice().try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "distinctCount index-only value payload was not a u64",
                    json!({ "payload_len": value.len() }),
                )
            })?;
            json!(u64::from_le_bytes(bytes))
        }
        AstAggregateName::Min | AstAggregateName::Max => match &answer.value {
            Some(value) => decode_index_only_min_max_value(logical_type, value)?,
            None => Value::Null,
        },
        AstAggregateName::Sum | AstAggregateName::Avg => {
            index_only_sum_avg_json_value(name, logical_type, answer)?
        }
    })
}

fn index_only_sum_avg_json_value(
    name: AstAggregateName,
    logical_type: &str,
    answer: &CoviIndexOnlyAnswerV2,
) -> Result<Value, BuildExecutionError> {
    if answer.non_null_count == 0 {
        return Ok(Value::Null);
    }
    let Some(value) = &answer.value else {
        return Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "sum/avg index-only answer did not include an exact sum payload",
            json!({ "aggregate": aggregate_output_name(name), "logical_type": logical_type }),
        ));
    };
    let sum = decode_index_only_numeric_sum_value(logical_type, value)?;
    match name {
        AstAggregateName::Sum => Ok(sum),
        AstAggregateName::Avg => {
            if matches!(logical_type, "float32" | "float64") {
                let Some(sum) = sum.as_f64() else {
                    return Err(exec_error(
                        "E_INDEX_ONLY_UNSAFE",
                        "float avg index-only sum payload was not a JSON number",
                        json!({ "logical_type": logical_type }),
                    ));
                };
                Ok(json!(sum / answer.non_null_count as f64))
            } else {
                Ok(parse_decimal_value(&sum)
                    .ok_or_else(|| {
                        exec_error(
                            "E_INDEX_ONLY_UNSAFE",
                            "exact avg index-only sum payload was not a decimal value",
                            json!({ "logical_type": logical_type }),
                        )
                    })?
                    .checked_div_u64(answer.non_null_count)
                    .ok_or_else(|| {
                        exec_error(
                            "E_INDEX_ONLY_UNSAFE",
                            "avg index-only sum payload overflowed decimal division",
                            json!({ "logical_type": logical_type }),
                        )
                    })?
                    .to_json_sum())
            }
        }
        _ => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "index-only numeric aggregate received a nonnumeric aggregate",
            json!({ "aggregate": aggregate_output_name(name) }),
        )),
    }
}

fn decode_index_only_numeric_sum_value(
    logical_type: &str,
    payload: &[u8],
) -> Result<Value, BuildExecutionError> {
    match logical_type {
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" => {
            decode_index_only_min_max_value(logical_type, payload)
        }
        _ => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "logical type is not supported for CoveQL index-only sum/avg output",
            json!({ "logical_type": logical_type }),
        )),
    }
}

fn decode_index_only_min_max_value(
    logical_type: &str,
    payload: &[u8],
) -> Result<Value, BuildExecutionError> {
    match logical_type {
        "int8" | "int16" | "int32" | "int64" | "decimal64" | "timestamp_micros"
        | "timestamp_nanos" => {
            let bytes: [u8; 8] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "signed index-only min/max payload was not 8 bytes",
                    json!({ "logical_type": logical_type, "payload_len": payload.len() }),
                )
            })?;
            Ok(json!(i64::from_le_bytes(bytes)))
        }
        "uint8" | "uint16" | "uint32" | "uint64" => {
            let bytes: [u8; 8] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "unsigned index-only min/max payload was not 8 bytes",
                    json!({ "logical_type": logical_type, "payload_len": payload.len() }),
                )
            })?;
            Ok(json!(u64::from_le_bytes(bytes)))
        }
        "float32" => {
            let bytes: [u8; 4] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "float32 index-only min/max payload was not 4 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            let value = f32::from_bits(u32::from_le_bytes(bytes));
            if !value.is_finite() {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "non-finite float32 index-only min/max payload cannot be represented as JSON",
                    json!({}),
                ));
            }
            Ok(json!(value))
        }
        "float64" => {
            let bytes: [u8; 8] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "float64 index-only min/max payload was not 8 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            let value = f64::from_bits(u64::from_le_bytes(bytes));
            if !value.is_finite() {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "non-finite float64 index-only min/max payload cannot be represented as JSON",
                    json!({}),
                ));
            }
            Ok(json!(value))
        }
        "date_days" => {
            let bytes: [u8; 4] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "date_days index-only min/max payload was not 4 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            Ok(json!(i32::from_le_bytes(bytes)))
        }
        "utf8" | "json" => {
            let (len, consumed) = wire::decode_u64_leb128(payload).map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload had an invalid length prefix",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            let len = usize::try_from(len).map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload length overflowed",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            let end = consumed.checked_add(len).ok_or_else(|| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload length overflowed",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            if end != payload.len() {
                return Err(exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload length did not match payload",
                    json!({ "logical_type": logical_type }),
                ));
            }
            let text = std::str::from_utf8(&payload[consumed..end]).map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "variable-width index-only min/max payload was not valid UTF-8",
                    json!({ "logical_type": logical_type }),
                )
            })?;
            if logical_type == "json" {
                serde_json::from_str(text).map_err(|_| {
                    exec_error(
                        "E_INDEX_ONLY_UNSAFE",
                        "json index-only min/max payload was not valid JSON",
                        json!({}),
                    )
                })
            } else {
                Ok(Value::String(text.into()))
            }
        }
        "uuid" => {
            let bytes: [u8; 16] = payload.try_into().map_err(|_| {
                exec_error(
                    "E_INDEX_ONLY_UNSAFE",
                    "uuid index-only min/max payload was not 16 bytes",
                    json!({ "payload_len": payload.len() }),
                )
            })?;
            Ok(Value::String(crate::materialized::hex(&bytes)))
        }
        "binary" | "decimal128" | "bool" | "null" | "list" | "struct" | "map" => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "logical type is not supported for CoveQL index-only min/max output",
            json!({ "logical_type": logical_type }),
        )),
        _ => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "unknown logical type for CoveQL index-only min/max output",
            json!({ "logical_type": logical_type }),
        )),
    }
}

fn disclose_index_only_aggregate(
    exact: Value,
    row_count: u64,
    planned: &crate::PlannedQuery,
) -> Result<Value, BuildExecutionError> {
    match planned
        .resolved
        .operation_context
        .security
        .aggregate_disclosure_policy
    {
        AggregateDisclosurePolicy::AllowExact => Ok(exact),
        AggregateDisclosurePolicy::AllowMaterializedOnly => Err(exec_error(
            "E_INDEX_ONLY_UNSAFE",
            "index-only answers are forbidden by aggregate materialized-only disclosure policy",
            json!({}),
        )),
        AggregateDisclosurePolicy::AllowThresholded => {
            let threshold = planned
                .resolved
                .operation_context
                .security
                .aggregate_disclosure_threshold
                .unwrap_or(1);
            if row_count >= threshold {
                Ok(exact)
            } else {
                Ok(aggregate_policy_marker(
                    "thresholded",
                    "suppressed",
                    Some(threshold),
                ))
            }
        }
        AggregateDisclosurePolicy::AllowRedacted => {
            Ok(aggregate_policy_marker("redacted", "redacted", None))
        }
        AggregateDisclosurePolicy::Reject => Err(exec_error(
            "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
            "aggregate disclosure is rejected by the active security context",
            json!({}),
        )),
    }
}

fn aggregate_policy_marker(policy: &str, status: &str, threshold: Option<u64>) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("policy".into(), Value::String(policy.into()));
    object.insert("status".into(), Value::String(status.into()));
    if let Some(threshold) = threshold {
        object.insert("threshold".into(), json!(threshold));
    }
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_only_min_max_decoder_handles_exact_scalar_payloads() {
        assert_eq!(
            decode_index_only_min_max_value("int64", &42i64.to_le_bytes()).unwrap(),
            json!(42)
        );
        assert_eq!(
            decode_index_only_min_max_value("uint64", &42u64.to_le_bytes()).unwrap(),
            json!(42)
        );
        assert_eq!(
            decode_index_only_min_max_value("date_days", &12i32.to_le_bytes()).unwrap(),
            json!(12)
        );

        let mut utf8 = Vec::new();
        wire::append_u64_leb128(&mut utf8, 3);
        utf8.extend_from_slice(b"abc");
        assert_eq!(
            decode_index_only_min_max_value("utf8", &utf8).unwrap(),
            json!("abc")
        );
    }

    #[test]
    fn index_only_min_max_decoder_rejects_unsupported_payloads() {
        assert!(decode_index_only_min_max_value("int64", &[1, 2, 3]).is_err());
        assert!(decode_index_only_min_max_value("binary", &[1, 2, 3]).is_err());

        let mut bad_utf8 = Vec::new();
        wire::append_u64_leb128(&mut bad_utf8, 2);
        bad_utf8.extend_from_slice(&[0xff, 0xff]);
        assert!(decode_index_only_min_max_value("utf8", &bad_utf8).is_err());
    }
}
