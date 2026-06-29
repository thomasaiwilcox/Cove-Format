//! COVE-AI companion artifact envelope (`.coveai` / `.covev`).
//!
//! This module implements the shared structural layer for `CVA2` and `CVV2`
//! artifacts: tail discovery, postscript/header validation, section directory
//! validation, section range checks, descriptor record parsing, reference-table
//! validation, payload-ref validation, and the Phase 1 token/vector payload
//! carrier rules. Higher-level COVE-CHUNK, COVE-TOK, COVE-VEC, COVE-TRAIN, and
//! COVE-MMSEQ semantics are layered on top of these validated descriptor tables.

use std::{borrow::Cow, collections::BTreeSet};

use crate::{
    checksum, compression,
    constants::{
        CompressionCodec, PrimaryProfile, SectionKind, AI_FEATURE_ASSET_REF,
        AI_FEATURE_CANONICAL_FIXED_POINT_VECTOR, AI_FEATURE_CHUNK, AI_FEATURE_COVEQL_AI,
        AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED, AI_FEATURE_GENERATOR_PROVENANCE,
        AI_FEATURE_MAP_AI_POLICY, AI_FEATURE_MMSEQ, AI_FEATURE_PRIVACY_SUMMARY,
        AI_FEATURE_TENSOR_LAYOUT, AI_FEATURE_TOKEN, AI_FEATURE_TRAIN, AI_FEATURE_VECTOR,
        AI_FEATURE_VECTOR_INDEX, AI_FEATURE_VECTOR_SPACE_COMPATIBILITY, MAGIC_COVEAI, MAGIC_COVEV,
    },
    feature_binding::{FeatureScopeV2, OperationKindV2},
    CoveError,
};

pub const COVEAI_VERSION_MAJOR_V1: u16 = 1;
pub const COVEAI_VERSION_MINOR_V1: u16 = 0;
pub const COVEAI_POSTSCRIPT_VERSION_V1: u16 = 1;
pub const COVEAI_POSTSCRIPT_LEN: u16 = 44;
pub const COVEAI_HEADER_LEN: u16 = 70;
pub const COVEAI_SECTION_ENTRY_LEN: u16 = 64;
pub const COVEAI_POSTSCRIPT_TAIL_SIZE: usize = 2 + 2 + 4;
pub const AI_RECORD_HEADER_LEN: usize = 24;

pub const AI_KNOWN_FEATURES_V1: u64 = AI_FEATURE_MAP_AI_POLICY
    | AI_FEATURE_CHUNK
    | AI_FEATURE_TOKEN
    | AI_FEATURE_VECTOR
    | AI_FEATURE_VECTOR_INDEX
    | AI_FEATURE_TENSOR_LAYOUT
    | AI_FEATURE_ASSET_REF
    | AI_FEATURE_MMSEQ
    | AI_FEATURE_TRAIN
    | AI_FEATURE_GENERATOR_PROVENANCE
    | AI_FEATURE_COVEQL_AI
    | AI_FEATURE_CANONICAL_FIXED_POINT_VECTOR
    | AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED
    | AI_FEATURE_PRIVACY_SUMMARY
    | AI_FEATURE_VECTOR_SPACE_COMPATIBILITY;

pub const AI_FLAG_REQUIRED_RECORD: u32 = 1 << 0;
pub const AI_FLAG_PAYLOAD_CRC32C_PRESENT: u32 = 1 << 1;
pub const AI_FLAG_POLICY_PROTECTED: u32 = 1 << 2;
pub const AI_FLAG_REVOKED: u32 = 1 << 3;

pub const AI_COMPANION_ARTIFACT_KIND_CVA2: u8 = 1;
pub const AI_COMPANION_ARTIFACT_KIND_CVV2: u8 = 2;

pub const AI_SOURCE_KIND_COVE_FILE: u8 = 0;
pub const AI_SOURCE_KIND_COVM_SNAPSHOT: u8 = 1;
pub const AI_SOURCE_KIND_COVEMAP_ARTIFACT: u8 = 2;
pub const AI_SOURCE_KIND_EXTERNAL_ASSET: u8 = 3;
pub const AI_SOURCE_KIND_EXTERNAL_DATASET: u8 = 4;

pub const AI_POLICY_KIND_VISIBILITY: u8 = 0;
pub const AI_POLICY_KIND_REDACTION: u8 = 1;
pub const AI_POLICY_KIND_SENSITIVITY: u8 = 2;
pub const AI_POLICY_KIND_LICENSE: u8 = 3;
pub const AI_POLICY_KIND_RETENTION: u8 = 4;
pub const AI_POLICY_KIND_DISCLOSURE: u8 = 5;
pub const AI_POLICY_KIND_SAFETY: u8 = 6;

pub const AI_TRANSFORM_KIND_NONE: u8 = 0;
pub const AI_TRANSFORM_KIND_TEXT_NORMALIZATION: u8 = 1;
pub const AI_TRANSFORM_KIND_TOKENIZER: u8 = 2;
pub const AI_TRANSFORM_KIND_CHUNKER: u8 = 3;
pub const AI_TRANSFORM_KIND_VECTORIZER: u8 = 4;
pub const AI_TRANSFORM_KIND_QUANTIZATION: u8 = 5;
pub const AI_TRANSFORM_KIND_IMAGE_PREPROCESS: u8 = 6;
pub const AI_TRANSFORM_KIND_AUDIO_PREPROCESS: u8 = 7;
pub const AI_TRANSFORM_KIND_VIDEO_FRAME_EXTRACTION: u8 = 8;
pub const AI_TRANSFORM_KIND_OCR: u8 = 9;
pub const AI_TRANSFORM_KIND_CAPTION: u8 = 10;
pub const AI_TRANSFORM_KIND_TRANSCRIPT: u8 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoveAiArtifactKind {
    CoveAiBundle,
    CoveVec,
}

impl CoveAiArtifactKind {
    pub fn magic(self) -> [u8; 4] {
        match self {
            Self::CoveAiBundle => MAGIC_COVEAI,
            Self::CoveVec => MAGIC_COVEV,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::CoveAiBundle => ".coveai",
            Self::CoveVec => ".covev",
        }
    }

