use crate::{
    checksum,
    codec::{
        materialize_registered_page_payload, CodecExtensionDescriptorV2, RegisteredCodecResolver,
        StableRegisteredCodecResolver,
    },
    compression,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, FEATURE_CODEC_LZ4,
        FEATURE_CODEC_ZSTD,
    },
    dictionary::FileDictionaryView,
    encoding::{
        bit_packed::{BitPacked, BitPackedPayload},
        constant::ConstantPayload,
        delta::{Delta, DeltaPayload},
        frame_of_reference::{ForPayload, FrameOfReference},
        local_codebook::{LocalCodebookPayload, LocalCodebookValue},
        patched_base::{PatchedBase, PatchedBasePayload},
        rle::{Rle, RlePayload},
        run_end::{RunEnd, RunEndPayload},
        sparse::{Sparse, SparsePayload},
        Encoding,
    },
    nested_schema::NestedSchemaNodeV1,
    page::{
        page_flag_codec, ColumnPageIndexEntryV1, PAGE_FLAG_ALL_NON_NULL, PAGE_FLAG_ALL_NULL,
        PAGE_FLAG_STATS_ONLY_CONSTANT, PAGE_FLAG_VALUE_STREAM_ELIDED,
    },
    page_payload::{ColumnPagePayloadV1, PageBufferKind, PagePayloadTreeNode},
    wire,
    zone_stats::{StatKind, StatScalar, ZoneStatFlags, ZoneStatsEntry},
    CoveError,
};

pub(crate) struct PageValidationContext<'a> {
    pub table_id: Option<u32>,
    pub segment_id: Option<u32>,
    pub column_id: u32,
    pub logical_type: CoveLogicalType,
    pub physical_kind: CovePhysicalKind,
    pub dictionary: Option<&'a FileDictionaryView<'a>>,
    pub zone_stats: Option<&'a [ZoneStatsEntry]>,
    pub codec_descriptors: &'a [CodecExtensionDescriptorV2],
    pub nested_schema: Option<&'a NestedSchemaNodeV1>,
}

#[derive(Debug, Clone, Copy)]
pub struct StatsOnlyPageMaterializationContext<'a> {
    pub table_id: Option<u32>,
    pub segment_id: Option<u32>,
    pub column_id: u32,
    pub logical_type: CoveLogicalType,
    pub physical_kind: CovePhysicalKind,
    pub dictionary_len: Option<u32>,
    pub zone_stats: &'a [ZoneStatsEntry],
}

pub(crate) fn page_codec_feature_bit(codec: CompressionCodec) -> u64 {
    match codec {
        CompressionCodec::None => 0,
        CompressionCodec::Lz4 => FEATURE_CODEC_LZ4,
        CompressionCodec::Zstd => FEATURE_CODEC_ZSTD,
    }
}

