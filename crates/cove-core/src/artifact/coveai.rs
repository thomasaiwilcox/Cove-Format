//! COVE-AI companion artifact envelope (`.coveai` / `.covev`).
//!
//! This module implements the shared structural layer for `CVA2` and `CVV2`
//! artifacts: tail discovery, postscript/header validation, section directory
//! validation, section range checks, descriptor record parsing, reference-table
//! validation, payload-ref validation, and the Phase 1 token/vector payload
//! carrier rules. Higher-level COVE-CHUNK, COVE-TOK, COVE-VEC, COVE-TRAIN, and
//! COVE-MMSEQ semantics are layered on top of these validated descriptor tables.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use crate::{
    checksum, compression,
    constants::{
        CompressionCodec, PrimaryProfile, SectionKind, AI_FEATURE_ASSET_REF,
        AI_FEATURE_CANONICAL_FIXED_POINT_VECTOR, AI_FEATURE_CHUNK, AI_FEATURE_COVEQL_AI,
        AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED, AI_FEATURE_GENERATOR_PROVENANCE,
        AI_FEATURE_MAP_AI_POLICY, AI_FEATURE_MMSEQ, AI_FEATURE_MODEL_INPUT_IDENTITY,
        AI_FEATURE_PRIVACY_SUMMARY, AI_FEATURE_TENSOR_LAYOUT, AI_FEATURE_TOKEN, AI_FEATURE_TRAIN,
        AI_FEATURE_VECTOR, AI_FEATURE_VECTOR_INDEX, AI_FEATURE_VECTOR_SPACE_COMPATIBILITY,
        MAGIC_COVEAI, MAGIC_COVEV,
    },
    feature_binding::{FeatureScopeV2, OperationKindV2},
    wire::{
        read_array_checked as read_array, read_i64_le_checked as read_i64,
        read_u16_le_checked as read_u16, read_u32_le_checked as read_u32,
        read_u64_le_checked as read_u64, read_u8_checked as read_u8,
    },
    CoveError,
};

mod model_input;
mod payload_access;
mod records;
mod runtime;
mod validation;
mod wire_parse;
mod wire_write;

use model_input::{EffectiveSourceRef, ModelInputDigestRef, ModelInputVectorKey, VectorSpaceId};
use payload_access::payload_access_state;
#[cfg(test)]
use runtime::*;
use validation::*;
use wire_parse::*;
use wire_write::*;

pub use payload_access::AiPayloadAccessState;
pub use records::*;
pub use runtime::*;
pub use wire_write::*;

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
    | AI_FEATURE_VECTOR_SPACE_COMPATIBILITY
    | AI_FEATURE_MODEL_INPUT_IDENTITY;

pub const AI_FLAG_REQUIRED_RECORD: u32 = 1 << 0;
pub const AI_FLAG_PAYLOAD_CRC32C_PRESENT: u32 = 1 << 1;
pub const AI_FLAG_POLICY_PROTECTED: u32 = 1 << 2;
pub const AI_FLAG_REVOKED: u32 = 1 << 3;

const AI_DIGEST_DOMAIN_MODEL_INPUT_BYTES: u8 = 4;

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

        descriptor_tables.validate(&sections, data.len() as u64, header.required_ai_features)?;
        let payload_access =
            payload_access_state(&sections, !descriptor_tables.privacy_summaries.is_empty());

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
    pub association_state_vector_bindings: Vec<AssociationStateVectorBindingV1>,
    pub asset_vector_bindings: Vec<AssetVectorBindingV1>,
    pub multimodal_sequence_vector_bindings: Vec<MultimodalSequenceVectorBindingV1>,
    pub vector_payload_blocks: Vec<VectorPayloadBlockHeaderV1>,
    pub vector_entries: Vec<VectorEntryV1>,
    pub vector_composition_profiles: Vec<VectorCompositionProfileV1>,
    pub vector_composition_components: Vec<VectorCompositionComponentV1>,
    pub vector_arithmetic_profiles: Vec<VectorArithmeticProfileV1>,
    pub vector_indexes: Vec<VectorIndexDescriptorV1>,
}

