use std::{collections::BTreeSet, time::Instant};

use cove_arrow::arrow::{
    arrow_buffer_owner, encoded_columns_to_record_batch_with_owners_options, ArrowDictionaryPolicy,
    ArrowEncodedColumn, ArrowExportOptions, ArrowRowSelection,
};
use cove_core::{
    array::EncodedArray,
    constants::{CoveEncodingKind, CovePhysicalKind},
    dictionary::FileDictionary,
    mount::{mount_cove_file, MountOptions, OutputRepresentation},
    page_payload::PageBufferKind,
    profile::cove_o::{
        read_retained_object_temporal_segments, RetainedTemporalPropertyPage,
        RetainedTemporalSegmentData,
    },
    reader::{OptionalPushdownPolicy, ValidationOptions},
    validity::ValidityBitmap,
    CoveError,
};
use cove_layout::{
    ObjectZeroCopyColumnAuthority, ObjectZeroCopyPageAuthority, ObjectZeroCopySegmentAuthority,
    ValidatedZeroCopyBufferMapV2, ZeroCopyBufferMapV2, ZeroCopyCompatibilityContext,
    ZeroCopyDictionarySemanticsV2, ZeroCopyLifetimeScopeV2, ZeroCopyNestedLayoutKindV2,
};
use serde_json::json;

use crate::{
    execution::{check_time, enforce_result_budgets, exec_error, exec_warning, result_fingerprint},
    BuildExecutionError, CoveQlExecutionResult, CoveQlOutputMode, CoveQlRetainedInput,
    ExecutedQuery, ExecutionAuthorityReport, ExecutionDiagnostic, ExecutionOptions,
    ExecutionRowCounts, FallbackPolicy, MetadataDisclosurePolicy, PhysicalPlannedQuery,
    ResolvedExpr, ResolvedRoot, TemporalMode, VisibilityPolicy,
};