pub(crate) fn validate_page_codec_feature_advertisement(
    page: &ColumnPageIndexEntryV1,
    file_advertised_features: Option<u64>,
    section_advertised_features: Option<u64>,
) -> Result<(), CoveError> {
    let required_feature = page_codec_feature_bit(page_flag_codec(page.flags)?);
    if required_feature == 0 {
        return Ok(());
    }
    if file_advertised_features.is_some_and(|features| features & required_feature == 0)
        || section_advertised_features.is_some_and(|features| features & required_feature == 0)
    {
        return Err(CoveError::BadSection(format!(
            "page codec requires missing feature bit 0x{required_feature:016x}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_column_page_wire(
    context: &PageValidationContext<'_>,
    page: &ColumnPageIndexEntryV1,
    page_wire: &[u8],
) -> Result<(), CoveError> {
    let payload = compression::column_page_payload(page_wire, page)?;
    let payload = ColumnPagePayloadV1::parse(payload.as_ref())?;
    validate_column_page_payload(context, page, &payload)
}

pub(crate) fn validate_column_page_payload(
    context: &PageValidationContext<'_>,
    page: &ColumnPageIndexEntryV1,
    payload: &ColumnPagePayloadV1,
) -> Result<(), CoveError> {
    validate_column_page_payload_with_registered_codecs(
        context,
        page,
        payload,
        &StableRegisteredCodecResolver,
    )
}

pub(crate) fn validate_column_page_payload_with_registered_codecs<
    R: RegisteredCodecResolver + ?Sized,
>(
    context: &PageValidationContext<'_>,
    page: &ColumnPageIndexEntryV1,
    payload: &ColumnPagePayloadV1,
    resolver: &R,
) -> Result<(), CoveError> {
    if page.flags & PAGE_FLAG_STATS_ONLY_CONSTANT != 0 {
        return validate_stats_only_constant_page(context, page);
    }

    let root = payload.root_node()?;
    if root.encoding_kind == CoveEncodingKind::RegisteredEncoding {
        let materialized = materialize_registered_page_payload(
            payload,
            page,
            context.logical_type,
            context.physical_kind,
            context.codec_descriptors,
            resolver,
            context.dictionary.map(|dictionary| dictionary.len()),
        )?
        .ok_or(CoveError::BadCodecExtension)?;
        let mut materialized_page = page.clone();
        materialized_page.encoding_root = materialized.payload.root_node()?.encoding_kind as u32;
        return validate_column_page_payload_with_registered_codecs(
            context,
            &materialized_page,
            &materialized.payload,
            resolver,
        );
    }
    if root.logical_type != context.logical_type
        || root.physical_kind != context.physical_kind
        || root.logical_len != page.row_count
        || page.encoding_root != root.encoding_kind as u32
    {
        return Err(CoveError::PageCorrupt);
    }

    let tree = payload.tree()?;
    if let Some(schema) = context.nested_schema {
        validate_tree_matches_nested_schema(&tree, schema)?;
    }
    validate_tree_null_bitmap(payload, &tree, page.row_count, Some(page.null_count))?;
    if page.flags & PAGE_FLAG_VALUE_STREAM_ELIDED != 0 {
        validate_value_stream_elided_page(context, page, root.encoding_kind)?;
    }

    match context.physical_kind {
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            validate_nested_tree(context, payload, &tree)?;
        }
        _ => {
            let values = tree_buffer_bytes(payload, &tree, PageBufferKind::Values)?
                .ok_or(CoveError::PageCorrupt)?;
            validate_values_buffer(
                context.logical_type,
                context.physical_kind,
                root.encoding_kind,
                page.row_count,
                values,
                context.dictionary,
            )?;
        }
    }

    Ok(())
}

fn validate_nested_tree(
    context: &PageValidationContext<'_>,
    payload: &ColumnPagePayloadV1,
    tree: &PagePayloadTreeNode<'_>,
) -> Result<(), CoveError> {
    validate_nested_node(context, payload, tree, context.nested_schema)
}

fn validate_nested_node(
    context: &PageValidationContext<'_>,
    payload: &ColumnPagePayloadV1,
    tree: &PagePayloadTreeNode<'_>,
    schema: Option<&NestedSchemaNodeV1>,
) -> Result<(), CoveError> {
    match tree.node.physical_kind {
        CovePhysicalKind::List => {
            if tree.children.len() != 1 {
                return Err(CoveError::PageCorrupt);
            }
            let layout_bytes = tree_buffer_bytes(payload, tree, PageBufferKind::ChildLayout)?
                .ok_or(CoveError::PageCorrupt)?;
            let layout = crate::encoding::nested::ListLayoutPayload::parse(layout_bytes)?;
            layout.validate()?;
            if layout.layout.row_count() != tree.node.logical_len as usize {
                return Err(CoveError::PageCorrupt);
            }
            if let Some(schema) = schema {
                if schema.fixed_size_list_len != 0 {
                    let width = schema.fixed_size_list_len;
                    for pair in layout.layout.offsets.windows(2) {
                        if pair[1]
                            .checked_sub(pair[0])
                            .ok_or(CoveError::ArithOverflow)?
                            != width
                        {
                            return Err(CoveError::PageCorrupt);
                        }
                    }
                    if layout.child_row_count
                        != tree
                            .node
                            .logical_len
                            .checked_mul(width)
                            .ok_or(CoveError::ArithOverflow)?
                    {
                        return Err(CoveError::PageCorrupt);
                    }
                }
            }
            let child = &tree.children[0];
            if child.node.logical_len != layout.child_row_count {
                return Err(CoveError::PageCorrupt);
            }
            validate_tree_null_bitmap(payload, child, child.node.logical_len, None)?;
            validate_nested_node(
                context,
                payload,
                child,
                schema.and_then(|schema| schema.children.first()),
            )
        }
        CovePhysicalKind::Struct => {
            let layout_bytes = tree_buffer_bytes(payload, tree, PageBufferKind::ChildLayout)?
                .ok_or(CoveError::PageCorrupt)?;
            let layout = crate::encoding::nested::StructLayoutPayload::parse(layout_bytes)?;
            layout.validate(u64::from(tree.node.logical_len))?;
            if tree.children.len() != layout.layout.field_row_counts.len() {
                return Err(CoveError::PageCorrupt);
            }
            for (child_index, (child, expected)) in tree
                .children
                .iter()
                .zip(&layout.layout.field_row_counts)
                .enumerate()
            {
                if child.node.logical_len as u64 != *expected
                    || child.node.logical_len != tree.node.logical_len
                {
                    return Err(CoveError::PageCorrupt);
                }
                validate_tree_null_bitmap(payload, child, child.node.logical_len, None)?;
                validate_nested_node(
                    context,
                    payload,
                    child,
                    schema.and_then(|schema| schema.children.get(child_index)),
                )?;
            }
            Ok(())
        }
        CovePhysicalKind::Map => {
            if tree.children.len() != 2 {
                return Err(CoveError::PageCorrupt);
            }
            let layout_bytes = tree_buffer_bytes(payload, tree, PageBufferKind::ChildLayout)?
                .ok_or(CoveError::PageCorrupt)?;
            let layout = crate::encoding::nested::MapLayoutPayload::parse(layout_bytes)?;
            layout.validate()?;
            if layout.layout.row_count() != tree.node.logical_len as usize {
                return Err(CoveError::PageCorrupt);
            }
            let key = &tree.children[0];
            let value = &tree.children[1];
            if matches!(
                key.node.physical_kind,
                CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map
            ) || key.buffer_of_kind(PageBufferKind::NullBitmap)?.is_some()
                || key.node.logical_len != layout.layout.key_row_count
                || value.node.logical_len != layout.layout.value_row_count
            {
                return Err(CoveError::PageCorrupt);
            }
            validate_nested_node(
                context,
                payload,
                key,
                schema.and_then(|schema| schema.children.first()),
            )?;
            validate_tree_null_bitmap(payload, value, value.node.logical_len, None)?;
            validate_nested_node(
                context,
                payload,
                value,
                schema.and_then(|schema| schema.children.get(1)),
            )
        }
        _ => {
            if !tree.children.is_empty() {
                return Err(CoveError::PageCorrupt);
            }
            let values = tree_buffer_bytes(payload, tree, PageBufferKind::Values)?
                .ok_or(CoveError::PageCorrupt)?;
            validate_values_buffer(
                tree.node.logical_type,
                tree.node.physical_kind,
                tree.node.encoding_kind,
                tree.node.logical_len,
                values,
                context.dictionary,
            )
        }
    }
}

fn validate_tree_matches_nested_schema(
    tree: &PagePayloadTreeNode<'_>,
    schema: &NestedSchemaNodeV1,
) -> Result<(), CoveError> {
    if tree.node.logical_type != schema.logical
        || tree.node.physical_kind != schema.physical
        || tree.children.len() != schema.children.len()
    {
        return Err(CoveError::PageCorrupt);
    }
    for (child_tree, child_schema) in tree.children.iter().zip(&schema.children) {
        validate_tree_matches_nested_schema(child_tree, child_schema)?;
    }
    Ok(())
}

fn validate_value_stream_elided_page(
    context: &PageValidationContext<'_>,
    page: &ColumnPageIndexEntryV1,
    encoding_kind: CoveEncodingKind,
) -> Result<(), CoveError> {
    if page.non_null_count == 0 || encoding_kind != CoveEncodingKind::Constant {
        return Err(CoveError::PageCorrupt);
    }
    match context.physical_kind {
        CovePhysicalKind::Boolean | CovePhysicalKind::FileCode | CovePhysicalKind::NumCode => {
            Ok(())
        }
        _ => Err(CoveError::PageCorrupt),
    }
}

pub(crate) fn validate_stats_only_constant_page(
    context: &PageValidationContext<'_>,
    page: &ColumnPageIndexEntryV1,
) -> Result<(), CoveError> {
    validate_stats_only_constant_page_envelope(page)?;

    if page.flags & PAGE_FLAG_ALL_NULL != 0 {
        validate_stats_only_all_null_reconstruction_source(context.physical_kind)?;
        return Ok(());
    }

    validate_stats_only_all_non_null_reconstruction_source(context.physical_kind)?;
    let Some(zone_stats) = context.zone_stats else {
        return Err(CoveError::PageCorrupt);
    };
    validate_stats_only_constant_stat(
        StatsOnlyPageMaterializationContext {
            table_id: context.table_id,
            segment_id: context.segment_id,
            column_id: context.column_id,
            logical_type: context.logical_type,
            physical_kind: context.physical_kind,
            dictionary_len: context.dictionary.map(|dictionary| dictionary.len()),
            zone_stats,
        },
        page,
    )?;
    Ok(())
}

pub(crate) fn validate_stats_only_constant_page_envelope(
    page: &ColumnPageIndexEntryV1,
) -> Result<(), CoveError> {
    if page_flag_codec(page.flags)? != CompressionCodec::None
        || page.page_offset != 0
        || page.page_length != 0
        || page.uncompressed_length != 0
        || page.encoding_root != u32::MAX
        || page.checksum != checksum::crc32c(&[])
    {
        return Err(CoveError::PageCorrupt);
    }

    if page.flags & PAGE_FLAG_ALL_NON_NULL == 0 {
        return if page.flags & PAGE_FLAG_ALL_NULL != 0 {
            Ok(())
        } else {
            Err(CoveError::PageCorrupt)
        };
    }

    Ok(())
}

pub fn materialize_stats_only_constant_page_payload(
    context: StatsOnlyPageMaterializationContext<'_>,
    page: &ColumnPageIndexEntryV1,
) -> Result<Vec<u8>, CoveError> {
    if page_flag_codec(page.flags)? != CompressionCodec::None
        || page.page_offset != 0
        || page.page_length != 0
        || page.uncompressed_length != 0
        || page.encoding_root != u32::MAX
        || page.checksum != checksum::crc32c(&[])
    {
        return Err(CoveError::PageCorrupt);
    }

    if page.flags & PAGE_FLAG_ALL_NON_NULL != 0 {
        validate_stats_only_all_non_null_reconstruction_source(context.physical_kind)?;
        let scalar = validate_stats_only_constant_stat(context, page)?;
        let values = stats_only_constant_values(context, page, scalar)?;
        return ColumnPagePayloadV1::build_single_node(
            page.row_count,
            stats_only_materialized_encoding(context.physical_kind),
            context.logical_type,
            context.physical_kind,
            None,
            values,
        );
    }

    if page.flags & PAGE_FLAG_ALL_NULL != 0 {
        let bitmap_len = bitmap_len(page.row_count)?;
        let mut bitmap = vec![0xff; bitmap_len];
        if !page.row_count.is_multiple_of(8) && !bitmap.is_empty() {
            let valid_bits = page.row_count % 8;
            bitmap[bitmap_len - 1] = (1u8 << valid_bits) - 1;
        }
        let values = stats_only_all_null_values(context, page)?;
        return ColumnPagePayloadV1::build_single_node(
            page.row_count,
            stats_only_materialized_encoding(context.physical_kind),
            context.logical_type,
            context.physical_kind,
            Some(bitmap),
            values,
        );
    }

    Err(CoveError::PageCorrupt)
}

fn validate_stats_only_all_non_null_reconstruction_source(
    physical_kind: CovePhysicalKind,
) -> Result<(), CoveError> {
    match physical_kind {
        CovePhysicalKind::Boolean
        | CovePhysicalKind::FileCode
        | CovePhysicalKind::NumCode
        | CovePhysicalKind::FixedBytes
        | CovePhysicalKind::VarBytes => Ok(()),
        _ => Err(CoveError::PageCorrupt),
    }
}

fn validate_stats_only_all_null_reconstruction_source(
    physical_kind: CovePhysicalKind,
) -> Result<(), CoveError> {
    match physical_kind {
        CovePhysicalKind::Boolean
        | CovePhysicalKind::FileCode
        | CovePhysicalKind::NumCode
        | CovePhysicalKind::FixedBytes
        | CovePhysicalKind::VarBytes => Ok(()),
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            Err(CoveError::PageCorrupt)
        }
    }
}

fn validate_stats_only_constant_stat<'a>(
    context: StatsOnlyPageMaterializationContext<'a>,
    page: &ColumnPageIndexEntryV1,
) -> Result<&'a StatScalar, CoveError> {
    // ZoneStatsEntry has no encoded page scope. A stats entry becomes
    // decode-required page data only through this stats_ref selection plus the
    // table/segment/morsel/column/count checks below.
    let entry = context
        .zone_stats
        .get(usize::try_from(page.stats_ref).map_err(|_| CoveError::ArithOverflow)?)
        .ok_or(CoveError::PageCorrupt)?;
    if let Some(table_id) = context.table_id {
        if entry.table_id != table_id {
            return Err(CoveError::PageCorrupt);
        }
    }
    if let Some(segment_id) = context.segment_id {
        if entry.segment_id != segment_id {
            return Err(CoveError::PageCorrupt);
        }
    }
    if entry.morsel_id != page.morsel_id
        || entry.column_id != context.column_id
        || entry.stats.row_count != u64::from(page.row_count)
        || entry.stats.null_count != 0
        || entry.non_null_count != page.row_count
        || page.null_count != 0
        || page.non_null_count != page.row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    if !entry.stats.flags.contains(ZoneStatFlags::CONSTANT)
        || !entry.stats.flags.contains(ZoneStatFlags::HAS_MIN_MAX)
        || entry.stats.flags.contains(ZoneStatFlags::MINMAX_TRUNCATED)
    {
        return Err(CoveError::PageCorrupt);
    }
    let (Some(min), Some(max)) = (&entry.stats.min, &entry.stats.max) else {
        return Err(CoveError::PageCorrupt);
    };
    if min.truncated || max.truncated || min != max {
        return Err(CoveError::PageCorrupt);
    }
    if context.physical_kind == CovePhysicalKind::FileCode {
        validate_stats_only_filecode_scalar(min, context.dictionary_len)?;
        return Ok(min);
    }
    if context.physical_kind == CovePhysicalKind::Boolean {
        validate_stats_only_boolean_scalar(min)?;
        return Ok(min);
    }
    if context.physical_kind == CovePhysicalKind::VarBytes {
        validate_stats_only_varbytes_scalar(context.logical_type, min)?;
        return Ok(min);
    }
    if context.physical_kind == CovePhysicalKind::FixedBytes {
        let width = fixed_width_for(context.logical_type, context.physical_kind)?;
        if min.bytes.len() != width {
            return Err(CoveError::PageCorrupt);
        }
    }
    if !stat_scalar_matches_logical(context.logical_type, min) {
        return Err(CoveError::PageCorrupt);
    }
    Ok(min)
}

