use super::*;

pub trait NativeColumnPagePayload {
    fn header_row_count(&self) -> u32;
    fn root_node_ref(&self) -> Result<&CoveEncodingNodeV1, CoveError>;
    fn buffer_bytes_ref(&self, kind: PageBufferKind) -> Result<Option<&[u8]>, CoveError>;
}

impl NativeColumnPagePayload for ColumnPagePayloadV1 {
    fn header_row_count(&self) -> u32 {
        self.header.row_count
    }

    fn root_node_ref(&self) -> Result<&CoveEncodingNodeV1, CoveError> {
        self.root_node()
    }

    fn buffer_bytes_ref(&self, kind: PageBufferKind) -> Result<Option<&[u8]>, CoveError> {
        self.buffer_bytes(kind)
    }
}

impl NativeColumnPagePayload for RetainedColumnPagePayloadV1 {
    fn header_row_count(&self) -> u32 {
        self.header.row_count
    }

    fn root_node_ref(&self) -> Result<&CoveEncodingNodeV1, CoveError> {
        self.root_node()
    }

    fn buffer_bytes_ref(&self, kind: PageBufferKind) -> Result<Option<&[u8]>, CoveError> {
        self.buffer_bytes(kind)
    }
}

pub fn native_lane_from_column_page_payload<'a, P: NativeColumnPagePayload + ?Sized>(
    column: &TableColumnDirectoryEntryV1,
    page: &crate::page::ColumnPageIndexEntryV1,
    payload: &'a P,
    mut domain: NativeCodeDomain,
) -> Result<LaneRef<'a>, CoveError> {
    if page.column_id != column.column_id {
        return Err(CoveError::PageCorrupt);
    }
    domain.column_id.get_or_insert(column.column_id);
    if column.domain_ref != 0 {
        domain
            .semantic_domain_id
            .get_or_insert_with(|| format!("table-domain:{}", column.domain_ref));
    }
    native_lane_from_object_page_payload(
        NativeLaneId(column.column_id),
        column.logical_type,
        column.physical_kind,
        page,
        payload,
        domain,
    )
}

pub(super) fn native_lane_from_object_page_payload<'a, P: NativeColumnPagePayload + ?Sized>(
    lane_id: NativeLaneId,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    page: &crate::page::ColumnPageIndexEntryV1,
    payload: &'a P,
    domain: NativeCodeDomain,
) -> Result<LaneRef<'a>, CoveError> {
    if payload.header_row_count() != page.row_count {
        return Err(CoveError::PageCorrupt);
    }
    let root = payload.root_node_ref()?;
    if root.logical_type != logical_type
        || root.physical_kind != physical_kind
        || root.logical_len != page.row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let row_count = usize::try_from(page.row_count).map_err(|_| CoveError::OffsetRange)?;
    let validity = native_validity_from_page(payload, page)?;
    let values = payload
        .buffer_bytes_ref(PageBufferKind::Values)?
        .unwrap_or(&[]);

    match (root.encoding_kind, root.physical_kind) {
        (CoveEncodingKind::NumCode, CovePhysicalKind::NumCode) => {
            validate_fixed_le_width_for_validity(
                values,
                row_count,
                std::mem::size_of::<u64>(),
                validity,
            )?;
            Ok(LaneRef::NumCodeU64LeBytes {
                lane_id,
                bytes: values,
                row_count,
                validity,
                logical_type,
                domain,
            })
        }
        (CoveEncodingKind::FileCode, CovePhysicalKind::FileCode) => {
            validate_fixed_le_width_for_validity(
                values,
                row_count,
                std::mem::size_of::<u32>(),
                validity,
            )?;
            Ok(LaneRef::FileCodeU32LeBytes {
                lane_id,
                bytes: values,
                row_count,
                validity,
                logical_type,
                domain,
            })
        }
        (CoveEncodingKind::LocalCodebook, physical_kind) => local_codebook_lane_from_payload(
            lane_id,
            logical_type,
            physical_kind,
            values,
            row_count,
            validity,
            domain,
        ),
        (CoveEncodingKind::PlainFixed, CovePhysicalKind::Boolean) => {
            validate_fixed_le_width_for_validity(values, row_count, 1, validity)?;
            validate_bool_bytes(values, row_count, validity)?;
            Ok(LaneRef::Bool {
                lane_id,
                values,
                row_count,
                validity,
                domain,
            })
        }
        (CoveEncodingKind::PlainFixed, CovePhysicalKind::FixedBytes) => {
            let width = logical_type_fixed_width(logical_type).ok_or_else(|| {
                CoveError::UnsupportedEncoding(format!(
                    "plain-fixed native lane requires fixed-width logical type, got {logical_type:?}"
                ))
            })?;
            validate_fixed_le_width_for_validity(values, row_count, width, validity)?;
            Ok(LaneRef::FixedBytes {
                lane_id,
                values,
                width,
                row_count,
                validity,
                logical_type,
                domain,
            })
        }
        (CoveEncodingKind::VarBytes, CovePhysicalKind::VarBytes) => {
            let row_offsets = prepare_varbytes_row_offsets(values, row_count, validity)?;
            Ok(LaneRef::VarBytes {
                lane_id,
                row_offsets: Cow::Owned(row_offsets),
                values,
                validity,
                logical_type,
                domain,
            })
        }
        (encoding_kind, physical_kind) => Ok(LaneRef::DecodeBoundary {
            lane_id,
            logical_type,
            physical_kind,
            row_count,
            validity: Some(validity),
            reason: decode_boundary_reason_for_encoding(encoding_kind),
        }),
    }
}

