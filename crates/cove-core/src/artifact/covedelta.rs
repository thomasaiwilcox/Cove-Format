//! COVE-O `.covedelta` immutable delta artifact envelope.
//!
//! This module implements the structural envelope from Spec §63.1:
//! `[header][parent refs][section payloads][section directory][footer][postscript][tail]`
//! with final magic `CVD2`. Payload-specific temporal delta semantics are layered
//! on top of this envelope by later phases.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    canonical, checksum,
    constants::{CoveLogicalType, DigestAlgorithm, ValueTag, MAGIC_COVEDELTA},
    digest::compute_digest,
    profile::{
        cove_map::{MapEvidenceIndex, MapProjectionCatalog},
        cove_o::{ObjectTypeCatalog, RecordKind, TemporalSegmentData},
    },
    wire, CoveError,
};

pub const COVEDELTA_VERSION_MAJOR_V1: u16 = 1;
pub const COVEDELTA_VERSION_MINOR_V1: u16 = 0;
pub const COVEDELTA_POSTSCRIPT_VERSION_V1: u16 = 1;

pub const COVEDELTA_HEADER_LEN: u16 = 238;
pub const COVEDELTA_PARENT_REF_LEN: usize = 85;
pub const COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN: usize = 68;
pub const COVEDELTA_FOOTER_LEN: u16 = 48;
pub const COVEDELTA_POSTSCRIPT_LEN: u16 = 44;
pub const COVEDELTA_POSTSCRIPT_TAIL_SIZE: usize = 2 + 2 + 4;
pub const DELTA_BRANCH_IDENTITY_LEN: usize = 37;
pub const DELTA_CONTINUATION_ANCHOR_LEN: usize = 95;
pub const DELTA_STATE_HASH_DESCRIPTOR_LEN: usize = 21;
pub const DELTA_DICTIONARY_ENTRY_LEN: usize = 57;
pub const DELTA_INLINE_VALUE_HEADER_LEN: usize = 16;
pub const DELTA_SIDECAR_HINT_LEN: usize = 32;
pub const DELTA_SCOPE_DESCRIPTOR_LEN: usize = 28;
pub const DELTA_SUMMARY_DESCRIPTOR_LEN: usize = 25;
pub const DELTA_SPARSE_PATCH_RECORD_HEADER_LEN: usize = 92;
pub const DELTA_SPARSE_PATCH_PROPERTY_OP_LEN: usize = 20;
pub const DELTA_TOUCHED_OBJECT_RANGE_LEN: usize = 74;

pub const DELTA_FLAG_SINGLE_SCOPE: u32 = 0x0000_0001;
pub const DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT: u32 = 0x0000_0002;
pub const DELTA_PARENT_REF_LINEAGE_PARENT: u32 = 0x0000_0001;

pub const DELTA_FEATURE_SPARSE_PATCH_ROWS: u64 = 1 << 0;
pub const DELTA_FEATURE_OBJECT_TOMBSTONES: u64 = 1 << 1;
pub const DELTA_FEATURE_PROPERTY_TOMBSTONES: u64 = 1 << 2;
pub const DELTA_FEATURE_ASSOCIATION_TOMBSTONES: u64 = 1 << 3;
pub const DELTA_FEATURE_CONTINUATION_ANCHORS: u64 = 1 << 4;
pub const DELTA_FEATURE_INLINE_DICTIONARY: u64 = 1 << 5;
pub const DELTA_FEATURE_PARENT_DICTIONARY_ALIASES: u64 = 1 << 6;
pub const DELTA_FEATURE_EXACT_TOUCHED_SET: u64 = 1 << 7;
pub const DELTA_FEATURE_EXACT_TOMBSTONE_SET: u64 = 1 << 8;
pub const DELTA_FEATURE_CHECKPOINT_BASELINES: u64 = 1 << 9;
pub const DELTA_FEATURE_COVERAGE_PATCH: u64 = 1 << 10;
pub const DELTA_FEATURE_INDEX_HINTS: u64 = 1 << 11;
pub const DELTA_FEATURE_MAP_EVIDENCE_PATCH: u64 = 1 << 12;
pub const DELTA_FEATURE_PROJECTION_PATCH: u64 = 1 << 13;
pub const DELTA_FEATURE_HISTORICAL_COMMIT_INSERT: u64 = 1 << 14;

pub const COVEDELTA_OBJECT_TEMPORAL_SUPPORTED_REQUIRED_FEATURES: u64 =
    DELTA_FEATURE_SPARSE_PATCH_ROWS
        | DELTA_FEATURE_OBJECT_TOMBSTONES
        | DELTA_FEATURE_CONTINUATION_ANCHORS
        | DELTA_FEATURE_INLINE_DICTIONARY
        | DELTA_FEATURE_PARENT_DICTIONARY_ALIASES
        | DELTA_FEATURE_EXACT_TOUCHED_SET
        | DELTA_FEATURE_EXACT_TOMBSTONE_SET
        | DELTA_FEATURE_CHECKPOINT_BASELINES
        | DELTA_FEATURE_COVERAGE_PATCH
        | DELTA_FEATURE_INDEX_HINTS
        | DELTA_FEATURE_MAP_EVIDENCE_PATCH
        | DELTA_FEATURE_PROJECTION_PATCH;

pub const DELTA_ANCHOR_STRENGTH_KEY_ONLY: u8 = 0;
pub const DELTA_ANCHOR_STRENGTH_KEY_AND_RECORD_ID: u8 = 1;
pub const DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH: u8 = 2;
pub const DELTA_ANCHOR_STRENGTH_KEY_RECORD_STATE_AND_TRUST_HASH: u8 = 3;
pub const DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF: u8 = 0;
pub const DELTA_BRANCH_IDENTITY_KIND_HASH_ONLY: u8 = 1;
pub const DELTA_BRANCH_IDENTITY_KIND_EXTENSION: u8 = 255;
pub const DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1: u8 = 0;
pub const DELTA_STATE_HASH_KIND_COVE_O_TRUST_HASH: u8 = 1;
pub const DELTA_STATE_HASH_KIND_EXTENSION: u8 = 255;
pub const DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE: u8 = 0;
pub const DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS: u8 = 1;
pub const DELTA_DICTIONARY_ENTRY_KIND_CANONICAL_HASH_HINT: u8 = 2;
pub const DELTA_SIDECAR_HINT_KIND_COVI_INDEX: u16 = 0;
pub const DELTA_SIDECAR_HINT_KIND_COVX_INDEX: u16 = 1;
pub const DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH: u16 = 2;
pub const DELTA_SIDECAR_HINT_KIND_LAYOUT_HINTS: u16 = 3;
pub const DELTA_SIDECAR_HINT_KIND_EXTENSION: u16 = u16::MAX;
pub const DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET: u8 = 0;
pub const DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET: u8 = 1;
pub const DELTA_SUMMARY_DESCRIPTOR_KIND_CONSERVATIVE_RANGE: u8 = 2;
pub const DELTA_SUMMARY_DESCRIPTOR_KIND_NO_FALSE_NEGATIVE_BLOOM: u8 = 3;
pub const DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP: u8 = 4;
pub const DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE: u8 = 5;
pub const DELTA_SUMMARY_DESCRIPTOR_KIND_EXTENSION: u8 = 255;
pub const DELTA_OBJECT_STATE_TOMBSTONE_LIVE: u8 = 0;
pub const DELTA_OBJECT_STATE_TOMBSTONE_DELETED: u8 = 1;
pub const DELTA_OBJECT_STATE_VALUE_VISIBLE: u8 = 0;
pub const DELTA_OBJECT_STATE_VALUE_NULL: u8 = 1;
pub const DELTA_OBJECT_STATE_VALUE_CLEAR: u8 = 2;
pub const DELTA_OBJECT_STATE_VALUE_TOMBSTONE: u8 = 3;
pub const DELTA_OBJECT_STATE_VALUE_REDACTED: u8 = 4;
pub const DELTA_PROPERTY_OP_SET_VALUE: u8 = 0;
pub const DELTA_PROPERTY_OP_SET_NULL: u8 = 1;
pub const DELTA_PROPERTY_OP_CLEAR: u8 = 2;
pub const DELTA_PROPERTY_OP_TOMBSTONE: u8 = 3;
pub const DELTA_PROPERTY_OP_REDACT: u8 = 4;
pub const DELTA_TOMBSTONE_KIND_OBJECT: u8 = 0;
pub const DELTA_TOMBSTONE_KIND_PROPERTY: u8 = 1;
pub const DELTA_TOMBSTONE_KIND_ASSOCIATION: u8 = 2;
pub const DELTA_TOMBSTONE_KIND_EVIDENCE: u8 = 3;
pub const DELTA_TOMBSTONE_KIND_PROJECTION_ROW: u8 = 4;
pub const DELTA_TOMBSTONE_KIND_NONE: u8 = u8::MAX;
pub const DELTA_REF_NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum CoveDeltaSectionKind {
    ParentRefs = 0,
    CatalogPatch = 1,
    DictionaryOverlay = 2,
    TemporalSegmentIndex = 3,
    TemporalSegmentData = 4,
    ContinuationAnchors = 5,
    TouchedObjectSet = 6,
    TombstoneSet = 7,
    PropertyOps = 8,
    EvidencePatch = 9,
    ProjectionPatch = 10,
    CoveragePatch = 11,
    IndexHints = 12,
    LayoutHints = 13,
    TrustContinuation = 14,
    StringTable = 15,
    BranchIdentityTable = 16,
    ScopeTable = 17,
    TemporalRoleSummaryTable = 18,
    TouchedSummaryTable = 19,
    TombstoneSummaryTable = 20,
    StateHashTable = 21,
    Extension = 255,
}

impl CoveDeltaSectionKind {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::ParentRefs),
            1 => Some(Self::CatalogPatch),
            2 => Some(Self::DictionaryOverlay),
            3 => Some(Self::TemporalSegmentIndex),
            4 => Some(Self::TemporalSegmentData),
            5 => Some(Self::ContinuationAnchors),
            6 => Some(Self::TouchedObjectSet),
            7 => Some(Self::TombstoneSet),
            8 => Some(Self::PropertyOps),
            9 => Some(Self::EvidencePatch),
            10 => Some(Self::ProjectionPatch),
            11 => Some(Self::CoveragePatch),
            12 => Some(Self::IndexHints),
            13 => Some(Self::LayoutHints),
            14 => Some(Self::TrustContinuation),
            15 => Some(Self::StringTable),
            16 => Some(Self::BranchIdentityTable),
            17 => Some(Self::ScopeTable),
            18 => Some(Self::TemporalRoleSummaryTable),
            19 => Some(Self::TouchedSummaryTable),
            20 => Some(Self::TombstoneSummaryTable),
            21 => Some(Self::StateHashTable),
            255 => Some(Self::Extension),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaBranchIdentityV1 {
    pub branch_identity_ref: u32,
    pub branch_identity_kind: u8,
    pub flags: u32,
    pub branch_value_ref: u32,
    pub branch_hash128: [u8; 16],
    pub branch_catalog_fingerprint_ref: u32,
    pub checksum: u32,
}