fn stats_only_constant_values(
    context: StatsOnlyPageMaterializationContext<'_>,
    page: &ColumnPageIndexEntryV1,
    scalar: &StatScalar,
) -> Result<Vec<u8>, CoveError> {
    match context.physical_kind {
        CovePhysicalKind::NumCode => {
            let raw = stats_only_numcode_bits(context.logical_type, scalar)?;
            let payload = ConstantPayload {
                value: i64::from_le_bytes(raw.to_le_bytes()),
                row_count: u64::from(page.row_count),
            };
            Ok(payload.encode().to_vec())
        }
        CovePhysicalKind::FixedBytes => {
            let width = fixed_width_for(context.logical_type, context.physical_kind)?;
            if scalar.bytes.len() != width {
                return Err(CoveError::PageCorrupt);
            }
            let row_count =
                usize::try_from(page.row_count).map_err(|_| CoveError::ArithOverflow)?;
            let mut values = Vec::with_capacity(
                row_count
                    .checked_mul(width)
                    .ok_or(CoveError::ArithOverflow)?,
            );
            for _ in 0..row_count {
                values.extend_from_slice(&scalar.bytes);
            }
            Ok(values)
        }
        CovePhysicalKind::Boolean => {
            let value = validate_stats_only_boolean_scalar(scalar)?;
            let row_count =
                usize::try_from(page.row_count).map_err(|_| CoveError::ArithOverflow)?;
            Ok(vec![value; row_count])
        }
        CovePhysicalKind::VarBytes => {
            let bytes = validate_stats_only_varbytes_scalar(context.logical_type, scalar)?;
            let len = u32::try_from(bytes.len()).map_err(|_| CoveError::ArithOverflow)?;
            let row_count =
                usize::try_from(page.row_count).map_err(|_| CoveError::ArithOverflow)?;
            let row_width = 4usize
                .checked_add(bytes.len())
                .ok_or(CoveError::ArithOverflow)?;
            let mut values = Vec::with_capacity(
                row_count
                    .checked_mul(row_width)
                    .ok_or(CoveError::ArithOverflow)?,
            );
            for _ in 0..row_count {
                values.extend_from_slice(&len.to_le_bytes());
                values.extend_from_slice(bytes);
            }
            Ok(values)
        }
        CovePhysicalKind::FileCode => {
            let code = validate_stats_only_filecode_scalar(scalar, context.dictionary_len)?;
            let row_count =
                usize::try_from(page.row_count).map_err(|_| CoveError::ArithOverflow)?;
            let mut values =
                Vec::with_capacity(row_count.checked_mul(4).ok_or(CoveError::ArithOverflow)?);
            for _ in 0..row_count {
                values.extend_from_slice(&code.to_le_bytes());
            }
            Ok(values)
        }
        _ => Err(CoveError::PageCorrupt),
    }
}

fn validate_stats_only_boolean_scalar(scalar: &StatScalar) -> Result<u8, CoveError> {
    if scalar.kind != StatKind::UInt64 || scalar.bytes.len() != 8 {
        return Err(CoveError::PageCorrupt);
    }
    match u64::from_le_bytes(
        scalar
            .bytes
            .as_slice()
            .try_into()
            .map_err(|_| CoveError::PageCorrupt)?,
    ) {
        0 => Ok(0),
        1 => Ok(1),
        _ => Err(CoveError::PageCorrupt),
    }
}

fn validate_stats_only_varbytes_scalar(
    logical_type: CoveLogicalType,
    scalar: &StatScalar,
) -> Result<&[u8], CoveError> {
    if scalar.kind != StatKind::FixedBytes {
        return Err(CoveError::PageCorrupt);
    }
    match logical_type {
        CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json => {
            validate_varbytes_logical_value(logical_type, &scalar.bytes)?;
            Ok(&scalar.bytes)
        }
        _ => Err(CoveError::PageCorrupt),
    }
}

fn validate_stats_only_filecode_scalar(
    scalar: &StatScalar,
    dictionary_len: Option<u32>,
) -> Result<u32, CoveError> {
    if scalar.kind != StatKind::UInt64 || scalar.bytes.len() != 8 {
        return Err(CoveError::PageCorrupt);
    }
    let raw = u64::from_le_bytes(
        scalar
            .bytes
            .as_slice()
            .try_into()
            .map_err(|_| CoveError::PageCorrupt)?,
    );
    let code = u32::try_from(raw).map_err(|_| CoveError::BadFileCode)?;
    let Some(dictionary_len) = dictionary_len else {
        return Err(CoveError::BadFileCode);
    };
    if code >= dictionary_len {
        return Err(CoveError::BadFileCode);
    }
    Ok(code)
}

fn stats_only_all_null_values(
    context: StatsOnlyPageMaterializationContext<'_>,
    page: &ColumnPageIndexEntryV1,
) -> Result<Vec<u8>, CoveError> {
    let row_count = usize::try_from(page.row_count).map_err(|_| CoveError::ArithOverflow)?;
    match context.physical_kind {
        CovePhysicalKind::NumCode => {
            let payload = ConstantPayload {
                value: 0,
                row_count: u64::from(page.row_count),
            };
            Ok(payload.encode().to_vec())
        }
        CovePhysicalKind::Boolean
        | CovePhysicalKind::FileCode
        | CovePhysicalKind::FixedBytes
        | CovePhysicalKind::VarBytes => {
            let width = match context.physical_kind {
                CovePhysicalKind::VarBytes => 4,
                _ => fixed_width_for(context.logical_type, context.physical_kind)?,
            };
            Ok(vec![
                0;
                row_count
                    .checked_mul(width)
                    .ok_or(CoveError::ArithOverflow)?
            ])
        }
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            Err(CoveError::PageCorrupt)
        }
    }
}

fn stats_only_numcode_bits(
    logical_type: CoveLogicalType,
    scalar: &StatScalar,
) -> Result<u64, CoveError> {
    match (logical_type, scalar.kind) {
        (
            CoveLogicalType::Int8
            | CoveLogicalType::Int16
            | CoveLogicalType::Int32
            | CoveLogicalType::Int64
            | CoveLogicalType::Decimal64,
            StatKind::Int64,
        )
        | (CoveLogicalType::TimestampMicros, StatKind::TimestampMicros)
        | (CoveLogicalType::TimestampNanos, StatKind::TimestampNanos) => {
            if scalar.bytes.len() != 8 {
                return Err(CoveError::PageCorrupt);
            }
            Ok(u64::from_le_bytes(scalar.bytes[..8].try_into().unwrap()))
        }
        (
            CoveLogicalType::UInt8
            | CoveLogicalType::UInt16
            | CoveLogicalType::UInt32
            | CoveLogicalType::UInt64,
            StatKind::UInt64,
        )
        | (CoveLogicalType::Float64, StatKind::Float64Bits) => {
            if scalar.bytes.len() != 8 {
                return Err(CoveError::PageCorrupt);
            }
            Ok(u64::from_le_bytes(scalar.bytes[..8].try_into().unwrap()))
        }
        (CoveLogicalType::Float32, StatKind::FixedBytes) => {
            if scalar.bytes.len() != 4 {
                return Err(CoveError::PageCorrupt);
            }
            Ok(u64::from(u32::from_le_bytes(
                scalar.bytes[..4].try_into().unwrap(),
            )))
        }
        (CoveLogicalType::DateDays, StatKind::DateDays) => {
            if scalar.bytes.len() != 4 {
                return Err(CoveError::PageCorrupt);
            }
            let value = i32::from_le_bytes(scalar.bytes[..4].try_into().unwrap());
            Ok(value as u64)
        }
        _ => Err(CoveError::PageCorrupt),
    }
}

fn stats_only_materialized_encoding(physical_kind: CovePhysicalKind) -> CoveEncodingKind {
    match physical_kind {
        CovePhysicalKind::FileCode => CoveEncodingKind::FileCode,
        CovePhysicalKind::NumCode => CoveEncodingKind::Constant,
        CovePhysicalKind::Boolean | CovePhysicalKind::FixedBytes => CoveEncodingKind::PlainFixed,
        CovePhysicalKind::VarBytes => CoveEncodingKind::VarBytes,
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            CoveEncodingKind::Canonical
        }
    }
}