pub(crate) fn try_execute_retained_zero_copy_arrow(
    input: &CoveQlRetainedInput,
    physical: &PhysicalPlannedQuery,
    options: &ExecutionOptions,
) -> Result<Option<ExecutedQuery>, BuildExecutionError> {
    let planned = &physical.planned;
    if !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true
        }
    ) {
        return Ok(None);
    }
    let started = Instant::now();
    if let Some(reason) = zero_copy_blocking_reason(physical, options) {
        return zero_copy_fallback_or_reject(planned, reason);
    }

    let Some(map_bytes) = physical.sidecars.zero_copy_buffer_map_bytes.as_deref() else {
        return zero_copy_fallback_or_reject(
            planned,
            "COVE-L zero-copy buffer map was not supplied".into(),
        );
    };
    let map = match ZeroCopyBufferMapV2::parse(map_bytes) {
        Ok(map) => map,
        Err(error) => {
            return zero_copy_fallback_or_reject(
                planned,
                format!("COVE-L zero-copy buffer map parse failed: {error}"),
            )
        }
    };
    let retained = match read_retained_object_temporal_segments(
        input.retained_bytes(),
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            optional_pushdown_policy: OptionalPushdownPolicy::Strict,
        },
    ) {
        Ok(retained) => retained,
        Err(error) => {
            return zero_copy_fallback_or_reject(
                planned,
                format!("retained COVE-O zero-copy read failed: {error}"),
            )
        }
    };
    let expected_dictionary_semantics =
        match zero_copy_dictionary_semantics_for_projection(&retained.segments, physical) {
            Ok(semantics) => semantics,
            Err(reason) => return zero_copy_fallback_or_reject(planned, reason),
        };
    let file_dictionary =
        if expected_dictionary_semantics == ZeroCopyDictionarySemanticsV2::FileCodeDictionary {
            match zero_copy_file_dictionary(input.as_slice()) {
                Ok(dictionary) => Some(dictionary),
                Err(reason) => return zero_copy_fallback_or_reject(planned, reason),
            }
        } else {
            None
        };
    let authority = object_authority(&retained.segments);
    let compatibility = ZeroCopyCompatibilityContext {
        active_visibility_overlay: false,
        accepts_cove_null_bitmap_polarity: true,
        expected_dictionary_semantics,
        expected_nested_layout_kind: ZeroCopyNestedLayoutKindV2::NotNested,
        required_lifetime_scope: ZeroCopyLifetimeScopeV2::ReaderSession,
    };
    if let Err(error) = ValidatedZeroCopyBufferMapV2::validate_object_temporal(
        map,
        &retained.catalog,
        &authority,
        &compatibility,
    ) {
        return zero_copy_fallback_or_reject(
            planned,
            format!("COVE-L zero-copy map did not validate against COVE-O pages: {error}"),
        );
    }

    let batch = match retained_object_projection_batch(
        &retained.segments,
        physical,
        options,
        started,
        file_dictionary.as_ref(),
    ) {
        Ok(batch) => batch,
        Err(error) => {
            if planned.resolved.operation_context.request.fallback_policy
                == FallbackPolicy::RejectOnFallback
            {
                return Err(error);
            }
            return Ok(None);
        }
    };
    let row_count = batch.num_rows();
    let result = CoveQlExecutionResult::ArrowRecordBatches(vec![batch]);
    let row_counts = ExecutionRowCounts {
        input_rows: row_count,
        filtered_rows: row_count,
        output_rows: row_count,
    };
    enforce_result_budgets(&result, &row_counts, planned, options, started)?;
    let output_fingerprint = result_fingerprint(&result)?;
    let mut diagnostics = planned
        .diagnostics
        .iter()
        .cloned()
        .map(ExecutionDiagnostic::from)
        .collect::<Vec<_>>();
    diagnostics.push(exec_warning(
        "W_ZERO_COPY_ARROW_EXECUTED",
        "zero-copy Arrow output used retained COVE-L object page buffers",
        json!({ "execution": "retained_cove_l_object_pages" }),
    ));
    Ok(Some(ExecutedQuery {
        planned: planned.clone(),
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report: crate::pushdown::PushdownReport::not_executed(&options.pushdown),
        evidence_authority: None,
        authority: ExecutionAuthorityReport::exact_zero_copy(
            "validated COVE-L zero-copy map produced retained Arrow output",
        ),
    }))
}

fn zero_copy_blocking_reason(
    physical: &PhysicalPlannedQuery,
    options: &ExecutionOptions,
) -> Option<String> {
    let planned = &physical.planned;
    if !physical.allow_zero_copy_output {
        return Some("physical plan does not allow zero-copy output".into());
    }
    if !physical.zero_copy_eligibility.compatible {
        return Some(physical.zero_copy_eligibility.reason.clone());
    }
    let security = &planned.resolved.operation_context.security;
    if !security.zero_copy_permission {
        return Some("operation context does not grant zero-copy permission".into());
    }
    if security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected {
        return Some(
            "metadata disclosure policy does not allow protected zero-copy metadata".into(),
        );
    }
    if matches!(
        security.visibility_policy,
        VisibilityPolicy::ExternalOverlay(_)
    ) || options.visibility_overlay.is_some()
    {
        return Some("active visibility overlay requires materialized visibility filtering".into());
    }
    if !matches!(planned.resolved.root, ResolvedRoot::Object(_)) {
        return Some("zero-copy v1 supports object roots only".into());
    }
    if !matches!(planned.resolved.temporal.mode, TemporalMode::Latest)
        || planned.resolved.temporal.role_binding.is_some()
    {
        return Some("zero-copy v1 supports only latest commit-time object rows".into());
    }
    if planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
    {
        return Some("zero-copy v1 requires a direct projection without filters, sort, paging, aggregates, history, or changes".into());
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return Some("zero-copy v1 requires an explicit direct property select".into());
    };
    if select.is_empty() {
        return Some("zero-copy v1 requires at least one projected property".into());
    }
    if select
        .iter()
        .any(|item| !matches!(item.expr, ResolvedExpr::Path(_)))
    {
        return Some("zero-copy v1 supports direct path projections only".into());
    }
    None
}

