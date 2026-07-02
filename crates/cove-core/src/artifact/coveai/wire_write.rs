use super::*;

pub fn write_covev_filecode_vectors(
    build: &CoveVecFileCodeVectorBuild,
) -> Result<Vec<u8>, CoveError> {
    write_covev_filecode_vectors_with_options(build, CoveVecFileCodeVectorBuildOptions::default())
}

pub fn write_covev_filecode_vectors_with_index(
    build: &CoveVecFileCodeVectorBuild,
    index_kind: u8,
) -> Result<Vec<u8>, CoveError> {
    write_covev_filecode_vectors_with_options(
        build,
        CoveVecFileCodeVectorBuildOptions {
            index_kind: Some(index_kind),
            ..CoveVecFileCodeVectorBuildOptions::default()
        },
    )
}

pub fn write_covev_filecode_vectors_with_options(
    build: &CoveVecFileCodeVectorBuild,
    options: CoveVecFileCodeVectorBuildOptions,
) -> Result<Vec<u8>, CoveError> {
    validate_ai_vector_metric(options.metric, "COVE-VEC build metric")?;
    validate_ai_quantization_kind(
        options.quantization_kind,
        "COVE-VEC build quantization_kind",
    )?;
    if let Some(index_kind) = options.index_kind {
        validate_ai_vector_index_kind(index_kind, "COVE-VEC build index_kind")?;
    }
    write_covev_filecode_vectors_inner(build, options)
}

pub(super) fn write_covev_filecode_vectors_inner(
    build: &CoveVecFileCodeVectorBuild,
    options: CoveVecFileCodeVectorBuildOptions,
) -> Result<Vec<u8>, CoveError> {
    if build.dimension_count == 0 {
        return Err(CoveError::BadSection(
            "COVE-VEC build requires non-zero dimension_count".into(),
        ));
    }
    if build.file_codes.is_empty() {
        return Err(CoveError::BadSection(
            "COVE-VEC build requires at least one FileCode".into(),
        ));
    }
    let mut seen_file_codes = BTreeSet::new();
    for file_code in &build.file_codes {
        if !seen_file_codes.insert(*file_code) {
            return Err(CoveError::BadSection(format!(
                "COVE-VEC build has duplicate FileCode {file_code}"
            )));
        }
    }

    let source_row_width = u64::from(build.dimension_count)
        .checked_mul(4)
        .ok_or(CoveError::ArithOverflow)?;
    let expected_source_payload_len = source_row_width
        .checked_mul(u64::try_from(build.file_codes.len()).map_err(|_| CoveError::ArithOverflow)?)
        .ok_or(CoveError::ArithOverflow)?;
    if build.vector_payload.len() as u64 != expected_source_payload_len {
        return Err(CoveError::BadSection(format!(
            "COVE-VEC payload length {} does not match {} FileCodes * {} dimensions * 4 bytes",
            build.vector_payload.len(),
            build.file_codes.len(),
            build.dimension_count
        )));
    }
    let (stored_element_type, stored_payload) =
        covev_stored_vector_payload(build.dimension_count, &build.vector_payload, options)?;
    let row_width = u64::from(build.dimension_count)
        .checked_mul(element_width_bytes(stored_element_type).ok_or_else(|| {
            CoveError::BadSection(format!(
                "COVE-VEC build unsupported stored element_type {stored_element_type}"
            ))
        })?)
        .ok_or(CoveError::ArithOverflow)?;
    let row_width_u32 = u32::try_from(row_width).map_err(|_| CoveError::ArithOverflow)?;
    let expected_payload_len = row_width
        .checked_mul(u64::try_from(build.file_codes.len()).map_err(|_| CoveError::ArithOverflow)?)
        .ok_or(CoveError::ArithOverflow)?;

    let vector_payload_crc32c = checksum::crc32c(&stored_payload);
    let digest_offset =
        u64::try_from(stored_payload.len()).map_err(|_| CoveError::ArithOverflow)?;
    let mut payload_bytes = stored_payload;
    payload_bytes.extend_from_slice(&vector_payload_crc32c.to_le_bytes());

    let reference_tables = encode_records([
        encode_digest_entry(AiDigestEntryV1 {
            digest_ref: 1,
            digest_algorithm: 1,
            digest_len: 4,
            digest_payload_ref: 2,
            domain_hint: 1,
            flags: 0,
            crc32c: 0,
        })?,
        encode_payload_ref_entry(AiPayloadRefEntryV1 {
            payload_ref: 1,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 0,
            payload_length: expected_payload_len,
            decoded_length: expected_payload_len,
            integrity_ref: 1,
            flags: 0,
            crc32c: 0,
        })?,
        encode_payload_ref_entry(AiPayloadRefEntryV1 {
            payload_ref: 2,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: digest_offset,
            payload_length: 4,
            decoded_length: 4,
            integrity_ref: 0,
            flags: 0,
            crc32c: 0,
        })?,
    ]);

    let privacy_summary = encode_records([encode_privacy_summary(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 0,
        sensitivity_bits_ref: 0,
        policy_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        retention_state: 0,
        disclosure_state: 0,
        flags: 0,
        crc32c: 0,
    })?]);

    let payload_integrity = encode_records([encode_payload_integrity(AiPayloadIntegrityV1 {
        integrity_ref: 1,
        payload_ref: 1,
        digest_domain: 1,
        reserved0: 0,
        digest_algorithm: 1,
        digest_len: 4,
        digest_ref: 1,
        payload_crc32c: vector_payload_crc32c,
        flags: 0,
    })?]);

    let vector_space = encode_records([encode_vector_space(VectorSpaceDescriptorV1 {
        vector_space_id: 1,
        vector_space_name_ref: 0,
        vector_space_fingerprint_ref: 0,
        embedding_namespace_ref: 0,
        embedding_model_ref: 0,
        embedding_model_version_ref: 0,
        embedding_model_digest_ref: 0,
        embedding_pipeline_ref: 0,
        tokenizer_profile_ref: 0,
        chunk_profile_ref: 0,
        dimension_count: build.dimension_count,
        element_type: stored_element_type,
        metric: options.metric,
        normalization_policy: 0,
        quantization_policy: options.quantization_kind,
        deterministic: 1,
        approximate: 0,
        reproducibility_class: 1,
        reserved: 0,
        flags: 0,
        checksum: 0,
    })?]);

    let vector_payload_block =
        encode_records([encode_vector_payload_block(VectorPayloadBlockHeaderV1 {
            block_id: 1,
            vector_space_id: 1,
            vector_count: build.file_codes.len() as u64,
            dimension_count: build.dimension_count,
            element_type: stored_element_type,
            compression_codec: CompressionCodec::None as u8,
            quantization_kind: options.quantization_kind,
            layout_kind: 0,
            tensor_layout_ref: 0,
            memory_alignment_bytes: 4,
            payload_stride_ref: 0,
            device_transfer_hint_ref: 0,
            payload_ref: 1,
            payload_offset: 0,
            payload_length: 0,
            integrity_ref: 1,
            checksum: 0,
        })?]);

    let mut vector_entries = Vec::with_capacity(build.file_codes.len());
    let mut filecode_bindings = Vec::with_capacity(build.file_codes.len());
    for (index, file_code) in build.file_codes.iter().copied().enumerate() {
        let ordinal = u64::try_from(index).map_err(|_| CoveError::ArithOverflow)?;
        let vector_ref = ordinal.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        vector_entries.push(encode_vector_entry(VectorEntryV1 {
            vector_ref,
            block_id: 1,
            vector_ordinal: ordinal,
            payload_offset: ordinal
                .checked_mul(row_width)
                .ok_or(CoveError::ArithOverflow)?,
            payload_length: row_width_u32,
            integrity_ref: 0,
            flags: 0,
            checksum: 0,
        })?);
        filecode_bindings.push(encode_filecode_vector_binding(FileCodeVectorBindingV1 {
            binding_id: vector_ref,
            vector_space_id: 1,
            slot_policy_ref: 0,
            file_ref: 0,
            dictionary_digest_ref: 0,
            schema_fingerprint_ref: 0,
            table_id: 0,
            column_id: 0,
            object_type_id: 0,
            property_id: 0,
            association_type_id: 0,
            path_ref: 0,
            file_code,
            reserved0: 0,
            canonical_value_hash_ref: 0,
            vector_ref,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        })?);
    }

    let mut sections = vec![
        coveai_payload_section(
            1,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveVec,
            payload_bytes,
        ),
        coveai_binary_section(
            2,
            SectionKind::AiReferenceTables,
            PrimaryProfile::CoveAiShared,
            reference_tables,
        ),
        coveai_binary_section(
            3,
            SectionKind::AiPrivacySummary,
            PrimaryProfile::CoveAiShared,
            privacy_summary,
        ),
        coveai_binary_section(
            4,
            SectionKind::AiPayloadIntegrity,
            PrimaryProfile::CoveAiShared,
            payload_integrity,
        ),
        coveai_binary_section(
            5,
            SectionKind::AiVectorSpace,
            PrimaryProfile::CoveVec,
            vector_space,
        ),
        coveai_binary_section(
            6,
            SectionKind::AiVectorPayloadBlock,
            PrimaryProfile::CoveVec,
            vector_payload_block,
        ),
        coveai_binary_section(
            7,
            SectionKind::AiVectorDirectory,
            PrimaryProfile::CoveVec,
            encode_records(vector_entries),
        ),
        coveai_binary_section(
            8,
            SectionKind::AiVectorBinding,
            PrimaryProfile::CoveVec,
            encode_records(filecode_bindings),
        ),
    ];
    if let Some(index_kind) = options.index_kind {
        let exactness_kind = if index_kind == 0 { 0 } else { 1 };
        sections.push(coveai_binary_section(
            9,
            SectionKind::AiVectorIndex,
            PrimaryProfile::CoveVec,
            encode_records([encode_vector_index(VectorIndexDescriptorV1 {
                vector_index_id: 1,
                vector_space_id: 1,
                stored_vector_space_id: 1,
                search_vector_space_id: 1,
                index_kind,
                exactness_kind,
                false_negative_policy: 0,
                metric: options.metric,
                score_space_authority: 0,
                dimension_count: build.dimension_count,
                indexed_binding_kind: 1,
                temporal_scope_ref: 0,
                visibility_scope_ref: 0,
                redaction_scope_ref: 0,
                dequantization_profile_ref: 0,
                quantization_error_profile_ref: 0,
                payload_ref: 0,
                checksum: 0,
            })?]),
        ));
    }

    let bytes = write_coveai_artifact(
        CoveAiArtifactKind::CoveVec,
        build.artifact_id,
        build.created_at_us,
        &sections,
    )?;
    CoveAiFile::parse(&bytes)?;
    Ok(bytes)
}

