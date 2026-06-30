use super::*;

/// A named top-level Arrow export column.
pub struct ArrowExportColumn<'a> {
    pub name: &'a str,
    pub node: ArrowExportNode<'a>,
    pub nullable: bool,
}

impl<'a> ArrowExportColumn<'a> {
    pub fn scalar(name: &'a str, array: &'a EncodedArray<'a>) -> Self {
        Self {
            name,
            node: ArrowExportNode::scalar(array),
            nullable: array.validity.is_some() || array.logical == CoveLogicalType::Null,
        }
    }
}

/// A layout-aware Arrow export node.
#[non_exhaustive]
pub enum ArrowExportNode<'a> {
    Scalar {
        array: &'a EncodedArray<'a>,
        dictionary_policy: ArrowDictionaryPolicy,
    },
    List {
        layout: &'a ListLayoutPayload,
        child: Box<ArrowExportNode<'a>>,
        validity: Option<ValidityBitmap<'a>>,
    },
    Struct {
        layout: &'a StructLayoutPayload,
        fields: Vec<ArrowExportColumn<'a>>,
        validity: Option<ValidityBitmap<'a>>,
    },
    Map {
        layout: &'a MapLayoutPayload,
        keys: Box<ArrowExportNode<'a>>,
        values: Box<ArrowExportNode<'a>>,
        validity: Option<ValidityBitmap<'a>>,
        ordered: bool,
    },
}

impl<'a> ArrowExportNode<'a> {
    pub fn scalar(array: &'a EncodedArray<'a>) -> Self {
        Self::Scalar {
            array,
            dictionary_policy: ArrowDictionaryPolicy::default(),
        }
    }

    pub fn scalar_with_policy(
        array: &'a EncodedArray<'a>,
        dictionary_policy: ArrowDictionaryPolicy,
    ) -> Self {
        Self::Scalar {
            array,
            dictionary_policy,
        }
    }
}

/// Export one layout-aware COVE node as an Arrow array.
pub fn arrow_export_node_to_array(node: &ArrowExportNode<'_>) -> Result<ArrayRef, CoveError> {
    match node {
        ArrowExportNode::Scalar {
            array,
            dictionary_policy,
        } => encoded_array_to_arrow_with_policy(array, *dictionary_policy),
        ArrowExportNode::List {
            layout,
            child,
            validity,
        } => {
            layout.validate()?;
            let offsets = arrow_i32_offsets(&layout.layout.offsets)?;
            let child_array = arrow_export_node_to_array(child)?;
            if child_array.len()
                != usize::try_from(layout.child_row_count).map_err(|_| CoveError::ArithOverflow)?
            {
                return Err(CoveError::PageCorrupt);
            }
            let row_count = layout.layout.row_count();
            let nulls = arrow_null_buffer(*validity, row_count)?;
            let field = Arc::new(Field::new(
                "item",
                child_array.data_type().clone(),
                arrow_node_nullable(child),
            ));
            ListArray::try_new(field, offsets, child_array, nulls)
                .map(|array| Arc::new(array) as ArrayRef)
                .map_err(|err| CoveError::BadSection(format!("Arrow ListArray: {err}")))
        }
        ArrowExportNode::Struct {
            layout,
            fields,
            validity,
        } => {
            let row_count = usize::try_from(layout.layout.row_count()?)
                .map_err(|_| CoveError::ArithOverflow)?;
            layout.validate(row_count as u64)?;
            if fields.len() != layout.layout.field_row_counts.len() {
                return Err(CoveError::PageCorrupt);
            }

            let mut arrow_fields = Vec::with_capacity(fields.len());
            let mut arrays = Vec::with_capacity(fields.len());
            for (index, column) in fields.iter().enumerate() {
                let array = arrow_export_node_to_array(&column.node)?;
                let expected = usize::try_from(layout.layout.field_row_counts[index])
                    .map_err(|_| CoveError::ArithOverflow)?;
                if array.len() != expected || expected != row_count {
                    return Err(CoveError::PageCorrupt);
                }
                arrow_fields.push(Field::new(
                    column.name,
                    array.data_type().clone(),
                    column.nullable || arrow_node_nullable(&column.node),
                ));
                arrays.push(array);
            }
            let nulls = arrow_null_buffer(*validity, row_count)?;
            StructArray::try_new(Fields::from(arrow_fields), arrays, nulls)
                .map(|array| Arc::new(array) as ArrayRef)
                .map_err(|err| CoveError::BadSection(format!("Arrow StructArray: {err}")))
        }
        ArrowExportNode::Map {
            layout,
            keys,
            values,
            validity,
            ordered,
        } => {
            layout.validate()?;
            let offsets = arrow_i32_offsets(&layout.layout.offsets)?;
            if !matches!(keys.as_ref(), ArrowExportNode::Scalar { .. }) {
                return Err(CoveError::PageCorrupt);
            }
            let key_array = arrow_export_node_to_array(keys)?;
            if key_array.null_count() != 0 {
                return Err(CoveError::UnsupportedEncoding(
                    "Arrow map export requires non-null map keys".into(),
                ));
            }
            let value_array = arrow_export_node_to_array(values)?;
            let key_count = usize::try_from(layout.layout.key_row_count)
                .map_err(|_| CoveError::ArithOverflow)?;
            let value_count = usize::try_from(layout.layout.value_row_count)
                .map_err(|_| CoveError::ArithOverflow)?;
            if key_array.len() != key_count || value_array.len() != value_count {
                return Err(CoveError::PageCorrupt);
            }
            let entry_fields = Fields::from(vec![
                Field::new("key", key_array.data_type().clone(), false),
                Field::new(
                    "value",
                    value_array.data_type().clone(),
                    arrow_node_nullable(values),
                ),
            ]);
            let entries = StructArray::try_new(entry_fields, vec![key_array, value_array], None)
                .map_err(|err| CoveError::BadSection(format!("Arrow Map entries: {err}")))?;
            let row_count = layout.layout.row_count();
            let nulls = arrow_null_buffer(*validity, row_count)?;
            let entries_field = Arc::new(Field::new("entries", entries.data_type().clone(), false));
            MapArray::try_new(entries_field, offsets, entries, nulls, *ordered)
                .map(|array| Arc::new(array) as ArrayRef)
                .map_err(|err| CoveError::BadSection(format!("Arrow MapArray: {err}")))
        }
    }
}

