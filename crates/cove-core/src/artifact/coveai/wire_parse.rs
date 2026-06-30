use super::*;

pub(super) fn parse_postscript_from_tail(
    data: &[u8],
) -> Result<(CoveAiArtifactKind, CoveAiPostscriptV1, usize), CoveError> {
    if data.len() < COVEAI_POSTSCRIPT_TAIL_SIZE + COVEAI_POSTSCRIPT_LEN as usize {
        return Err(CoveError::BufferTooShort);
    }
    let magic_offset = data.len() - 4;
    let magic = read_array::<4>(data, magic_offset)?;
    let Some(artifact_kind) = CoveAiArtifactKind::from_magic(magic) else {
        return Err(CoveError::BadMagic);
    };
    let postscript_len = read_u16(data, data.len() - 6)?;
    let postscript_version = read_u16(data, data.len() - 8)?;
    if postscript_version != COVEAI_POSTSCRIPT_VERSION_V1 {
        return Err(CoveError::BadVersion);
    }
    if postscript_len != COVEAI_POSTSCRIPT_LEN || !(44..=4096).contains(&postscript_len) {
        return Err(CoveError::BadSection(format!(
            "COVE-AI postscript_len must be {COVEAI_POSTSCRIPT_LEN}, got {postscript_len}"
        )));
    }
    let postscript_offset = data
        .len()
        .checked_sub(COVEAI_POSTSCRIPT_TAIL_SIZE)
        .and_then(|end| end.checked_sub(postscript_len as usize))
        .ok_or(CoveError::OffsetRange)?;
    let postscript = CoveAiPostscriptV1::parse(&data[postscript_offset..])?;
    Ok((artifact_kind, postscript, postscript_offset))
}

pub(super) fn validate_section_ranges(
    entries: &[CoveAiSectionEntryV1],
    file_len: u64,
    header_offset: u64,
    header_length: u64,
    postscript_offset: u64,
) -> Result<(), CoveError> {
    let mut ranges = Vec::with_capacity(entries.len() + 2);
    ranges.push((
        header_offset,
        header_offset
            .checked_add(header_length)
            .ok_or(CoveError::ArithOverflow)?,
        "header".to_string(),
    ));
    ranges.push((postscript_offset, file_len, "postscript".to_string()));
    for entry in entries {
        let end = checked_range(entry.offset, entry.length, file_len)?;
        ranges.push((entry.offset, end, format!("section {}", entry.section_id)));
    }
    ranges.sort_by_key(|(start, _, _)| *start);
    for pair in ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(CoveError::BadSection(format!(
                "COVE-AI range overlap between {} and {}",
                pair[0].2, pair[1].2
            )));
        }
    }
    Ok(())
}

pub(super) fn decoded_section_payload<'a>(
    data: &'a [u8],
    entry: &CoveAiSectionEntryV1,
) -> Result<Cow<'a, [u8]>, CoveError> {
    let raw = checked_slice(data, entry.offset, entry.length)?;
    compression::section_payload_from_raw(
        raw,
        entry.length,
        entry.uncompressed_length,
        entry.compression,
        checksum::crc32c(raw),
    )
}

pub(super) fn validate_section_payload_crc(
    entry: &CoveAiSectionEntryV1,
    payload: &[u8],
) -> Result<(), CoveError> {
    if entry.payload_crc32c == 0 {
        if entry.section_kind == SectionKind::AiPayloadBytes as u32 {
            return Ok(());
        }
        return Err(CoveError::BadSection(format!(
            "descriptor section {} is missing payload CRC32C",
            entry.section_id
        )));
    }
    if checksum::crc32c(payload) != entry.payload_crc32c {
        return Err(CoveError::ChecksumMismatch);
    }
    Ok(())
}

pub(super) fn parse_ai_records(
    entry: &CoveAiSectionEntryV1,
    payload: &[u8],
    tables: &mut AiDescriptorTablesV1,
) -> Result<Vec<AiRecordHeaderV1>, CoveError> {
    let mut cursor = 0usize;
    let mut headers = Vec::new();
    let mut local_ids = BTreeSet::new();
    while cursor < payload.len() {
        if payload.len() - cursor < AI_RECORD_HEADER_LEN {
            return Err(CoveError::BadSection(
                "truncated COVE-AI binary record header".into(),
            ));
        }
        let record_len = read_u32(payload, cursor + 4)? as usize;
        if record_len < AI_RECORD_HEADER_LEN {
            return Err(CoveError::BadSection(
                "COVE-AI record_len is shorter than AiRecordHeaderV1".into(),
            ));
        }
        let end = cursor
            .checked_add(record_len)
            .ok_or(CoveError::ArithOverflow)?;
        if end > payload.len() {
            return Err(CoveError::BadSection(
                "COVE-AI record extends beyond section payload".into(),
            ));
        }
        let record_bytes = &payload[cursor..end];
        let header = AiRecordHeaderV1::parse(record_bytes)?;
        if !local_ids.insert((header.record_kind, header.local_id)) {
            return Err(CoveError::BadSection(format!(
                "duplicate COVE-AI local_id {} for record_kind {} in section {}",
                header.local_id, header.record_kind, entry.section_id
            )));
        }
        parse_known_record(
            entry.section_kind,
            &header,
            &record_bytes[AI_RECORD_HEADER_LEN..],
            tables,
        )?;
        headers.push(header);
        cursor = end;
    }
    Ok(headers)
}