impl DeltaBranchIdentityV1 {
    pub fn serialize(&self) -> [u8; DELTA_BRANCH_IDENTITY_LEN] {
        let mut buf = [0u8; DELTA_BRANCH_IDENTITY_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.branch_identity_ref);
        put_u8(&mut buf, &mut pos, self.branch_identity_kind);
        put_u32(&mut buf, &mut pos, self.flags);
        put_u32(&mut buf, &mut pos, self.branch_value_ref);
        put(&mut buf, &mut pos, &self.branch_hash128);
        put_u32(&mut buf, &mut pos, self.branch_catalog_fingerprint_ref);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_BRANCH_IDENTITY_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_BRANCH_IDENTITY_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_BRANCH_IDENTITY_LEN];
        let mut pos = 0usize;
        let branch_identity_ref = take_u32(bytes, &mut pos)?;
        let branch_identity_kind = take_u8(bytes, &mut pos)?;
        let flags = take_u32(bytes, &mut pos)?;
        let branch_value_ref = take_u32(bytes, &mut pos)?;
        let branch_hash128 = take_array::<16>(bytes, &mut pos)?;
        let branch_catalog_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let identity = Self {
            branch_identity_ref,
            branch_identity_kind,
            flags,
            branch_value_ref,
            branch_hash128,
            branch_catalog_fingerprint_ref,
            checksum,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        match self.branch_identity_kind {
            DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF => {
                if self.branch_value_ref == DELTA_REF_NONE {
                    return Err(CoveError::BadSection(
                        "COVEDELTA canonical branch identity requires branch_value_ref".into(),
                    ));
                }
            }
            DELTA_BRANCH_IDENTITY_KIND_HASH_ONLY => {
                if self.branch_hash128 == [0; 16] {
                    return Err(CoveError::BadSection(
                        "COVEDELTA hash-only branch identity requires non-zero branch_hash128"
                            .into(),
                    ));
                }
            }
            DELTA_BRANCH_IDENTITY_KIND_EXTENSION => {}
            _ => {
                return Err(CoveError::BadSection(
                    "COVEDELTA branch identity has unknown branch_identity_kind".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaContinuationAnchorV1 {
    pub scope_kind: u16,
    pub scope_id: [u8; 16],
    pub object_type_id: u32,
    pub branch_identity_ref: u32,
    pub goid: [u8; 16],
    pub parent_ref: u32,
    pub predecessor_csn: u64,
    pub predecessor_timestamp_us: i64,
    pub predecessor_record_id: [u8; 16],
    pub predecessor_state_hash_ref: u32,
    pub predecessor_trust_hash_ref: u32,
    pub anchor_strength: u8,
    pub flags: u32,
    pub checksum: u32,
}

impl DeltaContinuationAnchorV1 {
    pub fn serialize(&self) -> [u8; DELTA_CONTINUATION_ANCHOR_LEN] {
        let mut buf = [0u8; DELTA_CONTINUATION_ANCHOR_LEN];
        let mut pos = 0usize;
        put_u16(&mut buf, &mut pos, self.scope_kind);
        put(&mut buf, &mut pos, &self.scope_id);
        put_u32(&mut buf, &mut pos, self.object_type_id);
        put_u32(&mut buf, &mut pos, self.branch_identity_ref);
        put(&mut buf, &mut pos, &self.goid);
        put_u32(&mut buf, &mut pos, self.parent_ref);
        put_u64(&mut buf, &mut pos, self.predecessor_csn);
        put_i64(&mut buf, &mut pos, self.predecessor_timestamp_us);
        put(&mut buf, &mut pos, &self.predecessor_record_id);
        put_u32(&mut buf, &mut pos, self.predecessor_state_hash_ref);
        put_u32(&mut buf, &mut pos, self.predecessor_trust_hash_ref);
        put_u8(&mut buf, &mut pos, self.anchor_strength);
        put_u32(&mut buf, &mut pos, self.flags);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_CONTINUATION_ANCHOR_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_CONTINUATION_ANCHOR_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_CONTINUATION_ANCHOR_LEN];
        let mut pos = 0usize;
        let scope_kind = take_u16(bytes, &mut pos)?;
        let scope_id = take_array::<16>(bytes, &mut pos)?;
        let object_type_id = take_u32(bytes, &mut pos)?;
        let branch_identity_ref = take_u32(bytes, &mut pos)?;
        let goid = take_array::<16>(bytes, &mut pos)?;
        let parent_ref = take_u32(bytes, &mut pos)?;
        let predecessor_csn = take_u64(bytes, &mut pos)?;
        let predecessor_timestamp_us = take_i64(bytes, &mut pos)?;
        let predecessor_record_id = take_array::<16>(bytes, &mut pos)?;
        let predecessor_state_hash_ref = take_u32(bytes, &mut pos)?;
        let predecessor_trust_hash_ref = take_u32(bytes, &mut pos)?;
        let anchor_strength = take_u8(bytes, &mut pos)?;
        let flags = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let anchor = Self {
            scope_kind,
            scope_id,
            object_type_id,
            branch_identity_ref,
            goid,
            parent_ref,
            predecessor_csn,
            predecessor_timestamp_us,
            predecessor_record_id,
            predecessor_state_hash_ref,
            predecessor_trust_hash_ref,
            anchor_strength,
            flags,
            checksum,
        };
        anchor.validate()?;
        Ok(anchor)
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.anchor_strength > DELTA_ANCHOR_STRENGTH_KEY_RECORD_STATE_AND_TRUST_HASH {
            return Err(CoveError::BadSection(
                "COVEDELTA continuation anchor has unknown anchor_strength".into(),
            ));
        }
        if self.parent_ref == DELTA_REF_NONE {
            return Err(CoveError::RefInvalid);
        }
        if self.anchor_strength >= DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH
            && self.predecessor_state_hash_ref == DELTA_REF_NONE
        {
            return Err(CoveError::BadSection(
                "COVEDELTA strong continuation anchor requires predecessor_state_hash_ref".into(),
            ));
        }
        if self.anchor_strength == DELTA_ANCHOR_STRENGTH_KEY_RECORD_STATE_AND_TRUST_HASH
            && self.predecessor_trust_hash_ref == DELTA_REF_NONE
        {
            return Err(CoveError::BadSection(
                "COVEDELTA trust-strength continuation anchor requires predecessor_trust_hash_ref"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn validate_for_existing_object_patch(&self) -> Result<(), CoveError> {
        self.validate()?;
        if self.anchor_strength < DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH {
            return Err(CoveError::BadSection(
                "COVEDELTA existing-object patch requires KeyRecordAndStateHash anchor strength"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaStateHashDescriptorV1 {
    pub state_hash_ref: u32,
    pub state_hash_kind: u8,
    pub hash_algorithm: u16,
    pub hash_len: u16,
    pub hash_payload_ref: u32,
    pub flags: u32,
    pub checksum: u32,
}

impl DeltaStateHashDescriptorV1 {
    pub fn serialize(&self) -> [u8; DELTA_STATE_HASH_DESCRIPTOR_LEN] {
        let mut buf = [0u8; DELTA_STATE_HASH_DESCRIPTOR_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.state_hash_ref);
        put_u8(&mut buf, &mut pos, self.state_hash_kind);
        put_u16(&mut buf, &mut pos, self.hash_algorithm);
        put_u16(&mut buf, &mut pos, self.hash_len);
        put_u32(&mut buf, &mut pos, self.hash_payload_ref);
        put_u32(&mut buf, &mut pos, self.flags);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_STATE_HASH_DESCRIPTOR_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_STATE_HASH_DESCRIPTOR_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_STATE_HASH_DESCRIPTOR_LEN];
        let mut pos = 0usize;
        let state_hash_ref = take_u32(bytes, &mut pos)?;
        let state_hash_kind = take_u8(bytes, &mut pos)?;
        let hash_algorithm = take_u16(bytes, &mut pos)?;
        let hash_len = take_u16(bytes, &mut pos)?;
        let hash_payload_ref = take_u32(bytes, &mut pos)?;
        let flags = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let descriptor = Self {
            state_hash_ref,
            state_hash_kind,
            hash_algorithm,
            hash_len,
            hash_payload_ref,
            flags,
            checksum,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        match self.state_hash_kind {
            DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1
            | DELTA_STATE_HASH_KIND_COVE_O_TRUST_HASH
            | DELTA_STATE_HASH_KIND_EXTENSION => {}
            _ => {
                return Err(CoveError::BadSection(
                    "COVEDELTA state hash descriptor has unknown state_hash_kind".into(),
                ));
            }
        }
        let algorithm = DigestAlgorithm::from_u16(self.hash_algorithm)
            .filter(|algorithm| *algorithm != DigestAlgorithm::None)
            .ok_or_else(|| {
                CoveError::BadSection(
                    "COVEDELTA state hash descriptor requires a cryptographic hash algorithm"
                        .into(),
                )
            })?;
        if usize::from(self.hash_len) != covedelta_expected_digest_len(algorithm) {
            return Err(CoveError::BadSection(
                "COVEDELTA state hash descriptor hash_len does not match algorithm".into(),
            ));
        }
        if self.hash_payload_ref == DELTA_REF_NONE {
            return Err(CoveError::BadSection(
                "COVEDELTA state hash descriptor requires hash_payload_ref".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_cove_object_delta_state_hash(&self) -> Result<(), CoveError> {
        self.validate()?;
        if self.state_hash_kind != DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1 {
            return Err(CoveError::BadSection(
                "COVEDELTA continuation anchor state hash must be CoveObjectDeltaStateHashV1"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaScopeDescriptorV1 {
    pub scope_ref: u32,
    pub scope_kind: u16,
    pub flags: u16,
    pub scope_id: [u8; 16],
    pub checksum: u32,
}

impl DeltaScopeDescriptorV1 {
    pub fn serialize(&self) -> Result<[u8; DELTA_SCOPE_DESCRIPTOR_LEN], CoveError> {
        self.validate()?;
        let mut buf = [0u8; DELTA_SCOPE_DESCRIPTOR_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.scope_ref);
        put_u16(&mut buf, &mut pos, self.scope_kind);
        put_u16(&mut buf, &mut pos, self.flags);
        put(&mut buf, &mut pos, &self.scope_id);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_SCOPE_DESCRIPTOR_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        Ok(buf)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_SCOPE_DESCRIPTOR_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_SCOPE_DESCRIPTOR_LEN];
        let mut pos = 0usize;
        let scope_ref = take_u32(bytes, &mut pos)?;
        let scope_kind = take_u16(bytes, &mut pos)?;
        let flags = take_u16(bytes, &mut pos)?;
        let scope_id = take_array::<16>(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let descriptor = Self {
            scope_ref,
            scope_kind,
            flags,
            scope_id,
            checksum,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        if self.scope_ref == DELTA_REF_NONE {
            return Err(CoveError::BadSection(
                "COVEDELTA scope descriptor uses invalid scope_ref".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaSummaryDescriptorV1 {
    pub summary_ref: u32,
    pub summary_kind: u8,
    pub flags: u32,
    pub payload_ref: u32,
    pub item_count: u64,
    pub checksum: u32,
}

impl DeltaSummaryDescriptorV1 {
    pub fn serialize(&self) -> Result<[u8; DELTA_SUMMARY_DESCRIPTOR_LEN], CoveError> {
        self.validate()?;
        let mut buf = [0u8; DELTA_SUMMARY_DESCRIPTOR_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.summary_ref);
        put_u8(&mut buf, &mut pos, self.summary_kind);
        put_u32(&mut buf, &mut pos, self.flags);
        put_u32(&mut buf, &mut pos, self.payload_ref);
        put_u64(&mut buf, &mut pos, self.item_count);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_SUMMARY_DESCRIPTOR_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        Ok(buf)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_SUMMARY_DESCRIPTOR_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_SUMMARY_DESCRIPTOR_LEN];
        let mut pos = 0usize;
        let summary_ref = take_u32(bytes, &mut pos)?;
        let summary_kind = take_u8(bytes, &mut pos)?;
        let flags = take_u32(bytes, &mut pos)?;
        let payload_ref = take_u32(bytes, &mut pos)?;
        let item_count = take_u64(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let descriptor = Self {
            summary_ref,
            summary_kind,
            flags,
            payload_ref,
            item_count,
            checksum,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        if self.summary_ref == DELTA_REF_NONE {
            return Err(CoveError::BadSection(
                "COVEDELTA summary descriptor uses invalid summary_ref".into(),
            ));
        }
        if self.payload_ref == DELTA_REF_NONE {
            return Err(CoveError::BadSection(
                "COVEDELTA summary descriptor requires payload_ref".into(),
            ));
        }
        match self.summary_kind {
            DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET
            | DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET
            | DELTA_SUMMARY_DESCRIPTOR_KIND_CONSERVATIVE_RANGE
            | DELTA_SUMMARY_DESCRIPTOR_KIND_NO_FALSE_NEGATIVE_BLOOM
            | DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP
            | DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE
            | DELTA_SUMMARY_DESCRIPTOR_KIND_EXTENSION => Ok(()),
            _ => Err(CoveError::BadSection(
                "COVEDELTA summary descriptor has unknown summary_kind".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectDeltaStateHashPropertyV1 {
    pub property_id: u32,
    pub logical_type: u16,
    pub collation_id: u32,
    pub value_state: u8,
    pub canonical_value: Vec<u8>,
    pub redaction_commitment: Vec<u8>,
    pub hidden_value_commitment: Option<Vec<u8>>,
}

impl CoveObjectDeltaStateHashPropertyV1 {
    pub fn validate(&self) -> Result<(), CoveError> {
        match self.value_state {
            DELTA_OBJECT_STATE_VALUE_VISIBLE => {
                if !self.redaction_commitment.is_empty() || self.hidden_value_commitment.is_some() {
                    return Err(CoveError::BadSection(
                        "visible state-hash properties cannot carry redaction commitments".into(),
                    ));
                }
            }
            DELTA_OBJECT_STATE_VALUE_NULL
            | DELTA_OBJECT_STATE_VALUE_CLEAR
            | DELTA_OBJECT_STATE_VALUE_TOMBSTONE => {
                if !self.canonical_value.is_empty()
                    || !self.redaction_commitment.is_empty()
                    || self.hidden_value_commitment.is_some()
                {
                    return Err(CoveError::BadSection(
                        "non-visible state-hash properties must not carry value bytes".into(),
                    ));
                }
            }
            DELTA_OBJECT_STATE_VALUE_REDACTED => {
                if !self.canonical_value.is_empty() || self.redaction_commitment.is_empty() {
                    return Err(CoveError::BadSection(
                        "redacted state-hash properties require only a redaction commitment".into(),
                    ));
                }
            }
            _ => {
                return Err(CoveError::BadSection(
                    "state-hash property has unknown value_state".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveObjectDeltaStateHashV1 {
    pub scope_kind: u16,
    pub scope_id: [u8; 16],
    pub canonical_branch_identity: Vec<u8>,
    pub object_type_id: u32,
    pub goid: [u8; 16],
    pub predecessor_record_id: [u8; 16],
    pub predecessor_csn: u64,
    pub predecessor_timestamp_us: i64,
    pub record_kind: RecordKind,
    pub tombstone_state: u8,
    pub properties: Vec<CoveObjectDeltaStateHashPropertyV1>,
}

impl CoveObjectDeltaStateHashV1 {
    pub fn canonical_material(&self) -> Result<Vec<u8>, CoveError> {
        self.validate()?;
        let mut out = Vec::new();
        out.extend_from_slice(b"COVEDELTA_OBJECT_STATE_HASH_V1\0");
        out.extend_from_slice(&self.scope_kind.to_le_bytes());
        out.extend_from_slice(&self.scope_id);
        append_len_prefixed(&mut out, &self.canonical_branch_identity)?;
        out.extend_from_slice(&self.object_type_id.to_le_bytes());
        out.extend_from_slice(&self.goid);
        out.extend_from_slice(&self.predecessor_record_id);
        out.extend_from_slice(&self.predecessor_csn.to_le_bytes());
        out.extend_from_slice(&self.predecessor_timestamp_us.to_le_bytes());
        out.push(self.record_kind as u8);
        out.push(self.tombstone_state);
        out.extend_from_slice(
            &u32::try_from(self.properties.len())
                .map_err(|_| CoveError::ArithOverflow)?
                .to_le_bytes(),
        );
        for property in &self.properties {
            out.extend_from_slice(&property.property_id.to_le_bytes());
            out.extend_from_slice(&property.logical_type.to_le_bytes());
            out.extend_from_slice(&property.collation_id.to_le_bytes());
            out.push(property.value_state);
            append_len_prefixed(&mut out, &property.canonical_value)?;
            append_len_prefixed(&mut out, &property.redaction_commitment)?;
            match &property.hidden_value_commitment {
                Some(commitment) => {
                    out.push(1);
                    append_len_prefixed(&mut out, commitment)?;
                }
                None => out.push(0),
            }
        }
        Ok(out)
    }

    pub fn compute_hash(&self, algorithm: DigestAlgorithm) -> Result<Vec<u8>, CoveError> {
        if algorithm == DigestAlgorithm::None {
            return Err(CoveError::BadSection(
                "COVEDELTA state hash requires a cryptographic hash algorithm".into(),
            ));
        }
        compute_digest(algorithm, &self.canonical_material()?)
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.record_kind == RecordKind::ReservedLegacyMaterializedDelta {
            return Err(CoveError::BadSection(
                "state-hash material cannot use reserved legacy record kind".into(),
            ));
        }
        if self.tombstone_state > DELTA_OBJECT_STATE_TOMBSTONE_DELETED {
            return Err(CoveError::BadSection(
                "state-hash material has unknown tombstone state".into(),
            ));
        }
        let mut previous_property_id = None;
        for property in &self.properties {
            if previous_property_id.is_some_and(|previous| previous >= property.property_id) {
                return Err(CoveError::BadSection(
                    "state-hash properties must be sorted by unique property_id".into(),
                ));
            }
            property.validate()?;
            previous_property_id = Some(property.property_id);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaDictionaryEntryV1 {
    pub local_dictionary_id: u32,
    pub local_code: u32,
    pub logical_type: u16,
    pub collation_id: u16,
    pub entry_kind: u8,
    pub flags: u32,
    pub inline_value_ref: u32,
    pub parent_ref: u32,
    pub parent_dictionary_id: u32,
    pub parent_code: u32,
    pub parent_dictionary_digest_ref: u32,
    pub canonical_hash128: [u8; 16],
    pub checksum: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaInlineValueV1 {
    pub value_ref: u32,
    pub value_tag: u16,
    pub flags: u32,
    pub value: Vec<u8>,
    pub checksum: u32,
}

impl DeltaInlineValueV1 {
    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        self.validate()?;
        let value_len = u32::try_from(self.value.len()).map_err(|_| CoveError::ArithOverflow)?;
        let record_len = DELTA_INLINE_VALUE_HEADER_LEN
            .checked_add(self.value.len())
            .and_then(|len| len.checked_add(4))
            .ok_or(CoveError::ArithOverflow)?;
        let mut out = Vec::with_capacity(record_len);
        out.extend_from_slice(&self.value_ref.to_le_bytes());
        out.extend_from_slice(&self.value_tag.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&value_len.to_le_bytes());
        debug_assert_eq!(out.len(), DELTA_INLINE_VALUE_HEADER_LEN);
        out.extend_from_slice(&self.value);
        out.extend_from_slice(&0u32.to_le_bytes());
        let checksum_pos = out.len() - 4;
        let crc = checksum::crc32c(&out);
        out[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        Ok(out)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let (record, consumed) = Self::parse_with_len(bytes)?;
        if consumed != bytes.len() {
            return Err(CoveError::BadSection(
                "COVEDELTA inline value record has trailing bytes".into(),
            ));
        }
        Ok(record)
    }

    pub fn parse_many(bytes: &[u8]) -> Result<Vec<Self>, CoveError> {
        let mut pos = 0usize;
        let mut records = Vec::new();
        while pos < bytes.len() {
            let (record, consumed) = Self::parse_with_len(&bytes[pos..])?;
            pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
            records.push(record);
        }
        Ok(records)
    }

    fn parse_with_len(bytes: &[u8]) -> Result<(Self, usize), CoveError> {
        if bytes.len() < DELTA_INLINE_VALUE_HEADER_LEN + 4 {
            return Err(CoveError::BufferTooShort);
        }
        let mut pos = 0usize;
        let value_ref = take_u32(bytes, &mut pos)?;
        let value_tag = take_u16(bytes, &mut pos)?;
        let reserved = take_u16(bytes, &mut pos)?;
        if reserved != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        let flags = take_u32(bytes, &mut pos)?;
        let value_len =
            usize::try_from(take_u32(bytes, &mut pos)?).map_err(|_| CoveError::ArithOverflow)?;
        if pos != DELTA_INLINE_VALUE_HEADER_LEN {
            return Err(CoveError::BadSection(
                "COVEDELTA inline value header length mismatch".into(),
            ));
        }
        let record_len = DELTA_INLINE_VALUE_HEADER_LEN
            .checked_add(value_len)
            .and_then(|len| len.checked_add(4))
            .ok_or(CoveError::ArithOverflow)?;
        if record_len > bytes.len() {
            return Err(CoveError::BufferTooShort);
        }
        let record_bytes = &bytes[..record_len];
        let value = take(record_bytes, &mut pos, value_len)?.to_vec();
        let checksum = take_u32(record_bytes, &mut pos)?;
        if pos != record_len {
            return Err(CoveError::BadSection(
                "COVEDELTA inline value parse length mismatch".into(),
            ));
        }
        let mut for_crc = record_bytes.to_vec();
        for_crc[record_len - 4..record_len].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let record = Self {
            value_ref,
            value_tag,
            flags,
            value,
            checksum,
        };
        record.validate()?;
        Ok((record, record_len))
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        let value_tag = ValueTag::from_u16(self.value_tag).ok_or_else(|| {
            CoveError::BadSection("COVEDELTA inline value has unknown value_tag".into())
        })?;
        canonical::validate_canonical_payload(value_tag, &self.value)?;
        Ok(())
    }
}

impl DeltaDictionaryEntryV1 {
    pub fn serialize(&self) -> Result<[u8; DELTA_DICTIONARY_ENTRY_LEN], CoveError> {
        self.validate_structural()?;
        let mut buf = [0u8; DELTA_DICTIONARY_ENTRY_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.local_dictionary_id);
        put_u32(&mut buf, &mut pos, self.local_code);
        put_u16(&mut buf, &mut pos, self.logical_type);
        put_u16(&mut buf, &mut pos, self.collation_id);
        put_u8(&mut buf, &mut pos, self.entry_kind);
        put_u32(&mut buf, &mut pos, self.flags);
        put_u32(&mut buf, &mut pos, self.inline_value_ref);
        put_u32(&mut buf, &mut pos, self.parent_ref);
        put_u32(&mut buf, &mut pos, self.parent_dictionary_id);
        put_u32(&mut buf, &mut pos, self.parent_code);
        put_u32(&mut buf, &mut pos, self.parent_dictionary_digest_ref);
        put(&mut buf, &mut pos, &self.canonical_hash128);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_DICTIONARY_ENTRY_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        Ok(buf)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_DICTIONARY_ENTRY_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_DICTIONARY_ENTRY_LEN];
        let mut pos = 0usize;
        let local_dictionary_id = take_u32(bytes, &mut pos)?;
        let local_code = take_u32(bytes, &mut pos)?;
        let logical_type = take_u16(bytes, &mut pos)?;
        let collation_id = take_u16(bytes, &mut pos)?;
        let entry_kind = take_u8(bytes, &mut pos)?;
        let flags = take_u32(bytes, &mut pos)?;
        let inline_value_ref = take_u32(bytes, &mut pos)?;
        let parent_ref = take_u32(bytes, &mut pos)?;
        let parent_dictionary_id = take_u32(bytes, &mut pos)?;
        let parent_code = take_u32(bytes, &mut pos)?;
        let parent_dictionary_digest_ref = take_u32(bytes, &mut pos)?;
        let canonical_hash128 = take_array::<16>(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let entry = Self {
            local_dictionary_id,
            local_code,
            logical_type,
            collation_id,
            entry_kind,
            flags,
            inline_value_ref,
            parent_ref,
            parent_dictionary_id,
            parent_code,
            parent_dictionary_digest_ref,
            canonical_hash128,
            checksum,
        };
        entry.validate_structural()?;
        Ok(entry)
    }

    pub fn validate_structural(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        if CoveLogicalType::from_u16(self.logical_type).is_none() {
            return Err(CoveError::BadSection(
                "COVEDELTA dictionary overlay entry has unknown logical_type".into(),
            ));
        }
        match self.entry_kind {
            DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE => {
                if self.inline_value_ref == DELTA_REF_NONE {
                    return Err(CoveError::BadSection(
                        "COVEDELTA inline dictionary entry requires inline_value_ref".into(),
                    ));
                }
                if self.parent_ref != DELTA_REF_NONE
                    || self.parent_dictionary_id != 0
                    || self.parent_code != 0
                    || self.parent_dictionary_digest_ref != DELTA_REF_NONE
                {
                    return Err(CoveError::BadSection(
                        "COVEDELTA inline dictionary entry must not carry parent refs".into(),
                    ));
                }
            }
            DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS => {
                if self.inline_value_ref != DELTA_REF_NONE {
                    return Err(CoveError::BadSection(
                        "COVEDELTA parent dictionary alias must not carry inline_value_ref".into(),
                    ));
                }
                if self.parent_ref == DELTA_REF_NONE
                    || self.parent_dictionary_digest_ref == DELTA_REF_NONE
                {
                    return Err(CoveError::BadSection(
                        "COVEDELTA parent dictionary alias requires parent refs and digest".into(),
                    ));
                }
            }
            DELTA_DICTIONARY_ENTRY_KIND_CANONICAL_HASH_HINT => {
                if self.inline_value_ref != DELTA_REF_NONE
                    || self.parent_ref != DELTA_REF_NONE
                    || self.parent_dictionary_id != 0
                    || self.parent_code != 0
                    || self.parent_dictionary_digest_ref != DELTA_REF_NONE
                {
                    return Err(CoveError::BadSection(
                        "COVEDELTA canonical hash hint must not carry value or parent refs".into(),
                    ));
                }
            }
            _ => {
                return Err(CoveError::BadSection(
                    "COVEDELTA dictionary overlay entry has unknown entry_kind".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn validate_for_object_delta(
        &self,
        delta_required_features: u64,
        section_required_features: u64,
        parent_refs: &BTreeSet<u32>,
    ) -> Result<(), CoveError> {
        self.validate_structural()?;
        match self.entry_kind {
            DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE => {
                if delta_required_features & DELTA_FEATURE_INLINE_DICTIONARY == 0
                    || section_required_features & DELTA_FEATURE_INLINE_DICTIONARY == 0
                {
                    return Err(CoveError::BadSection(
                        "COVEDELTA inline dictionary overlay requires inline dictionary feature"
                            .into(),
                    ));
                }
            }
            DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS => {
                if delta_required_features & DELTA_FEATURE_PARENT_DICTIONARY_ALIASES == 0
                    || section_required_features & DELTA_FEATURE_PARENT_DICTIONARY_ALIASES == 0
                {
                    return Err(CoveError::BadSection(
                        "COVEDELTA parent dictionary alias requires parent dictionary alias feature"
                            .into(),
                    ));
                }
                if !parent_refs.contains(&self.parent_ref) {
                    return Err(CoveError::BadSection(
                        "COVEDELTA parent dictionary alias references unknown parent_ref".into(),
                    ));
                }
            }
            DELTA_DICTIONARY_ENTRY_KIND_CANONICAL_HASH_HINT => {
                if section_required_features != 0 {
                    return Err(CoveError::BadSection(
                        "COVEDELTA canonical hash hint must not be required for materialization"
                            .into(),
                    ));
                }
                if self.canonical_hash128 == [0; 16] {
                    return Err(CoveError::BadSection(
                        "COVEDELTA canonical hash hint requires non-zero canonical_hash128".into(),
                    ));
                }
            }
            _ => unreachable!("structural validation rejects unknown dictionary entry kinds"),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaSidecarHintV1 {
    pub hint_ref: u32,
    pub hint_kind: u16,
    pub flags: u16,
    pub parent_ref: u32,
    pub target_section_id: u32,
    pub scope_ref: u32,
    pub object_type_id: u32,
    pub chain_digest_ref: u32,
    pub checksum: u32,
}

impl DeltaSidecarHintV1 {
    pub fn serialize(&self) -> [u8; DELTA_SIDECAR_HINT_LEN] {
        let mut buf = [0u8; DELTA_SIDECAR_HINT_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.hint_ref);
        put_u16(&mut buf, &mut pos, self.hint_kind);
        put_u16(&mut buf, &mut pos, self.flags);
        put_u32(&mut buf, &mut pos, self.parent_ref);
        put_u32(&mut buf, &mut pos, self.target_section_id);
        put_u32(&mut buf, &mut pos, self.scope_ref);
        put_u32(&mut buf, &mut pos, self.object_type_id);
        put_u32(&mut buf, &mut pos, self.chain_digest_ref);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_SIDECAR_HINT_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_SIDECAR_HINT_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_SIDECAR_HINT_LEN];
        let mut pos = 0usize;
        let hint_ref = take_u32(bytes, &mut pos)?;
        let hint_kind = take_u16(bytes, &mut pos)?;
        let flags = take_u16(bytes, &mut pos)?;
        let parent_ref = take_u32(bytes, &mut pos)?;
        let target_section_id = take_u32(bytes, &mut pos)?;
        let scope_ref = take_u32(bytes, &mut pos)?;
        let object_type_id = take_u32(bytes, &mut pos)?;
        let chain_digest_ref = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let hint = Self {
            hint_ref,
            hint_kind,
            flags,
            parent_ref,
            target_section_id,
            scope_ref,
            object_type_id,
            chain_digest_ref,
            checksum,
        };
        hint.validate_structural()?;
        Ok(hint)
    }

    pub fn validate_structural(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        if self.hint_ref == DELTA_REF_NONE {
            return Err(CoveError::BadSection(
                "COVEDELTA sidecar hint uses invalid hint_ref".into(),
            ));
        }
        if self.parent_ref == DELTA_REF_NONE {
            return Err(CoveError::BadSection(
                "COVEDELTA sidecar hint requires parent_ref".into(),
            ));
        }
        match self.hint_kind {
            DELTA_SIDECAR_HINT_KIND_COVI_INDEX
            | DELTA_SIDECAR_HINT_KIND_COVX_INDEX
            | DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH
            | DELTA_SIDECAR_HINT_KIND_LAYOUT_HINTS
            | DELTA_SIDECAR_HINT_KIND_EXTENSION => Ok(()),
            _ => Err(CoveError::BadSection(
                "COVEDELTA sidecar hint has unknown hint_kind".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaSparsePatchPropertyOpV1 {
    pub property_id: u32,
    pub property_op: u8,
    pub tombstone_kind: u8,
    pub value_ref: u32,
    pub redaction_ref: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaSparsePatchPropertyStateV1 {
    ValueRef(u32),
    Null,
    Clear,
    Tombstone(u8),
    Redacted { redaction_ref: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeltaSparseObjectKeyV1 {
    pub scope_kind: u16,
    pub scope_id: [u8; 16],
    pub branch_identity_ref: u32,
    pub object_type_id: u32,
    pub goid: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaSparseObjectTombstoneStatusV1 {
    Live,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaSparseObjectStateV1 {
    pub key: DeltaSparseObjectKeyV1,
    pub latest_record_id: [u8; 16],
    pub latest_timestamp_us: i64,
    pub latest_csn: u64,
    pub latest_record_kind: RecordKind,
    pub tombstone_status: DeltaSparseObjectTombstoneStatusV1,
    pub properties: BTreeMap<u32, DeltaSparsePatchPropertyStateV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaObjectPointLookupV1 {
    pub scope_kind: u16,
    pub scope_id: [u8; 16],
    pub object_type_id: u32,
    pub branch_identity_ref: u32,
    pub goid: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaExactObjectSetMembershipV1 {
    Present,
    Absent,
    Unavailable,
}

impl DeltaSparsePatchPropertyOpV1 {
    pub fn validate(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        match self.property_op {
            DELTA_PROPERTY_OP_SET_VALUE => {
                if self.tombstone_kind != DELTA_TOMBSTONE_KIND_NONE {
                    return Err(CoveError::BadSection(
                        "SetValue sparse patch operation cannot carry a tombstone kind".into(),
                    ));
                }
                if self.value_ref == DELTA_REF_NONE {
                    return Err(CoveError::BadSection(
                        "SetValue sparse patch operation requires value_ref".into(),
                    ));
                }
                if self.redaction_ref != DELTA_REF_NONE {
                    return Err(CoveError::BadSection(
                        "SetValue sparse patch operation cannot carry redaction_ref".into(),
                    ));
                }
            }
            DELTA_PROPERTY_OP_SET_NULL | DELTA_PROPERTY_OP_CLEAR => {
                if self.tombstone_kind != DELTA_TOMBSTONE_KIND_NONE
                    || self.value_ref != DELTA_REF_NONE
                    || self.redaction_ref != DELTA_REF_NONE
                {
                    return Err(CoveError::BadSection(
                        "null/clear sparse patch operations must not carry payload refs".into(),
                    ));
                }
            }
            DELTA_PROPERTY_OP_TOMBSTONE => {
                if !matches!(
                    self.tombstone_kind,
                    DELTA_TOMBSTONE_KIND_OBJECT
                        | DELTA_TOMBSTONE_KIND_PROPERTY
                        | DELTA_TOMBSTONE_KIND_ASSOCIATION
                        | DELTA_TOMBSTONE_KIND_EVIDENCE
                        | DELTA_TOMBSTONE_KIND_PROJECTION_ROW
                ) {
                    return Err(CoveError::BadSection(
                        "Tombstone sparse patch operation has unknown tombstone_kind".into(),
                    ));
                }
                if self.value_ref != DELTA_REF_NONE || self.redaction_ref != DELTA_REF_NONE {
                    return Err(CoveError::BadSection(
                        "Tombstone sparse patch operation must not carry payload refs".into(),
                    ));
                }
            }
            DELTA_PROPERTY_OP_REDACT => {
                if self.tombstone_kind != DELTA_TOMBSTONE_KIND_NONE {
                    return Err(CoveError::BadSection(
                        "Redact sparse patch operation cannot carry a tombstone kind".into(),
                    ));
                }
                if self.value_ref != DELTA_REF_NONE || self.redaction_ref == DELTA_REF_NONE {
                    return Err(CoveError::BadSection(
                        "Redact sparse patch operation requires only redaction_ref".into(),
                    ));
                }
            }
            _ => {
                return Err(CoveError::BadSection(
                    "sparse patch operation has unknown property_op".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn property_state(&self) -> Result<DeltaSparsePatchPropertyStateV1, CoveError> {
        self.validate()?;
        match self.property_op {
            DELTA_PROPERTY_OP_SET_VALUE => {
                Ok(DeltaSparsePatchPropertyStateV1::ValueRef(self.value_ref))
            }
            DELTA_PROPERTY_OP_SET_NULL => Ok(DeltaSparsePatchPropertyStateV1::Null),
            DELTA_PROPERTY_OP_CLEAR => Ok(DeltaSparsePatchPropertyStateV1::Clear),
            DELTA_PROPERTY_OP_TOMBSTONE => Ok(DeltaSparsePatchPropertyStateV1::Tombstone(
                self.tombstone_kind,
            )),
            DELTA_PROPERTY_OP_REDACT => Ok(DeltaSparsePatchPropertyStateV1::Redacted {
                redaction_ref: self.redaction_ref,
            }),
            _ => Err(CoveError::BadSection(
                "sparse patch operation has unknown property_op".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaSparsePatchRecordV1 {
    pub scope_kind: u16,
    pub scope_id: [u8; 16],
    pub branch_identity_ref: u32,
    pub object_type_id: u32,
    pub goid: [u8; 16],
    pub record_id: [u8; 16],
    pub timestamp_us: i64,
    pub csn: u64,
    pub record_kind: RecordKind,
    pub flags: u32,
    pub changed_properties: Vec<DeltaSparsePatchPropertyOpV1>,
    pub checksum: u32,
}

impl DeltaSparsePatchRecordV1 {
    pub fn object_key(&self) -> DeltaSparseObjectKeyV1 {
        DeltaSparseObjectKeyV1 {
            scope_kind: self.scope_kind,
            scope_id: self.scope_id,
            branch_identity_ref: self.branch_identity_ref,
            object_type_id: self.object_type_id,
            goid: self.goid,
        }
    }

    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        self.validate()?;
        let changed_property_count =
            u32::try_from(self.changed_properties.len()).map_err(|_| CoveError::ArithOverflow)?;
        let record_len = DELTA_SPARSE_PATCH_RECORD_HEADER_LEN
            .checked_add(
                self.changed_properties
                    .len()
                    .checked_mul(DELTA_SPARSE_PATCH_PROPERTY_OP_LEN)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .and_then(|len| len.checked_add(4))
            .ok_or(CoveError::ArithOverflow)?;
        let record_len_u32 = u32::try_from(record_len).map_err(|_| CoveError::ArithOverflow)?;
        let mut out = Vec::with_capacity(record_len);
        out.extend_from_slice(&self.scope_kind.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&self.scope_id);
        out.extend_from_slice(&self.branch_identity_ref.to_le_bytes());
        out.extend_from_slice(&self.object_type_id.to_le_bytes());
        out.extend_from_slice(&self.goid);
        out.extend_from_slice(&self.record_id);
        out.extend_from_slice(&self.timestamp_us.to_le_bytes());
        out.extend_from_slice(&self.csn.to_le_bytes());
        out.push(self.record_kind as u8);
        out.extend_from_slice(&[0; 3]);
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&changed_property_count.to_le_bytes());
        out.extend_from_slice(&record_len_u32.to_le_bytes());
        debug_assert_eq!(out.len(), DELTA_SPARSE_PATCH_RECORD_HEADER_LEN);
        for property in &self.changed_properties {
            out.extend_from_slice(&property.property_id.to_le_bytes());
            out.push(property.property_op);
            out.push(property.tombstone_kind);
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&property.value_ref.to_le_bytes());
            out.extend_from_slice(&property.redaction_ref.to_le_bytes());
            out.extend_from_slice(&property.flags.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        let checksum_pos = out.len() - 4;
        let crc = checksum::crc32c(&out);
        out[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        Ok(out)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let (record, consumed) = Self::parse_with_len(bytes)?;
        if consumed != bytes.len() {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse patch record has trailing bytes".into(),
            ));
        }
        Ok(record)
    }

    pub fn parse_many(bytes: &[u8]) -> Result<Vec<Self>, CoveError> {
        let mut pos = 0usize;
        let mut records = Vec::new();
        while pos < bytes.len() {
            let (record, consumed) = Self::parse_with_len(&bytes[pos..])?;
            pos = pos.checked_add(consumed).ok_or(CoveError::ArithOverflow)?;
            records.push(record);
        }
        Ok(records)
    }

    fn parse_with_len(bytes: &[u8]) -> Result<(Self, usize), CoveError> {
        if bytes.len() < DELTA_SPARSE_PATCH_RECORD_HEADER_LEN + 4 {
            return Err(CoveError::BufferTooShort);
        }
        let mut pos = 0usize;
        let scope_kind = take_u16(bytes, &mut pos)?;
        let reserved0 = take_u16(bytes, &mut pos)?;
        if reserved0 != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        let scope_id = take_array::<16>(bytes, &mut pos)?;
        let branch_identity_ref = take_u32(bytes, &mut pos)?;
        let object_type_id = take_u32(bytes, &mut pos)?;
        let goid = take_array::<16>(bytes, &mut pos)?;
        let record_id = take_array::<16>(bytes, &mut pos)?;
        let timestamp_us = take_i64(bytes, &mut pos)?;
        let csn = take_u64(bytes, &mut pos)?;
        let record_kind_raw = take_u8(bytes, &mut pos)?;
        let reserved1 = take(bytes, &mut pos, 3)?;
        if reserved1 != [0, 0, 0] {
            return Err(CoveError::ReservedNotZero);
        }
        let flags = take_u32(bytes, &mut pos)?;
        let changed_property_count = take_u32(bytes, &mut pos)?;
        let record_len =
            usize::try_from(take_u32(bytes, &mut pos)?).map_err(|_| CoveError::ArithOverflow)?;
        if pos != DELTA_SPARSE_PATCH_RECORD_HEADER_LEN {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse patch record header length mismatch".into(),
            ));
        }
        let expected_len = DELTA_SPARSE_PATCH_RECORD_HEADER_LEN
            .checked_add(
                usize::try_from(changed_property_count)
                    .map_err(|_| CoveError::ArithOverflow)?
                    .checked_mul(DELTA_SPARSE_PATCH_PROPERTY_OP_LEN)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .and_then(|len| len.checked_add(4))
            .ok_or(CoveError::ArithOverflow)?;
        if record_len != expected_len {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse patch record_len does not match property count".into(),
            ));
        }
        if record_len > bytes.len() {
            return Err(CoveError::BufferTooShort);
        }
        let record_bytes = &bytes[..record_len];
        let checksum = wire::read_u32_le_checked(record_bytes, record_len - 4)?;
        let mut for_crc = record_bytes.to_vec();
        for_crc[record_len - 4..record_len].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let record_kind = RecordKind::from_u8(record_kind_raw).ok_or_else(|| {
            CoveError::BadSection(format!(
                "COVEDELTA sparse patch record has unknown record_kind {record_kind_raw}"
            ))
        })?;
        let mut changed_properties = Vec::with_capacity(changed_property_count as usize);
        for _ in 0..changed_property_count {
            let property_id = take_u32(record_bytes, &mut pos)?;
            let property_op = take_u8(record_bytes, &mut pos)?;
            let tombstone_kind = take_u8(record_bytes, &mut pos)?;
            let reserved = take_u16(record_bytes, &mut pos)?;
            if reserved != 0 {
                return Err(CoveError::ReservedNotZero);
            }
            let value_ref = take_u32(record_bytes, &mut pos)?;
            let redaction_ref = take_u32(record_bytes, &mut pos)?;
            let flags = take_u32(record_bytes, &mut pos)?;
            changed_properties.push(DeltaSparsePatchPropertyOpV1 {
                property_id,
                property_op,
                tombstone_kind,
                value_ref,
                redaction_ref,
                flags,
            });
        }
        pos = pos.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        if pos != record_len {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse patch record parse length mismatch".into(),
            ));
        }
        let record = Self {
            scope_kind,
            scope_id,
            branch_identity_ref,
            object_type_id,
            goid,
            record_id,
            timestamp_us,
            csn,
            record_kind,
            flags,
            changed_properties,
            checksum,
        };
        record.validate()?;
        Ok((record, record_len))
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.flags != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        if self.record_kind == RecordKind::ReservedLegacyMaterializedDelta {
            return Err(CoveError::BadSection(
                "sparse patch record cannot use reserved legacy record kind".into(),
            ));
        }
        if self.changed_properties.is_empty() {
            return Err(CoveError::BadSection(
                "sparse patch record requires changed properties".into(),
            ));
        }
        let mut previous_property_id = None;
        for property in &self.changed_properties {
            if previous_property_id.is_some_and(|previous| previous >= property.property_id) {
                return Err(CoveError::BadSection(
                    "sparse patch properties must be sorted by unique property_id".into(),
                ));
            }
            property.validate()?;
            previous_property_id = Some(property.property_id);
        }
        Ok(())
    }

    pub fn apply_to_property_state(
        &self,
        state: &mut BTreeMap<u32, DeltaSparsePatchPropertyStateV1>,
    ) -> Result<(), CoveError> {
        self.validate()?;
        for property in &self.changed_properties {
            state.insert(property.property_id, property.property_state()?);
        }
        Ok(())
    }
}

impl CoveDeltaObjectValidation {
    pub fn reconstruct_sparse_patch_state_table(
        &self,
    ) -> Result<BTreeMap<DeltaSparseObjectKeyV1, DeltaSparseObjectStateV1>, CoveError> {
        reconstruct_sparse_patch_state_table(&self.sparse_patch_records)
    }

    pub fn exact_touched_membership(
        &self,
        point: DeltaObjectPointLookupV1,
    ) -> DeltaExactObjectSetMembershipV1 {
        exact_range_membership(
            self.has_touched_object_set_section,
            &self.touched_object_ranges,
            point,
        )
    }

    pub fn exact_tombstone_membership(
        &self,
        point: DeltaObjectPointLookupV1,
    ) -> DeltaExactObjectSetMembershipV1 {
        if !self.has_tombstone_object_set_section {
            return DeltaExactObjectSetMembershipV1::Unavailable;
        }
        if point.scope_kind != self.scope_kind || point.scope_id != self.scope_id {
            return DeltaExactObjectSetMembershipV1::Absent;
        }
        let temporal_tombstone = self.temporal_segments.iter().any(|segment| {
            segment.header.object_type_id == point.object_type_id
                && segment.rows.iter().any(|row| {
                    row.record_kind == RecordKind::Tombstone
                        && u64::from(point.branch_identity_ref) == row.branch_key
                        && row.goid == point.goid
                })
        });
        let sparse_tombstone = self.sparse_patch_records.iter().any(|record| {
            sparse_patch_record_matches_point(record, point)
                && sparse_record_tombstone_status(record)
                    == DeltaSparseObjectTombstoneStatusV1::Tombstoned
        });
        if temporal_tombstone || sparse_tombstone {
            DeltaExactObjectSetMembershipV1::Present
        } else {
            DeltaExactObjectSetMembershipV1::Absent
        }
    }

    pub fn can_skip_delta_for_point_lookup(&self, point: DeltaObjectPointLookupV1) -> bool {
        self.exact_touched_membership(point) == DeltaExactObjectSetMembershipV1::Absent
    }

    pub fn can_skip_delta_for_projection_properties(
        &self,
        point: DeltaObjectPointLookupV1,
        requested_property_ids: &[u32],
    ) -> bool {
        if self.can_skip_delta_for_point_lookup(point) {
            return true;
        }
        if requested_property_ids.is_empty()
            || self.should_suppress_parent_latest_state_for_tombstone(point)
        {
            return false;
        }

        let mut found_sparse_record = false;
        for record in &self.sparse_patch_records {
            if !sparse_patch_record_matches_point(record, point) {
                continue;
            }
            found_sparse_record = true;
            if record.record_kind != RecordKind::Delta {
                return false;
            }
            for property in &record.changed_properties {
                if requested_property_ids.contains(&property.property_id)
                    || property_op_forces_projection_read(property)
                {
                    return false;
                }
            }
        }

        found_sparse_record
    }

    pub fn should_suppress_parent_latest_state_for_tombstone(
        &self,
        point: DeltaObjectPointLookupV1,
    ) -> bool {
        self.exact_tombstone_membership(point) == DeltaExactObjectSetMembershipV1::Present
    }
}

pub fn reconstruct_sparse_patch_state_table(
    sparse_patch_records: &[DeltaSparsePatchRecordV1],
) -> Result<BTreeMap<DeltaSparseObjectKeyV1, DeltaSparseObjectStateV1>, CoveError> {
    let mut records = sparse_patch_records.iter().collect::<Vec<_>>();
    records.sort_by_key(|record| (record.csn, record.timestamp_us, record.record_id));

    let mut seen_record_ids = BTreeSet::new();
    let mut seen_object_csns = BTreeSet::new();
    let mut states = BTreeMap::new();
    for record in records {
        record.validate()?;
        let key = record.object_key();
        if !seen_record_ids.insert((key, record.record_id)) {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse state reconstruction found duplicate record_id for object".into(),
            ));
        }
        if !seen_object_csns.insert((key, record.csn)) {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse state reconstruction found duplicate CSN for object".into(),
            ));
        }

        let state = states
            .entry(key)
            .or_insert_with(|| DeltaSparseObjectStateV1 {
                key,
                latest_record_id: record.record_id,
                latest_timestamp_us: record.timestamp_us,
                latest_csn: record.csn,
                latest_record_kind: record.record_kind,
                tombstone_status: DeltaSparseObjectTombstoneStatusV1::Live,
                properties: BTreeMap::new(),
            });
        state.latest_record_id = record.record_id;
        state.latest_timestamp_us = record.timestamp_us;
        state.latest_csn = record.csn;
        state.latest_record_kind = record.record_kind;
        state.tombstone_status = sparse_record_tombstone_status(record);
        record.apply_to_property_state(&mut state.properties)?;
    }
    Ok(states)
}

fn sparse_record_tombstone_status(
    record: &DeltaSparsePatchRecordV1,
) -> DeltaSparseObjectTombstoneStatusV1 {
    let has_object_tombstone_op = record.changed_properties.iter().any(|property| {
        property.property_op == DELTA_PROPERTY_OP_TOMBSTONE
            && property.tombstone_kind == DELTA_TOMBSTONE_KIND_OBJECT
    });
    if record.record_kind == RecordKind::Tombstone || has_object_tombstone_op {
        DeltaSparseObjectTombstoneStatusV1::Tombstoned
    } else {
        DeltaSparseObjectTombstoneStatusV1::Live
    }
}

fn sparse_patch_record_matches_point(
    record: &DeltaSparsePatchRecordV1,
    point: DeltaObjectPointLookupV1,
) -> bool {
    record.scope_kind == point.scope_kind
        && record.scope_id == point.scope_id
        && record.branch_identity_ref == point.branch_identity_ref
        && record.object_type_id == point.object_type_id
        && record.goid == point.goid
}

fn property_op_forces_projection_read(property: &DeltaSparsePatchPropertyOpV1) -> bool {
    property.property_op == DELTA_PROPERTY_OP_TOMBSTONE
        && matches!(
            property.tombstone_kind,
            DELTA_TOMBSTONE_KIND_OBJECT | DELTA_TOMBSTONE_KIND_PROJECTION_ROW
        )
}

fn exact_range_membership(
    section_available: bool,
    ranges: &[DeltaTouchedObjectRangeV1],
    point: DeltaObjectPointLookupV1,
) -> DeltaExactObjectSetMembershipV1 {
    if !section_available {
        return DeltaExactObjectSetMembershipV1::Unavailable;
    }
    if ranges.iter().any(|range| range.covers_object_point(point)) {
        DeltaExactObjectSetMembershipV1::Present
    } else {
        DeltaExactObjectSetMembershipV1::Absent
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaTouchedObjectRangeV1 {
    pub scope_kind: u16,
    pub scope_id: [u8; 16],
    pub object_type_id: u32,
    pub branch_identity_ref: u32,
    pub min_goid: [u8; 16],
    pub max_goid: [u8; 16],
    pub touched_count: u32,
    pub property_bitmap_ref: u32,
    pub object_set_ref: u32,
    pub checksum: u32,
}

impl DeltaTouchedObjectRangeV1 {
    pub fn covers_object_point(&self, point: DeltaObjectPointLookupV1) -> bool {
        self.scope_kind == point.scope_kind
            && self.scope_id == point.scope_id
            && self.object_type_id == point.object_type_id
            && self.branch_identity_ref == point.branch_identity_ref
            && self.min_goid <= point.goid
            && point.goid <= self.max_goid
    }

    pub fn serialize(&self) -> [u8; DELTA_TOUCHED_OBJECT_RANGE_LEN] {
        let mut buf = [0u8; DELTA_TOUCHED_OBJECT_RANGE_LEN];
        let mut pos = 0usize;
        put_u16(&mut buf, &mut pos, self.scope_kind);
        put(&mut buf, &mut pos, &self.scope_id);
        put_u32(&mut buf, &mut pos, self.object_type_id);
        put_u32(&mut buf, &mut pos, self.branch_identity_ref);
        put(&mut buf, &mut pos, &self.min_goid);
        put(&mut buf, &mut pos, &self.max_goid);
        put_u32(&mut buf, &mut pos, self.touched_count);
        put_u32(&mut buf, &mut pos, self.property_bitmap_ref);
        put_u32(&mut buf, &mut pos, self.object_set_ref);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, DELTA_TOUCHED_OBJECT_RANGE_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < DELTA_TOUCHED_OBJECT_RANGE_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..DELTA_TOUCHED_OBJECT_RANGE_LEN];
        let mut pos = 0usize;
        let scope_kind = take_u16(bytes, &mut pos)?;
        let scope_id = take_array::<16>(bytes, &mut pos)?;
        let object_type_id = take_u32(bytes, &mut pos)?;
        let branch_identity_ref = take_u32(bytes, &mut pos)?;
        let min_goid = take_array::<16>(bytes, &mut pos)?;
        let max_goid = take_array::<16>(bytes, &mut pos)?;
        let touched_count = take_u32(bytes, &mut pos)?;
        let property_bitmap_ref = take_u32(bytes, &mut pos)?;
        let object_set_ref = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        let range = Self {
            scope_kind,
            scope_id,
            object_type_id,
            branch_identity_ref,
            min_goid,
            max_goid,
            touched_count,
            property_bitmap_ref,
            object_set_ref,
            checksum,
        };
        range.validate()?;
        Ok(range)
    }

    pub fn validate(&self) -> Result<(), CoveError> {
        if self.min_goid > self.max_goid {
            return Err(CoveError::BadSection(
                "COVEDELTA touched object range min_goid exceeds max_goid".into(),
            ));
        }
        if self.touched_count == 0 {
            return Err(CoveError::BadSection(
                "COVEDELTA touched object range must have non-zero touched_count".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveDeltaHeaderV1 {
    pub magic: [u8; 4],
    pub version_major: u16,
    pub version_minor: u16,
    pub header_len: u16,
    pub flags: u32,
    pub required_delta_features: u64,
    pub optional_delta_features: u64,
    pub delta_artifact_id: [u8; 16],
    pub dataset_id: [u8; 16],
    pub snapshot_id: [u8; 16],
    pub parent_snapshot_id: [u8; 16],
    pub chain_ordinal: u32,
    pub chain_depth: u32,
    pub parent_ref_count: u32,
    pub section_count: u32,
    pub csn_min: u64,
    pub csn_max: u64,
    pub commit_time_range_start_us: i64,
    pub commit_time_range_end_us: i64,
    pub scope_kind: u16,
    pub reserved0: u16,
    pub scope_id: [u8; 16],
    pub object_catalog_fingerprint_ref: u32,
    pub schema_fingerprint_ref: u32,
    pub semantic_map_fingerprint_ref: u32,
    pub projection_fingerprint_ref: u32,
    pub section_directory_offset: u64,
    pub section_directory_length: u64,
    pub parent_refs_offset: u64,
    pub parent_refs_length: u64,
    pub created_at_us: i64,
    pub source_publish_range_start_us: i64,
    pub source_publish_range_end_us: i64,
    pub checksum: u32,
}

impl CoveDeltaHeaderV1 {
    pub fn new(
        delta_artifact_id: [u8; 16],
        dataset_id: [u8; 16],
        snapshot_id: [u8; 16],
        parent_snapshot_id: [u8; 16],
    ) -> Self {
        Self {
            magic: MAGIC_COVEDELTA,
            version_major: COVEDELTA_VERSION_MAJOR_V1,
            version_minor: COVEDELTA_VERSION_MINOR_V1,
            header_len: COVEDELTA_HEADER_LEN,
            flags: 0,
            required_delta_features: 0,
            optional_delta_features: 0,
            delta_artifact_id,
            dataset_id,
            snapshot_id,
            parent_snapshot_id,
            chain_ordinal: 0,
            chain_depth: 1,
            parent_ref_count: 0,
            section_count: 0,
            csn_min: 0,
            csn_max: 0,
            commit_time_range_start_us: 0,
            commit_time_range_end_us: 0,
            scope_kind: 0,
            reserved0: 0,
            scope_id: [0; 16],
            object_catalog_fingerprint_ref: 0,
            schema_fingerprint_ref: 0,
            semantic_map_fingerprint_ref: 0,
            projection_fingerprint_ref: 0,
            section_directory_offset: 0,
            section_directory_length: 0,
            parent_refs_offset: 0,
            parent_refs_length: 0,
            created_at_us: 0,
            source_publish_range_start_us: 0,
            source_publish_range_end_us: 0,
            checksum: 0,
        }
    }

    pub fn serialize(&self) -> [u8; COVEDELTA_HEADER_LEN as usize] {
        let mut buf = [0u8; COVEDELTA_HEADER_LEN as usize];
        let mut pos = 0usize;
        put(&mut buf, &mut pos, &self.magic);
        put_u16(&mut buf, &mut pos, self.version_major);
        put_u16(&mut buf, &mut pos, self.version_minor);
        put_u16(&mut buf, &mut pos, self.header_len);
        put_u32(&mut buf, &mut pos, self.flags);
        put_u64(&mut buf, &mut pos, self.required_delta_features);
        put_u64(&mut buf, &mut pos, self.optional_delta_features);
        put(&mut buf, &mut pos, &self.delta_artifact_id);
        put(&mut buf, &mut pos, &self.dataset_id);
        put(&mut buf, &mut pos, &self.snapshot_id);
        put(&mut buf, &mut pos, &self.parent_snapshot_id);
        put_u32(&mut buf, &mut pos, self.chain_ordinal);
        put_u32(&mut buf, &mut pos, self.chain_depth);
        put_u32(&mut buf, &mut pos, self.parent_ref_count);
        put_u32(&mut buf, &mut pos, self.section_count);
        put_u64(&mut buf, &mut pos, self.csn_min);
        put_u64(&mut buf, &mut pos, self.csn_max);
        put_i64(&mut buf, &mut pos, self.commit_time_range_start_us);
        put_i64(&mut buf, &mut pos, self.commit_time_range_end_us);
        put_u16(&mut buf, &mut pos, self.scope_kind);
        put_u16(&mut buf, &mut pos, self.reserved0);
        put(&mut buf, &mut pos, &self.scope_id);
        put_u32(&mut buf, &mut pos, self.object_catalog_fingerprint_ref);
        put_u32(&mut buf, &mut pos, self.schema_fingerprint_ref);
        put_u32(&mut buf, &mut pos, self.semantic_map_fingerprint_ref);
        put_u32(&mut buf, &mut pos, self.projection_fingerprint_ref);
        put_u64(&mut buf, &mut pos, self.section_directory_offset);
        put_u64(&mut buf, &mut pos, self.section_directory_length);
        put_u64(&mut buf, &mut pos, self.parent_refs_offset);
        put_u64(&mut buf, &mut pos, self.parent_refs_length);
        put_i64(&mut buf, &mut pos, self.created_at_us);
        put_i64(&mut buf, &mut pos, self.source_publish_range_start_us);
        put_i64(&mut buf, &mut pos, self.source_publish_range_end_us);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, COVEDELTA_HEADER_LEN as usize);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEDELTA_HEADER_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEDELTA_HEADER_LEN as usize];
        let mut pos = 0usize;
        let magic = take_array::<4>(bytes, &mut pos)?;
        if magic != MAGIC_COVEDELTA {
            return Err(CoveError::BadMagic);
        }
        let version_major = take_u16(bytes, &mut pos)?;
        let version_minor = take_u16(bytes, &mut pos)?;
        if version_major != COVEDELTA_VERSION_MAJOR_V1
            || version_minor != COVEDELTA_VERSION_MINOR_V1
        {
            return Err(CoveError::BadVersion);
        }
        let header_len = take_u16(bytes, &mut pos)?;
        if header_len != COVEDELTA_HEADER_LEN {
            return Err(CoveError::BadSection(format!(
                "COVEDELTA header_len must be {COVEDELTA_HEADER_LEN}, got {header_len}"
            )));
        }
        let flags = take_u32(bytes, &mut pos)?;
        let required_delta_features = take_u64(bytes, &mut pos)?;
        let optional_delta_features = take_u64(bytes, &mut pos)?;
        let delta_artifact_id = take_array::<16>(bytes, &mut pos)?;
        let dataset_id = take_array::<16>(bytes, &mut pos)?;
        let snapshot_id = take_array::<16>(bytes, &mut pos)?;
        let parent_snapshot_id = take_array::<16>(bytes, &mut pos)?;
        let chain_ordinal = take_u32(bytes, &mut pos)?;
        let chain_depth = take_u32(bytes, &mut pos)?;
        let parent_ref_count = take_u32(bytes, &mut pos)?;
        let section_count = take_u32(bytes, &mut pos)?;
        let csn_min = take_u64(bytes, &mut pos)?;
        let csn_max = take_u64(bytes, &mut pos)?;
        let commit_time_range_start_us = take_i64(bytes, &mut pos)?;
        let commit_time_range_end_us = take_i64(bytes, &mut pos)?;
        let scope_kind = take_u16(bytes, &mut pos)?;
        let reserved0 = take_u16(bytes, &mut pos)?;
        if reserved0 != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        let scope_id = take_array::<16>(bytes, &mut pos)?;
        let object_catalog_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let schema_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let semantic_map_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let projection_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let section_directory_offset = take_u64(bytes, &mut pos)?;
        let section_directory_length = take_u64(bytes, &mut pos)?;
        let parent_refs_offset = take_u64(bytes, &mut pos)?;
        let parent_refs_length = take_u64(bytes, &mut pos)?;
        let created_at_us = take_i64(bytes, &mut pos)?;
        let source_publish_range_start_us = take_i64(bytes, &mut pos)?;
        let source_publish_range_end_us = take_i64(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        if flags & DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT == 0
            && (source_publish_range_start_us != 0 || source_publish_range_end_us != 0)
        {
            return Err(CoveError::BadSection(
                "source publish range fields require DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT"
                    .into(),
            ));
        }
        Ok(Self {
            magic,
            version_major,
            version_minor,
            header_len,
            flags,
            required_delta_features,
            optional_delta_features,
            delta_artifact_id,
            dataset_id,
            snapshot_id,
            parent_snapshot_id,
            chain_ordinal,
            chain_depth,
            parent_ref_count,
            section_count,
            csn_min,
            csn_max,
            commit_time_range_start_us,
            commit_time_range_end_us,
            scope_kind,
            reserved0,
            scope_id,
            object_catalog_fingerprint_ref,
            schema_fingerprint_ref,
            semantic_map_fingerprint_ref,
            projection_fingerprint_ref,
            section_directory_offset,
            section_directory_length,
            parent_refs_offset,
            parent_refs_length,
            created_at_us,
            source_publish_range_start_us,
            source_publish_range_end_us,
            checksum,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaParentRefV1 {
    pub parent_ref: u32,
    pub parent_kind: u8,
    pub flags: u32,
    pub artifact_id: [u8; 16],
    pub snapshot_id: [u8; 16],
    pub file_len: u64,
    pub footer_crc32c: u32,
    pub digest_algorithm: u16,
    pub digest_len: u16,
    pub digest_ref: u32,
    pub uri_ref: u32,
    pub schema_fingerprint_ref: u32,
    pub object_catalog_fingerprint_ref: u32,
    pub semantic_map_fingerprint_ref: u32,
    pub projection_fingerprint_ref: u32,
    pub checksum: u32,
}

impl DeltaParentRefV1 {
    pub fn serialize(&self) -> [u8; COVEDELTA_PARENT_REF_LEN] {
        let mut buf = [0u8; COVEDELTA_PARENT_REF_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.parent_ref);
        put_u8(&mut buf, &mut pos, self.parent_kind);
        put_u32(&mut buf, &mut pos, self.flags);
        put(&mut buf, &mut pos, &self.artifact_id);
        put(&mut buf, &mut pos, &self.snapshot_id);
        put_u64(&mut buf, &mut pos, self.file_len);
        put_u32(&mut buf, &mut pos, self.footer_crc32c);
        put_u16(&mut buf, &mut pos, self.digest_algorithm);
        put_u16(&mut buf, &mut pos, self.digest_len);
        put_u32(&mut buf, &mut pos, self.digest_ref);
        put_u32(&mut buf, &mut pos, self.uri_ref);
        put_u32(&mut buf, &mut pos, self.schema_fingerprint_ref);
        put_u32(&mut buf, &mut pos, self.object_catalog_fingerprint_ref);
        put_u32(&mut buf, &mut pos, self.semantic_map_fingerprint_ref);
        put_u32(&mut buf, &mut pos, self.projection_fingerprint_ref);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, COVEDELTA_PARENT_REF_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEDELTA_PARENT_REF_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEDELTA_PARENT_REF_LEN];
        let mut pos = 0usize;
        let parent_ref = take_u32(bytes, &mut pos)?;
        let parent_kind = take_u8(bytes, &mut pos)?;
        let flags = take_u32(bytes, &mut pos)?;
        let artifact_id = take_array::<16>(bytes, &mut pos)?;
        let snapshot_id = take_array::<16>(bytes, &mut pos)?;
        let file_len = take_u64(bytes, &mut pos)?;
        let footer_crc32c = take_u32(bytes, &mut pos)?;
        let digest_algorithm = take_u16(bytes, &mut pos)?;
        let digest_len = take_u16(bytes, &mut pos)?;
        let digest_ref = take_u32(bytes, &mut pos)?;
        let uri_ref = take_u32(bytes, &mut pos)?;
        let schema_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let object_catalog_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let semantic_map_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let projection_fingerprint_ref = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        Ok(Self {
            parent_ref,
            parent_kind,
            flags,
            artifact_id,
            snapshot_id,
            file_len,
            footer_crc32c,
            digest_algorithm,
            digest_len,
            digest_ref,
            uri_ref,
            schema_fingerprint_ref,
            object_catalog_fingerprint_ref,
            semantic_map_fingerprint_ref,
            projection_fingerprint_ref,
            checksum,
        })
    }

    pub fn validate_object_delta_binding(&self) -> Result<(), CoveError> {
        let algorithm = DigestAlgorithm::from_u16(self.digest_algorithm).ok_or_else(|| {
            CoveError::BadSection("COVEDELTA parent ref has unknown digest_algorithm".into())
        })?;
        if algorithm == DigestAlgorithm::None {
            return Err(CoveError::BadSection(
                "COVEDELTA parent ref requires cryptographic digest_algorithm".into(),
            ));
        }
        let expected_len = match algorithm {
            DigestAlgorithm::Sha256 | DigestAlgorithm::Blake3 => 32,
            DigestAlgorithm::None => unreachable!("None handled above"),
        };
        if self.digest_len as usize != expected_len {
            return Err(CoveError::BadSection(format!(
                "COVEDELTA parent ref digest_len must be {expected_len}, got {}",
                self.digest_len
            )));
        }
        if self.digest_ref == DELTA_REF_NONE {
            return Err(CoveError::BadSection(
                "COVEDELTA parent ref requires digest_ref".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveDeltaSectionDirectoryEntryV1 {
    pub section_id: u32,
    pub section_kind: u16,
    pub flags: u16,
    pub offset: u64,
    pub length: u64,
    pub uncompressed_length: u64,
    pub item_count: u64,
    pub compression: u8,
    pub encryption: u8,
    pub alignment_log2: u8,
    pub reserved0: u8,
    pub required_delta_features: u64,
    pub optional_delta_features: u64,
    pub crc32c: u32,
    pub checksum: u32,
}

impl CoveDeltaSectionDirectoryEntryV1 {
    pub fn serialize(&self) -> [u8; COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN] {
        let mut buf = [0u8; COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.section_id);
        put_u16(&mut buf, &mut pos, self.section_kind);
        put_u16(&mut buf, &mut pos, self.flags);
        put_u64(&mut buf, &mut pos, self.offset);
        put_u64(&mut buf, &mut pos, self.length);
        put_u64(&mut buf, &mut pos, self.uncompressed_length);
        put_u64(&mut buf, &mut pos, self.item_count);
        put_u8(&mut buf, &mut pos, self.compression);
        put_u8(&mut buf, &mut pos, self.encryption);
        put_u8(&mut buf, &mut pos, self.alignment_log2);
        put_u8(&mut buf, &mut pos, self.reserved0);
        put_u64(&mut buf, &mut pos, self.required_delta_features);
        put_u64(&mut buf, &mut pos, self.optional_delta_features);
        put_u32(&mut buf, &mut pos, self.crc32c);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN];
        let mut pos = 0usize;
        let section_id = take_u32(bytes, &mut pos)?;
        let section_kind = take_u16(bytes, &mut pos)?;
        let flags = take_u16(bytes, &mut pos)?;
        let offset = take_u64(bytes, &mut pos)?;
        let length = take_u64(bytes, &mut pos)?;
        let uncompressed_length = take_u64(bytes, &mut pos)?;
        let item_count = take_u64(bytes, &mut pos)?;
        let compression = take_u8(bytes, &mut pos)?;
        let encryption = take_u8(bytes, &mut pos)?;
        if encryption != 0 {
            return Err(CoveError::BadSection(
                "COVEDELTA section encryption must be 0 in v1".into(),
            ));
        }
        let alignment_log2 = take_u8(bytes, &mut pos)?;
        let reserved0 = take_u8(bytes, &mut pos)?;
        if reserved0 != 0 {
            return Err(CoveError::ReservedNotZero);
        }
        let required_delta_features = take_u64(bytes, &mut pos)?;
        let optional_delta_features = take_u64(bytes, &mut pos)?;
        let crc32c = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        Ok(Self {
            section_id,
            section_kind,
            flags,
            offset,
            length,
            uncompressed_length,
            item_count,
            compression,
            encryption,
            alignment_log2,
            reserved0,
            required_delta_features,
            optional_delta_features,
            crc32c,
            checksum,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveDeltaFooterV1 {
    pub header_offset: u64,
    pub header_length: u64,
    pub section_directory_offset: u64,
    pub section_directory_length: u64,
    pub section_count: u32,
    pub parent_ref_count: u32,
    pub footer_crc32c: u32,
    pub checksum: u32,
}

impl CoveDeltaFooterV1 {
    pub fn serialize(&self) -> [u8; COVEDELTA_FOOTER_LEN as usize] {
        let mut buf = [0u8; COVEDELTA_FOOTER_LEN as usize];
        let mut pos = 0usize;
        put_u64(&mut buf, &mut pos, self.header_offset);
        put_u64(&mut buf, &mut pos, self.header_length);
        put_u64(&mut buf, &mut pos, self.section_directory_offset);
        put_u64(&mut buf, &mut pos, self.section_directory_length);
        put_u32(&mut buf, &mut pos, self.section_count);
        put_u32(&mut buf, &mut pos, self.parent_ref_count);
        let footer_crc_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, COVEDELTA_FOOTER_LEN as usize);
        let footer_crc32c = checksum::crc32c(&buf);
        buf[footer_crc_pos..footer_crc_pos + 4].copy_from_slice(&footer_crc32c.to_le_bytes());
        let mut for_checksum = buf;
        for_checksum[checksum_pos..checksum_pos + 4].fill(0);
        let checksum = checksum::crc32c(&for_checksum);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&checksum.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEDELTA_FOOTER_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEDELTA_FOOTER_LEN as usize];
        let mut pos = 0usize;
        let header_offset = take_u64(bytes, &mut pos)?;
        let header_length = take_u64(bytes, &mut pos)?;
        let section_directory_offset = take_u64(bytes, &mut pos)?;
        let section_directory_length = take_u64(bytes, &mut pos)?;
        let section_count = take_u32(bytes, &mut pos)?;
        let parent_ref_count = take_u32(bytes, &mut pos)?;
        let footer_crc32c = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[40..44].fill(0);
        for_crc[44..48].fill(0);
        if checksum::crc32c(&for_crc) != footer_crc32c {
            return Err(CoveError::ChecksumMismatch);
        }
        let mut for_checksum = bytes.to_vec();
        for_checksum[44..48].fill(0);
        if checksum::crc32c(&for_checksum) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        Ok(Self {
            header_offset,
            header_length,
            section_directory_offset,
            section_directory_length,
            section_count,
            parent_ref_count,
            footer_crc32c,
            checksum,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveDeltaPostscriptV1 {
    pub required_delta_features: u64,
    pub optional_delta_features: u64,
    pub file_len: u64,
    pub footer_offset: u64,
    pub footer_length: u64,
    pub checksum: u32,
}

impl CoveDeltaPostscriptV1 {
    pub fn serialize(&self) -> [u8; COVEDELTA_POSTSCRIPT_LEN as usize] {
        let mut buf = [0u8; COVEDELTA_POSTSCRIPT_LEN as usize];
        let mut pos = 0usize;
        put_u64(&mut buf, &mut pos, self.required_delta_features);
        put_u64(&mut buf, &mut pos, self.optional_delta_features);
        put_u64(&mut buf, &mut pos, self.file_len);
        put_u64(&mut buf, &mut pos, self.footer_offset);
        put_u64(&mut buf, &mut pos, self.footer_length);
        let checksum_pos = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, COVEDELTA_POSTSCRIPT_LEN as usize);
        let crc = checksum::crc32c(&buf);
        buf[checksum_pos..checksum_pos + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEDELTA_POSTSCRIPT_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVEDELTA_POSTSCRIPT_LEN as usize];
        let mut pos = 0usize;
        let required_delta_features = take_u64(bytes, &mut pos)?;
        let optional_delta_features = take_u64(bytes, &mut pos)?;
        let file_len = take_u64(bytes, &mut pos)?;
        let footer_offset = take_u64(bytes, &mut pos)?;
        let footer_length = take_u64(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        let mut for_crc = bytes.to_vec();
        for_crc[pos - 4..pos].fill(0);
        if checksum::crc32c(&for_crc) != checksum {
            return Err(CoveError::ChecksumMismatch);
        }
        Ok(Self {
            required_delta_features,
            optional_delta_features,
            file_len,
            footer_offset,
            footer_length,
            checksum,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveDeltaSection {
    pub entry: CoveDeltaSectionDirectoryEntryV1,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveDeltaFile {
    pub header: CoveDeltaHeaderV1,
    pub parent_refs: Vec<DeltaParentRefV1>,
    pub sections: Vec<CoveDeltaSection>,
    pub footer: CoveDeltaFooterV1,
    pub postscript: CoveDeltaPostscriptV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveDeltaObjectValidation {
    pub scope_kind: u16,
    pub scope_id: [u8; 16],
    pub catalog_patches: Vec<ObjectTypeCatalog>,
    pub dictionary_overlay_entries: Vec<DeltaDictionaryEntryV1>,
    pub inline_values: Vec<DeltaInlineValueV1>,
    pub evidence_patches: Vec<MapEvidenceIndex>,
    pub projection_patches: Vec<MapProjectionCatalog>,
    pub index_hints: Vec<DeltaSidecarHintV1>,
    pub coverage_patches: Vec<DeltaSidecarHintV1>,
    pub effective_schema_fingerprint_ref: u32,
    pub effective_object_catalog_fingerprint_ref: u32,
    pub effective_semantic_map_fingerprint_ref: u32,
    pub effective_projection_fingerprint_ref: u32,
    pub temporal_segments: Vec<TemporalSegmentData>,
    pub branch_identities: Vec<DeltaBranchIdentityV1>,
    pub scope_descriptors: Vec<DeltaScopeDescriptorV1>,
    pub temporal_role_summary_descriptors: Vec<DeltaSummaryDescriptorV1>,
    pub touched_summary_descriptors: Vec<DeltaSummaryDescriptorV1>,
    pub tombstone_summary_descriptors: Vec<DeltaSummaryDescriptorV1>,
    pub continuation_anchors: Vec<DeltaContinuationAnchorV1>,
    pub state_hash_descriptors: Vec<DeltaStateHashDescriptorV1>,
    pub sparse_patch_records: Vec<DeltaSparsePatchRecordV1>,
    pub checkpoint_row_count: usize,
    pub has_touched_object_set_section: bool,
    pub touched_object_ranges: Vec<DeltaTouchedObjectRangeV1>,
    pub has_tombstone_object_set_section: bool,
    pub tombstone_object_ranges: Vec<DeltaTouchedObjectRangeV1>,
}

impl CoveDeltaFile {
    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        let mut header = self.header.clone();
        header.parent_ref_count = self.parent_refs.len() as u32;
        header.section_count = self.sections.len() as u32;
        header.parent_refs_offset = COVEDELTA_HEADER_LEN as u64;
        header.parent_refs_length = (self.parent_refs.len() * COVEDELTA_PARENT_REF_LEN) as u64;

        let mut out = vec![0; COVEDELTA_HEADER_LEN as usize];
        for parent in &self.parent_refs {
            out.extend_from_slice(&parent.serialize());
        }

        let mut sections = Vec::with_capacity(self.sections.len());
        for (index, section) in self.sections.iter().enumerate() {
            let mut entry = section.entry.clone();
            entry.section_id = if entry.section_id == 0 {
                (index + 1) as u32
            } else {
                entry.section_id
            };
            entry.offset = out.len() as u64;
            entry.length = section.payload.len() as u64;
            entry.uncompressed_length = entry.uncompressed_length.max(entry.length);
            entry.crc32c = checksum::crc32c(&section.payload);
            out.extend_from_slice(&section.payload);
            sections.push(CoveDeltaSection {
                entry,
                payload: section.payload.clone(),
            });
        }

        header.section_directory_offset = out.len() as u64;
        header.section_directory_length =
            (sections.len() * COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN) as u64;
        out[0..COVEDELTA_HEADER_LEN as usize].copy_from_slice(&header.serialize());
        for section in &sections {
            out.extend_from_slice(&section.entry.serialize());
        }

        let footer_offset = out.len() as u64;
        let footer = CoveDeltaFooterV1 {
            header_offset: 0,
            header_length: COVEDELTA_HEADER_LEN as u64,
            section_directory_offset: header.section_directory_offset,
            section_directory_length: header.section_directory_length,
            section_count: header.section_count,
            parent_ref_count: header.parent_ref_count,
            footer_crc32c: 0,
            checksum: 0,
        };
        let footer_bytes = footer.serialize();
        out.extend_from_slice(&footer_bytes);
        let file_len =
            out.len() + COVEDELTA_POSTSCRIPT_LEN as usize + COVEDELTA_POSTSCRIPT_TAIL_SIZE;
        let postscript = CoveDeltaPostscriptV1 {
            required_delta_features: header.required_delta_features,
            optional_delta_features: header.optional_delta_features,
            file_len: file_len as u64,
            footer_offset,
            footer_length: COVEDELTA_FOOTER_LEN as u64,
            checksum: 0,
        };
        out.extend_from_slice(&postscript.serialize());
        out.extend_from_slice(&COVEDELTA_POSTSCRIPT_VERSION_V1.to_le_bytes());
        out.extend_from_slice(&COVEDELTA_POSTSCRIPT_LEN.to_le_bytes());
        out.extend_from_slice(&MAGIC_COVEDELTA);
        Ok(out)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVEDELTA_POSTSCRIPT_LEN as usize + COVEDELTA_POSTSCRIPT_TAIL_SIZE {
            return Err(CoveError::BufferTooShort);
        }
        let tail_start = bytes
            .len()
            .checked_sub(COVEDELTA_POSTSCRIPT_TAIL_SIZE)
            .ok_or(CoveError::ArithOverflow)?;
        if bytes[tail_start + 4..tail_start + 8] != MAGIC_COVEDELTA {
            return Err(CoveError::BadMagic);
        }
        let postscript_version = wire::read_u16_le_checked(bytes, tail_start)?;
        if postscript_version != COVEDELTA_POSTSCRIPT_VERSION_V1 {
            return Err(CoveError::BadVersion);
        }
        let postscript_len = wire::read_u16_le_checked(bytes, tail_start + 2)?;
        if postscript_len != COVEDELTA_POSTSCRIPT_LEN {
            return Err(CoveError::BadSection(format!(
                "COVEDELTA postscript_len must be {COVEDELTA_POSTSCRIPT_LEN}, got {postscript_len}"
            )));
        }
        let postscript_start = tail_start
            .checked_sub(postscript_len as usize)
            .ok_or(CoveError::ArithOverflow)?;
        let postscript = CoveDeltaPostscriptV1::parse(&bytes[postscript_start..tail_start])?;
        if postscript.file_len != bytes.len() as u64 {
            return Err(CoveError::BadSection(
                "COVEDELTA postscript file_len mismatch".into(),
            ));
        }
        if postscript.footer_length != COVEDELTA_FOOTER_LEN as u64 {
            return Err(CoveError::BadSection(
                "COVEDELTA footer_length mismatch".into(),
            ));
        }
        let footer_range = checked_range(
            postscript.footer_offset,
            postscript.footer_length,
            bytes.len(),
        )?;
        if footer_range.end != postscript_start {
            return Err(CoveError::BadSection(
                "COVEDELTA footer must immediately precede postscript".into(),
            ));
        }
        let footer = CoveDeltaFooterV1::parse(&bytes[footer_range.clone()])?;
        let header_range = checked_range(footer.header_offset, footer.header_length, bytes.len())?;
        if header_range.start != 0 {
            return Err(CoveError::BadSection(
                "COVEDELTA header must start at offset 0".into(),
            ));
        }
        let header = CoveDeltaHeaderV1::parse(&bytes[header_range.clone()])?;
        if footer.header_length != COVEDELTA_HEADER_LEN as u64
            || header.section_count != footer.section_count
            || header.parent_ref_count != footer.parent_ref_count
            || header.section_directory_offset != footer.section_directory_offset
            || header.section_directory_length != footer.section_directory_length
        {
            return Err(CoveError::BadSection(
                "COVEDELTA header/footer metadata mismatch".into(),
            ));
        }
        let parent_range = checked_range(
            header.parent_refs_offset,
            header.parent_refs_length,
            bytes.len(),
        )?;
        if parent_range.start != header_range.end {
            return Err(CoveError::BadSection(
                "COVEDELTA parent refs must immediately follow header".into(),
            ));
        }
        let expected_parent_len = (footer.parent_ref_count as usize)
            .checked_mul(COVEDELTA_PARENT_REF_LEN)
            .ok_or(CoveError::ArithOverflow)?;
        if parent_range.len() != expected_parent_len {
            return Err(CoveError::BadSection(
                "COVEDELTA parent refs length mismatch".into(),
            ));
        }
        let mut parent_refs = Vec::with_capacity(footer.parent_ref_count as usize);
        for chunk in bytes[parent_range.clone()].chunks_exact(COVEDELTA_PARENT_REF_LEN) {
            parent_refs.push(DeltaParentRefV1::parse(chunk)?);
        }
        let lineage_parent_count = parent_refs
            .iter()
            .filter(|parent| parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0)
            .count();
        if lineage_parent_count != 1 {
            return Err(CoveError::BadSection(
                "COVEDELTA requires exactly one lineage parent ref".into(),
            ));
        }

        let directory_range = checked_range(
            footer.section_directory_offset,
            footer.section_directory_length,
            bytes.len(),
        )?;
        if directory_range.end != footer_range.start {
            return Err(CoveError::BadSection(
                "COVEDELTA section directory must immediately precede footer".into(),
            ));
        }
        let expected_directory_len = (footer.section_count as usize)
            .checked_mul(COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN)
            .ok_or(CoveError::ArithOverflow)?;
        if directory_range.len() != expected_directory_len {
            return Err(CoveError::BadSection(
                "COVEDELTA section directory length mismatch".into(),
            ));
        }
        let mut sections = Vec::with_capacity(footer.section_count as usize);
        let mut section_ids = std::collections::BTreeSet::new();
        let mut payload_cursor = parent_range.end;
        for chunk in
            bytes[directory_range.clone()].chunks_exact(COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN)
        {
            let entry = CoveDeltaSectionDirectoryEntryV1::parse(chunk)?;
            if !section_ids.insert(entry.section_id) {
                return Err(CoveError::BadSection(
                    "COVEDELTA duplicate section_id".into(),
                ));
            }
            if CoveDeltaSectionKind::from_u16(entry.section_kind).is_none() {
                return Err(CoveError::BadSection(format!(
                    "unknown COVEDELTA section kind {}",
                    entry.section_kind
                )));
            }
            let payload_range = checked_range(entry.offset, entry.length, bytes.len())?;
            if payload_range.start != payload_cursor || payload_range.end > directory_range.start {
                return Err(CoveError::BadSection(
                    "COVEDELTA section payload regions must be canonical and non-overlapping"
                        .into(),
                ));
            }
            payload_cursor = payload_range.end;
            let payload = bytes[payload_range].to_vec();
            if checksum::crc32c(&payload) != entry.crc32c {
                return Err(CoveError::ChecksumMismatch);
            }
            sections.push(CoveDeltaSection { entry, payload });
        }
        if payload_cursor != directory_range.start {
            return Err(CoveError::BadSection(
                "COVEDELTA section payloads must immediately precede section directory".into(),
            ));
        }

        Ok(Self {
            header,
            parent_refs,
            sections,
            footer,
            postscript,
        })
    }

    pub fn validate_object_delta_sections(&self) -> Result<Vec<TemporalSegmentData>, CoveError> {
        self.validate_object_delta()
            .map(|validated| validated.temporal_segments)
    }

    pub fn validate_object_delta(&self) -> Result<CoveDeltaObjectValidation, CoveError> {
        validate_delta_required_features(
            self.header.required_delta_features,
            COVEDELTA_OBJECT_TEMPORAL_SUPPORTED_REQUIRED_FEATURES,
        )?;
        validate_delta_required_features(
            self.postscript.required_delta_features,
            COVEDELTA_OBJECT_TEMPORAL_SUPPORTED_REQUIRED_FEATURES,
        )?;
        if self.header.required_delta_features != self.postscript.required_delta_features
            || self.header.optional_delta_features != self.postscript.optional_delta_features
        {
            return Err(CoveError::BadSection(
                "COVEDELTA header and postscript feature bits disagree".into(),
            ));
        }
        if self.header.csn_min > self.header.csn_max {
            return Err(CoveError::BadSection(
                "COVEDELTA header csn_min must be <= csn_max".into(),
            ));
        }
        if self.header.commit_time_range_start_us > self.header.commit_time_range_end_us {
            return Err(CoveError::BadSection(
                "COVEDELTA header commit-time range is inverted".into(),
            ));
        }
        if self.header.flags & DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT != 0 {
            if self.header.source_publish_range_start_us > self.header.source_publish_range_end_us {
                return Err(CoveError::BadSection(
                    "COVEDELTA source-publish range is inverted".into(),
                ));
            }
        } else if self.header.source_publish_range_start_us != 0
            || self.header.source_publish_range_end_us != 0
        {
            return Err(CoveError::BadSection(
                "COVEDELTA source-publish range requires header flag".into(),
            ));
        }

        let parent_ref_ids = validate_object_delta_parent_refs(&self.parent_refs)?;
        let lineage_parent = self
            .parent_refs
            .iter()
            .find(|parent| parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0);
        if let Some(lineage_parent) = lineage_parent {
            if lineage_parent.snapshot_id != self.header.parent_snapshot_id {
                return Err(CoveError::BadSection(
                    "COVEDELTA lineage parent snapshot_id must match header parent_snapshot_id"
                        .into(),
                ));
            }
        }
        let effective_schema_fingerprint_ref = if self.header.schema_fingerprint_ref == 0 {
            lineage_parent
                .map(|parent| parent.schema_fingerprint_ref)
                .unwrap_or(0)
        } else {
            self.header.schema_fingerprint_ref
        };
        let effective_object_catalog_fingerprint_ref =
            if self.header.object_catalog_fingerprint_ref == 0 {
                lineage_parent
                    .map(|parent| parent.object_catalog_fingerprint_ref)
                    .unwrap_or(0)
            } else {
                self.header.object_catalog_fingerprint_ref
            };
        let effective_semantic_map_fingerprint_ref =
            if self.header.semantic_map_fingerprint_ref == 0 {
                lineage_parent
                    .map(|parent| parent.semantic_map_fingerprint_ref)
                    .unwrap_or(0)
            } else {
                self.header.semantic_map_fingerprint_ref
            };
        let effective_projection_fingerprint_ref = if self.header.projection_fingerprint_ref == 0 {
            lineage_parent
                .map(|parent| parent.projection_fingerprint_ref)
                .unwrap_or(0)
        } else {
            self.header.projection_fingerprint_ref
        };

        let mut catalog_patches = Vec::new();
        let mut dictionary_overlay_entries = Vec::new();
        let mut dictionary_overlay_local_codes = BTreeSet::new();
        let mut inline_values = Vec::new();
        let mut inline_value_refs = BTreeSet::new();
        let mut evidence_patches = Vec::new();
        let mut projection_patches = Vec::new();
        let mut index_hints = Vec::new();
        let mut coverage_patches = Vec::new();
        let mut temporal_segments = Vec::new();
        let mut branch_identities = Vec::new();
        let mut scope_descriptors = Vec::new();
        let mut temporal_role_summary_descriptors = Vec::new();
        let mut touched_summary_descriptors = Vec::new();
        let mut tombstone_summary_descriptors = Vec::new();
        let mut continuation_anchors = Vec::new();
        let mut state_hash_descriptors = Vec::new();
        let mut sparse_patch_records = Vec::new();
        let mut touched_object_ranges = Vec::new();
        let mut tombstone_object_ranges = Vec::new();
        let mut has_touched_set_section = false;
        let mut has_tombstone_set_section = false;
        for section in &self.sections {
            let Some(kind) = CoveDeltaSectionKind::from_u16(section.entry.section_kind) else {
                return Err(CoveError::BadSection(format!(
                    "unknown COVEDELTA section kind {}",
                    section.entry.section_kind
                )));
            };
            if covedelta_object_delta_requires_section_features(kind) {
                validate_delta_required_features(
                    section.entry.required_delta_features,
                    COVEDELTA_OBJECT_TEMPORAL_SUPPORTED_REQUIRED_FEATURES,
                )?;
            }
            match kind {
                CoveDeltaSectionKind::CatalogPatch => {
                    if section.entry.compression != 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA catalog patch validation requires uncompressed payload"
                                .into(),
                        ));
                    }
                    let patch = ObjectTypeCatalog::parse(&section.payload)?;
                    if section.entry.item_count != patch.types.len() as u64 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA catalog patch item_count does not match payload".into(),
                        ));
                    }
                    catalog_patches.push(patch);
                }
                CoveDeltaSectionKind::DictionaryOverlay => {
                    if section.entry.compression != 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA dictionary overlay validation requires uncompressed payload"
                                .into(),
                        ));
                    }
                    if section.entry.item_count == 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA dictionary overlay section requires entries".into(),
                        ));
                    }
                    let entries = parse_fixed_records(
                        section,
                        DELTA_DICTIONARY_ENTRY_LEN,
                        DeltaDictionaryEntryV1::parse,
                        "COVEDELTA dictionary overlay",
                    )?;
                    for entry in &entries {
                        entry.validate_for_object_delta(
                            self.header.required_delta_features,
                            section.entry.required_delta_features,
                            &parent_ref_ids,
                        )?;
                        if !dictionary_overlay_local_codes
                            .insert((entry.local_dictionary_id, entry.local_code))
                        {
                            return Err(CoveError::BadSection(
                                "COVEDELTA dictionary overlay has duplicate local dictionary code"
                                    .into(),
                            ));
                        }
                    }
                    dictionary_overlay_entries.extend(entries);
                }
                CoveDeltaSectionKind::StringTable => {
                    if section.entry.compression != 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA string table validation requires uncompressed payload"
                                .into(),
                        ));
                    }
                    if section.entry.item_count == 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA string table section requires inline value records".into(),
                        ));
                    }
                    let records = DeltaInlineValueV1::parse_many(&section.payload)?;
                    if section.entry.item_count != records.len() as u64 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA string table item_count does not match payload".into(),
                        ));
                    }
                    for record in &records {
                        if !inline_value_refs.insert(record.value_ref) {
                            return Err(CoveError::BadSection(
                                "COVEDELTA string table contains duplicate value_ref".into(),
                            ));
                        }
                    }
                    inline_values.extend(records);
                }
                CoveDeltaSectionKind::EvidencePatch => {
                    if self.header.required_delta_features & DELTA_FEATURE_MAP_EVIDENCE_PATCH == 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA evidence patch section requires map evidence patch feature"
                                .into(),
                        ));
                    }
                    if section.entry.compression != 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA evidence patch validation requires uncompressed payload"
                                .into(),
                        ));
                    }
                    let patch = MapEvidenceIndex::parse(&section.payload)?;
                    if patch.entries.is_empty() {
                        return Err(CoveError::BadSection(
                            "COVEDELTA evidence patch requires evidence entries".into(),
                        ));
                    }
                    if section.entry.item_count != patch.entries.len() as u64 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA evidence patch item_count does not match payload".into(),
                        ));
                    }
                    evidence_patches.push(patch);
                }
                CoveDeltaSectionKind::ProjectionPatch => {
                    if self.header.required_delta_features & DELTA_FEATURE_PROJECTION_PATCH == 0
                        && section.entry.required_delta_features & DELTA_FEATURE_PROJECTION_PATCH
                            == 0
                    {
                        continue;
                    }
                    if self.header.required_delta_features & DELTA_FEATURE_PROJECTION_PATCH == 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA projection patch section requires projection patch feature"
                                .into(),
                        ));
                    }
                    validate_delta_required_features(
                        section.entry.required_delta_features,
                        COVEDELTA_OBJECT_TEMPORAL_SUPPORTED_REQUIRED_FEATURES,
                    )?;
                    if section.entry.required_delta_features & DELTA_FEATURE_PROJECTION_PATCH == 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA required projection patch feature requires section feature binding"
                                .into(),
                        ));
                    }
                    if section.entry.compression != 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA projection patch validation requires uncompressed payload"
                                .into(),
                        ));
                    }
                    let patch = MapProjectionCatalog::parse(&section.payload)?;
                    if patch.projections.is_empty() {
                        return Err(CoveError::BadSection(
                            "COVEDELTA projection patch requires projection entries".into(),
                        ));
                    }
                    if section.entry.item_count != patch.projections.len() as u64 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA projection patch item_count does not match payload".into(),
                        ));
                    }
                    projection_patches.push(patch);
                }
                CoveDeltaSectionKind::TemporalSegmentData => {
                    if section.entry.compression != 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA temporal segment validation requires uncompressed payload"
                                .into(),
                        ));
                    }
                    if section.entry.item_count != 1 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA temporal segment data section must contain one segment"
                                .into(),
                        ));
                    }
                    let segment = TemporalSegmentData::parse(&section.payload)?;
                    if segment.header.csn_min < self.header.csn_min
                        || segment.header.csn_max > self.header.csn_max
                    {
                        return Err(CoveError::BadSection(
                            "COVEDELTA temporal segment CSN range falls outside delta header range"
                                .into(),
                        ));
                    }
                    temporal_segments.push(segment);
                }
                CoveDeltaSectionKind::TombstoneSet
                | CoveDeltaSectionKind::ContinuationAnchors
                | CoveDeltaSectionKind::PropertyOps
                | CoveDeltaSectionKind::TouchedObjectSet
                | CoveDeltaSectionKind::StateHashTable
                | CoveDeltaSectionKind::BranchIdentityTable
                | CoveDeltaSectionKind::ScopeTable
                | CoveDeltaSectionKind::TemporalRoleSummaryTable
                | CoveDeltaSectionKind::TouchedSummaryTable
                | CoveDeltaSectionKind::TombstoneSummaryTable => {
                    if section.entry.compression != 0 {
                        return Err(CoveError::BadSection(
                            "COVEDELTA required object-temporal sections must be uncompressed for validation"
                                .into(),
                        ));
                    }
                    match kind {
                        CoveDeltaSectionKind::BranchIdentityTable => {
                            branch_identities.extend(parse_fixed_records(
                                section,
                                DELTA_BRANCH_IDENTITY_LEN,
                                DeltaBranchIdentityV1::parse,
                                "COVEDELTA branch identity",
                            )?);
                        }
                        CoveDeltaSectionKind::ContinuationAnchors => {
                            continuation_anchors.extend(parse_fixed_records(
                                section,
                                DELTA_CONTINUATION_ANCHOR_LEN,
                                DeltaContinuationAnchorV1::parse,
                                "COVEDELTA continuation anchor",
                            )?);
                        }
                        CoveDeltaSectionKind::TouchedObjectSet => {
                            has_touched_set_section = true;
                            touched_object_ranges.extend(parse_fixed_records(
                                section,
                                DELTA_TOUCHED_OBJECT_RANGE_LEN,
                                DeltaTouchedObjectRangeV1::parse,
                                "COVEDELTA touched object range",
                            )?);
                        }
                        CoveDeltaSectionKind::TombstoneSet => {
                            has_tombstone_set_section = true;
                            tombstone_object_ranges.extend(parse_fixed_records(
                                section,
                                DELTA_TOUCHED_OBJECT_RANGE_LEN,
                                DeltaTouchedObjectRangeV1::parse,
                                "COVEDELTA tombstone object range",
                            )?);
                        }
                        CoveDeltaSectionKind::StateHashTable => {
                            state_hash_descriptors.extend(parse_fixed_records(
                                section,
                                DELTA_STATE_HASH_DESCRIPTOR_LEN,
                                DeltaStateHashDescriptorV1::parse,
                                "COVEDELTA state hash descriptor",
                            )?);
                        }
                        CoveDeltaSectionKind::ScopeTable => {
                            scope_descriptors.extend(parse_fixed_records(
                                section,
                                DELTA_SCOPE_DESCRIPTOR_LEN,
                                DeltaScopeDescriptorV1::parse,
                                "COVEDELTA scope descriptor",
                            )?);
                        }
                        CoveDeltaSectionKind::TemporalRoleSummaryTable => {
                            temporal_role_summary_descriptors.extend(parse_fixed_records(
                                section,
                                DELTA_SUMMARY_DESCRIPTOR_LEN,
                                DeltaSummaryDescriptorV1::parse,
                                "COVEDELTA temporal role summary descriptor",
                            )?);
                        }
                        CoveDeltaSectionKind::TouchedSummaryTable => {
                            touched_summary_descriptors.extend(parse_fixed_records(
                                section,
                                DELTA_SUMMARY_DESCRIPTOR_LEN,
                                DeltaSummaryDescriptorV1::parse,
                                "COVEDELTA touched summary descriptor",
                            )?);
                        }
                        CoveDeltaSectionKind::TombstoneSummaryTable => {
                            tombstone_summary_descriptors.extend(parse_fixed_records(
                                section,
                                DELTA_SUMMARY_DESCRIPTOR_LEN,
                                DeltaSummaryDescriptorV1::parse,
                                "COVEDELTA tombstone summary descriptor",
                            )?);
                        }
                        CoveDeltaSectionKind::PropertyOps => {
                            if section.entry.item_count == 0 {
                                return Err(CoveError::BadSection(
                                    "COVEDELTA sparse property ops section requires records".into(),
                                ));
                            }
                            let records = DeltaSparsePatchRecordV1::parse_many(&section.payload)?;
                            if section.entry.item_count != records.len() as u64 {
                                return Err(CoveError::BadSection(
                                    "COVEDELTA sparse property ops item_count does not match payload"
                                        .into(),
                                ));
                            }
                            sparse_patch_records.extend(records);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if temporal_segments.is_empty() {
            return Err(CoveError::BadSection(
                "COVEDELTA object delta requires at least one temporal segment data section".into(),
            ));
        }
        if self.header.required_delta_features & DELTA_FEATURE_MAP_EVIDENCE_PATCH != 0
            && evidence_patches.is_empty()
        {
            return Err(CoveError::BadSection(
                "COVEDELTA required map evidence patch feature requires evidence patch section"
                    .into(),
            ));
        }
        if !evidence_patches.is_empty() && effective_semantic_map_fingerprint_ref == 0 {
            return Err(CoveError::BadSection(
                "COVEDELTA evidence patch requires inherited or declared semantic map fingerprint"
                    .into(),
            ));
        }
        if self.header.required_delta_features & DELTA_FEATURE_PROJECTION_PATCH != 0
            && projection_patches.is_empty()
        {
            return Err(CoveError::BadSection(
                "COVEDELTA required projection patch feature requires projection patch section"
                    .into(),
            ));
        }
        if !projection_patches.is_empty() && effective_projection_fingerprint_ref == 0 {
            return Err(CoveError::BadSection(
                "COVEDELTA projection patch requires inherited or declared projection fingerprint"
                    .into(),
            ));
        }
        if self.header.required_delta_features & DELTA_FEATURE_INDEX_HINTS != 0 {
            index_hints = validate_sidecar_hints_for_section(
                self,
                CoveDeltaSectionKind::IndexHints,
                "COVEDELTA index hints",
                &[
                    DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
                    DELTA_SIDECAR_HINT_KIND_COVX_INDEX,
                ],
                Some(DELTA_FEATURE_INDEX_HINTS),
            )?;
            if index_hints.is_empty() {
                return Err(CoveError::BadSection(
                    "COVEDELTA required index hint feature requires index hint section".into(),
                ));
            }
        }
        if self.header.required_delta_features & DELTA_FEATURE_COVERAGE_PATCH != 0 {
            coverage_patches = validate_sidecar_hints_for_section(
                self,
                CoveDeltaSectionKind::CoveragePatch,
                "COVEDELTA coverage patch",
                &[DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH],
                Some(DELTA_FEATURE_COVERAGE_PATCH),
            )?;
            if coverage_patches.is_empty() {
                return Err(CoveError::BadSection(
                    "COVEDELTA required coverage patch feature requires coverage patch section"
                        .into(),
                ));
            }
        }
        validate_single_scope_payloads(
            &self.header,
            &continuation_anchors,
            &sparse_patch_records,
            &touched_object_ranges,
            &tombstone_object_ranges,
        )?;
        if self.header.required_delta_features & DELTA_FEATURE_CONTINUATION_ANCHORS != 0
            && continuation_anchors.is_empty()
        {
            return Err(CoveError::BadSection(
                "COVEDELTA required continuation-anchor feature requires anchor records".into(),
            ));
        }
        if !continuation_anchors.is_empty() {
            validate_continuation_anchor_state_hash_refs(
                &continuation_anchors,
                &state_hash_descriptors,
            )?;
        }
        validate_dense_scope_descriptors(&scope_descriptors)?;
        validate_dense_summary_descriptors(
            "COVEDELTA temporal role summary descriptors",
            &temporal_role_summary_descriptors,
        )?;
        validate_dense_summary_descriptors(
            "COVEDELTA touched summary descriptors",
            &touched_summary_descriptors,
        )?;
        validate_dense_summary_descriptors(
            "COVEDELTA tombstone summary descriptors",
            &tombstone_summary_descriptors,
        )?;
        validate_dense_inline_value_refs(&inline_values)?;
        validate_dictionary_overlay_inline_value_refs(&dictionary_overlay_entries, &inline_values)?;
        validate_sparse_patch_value_refs(&sparse_patch_records, &inline_values)?;
        validate_property_bitmap_refs(
            "COVEDELTA touched range property_bitmap_ref",
            &touched_object_ranges,
            &touched_summary_descriptors,
        )?;
        validate_property_bitmap_refs(
            "COVEDELTA tombstone range property_bitmap_ref",
            &tombstone_object_ranges,
            &tombstone_summary_descriptors,
        )?;
        if self.header.required_delta_features & DELTA_FEATURE_SPARSE_PATCH_ROWS != 0
            && sparse_patch_records.is_empty()
        {
            return Err(CoveError::BadSection(
                "COVEDELTA required sparse patch feature requires property ops records".into(),
            ));
        }
        validate_unique_temporal_record_ids(&self.header, &temporal_segments)?;
        if !sparse_patch_records.is_empty() {
            validate_sparse_patch_records_cover_temporal_rows(
                &self.header,
                &temporal_segments,
                &sparse_patch_records,
            )?;
        }
        let checkpoint_row_count = checkpoint_row_count(&temporal_segments);
        if self.header.required_delta_features & DELTA_FEATURE_CHECKPOINT_BASELINES != 0
            && checkpoint_row_count == 0
        {
            return Err(CoveError::BadSection(
                "COVEDELTA checkpoint-baseline feature requires Snapshot or Baseline rows".into(),
            ));
        }
        if self.header.required_delta_features & DELTA_FEATURE_CONTINUATION_ANCHORS != 0 {
            validate_continuation_anchors_cover_temporal_rows(
                &self.header,
                &temporal_segments,
                &continuation_anchors,
            )?;
        }
        if self.header.required_delta_features & DELTA_FEATURE_EXACT_TOUCHED_SET != 0
            && !has_touched_set_section
        {
            return Err(CoveError::BadSection(
                "COVEDELTA required exact touched-set feature requires touched ranges".into(),
            ));
        }
        if has_touched_set_section {
            validate_touched_ranges_cover_temporal_rows(
                &self.header,
                &temporal_segments,
                &touched_object_ranges,
            )?;
        }
        if self.header.required_delta_features & DELTA_FEATURE_EXACT_TOMBSTONE_SET != 0
            && !has_tombstone_set_section
        {
            return Err(CoveError::BadSection(
                "COVEDELTA required exact tombstone-set feature requires tombstone ranges section"
                    .into(),
            ));
        }
        if has_tombstone_set_section {
            validate_tombstone_ranges_cover_temporal_rows(
                &self.header,
                &temporal_segments,
                &tombstone_object_ranges,
            )?;
        }
        Ok(CoveDeltaObjectValidation {
            scope_kind: self.header.scope_kind,
            scope_id: self.header.scope_id,
            catalog_patches,
            dictionary_overlay_entries,
            inline_values,
            evidence_patches,
            projection_patches,
            index_hints,
            coverage_patches,
            effective_schema_fingerprint_ref,
            effective_object_catalog_fingerprint_ref,
            effective_semantic_map_fingerprint_ref,
            effective_projection_fingerprint_ref,
            temporal_segments,
            branch_identities,
            scope_descriptors,
            temporal_role_summary_descriptors,
            touched_summary_descriptors,
            tombstone_summary_descriptors,
            continuation_anchors,
            state_hash_descriptors,
            sparse_patch_records,
            checkpoint_row_count,
            has_touched_object_set_section: has_touched_set_section,
            touched_object_ranges,
            has_tombstone_object_set_section: has_tombstone_set_section,
            tombstone_object_ranges,
        })
    }

    pub fn validate_index_hints(&self) -> Result<Vec<DeltaSidecarHintV1>, CoveError> {
        validate_sidecar_hints_for_section(
            self,
            CoveDeltaSectionKind::IndexHints,
            "COVEDELTA index hints",
            &[
                DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
                DELTA_SIDECAR_HINT_KIND_COVX_INDEX,
            ],
            None,
        )
    }

    pub fn validate_coverage_patches(&self) -> Result<Vec<DeltaSidecarHintV1>, CoveError> {
        validate_sidecar_hints_for_section(
            self,
            CoveDeltaSectionKind::CoveragePatch,
            "COVEDELTA coverage patch",
            &[DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH],
            None,
        )
    }

    pub fn validate_layout_hints(&self) -> Result<Vec<DeltaSidecarHintV1>, CoveError> {
        validate_sidecar_hints_for_section(
            self,
            CoveDeltaSectionKind::LayoutHints,
            "COVEDELTA layout hints",
            &[DELTA_SIDECAR_HINT_KIND_LAYOUT_HINTS],
            None,
        )
    }
}

type TemporalRecordIdentityKey = (u16, [u8; 16], u32, u64, [u8; 16], [u8; 16]);

fn validate_object_delta_parent_refs(
    parent_refs: &[DeltaParentRefV1],
) -> Result<BTreeSet<u32>, CoveError> {
    let mut parent_ref_ids = BTreeSet::new();
    for parent in parent_refs {
        if !parent_ref_ids.insert(parent.parent_ref) {
            return Err(CoveError::BadSection(
                "COVEDELTA duplicate parent_ref".into(),
            ));
        }
        parent.validate_object_delta_binding()?;
    }
    Ok(parent_ref_ids)
}

fn validate_single_scope_payloads(
    header: &CoveDeltaHeaderV1,
    continuation_anchors: &[DeltaContinuationAnchorV1],
    sparse_patch_records: &[DeltaSparsePatchRecordV1],
    touched_object_ranges: &[DeltaTouchedObjectRangeV1],
    tombstone_object_ranges: &[DeltaTouchedObjectRangeV1],
) -> Result<(), CoveError> {
    if header.flags & DELTA_FLAG_SINGLE_SCOPE == 0 {
        return Ok(());
    }
    for anchor in continuation_anchors {
        if anchor.scope_kind != header.scope_kind || anchor.scope_id != header.scope_id {
            return Err(CoveError::BadSection(
                "COVEDELTA single-scope continuation anchor scope does not match header".into(),
            ));
        }
    }
    for record in sparse_patch_records {
        if record.scope_kind != header.scope_kind || record.scope_id != header.scope_id {
            return Err(CoveError::BadSection(
                "COVEDELTA single-scope sparse patch scope does not match header".into(),
            ));
        }
    }
    for range in touched_object_ranges {
        if range.scope_kind != header.scope_kind || range.scope_id != header.scope_id {
            return Err(CoveError::BadSection(
                "COVEDELTA single-scope touched range scope does not match header".into(),
            ));
        }
    }
    for range in tombstone_object_ranges {
        if range.scope_kind != header.scope_kind || range.scope_id != header.scope_id {
            return Err(CoveError::BadSection(
                "COVEDELTA single-scope tombstone range scope does not match header".into(),
            ));
        }
    }
    Ok(())
}

fn validate_dense_scope_descriptors(
    descriptors: &[DeltaScopeDescriptorV1],
) -> Result<(), CoveError> {
    for (index, descriptor) in descriptors.iter().enumerate() {
        let expected = u32::try_from(index).map_err(|_| CoveError::ArithOverflow)?;
        if descriptor.scope_ref != expected {
            return Err(CoveError::BadSection(
                "COVEDELTA scope descriptors must be dense zero-based by scope_ref".into(),
            ));
        }
    }
    Ok(())
}

fn validate_dense_summary_descriptors(
    label: &str,
    descriptors: &[DeltaSummaryDescriptorV1],
) -> Result<(), CoveError> {
    for (index, descriptor) in descriptors.iter().enumerate() {
        let expected = u32::try_from(index).map_err(|_| CoveError::ArithOverflow)?;
        if descriptor.summary_ref != expected {
            return Err(CoveError::BadSection(format!(
                "{label} must be dense zero-based by summary_ref"
            )));
        }
    }
    Ok(())
}

fn validate_dense_inline_value_refs(values: &[DeltaInlineValueV1]) -> Result<(), CoveError> {
    for (index, value) in values.iter().enumerate() {
        let expected = u32::try_from(index).map_err(|_| CoveError::ArithOverflow)?;
        if value.value_ref != expected {
            return Err(CoveError::BadSection(
                "COVEDELTA inline values must be dense zero-based by value_ref".into(),
            ));
        }
    }
    Ok(())
}

fn validate_dictionary_overlay_inline_value_refs(
    entries: &[DeltaDictionaryEntryV1],
    values: &[DeltaInlineValueV1],
) -> Result<(), CoveError> {
    for entry in entries {
        if entry.entry_kind != DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE {
            continue;
        }
        let value = inline_value_by_ref(values, entry.inline_value_ref)?;
        let logical_type = CoveLogicalType::from_u16(entry.logical_type).ok_or_else(|| {
            CoveError::BadSection(
                "COVEDELTA inline dictionary entry has unknown logical_type".into(),
            )
        })?;
        let value_tag = ValueTag::from_u16(value.value_tag).ok_or_else(|| {
            CoveError::BadSection("COVEDELTA inline value has unknown value_tag".into())
        })?;
        if !logical_type_accepts_value_tag(logical_type, value_tag) {
            return Err(CoveError::BadSection(
                "COVEDELTA inline dictionary value tag is incompatible with entry logical type"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn validate_sparse_patch_value_refs(
    records: &[DeltaSparsePatchRecordV1],
    values: &[DeltaInlineValueV1],
) -> Result<(), CoveError> {
    for record in records {
        for property in &record.changed_properties {
            if property.property_op == DELTA_PROPERTY_OP_SET_VALUE {
                inline_value_by_ref(values, property.value_ref)?;
            }
        }
    }
    Ok(())
}

fn inline_value_by_ref(
    values: &[DeltaInlineValueV1],
    value_ref: u32,
) -> Result<&DeltaInlineValueV1, CoveError> {
    let index = usize::try_from(value_ref).map_err(|_| CoveError::ArithOverflow)?;
    values.get(index).ok_or_else(|| {
        CoveError::BadSection("COVEDELTA value_ref does not resolve to an inline value".into())
    })
}

fn logical_type_accepts_value_tag(logical_type: CoveLogicalType, value_tag: ValueTag) -> bool {
    match logical_type {
        CoveLogicalType::Null => value_tag == ValueTag::Null,
        CoveLogicalType::Bool => matches!(value_tag, ValueTag::BoolFalse | ValueTag::BoolTrue),
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => value_tag == ValueTag::Int64,
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => value_tag == ValueTag::UInt64,
        CoveLogicalType::Float32 => value_tag == ValueTag::Float32Bits,
        CoveLogicalType::Float64 => value_tag == ValueTag::Float64Bits,
        CoveLogicalType::Decimal64 => value_tag == ValueTag::Decimal64,
        CoveLogicalType::Decimal128 => value_tag == ValueTag::Decimal128,
        CoveLogicalType::DateDays => value_tag == ValueTag::DateDays,
        CoveLogicalType::TimestampMicros => value_tag == ValueTag::TimestampMicros,
        CoveLogicalType::TimestampNanos => value_tag == ValueTag::TimestampNanos,
        CoveLogicalType::Utf8 => value_tag == ValueTag::Utf8,
        CoveLogicalType::Binary => value_tag == ValueTag::Binary,
        CoveLogicalType::Uuid => value_tag == ValueTag::Uuid,
        CoveLogicalType::Json => value_tag == ValueTag::Json,
        CoveLogicalType::List => value_tag == ValueTag::List,
        CoveLogicalType::Struct => value_tag == ValueTag::Struct,
        CoveLogicalType::Map => value_tag == ValueTag::Map,
    }
}

fn validate_property_bitmap_refs(
    label: &str,
    ranges: &[DeltaTouchedObjectRangeV1],
    descriptors: &[DeltaSummaryDescriptorV1],
) -> Result<(), CoveError> {
    for range in ranges {
        if range.property_bitmap_ref == DELTA_REF_NONE {
            continue;
        }
        let index = usize::try_from(range.property_bitmap_ref).map_err(|_| {
            CoveError::BadSection(format!("{label} exceeds descriptor table index width"))
        })?;
        let descriptor = descriptors.get(index).ok_or_else(|| {
            CoveError::BadSection(format!("{label} does not resolve to a summary descriptor"))
        })?;
        if descriptor.summary_kind != DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP {
            return Err(CoveError::BadSection(format!(
                "{label} must reference a property bitmap descriptor"
            )));
        }
    }
    Ok(())
}

fn validate_sidecar_hints_for_section(
    delta: &CoveDeltaFile,
    section_kind: CoveDeltaSectionKind,
    label: &str,
    allowed_hint_kinds: &[u16],
    required_feature_binding: Option<u64>,
) -> Result<Vec<DeltaSidecarHintV1>, CoveError> {
    let parent_ref_ids = validate_object_delta_parent_refs(&delta.parent_refs)?;
    let mut hints = Vec::new();
    for section in &delta.sections {
        let Some(kind) = CoveDeltaSectionKind::from_u16(section.entry.section_kind) else {
            return Err(CoveError::BadSection(format!(
                "unknown COVEDELTA section kind {}",
                section.entry.section_kind
            )));
        };
        if kind != section_kind {
            continue;
        }
        validate_delta_required_features(
            section.entry.required_delta_features,
            COVEDELTA_OBJECT_TEMPORAL_SUPPORTED_REQUIRED_FEATURES,
        )?;
        if let Some(feature) = required_feature_binding {
            if section.entry.required_delta_features & feature == 0 {
                return Err(CoveError::BadSection(format!(
                    "{label} required feature requires section feature binding"
                )));
            }
        }
        if section.entry.compression != 0 {
            return Err(CoveError::BadSection(format!(
                "{label} validation requires uncompressed payload"
            )));
        }
        if section.entry.item_count == 0 {
            return Err(CoveError::BadSection(format!(
                "{label} section requires hint records"
            )));
        }
        let records = parse_fixed_records(
            section,
            DELTA_SIDECAR_HINT_LEN,
            DeltaSidecarHintV1::parse,
            label,
        )?;
        for hint in &records {
            if !allowed_hint_kinds.contains(&hint.hint_kind) {
                return Err(CoveError::BadSection(format!(
                    "{label} section contains incompatible hint_kind"
                )));
            }
            if !parent_ref_ids.contains(&hint.parent_ref) {
                return Err(CoveError::BadSection(format!(
                    "{label} references unknown parent_ref"
                )));
            }
            if delta.parent_refs.iter().any(|parent| {
                parent.parent_ref == hint.parent_ref
                    && parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0
            }) {
                return Err(CoveError::BadSection(format!(
                    "{label} must reference an ancillary parent artifact"
                )));
            }
        }
        hints.extend(records);
    }
    validate_dense_sidecar_hint_refs(label, &hints)?;
    Ok(hints)
}

fn validate_dense_sidecar_hint_refs(
    label: &str,
    hints: &[DeltaSidecarHintV1],
) -> Result<(), CoveError> {
    for (index, hint) in hints.iter().enumerate() {
        let expected = u32::try_from(index).map_err(|_| CoveError::ArithOverflow)?;
        if hint.hint_ref != expected {
            return Err(CoveError::BadSection(format!(
                "{label} must be dense zero-based by hint_ref"
            )));
        }
    }
    Ok(())
}

fn validate_unique_temporal_record_ids(
    header: &CoveDeltaHeaderV1,
    temporal_segments: &[TemporalSegmentData],
) -> Result<(), CoveError> {
    let mut seen = BTreeSet::<TemporalRecordIdentityKey>::new();
    for segment in temporal_segments {
        for row in &segment.rows {
            let key = (
                header.scope_kind,
                header.scope_id,
                segment.header.object_type_id,
                row.branch_key,
                row.goid,
                row.record_id,
            );
            if !seen.insert(key) {
                return Err(CoveError::BadSection(
                    "COVEDELTA temporal rows contain duplicate record_id for object/branch".into(),
                ));
            }
        }
    }
    Ok(())
}

fn checkpoint_row_count(temporal_segments: &[TemporalSegmentData]) -> usize {
    temporal_segments
        .iter()
        .flat_map(|segment| segment.rows.iter())
        .filter(|row| matches!(row.record_kind, RecordKind::Snapshot | RecordKind::Baseline))
        .count()
}

fn validate_continuation_anchor_state_hash_refs(
    continuation_anchors: &[DeltaContinuationAnchorV1],
    state_hash_descriptors: &[DeltaStateHashDescriptorV1],
) -> Result<(), CoveError> {
    for anchor in continuation_anchors {
        if anchor.anchor_strength < DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH {
            continue;
        }
        let index = usize::try_from(anchor.predecessor_state_hash_ref)
            .map_err(|_| CoveError::ArithOverflow)?;
        let descriptor = state_hash_descriptors.get(index).ok_or_else(|| {
            CoveError::BadSection(
                "COVEDELTA continuation anchor predecessor_state_hash_ref does not resolve".into(),
            )
        })?;
        descriptor.validate_cove_object_delta_state_hash()?;
    }
    Ok(())
}

type SparsePatchTemporalKey = (u16, [u8; 16], u32, u32, [u8; 16], [u8; 16], i64, u64);

fn validate_sparse_patch_records_cover_temporal_rows(
    header: &CoveDeltaHeaderV1,
    temporal_segments: &[TemporalSegmentData],
    sparse_patch_records: &[DeltaSparsePatchRecordV1],
) -> Result<(), CoveError> {
    let mut temporal_delta_keys = BTreeSet::new();
    for segment in temporal_segments {
        for row in &segment.rows {
            if row.record_kind != RecordKind::Delta {
                continue;
            }
            let branch_identity_ref = u32::try_from(row.branch_key).map_err(|_| {
                CoveError::BadSection(
                    "COVEDELTA temporal row branch_key exceeds sparse patch ref width".into(),
                )
            })?;
            temporal_delta_keys.insert((
                header.scope_kind,
                header.scope_id,
                segment.header.object_type_id,
                branch_identity_ref,
                row.goid,
                row.record_id,
                row.timestamp_us,
                row.csn,
            ));
        }
    }
    let mut sparse_keys = BTreeSet::new();
    for record in sparse_patch_records {
        let key = sparse_patch_record_key(record);
        if !sparse_keys.insert(key) {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse property ops contain duplicate record keys".into(),
            ));
        }
        if !temporal_delta_keys.contains(&key) {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse property ops reference no matching temporal delta row".into(),
            ));
        }
    }
    for key in temporal_delta_keys {
        if !sparse_keys.contains(&key) {
            return Err(CoveError::BadSection(
                "COVEDELTA sparse property ops under-include temporal delta rows".into(),
            ));
        }
    }
    Ok(())
}

fn sparse_patch_record_key(record: &DeltaSparsePatchRecordV1) -> SparsePatchTemporalKey {
    (
        record.scope_kind,
        record.scope_id,
        record.object_type_id,
        record.branch_identity_ref,
        record.goid,
        record.record_id,
        record.timestamp_us,
        record.csn,
    )
}

fn validate_continuation_anchors_cover_temporal_rows(
    header: &CoveDeltaHeaderV1,
    temporal_segments: &[TemporalSegmentData],
    continuation_anchors: &[DeltaContinuationAnchorV1],
) -> Result<(), CoveError> {
    for segment in temporal_segments {
        for row in &segment.rows {
            if !matches!(row.record_kind, RecordKind::Delta | RecordKind::Tombstone) {
                continue;
            }
            let covered = continuation_anchors.iter().any(|anchor| {
                anchor.validate_for_existing_object_patch().is_ok()
                    && anchor.scope_kind == header.scope_kind
                    && anchor.scope_id == header.scope_id
                    && anchor.object_type_id == segment.header.object_type_id
                    && u64::from(anchor.branch_identity_ref) == row.branch_key
                    && anchor.goid == row.goid
                    && anchor.predecessor_csn < row.csn
                    && anchor.predecessor_timestamp_us <= row.timestamp_us
            });
            if !covered {
                return Err(CoveError::BadSection(
                    "COVEDELTA continuation anchors under-include temporal patch rows".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_touched_ranges_cover_temporal_rows(
    header: &CoveDeltaHeaderV1,
    temporal_segments: &[TemporalSegmentData],
    touched_object_ranges: &[DeltaTouchedObjectRangeV1],
) -> Result<(), CoveError> {
    for segment in temporal_segments {
        for row in &segment.rows {
            let covered = row_is_covered_by_ranges(header, segment, row, touched_object_ranges);
            if !covered {
                return Err(CoveError::BadSection(
                    "COVEDELTA exact touched set under-includes temporal rows".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_tombstone_ranges_cover_temporal_rows(
    header: &CoveDeltaHeaderV1,
    temporal_segments: &[TemporalSegmentData],
    tombstone_object_ranges: &[DeltaTouchedObjectRangeV1],
) -> Result<(), CoveError> {
    for segment in temporal_segments {
        for row in &segment.rows {
            if row.record_kind != RecordKind::Tombstone {
                continue;
            }
            let covered = row_is_covered_by_ranges(header, segment, row, tombstone_object_ranges);
            if !covered {
                return Err(CoveError::BadSection(
                    "COVEDELTA exact tombstone set under-includes tombstone rows".into(),
                ));
            }
        }
    }
    Ok(())
}

fn row_is_covered_by_ranges(
    header: &CoveDeltaHeaderV1,
    segment: &TemporalSegmentData,
    row: &crate::profile::cove_o::TemporalRowEntryV1,
    ranges: &[DeltaTouchedObjectRangeV1],
) -> bool {
    ranges.iter().any(|range| {
        range.scope_kind == header.scope_kind
            && range.scope_id == header.scope_id
            && range.object_type_id == segment.header.object_type_id
            && u64::from(range.branch_identity_ref) == row.branch_key
            && range.min_goid <= row.goid
            && row.goid <= range.max_goid
    })
}

fn parse_fixed_records<T>(
    section: &CoveDeltaSection,
    record_len: usize,
    parse: impl Fn(&[u8]) -> Result<T, CoveError>,
    label: &str,
) -> Result<Vec<T>, CoveError> {
    if !section.payload.len().is_multiple_of(record_len) {
        return Err(CoveError::BadSection(format!(
            "{label} section payload length is not a multiple of record length"
        )));
    }
    let count = section.payload.len() / record_len;
    if section.entry.item_count != count as u64 {
        return Err(CoveError::BadSection(format!(
            "{label} section item_count does not match payload"
        )));
    }
    section
        .payload
        .chunks_exact(record_len)
        .map(parse)
        .collect()
}

fn covedelta_expected_digest_len(algorithm: DigestAlgorithm) -> usize {
    match algorithm {
        DigestAlgorithm::None => 0,
        DigestAlgorithm::Sha256 | DigestAlgorithm::Blake3 => 32,
    }
}

fn append_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CoveError> {
    let len = u32::try_from(bytes.len()).map_err(|_| CoveError::ArithOverflow)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn validate_delta_required_features(required: u64, supported: u64) -> Result<(), CoveError> {
    let unknown_required = required & !supported;
    if unknown_required != 0 {
        Err(CoveError::UnknownRequiredFeature(unknown_required))
    } else {
        Ok(())
    }
}

fn covedelta_object_delta_requires_section_features(kind: CoveDeltaSectionKind) -> bool {
    !matches!(
        kind,
        CoveDeltaSectionKind::TemporalSegmentIndex
            | CoveDeltaSectionKind::ProjectionPatch
            | CoveDeltaSectionKind::CoveragePatch
            | CoveDeltaSectionKind::IndexHints
            | CoveDeltaSectionKind::LayoutHints
    )
}

fn checked_range(
    offset: u64,
    length: u64,
    total_len: usize,
) -> Result<std::ops::Range<usize>, CoveError> {
    let start = usize::try_from(offset).map_err(|_| CoveError::ArithOverflow)?;
    let len = usize::try_from(length).map_err(|_| CoveError::ArithOverflow)?;
    let end = start.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > total_len {
        return Err(CoveError::BufferTooShort);
    }
    Ok(start..end)
}

fn put(buf: &mut [u8], pos: &mut usize, bytes: &[u8]) {
    let end = *pos + bytes.len();
    buf[*pos..end].copy_from_slice(bytes);
    *pos = end;
}

fn put_u8(buf: &mut [u8], pos: &mut usize, value: u8) {
    buf[*pos] = value;
    *pos += 1;
}

fn put_u16(buf: &mut [u8], pos: &mut usize, value: u16) {
    put(buf, pos, &value.to_le_bytes());
}

fn put_u32(buf: &mut [u8], pos: &mut usize, value: u32) {
    put(buf, pos, &value.to_le_bytes());
}

fn put_u64(buf: &mut [u8], pos: &mut usize, value: u64) {
    put(buf, pos, &value.to_le_bytes());
}

fn put_i64(buf: &mut [u8], pos: &mut usize, value: i64) {
    put(buf, pos, &value.to_le_bytes());
}

fn take<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], CoveError> {
    let end = pos.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let out = &bytes[*pos..end];
    *pos = end;
    Ok(out)
}

fn take_array<const N: usize>(bytes: &[u8], pos: &mut usize) -> Result<[u8; N], CoveError> {
    let mut out = [0u8; N];
    out.copy_from_slice(take(bytes, pos, N)?);
    Ok(out)
}

fn take_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, CoveError> {
    Ok(take(bytes, pos, 1)?[0])
}

fn take_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, CoveError> {
    Ok(u16::from_le_bytes(take_array::<2>(bytes, pos)?))
}

fn take_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, CoveError> {
    Ok(u32::from_le_bytes(take_array::<4>(bytes, pos)?))
}

fn take_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, CoveError> {
    Ok(u64::from_le_bytes(take_array::<8>(bytes, pos)?))
}

fn take_i64(bytes: &[u8], pos: &mut usize) -> Result<i64, CoveError> {
    Ok(i64::from_le_bytes(take_array::<8>(bytes, pos)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{CoveLogicalType, CovePhysicalKind};
    use crate::profile::cove_o::{
        ObjectTypeEntryV1, PropertyEntryV1, RecordKind, TemporalRowEntryV1,
        TemporalSegmentHeaderV1, TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
    };

    fn minimal_delta() -> CoveDeltaFile {
        CoveDeltaFile {
            header: CoveDeltaHeaderV1::new([1; 16], [2; 16], [3; 16], [4; 16]),
            parent_refs: vec![DeltaParentRefV1 {
                parent_ref: 0,
                parent_kind: 0,
                flags: DELTA_PARENT_REF_LINEAGE_PARENT,
                artifact_id: [9; 16],
                snapshot_id: [4; 16],
                file_len: 1024,
                footer_crc32c: 7,
                digest_algorithm: 1,
                digest_len: 32,
                digest_ref: 0,
                uri_ref: 1,
                schema_fingerprint_ref: 0,
                object_catalog_fingerprint_ref: 0,
                semantic_map_fingerprint_ref: 0,
                projection_fingerprint_ref: 0,
                checksum: 0,
            }],
            sections: vec![CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 1,
                    section_kind: CoveDeltaSectionKind::TemporalSegmentData as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: 0,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: b"delta-payload".to_vec(),
            }],
            footer: CoveDeltaFooterV1 {
                header_offset: 0,
                header_length: COVEDELTA_HEADER_LEN as u64,
                section_directory_offset: 0,
                section_directory_length: 0,
                section_count: 0,
                parent_ref_count: 0,
                footer_crc32c: 0,
                checksum: 0,
            },
            postscript: CoveDeltaPostscriptV1 {
                required_delta_features: 0,
                optional_delta_features: 0,
                file_len: 0,
                footer_offset: 0,
                footer_length: COVEDELTA_FOOTER_LEN as u64,
                checksum: 0,
            },
        }
    }

    fn object_delta() -> CoveDeltaFile {
        let mut delta = minimal_delta();
        delta.header.csn_min = 1;
        delta.header.csn_max = 1;
        delta.header.commit_time_range_start_us = 10;
        delta.header.commit_time_range_end_us = 10;
        delta.sections[0].payload = temporal_segment_payload();
        delta
    }

    fn object_delta_with_sparse_patch_section() -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_SPARSE_PATCH_ROWS;
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::PropertyOps as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_SPARSE_PATCH_ROWS,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_sparse_patch_record().serialize().unwrap(),
        });
        push_string_table_section(&mut delta, &[0]);
        delta
    }

    fn object_delta_with_dictionary_overlay_entries(
        entries: &[DeltaDictionaryEntryV1],
    ) -> CoveDeltaFile {
        object_delta_with_dictionary_overlay_entries_and_features(
            entries,
            dictionary_overlay_required_features(entries),
        )
    }

    fn object_delta_with_dictionary_overlay_entries_and_features(
        entries: &[DeltaDictionaryEntryV1],
        required_delta_features: u64,
    ) -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.header.required_delta_features = required_delta_features;
        let mut payload = Vec::new();
        for entry in entries {
            payload.extend_from_slice(&entry.serialize().unwrap());
        }
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::DictionaryOverlay as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: entries.len() as u64,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload,
        });
        let inline_value_refs = entries
            .iter()
            .filter_map(|entry| {
                (entry.entry_kind == DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE)
                    .then_some(entry.inline_value_ref)
            })
            .collect::<Vec<_>>();
        if !inline_value_refs.is_empty() {
            push_string_table_section(&mut delta, &inline_value_refs);
        }
        delta
    }

    fn push_string_table_section(delta: &mut CoveDeltaFile, value_refs: &[u32]) {
        let max_ref = value_refs.iter().copied().max().unwrap_or(0);
        let values = (0..=max_ref).map(sample_inline_value).collect::<Vec<_>>();
        let payload = values
            .iter()
            .map(DeltaInlineValueV1::serialize)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: delta.sections.len() as u32 + 1,
                section_kind: CoveDeltaSectionKind::StringTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: values.len() as u64,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload,
        });
    }

    fn sample_inline_value(value_ref: u32) -> DeltaInlineValueV1 {
        DeltaInlineValueV1 {
            value_ref,
            value_tag: ValueTag::Utf8 as u16,
            flags: 0,
            value: vec![1, b'x'],
            checksum: 0,
        }
    }

    fn dictionary_overlay_required_features(entries: &[DeltaDictionaryEntryV1]) -> u64 {
        let mut required = 0;
        for entry in entries {
            match entry.entry_kind {
                DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE => {
                    required |= DELTA_FEATURE_INLINE_DICTIONARY;
                }
                DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS => {
                    required |= DELTA_FEATURE_PARENT_DICTIONARY_ALIASES;
                }
                _ => {}
            }
        }
        required
    }

    fn object_delta_with_descriptor_table_sections() -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::ScopeTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_scope_descriptor().serialize().unwrap().to_vec(),
        });
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 3,
                section_kind: CoveDeltaSectionKind::TemporalRoleSummaryTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE)
                .serialize()
                .unwrap()
                .to_vec(),
        });
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 4,
                section_kind: CoveDeltaSectionKind::TouchedSummaryTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET)
                .serialize()
                .unwrap()
                .to_vec(),
        });
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 5,
                section_kind: CoveDeltaSectionKind::TombstoneSummaryTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET)
                .serialize()
                .unwrap()
                .to_vec(),
        });
        delta
    }

    fn object_delta_with_extra_section(
        section_kind: CoveDeltaSectionKind,
        required_delta_features: u64,
        payload: Vec<u8>,
    ) -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: section_kind as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload,
        });
        delta
    }

    fn object_delta_with_catalog_patch_section(patch: ObjectTypeCatalog) -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::CatalogPatch as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: patch.types.len() as u64,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: patch.serialize().unwrap(),
        });
        delta
    }

    fn object_delta_with_projection_patch_section(payload: Vec<u8>) -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_PROJECTION_PATCH;
        delta.header.projection_fingerprint_ref = 41;
        delta.sections.push(delta_projection_patch_section(
            2,
            payload,
            DELTA_FEATURE_PROJECTION_PATCH,
        ));
        delta
    }

    fn delta_projection_patch_section(
        section_id: u32,
        payload: Vec<u8>,
        required_delta_features: u64,
    ) -> CoveDeltaSection {
        CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id,
                section_kind: CoveDeltaSectionKind::ProjectionPatch as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload,
        }
    }

    fn projection_patch_payload() -> Vec<u8> {
        br#"{"schema_id":"org.coveformat.covemap.v2","section_id":68,"mapping_id":"crm-map","mapping_version":"v1","projections":[{"projection_id":"delta_projection","output_table":"delta_projection","row_grain":"one_row_per_object","anchor":{"object_type":"Company"},"temporal_mode":{"as_of":"latest_committed"},"multi_value_policy":"first","columns":[{"name":"company_name","value":"name","logical_type":"utf8"}],"output_modes":["json"]}]}"#.to_vec()
    }

    fn sample_sidecar_parent_ref(parent_ref: u32) -> DeltaParentRefV1 {
        DeltaParentRefV1 {
            parent_ref,
            parent_kind: 0,
            flags: 0,
            artifact_id: [0xA0; 16],
            snapshot_id: [0xA1; 16],
            file_len: 2048,
            footer_crc32c: 8,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest_len: 32,
            digest_ref: parent_ref + 2,
            uri_ref: parent_ref + 3,
            schema_fingerprint_ref: 0,
            object_catalog_fingerprint_ref: 0,
            semantic_map_fingerprint_ref: 0,
            projection_fingerprint_ref: 0,
            checksum: 0,
        }
    }

    fn sample_sidecar_hint(parent_ref: u32, hint_kind: u16) -> DeltaSidecarHintV1 {
        DeltaSidecarHintV1 {
            hint_ref: 0,
            hint_kind,
            flags: 0,
            parent_ref,
            target_section_id: 7,
            scope_ref: DELTA_REF_NONE,
            object_type_id: 1,
            chain_digest_ref: 5,
            checksum: 0,
        }
    }

    fn delta_sidecar_hint_section(
        section_id: u32,
        section_kind: CoveDeltaSectionKind,
        required_delta_features: u64,
        hint: DeltaSidecarHintV1,
    ) -> CoveDeltaSection {
        CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id,
                section_kind: section_kind as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: hint.serialize().to_vec(),
        }
    }

    fn object_delta_with_sidecar_hint_section(
        section_kind: CoveDeltaSectionKind,
        required_delta_features: u64,
        hint_kind: u16,
    ) -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.header.required_delta_features = required_delta_features;
        delta.parent_refs.push(sample_sidecar_parent_ref(1));
        delta.sections.push(delta_sidecar_hint_section(
            2,
            section_kind,
            required_delta_features,
            sample_sidecar_hint(1, hint_kind),
        ));
        delta
    }

    fn sample_catalog_patch() -> ObjectTypeCatalog {
        ObjectTypeCatalog {
            flags: 0,
            types: vec![ObjectTypeEntryV1 {
                object_type_id: 2,
                type_name: "Widget".into(),
                flags: 0,
                properties: vec![PropertyEntryV1 {
                    property_id: 1,
                    property_name: "name".into(),
                    logical_type: CoveLogicalType::Utf8,
                    physical_kind: CovePhysicalKind::VarBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                }],
            }],
        }
    }

    fn object_delta_with_anchor_and_touched_sections() -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.header.required_delta_features =
            DELTA_FEATURE_CONTINUATION_ANCHORS | DELTA_FEATURE_EXACT_TOUCHED_SET;
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::ContinuationAnchors as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_CONTINUATION_ANCHORS,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_anchor().serialize().to_vec(),
        });
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 3,
                section_kind: CoveDeltaSectionKind::TouchedObjectSet as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_EXACT_TOUCHED_SET,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_touched_range().serialize().to_vec(),
        });
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 4,
                section_kind: CoveDeltaSectionKind::StateHashTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_state_hash_descriptor().serialize().to_vec(),
        });
        delta
    }

    fn object_delta_with_touched_property_bitmap_section(
        descriptor_kind: Option<u8>,
    ) -> CoveDeltaFile {
        let mut delta = object_delta_with_anchor_and_touched_sections();
        let mut touched = sample_touched_range();
        touched.property_bitmap_ref = 0;
        delta.sections[2].payload = touched.serialize().to_vec();
        if let Some(summary_kind) = descriptor_kind {
            delta.sections.push(CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 5,
                    section_kind: CoveDeltaSectionKind::TouchedSummaryTable as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: 0,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: sample_summary_descriptor(summary_kind)
                    .serialize()
                    .unwrap()
                    .to_vec(),
            });
        }
        delta
    }

    fn object_delta_with_branch_identity_section() -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::BranchIdentityTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_branch_identity().serialize().to_vec(),
        });
        delta
    }

    fn object_delta_with_tombstone_set_section() -> CoveDeltaFile {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_EXACT_TOMBSTONE_SET;
        delta.sections[0].payload = tombstone_temporal_segment_payload();
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::TombstoneSet as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_EXACT_TOMBSTONE_SET,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: sample_tombstone_range().serialize().to_vec(),
        });
        delta
    }

    fn temporal_segment_payload() -> Vec<u8> {
        temporal_segment_payload_for_row(TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: [8; 16],
            record_kind: RecordKind::Delta,
            prev_ref: None,
        })
    }

    fn tombstone_temporal_segment_payload() -> Vec<u8> {
        temporal_segment_payload_for_row(TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [9; 16],
            record_id: [10; 16],
            record_kind: RecordKind::Tombstone,
            prev_ref: None,
        })
    }

    fn checkpoint_temporal_segment_payload() -> Vec<u8> {
        temporal_segment_payload_for_row(TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: [11; 16],
            record_kind: RecordKind::Snapshot,
            prev_ref: None,
        })
    }

    fn temporal_segment_payload_for_row(row: TemporalRowEntryV1) -> Vec<u8> {
        temporal_segment_payload_for_rows(&[row])
    }

    fn temporal_segment_payload_for_rows(rows: &[TemporalRowEntryV1]) -> Vec<u8> {
        let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
        let row_end = row_directory_offset
            + (rows.len() as u64).saturating_mul(TEMPORAL_ROW_ENTRY_LEN as u64);
        let header = TemporalSegmentHeaderV1 {
            segment_id: 1,
            object_type_id: 1,
            time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
            time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
            csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
            csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
            row_count: rows.len() as u32,
            morsel_count: 1,
            morsel_row_count: rows.len() as u32,
            column_count: 0,
            row_directory_offset,
            column_directory_offset: row_end,
            page_index_offset: row_end,
            data_offset: row_end,
            flags: 0,
            checksum: 0,
        };
        let mut out = header.serialize().to_vec();
        for row in rows {
            out.extend_from_slice(&row.serialize());
        }
        out
    }

    fn sample_anchor() -> DeltaContinuationAnchorV1 {
        DeltaContinuationAnchorV1 {
            scope_kind: 0,
            scope_id: [0; 16],
            object_type_id: 1,
            branch_identity_ref: 0,
            goid: [7; 16],
            parent_ref: 0,
            predecessor_csn: 0,
            predecessor_timestamp_us: 0,
            predecessor_record_id: [3; 16],
            predecessor_state_hash_ref: 0,
            predecessor_trust_hash_ref: DELTA_REF_NONE,
            anchor_strength: DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH,
            flags: 0,
            checksum: 0,
        }
    }

    fn sample_state_hash_descriptor() -> DeltaStateHashDescriptorV1 {
        DeltaStateHashDescriptorV1 {
            state_hash_ref: 0,
            state_hash_kind: DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1,
            hash_algorithm: DigestAlgorithm::Sha256 as u16,
            hash_len: 32,
            hash_payload_ref: 0,
            flags: 0,
            checksum: 0,
        }
    }

    fn sample_state_hash_material() -> CoveObjectDeltaStateHashV1 {
        CoveObjectDeltaStateHashV1 {
            scope_kind: 0,
            scope_id: [0; 16],
            canonical_branch_identity: b"main".to_vec(),
            object_type_id: 1,
            goid: [7; 16],
            predecessor_record_id: [3; 16],
            predecessor_csn: 0,
            predecessor_timestamp_us: 0,
            record_kind: RecordKind::Delta,
            tombstone_state: DELTA_OBJECT_STATE_TOMBSTONE_LIVE,
            properties: vec![
                CoveObjectDeltaStateHashPropertyV1 {
                    property_id: 1,
                    logical_type: 1,
                    collation_id: 0,
                    value_state: DELTA_OBJECT_STATE_VALUE_VISIBLE,
                    canonical_value: b"alice".to_vec(),
                    redaction_commitment: Vec::new(),
                    hidden_value_commitment: None,
                },
                CoveObjectDeltaStateHashPropertyV1 {
                    property_id: 2,
                    logical_type: 1,
                    collation_id: 0,
                    value_state: DELTA_OBJECT_STATE_VALUE_NULL,
                    canonical_value: Vec::new(),
                    redaction_commitment: Vec::new(),
                    hidden_value_commitment: None,
                },
            ],
        }
    }

    fn sample_sparse_patch_record() -> DeltaSparsePatchRecordV1 {
        DeltaSparsePatchRecordV1 {
            scope_kind: 0,
            scope_id: [0; 16],
            branch_identity_ref: 0,
            object_type_id: 1,
            goid: [7; 16],
            record_id: [8; 16],
            timestamp_us: 10,
            csn: 1,
            record_kind: RecordKind::Delta,
            flags: 0,
            changed_properties: vec![
                DeltaSparsePatchPropertyOpV1 {
                    property_id: 1,
                    property_op: DELTA_PROPERTY_OP_SET_VALUE,
                    tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                    value_ref: 0,
                    redaction_ref: DELTA_REF_NONE,
                    flags: 0,
                },
                DeltaSparsePatchPropertyOpV1 {
                    property_id: 2,
                    property_op: DELTA_PROPERTY_OP_SET_NULL,
                    tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                    value_ref: DELTA_REF_NONE,
                    redaction_ref: DELTA_REF_NONE,
                    flags: 0,
                },
                DeltaSparsePatchPropertyOpV1 {
                    property_id: 3,
                    property_op: DELTA_PROPERTY_OP_CLEAR,
                    tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                    value_ref: DELTA_REF_NONE,
                    redaction_ref: DELTA_REF_NONE,
                    flags: 0,
                },
                DeltaSparsePatchPropertyOpV1 {
                    property_id: 4,
                    property_op: DELTA_PROPERTY_OP_TOMBSTONE,
                    tombstone_kind: DELTA_TOMBSTONE_KIND_PROPERTY,
                    value_ref: DELTA_REF_NONE,
                    redaction_ref: DELTA_REF_NONE,
                    flags: 0,
                },
                DeltaSparsePatchPropertyOpV1 {
                    property_id: 5,
                    property_op: DELTA_PROPERTY_OP_REDACT,
                    tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                    value_ref: DELTA_REF_NONE,
                    redaction_ref: 12,
                    flags: 0,
                },
            ],
            checksum: 0,
        }
    }

    fn sample_dictionary_entry() -> DeltaDictionaryEntryV1 {
        DeltaDictionaryEntryV1 {
            local_dictionary_id: 1,
            local_code: 7,
            logical_type: CoveLogicalType::Utf8 as u16,
            collation_id: 0,
            entry_kind: DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE,
            flags: 0,
            inline_value_ref: 0,
            parent_ref: DELTA_REF_NONE,
            parent_dictionary_id: 0,
            parent_code: 0,
            parent_dictionary_digest_ref: DELTA_REF_NONE,
            canonical_hash128: [0; 16],
            checksum: 0,
        }
    }

    fn sample_parent_alias_dictionary_entry() -> DeltaDictionaryEntryV1 {
        DeltaDictionaryEntryV1 {
            local_dictionary_id: 1,
            local_code: 8,
            logical_type: CoveLogicalType::Utf8 as u16,
            collation_id: 0,
            entry_kind: DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS,
            flags: 0,
            inline_value_ref: DELTA_REF_NONE,
            parent_ref: 0,
            parent_dictionary_id: 1,
            parent_code: 2,
            parent_dictionary_digest_ref: 3,
            canonical_hash128: [9; 16],
            checksum: 0,
        }
    }

    fn sample_canonical_hash_hint_dictionary_entry() -> DeltaDictionaryEntryV1 {
        DeltaDictionaryEntryV1 {
            local_dictionary_id: 2,
            local_code: 9,
            logical_type: CoveLogicalType::Utf8 as u16,
            collation_id: 0,
            entry_kind: DELTA_DICTIONARY_ENTRY_KIND_CANONICAL_HASH_HINT,
            flags: 0,
            inline_value_ref: DELTA_REF_NONE,
            parent_ref: DELTA_REF_NONE,
            parent_dictionary_id: 0,
            parent_code: 0,
            parent_dictionary_digest_ref: DELTA_REF_NONE,
            canonical_hash128: [0x5A; 16],
            checksum: 0,
        }
    }

    fn sample_scope_descriptor() -> DeltaScopeDescriptorV1 {
        DeltaScopeDescriptorV1 {
            scope_ref: 0,
            scope_kind: 0,
            flags: 0,
            scope_id: [0; 16],
            checksum: 0,
        }
    }

    fn sample_summary_descriptor(summary_kind: u8) -> DeltaSummaryDescriptorV1 {
        DeltaSummaryDescriptorV1 {
            summary_ref: 0,
            summary_kind,
            flags: 0,
            payload_ref: 0,
            item_count: 1,
            checksum: 0,
        }
    }

    fn sample_branch_identity() -> DeltaBranchIdentityV1 {
        DeltaBranchIdentityV1 {
            branch_identity_ref: 0,
            branch_identity_kind: DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF,
            flags: 0,
            branch_value_ref: 11,
            branch_hash128: [5; 16],
            branch_catalog_fingerprint_ref: 0,
            checksum: 0,
        }
    }

    fn sample_touched_range() -> DeltaTouchedObjectRangeV1 {
        DeltaTouchedObjectRangeV1 {
            scope_kind: 0,
            scope_id: [0; 16],
            object_type_id: 1,
            branch_identity_ref: 0,
            min_goid: [7; 16],
            max_goid: [7; 16],
            touched_count: 1,
            property_bitmap_ref: DELTA_REF_NONE,
            object_set_ref: 0,
            checksum: 0,
        }
    }

    fn sample_tombstone_range() -> DeltaTouchedObjectRangeV1 {
        DeltaTouchedObjectRangeV1 {
            scope_kind: 0,
            scope_id: [0; 16],
            object_type_id: 1,
            branch_identity_ref: 0,
            min_goid: [9; 16],
            max_goid: [9; 16],
            touched_count: 1,
            property_bitmap_ref: DELTA_REF_NONE,
            object_set_ref: 0,
            checksum: 0,
        }
    }

    fn sample_object_point(goid: [u8; 16]) -> DeltaObjectPointLookupV1 {
        DeltaObjectPointLookupV1 {
            scope_kind: 0,
            scope_id: [0; 16],
            object_type_id: 1,
            branch_identity_ref: 0,
            goid,
        }
    }

    #[test]
    fn covedelta_round_trips_and_discovers_tail() {
        let delta = minimal_delta();
        let bytes = delta.serialize().unwrap();
        assert!(bytes.ends_with(&MAGIC_COVEDELTA));
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert_eq!(parsed.header.delta_artifact_id, [1; 16]);
        assert_eq!(parsed.parent_refs.len(), 1);
        assert_eq!(parsed.sections.len(), 1);
        assert_eq!(parsed.sections[0].payload, b"delta-payload");
        assert_eq!(
            parsed.sections[0].entry.section_kind,
            CoveDeltaSectionKind::TemporalSegmentData as u16
        );
    }

    #[test]
    fn covedelta_rejects_missing_lineage_parent_ref() {
        let mut delta = minimal_delta();
        delta.parent_refs.clear();
        let bytes = delta.serialize().unwrap();
        assert!(matches!(
            CoveDeltaFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("exactly one lineage parent")
        ));
    }

    #[test]
    fn covedelta_rejects_duplicate_lineage_parent_refs() {
        let mut delta = minimal_delta();
        let mut duplicate_parent = delta.parent_refs[0].clone();
        duplicate_parent.parent_ref = 1;
        duplicate_parent.artifact_id = [10; 16];
        delta.parent_refs.push(duplicate_parent);
        let bytes = delta.serialize().unwrap();
        assert!(matches!(
            CoveDeltaFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("exactly one lineage parent")
        ));
    }

    #[test]
    fn covedelta_rejects_corrupt_tail_and_section_payload() {
        let mut bytes = minimal_delta().serialize().unwrap();
        *bytes.last_mut().unwrap() = b'X';
        assert!(matches!(
            CoveDeltaFile::parse(&bytes),
            Err(CoveError::BadMagic)
        ));

        let mut bytes = minimal_delta().serialize().unwrap();
        let header = CoveDeltaHeaderV1::parse(&bytes[..COVEDELTA_HEADER_LEN as usize]).unwrap();
        bytes[header.parent_refs_offset as usize + COVEDELTA_PARENT_REF_LEN] ^= 0xff;
        assert!(matches!(
            CoveDeltaFile::parse(&bytes),
            Err(CoveError::ChecksumMismatch)
        ));
    }

    #[test]
    fn covedelta_rejects_unflagged_source_publish_range() {
        let mut header = CoveDeltaHeaderV1::new([1; 16], [2; 16], [3; 16], [4; 16]);
        header.source_publish_range_start_us = 10;
        header.source_publish_range_end_us = 20;
        let bytes = header.serialize();
        assert!(CoveDeltaHeaderV1::parse(&bytes).is_err());
    }

    #[test]
    fn covedelta_object_delta_validates_temporal_segments() {
        let bytes = object_delta().serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let segments = parsed.validate_object_delta_sections().unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].header.segment_id, 1);
        assert_eq!(segments[0].rows.len(), 1);
    }

    #[test]
    fn covedelta_parse_rejects_overlapping_section_payload_region() {
        let mut bytes = object_delta().serialize().unwrap();
        rewrite_first_section_payload_offset(&mut bytes, 0);

        assert!(matches!(
            CoveDeltaFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("payload regions must be canonical")
        ));
    }

    fn rewrite_first_section_payload_offset(bytes: &mut [u8], offset: u64) {
        let parsed = CoveDeltaFile::parse(bytes).unwrap();
        let directory_offset = parsed.header.section_directory_offset as usize;
        let entry_range =
            directory_offset..directory_offset + COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN;
        let mut entry =
            CoveDeltaSectionDirectoryEntryV1::parse(&bytes[entry_range.clone()]).unwrap();
        let start = offset as usize;
        let end = start + entry.length as usize;
        entry.offset = offset;
        entry.crc32c = checksum::crc32c(&bytes[start..end]);
        bytes[entry_range].copy_from_slice(&entry.serialize());
    }

    #[test]
    fn covedelta_object_delta_rejects_duplicate_record_id_for_object_branch() {
        let mut delta = object_delta();
        delta.header.csn_max = 2;
        delta.header.commit_time_range_end_us = 20;
        let duplicate_record_id = [0xAB; 16];
        delta.sections[0].payload = temporal_segment_payload_for_rows(&[
            TemporalRowEntryV1 {
                timestamp_us: 10,
                csn: 1,
                branch_key: 0,
                goid: [7; 16],
                record_id: duplicate_record_id,
                record_kind: RecordKind::Delta,
                prev_ref: None,
            },
            TemporalRowEntryV1 {
                timestamp_us: 20,
                csn: 2,
                branch_key: 0,
                goid: [7; 16],
                record_id: duplicate_record_id,
                record_kind: RecordKind::Delta,
                prev_ref: None,
            },
        ]);

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("duplicate record_id")
        ));
    }

    #[test]
    fn covedelta_object_delta_allows_same_record_id_for_different_object() {
        let mut delta = object_delta();
        delta.header.csn_max = 2;
        delta.header.commit_time_range_end_us = 20;
        let shared_record_id = [0xAB; 16];
        delta.sections[0].payload = temporal_segment_payload_for_rows(&[
            TemporalRowEntryV1 {
                timestamp_us: 10,
                csn: 1,
                branch_key: 0,
                goid: [7; 16],
                record_id: shared_record_id,
                record_kind: RecordKind::Delta,
                prev_ref: None,
            },
            TemporalRowEntryV1 {
                timestamp_us: 20,
                csn: 2,
                branch_key: 0,
                goid: [8; 16],
                record_id: shared_record_id,
                record_kind: RecordKind::Delta,
                prev_ref: None,
            },
        ]);

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.temporal_segments[0].rows.len(), 2);
    }

    #[test]
    fn covedelta_object_delta_effective_fingerprint_refs_inherit_and_override() {
        let mut delta = object_delta();
        delta.parent_refs[0].schema_fingerprint_ref = 11;
        delta.parent_refs[0].object_catalog_fingerprint_ref = 12;
        delta.parent_refs[0].semantic_map_fingerprint_ref = 13;
        delta.parent_refs[0].projection_fingerprint_ref = 14;
        delta.header.object_catalog_fingerprint_ref = 22;
        delta.header.projection_fingerprint_ref = 24;

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.effective_schema_fingerprint_ref, 11);
        assert_eq!(validation.effective_object_catalog_fingerprint_ref, 22);
        assert_eq!(validation.effective_semantic_map_fingerprint_ref, 13);
        assert_eq!(validation.effective_projection_fingerprint_ref, 24);
    }

    #[test]
    fn covedelta_object_delta_rejects_parent_ref_without_digest_binding() {
        let mut delta = object_delta();
        delta.parent_refs[0].digest_algorithm = DigestAlgorithm::None as u16;

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("cryptographic digest_algorithm")
        ));
    }

    #[test]
    fn covedelta_object_delta_accepts_checkpoint_baseline_feature() {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
        delta.sections[0].entry.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
        delta.sections[0].payload = checkpoint_temporal_segment_payload();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.checkpoint_row_count, 1);
        assert_eq!(
            validation.temporal_segments[0].rows[0].record_kind,
            RecordKind::Snapshot
        );
    }

    #[test]
    fn covedelta_object_delta_rejects_checkpoint_feature_without_checkpoint_rows() {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
        delta.sections[0].entry.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("checkpoint-baseline feature")
        ));
    }

    #[test]
    fn covedelta_object_delta_parses_sparse_patch_ops() {
        let bytes = object_delta_with_sparse_patch_section()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.temporal_segments.len(), 1);
        assert_eq!(validation.sparse_patch_records.len(), 1);
        assert_eq!(
            validation.sparse_patch_records[0].changed_properties.len(),
            5
        );
    }