pub fn nested_page_payload_to_arrow_array(
    payload: &RetainedColumnPagePayloadV1,
    schema: &NestedSchemaNodeV1,
    selection: ArrowRowSelection<'_>,
    dictionary: Option<&crate::dictionary::FileDictionary>,
    options: ArrowExportOptions,
) -> Result<ArrowExportResult<ArrayRef>, CoveError> {
    let tree = payload.tree()?;
    let mut report = ArrowExportReport::default();
    let value = nested_tree_to_arrow_array(
        payload,
        &tree,
        schema,
        selection,
        dictionary,
        options,
        &mut report,
    )?;
    Ok(ArrowExportResult { value, report })
}

fn nested_tree_to_arrow_array(
    payload: &RetainedColumnPagePayloadV1,
    tree: &PagePayloadTreeNode<'_>,
    schema: &NestedSchemaNodeV1,
    selection: ArrowRowSelection<'_>,
    dictionary: Option<&crate::dictionary::FileDictionary>,
    options: ArrowExportOptions,
    report: &mut ArrowExportReport,
) -> Result<ArrayRef, CoveError> {
    if tree.node.logical_type != schema.logical || tree.node.physical_kind != schema.physical {
        return Err(CoveError::PageCorrupt);
    }
    match schema.physical {
        CovePhysicalKind::List => {
            if tree.children.len() != 1 || schema.children.len() != 1 {
                return Err(CoveError::PageCorrupt);
            }
            let layout_bytes =
                retained_tree_buffer_bytes(payload, tree, PageBufferKind::ChildLayout)?
                    .ok_or(CoveError::PageCorrupt)?;
            let layout = ListLayoutPayload::parse(layout_bytes)?;
            layout.validate()?;
            let selected_rows = selection.to_rows(u64::from(tree.node.logical_len))?;
            let mut offsets = Vec::with_capacity(selected_rows.len() + 1);
            let mut child_rows = Vec::new();
            offsets.push(0u32);
            let mut next_offset = 0u32;
            for row in &selected_rows {
                let row = usize::try_from(*row).map_err(|_| CoveError::ArithOverflow)?;
                let start = layout.layout.offsets[row] as usize;
                let end = layout.layout.offsets[row + 1] as usize;
                let len = end.checked_sub(start).ok_or(CoveError::ArithOverflow)?;
                for child_row in start..end {
                    child_rows
                        .push(u32::try_from(child_row).map_err(|_| CoveError::ArithOverflow)?);
                }
                next_offset = next_offset
                    .checked_add(u32::try_from(len).map_err(|_| CoveError::ArithOverflow)?)
                    .ok_or(CoveError::ArithOverflow)?;
                offsets.push(next_offset);
            }
            let child = nested_tree_to_arrow_array(
                payload,
                &tree.children[0],
                &schema.children[0],
                ArrowRowSelection::Rows(&child_rows),
                dictionary,
                options,
                report,
            )?;
            let nulls = selected_arrow_null_buffer(
                retained_node_validity(payload, tree)?,
                u64::from(tree.node.logical_len),
                selection,
            )?;
            let field = Arc::new(Field::new(
                schema.children[0].name.clone(),
                child.data_type().clone(),
                schema.children[0].nullable,
            ));
            if schema.fixed_size_list_len == 0 {
                ListArray::try_new(field, arrow_i32_offsets(&offsets)?, child, nulls)
                    .map(|array| Arc::new(array) as ArrayRef)
                    .map_err(|err| CoveError::BadSection(format!("Arrow ListArray: {err}")))
            } else {
                let width = i32::try_from(schema.fixed_size_list_len)
                    .map_err(|_| CoveError::ArithOverflow)?;
                FixedSizeListArray::try_new(field, width, child, nulls)
                    .map(|array| Arc::new(array) as ArrayRef)
                    .map_err(|err| {
                        CoveError::BadSection(format!("Arrow FixedSizeListArray: {err}"))
                    })
            }
        }
        CovePhysicalKind::Struct => {
            if tree.children.len() != schema.children.len() {
                return Err(CoveError::PageCorrupt);
            }
            let layout_bytes =
                retained_tree_buffer_bytes(payload, tree, PageBufferKind::ChildLayout)?
                    .ok_or(CoveError::PageCorrupt)?;
            let layout = StructLayoutPayload::parse(layout_bytes)?;
            layout.validate(u64::from(tree.node.logical_len))?;
            let mut fields = Vec::with_capacity(schema.children.len());
            let mut arrays = Vec::with_capacity(schema.children.len());
            for (child_tree, child_schema) in tree.children.iter().zip(&schema.children) {
                let array = nested_tree_to_arrow_array(
                    payload,
                    child_tree,
                    child_schema,
                    selection,
                    dictionary,
                    options,
                    report,
                )?;
                fields.push(Field::new(
                    child_schema.name.clone(),
                    array.data_type().clone(),
                    child_schema.nullable,
                ));
                arrays.push(array);
            }
            let nulls = selected_arrow_null_buffer(
                retained_node_validity(payload, tree)?,
                u64::from(tree.node.logical_len),
                selection,
            )?;
            StructArray::try_new(Fields::from(fields), arrays, nulls)
                .map(|array| Arc::new(array) as ArrayRef)
                .map_err(|err| CoveError::BadSection(format!("Arrow StructArray: {err}")))
        }
        CovePhysicalKind::Map => {
            if tree.children.len() != 2 || schema.children.len() != 2 {
                return Err(CoveError::PageCorrupt);
            }
            let layout_bytes =
                retained_tree_buffer_bytes(payload, tree, PageBufferKind::ChildLayout)?
                    .ok_or(CoveError::PageCorrupt)?;
            let layout = MapLayoutPayload::parse(layout_bytes)?;
            layout.validate()?;
            let selected_rows = selection.to_rows(u64::from(tree.node.logical_len))?;
            let mut offsets = Vec::with_capacity(selected_rows.len() + 1);
            let mut child_rows = Vec::new();
            offsets.push(0u32);
            let mut next_offset = 0u32;
            for row in &selected_rows {
                let row = usize::try_from(*row).map_err(|_| CoveError::ArithOverflow)?;
                let start = layout.layout.offsets[row] as usize;
                let end = layout.layout.offsets[row + 1] as usize;
                let len = end.checked_sub(start).ok_or(CoveError::ArithOverflow)?;
                for child_row in start..end {
                    child_rows
                        .push(u32::try_from(child_row).map_err(|_| CoveError::ArithOverflow)?);
                }
                next_offset = next_offset
                    .checked_add(u32::try_from(len).map_err(|_| CoveError::ArithOverflow)?)
                    .ok_or(CoveError::ArithOverflow)?;
                offsets.push(next_offset);
            }
            let keys = nested_tree_to_arrow_array(
                payload,
                &tree.children[0],
                &schema.children[0],
                ArrowRowSelection::Rows(&child_rows),
                dictionary,
                options,
                report,
            )?;
            if keys.null_count() != 0 {
                return Err(CoveError::PageCorrupt);
            }
            let values = nested_tree_to_arrow_array(
                payload,
                &tree.children[1],
                &schema.children[1],
                ArrowRowSelection::Rows(&child_rows),
                dictionary,
                options,
                report,
            )?;
            let entry_fields = Fields::from(vec![
                Field::new("key", keys.data_type().clone(), false),
                Field::new(
                    "value",
                    values.data_type().clone(),
                    schema.children[1].nullable,
                ),
            ]);
            let entries = StructArray::try_new(entry_fields, vec![keys, values], None)
                .map_err(|err| CoveError::BadSection(format!("Arrow Map entries: {err}")))?;
            let nulls = selected_arrow_null_buffer(
                retained_node_validity(payload, tree)?,
                u64::from(tree.node.logical_len),
                selection,
            )?;
            let entries_field = Arc::new(Field::new("entries", entries.data_type().clone(), false));
            MapArray::try_new(
                entries_field,
                arrow_i32_offsets(&offsets)?,
                entries,
                nulls,
                false,
            )
            .map(|array| Arc::new(array) as ArrayRef)
            .map_err(|err| CoveError::BadSection(format!("Arrow MapArray: {err}")))
        }
        _ => {
            let values = retained_tree_buffer_bytes(payload, tree, PageBufferKind::Values)?
                .ok_or(CoveError::PageCorrupt)?;
            let validity = retained_node_validity(payload, tree)?;
            let array = EncodedArray::new(
                tree.node.logical_type,
                tree.node.physical_kind,
                u64::from(tree.node.logical_len),
                tree.node.encoding_kind,
                validity,
                values,
                dictionary,
            );
            let result =
                encoded_array_to_arrow_with_row_selection_options(&array, selection, options)?;
            report.extend_with_field(&schema.name, result.report);
            Ok(result.value)
        }
    }
}