pub(super) fn parse_known_record(
    section_kind: u32,
    header: &AiRecordHeaderV1,
    payload: &[u8],
    tables: &mut AiDescriptorTablesV1,
) -> Result<(), CoveError> {
    if !ai_record_version_supported(section_kind, header.record_kind, header.record_version) {
        return Err(CoveError::BadSection(format!(
            "unsupported COVE-AI record_version {} for record kind {} in section kind {}",
            header.record_version, header.record_kind, section_kind
        )));
    }
    match (section_kind, header.record_kind) {
        (k, 1) if k == SectionKind::AiCompanionArtifactRef as u32 => {
            tables
                .companion_artifacts
                .push(parse_companion_artifact_ref(payload)?);
        }
        (k, 1) if k == SectionKind::AiSourceBinding as u32 => {
            tables.source_bindings.push(parse_source_binding(payload)?);
        }
        (k, 1) if k == SectionKind::AiReferenceTables as u32 => {
            tables.strings.push(parse_string_entry(payload)?);
        }
        (k, 2) if k == SectionKind::AiReferenceTables as u32 => {
            tables.digests.push(parse_digest_entry(payload)?);
        }
        (k, 3) if k == SectionKind::AiReferenceTables as u32 => {
            tables.payload_refs.push(parse_payload_ref_entry(payload)?);
        }
        (k, 4) if k == SectionKind::AiReferenceTables as u32 => {
            tables.policies.push(parse_policy_ref_entry(payload)?);
        }
        (k, 5) if k == SectionKind::AiReferenceTables as u32 => {
            tables.source_spans.push(parse_source_span_entry(payload)?);
        }
        (k, 6) if k == SectionKind::AiReferenceTables as u32 => {
            tables.transforms.push(parse_transform_entry(payload)?);
        }
        (k, 1) if k == SectionKind::AiPrivacySummary as u32 => {
            tables
                .privacy_summaries
                .push(parse_privacy_summary(payload)?);
        }
        (k, 1) if k == SectionKind::AiPayloadIntegrity as u32 => {
            tables
                .payload_integrity
                .push(parse_payload_integrity(payload)?);
        }
        (k, 1) if k == SectionKind::AiSectionFeatureBinding as u32 => {
            tables
                .section_feature_bindings
                .push(parse_section_feature_binding(payload)?);
        }
        (k, 1) if k == SectionKind::AiChunkProfile as u32 => {
            tables.chunk_profiles.push(parse_chunk_profile(payload)?);
        }
        (k, 1) if k == SectionKind::AiTextChunkIndex as u32 => {
            tables.text_chunks.push(parse_text_chunk_entry(payload)?);
        }
        (k, 1) if k == SectionKind::AiTokenizerProfile as u32 => {
            tables
                .tokenizer_profiles
                .push(parse_tokenizer_profile(payload)?);
        }
        (k, 1) if k == SectionKind::AiTokenBlock as u32 => {
            tables.token_blocks.push(parse_token_block(payload)?);
        }
        (k, 1) if k == SectionKind::AiTokenizedSpan as u32 => {
            tables.tokenized_spans.push(parse_tokenized_span(payload)?);
        }
        (k, 1) if k == SectionKind::AiTokenSequencePack as u32 => {
            tables
                .token_sequence_packs
                .push(parse_token_sequence_pack(payload)?);
        }
        (k, 1) if k == SectionKind::AiTrainingProfile as u32 => {
            tables
                .training_profiles
                .push(parse_training_profile(payload)?);
        }
        (k, 1) if k == SectionKind::AiTrainingSampleIndex as u32 => {
            tables
                .training_samples
                .push(parse_training_sample_entry(payload)?);
        }
        (k, 1) if k == SectionKind::AiTrainingSplitDedupEpoch as u32 => {
            tables.dataset_splits.push(parse_dataset_split(payload)?);
        }
        (k, 2) if k == SectionKind::AiTrainingSplitDedupEpoch as u32 => {
            tables.dedup_groups.push(parse_dedup_group(payload)?);
        }
        (k, 3) if k == SectionKind::AiTrainingSplitDedupEpoch as u32 => {
            tables
                .training_epoch_plans
                .push(parse_training_epoch_plan(payload)?);
        }
        (k, 1) if k == SectionKind::AiLabelPreference as u32 => {
            tables
                .training_labels
                .push(parse_training_label_entry(payload)?);
        }
        (k, 2) if k == SectionKind::AiLabelPreference as u32 => {
            tables
                .preference_pairs
                .push(parse_preference_pair_entry(payload)?);
        }
        (k, 1) if k == SectionKind::AiGeneratorProvenance as u32 => {
            tables
                .generator_provenance
                .push(parse_generator_provenance(payload)?);
        }
        (k, 2) if k == SectionKind::AiGeneratorProvenance as u32 => {
            tables.model_actors.push(parse_model_actor(payload)?);
        }
        (k, 3) if k == SectionKind::AiGeneratorProvenance as u32 => {
            tables
                .generation_decoding_profiles
                .push(parse_generation_decoding_profile(payload)?);
        }
        (k, 4) if k == SectionKind::AiGeneratorProvenance as u32 => {
            tables.human_reviews.push(parse_human_review(payload)?);
        }
        (k, 1) if k == SectionKind::AiTensorLayout as u32 => {
            tables.tensor_layouts.push(parse_tensor_layout(payload)?);
        }
        (k, 2) if k == SectionKind::AiTensorLayout as u32 => {
            tables
                .device_transfer_hints
                .push(parse_device_transfer_hint(payload)?);
        }
        (k, 1) if k == SectionKind::AiAssetManifest as u32 => {
            tables.assets.push(parse_ai_asset_ref(payload)?);
        }
        (k, 1) if k == SectionKind::AiMultimodalSequence as u32 => {
            tables
                .multimodal_sequence_packs
                .push(parse_multimodal_sequence_pack(payload)?);
        }
        (k, 2) if k == SectionKind::AiMultimodalSequence as u32 => {
            tables
                .multimodal_sequence_elements
                .push(parse_multimodal_sequence_element(payload)?);
        }
        (k, 1) if k == SectionKind::AiVectorSpace as u32 => {
            tables.vector_spaces.push(parse_vector_space(payload)?);
        }
        (k, 2) if k == SectionKind::AiVectorSpace as u32 => {
            tables
                .vector_space_compatibilities
                .push(parse_vector_space_compatibility(payload)?);
        }
        (k, 1) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .filecode_vector_bindings
                .push(parse_filecode_vector_binding(
                    payload,
                    header.record_version,
                )?);
        }
        (k, 2) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .chunk_vector_bindings
                .push(parse_chunk_vector_binding(payload, header.record_version)?);
        }
        (k, 3) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .object_state_vector_bindings
                .push(parse_object_state_vector_binding(
                    payload,
                    header.record_version,
                )?);
        }
        (k, 4) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .training_sample_vector_bindings
                .push(parse_training_sample_vector_binding(
                    payload,
                    header.record_version,
                )?);
        }
        (k, 5) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .association_state_vector_bindings
                .push(parse_association_state_vector_binding(
                    payload,
                    header.record_version,
                )?);
        }
        (k, 6) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .asset_vector_bindings
                .push(parse_asset_vector_binding(payload, header.record_version)?);
        }
        (k, 7) if k == SectionKind::AiVectorBinding as u32 => {
            tables.multimodal_sequence_vector_bindings.push(
                parse_multimodal_sequence_vector_binding(payload, header.record_version)?,
            );
        }
        (k, 1) if k == SectionKind::AiVectorPayloadBlock as u32 => {
            tables
                .vector_payload_blocks
                .push(parse_vector_payload_block(payload)?);
        }
        (k, 1) if k == SectionKind::AiVectorDirectory as u32 => {
            tables.vector_entries.push(parse_vector_entry(payload)?);
        }
        (k, 1) if k == SectionKind::AiVectorComposition as u32 => {
            tables
                .vector_composition_profiles
                .push(parse_vector_composition_profile(payload)?);
        }
        (k, 2) if k == SectionKind::AiVectorComposition as u32 => {
            tables
                .vector_composition_components
                .push(parse_vector_composition_component(payload)?);
        }
        (k, 3) if k == SectionKind::AiVectorComposition as u32 => {
            tables
                .vector_arithmetic_profiles
                .push(parse_vector_arithmetic_profile(payload)?);
        }
        (k, 1) if k == SectionKind::AiVectorIndex as u32 => {
            tables.vector_indexes.push(parse_vector_index(payload)?);
        }
        _ if header.flags & AI_FLAG_REQUIRED_RECORD != 0 => {
            return Err(CoveError::BadSection(format!(
                "unsupported required COVE-AI record kind {} in section kind {}",
                header.record_kind, section_kind
            )));
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn ai_record_version_supported(
    section_kind: u32,
    record_kind: u16,
    record_version: u16,
) -> bool {
    record_version == 1
        || (record_version == 2
            && section_kind == SectionKind::AiVectorBinding as u32
            && matches!(record_kind, 1..=7))
}

macro_rules! exact_len {
    ($payload:expr, $len:expr, $name:expr) => {{
        if $payload.len() != $len {
            return Err(CoveError::BadSection(format!(
                "{} record length mismatch: expected {}, got {}",
                $name,
                $len,
                $payload.len()
            )));
        }
    }};
}

pub(super) fn parse_string_entry(payload: &[u8]) -> Result<AiStringEntryV1, CoveError> {
    exact_len!(payload, 20, "AiStringEntryV1");
    verify_payload_crc(payload, 16)?;
    Ok(AiStringEntryV1 {
        string_ref: read_u32(payload, 0)?,
        utf8_byte_length: read_u32(payload, 4)?,
        payload_ref: read_u32(payload, 8)?,
        flags: read_u32(payload, 12)?,
        crc32c: read_u32(payload, 16)?,
    })
}

pub(super) fn parse_digest_entry(payload: &[u8]) -> Result<AiDigestEntryV1, CoveError> {
    exact_len!(payload, 21, "AiDigestEntryV1");
    verify_payload_crc(payload, 17)?;
    Ok(AiDigestEntryV1 {
        digest_ref: read_u32(payload, 0)?,
        digest_algorithm: read_u16(payload, 4)?,
        digest_len: read_u16(payload, 6)?,
        digest_payload_ref: read_u32(payload, 8)?,
        domain_hint: read_u8(payload, 12)?,
        flags: read_u32(payload, 13)?,
        crc32c: read_u32(payload, 17)?,
    })
}

pub(super) fn parse_payload_ref_entry(payload: &[u8]) -> Result<AiPayloadRefEntryV1, CoveError> {
    exact_len!(payload, 61, "AiPayloadRefEntryV1");
    verify_payload_crc(payload, 57)?;
    Ok(AiPayloadRefEntryV1 {
        payload_ref: read_u32(payload, 0)?,
        storage_kind: read_u8(payload, 4)?,
        media_type_ref: read_u32(payload, 5)?,
        section_id: read_u32(payload, 9)?,
        uri_ref: read_u32(payload, 13)?,
        payload_offset: read_u64(payload, 17)?,
        section_payload_offset: read_u64(payload, 25)?,
        payload_length: read_u64(payload, 33)?,
        decoded_length: read_u64(payload, 41)?,
        integrity_ref: read_u32(payload, 49)?,
        flags: read_u32(payload, 53)?,
        crc32c: read_u32(payload, 57)?,
    })
}

pub(super) fn parse_policy_ref_entry(payload: &[u8]) -> Result<AiPolicyRefEntryV1, CoveError> {
    exact_len!(payload, 25, "AiPolicyRefEntryV1");
    verify_payload_crc(payload, 21)?;
    Ok(AiPolicyRefEntryV1 {
        policy_ref: read_u32(payload, 0)?,
        policy_kind: read_u8(payload, 4)?,
        authority_ref: read_u32(payload, 5)?,
        payload_ref: read_u32(payload, 9)?,
        digest_ref: read_u32(payload, 13)?,
        flags: read_u32(payload, 17)?,
        crc32c: read_u32(payload, 21)?,
    })
}

pub(super) fn parse_source_span_entry(payload: &[u8]) -> Result<AiSourceSpanEntryV1, CoveError> {
    exact_len!(payload, 65, "AiSourceSpanEntryV1");
    verify_payload_crc(payload, 61)?;
    Ok(AiSourceSpanEntryV1 {
        source_span_ref: read_u32(payload, 0)?,
        source_binding_ref: read_u32(payload, 4)?,
        source_kind: read_u8(payload, 8)?,
        source_row_ref: read_u64(payload, 9)?,
        source_object_ref: read_u64(payload, 17)?,
        byte_start: read_u64(payload, 25)?,
        byte_length: read_u64(payload, 33)?,
        token_start: read_u64(payload, 41)?,
        token_count: read_u32(payload, 49)?,
        evidence_ref: read_u32(payload, 53)?,
        flags: read_u32(payload, 57)?,
        crc32c: read_u32(payload, 61)?,
    })
}

pub(super) fn parse_transform_entry(payload: &[u8]) -> Result<AiTransformEntryV1, CoveError> {
    exact_len!(payload, 33, "AiTransformEntryV1");
    verify_payload_crc(payload, 29)?;
    Ok(AiTransformEntryV1 {
        transform_ref: read_u32(payload, 0)?,
        transform_kind: read_u8(payload, 4)?,
        function_or_template_ref: read_u32(payload, 5)?,
        input_digest_ref: read_u32(payload, 9)?,
        output_digest_ref: read_u32(payload, 13)?,
        parameter_payload_ref: read_u32(payload, 17)?,
        transform_digest_ref: read_u32(payload, 21)?,
        flags: read_u32(payload, 25)?,
        crc32c: read_u32(payload, 29)?,
    })
}

pub(super) fn parse_privacy_summary(payload: &[u8]) -> Result<AiPrivacySummaryEntryV1, CoveError> {
    exact_len!(payload, 38, "AiPrivacySummaryEntryV1");
    verify_payload_crc(payload, 34)?;
    Ok(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: read_u32(payload, 0)?,
        source_binding_ref: read_u32(payload, 4)?,
        sensitivity_mask: read_u32(payload, 8)?,
        sensitivity_bits_ref: read_u32(payload, 12)?,
        policy_ref: read_u32(payload, 16)?,
        visibility_scope_ref: read_u32(payload, 20)?,
        redaction_scope_ref: read_u32(payload, 24)?,
        retention_state: read_u8(payload, 28)?,
        disclosure_state: read_u8(payload, 29)?,
        flags: read_u32(payload, 30)?,
        crc32c: read_u32(payload, 34)?,
    })
}

