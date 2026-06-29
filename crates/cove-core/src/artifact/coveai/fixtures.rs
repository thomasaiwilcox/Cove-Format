use super::*;

pub(super) fn f32_payload(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub(super) fn i64_payload(values: &[i64]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 8);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub(super) fn test_vector_space(vector_space_id: u32) -> VectorSpaceDescriptorV1 {
    VectorSpaceDescriptorV1 {
        vector_space_id,
        vector_space_name_ref: 0,
        vector_space_fingerprint_ref: 0,
        embedding_namespace_ref: 0,
        embedding_model_ref: 0,
        embedding_model_version_ref: 0,
        embedding_model_digest_ref: 0,
        embedding_pipeline_ref: 0,
        tokenizer_profile_ref: 0,
        chunk_profile_ref: 0,
        dimension_count: 3,
        element_type: 0,
        metric: 1,
        normalization_policy: 0,
        quantization_policy: 0,
        deterministic: 1,
        approximate: 0,
        reproducibility_class: 1,
        reserved: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_vector_space_compatibility() -> VectorSpaceCompatibilityDescriptorV1 {
    VectorSpaceCompatibilityDescriptorV1 {
        compatibility_id: 1,
        left_vector_space_id: 1,
        right_vector_space_id: 2,
        compatibility_kind: 1,
        compatibility_authority: 0,
        metric: 1,
        normalization_policy: 0,
        transform_ref: 0,
        numeric_transform_error_ppm: 0,
        ranking_eval_ref: 0,
        calibration_dataset_ref: 0,
        evidence_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn add_model_input_digest(
    tables: &mut AiDescriptorTablesV1,
    digest_ref: u32,
    domain_hint: u8,
) {
    tables.digests.push(AiDigestEntryV1 {
        digest_ref,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint,
        flags: 0,
        crc32c: 0,
    });
}

pub(super) fn add_chunk_binding_support(tables: &mut AiDescriptorTablesV1, chunk_count: u64) {
    tables.chunk_profiles.push(test_chunk_profile());
    for chunk_id in 1..=chunk_count {
        let mut chunk = test_text_chunk(chunk_id, 0, 0, (chunk_id - 1) * 12, 12, 12, 1);
        chunk.source_ref = 0;
        tables.text_chunks.push(chunk);
    }
}

pub(super) fn vector_descriptor_tables_with_payload(
) -> (AiDescriptorTablesV1, Vec<CoveAiWritableSection>) {
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 36,
        decoded_length: 36,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.vector_spaces.push(test_vector_space(1));
    tables.vector_spaces.push(test_vector_space(2));
    tables
        .vector_payload_blocks
        .push(VectorPayloadBlockHeaderV1 {
            block_id: 1,
            vector_space_id: 1,
            vector_count: 3,
            dimension_count: 3,
            element_type: 0,
            compression_codec: 0,
            quantization_kind: 0,
            layout_kind: 0,
            tensor_layout_ref: 0,
            memory_alignment_bytes: 0,
            payload_stride_ref: 0,
            device_transfer_hint_ref: 0,
            payload_ref: 1,
            payload_offset: 0,
            payload_length: 0,
            integrity_ref: 0,
            checksum: 0,
        });
    for vector_ref in 1..=3 {
        tables.vector_entries.push(VectorEntryV1 {
            vector_ref,
            block_id: 1,
            vector_ordinal: vector_ref - 1,
            payload_offset: 0,
            payload_length: 0,
            integrity_ref: 0,
            flags: 0,
            checksum: 0,
        });
    }
    let sections = vec![coveai_payload_section(
        1,
        SectionKind::AiPayloadBytes,
        PrimaryProfile::CoveVec,
        f32_payload(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.25, 0.5, 0.75]),
    )];
    (tables, sections)
}

pub(super) fn vector_binding_artifact_sections(
    tables: &AiDescriptorTablesV1,
    mut payload_sections: Vec<CoveAiWritableSection>,
    vector_binding_records: Vec<Vec<u8>>,
    vector_binding_required_ai_features: u64,
) -> Vec<CoveAiWritableSection> {
    let mut reference_records = Vec::new();
    for record in &tables.digests {
        reference_records.push(encode_digest_entry(record.clone()).unwrap());
    }
    for record in &tables.payload_refs {
        reference_records.push(encode_payload_ref_entry(record.clone()).unwrap());
    }
    if !reference_records.is_empty() {
        payload_sections.push(coveai_binary_section(
            100,
            SectionKind::AiReferenceTables,
            PrimaryProfile::CoveAiShared,
            encode_records(reference_records).unwrap(),
        ));
    }
    if !tables.chunk_profiles.is_empty() {
        payload_sections.push(coveai_binary_section(
            101,
            SectionKind::AiChunkProfile,
            PrimaryProfile::CoveChunk,
            encode_records(
                tables
                    .chunk_profiles
                    .iter()
                    .cloned()
                    .map(|record| encode_chunk_profile(record).unwrap()),
            )
            .unwrap(),
        ));
    }
    if !tables.text_chunks.is_empty() {
        payload_sections.push(coveai_binary_section(
            102,
            SectionKind::AiTextChunkIndex,
            PrimaryProfile::CoveChunk,
            encode_records(
                tables
                    .text_chunks
                    .iter()
                    .cloned()
                    .map(|record| encode_text_chunk_entry(record).unwrap()),
            )
            .unwrap(),
        ));
    }
    if !tables.vector_spaces.is_empty() {
        payload_sections.push(coveai_binary_section(
            103,
            SectionKind::AiVectorSpace,
            PrimaryProfile::CoveVec,
            encode_records(
                tables
                    .vector_spaces
                    .iter()
                    .cloned()
                    .map(|record| encode_vector_space(record).unwrap()),
            )
            .unwrap(),
        ));
    }
    if !tables.vector_payload_blocks.is_empty() {
        payload_sections.push(coveai_binary_section(
            104,
            SectionKind::AiVectorPayloadBlock,
            PrimaryProfile::CoveVec,
            encode_records(
                tables
                    .vector_payload_blocks
                    .iter()
                    .cloned()
                    .map(|record| encode_vector_payload_block(record).unwrap()),
            )
            .unwrap(),
        ));
    }
    if !tables.vector_entries.is_empty() {
        payload_sections.push(coveai_binary_section(
            105,
            SectionKind::AiVectorDirectory,
            PrimaryProfile::CoveVec,
            encode_records(
                tables
                    .vector_entries
                    .iter()
                    .cloned()
                    .map(|record| encode_vector_entry(record).unwrap()),
            )
            .unwrap(),
        ));
    }
    let mut vector_binding_section = coveai_binary_section(
        106,
        SectionKind::AiVectorBinding,
        PrimaryProfile::CoveVec,
        encode_records(vector_binding_records).unwrap(),
    );
    vector_binding_section.required_ai_features = vector_binding_required_ai_features;
    payload_sections.push(vector_binding_section);
    payload_sections
}

pub(super) fn vector_composition_tables() -> (AiDescriptorTablesV1, Vec<CoveAiWritableSection>) {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    tables
        .vector_arithmetic_profiles
        .push(VectorArithmeticProfileV1 {
            arithmetic_profile_id: 1,
            profile_name_ref: 0,
            arithmetic_kind: 0,
            input_quantization_kind: 0,
            accumulator_kind: 0,
            rounding_mode: 0,
            overflow_policy: 0,
            component_order: 0,
            weight_scale: 1_000_000,
            output_quantization_kind: 0,
            output_element_type: 0,
            normalization_policy: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .vector_composition_components
        .push(VectorCompositionComponentV1 {
            component_id: 1,
            slot_policy_ref: 0,
            source_vector_space_id: 1,
            weight_ppm: 1_000_000,
            required: 1,
            redaction_behavior: 0,
            missing_behavior: 0,
            reserved: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .vector_composition_profiles
        .push(VectorCompositionProfileV1 {
            composition_profile_id: 1,
            composition_name_ref: 0,
            output_vector_space_id: 1,
            arithmetic_profile_ref: 1,
            method: 1,
            missing_policy: 0,
            normalize_inputs: 1,
            normalize_output: 1,
            result_authority: 0,
            reproducibility_class: 0,
            first_component_ref: 1,
            component_count: 1,
            template_ref: 0,
            flags: 0,
            checksum: 0,
        });
    (tables, payload_sections)
}

pub(super) fn test_chunk_profile() -> ChunkProfileV1 {
    ChunkProfileV1 {
        chunk_profile_id: 1,
        profile_name_ref: 0,
        chunker_namespace_ref: 0,
        chunker_name_ref: 0,
        chunker_version_major: 1,
        chunker_version_minor: 0,
        tokenizer_profile_ref: 0,
        boundary_kind: 1,
        overlap_policy: 0,
        parent_policy: 0,
        normalization_policy: 0,
        target_tokens: 0,
        min_tokens: 0,
        max_tokens: 0,
        overlap_tokens: 0,
        max_bytes: 4096,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn chunk_sections(text_chunks: Vec<TextChunkEntryV1>) -> Vec<CoveAiWritableSection> {
    chunk_sections_with_profile(test_chunk_profile(), text_chunks)
}

pub(super) fn chunk_sections_with_profile(
    chunk_profile: ChunkProfileV1,
    text_chunks: Vec<TextChunkEntryV1>,
) -> Vec<CoveAiWritableSection> {
    vec![
        coveai_binary_section(
            1,
            SectionKind::AiChunkProfile,
            PrimaryProfile::CoveChunk,
            encode_records([encode_chunk_profile(chunk_profile).unwrap()]).unwrap(),
        ),
        coveai_payload_section(
            3,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveChunk,
            vec![1, 2, 3, 4, 5, 6, 7, 8],
        ),
        coveai_binary_section(
            4,
            SectionKind::AiReferenceTables,
            PrimaryProfile::CoveAiShared,
            encode_records([
                encode_digest_entry(AiDigestEntryV1 {
                    digest_ref: 1,
                    digest_algorithm: 1,
                    digest_len: 4,
                    digest_payload_ref: 1,
                    domain_hint: 1,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
                encode_digest_entry(AiDigestEntryV1 {
                    digest_ref: 2,
                    digest_algorithm: 1,
                    digest_len: 4,
                    digest_payload_ref: 2,
                    domain_hint: 1,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
                encode_payload_ref_entry(AiPayloadRefEntryV1 {
                    payload_ref: 1,
                    storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
                    media_type_ref: 0,
                    section_id: 3,
                    uri_ref: 0,
                    payload_offset: 0,
                    section_payload_offset: 0,
                    payload_length: 4,
                    decoded_length: 4,
                    integrity_ref: 0,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
                encode_payload_ref_entry(AiPayloadRefEntryV1 {
                    payload_ref: 2,
                    storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
                    media_type_ref: 0,
                    section_id: 3,
                    uri_ref: 0,
                    payload_offset: 0,
                    section_payload_offset: 4,
                    payload_length: 4,
                    decoded_length: 4,
                    integrity_ref: 0,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
            ])
            .unwrap(),
        ),
        coveai_binary_section(
            5,
            SectionKind::AiSourceBinding,
            PrimaryProfile::CoveAiShared,
            encode_records([encode_source_binding(AiSourceBindingV1 {
                source_binding_id: 1,
                source_kind: AI_SOURCE_KIND_COVE_FILE,
                source_artifact_ref: 0,
                source_file_digest_ref: 0,
                covm_snapshot_ref: 0,
                schema_fingerprint_ref: 0,
                dictionary_digest_ref: 0,
                map_fingerprint_ref: 0,
                policy_context_ref: 0,
                visibility_scope_ref: 0,
                redaction_scope_ref: 0,
                branch_ref: 0,
                as_of_csn: 0,
                flags: 0,
                crc32c: 0,
            })
            .unwrap()])
            .unwrap(),
        ),
        coveai_binary_section(
            2,
            SectionKind::AiTextChunkIndex,
            PrimaryProfile::CoveChunk,
            encode_records(
                text_chunks
                    .into_iter()
                    .map(|chunk| encode_text_chunk_entry(chunk).unwrap()),
            )
            .unwrap(),
        ),
    ]
}

pub(super) fn test_text_chunk(
    chunk_id: u64,
    previous_chunk_id: u64,
    next_chunk_id: u64,
    byte_start: u64,
    byte_length: u64,
    unicode_scalar_length: u64,
    source_value_hash_ref: u32,
) -> TextChunkEntryV1 {
    TextChunkEntryV1 {
        chunk_id,
        source_ref: 1,
        table_id: 0,
        column_id: 0,
        object_type_id: 0,
        property_id: 0,
        association_type_id: 0,
        path_ref: 0,
        source_row_ref: 0,
        source_object_ref: 0,
        source_value_hash_ref,
        byte_start,
        byte_length,
        unicode_scalar_start: byte_start,
        unicode_scalar_length,
        token_start: 0,
        token_count: 0,
        parent_chunk_id: 0,
        first_child_ref: 0,
        child_count: 0,
        previous_chunk_id,
        next_chunk_id,
        chunk_text_hash_ref: 0,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn vector_index_sections(index: VectorIndexDescriptorV1) -> Vec<CoveAiWritableSection> {
    vec![
        coveai_binary_section(
            50,
            SectionKind::AiVectorSpace,
            PrimaryProfile::CoveVec,
            encode_records([encode_vector_space(VectorSpaceDescriptorV1 {
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
                dimension_count: 3,
                element_type: 0,
                metric: 1,
                normalization_policy: 0,
                quantization_policy: 0,
                deterministic: 1,
                approximate: 0,
                reproducibility_class: 1,
                reserved: 0,
                flags: 0,
                checksum: 0,
            })
            .unwrap()])
            .unwrap(),
        ),
        coveai_binary_section(
            51,
            SectionKind::AiVectorIndex,
            PrimaryProfile::CoveVec,
            encode_records([encode_vector_index(index).unwrap()]).unwrap(),
        ),
    ]
}

pub(super) fn test_vector_index(index_kind: u8, exactness_kind: u8) -> VectorIndexDescriptorV1 {
    VectorIndexDescriptorV1 {
        vector_index_id: 1,
        vector_space_id: 1,
        stored_vector_space_id: 1,
        search_vector_space_id: 1,
        index_kind,
        exactness_kind,
        false_negative_policy: 0,
        metric: 1,
        score_space_authority: 0,
        dimension_count: 3,
        indexed_binding_kind: 1,
        temporal_scope_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        dequantization_profile_ref: 0,
        quantization_error_profile_ref: 0,
        payload_ref: 0,
        checksum: 0,
    }
}

pub(super) fn token_sections(
    tokenized_span: TokenizedSpanV1,
    sequence_pack: TokenSequencePackV1,
) -> Vec<CoveAiWritableSection> {
    token_sections_with_profile(test_tokenizer_profile(), tokenized_span, sequence_pack)
}

pub(super) fn token_sections_with_profile(
    tokenizer_profile: TokenizerProfileV1,
    tokenized_span: TokenizedSpanV1,
    sequence_pack: TokenSequencePackV1,
) -> Vec<CoveAiWritableSection> {
    vec![
        coveai_payload_section(
            10,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveTok,
            vec![1, 0, 2, 0, 3, 0, 4, 0],
        ),
        coveai_binary_section(
            11,
            SectionKind::AiReferenceTables,
            PrimaryProfile::CoveAiShared,
            encode_records([
                encode_digest_entry(AiDigestEntryV1 {
                    digest_ref: 1,
                    digest_algorithm: 1,
                    digest_len: 4,
                    digest_payload_ref: 2,
                    domain_hint: 1,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
                encode_payload_ref_entry(AiPayloadRefEntryV1 {
                    payload_ref: 1,
                    storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
                    media_type_ref: 0,
                    section_id: 10,
                    uri_ref: 0,
                    payload_offset: 0,
                    section_payload_offset: 0,
                    payload_length: 8,
                    decoded_length: 8,
                    integrity_ref: 0,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
                encode_payload_ref_entry(AiPayloadRefEntryV1 {
                    payload_ref: 2,
                    storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
                    media_type_ref: 0,
                    section_id: 10,
                    uri_ref: 0,
                    payload_offset: 0,
                    section_payload_offset: 0,
                    payload_length: 4,
                    decoded_length: 4,
                    integrity_ref: 0,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
            ])
            .unwrap(),
        ),
        coveai_binary_section(
            12,
            SectionKind::AiPrivacySummary,
            PrimaryProfile::CoveAiShared,
            encode_records([encode_privacy_summary(AiPrivacySummaryEntryV1 {
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
            })
            .unwrap()])
            .unwrap(),
        ),
        coveai_binary_section(
            13,
            SectionKind::AiTokenizerProfile,
            PrimaryProfile::CoveTok,
            encode_records([encode_tokenizer_profile(tokenizer_profile).unwrap()]).unwrap(),
        ),
        coveai_binary_section(
            14,
            SectionKind::AiTokenBlock,
            PrimaryProfile::CoveTok,
            encode_records([encode_token_block(TokenBlockHeaderV1 {
                token_block_id: 1,
                tokenizer_profile_id: 1,
                token_count: 4,
                token_id_width: 2,
                compression_codec: CompressionCodec::None as u8,
                layout_kind: 0,
                payload_ref: 1,
                payload_offset: 0,
                payload_length: 0,
                integrity_ref: 0,
                checksum: 0,
            })
            .unwrap()])
            .unwrap(),
        ),
        coveai_binary_section(
            15,
            SectionKind::AiTokenizedSpan,
            PrimaryProfile::CoveTok,
            encode_records([encode_tokenized_span(tokenized_span).unwrap()]).unwrap(),
        ),
        coveai_binary_section(
            16,
            SectionKind::AiTokenSequencePack,
            PrimaryProfile::CoveTok,
            encode_records([encode_token_sequence_pack(sequence_pack).unwrap()]).unwrap(),
        ),
    ]
}

pub(super) fn test_tokenizer_profile() -> TokenizerProfileV1 {
    TokenizerProfileV1 {
        tokenizer_profile_id: 1,
        tokenizer_namespace_ref: 0,
        tokenizer_name_ref: 0,
        tokenizer_version_major: 1,
        tokenizer_version_minor: 0,
        vocab_digest_ref: 1,
        merges_digest_ref: 0,
        pre_tokenizer_digest_ref: 0,
        normalizer_digest_ref: 0,
        byte_encoder_digest_ref: 0,
        special_tokens_digest_ref: 0,
        added_tokens_digest_ref: 0,
        chat_template_ref: 0,
        unicode_version_ref: 0,
        truncation_policy_ref: 0,
        padding_policy_ref: 0,
        model_max_sequence_length: 4096,
        token_id_width: 2,
        byte_alignment_available: 0,
        reversible: 1,
        deterministic: 1,
        bos_token_id: 0,
        eos_token_id: 0,
        pad_token_id: 0,
        unk_token_id: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_tokenized_span(
    tokenized_span_id: u64,
    chunk_id: u64,
    token_offset: u64,
    token_count: u32,
    source_value_hash_ref: u32,
    byte_alignment_ref: u32,
) -> TokenizedSpanV1 {
    TokenizedSpanV1 {
        tokenized_span_id,
        chunk_id,
        tokenizer_profile_id: 1,
        token_block_ref: 1,
        token_offset,
        token_count,
        byte_alignment_ref,
        source_value_hash_ref,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_token_sequence_pack(
    sequence_pack_id: u64,
    token_offset: u64,
    token_count: u32,
    loss_mask_ref: u32,
) -> TokenSequencePackV1 {
    TokenSequencePackV1 {
        sequence_pack_id,
        tokenizer_profile_id: 1,
        training_profile_ref: 0,
        token_block_ref: 1,
        token_offset,
        token_count,
        source_span_count: 0,
        first_source_span_ref: 0,
        loss_mask_ref,
        attention_mask_ref: 0,
        position_ids_ref: 0,
        labels_ref: 0,
        split_ref: 0,
        sample_weight_ppm: 1_000_000,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn training_sections(
    include_profile: bool,
    sample: TrainingSampleEntryV1,
) -> Vec<CoveAiWritableSection> {
    training_sections_with_records(
        include_profile.then(test_training_profile),
        test_dataset_split(),
        test_dedup_group(),
        test_training_epoch_plan(),
        sample,
    )
}

pub(super) fn training_sections_with_records(
    profile: Option<TrainingProfileV1>,
    split: DatasetSplitV1,
    dedup_group: DedupGroupV1,
    epoch_plan: TrainingEpochPlanV1,
    sample: TrainingSampleEntryV1,
) -> Vec<CoveAiWritableSection> {
    let mut sections = Vec::new();
    if let Some(profile) = profile {
        sections.push(coveai_binary_section(
            20,
            SectionKind::AiTrainingProfile,
            PrimaryProfile::CoveTrain,
            encode_records([encode_training_profile(profile).unwrap()]).unwrap(),
        ));
    }
    sections.push(coveai_binary_section(
        21,
        SectionKind::AiTrainingSplitDedupEpoch,
        PrimaryProfile::CoveTrain,
        encode_records([
            encode_dataset_split(split).unwrap(),
            encode_dedup_group(dedup_group).unwrap(),
            encode_training_epoch_plan(epoch_plan).unwrap(),
        ])
        .unwrap(),
    ));
    sections.push(coveai_binary_section(
        22,
        SectionKind::AiTrainingSampleIndex,
        PrimaryProfile::CoveTrain,
        encode_records([encode_training_sample_entry(sample).unwrap()]).unwrap(),
    ));
    sections
}

pub(super) fn test_training_profile() -> TrainingProfileV1 {
    TrainingProfileV1 {
        training_profile_id: 1,
        profile_name_ref: 0,
        task_family: 1,
        modality_mask: 1,
        source_snapshot_ref: 0,
        map_profile_ref: 0,
        chunk_profile_ref: 0,
        tokenizer_profile_ref: 0,
        vector_space_ref: 0,
        multimodal_sequence_profile_ref: 0,
        split_policy_ref: 0,
        sampling_policy_ref: 0,
        dedup_policy_ref: 0,
        quality_policy_ref: 0,
        license_policy_ref: 0,
        redaction_policy_ref: 0,
        default_generator_provenance_ref: 0,
        reproducibility_class: 1,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_training_sample(
    sample_id: u64,
    training_profile_id: u32,
    split_ref: u32,
    dedup_group_ref: u32,
) -> TrainingSampleEntryV1 {
    TrainingSampleEntryV1 {
        sample_id,
        training_profile_id,
        example_kind: 1,
        split_ref,
        source_ref: 0,
        evidence_ref: 0,
        input_ref: 0,
        target_ref: 0,
        label_ref: 0,
        metadata_ref: 0,
        token_sequence_pack_ref: 0,
        multimodal_sequence_pack_ref: 0,
        vector_ref: 0,
        quality_score_ppm: 900_000,
        sample_weight_ppm: 1_000_000,
        dedup_group_ref,
        license_ref: 0,
        policy_ref: 0,
        teacher_model_ref: 0,
        generator_provenance_ref: 0,
        judge_generator_provenance_ref: 0,
        label_generator_provenance_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_dataset_split() -> DatasetSplitV1 {
    DatasetSplitV1 {
        split_id: 1,
        split_name_ref: 0,
        split_method: 1,
        source_snapshot_ref: 0,
        filter_policy_ref: 0,
        seed: 42,
        hash_function_ref: 0,
        stratification_path_ref: 0,
        grouping_ref: 0,
        ordering_policy_ref: 0,
        dedup_policy_ref: 0,
        sample_count: 1,
        first_sample_ref: 1,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_dedup_group() -> DedupGroupV1 {
    DedupGroupV1 {
        dedup_group_id: 1,
        dedup_policy_ref: 0,
        canonical_member_sample_id: 1,
        similarity_kind: 0,
        dedup_authority: 0,
        confidence_ppm: 1_000_000,
        first_member_ref: 1,
        member_count: 1,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_training_epoch_plan() -> TrainingEpochPlanV1 {
    TrainingEpochPlanV1 {
        epoch_plan_id: 1,
        training_profile_id: 1,
        split_ref: 1,
        seed: 42,
        permutation_kind: 0,
        rng_algorithm_ref: 0,
        permutation_function_ref: 0,
        shard_count: 0,
        first_shard_ref: 0,
        shard_ref_count: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn provenance_sections(
    include_model_actor: bool,
    generator: GeneratorProvenanceV1,
) -> Vec<CoveAiWritableSection> {
    provenance_sections_with_records(
        include_model_actor.then(test_model_actor),
        test_decoding_profile(),
        test_human_review(),
        generator,
    )
}

pub(super) fn provenance_sections_with_records(
    model_actor: Option<ModelActorDescriptorV1>,
    decoding: GenerationDecodingProfileV1,
    human_review: HumanReviewEntryV1,
    generator: GeneratorProvenanceV1,
) -> Vec<CoveAiWritableSection> {
    provenance_sections_with_label_pair(
        model_actor,
        decoding,
        human_review,
        generator,
        test_training_label(),
        test_preference_pair(),
    )
}

pub(super) fn provenance_sections_with_label_pair(
    model_actor: Option<ModelActorDescriptorV1>,
    decoding: GenerationDecodingProfileV1,
    human_review: HumanReviewEntryV1,
    generator: GeneratorProvenanceV1,
    label: TrainingLabelEntryV1,
    pair: PreferencePairEntryV1,
) -> Vec<CoveAiWritableSection> {
    let mut generator_records = Vec::new();
    if let Some(model_actor) = model_actor {
        generator_records.push(encode_model_actor(model_actor).unwrap());
    }
    generator_records.push(encode_generation_decoding_profile(decoding).unwrap());
    generator_records.push(encode_human_review(human_review).unwrap());
    generator_records.push(encode_generator_provenance(generator).unwrap());
    vec![
        coveai_binary_section(
            30,
            SectionKind::AiGeneratorProvenance,
            PrimaryProfile::CoveTrain,
            encode_records(generator_records).unwrap(),
        ),
        coveai_binary_section(
            31,
            SectionKind::AiLabelPreference,
            PrimaryProfile::CoveTrain,
            encode_records([
                encode_training_label_entry(label).unwrap(),
                encode_preference_pair_entry(pair).unwrap(),
            ])
            .unwrap(),
        ),
    ]
}

pub(super) fn test_model_actor() -> ModelActorDescriptorV1 {
    ModelActorDescriptorV1 {
        model_actor_id: 1,
        model_namespace_ref: 0,
        model_name_ref: 0,
        model_version_ref: 0,
        model_checkpoint_digest_ref: 0,
        provider_ref: 0,
        endpoint_ref: 0,
        endpoint_version_ref: 0,
        model_family_ref: 0,
        modality_mask: 1,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_decoding_profile() -> GenerationDecodingProfileV1 {
    GenerationDecodingProfileV1 {
        decoding_profile_id: 1,
        temperature_micros: 0,
        top_p_micros: 1_000_000,
        top_k: 0,
        seed: 42,
        max_output_tokens: 128,
        stop_sequence_ref: 0,
        safety_policy_ref: 0,
        deterministic_claim: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_human_review() -> HumanReviewEntryV1 {
    HumanReviewEntryV1 {
        human_review_id: 1,
        review_kind: 1,
        reviewer_role_ref: 0,
        review_time_us: 123,
        rating_ppm: 1_000_000,
        notes_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_generator_provenance(
    generator_provenance_id: u64,
    model_actor_ref: u32,
) -> GeneratorProvenanceV1 {
    GeneratorProvenanceV1 {
        generator_provenance_id,
        generator_kind: 1,
        model_actor_ref,
        prompt_template_ref: 0,
        decoding_profile_ref: 1,
        toolchain_ref: 0,
        source_input_ref: 0,
        source_context_ref: 0,
        source_sample_ref: 0,
        parent_generator_provenance_ref: 0,
        generation_time_us: 123,
        confidence_ppm: 900_000,
        human_review_ref: 1,
        policy_ref: 0,
        reproducibility_class: 1,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_training_label() -> TrainingLabelEntryV1 {
    TrainingLabelEntryV1 {
        label_id: 1,
        label_kind: 1,
        label_authority: 3,
        label_payload_ref: 0,
        generator_provenance_ref: 1,
        human_review_ref: 1,
        confidence_ppm: 900_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_preference_pair() -> PreferencePairEntryV1 {
    PreferencePairEntryV1 {
        preference_pair_id: 1,
        prompt_ref: 0,
        chosen_ref: 0,
        rejected_ref: 0,
        judge_generator_provenance_ref: 1,
        human_review_ref: 1,
        preference_strength_ppm: 700_000,
        confidence_ppm: 900_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn phase7_sections(
    include_asset: bool,
    pack: MultimodalSequencePackV1,
    elements: Vec<MultimodalSequenceElementV1>,
) -> Vec<CoveAiWritableSection> {
    phase7_sections_with_tensor(
        test_tensor_layout(),
        test_device_transfer_hint(),
        include_asset,
        pack,
        elements,
    )
}

pub(super) fn phase7_sections_with_tensor(
    tensor_layout: TensorLayoutDescriptorV1,
    transfer_hint: DeviceTransferHintV1,
    include_asset: bool,
    pack: MultimodalSequencePackV1,
    elements: Vec<MultimodalSequenceElementV1>,
) -> Vec<CoveAiWritableSection> {
    let mut sections = vec![coveai_binary_section(
        40,
        SectionKind::AiTensorLayout,
        PrimaryProfile::CoveMmseq,
        encode_records([
            encode_tensor_layout(tensor_layout).unwrap(),
            encode_device_transfer_hint(transfer_hint).unwrap(),
        ])
        .unwrap(),
    )];
    if include_asset {
        sections.push(coveai_binary_section(
            41,
            SectionKind::AiAssetManifest,
            PrimaryProfile::CoveMmseq,
            encode_records([encode_ai_asset_ref(test_asset_ref()).unwrap()]).unwrap(),
        ));
    }
    sections.push(coveai_binary_section(
        42,
        SectionKind::AiMultimodalSequence,
        PrimaryProfile::CoveMmseq,
        encode_records(
            std::iter::once(encode_multimodal_sequence_pack(pack).unwrap()).chain(
                elements
                    .into_iter()
                    .map(|element| encode_multimodal_sequence_element(element).unwrap()),
            ),
        )
        .unwrap(),
    ));
    sections
}

pub(super) fn phase7_sections_with_asset(
    asset: AiAssetRefV1,
    pack: MultimodalSequencePackV1,
    elements: Vec<MultimodalSequenceElementV1>,
) -> Vec<CoveAiWritableSection> {
    let mut sections = phase7_sections_with_tensor(
        test_tensor_layout(),
        test_device_transfer_hint(),
        false,
        pack,
        elements,
    );
    sections.insert(
        1,
        coveai_binary_section(
            41,
            SectionKind::AiAssetManifest,
            PrimaryProfile::CoveMmseq,
            encode_records([encode_ai_asset_ref(asset).unwrap()]).unwrap(),
        ),
    );
    sections
}

pub(super) fn test_tensor_layout() -> TensorLayoutDescriptorV1 {
    TensorLayoutDescriptorV1 {
        tensor_layout_id: 1,
        layout_name_ref: 0,
        rank: 2,
        dtype: 0,
        byte_order: 1,
        shape_ref: 0,
        stride_ref: 0,
        storage_offset_elements: 0,
        layout_kind: 0,
        memory_alignment_bytes: 64,
        preferred_page_alignment_bytes: 4096,
        tile_shape_ref: 0,
        block_shape_ref: 0,
        quantization_profile_ref: 0,
        sparsity_profile_ref: 0,
        framework_compatibility_ref: 0,
        device_affinity_hint: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_device_transfer_hint() -> DeviceTransferHintV1 {
    DeviceTransferHintV1 {
        transfer_hint_id: 1,
        target_kind: 0,
        preferred_alignment_bytes: 64,
        preferred_chunk_bytes: 4096,
        pinned_memory_required: 0,
        contiguous_required: 1,
        zero_copy_possible: 1,
        runtime_registry_binding_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_asset_ref() -> AiAssetRefV1 {
    AiAssetRefV1 {
        asset_ref_id: 1,
        parent_asset_ref: 0,
        asset_kind: 2,
        uri_ref: 0,
        embedded_section_ref: 0,
        media_type_ref: 0,
        byte_length: 0,
        digest_ref: 0,
        width: 64,
        height: 64,
        duration_us: 0,
        sample_rate_hz: 0,
        channel_count: 0,
        decode_profile_ref: 0,
        preprocessing_profile_ref: 0,
        transform_profile_ref: 0,
        transform_digest_ref: 0,
        tensor_layout_ref: 1,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_multimodal_pack(
    sequence_pack_id: u64,
    element_count: u32,
) -> MultimodalSequencePackV1 {
    MultimodalSequencePackV1 {
        sequence_pack_id,
        training_profile_id: 0,
        tokenizer_profile_id: 0,
        sequence_profile_ref: 0,
        element_count,
        first_element_ref: if element_count == 0 { 0 } else { 1 },
        split_ref: 0,
        sample_weight_ppm: 1_000_000,
        loss_mask_ref: 0,
        attention_mask_ref: 0,
        position_map_ref: 0,
        label_ref: 0,
        source_snapshot_ref: 0,
        evidence_ref: 0,
        generator_provenance_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn test_multimodal_element(
    element_id: u64,
    sequence_pack_id: u64,
    ordinal: u32,
    asset_ref: u64,
    tensor_ref: u64,
) -> MultimodalSequenceElementV1 {
    MultimodalSequenceElementV1 {
        element_id,
        sequence_pack_id,
        ordinal,
        element_kind: 2,
        modality: 2,
        role: 1,
        tokenized_span_ref: 0,
        token_sequence_pack_ref: 0,
        asset_ref,
        tensor_ref,
        vector_ref: 0,
        byte_start: 0,
        byte_length: 0,
        time_start_us: 0,
        time_duration_us: 0,
        position_stream_ref: 0,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    }
}