fn retained_tree_buffer_bytes<'a>(
    payload: &'a RetainedColumnPagePayloadV1,
    tree: &PagePayloadTreeNode<'_>,
    kind: PageBufferKind,
) -> Result<Option<&'a [u8]>, CoveError> {
    tree.buffer_of_kind(kind)?
        .map(|descriptor| payload.buffer_bytes_for_descriptor(descriptor))
        .transpose()
}

fn retained_node_validity<'a>(
    payload: &'a RetainedColumnPagePayloadV1,
    tree: &PagePayloadTreeNode<'_>,
) -> Result<Option<ValidityBitmap<'a>>, CoveError> {
    retained_tree_buffer_bytes(payload, tree, PageBufferKind::NullBitmap).map(|maybe| {
        maybe.map(|bytes| ValidityBitmap::new(bytes, u64::from(tree.node.logical_len)))
    })
}

fn selected_arrow_null_buffer(
    validity: Option<ValidityBitmap<'_>>,
    row_count: u64,
    selection: ArrowRowSelection<'_>,
) -> Result<Option<NullBuffer>, CoveError> {
    let Some(validity) = validity else {
        return Ok(None);
    };
    let selected_len = selection.selected_len(row_count)?;
    let mut builder = ArrowValidityBuilder::new(selected_len)?;
    selection.for_each_row(row_count, |row| {
        builder.append(validity.is_valid(row as u64)?);
        Ok(())
    })?;
    Ok(builder.finish())
}

/// Export layout-aware COVE columns as an Arrow [`RecordBatch`].
pub fn arrow_export_columns_to_record_batch(
    columns: &[ArrowExportColumn<'_>],
) -> Result<RecordBatch, CoveError> {
    let mut fields = Vec::with_capacity(columns.len());
    let mut arrays = Vec::with_capacity(columns.len());
    for column in columns {
        let arrow_array = arrow_export_node_to_array(&column.node)?;
        fields.push(Field::new(
            column.name,
            arrow_array.data_type().clone(),
            column.nullable || arrow_node_nullable(&column.node),
        ));
        arrays.push(arrow_array);
    }
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
        .map_err(|err| CoveError::BadSection(format!("Arrow RecordBatch: {err}")))
}