pub(super) fn parse_companion_artifact_ref(
    payload: &[u8],
) -> Result<AiCompanionArtifactRefV1, CoveError> {
    exact_len!(payload, 61, "AiCompanionArtifactRefV1");
    verify_payload_crc(payload, 57)?;
    Ok(AiCompanionArtifactRefV1 {
        artifact_ref: read_u32(payload, 0)?,
        artifact_kind: read_u8(payload, 4)?,
        artifact_id: read_array::<16>(payload, 5)?,
        uri_ref: read_u32(payload, 21)?,
        artifact_digest_ref: read_u32(payload, 25)?,
        source_binding_ref: read_u32(payload, 29)?,
        required_ai_features: read_u64(payload, 33)?,
        optional_ai_features: read_u64(payload, 41)?,
        flags: read_u32(payload, 49)?,
        crc32c: read_u32(payload, 57)?,
    })
}

pub(super) fn parse_source_binding(payload: &[u8]) -> Result<AiSourceBindingV1, CoveError> {
    exact_len!(payload, 61, "AiSourceBindingV1");
    verify_payload_crc(payload, 57)?;
    Ok(AiSourceBindingV1 {
        source_binding_id: read_u32(payload, 0)?,
        source_kind: read_u8(payload, 4)?,
        source_artifact_ref: read_u32(payload, 5)?,
        source_file_digest_ref: read_u32(payload, 9)?,
        covm_snapshot_ref: read_u32(payload, 13)?,
        schema_fingerprint_ref: read_u32(payload, 17)?,
        dictionary_digest_ref: read_u32(payload, 21)?,
        map_fingerprint_ref: read_u32(payload, 25)?,
        policy_context_ref: read_u32(payload, 29)?,
        visibility_scope_ref: read_u32(payload, 33)?,
        redaction_scope_ref: read_u32(payload, 37)?,
        branch_ref: read_u32(payload, 41)?,
        as_of_csn: read_u64(payload, 45)?,
        flags: read_u32(payload, 53)?,
        crc32c: read_u32(payload, 57)?,
    })
}

pub(super) fn parse_payload_integrity(payload: &[u8]) -> Result<AiPayloadIntegrityV1, CoveError> {
    exact_len!(payload, 26, "AiPayloadIntegrityV1");
    Ok(AiPayloadIntegrityV1 {
        integrity_ref: read_u32(payload, 0)?,
        payload_ref: read_u32(payload, 4)?,
        digest_domain: read_u8(payload, 8)?,
        reserved0: read_u8(payload, 9)?,
        digest_algorithm: read_u16(payload, 10)?,
        digest_len: read_u16(payload, 12)?,
        digest_ref: read_u32(payload, 14)?,
        payload_crc32c: read_u32(payload, 18)?,
        flags: read_u32(payload, 22)?,
    })
}

pub(super) fn parse_section_feature_binding(
    payload: &[u8],
) -> Result<AiSectionFeatureBindingV1, CoveError> {
    exact_len!(payload, 44, "AiSectionFeatureBindingV1");
    verify_payload_crc(payload, 40)?;
    Ok(AiSectionFeatureBindingV1 {
        binding_ref: read_u32(payload, 0)?,
        section_id: read_u32(payload, 4)?,
        scope: read_u8(payload, 8)?,
        profile_kind: read_u8(payload, 9)?,
        operation_kind: read_u16(payload, 10)?,
        required_ai_features: read_u64(payload, 12)?,
        optional_ai_features: read_u64(payload, 20)?,
        target_local_ref: read_u64(payload, 28)?,
        flags: read_u32(payload, 36)?,
        crc32c: read_u32(payload, 40)?,
    })
}