pub(super) fn covev_stored_vector_payload(
    dimension_count: u32,
    source_f32_payload: &[u8],
    options: CoveVecFileCodeVectorBuildOptions,
) -> Result<(u8, Vec<u8>), CoveError> {
    let values = source_f32_payload
        .chunks_exact(4)
        .map(|chunk| {
            let value = f32::from_bits(read_u32(chunk, 0)?);
            if !value.is_finite() {
                return Err(CoveError::BadSection(
                    "COVE-VEC build source payload contains non-finite Float32 values".into(),
                ));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, CoveError>>()?;
    match options.quantization_kind {
        0 => Ok((0, source_f32_payload.to_vec())),
        1 => {
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                let quantized = (value * 127.0).round().clamp(-128.0, 127.0) as i8;
                out.push(quantized as u8);
            }
            Ok((3, out))
        }
        2 => {
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                let quantized = ((value + 1.0) * 127.5).round().clamp(0.0, 255.0) as u8;
                out.push(quantized);
            }
            Ok((4, out))
        }
        3 => {
            let dimension =
                usize::try_from(dimension_count).map_err(|_| CoveError::ArithOverflow)?;
            if dimension == 0 || values.len() % dimension != 0 {
                return Err(CoveError::BadSection(
                    "COVE-VEC build PQ source payload is not a whole number of vectors".into(),
                ));
            }
            let mut ranges = vec![(f32::INFINITY, f32::NEG_INFINITY); dimension];
            for vector in values.chunks_exact(dimension) {
                for (lane, value) in vector.iter().enumerate() {
                    ranges[lane].0 = ranges[lane].0.min(*value);
                    ranges[lane].1 = ranges[lane].1.max(*value);
                }
            }
            let mut out = Vec::with_capacity(values.len());
            for vector in values.chunks_exact(dimension) {
                for (lane, value) in vector.iter().enumerate() {
                    let (min, max) = ranges[lane];
                    let span = max - min;
                    let code = if span <= f32::EPSILON {
                        0
                    } else {
                        (((*value - min) / span) * 255.0).round().clamp(0.0, 255.0) as u8
                    };
                    out.push(code);
                }
            }
            Ok((4, out))
        }
        other => Err(CoveError::BadSection(format!(
            "COVE-VEC build unsupported quantization_kind {other}"
        ))),
    }
}

pub(super) fn exact_flat_parse_covev_with_payload_access(
    artifact_bytes: &[u8],
) -> Result<CoveAiFile, CoveError> {
    let sidecar =
        CoveAiFile::parse_for_operation(artifact_bytes, OperationKindV2::AiSemanticSearch)?;
    if sidecar.artifact_kind != CoveAiArtifactKind::CoveVec {
        return Err(CoveError::BadSection(
            "exact flat FileCode vector search requires a COVE-VEC (.covev) artifact".into(),
        ));
    }
    if sidecar.payload_access != AiPayloadAccessState::StructurallyAllowed {
        return Err(CoveError::BadSection(
            "direct AI payload access is policy-blocked: missing AI_PRIVACY_SUMMARY".into(),
        ));
    }
    Ok(sidecar)
}

pub(super) fn exact_flat_validate_vector_space(
    vector_space: &VectorSpaceDescriptorV1,
) -> Result<(), CoveError> {
    if vector_space.element_type != 0 {
        return Err(CoveError::BadSection(format!(
            "exact flat FileCode vector search supports only Float32 vector spaces, got element_type {}",
            vector_space.element_type
        )));
    }
    if vector_space.approximate != 0 {
        return Err(CoveError::BadSection(format!(
            "exact flat FileCode vector search requires a non-approximate vector space, got approximate {}",
            vector_space.approximate
        )));
    }
    if !matches!(vector_space.metric, 0..=3) {
        return Err(CoveError::BadSection(format!(
            "exact flat FileCode vector search does not support vector metric {}",
            vector_space.metric
        )));
    }
    Ok(())
}

pub(super) fn validate_runtime_vector_space(
    vector_space: &VectorSpaceDescriptorV1,
) -> Result<(), CoveError> {
    if vector_space.approximate != 0 {
        return Err(CoveError::BadSection(format!(
            "AI runtime vector search requires a directly comparable vector space, got approximate {}",
            vector_space.approximate
        )));
    }
    if !matches!(vector_space.metric, 0..=3) {
        return Err(CoveError::BadSection(format!(
            "AI runtime vector search does not support vector metric {}",
            vector_space.metric
        )));
    }
    if !matches!(vector_space.element_type, 0..=4 | 6) {
        return Err(CoveError::BadSection(format!(
            "AI runtime vector search does not support vector element_type {}",
            vector_space.element_type
        )));
    }
    Ok(())
}

pub(super) fn vector_entry_by_ref(
    sidecar: &CoveAiFile,
    vector_ref: u64,
) -> Result<&VectorEntryV1, CoveError> {
    sidecar
        .descriptor_tables
        .vector_entries
        .iter()
        .find(|entry| entry.vector_ref == vector_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "AI vector_ref {vector_ref} is not present in vector directory"
            ))
        })
}

pub(super) fn vector_space_for_entry<'a>(
    sidecar: &'a CoveAiFile,
    vector_entry: &VectorEntryV1,
) -> Result<&'a VectorSpaceDescriptorV1, CoveError> {
    let block = sidecar
        .descriptor_tables
        .vector_block(vector_entry.block_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorEntry {} references missing block_id {}",
                vector_entry.vector_ref, vector_entry.block_id
            ))
        })?;
    sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .find(|space| space.vector_space_id == block.vector_space_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing vector_space_id {}",
                block.block_id, block.vector_space_id
            ))
        })
}

pub(super) fn vector_entry_values_as_f32(
    artifact_bytes: &[u8],
    sidecar: &CoveAiFile,
    vector_space: &VectorSpaceDescriptorV1,
    vector_entry: &VectorEntryV1,
) -> Result<Vec<f32>, CoveError> {
    validate_runtime_vector_space(vector_space)?;
    let block = sidecar
        .descriptor_tables
        .vector_block(vector_entry.block_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorEntry {} references missing block_id {}",
                vector_entry.vector_ref, vector_entry.block_id
            ))
        })?;
    if block.vector_space_id != vector_space.vector_space_id {
        return Err(CoveError::BadSection(format!(
            "VectorEntry {} block vector_space_id {} does not match selected vector_space_id {}",
            vector_entry.vector_ref, block.vector_space_id, vector_space.vector_space_id
        )));
    }
    if block.dimension_count != vector_space.dimension_count
        || block.element_type != vector_space.element_type
    {
        return Err(CoveError::BadSection(format!(
            "VectorPayloadBlock {} does not match selected vector-space dimension/element type",
            block.block_id
        )));
    }
    if block.compression_codec != CompressionCodec::None as u8 || block.layout_kind != 0 {
        return Err(CoveError::BadSection(format!(
            "VectorPayloadBlock {} is not an uncompressed dense row-major block",
            block.block_id
        )));
    }

    let row_width = block.dense_vector_width().ok_or_else(|| {
        CoveError::BadSection(format!(
            "VectorPayloadBlock {} has unsupported dense vector width",
            block.block_id
        ))
    })?;
    let block_payload_len = if block.payload_length == 0 {
        row_width
            .checked_mul(block.vector_count)
            .ok_or(CoveError::ArithOverflow)?
    } else {
        block.payload_length
    };
    let payload_ref = sidecar
        .descriptor_tables
        .payload_ref(block.payload_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing payload_ref {}",
                block.block_id, block.payload_ref
            ))
        })?;
    if payload_ref.integrity_ref != 0 {
        exact_flat_verify_payload_integrity(artifact_bytes, sidecar, payload_ref.integrity_ref)?;
    }
    if block.integrity_ref != 0 {
        exact_flat_verify_payload_integrity(artifact_bytes, sidecar, block.integrity_ref)?;
    }
    if vector_entry.integrity_ref != 0 {
        exact_flat_verify_payload_integrity(artifact_bytes, sidecar, vector_entry.integrity_ref)?;
    }

    let payload_bytes = exact_flat_payload_ref_bytes(artifact_bytes, sidecar, payload_ref)?;
    let block_payload = checked_slice(payload_bytes, block.payload_offset, block_payload_len)?;
    let (entry_offset, entry_length) = if vector_entry.payload_length == 0 {
        (
            vector_entry
                .vector_ordinal
                .checked_mul(row_width)
                .ok_or(CoveError::ArithOverflow)?,
            row_width,
        )
    } else {
        (
            vector_entry.payload_offset,
            u64::from(vector_entry.payload_length),
        )
    };
    if entry_length != row_width {
        return Err(CoveError::BadSection(format!(
            "VectorEntry {} payload_length {} does not match dense row width {}",
            vector_entry.vector_ref, entry_length, row_width
        )));
    }
    let vector_bytes = checked_slice(block_payload, entry_offset, entry_length)?;
    decode_dense_vector_as_f32(vector_space, vector_entry.vector_ref, vector_bytes)
}

pub(super) fn decode_dense_vector_as_f32(
    vector_space: &VectorSpaceDescriptorV1,
    vector_ref: u64,
    vector_bytes: &[u8],
) -> Result<Vec<f32>, CoveError> {
    let mut values = Vec::with_capacity(vector_space.dimension_count as usize);
    match vector_space.element_type {
        0 => {
            for chunk in vector_bytes.chunks_exact(4) {
                values.push(f32::from_bits(read_u32(chunk, 0)?));
            }
        }
        1 => {
            for chunk in vector_bytes.chunks_exact(2) {
                let bits = read_u16(chunk, 0)?;
                values.push(f16_to_f32(bits));
            }
        }
        2 => {
            for chunk in vector_bytes.chunks_exact(2) {
                let bits = read_u16(chunk, 0)?;
                values.push(f32::from_bits(u32::from(bits) << 16));
            }
        }
        3 => values.extend(vector_bytes.iter().map(|value| (*value as i8) as f32)),
        4 => values.extend(vector_bytes.iter().map(|value| f32::from(*value))),
        6 => {
            for chunk in vector_bytes.chunks_exact(8) {
                let value = f64::from_bits(read_u64(chunk, 0)?);
                if !value.is_finite() || value > f64::from(f32::MAX) || value < f64::from(f32::MIN)
                {
                    return Err(CoveError::BadSection(format!(
                        "VectorEntry {vector_ref} contains Float64 values outside finite Float32 runtime range"
                    )));
                }
                values.push(value as f32);
            }
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "AI runtime vector search does not support element_type {}",
                vector_space.element_type
            )));
        }
    }
    if values.len() != vector_space.dimension_count as usize {
        return Err(CoveError::BadSection(format!(
            "VectorEntry {vector_ref} decoded dimension {} does not match vector space dimension {}",
            values.len(),
            vector_space.dimension_count
        )));
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(CoveError::BadSection(format!(
            "VectorEntry {vector_ref} contains non-finite values"
        )));
    }
    Ok(values)
}

pub(super) fn f16_to_f32(bits: u16) -> f32 {
    let sign = (u32::from(bits & 0x8000)) << 16;
    let exp = (bits & 0x7c00) >> 10;
    let frac = u32::from(bits & 0x03ff);
    let out = if exp == 0 {
        if frac == 0 {
            sign
        } else {
            let mut mantissa = frac;
            let mut exponent = -14i32;
            while mantissa & 0x0400 == 0 {
                mantissa <<= 1;
                exponent -= 1;
            }
            mantissa &= 0x03ff;
            let exp_bits = u32::try_from(exponent + 127).unwrap_or(0) << 23;
            sign | exp_bits | (mantissa << 13)
        }
    } else if exp == 0x1f {
        sign | 0x7f80_0000 | (frac << 13)
    } else {
        sign | ((u32::from(exp) + 112) << 23) | (frac << 13)
    };
    f32::from_bits(out)
}

