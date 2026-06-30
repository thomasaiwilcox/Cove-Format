use super::*;

pub(super) fn coveai_full_descriptor_fixture() -> Vec<u8> {
    let mut tables = AiDescriptorTablesV1::default();
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 2,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 90,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 8,
        decoded_length: 8,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 90,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
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
    });
    tables.chunk_profiles.push(ChunkProfileV1 {
        chunk_profile_id: 1,
        profile_name_ref: 0,
        chunker_namespace_ref: 0,
        chunker_name_ref: 0,
        chunker_version_major: 1,
        chunker_version_minor: 0,
        tokenizer_profile_ref: 1,
        boundary_kind: 1,
        overlap_policy: 0,
        parent_policy: 0,
        normalization_policy: 0,
        target_tokens: 128,
        min_tokens: 1,
        max_tokens: 256,
        overlap_tokens: 0,
        max_bytes: 4096,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.text_chunks.push(TextChunkEntryV1 {
        chunk_id: 1,
        source_ref: 0,
        table_id: 1,
        column_id: 2,
        object_type_id: 3,
        property_id: 4,
        association_type_id: 0,
        path_ref: 0,
        source_row_ref: 7,
        source_object_ref: 8,
        source_value_hash_ref: 1,
        byte_start: 4,
        byte_length: 12,
        unicode_scalar_start: 4,
        unicode_scalar_length: 12,
        token_start: 0,
        token_count: 4,
        parent_chunk_id: 0,
        first_child_ref: 0,
        child_count: 0,
        previous_chunk_id: 0,
        next_chunk_id: 0,
        chunk_text_hash_ref: 2,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.tokenizer_profiles.push(TokenizerProfileV1 {
        tokenizer_profile_id: 1,
        tokenizer_namespace_ref: 0,
        tokenizer_name_ref: 0,
        tokenizer_version_major: 1,
        tokenizer_version_minor: 0,
        vocab_digest_ref: 0,
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
    });
    tables.token_blocks.push(TokenBlockHeaderV1 {
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
    });
    tables.tokenized_spans.push(TokenizedSpanV1 {
        tokenized_span_id: 1,
        chunk_id: 1,
        tokenizer_profile_id: 1,
        token_block_ref: 1,
        token_offset: 0,
        token_count: 4,
        byte_alignment_ref: 0,
        source_value_hash_ref: 1,
        flags: 0,
        checksum: 0,
    });
    tables.token_sequence_packs.push(TokenSequencePackV1 {
        sequence_pack_id: 1,
        tokenizer_profile_id: 1,
        training_profile_ref: 1,
        token_block_ref: 1,
        token_offset: 0,
        token_count: 4,
        source_span_count: 0,
        first_source_span_ref: 0,
        loss_mask_ref: 0,
        attention_mask_ref: 0,
        position_ids_ref: 0,
        labels_ref: 0,
        split_ref: 1,
        sample_weight_ppm: 1_000_000,
        flags: 0,
        checksum: 0,
    });
    tables.training_profiles.push(TrainingProfileV1 {
        training_profile_id: 1,
        profile_name_ref: 0,
        task_family: 1,
        modality_mask: 3,
        source_snapshot_ref: 0,
        map_profile_ref: 0,
        chunk_profile_ref: 1,
        tokenizer_profile_ref: 1,
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
    });
    tables.dataset_splits.push(DatasetSplitV1 {
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
    });
    tables.dedup_groups.push(DedupGroupV1 {
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
    });
    tables.training_epoch_plans.push(TrainingEpochPlanV1 {
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
    });
    tables.training_samples.push(TrainingSampleEntryV1 {
        sample_id: 1,
        training_profile_id: 1,
        example_kind: 1,
        split_ref: 1,
        source_ref: 0,
        evidence_ref: 0,
        input_ref: 0,
        target_ref: 0,
        label_ref: 0,
        metadata_ref: 0,
        token_sequence_pack_ref: 1,
        multimodal_sequence_pack_ref: 1,
        vector_ref: 0,
        quality_score_ppm: 900_000,
        sample_weight_ppm: 1_000_000,
        dedup_group_ref: 1,
        license_ref: 0,
        policy_ref: 0,
        teacher_model_ref: 0,
        generator_provenance_ref: 0,
        judge_generator_provenance_ref: 0,
        label_generator_provenance_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.model_actors.push(ModelActorDescriptorV1 {
        model_actor_id: 1,
        model_namespace_ref: 0,
        model_name_ref: 0,
        model_version_ref: 0,
        model_checkpoint_digest_ref: 0,
        provider_ref: 0,
        endpoint_ref: 0,
        endpoint_version_ref: 0,
        model_family_ref: 0,
        modality_mask: 3,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .generation_decoding_profiles
        .push(GenerationDecodingProfileV1 {
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
        });
    tables.human_reviews.push(HumanReviewEntryV1 {
        human_review_id: 1,
        review_kind: 1,
        reviewer_role_ref: 0,
        review_time_us: 1_234,
        rating_ppm: 900_000,
        notes_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.generator_provenance.push(GeneratorProvenanceV1 {
        generator_provenance_id: 1,
        generator_kind: 1,
        model_actor_ref: 1,
        prompt_template_ref: 0,
        decoding_profile_ref: 1,
        toolchain_ref: 0,
        source_input_ref: 0,
        source_context_ref: 0,
        source_sample_ref: 1,
        parent_generator_provenance_ref: 0,
        generation_time_us: 1_235,
        confidence_ppm: 800_000,
        human_review_ref: 1,
        policy_ref: 0,
        reproducibility_class: 2,
        flags: 0,
        checksum: 0,
    });
    tables.training_labels.push(TrainingLabelEntryV1 {
        label_id: 1,
        label_kind: 1,
        label_authority: 3,
        label_payload_ref: 0,
        generator_provenance_ref: 1,
        human_review_ref: 1,
        confidence_ppm: 850_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.preference_pairs.push(PreferencePairEntryV1 {
        preference_pair_id: 1,
        prompt_ref: 0,
        chosen_ref: 0,
        rejected_ref: 0,
        judge_generator_provenance_ref: 1,
        human_review_ref: 1,
        preference_strength_ppm: 750_000,
        confidence_ppm: 850_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.tensor_layouts.push(TensorLayoutDescriptorV1 {
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
    });
    tables.device_transfer_hints.push(DeviceTransferHintV1 {
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
    });
    tables.assets.push(AiAssetRefV1 {
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
    });
    tables
        .multimodal_sequence_packs
        .push(MultimodalSequencePackV1 {
            sequence_pack_id: 1,
            training_profile_id: 1,
            tokenizer_profile_id: 1,
            sequence_profile_ref: 0,
            element_count: 1,
            first_element_ref: 1,
            split_ref: 1,
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
        });
    tables
        .multimodal_sequence_elements
        .push(MultimodalSequenceElementV1 {
            element_id: 1,
            sequence_pack_id: 1,
            ordinal: 0,
            element_kind: 2,
            modality: 2,
            role: 1,
            tokenized_span_ref: 1,
            token_sequence_pack_ref: 1,
            asset_ref: 1,
            tensor_ref: 1,
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
        });

    write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [90u8; 16],
        created_at_us: 90,
        payload_sections: vec![CoveAiWritableSection {
            section_id: 90,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveTok as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![1, 0, 2, 0, 3, 0, 4, 0],
        }],
        descriptor_tables: tables,
    })
    .unwrap()
}

pub(super) fn coveai_model_input_identity_fixture() -> Vec<u8> {
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 91,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 12,
        decoded_length: 12,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 91,
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
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 2,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 4,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
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
    });
    tables.chunk_profiles.push(ChunkProfileV1 {
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
    });
    tables.text_chunks.push(TextChunkEntryV1 {
        chunk_id: 1,
        source_ref: 0,
        table_id: 1,
        column_id: 1,
        object_type_id: 0,
        property_id: 0,
        association_type_id: 0,
        path_ref: 0,
        source_row_ref: 1,
        source_object_ref: 0,
        source_value_hash_ref: 1,
        byte_start: 0,
        byte_length: 4,
        unicode_scalar_start: 0,
        unicode_scalar_length: 4,
        token_start: 0,
        token_count: 0,
        parent_chunk_id: 0,
        first_child_ref: 0,
        child_count: 0,
        previous_chunk_id: 0,
        next_chunk_id: 0,
        chunk_text_hash_ref: 0,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.vector_spaces.push(VectorSpaceDescriptorV1 {
        vector_space_id: 1,
        vector_space_name_ref: 0,
        vector_space_fingerprint_ref: 0,
        embedding_namespace_ref: 0,
        embedding_model_ref: 0,
        embedding_model_version_ref: 0,
        embedding_model_digest_ref: 0,
        embedding_pipeline_ref: 0,
        tokenizer_profile_ref: 0,
        chunk_profile_ref: 1,
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
    });
    tables
        .vector_payload_blocks
        .push(VectorPayloadBlockHeaderV1 {
            block_id: 1,
            vector_space_id: 1,
            vector_count: 1,
            dimension_count: 3,
            element_type: 0,
            compression_codec: CompressionCodec::None as u8,
            quantization_kind: 0,
            layout_kind: 0,
            tensor_layout_ref: 0,
            memory_alignment_bytes: 4,
            payload_stride_ref: 0,
            device_transfer_hint_ref: 0,
            payload_ref: 1,
            payload_offset: 0,
            payload_length: 0,
            integrity_ref: 0,
            checksum: 0,
        });
    tables.vector_entries.push(VectorEntryV1 {
        vector_ref: 1,
        block_id: 1,
        vector_ordinal: 0,
        payload_offset: 0,
        payload_length: 12,
        integrity_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 1,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 2,
        flags: 0,
        checksum: 0,
    });

    write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [91u8; 16],
        created_at_us: 91,
        payload_sections: vec![CoveAiWritableSection {
            section_id: 91,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveVec as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![0, 0, 0x80, 0x3f, 0, 0, 0, 0, 0, 0, 0, 0],
        }],
        descriptor_tables: tables,
    })
    .unwrap()
}

pub(super) fn coveai_vector_binding_families_fixture() -> Vec<u8> {
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 92,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 36,
        decoded_length: 36,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
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
    });
    tables.vector_spaces.push(VectorSpaceDescriptorV1 {
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
    });
    tables
        .vector_payload_blocks
        .push(VectorPayloadBlockHeaderV1 {
            block_id: 1,
            vector_space_id: 1,
            vector_count: 3,
            dimension_count: 3,
            element_type: 0,
            compression_codec: CompressionCodec::None as u8,
            quantization_kind: 0,
            layout_kind: 0,
            tensor_layout_ref: 0,
            memory_alignment_bytes: 4,
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
            payload_length: 12,
            integrity_ref: 0,
            flags: 0,
            checksum: 0,
        });
    }
    tables
        .association_state_vector_bindings
        .push(AssociationStateVectorBindingV1 {
            binding_id: 1,
            vector_space_id: 1,
            composition_profile_ref: 0,
            file_ref: 0,
            association_type_id: 7,
            association_key_ref: 0,
            branch_ref: 0,
            temporal_kind: 0,
            csn: 10,
            timestamp_us: 20,
            property_dependency_fingerprint_ref: 0,
            vector_ref: 1,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables.assets.push(AiAssetRefV1 {
        asset_ref_id: 1,
        parent_asset_ref: 0,
        asset_kind: 2,
        uri_ref: 0,
        embedded_section_ref: 0,
        media_type_ref: 0,
        byte_length: 0,
        digest_ref: 0,
        width: 32,
        height: 32,
        duration_us: 0,
        sample_rate_hz: 0,
        channel_count: 0,
        decode_profile_ref: 0,
        preprocessing_profile_ref: 0,
        transform_profile_ref: 0,
        transform_digest_ref: 0,
        tensor_layout_ref: 0,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.asset_vector_bindings.push(AssetVectorBindingV1 {
        binding_id: 2,
        vector_space_id: 1,
        asset_ref: 1,
        transform_ref: 0,
        asset_digest_ref: 0,
        vector_ref: 2,
        model_input_digest_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .multimodal_sequence_packs
        .push(MultimodalSequencePackV1 {
            sequence_pack_id: 1,
            training_profile_id: 0,
            tokenizer_profile_id: 0,
            sequence_profile_ref: 0,
            element_count: 0,
            first_element_ref: 0,
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
        });
    tables
        .multimodal_sequence_vector_bindings
        .push(MultimodalSequenceVectorBindingV1 {
            binding_id: 3,
            vector_space_id: 1,
            sequence_pack_id: 1,
            sequence_profile_ref: 0,
            source_snapshot_ref: 0,
            vector_ref: 3,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });

    write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [92u8; 16],
        created_at_us: 92,
        payload_sections: vec![CoveAiWritableSection {
            section_id: 92,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveVec as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![
                0, 0, 0x80, 0x3f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3f, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3f,
            ],
        }],
        descriptor_tables: tables,
    })
    .unwrap()
}

pub(super) fn covev_require_model_input_identity_reject_fixture(mut bytes: Vec<u8>) -> Vec<u8> {
    let postscript_start =
        bytes.len() - COVEAI_POSTSCRIPT_TAIL_SIZE - usize::from(COVEAI_POSTSCRIPT_LEN);
    let header_offset = u64::from_le_bytes(
        bytes[postscript_start + 24..postscript_start + 32]
            .try_into()
            .unwrap(),
    ) as usize;
    let section_count = u32::from_le_bytes(
        bytes[header_offset + 46..header_offset + 50]
            .try_into()
            .unwrap(),
    ) as usize;
    let section_entry_len = u16::from_le_bytes(
        bytes[header_offset + 50..header_offset + 52]
            .try_into()
            .unwrap(),
    );
    assert_eq!(section_entry_len, COVEAI_SECTION_ENTRY_LEN);

    let directory_start = header_offset + usize::from(COVEAI_HEADER_LEN);
    let mut found_vector_binding = false;
    for index in 0..section_count {
        let entry_offset = directory_start + index * usize::from(COVEAI_SECTION_ENTRY_LEN);
        let section_kind = u32::from_le_bytes(
            bytes[entry_offset + 4..entry_offset + 8]
                .try_into()
                .unwrap(),
        );
        if section_kind == SectionKind::AiVectorBinding as u32 {
            let feature_offset = entry_offset + 40;
            let mut required_ai_features = u64::from_le_bytes(
                bytes[feature_offset..feature_offset + 8]
                    .try_into()
                    .unwrap(),
            );
            required_ai_features |= AI_FEATURE_MODEL_INPUT_IDENTITY;
            bytes[feature_offset..feature_offset + 8]
                .copy_from_slice(&required_ai_features.to_le_bytes());
            found_vector_binding = true;
        }
    }
    assert!(found_vector_binding);

    let directory_len = section_count * usize::from(COVEAI_SECTION_ENTRY_LEN);
    let directory_crc = checksum::crc32c(&bytes[directory_start..directory_start + directory_len]);
    bytes[header_offset + 62..header_offset + 66].copy_from_slice(&directory_crc.to_le_bytes());
    bytes[header_offset + 66..header_offset + 70].fill(0);
    let header_crc =
        checksum::crc32c(&bytes[header_offset..header_offset + usize::from(COVEAI_HEADER_LEN)]);
    bytes[header_offset + 66..header_offset + 70].copy_from_slice(&header_crc.to_le_bytes());
    bytes
}

pub(super) fn write_ai_fixtures(writer: &mut CorpusWriter<'_>) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    let coveai_empty =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [83u8; 16], 83, &[]).unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "ai/coveai_empty_valid.coveai",
            "coveai",
            "accept",
            None,
            &["§83"],
        ),
        coveai_empty,
    );

    let mut ai_descriptor_tables = AiDescriptorTablesV1::default();
    ai_descriptor_tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 1,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    ai_descriptor_tables.digests.push(AiDigestEntryV1 {
        digest_ref: 2,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    ai_descriptor_tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 86,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    ai_descriptor_tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 86,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 4,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    ai_descriptor_tables.chunk_profiles.push(ChunkProfileV1 {
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
        target_tokens: 128,
        min_tokens: 1,
        max_tokens: 256,
        overlap_tokens: 0,
        max_bytes: 4096,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    });
    ai_descriptor_tables.text_chunks.push(TextChunkEntryV1 {
        chunk_id: 1,
        source_ref: 0,
        table_id: 1,
        column_id: 2,
        object_type_id: 0,
        property_id: 0,
        association_type_id: 0,
        path_ref: 0,
        source_row_ref: 7,
        source_object_ref: 0,
        source_value_hash_ref: 1,
        byte_start: 0,
        byte_length: 12,
        unicode_scalar_start: 0,
        unicode_scalar_length: 12,
        token_start: 0,
        token_count: 4,
        parent_chunk_id: 0,
        first_child_ref: 0,
        child_count: 0,
        previous_chunk_id: 0,
        next_chunk_id: 0,
        chunk_text_hash_ref: 2,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    let coveai_descriptor_bundle = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [86u8; 16],
        created_at_us: 86,
        payload_sections: vec![CoveAiWritableSection {
            section_id: 86,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveChunk as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![1, 2, 3, 4, 5, 6, 7, 8],
        }],
        descriptor_tables: ai_descriptor_tables,
    })
    .unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "ai/coveai_chunk_descriptors_valid.coveai",
            "coveai",
            "accept",
            None,
            &["§83"],
        ),
        coveai_descriptor_bundle,
    );

    write_fixture(
        root,
        entries,
        fixture(
            "ai/coveai_full_descriptors_valid.coveai",
            "coveai",
            "accept",
            None,
            &["§83"],
        ),
        coveai_full_descriptor_fixture(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "ai/coveai_vec_model_input_identity_valid.coveai",
            "coveai",
            "accept",
            None,
            &["§83"],
        ),
        coveai_model_input_identity_fixture(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "ai/coveai_vec_binding_families_valid.coveai",
            "coveai",
            "accept",
            None,
            &["§83"],
        ),
        coveai_vector_binding_families_fixture(),
    );

    let mut ai_vector_payload = Vec::new();
    for value in [1.0f32, 0.0, 0.0, 0.5, 0.5, 0.0] {
        ai_vector_payload.extend_from_slice(&value.to_le_bytes());
    }
    let covev_vectors = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [84u8; 16],
        created_at_us: 84,
        dimension_count: 3,
        file_codes: vec![10, 20],
        vector_payload: ai_vector_payload,
    })
    .unwrap();
    let covev_vectors_digest = compute_digest(DigestAlgorithm::Sha256, &covev_vectors).unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "ai/covev_filecode_vectors_valid.covev",
            "covev",
            "accept",
            None,
            &["§83"],
        ),
        covev_vectors.clone(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "ai/covev_model_input_identity_requires_v2_reject.covev",
            "covev",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§83", "§76"],
        ),
        covev_require_model_input_identity_reject_fixture(covev_vectors.clone()),
    );

    let companion_uri = b"covev_filecode_vectors_valid.covev";
    let companion_policy_authority = b"cove-ai-conformance-policy";
    let mut companion_payload = Vec::new();
    companion_payload.extend_from_slice(companion_uri);
    companion_payload.extend_from_slice(&covev_vectors_digest);
    companion_payload.extend_from_slice(companion_policy_authority);
    let mut companion_tables = AiDescriptorTablesV1::default();
    companion_tables.strings.push(AiStringEntryV1 {
        string_ref: 1,
        utf8_byte_length: companion_uri.len() as u32,
        payload_ref: 1,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.strings.push(AiStringEntryV1 {
        string_ref: 2,
        utf8_byte_length: companion_policy_authority.len() as u32,
        payload_ref: 3,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: u16::try_from(covev_vectors_digest.len()).unwrap(),
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: companion_uri.len() as u64,
        decoded_length: companion_uri.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: companion_uri.len() as u64,
        payload_length: covev_vectors_digest.len() as u64,
        decoded_length: covev_vectors_digest.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 3,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: (companion_uri.len() + covev_vectors_digest.len()) as u64,
        payload_length: companion_policy_authority.len() as u64,
        decoded_length: companion_policy_authority.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.policies.push(AiPolicyRefEntryV1 {
        policy_ref: 1,
        policy_kind: AI_POLICY_KIND_VISIBILITY,
        authority_ref: 2,
        payload_ref: 0,
        digest_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.source_bindings.push(AiSourceBindingV1 {
        source_binding_id: 1,
        source_kind: AI_SOURCE_KIND_COVE_FILE,
        source_artifact_ref: 1,
        source_file_digest_ref: 1,
        covm_snapshot_ref: 0,
        schema_fingerprint_ref: 0,
        dictionary_digest_ref: 0,
        map_fingerprint_ref: 0,
        policy_context_ref: 1,
        visibility_scope_ref: 1,
        redaction_scope_ref: 0,
        branch_ref: 0,
        as_of_csn: 0,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.source_spans.push(AiSourceSpanEntryV1 {
        source_span_ref: 1,
        source_binding_ref: 1,
        source_kind: AI_SOURCE_KIND_COVE_FILE,
        source_row_ref: 0,
        source_object_ref: 0,
        byte_start: 0,
        byte_length: companion_uri.len() as u64,
        token_start: 0,
        token_count: 0,
        evidence_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    companion_tables.transforms.push(AiTransformEntryV1 {
        transform_ref: 1,
        transform_kind: AI_TRANSFORM_KIND_VECTORIZER,
        function_or_template_ref: 0,
        input_digest_ref: 0,
        output_digest_ref: 0,
        parameter_payload_ref: 0,
        transform_digest_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    companion_tables
        .section_feature_bindings
        .push(AiSectionFeatureBindingV1 {
            binding_ref: 1,
            section_id: 1,
            scope: FeatureScopeV2::OperationRequired as u8,
            profile_kind: PrimaryProfile::CoveAiShared as u8,
            operation_kind: OperationKindV2::AiEmbedding as u16,
            required_ai_features: AI_FEATURE_VECTOR,
            optional_ai_features: 0,
            target_local_ref: 0,
            flags: 0,
            crc32c: 0,
        });
    companion_tables
        .companion_artifacts
        .push(AiCompanionArtifactRefV1 {
            artifact_ref: 1,
            artifact_kind: AI_COMPANION_ARTIFACT_KIND_CVV2,
            artifact_id: [84u8; 16],
            uri_ref: 1,
            artifact_digest_ref: 1,
            source_binding_ref: 1,
            required_ai_features: 0,
            optional_ai_features: 0,
            flags: 0,
            crc32c: 0,
        });
    let coveai_companion_ref = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [87u8; 16],
        created_at_us: 87,
        payload_sections: vec![CoveAiWritableSection {
            section_id: 1,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveAiShared as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: companion_payload,
        }],
        descriptor_tables: companion_tables,
    })
    .unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "ai/coveai_companion_ref_valid.coveai",
            "coveai",
            "accept",
            None,
            &["§83"],
        ),
        coveai_companion_ref,
    );

    let duplicate_ai_sections = [
        CoveAiWritableSection {
            section_id: 1,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveAiShared as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![1],
        },
        CoveAiWritableSection {
            section_id: 1,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveAiShared as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![2],
        },
    ];
    let coveai_duplicate_sections = write_coveai_artifact(
        CoveAiArtifactKind::CoveAiBundle,
        [85u8; 16],
        85,
        &duplicate_ai_sections,
    )
    .unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "ai/coveai_duplicate_section_reject.coveai",
            "coveai",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§83", "§76"],
        ),
        coveai_duplicate_sections,
    );
}