fn retained_object_projection_batch(
    segments: &[RetainedTemporalSegmentData],
    physical: &PhysicalPlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    file_dictionary: Option<&FileDictionary>,
) -> Result<arrow_array::RecordBatch, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let planned = &physical.planned;
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy object projection requires an object root",
            json!({}),
        ));
    };
    let segment = single_matching_segment(segments, root.object_type_id)?;
    validate_segment_rows_are_direct_latest(segment)?;
    let select = planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .ok_or_else(|| {
            exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "zero-copy object projection requires select items",
                json!({}),
            )
        })?;

    let mut names = Vec::with_capacity(select.len());
    let mut arrays = Vec::with_capacity(select.len());
    let mut owners = Vec::with_capacity(select.len());
    for item in select {
        let ResolvedExpr::Path(path) = &item.expr else {
            return Err(exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "zero-copy object projection supports direct path items only",
                json!({}),
            ));
        };
        let Some(property_id) = path.property_id else {
            return Err(exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "zero-copy v1 does not support system fields",
                json!({ "field": path.display_name }),
            ));
        };
        let column = segment
            .property_columns
            .iter()
            .find(|column| column.directory.column_id == property_id)
            .ok_or_else(|| {
                exec_error(
                    "E_ZERO_COPY_UNSUPPORTED",
                    "selected property has no retained temporal property column",
                    json!({ "property_id": property_id }),
                )
            })?;
        let page = single_payload_page(&column.pages, property_id)?;
        let payload = page.payload.as_ref().ok_or_else(|| {
            exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "selected property page has no retained payload",
                json!({ "property_id": property_id }),
            )
        })?;
        let root_node = payload.root_node().map_err(|error| {
            exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                format!("retained property page root validation failed: {error}"),
                json!({ "property_id": property_id }),
            )
        })?;
        if !zero_copy_supported_page(root_node.encoding_kind, root_node.physical_kind) {
            return Err(exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "zero-copy v1 supports retained NumCode, boolean, fixed-byte, and FileCode property pages only",
                json!({ "property_id": property_id }),
            ));
        }
        let validity = payload
            .buffer_bytes(PageBufferKind::NullBitmap)
            .map_err(|error| zero_copy_page_error(property_id, error))?
            .map(|bytes| ValidityBitmap::new(bytes, u64::from(page.index_entry.row_count)));
        let values = payload
            .buffer_bytes(PageBufferKind::Values)
            .map_err(|error| zero_copy_page_error(property_id, error))?
            .ok_or_else(|| {
                exec_error(
                    "E_ZERO_COPY_UNSUPPORTED",
                    "selected property page has no values buffer",
                    json!({ "property_id": property_id }),
                )
            })?;
        if root_node.physical_kind == CovePhysicalKind::NumCode
            && !(values.as_ptr() as usize).is_multiple_of(std::mem::align_of::<u64>())
        {
            return Err(exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "retained NumCode values buffer is not aligned for direct Arrow export",
                json!({ "property_id": property_id }),
            ));
        }
        let array = EncodedArray::new(
            root_node.logical_type,
            root_node.physical_kind,
            u64::from(page.index_entry.row_count),
            root_node.encoding_kind,
            validity,
            values,
            if root_node.physical_kind == CovePhysicalKind::FileCode {
                Some(file_dictionary.ok_or_else(|| {
                    exec_error(
                        "E_ZERO_COPY_UNSUPPORTED",
                        "retained FileCode zero-copy Arrow output requires a mounted file dictionary",
                        json!({ "property_id": property_id }),
                    )
                })?)
            } else {
                None
            },
        );
        names.push(
            item.alias
                .clone()
                .unwrap_or_else(|| path.display_name.clone()),
        );
        owners.push(arrow_buffer_owner(payload.data.owner()));
        arrays.push(array);
    }
    let columns = arrays
        .iter()
        .enumerate()
        .map(|(index, array)| {
            ArrowEncodedColumn::with_data_owner(
                names[index].as_str(),
                array,
                Some(owners[index].clone()),
            )
        })
        .collect::<Vec<_>>();
    let arrow_options = if file_dictionary.is_some() {
        ArrowExportOptions {
            dictionary_policy: ArrowDictionaryPolicy::DictionaryKeys,
            ..ArrowExportOptions::default()
        }
    } else {
        ArrowExportOptions::default()
    };
    let result = encoded_columns_to_record_batch_with_owners_options(
        &columns,
        ArrowRowSelection::All,
        arrow_options,
    )
    .map_err(|error| {
        exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            format!("owner-backed Arrow export failed: {error}"),
            json!({}),
        )
    })?;
    if result.report.has_lossy_or_unsupported() {
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "owner-backed Arrow export reported lossy or unsupported fidelity",
            json!({ "issues": result.report.issues.len() }),
        ));
    }
    Ok(result.value)
}