    #[test]
    fn covedelta_object_delta_accepts_inline_dictionary_overlay() {
        let entry = sample_dictionary_entry();
        let expected = DeltaDictionaryEntryV1::parse(&entry.serialize().unwrap()).unwrap();
        let bytes = object_delta_with_dictionary_overlay_entries(std::slice::from_ref(&entry))
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.dictionary_overlay_entries, vec![expected]);
    }

    #[test]
    fn covedelta_object_delta_accepts_parent_alias_dictionary_overlay() {
        let entry = sample_parent_alias_dictionary_entry();
        let expected = DeltaDictionaryEntryV1::parse(&entry.serialize().unwrap()).unwrap();

        let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.dictionary_overlay_entries, vec![expected]);
    }

    #[test]
    fn covedelta_object_delta_rejects_parent_alias_without_required_feature() {
        let entry = sample_parent_alias_dictionary_entry();

        let bytes = object_delta_with_dictionary_overlay_entries_and_features(
            &[entry],
            DELTA_FEATURE_INLINE_DICTIONARY,
        )
        .serialize()
        .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("parent dictionary alias feature")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_parent_alias_unknown_parent_ref() {
        let mut entry = sample_parent_alias_dictionary_entry();
        entry.parent_ref = 9;

        let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("unknown parent_ref")
        ));
    }

    #[test]
    fn covedelta_object_delta_accepts_canonical_hash_dictionary_hint() {
        let entry = sample_canonical_hash_hint_dictionary_entry();
        let expected = DeltaDictionaryEntryV1::parse(&entry.serialize().unwrap()).unwrap();

        let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.dictionary_overlay_entries, vec![expected]);
    }

    #[test]
    fn covedelta_object_delta_rejects_zero_canonical_hash_dictionary_hint() {
        let mut entry = sample_canonical_hash_hint_dictionary_entry();
        entry.canonical_hash128 = [0; 16];

        let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("non-zero canonical_hash128")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_required_canonical_hash_dictionary_hint() {
        let entry = sample_canonical_hash_hint_dictionary_entry();

        let bytes = object_delta_with_dictionary_overlay_entries_and_features(
            &[entry],
            DELTA_FEATURE_INLINE_DICTIONARY,
        )
        .serialize()
        .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("must not be required")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_duplicate_dictionary_overlay_local_code() {
        let first = sample_dictionary_entry();
        let mut second = sample_dictionary_entry();
        second.inline_value_ref = 1;

        let bytes = object_delta_with_dictionary_overlay_entries(&[first, second])
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("duplicate local dictionary code")
        ));
    }

    #[test]
    fn covedelta_object_delta_parses_descriptor_tables() {
        let bytes = object_delta_with_descriptor_table_sections()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.scope_descriptors.len(), 1);
        assert_eq!(validation.scope_descriptors[0].scope_ref, 0);
        assert_eq!(validation.temporal_role_summary_descriptors.len(), 1);
        assert_eq!(
            validation.temporal_role_summary_descriptors[0].summary_kind,
            DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE
        );
        assert_eq!(validation.touched_summary_descriptors.len(), 1);
        assert_eq!(validation.tombstone_summary_descriptors.len(), 1);
    }

    #[test]
    fn covedelta_object_delta_rejects_sparse_scope_descriptor_refs() {
        let mut delta = object_delta_with_descriptor_table_sections();
        let mut descriptor = sample_scope_descriptor();
        descriptor.scope_ref = 1;
        delta.sections[1].payload = descriptor.serialize().unwrap().to_vec();

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("dense zero-based by scope_ref")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_sparse_summary_descriptor_refs() {
        let mut delta = object_delta_with_descriptor_table_sections();
        let mut descriptor =
            sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET);
        descriptor.summary_ref = 1;
        delta.sections[3].payload = descriptor.serialize().unwrap().to_vec();

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("dense zero-based by summary_ref")
        ));
    }

    #[test]
    fn covedelta_object_delta_projection_property_skip_uses_sparse_ops() {
        let bytes = object_delta_with_sparse_patch_section()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        let point = sample_object_point([7; 16]);

        assert!(!validation.can_skip_delta_for_projection_properties(point, &[1]));
        assert!(!validation.can_skip_delta_for_projection_properties(point, &[2]));
        assert!(!validation.can_skip_delta_for_projection_properties(point, &[4]));
        assert!(!validation.can_skip_delta_for_projection_properties(point, &[5]));
        assert!(!validation.can_skip_delta_for_projection_properties(point, &[]));
        assert!(validation.can_skip_delta_for_projection_properties(point, &[99]));
        assert!(!validation
            .can_skip_delta_for_projection_properties(sample_object_point([8; 16]), &[99]));
    }

    #[test]
    fn covedelta_object_delta_projection_property_skip_uses_exact_touched_absence() {
        let bytes = object_delta_with_anchor_and_touched_sections()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();

        assert!(
            validation.can_skip_delta_for_projection_properties(sample_object_point([8; 16]), &[1])
        );
    }

    #[test]
    fn covedelta_object_delta_projection_property_skip_keeps_row_tombstones() {
        let mut delta = object_delta_with_sparse_patch_section();
        let mut record = sample_sparse_patch_record();
        record.changed_properties = vec![DeltaSparsePatchPropertyOpV1 {
            property_id: 1,
            property_op: DELTA_PROPERTY_OP_TOMBSTONE,
            tombstone_kind: DELTA_TOMBSTONE_KIND_OBJECT,
            value_ref: DELTA_REF_NONE,
            redaction_ref: DELTA_REF_NONE,
            flags: 0,
        }];
        delta.sections[1].payload = record.serialize().unwrap();

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();

        assert!(!validation
            .can_skip_delta_for_projection_properties(sample_object_point([7; 16]), &[99]));
    }

    #[test]
    fn covedelta_object_delta_parses_catalog_patch() {
        let patch = sample_catalog_patch();
        let bytes = object_delta_with_catalog_patch_section(patch.clone())
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.catalog_patches, vec![patch]);
    }

    #[test]
    fn covedelta_object_delta_rejects_catalog_patch_item_count_mismatch() {
        let mut delta = object_delta_with_catalog_patch_section(sample_catalog_patch());
        delta.sections[1].entry.item_count = 2;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("catalog patch item_count")
        ));
    }

    #[test]
    fn covedelta_object_delta_accepts_projection_patch() {
        let bytes = object_delta_with_projection_patch_section(projection_patch_payload())
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.projection_patches.len(), 1);
        assert_eq!(
            validation.projection_patches[0].projections[0].projection_id,
            "delta_projection"
        );
    }

    #[test]
    fn covedelta_object_delta_rejects_required_projection_patch_without_section() {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_PROJECTION_PATCH;
        delta.header.projection_fingerprint_ref = 41;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("requires projection patch section")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_projection_patch_without_fingerprint() {
        let mut delta = object_delta_with_projection_patch_section(projection_patch_payload());
        delta.header.projection_fingerprint_ref = 0;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("projection fingerprint")
        ));
    }

    #[test]
    fn covedelta_object_delta_ignores_corrupt_optional_projection_patch() {
        let mut delta = object_delta();
        delta.sections.push(delta_projection_patch_section(
            2,
            b"corrupt projection patch".to_vec(),
            0,
        ));
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert!(validation.projection_patches.is_empty());
    }

    #[test]
    fn covedelta_object_delta_accepts_required_index_hints() {
        let bytes = object_delta_with_sidecar_hint_section(
            CoveDeltaSectionKind::IndexHints,
            DELTA_FEATURE_INDEX_HINTS,
            DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
        )
        .serialize()
        .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.index_hints.len(), 1);
        assert_eq!(validation.index_hints[0].parent_ref, 1);
    }

    #[test]
    fn covedelta_object_delta_rejects_required_index_hints_without_section() {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_INDEX_HINTS;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("requires index hint section")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_index_hint_lineage_parent_ref() {
        let mut delta = object_delta_with_sidecar_hint_section(
            CoveDeltaSectionKind::IndexHints,
            DELTA_FEATURE_INDEX_HINTS,
            DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
        );
        delta.parent_refs.truncate(1);
        delta.sections[1].payload = sample_sidecar_hint(0, DELTA_SIDECAR_HINT_KIND_COVI_INDEX)
            .serialize()
            .to_vec();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("ancillary parent artifact")
        ));
    }

    #[test]
    fn covedelta_object_delta_accepts_required_coverage_patch() {
        let bytes = object_delta_with_sidecar_hint_section(
            CoveDeltaSectionKind::CoveragePatch,
            DELTA_FEATURE_COVERAGE_PATCH,
            DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH,
        )
        .serialize()
        .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.coverage_patches.len(), 1);
        assert_eq!(
            validation.coverage_patches[0].hint_kind,
            DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH
        );
    }

    #[test]
    fn covedelta_object_delta_rejects_coverage_patch_wrong_hint_kind() {
        let bytes = object_delta_with_sidecar_hint_section(
            CoveDeltaSectionKind::CoveragePatch,
            DELTA_FEATURE_COVERAGE_PATCH,
            DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
        )
        .serialize()
        .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("incompatible hint_kind")
        ));
    }

    #[test]
    fn covedelta_layout_hints_validate_on_request() {
        let mut delta = object_delta_with_sidecar_hint_section(
            CoveDeltaSectionKind::LayoutHints,
            0,
            DELTA_SIDECAR_HINT_KIND_LAYOUT_HINTS,
        );
        delta.header.required_delta_features = 0;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert_eq!(parsed.validate_layout_hints().unwrap().len(), 1);
    }

    #[test]
    fn covedelta_layout_hints_reject_wrong_hint_kind_on_request() {
        let mut delta = object_delta_with_sidecar_hint_section(
            CoveDeltaSectionKind::LayoutHints,
            0,
            DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH,
        );
        delta.header.required_delta_features = 0;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_layout_hints(),
            Err(CoveError::BadSection(message))
                if message.contains("incompatible hint_kind")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_required_sparse_patch_without_property_ops() {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_SPARSE_PATCH_ROWS;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("sparse patch feature")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_sparse_patch_without_temporal_row() {
        let mut delta = object_delta_with_sparse_patch_section();
        let mut record = sample_sparse_patch_record();
        record.goid = [8; 16];
        delta.sections[1].payload = record.serialize().unwrap();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("no matching temporal delta row")
        ));
    }

    #[test]
    fn covedelta_object_delta_parses_anchor_and_touched_sections() {
        let bytes = object_delta_with_anchor_and_touched_sections()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.temporal_segments.len(), 1);
        assert_eq!(validation.continuation_anchors.len(), 1);
        assert_eq!(validation.state_hash_descriptors.len(), 1);
        assert!(validation.has_touched_object_set_section);
        assert_eq!(validation.touched_object_ranges.len(), 1);
    }

    #[test]
    fn covedelta_object_delta_accepts_touched_property_bitmap_ref() {
        let bytes = object_delta_with_touched_property_bitmap_section(Some(
            DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP,
        ))
        .serialize()
        .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.touched_object_ranges[0].property_bitmap_ref, 0);
        assert_eq!(
            validation.touched_summary_descriptors[0].summary_kind,
            DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP
        );
    }

    #[test]
    fn covedelta_object_delta_rejects_touched_property_bitmap_missing_descriptor() {
        let bytes = object_delta_with_touched_property_bitmap_section(None)
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("property_bitmap_ref does not resolve")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_touched_property_bitmap_wrong_descriptor_kind() {
        let bytes = object_delta_with_touched_property_bitmap_section(Some(
            DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET,
        ))
        .serialize()
        .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("property bitmap descriptor")
        ));
    }

    #[test]
    fn covedelta_object_delta_skips_exact_untouched_point_lookup() {
        let bytes = object_delta_with_anchor_and_touched_sections()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();

        assert_eq!(
            validation.exact_touched_membership(sample_object_point([7; 16])),
            DeltaExactObjectSetMembershipV1::Present
        );
        assert!(!validation.can_skip_delta_for_point_lookup(sample_object_point([7; 16])));
        assert_eq!(
            validation.exact_touched_membership(sample_object_point([8; 16])),
            DeltaExactObjectSetMembershipV1::Absent
        );
        assert!(validation.can_skip_delta_for_point_lookup(sample_object_point([8; 16])));
    }

    #[test]
    fn covedelta_object_delta_does_not_skip_without_exact_touched_set() {
        let bytes = object_delta().serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();

        assert_eq!(
            validation.exact_touched_membership(sample_object_point([7; 16])),
            DeltaExactObjectSetMembershipV1::Unavailable
        );
        assert!(!validation.can_skip_delta_for_point_lookup(sample_object_point([7; 16])));
    }

    #[test]
    fn covedelta_object_delta_rejects_underinclusive_touched_set() {
        let mut delta = object_delta_with_anchor_and_touched_sections();
        let mut touched = sample_touched_range();
        touched.min_goid = [8; 16];
        touched.max_goid = [8; 16];
        delta.sections[2].payload = touched.serialize().to_vec();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("touched set under-includes")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_underinclusive_continuation_anchor() {
        let mut delta = object_delta_with_anchor_and_touched_sections();
        let mut anchor = sample_anchor();
        anchor.goid = [8; 16];
        delta.sections[1].payload = anchor.serialize().to_vec();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("continuation anchors under-include")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_stale_continuation_anchor() {
        let mut delta = object_delta_with_anchor_and_touched_sections();
        let mut anchor = sample_anchor();
        anchor.predecessor_csn = 1;
        delta.sections[1].payload = anchor.serialize().to_vec();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("continuation anchors under-include")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_unresolved_anchor_state_hash_ref() {
        let mut delta = object_delta_with_anchor_and_touched_sections();
        let mut anchor = sample_anchor();
        anchor.predecessor_state_hash_ref = 1;
        delta.sections[1].payload = anchor.serialize().to_vec();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("predecessor_state_hash_ref does not resolve")
        ));
    }

    #[test]
    fn covedelta_object_delta_parses_tombstone_set_section() {
        let bytes = object_delta_with_tombstone_set_section()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.temporal_segments.len(), 1);
        assert!(validation.has_tombstone_object_set_section);
        assert_eq!(validation.tombstone_object_ranges.len(), 1);
    }

    #[test]
    fn covedelta_object_delta_tombstone_summary_checks_parent_latest_state() {
        let bytes = object_delta_with_tombstone_set_section()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();

        assert_eq!(
            validation.exact_tombstone_membership(sample_object_point([9; 16])),
            DeltaExactObjectSetMembershipV1::Present
        );
        assert!(validation
            .should_suppress_parent_latest_state_for_tombstone(sample_object_point([9; 16])));
        assert_eq!(
            validation.exact_tombstone_membership(sample_object_point([8; 16])),
            DeltaExactObjectSetMembershipV1::Absent
        );
        assert!(!validation
            .should_suppress_parent_latest_state_for_tombstone(sample_object_point([8; 16])));
    }

    #[test]
    fn covedelta_object_delta_tombstone_membership_ignores_range_false_positives() {
        let mut delta = object_delta_with_tombstone_set_section();
        let mut tombstone = sample_tombstone_range();
        tombstone.min_goid = [8; 16];
        tombstone.max_goid = [10; 16];
        tombstone.touched_count = 1;
        delta.sections[1].payload = tombstone.serialize().to_vec();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();

        assert_eq!(
            validation.exact_tombstone_membership(sample_object_point([9; 16])),
            DeltaExactObjectSetMembershipV1::Present
        );
        assert!(validation
            .should_suppress_parent_latest_state_for_tombstone(sample_object_point([9; 16])));
        assert_eq!(
            validation.exact_tombstone_membership(sample_object_point([8; 16])),
            DeltaExactObjectSetMembershipV1::Absent
        );
        assert!(!validation
            .should_suppress_parent_latest_state_for_tombstone(sample_object_point([8; 16])));
    }

    #[test]
    fn covedelta_object_delta_rejects_underinclusive_tombstone_set() {
        let mut delta = object_delta_with_tombstone_set_section();
        let mut tombstone = sample_tombstone_range();
        tombstone.min_goid = [10; 16];
        tombstone.max_goid = [10; 16];
        delta.sections[1].payload = tombstone.serialize().to_vec();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("tombstone set under-includes")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_required_tombstone_set_without_section() {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_EXACT_TOMBSTONE_SET;
        delta.sections[0].payload = tombstone_temporal_segment_payload();
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("tombstone-set feature")
        ));
    }

    #[test]
    fn covedelta_object_delta_parses_branch_identity_section() {
        let bytes = object_delta_with_branch_identity_section()
            .serialize()
            .unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.branch_identities.len(), 1);
        assert_eq!(validation.branch_identities[0].branch_identity_ref, 0);
    }

    #[test]
    fn covedelta_object_delta_rejects_required_anchor_without_anchor_section() {
        let mut delta = object_delta();
        delta.header.required_delta_features = DELTA_FEATURE_CONTINUATION_ANCHORS;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("continuation-anchor feature")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_single_scope_anchor_mismatch() {
        let mut delta = object_delta();
        delta.header.flags |= DELTA_FLAG_SINGLE_SCOPE;
        let mut anchor = sample_anchor();
        anchor.scope_id = [1; 16];
        anchor.anchor_strength = DELTA_ANCHOR_STRENGTH_KEY_ONLY;
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::ContinuationAnchors as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: anchor.serialize().to_vec(),
        });

        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert!(matches!(
            parsed.validate_object_delta(),
            Err(CoveError::BadSection(message))
                if message.contains("single-scope continuation anchor scope")
        ));
    }

    #[test]
    fn covedelta_object_delta_rejects_placeholder_temporal_payload() {
        let bytes = minimal_delta().serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert_eq!(
            parsed.validate_object_delta_sections(),
            Err(CoveError::BufferTooShort)
        );
    }

    #[test]
    fn covedelta_object_delta_rejects_unknown_required_feature() {
        let mut delta = object_delta();
        delta.header.required_delta_features = 1u64 << 63;
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert_eq!(
            parsed.validate_object_delta_sections(),
            Err(CoveError::UnknownRequiredFeature(1u64 << 63))
        );
    }

    #[test]
    fn covedelta_object_delta_falls_back_for_optional_section_required_features() {
        let mut delta = object_delta_with_extra_section(
            CoveDeltaSectionKind::IndexHints,
            1u64 << 63,
            b"unsupported optional index hints".to_vec(),
        );
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 3,
                section_kind: CoveDeltaSectionKind::CoveragePatch as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 1u64 << 63,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: b"unsupported optional coverage patch".to_vec(),
        });
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 4,
                section_kind: CoveDeltaSectionKind::LayoutHints as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 1u64 << 63,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: b"unsupported optional layout hints".to_vec(),
        });
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        let validation = parsed.validate_object_delta().unwrap();
        assert_eq!(validation.temporal_segments.len(), 1);
    }

    #[test]
    fn covedelta_object_delta_rejects_unknown_section_feature_on_required_section() {
        let delta = object_delta_with_extra_section(
            CoveDeltaSectionKind::TouchedObjectSet,
            1u64 << 63,
            Vec::new(),
        );
        let bytes = delta.serialize().unwrap();
        let parsed = CoveDeltaFile::parse(&bytes).unwrap();
        assert_eq!(
            parsed.validate_object_delta(),
            Err(CoveError::UnknownRequiredFeature(1u64 << 63))
        );
    }

    #[test]
    fn continuation_anchor_roundtrip_and_existing_patch_strength() {
        let anchor = sample_anchor();
        let bytes = anchor.serialize();
        let parsed = DeltaContinuationAnchorV1::parse(&bytes).unwrap();
        assert_eq!(parsed.object_type_id, 1);
        parsed.validate_for_existing_object_patch().unwrap();
    }

    #[test]
    fn continuation_anchor_rejects_weak_existing_patch_anchor() {
        let mut anchor = sample_anchor();
        anchor.anchor_strength = DELTA_ANCHOR_STRENGTH_KEY_AND_RECORD_ID;
        let parsed = DeltaContinuationAnchorV1::parse(&anchor.serialize()).unwrap();
        assert!(matches!(
            parsed.validate_for_existing_object_patch(),
            Err(CoveError::BadSection(message))
                if message.contains("KeyRecordAndStateHash")
        ));
    }

    #[test]
    fn continuation_anchor_rejects_bad_checksum() {
        let mut bytes = sample_anchor().serialize();
        bytes[3] ^= 0xFF;
        assert_eq!(
            DeltaContinuationAnchorV1::parse(&bytes),
            Err(CoveError::ChecksumMismatch)
        );
    }

    #[test]
    fn state_hash_descriptor_roundtrip_and_rejects_bad_payload_ref() {
        let descriptor = sample_state_hash_descriptor();
        let bytes = descriptor.serialize();
        let parsed = DeltaStateHashDescriptorV1::parse(&bytes).unwrap();
        assert_eq!(
            parsed.state_hash_kind,
            DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1
        );

        let mut missing_payload = sample_state_hash_descriptor();
        missing_payload.hash_payload_ref = DELTA_REF_NONE;
        assert!(matches!(
            DeltaStateHashDescriptorV1::parse(&missing_payload.serialize()),
            Err(CoveError::BadSection(message))
                if message.contains("hash_payload_ref")
        ));

        let mut reserved = sample_state_hash_descriptor();
        reserved.flags = 1;
        assert_eq!(
            DeltaStateHashDescriptorV1::parse(&reserved.serialize()),
            Err(CoveError::ReservedNotZero)
        );
    }

    #[test]
    fn state_hash_descriptor_rejects_algorithm_len_mismatch() {
        let mut descriptor = sample_state_hash_descriptor();
        descriptor.hash_len = 31;
        assert!(matches!(
            DeltaStateHashDescriptorV1::parse(&descriptor.serialize()),
            Err(CoveError::BadSection(message))
                if message.contains("hash_len")
        ));
    }

    #[test]
    fn dictionary_overlay_entry_roundtrip_and_rejects_bad_checksum() {
        let entry = sample_dictionary_entry();
        let bytes = entry.serialize().unwrap();
        let parsed = DeltaDictionaryEntryV1::parse(&bytes).unwrap();
        assert_eq!(parsed.local_dictionary_id, 1);
        assert_eq!(parsed.local_code, 7);
        assert_eq!(parsed.entry_kind, DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE);

        let mut corrupt = bytes;
        corrupt[4] ^= 0xFF;
        assert_eq!(
            DeltaDictionaryEntryV1::parse(&corrupt),
            Err(CoveError::ChecksumMismatch)
        );
    }

    #[test]
    fn scope_descriptor_roundtrip_and_rejects_reserved_flags() {
        let descriptor = sample_scope_descriptor();
        let bytes = descriptor.serialize().unwrap();
        let parsed = DeltaScopeDescriptorV1::parse(&bytes).unwrap();
        assert_eq!(parsed.scope_ref, 0);
        assert_eq!(parsed.scope_id, [0; 16]);

        let mut reserved = descriptor;
        reserved.flags = 1;
        assert_eq!(reserved.serialize(), Err(CoveError::ReservedNotZero));
    }

    #[test]
    fn summary_descriptor_roundtrip_and_rejects_unknown_kind() {
        let descriptor = sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET);
        let bytes = descriptor.serialize().unwrap();
        let parsed = DeltaSummaryDescriptorV1::parse(&bytes).unwrap();
        assert_eq!(parsed.summary_ref, 0);
        assert_eq!(
            parsed.summary_kind,
            DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET
        );

        let mut unknown = descriptor;
        unknown.summary_kind = 200;
        assert!(matches!(
            unknown.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("unknown summary_kind")
        ));
    }

    #[test]
    fn object_delta_state_hash_material_is_stable_and_canonical() {
        let state = sample_state_hash_material();
        let hash = state.compute_hash(DigestAlgorithm::Sha256).unwrap();
        assert_eq!(hash.len(), 32);

        let same_state = sample_state_hash_material();
        assert_eq!(
            hash,
            same_state.compute_hash(DigestAlgorithm::Sha256).unwrap()
        );

        let mut changed_state = sample_state_hash_material();
        changed_state.properties[0].canonical_value = b"bob".to_vec();
        assert_ne!(
            hash,
            changed_state.compute_hash(DigestAlgorithm::Sha256).unwrap()
        );

        let mut unsorted = sample_state_hash_material();
        unsorted.properties.swap(0, 1);
        assert!(matches!(
            unsorted.canonical_material(),
            Err(CoveError::BadSection(message))
                if message.contains("sorted by unique property_id")
        ));
    }

    #[test]
    fn sparse_patch_record_roundtrip_and_rejects_bad_checksum() {
        let record = sample_sparse_patch_record();
        let bytes = record.serialize().unwrap();
        let parsed = DeltaSparsePatchRecordV1::parse(&bytes).unwrap();
        assert_eq!(parsed.changed_properties.len(), 5);

        let mut corrupt = bytes;
        corrupt[4] ^= 0xFF;
        assert_eq!(
            DeltaSparsePatchRecordV1::parse(&corrupt),
            Err(CoveError::ChecksumMismatch)
        );
    }

    #[test]
    fn sparse_patch_record_rejects_unsorted_properties() {
        let mut record = sample_sparse_patch_record();
        record.changed_properties.swap(0, 1);
        assert!(matches!(
            record.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("sorted by unique property_id")
        ));
    }

    #[test]
    fn sparse_patch_record_rejects_invalid_operation_payload_refs() {
        let mut missing_value = sample_sparse_patch_record();
        missing_value.changed_properties[0].value_ref = DELTA_REF_NONE;
        assert!(matches!(
            missing_value.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("requires value_ref")
        ));

        let mut missing_redaction = sample_sparse_patch_record();
        missing_redaction.changed_properties[4].redaction_ref = DELTA_REF_NONE;
        assert!(matches!(
            missing_redaction.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("requires only redaction_ref")
        ));
    }

    #[test]
    fn sparse_patch_record_applies_operations_and_preserves_omitted_properties() {
        let record = sample_sparse_patch_record();
        let mut state = BTreeMap::from([
            (3, DeltaSparsePatchPropertyStateV1::ValueRef(99)),
            (99, DeltaSparsePatchPropertyStateV1::ValueRef(100)),
        ]);
        record.apply_to_property_state(&mut state).unwrap();

        assert_eq!(
            state.get(&1),
            Some(&DeltaSparsePatchPropertyStateV1::ValueRef(0))
        );
        assert_eq!(state.get(&2), Some(&DeltaSparsePatchPropertyStateV1::Null));
        assert_eq!(state.get(&3), Some(&DeltaSparsePatchPropertyStateV1::Clear));
        assert_eq!(
            state.get(&4),
            Some(&DeltaSparsePatchPropertyStateV1::Tombstone(
                DELTA_TOMBSTONE_KIND_PROPERTY
            ))
        );
        assert_eq!(
            state.get(&5),
            Some(&DeltaSparsePatchPropertyStateV1::Redacted { redaction_ref: 12 })
        );
        assert_eq!(
            state.get(&99),
            Some(&DeltaSparsePatchPropertyStateV1::ValueRef(100))
        );
    }

    #[test]
    fn sparse_patch_state_table_applies_records_in_csn_order() {
        let mut first = sample_sparse_patch_record();
        first.record_id = [1; 16];
        first.timestamp_us = 10;
        first.csn = 1;
        first.changed_properties = vec![
            DeltaSparsePatchPropertyOpV1 {
                property_id: 1,
                property_op: DELTA_PROPERTY_OP_SET_VALUE,
                tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                value_ref: 11,
                redaction_ref: DELTA_REF_NONE,
                flags: 0,
            },
            DeltaSparsePatchPropertyOpV1 {
                property_id: 2,
                property_op: DELTA_PROPERTY_OP_SET_NULL,
                tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                value_ref: DELTA_REF_NONE,
                redaction_ref: DELTA_REF_NONE,
                flags: 0,
            },
        ];
        let mut second = sample_sparse_patch_record();
        second.record_id = [2; 16];
        second.timestamp_us = 20;
        second.csn = 2;
        second.changed_properties = vec![DeltaSparsePatchPropertyOpV1 {
            property_id: 1,
            property_op: DELTA_PROPERTY_OP_SET_VALUE,
            tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
            value_ref: 22,
            redaction_ref: DELTA_REF_NONE,
            flags: 0,
        }];

        let table = reconstruct_sparse_patch_state_table(&[second.clone(), first.clone()]).unwrap();
        let state = table.get(&first.object_key()).unwrap();
        assert_eq!(state.latest_record_id, [2; 16]);
        assert_eq!(state.latest_timestamp_us, 20);
        assert_eq!(state.latest_csn, 2);
        assert_eq!(
            state.tombstone_status,
            DeltaSparseObjectTombstoneStatusV1::Live
        );
        assert_eq!(
            state.properties.get(&1),
            Some(&DeltaSparsePatchPropertyStateV1::ValueRef(22))
        );
        assert_eq!(
            state.properties.get(&2),
            Some(&DeltaSparsePatchPropertyStateV1::Null)
        );
    }

    #[test]
    fn sparse_patch_state_table_rejects_duplicate_record_id_for_object() {
        let first = sample_sparse_patch_record();
        let mut second = sample_sparse_patch_record();
        second.csn = 2;
        second.timestamp_us = 20;

        assert!(matches!(
            reconstruct_sparse_patch_state_table(&[first, second]),
            Err(CoveError::BadSection(message))
                if message.contains("duplicate record_id")
        ));
    }

    #[test]
    fn sparse_patch_state_table_tracks_object_tombstone() {
        let mut record = sample_sparse_patch_record();
        record.changed_properties = vec![DeltaSparsePatchPropertyOpV1 {
            property_id: 1,
            property_op: DELTA_PROPERTY_OP_TOMBSTONE,
            tombstone_kind: DELTA_TOMBSTONE_KIND_OBJECT,
            value_ref: DELTA_REF_NONE,
            redaction_ref: DELTA_REF_NONE,
            flags: 0,
        }];

        let table = reconstruct_sparse_patch_state_table(&[record.clone()]).unwrap();
        let state = table.get(&record.object_key()).unwrap();
        assert_eq!(
            state.tombstone_status,
            DeltaSparseObjectTombstoneStatusV1::Tombstoned
        );
        assert_eq!(
            state.properties.get(&1),
            Some(&DeltaSparsePatchPropertyStateV1::Tombstone(
                DELTA_TOMBSTONE_KIND_OBJECT
            ))
        );
    }

    #[test]
    fn touched_object_range_roundtrip_and_rejects_inverted_range() {
        let range = sample_touched_range();
        let bytes = range.serialize();
        let parsed = DeltaTouchedObjectRangeV1::parse(&bytes).unwrap();
        assert_eq!(parsed.touched_count, 1);

        let mut inverted = sample_touched_range();
        inverted.min_goid = [8; 16];
        assert!(matches!(
            DeltaTouchedObjectRangeV1::parse(&inverted.serialize()),
            Err(CoveError::BadSection(message))
                if message.contains("min_goid")
        ));
    }

    #[test]
    fn branch_identity_roundtrip_and_rejects_missing_value_ref() {
        let identity = sample_branch_identity();
        let bytes = identity.serialize();
        let parsed = DeltaBranchIdentityV1::parse(&bytes).unwrap();
        assert_eq!(
            parsed.branch_identity_kind,
            DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF
        );

        let mut missing_ref = sample_branch_identity();
        missing_ref.branch_value_ref = DELTA_REF_NONE;
        assert!(matches!(
            DeltaBranchIdentityV1::parse(&missing_ref.serialize()),
            Err(CoveError::BadSection(message))
                if message.contains("branch_value_ref")
        ));

        let mut reserved = sample_branch_identity();
        reserved.flags = 1;
        assert_eq!(
            DeltaBranchIdentityV1::parse(&reserved.serialize()),
            Err(CoveError::ReservedNotZero)
        );
    }
}