impl AiDescriptorTablesV1 {
    fn validate(
        &self,
        sections: &[CoveAiSection],
        file_len: u64,
        artifact_required_ai_features: u64,
    ) -> Result<(), CoveError> {
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
        validate_unique_u64(
            self.association_state_vector_bindings
                .iter()
                .map(|record| record.binding_id),
            "AI_VECTOR_BINDING association_state binding_id",
        )?;
        validate_unique_u64(
            self.asset_vector_bindings
                .iter()
                .map(|record| record.binding_id),
            "AI_VECTOR_BINDING asset binding_id",
        )?;
        validate_unique_u64(
            self.multimodal_sequence_vector_bindings
                .iter()
                .map(|record| record.binding_id),
            "AI_VECTOR_BINDING multimodal_sequence binding_id",
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
        let model_input_digest_refs = self
            .digests
            .iter()
            .filter(|record| record.domain_hint == AI_DIGEST_DOMAIN_MODEL_INPUT_BYTES)
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
            pack.validate(TokenSequencePackValidateRefs {
                tables: self,
                tokenizer_profile_ids: &tokenizer_profile_ids,
                token_block_ids: &token_block_ids,
                payload_ref_ids: &payload_ref_ids,
                training_profile_ids: &training_profile_ids,
                split_ids: &split_ids,
                tokenized_span_ids: &tokenized_span_ids,
            })?;
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
            sample.validate(TrainingSampleValidateRefs {
                training_profile_ids: &training_profile_ids,
                split_ids: &split_ids,
                dedup_group_ids: &dedup_group_ids,
                token_sequence_pack_ids: &token_sequence_pack_ids,
                multimodal_sequence_pack_ids: &multimodal_sequence_pack_ids,
                vector_ref_ids: &vector_ref_ids,
                training_label_ids: &training_label_ids,
                generator_provenance_ids: &generator_provenance_ids,
                payload_ref_ids: &payload_ref_ids,
                policy_ref_ids: &policy_ref_ids,
                model_actor_ids: &model_actor_ids,
            })?;
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
            provenance.validate(GeneratorProvenanceValidateRefs {
                generator_provenance_ids: &generator_provenance_ids,
                model_actor_ids: &model_actor_ids,
                decoding_profile_ids: &decoding_profile_ids,
                human_review_ids: &human_review_ids,
                payload_ref_ids: &payload_ref_ids,
                policy_ref_ids: &policy_ref_ids,
                training_sample_ids: &training_sample_ids,
            })?;
        }
        for tensor_layout in &self.tensor_layouts {
            tensor_layout.validate(&string_ref_ids, &payload_ref_ids)?;
        }
        for transfer_hint in &self.device_transfer_hints {
            transfer_hint.validate(&string_ref_ids)?;
        }
        for asset in &self.assets {
            asset.validate(AiAssetRefValidateRefs {
                asset_ref_ids: &asset_ref_ids,
                tensor_layout_ids: &tensor_layout_ids,
                section_ids: &section_ids,
                string_ref_ids: &string_ref_ids,
                digest_ref_ids: &digest_ref_ids,
                transform_ids: &transform_ids,
                policy_ref_ids: &policy_ref_ids,
            })?;
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
                &model_input_digest_refs,
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
                &model_input_digest_refs,
                &vector_ref_ids,
            )?;
        }
        for binding in &self.object_state_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &composition_profile_ids,
                &source_binding_ids,
                &digest_ref_ids,
                &model_input_digest_refs,
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
                &model_input_digest_refs,
                &vector_ref_ids,
            )?;
        }
        for binding in &self.association_state_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &composition_profile_ids,
                &source_binding_ids,
                &digest_ref_ids,
                &model_input_digest_refs,
                &string_ref_ids,
                &vector_ref_ids,
            )?;
        }
        for binding in &self.asset_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &asset_ref_ids,
                &transform_ids,
                &digest_ref_ids,
                &model_input_digest_refs,
                &vector_ref_ids,
            )?;
        }
        for binding in &self.multimodal_sequence_vector_bindings {
            binding.validate(
                &vector_space_ids,
                &multimodal_sequence_pack_ids,
                &source_binding_ids,
                &payload_ref_ids,
                &digest_ref_ids,
                &model_input_digest_refs,
                &vector_ref_ids,
            )?;
        }
        self.validate_vector_model_input_identity(sections, artifact_required_ai_features)?;
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

    fn validate_vector_model_input_identity(
        &self,
        sections: &[CoveAiSection],
        artifact_required_ai_features: u64,
    ) -> Result<(), CoveError> {
        let identity_required =
            artifact_required_ai_features & AI_FEATURE_MODEL_INPUT_IDENTITY != 0
                || sections.iter().any(|section| {
                    section.entry.required_ai_features & AI_FEATURE_MODEL_INPUT_IDENTITY != 0
                })
                || self.section_feature_bindings.iter().any(|binding| {
                    binding.required_ai_features & AI_FEATURE_MODEL_INPUT_IDENTITY != 0
                });
        let vector_binding_section_source_ref =
            unique_section_source_binding_ref(sections, SectionKind::AiVectorBinding as u32)?;
        let chunk_section_source_ref =
            unique_section_source_binding_ref(sections, SectionKind::AiTextChunkIndex as u32)?;
        let chunk_source_refs = self
            .text_chunks
            .iter()
            .map(|chunk| {
                (
                    chunk.chunk_id,
                    if chunk.source_ref != 0 {
                        chunk.source_ref
                    } else {
                        chunk_section_source_ref
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        let mut model_input_vectors = BTreeMap::<ModelInputVectorKey, u64>::new();
        let mut semantic_keys = BTreeSet::<VectorSemanticBindingKey>::new();

        for binding in &self.filecode_vector_bindings {
            require_model_input_identity_if_needed(
                identity_required,
                "FileCodeVectorBinding",
                binding.binding_id,
                binding.model_input_digest_ref,
            )?;
            let effective_source =
                effective_source_binding(binding.file_ref, 0, vector_binding_section_source_ref);
            validate_model_input_vector_mapping(
                "FileCodeVectorBinding",
                binding.binding_id,
                effective_source,
                binding.vector_space_id,
                binding.model_input_digest_ref,
                binding.vector_ref,
                &mut model_input_vectors,
            )?;
            insert_vector_semantic_key(
                &mut semantic_keys,
                VectorSemanticBindingKey::FileCode {
                    effective_source,
                    vector_space_id: binding.vector_space_id,
                    slot_policy_ref: binding.slot_policy_ref,
                    file_ref: binding.file_ref,
                    dictionary_digest_ref: binding.dictionary_digest_ref,
                    schema_fingerprint_ref: binding.schema_fingerprint_ref,
                    table_id: binding.table_id,
                    column_id: binding.column_id,
                    object_type_id: binding.object_type_id,
                    property_id: binding.property_id,
                    association_type_id: binding.association_type_id,
                    path_ref: binding.path_ref,
                    file_code: binding.file_code,
                    canonical_value_hash_ref: binding.canonical_value_hash_ref,
                    model_input_digest_ref: binding.model_input_digest_ref,
                },
                "FileCodeVectorBinding",
                binding.binding_id,
            )?;
        }

        for binding in &self.chunk_vector_bindings {
            require_model_input_identity_if_needed(
                identity_required,
                "ChunkVectorBinding",
                binding.binding_id,
                binding.model_input_digest_ref,
            )?;
            let chunk_source_ref = chunk_source_refs
                .get(&binding.chunk_id)
                .copied()
                .unwrap_or_default();
            let effective_source =
                effective_source_binding(0, chunk_source_ref, vector_binding_section_source_ref);
            validate_model_input_vector_mapping(
                "ChunkVectorBinding",
                binding.binding_id,
                effective_source,
                binding.vector_space_id,
                binding.model_input_digest_ref,
                binding.vector_ref,
                &mut model_input_vectors,
            )?;
            insert_vector_semantic_key(
                &mut semantic_keys,
                VectorSemanticBindingKey::Chunk {
                    effective_source,
                    vector_space_id: binding.vector_space_id,
                    chunk_id: binding.chunk_id,
                    chunk_profile_id: binding.chunk_profile_id,
                    source_value_hash_ref: binding.source_value_hash_ref,
                    chunk_text_hash_ref: binding.chunk_text_hash_ref,
                    model_input_digest_ref: binding.model_input_digest_ref,
                },
                "ChunkVectorBinding",
                binding.binding_id,
            )?;
        }

        for binding in &self.object_state_vector_bindings {
            require_model_input_identity_if_needed(
                identity_required,
                "ObjectStateVectorBinding",
                binding.binding_id,
                binding.model_input_digest_ref,
            )?;
            let effective_source =
                effective_source_binding(binding.file_ref, 0, vector_binding_section_source_ref);
            validate_model_input_vector_mapping(
                "ObjectStateVectorBinding",
                binding.binding_id,
                effective_source,
                binding.vector_space_id,
                binding.model_input_digest_ref,
                binding.vector_ref,
                &mut model_input_vectors,
            )?;
            insert_vector_semantic_key(
                &mut semantic_keys,
                VectorSemanticBindingKey::ObjectState {
                    effective_source,
                    vector_space_id: binding.vector_space_id,
                    composition_profile_ref: binding.composition_profile_ref,
                    file_ref: binding.file_ref,
                    object_type_id: binding.object_type_id,
                    goid_ref: binding.goid_ref,
                    branch_ref: binding.branch_ref,
                    temporal_kind: binding.temporal_kind,
                    csn: binding.csn,
                    timestamp_us: binding.timestamp_us,
                    property_dependency_fingerprint_ref: binding
                        .property_dependency_fingerprint_ref,
                    model_input_digest_ref: binding.model_input_digest_ref,
                },
                "ObjectStateVectorBinding",
                binding.binding_id,
            )?;
        }

        for binding in &self.training_sample_vector_bindings {
            require_model_input_identity_if_needed(
                identity_required,
                "TrainingSampleVectorBinding",
                binding.binding_id,
                binding.model_input_digest_ref,
            )?;
            let effective_source = effective_source_binding(
                binding.source_snapshot_ref,
                0,
                vector_binding_section_source_ref,
            );
            validate_model_input_vector_mapping(
                "TrainingSampleVectorBinding",
                binding.binding_id,
                effective_source,
                binding.vector_space_id,
                binding.model_input_digest_ref,
                binding.vector_ref,
                &mut model_input_vectors,
            )?;
            insert_vector_semantic_key(
                &mut semantic_keys,
                VectorSemanticBindingKey::TrainingSample {
                    effective_source,
                    vector_space_id: binding.vector_space_id,
                    training_profile_ref: binding.training_profile_ref,
                    sample_id: binding.sample_id,
                    source_snapshot_ref: binding.source_snapshot_ref,
                    sample_fingerprint_ref: binding.sample_fingerprint_ref,
                    model_input_digest_ref: binding.model_input_digest_ref,
                },
                "TrainingSampleVectorBinding",
                binding.binding_id,
            )?;
        }

        for binding in &self.association_state_vector_bindings {
            require_model_input_identity_if_needed(
                identity_required,
                "AssociationStateVectorBinding",
                binding.binding_id,
                binding.model_input_digest_ref,
            )?;
            let effective_source =
                effective_source_binding(binding.file_ref, 0, vector_binding_section_source_ref);
            validate_model_input_vector_mapping(
                "AssociationStateVectorBinding",
                binding.binding_id,
                effective_source,
                binding.vector_space_id,
                binding.model_input_digest_ref,
                binding.vector_ref,
                &mut model_input_vectors,
            )?;
            insert_vector_semantic_key(
                &mut semantic_keys,
                VectorSemanticBindingKey::AssociationState {
                    effective_source,
                    vector_space_id: binding.vector_space_id,
                    composition_profile_ref: binding.composition_profile_ref,
                    file_ref: binding.file_ref,
                    association_type_id: binding.association_type_id,
                    association_key_ref: binding.association_key_ref,
                    branch_ref: binding.branch_ref,
                    temporal_kind: binding.temporal_kind,
                    csn: binding.csn,
                    timestamp_us: binding.timestamp_us,
                    property_dependency_fingerprint_ref: binding
                        .property_dependency_fingerprint_ref,
                    model_input_digest_ref: binding.model_input_digest_ref,
                },
                "AssociationStateVectorBinding",
                binding.binding_id,
            )?;
        }

        for binding in &self.asset_vector_bindings {
            require_model_input_identity_if_needed(
                identity_required,
                "AssetVectorBinding",
                binding.binding_id,
                binding.model_input_digest_ref,
            )?;
            validate_model_input_vector_mapping(
                "AssetVectorBinding",
                binding.binding_id,
                vector_binding_section_source_ref,
                binding.vector_space_id,
                binding.model_input_digest_ref,
                binding.vector_ref,
                &mut model_input_vectors,
            )?;
            insert_vector_semantic_key(
                &mut semantic_keys,
                VectorSemanticBindingKey::Asset {
                    effective_source: vector_binding_section_source_ref,
                    vector_space_id: binding.vector_space_id,
                    asset_ref: binding.asset_ref,
                    transform_ref: binding.transform_ref,
                    asset_digest_ref: binding.asset_digest_ref,
                    model_input_digest_ref: binding.model_input_digest_ref,
                },
                "AssetVectorBinding",
                binding.binding_id,
            )?;
        }

        for binding in &self.multimodal_sequence_vector_bindings {
            require_model_input_identity_if_needed(
                identity_required,
                "MultimodalSequenceVectorBinding",
                binding.binding_id,
                binding.model_input_digest_ref,
            )?;
            let effective_source = effective_source_binding(
                binding.source_snapshot_ref,
                0,
                vector_binding_section_source_ref,
            );
            validate_model_input_vector_mapping(
                "MultimodalSequenceVectorBinding",
                binding.binding_id,
                effective_source,
                binding.vector_space_id,
                binding.model_input_digest_ref,
                binding.vector_ref,
                &mut model_input_vectors,
            )?;
            insert_vector_semantic_key(
                &mut semantic_keys,
                VectorSemanticBindingKey::MultimodalSequence {
                    effective_source,
                    vector_space_id: binding.vector_space_id,
                    sequence_pack_id: binding.sequence_pack_id,
                    sequence_profile_ref: binding.sequence_profile_ref,
                    source_snapshot_ref: binding.source_snapshot_ref,
                    model_input_digest_ref: binding.model_input_digest_ref,
                },
                "MultimodalSequenceVectorBinding",
                binding.binding_id,
            )?;
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum VectorSemanticBindingKey {
    FileCode {
        effective_source: u32,
        vector_space_id: u32,
        slot_policy_ref: u32,
        file_ref: u32,
        dictionary_digest_ref: u32,
        schema_fingerprint_ref: u32,
        table_id: u32,
        column_id: u32,
        object_type_id: u32,
        property_id: u32,
        association_type_id: u32,
        path_ref: u32,
        file_code: u32,
        canonical_value_hash_ref: u32,
        model_input_digest_ref: u32,
    },
    Chunk {
        effective_source: u32,
        vector_space_id: u32,
        chunk_id: u64,
        chunk_profile_id: u32,
        source_value_hash_ref: u32,
        chunk_text_hash_ref: u32,
        model_input_digest_ref: u32,
    },
    ObjectState {
        effective_source: u32,
        vector_space_id: u32,
        composition_profile_ref: u32,
        file_ref: u32,
        object_type_id: u32,
        goid_ref: u32,
        branch_ref: u32,
        temporal_kind: u8,
        csn: u64,
        timestamp_us: i64,
        property_dependency_fingerprint_ref: u32,
        model_input_digest_ref: u32,
    },
    TrainingSample {
        effective_source: u32,
        vector_space_id: u32,
        training_profile_ref: u32,
        sample_id: u64,
        source_snapshot_ref: u32,
        sample_fingerprint_ref: u32,
        model_input_digest_ref: u32,
    },
    AssociationState {
        effective_source: u32,
        vector_space_id: u32,
        composition_profile_ref: u32,
        file_ref: u32,
        association_type_id: u32,
        association_key_ref: u32,
        branch_ref: u32,
        temporal_kind: u8,
        csn: u64,
        timestamp_us: i64,
        property_dependency_fingerprint_ref: u32,
        model_input_digest_ref: u32,
    },
    Asset {
        effective_source: u32,
        vector_space_id: u32,
        asset_ref: u64,
        transform_ref: u32,
        asset_digest_ref: u32,
        model_input_digest_ref: u32,
    },
    MultimodalSequence {
        effective_source: u32,
        vector_space_id: u32,
        sequence_pack_id: u64,
        sequence_profile_ref: u32,
        source_snapshot_ref: u32,
        model_input_digest_ref: u32,
    },
}

fn unique_section_source_binding_ref(
    sections: &[CoveAiSection],
    section_kind: u32,
) -> Result<u32, CoveError> {
    let mut source_binding_ref = 0;
    for section in sections
        .iter()
        .filter(|section| section.entry.section_kind == section_kind)
    {
        let candidate = section.entry.source_binding_ref;
        if candidate == 0 {
            continue;
        }
        if source_binding_ref != 0 && source_binding_ref != candidate {
            return Err(CoveError::BadSection(format!(
                "multiple source_binding_ref values for section kind {section_kind} cannot be used for effective source fallback"
            )));
        }
        source_binding_ref = candidate;
    }
    Ok(source_binding_ref)
}

fn effective_source_binding(
    own_source_ref: u32,
    referenced_source_ref: u32,
    section_source_ref: u32,
) -> u32 {
    if own_source_ref != 0 {
        own_source_ref
    } else if referenced_source_ref != 0 {
        referenced_source_ref
    } else {
        section_source_ref
    }
}

fn require_model_input_identity_if_needed(
    identity_required: bool,
    label: &str,
    binding_id: u64,
    model_input_digest_ref: u32,
) -> Result<(), CoveError> {
    if identity_required && model_input_digest_ref == 0 {
        return Err(CoveError::BadSection(format!(
            "AI_FEATURE_MODEL_INPUT_IDENTITY requires {label} {binding_id} to carry a non-zero model_input_digest_ref"
        )));
    }
    Ok(())
}

fn validate_model_input_vector_mapping(
    label: &str,
    binding_id: u64,
    effective_source: u32,
    vector_space_id: u32,
    model_input_digest_ref: u32,
    vector_ref: u64,
    model_input_vectors: &mut BTreeMap<ModelInputVectorKey, u64>,
) -> Result<(), CoveError> {
    let Some(model_input_digest_ref_id) = ModelInputDigestRef::non_zero(model_input_digest_ref)
    else {
        return Ok(());
    };
    let key = ModelInputVectorKey {
        effective_source: EffectiveSourceRef::from_raw(effective_source),
        vector_space_id: VectorSpaceId::from_raw(vector_space_id),
        model_input_digest_ref: model_input_digest_ref_id,
    };
    if let Some(existing_vector_ref) = model_input_vectors.get(&key) {
        if *existing_vector_ref != vector_ref {
            let model_input_digest_ref = model_input_digest_ref_id.raw();
            return Err(CoveError::BadSection(format!(
                "{label} {binding_id} maps model_input_digest_ref {model_input_digest_ref} in vector_space_id {vector_space_id} to vector_ref {vector_ref}, but the same effective source already maps it to vector_ref {existing_vector_ref}"
            )));
        }
    } else {
        model_input_vectors.insert(key, vector_ref);
    }
    Ok(())
}

fn insert_vector_semantic_key(
    semantic_keys: &mut BTreeSet<VectorSemanticBindingKey>,
    key: VectorSemanticBindingKey,
    label: &str,
    binding_id: u64,
) -> Result<(), CoveError> {
    if !semantic_keys.insert(key) {
        return Err(CoveError::BadSection(format!(
            "duplicate {label} semantic binding key for binding_id {binding_id}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
