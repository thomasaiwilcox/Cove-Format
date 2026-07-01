use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRecordHeaderV1 {
    pub record_kind: u16,
    pub record_version: u16,
    pub record_len: u32,
    pub local_id: u64,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiRecordHeaderV1 {
    pub fn parse(record_bytes: &[u8]) -> Result<Self, CoveError> {
        if record_bytes.len() < AI_RECORD_HEADER_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let header = Self {
            record_kind: read_u16(record_bytes, 0)?,
            record_version: read_u16(record_bytes, 2)?,
            record_len: read_u32(record_bytes, 4)?,
            local_id: read_u64(record_bytes, 8)?,
            flags: read_u32(record_bytes, 16)?,
            crc32c: read_u32(record_bytes, 20)?,
        };
        if header.record_len as usize != record_bytes.len() {
            return Err(CoveError::BadSection(
                "COVE-AI record_len does not match record bytes".into(),
            ));
        }
        verify_crc32c(record_bytes, 20, header.crc32c)?;
        Ok(header)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiStringEntryV1 {
    pub string_ref: u32,
    pub utf8_byte_length: u32,
    pub payload_ref: u32,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiStringEntryV1 {
    pub(super) fn validate(&self, tables: &AiDescriptorTablesV1) -> Result<(), CoveError> {
        if self.string_ref == 0 {
            return Err(CoveError::BadSection(
                "AiStringEntryV1 string_ref must be non-zero".into(),
            ));
        }
        if self.payload_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "AiStringEntry {} requires payload_ref",
                self.string_ref
            )));
        }
        let payload_ref = tables.payload_ref(self.payload_ref).ok_or_else(|| {
            CoveError::BadSection(format!(
                "AiStringEntry {} references missing payload_ref {}",
                self.string_ref, self.payload_ref
            ))
        })?;
        if payload_ref.decoded_length != u64::from(self.utf8_byte_length) {
            return Err(CoveError::BadSection(format!(
                "AiStringEntry {} utf8_byte_length does not match payload_ref decoded_length",
                self.string_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiDigestEntryV1 {
    pub digest_ref: u32,
    pub digest_algorithm: u16,
    pub digest_len: u16,
    pub digest_payload_ref: u32,
    pub domain_hint: u8,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiDigestEntryV1 {
    pub(super) fn validate(&self, tables: &AiDescriptorTablesV1) -> Result<(), CoveError> {
        if self.digest_ref == 0 {
            return Err(CoveError::BadSection(
                "AiDigestEntryV1 digest_ref must be non-zero".into(),
            ));
        }
        if self.digest_algorithm == 0 || self.digest_len == 0 || self.digest_payload_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "AiDigestEntry {} requires non-zero digest_algorithm, digest_len, and digest_payload_ref",
                self.digest_ref
            )));
        }
        let payload_ref = tables.payload_ref(self.digest_payload_ref).ok_or_else(|| {
            CoveError::BadSection(format!(
                "AiDigestEntry {} references missing digest_payload_ref {}",
                self.digest_ref, self.digest_payload_ref
            ))
        })?;
        if payload_ref.decoded_length != u64::from(self.digest_len) {
            return Err(CoveError::BadSection(format!(
                "AiDigestEntry {} digest_len does not match payload_ref decoded_length",
                self.digest_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPayloadRefEntryV1 {
    pub payload_ref: u32,
    pub storage_kind: u8,
    pub media_type_ref: u32,
    pub section_id: u32,
    pub uri_ref: u32,
    pub payload_offset: u64,
    pub section_payload_offset: u64,
    pub payload_length: u64,
    pub decoded_length: u64,
    pub integrity_ref: u32,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiPayloadRefEntryV1 {
    pub(super) fn validate_storage(
        &self,
        sections: &[CoveAiSection],
        file_len: u64,
    ) -> Result<(), CoveError> {
        match AiStorageKindV1::from_u8(self.storage_kind)
            .ok_or_else(|| CoveError::BadSection("unknown AI payload storage_kind".into()))?
        {
            AiStorageKindV1::ArtifactAbsolute => {
                if self.section_id != 0 || self.section_payload_offset != 0 || self.uri_ref != 0 {
                    return Err(CoveError::BadSection(format!(
                        "payload_ref {} has invalid fields for ArtifactAbsolute storage",
                        self.payload_ref
                    )));
                }
                checked_range(self.payload_offset, self.payload_length, file_len)?;
            }
            AiStorageKindV1::SectionDecodedRelative => {
                if self.section_id == 0 || self.payload_offset != 0 || self.uri_ref != 0 {
                    return Err(CoveError::BadSection(format!(
                        "payload_ref {} has invalid fields for SectionDecodedRelative storage",
                        self.payload_ref
                    )));
                }
                let section = section_by_id(sections, self.section_id).ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "payload_ref {} references missing section_id {}",
                        self.payload_ref, self.section_id
                    ))
                })?;
                checked_range(
                    self.section_payload_offset,
                    self.payload_length,
                    section.entry.uncompressed_length,
                )?;
            }
            AiStorageKindV1::ExternalUri => {
                if self.uri_ref == 0
                    || self.payload_offset != 0
                    || self.section_payload_offset != 0
                    || self.section_id != 0
                {
                    return Err(CoveError::BadSection(format!(
                        "payload_ref {} has invalid fields for ExternalUri storage",
                        self.payload_ref
                    )));
                }
            }
            AiStorageKindV1::EmbeddedSection => {
                if self.section_id == 0
                    || self.payload_offset != 0
                    || self.section_payload_offset != 0
                    || self.uri_ref != 0
                {
                    return Err(CoveError::BadSection(format!(
                        "payload_ref {} has invalid fields for EmbeddedSection storage",
                        self.payload_ref
                    )));
                }
                section_by_id(sections, self.section_id).ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "payload_ref {} references missing embedded section_id {}",
                        self.payload_ref, self.section_id
                    ))
                })?;
            }
            AiStorageKindV1::Extension => {
                return Err(CoveError::BadExtension);
            }
        }
        Ok(())
    }

    pub(super) fn validate_token_or_vector_payload_carrier(
        &self,
        sections: &[CoveAiSection],
    ) -> Result<(), CoveError> {
        match AiStorageKindV1::from_u8(self.storage_kind)
            .ok_or_else(|| CoveError::BadSection("unknown AI payload storage_kind".into()))?
        {
            AiStorageKindV1::ArtifactAbsolute => {
                let mut containing = sections.iter().filter(|section| {
                    section.entry.section_kind == SectionKind::AiPayloadBytes as u32
                        && range_contains(
                            section.entry.offset,
                            section.entry.length,
                            self.payload_offset,
                            self.payload_length,
                        )
                });
                let Some(section) = containing.next() else {
                    return Err(CoveError::BadSection(format!(
                        "Phase 1 token/vector payload_ref {} is not contained in AI_PAYLOAD_BYTES",
                        self.payload_ref
                    )));
                };
                if containing.next().is_some() {
                    return Err(CoveError::BadSection(format!(
                        "Phase 1 token/vector payload_ref {} overlaps multiple AI_PAYLOAD_BYTES sections",
                        self.payload_ref
                    )));
                }
                if section.entry.compression != CompressionCodec::None as u8 {
                    return Err(CoveError::BadSection(
                        "ArtifactAbsolute token/vector payload refs into compressed AI_PAYLOAD_BYTES are invalid".into(),
                    ));
                }
            }
            AiStorageKindV1::SectionDecodedRelative => {
                let section = section_by_id(sections, self.section_id).ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "payload_ref {} references missing AI_PAYLOAD_BYTES section {}",
                        self.payload_ref, self.section_id
                    ))
                })?;
                if section.entry.section_kind != SectionKind::AiPayloadBytes as u32 {
                    return Err(CoveError::BadSection(format!(
                        "payload_ref {} SectionDecodedRelative target is not AI_PAYLOAD_BYTES",
                        self.payload_ref
                    )));
                }
            }
            _ => {
                return Err(CoveError::BadSection(format!(
                    "Phase 1 token/vector payload_ref {} must resolve to AI_PAYLOAD_BYTES",
                    self.payload_ref
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPolicyRefEntryV1 {
    pub policy_ref: u32,
    pub policy_kind: u8,
    pub authority_ref: u32,
    pub payload_ref: u32,
    pub digest_ref: u32,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiPolicyRefEntryV1 {
    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.policy_ref == 0 {
            return Err(CoveError::BadSection(
                "AiPolicyRefEntryV1 policy_ref must be non-zero".into(),
            ));
        }
        if !matches!(
            self.policy_kind,
            AI_POLICY_KIND_VISIBILITY
                | AI_POLICY_KIND_REDACTION
                | AI_POLICY_KIND_SENSITIVITY
                | AI_POLICY_KIND_LICENSE
                | AI_POLICY_KIND_RETENTION
                | AI_POLICY_KIND_DISCLOSURE
                | AI_POLICY_KIND_SAFETY
        ) {
            return Err(CoveError::BadSection(format!(
                "AiPolicyRefEntry {} has unknown policy_kind {}",
                self.policy_ref, self.policy_kind
            )));
        }
        if self.authority_ref != 0 && !string_ref_ids.contains(&self.authority_ref) {
            return Err(CoveError::BadSection(format!(
                "AiPolicyRefEntry {} references missing authority_ref {}",
                self.policy_ref, self.authority_ref
            )));
        }
        if self.payload_ref != 0 && !payload_ref_ids.contains(&self.payload_ref) {
            return Err(CoveError::BadSection(format!(
                "AiPolicyRefEntry {} references missing payload_ref {}",
                self.policy_ref, self.payload_ref
            )));
        }
        if self.digest_ref != 0 && !digest_ref_ids.contains(&self.digest_ref) {
            return Err(CoveError::BadSection(format!(
                "AiPolicyRefEntry {} references missing digest_ref {}",
                self.policy_ref, self.digest_ref
            )));
        }
        if self.payload_ref != 0 && self.digest_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "AiPolicyRefEntry {} with payload_ref requires digest_ref",
                self.policy_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSourceSpanEntryV1 {
    pub source_span_ref: u32,
    pub source_binding_ref: u32,
    pub source_kind: u8,
    pub source_row_ref: u64,
    pub source_object_ref: u64,
    pub byte_start: u64,
    pub byte_length: u64,
    pub token_start: u64,
    pub token_count: u32,
    pub evidence_ref: u32,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiSourceSpanEntryV1 {
    pub(super) fn validate(&self, source_binding_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
        if self.source_span_ref == 0 {
            return Err(CoveError::BadSection(
                "AiSourceSpanEntryV1 source_span_ref must be non-zero".into(),
            ));
        }
        if self.source_binding_ref != 0 && !source_binding_ids.contains(&self.source_binding_ref) {
            return Err(CoveError::BadSection(format!(
                "AiSourceSpanEntry {} references missing source_binding_ref {}",
                self.source_span_ref, self.source_binding_ref
            )));
        }
        if !matches!(
            self.source_kind,
            AI_SOURCE_KIND_COVE_FILE
                | AI_SOURCE_KIND_COVM_SNAPSHOT
                | AI_SOURCE_KIND_COVEMAP_ARTIFACT
                | AI_SOURCE_KIND_EXTERNAL_ASSET
                | AI_SOURCE_KIND_EXTERNAL_DATASET
        ) {
            return Err(CoveError::BadSection(format!(
                "AiSourceSpanEntry {} has unknown source_kind {}",
                self.source_span_ref, self.source_kind
            )));
        }
        self.byte_start
            .checked_add(self.byte_length)
            .ok_or(CoveError::ArithOverflow)?;
        self.token_start
            .checked_add(u64::from(self.token_count))
            .ok_or(CoveError::ArithOverflow)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiTransformEntryV1 {
    pub transform_ref: u32,
    pub transform_kind: u8,
    pub function_or_template_ref: u32,
    pub input_digest_ref: u32,
    pub output_digest_ref: u32,
    pub parameter_payload_ref: u32,
    pub transform_digest_ref: u32,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiTransformEntryV1 {
    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.transform_ref == 0 {
            return Err(CoveError::BadSection(
                "AiTransformEntryV1 transform_ref must be non-zero".into(),
            ));
        }
        if !matches!(
            self.transform_kind,
            AI_TRANSFORM_KIND_NONE
                | AI_TRANSFORM_KIND_TEXT_NORMALIZATION
                | AI_TRANSFORM_KIND_TOKENIZER
                | AI_TRANSFORM_KIND_CHUNKER
                | AI_TRANSFORM_KIND_VECTORIZER
                | AI_TRANSFORM_KIND_QUANTIZATION
                | AI_TRANSFORM_KIND_IMAGE_PREPROCESS
                | AI_TRANSFORM_KIND_AUDIO_PREPROCESS
                | AI_TRANSFORM_KIND_VIDEO_FRAME_EXTRACTION
                | AI_TRANSFORM_KIND_OCR
                | AI_TRANSFORM_KIND_CAPTION
                | AI_TRANSFORM_KIND_TRANSCRIPT
        ) {
            return Err(CoveError::BadSection(format!(
                "AiTransformEntry {} has unknown transform_kind {}",
                self.transform_ref, self.transform_kind
            )));
        }
        if self.function_or_template_ref != 0
            && !string_ref_ids.contains(&self.function_or_template_ref)
        {
            return Err(CoveError::BadSection(format!(
                "AiTransformEntry {} references missing function_or_template_ref {}",
                self.transform_ref, self.function_or_template_ref
            )));
        }
        for (label, digest_ref) in [
            ("input_digest_ref", self.input_digest_ref),
            ("output_digest_ref", self.output_digest_ref),
            ("transform_digest_ref", self.transform_digest_ref),
        ] {
            if digest_ref != 0 && !digest_ref_ids.contains(&digest_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiTransformEntry {} references missing {label} {}",
                    self.transform_ref, digest_ref
                )));
            }
        }
        if self.parameter_payload_ref != 0 && !payload_ref_ids.contains(&self.parameter_payload_ref)
        {
            return Err(CoveError::BadSection(format!(
                "AiTransformEntry {} references missing parameter_payload_ref {}",
                self.transform_ref, self.parameter_payload_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPrivacySummaryEntryV1 {
    pub privacy_summary_ref: u32,
    pub source_binding_ref: u32,
    pub sensitivity_mask: u32,
    pub sensitivity_bits_ref: u32,
    pub policy_ref: u32,
    pub visibility_scope_ref: u32,
    pub redaction_scope_ref: u32,
    pub retention_state: u8,
    pub disclosure_state: u8,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiPrivacySummaryEntryV1 {
    pub(super) fn validate(
        &self,
        source_binding_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.privacy_summary_ref == 0 {
            return Err(CoveError::BadSection(
                "AiPrivacySummaryEntryV1 privacy_summary_ref must be non-zero".into(),
            ));
        }
        if self.source_binding_ref != 0 && !source_binding_ids.contains(&self.source_binding_ref) {
            return Err(CoveError::BadSection(format!(
                "AiPrivacySummary {} references missing source_binding_ref {}",
                self.privacy_summary_ref, self.source_binding_ref
            )));
        }
        if self.sensitivity_bits_ref != 0 && !payload_ref_ids.contains(&self.sensitivity_bits_ref) {
            return Err(CoveError::BadSection(format!(
                "AiPrivacySummary {} references missing sensitivity_bits_ref {}",
                self.privacy_summary_ref, self.sensitivity_bits_ref
            )));
        }
        for (label, policy_ref) in [
            ("policy_ref", self.policy_ref),
            ("visibility_scope_ref", self.visibility_scope_ref),
            ("redaction_scope_ref", self.redaction_scope_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiPrivacySummary {} references missing {label} {}",
                    self.privacy_summary_ref, policy_ref
                )));
            }
        }
        validate_ai_privacy_state(self.retention_state, "AiPrivacySummary retention_state")?;
        validate_ai_privacy_state(self.disclosure_state, "AiPrivacySummary disclosure_state")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiCompanionArtifactRefV1 {
    pub artifact_ref: u32,
    pub artifact_kind: u8,
    pub artifact_id: [u8; 16],
    pub uri_ref: u32,
    pub artifact_digest_ref: u32,
    pub source_binding_ref: u32,
    pub required_ai_features: u64,
    pub optional_ai_features: u64,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiCompanionArtifactRefV1 {
    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        source_binding_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.artifact_ref == 0 {
            return Err(CoveError::BadSection(
                "AiCompanionArtifactRefV1 artifact_ref must be non-zero".into(),
            ));
        }
        if !matches!(
            self.artifact_kind,
            AI_COMPANION_ARTIFACT_KIND_CVA2 | AI_COMPANION_ARTIFACT_KIND_CVV2
        ) {
            return Err(CoveError::BadSection(format!(
                "AiCompanionArtifactRef {} has unknown artifact_kind {}",
                self.artifact_ref, self.artifact_kind
            )));
        }
        if self.artifact_id == [0; 16] {
            return Err(CoveError::BadSection(format!(
                "AiCompanionArtifactRef {} requires non-zero artifact_id",
                self.artifact_ref
            )));
        }
        if self.uri_ref == 0 || !string_ref_ids.contains(&self.uri_ref) {
            return Err(CoveError::BadSection(format!(
                "AiCompanionArtifactRef {} references missing uri_ref {}",
                self.artifact_ref, self.uri_ref
            )));
        }
        if self.artifact_digest_ref == 0 || !digest_ref_ids.contains(&self.artifact_digest_ref) {
            return Err(CoveError::BadSection(format!(
                "AiCompanionArtifactRef {} references missing artifact_digest_ref {}",
                self.artifact_ref, self.artifact_digest_ref
            )));
        }
        if self.source_binding_ref != 0 && !source_binding_ids.contains(&self.source_binding_ref) {
            return Err(CoveError::BadSection(format!(
                "AiCompanionArtifactRef {} references missing source_binding_ref {}",
                self.artifact_ref, self.source_binding_ref
            )));
        }
        if self.required_ai_features & !AI_KNOWN_FEATURES_V1 != 0 {
            return Err(CoveError::BadSection(format!(
                "AiCompanionArtifactRef {} requires unknown COVE-AI features",
                self.artifact_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSourceBindingV1 {
    pub source_binding_id: u32,
    pub source_kind: u8,
    pub source_artifact_ref: u32,
    pub source_file_digest_ref: u32,
    pub covm_snapshot_ref: u32,
    pub schema_fingerprint_ref: u32,
    pub dictionary_digest_ref: u32,
    pub map_fingerprint_ref: u32,
    pub policy_context_ref: u32,
    pub visibility_scope_ref: u32,
    pub redaction_scope_ref: u32,
    pub branch_ref: u32,
    pub as_of_csn: u64,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiSourceBindingV1 {
    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.source_binding_id == 0 {
            return Err(CoveError::BadSection(
                "AiSourceBindingV1 source_binding_id must be non-zero".into(),
            ));
        }
        if !matches!(
            self.source_kind,
            AI_SOURCE_KIND_COVE_FILE
                | AI_SOURCE_KIND_COVM_SNAPSHOT
                | AI_SOURCE_KIND_COVEMAP_ARTIFACT
                | AI_SOURCE_KIND_EXTERNAL_ASSET
                | AI_SOURCE_KIND_EXTERNAL_DATASET
        ) {
            return Err(CoveError::BadSection(format!(
                "AiSourceBinding {} has unknown source_kind {}",
                self.source_binding_id, self.source_kind
            )));
        }
        if self.source_artifact_ref != 0
            && !string_ref_ids.contains(&self.source_artifact_ref)
            && !payload_ref_ids.contains(&self.source_artifact_ref)
        {
            return Err(CoveError::BadSection(format!(
                "AiSourceBinding {} references missing source_artifact_ref {}",
                self.source_binding_id, self.source_artifact_ref
            )));
        }
        for (label, digest_ref) in [
            ("source_file_digest_ref", self.source_file_digest_ref),
            ("schema_fingerprint_ref", self.schema_fingerprint_ref),
            ("dictionary_digest_ref", self.dictionary_digest_ref),
            ("map_fingerprint_ref", self.map_fingerprint_ref),
        ] {
            if digest_ref != 0 && !digest_ref_ids.contains(&digest_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiSourceBinding {} references missing {label} {}",
                    self.source_binding_id, digest_ref
                )));
            }
        }
        for (label, policy_ref) in [
            ("policy_context_ref", self.policy_context_ref),
            ("visibility_scope_ref", self.visibility_scope_ref),
            ("redaction_scope_ref", self.redaction_scope_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiSourceBinding {} references missing {label} {}",
                    self.source_binding_id, policy_ref
                )));
            }
        }
        if self.branch_ref != 0 && !string_ref_ids.contains(&self.branch_ref) {
            return Err(CoveError::BadSection(format!(
                "AiSourceBinding {} references missing branch_ref {}",
                self.source_binding_id, self.branch_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiPayloadIntegrityV1 {
    pub integrity_ref: u32,
    pub payload_ref: u32,
    pub digest_domain: u8,
    pub reserved0: u8,
    pub digest_algorithm: u16,
    pub digest_len: u16,
    pub digest_ref: u32,
    pub payload_crc32c: u32,
    pub flags: u32,
}

impl AiPayloadIntegrityV1 {
    pub(super) fn validate(
        &self,
        tables: &AiDescriptorTablesV1,
        payload_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.integrity_ref == 0 {
            return Err(CoveError::BadSection(
                "AiPayloadIntegrityV1 integrity_ref must be non-zero".into(),
            ));
        }
        if !payload_ref_ids.contains(&self.payload_ref) {
            return Err(CoveError::BadSection(format!(
                "AI_PAYLOAD_INTEGRITY {} references missing payload_ref {}",
                self.integrity_ref, self.payload_ref
            )));
        }
        validate_ai_digest_domain(self.digest_domain, "AI_PAYLOAD_INTEGRITY digest_domain")?;
        if self.reserved0 != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        if self.digest_algorithm == 0 || self.digest_len == 0 || self.digest_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "AI_PAYLOAD_INTEGRITY {} requires non-zero digest_algorithm, digest_len, and digest_ref",
                self.integrity_ref
            )));
        }
        let digest = tables.digest_ref(self.digest_ref).ok_or_else(|| {
            CoveError::BadSection(format!(
                "AI_PAYLOAD_INTEGRITY {} references missing digest_ref {}",
                self.integrity_ref, self.digest_ref
            ))
        })?;
        if digest.digest_algorithm != self.digest_algorithm || digest.digest_len != self.digest_len
        {
            return Err(CoveError::BadSection(format!(
                "AI_PAYLOAD_INTEGRITY {} digest algorithm/length mismatch",
                self.integrity_ref
            )));
        }
        if digest.digest_payload_ref == self.payload_ref {
            return Err(CoveError::BadSection(format!(
                "AI_PAYLOAD_INTEGRITY {} digest payload must not be the protected payload_ref",
                self.integrity_ref
            )));
        }
        let digest_payload = tables
            .payload_ref(digest.digest_payload_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "AI_PAYLOAD_INTEGRITY {} references missing digest payload_ref {}",
                    self.integrity_ref, digest.digest_payload_ref
                ))
            })?;
        if digest_payload.integrity_ref == self.integrity_ref {
            return Err(CoveError::BadSection(format!(
                "AI_PAYLOAD_INTEGRITY {} has cyclic digest payload integrity",
                self.integrity_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSectionFeatureBindingV1 {
    pub binding_ref: u32,
    pub section_id: u32,
    pub scope: u8,
    pub profile_kind: u8,
    pub operation_kind: u16,
    pub required_ai_features: u64,
    pub optional_ai_features: u64,
    pub target_local_ref: u64,
    pub flags: u32,
    pub crc32c: u32,
}

impl AiSectionFeatureBindingV1 {
    pub(super) fn validate(&self, sections: &[CoveAiSection]) -> Result<(), CoveError> {
        if self.binding_ref == 0 {
            return Err(CoveError::BadSection(
                "AiSectionFeatureBindingV1 binding_ref must be non-zero".into(),
            ));
        }
        let Some(section) = section_by_id(sections, self.section_id) else {
            return Err(CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} references missing section_id {}",
                self.binding_ref, self.section_id
            )));
        };
        if section.entry.section_kind == SectionKind::AiSectionFeatureBinding as u32 {
            return Err(CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} must not target AI_SECTION_FEATURE_BINDING",
                self.binding_ref
            )));
        }
        let scope = FeatureScopeV2::from_u8(self.scope).ok_or_else(|| {
            CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} has unknown scope {}",
                self.binding_ref, self.scope
            ))
        })?;
        if PrimaryProfile::from_u8(self.profile_kind).is_none() {
            return Err(CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} has unknown profile_kind {}",
                self.binding_ref, self.profile_kind
            )));
        }
        if OperationKindV2::from_u16(self.operation_kind).is_none() {
            return Err(CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} has unknown operation_kind {}",
                self.binding_ref, self.operation_kind
            )));
        }
        if scope == FeatureScopeV2::OperationRequired && self.operation_kind == 0 {
            return Err(CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} operation-required scope needs operation_kind",
                self.binding_ref
            )));
        }
        if scope != FeatureScopeV2::OperationRequired && self.operation_kind != 0 {
            return Err(CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} non-operation scope must not set operation_kind",
                self.binding_ref
            )));
        }
        if self.required_ai_features & !AI_KNOWN_FEATURES_V1 != 0
            && scope == FeatureScopeV2::FileRequired
        {
            return Err(CoveError::UnknownRequiredFeature(
                self.required_ai_features & !AI_KNOWN_FEATURES_V1,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkProfileV1 {
    pub chunk_profile_id: u32,
    pub profile_name_ref: u32,
    pub chunker_namespace_ref: u32,
    pub chunker_name_ref: u32,
    pub chunker_version_major: u16,
    pub chunker_version_minor: u16,
    pub tokenizer_profile_ref: u32,
    pub boundary_kind: u8,
    pub overlap_policy: u8,
    pub parent_policy: u8,
    pub normalization_policy: u8,
    pub target_tokens: u32,
    pub min_tokens: u32,
    pub max_tokens: u32,
    pub overlap_tokens: u32,
    pub max_bytes: u32,
    pub locale_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl ChunkProfileV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.chunk_profile_id == 0 {
            return Err(CoveError::BadSection(
                "ChunkProfileV1 chunk_profile_id must be non-zero".into(),
            ));
        }
        if self.min_tokens > self.max_tokens && self.max_tokens != 0 {
            return Err(CoveError::BadSection(format!(
                "ChunkProfile {} min_tokens exceeds max_tokens",
                self.chunk_profile_id
            )));
        }
        if self.overlap_tokens > self.max_tokens && self.max_tokens != 0 {
            return Err(CoveError::BadSection(format!(
                "ChunkProfile {} overlap_tokens exceeds max_tokens",
                self.chunk_profile_id
            )));
        }
        if self.max_tokens != 0 && self.target_tokens > self.max_tokens {
            return Err(CoveError::BadSection(format!(
                "ChunkProfile {} target_tokens exceeds max_tokens",
                self.chunk_profile_id
            )));
        }
        if self.target_tokens != 0 && self.target_tokens < self.min_tokens {
            return Err(CoveError::BadSection(format!(
                "ChunkProfile {} target_tokens is below min_tokens",
                self.chunk_profile_id
            )));
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        tokenizer_profile_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        validate_ai_chunk_boundary_kind(self.boundary_kind, "ChunkProfile boundary_kind")?;
        validate_ai_chunk_overlap_policy(self.overlap_policy, "ChunkProfile overlap_policy")?;
        validate_ai_chunk_parent_policy(self.parent_policy, "ChunkProfile parent_policy")?;
        validate_ai_normalization_policy(
            self.normalization_policy,
            "ChunkProfile normalization_policy",
        )?;
        for (label, string_ref) in [
            ("profile_name_ref", self.profile_name_ref),
            ("chunker_namespace_ref", self.chunker_namespace_ref),
            ("chunker_name_ref", self.chunker_name_ref),
            ("locale_ref", self.locale_ref),
        ] {
            if string_ref != 0 && !string_ref_ids.contains(&string_ref) {
                return Err(CoveError::BadSection(format!(
                    "ChunkProfile {} references missing {label} {}",
                    self.chunk_profile_id, string_ref
                )));
            }
        }
        if self.tokenizer_profile_ref != 0
            && !tokenizer_profile_ids.contains(&self.tokenizer_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "ChunkProfile {} references missing tokenizer_profile_ref {}",
                self.chunk_profile_id, self.tokenizer_profile_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunkEntryV1 {
    pub chunk_id: u64,
    pub source_ref: u32,
    pub table_id: u32,
    pub column_id: u32,
    pub object_type_id: u32,
    pub property_id: u32,
    pub association_type_id: u32,
    pub path_ref: u32,
    pub source_row_ref: u64,
    pub source_object_ref: u64,
    pub source_value_hash_ref: u32,
    pub byte_start: u64,
    pub byte_length: u64,
    pub unicode_scalar_start: u64,
    pub unicode_scalar_length: u64,
    pub token_start: u64,
    pub token_count: u32,
    pub parent_chunk_id: u64,
    pub first_child_ref: u32,
    pub child_count: u32,
    pub previous_chunk_id: u64,
    pub next_chunk_id: u64,
    pub chunk_text_hash_ref: u32,
    pub evidence_ref: u32,
    pub policy_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl TextChunkEntryV1 {
    pub(super) fn validate_refs(
        &self,
        chunk_ids: &BTreeSet<u64>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.chunk_id == 0 {
            return Err(CoveError::BadSection(
                "TextChunkEntry chunk_id must be non-zero".into(),
            ));
        }
        if self.source_ref != 0 && !source_binding_ids.contains(&self.source_ref) {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} references missing source_ref {}",
                self.chunk_id, self.source_ref
            )));
        }
        if self.source_value_hash_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} requires source_value_hash_ref",
                self.chunk_id
            )));
        }
        if !digest_ref_ids.contains(&self.source_value_hash_ref) {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} references missing source_value_hash_ref {}",
                self.chunk_id, self.source_value_hash_ref
            )));
        }
        if self.chunk_text_hash_ref != 0 && !digest_ref_ids.contains(&self.chunk_text_hash_ref) {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} references missing chunk_text_hash_ref {}",
                self.chunk_id, self.chunk_text_hash_ref
            )));
        }
        if self.evidence_ref != 0 && !payload_ref_ids.contains(&self.evidence_ref) {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} references missing evidence_ref {}",
                self.chunk_id, self.evidence_ref
            )));
        }
        if self.policy_ref != 0 && !policy_ref_ids.contains(&self.policy_ref) {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} references missing policy_ref {}",
                self.chunk_id, self.policy_ref
            )));
        }
        if self.byte_length == 0 {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} requires non-zero byte_length",
                self.chunk_id
            )));
        }
        self.byte_start
            .checked_add(self.byte_length)
            .ok_or(CoveError::ArithOverflow)?;
        self.unicode_scalar_start
            .checked_add(self.unicode_scalar_length)
            .ok_or(CoveError::ArithOverflow)?;
        self.token_start
            .checked_add(u64::from(self.token_count))
            .ok_or(CoveError::ArithOverflow)?;
        if self.parent_chunk_id == self.chunk_id
            || self.previous_chunk_id == self.chunk_id
            || self.next_chunk_id == self.chunk_id
        {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} has a self-referential chunk link",
                self.chunk_id
            )));
        }
        for (label, id) in [
            ("parent_chunk_id", self.parent_chunk_id),
            ("previous_chunk_id", self.previous_chunk_id),
            ("next_chunk_id", self.next_chunk_id),
        ] {
            if id != 0 && !chunk_ids.contains(&id) {
                return Err(CoveError::BadSection(format!(
                    "TextChunkEntry {} references missing {label} {}",
                    self.chunk_id, id
                )));
            }
        }
        if (self.first_child_ref == 0) != (self.child_count == 0) {
            return Err(CoveError::BadSection(format!(
                "TextChunkEntry {} first_child_ref/child_count mismatch",
                self.chunk_id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizerProfileV1 {
    pub tokenizer_profile_id: u32,
    pub tokenizer_namespace_ref: u32,
    pub tokenizer_name_ref: u32,
    pub tokenizer_version_major: u16,
    pub tokenizer_version_minor: u16,
    pub vocab_digest_ref: u32,
    pub merges_digest_ref: u32,
    pub pre_tokenizer_digest_ref: u32,
    pub normalizer_digest_ref: u32,
    pub byte_encoder_digest_ref: u32,
    pub special_tokens_digest_ref: u32,
    pub added_tokens_digest_ref: u32,
    pub chat_template_ref: u32,
    pub unicode_version_ref: u32,
    pub truncation_policy_ref: u32,
    pub padding_policy_ref: u32,
    pub model_max_sequence_length: u32,
    pub token_id_width: u8,
    pub byte_alignment_available: u8,
    pub reversible: u8,
    pub deterministic: u8,
    pub bos_token_id: u32,
    pub eos_token_id: u32,
    pub pad_token_id: u32,
    pub unk_token_id: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl TokenizerProfileV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.tokenizer_profile_id == 0 {
            return Err(CoveError::BadSection(
                "TokenizerProfileV1 tokenizer_profile_id must be non-zero".into(),
            ));
        }
        if !matches!(self.token_id_width, 1 | 2 | 4 | 8) {
            return Err(CoveError::BadSection(format!(
                "TokenizerProfile {} has unsupported token_id_width {}",
                self.tokenizer_profile_id, self.token_id_width
            )));
        }
        validate_bool_byte(
            self.byte_alignment_available,
            "TokenizerProfile byte_alignment_available",
        )?;
        validate_bool_byte(self.reversible, "TokenizerProfile reversible")?;
        validate_bool_byte(self.deterministic, "TokenizerProfile deterministic")?;
        Ok(())
    }

    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        for (label, string_ref) in [
            ("tokenizer_namespace_ref", self.tokenizer_namespace_ref),
            ("tokenizer_name_ref", self.tokenizer_name_ref),
            ("unicode_version_ref", self.unicode_version_ref),
        ] {
            if string_ref != 0 && !string_ref_ids.contains(&string_ref) {
                return Err(CoveError::BadSection(format!(
                    "TokenizerProfile {} references missing {label} {}",
                    self.tokenizer_profile_id, string_ref
                )));
            }
        }
        for (label, digest_ref) in [
            ("vocab_digest_ref", self.vocab_digest_ref),
            ("merges_digest_ref", self.merges_digest_ref),
            ("pre_tokenizer_digest_ref", self.pre_tokenizer_digest_ref),
            ("normalizer_digest_ref", self.normalizer_digest_ref),
            ("byte_encoder_digest_ref", self.byte_encoder_digest_ref),
            ("special_tokens_digest_ref", self.special_tokens_digest_ref),
            ("added_tokens_digest_ref", self.added_tokens_digest_ref),
        ] {
            if digest_ref != 0 && !digest_ref_ids.contains(&digest_ref) {
                return Err(CoveError::BadSection(format!(
                    "TokenizerProfile {} references missing {label} {}",
                    self.tokenizer_profile_id, digest_ref
                )));
            }
        }
        if self.chat_template_ref != 0 && !payload_ref_ids.contains(&self.chat_template_ref) {
            return Err(CoveError::BadSection(format!(
                "TokenizerProfile {} references missing chat_template_ref {}",
                self.tokenizer_profile_id, self.chat_template_ref
            )));
        }
        for (label, policy_ref) in [
            ("truncation_policy_ref", self.truncation_policy_ref),
            ("padding_policy_ref", self.padding_policy_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "TokenizerProfile {} references missing {label} {}",
                    self.tokenizer_profile_id, policy_ref
                )));
            }
        }
        let max_token_id = match self.token_id_width {
            1 => u8::MAX as u32,
            2 => u16::MAX as u32,
            4 | 8 => u32::MAX,
            _ => unreachable!("validate_static rejects unsupported token_id_width"),
        };
        for (label, token_id) in [
            ("bos_token_id", self.bos_token_id),
            ("eos_token_id", self.eos_token_id),
            ("pad_token_id", self.pad_token_id),
            ("unk_token_id", self.unk_token_id),
        ] {
            if token_id > max_token_id {
                return Err(CoveError::BadSection(format!(
                    "TokenizerProfile {} {label} {} exceeds token_id_width {}",
                    self.tokenizer_profile_id, token_id, self.token_id_width
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBlockHeaderV1 {
    pub token_block_id: u32,
    pub tokenizer_profile_id: u32,
    pub token_count: u64,
    pub token_id_width: u8,
    pub compression_codec: u8,
    pub layout_kind: u8,
    pub payload_ref: u32,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub integrity_ref: u32,
    pub checksum: u32,
}

impl TokenBlockHeaderV1 {
    pub(super) fn validate(
        &self,
        tables: &AiDescriptorTablesV1,
        sections: &[CoveAiSection],
        tokenizer_profile_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        integrity_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.token_block_id == 0 {
            return Err(CoveError::BadSection(
                "TokenBlockHeaderV1 token_block_id must be non-zero".into(),
            ));
        }
        if !tokenizer_profile_ids.contains(&self.tokenizer_profile_id) {
            return Err(CoveError::BadSection(format!(
                "TokenBlockHeader {} references missing tokenizer_profile_id {}",
                self.token_block_id, self.tokenizer_profile_id
            )));
        }
        if self.token_count == 0 {
            return Err(CoveError::BadSection(format!(
                "TokenBlockHeader {} requires non-zero token_count",
                self.token_block_id
            )));
        }
        if !matches!(self.token_id_width, 1 | 2 | 4 | 8) {
            return Err(CoveError::BadSection(format!(
                "TokenBlockHeader {} has unsupported token_id_width {}",
                self.token_block_id, self.token_id_width
            )));
        }
        validate_ai_compression_codec(
            self.compression_codec,
            "TokenBlockHeader compression_codec",
        )?;
        validate_ai_layout_kind(self.layout_kind, "TokenBlockHeader layout_kind")?;
        if !payload_ref_ids.contains(&self.payload_ref) {
            return Err(CoveError::BadSection(format!(
                "TokenBlockHeader {} references missing payload_ref {}",
                self.token_block_id, self.payload_ref
            )));
        }
        let payload_ref = tables.payload_ref(self.payload_ref).ok_or_else(|| {
            CoveError::BadSection(format!(
                "TokenBlockHeader {} references missing payload_ref {}",
                self.token_block_id, self.payload_ref
            ))
        })?;
        payload_ref.validate_token_or_vector_payload_carrier(sections)?;
        validate_cached_payload_range(
            "TokenBlockHeader",
            self.token_block_id,
            payload_ref,
            self.payload_offset,
            self.payload_length,
        )?;
        if self.compression_codec == CompressionCodec::None as u8 && self.layout_kind == 0 {
            let expected_min_len = self
                .token_count
                .checked_mul(u64::from(self.token_id_width))
                .ok_or(CoveError::ArithOverflow)?;
            if payload_ref.decoded_length < expected_min_len {
                return Err(CoveError::BadSection(format!(
                    "TokenBlockHeader {} payload decoded_length is shorter than token_count * token_id_width",
                    self.token_block_id
                )));
            }
        }
        if self.integrity_ref != 0 {
            if !integrity_ids.contains(&self.integrity_ref) {
                return Err(CoveError::BadSection(format!(
                    "TokenBlockHeader {} references missing integrity_ref {}",
                    self.token_block_id, self.integrity_ref
                )));
            }
            let integrity = tables.integrity_ref(self.integrity_ref).ok_or_else(|| {
                CoveError::BadSection(format!(
                    "TokenBlockHeader {} references missing integrity_ref {}",
                    self.token_block_id, self.integrity_ref
                ))
            })?;
            if integrity.payload_ref != self.payload_ref {
                return Err(CoveError::BadSection(format!(
                    "TokenBlockHeader {} integrity_ref payload_ref mismatch",
                    self.token_block_id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizedSpanV1 {
    pub tokenized_span_id: u64,
    pub chunk_id: u64,
    pub tokenizer_profile_id: u32,
    pub token_block_ref: u32,
    pub token_offset: u64,
    pub token_count: u32,
    pub byte_alignment_ref: u32,
    pub source_value_hash_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl TokenizedSpanV1 {
    pub(super) fn validate(
        &self,
        tables: &AiDescriptorTablesV1,
        tokenizer_profile_ids: &BTreeSet<u32>,
        token_block_ids: &BTreeSet<u32>,
        chunk_ids: &BTreeSet<u64>,
        payload_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.tokenized_span_id == 0 {
            return Err(CoveError::BadSection(
                "TokenizedSpanV1 tokenized_span_id must be non-zero".into(),
            ));
        }
        if !tokenizer_profile_ids.contains(&self.tokenizer_profile_id) {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} references missing tokenizer_profile_id {}",
                self.tokenized_span_id, self.tokenizer_profile_id
            )));
        }
        if !token_block_ids.contains(&self.token_block_ref) {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} references missing token_block_ref {}",
                self.tokenized_span_id, self.token_block_ref
            )));
        }
        if self.chunk_id != 0 && !chunk_ids.contains(&self.chunk_id) {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} references missing chunk_id {}",
                self.tokenized_span_id, self.chunk_id
            )));
        }
        if self.token_count == 0 {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} requires non-zero token_count",
                self.tokenized_span_id
            )));
        }
        if self.source_value_hash_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} requires source_value_hash_ref",
                self.tokenized_span_id
            )));
        }
        if !digest_ref_ids.contains(&self.source_value_hash_ref) {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} references missing source_value_hash_ref {}",
                self.tokenized_span_id, self.source_value_hash_ref
            )));
        }
        if self.byte_alignment_ref != 0 && !payload_ref_ids.contains(&self.byte_alignment_ref) {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} references missing byte_alignment_ref {}",
                self.tokenized_span_id, self.byte_alignment_ref
            )));
        }
        let block = tables
            .token_blocks
            .iter()
            .find(|block| block.token_block_id == self.token_block_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "TokenizedSpan {} references missing token_block_ref {}",
                    self.tokenized_span_id, self.token_block_ref
                ))
            })?;
        validate_token_range(
            "TokenizedSpan",
            self.tokenized_span_id,
            self.token_offset,
            self.token_count,
            block,
        )?;
        if block.tokenizer_profile_id != self.tokenizer_profile_id {
            return Err(CoveError::BadSection(format!(
                "TokenizedSpan {} tokenizer_profile_id does not match token block",
                self.tokenized_span_id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSequencePackV1 {
    pub sequence_pack_id: u64,
    pub tokenizer_profile_id: u32,
    pub training_profile_ref: u32,
    pub token_block_ref: u32,
    pub token_offset: u64,
    pub token_count: u32,
    pub source_span_count: u32,
    pub first_source_span_ref: u32,
    pub loss_mask_ref: u32,
    pub attention_mask_ref: u32,
    pub position_ids_ref: u32,
    pub labels_ref: u32,
    pub split_ref: u32,
    pub sample_weight_ppm: u32,
    pub flags: u32,
    pub checksum: u32,
}

pub(super) struct TokenSequencePackValidateRefs<'a> {
    pub(super) tables: &'a AiDescriptorTablesV1,
    pub(super) tokenizer_profile_ids: &'a BTreeSet<u32>,
    pub(super) token_block_ids: &'a BTreeSet<u32>,
    pub(super) payload_ref_ids: &'a BTreeSet<u32>,
    pub(super) training_profile_ids: &'a BTreeSet<u32>,
    pub(super) split_ids: &'a BTreeSet<u32>,
    pub(super) tokenized_span_ids: &'a BTreeSet<u64>,
}

impl TokenSequencePackV1 {
    pub(super) fn validate(
        &self,
        refs: TokenSequencePackValidateRefs<'_>,
    ) -> Result<(), CoveError> {
        let TokenSequencePackValidateRefs {
            tables,
            tokenizer_profile_ids,
            token_block_ids,
            payload_ref_ids,
            training_profile_ids,
            split_ids,
            tokenized_span_ids,
        } = refs;
        if self.sequence_pack_id == 0 {
            return Err(CoveError::BadSection(
                "TokenSequencePackV1 sequence_pack_id must be non-zero".into(),
            ));
        }
        if !tokenizer_profile_ids.contains(&self.tokenizer_profile_id) {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} references missing tokenizer_profile_id {}",
                self.sequence_pack_id, self.tokenizer_profile_id
            )));
        }
        if !token_block_ids.contains(&self.token_block_ref) {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} references missing token_block_ref {}",
                self.sequence_pack_id, self.token_block_ref
            )));
        }
        if self.token_count == 0 {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} requires non-zero token_count",
                self.sequence_pack_id
            )));
        }
        if self.sample_weight_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} sample_weight_ppm exceeds 1_000_000",
                self.sequence_pack_id
            )));
        }
        if self.training_profile_ref != 0
            && !training_profile_ids.contains(&self.training_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} references missing training_profile_ref {}",
                self.sequence_pack_id, self.training_profile_ref
            )));
        }
        if self.split_ref != 0 && !split_ids.contains(&self.split_ref) {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} references missing split_ref {}",
                self.sequence_pack_id, self.split_ref
            )));
        }
        if (self.source_span_count == 0) != (self.first_source_span_ref == 0) {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} first_source_span_ref/source_span_count mismatch",
                self.sequence_pack_id
            )));
        }
        if self.first_source_span_ref != 0
            && !tokenized_span_ids.contains(&u64::from(self.first_source_span_ref))
        {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} references missing first_source_span_ref {}",
                self.sequence_pack_id, self.first_source_span_ref
            )));
        }
        for (label, payload_ref) in [
            ("loss_mask_ref", self.loss_mask_ref),
            ("attention_mask_ref", self.attention_mask_ref),
            ("position_ids_ref", self.position_ids_ref),
            ("labels_ref", self.labels_ref),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "TokenSequencePack {} references missing {label} {}",
                    self.sequence_pack_id, payload_ref
                )));
            }
        }
        let block = tables
            .token_blocks
            .iter()
            .find(|block| block.token_block_id == self.token_block_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "TokenSequencePack {} references missing token_block_ref {}",
                    self.sequence_pack_id, self.token_block_ref
                ))
            })?;
        validate_token_range(
            "TokenSequencePack",
            self.sequence_pack_id,
            self.token_offset,
            self.token_count,
            block,
        )?;
        if block.tokenizer_profile_id != self.tokenizer_profile_id {
            return Err(CoveError::BadSection(format!(
                "TokenSequencePack {} tokenizer_profile_id does not match token block",
                self.sequence_pack_id
            )));
        }
        for (label, payload_ref, width) in [
            ("loss_mask_ref", self.loss_mask_ref, 1u64),
            ("attention_mask_ref", self.attention_mask_ref, 1u64),
            ("position_ids_ref", self.position_ids_ref, 4u64),
            (
                "labels_ref",
                self.labels_ref,
                u64::from(block.token_id_width),
            ),
        ] {
            if payload_ref == 0 {
                continue;
            }
            let payload = tables.payload_ref(payload_ref).ok_or_else(|| {
                CoveError::BadSection(format!(
                    "TokenSequencePack {} references missing {label} {}",
                    self.sequence_pack_id, payload_ref
                ))
            })?;
            let expected_min_len = u64::from(self.token_count)
                .checked_mul(width)
                .ok_or(CoveError::ArithOverflow)?;
            if payload.decoded_length < expected_min_len {
                return Err(CoveError::BadSection(format!(
                    "TokenSequencePack {} {label} decoded_length is shorter than token_count * element width",
                    self.sequence_pack_id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingProfileV1 {
    pub training_profile_id: u32,
    pub profile_name_ref: u32,
    pub task_family: u8,
    pub modality_mask: u32,
    pub source_snapshot_ref: u32,
    pub map_profile_ref: u32,
    pub chunk_profile_ref: u32,
    pub tokenizer_profile_ref: u32,
    pub vector_space_ref: u32,
    pub multimodal_sequence_profile_ref: u32,
    pub split_policy_ref: u32,
    pub sampling_policy_ref: u32,
    pub dedup_policy_ref: u32,
    pub quality_policy_ref: u32,
    pub license_policy_ref: u32,
    pub redaction_policy_ref: u32,
    pub default_generator_provenance_ref: u64,
    pub reproducibility_class: u8,
    pub flags: u32,
    pub checksum: u32,
}

impl TrainingProfileV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.training_profile_id == 0 {
            return Err(CoveError::BadSection(
                "TrainingProfileV1 training_profile_id must be non-zero".into(),
            ));
        }
        if !(self.task_family <= 15 || self.task_family == 255) {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} has unsupported task_family {}",
                self.training_profile_id, self.task_family
            )));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
        chunk_profile_ids: &BTreeSet<u32>,
        tokenizer_profile_ids: &BTreeSet<u32>,
        vector_space_ids: &BTreeSet<u32>,
        generator_provenance_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        validate_ai_reproducibility_class(
            self.reproducibility_class,
            "TrainingProfile reproducibility_class",
        )?;
        if self.profile_name_ref != 0 && !string_ref_ids.contains(&self.profile_name_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing profile_name_ref {}",
                self.training_profile_id, self.profile_name_ref
            )));
        }
        if self.source_snapshot_ref != 0 && !payload_ref_ids.contains(&self.source_snapshot_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing source_snapshot_ref {}",
                self.training_profile_id, self.source_snapshot_ref
            )));
        }
        if self.map_profile_ref != 0 && !payload_ref_ids.contains(&self.map_profile_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing map_profile_ref {}",
                self.training_profile_id, self.map_profile_ref
            )));
        }
        if self.chunk_profile_ref != 0 && !chunk_profile_ids.contains(&self.chunk_profile_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing chunk_profile_ref {}",
                self.training_profile_id, self.chunk_profile_ref
            )));
        }
        if self.tokenizer_profile_ref != 0
            && !tokenizer_profile_ids.contains(&self.tokenizer_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing tokenizer_profile_ref {}",
                self.training_profile_id, self.tokenizer_profile_ref
            )));
        }
        if self.vector_space_ref != 0 && !vector_space_ids.contains(&self.vector_space_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing vector_space_ref {}",
                self.training_profile_id, self.vector_space_ref
            )));
        }
        if self.multimodal_sequence_profile_ref != 0
            && !payload_ref_ids.contains(&self.multimodal_sequence_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing multimodal_sequence_profile_ref {}",
                self.training_profile_id, self.multimodal_sequence_profile_ref
            )));
        }
        for (label, policy_ref) in [
            ("split_policy_ref", self.split_policy_ref),
            ("sampling_policy_ref", self.sampling_policy_ref),
            ("dedup_policy_ref", self.dedup_policy_ref),
            ("quality_policy_ref", self.quality_policy_ref),
            ("license_policy_ref", self.license_policy_ref),
            ("redaction_policy_ref", self.redaction_policy_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "TrainingProfile {} references missing {label} {}",
                    self.training_profile_id, policy_ref
                )));
            }
        }
        if self.default_generator_provenance_ref != 0
            && !generator_provenance_ids.contains(&self.default_generator_provenance_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingProfile {} references missing default_generator_provenance_ref {}",
                self.training_profile_id, self.default_generator_provenance_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingSampleEntryV1 {
    pub sample_id: u64,
    pub training_profile_id: u32,
    pub example_kind: u8,
    pub split_ref: u32,
    pub source_ref: u32,
    pub evidence_ref: u32,
    pub input_ref: u32,
    pub target_ref: u32,
    pub label_ref: u32,
    pub metadata_ref: u32,
    pub token_sequence_pack_ref: u64,
    pub multimodal_sequence_pack_ref: u64,
    pub vector_ref: u64,
    pub quality_score_ppm: u32,
    pub sample_weight_ppm: u32,
    pub dedup_group_ref: u32,
    pub license_ref: u32,
    pub policy_ref: u32,
    pub teacher_model_ref: u32,
    pub generator_provenance_ref: u64,
    pub judge_generator_provenance_ref: u64,
    pub label_generator_provenance_ref: u64,
    pub flags: u32,
    pub checksum: u32,
}

pub(super) struct TrainingSampleValidateRefs<'a> {
    pub(super) training_profile_ids: &'a BTreeSet<u32>,
    pub(super) split_ids: &'a BTreeSet<u32>,
    pub(super) dedup_group_ids: &'a BTreeSet<u64>,
    pub(super) token_sequence_pack_ids: &'a BTreeSet<u64>,
    pub(super) multimodal_sequence_pack_ids: &'a BTreeSet<u64>,
    pub(super) vector_ref_ids: &'a BTreeSet<u64>,
    pub(super) training_label_ids: &'a BTreeSet<u64>,
    pub(super) generator_provenance_ids: &'a BTreeSet<u64>,
    pub(super) payload_ref_ids: &'a BTreeSet<u32>,
    pub(super) policy_ref_ids: &'a BTreeSet<u32>,
    pub(super) model_actor_ids: &'a BTreeSet<u32>,
}