pub(super) fn exact_flat_vector_entry_f32(
    artifact_bytes: &[u8],
    sidecar: &CoveAiFile,
    vector_space: &VectorSpaceDescriptorV1,
    vector_entry: &VectorEntryV1,
) -> Result<Vec<f32>, CoveError> {
    let block = sidecar
        .descriptor_tables
        .vector_block(vector_entry.block_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorEntry {} references missing block_id {}",
                vector_entry.vector_ref, vector_entry.block_id
            ))
        })?;
    if block.vector_space_id != vector_space.vector_space_id {
        return Err(CoveError::BadSection(format!(
            "VectorEntry {} block vector_space_id {} does not match selected vector_space_id {}",
            vector_entry.vector_ref, block.vector_space_id, vector_space.vector_space_id
        )));
    }
    if block.dimension_count != vector_space.dimension_count
        || block.element_type != vector_space.element_type
    {
        return Err(CoveError::BadSection(format!(
            "VectorPayloadBlock {} does not match selected vector-space dimension/element type",
            block.block_id
        )));
    }
    if block.compression_codec != CompressionCodec::None as u8
        || block.quantization_kind != 0
        || block.layout_kind != 0
    {
        return Err(CoveError::BadSection(format!(
            "VectorPayloadBlock {} is not an uncompressed dense row-major Float32 block",
            block.block_id
        )));
    }

    let row_width = block.dense_vector_width().ok_or_else(|| {
        CoveError::BadSection(format!(
            "VectorPayloadBlock {} has unsupported dense vector width",
            block.block_id
        ))
    })?;
    let block_payload_len = if block.payload_length == 0 {
        row_width
            .checked_mul(block.vector_count)
            .ok_or(CoveError::ArithOverflow)?
    } else {
        block.payload_length
    };
    let payload_ref = sidecar
        .descriptor_tables
        .payload_ref(block.payload_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing payload_ref {}",
                block.block_id, block.payload_ref
            ))
        })?;
    if payload_ref.integrity_ref != 0 {
        exact_flat_verify_payload_integrity(artifact_bytes, sidecar, payload_ref.integrity_ref)?;
    }
    if block.integrity_ref != 0 {
        exact_flat_verify_payload_integrity(artifact_bytes, sidecar, block.integrity_ref)?;
    }
    if vector_entry.integrity_ref != 0 {
        exact_flat_verify_payload_integrity(artifact_bytes, sidecar, vector_entry.integrity_ref)?;
    }

    let payload_bytes = exact_flat_payload_ref_bytes(artifact_bytes, sidecar, payload_ref)?;
    let block_payload = checked_slice(payload_bytes, block.payload_offset, block_payload_len)?;
    let (entry_offset, entry_length) = if vector_entry.payload_length == 0 {
        (
            vector_entry
                .vector_ordinal
                .checked_mul(row_width)
                .ok_or(CoveError::ArithOverflow)?,
            row_width,
        )
    } else {
        (
            vector_entry.payload_offset,
            u64::from(vector_entry.payload_length),
        )
    };
    if entry_length != row_width {
        return Err(CoveError::BadSection(format!(
            "VectorEntry {} payload_length {} does not match dense row width {}",
            vector_entry.vector_ref, entry_length, row_width
        )));
    }
    let vector_bytes = checked_slice(block_payload, entry_offset, entry_length)?;
    let mut values = Vec::with_capacity(vector_space.dimension_count as usize);
    for chunk in vector_bytes.chunks_exact(4) {
        let value = f32::from_bits(read_u32(chunk, 0)?);
        if !value.is_finite() {
            return Err(CoveError::BadSection(format!(
                "VectorEntry {} contains non-finite Float32 values",
                vector_entry.vector_ref
            )));
        }
        values.push(value);
    }
    Ok(values)
}

pub(super) fn exact_flat_payload_ref_bytes<'a>(
    artifact_bytes: &'a [u8],
    sidecar: &CoveAiFile,
    payload_ref: &AiPayloadRefEntryV1,
) -> Result<&'a [u8], CoveError> {
    if payload_ref.decoded_length != payload_ref.payload_length {
        return Err(CoveError::BadSection(format!(
            "payload_ref {} has decoded_length different from payload_length; exact flat scan only supports uncompressed payloads",
            payload_ref.payload_ref
        )));
    }
    match AiStorageKindV1::from_u8(payload_ref.storage_kind)
        .ok_or_else(|| CoveError::BadSection("unknown AI payload storage_kind".into()))?
    {
        AiStorageKindV1::ArtifactAbsolute => checked_slice(
            artifact_bytes,
            payload_ref.payload_offset,
            payload_ref.payload_length,
        ),
        AiStorageKindV1::SectionDecodedRelative => {
            let section =
                section_by_id(&sidecar.sections, payload_ref.section_id).ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "payload_ref {} references missing section_id {}",
                        payload_ref.payload_ref, payload_ref.section_id
                    ))
                })?;
            if section.entry.section_kind != SectionKind::AiPayloadBytes as u32 {
                return Err(CoveError::BadSection(format!(
                    "payload_ref {} SectionDecodedRelative target is not AI_PAYLOAD_BYTES",
                    payload_ref.payload_ref
                )));
            }
            if section.entry.compression != CompressionCodec::None as u8 {
                return Err(CoveError::UnsupportedEncoding(
                    "exact flat vector scan supports only uncompressed AI_PAYLOAD_BYTES".into(),
                ));
            }
            let section_payload =
                checked_slice(artifact_bytes, section.entry.offset, section.entry.length)?;
            checked_slice(
                section_payload,
                payload_ref.section_payload_offset,
                payload_ref.payload_length,
            )
        }
        AiStorageKindV1::ExternalUri | AiStorageKindV1::EmbeddedSection => {
            Err(CoveError::BadSection(format!(
                "payload_ref {} is not directly readable from AI_PAYLOAD_BYTES",
                payload_ref.payload_ref
            )))
        }
        AiStorageKindV1::Extension => Err(CoveError::BadExtension),
    }
}

pub(super) fn exact_flat_verify_payload_integrity(
    artifact_bytes: &[u8],
    sidecar: &CoveAiFile,
    integrity_ref: u32,
) -> Result<(), CoveError> {
    let integrity = sidecar
        .descriptor_tables
        .integrity_ref(integrity_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "exact flat vector scan references missing integrity_ref {integrity_ref}"
            ))
        })?;
    if integrity.payload_crc32c == 0 {
        return Ok(());
    }
    let payload_ref = sidecar
        .descriptor_tables
        .payload_ref(integrity.payload_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "AI_PAYLOAD_INTEGRITY {} references missing payload_ref {}",
                integrity.integrity_ref, integrity.payload_ref
            ))
        })?;
    let payload_bytes = exact_flat_payload_ref_bytes(artifact_bytes, sidecar, payload_ref)?;
    if checksum::crc32c(payload_bytes) != integrity.payload_crc32c {
        return Err(CoveError::ChecksumMismatch);
    }
    Ok(())
}

pub(super) fn exact_flat_metric_score(
    metric: u8,
    query: &[f32],
    vector: &[f32],
) -> Result<f32, CoveError> {
    if query.len() != vector.len() {
        return Err(CoveError::BadSection(format!(
            "query dimension {} does not match vector dimension {}",
            query.len(),
            vector.len()
        )));
    }
    let mut dot = 0.0f64;
    let mut query_norm_sq = 0.0f64;
    let mut vector_norm_sq = 0.0f64;
    let mut l2 = 0.0f64;
    let mut l1 = 0.0f64;
    for (query_value, vector_value) in query.iter().zip(vector) {
        let query_value = f64::from(*query_value);
        let vector_value = f64::from(*vector_value);
        dot += query_value * vector_value;
        query_norm_sq += query_value * query_value;
        vector_norm_sq += vector_value * vector_value;
        let diff = query_value - vector_value;
        l2 += diff * diff;
        l1 += diff.abs();
    }
    let score = match metric {
        0 => {
            if query_norm_sq == 0.0 || vector_norm_sq == 0.0 {
                return Err(CoveError::BadSection(
                    "cosine vector search requires non-zero query and vector norms".into(),
                ));
            }
            dot / (query_norm_sq.sqrt() * vector_norm_sq.sqrt())
        }
        1 => dot,
        2 => -l2,
        3 => -l1,
        _ => {
            return Err(CoveError::BadSection(format!(
                "exact flat FileCode vector search does not support vector metric {metric}"
            )));
        }
    };
    if !score.is_finite() || score > f64::from(f32::MAX) || score < f64::from(f32::MIN) {
        return Err(CoveError::BadSection(
            "exact flat vector search produced a non-finite score".into(),
        ));
    }
    Ok(score as f32)
}

pub fn write_coveai_artifact(
    artifact_kind: CoveAiArtifactKind,
    artifact_id: [u8; 16],
    created_at_us: i64,
    sections: &[CoveAiWritableSection],
) -> Result<Vec<u8>, CoveError> {
    let mut out = Vec::new();
    let mut entries = Vec::with_capacity(sections.len());
    for section in sections {
        let offset = out.len() as u64;
        out.extend_from_slice(&section.payload);
        let payload_crc32c = if section.payload_encoding == AiPayloadEncodingV1::OpaqueBytes {
            0
        } else {
            checksum::crc32c(&section.payload)
        };
        entries.push(CoveAiSectionEntryV1 {
            section_id: section.section_id,
            section_kind: section.section_kind,
            offset,
            length: section.payload.len() as u64,
            uncompressed_length: section.payload.len() as u64,
            compression: CompressionCodec::None as u8,
            payload_encoding: section.payload_encoding as u8,
            requiredness_scope: section.requiredness_scope as u8,
            profile_kind: section.profile_kind,
            source_binding_ref: section.source_binding_ref,
            required_ai_features: section.required_ai_features,
            optional_ai_features: section.optional_ai_features,
            feature_binding_ref: section.feature_binding_ref,
            payload_crc32c,
        });
    }

    let header_offset = out.len() as u64;
    let mut section_directory =
        Vec::with_capacity(entries.len() * COVEAI_SECTION_ENTRY_LEN as usize);
    for entry in &entries {
        section_directory.extend_from_slice(&entry.serialize()?);
    }
    let header = CoveAiHeaderV1 {
        magic: artifact_kind.magic(),
        header_len: COVEAI_HEADER_LEN,
        version_major: COVEAI_VERSION_MAJOR_V1,
        version_minor: COVEAI_VERSION_MINOR_V1,
        flags: 0,
        artifact_id,
        required_ai_features: 0,
        optional_ai_features: 0,
        section_count: entries.len() as u32,
        section_entry_len: COVEAI_SECTION_ENTRY_LEN,
        reserved0: 0,
        created_at_us,
        section_directory_crc32c: 0,
        crc32c: 0,
    };
    out.extend_from_slice(&header.serialize(&section_directory)?);
    out.extend_from_slice(&section_directory);
    let header_length = u64::from(COVEAI_HEADER_LEN)
        + u64::try_from(section_directory.len()).map_err(|_| CoveError::ArithOverflow)?;
    let file_len =
        out.len() as u64 + u64::from(COVEAI_POSTSCRIPT_LEN) + COVEAI_POSTSCRIPT_TAIL_SIZE as u64;
    let postscript = CoveAiPostscriptV1 {
        required_ai_features: 0,
        optional_ai_features: 0,
        file_len,
        header_offset,
        header_length,
        crc32c: 0,
    };
    out.extend_from_slice(&postscript.serialize());
    out.extend_from_slice(&COVEAI_POSTSCRIPT_VERSION_V1.to_le_bytes());
    out.extend_from_slice(&COVEAI_POSTSCRIPT_LEN.to_le_bytes());
    out.extend_from_slice(&artifact_kind.magic());
    Ok(out)
}