pub(super) fn local_codebook_lane_from_payload<'a>(
    lane_id: NativeLaneId,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'a>,
    domain: NativeCodeDomain,
) -> Result<LaneRef<'a>, CoveError> {
    let payload = LocalCodebookPayload::parse(values)?;
    let local_indexes = payload.decode_local_indexes()?;
    if local_indexes.len() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let local_to_global = match (physical_kind, &payload.values) {
        (CovePhysicalKind::FileCode, LocalCodebookValues::FileCode(values)) => {
            values.iter().copied().map(u64::from).collect::<Vec<_>>()
        }
        (CovePhysicalKind::NumCode, LocalCodebookValues::NumCode(values)) => values.clone(),
        (CovePhysicalKind::Boolean, LocalCodebookValues::Boolean(values)) => values
            .iter()
            .copied()
            .map(|value| u64::from(u8::from(value)))
            .collect::<Vec<_>>(),
        (CovePhysicalKind::VarBytes, LocalCodebookValues::VarBytes(_)) => {
            return Ok(LaneRef::DecodeBoundary {
                lane_id,
                logical_type,
                physical_kind,
                row_count,
                validity: Some(validity),
                reason: "local-codebook varbytes page needs local byte dictionary binding",
            });
        }
        _ => return Err(CoveError::PageCorrupt),
    };

    if local_to_global.len() <= (u8::MAX as usize) + 1 {
        let values = local_indexes
            .into_iter()
            .map(|value| u8::try_from(value).map_err(|_| CoveError::PageCorrupt))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(LaneRef::LocalCodeU8 {
            lane_id,
            values: Cow::Owned(values),
            validity,
            local_to_global: Cow::Owned(local_to_global),
            logical_type,
            physical_kind,
            domain,
        });
    }

    if local_to_global.len() <= (u16::MAX as usize) + 1 {
        let values = local_indexes
            .into_iter()
            .map(|value| u16::try_from(value).map_err(|_| CoveError::PageCorrupt))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(LaneRef::LocalCodeU16 {
            lane_id,
            values: Cow::Owned(values),
            validity,
            local_to_global: Cow::Owned(local_to_global),
            logical_type,
            physical_kind,
            domain,
        });
    }

    Ok(LaneRef::LocalCodeU32 {
        lane_id,
        values: Cow::Owned(local_indexes),
        validity,
        local_to_global: Cow::Owned(local_to_global),
        logical_type,
        physical_kind,
        domain,
    })
}

pub(super) fn native_validity_from_page<'a, P: NativeColumnPagePayload + ?Sized>(
    payload: &'a P,
    page: &crate::page::ColumnPageIndexEntryV1,
) -> Result<ValidityRef<'a>, CoveError> {
    let row_count = usize::try_from(page.row_count).map_err(|_| CoveError::OffsetRange)?;
    if page.null_count == page.row_count || page.flags & PAGE_FLAG_ALL_NULL != 0 {
        return Ok(ValidityRef::AllNull { row_count });
    }
    if page.null_count == 0 || page.flags & PAGE_FLAG_ALL_NON_NULL != 0 {
        return Ok(ValidityRef::AllValid { row_count });
    }
    let Some(bytes) = payload.buffer_bytes_ref(PageBufferKind::NullBitmap)? else {
        return Err(CoveError::PageCorrupt);
    };
    validate_bitmap_width(bytes, row_count)?;
    Ok(ValidityRef::CoveNullBitmap { bytes, row_count })
}

pub(super) fn elided_page_validity<'a>(
    page: &crate::page::ColumnPageIndexEntryV1,
) -> Option<ValidityRef<'a>> {
    let row_count = usize::try_from(page.row_count).ok()?;
    if page.null_count == page.row_count || page.flags & PAGE_FLAG_ALL_NULL != 0 {
        Some(ValidityRef::AllNull { row_count })
    } else if page.null_count == 0 || page.flags & PAGE_FLAG_ALL_NON_NULL != 0 {
        Some(ValidityRef::AllValid { row_count })
    } else {
        None
    }
}

pub(super) fn decode_boundary_reason_for_encoding(encoding_kind: CoveEncodingKind) -> &'static str {
    match encoding_kind {
        CoveEncodingKind::LocalCodebook => "local-codebook page value kind is not code-native",
        CoveEncodingKind::VarBytes => "varbytes page needs offsets view",
        CoveEncodingKind::PlainFixed => "plain-fixed page needs width-specific lane binding",
        CoveEncodingKind::Validity => "validity page needs boolean lane binding",
        CoveEncodingKind::Constant => "constant page needs constant-lane binding",
        _ => "encoding is not lane-native in scalar native kernel yet",
    }
}

pub(super) fn decode_boundary_reason_for_elided_page(flags: u32) -> &'static str {
    if flags & PAGE_FLAG_ALL_NULL != 0 {
        "all-null elided page"
    } else if flags & PAGE_FLAG_ALL_NON_NULL != 0 {
        "stats-only constant/elided non-null page requires stats materialization"
    } else {
        "property page has no retained payload"
    }
}

pub(super) fn validate_fixed_le_width_for_validity(
    bytes: &[u8],
    row_count: usize,
    width: usize,
    validity: ValidityRef<'_>,
) -> Result<(), CoveError> {
    let expected = row_count
        .checked_mul(width)
        .ok_or(CoveError::ArithOverflow)?;
    if matches!(validity, ValidityRef::AllNull { .. }) && bytes.is_empty() {
        return Ok(());
    }
    if bytes.len() != expected {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

pub(super) fn validate_bool_bytes(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
) -> Result<(), CoveError> {
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok(());
    }
    if values.len() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    if values.iter().any(|value| !matches!(value, 0 | 1)) {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

pub(super) fn prepare_varbytes_row_offsets(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
) -> Result<Vec<u32>, CoveError> {
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok(vec![0; row_count]);
    }
    let mut row_offsets = Vec::with_capacity(row_count);
    let mut pos = 0usize;
    for _ in 0..row_count {
        row_offsets.push(u32::try_from(pos).map_err(|_| CoveError::ArithOverflow)?);
        let len_end = pos.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        if values.get(pos..len_end).is_none() {
            return Err(CoveError::BufferTooShort);
        }
        let len = wire::read_u32_le_checked(values, pos)? as usize;
        pos = len_end.checked_add(len).ok_or(CoveError::ArithOverflow)?;
        if pos > values.len() {
            return Err(CoveError::BufferTooShort);
        }
    }
    if pos != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok(row_offsets)
}

pub(super) fn varbytes_payload_at<'a>(
    row_offsets: &[u32],
    values: &'a [u8],
    row: usize,
) -> Result<&'a [u8], CoveError> {
    let offset = row_offsets
        .get(row)
        .copied()
        .ok_or(CoveError::OffsetRange)? as usize;
    let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
    if values.get(offset..len_end).is_none() {
        return Err(CoveError::BufferTooShort);
    }
    let len = wire::read_u32_le_checked(values, offset)? as usize;
    let value_end = len_end.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    values
        .get(len_end..value_end)
        .ok_or(CoveError::BufferTooShort)
}