impl TrainingSampleEntryV1 {
    pub(super) fn validate(&self, refs: TrainingSampleValidateRefs<'_>) -> Result<(), CoveError> {
        let TrainingSampleValidateRefs {
            training_profile_ids,
            split_ids,
            dedup_group_ids,
            token_sequence_pack_ids,
            multimodal_sequence_pack_ids,
            vector_ref_ids,
            training_label_ids,
            generator_provenance_ids,
            payload_ref_ids,
            policy_ref_ids,
            model_actor_ids,
        } = refs;
        if self.sample_id == 0 {
            return Err(CoveError::BadSection(
                "TrainingSampleEntryV1 sample_id must be non-zero".into(),
            ));
        }
        if !training_profile_ids.contains(&self.training_profile_id) {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing training_profile_id {}",
                self.sample_id, self.training_profile_id
            )));
        }
        if !(self.example_kind <= 15 || self.example_kind == 255) {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} has unsupported example_kind {}",
                self.sample_id, self.example_kind
            )));
        }
        if self.quality_score_ppm > 1_000_000 || self.sample_weight_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} ppm field exceeds 1_000_000",
                self.sample_id
            )));
        }
        if self.split_ref != 0 && !split_ids.contains(&self.split_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing split_ref {}",
                self.sample_id, self.split_ref
            )));
        }
        if self.dedup_group_ref != 0 && !dedup_group_ids.contains(&u64::from(self.dedup_group_ref))
        {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing dedup_group_ref {}",
                self.sample_id, self.dedup_group_ref
            )));
        }
        if self.token_sequence_pack_ref != 0
            && !token_sequence_pack_ids.contains(&self.token_sequence_pack_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing token_sequence_pack_ref {}",
                self.sample_id, self.token_sequence_pack_ref
            )));
        }
        if self.multimodal_sequence_pack_ref != 0
            && !multimodal_sequence_pack_ids.contains(&self.multimodal_sequence_pack_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing multimodal_sequence_pack_ref {}",
                self.sample_id, self.multimodal_sequence_pack_ref
            )));
        }
        if self.vector_ref != 0 && !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing vector_ref {}",
                self.sample_id, self.vector_ref
            )));
        }
        if self.label_ref != 0 && !training_label_ids.contains(&u64::from(self.label_ref)) {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing label_ref {}",
                self.sample_id, self.label_ref
            )));
        }
        for (label, generator_ref) in [
            ("generator_provenance_ref", self.generator_provenance_ref),
            (
                "judge_generator_provenance_ref",
                self.judge_generator_provenance_ref,
            ),
            (
                "label_generator_provenance_ref",
                self.label_generator_provenance_ref,
            ),
        ] {
            if generator_ref != 0 && !generator_provenance_ids.contains(&generator_ref) {
                return Err(CoveError::BadSection(format!(
                    "TrainingSample {} references missing {label} {}",
                    self.sample_id, generator_ref
                )));
            }
        }
        for (label, payload_ref) in [
            ("source_ref", self.source_ref),
            ("evidence_ref", self.evidence_ref),
            ("input_ref", self.input_ref),
            ("target_ref", self.target_ref),
            ("metadata_ref", self.metadata_ref),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "TrainingSample {} references missing {label} {}",
                    self.sample_id, payload_ref
                )));
            }
        }
        for (label, policy_ref) in [
            ("license_ref", self.license_ref),
            ("policy_ref", self.policy_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "TrainingSample {} references missing {label} {}",
                    self.sample_id, policy_ref
                )));
            }
        }
        if self.teacher_model_ref != 0 && !model_actor_ids.contains(&self.teacher_model_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingSample {} references missing teacher_model_ref {}",
                self.sample_id, self.teacher_model_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSplitV1 {
    pub split_id: u32,
    pub split_name_ref: u32,
    pub split_method: u8,
    pub source_snapshot_ref: u32,
    pub filter_policy_ref: u32,
    pub seed: u64,
    pub hash_function_ref: u32,
    pub stratification_path_ref: u32,
    pub grouping_ref: u32,
    pub ordering_policy_ref: u32,
    pub dedup_policy_ref: u32,
    pub sample_count: u64,
    pub first_sample_ref: u64,
    pub flags: u32,
    pub checksum: u32,
}

impl DatasetSplitV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.split_id == 0 {
            return Err(CoveError::BadSection(
                "DatasetSplitV1 split_id must be non-zero".into(),
            ));
        }
        if self.first_sample_ref != 0 {
            if self.sample_count == 0 {
                return Err(CoveError::BadSection(format!(
                    "DatasetSplit {} has first_sample_ref with zero sample_count",
                    self.split_id
                )));
            }
            self.first_sample_ref
                .checked_add(self.sample_count)
                .ok_or(CoveError::ArithOverflow)?;
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        validate_ai_split_method(self.split_method, "DatasetSplit split_method")?;
        if self.split_name_ref != 0 && !string_ref_ids.contains(&self.split_name_ref) {
            return Err(CoveError::BadSection(format!(
                "DatasetSplit {} references missing split_name_ref {}",
                self.split_id, self.split_name_ref
            )));
        }
        for (label, payload_ref) in [
            ("source_snapshot_ref", self.source_snapshot_ref),
            ("hash_function_ref", self.hash_function_ref),
            ("stratification_path_ref", self.stratification_path_ref),
            ("grouping_ref", self.grouping_ref),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "DatasetSplit {} references missing {label} {}",
                    self.split_id, payload_ref
                )));
            }
        }
        for (label, policy_ref) in [
            ("filter_policy_ref", self.filter_policy_ref),
            ("ordering_policy_ref", self.ordering_policy_ref),
            ("dedup_policy_ref", self.dedup_policy_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "DatasetSplit {} references missing {label} {}",
                    self.split_id, policy_ref
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DedupGroupV1 {
    pub dedup_group_id: u64,
    pub dedup_policy_ref: u32,
    pub canonical_member_sample_id: u64,
    pub similarity_kind: u8,
    pub dedup_authority: u8,
    pub confidence_ppm: u32,
    pub first_member_ref: u32,
    pub member_count: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl DedupGroupV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.dedup_group_id == 0 {
            return Err(CoveError::BadSection(
                "DedupGroupV1 dedup_group_id must be non-zero".into(),
            ));
        }
        if self.confidence_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "DedupGroup {} confidence_ppm exceeds 1_000_000",
                self.dedup_group_id
            )));
        }
        if (self.first_member_ref == 0) != (self.member_count == 0) {
            return Err(CoveError::BadSection(format!(
                "DedupGroup {} first_member_ref/member_count mismatch",
                self.dedup_group_id
            )));
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        policy_ref_ids: &BTreeSet<u32>,
        training_sample_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        validate_ai_similarity_kind(self.similarity_kind, "DedupGroup similarity_kind")?;
        validate_ai_dedup_authority(self.dedup_authority, "DedupGroup dedup_authority")?;
        if self.dedup_policy_ref != 0 && !policy_ref_ids.contains(&self.dedup_policy_ref) {
            return Err(CoveError::BadSection(format!(
                "DedupGroup {} references missing dedup_policy_ref {}",
                self.dedup_group_id, self.dedup_policy_ref
            )));
        }
        if self.canonical_member_sample_id != 0
            && !training_sample_ids.contains(&self.canonical_member_sample_id)
        {
            return Err(CoveError::BadSection(format!(
                "DedupGroup {} references missing canonical_member_sample_id {}",
                self.dedup_group_id, self.canonical_member_sample_id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingEpochPlanV1 {
    pub epoch_plan_id: u64,
    pub training_profile_id: u32,
    pub split_ref: u32,
    pub seed: u64,
    pub permutation_kind: u8,
    pub rng_algorithm_ref: u32,
    pub permutation_function_ref: u32,
    pub shard_count: u32,
    pub first_shard_ref: u32,
    pub shard_ref_count: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl TrainingEpochPlanV1 {
    pub(super) fn validate(
        &self,
        training_profile_ids: &BTreeSet<u32>,
        split_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.epoch_plan_id == 0 {
            return Err(CoveError::BadSection(
                "TrainingEpochPlanV1 epoch_plan_id must be non-zero".into(),
            ));
        }
        if !training_profile_ids.contains(&self.training_profile_id) {
            return Err(CoveError::BadSection(format!(
                "TrainingEpochPlan {} references missing training_profile_id {}",
                self.epoch_plan_id, self.training_profile_id
            )));
        }
        if self.split_ref != 0 && !split_ids.contains(&self.split_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingEpochPlan {} references missing split_ref {}",
                self.epoch_plan_id, self.split_ref
            )));
        }
        validate_ai_permutation_kind(self.permutation_kind, "TrainingEpochPlan permutation_kind")?;
        for (label, payload_ref) in [
            ("rng_algorithm_ref", self.rng_algorithm_ref),
            ("permutation_function_ref", self.permutation_function_ref),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "TrainingEpochPlan {} references missing {label} {}",
                    self.epoch_plan_id, payload_ref
                )));
            }
        }
        if (self.first_shard_ref == 0) != (self.shard_ref_count == 0) {
            return Err(CoveError::BadSection(format!(
                "TrainingEpochPlan {} first_shard_ref/shard_ref_count mismatch",
                self.epoch_plan_id
            )));
        }
        if self.shard_count == 0 && self.shard_ref_count != 0 {
            return Err(CoveError::BadSection(format!(
                "TrainingEpochPlan {} has shard refs with zero shard_count",
                self.epoch_plan_id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingLabelEntryV1 {
    pub label_id: u64,
    pub label_kind: u8,
    pub label_authority: u8,
    pub label_payload_ref: u32,
    pub generator_provenance_ref: u64,
    pub human_review_ref: u32,
    pub confidence_ppm: u32,
    pub evidence_ref: u32,
    pub policy_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl TrainingLabelEntryV1 {
    pub(super) fn validate(
        &self,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
        generator_provenance_ids: &BTreeSet<u64>,
        human_review_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.label_id == 0 {
            return Err(CoveError::BadSection(
                "TrainingLabelEntryV1 label_id must be non-zero".into(),
            ));
        }
        if !(self.label_kind <= 6 || self.label_kind == 255) {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} has unsupported label_kind {}",
                self.label_id, self.label_kind
            )));
        }
        if !(self.label_authority <= 6 || self.label_authority == 255) {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} has unsupported label_authority {}",
                self.label_id, self.label_authority
            )));
        }
        if self.confidence_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} confidence_ppm exceeds 1_000_000",
                self.label_id
            )));
        }
        if self.label_payload_ref != 0 && !payload_ref_ids.contains(&self.label_payload_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} references missing label_payload_ref {}",
                self.label_id, self.label_payload_ref
            )));
        }
        if self.evidence_ref != 0 && !payload_ref_ids.contains(&self.evidence_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} references missing evidence_ref {}",
                self.label_id, self.evidence_ref
            )));
        }
        if self.policy_ref != 0 && !policy_ref_ids.contains(&self.policy_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} references missing policy_ref {}",
                self.label_id, self.policy_ref
            )));
        }
        if self.generator_provenance_ref != 0
            && !generator_provenance_ids.contains(&self.generator_provenance_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} references missing generator_provenance_ref {}",
                self.label_id, self.generator_provenance_ref
            )));
        }
        if self.human_review_ref != 0 && !human_review_ids.contains(&self.human_review_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingLabel {} references missing human_review_ref {}",
                self.label_id, self.human_review_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferencePairEntryV1 {
    pub preference_pair_id: u64,
    pub prompt_ref: u32,
    pub chosen_ref: u32,
    pub rejected_ref: u32,
    pub judge_generator_provenance_ref: u64,
    pub human_review_ref: u32,
    pub preference_strength_ppm: u32,
    pub confidence_ppm: u32,
    pub evidence_ref: u32,
    pub policy_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl PreferencePairEntryV1 {
    pub(super) fn validate(
        &self,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
        generator_provenance_ids: &BTreeSet<u64>,
        human_review_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.preference_pair_id == 0 {
            return Err(CoveError::BadSection(
                "PreferencePairEntryV1 preference_pair_id must be non-zero".into(),
            ));
        }
        if self.preference_strength_ppm > 1_000_000 || self.confidence_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "PreferencePair {} ppm field exceeds 1_000_000",
                self.preference_pair_id
            )));
        }
        for (label, payload_ref) in [
            ("prompt_ref", self.prompt_ref),
            ("chosen_ref", self.chosen_ref),
            ("rejected_ref", self.rejected_ref),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "PreferencePair {} references missing {label} {}",
                    self.preference_pair_id, payload_ref
                )));
            }
        }
        if self.evidence_ref != 0 && !payload_ref_ids.contains(&self.evidence_ref) {
            return Err(CoveError::BadSection(format!(
                "PreferencePair {} references missing evidence_ref {}",
                self.preference_pair_id, self.evidence_ref
            )));
        }
        if self.policy_ref != 0 && !policy_ref_ids.contains(&self.policy_ref) {
            return Err(CoveError::BadSection(format!(
                "PreferencePair {} references missing policy_ref {}",
                self.preference_pair_id, self.policy_ref
            )));
        }
        if self.judge_generator_provenance_ref != 0
            && !generator_provenance_ids.contains(&self.judge_generator_provenance_ref)
        {
            return Err(CoveError::BadSection(format!(
                "PreferencePair {} references missing judge_generator_provenance_ref {}",
                self.preference_pair_id, self.judge_generator_provenance_ref
            )));
        }
        if self.human_review_ref != 0 && !human_review_ids.contains(&self.human_review_ref) {
            return Err(CoveError::BadSection(format!(
                "PreferencePair {} references missing human_review_ref {}",
                self.preference_pair_id, self.human_review_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratorProvenanceV1 {
    pub generator_provenance_id: u64,
    pub generator_kind: u8,
    pub model_actor_ref: u32,
    pub prompt_template_ref: u32,
    pub decoding_profile_ref: u32,
    pub toolchain_ref: u32,
    pub source_input_ref: u32,
    pub source_context_ref: u32,
    pub source_sample_ref: u64,
    pub parent_generator_provenance_ref: u64,
    pub generation_time_us: i64,
    pub confidence_ppm: u32,
    pub human_review_ref: u32,
    pub policy_ref: u32,
    pub reproducibility_class: u8,
    pub flags: u32,
    pub checksum: u32,
}

pub(super) struct GeneratorProvenanceValidateRefs<'a> {
    pub(super) generator_provenance_ids: &'a BTreeSet<u64>,
    pub(super) model_actor_ids: &'a BTreeSet<u32>,
    pub(super) decoding_profile_ids: &'a BTreeSet<u32>,
    pub(super) human_review_ids: &'a BTreeSet<u32>,
    pub(super) payload_ref_ids: &'a BTreeSet<u32>,
    pub(super) policy_ref_ids: &'a BTreeSet<u32>,
    pub(super) training_sample_ids: &'a BTreeSet<u64>,
}

impl GeneratorProvenanceV1 {
    pub(super) fn validate(
        &self,
        refs: GeneratorProvenanceValidateRefs<'_>,
    ) -> Result<(), CoveError> {
        let GeneratorProvenanceValidateRefs {
            generator_provenance_ids,
            model_actor_ids,
            decoding_profile_ids,
            human_review_ids,
            payload_ref_ids,
            policy_ref_ids,
            training_sample_ids,
        } = refs;
        if self.generator_provenance_id == 0 {
            return Err(CoveError::BadSection(
                "GeneratorProvenanceV1 generator_provenance_id must be non-zero".into(),
            ));
        }
        validate_ai_generator_kind(self.generator_kind, "GeneratorProvenance generator_kind")?;
        validate_ai_reproducibility_class(
            self.reproducibility_class,
            "GeneratorProvenance reproducibility_class",
        )?;
        if self.confidence_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} confidence_ppm exceeds 1_000_000",
                self.generator_provenance_id
            )));
        }
        if self.generator_kind == 1 && self.model_actor_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} model generation requires model_actor_ref",
                self.generator_provenance_id
            )));
        }
        if self.model_actor_ref != 0 && !model_actor_ids.contains(&self.model_actor_ref) {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} references missing model_actor_ref {}",
                self.generator_provenance_id, self.model_actor_ref
            )));
        }
        if self.decoding_profile_ref != 0
            && !decoding_profile_ids.contains(&self.decoding_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} references missing decoding_profile_ref {}",
                self.generator_provenance_id, self.decoding_profile_ref
            )));
        }
        if self.human_review_ref != 0 && !human_review_ids.contains(&self.human_review_ref) {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} references missing human_review_ref {}",
                self.generator_provenance_id, self.human_review_ref
            )));
        }
        for (label, payload_ref) in [
            ("prompt_template_ref", self.prompt_template_ref),
            ("toolchain_ref", self.toolchain_ref),
            ("source_input_ref", self.source_input_ref),
            ("source_context_ref", self.source_context_ref),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "GeneratorProvenance {} references missing {label} {}",
                    self.generator_provenance_id, payload_ref
                )));
            }
        }
        if self.source_sample_ref != 0 && !training_sample_ids.contains(&self.source_sample_ref) {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} references missing source_sample_ref {}",
                self.generator_provenance_id, self.source_sample_ref
            )));
        }
        if self.policy_ref != 0 && !policy_ref_ids.contains(&self.policy_ref) {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} references missing policy_ref {}",
                self.generator_provenance_id, self.policy_ref
            )));
        }
        if self.parent_generator_provenance_ref == self.generator_provenance_id {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} parent reference is self-referential",
                self.generator_provenance_id
            )));
        }
        if self.parent_generator_provenance_ref != 0
            && !generator_provenance_ids.contains(&self.parent_generator_provenance_ref)
        {
            return Err(CoveError::BadSection(format!(
                "GeneratorProvenance {} references missing parent_generator_provenance_ref {}",
                self.generator_provenance_id, self.parent_generator_provenance_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelActorDescriptorV1 {
    pub model_actor_id: u32,
    pub model_namespace_ref: u32,
    pub model_name_ref: u32,
    pub model_version_ref: u32,
    pub model_checkpoint_digest_ref: u32,
    pub provider_ref: u32,
    pub endpoint_ref: u32,
    pub endpoint_version_ref: u32,
    pub model_family_ref: u32,
    pub modality_mask: u32,
    pub license_ref: u32,
    pub policy_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl ModelActorDescriptorV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.model_actor_id == 0 {
            return Err(CoveError::BadSection(
                "ModelActorDescriptorV1 model_actor_id must be non-zero".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        for (label, string_ref) in [
            ("model_namespace_ref", self.model_namespace_ref),
            ("model_name_ref", self.model_name_ref),
            ("model_version_ref", self.model_version_ref),
            ("provider_ref", self.provider_ref),
            ("endpoint_ref", self.endpoint_ref),
            ("endpoint_version_ref", self.endpoint_version_ref),
            ("model_family_ref", self.model_family_ref),
        ] {
            if string_ref != 0 && !string_ref_ids.contains(&string_ref) {
                return Err(CoveError::BadSection(format!(
                    "ModelActor {} references missing {label} {}",
                    self.model_actor_id, string_ref
                )));
            }
        }
        if self.model_checkpoint_digest_ref != 0
            && !digest_ref_ids.contains(&self.model_checkpoint_digest_ref)
        {
            return Err(CoveError::BadSection(format!(
                "ModelActor {} references missing model_checkpoint_digest_ref {}",
                self.model_actor_id, self.model_checkpoint_digest_ref
            )));
        }
        for (label, policy_ref) in [
            ("license_ref", self.license_ref),
            ("policy_ref", self.policy_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "ModelActor {} references missing {label} {}",
                    self.model_actor_id, policy_ref
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationDecodingProfileV1 {
    pub decoding_profile_id: u32,
    pub temperature_micros: u32,
    pub top_p_micros: u32,
    pub top_k: u32,
    pub seed: u64,
    pub max_output_tokens: u32,
    pub stop_sequence_ref: u32,
    pub safety_policy_ref: u32,
    pub deterministic_claim: u8,
    pub flags: u32,
    pub checksum: u32,
}

impl GenerationDecodingProfileV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.decoding_profile_id == 0 {
            return Err(CoveError::BadSection(
                "GenerationDecodingProfileV1 decoding_profile_id must be non-zero".into(),
            ));
        }
        if self.top_p_micros > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "GenerationDecodingProfile {} top_p_micros exceeds 1_000_000",
                self.decoding_profile_id
            )));
        }
        validate_bool_byte(
            self.deterministic_claim,
            "GenerationDecodingProfile deterministic_claim",
        )?;
        Ok(())
    }

    pub(super) fn validate(
        &self,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        if self.stop_sequence_ref != 0 && !payload_ref_ids.contains(&self.stop_sequence_ref) {
            return Err(CoveError::BadSection(format!(
                "GenerationDecodingProfile {} references missing stop_sequence_ref {}",
                self.decoding_profile_id, self.stop_sequence_ref
            )));
        }
        if self.safety_policy_ref != 0 && !policy_ref_ids.contains(&self.safety_policy_ref) {
            return Err(CoveError::BadSection(format!(
                "GenerationDecodingProfile {} references missing safety_policy_ref {}",
                self.decoding_profile_id, self.safety_policy_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanReviewEntryV1 {
    pub human_review_id: u32,
    pub review_kind: u8,
    pub reviewer_role_ref: u32,
    pub review_time_us: i64,
    pub rating_ppm: u32,
    pub notes_ref: u32,
    pub policy_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl HumanReviewEntryV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.human_review_id == 0 {
            return Err(CoveError::BadSection(
                "HumanReviewEntryV1 human_review_id must be non-zero".into(),
            ));
        }
        if self.rating_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "HumanReview {} rating_ppm exceeds 1_000_000",
                self.human_review_id
            )));
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        validate_ai_review_kind(self.review_kind, "HumanReview review_kind")?;
        if self.reviewer_role_ref != 0 && !string_ref_ids.contains(&self.reviewer_role_ref) {
            return Err(CoveError::BadSection(format!(
                "HumanReview {} references missing reviewer_role_ref {}",
                self.human_review_id, self.reviewer_role_ref
            )));
        }
        if self.notes_ref != 0 && !payload_ref_ids.contains(&self.notes_ref) {
            return Err(CoveError::BadSection(format!(
                "HumanReview {} references missing notes_ref {}",
                self.human_review_id, self.notes_ref
            )));
        }
        if self.policy_ref != 0 && !policy_ref_ids.contains(&self.policy_ref) {
            return Err(CoveError::BadSection(format!(
                "HumanReview {} references missing policy_ref {}",
                self.human_review_id, self.policy_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorLayoutDescriptorV1 {
    pub tensor_layout_id: u32,
    pub layout_name_ref: u32,
    pub rank: u8,
    pub dtype: u8,
    pub byte_order: u8,
    pub shape_ref: u32,
    pub stride_ref: u32,
    pub storage_offset_elements: i64,
    pub layout_kind: u8,
    pub memory_alignment_bytes: u32,
    pub preferred_page_alignment_bytes: u32,
    pub tile_shape_ref: u32,
    pub block_shape_ref: u32,
    pub quantization_profile_ref: u32,
    pub sparsity_profile_ref: u32,
    pub framework_compatibility_ref: u32,
    pub device_affinity_hint: u8,
    pub flags: u32,
    pub checksum: u32,
}

impl TensorLayoutDescriptorV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.tensor_layout_id == 0 {
            return Err(CoveError::BadSection(
                "TensorLayoutDescriptorV1 tensor_layout_id must be non-zero".into(),
            ));
        }
        if self.rank == 0 {
            return Err(CoveError::BadSection(format!(
                "TensorLayout {} requires non-zero rank",
                self.tensor_layout_id
            )));
        }
        validate_power_of_two_alignment(
            self.memory_alignment_bytes,
            "TensorLayout memory_alignment_bytes",
        )?;
        validate_power_of_two_alignment(
            self.preferred_page_alignment_bytes,
            "TensorLayout preferred_page_alignment_bytes",
        )?;
        Ok(())
    }

    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_static()?;
        validate_ai_tensor_dtype(self.dtype, "TensorLayout dtype")?;
        validate_ai_byte_order(self.byte_order, "TensorLayout byte_order")?;
        validate_ai_layout_kind(self.layout_kind, "TensorLayout layout_kind")?;
        validate_ai_device_kind(
            self.device_affinity_hint,
            "TensorLayout device_affinity_hint",
        )?;
        if self.storage_offset_elements < 0 {
            return Err(CoveError::BadSection(format!(
                "TensorLayout {} storage_offset_elements must be non-negative",
                self.tensor_layout_id
            )));
        }
        if self.layout_name_ref != 0 && !string_ref_ids.contains(&self.layout_name_ref) {
            return Err(CoveError::BadSection(format!(
                "TensorLayout {} references missing layout_name_ref {}",
                self.tensor_layout_id, self.layout_name_ref
            )));
        }
        for (label, payload_ref) in [
            ("shape_ref", self.shape_ref),
            ("stride_ref", self.stride_ref),
            ("tile_shape_ref", self.tile_shape_ref),
            ("block_shape_ref", self.block_shape_ref),
            ("quantization_profile_ref", self.quantization_profile_ref),
            ("sparsity_profile_ref", self.sparsity_profile_ref),
            (
                "framework_compatibility_ref",
                self.framework_compatibility_ref,
            ),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "TensorLayout {} references missing {label} {}",
                    self.tensor_layout_id, payload_ref
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTransferHintV1 {
    pub transfer_hint_id: u32,
    pub target_kind: u8,
    pub preferred_alignment_bytes: u32,
    pub preferred_chunk_bytes: u32,
    pub pinned_memory_required: u8,
    pub contiguous_required: u8,
    pub zero_copy_possible: u8,
    pub runtime_registry_binding_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl DeviceTransferHintV1 {
    pub(super) fn validate_static(&self) -> Result<(), CoveError> {
        if self.transfer_hint_id == 0 {
            return Err(CoveError::BadSection(
                "DeviceTransferHintV1 transfer_hint_id must be non-zero".into(),
            ));
        }
        validate_power_of_two_alignment(
            self.preferred_alignment_bytes,
            "DeviceTransferHint preferred_alignment_bytes",
        )?;
        validate_bool_byte(
            self.pinned_memory_required,
            "DeviceTransferHint pinned_memory_required",
        )?;
        validate_bool_byte(
            self.contiguous_required,
            "DeviceTransferHint contiguous_required",
        )?;
        validate_bool_byte(
            self.zero_copy_possible,
            "DeviceTransferHint zero_copy_possible",
        )?;
        Ok(())
    }

    pub(super) fn validate(&self, string_ref_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
        self.validate_static()?;
        validate_ai_device_kind(self.target_kind, "DeviceTransferHint target_kind")?;
        if self.zero_copy_possible == 1 && self.preferred_alignment_bytes == 0 {
            return Err(CoveError::BadSection(format!(
                "DeviceTransferHint {} zero_copy_possible requires preferred_alignment_bytes",
                self.transfer_hint_id
            )));
        }
        if self.runtime_registry_binding_ref != 0
            && !string_ref_ids.contains(&self.runtime_registry_binding_ref)
        {
            return Err(CoveError::BadSection(format!(
                "DeviceTransferHint {} references missing runtime_registry_binding_ref {}",
                self.transfer_hint_id, self.runtime_registry_binding_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiAssetRefV1 {
    pub asset_ref_id: u64,
    pub parent_asset_ref: u64,
    pub asset_kind: u8,
    pub uri_ref: u32,
    pub embedded_section_ref: u32,
    pub media_type_ref: u32,
    pub byte_length: u64,
    pub digest_ref: u32,
    pub width: u32,
    pub height: u32,
    pub duration_us: u64,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub decode_profile_ref: u32,
    pub preprocessing_profile_ref: u32,
    pub transform_profile_ref: u32,
    pub transform_digest_ref: u32,
    pub tensor_layout_ref: u32,
    pub license_ref: u32,
    pub policy_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

pub(super) struct AiAssetRefValidateRefs<'a> {
    pub(super) asset_ref_ids: &'a BTreeSet<u64>,
    pub(super) tensor_layout_ids: &'a BTreeSet<u32>,
    pub(super) section_ids: &'a BTreeSet<u32>,
    pub(super) string_ref_ids: &'a BTreeSet<u32>,
    pub(super) digest_ref_ids: &'a BTreeSet<u32>,
    pub(super) transform_ids: &'a BTreeSet<u32>,
    pub(super) policy_ref_ids: &'a BTreeSet<u32>,
}

impl AiAssetRefV1 {
    pub(super) fn validate(&self, refs: AiAssetRefValidateRefs<'_>) -> Result<(), CoveError> {
        let AiAssetRefValidateRefs {
            asset_ref_ids,
            tensor_layout_ids,
            section_ids,
            string_ref_ids,
            digest_ref_ids,
            transform_ids,
            policy_ref_ids,
        } = refs;
        if self.asset_ref_id == 0 {
            return Err(CoveError::BadSection(
                "AiAssetRefV1 asset_ref_id must be non-zero".into(),
            ));
        }
        validate_ai_asset_kind(self.asset_kind, "AiAssetRef asset_kind")?;
        if self.asset_kind == 0 && self.uri_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "AiAssetRef {} URI asset requires uri_ref",
                self.asset_ref_id
            )));
        }
        if self.asset_kind == 1 && self.embedded_section_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "AiAssetRef {} embedded asset requires embedded_section_ref",
                self.asset_ref_id
            )));
        }
        if self.parent_asset_ref == self.asset_ref_id {
            return Err(CoveError::BadSection(format!(
                "AiAssetRef {} parent_asset_ref is self-referential",
                self.asset_ref_id
            )));
        }
        if self.parent_asset_ref != 0 && !asset_ref_ids.contains(&self.parent_asset_ref) {
            return Err(CoveError::BadSection(format!(
                "AiAssetRef {} references missing parent_asset_ref {}",
                self.asset_ref_id, self.parent_asset_ref
            )));
        }
        if self.embedded_section_ref != 0 && !section_ids.contains(&self.embedded_section_ref) {
            return Err(CoveError::BadSection(format!(
                "AiAssetRef {} references missing embedded_section_ref {}",
                self.asset_ref_id, self.embedded_section_ref
            )));
        }
        if self.tensor_layout_ref != 0 && !tensor_layout_ids.contains(&self.tensor_layout_ref) {
            return Err(CoveError::BadSection(format!(
                "AiAssetRef {} references missing tensor_layout_ref {}",
                self.asset_ref_id, self.tensor_layout_ref
            )));
        }
        for (label, string_ref) in [
            ("uri_ref", self.uri_ref),
            ("media_type_ref", self.media_type_ref),
        ] {
            if string_ref != 0 && !string_ref_ids.contains(&string_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiAssetRef {} references missing {label} {}",
                    self.asset_ref_id, string_ref
                )));
            }
        }
        for (label, digest_ref) in [
            ("digest_ref", self.digest_ref),
            ("transform_digest_ref", self.transform_digest_ref),
        ] {
            if digest_ref != 0 && !digest_ref_ids.contains(&digest_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiAssetRef {} references missing {label} {}",
                    self.asset_ref_id, digest_ref
                )));
            }
        }
        for (label, transform_ref) in [
            ("decode_profile_ref", self.decode_profile_ref),
            ("preprocessing_profile_ref", self.preprocessing_profile_ref),
            ("transform_profile_ref", self.transform_profile_ref),
        ] {
            if transform_ref != 0 && !transform_ids.contains(&transform_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiAssetRef {} references missing {label} {}",
                    self.asset_ref_id, transform_ref
                )));
            }
        }
        for (label, policy_ref) in [
            ("license_ref", self.license_ref),
            ("policy_ref", self.policy_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "AiAssetRef {} references missing {label} {}",
                    self.asset_ref_id, policy_ref
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultimodalSequencePackV1 {
    pub sequence_pack_id: u64,
    pub training_profile_id: u32,
    pub tokenizer_profile_id: u32,
    pub sequence_profile_ref: u32,
    pub element_count: u32,
    pub first_element_ref: u32,
    pub split_ref: u32,
    pub sample_weight_ppm: u32,
    pub loss_mask_ref: u32,
    pub attention_mask_ref: u32,
    pub position_map_ref: u32,
    pub label_ref: u32,
    pub source_snapshot_ref: u32,
    pub evidence_ref: u32,
    pub generator_provenance_ref: u64,
    pub flags: u32,
    pub checksum: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultimodalSequenceElementV1 {
    pub element_id: u64,
    pub sequence_pack_id: u64,
    pub ordinal: u32,
    pub element_kind: u8,
    pub modality: u8,
    pub role: u8,
    pub tokenized_span_ref: u64,
    pub token_sequence_pack_ref: u64,
    pub asset_ref: u64,
    pub tensor_ref: u64,
    pub vector_ref: u64,
    pub byte_start: u64,
    pub byte_length: u64,
    pub time_start_us: i64,
    pub time_duration_us: i64,
    pub position_stream_ref: u32,
    pub evidence_ref: u32,
    pub policy_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorSpaceDescriptorV1 {
    pub vector_space_id: u32,
    pub vector_space_name_ref: u32,
    pub vector_space_fingerprint_ref: u32,
    pub embedding_namespace_ref: u32,
    pub embedding_model_ref: u32,
    pub embedding_model_version_ref: u32,
    pub embedding_model_digest_ref: u32,
    pub embedding_pipeline_ref: u32,
    pub tokenizer_profile_ref: u32,
    pub chunk_profile_ref: u32,
    pub dimension_count: u32,
    pub element_type: u8,
    pub metric: u8,
    pub normalization_policy: u8,
    pub quantization_policy: u8,
    pub deterministic: u8,
    pub approximate: u8,
    pub reproducibility_class: u8,
    pub reserved: u8,
    pub flags: u32,
    pub checksum: u32,
}

impl VectorSpaceDescriptorV1 {
    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        transform_ids: &BTreeSet<u32>,
        tokenizer_profile_ids: &BTreeSet<u32>,
        chunk_profile_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.vector_space_id == 0 {
            return Err(CoveError::BadSection(
                "VectorSpaceDescriptorV1 vector_space_id must be non-zero".into(),
            ));
        }
        if self.dimension_count == 0 {
            return Err(CoveError::BadSection(format!(
                "VectorSpace {} requires non-zero dimension_count",
                self.vector_space_id
            )));
        }
        validate_ai_vector_element_type(self.element_type, "VectorSpace element_type")?;
        validate_ai_vector_metric(self.metric, "VectorSpace metric")?;
        validate_ai_normalization_policy(
            self.normalization_policy,
            "VectorSpace normalization_policy",
        )?;
        validate_ai_quantization_kind(self.quantization_policy, "VectorSpace quantization_policy")?;
        validate_bool_byte(self.deterministic, "VectorSpace deterministic")?;
        validate_bool_byte(self.approximate, "VectorSpace approximate")?;
        validate_ai_reproducibility_class(
            self.reproducibility_class,
            "VectorSpace reproducibility_class",
        )?;
        if self.reserved != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        for (label, string_ref) in [
            ("vector_space_name_ref", self.vector_space_name_ref),
            ("embedding_namespace_ref", self.embedding_namespace_ref),
            ("embedding_model_ref", self.embedding_model_ref),
            (
                "embedding_model_version_ref",
                self.embedding_model_version_ref,
            ),
        ] {
            if string_ref != 0 && !string_ref_ids.contains(&string_ref) {
                return Err(CoveError::BadSection(format!(
                    "VectorSpace {} references missing {label} {}",
                    self.vector_space_id, string_ref
                )));
            }
        }
        for (label, digest_ref) in [
            (
                "vector_space_fingerprint_ref",
                self.vector_space_fingerprint_ref,
            ),
            (
                "embedding_model_digest_ref",
                self.embedding_model_digest_ref,
            ),
        ] {
            if digest_ref != 0 && !digest_ref_ids.contains(&digest_ref) {
                return Err(CoveError::BadSection(format!(
                    "VectorSpace {} references missing {label} {}",
                    self.vector_space_id, digest_ref
                )));
            }
        }
        if self.embedding_pipeline_ref != 0 && !transform_ids.contains(&self.embedding_pipeline_ref)
        {
            return Err(CoveError::BadSection(format!(
                "VectorSpace {} references missing embedding_pipeline_ref {}",
                self.vector_space_id, self.embedding_pipeline_ref
            )));
        }
        if self.tokenizer_profile_ref != 0
            && !tokenizer_profile_ids.contains(&self.tokenizer_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "VectorSpace {} references missing tokenizer_profile_ref {}",
                self.vector_space_id, self.tokenizer_profile_ref
            )));
        }
        if self.chunk_profile_ref != 0 && !chunk_profile_ids.contains(&self.chunk_profile_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorSpace {} references missing chunk_profile_ref {}",
                self.vector_space_id, self.chunk_profile_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorSpaceCompatibilityDescriptorV1 {
    pub compatibility_id: u32,
    pub left_vector_space_id: u32,
    pub right_vector_space_id: u32,
    pub compatibility_kind: u8,
    pub compatibility_authority: u8,
    pub metric: u8,
    pub normalization_policy: u8,
    pub transform_ref: u32,
    pub numeric_transform_error_ppm: u32,
    pub ranking_eval_ref: u32,
    pub calibration_dataset_ref: u32,
    pub evidence_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl VectorSpaceCompatibilityDescriptorV1 {
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        transform_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.compatibility_id == 0 {
            return Err(CoveError::BadSection(
                "VectorSpaceCompatibilityDescriptorV1 compatibility_id must be non-zero".into(),
            ));
        }
        for (label, vector_space_id) in [
            ("left_vector_space_id", self.left_vector_space_id),
            ("right_vector_space_id", self.right_vector_space_id),
        ] {
            if !vector_space_ids.contains(&vector_space_id) {
                return Err(CoveError::BadSection(format!(
                    "VectorSpaceCompatibility {} references missing {label} {}",
                    self.compatibility_id, vector_space_id
                )));
            }
        }
        validate_ai_vector_compatibility_kind(
            self.compatibility_kind,
            "VectorSpaceCompatibility compatibility_kind",
        )?;
        validate_ai_vector_compatibility_authority(
            self.compatibility_authority,
            "VectorSpaceCompatibility compatibility_authority",
        )?;
        validate_ai_vector_metric(self.metric, "VectorSpaceCompatibility metric")?;
        validate_ai_normalization_policy(
            self.normalization_policy,
            "VectorSpaceCompatibility normalization_policy",
        )?;
        if self.numeric_transform_error_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "VectorSpaceCompatibility {} numeric_transform_error_ppm exceeds 1_000_000",
                self.compatibility_id
            )));
        }
        if self.transform_ref != 0 && !transform_ids.contains(&self.transform_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorSpaceCompatibility {} references missing transform_ref {}",
                self.compatibility_id, self.transform_ref
            )));
        }
        for (label, payload_ref) in [
            ("ranking_eval_ref", self.ranking_eval_ref),
            ("calibration_dataset_ref", self.calibration_dataset_ref),
            ("evidence_ref", self.evidence_ref),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "VectorSpaceCompatibility {} references missing {label} {}",
                    self.compatibility_id, payload_ref
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCodeVectorBindingV1 {
    pub binding_id: u64,
    pub vector_space_id: u32,
    pub slot_policy_ref: u32,
    pub file_ref: u32,
    pub dictionary_digest_ref: u32,
    pub schema_fingerprint_ref: u32,
    pub table_id: u32,
    pub column_id: u32,
    pub object_type_id: u32,
    pub property_id: u32,
    pub association_type_id: u32,
    pub path_ref: u32,
    pub file_code: u32,
    pub reserved0: u32,
    pub canonical_value_hash_ref: u32,
    pub vector_ref: u64,
    pub model_input_digest_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl FileCodeVectorBindingV1 {
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        model_input_digest_refs: &BTreeSet<u32>,
        string_ref_ids: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        if self.binding_id == 0 {
            return Err(CoveError::BadSection(
                "FileCodeVectorBindingV1 binding_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_space_id {}",
                self.binding_id, self.vector_space_id
            )));
        }
        if self.file_ref != 0 && !source_binding_ids.contains(&self.file_ref) {
            return Err(CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing file_ref {}",
                self.binding_id, self.file_ref
            )));
        }
        for (label, digest_ref) in [
            ("dictionary_digest_ref", self.dictionary_digest_ref),
            ("schema_fingerprint_ref", self.schema_fingerprint_ref),
            ("canonical_value_hash_ref", self.canonical_value_hash_ref),
        ] {
            if digest_ref != 0 && !digest_ref_ids.contains(&digest_ref) {
                return Err(CoveError::BadSection(format!(
                    "FileCodeVectorBinding {} references missing {label} {}",
                    self.binding_id, digest_ref
                )));
            }
        }
        if self.path_ref != 0 && !string_ref_ids.contains(&self.path_ref) {
            return Err(CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing path_ref {}",
                self.binding_id, self.path_ref
            )));
        }
        if self.reserved0 != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        validate_model_input_digest_ref(
            "FileCodeVectorBinding",
            self.binding_id,
            self.model_input_digest_ref,
            digest_ref_ids,
            model_input_digest_refs,
        )?;
        if !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_ref {}",
                self.binding_id, self.vector_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkVectorBindingV1 {
    pub binding_id: u64,
    pub vector_space_id: u32,
    pub chunk_id: u64,
    pub chunk_profile_id: u32,
    pub source_value_hash_ref: u32,
    pub chunk_text_hash_ref: u32,
    pub vector_ref: u64,
    pub model_input_digest_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl ChunkVectorBindingV1 {
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        chunk_ids: &BTreeSet<u64>,
        chunk_profile_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        model_input_digest_refs: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        if self.binding_id == 0 {
            return Err(CoveError::BadSection(
                "ChunkVectorBindingV1 binding_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "ChunkVectorBinding {} references missing vector_space_id {}",
                self.binding_id, self.vector_space_id
            )));
        }
        if self.chunk_id == 0 || !chunk_ids.contains(&self.chunk_id) {
            return Err(CoveError::BadSection(format!(
                "ChunkVectorBinding {} references missing chunk_id {}",
                self.binding_id, self.chunk_id
            )));
        }
        if self.chunk_profile_id != 0 && !chunk_profile_ids.contains(&self.chunk_profile_id) {
            return Err(CoveError::BadSection(format!(
                "ChunkVectorBinding {} references missing chunk_profile_id {}",
                self.binding_id, self.chunk_profile_id
            )));
        }
        for (label, digest_ref) in [
            ("source_value_hash_ref", self.source_value_hash_ref),
            ("chunk_text_hash_ref", self.chunk_text_hash_ref),
        ] {
            if digest_ref != 0 && !digest_ref_ids.contains(&digest_ref) {
                return Err(CoveError::BadSection(format!(
                    "ChunkVectorBinding {} references missing {label} {}",
                    self.binding_id, digest_ref
                )));
            }
        }
        validate_model_input_digest_ref(
            "ChunkVectorBinding",
            self.binding_id,
            self.model_input_digest_ref,
            digest_ref_ids,
            model_input_digest_refs,
        )?;
        if !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "ChunkVectorBinding {} references missing vector_ref {}",
                self.binding_id, self.vector_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectStateVectorBindingV1 {
    pub binding_id: u64,
    pub vector_space_id: u32,
    pub composition_profile_ref: u32,
    pub file_ref: u32,
    pub object_type_id: u32,
    pub goid_ref: u32,
    pub branch_ref: u32,
    pub temporal_kind: u8,
    pub csn: u64,
    pub timestamp_us: i64,
    pub property_dependency_fingerprint_ref: u32,
    pub vector_ref: u64,
    pub model_input_digest_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl ObjectStateVectorBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        composition_profile_ids: &BTreeSet<u32>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        model_input_digest_refs: &BTreeSet<u32>,
        string_ref_ids: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        if self.binding_id == 0 {
            return Err(CoveError::BadSection(
                "ObjectStateVectorBindingV1 binding_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "ObjectStateVectorBinding {} references missing vector_space_id {}",
                self.binding_id, self.vector_space_id
            )));
        }
        if self.composition_profile_ref != 0
            && !composition_profile_ids.contains(&self.composition_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "ObjectStateVectorBinding {} references missing composition_profile_ref {}",
                self.binding_id, self.composition_profile_ref
            )));
        }
        if self.file_ref != 0 && !source_binding_ids.contains(&self.file_ref) {
            return Err(CoveError::BadSection(format!(
                "ObjectStateVectorBinding {} references missing file_ref {}",
                self.binding_id, self.file_ref
            )));
        }
        if self.branch_ref != 0 && !string_ref_ids.contains(&self.branch_ref) {
            return Err(CoveError::BadSection(format!(
                "ObjectStateVectorBinding {} references missing branch_ref {}",
                self.binding_id, self.branch_ref
            )));
        }
        if self.property_dependency_fingerprint_ref != 0
            && !digest_ref_ids.contains(&self.property_dependency_fingerprint_ref)
        {
            return Err(CoveError::BadSection(format!(
                "ObjectStateVectorBinding {} references missing property_dependency_fingerprint_ref {}",
                self.binding_id, self.property_dependency_fingerprint_ref
            )));
        }
        validate_model_input_digest_ref(
            "ObjectStateVectorBinding",
            self.binding_id,
            self.model_input_digest_ref,
            digest_ref_ids,
            model_input_digest_refs,
        )?;
        if !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "ObjectStateVectorBinding {} references missing vector_ref {}",
                self.binding_id, self.vector_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingSampleVectorBindingV1 {
    pub binding_id: u64,
    pub vector_space_id: u32,
    pub training_profile_ref: u32,
    pub sample_id: u64,
    pub source_snapshot_ref: u32,
    pub sample_fingerprint_ref: u32,
    pub vector_ref: u64,
    pub model_input_digest_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl TrainingSampleVectorBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        training_profile_ids: &BTreeSet<u32>,
        training_sample_ids: &BTreeSet<u64>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        model_input_digest_refs: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        if self.binding_id == 0 {
            return Err(CoveError::BadSection(
                "TrainingSampleVectorBindingV1 binding_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "TrainingSampleVectorBinding {} references missing vector_space_id {}",
                self.binding_id, self.vector_space_id
            )));
        }
        if self.training_profile_ref != 0
            && !training_profile_ids.contains(&self.training_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingSampleVectorBinding {} references missing training_profile_ref {}",
                self.binding_id, self.training_profile_ref
            )));
        }
        if self.sample_id == 0 || !training_sample_ids.contains(&self.sample_id) {
            return Err(CoveError::BadSection(format!(
                "TrainingSampleVectorBinding {} references missing sample_id {}",
                self.binding_id, self.sample_id
            )));
        }
        if self.source_snapshot_ref != 0 && !source_binding_ids.contains(&self.source_snapshot_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingSampleVectorBinding {} references missing source_snapshot_ref {}",
                self.binding_id, self.source_snapshot_ref
            )));
        }
        if self.sample_fingerprint_ref != 0
            && !digest_ref_ids.contains(&self.sample_fingerprint_ref)
        {
            return Err(CoveError::BadSection(format!(
                "TrainingSampleVectorBinding {} references missing sample_fingerprint_ref {}",
                self.binding_id, self.sample_fingerprint_ref
            )));
        }
        validate_model_input_digest_ref(
            "TrainingSampleVectorBinding",
            self.binding_id,
            self.model_input_digest_ref,
            digest_ref_ids,
            model_input_digest_refs,
        )?;
        if !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "TrainingSampleVectorBinding {} references missing vector_ref {}",
                self.binding_id, self.vector_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociationStateVectorBindingV1 {
    pub binding_id: u64,
    pub vector_space_id: u32,
    pub composition_profile_ref: u32,
    pub file_ref: u32,
    pub association_type_id: u32,
    pub association_key_ref: u32,
    pub branch_ref: u32,
    pub temporal_kind: u8,
    pub csn: u64,
    pub timestamp_us: i64,
    pub property_dependency_fingerprint_ref: u32,
    pub vector_ref: u64,
    pub model_input_digest_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl AssociationStateVectorBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        composition_profile_ids: &BTreeSet<u32>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        model_input_digest_refs: &BTreeSet<u32>,
        string_ref_ids: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        if self.binding_id == 0 {
            return Err(CoveError::BadSection(
                "AssociationStateVectorBindingV1 binding_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "AssociationStateVectorBinding {} references missing vector_space_id {}",
                self.binding_id, self.vector_space_id
            )));
        }
        if self.composition_profile_ref != 0
            && !composition_profile_ids.contains(&self.composition_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "AssociationStateVectorBinding {} references missing composition_profile_ref {}",
                self.binding_id, self.composition_profile_ref
            )));
        }
        if self.file_ref != 0 && !source_binding_ids.contains(&self.file_ref) {
            return Err(CoveError::BadSection(format!(
                "AssociationStateVectorBinding {} references missing file_ref {}",
                self.binding_id, self.file_ref
            )));
        }
        for (label, string_ref) in [
            ("association_key_ref", self.association_key_ref),
            ("branch_ref", self.branch_ref),
        ] {
            if string_ref != 0 && !string_ref_ids.contains(&string_ref) {
                return Err(CoveError::BadSection(format!(
                    "AssociationStateVectorBinding {} references missing {label} {}",
                    self.binding_id, string_ref
                )));
            }
        }
        if self.property_dependency_fingerprint_ref != 0
            && !digest_ref_ids.contains(&self.property_dependency_fingerprint_ref)
        {
            return Err(CoveError::BadSection(format!(
                "AssociationStateVectorBinding {} references missing property_dependency_fingerprint_ref {}",
                self.binding_id, self.property_dependency_fingerprint_ref
            )));
        }
        validate_model_input_digest_ref(
            "AssociationStateVectorBinding",
            self.binding_id,
            self.model_input_digest_ref,
            digest_ref_ids,
            model_input_digest_refs,
        )?;
        if !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "AssociationStateVectorBinding {} references missing vector_ref {}",
                self.binding_id, self.vector_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetVectorBindingV1 {
    pub binding_id: u64,
    pub vector_space_id: u32,
    pub asset_ref: u64,
    pub transform_ref: u32,
    pub asset_digest_ref: u32,
    pub vector_ref: u64,
    pub model_input_digest_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl AssetVectorBindingV1 {
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        asset_ref_ids: &BTreeSet<u64>,
        transform_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        model_input_digest_refs: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        if self.binding_id == 0 {
            return Err(CoveError::BadSection(
                "AssetVectorBindingV1 binding_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "AssetVectorBinding {} references missing vector_space_id {}",
                self.binding_id, self.vector_space_id
            )));
        }
        if self.asset_ref == 0 || !asset_ref_ids.contains(&self.asset_ref) {
            return Err(CoveError::BadSection(format!(
                "AssetVectorBinding {} references missing asset_ref {}",
                self.binding_id, self.asset_ref
            )));
        }
        if self.transform_ref != 0 && !transform_ids.contains(&self.transform_ref) {
            return Err(CoveError::BadSection(format!(
                "AssetVectorBinding {} references missing transform_ref {}",
                self.binding_id, self.transform_ref
            )));
        }
        if self.asset_digest_ref != 0 && !digest_ref_ids.contains(&self.asset_digest_ref) {
            return Err(CoveError::BadSection(format!(
                "AssetVectorBinding {} references missing asset_digest_ref {}",
                self.binding_id, self.asset_digest_ref
            )));
        }
        validate_model_input_digest_ref(
            "AssetVectorBinding",
            self.binding_id,
            self.model_input_digest_ref,
            digest_ref_ids,
            model_input_digest_refs,
        )?;
        if !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "AssetVectorBinding {} references missing vector_ref {}",
                self.binding_id, self.vector_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultimodalSequenceVectorBindingV1 {
    pub binding_id: u64,
    pub vector_space_id: u32,
    pub sequence_pack_id: u64,
    pub sequence_profile_ref: u32,
    pub source_snapshot_ref: u32,
    pub vector_ref: u64,
    pub model_input_digest_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl MultimodalSequenceVectorBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        multimodal_sequence_pack_ids: &BTreeSet<u64>,
        source_binding_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        model_input_digest_refs: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        if self.binding_id == 0 {
            return Err(CoveError::BadSection(
                "MultimodalSequenceVectorBindingV1 binding_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "MultimodalSequenceVectorBinding {} references missing vector_space_id {}",
                self.binding_id, self.vector_space_id
            )));
        }
        if self.sequence_pack_id == 0
            || !multimodal_sequence_pack_ids.contains(&self.sequence_pack_id)
        {
            return Err(CoveError::BadSection(format!(
                "MultimodalSequenceVectorBinding {} references missing sequence_pack_id {}",
                self.binding_id, self.sequence_pack_id
            )));
        }
        if self.sequence_profile_ref != 0 && !payload_ref_ids.contains(&self.sequence_profile_ref) {
            return Err(CoveError::BadSection(format!(
                "MultimodalSequenceVectorBinding {} references missing sequence_profile_ref {}",
                self.binding_id, self.sequence_profile_ref
            )));
        }
        if self.source_snapshot_ref != 0 && !source_binding_ids.contains(&self.source_snapshot_ref)
        {
            return Err(CoveError::BadSection(format!(
                "MultimodalSequenceVectorBinding {} references missing source_snapshot_ref {}",
                self.binding_id, self.source_snapshot_ref
            )));
        }
        validate_model_input_digest_ref(
            "MultimodalSequenceVectorBinding",
            self.binding_id,
            self.model_input_digest_ref,
            digest_ref_ids,
            model_input_digest_refs,
        )?;
        if !vector_ref_ids.contains(&self.vector_ref) {
            return Err(CoveError::BadSection(format!(
                "MultimodalSequenceVectorBinding {} references missing vector_ref {}",
                self.binding_id, self.vector_ref
            )));
        }
        Ok(())
    }
}