fn zero_copy_dictionary_semantics_for_projection(
    segments: &[RetainedTemporalSegmentData],
    physical: &PhysicalPlannedQuery,
) -> Result<ZeroCopyDictionarySemanticsV2, String> {
    let planned = &physical.planned;
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Err("zero-copy object projection requires an object root".into());
    };
    let segment = single_matching_segment_for_reason(segments, root.object_type_id)?;
    let select = planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .ok_or_else(|| "zero-copy v1 requires an explicit direct property select".to_string())?;
    let mut selected_semantics = None;
    for item in select {
        let ResolvedExpr::Path(path) = &item.expr else {
            return Err("zero-copy object projection supports direct path items only".into());
        };
        let property_id = path
            .property_id
            .ok_or_else(|| "zero-copy v1 does not support system fields".to_string())?;
        let column = segment
            .property_columns
            .iter()
            .find(|column| column.directory.column_id == property_id)
            .ok_or_else(|| {
                format!("selected property has no retained temporal property column: {property_id}")
            })?;
        let semantics = if column.directory.physical_kind == CovePhysicalKind::FileCode {
            ZeroCopyDictionarySemanticsV2::FileCodeDictionary
        } else {
            ZeroCopyDictionarySemanticsV2::NoDictionary
        };
        if selected_semantics.is_some_and(|selected| selected != semantics) {
            return Err("zero-copy v1 does not support mixed FileCode dictionary and non-dictionary projection columns".into());
        }
        selected_semantics = Some(semantics);
    }
    Ok(selected_semantics.unwrap_or(ZeroCopyDictionarySemanticsV2::NoDictionary))
}

fn zero_copy_file_dictionary(bytes: &[u8]) -> Result<FileDictionary, String> {
    let mounted = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            covx: None,
            covm: None,
        },
        None,
    )
    .map_err(|error| format!("retained FileCode zero-copy dictionary mount failed: {error}"))?;
    mounted.dictionary.ok_or_else(|| {
        "retained FileCode zero-copy Arrow output requires a file dictionary".to_string()
    })
}

fn zero_copy_supported_page(
    encoding_kind: CoveEncodingKind,
    physical_kind: CovePhysicalKind,
) -> bool {
    matches!(
        (encoding_kind, physical_kind),
        (CoveEncodingKind::NumCode, CovePhysicalKind::NumCode)
            | (CoveEncodingKind::PlainFixed, CovePhysicalKind::Boolean)
            | (CoveEncodingKind::PlainFixed, CovePhysicalKind::FixedBytes)
            | (CoveEncodingKind::FileCode, CovePhysicalKind::FileCode)
    )
}

fn single_matching_segment_for_reason(
    segments: &[RetainedTemporalSegmentData],
    object_type_id: u32,
) -> Result<&RetainedTemporalSegmentData, String> {
    let mut matches = segments
        .iter()
        .filter(|segment| segment.header.object_type_id == object_type_id);
    let Some(segment) = matches.next() else {
        return Err(format!(
            "zero-copy object projection found no retained segment for root object type: {object_type_id}"
        ));
    };
    if matches.next().is_some() {
        return Err(format!(
            "zero-copy v1 supports exactly one retained segment for the root object type: {object_type_id}"
        ));
    }
    Ok(segment)
}

