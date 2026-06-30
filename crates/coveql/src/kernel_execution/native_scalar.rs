use super::*;

pub(super) fn try_native_scalar_predicate_prune(
    bytes: &[u8],
    retained_input: Option<&CoveQlRetainedInput>,
    physical: &PhysicalPlannedQuery,
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
    predicates: &[KernelPredicate],
) -> Result<Option<NativeScalarPredicatePrune>, BuildExecutionError> {
    let planned = &physical.planned;
    let security = &planned.resolved.operation_context.security;
    if planned.resolved.operation_context.dataset.files.len() > 1
        || security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        )
        || planned
            .resolved
            .operation_context
            .tombstone
            .include_tombstones
        || planned.resolved.temporal.role_binding.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || matches!(
            planned.resolved.output_mode,
            crate::CoveQlOutputMode::DataFusionTableProvider | crate::CoveQlOutputMode::ExplainJson
        )
    {
        return Ok(None);
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return Ok(None);
    };
    if root.object_type_id != root_object_type_id {
        return Ok(None);
    }

    let mut requests = predicates
        .iter()
        .filter_map(|predicate| native_scalar_request_for_predicate(predicate, root_object_type_id))
        .collect::<Vec<_>>();
    if requests.is_empty() {
        return Ok(None);
    }
    order_native_scalar_requests_by_cost(&mut requests);

    let loaded_rows = surface_row_lookup_for_object(surface, root_object_type_id);

    if let Some(input) = retained_input {
        match read_retained_object_temporal_segments(
            input.retained_bytes(),
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        ) {
            Ok(retained) => {
                return native_scalar_prune_from_retained_segments(
                    &retained.segments,
                    surface,
                    &loaded_rows,
                    &requests,
                );
            }
            Err(CoveError::UnsupportedEncoding(_)) => {}
            Err(_) => {}
        }
    }

    let segments = temporal_segments_from_bytes_with_context(
        bytes,
        "E_NATIVE_SCALAR_LITERAL_PRUNE",
        "native scalar literal prune",
    )?;
    native_scalar_prune_from_segments(&segments, surface, &loaded_rows, &requests)
}

pub(super) fn native_scalar_request_for_predicate(
    predicate: &KernelPredicate,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    match predicate {
        KernelPredicate::BoolPath { path } => {
            native_bool_eq_request(path, true, root_object_type_id)
        }
        KernelPredicate::Not(inner) => match inner.as_ref() {
            KernelPredicate::BoolPath { path } => {
                native_bool_eq_request(path, false, root_object_type_id)
            }
            KernelPredicate::InList { path, literals } => {
                native_not_in_list_request(path, literals, root_object_type_id)
            }
            KernelPredicate::ComparePathLiteral { path, op, literal } => {
                native_negated_compare_request(path, *op, literal, root_object_type_id)
            }
            _ => None,
        },
        KernelPredicate::NullCheck { path, negated } => {
            native_null_check_request(path, *negated, root_object_type_id)
        }
        KernelPredicate::ComparePathLiteral { path, op, literal }
            if matches!(
                op,
                AstCompareOp::Eq
                    | AstCompareOp::Ne
                    | AstCompareOp::Lt
                    | AstCompareOp::Le
                    | AstCompareOp::Gt
                    | AstCompareOp::Ge
            ) =>
        {
            native_compare_request(path, *op, literal, root_object_type_id)
        }
        KernelPredicate::InList { path, literals } => {
            native_in_list_request(path, literals, root_object_type_id)
        }
        KernelPredicate::Or(parts) => native_or_request(parts, root_object_type_id),
        KernelPredicate::StartsWithPathLiteral { path, prefix } => {
            let (object_type_id, property_id, logical_type) =
                native_object_property_path(path, root_object_type_id)?;
            if !matches!(logical_type, CoveLogicalType::Utf8)
                || path.physical_kind.as_str() != "var_bytes"
            {
                return None;
            }
            Some(NativeScalarPredicateRequest::VarBytesPrefix {
                object_type_id,
                property_id,
                logical_type,
                prefix: prefix.as_bytes().to_vec(),
            })
        }
        _ => None,
    }
}

pub(super) fn native_or_request(
    parts: &[KernelPredicate],
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let mut path: Option<&ResolvedPath> = None;
    let mut literals = Vec::with_capacity(parts.len());
    for part in parts {
        let KernelPredicate::ComparePathLiteral {
            path: part_path,
            op,
            literal,
        } = part
        else {
            return None;
        };
        if *op != AstCompareOp::Eq {
            return None;
        }
        if path.is_some_and(|path| !same_native_path(path, part_path)) {
            return None;
        }
        path = Some(part_path);
        literals.push(literal.clone());
    }
    native_in_list_request(path?, &literals, root_object_type_id)
}

pub(super) fn native_negated_compare_request(
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let inverted = match op {
        AstCompareOp::Eq => return native_not_equal_request(path, literal, root_object_type_id),
        AstCompareOp::Ne => AstCompareOp::Eq,
        AstCompareOp::Lt => AstCompareOp::Ge,
        AstCompareOp::Le => AstCompareOp::Gt,
        AstCompareOp::Gt => AstCompareOp::Le,
        AstCompareOp::Ge => AstCompareOp::Lt,
    };
    native_compare_request(path, inverted, literal, root_object_type_id)
}