pub fn write_coveai_descriptor_bundle(
    build: &CoveAiDescriptorBundleBuild,
) -> Result<Vec<u8>, CoveError> {
    let tables = &build.descriptor_tables;

    let mut sections = build.payload_sections.clone();
    let mut next_section_id = 1_000u32;
    let push_section = |sections: &mut Vec<CoveAiWritableSection>,
                        next_section_id: &mut u32,
                        section_kind: SectionKind,
                        profile_kind: PrimaryProfile,
                        records: Vec<Vec<u8>>|
     -> Result<(), CoveError> {
        if records.is_empty() {
            return Ok(());
        }
        let section_id = *next_section_id;
        *next_section_id = next_section_id
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        sections.push(coveai_binary_section(
            section_id,
            section_kind,
            profile_kind,
            encode_records(records),
        ));
        Ok(())
    };

    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiCompanionArtifactRef,
        PrimaryProfile::CoveAiShared,
        tables
            .companion_artifacts
            .iter()
            .cloned()
            .map(encode_companion_artifact_ref)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiSourceBinding,
        PrimaryProfile::CoveAiShared,
        tables
            .source_bindings
            .iter()
            .cloned()
            .map(encode_source_binding)
            .collect::<Result<Vec<_>, _>>()?,
    )?;

    let mut reference_records = Vec::new();
    for record in &tables.strings {
        reference_records.push(encode_string_entry(record.clone())?);
    }
    for record in &tables.digests {
        reference_records.push(encode_digest_entry(record.clone())?);
    }
    for record in &tables.payload_refs {
        reference_records.push(encode_payload_ref_entry(record.clone())?);
    }
    for record in &tables.policies {
        reference_records.push(encode_policy_ref_entry(record.clone())?);
    }
    for record in &tables.source_spans {
        reference_records.push(encode_source_span_entry(record.clone())?);
    }
    for record in &tables.transforms {
        reference_records.push(encode_transform_entry(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiReferenceTables,
        PrimaryProfile::CoveAiShared,
        reference_records,
    )?;

    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiPrivacySummary,
        PrimaryProfile::CoveAiShared,
        tables
            .privacy_summaries
            .iter()
            .cloned()
            .map(encode_privacy_summary)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiPayloadIntegrity,
        PrimaryProfile::CoveAiShared,
        tables
            .payload_integrity
            .iter()
            .cloned()
            .map(encode_payload_integrity)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiSectionFeatureBinding,
        PrimaryProfile::CoveAiShared,
        tables
            .section_feature_bindings
            .iter()
            .cloned()
            .map(encode_section_feature_binding)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiChunkProfile,
        PrimaryProfile::CoveChunk,
        tables
            .chunk_profiles
            .iter()
            .cloned()
            .map(encode_chunk_profile)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTextChunkIndex,
        PrimaryProfile::CoveChunk,
        tables
            .text_chunks
            .iter()
            .cloned()
            .map(encode_text_chunk_entry)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTokenizerProfile,
        PrimaryProfile::CoveTok,
        tables
            .tokenizer_profiles
            .iter()
            .cloned()
            .map(encode_tokenizer_profile)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTokenBlock,
        PrimaryProfile::CoveTok,
        tables
            .token_blocks
            .iter()
            .cloned()
            .map(encode_token_block)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTokenizedSpan,
        PrimaryProfile::CoveTok,
        tables
            .tokenized_spans
            .iter()
            .cloned()
            .map(encode_tokenized_span)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTokenSequencePack,
        PrimaryProfile::CoveTok,
        tables
            .token_sequence_packs
            .iter()
            .cloned()
            .map(encode_token_sequence_pack)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTrainingProfile,
        PrimaryProfile::CoveTrain,
        tables
            .training_profiles
            .iter()
            .cloned()
            .map(encode_training_profile)
            .collect::<Result<Vec<_>, _>>()?,
    )?;

    let mut training_policy_records = Vec::new();
    for record in &tables.dataset_splits {
        training_policy_records.push(encode_dataset_split(record.clone())?);
    }
    for record in &tables.dedup_groups {
        training_policy_records.push(encode_dedup_group(record.clone())?);
    }
    for record in &tables.training_epoch_plans {
        training_policy_records.push(encode_training_epoch_plan(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTrainingSplitDedupEpoch,
        PrimaryProfile::CoveTrain,
        training_policy_records,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTrainingSampleIndex,
        PrimaryProfile::CoveTrain,
        tables
            .training_samples
            .iter()
            .cloned()
            .map(encode_training_sample_entry)
            .collect::<Result<Vec<_>, _>>()?,
    )?;

    let mut label_records = Vec::new();
    for record in &tables.training_labels {
        label_records.push(encode_training_label_entry(record.clone())?);
    }
    for record in &tables.preference_pairs {
        label_records.push(encode_preference_pair_entry(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiLabelPreference,
        PrimaryProfile::CoveTrain,
        label_records,
    )?;

    let mut provenance_records = Vec::new();
    for record in &tables.generator_provenance {
        provenance_records.push(encode_generator_provenance(record.clone())?);
    }
    for record in &tables.model_actors {
        provenance_records.push(encode_model_actor(record.clone())?);
    }
    for record in &tables.generation_decoding_profiles {
        provenance_records.push(encode_generation_decoding_profile(record.clone())?);
    }
    for record in &tables.human_reviews {
        provenance_records.push(encode_human_review(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiGeneratorProvenance,
        PrimaryProfile::CoveTrain,
        provenance_records,
    )?;

    let mut tensor_records = Vec::new();
    for record in &tables.tensor_layouts {
        tensor_records.push(encode_tensor_layout(record.clone())?);
    }
    for record in &tables.device_transfer_hints {
        tensor_records.push(encode_device_transfer_hint(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiTensorLayout,
        PrimaryProfile::CoveMmseq,
        tensor_records,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiAssetManifest,
        PrimaryProfile::CoveMmseq,
        tables
            .assets
            .iter()
            .cloned()
            .map(encode_ai_asset_ref)
            .collect::<Result<Vec<_>, _>>()?,
    )?;

    let mut multimodal_records = Vec::new();
    for record in &tables.multimodal_sequence_packs {
        multimodal_records.push(encode_multimodal_sequence_pack(record.clone())?);
    }
    for record in &tables.multimodal_sequence_elements {
        multimodal_records.push(encode_multimodal_sequence_element(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiMultimodalSequence,
        PrimaryProfile::CoveMmseq,
        multimodal_records,
    )?;

    let mut vector_space_records = Vec::new();
    for record in &tables.vector_spaces {
        vector_space_records.push(encode_vector_space(record.clone())?);
    }
    for record in &tables.vector_space_compatibilities {
        vector_space_records.push(encode_vector_space_compatibility(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiVectorSpace,
        PrimaryProfile::CoveVec,
        vector_space_records,
    )?;

    let mut vector_binding_records = Vec::new();
    for record in &tables.filecode_vector_bindings {
        vector_binding_records.push(encode_filecode_vector_binding(record.clone())?);
    }
    for record in &tables.chunk_vector_bindings {
        vector_binding_records.push(encode_chunk_vector_binding(record.clone())?);
    }
    for record in &tables.object_state_vector_bindings {
        vector_binding_records.push(encode_object_state_vector_binding(record.clone())?);
    }
    for record in &tables.training_sample_vector_bindings {
        vector_binding_records.push(encode_training_sample_vector_binding(record.clone())?);
    }
    for record in &tables.association_state_vector_bindings {
        vector_binding_records.push(encode_association_state_vector_binding(record.clone())?);
    }
    for record in &tables.asset_vector_bindings {
        vector_binding_records.push(encode_asset_vector_binding(record.clone())?);
    }
    for record in &tables.multimodal_sequence_vector_bindings {
        vector_binding_records.push(encode_multimodal_sequence_vector_binding(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiVectorBinding,
        PrimaryProfile::CoveVec,
        vector_binding_records,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiVectorPayloadBlock,
        PrimaryProfile::CoveVec,
        tables
            .vector_payload_blocks
            .iter()
            .cloned()
            .map(encode_vector_payload_block)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiVectorDirectory,
        PrimaryProfile::CoveVec,
        tables
            .vector_entries
            .iter()
            .cloned()
            .map(encode_vector_entry)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    let mut vector_composition_records = Vec::new();
    for record in &tables.vector_composition_profiles {
        vector_composition_records.push(encode_vector_composition_profile(record.clone())?);
    }
    for record in &tables.vector_composition_components {
        vector_composition_records.push(encode_vector_composition_component(record.clone())?);
    }
    for record in &tables.vector_arithmetic_profiles {
        vector_composition_records.push(encode_vector_arithmetic_profile(record.clone())?);
    }
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiVectorComposition,
        PrimaryProfile::CoveVec,
        vector_composition_records,
    )?;
    push_section(
        &mut sections,
        &mut next_section_id,
        SectionKind::AiVectorIndex,
        PrimaryProfile::CoveVec,
        tables
            .vector_indexes
            .iter()
            .cloned()
            .map(encode_vector_index)
            .collect::<Result<Vec<_>, _>>()?,
    )?;

    let bytes = write_coveai_artifact(
        CoveAiArtifactKind::CoveAiBundle,
        build.artifact_id,
        build.created_at_us,
        &sections,
    )?;
    CoveAiFile::parse(&bytes)?;
    Ok(bytes)
}

pub(super) fn coveai_binary_section(
    section_id: u32,
    section_kind: SectionKind,
    profile_kind: PrimaryProfile,
    payload: Vec<u8>,
) -> CoveAiWritableSection {
    CoveAiWritableSection {
        section_id,
        section_kind: section_kind as u32,
        profile_kind: profile_kind as u8,
        payload_encoding: AiPayloadEncodingV1::BinaryRecords,
        requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
        source_binding_ref: 0,
        required_ai_features: 0,
        optional_ai_features: optional_features_for_ai_section(section_kind),
        feature_binding_ref: 0,
        payload,
    }
}

pub(super) fn coveai_payload_section(
    section_id: u32,
    section_kind: SectionKind,
    profile_kind: PrimaryProfile,
    payload: Vec<u8>,
) -> CoveAiWritableSection {
    CoveAiWritableSection {
        section_id,
        section_kind: section_kind as u32,
        profile_kind: profile_kind as u8,
        payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
        requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
        source_binding_ref: 0,
        required_ai_features: 0,
        optional_ai_features: optional_features_for_ai_section(section_kind),
        feature_binding_ref: 0,
        payload,
    }
}

pub(super) fn optional_features_for_ai_section(section_kind: SectionKind) -> u64 {
    match section_kind {
        SectionKind::AiChunkProfile | SectionKind::AiTextChunkIndex => AI_FEATURE_CHUNK,
        SectionKind::AiTokenizerProfile
        | SectionKind::AiTokenBlock
        | SectionKind::AiTokenizedSpan
        | SectionKind::AiTokenSequencePack => AI_FEATURE_TOKEN,
        SectionKind::AiVectorSpace
        | SectionKind::AiVectorBinding
        | SectionKind::AiVectorPayloadBlock
        | SectionKind::AiVectorComposition
        | SectionKind::AiVectorIndex
        | SectionKind::AiVectorDirectory => AI_FEATURE_VECTOR,
        SectionKind::AiPrivacySummary => AI_FEATURE_PRIVACY_SUMMARY,
        _ => 0,
    }
}

pub(super) fn encode_records(records: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        out.extend_from_slice(&record);
    }
    out
}

pub(super) fn encode_ai_record(
    record_kind: u16,
    local_id: u64,
    flags: u32,
    payload: &[u8],
) -> Result<Vec<u8>, CoveError> {
    encode_ai_record_with_version(record_kind, 1, local_id, flags, payload)
}

pub(super) fn encode_ai_record_with_version(
    record_kind: u16,
    record_version: u16,
    local_id: u64,
    flags: u32,
    payload: &[u8],
) -> Result<Vec<u8>, CoveError> {
    let record_len = AI_RECORD_HEADER_LEN
        .checked_add(payload.len())
        .ok_or(CoveError::ArithOverflow)?;
    let record_len_u32 = u32::try_from(record_len).map_err(|_| CoveError::ArithOverflow)?;
    let mut out = vec![0u8; record_len];
    put_u16(&mut out, 0, record_kind);
    put_u16(&mut out, 2, record_version);
    put_u32(&mut out, 4, record_len_u32);
    put_u64(&mut out, 8, local_id);
    put_u32(&mut out, 16, flags);
    out[AI_RECORD_HEADER_LEN..].copy_from_slice(payload);
    let crc32c = checksum_with_zeroed_field(&out, 20)?;
    put_u32(&mut out, 20, crc32c);
    Ok(out)
}

pub(super) fn with_payload_crc32c(
    mut payload: Vec<u8>,
    checksum_offset: usize,
) -> Result<Vec<u8>, CoveError> {
    let crc32c = checksum_with_zeroed_field(&payload, checksum_offset)?;
    put_u32(&mut payload, checksum_offset, crc32c);
    Ok(payload)
}

pub(super) fn checksum_with_zeroed_field(
    bytes: &[u8],
    checksum_offset: usize,
) -> Result<u32, CoveError> {
    let end = checksum_offset
        .checked_add(4)
        .ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let mut for_crc = bytes.to_vec();
    for_crc[checksum_offset..end].fill(0);
    Ok(checksum::crc32c(&for_crc))
}

pub(super) fn encode_string_entry(record: AiStringEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 20];
    put_u32(&mut payload, 0, record.string_ref);
    put_u32(&mut payload, 4, record.utf8_byte_length);
    put_u32(&mut payload, 8, record.payload_ref);
    put_u32(&mut payload, 12, record.flags);
    let payload = with_payload_crc32c(payload, 16)?;
    encode_ai_record(
        1,
        u64::from(record.string_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_digest_entry(record: AiDigestEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 21];
    put_u32(&mut payload, 0, record.digest_ref);
    put_u16(&mut payload, 4, record.digest_algorithm);
    put_u16(&mut payload, 6, record.digest_len);
    put_u32(&mut payload, 8, record.digest_payload_ref);
    put_u8(&mut payload, 12, record.domain_hint);
    put_u32(&mut payload, 13, record.flags);
    let payload = with_payload_crc32c(payload, 17)?;
    encode_ai_record(
        2,
        u64::from(record.digest_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_companion_artifact_ref(
    record: AiCompanionArtifactRefV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 61];
    put_u32(&mut payload, 0, record.artifact_ref);
    put_u8(&mut payload, 4, record.artifact_kind);
    payload[5..21].copy_from_slice(&record.artifact_id);
    put_u32(&mut payload, 21, record.uri_ref);
    put_u32(&mut payload, 25, record.artifact_digest_ref);
    put_u32(&mut payload, 29, record.source_binding_ref);
    put_u64(&mut payload, 33, record.required_ai_features);
    put_u64(&mut payload, 41, record.optional_ai_features);
    put_u32(&mut payload, 49, record.flags);
    let payload = with_payload_crc32c(payload, 57)?;
    encode_ai_record(
        1,
        u64::from(record.artifact_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_source_binding(record: AiSourceBindingV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 61];
    put_u32(&mut payload, 0, record.source_binding_id);
    put_u8(&mut payload, 4, record.source_kind);
    put_u32(&mut payload, 5, record.source_artifact_ref);
    put_u32(&mut payload, 9, record.source_file_digest_ref);
    put_u32(&mut payload, 13, record.covm_snapshot_ref);
    put_u32(&mut payload, 17, record.schema_fingerprint_ref);
    put_u32(&mut payload, 21, record.dictionary_digest_ref);
    put_u32(&mut payload, 25, record.map_fingerprint_ref);
    put_u32(&mut payload, 29, record.policy_context_ref);
    put_u32(&mut payload, 33, record.visibility_scope_ref);
    put_u32(&mut payload, 37, record.redaction_scope_ref);
    put_u32(&mut payload, 41, record.branch_ref);
    put_u64(&mut payload, 45, record.as_of_csn);
    put_u32(&mut payload, 53, record.flags);
    let payload = with_payload_crc32c(payload, 57)?;
    encode_ai_record(
        1,
        u64::from(record.source_binding_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_payload_ref_entry(record: AiPayloadRefEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 61];
    put_u32(&mut payload, 0, record.payload_ref);
    put_u8(&mut payload, 4, record.storage_kind);
    put_u32(&mut payload, 5, record.media_type_ref);
    put_u32(&mut payload, 9, record.section_id);
    put_u32(&mut payload, 13, record.uri_ref);
    put_u64(&mut payload, 17, record.payload_offset);
    put_u64(&mut payload, 25, record.section_payload_offset);
    put_u64(&mut payload, 33, record.payload_length);
    put_u64(&mut payload, 41, record.decoded_length);
    put_u32(&mut payload, 49, record.integrity_ref);
    put_u32(&mut payload, 53, record.flags);
    let payload = with_payload_crc32c(payload, 57)?;
    encode_ai_record(
        3,
        u64::from(record.payload_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_policy_ref_entry(record: AiPolicyRefEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 25];
    put_u32(&mut payload, 0, record.policy_ref);
    put_u8(&mut payload, 4, record.policy_kind);
    put_u32(&mut payload, 5, record.authority_ref);
    put_u32(&mut payload, 9, record.payload_ref);
    put_u32(&mut payload, 13, record.digest_ref);
    put_u32(&mut payload, 17, record.flags);
    let payload = with_payload_crc32c(payload, 21)?;
    encode_ai_record(
        4,
        u64::from(record.policy_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_source_span_entry(record: AiSourceSpanEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 65];
    put_u32(&mut payload, 0, record.source_span_ref);
    put_u32(&mut payload, 4, record.source_binding_ref);
    put_u8(&mut payload, 8, record.source_kind);
    put_u64(&mut payload, 9, record.source_row_ref);
    put_u64(&mut payload, 17, record.source_object_ref);
    put_u64(&mut payload, 25, record.byte_start);
    put_u64(&mut payload, 33, record.byte_length);
    put_u64(&mut payload, 41, record.token_start);
    put_u32(&mut payload, 49, record.token_count);
    put_u32(&mut payload, 53, record.evidence_ref);
    put_u32(&mut payload, 57, record.flags);
    let payload = with_payload_crc32c(payload, 61)?;
    encode_ai_record(
        5,
        u64::from(record.source_span_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_transform_entry(record: AiTransformEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 33];
    put_u32(&mut payload, 0, record.transform_ref);
    put_u8(&mut payload, 4, record.transform_kind);
    put_u32(&mut payload, 5, record.function_or_template_ref);
    put_u32(&mut payload, 9, record.input_digest_ref);
    put_u32(&mut payload, 13, record.output_digest_ref);
    put_u32(&mut payload, 17, record.parameter_payload_ref);
    put_u32(&mut payload, 21, record.transform_digest_ref);
    put_u32(&mut payload, 25, record.flags);
    let payload = with_payload_crc32c(payload, 29)?;
    encode_ai_record(
        6,
        u64::from(record.transform_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_privacy_summary(
    record: AiPrivacySummaryEntryV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 38];
    put_u32(&mut payload, 0, record.privacy_summary_ref);
    put_u32(&mut payload, 4, record.source_binding_ref);
    put_u32(&mut payload, 8, record.sensitivity_mask);
    put_u32(&mut payload, 12, record.sensitivity_bits_ref);
    put_u32(&mut payload, 16, record.policy_ref);
    put_u32(&mut payload, 20, record.visibility_scope_ref);
    put_u32(&mut payload, 24, record.redaction_scope_ref);
    put_u8(&mut payload, 28, record.retention_state);
    put_u8(&mut payload, 29, record.disclosure_state);
    put_u32(&mut payload, 30, record.flags);
    let payload = with_payload_crc32c(payload, 34)?;
    encode_ai_record(
        1,
        u64::from(record.privacy_summary_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_payload_integrity(record: AiPayloadIntegrityV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 26];
    put_u32(&mut payload, 0, record.integrity_ref);
    put_u32(&mut payload, 4, record.payload_ref);
    put_u8(&mut payload, 8, record.digest_domain);
    put_u8(&mut payload, 9, record.reserved0);
    put_u16(&mut payload, 10, record.digest_algorithm);
    put_u16(&mut payload, 12, record.digest_len);
    put_u32(&mut payload, 14, record.digest_ref);
    put_u32(&mut payload, 18, record.payload_crc32c);
    put_u32(&mut payload, 22, record.flags);
    encode_ai_record(1, u64::from(record.integrity_ref), 0, &payload)
}

pub(super) fn encode_section_feature_binding(
    record: AiSectionFeatureBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 44];
    put_u32(&mut payload, 0, record.binding_ref);
    put_u32(&mut payload, 4, record.section_id);
    put_u8(&mut payload, 8, record.scope);
    put_u8(&mut payload, 9, record.profile_kind);
    put_u16(&mut payload, 10, record.operation_kind);
    put_u64(&mut payload, 12, record.required_ai_features);
    put_u64(&mut payload, 20, record.optional_ai_features);
    put_u64(&mut payload, 28, record.target_local_ref);
    put_u32(&mut payload, 36, record.flags);
    let payload = with_payload_crc32c(payload, 40)?;
    encode_ai_record(
        1,
        u64::from(record.binding_ref),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_chunk_profile(record: ChunkProfileV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 60];
    put_u32(&mut payload, 0, record.chunk_profile_id);
    put_u32(&mut payload, 4, record.profile_name_ref);
    put_u32(&mut payload, 8, record.chunker_namespace_ref);
    put_u32(&mut payload, 12, record.chunker_name_ref);
    put_u16(&mut payload, 16, record.chunker_version_major);
    put_u16(&mut payload, 18, record.chunker_version_minor);
    put_u32(&mut payload, 20, record.tokenizer_profile_ref);
    put_u8(&mut payload, 24, record.boundary_kind);
    put_u8(&mut payload, 25, record.overlap_policy);
    put_u8(&mut payload, 26, record.parent_policy);
    put_u8(&mut payload, 27, record.normalization_policy);
    put_u32(&mut payload, 28, record.target_tokens);
    put_u32(&mut payload, 32, record.min_tokens);
    put_u32(&mut payload, 36, record.max_tokens);
    put_u32(&mut payload, 40, record.overlap_tokens);
    put_u32(&mut payload, 44, record.max_bytes);
    put_u32(&mut payload, 48, record.locale_ref);
    put_u32(&mut payload, 52, record.flags);
    let payload = with_payload_crc32c(payload, 56)?;
    encode_ai_record(
        1,
        u64::from(record.chunk_profile_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_text_chunk_entry(record: TextChunkEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 152];
    put_u64(&mut payload, 0, record.chunk_id);
    put_u32(&mut payload, 8, record.source_ref);
    put_u32(&mut payload, 12, record.table_id);
    put_u32(&mut payload, 16, record.column_id);
    put_u32(&mut payload, 20, record.object_type_id);
    put_u32(&mut payload, 24, record.property_id);
    put_u32(&mut payload, 28, record.association_type_id);
    put_u32(&mut payload, 32, record.path_ref);
    put_u64(&mut payload, 36, record.source_row_ref);
    put_u64(&mut payload, 44, record.source_object_ref);
    put_u32(&mut payload, 52, record.source_value_hash_ref);
    put_u64(&mut payload, 56, record.byte_start);
    put_u64(&mut payload, 64, record.byte_length);
    put_u64(&mut payload, 72, record.unicode_scalar_start);
    put_u64(&mut payload, 80, record.unicode_scalar_length);
    put_u64(&mut payload, 88, record.token_start);
    put_u32(&mut payload, 96, record.token_count);
    put_u64(&mut payload, 100, record.parent_chunk_id);
    put_u32(&mut payload, 108, record.first_child_ref);
    put_u32(&mut payload, 112, record.child_count);
    put_u64(&mut payload, 116, record.previous_chunk_id);
    put_u64(&mut payload, 124, record.next_chunk_id);
    put_u32(&mut payload, 132, record.chunk_text_hash_ref);
    put_u32(&mut payload, 136, record.evidence_ref);
    put_u32(&mut payload, 140, record.policy_ref);
    put_u32(&mut payload, 144, record.flags);
    let payload = with_payload_crc32c(payload, 148)?;
    encode_ai_record(1, record.chunk_id, AI_FLAG_PAYLOAD_CRC32C_PRESENT, &payload)
}

pub(super) fn encode_tokenizer_profile(record: TokenizerProfileV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 92];
    put_u32(&mut payload, 0, record.tokenizer_profile_id);
    put_u32(&mut payload, 4, record.tokenizer_namespace_ref);
    put_u32(&mut payload, 8, record.tokenizer_name_ref);
    put_u16(&mut payload, 12, record.tokenizer_version_major);
    put_u16(&mut payload, 14, record.tokenizer_version_minor);
    put_u32(&mut payload, 16, record.vocab_digest_ref);
    put_u32(&mut payload, 20, record.merges_digest_ref);
    put_u32(&mut payload, 24, record.pre_tokenizer_digest_ref);
    put_u32(&mut payload, 28, record.normalizer_digest_ref);
    put_u32(&mut payload, 32, record.byte_encoder_digest_ref);
    put_u32(&mut payload, 36, record.special_tokens_digest_ref);
    put_u32(&mut payload, 40, record.added_tokens_digest_ref);
    put_u32(&mut payload, 44, record.chat_template_ref);
    put_u32(&mut payload, 48, record.unicode_version_ref);
    put_u32(&mut payload, 52, record.truncation_policy_ref);
    put_u32(&mut payload, 56, record.padding_policy_ref);
    put_u32(&mut payload, 60, record.model_max_sequence_length);
    put_u8(&mut payload, 64, record.token_id_width);
    put_u8(&mut payload, 65, record.byte_alignment_available);
    put_u8(&mut payload, 66, record.reversible);
    put_u8(&mut payload, 67, record.deterministic);
    put_u32(&mut payload, 68, record.bos_token_id);
    put_u32(&mut payload, 72, record.eos_token_id);
    put_u32(&mut payload, 76, record.pad_token_id);
    put_u32(&mut payload, 80, record.unk_token_id);
    put_u32(&mut payload, 84, record.flags);
    let payload = with_payload_crc32c(payload, 88)?;
    encode_ai_record(
        1,
        u64::from(record.tokenizer_profile_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_token_block(record: TokenBlockHeaderV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 49];
    put_u32(&mut payload, 0, record.token_block_id);
    put_u32(&mut payload, 4, record.tokenizer_profile_id);
    put_u64(&mut payload, 8, record.token_count);
    put_u8(&mut payload, 16, record.token_id_width);
    put_u8(&mut payload, 17, record.compression_codec);
    put_u8(&mut payload, 18, record.layout_kind);
    put_u32(&mut payload, 19, record.payload_ref);
    put_u64(&mut payload, 23, record.payload_offset);
    put_u64(&mut payload, 31, record.payload_length);
    put_u32(&mut payload, 39, record.integrity_ref);
    let payload = with_payload_crc32c(payload, 45)?;
    encode_ai_record(
        1,
        u64::from(record.token_block_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_tokenized_span(record: TokenizedSpanV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 52];
    put_u64(&mut payload, 0, record.tokenized_span_id);
    put_u64(&mut payload, 8, record.chunk_id);
    put_u32(&mut payload, 16, record.tokenizer_profile_id);
    put_u32(&mut payload, 20, record.token_block_ref);
    put_u64(&mut payload, 24, record.token_offset);
    put_u32(&mut payload, 32, record.token_count);
    put_u32(&mut payload, 36, record.byte_alignment_ref);
    put_u32(&mut payload, 40, record.source_value_hash_ref);
    put_u32(&mut payload, 44, record.flags);
    let payload = with_payload_crc32c(payload, 48)?;
    encode_ai_record(
        1,
        record.tokenized_span_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_token_sequence_pack(
    record: TokenSequencePackV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 72];
    put_u64(&mut payload, 0, record.sequence_pack_id);
    put_u32(&mut payload, 8, record.tokenizer_profile_id);
    put_u32(&mut payload, 12, record.training_profile_ref);
    put_u32(&mut payload, 16, record.token_block_ref);
    put_u64(&mut payload, 20, record.token_offset);
    put_u32(&mut payload, 28, record.token_count);
    put_u32(&mut payload, 32, record.source_span_count);
    put_u32(&mut payload, 36, record.first_source_span_ref);
    put_u32(&mut payload, 40, record.loss_mask_ref);
    put_u32(&mut payload, 44, record.attention_mask_ref);
    put_u32(&mut payload, 48, record.position_ids_ref);
    put_u32(&mut payload, 52, record.labels_ref);
    put_u32(&mut payload, 56, record.split_ref);
    put_u32(&mut payload, 60, record.sample_weight_ppm);
    put_u32(&mut payload, 64, record.flags);
    let payload = with_payload_crc32c(payload, 68)?;
    encode_ai_record(
        1,
        record.sequence_pack_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_training_profile(record: TrainingProfileV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 78];
    put_u32(&mut payload, 0, record.training_profile_id);
    put_u32(&mut payload, 4, record.profile_name_ref);
    put_u8(&mut payload, 8, record.task_family);
    put_u32(&mut payload, 9, record.modality_mask);
    put_u32(&mut payload, 13, record.source_snapshot_ref);
    put_u32(&mut payload, 17, record.map_profile_ref);
    put_u32(&mut payload, 21, record.chunk_profile_ref);
    put_u32(&mut payload, 25, record.tokenizer_profile_ref);
    put_u32(&mut payload, 29, record.vector_space_ref);
    put_u32(&mut payload, 33, record.multimodal_sequence_profile_ref);
    put_u32(&mut payload, 37, record.split_policy_ref);
    put_u32(&mut payload, 41, record.sampling_policy_ref);
    put_u32(&mut payload, 45, record.dedup_policy_ref);
    put_u32(&mut payload, 49, record.quality_policy_ref);
    put_u32(&mut payload, 53, record.license_policy_ref);
    put_u32(&mut payload, 57, record.redaction_policy_ref);
    put_u64(&mut payload, 61, record.default_generator_provenance_ref);
    put_u8(&mut payload, 69, record.reproducibility_class);
    put_u32(&mut payload, 70, record.flags);
    let payload = with_payload_crc32c(payload, 74)?;
    encode_ai_record(
        1,
        u64::from(record.training_profile_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_training_sample_entry(
    record: TrainingSampleEntryV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 121];
    put_u64(&mut payload, 0, record.sample_id);
    put_u32(&mut payload, 8, record.training_profile_id);
    put_u8(&mut payload, 12, record.example_kind);
    put_u32(&mut payload, 13, record.split_ref);
    put_u32(&mut payload, 17, record.source_ref);
    put_u32(&mut payload, 21, record.evidence_ref);
    put_u32(&mut payload, 25, record.input_ref);
    put_u32(&mut payload, 29, record.target_ref);
    put_u32(&mut payload, 33, record.label_ref);
    put_u32(&mut payload, 37, record.metadata_ref);
    put_u64(&mut payload, 41, record.token_sequence_pack_ref);
    put_u64(&mut payload, 49, record.multimodal_sequence_pack_ref);
    put_u64(&mut payload, 57, record.vector_ref);
    put_u32(&mut payload, 65, record.quality_score_ppm);
    put_u32(&mut payload, 69, record.sample_weight_ppm);
    put_u32(&mut payload, 73, record.dedup_group_ref);
    put_u32(&mut payload, 77, record.license_ref);
    put_u32(&mut payload, 81, record.policy_ref);
    put_u32(&mut payload, 85, record.teacher_model_ref);
    put_u64(&mut payload, 89, record.generator_provenance_ref);
    put_u64(&mut payload, 97, record.judge_generator_provenance_ref);
    put_u64(&mut payload, 105, record.label_generator_provenance_ref);
    put_u32(&mut payload, 113, record.flags);
    let payload = with_payload_crc32c(payload, 117)?;
    encode_ai_record(
        1,
        record.sample_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_dataset_split(record: DatasetSplitV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 69];
    put_u32(&mut payload, 0, record.split_id);
    put_u32(&mut payload, 4, record.split_name_ref);
    put_u8(&mut payload, 8, record.split_method);
    put_u32(&mut payload, 9, record.source_snapshot_ref);
    put_u32(&mut payload, 13, record.filter_policy_ref);
    put_u64(&mut payload, 17, record.seed);
    put_u32(&mut payload, 25, record.hash_function_ref);
    put_u32(&mut payload, 29, record.stratification_path_ref);
    put_u32(&mut payload, 33, record.grouping_ref);
    put_u32(&mut payload, 37, record.ordering_policy_ref);
    put_u32(&mut payload, 41, record.dedup_policy_ref);
    put_u64(&mut payload, 45, record.sample_count);
    put_u64(&mut payload, 53, record.first_sample_ref);
    put_u32(&mut payload, 61, record.flags);
    let payload = with_payload_crc32c(payload, 65)?;
    encode_ai_record(
        1,
        u64::from(record.split_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_dedup_group(record: DedupGroupV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 42];
    put_u64(&mut payload, 0, record.dedup_group_id);
    put_u32(&mut payload, 8, record.dedup_policy_ref);
    put_u64(&mut payload, 12, record.canonical_member_sample_id);
    put_u8(&mut payload, 20, record.similarity_kind);
    put_u8(&mut payload, 21, record.dedup_authority);
    put_u32(&mut payload, 22, record.confidence_ppm);
    put_u32(&mut payload, 26, record.first_member_ref);
    put_u32(&mut payload, 30, record.member_count);
    put_u32(&mut payload, 34, record.flags);
    let payload = with_payload_crc32c(payload, 38)?;
    encode_ai_record(
        2,
        record.dedup_group_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_training_epoch_plan(
    record: TrainingEpochPlanV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 53];
    put_u64(&mut payload, 0, record.epoch_plan_id);
    put_u32(&mut payload, 8, record.training_profile_id);
    put_u32(&mut payload, 12, record.split_ref);
    put_u64(&mut payload, 16, record.seed);
    put_u8(&mut payload, 24, record.permutation_kind);
    put_u32(&mut payload, 25, record.rng_algorithm_ref);
    put_u32(&mut payload, 29, record.permutation_function_ref);
    put_u32(&mut payload, 33, record.shard_count);
    put_u32(&mut payload, 37, record.first_shard_ref);
    put_u32(&mut payload, 41, record.shard_ref_count);
    put_u32(&mut payload, 45, record.flags);
    let payload = with_payload_crc32c(payload, 49)?;
    encode_ai_record(
        3,
        record.epoch_plan_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_training_label_entry(
    record: TrainingLabelEntryV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 46];
    put_u64(&mut payload, 0, record.label_id);
    put_u8(&mut payload, 8, record.label_kind);
    put_u8(&mut payload, 9, record.label_authority);
    put_u32(&mut payload, 10, record.label_payload_ref);
    put_u64(&mut payload, 14, record.generator_provenance_ref);
    put_u32(&mut payload, 22, record.human_review_ref);
    put_u32(&mut payload, 26, record.confidence_ppm);
    put_u32(&mut payload, 30, record.evidence_ref);
    put_u32(&mut payload, 34, record.policy_ref);
    put_u32(&mut payload, 38, record.flags);
    let payload = with_payload_crc32c(payload, 42)?;
    encode_ai_record(1, record.label_id, AI_FLAG_PAYLOAD_CRC32C_PRESENT, &payload)
}

pub(super) fn encode_preference_pair_entry(
    record: PreferencePairEntryV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 56];
    put_u64(&mut payload, 0, record.preference_pair_id);
    put_u32(&mut payload, 8, record.prompt_ref);
    put_u32(&mut payload, 12, record.chosen_ref);
    put_u32(&mut payload, 16, record.rejected_ref);
    put_u64(&mut payload, 20, record.judge_generator_provenance_ref);
    put_u32(&mut payload, 28, record.human_review_ref);
    put_u32(&mut payload, 32, record.preference_strength_ppm);
    put_u32(&mut payload, 36, record.confidence_ppm);
    put_u32(&mut payload, 40, record.evidence_ref);
    put_u32(&mut payload, 44, record.policy_ref);
    put_u32(&mut payload, 48, record.flags);
    let payload = with_payload_crc32c(payload, 52)?;
    encode_ai_record(
        2,
        record.preference_pair_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_generator_provenance(
    record: GeneratorProvenanceV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 78];
    put_u64(&mut payload, 0, record.generator_provenance_id);
    put_u8(&mut payload, 8, record.generator_kind);
    put_u32(&mut payload, 9, record.model_actor_ref);
    put_u32(&mut payload, 13, record.prompt_template_ref);
    put_u32(&mut payload, 17, record.decoding_profile_ref);
    put_u32(&mut payload, 21, record.toolchain_ref);
    put_u32(&mut payload, 25, record.source_input_ref);
    put_u32(&mut payload, 29, record.source_context_ref);
    put_u64(&mut payload, 33, record.source_sample_ref);
    put_u64(&mut payload, 41, record.parent_generator_provenance_ref);
    put_i64(&mut payload, 49, record.generation_time_us);
    put_u32(&mut payload, 57, record.confidence_ppm);
    put_u32(&mut payload, 61, record.human_review_ref);
    put_u32(&mut payload, 65, record.policy_ref);
    put_u8(&mut payload, 69, record.reproducibility_class);
    put_u32(&mut payload, 70, record.flags);
    let payload = with_payload_crc32c(payload, 74)?;
    encode_ai_record(
        1,
        record.generator_provenance_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_model_actor(record: ModelActorDescriptorV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 56];
    put_u32(&mut payload, 0, record.model_actor_id);
    put_u32(&mut payload, 4, record.model_namespace_ref);
    put_u32(&mut payload, 8, record.model_name_ref);
    put_u32(&mut payload, 12, record.model_version_ref);
    put_u32(&mut payload, 16, record.model_checkpoint_digest_ref);
    put_u32(&mut payload, 20, record.provider_ref);
    put_u32(&mut payload, 24, record.endpoint_ref);
    put_u32(&mut payload, 28, record.endpoint_version_ref);
    put_u32(&mut payload, 32, record.model_family_ref);
    put_u32(&mut payload, 36, record.modality_mask);
    put_u32(&mut payload, 40, record.license_ref);
    put_u32(&mut payload, 44, record.policy_ref);
    put_u32(&mut payload, 48, record.flags);
    let payload = with_payload_crc32c(payload, 52)?;
    encode_ai_record(
        2,
        u64::from(record.model_actor_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_generation_decoding_profile(
    record: GenerationDecodingProfileV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 45];
    put_u32(&mut payload, 0, record.decoding_profile_id);
    put_u32(&mut payload, 4, record.temperature_micros);
    put_u32(&mut payload, 8, record.top_p_micros);
    put_u32(&mut payload, 12, record.top_k);
    put_u64(&mut payload, 16, record.seed);
    put_u32(&mut payload, 24, record.max_output_tokens);
    put_u32(&mut payload, 28, record.stop_sequence_ref);
    put_u32(&mut payload, 32, record.safety_policy_ref);
    put_u8(&mut payload, 36, record.deterministic_claim);
    put_u32(&mut payload, 37, record.flags);
    let payload = with_payload_crc32c(payload, 41)?;
    encode_ai_record(
        3,
        u64::from(record.decoding_profile_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_human_review(record: HumanReviewEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 37];
    put_u32(&mut payload, 0, record.human_review_id);
    put_u8(&mut payload, 4, record.review_kind);
    put_u32(&mut payload, 5, record.reviewer_role_ref);
    put_i64(&mut payload, 9, record.review_time_us);
    put_u32(&mut payload, 17, record.rating_ppm);
    put_u32(&mut payload, 21, record.notes_ref);
    put_u32(&mut payload, 25, record.policy_ref);
    put_u32(&mut payload, 29, record.flags);
    let payload = with_payload_crc32c(payload, 33)?;
    encode_ai_record(
        4,
        u64::from(record.human_review_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_tensor_layout(record: TensorLayoutDescriptorV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 65];
    put_u32(&mut payload, 0, record.tensor_layout_id);
    put_u32(&mut payload, 4, record.layout_name_ref);
    put_u8(&mut payload, 8, record.rank);
    put_u8(&mut payload, 9, record.dtype);
    put_u8(&mut payload, 10, record.byte_order);
    put_u32(&mut payload, 11, record.shape_ref);
    put_u32(&mut payload, 15, record.stride_ref);
    put_i64(&mut payload, 19, record.storage_offset_elements);
    put_u8(&mut payload, 27, record.layout_kind);
    put_u32(&mut payload, 28, record.memory_alignment_bytes);
    put_u32(&mut payload, 32, record.preferred_page_alignment_bytes);
    put_u32(&mut payload, 36, record.tile_shape_ref);
    put_u32(&mut payload, 40, record.block_shape_ref);
    put_u32(&mut payload, 44, record.quantization_profile_ref);
    put_u32(&mut payload, 48, record.sparsity_profile_ref);
    put_u32(&mut payload, 52, record.framework_compatibility_ref);
    put_u8(&mut payload, 56, record.device_affinity_hint);
    put_u32(&mut payload, 57, record.flags);
    let payload = with_payload_crc32c(payload, 61)?;
    encode_ai_record(
        1,
        u64::from(record.tensor_layout_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_device_transfer_hint(
    record: DeviceTransferHintV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 28];
    put_u32(&mut payload, 0, record.transfer_hint_id);
    put_u8(&mut payload, 4, record.target_kind);
    put_u32(&mut payload, 5, record.preferred_alignment_bytes);
    put_u32(&mut payload, 9, record.preferred_chunk_bytes);
    put_u8(&mut payload, 13, record.pinned_memory_required);
    put_u8(&mut payload, 14, record.contiguous_required);
    put_u8(&mut payload, 15, record.zero_copy_possible);
    put_u32(&mut payload, 16, record.runtime_registry_binding_ref);
    put_u32(&mut payload, 20, record.flags);
    let payload = with_payload_crc32c(payload, 24)?;
    encode_ai_record(
        2,
        u64::from(record.transfer_hint_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_ai_asset_ref(record: AiAssetRefV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 99];
    put_u64(&mut payload, 0, record.asset_ref_id);
    put_u64(&mut payload, 8, record.parent_asset_ref);
    put_u8(&mut payload, 16, record.asset_kind);
    put_u32(&mut payload, 17, record.uri_ref);
    put_u32(&mut payload, 21, record.embedded_section_ref);
    put_u32(&mut payload, 25, record.media_type_ref);
    put_u64(&mut payload, 29, record.byte_length);
    put_u32(&mut payload, 37, record.digest_ref);
    put_u32(&mut payload, 41, record.width);
    put_u32(&mut payload, 45, record.height);
    put_u64(&mut payload, 49, record.duration_us);
    put_u32(&mut payload, 57, record.sample_rate_hz);
    put_u16(&mut payload, 61, record.channel_count);
    put_u32(&mut payload, 63, record.decode_profile_ref);
    put_u32(&mut payload, 67, record.preprocessing_profile_ref);
    put_u32(&mut payload, 71, record.transform_profile_ref);
    put_u32(&mut payload, 75, record.transform_digest_ref);
    put_u32(&mut payload, 79, record.tensor_layout_ref);
    put_u32(&mut payload, 83, record.license_ref);
    put_u32(&mut payload, 87, record.policy_ref);
    put_u32(&mut payload, 91, record.flags);
    let payload = with_payload_crc32c(payload, 95)?;
    encode_ai_record(
        1,
        record.asset_ref_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_multimodal_sequence_pack(
    record: MultimodalSequencePackV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 76];
    put_u64(&mut payload, 0, record.sequence_pack_id);
    put_u32(&mut payload, 8, record.training_profile_id);
    put_u32(&mut payload, 12, record.tokenizer_profile_id);
    put_u32(&mut payload, 16, record.sequence_profile_ref);
    put_u32(&mut payload, 20, record.element_count);
    put_u32(&mut payload, 24, record.first_element_ref);
    put_u32(&mut payload, 28, record.split_ref);
    put_u32(&mut payload, 32, record.sample_weight_ppm);
    put_u32(&mut payload, 36, record.loss_mask_ref);
    put_u32(&mut payload, 40, record.attention_mask_ref);
    put_u32(&mut payload, 44, record.position_map_ref);
    put_u32(&mut payload, 48, record.label_ref);
    put_u32(&mut payload, 52, record.source_snapshot_ref);
    put_u32(&mut payload, 56, record.evidence_ref);
    put_u64(&mut payload, 60, record.generator_provenance_ref);
    put_u32(&mut payload, 68, record.flags);
    let payload = with_payload_crc32c(payload, 72)?;
    encode_ai_record(
        1,
        record.sequence_pack_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_multimodal_sequence_element(
    record: MultimodalSequenceElementV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 115];
    put_u64(&mut payload, 0, record.element_id);
    put_u64(&mut payload, 8, record.sequence_pack_id);
    put_u32(&mut payload, 16, record.ordinal);
    put_u8(&mut payload, 20, record.element_kind);
    put_u8(&mut payload, 21, record.modality);
    put_u8(&mut payload, 22, record.role);
    put_u64(&mut payload, 23, record.tokenized_span_ref);
    put_u64(&mut payload, 31, record.token_sequence_pack_ref);
    put_u64(&mut payload, 39, record.asset_ref);
    put_u64(&mut payload, 47, record.tensor_ref);
    put_u64(&mut payload, 55, record.vector_ref);
    put_u64(&mut payload, 63, record.byte_start);
    put_u64(&mut payload, 71, record.byte_length);
    put_i64(&mut payload, 79, record.time_start_us);
    put_i64(&mut payload, 87, record.time_duration_us);
    put_u32(&mut payload, 95, record.position_stream_ref);
    put_u32(&mut payload, 99, record.evidence_ref);
    put_u32(&mut payload, 103, record.policy_ref);
    put_u32(&mut payload, 107, record.flags);
    let payload = with_payload_crc32c(payload, 111)?;
    encode_ai_record(
        2,
        record.element_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_space(record: VectorSpaceDescriptorV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 60];
    put_u32(&mut payload, 0, record.vector_space_id);
    put_u32(&mut payload, 4, record.vector_space_name_ref);
    put_u32(&mut payload, 8, record.vector_space_fingerprint_ref);
    put_u32(&mut payload, 12, record.embedding_namespace_ref);
    put_u32(&mut payload, 16, record.embedding_model_ref);
    put_u32(&mut payload, 20, record.embedding_model_version_ref);
    put_u32(&mut payload, 24, record.embedding_model_digest_ref);
    put_u32(&mut payload, 28, record.embedding_pipeline_ref);
    put_u32(&mut payload, 32, record.tokenizer_profile_ref);
    put_u32(&mut payload, 36, record.chunk_profile_ref);
    put_u32(&mut payload, 40, record.dimension_count);
    put_u8(&mut payload, 44, record.element_type);
    put_u8(&mut payload, 45, record.metric);
    put_u8(&mut payload, 46, record.normalization_policy);
    put_u8(&mut payload, 47, record.quantization_policy);
    put_u8(&mut payload, 48, record.deterministic);
    put_u8(&mut payload, 49, record.approximate);
    put_u8(&mut payload, 50, record.reproducibility_class);
    put_u8(&mut payload, 51, record.reserved);
    put_u32(&mut payload, 52, record.flags);
    let payload = with_payload_crc32c(payload, 56)?;
    encode_ai_record(
        1,
        u64::from(record.vector_space_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_space_compatibility(
    record: VectorSpaceCompatibilityDescriptorV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 44];
    put_u32(&mut payload, 0, record.compatibility_id);
    put_u32(&mut payload, 4, record.left_vector_space_id);
    put_u32(&mut payload, 8, record.right_vector_space_id);
    put_u8(&mut payload, 12, record.compatibility_kind);
    put_u8(&mut payload, 13, record.compatibility_authority);
    put_u8(&mut payload, 14, record.metric);
    put_u8(&mut payload, 15, record.normalization_policy);
    put_u32(&mut payload, 16, record.transform_ref);
    put_u32(&mut payload, 20, record.numeric_transform_error_ppm);
    put_u32(&mut payload, 24, record.ranking_eval_ref);
    put_u32(&mut payload, 28, record.calibration_dataset_ref);
    put_u32(&mut payload, 32, record.evidence_ref);
    put_u32(&mut payload, 36, record.flags);
    let payload = with_payload_crc32c(payload, 40)?;
    encode_ai_record(
        2,
        u64::from(record.compatibility_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_filecode_vector_binding(
    record: FileCodeVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let record_version = if record.model_input_digest_ref != 0 {
        2
    } else {
        1
    };
    let mut payload = vec![0u8; if record_version == 2 { 84 } else { 80 }];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u32(&mut payload, 12, record.slot_policy_ref);
    put_u32(&mut payload, 16, record.file_ref);
    put_u32(&mut payload, 20, record.dictionary_digest_ref);
    put_u32(&mut payload, 24, record.schema_fingerprint_ref);
    put_u32(&mut payload, 28, record.table_id);
    put_u32(&mut payload, 32, record.column_id);
    put_u32(&mut payload, 36, record.object_type_id);
    put_u32(&mut payload, 40, record.property_id);
    put_u32(&mut payload, 44, record.association_type_id);
    put_u32(&mut payload, 48, record.path_ref);
    put_u32(&mut payload, 52, record.file_code);
    put_u32(&mut payload, 56, record.reserved0);
    put_u32(&mut payload, 60, record.canonical_value_hash_ref);
    put_u64(&mut payload, 64, record.vector_ref);
    let checksum_offset = if record_version == 2 {
        put_u32(&mut payload, 72, record.model_input_digest_ref);
        put_u32(&mut payload, 76, record.flags);
        80
    } else {
        put_u32(&mut payload, 72, record.flags);
        76
    };
    let payload = with_payload_crc32c(payload, checksum_offset)?;
    encode_ai_record_with_version(
        1,
        record_version,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_chunk_vector_binding(
    record: ChunkVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let record_version = if record.model_input_digest_ref != 0 {
        2
    } else {
        1
    };
    let mut payload = vec![0u8; if record_version == 2 { 52 } else { 48 }];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u64(&mut payload, 12, record.chunk_id);
    put_u32(&mut payload, 20, record.chunk_profile_id);
    put_u32(&mut payload, 24, record.source_value_hash_ref);
    put_u32(&mut payload, 28, record.chunk_text_hash_ref);
    put_u64(&mut payload, 32, record.vector_ref);
    let checksum_offset = if record_version == 2 {
        put_u32(&mut payload, 40, record.model_input_digest_ref);
        put_u32(&mut payload, 44, record.flags);
        48
    } else {
        put_u32(&mut payload, 40, record.flags);
        44
    };
    let payload = with_payload_crc32c(payload, checksum_offset)?;
    encode_ai_record_with_version(
        2,
        record_version,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_object_state_vector_binding(
    record: ObjectStateVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let record_version = if record.model_input_digest_ref != 0 {
        2
    } else {
        1
    };
    let mut payload = vec![0u8; if record_version == 2 { 73 } else { 69 }];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u32(&mut payload, 12, record.composition_profile_ref);
    put_u32(&mut payload, 16, record.file_ref);
    put_u32(&mut payload, 20, record.object_type_id);
    put_u32(&mut payload, 24, record.goid_ref);
    put_u32(&mut payload, 28, record.branch_ref);
    put_u8(&mut payload, 32, record.temporal_kind);
    put_u64(&mut payload, 33, record.csn);
    put_i64(&mut payload, 41, record.timestamp_us);
    put_u32(&mut payload, 49, record.property_dependency_fingerprint_ref);
    put_u64(&mut payload, 53, record.vector_ref);
    let checksum_offset = if record_version == 2 {
        put_u32(&mut payload, 61, record.model_input_digest_ref);
        put_u32(&mut payload, 65, record.flags);
        69
    } else {
        put_u32(&mut payload, 61, record.flags);
        65
    };
    let payload = with_payload_crc32c(payload, checksum_offset)?;
    encode_ai_record_with_version(
        3,
        record_version,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_training_sample_vector_binding(
    record: TrainingSampleVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let record_version = if record.model_input_digest_ref != 0 {
        2
    } else {
        1
    };
    let mut payload = vec![0u8; if record_version == 2 { 52 } else { 48 }];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u32(&mut payload, 12, record.training_profile_ref);
    put_u64(&mut payload, 16, record.sample_id);
    put_u32(&mut payload, 24, record.source_snapshot_ref);
    put_u32(&mut payload, 28, record.sample_fingerprint_ref);
    put_u64(&mut payload, 32, record.vector_ref);
    let checksum_offset = if record_version == 2 {
        put_u32(&mut payload, 40, record.model_input_digest_ref);
        put_u32(&mut payload, 44, record.flags);
        48
    } else {
        put_u32(&mut payload, 40, record.flags);
        44
    };
    let payload = with_payload_crc32c(payload, checksum_offset)?;
    encode_ai_record_with_version(
        4,
        record_version,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_association_state_vector_binding(
    record: AssociationStateVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let record_version = if record.model_input_digest_ref != 0 {
        2
    } else {
        1
    };
    let mut payload = vec![0u8; if record_version == 2 { 73 } else { 69 }];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u32(&mut payload, 12, record.composition_profile_ref);
    put_u32(&mut payload, 16, record.file_ref);
    put_u32(&mut payload, 20, record.association_type_id);
    put_u32(&mut payload, 24, record.association_key_ref);
    put_u32(&mut payload, 28, record.branch_ref);
    put_u8(&mut payload, 32, record.temporal_kind);
    put_u64(&mut payload, 33, record.csn);
    put_i64(&mut payload, 41, record.timestamp_us);
    put_u32(&mut payload, 49, record.property_dependency_fingerprint_ref);
    put_u64(&mut payload, 53, record.vector_ref);
    let checksum_offset = if record_version == 2 {
        put_u32(&mut payload, 61, record.model_input_digest_ref);
        put_u32(&mut payload, 65, record.flags);
        69
    } else {
        put_u32(&mut payload, 61, record.flags);
        65
    };
    let payload = with_payload_crc32c(payload, checksum_offset)?;
    encode_ai_record_with_version(
        5,
        record_version,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_asset_vector_binding(
    record: AssetVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let record_version = if record.model_input_digest_ref != 0 {
        2
    } else {
        1
    };
    let mut payload = vec![0u8; if record_version == 2 { 48 } else { 44 }];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u64(&mut payload, 12, record.asset_ref);
    put_u32(&mut payload, 20, record.transform_ref);
    put_u32(&mut payload, 24, record.asset_digest_ref);
    put_u64(&mut payload, 28, record.vector_ref);
    let checksum_offset = if record_version == 2 {
        put_u32(&mut payload, 36, record.model_input_digest_ref);
        put_u32(&mut payload, 40, record.flags);
        44
    } else {
        put_u32(&mut payload, 36, record.flags);
        40
    };
    let payload = with_payload_crc32c(payload, checksum_offset)?;
    encode_ai_record_with_version(
        6,
        record_version,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_multimodal_sequence_vector_binding(
    record: MultimodalSequenceVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let record_version = if record.model_input_digest_ref != 0 {
        2
    } else {
        1
    };
    let mut payload = vec![0u8; if record_version == 2 { 48 } else { 44 }];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u64(&mut payload, 12, record.sequence_pack_id);
    put_u32(&mut payload, 20, record.sequence_profile_ref);
    put_u32(&mut payload, 24, record.source_snapshot_ref);
    put_u64(&mut payload, 28, record.vector_ref);
    let checksum_offset = if record_version == 2 {
        put_u32(&mut payload, 36, record.model_input_digest_ref);
        put_u32(&mut payload, 40, record.flags);
        44
    } else {
        put_u32(&mut payload, 36, record.flags);
        40
    };
    let payload = with_payload_crc32c(payload, checksum_offset)?;
    encode_ai_record_with_version(
        7,
        record_version,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_payload_block(
    record: VectorPayloadBlockHeaderV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 68];
    put_u32(&mut payload, 0, record.block_id);
    put_u32(&mut payload, 4, record.vector_space_id);
    put_u64(&mut payload, 8, record.vector_count);
    put_u32(&mut payload, 16, record.dimension_count);
    put_u8(&mut payload, 20, record.element_type);
    put_u8(&mut payload, 21, record.compression_codec);
    put_u8(&mut payload, 22, record.quantization_kind);
    put_u8(&mut payload, 23, record.layout_kind);
    put_u32(&mut payload, 24, record.tensor_layout_ref);
    put_u32(&mut payload, 28, record.memory_alignment_bytes);
    put_u32(&mut payload, 32, record.payload_stride_ref);
    put_u32(&mut payload, 36, record.device_transfer_hint_ref);
    put_u32(&mut payload, 40, record.payload_ref);
    put_u64(&mut payload, 44, record.payload_offset);
    put_u64(&mut payload, 52, record.payload_length);
    put_u32(&mut payload, 60, record.integrity_ref);
    let payload = with_payload_crc32c(payload, 64)?;
    encode_ai_record(
        1,
        u64::from(record.block_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_entry(record: VectorEntryV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 44];
    put_u64(&mut payload, 0, record.vector_ref);
    put_u32(&mut payload, 8, record.block_id);
    put_u64(&mut payload, 12, record.vector_ordinal);
    put_u64(&mut payload, 20, record.payload_offset);
    put_u32(&mut payload, 28, record.payload_length);
    put_u32(&mut payload, 32, record.integrity_ref);
    put_u32(&mut payload, 36, record.flags);
    let payload = with_payload_crc32c(payload, 40)?;
    encode_ai_record(
        1,
        record.vector_ref,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_composition_profile(
    record: VectorCompositionProfileV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 42];
    put_u32(&mut payload, 0, record.composition_profile_id);
    put_u32(&mut payload, 4, record.composition_name_ref);
    put_u32(&mut payload, 8, record.output_vector_space_id);
    put_u32(&mut payload, 12, record.arithmetic_profile_ref);
    put_u8(&mut payload, 16, record.method);
    put_u8(&mut payload, 17, record.missing_policy);
    put_u8(&mut payload, 18, record.normalize_inputs);
    put_u8(&mut payload, 19, record.normalize_output);
    put_u8(&mut payload, 20, record.result_authority);
    put_u8(&mut payload, 21, record.reproducibility_class);
    put_u32(&mut payload, 22, record.first_component_ref);
    put_u32(&mut payload, 26, record.component_count);
    put_u32(&mut payload, 30, record.template_ref);
    put_u32(&mut payload, 34, record.flags);
    let payload = with_payload_crc32c(payload, 38)?;
    encode_ai_record(
        1,
        u64::from(record.composition_profile_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_composition_component(
    record: VectorCompositionComponentV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 28];
    put_u32(&mut payload, 0, record.component_id);
    put_u32(&mut payload, 4, record.slot_policy_ref);
    put_u32(&mut payload, 8, record.source_vector_space_id);
    put_u32(&mut payload, 12, record.weight_ppm);
    put_u8(&mut payload, 16, record.required);
    put_u8(&mut payload, 17, record.redaction_behavior);
    put_u8(&mut payload, 18, record.missing_behavior);
    put_u8(&mut payload, 19, record.reserved);
    put_u32(&mut payload, 20, record.flags);
    let payload = with_payload_crc32c(payload, 24)?;
    encode_ai_record(
        2,
        u64::from(record.component_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_arithmetic_profile(
    record: VectorArithmeticProfileV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 29];
    put_u32(&mut payload, 0, record.arithmetic_profile_id);
    put_u32(&mut payload, 4, record.profile_name_ref);
    put_u8(&mut payload, 8, record.arithmetic_kind);
    put_u8(&mut payload, 9, record.input_quantization_kind);
    put_u8(&mut payload, 10, record.accumulator_kind);
    put_u8(&mut payload, 11, record.rounding_mode);
    put_u8(&mut payload, 12, record.overflow_policy);
    put_u8(&mut payload, 13, record.component_order);
    put_u32(&mut payload, 14, record.weight_scale);
    put_u8(&mut payload, 18, record.output_quantization_kind);
    put_u8(&mut payload, 19, record.output_element_type);
    put_u8(&mut payload, 20, record.normalization_policy);
    put_u32(&mut payload, 21, record.flags);
    let payload = with_payload_crc32c(payload, 25)?;
    encode_ai_record(
        3,
        u64::from(record.arithmetic_profile_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}

pub(super) fn encode_vector_index(record: VectorIndexDescriptorV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 54];
    put_u32(&mut payload, 0, record.vector_index_id);
    put_u32(&mut payload, 4, record.vector_space_id);
    put_u32(&mut payload, 8, record.stored_vector_space_id);
    put_u32(&mut payload, 12, record.search_vector_space_id);
    put_u8(&mut payload, 16, record.index_kind);
    put_u8(&mut payload, 17, record.exactness_kind);
    put_u8(&mut payload, 18, record.false_negative_policy);
    put_u8(&mut payload, 19, record.metric);
    put_u8(&mut payload, 20, record.score_space_authority);
    put_u32(&mut payload, 21, record.dimension_count);
    put_u8(&mut payload, 25, record.indexed_binding_kind);
    put_u32(&mut payload, 26, record.temporal_scope_ref);
    put_u32(&mut payload, 30, record.visibility_scope_ref);
    put_u32(&mut payload, 34, record.redaction_scope_ref);
    put_u32(&mut payload, 38, record.dequantization_profile_ref);
    put_u32(&mut payload, 42, record.quantization_error_profile_ref);
    put_u32(&mut payload, 46, record.payload_ref);
    let payload = with_payload_crc32c(payload, 50)?;
    encode_ai_record(
        1,
        u64::from(record.vector_index_id),
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        &payload,
    )
}