pub(super) fn validate_bitmap_width(bytes: &[u8], row_count: usize) -> Result<(), CoveError> {
    let expected = row_count.checked_add(7).ok_or(CoveError::ArithOverflow)? / 8;
    if bytes.len() < expected {
        return Err(CoveError::BufferTooShort);
    }
    Ok(())
}

pub(super) fn try_filter_u32_values_in_simd(
    values: &[u32],
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if needles.is_empty()
        || needles.len() > 8
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    if let Some(result) = filter_local_u32_membership_neon_dispatch(values, needles) {
        return Some(result);
    }
    filter_local_u32_membership_avx2_dispatch(values, needles)
}

pub(super) fn try_filter_bool_eq_simd(
    values: &[u8],
    row_count: usize,
    needle: u8,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    debug_assert_eq!(values.len(), row_count);
    let needle = [needle];
    if let Some(result) = filter_local_u8_membership_neon_dispatch(values, &needle) {
        return Some(result);
    }
    filter_local_u8_membership_avx2_dispatch(values, &needle)
}

pub(super) fn try_filter_local_u8_membership_simd(
    values: &[u8],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    let mut needles = [0u8; 8];
    let mut needle_count = 0usize;
    for (local, matches) in local_membership.iter().take(256).copied().enumerate() {
        if !matches {
            continue;
        }
        if needle_count == needles.len() {
            return None;
        }
        needles[needle_count] = local as u8;
        needle_count += 1;
    }
    if needle_count == 0 {
        return None;
    }
    if let Some(result) = filter_local_u8_membership_neon_dispatch(values, &needles[..needle_count])
    {
        return Some(result);
    }
    filter_local_u8_membership_avx2_dispatch(values, &needles[..needle_count])
}

pub(super) fn try_filter_local_u16_membership_simd(
    values: &[u16],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    let (needles, needle_count) = collect_small_local_membership_u16(local_membership)?;
    if let Some(result) =
        filter_local_u16_membership_neon_dispatch(values, &needles[..needle_count])
    {
        return Some(result);
    }
    filter_local_u16_membership_avx2_dispatch(values, &needles[..needle_count])
}

pub(super) fn try_filter_local_u32_membership_simd(
    values: &[u32],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    let (needles, needle_count) = collect_small_local_membership_u32(local_membership)?;
    if let Some(result) =
        filter_local_u32_membership_neon_dispatch(values, &needles[..needle_count])
    {
        return Some(result);
    }
    filter_local_u32_membership_avx2_dispatch(values, &needles[..needle_count])
}

pub(super) fn collect_small_local_membership_u16(
    local_membership: &[bool],
) -> Option<([u16; 8], usize)> {
    let mut needles = [0u16; 8];
    let mut needle_count = 0usize;
    for (local, matches) in local_membership
        .iter()
        .take(usize::from(u16::MAX) + 1)
        .copied()
        .enumerate()
    {
        if !matches {
            continue;
        }
        if needle_count == needles.len() {
            return None;
        }
        needles[needle_count] = local as u16;
        needle_count += 1;
    }
    (needle_count != 0).then_some((needles, needle_count))
}

pub(super) fn collect_small_local_membership_u32(
    local_membership: &[bool],
) -> Option<([u32; 8], usize)> {
    const MAX_SIMD_MEMBERSHIP_SCAN: usize = 1 << 20;
    if local_membership.len() > MAX_SIMD_MEMBERSHIP_SCAN {
        return None;
    }
    let mut needles = [0u32; 8];
    let mut needle_count = 0usize;
    for (local, matches) in local_membership.iter().copied().enumerate() {
        if !matches {
            continue;
        }
        if needle_count == needles.len() {
            return None;
        }
        needles[needle_count] = u32::try_from(local).ok()?;
        needle_count += 1;
    }
    (needle_count != 0).then_some((needles, needle_count))
}

pub(super) fn try_filter_u64_le_eq_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: u64,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    match policy {
        NativeKernelDispatch::Scalar => None,
        NativeKernelDispatch::Auto => filter_u64_le_eq_auto(bytes, row_count, needle),
        NativeKernelDispatch::Avx2 => filter_u64_le_eq_avx2_dispatch(bytes, row_count, needle),
        NativeKernelDispatch::Neon => filter_u64_le_eq_neon_dispatch(bytes, row_count, needle),
    }
}

pub(super) fn try_filter_u32_le_eq_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: u32,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    match policy {
        NativeKernelDispatch::Scalar => None,
        NativeKernelDispatch::Auto => filter_u32_le_eq_auto(bytes, row_count, needle),
        NativeKernelDispatch::Avx2 => filter_u32_le_eq_avx2_dispatch(bytes, row_count, needle),
        NativeKernelDispatch::Neon => filter_u32_le_eq_neon_dispatch(bytes, row_count, needle),
    }
}

pub(super) fn try_filter_i64_le_cmp_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    op: NativeNumericPredicateOp,
    needle: i64,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    match policy {
        NativeKernelDispatch::Scalar => None,
        NativeKernelDispatch::Auto => filter_i64_le_cmp_auto(bytes, row_count, op, needle),
        NativeKernelDispatch::Avx2 => filter_i64_le_cmp_avx2_dispatch(bytes, row_count, op, needle),
        NativeKernelDispatch::Neon => filter_i64_le_cmp_neon_dispatch(bytes, row_count, op, needle),
    }
}

pub(super) fn try_filter_i64_le_range_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    if let Some(result) = filter_i64_le_range_neon_dispatch(bytes, row_count, lower, upper) {
        return Some(result);
    }
    filter_i64_le_range_avx2_dispatch(bytes, row_count, lower, upper)
}

pub(super) fn filter_u64_le_eq_auto(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        filter_u64_le_eq_neon_dispatch(bytes, row_count, needle)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                filter_u64_le_eq_avx2_dispatch(bytes, row_count, needle)
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (bytes, row_count, needle);
            None
        }
    }
}