pub(super) fn parse_chunk_profile(payload: &[u8]) -> Result<ChunkProfileV1, CoveError> {
    exact_len!(payload, 60, "ChunkProfileV1");
    verify_payload_crc(payload, 56)?;
    let record = ChunkProfileV1 {
        chunk_profile_id: read_u32(payload, 0)?,
        profile_name_ref: read_u32(payload, 4)?,
        chunker_namespace_ref: read_u32(payload, 8)?,
        chunker_name_ref: read_u32(payload, 12)?,
        chunker_version_major: read_u16(payload, 16)?,
        chunker_version_minor: read_u16(payload, 18)?,
        tokenizer_profile_ref: read_u32(payload, 20)?,
        boundary_kind: read_u8(payload, 24)?,
        overlap_policy: read_u8(payload, 25)?,
        parent_policy: read_u8(payload, 26)?,
        normalization_policy: read_u8(payload, 27)?,
        target_tokens: read_u32(payload, 28)?,
        min_tokens: read_u32(payload, 32)?,
        max_tokens: read_u32(payload, 36)?,
        overlap_tokens: read_u32(payload, 40)?,
        max_bytes: read_u32(payload, 44)?,
        locale_ref: read_u32(payload, 48)?,
        flags: read_u32(payload, 52)?,
        checksum: read_u32(payload, 56)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_text_chunk_entry(payload: &[u8]) -> Result<TextChunkEntryV1, CoveError> {
    exact_len!(payload, 152, "TextChunkEntryV1");
    verify_payload_crc(payload, 148)?;
    Ok(TextChunkEntryV1 {
        chunk_id: read_u64(payload, 0)?,
        source_ref: read_u32(payload, 8)?,
        table_id: read_u32(payload, 12)?,
        column_id: read_u32(payload, 16)?,
        object_type_id: read_u32(payload, 20)?,
        property_id: read_u32(payload, 24)?,
        association_type_id: read_u32(payload, 28)?,
        path_ref: read_u32(payload, 32)?,
        source_row_ref: read_u64(payload, 36)?,
        source_object_ref: read_u64(payload, 44)?,
        source_value_hash_ref: read_u32(payload, 52)?,
        byte_start: read_u64(payload, 56)?,
        byte_length: read_u64(payload, 64)?,
        unicode_scalar_start: read_u64(payload, 72)?,
        unicode_scalar_length: read_u64(payload, 80)?,
        token_start: read_u64(payload, 88)?,
        token_count: read_u32(payload, 96)?,
        parent_chunk_id: read_u64(payload, 100)?,
        first_child_ref: read_u32(payload, 108)?,
        child_count: read_u32(payload, 112)?,
        previous_chunk_id: read_u64(payload, 116)?,
        next_chunk_id: read_u64(payload, 124)?,
        chunk_text_hash_ref: read_u32(payload, 132)?,
        evidence_ref: read_u32(payload, 136)?,
        policy_ref: read_u32(payload, 140)?,
        flags: read_u32(payload, 144)?,
        checksum: read_u32(payload, 148)?,
    })
}

pub(super) fn parse_tokenizer_profile(payload: &[u8]) -> Result<TokenizerProfileV1, CoveError> {
    exact_len!(payload, 92, "TokenizerProfileV1");
    verify_payload_crc(payload, 88)?;
    let record = TokenizerProfileV1 {
        tokenizer_profile_id: read_u32(payload, 0)?,
        tokenizer_namespace_ref: read_u32(payload, 4)?,
        tokenizer_name_ref: read_u32(payload, 8)?,
        tokenizer_version_major: read_u16(payload, 12)?,
        tokenizer_version_minor: read_u16(payload, 14)?,
        vocab_digest_ref: read_u32(payload, 16)?,
        merges_digest_ref: read_u32(payload, 20)?,
        pre_tokenizer_digest_ref: read_u32(payload, 24)?,
        normalizer_digest_ref: read_u32(payload, 28)?,
        byte_encoder_digest_ref: read_u32(payload, 32)?,
        special_tokens_digest_ref: read_u32(payload, 36)?,
        added_tokens_digest_ref: read_u32(payload, 40)?,
        chat_template_ref: read_u32(payload, 44)?,
        unicode_version_ref: read_u32(payload, 48)?,
        truncation_policy_ref: read_u32(payload, 52)?,
        padding_policy_ref: read_u32(payload, 56)?,
        model_max_sequence_length: read_u32(payload, 60)?,
        token_id_width: read_u8(payload, 64)?,
        byte_alignment_available: read_u8(payload, 65)?,
        reversible: read_u8(payload, 66)?,
        deterministic: read_u8(payload, 67)?,
        bos_token_id: read_u32(payload, 68)?,
        eos_token_id: read_u32(payload, 72)?,
        pad_token_id: read_u32(payload, 76)?,
        unk_token_id: read_u32(payload, 80)?,
        flags: read_u32(payload, 84)?,
        checksum: read_u32(payload, 88)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_token_block(payload: &[u8]) -> Result<TokenBlockHeaderV1, CoveError> {
    exact_len!(payload, 49, "TokenBlockHeaderV1");
    verify_payload_crc(payload, 45)?;
    Ok(TokenBlockHeaderV1 {
        token_block_id: read_u32(payload, 0)?,
        tokenizer_profile_id: read_u32(payload, 4)?,
        token_count: read_u64(payload, 8)?,
        token_id_width: read_u8(payload, 16)?,
        compression_codec: read_u8(payload, 17)?,
        layout_kind: read_u8(payload, 18)?,
        payload_ref: read_u32(payload, 19)?,
        payload_offset: read_u64(payload, 23)?,
        payload_length: read_u64(payload, 31)?,
        integrity_ref: read_u32(payload, 39)?,
        checksum: read_u32(payload, 45)?,
    })
}

pub(super) fn parse_tokenized_span(payload: &[u8]) -> Result<TokenizedSpanV1, CoveError> {
    exact_len!(payload, 52, "TokenizedSpanV1");
    verify_payload_crc(payload, 48)?;
    Ok(TokenizedSpanV1 {
        tokenized_span_id: read_u64(payload, 0)?,
        chunk_id: read_u64(payload, 8)?,
        tokenizer_profile_id: read_u32(payload, 16)?,
        token_block_ref: read_u32(payload, 20)?,
        token_offset: read_u64(payload, 24)?,
        token_count: read_u32(payload, 32)?,
        byte_alignment_ref: read_u32(payload, 36)?,
        source_value_hash_ref: read_u32(payload, 40)?,
        flags: read_u32(payload, 44)?,
        checksum: read_u32(payload, 48)?,
    })
}

pub(super) fn parse_token_sequence_pack(payload: &[u8]) -> Result<TokenSequencePackV1, CoveError> {
    exact_len!(payload, 72, "TokenSequencePackV1");
    verify_payload_crc(payload, 68)?;
    Ok(TokenSequencePackV1 {
        sequence_pack_id: read_u64(payload, 0)?,
        tokenizer_profile_id: read_u32(payload, 8)?,
        training_profile_ref: read_u32(payload, 12)?,
        token_block_ref: read_u32(payload, 16)?,
        token_offset: read_u64(payload, 20)?,
        token_count: read_u32(payload, 28)?,
        source_span_count: read_u32(payload, 32)?,
        first_source_span_ref: read_u32(payload, 36)?,
        loss_mask_ref: read_u32(payload, 40)?,
        attention_mask_ref: read_u32(payload, 44)?,
        position_ids_ref: read_u32(payload, 48)?,
        labels_ref: read_u32(payload, 52)?,
        split_ref: read_u32(payload, 56)?,
        sample_weight_ppm: read_u32(payload, 60)?,
        flags: read_u32(payload, 64)?,
        checksum: read_u32(payload, 68)?,
    })
}

pub(super) fn parse_training_profile(payload: &[u8]) -> Result<TrainingProfileV1, CoveError> {
    exact_len!(payload, 78, "TrainingProfileV1");
    verify_payload_crc(payload, 74)?;
    let record = TrainingProfileV1 {
        training_profile_id: read_u32(payload, 0)?,
        profile_name_ref: read_u32(payload, 4)?,
        task_family: read_u8(payload, 8)?,
        modality_mask: read_u32(payload, 9)?,
        source_snapshot_ref: read_u32(payload, 13)?,
        map_profile_ref: read_u32(payload, 17)?,
        chunk_profile_ref: read_u32(payload, 21)?,
        tokenizer_profile_ref: read_u32(payload, 25)?,
        vector_space_ref: read_u32(payload, 29)?,
        multimodal_sequence_profile_ref: read_u32(payload, 33)?,
        split_policy_ref: read_u32(payload, 37)?,
        sampling_policy_ref: read_u32(payload, 41)?,
        dedup_policy_ref: read_u32(payload, 45)?,
        quality_policy_ref: read_u32(payload, 49)?,
        license_policy_ref: read_u32(payload, 53)?,
        redaction_policy_ref: read_u32(payload, 57)?,
        default_generator_provenance_ref: read_u64(payload, 61)?,
        reproducibility_class: read_u8(payload, 69)?,
        flags: read_u32(payload, 70)?,
        checksum: read_u32(payload, 74)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_training_sample_entry(
    payload: &[u8],
) -> Result<TrainingSampleEntryV1, CoveError> {
    exact_len!(payload, 121, "TrainingSampleEntryV1");
    verify_payload_crc(payload, 117)?;
    Ok(TrainingSampleEntryV1 {
        sample_id: read_u64(payload, 0)?,
        training_profile_id: read_u32(payload, 8)?,
        example_kind: read_u8(payload, 12)?,
        split_ref: read_u32(payload, 13)?,
        source_ref: read_u32(payload, 17)?,
        evidence_ref: read_u32(payload, 21)?,
        input_ref: read_u32(payload, 25)?,
        target_ref: read_u32(payload, 29)?,
        label_ref: read_u32(payload, 33)?,
        metadata_ref: read_u32(payload, 37)?,
        token_sequence_pack_ref: read_u64(payload, 41)?,
        multimodal_sequence_pack_ref: read_u64(payload, 49)?,
        vector_ref: read_u64(payload, 57)?,
        quality_score_ppm: read_u32(payload, 65)?,
        sample_weight_ppm: read_u32(payload, 69)?,
        dedup_group_ref: read_u32(payload, 73)?,
        license_ref: read_u32(payload, 77)?,
        policy_ref: read_u32(payload, 81)?,
        teacher_model_ref: read_u32(payload, 85)?,
        generator_provenance_ref: read_u64(payload, 89)?,
        judge_generator_provenance_ref: read_u64(payload, 97)?,
        label_generator_provenance_ref: read_u64(payload, 105)?,
        flags: read_u32(payload, 113)?,
        checksum: read_u32(payload, 117)?,
    })
}

pub(super) fn parse_dataset_split(payload: &[u8]) -> Result<DatasetSplitV1, CoveError> {
    exact_len!(payload, 69, "DatasetSplitV1");
    verify_payload_crc(payload, 65)?;
    let record = DatasetSplitV1 {
        split_id: read_u32(payload, 0)?,
        split_name_ref: read_u32(payload, 4)?,
        split_method: read_u8(payload, 8)?,
        source_snapshot_ref: read_u32(payload, 9)?,
        filter_policy_ref: read_u32(payload, 13)?,
        seed: read_u64(payload, 17)?,
        hash_function_ref: read_u32(payload, 25)?,
        stratification_path_ref: read_u32(payload, 29)?,
        grouping_ref: read_u32(payload, 33)?,
        ordering_policy_ref: read_u32(payload, 37)?,
        dedup_policy_ref: read_u32(payload, 41)?,
        sample_count: read_u64(payload, 45)?,
        first_sample_ref: read_u64(payload, 53)?,
        flags: read_u32(payload, 61)?,
        checksum: read_u32(payload, 65)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_dedup_group(payload: &[u8]) -> Result<DedupGroupV1, CoveError> {
    exact_len!(payload, 42, "DedupGroupV1");
    verify_payload_crc(payload, 38)?;
    let record = DedupGroupV1 {
        dedup_group_id: read_u64(payload, 0)?,
        dedup_policy_ref: read_u32(payload, 8)?,
        canonical_member_sample_id: read_u64(payload, 12)?,
        similarity_kind: read_u8(payload, 20)?,
        dedup_authority: read_u8(payload, 21)?,
        confidence_ppm: read_u32(payload, 22)?,
        first_member_ref: read_u32(payload, 26)?,
        member_count: read_u32(payload, 30)?,
        flags: read_u32(payload, 34)?,
        checksum: read_u32(payload, 38)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_training_epoch_plan(payload: &[u8]) -> Result<TrainingEpochPlanV1, CoveError> {
    exact_len!(payload, 53, "TrainingEpochPlanV1");
    verify_payload_crc(payload, 49)?;
    Ok(TrainingEpochPlanV1 {
        epoch_plan_id: read_u64(payload, 0)?,
        training_profile_id: read_u32(payload, 8)?,
        split_ref: read_u32(payload, 12)?,
        seed: read_u64(payload, 16)?,
        permutation_kind: read_u8(payload, 24)?,
        rng_algorithm_ref: read_u32(payload, 25)?,
        permutation_function_ref: read_u32(payload, 29)?,
        shard_count: read_u32(payload, 33)?,
        first_shard_ref: read_u32(payload, 37)?,
        shard_ref_count: read_u32(payload, 41)?,
        flags: read_u32(payload, 45)?,
        checksum: read_u32(payload, 49)?,
    })
}

pub(super) fn parse_training_label_entry(
    payload: &[u8],
) -> Result<TrainingLabelEntryV1, CoveError> {
    exact_len!(payload, 46, "TrainingLabelEntryV1");
    verify_payload_crc(payload, 42)?;
    Ok(TrainingLabelEntryV1 {
        label_id: read_u64(payload, 0)?,
        label_kind: read_u8(payload, 8)?,
        label_authority: read_u8(payload, 9)?,
        label_payload_ref: read_u32(payload, 10)?,
        generator_provenance_ref: read_u64(payload, 14)?,
        human_review_ref: read_u32(payload, 22)?,
        confidence_ppm: read_u32(payload, 26)?,
        evidence_ref: read_u32(payload, 30)?,
        policy_ref: read_u32(payload, 34)?,
        flags: read_u32(payload, 38)?,
        checksum: read_u32(payload, 42)?,
    })
}

pub(super) fn parse_preference_pair_entry(
    payload: &[u8],
) -> Result<PreferencePairEntryV1, CoveError> {
    exact_len!(payload, 56, "PreferencePairEntryV1");
    verify_payload_crc(payload, 52)?;
    Ok(PreferencePairEntryV1 {
        preference_pair_id: read_u64(payload, 0)?,
        prompt_ref: read_u32(payload, 8)?,
        chosen_ref: read_u32(payload, 12)?,
        rejected_ref: read_u32(payload, 16)?,
        judge_generator_provenance_ref: read_u64(payload, 20)?,
        human_review_ref: read_u32(payload, 28)?,
        preference_strength_ppm: read_u32(payload, 32)?,
        confidence_ppm: read_u32(payload, 36)?,
        evidence_ref: read_u32(payload, 40)?,
        policy_ref: read_u32(payload, 44)?,
        flags: read_u32(payload, 48)?,
        checksum: read_u32(payload, 52)?,
    })
}

pub(super) fn parse_generator_provenance(
    payload: &[u8],
) -> Result<GeneratorProvenanceV1, CoveError> {
    exact_len!(payload, 78, "GeneratorProvenanceV1");
    verify_payload_crc(payload, 74)?;
    Ok(GeneratorProvenanceV1 {
        generator_provenance_id: read_u64(payload, 0)?,
        generator_kind: read_u8(payload, 8)?,
        model_actor_ref: read_u32(payload, 9)?,
        prompt_template_ref: read_u32(payload, 13)?,
        decoding_profile_ref: read_u32(payload, 17)?,
        toolchain_ref: read_u32(payload, 21)?,
        source_input_ref: read_u32(payload, 25)?,
        source_context_ref: read_u32(payload, 29)?,
        source_sample_ref: read_u64(payload, 33)?,
        parent_generator_provenance_ref: read_u64(payload, 41)?,
        generation_time_us: read_i64(payload, 49)?,
        confidence_ppm: read_u32(payload, 57)?,
        human_review_ref: read_u32(payload, 61)?,
        policy_ref: read_u32(payload, 65)?,
        reproducibility_class: read_u8(payload, 69)?,
        flags: read_u32(payload, 70)?,
        checksum: read_u32(payload, 74)?,
    })
}

pub(super) fn parse_model_actor(payload: &[u8]) -> Result<ModelActorDescriptorV1, CoveError> {
    exact_len!(payload, 56, "ModelActorDescriptorV1");
    verify_payload_crc(payload, 52)?;
    let record = ModelActorDescriptorV1 {
        model_actor_id: read_u32(payload, 0)?,
        model_namespace_ref: read_u32(payload, 4)?,
        model_name_ref: read_u32(payload, 8)?,
        model_version_ref: read_u32(payload, 12)?,
        model_checkpoint_digest_ref: read_u32(payload, 16)?,
        provider_ref: read_u32(payload, 20)?,
        endpoint_ref: read_u32(payload, 24)?,
        endpoint_version_ref: read_u32(payload, 28)?,
        model_family_ref: read_u32(payload, 32)?,
        modality_mask: read_u32(payload, 36)?,
        license_ref: read_u32(payload, 40)?,
        policy_ref: read_u32(payload, 44)?,
        flags: read_u32(payload, 48)?,
        checksum: read_u32(payload, 52)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_generation_decoding_profile(
    payload: &[u8],
) -> Result<GenerationDecodingProfileV1, CoveError> {
    exact_len!(payload, 45, "GenerationDecodingProfileV1");
    verify_payload_crc(payload, 41)?;
    let record = GenerationDecodingProfileV1 {
        decoding_profile_id: read_u32(payload, 0)?,
        temperature_micros: read_u32(payload, 4)?,
        top_p_micros: read_u32(payload, 8)?,
        top_k: read_u32(payload, 12)?,
        seed: read_u64(payload, 16)?,
        max_output_tokens: read_u32(payload, 24)?,
        stop_sequence_ref: read_u32(payload, 28)?,
        safety_policy_ref: read_u32(payload, 32)?,
        deterministic_claim: read_u8(payload, 36)?,
        flags: read_u32(payload, 37)?,
        checksum: read_u32(payload, 41)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_human_review(payload: &[u8]) -> Result<HumanReviewEntryV1, CoveError> {
    exact_len!(payload, 37, "HumanReviewEntryV1");
    verify_payload_crc(payload, 33)?;
    let record = HumanReviewEntryV1 {
        human_review_id: read_u32(payload, 0)?,
        review_kind: read_u8(payload, 4)?,
        reviewer_role_ref: read_u32(payload, 5)?,
        review_time_us: read_i64(payload, 9)?,
        rating_ppm: read_u32(payload, 17)?,
        notes_ref: read_u32(payload, 21)?,
        policy_ref: read_u32(payload, 25)?,
        flags: read_u32(payload, 29)?,
        checksum: read_u32(payload, 33)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_tensor_layout(payload: &[u8]) -> Result<TensorLayoutDescriptorV1, CoveError> {
    exact_len!(payload, 65, "TensorLayoutDescriptorV1");
    verify_payload_crc(payload, 61)?;
    let record = TensorLayoutDescriptorV1 {
        tensor_layout_id: read_u32(payload, 0)?,
        layout_name_ref: read_u32(payload, 4)?,
        rank: read_u8(payload, 8)?,
        dtype: read_u8(payload, 9)?,
        byte_order: read_u8(payload, 10)?,
        shape_ref: read_u32(payload, 11)?,
        stride_ref: read_u32(payload, 15)?,
        storage_offset_elements: read_i64(payload, 19)?,
        layout_kind: read_u8(payload, 27)?,
        memory_alignment_bytes: read_u32(payload, 28)?,
        preferred_page_alignment_bytes: read_u32(payload, 32)?,
        tile_shape_ref: read_u32(payload, 36)?,
        block_shape_ref: read_u32(payload, 40)?,
        quantization_profile_ref: read_u32(payload, 44)?,
        sparsity_profile_ref: read_u32(payload, 48)?,
        framework_compatibility_ref: read_u32(payload, 52)?,
        device_affinity_hint: read_u8(payload, 56)?,
        flags: read_u32(payload, 57)?,
        checksum: read_u32(payload, 61)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_device_transfer_hint(
    payload: &[u8],
) -> Result<DeviceTransferHintV1, CoveError> {
    exact_len!(payload, 28, "DeviceTransferHintV1");
    verify_payload_crc(payload, 24)?;
    let record = DeviceTransferHintV1 {
        transfer_hint_id: read_u32(payload, 0)?,
        target_kind: read_u8(payload, 4)?,
        preferred_alignment_bytes: read_u32(payload, 5)?,
        preferred_chunk_bytes: read_u32(payload, 9)?,
        pinned_memory_required: read_u8(payload, 13)?,
        contiguous_required: read_u8(payload, 14)?,
        zero_copy_possible: read_u8(payload, 15)?,
        runtime_registry_binding_ref: read_u32(payload, 16)?,
        flags: read_u32(payload, 20)?,
        checksum: read_u32(payload, 24)?,
    };
    record.validate_static()?;
    Ok(record)
}

pub(super) fn parse_ai_asset_ref(payload: &[u8]) -> Result<AiAssetRefV1, CoveError> {
    exact_len!(payload, 99, "AiAssetRefV1");
    verify_payload_crc(payload, 95)?;
    Ok(AiAssetRefV1 {
        asset_ref_id: read_u64(payload, 0)?,
        parent_asset_ref: read_u64(payload, 8)?,
        asset_kind: read_u8(payload, 16)?,
        uri_ref: read_u32(payload, 17)?,
        embedded_section_ref: read_u32(payload, 21)?,
        media_type_ref: read_u32(payload, 25)?,
        byte_length: read_u64(payload, 29)?,
        digest_ref: read_u32(payload, 37)?,
        width: read_u32(payload, 41)?,
        height: read_u32(payload, 45)?,
        duration_us: read_u64(payload, 49)?,
        sample_rate_hz: read_u32(payload, 57)?,
        channel_count: read_u16(payload, 61)?,
        decode_profile_ref: read_u32(payload, 63)?,
        preprocessing_profile_ref: read_u32(payload, 67)?,
        transform_profile_ref: read_u32(payload, 71)?,
        transform_digest_ref: read_u32(payload, 75)?,
        tensor_layout_ref: read_u32(payload, 79)?,
        license_ref: read_u32(payload, 83)?,
        policy_ref: read_u32(payload, 87)?,
        flags: read_u32(payload, 91)?,
        checksum: read_u32(payload, 95)?,
    })
}

pub(super) fn parse_multimodal_sequence_pack(
    payload: &[u8],
) -> Result<MultimodalSequencePackV1, CoveError> {
    exact_len!(payload, 76, "MultimodalSequencePackV1");
    verify_payload_crc(payload, 72)?;
    Ok(MultimodalSequencePackV1 {
        sequence_pack_id: read_u64(payload, 0)?,
        training_profile_id: read_u32(payload, 8)?,
        tokenizer_profile_id: read_u32(payload, 12)?,
        sequence_profile_ref: read_u32(payload, 16)?,
        element_count: read_u32(payload, 20)?,
        first_element_ref: read_u32(payload, 24)?,
        split_ref: read_u32(payload, 28)?,
        sample_weight_ppm: read_u32(payload, 32)?,
        loss_mask_ref: read_u32(payload, 36)?,
        attention_mask_ref: read_u32(payload, 40)?,
        position_map_ref: read_u32(payload, 44)?,
        label_ref: read_u32(payload, 48)?,
        source_snapshot_ref: read_u32(payload, 52)?,
        evidence_ref: read_u32(payload, 56)?,
        generator_provenance_ref: read_u64(payload, 60)?,
        flags: read_u32(payload, 68)?,
        checksum: read_u32(payload, 72)?,
    })
}

pub(super) fn parse_multimodal_sequence_element(
    payload: &[u8],
) -> Result<MultimodalSequenceElementV1, CoveError> {
    exact_len!(payload, 115, "MultimodalSequenceElementV1");
    verify_payload_crc(payload, 111)?;
    Ok(MultimodalSequenceElementV1 {
        element_id: read_u64(payload, 0)?,
        sequence_pack_id: read_u64(payload, 8)?,
        ordinal: read_u32(payload, 16)?,
        element_kind: read_u8(payload, 20)?,
        modality: read_u8(payload, 21)?,
        role: read_u8(payload, 22)?,
        tokenized_span_ref: read_u64(payload, 23)?,
        token_sequence_pack_ref: read_u64(payload, 31)?,
        asset_ref: read_u64(payload, 39)?,
        tensor_ref: read_u64(payload, 47)?,
        vector_ref: read_u64(payload, 55)?,
        byte_start: read_u64(payload, 63)?,
        byte_length: read_u64(payload, 71)?,
        time_start_us: read_i64(payload, 79)?,
        time_duration_us: read_i64(payload, 87)?,
        position_stream_ref: read_u32(payload, 95)?,
        evidence_ref: read_u32(payload, 99)?,
        policy_ref: read_u32(payload, 103)?,
        flags: read_u32(payload, 107)?,
        checksum: read_u32(payload, 111)?,
    })
}

pub(super) fn parse_vector_space(payload: &[u8]) -> Result<VectorSpaceDescriptorV1, CoveError> {
    exact_len!(payload, 60, "VectorSpaceDescriptorV1");
    verify_payload_crc(payload, 56)?;
    Ok(VectorSpaceDescriptorV1 {
        vector_space_id: read_u32(payload, 0)?,
        vector_space_name_ref: read_u32(payload, 4)?,
        vector_space_fingerprint_ref: read_u32(payload, 8)?,
        embedding_namespace_ref: read_u32(payload, 12)?,
        embedding_model_ref: read_u32(payload, 16)?,
        embedding_model_version_ref: read_u32(payload, 20)?,
        embedding_model_digest_ref: read_u32(payload, 24)?,
        embedding_pipeline_ref: read_u32(payload, 28)?,
        tokenizer_profile_ref: read_u32(payload, 32)?,
        chunk_profile_ref: read_u32(payload, 36)?,
        dimension_count: read_u32(payload, 40)?,
        element_type: read_u8(payload, 44)?,
        metric: read_u8(payload, 45)?,
        normalization_policy: read_u8(payload, 46)?,
        quantization_policy: read_u8(payload, 47)?,
        deterministic: read_u8(payload, 48)?,
        approximate: read_u8(payload, 49)?,
        reproducibility_class: read_u8(payload, 50)?,
        reserved: read_u8(payload, 51)?,
        flags: read_u32(payload, 52)?,
        checksum: read_u32(payload, 56)?,
    })
}

pub(super) fn parse_vector_space_compatibility(
    payload: &[u8],
) -> Result<VectorSpaceCompatibilityDescriptorV1, CoveError> {
    exact_len!(payload, 44, "VectorSpaceCompatibilityDescriptorV1");
    verify_payload_crc(payload, 40)?;
    Ok(VectorSpaceCompatibilityDescriptorV1 {
        compatibility_id: read_u32(payload, 0)?,
        left_vector_space_id: read_u32(payload, 4)?,
        right_vector_space_id: read_u32(payload, 8)?,
        compatibility_kind: read_u8(payload, 12)?,
        compatibility_authority: read_u8(payload, 13)?,
        metric: read_u8(payload, 14)?,
        normalization_policy: read_u8(payload, 15)?,
        transform_ref: read_u32(payload, 16)?,
        numeric_transform_error_ppm: read_u32(payload, 20)?,
        ranking_eval_ref: read_u32(payload, 24)?,
        calibration_dataset_ref: read_u32(payload, 28)?,
        evidence_ref: read_u32(payload, 32)?,
        flags: read_u32(payload, 36)?,
        checksum: read_u32(payload, 40)?,
    })
}

pub(super) fn parse_filecode_vector_binding(
    payload: &[u8],
    record_version: u16,
) -> Result<FileCodeVectorBindingV1, CoveError> {
    let (expected_len, checksum_offset) = match record_version {
        1 => (80, 76),
        2 => (84, 80),
        _ => unreachable!("record version was checked before vector binding parse"),
    };
    exact_len!(payload, expected_len, "FileCodeVectorBinding");
    verify_payload_crc(payload, checksum_offset)?;
    let model_input_digest_ref = if record_version == 2 {
        read_u32(payload, 72)?
    } else {
        0
    };
    reject_zero_v2_model_input_digest_ref(
        record_version,
        "FileCodeVectorBinding",
        model_input_digest_ref,
    )?;
    Ok(FileCodeVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        slot_policy_ref: read_u32(payload, 12)?,
        file_ref: read_u32(payload, 16)?,
        dictionary_digest_ref: read_u32(payload, 20)?,
        schema_fingerprint_ref: read_u32(payload, 24)?,
        table_id: read_u32(payload, 28)?,
        column_id: read_u32(payload, 32)?,
        object_type_id: read_u32(payload, 36)?,
        property_id: read_u32(payload, 40)?,
        association_type_id: read_u32(payload, 44)?,
        path_ref: read_u32(payload, 48)?,
        file_code: read_u32(payload, 52)?,
        reserved0: read_u32(payload, 56)?,
        canonical_value_hash_ref: read_u32(payload, 60)?,
        vector_ref: read_u64(payload, 64)?,
        model_input_digest_ref,
        flags: read_u32(payload, if record_version == 2 { 76 } else { 72 })?,
        checksum: read_u32(payload, checksum_offset)?,
    })
}

pub(super) fn parse_chunk_vector_binding(
    payload: &[u8],
    record_version: u16,
) -> Result<ChunkVectorBindingV1, CoveError> {
    let (expected_len, checksum_offset) = match record_version {
        1 => (48, 44),
        2 => (52, 48),
        _ => unreachable!("record version was checked before vector binding parse"),
    };
    exact_len!(payload, expected_len, "ChunkVectorBinding");
    verify_payload_crc(payload, checksum_offset)?;
    let model_input_digest_ref = if record_version == 2 {
        read_u32(payload, 40)?
    } else {
        0
    };
    reject_zero_v2_model_input_digest_ref(
        record_version,
        "ChunkVectorBinding",
        model_input_digest_ref,
    )?;
    Ok(ChunkVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        chunk_id: read_u64(payload, 12)?,
        chunk_profile_id: read_u32(payload, 20)?,
        source_value_hash_ref: read_u32(payload, 24)?,
        chunk_text_hash_ref: read_u32(payload, 28)?,
        vector_ref: read_u64(payload, 32)?,
        model_input_digest_ref,
        flags: read_u32(payload, if record_version == 2 { 44 } else { 40 })?,
        checksum: read_u32(payload, checksum_offset)?,
    })
}

pub(super) fn parse_object_state_vector_binding(
    payload: &[u8],
    record_version: u16,
) -> Result<ObjectStateVectorBindingV1, CoveError> {
    let (expected_len, checksum_offset) = match record_version {
        1 => (69, 65),
        2 => (73, 69),
        _ => unreachable!("record version was checked before vector binding parse"),
    };
    exact_len!(payload, expected_len, "ObjectStateVectorBinding");
    verify_payload_crc(payload, checksum_offset)?;
    let model_input_digest_ref = if record_version == 2 {
        read_u32(payload, 61)?
    } else {
        0
    };
    reject_zero_v2_model_input_digest_ref(
        record_version,
        "ObjectStateVectorBinding",
        model_input_digest_ref,
    )?;
    Ok(ObjectStateVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        composition_profile_ref: read_u32(payload, 12)?,
        file_ref: read_u32(payload, 16)?,
        object_type_id: read_u32(payload, 20)?,
        goid_ref: read_u32(payload, 24)?,
        branch_ref: read_u32(payload, 28)?,
        temporal_kind: read_u8(payload, 32)?,
        csn: read_u64(payload, 33)?,
        timestamp_us: read_i64(payload, 41)?,
        property_dependency_fingerprint_ref: read_u32(payload, 49)?,
        vector_ref: read_u64(payload, 53)?,
        model_input_digest_ref,
        flags: read_u32(payload, if record_version == 2 { 65 } else { 61 })?,
        checksum: read_u32(payload, checksum_offset)?,
    })
}

pub(super) fn parse_training_sample_vector_binding(
    payload: &[u8],
    record_version: u16,
) -> Result<TrainingSampleVectorBindingV1, CoveError> {
    let (expected_len, checksum_offset) = match record_version {
        1 => (48, 44),
        2 => (52, 48),
        _ => unreachable!("record version was checked before vector binding parse"),
    };
    exact_len!(payload, expected_len, "TrainingSampleVectorBinding");
    verify_payload_crc(payload, checksum_offset)?;
    let model_input_digest_ref = if record_version == 2 {
        read_u32(payload, 40)?
    } else {
        0
    };
    reject_zero_v2_model_input_digest_ref(
        record_version,
        "TrainingSampleVectorBinding",
        model_input_digest_ref,
    )?;
    Ok(TrainingSampleVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        training_profile_ref: read_u32(payload, 12)?,
        sample_id: read_u64(payload, 16)?,
        source_snapshot_ref: read_u32(payload, 24)?,
        sample_fingerprint_ref: read_u32(payload, 28)?,
        vector_ref: read_u64(payload, 32)?,
        model_input_digest_ref,
        flags: read_u32(payload, if record_version == 2 { 44 } else { 40 })?,
        checksum: read_u32(payload, checksum_offset)?,
    })
}

pub(super) fn parse_association_state_vector_binding(
    payload: &[u8],
    record_version: u16,
) -> Result<AssociationStateVectorBindingV1, CoveError> {
    let (expected_len, checksum_offset) = match record_version {
        1 => (69, 65),
        2 => (73, 69),
        _ => unreachable!("record version was checked before vector binding parse"),
    };
    exact_len!(payload, expected_len, "AssociationStateVectorBinding");
    verify_payload_crc(payload, checksum_offset)?;
    let model_input_digest_ref = if record_version == 2 {
        read_u32(payload, 61)?
    } else {
        0
    };
    reject_zero_v2_model_input_digest_ref(
        record_version,
        "AssociationStateVectorBinding",
        model_input_digest_ref,
    )?;
    Ok(AssociationStateVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        composition_profile_ref: read_u32(payload, 12)?,
        file_ref: read_u32(payload, 16)?,
        association_type_id: read_u32(payload, 20)?,
        association_key_ref: read_u32(payload, 24)?,
        branch_ref: read_u32(payload, 28)?,
        temporal_kind: read_u8(payload, 32)?,
        csn: read_u64(payload, 33)?,
        timestamp_us: read_i64(payload, 41)?,
        property_dependency_fingerprint_ref: read_u32(payload, 49)?,
        vector_ref: read_u64(payload, 53)?,
        model_input_digest_ref,
        flags: read_u32(payload, if record_version == 2 { 65 } else { 61 })?,
        checksum: read_u32(payload, checksum_offset)?,
    })
}

pub(super) fn parse_asset_vector_binding(
    payload: &[u8],
    record_version: u16,
) -> Result<AssetVectorBindingV1, CoveError> {
    let (expected_len, checksum_offset) = match record_version {
        1 => (44, 40),
        2 => (48, 44),
        _ => unreachable!("record version was checked before vector binding parse"),
    };
    exact_len!(payload, expected_len, "AssetVectorBinding");
    verify_payload_crc(payload, checksum_offset)?;
    let model_input_digest_ref = if record_version == 2 {
        read_u32(payload, 36)?
    } else {
        0
    };
    reject_zero_v2_model_input_digest_ref(
        record_version,
        "AssetVectorBinding",
        model_input_digest_ref,
    )?;
    Ok(AssetVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        asset_ref: read_u64(payload, 12)?,
        transform_ref: read_u32(payload, 20)?,
        asset_digest_ref: read_u32(payload, 24)?,
        vector_ref: read_u64(payload, 28)?,
        model_input_digest_ref,
        flags: read_u32(payload, if record_version == 2 { 40 } else { 36 })?,
        checksum: read_u32(payload, checksum_offset)?,
    })
}

pub(super) fn parse_multimodal_sequence_vector_binding(
    payload: &[u8],
    record_version: u16,
) -> Result<MultimodalSequenceVectorBindingV1, CoveError> {
    let (expected_len, checksum_offset) = match record_version {
        1 => (44, 40),
        2 => (48, 44),
        _ => unreachable!("record version was checked before vector binding parse"),
    };
    exact_len!(payload, expected_len, "MultimodalSequenceVectorBinding");
    verify_payload_crc(payload, checksum_offset)?;
    let model_input_digest_ref = if record_version == 2 {
        read_u32(payload, 36)?
    } else {
        0
    };
    reject_zero_v2_model_input_digest_ref(
        record_version,
        "MultimodalSequenceVectorBinding",
        model_input_digest_ref,
    )?;
    Ok(MultimodalSequenceVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        sequence_pack_id: read_u64(payload, 12)?,
        sequence_profile_ref: read_u32(payload, 20)?,
        source_snapshot_ref: read_u32(payload, 24)?,
        vector_ref: read_u64(payload, 28)?,
        model_input_digest_ref,
        flags: read_u32(payload, if record_version == 2 { 40 } else { 36 })?,
        checksum: read_u32(payload, checksum_offset)?,
    })
}

pub(super) fn reject_zero_v2_model_input_digest_ref(
    record_version: u16,
    label: &str,
    model_input_digest_ref: u32,
) -> Result<(), CoveError> {
    if record_version == 2 && model_input_digest_ref == 0 {
        return Err(CoveError::BadSection(format!(
            "{label}V2 requires non-zero model_input_digest_ref"
        )));
    }
    Ok(())
}

pub(super) fn parse_vector_payload_block(
    payload: &[u8],
) -> Result<VectorPayloadBlockHeaderV1, CoveError> {
    exact_len!(payload, 68, "VectorPayloadBlockHeaderV1");
    verify_payload_crc(payload, 64)?;
    Ok(VectorPayloadBlockHeaderV1 {
        block_id: read_u32(payload, 0)?,
        vector_space_id: read_u32(payload, 4)?,
        vector_count: read_u64(payload, 8)?,
        dimension_count: read_u32(payload, 16)?,
        element_type: read_u8(payload, 20)?,
        compression_codec: read_u8(payload, 21)?,
        quantization_kind: read_u8(payload, 22)?,
        layout_kind: read_u8(payload, 23)?,
        tensor_layout_ref: read_u32(payload, 24)?,
        memory_alignment_bytes: read_u32(payload, 28)?,
        payload_stride_ref: read_u32(payload, 32)?,
        device_transfer_hint_ref: read_u32(payload, 36)?,
        payload_ref: read_u32(payload, 40)?,
        payload_offset: read_u64(payload, 44)?,
        payload_length: read_u64(payload, 52)?,
        integrity_ref: read_u32(payload, 60)?,
        checksum: read_u32(payload, 64)?,
    })
}

pub(super) fn parse_vector_entry(payload: &[u8]) -> Result<VectorEntryV1, CoveError> {
    exact_len!(payload, 44, "VectorEntryV1");
    verify_payload_crc(payload, 40)?;
    Ok(VectorEntryV1 {
        vector_ref: read_u64(payload, 0)?,
        block_id: read_u32(payload, 8)?,
        vector_ordinal: read_u64(payload, 12)?,
        payload_offset: read_u64(payload, 20)?,
        payload_length: read_u32(payload, 28)?,
        integrity_ref: read_u32(payload, 32)?,
        flags: read_u32(payload, 36)?,
        checksum: read_u32(payload, 40)?,
    })
}

pub(super) fn parse_vector_composition_profile(
    payload: &[u8],
) -> Result<VectorCompositionProfileV1, CoveError> {
    exact_len!(payload, 42, "VectorCompositionProfileV1");
    verify_payload_crc(payload, 38)?;
    Ok(VectorCompositionProfileV1 {
        composition_profile_id: read_u32(payload, 0)?,
        composition_name_ref: read_u32(payload, 4)?,
        output_vector_space_id: read_u32(payload, 8)?,
        arithmetic_profile_ref: read_u32(payload, 12)?,
        method: read_u8(payload, 16)?,
        missing_policy: read_u8(payload, 17)?,
        normalize_inputs: read_u8(payload, 18)?,
        normalize_output: read_u8(payload, 19)?,
        result_authority: read_u8(payload, 20)?,
        reproducibility_class: read_u8(payload, 21)?,
        first_component_ref: read_u32(payload, 22)?,
        component_count: read_u32(payload, 26)?,
        template_ref: read_u32(payload, 30)?,
        flags: read_u32(payload, 34)?,
        checksum: read_u32(payload, 38)?,
    })
}

pub(super) fn parse_vector_composition_component(
    payload: &[u8],
) -> Result<VectorCompositionComponentV1, CoveError> {
    exact_len!(payload, 28, "VectorCompositionComponentV1");
    verify_payload_crc(payload, 24)?;
    Ok(VectorCompositionComponentV1 {
        component_id: read_u32(payload, 0)?,
        slot_policy_ref: read_u32(payload, 4)?,
        source_vector_space_id: read_u32(payload, 8)?,
        weight_ppm: read_u32(payload, 12)?,
        required: read_u8(payload, 16)?,
        redaction_behavior: read_u8(payload, 17)?,
        missing_behavior: read_u8(payload, 18)?,
        reserved: read_u8(payload, 19)?,
        flags: read_u32(payload, 20)?,
        checksum: read_u32(payload, 24)?,
    })
}

pub(super) fn parse_vector_arithmetic_profile(
    payload: &[u8],
) -> Result<VectorArithmeticProfileV1, CoveError> {
    exact_len!(payload, 29, "VectorArithmeticProfileV1");
    verify_payload_crc(payload, 25)?;
    Ok(VectorArithmeticProfileV1 {
        arithmetic_profile_id: read_u32(payload, 0)?,
        profile_name_ref: read_u32(payload, 4)?,
        arithmetic_kind: read_u8(payload, 8)?,
        input_quantization_kind: read_u8(payload, 9)?,
        accumulator_kind: read_u8(payload, 10)?,
        rounding_mode: read_u8(payload, 11)?,
        overflow_policy: read_u8(payload, 12)?,
        component_order: read_u8(payload, 13)?,
        weight_scale: read_u32(payload, 14)?,
        output_quantization_kind: read_u8(payload, 18)?,
        output_element_type: read_u8(payload, 19)?,
        normalization_policy: read_u8(payload, 20)?,
        flags: read_u32(payload, 21)?,
        checksum: read_u32(payload, 25)?,
    })
}

pub(super) fn parse_vector_index(payload: &[u8]) -> Result<VectorIndexDescriptorV1, CoveError> {
    exact_len!(payload, 54, "VectorIndexDescriptorV1");
    verify_payload_crc(payload, 50)?;
    Ok(VectorIndexDescriptorV1 {
        vector_index_id: read_u32(payload, 0)?,
        vector_space_id: read_u32(payload, 4)?,
        stored_vector_space_id: read_u32(payload, 8)?,
        search_vector_space_id: read_u32(payload, 12)?,
        index_kind: read_u8(payload, 16)?,
        exactness_kind: read_u8(payload, 17)?,
        false_negative_policy: read_u8(payload, 18)?,
        metric: read_u8(payload, 19)?,
        score_space_authority: read_u8(payload, 20)?,
        dimension_count: read_u32(payload, 21)?,
        indexed_binding_kind: read_u8(payload, 25)?,
        temporal_scope_ref: read_u32(payload, 26)?,
        visibility_scope_ref: read_u32(payload, 30)?,
        redaction_scope_ref: read_u32(payload, 34)?,
        dequantization_profile_ref: read_u32(payload, 38)?,
        quantization_error_profile_ref: read_u32(payload, 42)?,
        payload_ref: read_u32(payload, 46)?,
        checksum: read_u32(payload, 50)?,
    })
}

pub(super) fn put_u8(bytes: &mut [u8], offset: usize, value: u8) {
    bytes[offset] = value;
}

pub(super) fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_i64(bytes: &mut [u8], offset: usize, value: i64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