fn validate_tree_null_bitmap(
    payload: &ColumnPagePayloadV1,
    tree: &PagePayloadTreeNode<'_>,
    row_count: u32,
    expected_null_count: Option<u32>,
) -> Result<(), CoveError> {
    let null_bitmap = tree_buffer_bytes(payload, tree, PageBufferKind::NullBitmap)?;
    if expected_null_count.is_some_and(|count| count != 0) && null_bitmap.is_none() {
        return Err(CoveError::PageCorrupt);
    }
    let Some(bytes) = null_bitmap else {
        return Ok(());
    };
    let expected_len = bitmap_len(row_count)?;
    if bytes.len() != expected_len {
        return Err(CoveError::PageCorrupt);
    }
    if !row_count.is_multiple_of(8) && expected_len != 0 {
        let valid_bits = row_count % 8;
        let mask = (1u8 << valid_bits) - 1;
        if bytes[expected_len - 1] & !mask != 0 {
            return Err(CoveError::PageCorrupt);
        }
    }
    let mut counted = 0u32;
    for byte in bytes {
        counted = counted
            .checked_add(byte.count_ones())
            .ok_or(CoveError::ArithOverflow)?;
    }
    if expected_null_count.is_some_and(|expected| counted != expected) {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

fn tree_buffer_bytes<'a>(
    payload: &'a ColumnPagePayloadV1,
    tree: &PagePayloadTreeNode<'_>,
    kind: PageBufferKind,
) -> Result<Option<&'a [u8]>, CoveError> {
    tree.buffer_of_kind(kind)?
        .map(|descriptor| payload.buffer_bytes_for_descriptor(descriptor))
        .transpose()
}

fn validate_values_buffer(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    encoding_kind: CoveEncodingKind,
    row_count: u32,
    values: &[u8],
    dictionary: Option<&FileDictionaryView<'_>>,
) -> Result<(), CoveError> {
    match encoding_kind {
        CoveEncodingKind::FileCode => validate_filecodes(values, row_count, dictionary),
        CoveEncodingKind::NumCode => validate_numcode_values(logical_type, values, row_count),
        CoveEncodingKind::PlainFixed => {
            let width = fixed_width_for(logical_type, physical_kind)?;
            require_len(values.len(), fixed_rows_len(row_count, width)?)?;
            if physical_kind == CovePhysicalKind::Boolean {
                validate_boolean_bytes(values)?;
            }
            if physical_kind == CovePhysicalKind::NumCode {
                validate_numcode_values(logical_type, values, row_count)?;
            }
            Ok(())
        }
        CoveEncodingKind::VarBytes => {
            validate_length_prefixed_u32_rows(logical_type, values, row_count)
        }
        CoveEncodingKind::PlainVarint => {
            validate_plain_varint_values(logical_type, physical_kind, values, row_count, dictionary)
        }
        CoveEncodingKind::Canonical => validate_canonical_rows(values, logical_type, row_count),
        CoveEncodingKind::Constant => {
            require_len(values.len(), ConstantPayload::ENCODED_LEN)?;
            let payload = ConstantPayload::parse(values)?;
            if payload.row_count != u64::from(row_count) {
                return Err(CoveError::PageCorrupt);
            }
            if physical_kind == CovePhysicalKind::Boolean && !matches!(payload.value, 0 | 1) {
                return Err(CoveError::PageCorrupt);
            }
            if physical_kind == CovePhysicalKind::FileCode {
                let code = u32::try_from(payload.value).map_err(|_| CoveError::PageCorrupt)?;
                validate_filecode_value(code, dictionary)?;
            }
            if physical_kind == CovePhysicalKind::NumCode {
                validate_numcode(logical_type, physical_kind, payload.raw_value_bits())?;
            }
            Ok(())
        }
        CoveEncodingKind::LocalCodebook => {
            let payload = LocalCodebookPayload::parse(values)?;
            require_len(values.len(), payload.encode().len())?;
            let decoded = payload.decode_values()?;
            if decoded.len() != row_count as usize {
                return Err(CoveError::PageCorrupt);
            }
            validate_local_codebook_values(&decoded, logical_type, physical_kind, dictionary)
        }
        CoveEncodingKind::Rle => {
            let payload = RlePayload::parse(values)?;
            require_len(values.len(), payload.encode().len())?;
            validate_i64_values(
                Rle::fast_decode(&payload)?,
                logical_type,
                physical_kind,
                row_count,
                dictionary,
            )
        }
        CoveEncodingKind::RunEnd => {
            let payload = RunEndPayload::parse(values)?;
            let expected = 4usize
                .checked_add(
                    payload
                        .values
                        .len()
                        .checked_mul(12)
                        .ok_or(CoveError::ArithOverflow)?,
                )
                .ok_or(CoveError::ArithOverflow)?;
            require_len(values.len(), expected)?;
            validate_i64_values(
                RunEnd::fast_decode(&payload)?,
                logical_type,
                physical_kind,
                row_count,
                dictionary,
            )
        }
        CoveEncodingKind::BitPacked => {
            let payload = BitPackedPayload::parse(values)?;
            let expected = 9usize
                .checked_add(payload.bits.len())
                .ok_or(CoveError::ArithOverflow)?;
            require_len(values.len(), expected)?;
            if payload.row_count != row_count {
                return Err(CoveError::PageCorrupt);
            }
            validate_i64_values(
                BitPacked::fast_decode(&payload)?,
                logical_type,
                physical_kind,
                row_count,
                dictionary,
            )
        }
        CoveEncodingKind::Delta => {
            let payload = DeltaPayload::parse(values)?;
            require_len(values.len(), payload.encode().len())?;
            validate_i64_values(
                Delta::fast_decode(&payload)?,
                logical_type,
                physical_kind,
                row_count,
                dictionary,
            )
        }
        CoveEncodingKind::FrameOfReference => {
            let payload = ForPayload::parse(values)?;
            require_len(values.len(), payload.encode().len())?;
            validate_i64_values(
                FrameOfReference::fast_decode(&payload)?,
                logical_type,
                physical_kind,
                row_count,
                dictionary,
            )
        }
        CoveEncodingKind::PatchedBase => {
            let payload = PatchedBasePayload::parse(values)?;
            let expected = 4usize
                .checked_add(
                    payload
                        .base
                        .len()
                        .checked_mul(8)
                        .ok_or(CoveError::ArithOverflow)?,
                )
                .and_then(|value| value.checked_add(4))
                .and_then(|value| {
                    payload
                        .patches
                        .len()
                        .checked_mul(12)
                        .and_then(|patch_len| value.checked_add(patch_len))
                })
                .ok_or(CoveError::ArithOverflow)?;
            require_len(values.len(), expected)?;
            validate_i64_values(
                PatchedBase::fast_decode(&payload)?,
                logical_type,
                physical_kind,
                row_count,
                dictionary,
            )
        }
        CoveEncodingKind::Sparse => {
            let payload = SparsePayload::parse(values)?;
            let expected = 16usize
                .checked_add(
                    payload
                        .overrides
                        .len()
                        .checked_mul(12)
                        .ok_or(CoveError::ArithOverflow)?,
                )
                .ok_or(CoveError::ArithOverflow)?;
            require_len(values.len(), expected)?;
            validate_i64_values(
                Sparse::fast_decode(&payload)?,
                logical_type,
                physical_kind,
                row_count,
                dictionary,
            )
        }
        CoveEncodingKind::Validity
        | CoveEncodingKind::Sequence
        | CoveEncodingKind::Lz4Block
        | CoveEncodingKind::ZstdBlock
        | CoveEncodingKind::RegisteredEncoding => {
            Err(CoveError::UnsupportedEncoding(format!("{encoding_kind:?}")))
        }
    }
}

fn validate_filecodes(
    values: &[u8],
    row_count: u32,
    dictionary: Option<&FileDictionaryView<'_>>,
) -> Result<(), CoveError> {
    require_len(values.len(), fixed_rows_len(row_count, 4)?)?;
    for chunk in values.chunks_exact(4) {
        let code = u32::from_le_bytes(chunk.try_into().unwrap());
        validate_filecode_value(code, dictionary)?;
    }
    Ok(())
}

fn validate_filecode_value(
    code: u32,
    dictionary: Option<&FileDictionaryView<'_>>,
) -> Result<(), CoveError> {
    if let Some(dictionary) = dictionary {
        if code >= dictionary.len() {
            return Err(CoveError::BadFileCode);
        }
    }
    Ok(())
}

fn validate_numcode_values(
    logical_type: CoveLogicalType,
    values: &[u8],
    row_count: u32,
) -> Result<(), CoveError> {
    require_len(values.len(), fixed_rows_len(row_count, 8)?)?;
    if logical_type != CoveLogicalType::Bool {
        return Ok(());
    }
    for chunk in values.chunks_exact(8) {
        let code = u64::from_le_bytes(chunk.try_into().unwrap());
        validate_numcode(logical_type, CovePhysicalKind::NumCode, code)?;
    }
    Ok(())
}