pub(super) fn native_in_list_request(
    path: &ResolvedPath,
    literals: &[KernelLiteral],
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    match path.physical_kind.as_str() {
        "boolean" if logical_type == CoveLogicalType::Bool => {
            let (has_true, has_false) = native_bool_literals_for_kernel(literals)?;
            match (has_true, has_false) {
                (true, true) => Some(NativeScalarPredicateRequest::NullCheck {
                    object_type_id,
                    property_id,
                    want_valid: true,
                }),
                (true, false) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: true,
                }),
                (false, true) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: false,
                }),
                (false, false) => None,
            }
        }
        "num_code" => {
            let literals = literals
                .iter()
                .map(native_numeric_literal_for_kernel)
                .collect::<Option<Vec<_>>>()?;
            (!literals.is_empty()).then_some(NativeScalarPredicateRequest::NumCodeIn {
                object_type_id,
                property_id,
                logical_type,
                literals,
            })
        }
        "fixed_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_fixed_bytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::FixedBytesIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        "var_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_varbytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::VarBytesIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        _ => None,
    }
}

pub(super) fn native_not_equal_request(
    path: &ResolvedPath,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    match (path.physical_kind.as_str(), logical_type, literal) {
        ("boolean", CoveLogicalType::Bool, KernelLiteral::Bool(value)) => {
            Some(NativeScalarPredicateRequest::BoolEq {
                object_type_id,
                property_id,
                value: !*value,
            })
        }
        ("num_code", _, _) => {
            let literal = native_numeric_literal_for_kernel(literal)?;
            Some(NativeScalarPredicateRequest::NumCodeNotIn {
                object_type_id,
                property_id,
                logical_type,
                literals: vec![literal],
            })
        }
        ("fixed_bytes", CoveLogicalType::Uuid, KernelLiteral::String(_)) => {
            let value = native_fixed_bytes_literal_for_kernel(logical_type, literal)?;
            Some(NativeScalarPredicateRequest::FixedBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values: value,
            })
        }
        ("var_bytes", CoveLogicalType::Utf8, KernelLiteral::String(_)) => {
            let value = native_varbytes_literal_for_kernel(logical_type, literal)?;
            Some(NativeScalarPredicateRequest::VarBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values: vec![value],
            })
        }
        _ => None,
    }
}

pub(super) fn native_not_in_list_request(
    path: &ResolvedPath,
    literals: &[KernelLiteral],
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    if literals
        .iter()
        .any(|literal| matches!(literal, KernelLiteral::Null))
    {
        return None;
    }
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    match path.physical_kind.as_str() {
        "boolean" if logical_type == CoveLogicalType::Bool => {
            let (has_true, has_false) = native_bool_literals_for_kernel(literals)?;
            match (has_true, has_false) {
                (true, false) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: false,
                }),
                (false, true) => Some(NativeScalarPredicateRequest::BoolEq {
                    object_type_id,
                    property_id,
                    value: true,
                }),
                _ => None,
            }
        }
        "num_code" => {
            let literals = literals
                .iter()
                .map(native_numeric_literal_for_kernel)
                .collect::<Option<Vec<_>>>()?;
            (!literals.is_empty()).then_some(NativeScalarPredicateRequest::NumCodeNotIn {
                object_type_id,
                property_id,
                logical_type,
                literals,
            })
        }
        "fixed_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_fixed_bytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::FixedBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        "var_bytes" => {
            let values = literals
                .iter()
                .map(|literal| native_varbytes_literal_for_kernel(logical_type, literal))
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty()).then_some(NativeScalarPredicateRequest::VarBytesNotIn {
                object_type_id,
                property_id,
                logical_type,
                values,
            })
        }
        _ => None,
    }
}

pub(super) fn same_native_path(left: &ResolvedPath, right: &ResolvedPath) -> bool {
    left.root_kind == right.root_kind
        && left.object_type_id == right.object_type_id
        && left.property_id == right.property_id
        && left.system_field == right.system_field
        && left.logical_type == right.logical_type
        && left.physical_kind == right.physical_kind
}

pub(super) fn native_bool_literals_for_kernel(literals: &[KernelLiteral]) -> Option<(bool, bool)> {
    let mut has_true = false;
    let mut has_false = false;
    for literal in literals {
        match literal {
            KernelLiteral::Bool(true) => has_true = true,
            KernelLiteral::Bool(false) => has_false = true,
            KernelLiteral::Null => {}
            _ => return None,
        }
    }
    Some((has_true, has_false))
}

