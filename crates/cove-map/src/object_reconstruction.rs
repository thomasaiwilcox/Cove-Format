use super::*;

pub(crate) fn build_temporal_segments(
    materialized: &MaterializedModel,
    nested_shapes: &NestedShapeByProperty,
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<Vec<TemporalSegmentBuild>, String> {
    let mut grouped = BTreeMap::<u32, Vec<ObjectRow>>::new();
    for row in &materialized.rows {
        grouped
            .entry(row.object_type_id)
            .or_default()
            .push(row.clone());
    }
    let object_types = materialized
        .object_types
        .iter()
        .map(|ty| (ty.object_type_id, ty))
        .collect::<BTreeMap<_, _>>();
    let mut out = Vec::new();
    for (segment_index, (object_type_id, mut rows)) in grouped.into_iter().enumerate() {
        rows.sort_by_key(|row| (row.source_row_index, row.goid, row.record_id));
        let object_type = object_types
            .get(&object_type_id)
            .ok_or_else(|| format!("missing object_type_id {object_type_id}"))?;
        let segment_id = u32::try_from(segment_index)
            .map_err(|_| "too many COVE-O temporal segments".to_string())?;
        let payload =
            temporal_segment_payload(segment_id, object_type, &rows, nested_shapes, dictionary)?;
        out.push(TemporalSegmentBuild {
            segment_id,
            object_type_id,
            rows,
            payload,
        });
    }
    Ok(out)
}

pub fn compact_cove_o_from_object_states(
    object_types: Vec<ObjectTypeEntryV1>,
    states: &[CoveObjectState],
) -> Result<Vec<u8>, String> {
    let segments = reconstructed_temporal_segments(&object_types, states)?;
    let segment_index = reconstructed_temporal_segment_index(&segments)?;
    let trust_manifest = reconstructed_trust_manifest(&segments)?;
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: object_types,
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_TRUST_CHAIN;
    writer.sections.push(object_section(
        SectionKind::ObjectTypeCatalog,
        catalog.types.len() as u64,
        0,
        catalog.serialize().map_err(|err| err.to_string())?,
    ));
    writer.sections.push(object_section(
        SectionKind::TemporalSegmentIndex,
        segments.len() as u64,
        states.len() as u64,
        segment_index.serialize().map_err(|err| err.to_string())?,
    ));
    for segment in &segments {
        writer.sections.push(object_section(
            SectionKind::TemporalSegmentData,
            1,
            segment.rows.len() as u64,
            segment.payload.clone(),
        ));
    }
    writer.sections.push(object_section(
        SectionKind::TrustManifest,
        trust_manifest.entries.len() as u64,
        0,
        trust_manifest.serialize().map_err(|err| err.to_string())?,
    ));

    let bytes = writer.write().map_err(|err| err.to_string())?;
    validate_bytes_with_options(
        &bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .map_err(|err| err.to_string())?;
    Ok(bytes)
}

pub fn checkpoint_temporal_sections_from_object_states(
    object_types: &[ObjectTypeEntryV1],
    states: &[CoveObjectState],
) -> Result<Vec<CoveObjectCheckpointTemporalSection>, String> {
    reconstructed_temporal_segments_with_record_kind(
        object_types,
        states,
        Some(RecordKind::Snapshot),
    )
    .map(|segments| {
        segments
            .into_iter()
            .map(|segment| CoveObjectCheckpointTemporalSection {
                object_type_id: segment.object_type_id,
                row_count: segment.rows.len() as u64,
                payload: segment.payload,
            })
            .collect()
    })
}

fn reconstructed_temporal_segments(
    object_types: &[ObjectTypeEntryV1],
    states: &[CoveObjectState],
) -> Result<Vec<ReconstructedTemporalSegmentBuild>, String> {
    reconstructed_temporal_segments_with_record_kind(object_types, states, None)
}

fn reconstructed_temporal_segments_with_record_kind(
    object_types: &[ObjectTypeEntryV1],
    states: &[CoveObjectState],
    record_kind_override: Option<RecordKind>,
) -> Result<Vec<ReconstructedTemporalSegmentBuild>, String> {
    let object_types_by_id = object_types
        .iter()
        .map(|object_type| (object_type.object_type_id, object_type))
        .collect::<BTreeMap<_, _>>();
    let mut grouped = BTreeMap::<u32, Vec<CoveObjectState>>::new();
    for state in states {
        if state.record_kind == RecordKind::ReservedLegacyMaterializedDelta {
            return Err("cannot compact reserved legacy materialized-delta records".into());
        }
        grouped
            .entry(state.object_type_id)
            .or_default()
            .push(state.clone());
    }

    let mut out = Vec::new();
    for (segment_index, (object_type_id, mut rows)) in grouped.into_iter().enumerate() {
        rows.sort_by_key(|state| {
            (
                state.timestamp_us,
                state.csn,
                state.branch_key,
                state.goid,
                state.latest_record_id,
            )
        });
        let object_type = object_types_by_id
            .get(&object_type_id)
            .ok_or_else(|| format!("missing object_type_id {object_type_id}"))?;
        let segment_id = u32::try_from(segment_index)
            .map_err(|_| "too many reconstructed COVE-O temporal segments".to_string())?;
        let payload = reconstructed_temporal_segment_payload(
            segment_id,
            object_type,
            &rows,
            record_kind_override,
        )?;
        out.push(ReconstructedTemporalSegmentBuild {
            segment_id,
            object_type_id,
            rows,
            payload,
        });
    }
    Ok(out)
}

fn reconstructed_temporal_segment_payload(
    segment_id: u32,
    object_type: &ObjectTypeEntryV1,
    rows: &[CoveObjectState],
    record_kind_override: Option<RecordKind>,
) -> Result<Vec<u8>, String> {
    let row_count =
        u32::try_from(rows.len()).map_err(|_| "too many reconstructed COVE-O rows".to_string())?;
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes_len = rows
        .len()
        .checked_mul(TEMPORAL_ROW_ENTRY_LEN)
        .ok_or_else(|| "temporal row directory length overflow".to_string())?;
    let column_directory_offset = row_directory_offset
        .checked_add(row_bytes_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let column_count = u32::try_from(object_type.properties.len())
        .map_err(|_| "too many reconstructed COVE-O property columns".to_string())?;
    let column_dir_len = object_type
        .properties
        .len()
        .checked_mul(TABLE_COLUMN_DIRECTORY_ENTRY_LEN)
        .ok_or_else(|| "temporal column directory length overflow".to_string())?;
    let page_index_offset = column_directory_offset
        .checked_add(column_dir_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let total_page_index_len = object_type
        .properties
        .len()
        .checked_mul(COLUMN_PAGE_INDEX_ENTRY_LEN)
        .ok_or_else(|| "temporal page index length overflow".to_string())?;
    let data_offset = page_index_offset
        .checked_add(total_page_index_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let header = TemporalSegmentHeaderV1 {
        segment_id,
        object_type_id: object_type.object_type_id,
        time_range_start_us: rows.first().map_or(0, |row| row.timestamp_us),
        time_range_end_us: rows.last().map_or(0, |row| row.timestamp_us),
        csn_min: rows.first().map_or(0, |row| row.csn),
        csn_max: rows.last().map_or(0, |row| row.csn),
        row_count,
        morsel_count: if row_count == 0 { 0 } else { 1 },
        morsel_row_count: if row_count == 0 { 0 } else { row_count },
        column_count,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };

    let mut out = header.serialize().to_vec();
    for row in rows {
        out.extend_from_slice(
            &TemporalRowEntryV1 {
                timestamp_us: row.timestamp_us,
                csn: row.csn,
                branch_key: row.branch_key,
                goid: row.goid,
                record_id: row.latest_record_id,
                record_kind: record_kind_override.unwrap_or(row.record_kind),
                prev_ref: None,
            }
            .serialize(),
        );
    }

    let mut column_directory = Vec::new();
    let mut page_index_bytes = Vec::new();
    let mut page_payload_bytes = Vec::new();
    let mut next_page_index_offset = page_index_offset;
    let mut next_data_offset = data_offset;
    for property in &object_type.properties {
        let column_page_index_offset = next_page_index_offset;
        let column_data_offset = next_data_offset;
        let page_payload = reconstructed_property_page_payload(property, rows)?;
        let page_length = page_payload.len() as u64;
        let page_checksum = checksum::crc32c(&page_payload);
        let null_count = rows
            .iter()
            .filter(|row| {
                reconstructed_property_value(row, property.property_id).is_none_or(Value::is_null)
            })
            .count() as u32;
        let page = ColumnPageIndexEntryV1 {
            column_id: property.property_id,
            morsel_id: 0,
            row_count,
            non_null_count: row_count.saturating_sub(null_count),
            null_count,
            encoding_root: encoding_for_physical(property.physical_kind) as u32,
            page_offset: next_data_offset,
            page_length,
            uncompressed_length: page_length,
            stats_ref: 0,
            flags: CompressionCodec::None as u32,
            checksum: page_checksum,
        };
        page_index_bytes.extend_from_slice(&page.serialize());
        page_payload_bytes.extend_from_slice(&page_payload);
        next_page_index_offset = next_page_index_offset
            .checked_add(COLUMN_PAGE_INDEX_ENTRY_LEN as u64)
            .ok_or_else(|| "temporal page index offset overflow".to_string())?;
        next_data_offset = next_data_offset
            .checked_add(page_length)
            .ok_or_else(|| "temporal data offset overflow".to_string())?;
        column_directory.push(TableColumnDirectoryEntryV1 {
            column_id: property.property_id,
            logical_type: property.logical_type,
            physical_kind: property.physical_kind,
            flags: 0,
            page_index_offset: column_page_index_offset,
            page_index_length: COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
            data_offset: column_data_offset,
            data_length: next_data_offset - column_data_offset,
            stats_ref: 0,
            domain_ref: 0,
            checksum: 0,
        });
    }
    for entry in &column_directory {
        out.extend_from_slice(&entry.serialize());
    }
    out.extend_from_slice(&page_index_bytes);
    out.extend_from_slice(&page_payload_bytes);
    Ok(out)
}

fn reconstructed_property_page_payload(
    property: &PropertyEntryV1,
    rows: &[CoveObjectState],
) -> Result<Vec<u8>, String> {
    let row_count = u32::try_from(rows.len()).map_err(|_| "too many rows".to_string())?;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut values = Vec::new();
    let mut null_count = 0usize;
    for (row_index, row) in rows.iter().enumerate() {
        let value = reconstructed_property_value(row, property.property_id).unwrap_or(&Value::Null);
        if row
            .properties
            .iter()
            .any(|candidate| candidate.property_id == property.property_id && candidate.redacted)
        {
            return Err("cannot compact redacted COVE-O property values".into());
        }
        if value.is_null() {
            null_count += 1;
            null_bitmap[row_index / 8] |= 1u8 << (row_index % 8);
        }
        append_property_value_bytes(property, value, None, None, &mut values)?;
    }
    ColumnPagePayloadV1::build_single_node(
        row_count,
        encoding_for_physical(property.physical_kind),
        property.logical_type,
        property.physical_kind,
        (null_count != 0).then_some(null_bitmap),
        values,
    )
    .map_err(|err| err.to_string())
}

fn reconstructed_property_value(state: &CoveObjectState, property_id: u32) -> Option<&Value> {
    state
        .properties
        .iter()
        .find(|property| property.property_id == property_id)
        .map(|property| &property.value)
}

fn reconstructed_temporal_segment_index(
    segments: &[ReconstructedTemporalSegmentBuild],
) -> Result<TemporalSegmentIndex, String> {
    let mut entries = Vec::with_capacity(segments.len());
    for segment in segments {
        let min_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .min()
            .unwrap_or([0; 16]);
        let max_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .max()
            .unwrap_or([0; 16]);
        let (delta_count, snapshot_count, baseline_count, tombstone_count) =
            reconstructed_row_kind_counts(&segment.rows);
        entries.push(TemporalSegmentIndexEntryV1 {
            segment_id: segment.segment_id,
            object_type_id: segment.object_type_id,
            time_range_start_us: segment.rows.first().map_or(0, |row| row.timestamp_us),
            time_range_end_us: segment.rows.last().map_or(0, |row| row.timestamp_us),
            csn_min: segment.rows.first().map_or(0, |row| row.csn),
            csn_max: segment.rows.last().map_or(0, |row| row.csn),
            row_count: u32::try_from(segment.rows.len())
                .map_err(|_| "too many COVE-O rows".to_string())?,
            delta_count,
            snapshot_count,
            baseline_count,
            tombstone_count,
            min_goid,
            max_goid,
            offset: 0,
            length: segment.payload.len() as u64,
            checksum: 0,
        });
    }
    Ok(TemporalSegmentIndex { flags: 0, entries })
}

fn reconstructed_row_kind_counts(rows: &[CoveObjectState]) -> (u32, u32, u32, u32) {
    let mut delta = 0;
    let mut snapshot = 0;
    let mut baseline = 0;
    let mut tombstone = 0;
    for row in rows {
        match row.record_kind {
            RecordKind::Delta => delta += 1,
            RecordKind::Snapshot => snapshot += 1,
            RecordKind::Baseline => baseline += 1,
            RecordKind::Tombstone => tombstone += 1,
            RecordKind::ReservedLegacyMaterializedDelta => {}
            _ => {}
        }
    }
    (delta, snapshot, baseline, tombstone)
}

fn reconstructed_trust_manifest(
    segments: &[ReconstructedTemporalSegmentBuild],
) -> Result<TrustManifest, String> {
    let mut previous = [0u8; 32];
    let mut entries = Vec::new();
    for segment in segments {
        let parsed_segment =
            TemporalSegmentData::parse(&segment.payload).map_err(|err| err.to_string())?;
        for index in 0..parsed_segment.rows.len() {
            let payload = temporal_row_trust_payload(
                &parsed_segment,
                index as u32,
                Option::<&FileDictionary>::None,
                &[],
            )
            .map_err(|err| err.to_string())?;
            let expected_hash =
                trust_chain::chain(&previous, &payload).map_err(|err| err.to_string())?;
            entries.push(TrustManifestEntryV1 {
                segment_id: segment.segment_id,
                row_index: index as u32,
                expected_hash,
            });
            previous = expected_hash;
        }
    }
    Ok(TrustManifest { entries })
}

pub(crate) fn object_section(
    kind: SectionKind,
    item_count: u64,
    row_count: u64,
    data: Vec<u8>,
) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count,
        row_count,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data,
    }
}

pub(crate) fn dictionary_section(
    kind: SectionKind,
    item_count: u64,
    data: Vec<u8>,
) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::Mixed as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_FILE_DICTIONARY,
        optional_features: 0,
        data,
    }
}

pub(crate) fn map_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_SEMANTIC_MAP,
        data: ensure_covemap_payload_envelope(kind, data),
    }
}

pub(crate) fn ensure_covemap_payload_envelope(kind: SectionKind, data: Vec<u8>) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<Value>(&data) else {
        return data;
    };
    let Value::Object(object) = &mut value else {
        return data;
    };
    object.insert(
        "schema_id".to_string(),
        Value::String("org.coveformat.covemap.v2".to_string()),
    );
    object.insert(
        "section_id".to_string(),
        Value::Number(serde_json::Number::from(kind as u16)),
    );
    serde_json::to_vec_pretty(&value).unwrap_or(data)
}