pub(super) fn filter_u32_le_eq_auto(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        filter_u32_le_eq_neon_dispatch(bytes, row_count, needle)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                filter_u32_le_eq_avx2_dispatch(bytes, row_count, needle)
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (bytes, row_count, needle);
            None
        }
    }
}

pub(super) fn filter_i64_le_cmp_auto(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        filter_i64_le_cmp_neon_dispatch(bytes, row_count, op, needle)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                filter_i64_le_cmp_avx2_dispatch(bytes, row_count, op, needle)
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (bytes, row_count, op, needle);
            None
        }
    }
}

pub(super) fn filter_local_u8_membership_avx2_dispatch(
    values: &[u8],
    needles: &[u8],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // passes initialized `values`, at most eight initialized needle bytes,
        // and the SIMD routine performs only in-bounds unaligned loads plus a
        // scalar tail.
        unsafe {
            filter_local_u8_membership_avx2(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (values, needles);
        None
    }
}

// Keep the dispatch contract identical across architectures; this returns
// `Some` on AArch64 and `None` elsewhere.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn filter_local_u8_membership_neon_dispatch(
    values: &[u8],
    needles: &[u8],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // passes initialized `values`, at most eight initialized needle bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u8_membership_neon(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (values, needles);
        None
    }
}

pub(super) fn filter_local_u16_membership_avx2_dispatch(
    values: &[u16],
    needles: &[u16],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u16_membership_avx2(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (values, needles);
        None
    }
}

// Keep the dispatch contract identical across architectures; this returns
// `Some` on AArch64 and `None` elsewhere.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn filter_local_u16_membership_neon_dispatch(
    values: &[u16],
    needles: &[u16],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u16_membership_neon(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (values, needles);
        None
    }
}

pub(super) fn filter_local_u32_membership_avx2_dispatch(
    values: &[u32],
    needles: &[u32],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u32_membership_avx2(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (values, needles);
        None
    }
}

// Keep the dispatch contract identical across architectures; this returns
// `Some` on AArch64 and `None` elsewhere.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn filter_local_u32_membership_neon_dispatch(
    values: &[u32],
    needles: &[u32],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u32_membership_neon(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (values, needles);
        None
    }
}

pub(super) fn filter_u64_le_eq_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u64_le_eq_avx2(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

pub(super) fn filter_u32_le_eq_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 4` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u32_le_eq_avx2(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

pub(super) fn filter_i64_le_cmp_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_cmp_avx2(bytes, row_count, op, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, op, needle);
        None
    }
}

pub(super) fn filter_i64_le_range_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_range_avx2(bytes, row_count, lower, upper, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, lower, upper);
        None
    }
}

// Keep the dispatch contract identical across architectures; this returns
// `Some` on AArch64 and `None` elsewhere.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn filter_u64_le_eq_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u64_le_eq_neon(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

// Keep the dispatch contract identical across architectures; this returns
// `Some` on AArch64 and `None` elsewhere.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn filter_u32_le_eq_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 4` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u32_le_eq_neon(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

// Keep the dispatch contract identical across architectures; this returns
// `Some` on AArch64 and `None` elsewhere.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn filter_i64_le_cmp_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_cmp_neon(bytes, row_count, op, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, op, needle);
        None
    }
}

// Keep the dispatch contract identical across architectures; this returns
// `Some` on AArch64 and `None` elsewhere.
#[allow(clippy::unnecessary_wraps)]
pub(super) fn filter_i64_le_range_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_range_neon(bytes, row_count, lower, upper, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, lower, upper);
        None
    }
}

#[inline]
pub(super) fn intersect_words_scalar(left: &mut [u64], right: &[u64]) {
    for (left, right) in left.iter_mut().zip(right) {
        *left &= *right;
    }
}

