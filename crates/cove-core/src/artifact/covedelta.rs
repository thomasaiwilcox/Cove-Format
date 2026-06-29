//! COVE-O `.covedelta` immutable delta artifact envelope.
//!
//! This module implements the structural envelope from Spec §63.1:
//! `[header][parent refs][section payloads][section directory][footer][postscript][tail]`
//! with final magic `CVD2`. Payload-specific temporal delta semantics are layered
//! on top of this envelope by later phases.

use std::collections::{BTreeMap, BTreeSet};

mod wire;

use wire::*;

use crate::{
    canonical, checksum,
    constants::{CoveLogicalType, DigestAlgorithm, ValueTag, MAGIC_COVEDELTA},
    digest::compute_digest,
    profile::{
        cove_map::{MapEvidenceIndex, MapProjectionCatalog},
        cove_o::{ObjectTypeCatalog, RecordKind, TemporalSegmentData},
    },
    CoveError,
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
        let checksum =
            u32::from_le_bytes(record_bytes[record_len - 4..record_len].try_into().unwrap());
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
        let postscript_version =
            u16::from_le_bytes(bytes[tail_start..tail_start + 2].try_into().unwrap());
        if postscript_version != COVEDELTA_POSTSCRIPT_VERSION_V1 {
            return Err(CoveError::BadVersion);
        }
        let postscript_len =
            u16::from_le_bytes(bytes[tail_start + 2..tail_start + 4].try_into().unwrap());
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

#[cfg(test)]
mod fixtures;

#[cfg(test)]
mod tests;