pub(super) fn native_compare_request(
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    if op == AstCompareOp::Ne {
        return native_not_equal_request(path, literal, root_object_type_id);
    }
    if let Some(request) = native_system_compare_request(path, op, literal, root_object_type_id) {
        return Some(request);
    }
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    if path.physical_kind.as_str() == "num_code" {
        let op = native_numeric_op_for_ast(op)?;
        let literal = native_numeric_literal_for_kernel(literal)?;
        return Some(NativeScalarPredicateRequest::NumCode {
            object_type_id,
            property_id,
            logical_type,
            op,
            literal,
        });
    }
    if op != AstCompareOp::Eq {
        return None;
    }
    match (path.physical_kind.as_str(), logical_type, literal) {
        ("boolean", CoveLogicalType::Bool, KernelLiteral::Bool(value)) => {
            Some(NativeScalarPredicateRequest::BoolEq {
                object_type_id,
                property_id,
                value: *value,
            })
        }
        ("fixed_bytes", CoveLogicalType::Uuid, KernelLiteral::String(value)) => {
            let value = decode_compact_hex_bytes(value).filter(|value| value.len() == 16)?;
            Some(NativeScalarPredicateRequest::FixedBytesEq {
                object_type_id,
                property_id,
                logical_type,
                value,
            })
        }
        ("var_bytes", CoveLogicalType::Utf8, KernelLiteral::String(value)) => {
            Some(NativeScalarPredicateRequest::VarBytesEq {
                object_type_id,
                property_id,
                logical_type,
                value: value.as_bytes().to_vec(),
            })
        }
        _ => None,
    }
}

pub(super) fn native_system_compare_request(
    path: &ResolvedPath,
    op: AstCompareOp,
    literal: &KernelLiteral,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    if !matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
        || path.object_type_id != Some(root_object_type_id)
    {
        return None;
    }
    match path.system_field.as_ref()? {
        ResolvedSystemField::BranchKey => Some(NativeScalarPredicateRequest::SystemNumeric {
            object_type_id: root_object_type_id,
            field: NativeSystemNumericField::BranchKey,
            op: native_numeric_op_for_ast(op)?,
            literal: native_numeric_literal_for_kernel(literal)?,
        }),
        ResolvedSystemField::Csn => Some(NativeScalarPredicateRequest::SystemNumeric {
            object_type_id: root_object_type_id,
            field: NativeSystemNumericField::Csn,
            op: native_numeric_op_for_ast(op)?,
            literal: native_numeric_literal_for_kernel(literal)?,
        }),
        ResolvedSystemField::TimestampUs => Some(NativeScalarPredicateRequest::SystemNumeric {
            object_type_id: root_object_type_id,
            field: NativeSystemNumericField::TimestampUs,
            op: native_numeric_op_for_ast(op)?,
            literal: native_numeric_literal_for_kernel(literal)?,
        }),
        ResolvedSystemField::Goid if op == AstCompareOp::Eq => {
            let KernelLiteral::String(value) = literal else {
                return None;
            };
            let decoded = decode_compact_hex_bytes(value)?;
            if decoded.len() != 16 {
                return None;
            }
            let mut value = [0u8; 16];
            value.copy_from_slice(&decoded);
            Some(NativeScalarPredicateRequest::SystemGoidEq {
                object_type_id: root_object_type_id,
                value,
            })
        }
        _ => None,
    }
}

pub(super) fn native_numeric_op_for_ast(op: AstCompareOp) -> Option<NativeNumericPredicateOp> {
    match op {
        AstCompareOp::Eq => Some(NativeNumericPredicateOp::Eq),
        AstCompareOp::Lt => Some(NativeNumericPredicateOp::Lt),
        AstCompareOp::Le => Some(NativeNumericPredicateOp::LtEq),
        AstCompareOp::Gt => Some(NativeNumericPredicateOp::Gt),
        AstCompareOp::Ge => Some(NativeNumericPredicateOp::GtEq),
        AstCompareOp::Ne => None,
    }
}

pub(super) fn native_numeric_literal_for_kernel(
    literal: &KernelLiteral,
) -> Option<NativeNumericLiteral> {
    match literal {
        KernelLiteral::I64(value) => Some(NativeNumericLiteral::Int64(*value)),
        KernelLiteral::U64(value) => Some(NativeNumericLiteral::UInt64(*value)),
        KernelLiteral::F64(value) => Some(NativeNumericLiteral::Float64(*value)),
        _ => None,
    }
}

pub(super) fn native_fixed_bytes_literal_for_kernel(
    logical_type: CoveLogicalType,
    literal: &KernelLiteral,
) -> Option<Vec<u8>> {
    match (logical_type, literal) {
        (CoveLogicalType::Uuid, KernelLiteral::String(value)) => {
            decode_compact_hex_bytes(value).filter(|value| value.len() == 16)
        }
        _ => None,
    }
}

pub(super) fn native_varbytes_literal_for_kernel(
    logical_type: CoveLogicalType,
    literal: &KernelLiteral,
) -> Option<Vec<u8>> {
    match (logical_type, literal) {
        (CoveLogicalType::Utf8, KernelLiteral::String(value)) => Some(value.as_bytes().to_vec()),
        _ => None,
    }
}

pub(super) fn native_bool_eq_request(
    path: &ResolvedPath,
    value: bool,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, logical_type) =
        native_object_property_path(path, root_object_type_id)?;
    (path.physical_kind.as_str() == "boolean" && logical_type == CoveLogicalType::Bool).then_some(
        NativeScalarPredicateRequest::BoolEq {
            object_type_id,
            property_id,
            value,
        },
    )
}

pub(super) fn native_null_check_request(
    path: &ResolvedPath,
    want_valid: bool,
    root_object_type_id: u32,
) -> Option<NativeScalarPredicateRequest> {
    let (object_type_id, property_id, _) = native_object_property_path(path, root_object_type_id)?;
    Some(NativeScalarPredicateRequest::NullCheck {
        object_type_id,
        property_id,
        want_valid,
    })
}