#[inline]
pub(super) fn intersect_words_auto(left: &mut [u64], right: &[u64]) -> NativeKernelDispatch {
    if left.len() < 16 {
        intersect_words_scalar(left, right);
        return NativeKernelDispatch::Scalar;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: Runtime feature detection proves AVX2 support. The
            // slices are valid for `left.len()` u64 words and the function
            // handles unaligned loads/stores plus a scalar tail.
            unsafe {
                intersect_words_avx2(left, right);
            }
            return NativeKernelDispatch::Avx2;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 has Advanced SIMD/NEON as part of the baseline ISA.
        // SAFETY: The slices are valid for `left.len()` u64 words and the
        // function handles unaligned loads/stores plus a scalar tail.
        unsafe {
            intersect_words_neon(left, right);
        }
        NativeKernelDispatch::Neon
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        intersect_words_scalar(left, right);
        NativeKernelDispatch::Scalar
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `right` must contain
/// at least `left.len()` words, and `left` must be the exclusive mutable bitmap
/// buffer being intersected. The function handles unaligned vector accesses and
/// delegates any non-vector tail to the scalar implementation.
pub(super) unsafe fn intersect_words_avx2(left: &mut [u64], right: &[u64]) {
    use std::arch::x86_64::{__m256i, _mm256_and_si256, _mm256_loadu_si256, _mm256_storeu_si256};

    let chunks = left.len() / 4;
    for chunk in 0..chunks {
        let offset = chunk * 4;
        // SAFETY: `offset..offset+4` is inside both slices by construction.
        // AVX2 unaligned loads/stores accept arbitrary byte alignment.
        unsafe {
            let left_ptr = left.as_mut_ptr().add(offset).cast::<__m256i>();
            let right_ptr = right.as_ptr().add(offset).cast::<__m256i>();
            let lhs = _mm256_loadu_si256(left_ptr);
            let rhs = _mm256_loadu_si256(right_ptr);
            _mm256_storeu_si256(left_ptr, _mm256_and_si256(lhs, rhs));
        }
    }
    let tail = chunks * 4;
    intersect_words_scalar(&mut left[tail..], &right[tail..]);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `rows` must be an
/// exclusive mutable row buffer, and appending the 64 dense row ids starting at
/// `base` must be valid for the current scan morsel. The function reserves and
/// initializes all appended slots before extending the vector length.
pub(super) unsafe fn append_dense_u32_rows_avx2(rows: &mut Vec<u32>, base: u32) {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi32, _mm256_set1_epi32, _mm256_setr_epi32, _mm256_storeu_si256,
    };

    rows.reserve(64);
    let start = rows.len();
    // SAFETY: `rows.reserve(64)` guarantees space for 64 additional `u32`
    // values. The loop writes each new slot exactly once before `set_len`.
    unsafe {
        let ptr = rows.as_mut_ptr().add(start);
        let increments = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
        for offset in (0..64).step_by(8) {
            let chunk_base = base + offset as u32;
            let values = _mm256_add_epi32(_mm256_set1_epi32(chunk_base as i32), increments);
            _mm256_storeu_si256(ptr.add(offset).cast::<__m256i>(), values);
        }
        rows.set_len(start + 64);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `out` must have a
/// word capacity covering `values.len()` rows and must be exclusively mutable;
/// the bitmap may already contain set bits that are ORed with membership
/// matches. Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_local_u8_membership_avx2(
    values: &[u8],
    needles: &[u8],
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_or_si256,
        _mm256_set1_epi8, _mm256_setzero_si256,
    };

    let chunks = values.len() / 32;
    for chunk in 0..chunks {
        let row = chunk * 32;
        // SAFETY: `row..row+32` is inside `values` by construction. AVX2
        // unaligned loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row).cast::<__m256i>();
            let haystack = _mm256_loadu_si256(ptr);
            let mut matched = _mm256_setzero_si256();
            for needle in needles {
                let needle_vector = _mm256_set1_epi8(*needle as i8);
                matched = _mm256_or_si256(matched, _mm256_cmpeq_epi8(haystack, needle_vector));
            }
            _mm256_movemask_epi8(matched) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u8_membership_scalar_tail(values, chunks * 32, needles, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `out` must have a
/// word capacity covering `values.len()` rows and must be exclusively mutable;
/// the bitmap may already contain set bits that are ORed with membership
/// matches. Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_local_u16_membership_avx2(
    values: &[u16],
    needles: &[u16],
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi16, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_or_si256,
        _mm256_set1_epi16, _mm256_setzero_si256,
    };

    let chunks = values.len() / 16;
    for chunk in 0..chunks {
        let row = chunk * 16;
        // SAFETY: `row..row+16` is inside `values` by construction. AVX2
        // unaligned loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row).cast::<__m256i>();
            let haystack = _mm256_loadu_si256(ptr);
            let mut matched = _mm256_setzero_si256();
            for needle in needles {
                let needle_vector = _mm256_set1_epi16(*needle as i16);
                matched = _mm256_or_si256(matched, _mm256_cmpeq_epi16(haystack, needle_vector));
            }
            avx2_u16_lane_mask(_mm256_movemask_epi8(matched) as u32)
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u16_membership_scalar_tail(values, chunks * 16, needles, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `out` must have a
/// word capacity covering `values.len()` rows and must be exclusively mutable;
/// the bitmap may already contain set bits that are ORed with membership
/// matches. Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_local_u32_membership_avx2(
    values: &[u32],
    needles: &[u32],
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_ps, _mm256_cmpeq_epi32, _mm256_loadu_si256, _mm256_movemask_ps,
        _mm256_or_si256, _mm256_set1_epi32, _mm256_setzero_si256,
    };

    let chunks = values.len() / 8;
    for chunk in 0..chunks {
        let row = chunk * 8;
        // SAFETY: `row..row+8` is inside `values` by construction. AVX2
        // unaligned loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row).cast::<__m256i>();
            let haystack = _mm256_loadu_si256(ptr);
            let mut matched = _mm256_setzero_si256();
            for needle in needles {
                let needle_vector = _mm256_set1_epi32(*needle as i32);
                matched = _mm256_or_si256(matched, _mm256_cmpeq_epi32(haystack, needle_vector));
            }
            _mm256_movemask_ps(_mm256_castsi256_ps(matched)) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u32_membership_scalar_tail(values, chunks * 8, needles, out);
}

#[cfg(target_arch = "x86_64")]
#[inline]
pub(super) fn avx2_u16_lane_mask(byte_mask: u32) -> u64 {
    let mut lane_mask = 0u64;
    for lane in 0..16 {
        if byte_mask & (0b11u32 << (lane * 2)) != 0 {
            lane_mask |= 1u64 << lane;
        }
    }
    lane_mask
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `bytes` must contain
/// at least `row_count * 8` initialized little-endian bytes, and `out` must have
/// a word capacity covering `row_count` rows with exclusive mutable access.
/// Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_u64_le_eq_avx2(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_pd, _mm256_cmpeq_epi64, _mm256_loadu_si256, _mm256_movemask_pd,
        _mm256_set1_epi64x,
    };

    let needle_vector = _mm256_set1_epi64x(needle as i64);
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let cmp = _mm256_cmpeq_epi64(values, needle_vector);
            _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u64_le_eq_scalar_tail(bytes, chunks * 4, row_count, needle, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `bytes` must contain
/// at least `row_count * 4` initialized little-endian bytes, and `out` must have
/// a word capacity covering `row_count` rows with exclusive mutable access.
/// Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_u32_le_eq_avx2(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_ps, _mm256_cmpeq_epi32, _mm256_loadu_si256, _mm256_movemask_ps,
        _mm256_set1_epi32,
    };

    let needle_vector = _mm256_set1_epi32(needle as i32);
    let chunks = row_count / 8;
    for chunk in 0..chunks {
        let row = chunk * 8;
        // SAFETY: `row..row+8` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 4` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 4).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let cmp = _mm256_cmpeq_epi32(values, needle_vector);
            _mm256_movemask_ps(_mm256_castsi256_ps(cmp)) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u32_le_eq_scalar_tail(bytes, chunks * 8, row_count, needle, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `bytes` must contain
/// at least `row_count * 8` initialized little-endian bytes, and `out` must have
/// a word capacity covering `row_count` rows with exclusive mutable access.
/// Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_i64_le_cmp_avx2(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_pd, _mm256_cmpeq_epi64, _mm256_cmpgt_epi64, _mm256_loadu_si256,
        _mm256_movemask_pd, _mm256_set1_epi64x,
    };

    let needle_vector = _mm256_set1_epi64x(needle);
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let raw_mask = match op {
                NativeNumericPredicateOp::Eq => {
                    let cmp = _mm256_cmpeq_epi64(values, needle_vector);
                    _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
                }
                NativeNumericPredicateOp::Gt => {
                    let cmp = _mm256_cmpgt_epi64(values, needle_vector);
                    _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
                }
                NativeNumericPredicateOp::GtEq => {
                    let cmp = _mm256_cmpgt_epi64(needle_vector, values);
                    !(_mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64)
                }
                NativeNumericPredicateOp::Lt => {
                    let cmp = _mm256_cmpgt_epi64(needle_vector, values);
                    _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
                }
                NativeNumericPredicateOp::LtEq => {
                    let cmp = _mm256_cmpgt_epi64(values, needle_vector);
                    !(_mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64)
                }
            };
            raw_mask & 0b1111
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_cmp_scalar_tail(bytes, chunks * 4, row_count, op, needle, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
/// # Safety
///
/// Callers must dispatch this only when AVX2 is available. `bytes` must contain
/// at least `row_count * 8` initialized little-endian bytes, and `out` must have
/// a word capacity covering `row_count` rows with exclusive mutable access.
/// Bounds must be normalized by the caller to the native predicate semantics;
/// unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_i64_le_range_avx2(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_pd, _mm256_cmpgt_epi64, _mm256_loadu_si256, _mm256_movemask_pd,
        _mm256_set1_epi64x,
    };

    let lower_vector = lower.map(|(bound, inclusive)| (_mm256_set1_epi64x(bound), inclusive));
    let upper_vector = upper.map(|(bound, inclusive)| (_mm256_set1_epi64x(bound), inclusive));
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let mut mask = 0b1111u64;
            if let Some((bound_vector, inclusive)) = lower_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        let less_than_lower = _mm256_cmpgt_epi64(bound_vector, values);
                        !(_mm256_movemask_pd(_mm256_castsi256_pd(less_than_lower)) as u64)
                    }
                    BoundInclusive::Exclusive => {
                        let greater_than_lower = _mm256_cmpgt_epi64(values, bound_vector);
                        _mm256_movemask_pd(_mm256_castsi256_pd(greater_than_lower)) as u64
                    }
                };
                mask &= raw & 0b1111;
            }
            if let Some((bound_vector, inclusive)) = upper_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        let greater_than_upper = _mm256_cmpgt_epi64(values, bound_vector);
                        !(_mm256_movemask_pd(_mm256_castsi256_pd(greater_than_upper)) as u64)
                    }
                    BoundInclusive::Exclusive => {
                        let less_than_upper = _mm256_cmpgt_epi64(bound_vector, values);
                        _mm256_movemask_pd(_mm256_castsi256_pd(less_than_upper)) as u64
                    }
                };
                mask &= raw & 0b1111;
            }
            mask
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_range_scalar_tail(bytes, chunks * 4, row_count, lower, upper, out);
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `rows`
/// must be an exclusive mutable row buffer, and appending the 64 dense row ids
/// starting at `base` must be valid for the current scan morsel. The function
/// reserves and initializes all appended slots before extending the vector
/// length.
pub(super) unsafe fn append_dense_u32_rows_neon(rows: &mut Vec<u32>, base: u32) {
    use std::arch::aarch64::{uint32x4_t, vaddq_u32, vdupq_n_u32, vld1q_u32, vst1q_u32};

    const INCREMENTS: [u32; 4] = [0, 1, 2, 3];

    rows.reserve(64);
    let start = rows.len();
    // SAFETY: `INCREMENTS` is a 4-lane initialized table, and
    // `rows.reserve(64)` guarantees space for 64 additional `u32` values.
    // The loop writes each new slot exactly once before `set_len`.
    unsafe {
        let increments: uint32x4_t = vld1q_u32(INCREMENTS.as_ptr());
        let ptr = rows.as_mut_ptr().add(start);
        for offset in (0..64).step_by(4) {
            let chunk_base = base + offset as u32;
            let values = vaddq_u32(vdupq_n_u32(chunk_base), increments);
            vst1q_u32(ptr.add(offset), values);
        }
        rows.set_len(start + 64);
    }
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `out`
/// must have a word capacity covering `values.len()` rows and must be
/// exclusively mutable; the bitmap may already contain set bits that are ORed
/// with membership matches. Unaligned loads and scalar tail handling are
/// internal.
pub(super) unsafe fn filter_local_u8_membership_neon(
    values: &[u8],
    needles: &[u8],
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        uint8x16_t, vaddv_u8, vandq_u8, vceqq_u8, vdupq_n_u8, vget_high_u8, vget_low_u8, vld1q_u8,
        vorrq_u8,
    };

    const BIT_WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];

    // SAFETY: `BIT_WEIGHTS` is a 16-byte initialized table.
    let weights = unsafe { vld1q_u8(BIT_WEIGHTS.as_ptr()) };
    let chunks = values.len() / 16;
    for chunk in 0..chunks {
        let row = chunk * 16;
        // SAFETY: `row..row+16` is inside `values` by construction. AArch64
        // vector loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row);
            let haystack: uint8x16_t = vld1q_u8(ptr);
            let mut matched = vdupq_n_u8(0);
            for needle in needles {
                let needle_vector = vdupq_n_u8(*needle);
                matched = vorrq_u8(matched, vceqq_u8(haystack, needle_vector));
            }
            let weighted = vandq_u8(matched, weights);
            let low = u64::from(vaddv_u8(vget_low_u8(weighted)));
            let high = u64::from(vaddv_u8(vget_high_u8(weighted))) << 8;
            low | high
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u8_membership_scalar_tail(values, chunks * 16, needles, out);
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `out`
/// must have a word capacity covering `values.len()` rows and must be
/// exclusively mutable; the bitmap may already contain set bits that are ORed
/// with membership matches. Unaligned loads and scalar tail handling are
/// internal.
pub(super) unsafe fn filter_local_u16_membership_neon(
    values: &[u16],
    needles: &[u16],
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        uint16x8_t, vaddvq_u16, vandq_u16, vceqq_u16, vdupq_n_u16, vld1q_u16, vorrq_u16,
    };

    const BIT_WEIGHTS: [u16; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

    // SAFETY: `BIT_WEIGHTS` is an 8-lane initialized table.
    let weights = unsafe { vld1q_u16(BIT_WEIGHTS.as_ptr()) };
    let chunks = values.len() / 8;
    for chunk in 0..chunks {
        let row = chunk * 8;
        // SAFETY: `row..row+8` is inside `values` by construction. AArch64
        // vector loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row);
            let haystack: uint16x8_t = vld1q_u16(ptr);
            let mut matched = vdupq_n_u16(0);
            for needle in needles {
                let needle_vector = vdupq_n_u16(*needle);
                matched = vorrq_u16(matched, vceqq_u16(haystack, needle_vector));
            }
            u64::from(vaddvq_u16(vandq_u16(matched, weights)))
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u16_membership_scalar_tail(values, chunks * 8, needles, out);
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `out`
/// must have a word capacity covering `values.len()` rows and must be
/// exclusively mutable; the bitmap may already contain set bits that are ORed
/// with membership matches. Unaligned loads and scalar tail handling are
/// internal.
pub(super) unsafe fn filter_local_u32_membership_neon(
    values: &[u32],
    needles: &[u32],
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        uint32x4_t, vaddvq_u32, vandq_u32, vceqq_u32, vdupq_n_u32, vld1q_u32, vorrq_u32,
    };

    const BIT_WEIGHTS: [u32; 4] = [1, 2, 4, 8];

    // SAFETY: `BIT_WEIGHTS` is a 4-lane initialized table.
    let weights = unsafe { vld1q_u32(BIT_WEIGHTS.as_ptr()) };
    let chunks = values.len() / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `values` by construction. AArch64
        // vector loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row);
            let haystack: uint32x4_t = vld1q_u32(ptr);
            let mut matched = vdupq_n_u32(0);
            for needle in needles {
                let needle_vector = vdupq_n_u32(*needle);
                matched = vorrq_u32(matched, vceqq_u32(haystack, needle_vector));
            }
            u64::from(vaddvq_u32(vandq_u32(matched, weights)))
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u32_membership_scalar_tail(values, chunks * 4, needles, out);
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `right`
/// must contain at least `left.len()` words, and `left` must be the exclusive
/// mutable bitmap buffer being intersected. The function handles unaligned
/// vector accesses and delegates any non-vector tail to the scalar
/// implementation.
pub(super) unsafe fn intersect_words_neon(left: &mut [u64], right: &[u64]) {
    use std::arch::aarch64::{uint64x2_t, vandq_u64, vld1q_u64, vst1q_u64};

    let chunks = left.len() / 2;
    for chunk in 0..chunks {
        let offset = chunk * 2;
        // SAFETY: `offset..offset+2` is inside both slices by construction.
        // AArch64 vector loads/stores used here permit unaligned addresses.
        unsafe {
            let left_ptr = left.as_mut_ptr().add(offset);
            let right_ptr = right.as_ptr().add(offset);
            let lhs: uint64x2_t = vld1q_u64(left_ptr);
            let rhs: uint64x2_t = vld1q_u64(right_ptr);
            vst1q_u64(left_ptr, vandq_u64(lhs, rhs));
        }
    }
    let tail = chunks * 2;
    intersect_words_scalar(&mut left[tail..], &right[tail..]);
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `bytes`
/// must contain at least `row_count * 8` initialized little-endian bytes, and
/// `out` must have a word capacity covering `row_count` rows with exclusive
/// mutable access. Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_u64_le_eq_neon(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{uint64x2_t, vceqq_u64, vdupq_n_u64, vgetq_lane_u64, vld1q_u64};

    let needle_vector = vdupq_n_u64(needle);
    let chunks = row_count / 2;
    for chunk in 0..chunks {
        let row = chunk * 2;
        // SAFETY: `row..row+2` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let cmp = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<u64>();
            let values: uint64x2_t = vld1q_u64(ptr);
            vceqq_u64(values, needle_vector)
        };
        let mask = u64::from(vgetq_lane_u64::<0>(cmp) == u64::MAX)
            | (u64::from(vgetq_lane_u64::<1>(cmp) == u64::MAX) << 1);
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u64_le_eq_scalar_tail(bytes, chunks * 2, row_count, needle, out);
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `bytes`
/// must contain at least `row_count * 4` initialized little-endian bytes, and
/// `out` must have a word capacity covering `row_count` rows with exclusive
/// mutable access. Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_u32_le_eq_neon(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{uint32x4_t, vceqq_u32, vdupq_n_u32, vgetq_lane_u32, vld1q_u32};

    let needle_vector = vdupq_n_u32(needle);
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 4` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let cmp = unsafe {
            let ptr = bytes.as_ptr().add(row * 4).cast::<u32>();
            let values: uint32x4_t = vld1q_u32(ptr);
            vceqq_u32(values, needle_vector)
        };
        let mask = u64::from(vgetq_lane_u32::<0>(cmp) == u32::MAX)
            | (u64::from(vgetq_lane_u32::<1>(cmp) == u32::MAX) << 1)
            | (u64::from(vgetq_lane_u32::<2>(cmp) == u32::MAX) << 2)
            | (u64::from(vgetq_lane_u32::<3>(cmp) == u32::MAX) << 3);
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u32_le_eq_scalar_tail(bytes, chunks * 4, row_count, needle, out);
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `bytes`
/// must contain at least `row_count * 8` initialized little-endian bytes, and
/// `out` must have a word capacity covering `row_count` rows with exclusive
/// mutable access. Unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_i64_le_cmp_neon(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        int64x2_t, uint64x2_t, vceqq_s64, vcgtq_s64, vdupq_n_s64, vgetq_lane_u64, vld1q_s64,
    };

    let needle_vector = vdupq_n_s64(needle);
    let chunks = row_count / 2;
    for chunk in 0..chunks {
        let row = chunk * 2;
        // SAFETY: `row..row+2` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<i64>();
            let values: int64x2_t = vld1q_s64(ptr);
            let raw_mask = match op {
                NativeNumericPredicateOp::Eq => neon_i64_cmp_mask(vceqq_s64(values, needle_vector)),
                NativeNumericPredicateOp::Gt => neon_i64_cmp_mask(vcgtq_s64(values, needle_vector)),
                NativeNumericPredicateOp::GtEq => {
                    !neon_i64_cmp_mask(vcgtq_s64(needle_vector, values))
                }
                NativeNumericPredicateOp::Lt => neon_i64_cmp_mask(vcgtq_s64(needle_vector, values)),
                NativeNumericPredicateOp::LtEq => {
                    !neon_i64_cmp_mask(vcgtq_s64(values, needle_vector))
                }
            };
            raw_mask & 0b11
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_cmp_scalar_tail(bytes, chunks * 2, row_count, op, needle, out);

    #[inline]
    fn neon_i64_cmp_mask(cmp: uint64x2_t) -> u64 {
        u64::from(unsafe { vgetq_lane_u64::<0>(cmp) } == u64::MAX)
            | (u64::from(unsafe { vgetq_lane_u64::<1>(cmp) } == u64::MAX) << 1)
    }
}

#[cfg(target_arch = "aarch64")]
/// # Safety
///
/// AArch64 NEON is a baseline target feature for this implementation. `bytes`
/// must contain at least `row_count * 8` initialized little-endian bytes, and
/// `out` must have a word capacity covering `row_count` rows with exclusive
/// mutable access. Bounds must be normalized by the caller to the native
/// predicate semantics; unaligned loads and scalar tail handling are internal.
pub(super) unsafe fn filter_i64_le_range_neon(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        int64x2_t, uint64x2_t, vcgtq_s64, vdupq_n_s64, vgetq_lane_u64, vld1q_s64,
    };

    let lower_vector = lower.map(|(bound, inclusive)| (vdupq_n_s64(bound), inclusive));
    let upper_vector = upper.map(|(bound, inclusive)| (vdupq_n_s64(bound), inclusive));
    let chunks = row_count / 2;
    for chunk in 0..chunks {
        let row = chunk * 2;
        // SAFETY: `row..row+2` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<i64>();
            let values: int64x2_t = vld1q_s64(ptr);
            let mut mask = 0b11u64;
            if let Some((bound_vector, inclusive)) = lower_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        !neon_i64_cmp_mask(vcgtq_s64(bound_vector, values))
                    }
                    BoundInclusive::Exclusive => neon_i64_cmp_mask(vcgtq_s64(values, bound_vector)),
                };
                mask &= raw & 0b11;
            }
            if let Some((bound_vector, inclusive)) = upper_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        !neon_i64_cmp_mask(vcgtq_s64(values, bound_vector))
                    }
                    BoundInclusive::Exclusive => neon_i64_cmp_mask(vcgtq_s64(bound_vector, values)),
                };
                mask &= raw & 0b11;
            }
            mask
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_range_scalar_tail(bytes, chunks * 2, row_count, lower, upper, out);

    #[inline]
    fn neon_i64_cmp_mask(cmp: uint64x2_t) -> u64 {
        u64::from(unsafe { vgetq_lane_u64::<0>(cmp) } == u64::MAX)
            | (u64::from(unsafe { vgetq_lane_u64::<1>(cmp) } == u64::MAX) << 1)
    }
}

pub(super) fn filter_u64_le_eq_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    needle: u64,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<u64>();
        let mut value_bytes = [0u8; 8];
        value_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let value = u64::from_le_bytes(value_bytes);
        if value == needle {
            out.set(row);
        }
    }
}

pub(super) fn filter_u32_le_eq_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    needle: u32,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<u32>();
        let mut value_bytes = [0u8; 4];
        value_bytes.copy_from_slice(&bytes[offset..offset + 4]);
        let value = u32::from_le_bytes(value_bytes);
        if value == needle {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) fn filter_local_u8_membership_scalar_tail(
    values: &[u8],
    start: usize,
    needles: &[u8],
    out: &mut SelectionBitmap,
) {
    for (row, value) in values.iter().copied().enumerate().skip(start) {
        if needles.contains(&value) {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) fn filter_local_u16_membership_scalar_tail(
    values: &[u16],
    start: usize,
    needles: &[u16],
    out: &mut SelectionBitmap,
) {
    for (row, value) in values.iter().copied().enumerate().skip(start) {
        if needles.contains(&value) {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) fn filter_local_u32_membership_scalar_tail(
    values: &[u32],
    start: usize,
    needles: &[u32],
    out: &mut SelectionBitmap,
) {
    for (row, value) in values.iter().copied().enumerate().skip(start) {
        if needles.contains(&value) {
            out.set(row);
        }
    }
}

pub(super) fn filter_i64_le_cmp_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<i64>();
        let mut value_bytes = [0u8; 8];
        value_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let value = i64::from_le_bytes(value_bytes);
        if compare_i64_predicate(value, op, needle) {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) fn filter_i64_le_range_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<i64>();
        let mut value_bytes = [0u8; 8];
        value_bytes.copy_from_slice(&bytes[offset..offset + 8]);
        let value = i64::from_le_bytes(value_bytes);
        if i64_value_in_range(value, lower, upper) {
            out.set(row);
        }
    }
}

#[inline]
pub(super) fn i64_value_in_range(
    value: i64,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
) -> bool {
    if lower.is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, false)) {
        return false;
    }
    if upper.is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, true)) {
        return false;
    }
    true
}

#[inline]
pub(super) fn compare_i64_predicate(value: i64, op: NativeNumericPredicateOp, needle: i64) -> bool {
    match op {
        NativeNumericPredicateOp::Eq => value == needle,
        NativeNumericPredicateOp::Lt => value < needle,
        NativeNumericPredicateOp::LtEq => value <= needle,
        NativeNumericPredicateOp::Gt => value > needle,
        NativeNumericPredicateOp::GtEq => value >= needle,
    }
}

pub(super) fn mask_last_word(words: &mut [u64], len: usize) {
    let used = len % 64;
    if used == 0 {
        return;
    }
    if let Some(last) = words.last_mut() {
        *last &= (1u64 << used) - 1;
    }
}