fn validate_numcode(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    code: u64,
) -> Result<(), CoveError> {
    if logical_type == CoveLogicalType::Bool
        && physical_kind == CovePhysicalKind::NumCode
        && !matches!(code, 0 | 1)
    {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

fn validate_i64_values(
    values: Vec<i64>,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    row_count: u32,
    dictionary: Option<&FileDictionaryView<'_>>,
) -> Result<(), CoveError> {
    if values.len() != row_count as usize {
        return Err(CoveError::PageCorrupt);
    }
    match physical_kind {
        CovePhysicalKind::Boolean if values.iter().any(|value| !matches!(*value, 0 | 1)) => {
            return Err(CoveError::PageCorrupt);
        }
        CovePhysicalKind::Boolean => {}
        CovePhysicalKind::FileCode => {
            for value in &values {
                let code = u32::try_from(*value).map_err(|_| CoveError::PageCorrupt)?;
                validate_filecode_value(code, dictionary)?;
            }
        }
        CovePhysicalKind::NumCode => {
            for value in values {
                let code = u64::try_from(value).map_err(|_| CoveError::PageCorrupt)?;
                validate_numcode(logical_type, physical_kind, code)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_local_codebook_values(
    values: &[LocalCodebookValue],
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    dictionary: Option<&FileDictionaryView<'_>>,
) -> Result<(), CoveError> {
    for value in values {
        match (physical_kind, value) {
            (CovePhysicalKind::FileCode, LocalCodebookValue::FileCode(code)) => {
                validate_filecode_value(*code, dictionary)?;
            }
            (CovePhysicalKind::NumCode, LocalCodebookValue::NumCode(code)) => {
                validate_numcode(logical_type, physical_kind, *code)?;
            }
            (CovePhysicalKind::Boolean, LocalCodebookValue::Boolean(_)) => {}
            (CovePhysicalKind::VarBytes, LocalCodebookValue::VarBytes(bytes)) => {
                validate_varbytes_logical_value(logical_type, bytes)?;
            }
            _ => return Err(CoveError::PageCorrupt),
        }
    }
    Ok(())
}

fn validate_length_prefixed_u32_rows(
    logical_type: CoveLogicalType,
    values: &[u8],
    row_count: u32,
) -> Result<(), CoveError> {
    let mut pos = 0usize;
    for _ in 0..row_count {
        let len_end = pos.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        if len_end > values.len() {
            return Err(CoveError::BufferTooShort);
        }
        let len = u32::from_le_bytes(values[pos..len_end].try_into().unwrap()) as usize;
        let value_end = len_end.checked_add(len).ok_or(CoveError::ArithOverflow)?;
        if value_end > values.len() {
            return Err(CoveError::BufferTooShort);
        }
        validate_varbytes_logical_value(logical_type, &values[len_end..value_end])?;
        pos = value_end;
    }
    require_len(pos, values.len())
}

fn validate_varbytes_logical_value(
    logical_type: CoveLogicalType,
    bytes: &[u8],
) -> Result<(), CoveError> {
    match logical_type {
        CoveLogicalType::Utf8 => {
            std::str::from_utf8(bytes).map_err(|_| CoveError::PageCorrupt)?;
            Ok(())
        }
        CoveLogicalType::Json => {
            serde_json::from_slice::<serde_json::Value>(bytes)
                .map_err(|_| CoveError::PageCorrupt)?;
            Ok(())
        }
        CoveLogicalType::Binary => Ok(()),
        _ => Err(CoveError::PageCorrupt),
    }
}

fn validate_plain_varint_values(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    values: &[u8],
    row_count: u32,
    dictionary: Option<&FileDictionaryView<'_>>,
) -> Result<(), CoveError> {
    let mut pos = 0usize;
    for _ in 0..row_count {
        let (value, consumed) = wire::decode_u64_leb128(&values[pos..])?;
        pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
        match physical_kind {
            CovePhysicalKind::FileCode => {
                let code = u32::try_from(value).map_err(|_| CoveError::PageCorrupt)?;
                validate_filecode_value(code, dictionary)?;
            }
            CovePhysicalKind::NumCode => {
                validate_numcode(logical_type, physical_kind, value)?;
            }
            _ => {}
        }
    }
    require_len(pos, values.len())
}

fn validate_canonical_rows(
    values: &[u8],
    logical_type: CoveLogicalType,
    row_count: u32,
) -> Result<(), CoveError> {
    match logical_type {
        CoveLogicalType::Null => require_len(values.len(), 0),
        CoveLogicalType::Bool => Err(CoveError::UnsupportedEncoding(
            "Canonical Bool rows require an explicit value-tag stream".into(),
        )),
        CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json => {
            let mut pos = 0usize;
            for _ in 0..row_count {
                let (len, consumed) = wire::decode_u64_leb128(&values[pos..])?;
                let len = usize::try_from(len).map_err(|_| CoveError::ArithOverflow)?;
                let value_start = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
                pos = pos
                    .checked_add(consumed)
                    .and_then(|value| value.checked_add(len))
                    .ok_or(CoveError::ArithOverflow)?;
                if pos > values.len() {
                    return Err(CoveError::BufferTooShort);
                }
                validate_varbytes_logical_value(logical_type, &values[value_start..pos])?;
            }
            require_len(pos, values.len())
        }
        logical => {
            let width = fixed_width_for(logical, CovePhysicalKind::FixedBytes)?;
            require_len(values.len(), fixed_rows_len(row_count, width)?)
        }
    }
}

fn validate_boolean_bytes(values: &[u8]) -> Result<(), CoveError> {
    if values.iter().any(|value| !matches!(value, 0 | 1)) {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

fn fixed_width_for(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
) -> Result<usize, CoveError> {
    match physical_kind {
        CovePhysicalKind::Boolean => Ok(1),
        CovePhysicalKind::NumCode => Ok(8),
        CovePhysicalKind::FileCode => Ok(4),
        CovePhysicalKind::FixedBytes => logical_fixed_width(logical_type).ok_or_else(|| {
            CoveError::UnsupportedEncoding(format!(
                "fixed-width page validation for {logical_type:?}"
            ))
        }),
        _ => logical_fixed_width(logical_type).ok_or_else(|| {
            CoveError::UnsupportedEncoding(format!(
                "fixed-width page validation for {logical_type:?}"
            ))
        }),
    }
}

fn logical_fixed_width(logical_type: CoveLogicalType) -> Option<usize> {
    match logical_type {
        CoveLogicalType::Bool | CoveLogicalType::Int8 | CoveLogicalType::UInt8 => Some(1),
        CoveLogicalType::Int16 | CoveLogicalType::UInt16 => Some(2),
        CoveLogicalType::Int32
        | CoveLogicalType::UInt32
        | CoveLogicalType::Float32
        | CoveLogicalType::DateDays => Some(4),
        CoveLogicalType::Int64
        | CoveLogicalType::UInt64
        | CoveLogicalType::Float64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => Some(8),
        CoveLogicalType::Decimal128 | CoveLogicalType::Uuid => Some(16),
        _ => None,
    }
}

fn fixed_rows_len(row_count: u32, width: usize) -> Result<usize, CoveError> {
    (row_count as usize)
        .checked_mul(width)
        .ok_or(CoveError::ArithOverflow)
}

fn bitmap_len(row_count: u32) -> Result<usize, CoveError> {
    let len = row_count.checked_add(7).ok_or(CoveError::ArithOverflow)? / 8;
    usize::try_from(len).map_err(|_| CoveError::ArithOverflow)
}

fn require_len(actual: usize, expected: usize) -> Result<(), CoveError> {
    if actual == expected {
        Ok(())
    } else {
        Err(CoveError::PageCorrupt)
    }
}

fn stat_scalar_matches_logical(logical_type: CoveLogicalType, scalar: &StatScalar) -> bool {
    let kind = scalar.kind;
    match logical_type {
        CoveLogicalType::Bool => kind == StatKind::UInt64,
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64
        | CoveLogicalType::Decimal64 => kind == StatKind::Int64,
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => kind == StatKind::UInt64,
        CoveLogicalType::Float32 => kind == StatKind::FixedBytes && scalar.bytes.len() == 4,
        CoveLogicalType::Float64 => kind == StatKind::Float64Bits,
        CoveLogicalType::Decimal128 => kind == StatKind::Decimal128,
        CoveLogicalType::DateDays => kind == StatKind::DateDays,
        CoveLogicalType::TimestampMicros => kind == StatKind::TimestampMicros,
        CoveLogicalType::TimestampNanos => kind == StatKind::TimestampNanos,
        CoveLogicalType::Uuid => kind == StatKind::FixedBytes,
        CoveLogicalType::Utf8 | CoveLogicalType::Binary | CoveLogicalType::Json => {
            kind == StatKind::FixedBytes
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::{CoveEncodingKind, StorageClass, ValueTag},
        dictionary::{FileDictionaryHeaderV1, FileDictionaryIndexEntryV1},
        encoding::local_codebook::{LocalCodebookValues, LocalIndexPayload},
        page::{
            PAGE_FLAG_ALL_NON_NULL, PAGE_FLAG_ALL_NULL, PAGE_FLAG_STATS_ONLY_CONSTANT,
            PAGE_FLAG_VALUE_STREAM_ELIDED,
        },
        page_payload::ColumnPagePayloadV1,
        zone_stats::{StatKind, StatScalar, ZoneScope, ZoneStats},
    };

    fn base_page(row_count: u32, encoding: CoveEncodingKind) -> ColumnPageIndexEntryV1 {
        ColumnPageIndexEntryV1 {
            column_id: 7,
            morsel_id: 0,
            row_count,
            non_null_count: row_count,
            null_count: 0,
            encoding_root: encoding as u32,
            page_offset: 0,
            page_length: 1,
            uncompressed_length: 1,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        }
    }

    fn context<'a>(
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        zone_stats: Option<&'a [ZoneStatsEntry]>,
    ) -> PageValidationContext<'a> {
        PageValidationContext {
            table_id: Some(3),
            segment_id: Some(5),
            column_id: 7,
            logical_type,
            physical_kind,
            dictionary: None,
            zone_stats,
            codec_descriptors: &[],
            nested_schema: None,
        }
    }

    fn context_with_dictionary<'a>(
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        dictionary: &'a FileDictionaryView<'a>,
    ) -> PageValidationContext<'a> {
        PageValidationContext {
            dictionary: Some(dictionary),
            ..context(logical_type, physical_kind, None)
        }
    }

    fn one_entry_dictionary_bytes() -> (Vec<u8>, Vec<u8>) {
        let header = FileDictionaryHeaderV1 {
            entry_count: 1,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        };
        let entry = FileDictionaryIndexEntryV1 {
            value_tag: ValueTag::Utf8 as u16,
            storage_class: StorageClass::Inline as u8,
            flags: 0,
            inline_len: 5,
            reserved0: [0; 3],
            inline_data: {
                let mut data = [0u8; 16];
                data[..5].copy_from_slice(b"alpha");
                data
            },
            payload_offset: 0,
            payload_length: 0,
            canonical_hash64: 0,
            reserved1: 0,
        };
        let mut index = Vec::new();
        index.extend_from_slice(&header.serialize());
        index.extend_from_slice(&entry.serialize());
        (index, Vec::new())
    }

    fn one_entry_dictionary_view<'a>(index: &'a [u8], payload: &'a [u8]) -> FileDictionaryView<'a> {
        FileDictionaryView::borrowed(index, payload).unwrap()
    }

    fn constant_numcode_payload(
        logical_type: CoveLogicalType,
        raw_value_bits: u64,
        row_count: u32,
    ) -> ColumnPagePayloadV1 {
        let values = ConstantPayload {
            value: i64::from_le_bytes(raw_value_bits.to_le_bytes()),
            row_count: u64::from(row_count),
        }
        .encode()
        .to_vec();
        let payload = ColumnPagePayloadV1::build_single_node(
            row_count,
            CoveEncodingKind::Constant,
            logical_type,
            CovePhysicalKind::NumCode,
            None,
            values,
        )
        .unwrap();
        ColumnPagePayloadV1::parse(&payload).unwrap()
    }

    #[test]
    fn rejects_plain_varint_filecode_dictionary_miss() {
        let (index, dictionary_payload) = one_entry_dictionary_bytes();
        let dictionary = one_entry_dictionary_view(&index, &dictionary_payload);
        let values = crate::wire::encode_u64_leb128(1);
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::PlainVarint,
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context_with_dictionary(
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    &dictionary,
                ),
                &base_page(1, CoveEncodingKind::PlainVarint),
                &payload,
            ),
            Err(CoveError::BadFileCode)
        );
    }

    #[test]
    fn rejects_plain_varint_bool_numcode_invalid_value() {
        let values = crate::wire::encode_u64_leb128(2);
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::PlainVarint,
            CoveLogicalType::Bool,
            CovePhysicalKind::NumCode,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Bool, CovePhysicalKind::NumCode, None),
                &base_page(1, CoveEncodingKind::PlainVarint),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn rejects_constant_filecode_dictionary_miss() {
        let (index, dictionary_payload) = one_entry_dictionary_bytes();
        let dictionary = one_entry_dictionary_view(&index, &dictionary_payload);
        let values = ConstantPayload {
            value: 1,
            row_count: 1,
        }
        .encode()
        .to_vec();
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::Constant,
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context_with_dictionary(
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    &dictionary,
                ),
                &base_page(1, CoveEncodingKind::Constant),
                &payload,
            ),
            Err(CoveError::BadFileCode)
        );
    }

    #[test]
    fn rejects_short_numcode_values() {
        let payload = ColumnPagePayloadV1::build_single_node(
            2,
            CoveEncodingKind::NumCode,
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            None,
            1u64.to_le_bytes().to_vec(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let err = validate_column_page_payload(
            &context(CoveLogicalType::UInt64, CovePhysicalKind::NumCode, None),
            &base_page(2, CoveEncodingKind::NumCode),
            &payload,
        );
        assert_eq!(err, Err(CoveError::PageCorrupt));
    }

    #[test]
    fn bool_numcode_rejects_invalid_direct_code() {
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::NumCode,
            CoveLogicalType::Bool,
            CovePhysicalKind::NumCode,
            None,
            2u64.to_le_bytes().to_vec(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let err = validate_column_page_payload(
            &context(CoveLogicalType::Bool, CovePhysicalKind::NumCode, None),
            &base_page(1, CoveEncodingKind::NumCode),
            &payload,
        );
        assert_eq!(err, Err(CoveError::PageCorrupt));
    }

    #[test]
    fn bool_numcode_rejects_invalid_transform_decoded_code() {
        let values = RlePayload { runs: vec![(2, 1)] }.encode();
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::Rle,
            CoveLogicalType::Bool,
            CovePhysicalKind::NumCode,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let err = validate_column_page_payload(
            &context(CoveLogicalType::Bool, CovePhysicalKind::NumCode, None),
            &base_page(1, CoveEncodingKind::Rle),
            &payload,
        );
        assert_eq!(err, Err(CoveError::PageCorrupt));
    }

    #[test]
    fn bool_numcode_rejects_invalid_local_codebook_code() {
        let values = LocalCodebookPayload {
            values: LocalCodebookValues::NumCode(vec![0, 2]),
            indexes: LocalIndexPayload::Rle(RlePayload { runs: vec![(1, 1)] }),
        }
        .encode();
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::LocalCodebook,
            CoveLogicalType::Bool,
            CovePhysicalKind::NumCode,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let err = validate_column_page_payload(
            &context(CoveLogicalType::Bool, CovePhysicalKind::NumCode, None),
            &base_page(1, CoveEncodingKind::LocalCodebook),
            &payload,
        );
        assert_eq!(err, Err(CoveError::PageCorrupt));
    }

    #[test]
    fn varbytes_utf8_and_json_validate_logical_payloads() {
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::VarBytes,
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            None,
            [1u32.to_le_bytes().as_slice(), &[0xff]].concat(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Utf8, CovePhysicalKind::VarBytes, None),
                &base_page(1, CoveEncodingKind::VarBytes),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );

        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::VarBytes,
            CoveLogicalType::Json,
            CovePhysicalKind::VarBytes,
            None,
            [3u32.to_le_bytes().as_slice(), b"{x}"].concat(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Json, CovePhysicalKind::VarBytes, None),
                &base_page(1, CoveEncodingKind::VarBytes),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn canonical_utf8_and_json_validate_logical_payloads() {
        let mut invalid_utf8 = crate::wire::encode_u64_leb128(1);
        invalid_utf8.push(0xff);
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::Canonical,
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            None,
            invalid_utf8,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Utf8, CovePhysicalKind::VarBytes, None),
                &base_page(1, CoveEncodingKind::Canonical),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );

        let mut invalid_json = crate::wire::encode_u64_leb128(3);
        invalid_json.extend_from_slice(b"{x}");
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::Canonical,
            CoveLogicalType::Json,
            CovePhysicalKind::VarBytes,
            None,
            invalid_json,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Json, CovePhysicalKind::VarBytes, None),
                &base_page(1, CoveEncodingKind::Canonical),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn canonical_bool_rows_are_unsupported_without_value_tags() {
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::Canonical,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            None,
            Vec::new(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert!(validate_column_page_payload(
            &context(CoveLogicalType::Bool, CovePhysicalKind::Boolean, None),
            &base_page(1, CoveEncodingKind::Canonical),
            &payload,
        )
        .is_err());
    }

    #[test]
    fn local_codebook_varbytes_validates_logical_payloads() {
        let values = LocalCodebookPayload {
            values: LocalCodebookValues::VarBytes(vec![b"{x}".to_vec()]),
            indexes: LocalIndexPayload::Rle(RlePayload { runs: vec![(0, 1)] }),
        }
        .encode();
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::LocalCodebook,
            CoveLogicalType::Json,
            CovePhysicalKind::VarBytes,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Json, CovePhysicalKind::VarBytes, None),
                &base_page(1, CoveEncodingKind::LocalCodebook),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn plain_fixed_numcode_uses_eight_byte_physical_width() {
        let payload = ColumnPagePayloadV1::build_single_node(
            2,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Int8,
            CovePhysicalKind::NumCode,
            None,
            [(-1i64 as u64).to_le_bytes(), 7u64.to_le_bytes()].concat(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Int8, CovePhysicalKind::NumCode, None),
                &base_page(2, CoveEncodingKind::PlainFixed),
                &payload,
            ),
            Ok(())
        );
    }

    #[test]
    fn plain_fixed_numcode_rejects_logical_width_buffer() {
        let payload = ColumnPagePayloadV1::build_single_node(
            2,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Float32,
            CovePhysicalKind::NumCode,
            None,
            [
                1.0f32.to_bits().to_le_bytes(),
                2.0f32.to_bits().to_le_bytes(),
            ]
            .concat(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Float32, CovePhysicalKind::NumCode, None),
                &base_page(2, CoveEncodingKind::PlainFixed),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn rejects_invalid_boolean_byte() {
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            None,
            vec![2],
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let err = validate_column_page_payload(
            &context(CoveLogicalType::Bool, CovePhysicalKind::Boolean, None),
            &base_page(1, CoveEncodingKind::PlainFixed),
            &payload,
        );
        assert_eq!(err, Err(CoveError::PageCorrupt));
    }

    #[test]
    fn rejects_null_bitmap_tail_bits() {
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            Some(vec![0b1000_0001]),
            vec![0],
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let mut page = base_page(1, CoveEncodingKind::PlainFixed);
        page.non_null_count = 0;
        page.null_count = 1;
        let err = validate_column_page_payload(
            &context(CoveLogicalType::Bool, CovePhysicalKind::Boolean, None),
            &page,
            &payload,
        );
        assert_eq!(err, Err(CoveError::PageCorrupt));
    }

    #[test]
    fn accepts_explicit_all_zero_null_bitmap_when_null_count_is_zero() {
        let payload = ColumnPagePayloadV1::build_single_node(
            8,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            Some(vec![0]),
            vec![0; 8],
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let page = base_page(8, CoveEncodingKind::PlainFixed);
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Bool, CovePhysicalKind::Boolean, None),
                &page,
                &payload,
            ),
            Ok(())
        );
    }

    #[test]
    fn rejects_explicit_null_bitmap_with_set_bit_when_null_count_is_zero() {
        let payload = ColumnPagePayloadV1::build_single_node(
            8,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            Some(vec![0b0000_0001]),
            vec![0; 8],
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let page = base_page(8, CoveEncodingKind::PlainFixed);
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Bool, CovePhysicalKind::Boolean, None),
                &page,
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn rejects_explicit_zero_null_count_bitmap_with_unused_high_bits() {
        let payload = ColumnPagePayloadV1::build_single_node(
            1,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            Some(vec![0b1000_0000]),
            vec![0],
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let page = base_page(1, CoveEncodingKind::PlainFixed);
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Bool, CovePhysicalKind::Boolean, None),
                &page,
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn validates_page_codec_feature_advertisement() {
        let none_page = base_page(1, CoveEncodingKind::NumCode);
        assert_eq!(
            validate_page_codec_feature_advertisement(&none_page, Some(0), Some(0)),
            Ok(())
        );

        let mut lz4_page = base_page(1, CoveEncodingKind::NumCode);
        lz4_page.flags = CompressionCodec::Lz4 as u32;
        assert_eq!(
            validate_page_codec_feature_advertisement(
                &lz4_page,
                Some(FEATURE_CODEC_LZ4),
                Some(FEATURE_CODEC_LZ4),
            ),
            Ok(())
        );
        assert!(matches!(
            validate_page_codec_feature_advertisement(&lz4_page, Some(0), Some(FEATURE_CODEC_LZ4)),
            Err(CoveError::BadSection(_))
        ));
        assert!(matches!(
            validate_page_codec_feature_advertisement(&lz4_page, Some(FEATURE_CODEC_LZ4), Some(0)),
            Err(CoveError::BadSection(_))
        ));

        let mut zstd_page = base_page(1, CoveEncodingKind::NumCode);
        zstd_page.flags = CompressionCodec::Zstd as u32;
        assert_eq!(
            validate_page_codec_feature_advertisement(
                &zstd_page,
                Some(FEATURE_CODEC_ZSTD),
                Some(FEATURE_CODEC_ZSTD),
            ),
            Ok(())
        );
    }

    #[test]
    fn accepts_mixed_value_stream_elided_constant_numcode() {
        let values = ConstantPayload {
            value: 42,
            row_count: 4,
        }
        .encode()
        .to_vec();
        let payload = ColumnPagePayloadV1::build_single_node(
            4,
            CoveEncodingKind::Constant,
            CoveLogicalType::Int64,
            CovePhysicalKind::NumCode,
            Some(vec![0b0000_1010]),
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let mut page = base_page(4, CoveEncodingKind::Constant);
        page.non_null_count = 2;
        page.null_count = 2;
        page.flags = PAGE_FLAG_VALUE_STREAM_ELIDED;
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Int64, CovePhysicalKind::NumCode, None),
                &page,
                &payload,
            ),
            Ok(())
        );
    }

    #[test]
    fn constant_numcode_preserves_raw_high_bits_for_signed_logicals() {
        for (logical, raw) in [
            (CoveLogicalType::Int64, (-1i64 as u64)),
            (CoveLogicalType::TimestampNanos, (-123i64 as u64)),
            (CoveLogicalType::Decimal64, (-456i64 as u64)),
            (CoveLogicalType::Float64, (-2.5f64).to_bits()),
        ] {
            let payload = constant_numcode_payload(logical, raw, 2);
            assert_eq!(
                validate_column_page_payload(
                    &context(logical, CovePhysicalKind::NumCode, None),
                    &base_page(2, CoveEncodingKind::Constant),
                    &payload,
                ),
                Ok(()),
                "{logical:?}"
            );
        }
    }

    #[test]
    fn constant_numcode_accepts_uint64_above_i64_max() {
        let payload = constant_numcode_payload(CoveLogicalType::UInt64, i64::MAX as u64 + 1, 2);
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::UInt64, CovePhysicalKind::NumCode, None),
                &base_page(2, CoveEncodingKind::Constant),
                &payload,
            ),
            Ok(())
        );
    }

    #[test]
    fn constant_bool_numcode_still_rejects_non_bool_code() {
        let payload = constant_numcode_payload(CoveLogicalType::Bool, 2, 1);
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Bool, CovePhysicalKind::NumCode, None),
                &base_page(1, CoveEncodingKind::Constant),
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn rejects_value_stream_elided_non_constant_root() {
        let payload = ColumnPagePayloadV1::build_single_node(
            2,
            CoveEncodingKind::NumCode,
            CoveLogicalType::Int64,
            CovePhysicalKind::NumCode,
            None,
            [1u64.to_le_bytes(), 1u64.to_le_bytes()].concat(),
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.flags = PAGE_FLAG_VALUE_STREAM_ELIDED;
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Int64, CovePhysicalKind::NumCode, None),
                &page,
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn rejects_all_null_value_stream_elided_page() {
        let values = ConstantPayload {
            value: 1,
            row_count: 2,
        }
        .encode()
        .to_vec();
        let payload = ColumnPagePayloadV1::build_single_node(
            2,
            CoveEncodingKind::Constant,
            CoveLogicalType::Int64,
            CovePhysicalKind::NumCode,
            Some(vec![0b0000_0011]),
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let mut page = base_page(2, CoveEncodingKind::Constant);
        page.non_null_count = 0;
        page.null_count = 2;
        page.flags = PAGE_FLAG_VALUE_STREAM_ELIDED;
        assert_eq!(
            validate_column_page_payload(
                &context(CoveLogicalType::Int64, CovePhysicalKind::NumCode, None),
                &page,
                &payload,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn validates_stats_only_all_non_null_constant() {
        let scalar = StatScalar {
            kind: StatKind::Int64,
            bytes: 9i64.to_le_bytes().to_vec(),
            truncated: false,
        };
        let stats = ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 2,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 2,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        };
        let stats = [stats];
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);
        assert_eq!(
            validate_stats_only_constant_page(
                &context(
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    Some(&stats)
                ),
                &page,
            ),
            Ok(())
        );
        let mut bad = stats[0].clone();
        bad.stats.flags = ZoneStatFlags::HAS_MIN_MAX;
        assert_eq!(
            validate_stats_only_constant_page(
                &context(
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    Some(&[bad])
                ),
                &page,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn validates_stats_only_all_non_null_filecode_reconstruction() {
        let scalar = StatScalar {
            kind: StatKind::UInt64,
            bytes: 0u64.to_le_bytes().to_vec(),
            truncated: false,
        };
        let stats = [ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 2,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 2,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        }];
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        let (dictionary_bytes, payload) = one_entry_dictionary_bytes();
        let dictionary = FileDictionaryView::borrowed(&dictionary_bytes, &payload).unwrap();
        assert_eq!(
            validate_stats_only_constant_page(
                &PageValidationContext {
                    dictionary: Some(&dictionary),
                    zone_stats: Some(&stats),
                    ..context(CoveLogicalType::Utf8, CovePhysicalKind::FileCode, None)
                },
                &page,
            ),
            Ok(())
        );
        let payload = materialize_stats_only_constant_page_payload(
            StatsOnlyPageMaterializationContext {
                table_id: Some(3),
                segment_id: Some(5),
                column_id: 7,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                dictionary_len: Some(dictionary.len()),
                zone_stats: &stats,
            },
            &page,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let values = tree_buffer_bytes(&payload, &payload.tree().unwrap(), PageBufferKind::Values)
            .unwrap()
            .unwrap();
        assert_eq!(values, [0u8; 8].as_slice());
    }

    #[test]
    fn validates_stats_only_all_null_filecode_without_reconstruction_source() {
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.non_null_count = 0;
        page.null_count = 2;
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NULL;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(CoveLogicalType::Utf8, CovePhysicalKind::FileCode, None),
                &page,
            ),
            Ok(())
        );
    }

    #[test]
    fn validates_and_materializes_boolean_stats_only_constant() {
        let scalar = StatScalar {
            kind: StatKind::UInt64,
            bytes: 1u64.to_le_bytes().to_vec(),
            truncated: false,
        };
        let stats = [ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 3,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 3,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        }];
        let mut page = base_page(3, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(
                    CoveLogicalType::Bool,
                    CovePhysicalKind::Boolean,
                    Some(&stats)
                ),
                &page,
            ),
            Ok(())
        );
        let payload = materialize_stats_only_constant_page_payload(
            StatsOnlyPageMaterializationContext {
                table_id: Some(3),
                segment_id: Some(5),
                column_id: 7,
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                dictionary_len: None,
                zone_stats: &stats,
            },
            &page,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let values = tree_buffer_bytes(&payload, &payload.tree().unwrap(), PageBufferKind::Values)
            .unwrap()
            .unwrap();
        assert_eq!(values, [1, 1, 1].as_slice());
    }

    #[test]
    fn validates_and_materializes_varbytes_stats_only_constant() {
        let bytes = b"alpha".to_vec();
        let scalar = StatScalar {
            kind: StatKind::FixedBytes,
            bytes: bytes.clone(),
            truncated: false,
        };
        let stats = [ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 2,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 2,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        }];
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    Some(&stats)
                ),
                &page,
            ),
            Ok(())
        );
        let payload = materialize_stats_only_constant_page_payload(
            StatsOnlyPageMaterializationContext {
                table_id: Some(3),
                segment_id: Some(5),
                column_id: 7,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                dictionary_len: None,
                zone_stats: &stats,
            },
            &page,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let values = tree_buffer_bytes(&payload, &payload.tree().unwrap(), PageBufferKind::Values)
            .unwrap()
            .unwrap();
        let expected = [
            5u32.to_le_bytes().as_slice(),
            b"alpha",
            5u32.to_le_bytes().as_slice(),
            b"alpha",
        ]
        .concat();
        assert_eq!(values, expected.as_slice());
    }

    #[test]
    fn rejects_invalid_json_stats_only_varbytes_constant() {
        let scalar = StatScalar {
            kind: StatKind::FixedBytes,
            bytes: b"{x}".to_vec(),
            truncated: false,
        };
        let stats = [ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 2,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 2,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        }];
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(
                    CoveLogicalType::Json,
                    CovePhysicalKind::VarBytes,
                    Some(&stats)
                ),
                &page,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn rejects_stats_only_without_null_polarity_fact() {
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(CoveLogicalType::Int64, CovePhysicalKind::NumCode, None),
                &page,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn rejects_stats_only_all_null_nested_without_reconstruction_support() {
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.non_null_count = 0;
        page.null_count = 2;
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NULL;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(CoveLogicalType::List, CovePhysicalKind::List, None),
                &page,
            ),
            Err(CoveError::PageCorrupt)
        );
    }

    #[test]
    fn validates_and_materializes_uuid_stats_only_fixedbytes_constant() {
        let bytes = [7u8; 16];
        let scalar = StatScalar {
            kind: StatKind::FixedBytes,
            bytes: bytes.to_vec(),
            truncated: false,
        };
        let stats = [ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 2,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 2,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        }];
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(
                    CoveLogicalType::Uuid,
                    CovePhysicalKind::FixedBytes,
                    Some(&stats)
                ),
                &page,
            ),
            Ok(())
        );
        let payload = materialize_stats_only_constant_page_payload(
            StatsOnlyPageMaterializationContext {
                table_id: Some(3),
                segment_id: Some(5),
                column_id: 7,
                logical_type: CoveLogicalType::Uuid,
                physical_kind: CovePhysicalKind::FixedBytes,
                dictionary_len: None,
                zone_stats: &stats,
            },
            &page,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let values = tree_buffer_bytes(&payload, &payload.tree().unwrap(), PageBufferKind::Values)
            .unwrap()
            .unwrap();
        assert_eq!(values, [bytes, bytes].concat().as_slice());
    }

    #[test]
    fn stats_only_page_stats_are_contextual() {
        let scalar = StatScalar {
            kind: StatKind::Int64,
            bytes: 9i64.to_le_bytes().to_vec(),
            truncated: false,
        };
        let valid = ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 2,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 2,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        };
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        assert_eq!(
            validate_stats_only_constant_page(
                &context(
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    Some(&[valid.clone()])
                ),
                &page,
            ),
            Ok(())
        );

        let mut bad_entries = Vec::new();
        let mut bad = valid.clone();
        bad.table_id = 4;
        bad_entries.push(bad);
        let mut bad = valid.clone();
        bad.segment_id = 6;
        bad_entries.push(bad);
        let mut bad = valid.clone();
        bad.morsel_id = u32::MAX;
        bad_entries.push(bad);
        let mut bad = valid.clone();
        bad.column_id = 8;
        bad_entries.push(bad);
        let mut bad = valid;
        bad.stats.row_count = 3;
        bad.non_null_count = 3;
        bad_entries.push(bad);

        for bad in bad_entries {
            assert_eq!(
                validate_stats_only_constant_page(
                    &context(
                        CoveLogicalType::Int64,
                        CovePhysicalKind::NumCode,
                        Some(&[bad])
                    ),
                    &page,
                ),
                Err(CoveError::PageCorrupt)
            );
        }
    }

    #[test]
    fn validates_and_materializes_float32_stats_only_fixedbytes_constant() {
        let bits = (-0.0f32).to_bits();
        let scalar = StatScalar {
            kind: StatKind::FixedBytes,
            bytes: bits.to_le_bytes().to_vec(),
            truncated: false,
        };
        let stats = [ZoneStatsEntry {
            table_id: 3,
            segment_id: 5,
            morsel_id: 0,
            column_id: 7,
            non_null_count: 2,
            distinct_count: 1,
            run_count: 1,
            stats: ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: 2,
                null_count: 0,
                min: Some(scalar.clone()),
                max: Some(scalar),
                flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
            },
            min_domain_rank: 0,
            max_domain_rank: 0,
            exact_set_ref: u32::MAX,
            bloom_ref: u32::MAX,
        }];
        let mut page = base_page(2, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        let context = StatsOnlyPageMaterializationContext {
            table_id: Some(3),
            segment_id: Some(5),
            column_id: 7,
            logical_type: CoveLogicalType::Float32,
            physical_kind: CovePhysicalKind::NumCode,
            dictionary_len: None,
            zone_stats: &stats,
        };
        let payload = materialize_stats_only_constant_page_payload(context, &page).unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload).unwrap();
        let values = tree_buffer_bytes(&payload, &payload.tree().unwrap(), PageBufferKind::Values)
            .unwrap()
            .unwrap();
        let constant = ConstantPayload::parse(values).unwrap();
        assert_eq!(constant.raw_value_bits(), u64::from(bits));
    }

    #[test]
    fn rejects_float32_stats_only_wrong_stat_shapes() {
        let mut page = base_page(1, CoveEncodingKind::NumCode);
        page.encoding_root = u32::MAX;
        page.page_length = 0;
        page.uncompressed_length = 0;
        page.flags = PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL;
        page.checksum = checksum::crc32c(&[]);

        for scalar in [
            StatScalar {
                kind: StatKind::FixedBytes,
                bytes: vec![0; 3],
                truncated: false,
            },
            StatScalar {
                kind: StatKind::Float64Bits,
                bytes: 0f64.to_bits().to_le_bytes().to_vec(),
                truncated: false,
            },
        ] {
            let stats = [ZoneStatsEntry {
                table_id: 3,
                segment_id: 5,
                morsel_id: 0,
                column_id: 7,
                non_null_count: 1,
                distinct_count: 1,
                run_count: 1,
                stats: ZoneStats {
                    scope: ZoneScope::Morsel,
                    row_count: 1,
                    null_count: 0,
                    min: Some(scalar.clone()),
                    max: Some(scalar),
                    flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
                },
                min_domain_rank: 0,
                max_domain_rank: 0,
                exact_set_ref: u32::MAX,
                bloom_ref: u32::MAX,
            }];
            let context = StatsOnlyPageMaterializationContext {
                table_id: Some(3),
                segment_id: Some(5),
                column_id: 7,
                logical_type: CoveLogicalType::Float32,
                physical_kind: CovePhysicalKind::NumCode,
                dictionary_len: None,
                zone_stats: &stats,
            };
            assert_eq!(
                materialize_stats_only_constant_page_payload(context, &page),
                Err(CoveError::PageCorrupt)
            );
        }
    }
}