pub(super) fn native_object_property_path(
    path: &ResolvedPath,
    root_object_type_id: u32,
) -> Option<(u32, u32, CoveLogicalType)> {
    if !matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
        || path.system_field.is_some()
        || path.object_type_id != Some(root_object_type_id)
    {
        return None;
    }
    let object_type_id = path.object_type_id?;
    let property_id = path.property_id?;
    let logical_type = logical_type_to_cove(path.logical_type.as_str())?;
    Some((object_type_id, property_id, logical_type))
}

pub(super) fn native_scalar_prune_from_segments(
    segments: &[TemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    requests: &[NativeScalarPredicateRequest],
) -> Result<Option<NativeScalarPredicatePrune>, BuildExecutionError> {
    let mut combined: Option<SelectionBitmap> = None;
    let mut bitmap_dispatch = NativeBitmapDispatchCounts::default();
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    let mut total_pages = 0usize;
    let mut executed_predicate_count = 0usize;
    let mut short_circuited = false;
    for request in requests {
        if combined.as_ref().is_some_and(SelectionBitmap::all_zero) {
            short_circuited = true;
            break;
        }
        let Some(scan) = native_scalar_bitmap_for_request(segments, surface, loaded_rows, request)?
        else {
            return Ok(None);
        };
        executed_predicate_count += 1;
        total_pages += scan.page_count;
        predicate_dispatch.merge(scan.predicate_dispatch);
        if let Some(existing) = &mut combined {
            let dispatch =
                existing.intersect_with_dispatch(&scan.bitmap, NativeKernelDispatch::Auto);
            bitmap_dispatch.record(dispatch);
        } else {
            combined = Some(scan.bitmap);
        }
    }
    Ok(combined.map(|bitmap| NativeScalarPredicatePrune {
        matched_rows: bitmap.count_ones(),
        bitmap,
        predicate_count: requests.len(),
        executed_predicate_count,
        page_count: total_pages,
        predicate_order: requests
            .iter()
            .map(NativeScalarPredicateRequest::kind_name)
            .collect(),
        predicate_dispatch,
        bitmap_dispatch,
        retained_page_buffers: false,
        short_circuited,
    }))
}

pub(super) fn native_scalar_prune_from_retained_segments(
    segments: &[RetainedTemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    requests: &[NativeScalarPredicateRequest],
) -> Result<Option<NativeScalarPredicatePrune>, BuildExecutionError> {
    let mut combined: Option<SelectionBitmap> = None;
    let mut bitmap_dispatch = NativeBitmapDispatchCounts::default();
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    let mut total_pages = 0usize;
    let mut executed_predicate_count = 0usize;
    let mut short_circuited = false;
    for request in requests {
        if combined.as_ref().is_some_and(SelectionBitmap::all_zero) {
            short_circuited = true;
            break;
        }
        let Some(scan) =
            native_scalar_bitmap_for_retained_request(segments, surface, loaded_rows, request)?
        else {
            return Ok(None);
        };
        executed_predicate_count += 1;
        total_pages += scan.page_count;
        predicate_dispatch.merge(scan.predicate_dispatch);
        if let Some(existing) = &mut combined {
            let dispatch =
                existing.intersect_with_dispatch(&scan.bitmap, NativeKernelDispatch::Auto);
            bitmap_dispatch.record(dispatch);
        } else {
            combined = Some(scan.bitmap);
        }
    }
    Ok(combined.map(|bitmap| NativeScalarPredicatePrune {
        matched_rows: bitmap.count_ones(),
        bitmap,
        predicate_count: requests.len(),
        executed_predicate_count,
        page_count: total_pages,
        predicate_order: requests
            .iter()
            .map(NativeScalarPredicateRequest::kind_name)
            .collect(),
        predicate_dispatch,
        bitmap_dispatch,
        retained_page_buffers: true,
        short_circuited,
    }))
}

pub(super) fn native_scalar_bitmap_for_request(
    segments: &[TemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
) -> Result<Option<NativeScalarBitmapScan>, BuildExecutionError> {
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    let mut page_count = 0usize;
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    for segment in segments {
        if segment.header.object_type_id != request.object_type_id() {
            continue;
        }
        let batch = native_object_temporal_batch_from_segment(
            segment,
            NativeCodeDomain {
                object_type_id: Some(segment.header.object_type_id),
                ..NativeCodeDomain::default()
            },
        )
        .map_err(|error| {
            exec_error(
                "E_NATIVE_SCALAR_LITERAL_PRUNE",
                format!("native scalar literal prune page binding failed: {error}"),
                json!({ "segment_id": segment.header.segment_id }),
            )
        })?;
        let Some(batch_scan) =
            native_scalar_bitmap_for_batch(&mut bitmap, loaded_rows, request, &batch)?
        else {
            return Ok(None);
        };
        page_count += batch_scan.page_count;
        predicate_dispatch.merge(batch_scan.predicate_dispatch);
    }
    Ok(Some(NativeScalarBitmapScan {
        bitmap,
        page_count,
        predicate_dispatch,
    }))
}

pub(super) fn native_scalar_bitmap_for_retained_request(
    segments: &[RetainedTemporalSegmentData],
    surface: &CoveObjectKernelSurface,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
) -> Result<Option<NativeScalarBitmapScan>, BuildExecutionError> {
    let mut bitmap = SelectionBitmap::none(surface.system.len());
    let mut page_count = 0usize;
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    for segment in segments {
        if segment.header.object_type_id != request.object_type_id() {
            continue;
        }
        let batch = native_object_temporal_batch_from_retained_segment(
            segment,
            NativeCodeDomain {
                object_type_id: Some(segment.header.object_type_id),
                ..NativeCodeDomain::default()
            },
        )
        .map_err(|error| {
            exec_error(
                "E_NATIVE_SCALAR_LITERAL_PRUNE",
                format!("retained native scalar literal prune page binding failed: {error}"),
                json!({ "segment_id": segment.header.segment_id }),
            )
        })?;
        let Some(batch_scan) =
            native_scalar_bitmap_for_batch(&mut bitmap, loaded_rows, request, &batch)?
        else {
            return Ok(None);
        };
        page_count += batch_scan.page_count;
        predicate_dispatch.merge(batch_scan.predicate_dispatch);
    }
    Ok(Some(NativeScalarBitmapScan {
        bitmap,
        page_count,
        predicate_dispatch,
    }))
}

pub(super) fn native_selection_from_kernel(
    result: Result<(SelectionBitmap, KernelStats), CoveError>,
) -> Result<(SelectionBitmap, NativeKernelDispatch), CoveError> {
    result.map(|(selected, stats)| (selected, stats.dispatch))
}

pub(super) fn native_selection_from_bitmap(
    selected: SelectionBitmap,
) -> (SelectionBitmap, NativeKernelDispatch) {
    (selected, NativeKernelDispatch::Scalar)
}

pub(super) fn native_scalar_bitmap_for_batch(
    bitmap: &mut SelectionBitmap,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
    batch: &NativeObjectTemporalBatch<'_>,
) -> Result<Option<NativeScalarBatchScan>, BuildExecutionError> {
    if let Some(scanned_units) =
        native_scalar_bitmap_for_system_batch(bitmap, loaded_rows, request, batch)?
    {
        return Ok(Some(scanned_units));
    }
    let mut page_count = 0usize;
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    for page in &batch.property_pages {
        if page.property_id != request.property_id() {
            continue;
        }
        page_count += 1;
        let (selected, dispatch) = match (request, &page.lane) {
            (NativeScalarPredicateRequest::NullCheck { want_valid, .. }, lane) => {
                native_validity_selection_for_lane(lane, page.row_count, *want_valid).ok_or(
                    CoveError::UnsupportedEncoding(
                        "native null check requires a validity-backed lane".into(),
                    ),
                )
            }
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::Bool {
                    values,
                    row_count,
                    validity,
                    ..
                },
            ) => native_selection_from_kernel(filter_bool_eq(
                values, *row_count, *validity, *value, None,
            )),
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                },
            ) if *logical_type == CoveLogicalType::Bool
                && *physical_kind == CovePhysicalKind::Boolean =>
            {
                Ok(local_u8_selection_for_targets(
                    values,
                    *validity,
                    local_to_global,
                    &[u64::from(u8::from(*value))],
                    true,
                ))
            }
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                },
            ) if *logical_type == CoveLogicalType::Bool
                && *physical_kind == CovePhysicalKind::Boolean =>
            {
                Ok(local_u16_selection_for_targets(
                    values,
                    *validity,
                    local_to_global,
                    &[u64::from(u8::from(*value))],
                    true,
                ))
            }
            (
                NativeScalarPredicateRequest::BoolEq { value, .. },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type,
                    physical_kind,
                    ..
                },
            ) if *logical_type == CoveLogicalType::Bool
                && *physical_kind == CovePhysicalKind::Boolean =>
            {
                Ok(local_u32_selection_for_targets(
                    values,
                    *validity,
                    local_to_global,
                    &[u64::from(u8::from(*value))],
                    true,
                ))
            }
            (
                NativeScalarPredicateRequest::FixedBytesEq {
                    logical_type,
                    value,
                    ..
                },
                LaneRef::FixedBytes {
                    values,
                    width,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_fixed_bytes_eq(values, *row_count, *width, *validity, value, None),
            ),
            (
                NativeScalarPredicateRequest::FixedBytesIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::FixedBytes {
                    values,
                    width,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_fixed_bytes_in(values, *row_count, *width, *validity, needles, None),
            ),
            (
                NativeScalarPredicateRequest::FixedBytesNotIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::FixedBytes {
                    values,
                    width,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => filter_fixed_bytes_in(
                values, *row_count, *width, *validity, needles, None,
            )
            .map(|(matched, stats)| {
                (
                    valid_rows_except(*row_count, *validity, &matched),
                    stats.dispatch,
                )
            }),
            (
                NativeScalarPredicateRequest::VarBytesEq {
                    logical_type,
                    value,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_varbytes_eq(row_offsets, values, *validity, value, None),
            ),
            (
                NativeScalarPredicateRequest::VarBytesIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                let needles = needles.iter().map(Vec::as_slice).collect::<Vec<_>>();
                native_selection_from_kernel(filter_varbytes_in(
                    row_offsets,
                    values,
                    *validity,
                    &needles,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::VarBytesNotIn {
                    logical_type,
                    values: needles,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                let needles = needles.iter().map(Vec::as_slice).collect::<Vec<_>>();
                filter_varbytes_in(row_offsets, values, *validity, &needles, None).map(
                    |(matched, stats)| {
                        (
                            valid_rows_except(row_offsets.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                NativeScalarPredicateRequest::VarBytesPrefix {
                    logical_type,
                    prefix,
                    ..
                },
                LaneRef::VarBytes {
                    row_offsets,
                    values,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => native_selection_from_kernel(
                filter_varbytes_prefix(row_offsets, values, *validity, prefix, None),
            ),
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::NumCodeU64LeBytes {
                    bytes,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                native_selection_from_kernel(filter_numcode_le_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    *op,
                    *literal,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::NumCodeU64LeBytes {
                    bytes,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                native_selection_from_kernel(filter_numcode_le_in_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    literals,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::NumCodeU64LeBytes {
                    bytes,
                    row_count,
                    validity,
                    logical_type: lane_logical_type,
                    ..
                },
            ) if lane_logical_type == logical_type => {
                native_selection_from_kernel(filter_numcode_le_not_in_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    literals,
                    None,
                ))
            }
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_membership(local_to_global, *logical_type, *op, *literal).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u8_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_membership(local_to_global, *logical_type, *op, *literal).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u16_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCode {
                    logical_type,
                    op,
                    literal,
                    ..
                },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_membership(local_to_global, *logical_type, *op, *literal).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u32_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u8_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU8 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (matched, stats) =
                            filter_local_u8_membership(values, *validity, &membership, None);
                        (
                            valid_rows_except(values.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u16_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU16 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (matched, stats) =
                            filter_local_u16_membership(values, *validity, &membership, None);
                        (
                            valid_rows_except(values.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (selected, stats) =
                            filter_local_u32_membership(values, *validity, &membership, None);
                        (selected, stats.dispatch)
                    },
                )
            }
            (
                NativeScalarPredicateRequest::NumCodeNotIn {
                    logical_type,
                    literals,
                    ..
                },
                LaneRef::LocalCodeU32 {
                    values,
                    validity,
                    local_to_global,
                    logical_type: lane_logical_type,
                    physical_kind,
                    ..
                },
            ) if lane_logical_type == logical_type
                && *physical_kind == CovePhysicalKind::NumCode =>
            {
                local_numcode_in_membership(local_to_global, *logical_type, literals).map(
                    |membership| {
                        let (matched, stats) =
                            filter_local_u32_membership(values, *validity, &membership, None);
                        (
                            valid_rows_except(values.len(), *validity, &matched),
                            stats.dispatch,
                        )
                    },
                )
            }
            (
                _,
                LaneRef::DecodeBoundary {
                    reason: "all-null elided page",
                    ..
                },
            ) => Ok(native_selection_from_bitmap(SelectionBitmap::none(
                page.row_count,
            ))),
            _ => return Ok(None),
        }
        .map_err(|error| {
            exec_error(
                "E_NATIVE_SCALAR_LITERAL_PRUNE",
                format!("native scalar literal prune page scan failed: {error}"),
                json!({ "segment_id": batch.segment_id }),
            )
        })?;
        predicate_dispatch.record(dispatch);

        for local_row in selected.to_selection_vector().rows() {
            set_native_scalar_prune_surface_row(
                bitmap,
                loaded_rows,
                batch.segment_id,
                page.row_start,
                *local_row as usize,
            )?;
        }
    }
    if page_count == 0 {
        return Ok(None);
    }
    Ok(Some(NativeScalarBatchScan {
        page_count,
        predicate_dispatch,
    }))
}

pub(super) fn native_scalar_bitmap_for_system_batch(
    bitmap: &mut SelectionBitmap,
    loaded_rows: &SurfaceRowLookup,
    request: &NativeScalarPredicateRequest,
    batch: &NativeObjectTemporalBatch<'_>,
) -> Result<Option<NativeScalarBatchScan>, BuildExecutionError> {
    if !matches!(
        request,
        NativeScalarPredicateRequest::SystemNumeric { .. }
            | NativeScalarPredicateRequest::SystemGoidEq { .. }
    ) {
        return Ok(None);
    }
    for (row_index, row) in batch.rows.iter().enumerate() {
        let matched = match request {
            NativeScalarPredicateRequest::SystemNumeric {
                field, op, literal, ..
            } => native_system_numeric_row_matches(row, *field, *op, *literal)?,
            NativeScalarPredicateRequest::SystemGoidEq { value, .. } => row.goid == *value,
            _ => unreachable!(),
        };
        if matched {
            set_native_scalar_prune_surface_row(
                bitmap,
                loaded_rows,
                batch.segment_id,
                0,
                row_index,
            )?;
        }
    }
    let mut predicate_dispatch = NativeKernelDispatchCounts::default();
    predicate_dispatch.record(NativeKernelDispatch::Scalar);
    Ok(Some(NativeScalarBatchScan {
        page_count: 1,
        predicate_dispatch,
    }))
}

pub(super) fn native_system_numeric_row_matches(
    row: &cove_core::profile::cove_o::TemporalRowEntryV1,
    field: NativeSystemNumericField,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<bool, BuildExecutionError> {
    let (logical_type, raw_value) = match field {
        NativeSystemNumericField::BranchKey => (CoveLogicalType::UInt64, row.branch_key),
        NativeSystemNumericField::Csn => (CoveLogicalType::UInt64, row.csn),
        NativeSystemNumericField::TimestampUs => {
            (CoveLogicalType::TimestampMicros, row.timestamp_us as u64)
        }
    };
    native_numcode_matches(logical_type, raw_value, op, literal).map_err(|error| {
        exec_error(
            "E_NATIVE_SCALAR_LITERAL_PRUNE",
            format!("native system row predicate failed: {error}"),
            json!({ "logical_type": format!("{logical_type:?}") }),
        )
    })
}

pub(super) fn local_numcode_membership(
    local_to_global: &[u64],
    logical_type: CoveLogicalType,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<Vec<bool>, CoveError> {
    local_to_global
        .iter()
        .copied()
        .map(|value| native_numcode_matches(logical_type, value, op, literal))
        .collect()
}

pub(super) fn local_numcode_in_membership(
    local_to_global: &[u64],
    logical_type: CoveLogicalType,
    literals: &[NativeNumericLiteral],
) -> Result<Vec<bool>, CoveError> {
    local_to_global
        .iter()
        .copied()
        .map(|value| {
            for literal in literals {
                if native_numcode_matches(
                    logical_type,
                    value,
                    NativeNumericPredicateOp::Eq,
                    *literal,
                )? {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .collect()
}

pub(super) fn local_u8_selection_for_targets(
    values: &[u8],
    validity: ValidityRef<'_>,
    local_to_global: &[u64],
    target_codes: &[u64],
    include: bool,
) -> (SelectionBitmap, NativeKernelDispatch) {
    let membership = local_membership_u8(local_to_global, target_codes);
    let (matched, stats) = filter_local_u8_membership(values, validity, &membership, None);
    let selected = if include {
        matched
    } else {
        valid_rows_except(values.len(), validity, &matched)
    };
    (selected, stats.dispatch)
}

pub(super) fn local_u16_selection_for_targets(
    values: &[u16],
    validity: ValidityRef<'_>,
    local_to_global: &[u64],
    target_codes: &[u64],
    include: bool,
) -> (SelectionBitmap, NativeKernelDispatch) {
    let membership = local_membership_u8(local_to_global, target_codes);
    let (matched, stats) = filter_local_u16_membership(values, validity, &membership, None);
    let selected = if include {
        matched
    } else {
        valid_rows_except(values.len(), validity, &matched)
    };
    (selected, stats.dispatch)
}

pub(super) fn local_u32_selection_for_targets(
    values: &[u32],
    validity: ValidityRef<'_>,
    local_to_global: &[u64],
    target_codes: &[u64],
    include: bool,
) -> (SelectionBitmap, NativeKernelDispatch) {
    let membership = local_membership_u8(local_to_global, target_codes);
    let (matched, stats) = filter_local_u32_membership(values, validity, &membership, None);
    let selected = if include {
        matched
    } else {
        valid_rows_except(values.len(), validity, &matched)
    };
    (selected, stats.dispatch)
}

pub(super) fn native_validity_selection_for_lane(
    lane: &LaneRef<'_>,
    page_row_count: usize,
    want_valid: bool,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if lane.row_count() != page_row_count {
        return None;
    }
    let validity = lane.validity()?;
    let (selected, stats) = filter_validity(page_row_count, validity, want_valid, None);
    Some((selected, stats.dispatch))
}

impl NativeScalarPredicateRequest {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::SystemNumeric { .. } => "system_numeric",
            Self::SystemGoidEq { .. } => "system_goid_eq",
            Self::NullCheck { .. } => "null_check",
            Self::BoolEq { .. } => "bool_eq",
            Self::FixedBytesEq { .. } => "fixed_bytes_eq",
            Self::FixedBytesIn { .. } => "fixed_bytes_in",
            Self::FixedBytesNotIn { .. } => "fixed_bytes_not_in",
            Self::VarBytesEq { .. } => "varbytes_eq",
            Self::VarBytesIn { .. } => "varbytes_in",
            Self::VarBytesNotIn { .. } => "varbytes_not_in",
            Self::VarBytesPrefix { .. } => "varbytes_prefix",
            Self::NumCode { .. } => "numcode",
            Self::NumCodeIn { .. } => "numcode_in",
            Self::NumCodeNotIn { .. } => "numcode_not_in",
        }
    }

    fn object_type_id(&self) -> u32 {
        match self {
            Self::SystemNumeric { object_type_id, .. }
            | Self::SystemGoidEq { object_type_id, .. }
            | Self::NullCheck { object_type_id, .. }
            | Self::BoolEq { object_type_id, .. }
            | Self::FixedBytesEq { object_type_id, .. }
            | Self::FixedBytesIn { object_type_id, .. }
            | Self::FixedBytesNotIn { object_type_id, .. }
            | Self::VarBytesEq { object_type_id, .. }
            | Self::VarBytesIn { object_type_id, .. }
            | Self::VarBytesNotIn { object_type_id, .. }
            | Self::VarBytesPrefix { object_type_id, .. }
            | Self::NumCode { object_type_id, .. }
            | Self::NumCodeIn { object_type_id, .. }
            | Self::NumCodeNotIn { object_type_id, .. } => *object_type_id,
        }
    }

    fn property_id(&self) -> u32 {
        match self {
            Self::NullCheck { property_id, .. }
            | Self::BoolEq { property_id, .. }
            | Self::FixedBytesEq { property_id, .. }
            | Self::FixedBytesIn { property_id, .. }
            | Self::FixedBytesNotIn { property_id, .. }
            | Self::VarBytesEq { property_id, .. }
            | Self::VarBytesIn { property_id, .. }
            | Self::VarBytesNotIn { property_id, .. }
            | Self::VarBytesPrefix { property_id, .. }
            | Self::NumCode { property_id, .. }
            | Self::NumCodeIn { property_id, .. }
            | Self::NumCodeNotIn { property_id, .. } => *property_id,
            Self::SystemNumeric { .. } | Self::SystemGoidEq { .. } => {
                unreachable!("system native scalar predicates do not have property ids")
            }
        }
    }
}

pub(super) fn native_scalar_prune_covers_all_kernel_predicates(
    prune: &NativeScalarPredicatePrune,
    predicates: &[KernelPredicate],
) -> bool {
    native_scalar_prune_covers_kernel_predicate_count(prune, predicates.len())
}

pub(super) fn native_scalar_prune_covers_kernel_predicate_count(
    prune: &NativeScalarPredicatePrune,
    predicate_count: usize,
) -> bool {
    predicate_count > 0
        && prune.page_count > 0
        && prune.predicate_count == predicate_count
        && (prune.executed_predicate_count == prune.predicate_count || prune.short_circuited)
}

pub(super) fn code_prune_covers_kernel_predicate_count(
    prune_predicate_count: usize,
    prune_page_count: usize,
    predicate_count: usize,
) -> bool {
    predicate_count > 0 && prune_page_count > 0 && prune_predicate_count == predicate_count
}

pub(super) fn order_native_scalar_requests_by_cost(
    requests: &mut [NativeScalarPredicateRequest],
) -> bool {
    let mut indexed = requests.iter().cloned().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(index, request)| (native_scalar_request_cost_key(request), *index));
    let reordered = indexed
        .iter()
        .enumerate()
        .any(|(slot, (original, _))| slot != *original);
    for (slot, (_, request)) in requests.iter_mut().zip(indexed) {
        *slot = request;
    }
    reordered
}

pub(super) fn native_scalar_request_cost_key(request: &NativeScalarPredicateRequest) -> (u8, u8) {
    match request {
        NativeScalarPredicateRequest::SystemNumeric { .. } => (0, 0),
        NativeScalarPredicateRequest::SystemGoidEq { .. } => (0, 1),
        NativeScalarPredicateRequest::NullCheck { .. } => (1, 0),
        NativeScalarPredicateRequest::BoolEq { .. } => (2, 0),
        NativeScalarPredicateRequest::NumCode { .. } => (3, 0),
        NativeScalarPredicateRequest::NumCodeIn { .. } => (3, 1),
        NativeScalarPredicateRequest::NumCodeNotIn { .. } => (3, 2),
        NativeScalarPredicateRequest::FixedBytesEq { .. } => (4, 0),
        NativeScalarPredicateRequest::FixedBytesIn { .. } => (4, 1),
        NativeScalarPredicateRequest::FixedBytesNotIn { .. } => (4, 2),
        NativeScalarPredicateRequest::VarBytesEq { .. } => (5, 0),
        NativeScalarPredicateRequest::VarBytesIn { .. } => (5, 1),
        NativeScalarPredicateRequest::VarBytesNotIn { .. } => (5, 2),
        NativeScalarPredicateRequest::VarBytesPrefix { .. } => (5, 3),
    }
}

pub(super) fn surface_row_lookup_for_object(
    surface: &CoveObjectKernelSurface,
    root_object_type_id: u32,
) -> SurfaceRowLookup {
    let mut segment_rows = BTreeMap::<u32, Vec<Option<usize>>>::new();
    for row in 0..surface.system.len() {
        if surface.system.object_type_ids[row] == root_object_type_id {
            let segment_id = surface.system.segment_ids[row];
            let row_index = surface.system.row_indices[row] as usize;
            let rows = segment_rows.entry(segment_id).or_default();
            if rows.len() <= row_index {
                rows.resize(row_index + 1, None);
            }
            rows[row_index] = Some(row);
        }
    }
    SurfaceRowLookup::from_segment_rows(segment_rows)
}

pub(super) fn set_native_scalar_prune_surface_row(
    bitmap: &mut SelectionBitmap,
    loaded_rows: &SurfaceRowLookup,
    segment_id: u32,
    page_row_start: usize,
    local_row: usize,
) -> Result<(), BuildExecutionError> {
    let row_index = page_row_start.checked_add(local_row).ok_or_else(|| {
        exec_error(
            "E_NATIVE_SCALAR_LITERAL_PRUNE",
            "native scalar literal prune page row offset overflowed",
            json!({}),
        )
    })?;
    let row_index = u32::try_from(row_index).map_err(|_| {
        exec_error(
            "E_NATIVE_SCALAR_LITERAL_PRUNE",
            "native scalar literal prune page row index exceeded u32",
            json!({}),
        )
    })?;
    if let Some(surface_row) = loaded_rows.get(segment_id, row_index) {
        bitmap.set(surface_row);
    }
    Ok(())
}

pub(super) fn decode_compact_hex_bytes(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk).ok()?;
        out.push(u8::from_str_radix(text, 16).ok()?);
    }
    Some(out)
}