fn single_matching_segment(
    segments: &[RetainedTemporalSegmentData],
    object_type_id: u32,
) -> Result<&RetainedTemporalSegmentData, BuildExecutionError> {
    let mut matches = segments
        .iter()
        .filter(|segment| segment.header.object_type_id == object_type_id);
    let Some(segment) = matches.next() else {
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy object projection found no retained segment for root object type",
            json!({ "object_type_id": object_type_id }),
        ));
    };
    if matches.next().is_some() {
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy v1 supports exactly one retained segment for the root object type",
            json!({ "object_type_id": object_type_id }),
        ));
    }
    Ok(segment)
}

fn validate_segment_rows_are_direct_latest(
    segment: &RetainedTemporalSegmentData,
) -> Result<(), BuildExecutionError> {
    let mut seen = BTreeSet::new();
    for row in &segment.rows {
        if !matches!(
            row.record_kind,
            cove_core::profile::cove_o::RecordKind::Baseline
                | cove_core::profile::cove_o::RecordKind::Snapshot
        ) || row.prev_ref.is_some()
        {
            return Err(exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "zero-copy v1 requires baseline/snapshot rows without reconstruction chains",
                json!({ "segment_id": segment.header.segment_id }),
            ));
        }
        if !seen.insert((row.branch_key, row.goid)) {
            return Err(exec_error(
                "E_ZERO_COPY_UNSUPPORTED",
                "zero-copy v1 requires one visible latest row per branch/object key",
                json!({ "segment_id": segment.header.segment_id }),
            ));
        }
    }
    Ok(())
}

fn single_payload_page(
    pages: &[cove_core::profile::cove_o::RetainedTemporalPropertyPage],
    property_id: u32,
) -> Result<&RetainedTemporalPropertyPage, BuildExecutionError> {
    let mut payload_pages = pages.iter().filter(|page| page.payload.is_some());
    let Some(page) = payload_pages.next() else {
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy selected property has no payload page",
            json!({ "property_id": property_id }),
        ));
    };
    if payload_pages.next().is_some() {
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy v1 supports exactly one payload page per selected property",
            json!({ "property_id": property_id }),
        ));
    }
    Ok(page)
}

fn object_authority(
    segments: &[RetainedTemporalSegmentData],
) -> Vec<ObjectZeroCopySegmentAuthority> {
    segments
        .iter()
        .map(|segment| ObjectZeroCopySegmentAuthority {
            object_type_id: segment.header.object_type_id,
            segment_id: segment.header.segment_id,
            morsel_count: segment.header.morsel_count,
            columns: segment
                .property_columns
                .iter()
                .map(|column| ObjectZeroCopyColumnAuthority {
                    property_id: column.directory.column_id,
                    logical_type: column.directory.logical_type,
                    physical_kind: column.directory.physical_kind,
                    pages: column
                        .pages
                        .iter()
                        .enumerate()
                        .map(|(index, page)| ObjectZeroCopyPageAuthority {
                            page_ref: u32::try_from(index + 1).unwrap_or(u32::MAX),
                            morsel_id: page.index_entry.morsel_id,
                            row_count: page.index_entry.row_count,
                            flags: page.index_entry.flags,
                            has_payload: page.payload.is_some(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

fn zero_copy_fallback_or_reject(
    planned: &crate::PlannedQuery,
    reason: String,
) -> Result<Option<ExecutedQuery>, BuildExecutionError> {
    if planned.resolved.operation_context.request.fallback_policy
        == FallbackPolicy::RejectOnFallback
    {
        Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy Arrow output could not be proven for this query",
            json!({ "reason": reason }),
        ))
    } else {
        Ok(None)
    }
}

fn zero_copy_page_error(property_id: u32, error: CoveError) -> BuildExecutionError {
    exec_error(
        "E_ZERO_COPY_UNSUPPORTED",
        format!("retained zero-copy property page read failed: {error}"),
        json!({ "property_id": property_id }),
    )
}