fn validate_model_input_digest_ref(
    label: &str,
    binding_id: u64,
    model_input_digest_ref: u32,
    digest_ref_ids: &BTreeSet<u32>,
    model_input_digest_refs: &BTreeSet<u32>,
) -> Result<(), CoveError> {
    let Some(model_input_digest_ref_id) = ModelInputDigestRef::non_zero(model_input_digest_ref)
    else {
        return Ok(());
    };
    let model_input_digest_ref = model_input_digest_ref_id.raw();
    if !digest_ref_ids.contains(&model_input_digest_ref) {
        return Err(CoveError::BadSection(format!(
            "{label} {binding_id} references missing model_input_digest_ref {model_input_digest_ref}"
        )));
    }
    if !model_input_digest_refs.contains(&model_input_digest_ref) {
        return Err(CoveError::BadSection(format!(
            "{label} {binding_id} model_input_digest_ref {model_input_digest_ref} must use ModelInputBytes digest domain"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorPayloadBlockHeaderV1 {
    pub block_id: u32,
    pub vector_space_id: u32,
    pub vector_count: u64,
    pub dimension_count: u32,
    pub element_type: u8,
    pub compression_codec: u8,
    pub quantization_kind: u8,
    pub layout_kind: u8,
    pub tensor_layout_ref: u32,
    pub memory_alignment_bytes: u32,
    pub payload_stride_ref: u32,
    pub device_transfer_hint_ref: u32,
    pub payload_ref: u32,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub integrity_ref: u32,
    pub checksum: u32,
}

impl VectorPayloadBlockHeaderV1 {
    pub(super) fn validate(
        &self,
        tables: &AiDescriptorTablesV1,
        sections: &[CoveAiSection],
        payload_ref_ids: &BTreeSet<u32>,
        integrity_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.block_id == 0 {
            return Err(CoveError::BadSection(
                "VectorPayloadBlockHeaderV1 block_id must be non-zero".into(),
            ));
        }
        let vector_space = tables
            .vector_spaces
            .iter()
            .find(|space| space.vector_space_id == self.vector_space_id)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "VectorPayloadBlock {} references missing vector_space_id {}",
                    self.block_id, self.vector_space_id
                ))
            })?;
        if vector_space.dimension_count != self.dimension_count {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} dimension_count does not match vector_space_id {}",
                self.block_id, self.vector_space_id
            )));
        }
        if vector_space.element_type != self.element_type {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} element_type does not match vector_space_id {}",
                self.block_id, self.vector_space_id
            )));
        }
        validate_ai_compression_codec(
            self.compression_codec,
            "VectorPayloadBlock compression_codec",
        )?;
        validate_ai_quantization_kind(
            self.quantization_kind,
            "VectorPayloadBlock quantization_kind",
        )?;
        validate_ai_layout_kind(self.layout_kind, "VectorPayloadBlock layout_kind")?;
        validate_power_of_two_alignment(
            self.memory_alignment_bytes,
            "VectorPayloadBlock memory_alignment_bytes",
        )?;
        if self.dimension_count == 0 {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} has zero dimension_count",
                self.block_id
            )));
        }
        if element_width_bytes(self.element_type).is_none() {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} has unsupported element_type {}",
                self.block_id, self.element_type
            )));
        }
        if !payload_ref_ids.contains(&self.payload_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing payload_ref {}",
                self.block_id, self.payload_ref
            )));
        }
        let tensor_layout_ids = tables
            .tensor_layouts
            .iter()
            .map(|record| record.tensor_layout_id)
            .collect::<BTreeSet<_>>();
        let device_transfer_hint_ids = tables
            .device_transfer_hints
            .iter()
            .map(|record| record.transfer_hint_id)
            .collect::<BTreeSet<_>>();
        if self.tensor_layout_ref != 0 && !tensor_layout_ids.contains(&self.tensor_layout_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing tensor_layout_ref {}",
                self.block_id, self.tensor_layout_ref
            )));
        }
        if self.payload_stride_ref != 0
            && !payload_ref_ids.contains(&self.payload_stride_ref)
            && !tensor_layout_ids.contains(&self.payload_stride_ref)
        {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing payload_stride_ref {}",
                self.block_id, self.payload_stride_ref
            )));
        }
        if self.device_transfer_hint_ref != 0
            && !device_transfer_hint_ids.contains(&self.device_transfer_hint_ref)
        {
            return Err(CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing device_transfer_hint_ref {}",
                self.block_id, self.device_transfer_hint_ref
            )));
        }
        let payload_ref = tables.payload_ref(self.payload_ref).ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorPayloadBlock {} references missing payload_ref {}",
                self.block_id, self.payload_ref
            ))
        })?;
        payload_ref.validate_token_or_vector_payload_carrier(sections)?;
        validate_cached_payload_range(
            "VectorPayloadBlock",
            self.block_id,
            payload_ref,
            self.payload_offset,
            self.payload_length,
        )?;
        if self.integrity_ref != 0 {
            if !integrity_ids.contains(&self.integrity_ref) {
                return Err(CoveError::BadSection(format!(
                    "VectorPayloadBlock {} references missing integrity_ref {}",
                    self.block_id, self.integrity_ref
                )));
            }
            let integrity = tables.integrity_ref(self.integrity_ref).ok_or_else(|| {
                CoveError::BadSection(format!(
                    "VectorPayloadBlock {} references missing integrity_ref {}",
                    self.block_id, self.integrity_ref
                ))
            })?;
            if integrity.payload_ref != self.payload_ref {
                return Err(CoveError::BadSection(format!(
                    "VectorPayloadBlock {} integrity_ref payload_ref mismatch",
                    self.block_id
                )));
            }
        }
        Ok(())
    }

    pub(super) fn dense_vector_width(&self) -> Option<u64> {
        let element_width = element_width_bytes(self.element_type)?;
        u64::from(self.dimension_count).checked_mul(element_width)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorEntryV1 {
    pub vector_ref: u64,
    pub block_id: u32,
    pub vector_ordinal: u64,
    pub payload_offset: u64,
    pub payload_length: u32,
    pub integrity_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl VectorEntryV1 {
    pub(super) fn validate(
        &self,
        tables: &AiDescriptorTablesV1,
        vector_block_ids: &BTreeSet<u32>,
        integrity_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if !vector_block_ids.contains(&self.block_id) {
            return Err(CoveError::BadSection(format!(
                "VectorEntry {} references missing block_id {}",
                self.vector_ref, self.block_id
            )));
        }
        let block = tables.vector_block(self.block_id).ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorEntry {} references missing block_id {}",
                self.vector_ref, self.block_id
            ))
        })?;
        if self.vector_ordinal >= block.vector_count {
            return Err(CoveError::BadSection(format!(
                "VectorEntry {} vector_ordinal exceeds block vector_count",
                self.vector_ref
            )));
        }
        let block_payload = tables.payload_ref(block.payload_ref).ok_or_else(|| {
            CoveError::BadSection(format!(
                "VectorEntry {} references missing block payload_ref {}",
                self.vector_ref, block.payload_ref
            ))
        })?;
        if self.payload_length == 0 {
            if self.payload_offset != 0 {
                return Err(CoveError::BadSection(format!(
                    "VectorEntry {} has zero payload_length but non-zero payload_offset",
                    self.vector_ref
                )));
            }
            let Some(dense_width) = block.dense_vector_width() else {
                return Err(CoveError::BadSection(format!(
                    "VectorEntry {} can derive payload range only for fixed dense row-major vectors",
                    self.vector_ref
                )));
            };
            if block.layout_kind != 0 {
                return Err(CoveError::BadSection(format!(
                    "VectorEntry {} can derive payload range only for fixed dense row-major vectors",
                    self.vector_ref
                )));
            }
            let derived_start = self
                .vector_ordinal
                .checked_mul(dense_width)
                .ok_or(CoveError::ArithOverflow)?;
            let derived_end = derived_start
                .checked_add(dense_width)
                .ok_or(CoveError::ArithOverflow)?;
            if derived_end > block_payload.decoded_length {
                return Err(CoveError::BadSection(format!(
                    "VectorEntry {} derived payload range exceeds vector block payload length",
                    self.vector_ref
                )));
            }
        } else {
            let entry_len = u64::from(self.payload_length);
            if block.layout_kind == 0 {
                let Some(dense_width) = block.dense_vector_width() else {
                    return Err(CoveError::BadSection(format!(
                        "VectorEntry {} has unsupported dense vector width",
                        self.vector_ref
                    )));
                };
                if entry_len != dense_width {
                    return Err(CoveError::BadSection(format!(
                        "VectorEntry {} payload_length does not match dense vector width",
                        self.vector_ref
                    )));
                }
            }
            let entry_end = self
                .payload_offset
                .checked_add(entry_len)
                .ok_or(CoveError::ArithOverflow)?;
            if entry_end > block_payload.decoded_length {
                return Err(CoveError::BadSection(format!(
                    "VectorEntry {} payload range exceeds vector block payload length",
                    self.vector_ref
                )));
            }
        }
        if self.integrity_ref != 0 && !integrity_ids.contains(&self.integrity_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorEntry {} references missing integrity_ref {}",
                self.vector_ref, self.integrity_ref
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorCompositionProfileV1 {
    pub composition_profile_id: u32,
    pub composition_name_ref: u32,
    pub output_vector_space_id: u32,
    pub arithmetic_profile_ref: u32,
    pub method: u8,
    pub missing_policy: u8,
    pub normalize_inputs: u8,
    pub normalize_output: u8,
    pub result_authority: u8,
    pub reproducibility_class: u8,
    pub first_component_ref: u32,
    pub component_count: u32,
    pub template_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl VectorCompositionProfileV1 {
    pub(super) fn validate(
        &self,
        string_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        vector_space_ids: &BTreeSet<u32>,
        arithmetic_profile_ids: &BTreeSet<u32>,
        component_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.composition_profile_id == 0 {
            return Err(CoveError::BadSection(
                "VectorCompositionProfileV1 composition_profile_id must be non-zero".into(),
            ));
        }
        if self.composition_name_ref != 0 && !string_ref_ids.contains(&self.composition_name_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionProfile {} references missing composition_name_ref {}",
                self.composition_profile_id, self.composition_name_ref
            )));
        }
        if !vector_space_ids.contains(&self.output_vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionProfile {} references missing output_vector_space_id {}",
                self.composition_profile_id, self.output_vector_space_id
            )));
        }
        if self.arithmetic_profile_ref != 0
            && !arithmetic_profile_ids.contains(&self.arithmetic_profile_ref)
        {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionProfile {} references missing arithmetic_profile_ref {}",
                self.composition_profile_id, self.arithmetic_profile_ref
            )));
        }
        validate_ai_vector_composition_method(self.method, "VectorCompositionProfile method")?;
        validate_ai_vector_composition_missing_policy(
            self.missing_policy,
            "VectorCompositionProfile missing_policy",
        )?;
        validate_bool_byte(
            self.normalize_inputs,
            "VectorCompositionProfile normalize_inputs",
        )?;
        validate_bool_byte(
            self.normalize_output,
            "VectorCompositionProfile normalize_output",
        )?;
        validate_ai_result_authority(
            self.result_authority,
            "VectorCompositionProfile result_authority",
        )?;
        validate_ai_reproducibility_class(
            self.reproducibility_class,
            "VectorCompositionProfile reproducibility_class",
        )?;
        if self.result_authority == 2 && self.arithmetic_profile_ref == 0 {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionProfile {} CanonicalFixedPointRecompute requires arithmetic_profile_ref",
                self.composition_profile_id
            )));
        }
        if self.template_ref != 0 && !payload_ref_ids.contains(&self.template_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionProfile {} references missing template_ref {}",
                self.composition_profile_id, self.template_ref
            )));
        }
        if (self.first_component_ref == 0) != (self.component_count == 0) {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionProfile {} first_component_ref/component_count mismatch",
                self.composition_profile_id
            )));
        }
        if self.component_count != 0 {
            let last_component_ref = self
                .first_component_ref
                .checked_add(self.component_count - 1)
                .ok_or(CoveError::ArithOverflow)?;
            let actual_count = component_ids
                .range(self.first_component_ref..=last_component_ref)
                .count();
            if actual_count
                != usize::try_from(self.component_count).map_err(|_| CoveError::ArithOverflow)?
            {
                return Err(CoveError::BadSection(format!(
                    "VectorCompositionProfile {} references missing component range",
                    self.composition_profile_id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorCompositionComponentV1 {
    pub component_id: u32,
    pub slot_policy_ref: u32,
    pub source_vector_space_id: u32,
    pub weight_ppm: u32,
    pub required: u8,
    pub redaction_behavior: u8,
    pub missing_behavior: u8,
    pub reserved: u8,
    pub flags: u32,
    pub checksum: u32,
}

impl VectorCompositionComponentV1 {
    pub(super) fn validate(&self, vector_space_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
        if self.component_id == 0 {
            return Err(CoveError::BadSection(
                "VectorCompositionComponentV1 component_id must be non-zero".into(),
            ));
        }
        if !vector_space_ids.contains(&self.source_vector_space_id) {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionComponent {} references missing source_vector_space_id {}",
                self.component_id, self.source_vector_space_id
            )));
        }
        if self.weight_ppm > 1_000_000 {
            return Err(CoveError::BadSection(format!(
                "VectorCompositionComponent {} weight_ppm exceeds 1_000_000",
                self.component_id
            )));
        }
        validate_bool_byte(self.required, "VectorCompositionComponent required")?;
        validate_ai_vector_redaction_behavior(
            self.redaction_behavior,
            "VectorCompositionComponent redaction_behavior",
        )?;
        validate_ai_vector_component_missing_behavior(
            self.missing_behavior,
            "VectorCompositionComponent missing_behavior",
        )?;
        if self.reserved != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorArithmeticProfileV1 {
    pub arithmetic_profile_id: u32,
    pub profile_name_ref: u32,
    pub arithmetic_kind: u8,
    pub input_quantization_kind: u8,
    pub accumulator_kind: u8,
    pub rounding_mode: u8,
    pub overflow_policy: u8,
    pub component_order: u8,
    pub weight_scale: u32,
    pub output_quantization_kind: u8,
    pub output_element_type: u8,
    pub normalization_policy: u8,
    pub flags: u32,
    pub checksum: u32,
}

impl VectorArithmeticProfileV1 {
    pub(super) fn validate(&self, string_ref_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
        if self.arithmetic_profile_id == 0 {
            return Err(CoveError::BadSection(
                "VectorArithmeticProfileV1 arithmetic_profile_id must be non-zero".into(),
            ));
        }
        if self.profile_name_ref != 0 && !string_ref_ids.contains(&self.profile_name_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorArithmeticProfile {} references missing profile_name_ref {}",
                self.arithmetic_profile_id, self.profile_name_ref
            )));
        }
        validate_ai_vector_arithmetic_kind(
            self.arithmetic_kind,
            "VectorArithmeticProfile arithmetic_kind",
        )?;
        validate_ai_quantization_kind(
            self.input_quantization_kind,
            "VectorArithmeticProfile input_quantization_kind",
        )?;
        validate_ai_vector_accumulator_kind(
            self.accumulator_kind,
            "VectorArithmeticProfile accumulator_kind",
        )?;
        validate_ai_vector_rounding_mode(
            self.rounding_mode,
            "VectorArithmeticProfile rounding_mode",
        )?;
        validate_ai_vector_overflow_policy(
            self.overflow_policy,
            "VectorArithmeticProfile overflow_policy",
        )?;
        validate_ai_vector_component_order(
            self.component_order,
            "VectorArithmeticProfile component_order",
        )?;
        validate_ai_quantization_kind(
            self.output_quantization_kind,
            "VectorArithmeticProfile output_quantization_kind",
        )?;
        validate_ai_vector_element_type(
            self.output_element_type,
            "VectorArithmeticProfile output_element_type",
        )?;
        validate_ai_normalization_policy(
            self.normalization_policy,
            "VectorArithmeticProfile normalization_policy",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorIndexDescriptorV1 {
    pub vector_index_id: u32,
    pub vector_space_id: u32,
    pub stored_vector_space_id: u32,
    pub search_vector_space_id: u32,
    pub index_kind: u8,
    pub exactness_kind: u8,
    pub false_negative_policy: u8,
    pub metric: u8,
    pub score_space_authority: u8,
    pub dimension_count: u32,
    pub indexed_binding_kind: u8,
    pub temporal_scope_ref: u32,
    pub visibility_scope_ref: u32,
    pub redaction_scope_ref: u32,
    pub dequantization_profile_ref: u32,
    pub quantization_error_profile_ref: u32,
    pub payload_ref: u32,
    pub checksum: u32,
}

impl VectorIndexDescriptorV1 {
    pub(super) fn validate(
        &self,
        vector_spaces: &[VectorSpaceDescriptorV1],
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        if self.vector_index_id == 0 {
            return Err(CoveError::BadSection(
                "VectorIndexDescriptorV1 vector_index_id must be non-zero".into(),
            ));
        }
        if self.dimension_count == 0 {
            return Err(CoveError::BadSection(format!(
                "VectorIndex {} requires non-zero dimension_count",
                self.vector_index_id
            )));
        }
        validate_ai_vector_index_kind(self.index_kind, "VectorIndex index_kind")?;
        validate_ai_vector_index_exactness(self.exactness_kind, "VectorIndex exactness_kind")?;
        validate_ai_vector_false_negative_policy(
            self.false_negative_policy,
            "VectorIndex false_negative_policy",
        )?;
        validate_ai_vector_metric(self.metric, "VectorIndex metric")?;
        validate_ai_vector_score_space_authority(
            self.score_space_authority,
            "VectorIndex score_space_authority",
        )?;
        validate_ai_vector_indexed_binding_kind(
            self.indexed_binding_kind,
            "VectorIndex indexed_binding_kind",
        )?;
        if self.index_kind != 0 && self.exactness_kind == 0 {
            return Err(CoveError::BadSection(format!(
                "VectorIndex {} unsupported ANN metadata must not claim exactness",
                self.vector_index_id
            )));
        }
        if self.exactness_kind == 0 && self.false_negative_policy != 0 {
            return Err(CoveError::BadSection(format!(
                "VectorIndex {} exact index must not declare false negatives",
                self.vector_index_id
            )));
        }
        for (label, vector_space_id) in [
            ("vector_space_id", self.vector_space_id),
            ("stored_vector_space_id", self.stored_vector_space_id),
            ("search_vector_space_id", self.search_vector_space_id),
        ] {
            let Some(space) = vector_spaces
                .iter()
                .find(|space| space.vector_space_id == vector_space_id)
            else {
                return Err(CoveError::BadSection(format!(
                    "VectorIndex {} references missing {label} {}",
                    self.vector_index_id, vector_space_id
                )));
            };
            if space.dimension_count != self.dimension_count {
                return Err(CoveError::BadSection(format!(
                    "VectorIndex {} dimension_count does not match {label}",
                    self.vector_index_id
                )));
            }
        }
        for (label, policy_ref) in [
            ("temporal_scope_ref", self.temporal_scope_ref),
            ("visibility_scope_ref", self.visibility_scope_ref),
            ("redaction_scope_ref", self.redaction_scope_ref),
        ] {
            if policy_ref != 0 && !policy_ref_ids.contains(&policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "VectorIndex {} references missing {label} {}",
                    self.vector_index_id, policy_ref
                )));
            }
        }
        for (label, payload_ref) in [
            (
                "dequantization_profile_ref",
                self.dequantization_profile_ref,
            ),
            (
                "quantization_error_profile_ref",
                self.quantization_error_profile_ref,
            ),
        ] {
            if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                return Err(CoveError::BadSection(format!(
                    "VectorIndex {} references missing {label} {}",
                    self.vector_index_id, payload_ref
                )));
            }
        }
        if self.payload_ref != 0 && !payload_ref_ids.contains(&self.payload_ref) {
            return Err(CoveError::BadSection(format!(
                "VectorIndex {} references missing payload_ref {}",
                self.vector_index_id, self.payload_ref
            )));
        }
        Ok(())
    }
}