    fn from_magic(magic: [u8; 4]) -> Option<Self> {
        match magic {
            MAGIC_COVEAI => Some(Self::CoveAiBundle),
            MAGIC_COVEV => Some(Self::CoveVec),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AiPayloadEncodingV1 {
    BinaryRecords = 1,
    CanonicalJson = 2,
    DeterministicCbor = 3,
    OpaqueBytes = 4,
    Extension = 255,
}

impl AiPayloadEncodingV1 {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::BinaryRecords),
            2 => Some(Self::CanonicalJson),
            3 => Some(Self::DeterministicCbor),
            4 => Some(Self::OpaqueBytes),
            255 => Some(Self::Extension),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AiRequirednessScopeV1 {
    ArtifactRequired = 0,
    SectionRequired = 1,
    ProfileRequired = 2,
    OperationRequired = 3,
    AdvisoryOnly = 4,
    Extension = 255,
}

impl AiRequirednessScopeV1 {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::ArtifactRequired),
            1 => Some(Self::SectionRequired),
            2 => Some(Self::ProfileRequired),
            3 => Some(Self::OperationRequired),
            4 => Some(Self::AdvisoryOnly),
            255 => Some(Self::Extension),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AiStorageKindV1 {
    ArtifactAbsolute = 0,
    SectionDecodedRelative = 1,
    ExternalUri = 2,
    EmbeddedSection = 3,
    Extension = 255,
}

impl AiStorageKindV1 {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::ArtifactAbsolute),
            1 => Some(Self::SectionDecodedRelative),
            2 => Some(Self::ExternalUri),
            3 => Some(Self::EmbeddedSection),
            255 => Some(Self::Extension),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiPostscriptV1 {
    pub required_ai_features: u64,
    pub optional_ai_features: u64,
    pub file_len: u64,
    pub header_offset: u64,
    pub header_length: u64,
    pub crc32c: u32,
}

impl CoveAiPostscriptV1 {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEAI_POSTSCRIPT_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEAI_POSTSCRIPT_LEN as usize];
        let postscript = Self {
            required_ai_features: read_u64(bytes, 0)?,
            optional_ai_features: read_u64(bytes, 8)?,
            file_len: read_u64(bytes, 16)?,
            header_offset: read_u64(bytes, 24)?,
            header_length: read_u64(bytes, 32)?,
            crc32c: read_u32(bytes, 40)?,
        };
        verify_crc32c(bytes, 40, postscript.crc32c)?;
        Ok(postscript)
    }

    pub fn serialize(&self) -> [u8; COVEAI_POSTSCRIPT_LEN as usize] {
        let mut out = [0u8; COVEAI_POSTSCRIPT_LEN as usize];
        put_u64(&mut out, 0, self.required_ai_features);
        put_u64(&mut out, 8, self.optional_ai_features);
        put_u64(&mut out, 16, self.file_len);
        put_u64(&mut out, 24, self.header_offset);
        put_u64(&mut out, 32, self.header_length);
        let crc = checksum::crc32c(&out);
        put_u32(&mut out, 40, crc);
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiHeaderV1 {
    pub magic: [u8; 4],
    pub header_len: u16,
    pub version_major: u16,
    pub version_minor: u16,
    pub flags: u32,
    pub artifact_id: [u8; 16],
    pub required_ai_features: u64,
    pub optional_ai_features: u64,
    pub section_count: u32,
    pub section_entry_len: u16,
    pub reserved0: u16,
    pub created_at_us: i64,
    pub section_directory_crc32c: u32,
    pub crc32c: u32,
}

impl CoveAiHeaderV1 {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEAI_HEADER_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEAI_HEADER_LEN as usize];
        let header = Self {
            magic: read_array::<4>(bytes, 0)?,
            header_len: read_u16(bytes, 4)?,
            version_major: read_u16(bytes, 6)?,
            version_minor: read_u16(bytes, 8)?,
            flags: read_u32(bytes, 10)?,
            artifact_id: read_array::<16>(bytes, 14)?,
            required_ai_features: read_u64(bytes, 30)?,
            optional_ai_features: read_u64(bytes, 38)?,
            section_count: read_u32(bytes, 46)?,
            section_entry_len: read_u16(bytes, 50)?,
            reserved0: read_u16(bytes, 52)?,
            created_at_us: read_i64(bytes, 54)?,
            section_directory_crc32c: read_u32(bytes, 62)?,
            crc32c: read_u32(bytes, 66)?,
        };
        if CoveAiArtifactKind::from_magic(header.magic).is_none() {
            return Err(CoveError::BadMagic);
        }
        if header.header_len != COVEAI_HEADER_LEN {
            return Err(CoveError::BadSection(format!(
                "COVE-AI header_len must be {COVEAI_HEADER_LEN}, got {}",
                header.header_len
            )));
        }
        if header.version_major != COVEAI_VERSION_MAJOR_V1 {
            return Err(CoveError::BadVersion);
        }
        if header.section_entry_len != COVEAI_SECTION_ENTRY_LEN {
            return Err(CoveError::BadSection(format!(
                "COVE-AI section_entry_len must be {COVEAI_SECTION_ENTRY_LEN}, got {}",
                header.section_entry_len
            )));
        }
        if header.reserved0 != 0 || header.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        verify_crc32c(bytes, 66, header.crc32c)?;
        Ok(header)
    }

    pub fn serialize(
        &self,
        section_directory: &[u8],
    ) -> Result<[u8; COVEAI_HEADER_LEN as usize], CoveError> {
        if self.header_len != COVEAI_HEADER_LEN
            || self.section_entry_len != COVEAI_SECTION_ENTRY_LEN
            || self.reserved0 != 0
            || self.flags != 0
        {
            return Err(CoveError::BadSection(
                "invalid COVE-AI header fields for serialization".into(),
            ));
        }
        let mut out = [0u8; COVEAI_HEADER_LEN as usize];
        out[0..4].copy_from_slice(&self.magic);
        put_u16(&mut out, 4, self.header_len);
        put_u16(&mut out, 6, self.version_major);
        put_u16(&mut out, 8, self.version_minor);
        put_u32(&mut out, 10, self.flags);
        out[14..30].copy_from_slice(&self.artifact_id);
        put_u64(&mut out, 30, self.required_ai_features);
        put_u64(&mut out, 38, self.optional_ai_features);
        put_u32(&mut out, 46, self.section_count);
        put_u16(&mut out, 50, self.section_entry_len);
        put_u16(&mut out, 52, self.reserved0);
        put_i64(&mut out, 54, self.created_at_us);
        put_u32(&mut out, 62, checksum::crc32c(section_directory));
        let crc = checksum::crc32c(&out);
        put_u32(&mut out, 66, crc);
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiSectionEntryV1 {
    pub section_id: u32,
    pub section_kind: u32,
    pub offset: u64,
    pub length: u64,
    pub uncompressed_length: u64,
    pub compression: u8,
    pub payload_encoding: u8,
    pub requiredness_scope: u8,
    pub profile_kind: u8,
    pub source_binding_ref: u32,
    pub required_ai_features: u64,
    pub optional_ai_features: u64,
    pub feature_binding_ref: u32,
    pub payload_crc32c: u32,
}

impl CoveAiSectionEntryV1 {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEAI_SECTION_ENTRY_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEAI_SECTION_ENTRY_LEN as usize];
        let entry = Self {
            section_id: read_u32(bytes, 0)?,
            section_kind: read_u32(bytes, 4)?,
            offset: read_u64(bytes, 8)?,
            length: read_u64(bytes, 16)?,
            uncompressed_length: read_u64(bytes, 24)?,
            compression: read_u8(bytes, 32)?,
            payload_encoding: read_u8(bytes, 33)?,
            requiredness_scope: read_u8(bytes, 34)?,
            profile_kind: read_u8(bytes, 35)?,
            source_binding_ref: read_u32(bytes, 36)?,
            required_ai_features: read_u64(bytes, 40)?,
            optional_ai_features: read_u64(bytes, 48)?,
            feature_binding_ref: read_u32(bytes, 56)?,
            payload_crc32c: read_u32(bytes, 60)?,
        };
        entry.validate_static()?;
        Ok(entry)
    }

    pub fn serialize(&self) -> Result<[u8; COVEAI_SECTION_ENTRY_LEN as usize], CoveError> {
        self.validate_static()?;
        let mut out = [0u8; COVEAI_SECTION_ENTRY_LEN as usize];
        put_u32(&mut out, 0, self.section_id);
        put_u32(&mut out, 4, self.section_kind);
        put_u64(&mut out, 8, self.offset);
        put_u64(&mut out, 16, self.length);
        put_u64(&mut out, 24, self.uncompressed_length);
        put_u8(&mut out, 32, self.compression);
        put_u8(&mut out, 33, self.payload_encoding);
        put_u8(&mut out, 34, self.requiredness_scope);
        put_u8(&mut out, 35, self.profile_kind);
        put_u32(&mut out, 36, self.source_binding_ref);
        put_u64(&mut out, 40, self.required_ai_features);
        put_u64(&mut out, 48, self.optional_ai_features);
        put_u32(&mut out, 56, self.feature_binding_ref);
        put_u32(&mut out, 60, self.payload_crc32c);
        Ok(out)
    }

    fn validate_static(&self) -> Result<(), CoveError> {
        if CompressionCodec::from_u8(self.compression).is_none() {
            return Err(CoveError::BadSection(format!(
                "unknown COVE-AI section compression codec {}",
                self.compression
            )));
        }
        if AiPayloadEncodingV1::from_u8(self.payload_encoding).is_none() {
            return Err(CoveError::BadSection(format!(
                "unknown COVE-AI payload encoding {}",
                self.payload_encoding
            )));
        }
        if AiRequirednessScopeV1::from_u8(self.requiredness_scope).is_none() {
            return Err(CoveError::BadSection(format!(
                "unknown COVE-AI requiredness scope {}",
                self.requiredness_scope
            )));
        }
        if self.required_ai_features & !AI_KNOWN_FEATURES_V1 != 0 {
            return Err(CoveError::UnknownRequiredFeature(
                self.required_ai_features & !AI_KNOWN_FEATURES_V1,
            ));
        }
        if self.section_kind == SectionKind::AiPayloadBytes as u32 {
            if self.payload_encoding != AiPayloadEncodingV1::OpaqueBytes as u8 {
                return Err(CoveError::BadSection(
                    "AI_PAYLOAD_BYTES must declare OpaqueBytes payload encoding".into(),
                ));
            }
        } else if self.payload_encoding == AiPayloadEncodingV1::OpaqueBytes as u8 {
            return Err(CoveError::BadSection(
                "only AI_PAYLOAD_BYTES may declare OpaqueBytes payload encoding".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiSection {
    pub entry: CoveAiSectionEntryV1,
    pub record_headers: Vec<AiRecordHeaderV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiFile {
    pub artifact_kind: CoveAiArtifactKind,
    pub postscript: CoveAiPostscriptV1,
    pub header: CoveAiHeaderV1,
    pub sections: Vec<CoveAiSection>,
    pub descriptor_tables: AiDescriptorTablesV1,
    pub payload_access: AiPayloadAccessState,
}

impl CoveAiFile {
    pub fn parse(data: &[u8]) -> Result<Self, CoveError> {
        let (artifact_kind, postscript, postscript_offset) = parse_postscript_from_tail(data)?;
        if postscript.file_len != data.len() as u64 {
            return Err(CoveError::BadSection(
                "COVE-AI postscript file_len does not match actual file length".into(),
            ));
        }
        if postscript.header_length
            != u64::from(COVEAI_HEADER_LEN)
                + postscript
                    .header_length
                    .saturating_sub(u64::from(COVEAI_HEADER_LEN))
        {
            // Kept as an explicit arithmetic point below; this branch is never
            // expected, but avoids accidental future wrapping refactors.
            return Err(CoveError::ArithOverflow);
        }
        let header_region =
            checked_slice(data, postscript.header_offset, postscript.header_length)?;
        let header = CoveAiHeaderV1::parse(header_region)?;
        if CoveAiArtifactKind::from_magic(header.magic) != Some(artifact_kind) {
            return Err(CoveError::BadMagic);
        }
        if postscript.required_ai_features != header.required_ai_features
            || postscript.optional_ai_features != header.optional_ai_features
        {
            return Err(CoveError::BadSection(
                "COVE-AI postscript/header feature words mismatch".into(),
            ));
        }
        let unknown_header_required = header.required_ai_features & !AI_KNOWN_FEATURES_V1;
        if unknown_header_required != 0 {
            return Err(CoveError::UnknownRequiredFeature(unknown_header_required));
        }
        let expected_header_length = u64::from(header.header_len)
            .checked_add(
                u64::from(header.section_count)
                    .checked_mul(u64::from(header.section_entry_len))
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .ok_or(CoveError::ArithOverflow)?;
        if expected_header_length != postscript.header_length {
            return Err(CoveError::BadSection(
                "COVE-AI postscript header_length does not equal fixed header plus section entries"
                    .into(),
            ));
        }
        let entries_bytes = &header_region[header.header_len as usize..];
        if checksum::crc32c(entries_bytes) != header.section_directory_crc32c {
            return Err(CoveError::ChecksumMismatch);
        }

        let mut raw_entries = Vec::with_capacity(header.section_count as usize);
        let mut section_ids = BTreeSet::new();
        for chunk in entries_bytes.chunks_exact(COVEAI_SECTION_ENTRY_LEN as usize) {
            let entry = CoveAiSectionEntryV1::parse(chunk)?;
            if !section_ids.insert(entry.section_id) {
                return Err(CoveError::BadSection(format!(
                    "duplicate COVE-AI section_id {}",
                    entry.section_id
                )));
            }
            raw_entries.push(entry);
        }
        validate_section_ranges(
            &raw_entries,
            data.len() as u64,
            postscript.header_offset,
            postscript.header_length,
            postscript_offset as u64,
        )?;

        let mut descriptor_tables = AiDescriptorTablesV1::default();
        let mut sections = Vec::with_capacity(raw_entries.len());
        for entry in raw_entries {
            let decoded_payload = decoded_section_payload(data, &entry)?;
            validate_section_payload_crc(&entry, &decoded_payload)?;
            let mut record_headers = Vec::new();
            if entry.payload_encoding == AiPayloadEncodingV1::BinaryRecords as u8 {
                record_headers =
                    parse_ai_records(&entry, &decoded_payload, &mut descriptor_tables)?;
            }
            sections.push(CoveAiSection {
                entry,
                record_headers,
            });
        }

        descriptor_tables.validate(&sections, data.len() as u64)?;
        let payload_access = if descriptor_tables.privacy_summaries.is_empty()
            && sections
                .iter()
                .any(|section| is_payload_bearing_section(section.entry.section_kind))
        {
            AiPayloadAccessState::PolicyBlockedMissingPrivacySummary
        } else {
            AiPayloadAccessState::StructurallyAllowed
        };

        Ok(Self {
            artifact_kind,
            postscript,
            header,
            sections,
            descriptor_tables,
            payload_access,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPayloadAccessState {
    StructurallyAllowed,
    PolicyBlockedMissingPrivacySummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AiDescriptorTablesV1 {
    pub companion_artifacts: Vec<AiCompanionArtifactRefV1>,
    pub source_bindings: Vec<AiSourceBindingV1>,
    pub strings: Vec<AiStringEntryV1>,
    pub digests: Vec<AiDigestEntryV1>,
    pub payload_refs: Vec<AiPayloadRefEntryV1>,
    pub policies: Vec<AiPolicyRefEntryV1>,
    pub source_spans: Vec<AiSourceSpanEntryV1>,
    pub transforms: Vec<AiTransformEntryV1>,
    pub privacy_summaries: Vec<AiPrivacySummaryEntryV1>,
    pub payload_integrity: Vec<AiPayloadIntegrityV1>,
    pub section_feature_bindings: Vec<AiSectionFeatureBindingV1>,
    pub chunk_profiles: Vec<ChunkProfileV1>,
    pub text_chunks: Vec<TextChunkEntryV1>,
    pub tokenizer_profiles: Vec<TokenizerProfileV1>,
    pub token_blocks: Vec<TokenBlockHeaderV1>,
    pub tokenized_spans: Vec<TokenizedSpanV1>,
    pub token_sequence_packs: Vec<TokenSequencePackV1>,
    pub training_profiles: Vec<TrainingProfileV1>,
    pub training_samples: Vec<TrainingSampleEntryV1>,
    pub dataset_splits: Vec<DatasetSplitV1>,
    pub dedup_groups: Vec<DedupGroupV1>,
    pub training_epoch_plans: Vec<TrainingEpochPlanV1>,
    pub training_labels: Vec<TrainingLabelEntryV1>,
    pub preference_pairs: Vec<PreferencePairEntryV1>,
    pub generator_provenance: Vec<GeneratorProvenanceV1>,
    pub model_actors: Vec<ModelActorDescriptorV1>,
    pub generation_decoding_profiles: Vec<GenerationDecodingProfileV1>,
    pub human_reviews: Vec<HumanReviewEntryV1>,
    pub tensor_layouts: Vec<TensorLayoutDescriptorV1>,
    pub device_transfer_hints: Vec<DeviceTransferHintV1>,
    pub assets: Vec<AiAssetRefV1>,
    pub multimodal_sequence_packs: Vec<MultimodalSequencePackV1>,
    pub multimodal_sequence_elements: Vec<MultimodalSequenceElementV1>,
    pub vector_spaces: Vec<VectorSpaceDescriptorV1>,
    pub vector_space_compatibilities: Vec<VectorSpaceCompatibilityDescriptorV1>,
    pub filecode_vector_bindings: Vec<FileCodeVectorBindingV1>,
    pub chunk_vector_bindings: Vec<ChunkVectorBindingV1>,
    pub object_state_vector_bindings: Vec<ObjectStateVectorBindingV1>,
    pub training_sample_vector_bindings: Vec<TrainingSampleVectorBindingV1>,
    pub vector_payload_blocks: Vec<VectorPayloadBlockHeaderV1>,
    pub vector_entries: Vec<VectorEntryV1>,
    pub vector_composition_profiles: Vec<VectorCompositionProfileV1>,
    pub vector_composition_components: Vec<VectorCompositionComponentV1>,
    pub vector_arithmetic_profiles: Vec<VectorArithmeticProfileV1>,
    pub vector_indexes: Vec<VectorIndexDescriptorV1>,
}

impl AiDescriptorTablesV1 {
    fn validate(&self, sections: &[CoveAiSection], file_len: u64) -> Result<(), CoveError> {
        validate_unique(
            self.companion_artifacts
                .iter()
                .map(|record| record.artifact_ref),
            "AI_COMPANION_ARTIFACT_REF artifact_ref",
        )?;
        validate_unique(
            self.source_bindings
                .iter()
                .map(|record| record.source_binding_id),
            "AI_SOURCE_BINDING source_binding_id",
        )?;
        validate_unique(
            self.strings.iter().map(|record| record.string_ref),
            "AI_STRING_TABLE string_ref",
        )?;
        validate_unique(
            self.digests.iter().map(|record| record.digest_ref),
            "AI_DIGEST_TABLE digest_ref",
        )?;
        validate_unique(
            self.payload_refs.iter().map(|record| record.payload_ref),
            "AI_PAYLOAD_REF_TABLE payload_ref",
        )?;
        validate_unique(
            self.policies.iter().map(|record| record.policy_ref),
            "AI_POLICY_TABLE policy_ref",
        )?;
        validate_unique(
            self.source_spans
                .iter()
                .map(|record| record.source_span_ref),
            "AI_SOURCE_SPAN_TABLE source_span_ref",
        )?;
        validate_unique(
            self.transforms.iter().map(|record| record.transform_ref),
            "AI_TRANSFORM_TABLE transform_ref",
        )?;
        validate_unique(
            self.privacy_summaries
                .iter()
                .map(|record| record.privacy_summary_ref),
            "AI_PRIVACY_SUMMARY privacy_summary_ref",
        )?;
        validate_unique(
            self.payload_integrity
                .iter()
                .map(|record| record.integrity_ref),
            "AI_PAYLOAD_INTEGRITY integrity_ref",
        )?;
        validate_unique(
            self.section_feature_bindings
                .iter()
                .map(|record| record.binding_ref),
            "AI_SECTION_FEATURE_BINDING binding_ref",
        )?;
        validate_unique(
            self.chunk_profiles
                .iter()
                .map(|record| record.chunk_profile_id),
            "AI_CHUNK_PROFILE chunk_profile_id",
        )?;
        validate_unique_u64(
            self.text_chunks.iter().map(|record| record.chunk_id),
            "AI_TEXT_CHUNK_INDEX chunk_id",
        )?;
        validate_unique(
            self.tokenizer_profiles
                .iter()
                .map(|record| record.tokenizer_profile_id),
            "AI_TOKENIZER_PROFILE tokenizer_profile_id",
        )?;
        validate_unique_u64(
            self.tokenized_spans
                .iter()
                .map(|record| record.tokenized_span_id),
            "AI_TOKENIZED_SPAN tokenized_span_id",
        )?;
        validate_unique_u64(
            self.token_sequence_packs
                .iter()
                .map(|record| record.sequence_pack_id),
            "AI_TOKEN_SEQUENCE_PACK sequence_pack_id",
        )?;
        validate_unique(
            self.token_blocks.iter().map(|record| record.token_block_id),
            "AI_TOKEN_BLOCK token_block_id",
        )?;
        validate_unique(
            self.training_profiles
                .iter()
                .map(|record| record.training_profile_id),
            "AI_TRAINING_PROFILE training_profile_id",
        )?;
        validate_unique_u64(
            self.training_samples.iter().map(|record| record.sample_id),
            "AI_TRAINING_SAMPLE_INDEX sample_id",
        )?;
        validate_unique(
            self.dataset_splits.iter().map(|record| record.split_id),
            "AI_TRAINING_SPLIT_DEDUP_EPOCH split_id",
        )?;
        validate_unique_u64(
            self.dedup_groups.iter().map(|record| record.dedup_group_id),
            "AI_TRAINING_SPLIT_DEDUP_EPOCH dedup_group_id",
        )?;
        validate_unique_u64(
            self.training_epoch_plans
                .iter()
                .map(|record| record.epoch_plan_id),
            "AI_TRAINING_SPLIT_DEDUP_EPOCH epoch_plan_id",
        )?;
        validate_unique_u64(
            self.training_labels.iter().map(|record| record.label_id),
            "AI_LABEL_PREFERENCE label_id",
        )?;
        validate_unique_u64(
            self.preference_pairs
                .iter()
                .map(|record| record.preference_pair_id),
            "AI_LABEL_PREFERENCE preference_pair_id",
        )?;
        validate_unique_u64(
            self.generator_provenance
                .iter()
                .map(|record| record.generator_provenance_id),
            "AI_GENERATOR_PROVENANCE generator_provenance_id",
        )?;
        validate_unique(
            self.model_actors.iter().map(|record| record.model_actor_id),
            "AI_GENERATOR_PROVENANCE model_actor_id",
        )?;
        validate_unique(
            self.generation_decoding_profiles
                .iter()
                .map(|record| record.decoding_profile_id),
            "AI_GENERATOR_PROVENANCE decoding_profile_id",
        )?;
        validate_unique(
            self.human_reviews
                .iter()
                .map(|record| record.human_review_id),
            "AI_GENERATOR_PROVENANCE human_review_id",
        )?;
        validate_unique(
            self.tensor_layouts
                .iter()
                .map(|record| record.tensor_layout_id),
            "AI_TENSOR_LAYOUT tensor_layout_id",
        )?;
        validate_unique(
            self.device_transfer_hints
                .iter()
                .map(|record| record.transfer_hint_id),
            "AI_TENSOR_LAYOUT transfer_hint_id",
        )?;
        validate_unique_u64(
            self.assets.iter().map(|record| record.asset_ref_id),
            "AI_ASSET_MANIFEST asset_ref_id",
        )?;
        validate_unique_u64(
            self.multimodal_sequence_packs
                .iter()
                .map(|record| record.sequence_pack_id),
            "AI_MULTIMODAL_SEQUENCE sequence_pack_id",
        )?;
        validate_unique_u64(
            self.multimodal_sequence_elements
                .iter()
                .map(|record| record.element_id),
            "AI_MULTIMODAL_SEQUENCE element_id",
        )?;
        validate_unique(
            self.vector_spaces
                .iter()
                .map(|record| record.vector_space_id),
            "AI_VECTOR_SPACE vector_space_id",
        )?;
        validate_unique(
            self.vector_space_compatibilities
                .iter()
                .map(|record| record.compatibility_id),
            "AI_VECTOR_SPACE compatibility_id",
        )?;
        validate_unique_u64(
            self.filecode_vector_bindings
                .iter()
                .map(|record| record.binding_id),
            "AI_VECTOR_BINDING filecode binding_id",
        )?;
        validate_unique_u64(
            self.chunk_vector_bindings
                .iter()
                .map(|record| record.binding_id),
            "AI_VECTOR_BINDING chunk binding_id",
        )?;
        validate_unique_u64(
            self.object_state_vector_bindings
                .iter()
                .map(|record| record.binding_id),
            "AI_VECTOR_BINDING object_state binding_id",
        )?;
        validate_unique_u64(
            self.training_sample_vector_bindings
                .iter()
                .map(|record| record.binding_id),
            "AI_VECTOR_BINDING training_sample binding_id",
        )?;
        validate_unique(
            self.vector_composition_profiles
                .iter()
                .map(|record| record.composition_profile_id),
            "AI_VECTOR_COMPOSITION composition_profile_id",
        )?;
        validate_unique(
            self.vector_composition_components
                .iter()
                .map(|record| record.component_id),
            "AI_VECTOR_COMPOSITION component_id",
        )?;
        validate_unique(
            self.vector_arithmetic_profiles
                .iter()
                .map(|record| record.arithmetic_profile_id),
            "AI_VECTOR_COMPOSITION arithmetic_profile_id",
        )?;
        validate_unique(
            self.vector_indexes
                .iter()
                .map(|record| record.vector_index_id),
            "AI_VECTOR_INDEX vector_index_id",
        )?;
        validate_unique(
            self.vector_payload_blocks
                .iter()
                .map(|record| record.block_id),
            "AI_VECTOR_PAYLOAD_BLOCK block_id",
        )?;
        validate_unique_u64(
            self.vector_entries.iter().map(|record| record.vector_ref),
            "AI_VECTOR_DIRECTORY vector_ref",
        )?;

        for payload_ref in &self.payload_refs {
            payload_ref.validate_storage(sections, file_len)?;
        }

        let payload_ref_ids = self
            .payload_refs
            .iter()
            .map(|record| record.payload_ref)
            .collect::<BTreeSet<_>>();
        let string_ref_ids = self
            .strings
            .iter()
            .map(|record| record.string_ref)
            .collect::<BTreeSet<_>>();
        let digest_ref_ids = self
            .digests
            .iter()
            .map(|record| record.digest_ref)
            .collect::<BTreeSet<_>>();
        let source_binding_ids = self
            .source_bindings
            .iter()
            .map(|record| record.source_binding_id)
            .collect::<BTreeSet<_>>();
        let policy_ref_ids = self
            .policies
            .iter()
            .map(|record| record.policy_ref)
            .collect::<BTreeSet<_>>();
        let integrity_ids = self
            .payload_integrity
            .iter()
            .map(|record| record.integrity_ref)
            .collect::<BTreeSet<_>>();
        let transform_ids = self
            .transforms
            .iter()
            .map(|record| record.transform_ref)
            .collect::<BTreeSet<_>>();
        let vector_space_ids = self
            .vector_spaces
            .iter()
            .map(|record| record.vector_space_id)
            .collect::<BTreeSet<_>>();
        let vector_ref_ids = self
            .vector_entries
            .iter()
            .map(|record| record.vector_ref)
            .collect::<BTreeSet<_>>();
        let vector_block_ids = self
            .vector_payload_blocks
            .iter()
            .map(|record| record.block_id)
            .collect::<BTreeSet<_>>();
        let tokenizer_profile_ids = self
            .tokenizer_profiles
            .iter()
            .map(|record| record.tokenizer_profile_id)
            .collect::<BTreeSet<_>>();
        let token_block_ids = self
            .token_blocks
            .iter()
            .map(|record| record.token_block_id)
            .collect::<BTreeSet<_>>();
        let chunk_ids = self
            .text_chunks
            .iter()
            .map(|record| record.chunk_id)
            .collect::<BTreeSet<_>>();
        let chunk_profile_ids = self
            .chunk_profiles
            .iter()
            .map(|record| record.chunk_profile_id)
            .collect::<BTreeSet<_>>();
        let token_sequence_pack_ids = self
            .token_sequence_packs
            .iter()
            .map(|record| record.sequence_pack_id)
            .collect::<BTreeSet<_>>();
        let tokenized_span_ids = self
            .tokenized_spans
            .iter()
            .map(|record| record.tokenized_span_id)
            .collect::<BTreeSet<_>>();
        let training_profile_ids = self
            .training_profiles
            .iter()
            .map(|record| record.training_profile_id)
            .collect::<BTreeSet<_>>();
        let training_sample_ids = self
            .training_samples
            .iter()
            .map(|record| record.sample_id)
            .collect::<BTreeSet<_>>();
        let split_ids = self
            .dataset_splits
            .iter()
            .map(|record| record.split_id)
            .collect::<BTreeSet<_>>();
        let dedup_group_ids = self
            .dedup_groups
            .iter()
            .map(|record| record.dedup_group_id)
            .collect::<BTreeSet<_>>();
        let training_label_ids = self
            .training_labels
            .iter()
            .map(|record| record.label_id)
            .collect::<BTreeSet<_>>();
        let generator_provenance_ids = self
            .generator_provenance
            .iter()
            .map(|record| record.generator_provenance_id)
            .collect::<BTreeSet<_>>();
        let model_actor_ids = self
            .model_actors
            .iter()
            .map(|record| record.model_actor_id)
            .collect::<BTreeSet<_>>();
        let decoding_profile_ids = self
            .generation_decoding_profiles
            .iter()
            .map(|record| record.decoding_profile_id)
            .collect::<BTreeSet<_>>();
        let human_review_ids = self
            .human_reviews
            .iter()
            .map(|record| record.human_review_id)
            .collect::<BTreeSet<_>>();
        let section_ids = sections
            .iter()
            .map(|section| section.entry.section_id)
            .collect::<BTreeSet<_>>();
        let tensor_layout_ids = self
            .tensor_layouts
            .iter()
            .map(|record| record.tensor_layout_id)
            .collect::<BTreeSet<_>>();
        let asset_ref_ids = self
            .assets
            .iter()
            .map(|record| record.asset_ref_id)
            .collect::<BTreeSet<_>>();
        let multimodal_sequence_pack_ids = self
            .multimodal_sequence_packs
            .iter()
            .map(|record| record.sequence_pack_id)
            .collect::<BTreeSet<_>>();
        let composition_profile_ids = self
            .vector_composition_profiles
            .iter()
            .map(|record| record.composition_profile_id)
            .collect::<BTreeSet<_>>();
        let composition_component_ids = self
            .vector_composition_components
            .iter()
            .map(|record| record.component_id)
            .collect::<BTreeSet<_>>();
        let arithmetic_profile_ids = self
            .vector_arithmetic_profiles
            .iter()
            .map(|record| record.arithmetic_profile_id)
            .collect::<BTreeSet<_>>();

        for string in &self.strings {
            string.validate(self)?;
        }
        for digest in &self.digests {
            digest.validate(self)?;
        }
        for policy in &self.policies {
            policy.validate(&string_ref_ids, &payload_ref_ids, &digest_ref_ids)?;
        }
        for source_span in &self.source_spans {
            source_span.validate(&source_binding_ids)?;
        }
        for transform in &self.transforms {
            transform.validate(&string_ref_ids, &payload_ref_ids, &digest_ref_ids)?;
        }
        for privacy in &self.privacy_summaries {
            privacy.validate(&source_binding_ids, &payload_ref_ids, &policy_ref_ids)?;
        }
        for binding in &self.section_feature_bindings {
            binding.validate(sections)?;
        }
        for companion in &self.companion_artifacts {
            companion.validate(&string_ref_ids, &digest_ref_ids, &source_binding_ids)?;
        }
        for source_binding in &self.source_bindings {
            source_binding.validate(
                &string_ref_ids,
                &digest_ref_ids,
                &payload_ref_ids,
                &policy_ref_ids,
            )?;
        }
        for integrity in &self.payload_integrity {
            integrity.validate(self, &payload_ref_ids)?;
        }

        for profile in &self.chunk_profiles {
            profile.validate(&string_ref_ids, &tokenizer_profile_ids)?;
        }
        self.validate_text_chunks(
            &source_binding_ids,
            &digest_ref_ids,
            &payload_ref_ids,
            &policy_ref_ids,
        )?;
        for profile in &self.tokenizer_profiles {
            profile.validate(
                &string_ref_ids,
                &digest_ref_ids,
                &payload_ref_ids,
                &policy_ref_ids,
            )?;
        }
        for token_block in &self.token_blocks {
            token_block.validate(
                self,
                sections,
                &tokenizer_profile_ids,
                &payload_ref_ids,
                &integrity_ids,
            )?;
        }
        for span in &self.tokenized_spans {
            span.validate(
                self,
                &tokenizer_profile_ids,
                &token_block_ids,
                &chunk_ids,
                &payload_ref_ids,
                &digest_ref_ids,
            )?;
        }
        for pack in &self.token_sequence_packs {
            pack.validate(
                self,
                &tokenizer_profile_ids,
                &token_block_ids,
                &payload_ref_ids,
                &training_profile_ids,
                &split_ids,
                &tokenized_span_ids,
            )?;
        }
        for profile in &self.training_profiles {
            profile.validate(
                &string_ref_ids,
                &payload_ref_ids,
                &policy_ref_ids,
                &chunk_profile_ids,
                &tokenizer_profile_ids,
                &vector_space_ids,
                &generator_provenance_ids,
            )?;
        }
        for split in &self.dataset_splits {
            split.validate(&string_ref_ids, &payload_ref_ids, &policy_ref_ids)?;
        }
        for dedup_group in &self.dedup_groups {
            dedup_group.validate(&policy_ref_ids, &training_sample_ids)?;
        }
        for sample in &self.training_samples {
            sample.validate(
                &training_profile_ids,
                &split_ids,
                &dedup_group_ids,
                &token_sequence_pack_ids,
                &multimodal_sequence_pack_ids,
                &vector_ref_ids,
                &training_label_ids,
                &generator_provenance_ids,
                &payload_ref_ids,
                &policy_ref_ids,
                &model_actor_ids,
            )?;
        }
        for epoch_plan in &self.training_epoch_plans {
            epoch_plan.validate(&training_profile_ids, &split_ids, &payload_ref_ids)?;
        }
        for label in &self.training_labels {
            label.validate(
                &payload_ref_ids,
                &policy_ref_ids,
                &generator_provenance_ids,
                &human_review_ids,
            )?;
        }
        for pair in &self.preference_pairs {
            pair.validate(
                &payload_ref_ids,
                &policy_ref_ids,
                &generator_provenance_ids,
                &human_review_ids,
            )?;
        }
        for actor in &self.model_actors {
            actor.validate(&string_ref_ids, &digest_ref_ids, &policy_ref_ids)?;
        }
        for profile in &self.generation_decoding_profiles {
            profile.validate(&payload_ref_ids, &policy_ref_ids)?;
        }
        for review in &self.human_reviews {
            review.validate(&string_ref_ids, &payload_ref_ids, &policy_ref_ids)?;
        }
        for provenance in &self.generator_provenance {
            provenance.validate(
                &generator_provenance_ids,
                &model_actor_ids,
                &decoding_profile_ids,
                &human_review_ids,
                &payload_ref_ids,
                &policy_ref_ids,
                &training_sample_ids,
            )?;
        }
        for tensor_layout in &self.tensor_layouts {
            tensor_layout.validate(&string_ref_ids, &payload_ref_ids)?;
        }
        for transfer_hint in &self.device_transfer_hints {
            transfer_hint.validate(&string_ref_ids)?;
        }
        for asset in &self.assets {
            asset.validate(
                &asset_ref_ids,
                &tensor_layout_ids,
                &section_ids,
                &string_ref_ids,
                &digest_ref_ids,
                &transform_ids,
                &policy_ref_ids,
            )?;
        }
        self.validate_multimodal_sequences(
            &training_profile_ids,
            &tokenizer_profile_ids,
            &split_ids,
            &payload_ref_ids,
            &policy_ref_ids,
            &training_label_ids,
            &generator_provenance_ids,
            &tokenized_span_ids,
            &token_sequence_pack_ids,
            &asset_ref_ids,
            &tensor_layout_ids,
            &vector_ref_ids,
            &multimodal_sequence_pack_ids,
        )?;
        for vector_space in &self.vector_spaces {
            vector_space.validate(
                &string_ref_ids,
                &digest_ref_ids,
                &transform_ids,
                &tokenizer_profile_ids,
                &chunk_profile_ids,
            )?;
        }
        for compatibility in &self.vector_space_compatibilities {
            compatibility.validate(&vector_space_ids, &transform_ids, &payload_ref_ids)?;
        }
        for block in &self.vector_payload_blocks {
            block.validate(self, sections, &payload_ref_ids, &integrity_ids)?;
        }
        for entry in &self.vector_entries {
            entry.validate(self, &vector_block_ids, &integrity_ids)?;
        }
        for binding in &self.filecode_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &source_binding_ids,
                &digest_ref_ids,
                &string_ref_ids,
                &vector_ref_ids,
            )?;
        }
        for binding in &self.chunk_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &chunk_ids,
                &chunk_profile_ids,
                &digest_ref_ids,
                &vector_ref_ids,
            )?;
        }
        for binding in &self.object_state_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &composition_profile_ids,
                &source_binding_ids,
                &digest_ref_ids,
                &string_ref_ids,
                &vector_ref_ids,
            )?;
        }
        for binding in &self.training_sample_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &training_profile_ids,
                &training_sample_ids,
                &source_binding_ids,
                &digest_ref_ids,
                &vector_ref_ids,
            )?;
        }
        for arithmetic in &self.vector_arithmetic_profiles {
            arithmetic.validate(&string_ref_ids)?;
        }
        for component in &self.vector_composition_components {
            component.validate(&vector_space_ids)?;
        }
        for profile in &self.vector_composition_profiles {
            profile.validate(
                &string_ref_ids,
                &payload_ref_ids,
                &vector_space_ids,
                &arithmetic_profile_ids,
                &composition_component_ids,
            )?;
        }
        for index in &self.vector_indexes {
            index.validate(&self.vector_spaces, &payload_ref_ids, &policy_ref_ids)?;
        }
        Ok(())
    }

    fn validate_text_chunks(
        &self,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        let chunk_ids = self
            .text_chunks
            .iter()
            .map(|record| record.chunk_id)
            .collect::<BTreeSet<_>>();
        for chunk in &self.text_chunks {
            chunk.validate_refs(
                &chunk_ids,
                source_binding_ids,
                digest_ref_ids,
                payload_ref_ids,
                policy_ref_ids,
            )?;
            if let Some(previous) = self
                .text_chunks
                .iter()
                .find(|candidate| candidate.chunk_id == chunk.previous_chunk_id)
            {
                if previous.next_chunk_id != 0 && previous.next_chunk_id != chunk.chunk_id {
                    return Err(CoveError::BadSection(format!(
                        "TextChunkEntry {} previous_chunk_id {} does not point back",
                        chunk.chunk_id, chunk.previous_chunk_id
                    )));
                }
            }
            if let Some(next) = self
                .text_chunks
                .iter()
                .find(|candidate| candidate.chunk_id == chunk.next_chunk_id)
            {
                if next.previous_chunk_id != 0 && next.previous_chunk_id != chunk.chunk_id {
                    return Err(CoveError::BadSection(format!(
                        "TextChunkEntry {} next_chunk_id {} does not point back",
                        chunk.chunk_id, chunk.next_chunk_id
                    )));
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_multimodal_sequences(
        &self,
        training_profile_ids: &BTreeSet<u32>,
        tokenizer_profile_ids: &BTreeSet<u32>,
        split_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
        training_label_ids: &BTreeSet<u64>,
        generator_provenance_ids: &BTreeSet<u64>,
        tokenized_span_ids: &BTreeSet<u64>,
        token_sequence_pack_ids: &BTreeSet<u64>,
        asset_ref_ids: &BTreeSet<u64>,
        tensor_layout_ids: &BTreeSet<u32>,
        vector_ref_ids: &BTreeSet<u64>,
        multimodal_sequence_pack_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
        for pack in &self.multimodal_sequence_packs {
            if pack.sequence_pack_id == 0 {
                return Err(CoveError::BadSection(
                    "MultimodalSequencePackV1 sequence_pack_id must be non-zero".into(),
                ));
            }
            if pack.training_profile_id != 0
                && !training_profile_ids.contains(&pack.training_profile_id)
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} references missing training_profile_id {}",
                    pack.sequence_pack_id, pack.training_profile_id
                )));
            }
            if pack.tokenizer_profile_id != 0
                && !tokenizer_profile_ids.contains(&pack.tokenizer_profile_id)
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} references missing tokenizer_profile_id {}",
                    pack.sequence_pack_id, pack.tokenizer_profile_id
                )));
            }
            if pack.split_ref != 0 && !split_ids.contains(&pack.split_ref) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} references missing split_ref {}",
                    pack.sequence_pack_id, pack.split_ref
                )));
            }
            if pack.sequence_profile_ref != 0
                && !payload_ref_ids.contains(&pack.sequence_profile_ref)
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} references missing sequence_profile_ref {}",
                    pack.sequence_pack_id, pack.sequence_profile_ref
                )));
            }
            if pack.sample_weight_ppm > 1_000_000 {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} sample_weight_ppm exceeds 1_000_000",
                    pack.sequence_pack_id
                )));
            }
            if (pack.element_count == 0) != (pack.first_element_ref == 0) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} first_element_ref/element_count mismatch",
                    pack.sequence_pack_id
                )));
            }
            for (label, payload_ref) in [
                ("loss_mask_ref", pack.loss_mask_ref),
                ("attention_mask_ref", pack.attention_mask_ref),
                ("position_map_ref", pack.position_map_ref),
            ] {
                if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                    return Err(CoveError::BadSection(format!(
                        "MultimodalSequencePack {} references missing {label} {}",
                        pack.sequence_pack_id, payload_ref
                    )));
                }
            }
            if pack.label_ref != 0 && !training_label_ids.contains(&u64::from(pack.label_ref)) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} references missing label_ref {}",
                    pack.sequence_pack_id, pack.label_ref
                )));
            }
            if pack.generator_provenance_ref != 0
                && !generator_provenance_ids.contains(&pack.generator_provenance_ref)
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} references missing generator_provenance_ref {}",
                    pack.sequence_pack_id, pack.generator_provenance_ref
                )));
            }
            for (label, payload_ref) in [
                ("source_snapshot_ref", pack.source_snapshot_ref),
                ("evidence_ref", pack.evidence_ref),
            ] {
                if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                    return Err(CoveError::BadSection(format!(
                        "MultimodalSequencePack {} references missing {label} {}",
                        pack.sequence_pack_id, payload_ref
                    )));
                }
            }
            let actual_count = self
                .multimodal_sequence_elements
                .iter()
                .filter(|element| element.sequence_pack_id == pack.sequence_pack_id)
                .count();
            if usize::try_from(pack.element_count).map_err(|_| CoveError::ArithOverflow)?
                != actual_count
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} element_count does not match elements",
                    pack.sequence_pack_id
                )));
            }
            if pack.first_element_ref != 0
                && !self.multimodal_sequence_elements.iter().any(|element| {
                    element.sequence_pack_id == pack.sequence_pack_id
                        && element.element_id == u64::from(pack.first_element_ref)
                })
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequencePack {} references missing first_element_ref {}",
                    pack.sequence_pack_id, pack.first_element_ref
                )));
            }
        }

        let mut ordinals = BTreeSet::new();
        for element in &self.multimodal_sequence_elements {
            if element.element_id == 0 {
                return Err(CoveError::BadSection(
                    "MultimodalSequenceElementV1 element_id must be non-zero".into(),
                ));
            }
            if !multimodal_sequence_pack_ids.contains(&element.sequence_pack_id) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement {} references missing sequence_pack_id {}",
                    element.element_id, element.sequence_pack_id
                )));
            }
            if !ordinals.insert((element.sequence_pack_id, element.ordinal)) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement duplicate ordinal {} in sequence_pack {}",
                    element.ordinal, element.sequence_pack_id
                )));
            }
            validate_ai_multimodal_element_kind(
                element.element_kind,
                "MultimodalSequenceElement element_kind",
            )?;
            validate_ai_modality(element.modality, "MultimodalSequenceElement modality")?;
            validate_ai_role(element.role, "MultimodalSequenceElement role")?;
            if element.tokenized_span_ref != 0
                && !tokenized_span_ids.contains(&element.tokenized_span_ref)
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement {} references missing tokenized_span_ref {}",
                    element.element_id, element.tokenized_span_ref
                )));
            }
            if element.token_sequence_pack_ref != 0
                && !token_sequence_pack_ids.contains(&element.token_sequence_pack_ref)
            {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement {} references missing token_sequence_pack_ref {}",
                    element.element_id, element.token_sequence_pack_ref
                )));
            }
            if element.asset_ref != 0 && !asset_ref_ids.contains(&element.asset_ref) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement {} references missing asset_ref {}",
                    element.element_id, element.asset_ref
                )));
            }
            if element.tensor_ref != 0 {
                let tensor_ref = u32::try_from(element.tensor_ref).map_err(|_| {
                    CoveError::BadSection("tensor_ref exceeds u32 layout id".into())
                })?;
                if !tensor_layout_ids.contains(&tensor_ref) {
                    return Err(CoveError::BadSection(format!(
                        "MultimodalSequenceElement {} references missing tensor_ref {}",
                        element.element_id, element.tensor_ref
                    )));
                }
            }
            if element.vector_ref != 0 && !vector_ref_ids.contains(&element.vector_ref) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement {} references missing vector_ref {}",
                    element.element_id, element.vector_ref
                )));
            }
            for (label, payload_ref) in [
                ("position_stream_ref", element.position_stream_ref),
                ("evidence_ref", element.evidence_ref),
            ] {
                if payload_ref != 0 && !payload_ref_ids.contains(&payload_ref) {
                    return Err(CoveError::BadSection(format!(
                        "MultimodalSequenceElement {} references missing {label} {}",
                        element.element_id, payload_ref
                    )));
                }
            }
            if element.policy_ref != 0 && !policy_ref_ids.contains(&element.policy_ref) {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement {} references missing policy_ref {}",
                    element.element_id, element.policy_ref
                )));
            }
            element
                .byte_start
                .checked_add(element.byte_length)
                .ok_or(CoveError::ArithOverflow)?;
            if element.time_duration_us < 0 {
                return Err(CoveError::BadSection(format!(
                    "MultimodalSequenceElement {} time_duration_us must be non-negative",
                    element.element_id
                )));
            }
            element
                .time_start_us
                .checked_add(element.time_duration_us)
                .ok_or(CoveError::ArithOverflow)?;
        }
        Ok(())
    }

    fn payload_ref(&self, payload_ref: u32) -> Option<&AiPayloadRefEntryV1> {
        self.payload_refs
            .iter()
            .find(|record| record.payload_ref == payload_ref)
    }

    fn integrity_ref(&self, integrity_ref: u32) -> Option<&AiPayloadIntegrityV1> {
        self.payload_integrity
            .iter()
            .find(|record| record.integrity_ref == integrity_ref)
    }

    fn digest_ref(&self, digest_ref: u32) -> Option<&AiDigestEntryV1> {
        self.digests
            .iter()
            .find(|record| record.digest_ref == digest_ref)
    }

    fn vector_block(&self, block_id: u32) -> Option<&VectorPayloadBlockHeaderV1> {
        self.vector_payload_blocks
            .iter()
            .find(|record| record.block_id == block_id)
    }
}

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
        if header.record_version != 1 {
            return Err(CoveError::BadSection(format!(
                "unsupported COVE-AI record_version {}",
                header.record_version
            )));
        }
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
    fn validate(&self, tables: &AiDescriptorTablesV1) -> Result<(), CoveError> {
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
    fn validate(&self, tables: &AiDescriptorTablesV1) -> Result<(), CoveError> {
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
    fn validate_storage(&self, sections: &[CoveAiSection], file_len: u64) -> Result<(), CoveError> {
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

    fn validate_token_or_vector_payload_carrier(
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
    fn validate(
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
    fn validate(&self, source_binding_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
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
    fn validate(
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
    fn validate(
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
    fn validate(
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
    fn validate(
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
    fn validate(
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
    fn validate(&self, sections: &[CoveAiSection]) -> Result<(), CoveError> {
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
        if self.required_ai_features & !AI_KNOWN_FEATURES_V1 != 0 {
            return Err(CoveError::BadSection(format!(
                "AiSectionFeatureBinding {} requires unknown COVE-AI features",
                self.binding_ref
            )));
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(
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
    fn validate_refs(
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(
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
    fn validate(
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
        let payload_ref = tables.payload_ref(self.payload_ref).unwrap();
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
            let integrity = tables.integrity_ref(self.integrity_ref).unwrap();
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
    fn validate(
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
            .unwrap();
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

impl TokenSequencePackV1 {
    fn validate(
        &self,
        tables: &AiDescriptorTablesV1,
        tokenizer_profile_ids: &BTreeSet<u32>,
        token_block_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        training_profile_ids: &BTreeSet<u32>,
        split_ids: &BTreeSet<u32>,
        tokenized_span_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
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
            .unwrap();
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
            let payload = tables.payload_ref(payload_ref).unwrap();
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
    fn validate_static(&self) -> Result<(), CoveError> {
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
    fn validate(
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

impl TrainingSampleEntryV1 {
    fn validate(
        &self,
        training_profile_ids: &BTreeSet<u32>,
        split_ids: &BTreeSet<u32>,
        dedup_group_ids: &BTreeSet<u64>,
        token_sequence_pack_ids: &BTreeSet<u64>,
        multimodal_sequence_pack_ids: &BTreeSet<u64>,
        vector_ref_ids: &BTreeSet<u64>,
        training_label_ids: &BTreeSet<u64>,
        generator_provenance_ids: &BTreeSet<u64>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
        model_actor_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(
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
    fn validate(
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
    fn validate(
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
    fn validate(
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

impl GeneratorProvenanceV1 {
    fn validate(
        &self,
        generator_provenance_ids: &BTreeSet<u64>,
        model_actor_ids: &BTreeSet<u32>,
        decoding_profile_ids: &BTreeSet<u32>,
        human_review_ids: &BTreeSet<u32>,
        payload_ref_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
        training_sample_ids: &BTreeSet<u64>,
    ) -> Result<(), CoveError> {
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
    fn validate_static(&self) -> Result<(), CoveError> {
        if self.model_actor_id == 0 {
            return Err(CoveError::BadSection(
                "ModelActorDescriptorV1 model_actor_id must be non-zero".into(),
            ));
        }
        Ok(())
    }

    fn validate(
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(
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
    fn validate_static(&self) -> Result<(), CoveError> {
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

    fn validate(&self, string_ref_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
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

impl AiAssetRefV1 {
    fn validate(
        &self,
        asset_ref_ids: &BTreeSet<u64>,
        tensor_layout_ids: &BTreeSet<u32>,
        section_ids: &BTreeSet<u32>,
        string_ref_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
        transform_ids: &BTreeSet<u32>,
        policy_ref_ids: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
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
    fn validate(
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
    fn validate(
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
    pub flags: u32,
    pub checksum: u32,
}

impl FileCodeVectorBindingV1 {
    fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
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
    pub flags: u32,
    pub checksum: u32,
}

impl ChunkVectorBindingV1 {
    fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        chunk_ids: &BTreeSet<u64>,
        chunk_profile_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
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
    pub flags: u32,
    pub checksum: u32,
}

impl ObjectStateVectorBindingV1 {
    fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        composition_profile_ids: &BTreeSet<u32>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
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
        self.csn.checked_add(0).ok_or(CoveError::ArithOverflow)?;
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
    pub flags: u32,
    pub checksum: u32,
}

impl TrainingSampleVectorBindingV1 {
    fn validate(
        &self,
        vector_space_ids: &BTreeSet<u32>,
        training_profile_ids: &BTreeSet<u32>,
        training_sample_ids: &BTreeSet<u64>,
        source_binding_ids: &BTreeSet<u32>,
        digest_ref_ids: &BTreeSet<u32>,
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
    fn validate(
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
        let payload_ref = tables.payload_ref(self.payload_ref).unwrap();
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
            let integrity = tables.integrity_ref(self.integrity_ref).unwrap();
            if integrity.payload_ref != self.payload_ref {
                return Err(CoveError::BadSection(format!(
                    "VectorPayloadBlock {} integrity_ref payload_ref mismatch",
                    self.block_id
                )));
            }
        }
        Ok(())
    }

    fn dense_vector_width(&self) -> Option<u64> {
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
    fn validate(
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
        let block = tables.vector_block(self.block_id).unwrap();
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
    fn validate(
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
    fn validate(&self, vector_space_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
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
    fn validate(&self, string_ref_ids: &BTreeSet<u32>) -> Result<(), CoveError> {
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
    fn validate(
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiWritableSection {
    pub section_id: u32,
    pub section_kind: u32,
    pub profile_kind: u8,
    pub payload_encoding: AiPayloadEncodingV1,
    pub requiredness_scope: AiRequirednessScopeV1,
    pub source_binding_ref: u32,
    pub required_ai_features: u64,
    pub optional_ai_features: u64,
    pub feature_binding_ref: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveVecFileCodeVectorBuild {
    pub artifact_id: [u8; 16],
    pub created_at_us: i64,
    pub dimension_count: u32,
    pub file_codes: Vec<u32>,
    pub vector_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiDescriptorBundleBuild {
    pub artifact_id: [u8; 16],
    pub created_at_us: i64,
    pub payload_sections: Vec<CoveAiWritableSection>,
    pub descriptor_tables: AiDescriptorTablesV1,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExactFlatFileCodeVectorSearchResult {
    pub file_code: u32,
    pub vector_ref: u64,
    pub vector_space_id: u32,
    /// Larger is better. For distance metrics this is the negative distance.
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileCodeEmbeddingV1 {
    pub file_code: u32,
    pub vector_ref: u64,
    pub vector_space_id: u32,
    pub dimension_count: u32,
    pub values: Vec<f32>,
}

pub fn exact_flat_filecode_vector_search(
    artifact_bytes: &[u8],
    query: &[f32],
    top_k: usize,
) -> Result<Vec<ExactFlatFileCodeVectorSearchResult>, CoveError> {
    if query.is_empty() {
        return Err(CoveError::BadSection(
            "exact flat vector search requires a non-empty query vector".into(),
        ));
    }
    for value in query {
        if !value.is_finite() {
            return Err(CoveError::BadSection(
                "exact flat vector search query contains non-finite values".into(),
            ));
        }
    }

    let sidecar = exact_flat_parse_covev_with_payload_access(artifact_bytes)?;

    let query_dimension = u32::try_from(query.len()).map_err(|_| CoveError::ArithOverflow)?;
    let matching_spaces = sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .filter(|space| space.dimension_count == query_dimension)
        .collect::<Vec<_>>();
    let vector_space = match matching_spaces.as_slice() {
        [space] => *space,
        [] => {
            return Err(CoveError::BadSection(format!(
                "no COVE-VEC vector space matches query dimension {query_dimension}"
            )));
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "multiple COVE-VEC vector spaces match query dimension {query_dimension}; exact flat search is ambiguous"
            )));
        }
    };
    exact_flat_validate_vector_space(vector_space)?;

    exact_flat_filecode_vector_search_in_space(artifact_bytes, &sidecar, vector_space, query, top_k)
}

pub fn exact_flat_filecode_vector_search_by_file_code(
    artifact_bytes: &[u8],
    query_file_code: u32,
    top_k: usize,
) -> Result<Vec<ExactFlatFileCodeVectorSearchResult>, CoveError> {
    let sidecar = exact_flat_parse_covev_with_payload_access(artifact_bytes)?;
    let (_query_binding, vector_space, query_entry) =
        exact_flat_filecode_binding_parts(&sidecar, query_file_code)?;
    let query = exact_flat_vector_entry_f32(artifact_bytes, &sidecar, vector_space, query_entry)?;
    exact_flat_filecode_vector_search_in_space(
        artifact_bytes,
        &sidecar,
        vector_space,
        &query,
        top_k,
    )
}

pub fn filecode_embedding(
    artifact_bytes: &[u8],
    file_code: u32,
) -> Result<FileCodeEmbeddingV1, CoveError> {
    let sidecar = exact_flat_parse_covev_with_payload_access(artifact_bytes)?;
    let (binding, vector_space, vector_entry) =
        exact_flat_filecode_binding_parts(&sidecar, file_code)?;
    let values = exact_flat_vector_entry_f32(artifact_bytes, &sidecar, vector_space, vector_entry)?;
    Ok(FileCodeEmbeddingV1 {
        file_code,
        vector_ref: binding.vector_ref,
        vector_space_id: vector_space.vector_space_id,
        dimension_count: vector_space.dimension_count,
        values,
    })
}

fn exact_flat_filecode_binding_parts<'a>(
    sidecar: &'a CoveAiFile,
    file_code: u32,
) -> Result<
    (
        &'a FileCodeVectorBindingV1,
        &'a VectorSpaceDescriptorV1,
        &'a VectorEntryV1,
    ),
    CoveError,
> {
    let matching_bindings = sidecar
        .descriptor_tables
        .filecode_vector_bindings
        .iter()
        .filter(|binding| binding.file_code == file_code)
        .collect::<Vec<_>>();
    let binding = match matching_bindings.as_slice() {
        [binding] => *binding,
        [] => {
            return Err(CoveError::BadSection(format!(
                "query FileCode {file_code} is not present in COVE-VEC FileCode vector bindings"
            )));
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "query FileCode {file_code} has multiple COVE-VEC FileCode vector bindings"
            )));
        }
    };
    let vector_space = sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .find(|space| space.vector_space_id == binding.vector_space_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_space_id {}",
                binding.binding_id, binding.vector_space_id
            ))
        })?;
    exact_flat_validate_vector_space(vector_space)?;
    let vector_entry = sidecar
        .descriptor_tables
        .vector_entries
        .iter()
        .find(|entry| entry.vector_ref == binding.vector_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_ref {}",
                binding.binding_id, binding.vector_ref
            ))
        })?;
    Ok((binding, vector_space, vector_entry))
}

fn exact_flat_filecode_vector_search_in_space(
    artifact_bytes: &[u8],
    sidecar: &CoveAiFile,
    vector_space: &VectorSpaceDescriptorV1,
    query: &[f32],
    top_k: usize,
) -> Result<Vec<ExactFlatFileCodeVectorSearchResult>, CoveError> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    let mut seen_file_codes = BTreeSet::new();
    let mut results = Vec::new();
    for binding in &sidecar.descriptor_tables.filecode_vector_bindings {
        if binding.vector_space_id != vector_space.vector_space_id {
            continue;
        }
        if !seen_file_codes.insert(binding.file_code) {
            return Err(CoveError::BadSection(format!(
                "duplicate FileCode {} in FileCode vector bindings",
                binding.file_code
            )));
        }
        let vector_entry = sidecar
            .descriptor_tables
            .vector_entries
            .iter()
            .find(|entry| entry.vector_ref == binding.vector_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "FileCodeVectorBinding {} references missing vector_ref {}",
                    binding.binding_id, binding.vector_ref
                ))
            })?;
        let vector =
            exact_flat_vector_entry_f32(artifact_bytes, &sidecar, vector_space, vector_entry)?;
        let score = exact_flat_metric_score(vector_space.metric, query, &vector)?;
        results.push(ExactFlatFileCodeVectorSearchResult {
            file_code: binding.file_code,
            vector_ref: binding.vector_ref,
            vector_space_id: vector_space.vector_space_id,
            score,
        });
    }

    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.file_code.cmp(&right.file_code))
            .then_with(|| left.vector_ref.cmp(&right.vector_ref))
    });
    results.truncate(top_k.min(results.len()));
    Ok(results)
}

pub fn write_covev_filecode_vectors(
    build: &CoveVecFileCodeVectorBuild,
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

    let row_width = u64::from(build.dimension_count)
        .checked_mul(4)
        .ok_or(CoveError::ArithOverflow)?;
    let row_width_u32 = u32::try_from(row_width).map_err(|_| CoveError::ArithOverflow)?;
    let expected_payload_len = row_width
        .checked_mul(u64::try_from(build.file_codes.len()).map_err(|_| CoveError::ArithOverflow)?)
        .ok_or(CoveError::ArithOverflow)?;
    if build.vector_payload.len() as u64 != expected_payload_len {
        return Err(CoveError::BadSection(format!(
            "COVE-VEC payload length {} does not match {} FileCodes * {} dimensions * 4 bytes",
            build.vector_payload.len(),
            build.file_codes.len(),
            build.dimension_count
        )));
    }

    let vector_payload_crc32c = checksum::crc32c(&build.vector_payload);
    let digest_offset =
        u64::try_from(build.vector_payload.len()).map_err(|_| CoveError::ArithOverflow)?;
    let mut payload_bytes = build.vector_payload.clone();
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
    ])?;

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
    })?])?;

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
    })?])?;

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
    })?])?;

    let vector_payload_block =
        encode_records([encode_vector_payload_block(VectorPayloadBlockHeaderV1 {
            block_id: 1,
            vector_space_id: 1,
            vector_count: build.file_codes.len() as u64,
            dimension_count: build.dimension_count,
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
            integrity_ref: 1,
            checksum: 0,
        })?])?;

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
            flags: 0,
            checksum: 0,
        })?);
    }

    let sections = vec![
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
            encode_records(vector_entries)?,
        ),
        coveai_binary_section(
            8,
            SectionKind::AiVectorBinding,
            PrimaryProfile::CoveVec,
            encode_records(filecode_bindings)?,
        ),
    ];

    let bytes = write_coveai_artifact(
        CoveAiArtifactKind::CoveVec,
        build.artifact_id,
        build.created_at_us,
        &sections,
    )?;
    CoveAiFile::parse(&bytes)?;
    Ok(bytes)
}

fn exact_flat_parse_covev_with_payload_access(
    artifact_bytes: &[u8],
) -> Result<CoveAiFile, CoveError> {
    let sidecar = CoveAiFile::parse(artifact_bytes)?;
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

fn exact_flat_validate_vector_space(
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

fn exact_flat_vector_entry_f32(
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
        let value = f32::from_le_bytes(chunk.try_into().map_err(|_| CoveError::BufferTooShort)?);
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

fn exact_flat_payload_ref_bytes<'a>(
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

fn exact_flat_verify_payload_integrity(
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

fn exact_flat_metric_score(metric: u8, query: &[f32], vector: &[f32]) -> Result<f32, CoveError> {
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
            encode_records(records)?,
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

fn coveai_binary_section(
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

fn coveai_payload_section(
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

fn optional_features_for_ai_section(section_kind: SectionKind) -> u64 {
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

fn encode_records(records: impl IntoIterator<Item = Vec<u8>>) -> Result<Vec<u8>, CoveError> {
    let mut out = Vec::new();
    for record in records {
        out.extend_from_slice(&record);
    }
    Ok(out)
}

fn encode_ai_record(
    record_kind: u16,
    local_id: u64,
    flags: u32,
    payload: Vec<u8>,
) -> Result<Vec<u8>, CoveError> {
    let record_len = AI_RECORD_HEADER_LEN
        .checked_add(payload.len())
        .ok_or(CoveError::ArithOverflow)?;
    let record_len_u32 = u32::try_from(record_len).map_err(|_| CoveError::ArithOverflow)?;
    let mut out = vec![0u8; record_len];
    put_u16(&mut out, 0, record_kind);
    put_u16(&mut out, 2, 1);
    put_u32(&mut out, 4, record_len_u32);
    put_u64(&mut out, 8, local_id);
    put_u32(&mut out, 16, flags);
    out[AI_RECORD_HEADER_LEN..].copy_from_slice(&payload);
    let crc32c = checksum_with_zeroed_field(&out, 20)?;
    put_u32(&mut out, 20, crc32c);
    Ok(out)
}

fn with_payload_crc32c(mut payload: Vec<u8>, checksum_offset: usize) -> Result<Vec<u8>, CoveError> {
    let crc32c = checksum_with_zeroed_field(&payload, checksum_offset)?;
    put_u32(&mut payload, checksum_offset, crc32c);
    Ok(payload)
}

fn checksum_with_zeroed_field(bytes: &[u8], checksum_offset: usize) -> Result<u32, CoveError> {
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

fn encode_string_entry(record: AiStringEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_digest_entry(record: AiDigestEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_companion_artifact_ref(record: AiCompanionArtifactRefV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_source_binding(record: AiSourceBindingV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_payload_ref_entry(record: AiPayloadRefEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_policy_ref_entry(record: AiPolicyRefEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_source_span_entry(record: AiSourceSpanEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_transform_entry(record: AiTransformEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_privacy_summary(record: AiPrivacySummaryEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_payload_integrity(record: AiPayloadIntegrityV1) -> Result<Vec<u8>, CoveError> {
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
    encode_ai_record(1, u64::from(record.integrity_ref), 0, payload)
}

fn encode_section_feature_binding(record: AiSectionFeatureBindingV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_chunk_profile(record: ChunkProfileV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_text_chunk_entry(record: TextChunkEntryV1) -> Result<Vec<u8>, CoveError> {
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
    encode_ai_record(1, record.chunk_id, AI_FLAG_PAYLOAD_CRC32C_PRESENT, payload)
}

fn encode_tokenizer_profile(record: TokenizerProfileV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_token_block(record: TokenBlockHeaderV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_tokenized_span(record: TokenizedSpanV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_token_sequence_pack(record: TokenSequencePackV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_training_profile(record: TrainingProfileV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_training_sample_entry(record: TrainingSampleEntryV1) -> Result<Vec<u8>, CoveError> {
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
    encode_ai_record(1, record.sample_id, AI_FLAG_PAYLOAD_CRC32C_PRESENT, payload)
}

fn encode_dataset_split(record: DatasetSplitV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_dedup_group(record: DedupGroupV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_training_epoch_plan(record: TrainingEpochPlanV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_training_label_entry(record: TrainingLabelEntryV1) -> Result<Vec<u8>, CoveError> {
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
    encode_ai_record(1, record.label_id, AI_FLAG_PAYLOAD_CRC32C_PRESENT, payload)
}

fn encode_preference_pair_entry(record: PreferencePairEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_generator_provenance(record: GeneratorProvenanceV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_model_actor(record: ModelActorDescriptorV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_generation_decoding_profile(
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
        payload,
    )
}

fn encode_human_review(record: HumanReviewEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_tensor_layout(record: TensorLayoutDescriptorV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_device_transfer_hint(record: DeviceTransferHintV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_ai_asset_ref(record: AiAssetRefV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_multimodal_sequence_pack(record: MultimodalSequencePackV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_multimodal_sequence_element(
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
        payload,
    )
}

fn encode_vector_space(record: VectorSpaceDescriptorV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_vector_space_compatibility(
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
        payload,
    )
}

fn encode_filecode_vector_binding(record: FileCodeVectorBindingV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 80];
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
    put_u32(&mut payload, 72, record.flags);
    let payload = with_payload_crc32c(payload, 76)?;
    encode_ai_record(
        1,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        payload,
    )
}

fn encode_chunk_vector_binding(record: ChunkVectorBindingV1) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 48];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u64(&mut payload, 12, record.chunk_id);
    put_u32(&mut payload, 20, record.chunk_profile_id);
    put_u32(&mut payload, 24, record.source_value_hash_ref);
    put_u32(&mut payload, 28, record.chunk_text_hash_ref);
    put_u64(&mut payload, 32, record.vector_ref);
    put_u32(&mut payload, 40, record.flags);
    let payload = with_payload_crc32c(payload, 44)?;
    encode_ai_record(
        2,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        payload,
    )
}

fn encode_object_state_vector_binding(
    record: ObjectStateVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 69];
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
    put_u32(&mut payload, 61, record.flags);
    let payload = with_payload_crc32c(payload, 65)?;
    encode_ai_record(
        3,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        payload,
    )
}

fn encode_training_sample_vector_binding(
    record: TrainingSampleVectorBindingV1,
) -> Result<Vec<u8>, CoveError> {
    let mut payload = vec![0u8; 48];
    put_u64(&mut payload, 0, record.binding_id);
    put_u32(&mut payload, 8, record.vector_space_id);
    put_u32(&mut payload, 12, record.training_profile_ref);
    put_u64(&mut payload, 16, record.sample_id);
    put_u32(&mut payload, 24, record.source_snapshot_ref);
    put_u32(&mut payload, 28, record.sample_fingerprint_ref);
    put_u64(&mut payload, 32, record.vector_ref);
    put_u32(&mut payload, 40, record.flags);
    let payload = with_payload_crc32c(payload, 44)?;
    encode_ai_record(
        4,
        record.binding_id,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        payload,
    )
}

fn encode_vector_payload_block(record: VectorPayloadBlockHeaderV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_vector_entry(record: VectorEntryV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn encode_vector_composition_profile(
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
        payload,
    )
}

fn encode_vector_composition_component(
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
        payload,
    )
}

fn encode_vector_arithmetic_profile(
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
        payload,
    )
}

fn encode_vector_index(record: VectorIndexDescriptorV1) -> Result<Vec<u8>, CoveError> {
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
        payload,
    )
}

fn parse_postscript_from_tail(
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

fn validate_section_ranges(
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

fn decoded_section_payload<'a>(
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

fn validate_section_payload_crc(
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

fn parse_ai_records(
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

fn parse_known_record(
    section_kind: u32,
    header: &AiRecordHeaderV1,
    payload: &[u8],
    tables: &mut AiDescriptorTablesV1,
) -> Result<(), CoveError> {
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
                .push(parse_filecode_vector_binding(payload)?);
        }
        (k, 2) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .chunk_vector_bindings
                .push(parse_chunk_vector_binding(payload)?);
        }
        (k, 3) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .object_state_vector_bindings
                .push(parse_object_state_vector_binding(payload)?);
        }
        (k, 4) if k == SectionKind::AiVectorBinding as u32 => {
            tables
                .training_sample_vector_bindings
                .push(parse_training_sample_vector_binding(payload)?);
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

fn parse_string_entry(payload: &[u8]) -> Result<AiStringEntryV1, CoveError> {
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

fn parse_digest_entry(payload: &[u8]) -> Result<AiDigestEntryV1, CoveError> {
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

fn parse_payload_ref_entry(payload: &[u8]) -> Result<AiPayloadRefEntryV1, CoveError> {
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

fn parse_policy_ref_entry(payload: &[u8]) -> Result<AiPolicyRefEntryV1, CoveError> {
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

fn parse_source_span_entry(payload: &[u8]) -> Result<AiSourceSpanEntryV1, CoveError> {
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

fn parse_transform_entry(payload: &[u8]) -> Result<AiTransformEntryV1, CoveError> {
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

fn parse_privacy_summary(payload: &[u8]) -> Result<AiPrivacySummaryEntryV1, CoveError> {
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

fn parse_companion_artifact_ref(payload: &[u8]) -> Result<AiCompanionArtifactRefV1, CoveError> {
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

fn parse_source_binding(payload: &[u8]) -> Result<AiSourceBindingV1, CoveError> {
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

fn parse_payload_integrity(payload: &[u8]) -> Result<AiPayloadIntegrityV1, CoveError> {
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

fn parse_section_feature_binding(payload: &[u8]) -> Result<AiSectionFeatureBindingV1, CoveError> {
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

fn parse_chunk_profile(payload: &[u8]) -> Result<ChunkProfileV1, CoveError> {
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

fn parse_text_chunk_entry(payload: &[u8]) -> Result<TextChunkEntryV1, CoveError> {
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

fn parse_tokenizer_profile(payload: &[u8]) -> Result<TokenizerProfileV1, CoveError> {
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

fn parse_token_block(payload: &[u8]) -> Result<TokenBlockHeaderV1, CoveError> {
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

fn parse_tokenized_span(payload: &[u8]) -> Result<TokenizedSpanV1, CoveError> {
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

fn parse_token_sequence_pack(payload: &[u8]) -> Result<TokenSequencePackV1, CoveError> {
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

fn parse_training_profile(payload: &[u8]) -> Result<TrainingProfileV1, CoveError> {
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

fn parse_training_sample_entry(payload: &[u8]) -> Result<TrainingSampleEntryV1, CoveError> {
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

fn parse_dataset_split(payload: &[u8]) -> Result<DatasetSplitV1, CoveError> {
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

fn parse_dedup_group(payload: &[u8]) -> Result<DedupGroupV1, CoveError> {
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

fn parse_training_epoch_plan(payload: &[u8]) -> Result<TrainingEpochPlanV1, CoveError> {
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

fn parse_training_label_entry(payload: &[u8]) -> Result<TrainingLabelEntryV1, CoveError> {
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

fn parse_preference_pair_entry(payload: &[u8]) -> Result<PreferencePairEntryV1, CoveError> {
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

fn parse_generator_provenance(payload: &[u8]) -> Result<GeneratorProvenanceV1, CoveError> {
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

fn parse_model_actor(payload: &[u8]) -> Result<ModelActorDescriptorV1, CoveError> {
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

fn parse_generation_decoding_profile(
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

fn parse_human_review(payload: &[u8]) -> Result<HumanReviewEntryV1, CoveError> {
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

fn parse_tensor_layout(payload: &[u8]) -> Result<TensorLayoutDescriptorV1, CoveError> {
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

fn parse_device_transfer_hint(payload: &[u8]) -> Result<DeviceTransferHintV1, CoveError> {
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

fn parse_ai_asset_ref(payload: &[u8]) -> Result<AiAssetRefV1, CoveError> {
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

fn parse_multimodal_sequence_pack(payload: &[u8]) -> Result<MultimodalSequencePackV1, CoveError> {
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

fn parse_multimodal_sequence_element(
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

fn parse_vector_space(payload: &[u8]) -> Result<VectorSpaceDescriptorV1, CoveError> {
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

fn parse_vector_space_compatibility(
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

fn parse_filecode_vector_binding(payload: &[u8]) -> Result<FileCodeVectorBindingV1, CoveError> {
    exact_len!(payload, 80, "FileCodeVectorBindingV1");
    verify_payload_crc(payload, 76)?;
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
        flags: read_u32(payload, 72)?,
        checksum: read_u32(payload, 76)?,
    })
}

fn parse_chunk_vector_binding(payload: &[u8]) -> Result<ChunkVectorBindingV1, CoveError> {
    exact_len!(payload, 48, "ChunkVectorBindingV1");
    verify_payload_crc(payload, 44)?;
    Ok(ChunkVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        chunk_id: read_u64(payload, 12)?,
        chunk_profile_id: read_u32(payload, 20)?,
        source_value_hash_ref: read_u32(payload, 24)?,
        chunk_text_hash_ref: read_u32(payload, 28)?,
        vector_ref: read_u64(payload, 32)?,
        flags: read_u32(payload, 40)?,
        checksum: read_u32(payload, 44)?,
    })
}

fn parse_object_state_vector_binding(
    payload: &[u8],
) -> Result<ObjectStateVectorBindingV1, CoveError> {
    exact_len!(payload, 69, "ObjectStateVectorBindingV1");
    verify_payload_crc(payload, 65)?;
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
        flags: read_u32(payload, 61)?,
        checksum: read_u32(payload, 65)?,
    })
}

fn parse_training_sample_vector_binding(
    payload: &[u8],
) -> Result<TrainingSampleVectorBindingV1, CoveError> {
    exact_len!(payload, 48, "TrainingSampleVectorBindingV1");
    verify_payload_crc(payload, 44)?;
    Ok(TrainingSampleVectorBindingV1 {
        binding_id: read_u64(payload, 0)?,
        vector_space_id: read_u32(payload, 8)?,
        training_profile_ref: read_u32(payload, 12)?,
        sample_id: read_u64(payload, 16)?,
        source_snapshot_ref: read_u32(payload, 24)?,
        sample_fingerprint_ref: read_u32(payload, 28)?,
        vector_ref: read_u64(payload, 32)?,
        flags: read_u32(payload, 40)?,
        checksum: read_u32(payload, 44)?,
    })
}

fn parse_vector_payload_block(payload: &[u8]) -> Result<VectorPayloadBlockHeaderV1, CoveError> {
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

fn parse_vector_entry(payload: &[u8]) -> Result<VectorEntryV1, CoveError> {
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

fn parse_vector_composition_profile(
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

fn parse_vector_composition_component(
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

fn parse_vector_arithmetic_profile(payload: &[u8]) -> Result<VectorArithmeticProfileV1, CoveError> {
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

fn parse_vector_index(payload: &[u8]) -> Result<VectorIndexDescriptorV1, CoveError> {
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

fn validate_cached_payload_range(
    label: &str,
    id: u32,
    payload_ref: &AiPayloadRefEntryV1,
    cached_offset: u64,
    cached_length: u64,
) -> Result<(), CoveError> {
    match AiStorageKindV1::from_u8(payload_ref.storage_kind)
        .ok_or_else(|| CoveError::BadSection("unknown AI payload storage_kind".into()))?
    {
        AiStorageKindV1::ArtifactAbsolute => {
            if cached_length != 0 {
                if cached_offset != payload_ref.payload_offset
                    || cached_length != payload_ref.payload_length
                {
                    return Err(CoveError::BadSection(format!(
                        "{label} {id} cached payload range does not match payload_ref"
                    )));
                }
            } else if cached_offset != 0 {
                return Err(CoveError::BadSection(format!(
                    "{label} {id} has zero cached payload length but non-zero offset"
                )));
            }
        }
        _ => {
            if cached_offset != 0 || cached_length != 0 {
                return Err(CoveError::BadSection(format!(
                    "{label} {id} must not cache artifact offsets for non-ArtifactAbsolute payload refs"
                )));
            }
        }
    }
    Ok(())
}

fn validate_unique<I>(values: I, label: &str) -> Result<(), CoveError>
where
    I: IntoIterator<Item = u32>,
{
    let mut seen = BTreeSet::new();
    for value in values {
        if value == 0 {
            return Err(CoveError::BadSection(format!("{label} must be non-zero")));
        }
        if !seen.insert(value) {
            return Err(CoveError::BadSection(format!("duplicate {label} {value}")));
        }
    }
    Ok(())
}

fn validate_unique_u64<I>(values: I, label: &str) -> Result<(), CoveError>
where
    I: IntoIterator<Item = u64>,
{
    let mut seen = BTreeSet::new();
    for value in values {
        if value == 0 {
            return Err(CoveError::BadSection(format!("{label} must be non-zero")));
        }
        if !seen.insert(value) {
            return Err(CoveError::BadSection(format!("duplicate {label} {value}")));
        }
    }
    Ok(())
}

fn section_by_id(sections: &[CoveAiSection], section_id: u32) -> Option<&CoveAiSection> {
    sections
        .iter()
        .find(|section| section.entry.section_id == section_id)
}

fn is_payload_bearing_section(section_kind: u32) -> bool {
    matches!(
        SectionKind::from_u16(section_kind as u16),
        Some(
            SectionKind::AiPayloadBytes
                | SectionKind::AiTokenBlock
                | SectionKind::AiVectorPayloadBlock
                | SectionKind::AiVectorDirectory
                | SectionKind::AiTokenSequencePack
                | SectionKind::AiTrainingSampleIndex
                | SectionKind::AiMultimodalSequence
                | SectionKind::AiAssetManifest
        )
    )
}

fn element_width_bytes(element_type: u8) -> Option<u64> {
    match element_type {
        0 => Some(4),
        1 | 2 => Some(2),
        3 | 4 => Some(1),
        5 => None,
        _ => None,
    }
}

fn validate_ai_vector_element_type(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_metric(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_index_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 8 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_index_exactness(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_false_negative_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_score_space_authority(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_indexed_binding_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_compatibility_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_compatibility_authority(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_normalization_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_quantization_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_compression_codec(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 2 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_layout_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_chunk_boundary_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_chunk_overlap_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_chunk_parent_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_digest_domain(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_privacy_state(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_reproducibility_class(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_result_authority(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_composition_method(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_composition_missing_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_redaction_behavior(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_component_missing_behavior(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_arithmetic_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_accumulator_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_rounding_mode(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_overflow_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_vector_component_order(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_tensor_dtype(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_byte_order(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 2 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_device_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_asset_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 6 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_modality(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_role(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 6 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_multimodal_element_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_generator_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_review_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_split_method(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_similarity_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_dedup_authority(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_ai_permutation_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

fn validate_bool_byte(value: u8, label: &str) -> Result<(), CoveError> {
    if value > 1 {
        return Err(CoveError::BadSection(format!(
            "{label} must be 0 or 1, got {value}"
        )));
    }
    Ok(())
}

fn validate_power_of_two_alignment(value: u32, label: &str) -> Result<(), CoveError> {
    if value != 0 && !value.is_power_of_two() {
        return Err(CoveError::BadSection(format!(
            "{label} must be zero or a power of two, got {value}"
        )));
    }
    Ok(())
}

fn validate_token_range(
    label: &str,
    id: u64,
    token_offset: u64,
    token_count: u32,
    block: &TokenBlockHeaderV1,
) -> Result<(), CoveError> {
    let end = token_offset
        .checked_add(u64::from(token_count))
        .ok_or(CoveError::ArithOverflow)?;
    if end > block.token_count {
        return Err(CoveError::BadSection(format!(
            "{label} {id} token range exceeds token_block_ref {} token_count",
            block.token_block_id
        )));
    }
    Ok(())
}

fn verify_payload_crc(payload: &[u8], checksum_offset: usize) -> Result<(), CoveError> {
    if checksum_offset + 4 > payload.len() {
        return Err(CoveError::BufferTooShort);
    }
    let expected = read_u32(payload, checksum_offset)?;
    verify_crc32c(payload, checksum_offset, expected)
}

fn verify_crc32c(bytes: &[u8], checksum_offset: usize, expected: u32) -> Result<(), CoveError> {
    if checksum_offset + 4 > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let mut for_crc = bytes.to_vec();
    for_crc[checksum_offset..checksum_offset + 4].fill(0);
    if checksum::crc32c(&for_crc) != expected {
        return Err(CoveError::ChecksumMismatch);
    }
    Ok(())
}

fn checked_slice(data: &[u8], offset: u64, length: u64) -> Result<&[u8], CoveError> {
    let end = checked_range(offset, length, data.len() as u64)?;
    Ok(&data[offset as usize..end as usize])
}

fn checked_range(offset: u64, length: u64, bound: u64) -> Result<u64, CoveError> {
    let end = offset.checked_add(length).ok_or(CoveError::ArithOverflow)?;
    if end > bound {
        return Err(CoveError::OffsetRange);
    }
    Ok(end)
}

fn range_contains(outer_offset: u64, outer_len: u64, inner_offset: u64, inner_len: u64) -> bool {
    let Some(outer_end) = outer_offset.checked_add(outer_len) else {
        return false;
    };
    let Some(inner_end) = inner_offset.checked_add(inner_len) else {
        return false;
    };
    inner_offset >= outer_offset && inner_end <= outer_end
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, CoveError> {
    bytes.get(offset).copied().ok_or(CoveError::BufferTooShort)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, CoveError> {
    let end = offset.checked_add(2).ok_or(CoveError::ArithOverflow)?;
    let slice = bytes.get(offset..end).ok_or(CoveError::BufferTooShort)?;
    Ok(u16::from_le_bytes(slice.try_into().unwrap()))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, CoveError> {
    let end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
    let slice = bytes.get(offset..end).ok_or(CoveError::BufferTooShort)?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, CoveError> {
    let end = offset.checked_add(8).ok_or(CoveError::ArithOverflow)?;
    let slice = bytes.get(offset..end).ok_or(CoveError::BufferTooShort)?;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, CoveError> {
    let end = offset.checked_add(8).ok_or(CoveError::ArithOverflow)?;
    let slice = bytes.get(offset..end).ok_or(CoveError::BufferTooShort)?;
    Ok(i64::from_le_bytes(slice.try_into().unwrap()))
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], CoveError> {
    let end = offset.checked_add(N).ok_or(CoveError::ArithOverflow)?;
    let slice = bytes.get(offset..end).ok_or(CoveError::BufferTooShort)?;
    Ok(slice.try_into().unwrap())
}

fn put_u8(bytes: &mut [u8], offset: usize, value: u8) {
    bytes[offset] = value;
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_i64(bytes: &mut [u8], offset: usize, value: i64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32_payload(values: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(values.len() * 4);
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn test_vector_space(vector_space_id: u32) -> VectorSpaceDescriptorV1 {
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

    fn test_vector_space_compatibility() -> VectorSpaceCompatibilityDescriptorV1 {
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

    fn vector_descriptor_tables_with_payload() -> (AiDescriptorTablesV1, Vec<CoveAiWritableSection>)
    {
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

    fn vector_composition_tables() -> (AiDescriptorTablesV1, Vec<CoveAiWritableSection>) {
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

    #[test]
    fn writes_and_parses_empty_coveai_artifact() {
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 123, &[]).unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.artifact_kind, CoveAiArtifactKind::CoveAiBundle);
        assert_eq!(parsed.header.artifact_id, [7u8; 16]);
        assert_eq!(parsed.sections.len(), 0);
    }

    #[test]
    fn rejects_duplicate_section_id() {
        let sections = [
            CoveAiWritableSection {
                section_id: 1,
                section_kind: SectionKind::AiPayloadBytes as u32,
                profile_kind: 16,
                payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
                requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
                source_binding_ref: 0,
                required_ai_features: 0,
                optional_ai_features: 0,
                feature_binding_ref: 0,
                payload: vec![1, 2, 3],
            },
            CoveAiWritableSection {
                section_id: 1,
                section_kind: SectionKind::AiPayloadBytes as u32,
                profile_kind: 16,
                payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
                requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
                source_binding_ref: 0,
                required_ai_features: 0,
                optional_ai_features: 0,
                feature_binding_ref: 0,
                payload: vec![4, 5, 6],
            },
        ];
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [0; 16], 0, &sections).unwrap();
        assert!(CoveAiFile::parse(&bytes).is_err());
    }

    #[test]
    fn descriptor_bundle_serializes_digest_bound_companion_ref() {
        let uri = b"vectors.covev";
        let digest = [0xAB; 32];
        let mut payload = Vec::new();
        payload.extend_from_slice(uri);
        payload.extend_from_slice(&digest);

        let mut tables = AiDescriptorTablesV1::default();
        tables.strings.push(AiStringEntryV1 {
            string_ref: 1,
            utf8_byte_length: uri.len() as u32,
            payload_ref: 1,
            flags: 0,
            crc32c: 0,
        });
        tables.digests.push(AiDigestEntryV1 {
            digest_ref: 1,
            digest_algorithm: 1,
            digest_len: digest.len() as u16,
            digest_payload_ref: 2,
            domain_hint: 1,
            flags: 0,
            crc32c: 0,
        });
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 1,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 0,
            payload_length: uri.len() as u64,
            decoded_length: uri.len() as u64,
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
            section_payload_offset: uri.len() as u64,
            payload_length: digest.len() as u64,
            decoded_length: digest.len() as u64,
            integrity_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.companion_artifacts.push(AiCompanionArtifactRefV1 {
            artifact_ref: 1,
            artifact_kind: AI_COMPANION_ARTIFACT_KIND_CVV2,
            artifact_id: [9; 16],
            uri_ref: 1,
            artifact_digest_ref: 1,
            source_binding_ref: 0,
            required_ai_features: AI_FEATURE_VECTOR,
            optional_ai_features: 0,
            flags: 0,
            crc32c: 0,
        });

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [21u8; 16],
            created_at_us: 908,
            payload_sections: vec![coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveAiShared,
                payload,
            )],
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.companion_artifacts.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.companion_artifacts[0].artifact_kind,
            AI_COMPANION_ARTIFACT_KIND_CVV2
        );
        assert_eq!(parsed.descriptor_tables.strings.len(), 1);
        assert_eq!(parsed.descriptor_tables.digests.len(), 1);
    }

    #[test]
    fn rejects_companion_ref_missing_artifact_digest() {
        let uri = b"vectors.covev";
        let sections = vec![
            coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveAiShared,
                uri.to_vec(),
            ),
            coveai_binary_section(
                2,
                SectionKind::AiReferenceTables,
                PrimaryProfile::CoveAiShared,
                encode_records([
                    encode_string_entry(AiStringEntryV1 {
                        string_ref: 1,
                        utf8_byte_length: uri.len() as u32,
                        payload_ref: 1,
                        flags: 0,
                        crc32c: 0,
                    })
                    .unwrap(),
                    encode_payload_ref_entry(AiPayloadRefEntryV1 {
                        payload_ref: 1,
                        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
                        media_type_ref: 0,
                        section_id: 1,
                        uri_ref: 0,
                        payload_offset: 0,
                        section_payload_offset: 0,
                        payload_length: uri.len() as u64,
                        decoded_length: uri.len() as u64,
                        integrity_ref: 0,
                        flags: 0,
                        crc32c: 0,
                    })
                    .unwrap(),
                ])
                .unwrap(),
            ),
            coveai_binary_section(
                3,
                SectionKind::AiCompanionArtifactRef,
                PrimaryProfile::CoveAiShared,
                encode_records([encode_companion_artifact_ref(AiCompanionArtifactRefV1 {
                    artifact_ref: 1,
                    artifact_kind: AI_COMPANION_ARTIFACT_KIND_CVV2,
                    artifact_id: [9; 16],
                    uri_ref: 1,
                    artifact_digest_ref: 99,
                    source_binding_ref: 0,
                    required_ai_features: AI_FEATURE_VECTOR,
                    optional_ai_features: 0,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap()])
                .unwrap(),
            ),
        ];
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [22u8; 16], 909, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing artifact_digest_ref")
        ));
    }

    #[test]
    fn descriptor_bundle_serializes_source_binding() {
        let mut tables = AiDescriptorTablesV1::default();
        tables.source_bindings.push(AiSourceBindingV1 {
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
            as_of_csn: 42,
            flags: 0,
            crc32c: 0,
        });

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [23u8; 16],
            created_at_us: 910,
            payload_sections: Vec::new(),
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.source_bindings.len(), 1);
        assert_eq!(parsed.descriptor_tables.source_bindings[0].as_of_csn, 42);
    }

    #[test]
    fn descriptor_bundle_rejects_source_binding_missing_digest_ref() {
        let mut tables = AiDescriptorTablesV1::default();
        tables.source_bindings.push(AiSourceBindingV1 {
            source_binding_id: 1,
            source_kind: AI_SOURCE_KIND_COVE_FILE,
            source_artifact_ref: 0,
            source_file_digest_ref: 99,
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
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [24u8; 16],
                created_at_us: 911,
                payload_sections: Vec::new(),
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("missing source_file_digest_ref")
        ));
    }

    #[test]
    fn descriptor_bundle_serializes_policy_ref_and_source_policy_ref() {
        let authority = b"visibility-policy";
        let mut tables = AiDescriptorTablesV1::default();
        tables.strings.push(AiStringEntryV1 {
            string_ref: 1,
            utf8_byte_length: authority.len() as u32,
            payload_ref: 1,
            flags: 0,
            crc32c: 0,
        });
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 1,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 0,
            payload_length: authority.len() as u64,
            decoded_length: authority.len() as u64,
            integrity_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.policies.push(AiPolicyRefEntryV1 {
            policy_ref: 1,
            policy_kind: AI_POLICY_KIND_VISIBILITY,
            authority_ref: 1,
            payload_ref: 0,
            digest_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.source_bindings.push(AiSourceBindingV1 {
            source_binding_id: 1,
            source_kind: AI_SOURCE_KIND_COVE_FILE,
            source_artifact_ref: 0,
            source_file_digest_ref: 0,
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

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [25u8; 16],
            created_at_us: 912,
            payload_sections: vec![coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveAiShared,
                authority.to_vec(),
            )],
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.policies.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.source_bindings[0].visibility_scope_ref,
            1
        );
    }

    #[test]
    fn descriptor_bundle_rejects_policy_payload_without_digest() {
        let policy_payload = b"policy";
        let mut tables = AiDescriptorTablesV1::default();
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 1,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 0,
            payload_length: policy_payload.len() as u64,
            decoded_length: policy_payload.len() as u64,
            integrity_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.policies.push(AiPolicyRefEntryV1 {
            policy_ref: 1,
            policy_kind: AI_POLICY_KIND_VISIBILITY,
            authority_ref: 0,
            payload_ref: 1,
            digest_ref: 0,
            flags: 0,
            crc32c: 0,
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [26u8; 16],
                created_at_us: 913,
                payload_sections: vec![coveai_payload_section(
                    1,
                    SectionKind::AiPayloadBytes,
                    PrimaryProfile::CoveAiShared,
                    policy_payload.to_vec(),
                )],
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("payload_ref requires digest_ref")
        ));
    }

    #[test]
    fn descriptor_bundle_serializes_privacy_summary_refs() {
        let mut tables = AiDescriptorTablesV1::default();
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 1,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 0,
            payload_length: 1,
            decoded_length: 1,
            integrity_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.policies.push(AiPolicyRefEntryV1 {
            policy_ref: 1,
            policy_kind: AI_POLICY_KIND_VISIBILITY,
            authority_ref: 0,
            payload_ref: 0,
            digest_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
            privacy_summary_ref: 1,
            source_binding_ref: 0,
            sensitivity_mask: 1,
            sensitivity_bits_ref: 1,
            policy_ref: 1,
            visibility_scope_ref: 1,
            redaction_scope_ref: 0,
            retention_state: 1,
            disclosure_state: 1,
            flags: 0,
            crc32c: 0,
        });

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [38u8; 16],
            created_at_us: 925,
            payload_sections: vec![coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveAiShared,
                vec![1],
            )],
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.privacy_summaries.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.privacy_summaries[0].visibility_scope_ref,
            1
        );
    }

    #[test]
    fn descriptor_bundle_rejects_privacy_summary_missing_visibility_policy() {
        let mut tables = AiDescriptorTablesV1::default();
        tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
            privacy_summary_ref: 1,
            source_binding_ref: 0,
            sensitivity_mask: 0,
            sensitivity_bits_ref: 0,
            policy_ref: 0,
            visibility_scope_ref: 99,
            redaction_scope_ref: 0,
            retention_state: 0,
            disclosure_state: 0,
            flags: 0,
            crc32c: 0,
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [39u8; 16],
                created_at_us: 926,
                payload_sections: Vec::new(),
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("missing visibility_scope_ref 99")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_payload_integrity_digest_length_mismatch() {
        let mut payload = vec![0xAA; 4];
        payload.extend_from_slice(&[0xBB; 32]);
        let mut tables = AiDescriptorTablesV1::default();
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 1,
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
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 2,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 4,
            payload_length: 32,
            decoded_length: 32,
            integrity_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.digests.push(AiDigestEntryV1 {
            digest_ref: 1,
            digest_algorithm: 1,
            digest_len: 32,
            digest_payload_ref: 2,
            domain_hint: 6,
            flags: 0,
            crc32c: 0,
        });
        tables.payload_integrity.push(AiPayloadIntegrityV1 {
            integrity_ref: 1,
            payload_ref: 1,
            digest_domain: 6,
            reserved0: 0,
            digest_algorithm: 1,
            digest_len: 16,
            digest_ref: 1,
            payload_crc32c: 0,
            flags: 0,
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [36u8; 16],
                created_at_us: 923,
                payload_sections: vec![coveai_payload_section(
                    1,
                    SectionKind::AiPayloadBytes,
                    PrimaryProfile::CoveAiShared,
                    payload,
                )],
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("digest algorithm/length mismatch")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_payload_integrity_digest_payload_cycle() {
        let mut payload = vec![0xAA; 4];
        payload.extend_from_slice(&[0xBB; 32]);
        let mut tables = AiDescriptorTablesV1::default();
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 1,
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
        tables.payload_refs.push(AiPayloadRefEntryV1 {
            payload_ref: 2,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 1,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 4,
            payload_length: 32,
            decoded_length: 32,
            integrity_ref: 1,
            flags: 0,
            crc32c: 0,
        });
        tables.digests.push(AiDigestEntryV1 {
            digest_ref: 1,
            digest_algorithm: 1,
            digest_len: 32,
            digest_payload_ref: 2,
            domain_hint: 6,
            flags: 0,
            crc32c: 0,
        });
        tables.payload_integrity.push(AiPayloadIntegrityV1 {
            integrity_ref: 1,
            payload_ref: 1,
            digest_domain: 6,
            reserved0: 0,
            digest_algorithm: 1,
            digest_len: 32,
            digest_ref: 1,
            payload_crc32c: 0,
            flags: 0,
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [37u8; 16],
                created_at_us: 924,
                payload_sections: vec![coveai_payload_section(
                    1,
                    SectionKind::AiPayloadBytes,
                    PrimaryProfile::CoveAiShared,
                    payload,
                )],
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("cyclic digest payload integrity")
        ));
    }

    #[test]
    fn descriptor_bundle_serializes_source_span_and_transform() {
        let mut tables = AiDescriptorTablesV1::default();
        tables.source_spans.push(AiSourceSpanEntryV1 {
            source_span_ref: 1,
            source_binding_ref: 0,
            source_kind: AI_SOURCE_KIND_COVE_FILE,
            source_row_ref: 7,
            source_object_ref: 0,
            byte_start: 11,
            byte_length: 5,
            token_start: 0,
            token_count: 0,
            evidence_ref: 0,
            flags: 0,
            crc32c: 0,
        });
        tables.transforms.push(AiTransformEntryV1 {
            transform_ref: 1,
            transform_kind: AI_TRANSFORM_KIND_TEXT_NORMALIZATION,
            function_or_template_ref: 0,
            input_digest_ref: 0,
            output_digest_ref: 0,
            parameter_payload_ref: 0,
            transform_digest_ref: 0,
            flags: 0,
            crc32c: 0,
        });

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [27u8; 16],
            created_at_us: 914,
            payload_sections: Vec::new(),
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.source_spans.len(), 1);
        assert_eq!(parsed.descriptor_tables.transforms.len(), 1);
        assert_eq!(parsed.descriptor_tables.source_spans[0].byte_start, 11);
    }

    #[test]
    fn descriptor_bundle_rejects_transform_missing_parameter_payload() {
        let mut tables = AiDescriptorTablesV1::default();
        tables.transforms.push(AiTransformEntryV1 {
            transform_ref: 1,
            transform_kind: AI_TRANSFORM_KIND_TEXT_NORMALIZATION,
            function_or_template_ref: 0,
            input_digest_ref: 0,
            output_digest_ref: 0,
            parameter_payload_ref: 99,
            transform_digest_ref: 0,
            flags: 0,
            crc32c: 0,
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [28u8; 16],
                created_at_us: 915,
                payload_sections: Vec::new(),
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("missing parameter_payload_ref")
        ));
    }

    #[test]
    fn descriptor_bundle_serializes_ai_section_feature_binding() {
        let mut tables = AiDescriptorTablesV1::default();
        tables
            .section_feature_bindings
            .push(AiSectionFeatureBindingV1 {
                binding_ref: 1,
                section_id: 1,
                scope: FeatureScopeV2::OperationRequired as u8,
                profile_kind: PrimaryProfile::CoveVec as u8,
                operation_kind: OperationKindV2::AiEmbedding as u16,
                required_ai_features: AI_FEATURE_VECTOR,
                optional_ai_features: 0,
                target_local_ref: 0,
                flags: 0,
                crc32c: 0,
            });

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [29u8; 16],
            created_at_us: 916,
            payload_sections: vec![coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveVec,
                vec![0],
            )],
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.section_feature_bindings.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.section_feature_bindings[0].operation_kind,
            OperationKindV2::AiEmbedding as u16
        );
    }

    #[test]
    fn descriptor_bundle_rejects_ai_section_feature_binding_self_target() {
        let mut tables = AiDescriptorTablesV1::default();
        tables
            .section_feature_bindings
            .push(AiSectionFeatureBindingV1 {
                binding_ref: 1,
                section_id: 1_000,
                scope: FeatureScopeV2::OperationRequired as u8,
                profile_kind: PrimaryProfile::CoveVec as u8,
                operation_kind: OperationKindV2::AiEmbedding as u16,
                required_ai_features: AI_FEATURE_VECTOR,
                optional_ai_features: 0,
                target_local_ref: 0,
                flags: 0,
                crc32c: 0,
            });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [30u8; 16],
                created_at_us: 917,
                payload_sections: Vec::new(),
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("must not target AI_SECTION_FEATURE_BINDING")
        ));
    }

    #[test]
    fn descriptor_bundle_serializes_vector_compatibility_and_composition() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        tables
            .vector_space_compatibilities
            .push(VectorSpaceCompatibilityDescriptorV1 {
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
            });
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

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [31u8; 16],
            created_at_us: 918,
            payload_sections,
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(
            parsed.descriptor_tables.vector_space_compatibilities.len(),
            1
        );
        assert_eq!(parsed.descriptor_tables.vector_arithmetic_profiles.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.vector_composition_components.len(),
            1
        );
        assert_eq!(
            parsed.descriptor_tables.vector_composition_profiles.len(),
            1
        );
    }

    #[test]
    fn descriptor_bundle_rejects_vector_compatibility_bad_kind() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        let mut compatibility = test_vector_space_compatibility();
        compatibility.compatibility_kind = 42;
        tables.vector_space_compatibilities.push(compatibility);

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [46u8; 16],
                created_at_us: 933,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("VectorSpaceCompatibility compatibility_kind")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_compatibility_bad_authority() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        let mut compatibility = test_vector_space_compatibility();
        compatibility.compatibility_authority = 42;
        tables.vector_space_compatibilities.push(compatibility);

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [47u8; 16],
                created_at_us: 934,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("VectorSpaceCompatibility compatibility_authority")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_compatibility_excessive_numeric_error() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        let mut compatibility = test_vector_space_compatibility();
        compatibility.numeric_transform_error_ppm = 1_000_001;
        tables.vector_space_compatibilities.push(compatibility);

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [48u8; 16],
                created_at_us: 935,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("numeric_transform_error_ppm exceeds 1_000_000")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_compatibility_missing_evidence_ref() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        let mut compatibility = test_vector_space_compatibility();
        compatibility.evidence_ref = 99;
        tables.vector_space_compatibilities.push(compatibility);

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [49u8; 16],
                created_at_us: 936,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 99")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_composition_bad_method() {
        let (mut tables, payload_sections) = vector_composition_tables();
        tables.vector_composition_profiles[0].method = 42;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [41u8; 16],
                created_at_us: 928,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("VectorCompositionProfile method")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_composition_missing_template_ref() {
        let (mut tables, payload_sections) = vector_composition_tables();
        tables.vector_composition_profiles[0].template_ref = 99;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [42u8; 16],
                created_at_us: 929,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message)) if message.contains("missing template_ref 99")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_canonical_composition_without_arithmetic_profile() {
        let (mut tables, payload_sections) = vector_composition_tables();
        tables.vector_composition_profiles[0].result_authority = 2;
        tables.vector_composition_profiles[0].arithmetic_profile_ref = 0;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [43u8; 16],
                created_at_us: 930,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("CanonicalFixedPointRecompute requires arithmetic_profile_ref")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_component_bad_redaction_behavior() {
        let (mut tables, payload_sections) = vector_composition_tables();
        tables.vector_composition_components[0].redaction_behavior = 42;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [44u8; 16],
                created_at_us: 931,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("VectorCompositionComponent redaction_behavior")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_arithmetic_bad_rounding_mode() {
        let (mut tables, payload_sections) = vector_composition_tables();
        tables.vector_arithmetic_profiles[0].rounding_mode = 42;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [45u8; 16],
                created_at_us: 932,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("VectorArithmeticProfile rounding_mode")
        ));
    }

    #[test]
    fn descriptor_bundle_serializes_chunk_object_and_training_vector_bindings() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
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
        let mut chunk = test_text_chunk(1, 0, 0, 0, 12, 12, 1);
        chunk.source_ref = 0;
        tables.text_chunks.push(chunk);
        tables.training_profiles.push(test_training_profile());
        tables
            .training_samples
            .push(test_training_sample(1, 1, 0, 0));
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
                arithmetic_profile_ref: 0,
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
        tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
            binding_id: 10,
            vector_space_id: 1,
            chunk_id: 1,
            chunk_profile_id: 1,
            source_value_hash_ref: 0,
            chunk_text_hash_ref: 0,
            vector_ref: 1,
            flags: 0,
            checksum: 0,
        });
        tables
            .object_state_vector_bindings
            .push(ObjectStateVectorBindingV1 {
                binding_id: 20,
                vector_space_id: 1,
                composition_profile_ref: 1,
                file_ref: 0,
                object_type_id: 7,
                goid_ref: 0,
                branch_ref: 0,
                temporal_kind: 0,
                csn: 123,
                timestamp_us: 456,
                property_dependency_fingerprint_ref: 0,
                vector_ref: 2,
                flags: 0,
                checksum: 0,
            });
        tables
            .training_sample_vector_bindings
            .push(TrainingSampleVectorBindingV1 {
                binding_id: 30,
                vector_space_id: 1,
                training_profile_ref: 1,
                sample_id: 1,
                source_snapshot_ref: 0,
                sample_fingerprint_ref: 0,
                vector_ref: 3,
                flags: 0,
                checksum: 0,
            });

        let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [32u8; 16],
            created_at_us: 919,
            payload_sections,
            descriptor_tables: tables,
        })
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.chunk_vector_bindings.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.object_state_vector_bindings.len(),
            1
        );
        assert_eq!(
            parsed
                .descriptor_tables
                .training_sample_vector_bindings
                .len(),
            1
        );
    }

    #[test]
    fn descriptor_bundle_rejects_chunk_vector_binding_missing_vector_ref() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
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
        let mut chunk = test_text_chunk(1, 0, 0, 0, 12, 12, 1);
        chunk.source_ref = 0;
        tables.text_chunks.push(chunk);
        tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
            binding_id: 10,
            vector_space_id: 1,
            chunk_id: 1,
            chunk_profile_id: 1,
            source_value_hash_ref: 0,
            chunk_text_hash_ref: 0,
            vector_ref: 99,
            flags: 0,
            checksum: 0,
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [33u8; 16],
                created_at_us: 920,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message)) if message.contains("missing vector_ref 99")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_payload_block_dimension_mismatch() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        tables.vector_payload_blocks[0].dimension_count = 4;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [34u8; 16],
                created_at_us: 921,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("dimension_count does not match vector_space_id")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_payload_block_missing_stride_ref() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        tables.vector_payload_blocks[0].payload_stride_ref = 99;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [35u8; 16],
                created_at_us: 922,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("missing payload_stride_ref 99")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_entry_dense_length_mismatch() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        tables.vector_entries[0].payload_length = 8;

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [41u8; 16],
                created_at_us: 928,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("payload_length does not match dense vector width")
        ));
    }

    #[test]
    fn descriptor_bundle_rejects_vector_entry_derived_range_out_of_block() {
        let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
        tables.vector_payload_blocks[0].vector_count = 4;
        tables.vector_entries.push(VectorEntryV1 {
            vector_ref: 4,
            block_id: 1,
            vector_ordinal: 3,
            payload_offset: 0,
            payload_length: 0,
            integrity_ref: 0,
            flags: 0,
            checksum: 0,
        });

        assert!(matches!(
            write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
                artifact_id: [42u8; 16],
                created_at_us: 929,
                payload_sections,
                descriptor_tables: tables,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("derived payload range exceeds")
        ));
    }

    #[test]
    fn writes_and_parses_filecode_vector_sidecar() {
        let mut vector_payload = Vec::new();
        for value in [1.0f32, 0.0, 0.5, 0.25, 0.75, 1.0] {
            vector_payload.extend_from_slice(&value.to_le_bytes());
        }
        let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
            artifact_id: [3u8; 16],
            created_at_us: 456,
            dimension_count: 3,
            file_codes: vec![10, 20],
            vector_payload,
        })
        .unwrap();

        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.artifact_kind, CoveAiArtifactKind::CoveVec);
        assert_eq!(
            parsed.payload_access,
            AiPayloadAccessState::StructurallyAllowed
        );
        assert_eq!(parsed.descriptor_tables.payload_refs.len(), 2);
        assert_eq!(parsed.descriptor_tables.payload_integrity.len(), 1);
        assert_eq!(parsed.descriptor_tables.privacy_summaries.len(), 1);
        assert_eq!(parsed.descriptor_tables.vector_spaces.len(), 1);
        assert_eq!(parsed.descriptor_tables.vector_payload_blocks.len(), 1);
        assert_eq!(parsed.descriptor_tables.vector_entries.len(), 2);
        assert_eq!(parsed.descriptor_tables.filecode_vector_bindings.len(), 2);
        assert_eq!(
            parsed.descriptor_tables.filecode_vector_bindings[1].file_code,
            20
        );
    }

    #[test]
    fn exact_flat_filecode_vector_search_returns_nearest_filecodes() {
        let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
            artifact_id: [18u8; 16],
            created_at_us: 905,
            dimension_count: 3,
            file_codes: vec![10, 20, 30],
            vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0]),
        })
        .unwrap();

        let results = exact_flat_filecode_vector_search(&bytes, &[0.9, 0.1, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].file_code, 10);
        assert_eq!(results[0].vector_ref, 1);
        assert_eq!(results[1].file_code, 20);
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn exact_flat_filecode_vector_search_by_file_code_uses_stored_query_vector() {
        let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
            artifact_id: [21u8; 16],
            created_at_us: 908,
            dimension_count: 3,
            file_codes: vec![10, 20, 30],
            vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.8, 0.2, 0.0, -1.0, 0.0, 0.0]),
        })
        .unwrap();

        let results = exact_flat_filecode_vector_search_by_file_code(&bytes, 10, 3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].file_code, 10);
        assert_eq!(results[1].file_code, 20);
        assert_eq!(results[2].file_code, 30);
        assert!(matches!(
            exact_flat_filecode_vector_search_by_file_code(&bytes, 99, 1),
            Err(CoveError::BadSection(message)) if message.contains("query FileCode 99")
        ));
    }

    #[test]
    fn filecode_embedding_returns_validated_stored_vector() {
        let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
            artifact_id: [22u8; 16],
            created_at_us: 909,
            dimension_count: 3,
            file_codes: vec![10, 20],
            vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.25, 0.5, 0.75]),
        })
        .unwrap();

        let embedding = filecode_embedding(&bytes, 20).unwrap();
        assert_eq!(embedding.file_code, 20);
        assert_eq!(embedding.vector_ref, 2);
        assert_eq!(embedding.vector_space_id, 1);
        assert_eq!(embedding.dimension_count, 3);
        assert_eq!(embedding.values, vec![0.25, 0.5, 0.75]);
    }

    #[test]
    fn rejects_exact_flat_filecode_vector_search_dimension_mismatch() {
        let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
            artifact_id: [19u8; 16],
            created_at_us: 906,
            dimension_count: 3,
            file_codes: vec![10],
            vector_payload: f32_payload(&[1.0, 0.0, 0.0]),
        })
        .unwrap();

        assert!(matches!(
            exact_flat_filecode_vector_search(&bytes, &[1.0, 0.0], 1),
            Err(CoveError::BadSection(message))
                if message.contains("no COVE-VEC vector space matches query dimension")
        ));
    }

    #[test]
    fn rejects_exact_flat_filecode_vector_search_when_payload_policy_blocked() {
        let sections = [coveai_payload_section(
            1,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveVec,
            f32_payload(&[1.0]),
        )];
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [20u8; 16], 907, &sections).unwrap();

        assert!(matches!(
            exact_flat_filecode_vector_search(&bytes, &[1.0], 1),
            Err(CoveError::BadSection(message)) if message.contains("policy-blocked")
        ));
    }

    #[test]
    fn parses_exact_flat_vector_index_descriptor() {
        let sections = vector_index_sections(test_vector_index(0, 0));
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [16u8; 16], 904, &sections).unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.vector_spaces.len(), 1);
        assert_eq!(parsed.descriptor_tables.vector_indexes.len(), 1);
    }

    #[test]
    fn rejects_ann_vector_index_claiming_exactness() {
        let sections = vector_index_sections(test_vector_index(1, 0));
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("must not claim exactness")
        ));
    }

    #[test]
    fn rejects_vector_index_unsupported_index_kind() {
        let sections = vector_index_sections(test_vector_index(42, 1));
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("VectorIndex index_kind")
        ));
    }

    #[test]
    fn rejects_vector_index_exact_false_negative_policy() {
        let mut index = test_vector_index(0, 0);
        index.false_negative_policy = 1;
        let sections = vector_index_sections(index);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("exact index must not declare false negatives")
        ));
    }

    #[test]
    fn rejects_vector_index_unsupported_score_space_authority() {
        let mut index = test_vector_index(0, 0);
        index.score_space_authority = 42;
        let sections = vector_index_sections(index);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("VectorIndex score_space_authority")
        ));
    }

    #[test]
    fn rejects_vector_index_missing_visibility_scope_ref() {
        let mut index = test_vector_index(0, 0);
        index.visibility_scope_ref = 99;
        let sections = vector_index_sections(index);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing visibility_scope_ref 99")
        ));
    }

    #[test]
    fn rejects_vector_index_missing_dequantization_profile_ref() {
        let mut index = test_vector_index(0, 0);
        index.dequantization_profile_ref = 99;
        let sections = vector_index_sections(index);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing dequantization_profile_ref 99")
        ));
    }

    #[test]
    fn parses_chunk_profile_and_text_chunks() {
        let sections = chunk_sections(vec![
            test_text_chunk(1, 0, 2, 0, 5, 5, 1),
            test_text_chunk(2, 1, 0, 5, 7, 7, 2),
        ]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections)
                .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.chunk_profiles.len(), 1);
        assert_eq!(parsed.descriptor_tables.text_chunks.len(), 2);
        assert_eq!(parsed.descriptor_tables.text_chunks[1].previous_chunk_id, 1);
    }

    #[test]
    fn rejects_chunk_profile_missing_profile_name_ref() {
        let mut profile = test_chunk_profile();
        profile.profile_name_ref = 99;
        let sections =
            chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing profile_name_ref 99")
        ));
    }

    #[test]
    fn rejects_chunk_profile_missing_tokenizer_profile_ref() {
        let mut profile = test_chunk_profile();
        profile.tokenizer_profile_ref = 99;
        let sections =
            chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing tokenizer_profile_ref 99")
        ));
    }

    #[test]
    fn rejects_chunk_profile_unsupported_boundary_kind() {
        let mut profile = test_chunk_profile();
        profile.boundary_kind = 42;
        let sections =
            chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("ChunkProfile boundary_kind")
        ));
    }

    #[test]
    fn rejects_chunk_profile_unsupported_normalization_policy() {
        let mut profile = test_chunk_profile();
        profile.normalization_policy = 42;
        let sections =
            chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("ChunkProfile normalization_policy")
        ));
    }

    #[test]
    fn rejects_chunk_profile_target_tokens_outside_bounds() {
        let mut profile = test_chunk_profile();
        profile.target_tokens = 16;
        profile.max_tokens = 8;
        let sections =
            chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("target_tokens exceeds max_tokens")
        ));
    }

    #[test]
    fn rejects_text_chunk_missing_source_value_hash() {
        let sections = chunk_sections(vec![test_text_chunk(1, 0, 0, 0, 5, 5, 0)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("requires source_value_hash_ref")
        ));
    }

    #[test]
    fn rejects_text_chunk_missing_source_ref() {
        let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
        chunk.source_ref = 99;
        let sections = chunk_sections(vec![chunk]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing source_ref 99")
        ));
    }

    #[test]
    fn rejects_text_chunk_missing_source_value_hash_ref() {
        let sections = chunk_sections(vec![test_text_chunk(1, 0, 0, 0, 5, 5, 99)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing source_value_hash_ref 99")
        ));
    }

    #[test]
    fn rejects_text_chunk_missing_chunk_text_hash_ref() {
        let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
        chunk.chunk_text_hash_ref = 99;
        let sections = chunk_sections(vec![chunk]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing chunk_text_hash_ref 99")
        ));
    }

    #[test]
    fn rejects_text_chunk_missing_evidence_ref() {
        let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
        chunk.evidence_ref = 99;
        let sections = chunk_sections(vec![chunk]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 99")
        ));
    }

    #[test]
    fn rejects_text_chunk_missing_policy_ref() {
        let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
        chunk.policy_ref = 99;
        let sections = chunk_sections(vec![chunk]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 99")
        ));
    }

    #[test]
    fn rejects_text_chunk_broken_neighbor_ref() {
        let sections = chunk_sections(vec![
            test_text_chunk(1, 0, 3, 0, 5, 5, 1),
            test_text_chunk(2, 0, 0, 5, 7, 7, 2),
        ]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [6u8; 16], 789, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing next_chunk_id")
        ));
    }

    #[test]
    fn parses_tokenizer_span_and_sequence_pack() {
        let sections = token_sections(
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections)
                .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(
            parsed.payload_access,
            AiPayloadAccessState::StructurallyAllowed
        );
        assert_eq!(parsed.descriptor_tables.tokenizer_profiles.len(), 1);
        assert_eq!(parsed.descriptor_tables.token_blocks.len(), 1);
        assert_eq!(parsed.descriptor_tables.tokenized_spans.len(), 1);
        assert_eq!(parsed.descriptor_tables.token_sequence_packs.len(), 1);
    }

    #[test]
    fn rejects_tokenizer_profile_missing_namespace_ref() {
        let mut profile = test_tokenizer_profile();
        profile.tokenizer_namespace_ref = 99;
        let sections = token_sections_with_profile(
            profile,
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing tokenizer_namespace_ref 99")
        ));
    }

    #[test]
    fn rejects_tokenizer_profile_missing_vocab_digest_ref() {
        let mut profile = test_tokenizer_profile();
        profile.vocab_digest_ref = 99;
        let sections = token_sections_with_profile(
            profile,
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing vocab_digest_ref 99")
        ));
    }

    #[test]
    fn rejects_tokenizer_profile_missing_chat_template_ref() {
        let mut profile = test_tokenizer_profile();
        profile.chat_template_ref = 99;
        let sections = token_sections_with_profile(
            profile,
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing chat_template_ref 99")
        ));
    }

    #[test]
    fn rejects_tokenizer_profile_missing_truncation_policy_ref() {
        let mut profile = test_tokenizer_profile();
        profile.truncation_policy_ref = 99;
        let sections = token_sections_with_profile(
            profile,
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing truncation_policy_ref 99")
        ));
    }

    #[test]
    fn rejects_tokenizer_profile_special_token_outside_width() {
        let mut profile = test_tokenizer_profile();
        profile.eos_token_id = u32::from(u16::MAX) + 1;
        let sections = token_sections_with_profile(
            profile,
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("eos_token_id 65536 exceeds token_id_width 2")
        ));
    }

    #[test]
    fn rejects_tokenized_span_range_out_of_block() {
        let sections = token_sections(
            test_tokenized_span(1, 0, 3, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [8u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("token range exceeds")
        ));
    }

    #[test]
    fn rejects_token_sequence_pack_missing_mask_payload_ref() {
        let sections = token_sections(
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 99),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing loss_mask_ref")
        ));
    }

    #[test]
    fn rejects_tokenized_span_missing_source_hash_ref() {
        let sections = token_sections(
            test_tokenized_span(1, 0, 1, 2, 99, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing source_value_hash_ref 99")
        ));
    }

    #[test]
    fn rejects_token_sequence_pack_missing_first_source_span_ref() {
        let mut pack = test_token_sequence_pack(1, 0, 4, 0);
        pack.source_span_count = 1;
        pack.first_source_span_ref = 99;
        let sections = token_sections(test_tokenized_span(1, 0, 1, 2, 1, 0), pack);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing first_source_span_ref 99")
        ));
    }

    #[test]
    fn rejects_token_sequence_pack_missing_split_ref() {
        let mut pack = test_token_sequence_pack(1, 0, 4, 0);
        pack.split_ref = 77;
        let sections = token_sections(test_tokenized_span(1, 0, 1, 2, 1, 0), pack);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing split_ref 77")
        ));
    }

    #[test]
    fn rejects_token_sequence_pack_position_payload_too_short() {
        let mut pack = test_token_sequence_pack(1, 0, 2, 0);
        pack.position_ids_ref = 2;
        let sections = token_sections(test_tokenized_span(1, 0, 1, 2, 1, 0), pack);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("position_ids_ref decoded_length is shorter")
        ));
    }

    #[test]
    fn rejects_token_block_payload_too_short() {
        let mut sections = token_sections(
            test_tokenized_span(1, 0, 1, 2, 1, 0),
            test_token_sequence_pack(1, 0, 4, 0),
        );
        sections
            .iter_mut()
            .find(|section| section.section_id == 10)
            .unwrap()
            .payload
            .truncate(6);
        sections
            .iter_mut()
            .find(|section| section.section_id == 11)
            .unwrap()
            .payload = encode_records([
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
                payload_length: 6,
                decoded_length: 6,
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
        .unwrap();

        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [40u8; 16], 927, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("payload decoded_length is shorter")
        ));
    }

    #[test]
    fn parses_training_profile_sample_split_and_epoch() {
        let sections = training_sections(true, test_training_sample(1, 1, 1, 1));
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [10u8; 16], 901, &sections)
                .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.training_profiles.len(), 1);
        assert_eq!(parsed.descriptor_tables.dataset_splits.len(), 1);
        assert_eq!(parsed.descriptor_tables.dedup_groups.len(), 1);
        assert_eq!(parsed.descriptor_tables.training_epoch_plans.len(), 1);
        assert_eq!(parsed.descriptor_tables.training_samples.len(), 1);
    }

    #[test]
    fn rejects_training_sample_missing_profile() {
        let sections = training_sections(false, test_training_sample(1, 99, 1, 1));
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing training_profile_id")
        ));
    }

    #[test]
    fn rejects_training_profile_missing_split_policy_ref() {
        let mut profile = test_training_profile();
        profile.split_policy_ref = 77;
        let sections = training_sections_with_records(
            Some(profile),
            test_dataset_split(),
            test_dedup_group(),
            test_training_epoch_plan(),
            test_training_sample(1, 1, 1, 1),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing split_policy_ref 77")
        ));
    }

    #[test]
    fn rejects_training_sample_missing_input_ref() {
        let mut sample = test_training_sample(1, 1, 1, 1);
        sample.input_ref = 77;
        let sections = training_sections(true, sample);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing input_ref 77")
        ));
    }

    #[test]
    fn rejects_dataset_split_missing_filter_policy_ref() {
        let mut split = test_dataset_split();
        split.filter_policy_ref = 77;
        let sections = training_sections_with_records(
            Some(test_training_profile()),
            split,
            test_dedup_group(),
            test_training_epoch_plan(),
            test_training_sample(1, 1, 1, 1),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing filter_policy_ref 77")
        ));
    }

    #[test]
    fn rejects_dedup_group_bad_authority() {
        let mut dedup = test_dedup_group();
        dedup.dedup_authority = 42;
        let sections = training_sections_with_records(
            Some(test_training_profile()),
            test_dataset_split(),
            dedup,
            test_training_epoch_plan(),
            test_training_sample(1, 1, 1, 1),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("DedupGroup dedup_authority")
        ));
    }

    #[test]
    fn rejects_training_label_missing_policy_ref() {
        let mut label = test_training_label();
        label.policy_ref = 77;
        let sections = provenance_sections_with_label_pair(
            Some(test_model_actor()),
            test_decoding_profile(),
            test_human_review(),
            test_generator_provenance(1, 1),
            label,
            test_preference_pair(),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [12u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 77")
        ));
    }

    #[test]
    fn rejects_preference_pair_missing_evidence_ref() {
        let mut pair = test_preference_pair();
        pair.evidence_ref = 77;
        let sections = provenance_sections_with_label_pair(
            Some(test_model_actor()),
            test_decoding_profile(),
            test_human_review(),
            test_generator_provenance(1, 1),
            test_training_label(),
            pair,
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [12u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 77")
        ));
    }

    #[test]
    fn parses_generator_provenance_labels_and_preferences() {
        let sections = provenance_sections(true, test_generator_provenance(1, 1));
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [12u8; 16], 902, &sections)
                .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.model_actors.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.generation_decoding_profiles.len(),
            1
        );
        assert_eq!(parsed.descriptor_tables.human_reviews.len(), 1);
        assert_eq!(parsed.descriptor_tables.generator_provenance.len(), 1);
        assert_eq!(parsed.descriptor_tables.training_labels.len(), 1);
        assert_eq!(parsed.descriptor_tables.preference_pairs.len(), 1);
    }

    #[test]
    fn rejects_model_generator_missing_actor() {
        let sections = provenance_sections(false, test_generator_provenance(1, 99));
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing model_actor_ref")
        ));
    }

    #[test]
    fn rejects_model_actor_missing_checkpoint_digest_ref() {
        let mut actor = test_model_actor();
        actor.model_checkpoint_digest_ref = 77;
        let sections = provenance_sections_with_records(
            Some(actor),
            test_decoding_profile(),
            test_human_review(),
            test_generator_provenance(1, 1),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing model_checkpoint_digest_ref 77")
        ));
    }

    #[test]
    fn rejects_decoding_profile_missing_safety_policy_ref() {
        let mut decoding = test_decoding_profile();
        decoding.safety_policy_ref = 77;
        let sections = provenance_sections_with_records(
            Some(test_model_actor()),
            decoding,
            test_human_review(),
            test_generator_provenance(1, 1),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing safety_policy_ref 77")
        ));
    }

    #[test]
    fn rejects_human_review_missing_notes_ref() {
        let mut review = test_human_review();
        review.notes_ref = 77;
        let sections = provenance_sections_with_records(
            Some(test_model_actor()),
            test_decoding_profile(),
            review,
            test_generator_provenance(1, 1),
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing notes_ref 77")
        ));
    }

    #[test]
    fn rejects_generator_provenance_missing_prompt_template_ref() {
        let mut generator = test_generator_provenance(1, 1);
        generator.prompt_template_ref = 77;
        let sections = provenance_sections_with_records(
            Some(test_model_actor()),
            test_decoding_profile(),
            test_human_review(),
            generator,
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("missing prompt_template_ref 77")
        ));
    }

    #[test]
    fn rejects_generator_provenance_bad_reproducibility_class() {
        let mut generator = test_generator_provenance(1, 1);
        generator.reproducibility_class = 42;
        let sections = provenance_sections_with_records(
            Some(test_model_actor()),
            test_decoding_profile(),
            test_human_review(),
            generator,
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("GeneratorProvenance reproducibility_class")
        ));
    }

    #[test]
    fn parses_tensor_asset_and_multimodal_sequence() {
        let sections = phase7_sections(
            true,
            test_multimodal_pack(1, 1),
            vec![test_multimodal_element(1, 1, 0, 1, 1)],
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [14u8; 16], 903, &sections)
                .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(parsed.descriptor_tables.tensor_layouts.len(), 1);
        assert_eq!(parsed.descriptor_tables.device_transfer_hints.len(), 1);
        assert_eq!(parsed.descriptor_tables.assets.len(), 1);
        assert_eq!(parsed.descriptor_tables.multimodal_sequence_packs.len(), 1);
        assert_eq!(
            parsed.descriptor_tables.multimodal_sequence_elements.len(),
            1
        );
    }

    #[test]
    fn rejects_tensor_layout_missing_shape_ref() {
        let mut tensor_layout = test_tensor_layout();
        tensor_layout.shape_ref = 99;
        let sections = phase7_sections_with_tensor(
            tensor_layout,
            test_device_transfer_hint(),
            true,
            test_multimodal_pack(1, 1),
            vec![test_multimodal_element(1, 1, 0, 1, 1)],
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing shape_ref 99")
        ));
    }

    #[test]
    fn rejects_tensor_layout_unsupported_dtype() {
        let mut tensor_layout = test_tensor_layout();
        tensor_layout.dtype = 42;
        let sections = phase7_sections_with_tensor(
            tensor_layout,
            test_device_transfer_hint(),
            true,
            test_multimodal_pack(1, 1),
            vec![test_multimodal_element(1, 1, 0, 1, 1)],
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("TensorLayout dtype")
        ));
    }

    #[test]
    fn rejects_device_transfer_missing_runtime_binding() {
        let mut transfer_hint = test_device_transfer_hint();
        transfer_hint.runtime_registry_binding_ref = 77;
        let sections = phase7_sections_with_tensor(
            test_tensor_layout(),
            transfer_hint,
            true,
            test_multimodal_pack(1, 1),
            vec![test_multimodal_element(1, 1, 0, 1, 1)],
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing runtime_registry_binding_ref 77")
        ));
    }

    #[test]
    fn rejects_uri_asset_without_uri_ref() {
        let mut asset = test_asset_ref();
        asset.asset_kind = 0;
        asset.uri_ref = 0;
        let sections = phase7_sections_with_asset(
            asset,
            test_multimodal_pack(1, 1),
            vec![test_multimodal_element(1, 1, 0, 1, 1)],
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("URI asset requires uri_ref")
        ));
    }

    #[test]
    fn rejects_asset_missing_policy_ref() {
        let mut asset = test_asset_ref();
        asset.policy_ref = 77;
        let sections = phase7_sections_with_asset(
            asset,
            test_multimodal_pack(1, 1),
            vec![test_multimodal_element(1, 1, 0, 1, 1)],
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 77")
        ));
    }

    #[test]
    fn rejects_multimodal_element_unsupported_role() {
        let mut element = test_multimodal_element(1, 1, 0, 1, 1);
        element.role = 42;
        let sections = phase7_sections(true, test_multimodal_pack(1, 1), vec![element]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("MultimodalSequenceElement role")
        ));
    }

    #[test]
    fn rejects_multimodal_element_missing_policy_ref() {
        let mut element = test_multimodal_element(1, 1, 0, 1, 1);
        element.policy_ref = 77;
        let sections = phase7_sections(true, test_multimodal_pack(1, 1), vec![element]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 77")
        ));
    }

    #[test]
    fn rejects_multimodal_pack_missing_evidence_ref() {
        let mut pack = test_multimodal_pack(1, 1);
        pack.evidence_ref = 88;
        let sections = phase7_sections(true, pack, vec![test_multimodal_element(1, 1, 0, 1, 1)]);
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 88")
        ));
    }

    #[test]
    fn rejects_multimodal_element_missing_asset_ref() {
        let sections = phase7_sections(
            false,
            test_multimodal_pack(1, 1),
            vec![test_multimodal_element(1, 1, 0, 99, 1)],
        );
        let bytes =
            write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
                .unwrap();
        assert!(matches!(
            CoveAiFile::parse(&bytes),
            Err(CoveError::BadSection(message)) if message.contains("missing asset_ref")
        ));
    }

    fn test_chunk_profile() -> ChunkProfileV1 {
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

    fn chunk_sections(text_chunks: Vec<TextChunkEntryV1>) -> Vec<CoveAiWritableSection> {
        chunk_sections_with_profile(test_chunk_profile(), text_chunks)
    }

    fn chunk_sections_with_profile(
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

    fn test_text_chunk(
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

    fn vector_index_sections(index: VectorIndexDescriptorV1) -> Vec<CoveAiWritableSection> {
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

    fn test_vector_index(index_kind: u8, exactness_kind: u8) -> VectorIndexDescriptorV1 {
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

    fn token_sections(
        tokenized_span: TokenizedSpanV1,
        sequence_pack: TokenSequencePackV1,
    ) -> Vec<CoveAiWritableSection> {
        token_sections_with_profile(test_tokenizer_profile(), tokenized_span, sequence_pack)
    }

    fn token_sections_with_profile(
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

    fn test_tokenizer_profile() -> TokenizerProfileV1 {
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

    fn test_tokenized_span(
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

    fn test_token_sequence_pack(
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

    fn training_sections(
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

    fn training_sections_with_records(
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

    fn test_training_profile() -> TrainingProfileV1 {
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

    fn test_training_sample(
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

    fn test_dataset_split() -> DatasetSplitV1 {
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

    fn test_dedup_group() -> DedupGroupV1 {
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

    fn test_training_epoch_plan() -> TrainingEpochPlanV1 {
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

    fn provenance_sections(
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

    fn provenance_sections_with_records(
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

    fn provenance_sections_with_label_pair(
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

    fn test_model_actor() -> ModelActorDescriptorV1 {
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

    fn test_decoding_profile() -> GenerationDecodingProfileV1 {
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

    fn test_human_review() -> HumanReviewEntryV1 {
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

    fn test_generator_provenance(
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

    fn test_training_label() -> TrainingLabelEntryV1 {
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

    fn test_preference_pair() -> PreferencePairEntryV1 {
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

    fn phase7_sections(
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

    fn phase7_sections_with_tensor(
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

    fn phase7_sections_with_asset(
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

    fn test_tensor_layout() -> TensorLayoutDescriptorV1 {
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

    fn test_device_transfer_hint() -> DeviceTransferHintV1 {
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

    fn test_asset_ref() -> AiAssetRefV1 {
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

    fn test_multimodal_pack(sequence_pack_id: u64, element_count: u32) -> MultimodalSequencePackV1 {
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

    fn test_multimodal_element(
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
}
