//! Spec §69 — COVM dataset manifest (spec-exact wire format).
//!
//! A COVM aggregates file-level metadata for a collection of COVE files so a
//! planner can prune at file level before opening any COVE footer. Per
//! Spec §69:
//!
//! * The file ends with the pattern
//!   `[postscript bytes][postscript_version: u16][postscript_len: u16][magic: "CVM2"]`.
//! * The header is [`CovmHeaderV1`] (Spec §69.1) carrying a `dataset_id`,
//!   `table_count`, `file_count`, and a CRC32C checksum.
//! * Each file is described by a [`CovmFileEntryV1`] (Spec §69.2) with
//!   variable-length `uri` and variable-length cryptographic digest plus
//!   `file_len`, `footer_crc32c`, `row_count`, `segment_count`, and refs to
//!   optional file-level stats and exact-set artifacts.
//!
//! Spec §69 Rules enforced by this module:
//! * COVM MUST be ignored if stale (any of `file_id`, `file_len`,
//!   `footer_crc32c`, `digest` mismatches the host file).
//! * COVM MUST NOT change COVE semantics — it is purely advisory pruning data.
//!
//! The bytes between the header and the postscript hold the file-entry
//! array; this implementation packs them sequentially in declaration order.
//! Spec §69 does not standardise table-schema-fingerprint or partition
//! payload layouts; they are not modelled here yet.

use std::collections::BTreeSet;

use super::covedelta::{
    CoveDeltaFile, DeltaParentRefV1, DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT,
    DELTA_PARENT_REF_LINEAGE_PARENT, DELTA_REF_NONE,
};
use crate::checksum;
use crate::constants::{DigestAlgorithm, MAGIC_COVM, POSTSCRIPT_VERSION_V1};
use crate::digest::compute_digest;
use crate::error::CoveError;

// ── Constants ────────────────────────────────────────────────────────────────

/// Encoded length of [`CovmHeaderV1`] in bytes.
///
/// Layout: magic(4) + header_len(2) + version_major(2) + version_minor(2)
///       + flags(4) + dataset_id(16) + table_count(4) + file_count(4)
///       + created_at_us(8) + reserved(32) + checksum(4) = 82.
pub const COVM_HEADER_LEN: u16 = 82;

/// Required artifact header `version_major` for COVM v2.
pub const COVM_VERSION_MAJOR_V1: u16 = 1;

/// Required artifact header `version_minor` for COVM v2.
pub const COVM_VERSION_MINOR_V1: u16 = 0;

/// Encoded length of [`CovmPostscriptV1`] in bytes (implementation-defined
/// payload; the tail framing of `[version u16][len u16][magic 4]` is
/// standardised by Spec §69).
pub const COVM_POSTSCRIPT_LEN: u16 = 48;

/// Postscript version field value for COVM v2.
pub const COVM_POSTSCRIPT_VERSION_V1: u16 = POSTSCRIPT_VERSION_V1;

/// Size of the fixed tail after the postscript payload.
pub const COVM_POSTSCRIPT_TAIL_SIZE: usize = 2 + 2 + 4;

/// Postscript flag marking a COVM manifest as selecting a base-plus-delta
/// snapshot that requires delta-chain validation before object-temporal data
/// may be read.
pub const COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED: u32 = 0x0000_0001;

const COVM_POSTSCRIPT_KNOWN_FLAGS: u32 = COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED;

/// Required-extension profile ID for the COVM delta-chain snapshot selector.
///
/// Encoded little-endian bytes are `CDC1`.
pub const COVM_DELTA_CHAIN_PROFILE_ID_V1: u32 = 0x3143_4443;

pub const COVM_DELTA_CHAIN_PROFILE_VERSION_MAJOR_V1: u16 = 1;
pub const COVM_DELTA_CHAIN_PROFILE_VERSION_MINOR_V1: u16 = 0;

/// MVP readers support no required delta feature bits yet.
pub const COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES: u64 = 0;

/// Encoded length of [`CovmDeltaArtifactRefV1`].
pub const COVM_DELTA_ARTIFACT_REF_LEN: usize = 112;

/// Encoded length of the fixed delta-chain extension header.
pub const COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN: usize = 298;

pub const COVM_DELTA_CHAIN_SUMMARY_KIND_NONE: u16 = 0;
pub const COVM_DELTA_CHAIN_SUMMARY_KIND_CDS1: u16 = 1;

pub const COVM_DELTA_CHAIN_SUMMARY_MAGIC: [u8; 4] = *b"CDS1";
pub const COVM_DELTA_CHAIN_SUMMARY_VERSION_MAJOR_V1: u16 = 1;
pub const COVM_DELTA_CHAIN_SUMMARY_VERSION_MINOR_V1: u16 = 0;
pub const COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN: u16 = 122;
pub const COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN: usize = 296;

pub const DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT: u32 = 0x0000_0001;
pub const DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT: u32 = 0x0000_0002;
pub const DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE: u32 = 0x0000_0001;
pub const DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_EXACT: u32 = 0x0000_0002;

// ── CovmHeaderV1 ──────────────────────────────────────────────────────────────

/// Spec §69.1 `CovmHeaderV1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmHeaderV1 {
    /// Magic bytes — MUST equal [`MAGIC_COVM`] (`"CVM2"`).
    pub magic: [u8; 4],
    /// Header length in bytes — MUST equal [`COVM_HEADER_LEN`] for the v2 artifact.
    pub header_len: u16,
    /// Major version — MUST equal [`COVM_VERSION_MAJOR_V1`] for the v2 artifact.
    pub version_major: u16,
    /// Minor version — MUST equal [`COVM_VERSION_MINOR_V1`] for the v2 artifact.
    pub version_minor: u16,
    /// Header flags reserved for future artifact versions.
    pub flags: u32,
    /// Stable identifier for this dataset.
    pub dataset_id: [u8; 16],
    /// Number of distinct tables aggregated by this manifest.
    pub table_count: u32,
    /// Number of [`CovmFileEntryV1`] entries that follow the header.
    pub file_count: u32,
    /// Creation timestamp in microseconds since the Unix epoch.
    pub created_at_us: i64,
    /// Reserved — MUST be zero in the v2 artifact.
    pub reserved: [u8; 32],
    /// CRC32C of the 82-byte header with this `checksum` field zeroed.
    pub checksum: u32,
}

impl CovmHeaderV1 {
    pub fn serialize(&self) -> [u8; COVM_HEADER_LEN as usize] {
        let mut buf = [0u8; COVM_HEADER_LEN as usize];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..6].copy_from_slice(&self.header_len.to_le_bytes());
        buf[6..8].copy_from_slice(&self.version_major.to_le_bytes());
        buf[8..10].copy_from_slice(&self.version_minor.to_le_bytes());
        buf[10..14].copy_from_slice(&self.flags.to_le_bytes());
        buf[14..30].copy_from_slice(&self.dataset_id);
        buf[30..34].copy_from_slice(&self.table_count.to_le_bytes());
        buf[34..38].copy_from_slice(&self.file_count.to_le_bytes());
        buf[38..46].copy_from_slice(&self.created_at_us.to_le_bytes());
        buf[46..78].copy_from_slice(&self.reserved);
        // Bytes [78..82] = checksum, left zero during CRC.
        let crc = checksum::crc32c(&buf);
        buf[78..82].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVM_HEADER_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVM_HEADER_LEN as usize];

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        if magic != MAGIC_COVM {
            return Err(CoveError::BadMagic);
        }
        let header_len = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if header_len != COVM_HEADER_LEN {
            return Err(CoveError::BadSection(format!(
                "COVM header_len must be {COVM_HEADER_LEN}, got {header_len}"
            )));
        }
        let version_major = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        let version_minor = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        if version_major != COVM_VERSION_MAJOR_V1 {
            return Err(CoveError::BadVersion);
        }
        let flags = u32::from_le_bytes(bytes[10..14].try_into().unwrap());
        let mut dataset_id = [0u8; 16];
        dataset_id.copy_from_slice(&bytes[14..30]);
        let table_count = u32::from_le_bytes(bytes[30..34].try_into().unwrap());
        let file_count = u32::from_le_bytes(bytes[34..38].try_into().unwrap());
        let created_at_us = i64::from_le_bytes(bytes[38..46].try_into().unwrap());
        let mut reserved = [0u8; 32];
        reserved.copy_from_slice(&bytes[46..78]);
        if reserved.iter().any(|b| *b != 0) {
            return Err(CoveError::ReservedNotZero);
        }
        let checksum_field = u32::from_le_bytes(bytes[78..82].try_into().unwrap());

        let mut for_crc = [0u8; COVM_HEADER_LEN as usize];
        for_crc.copy_from_slice(bytes);
        for_crc[78..82].fill(0);
        if checksum::crc32c(&for_crc) != checksum_field {
            return Err(CoveError::ChecksumMismatch);
        }

        Ok(Self {
            magic,
            header_len,
            version_major,
            version_minor,
            flags,
            dataset_id,
            table_count,
            file_count,
            created_at_us,
            reserved,
            checksum: checksum_field,
        })
    }

    pub fn new(
        dataset_id: [u8; 16],
        table_count: u32,
        file_count: u32,
        created_at_us: i64,
    ) -> Self {
        Self {
            magic: MAGIC_COVM,
            header_len: COVM_HEADER_LEN,
            version_major: COVM_VERSION_MAJOR_V1,
            version_minor: COVM_VERSION_MINOR_V1,
            flags: 0,
            dataset_id,
            table_count,
            file_count,
            created_at_us,
            reserved: [0u8; 32],
            checksum: 0,
        }
    }
}

// ── CovmFileEntryV1 ───────────────────────────────────────────────────────────

/// Spec §69.2 `CovmFileEntryV1`.
///
/// Wire layout (little-endian):
/// `file_id(16) + uri_len(2) + uri(uri_len) + file_len(8) + footer_crc32c(4)
/// + digest_algorithm(2) + digest_len(2) + digest(digest_len) + row_count(8)
/// + segment_count(4) + file_stats_ref(4) + file_exact_set_ref(4) + flags(4)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmFileEntryV1 {
    pub file_id: [u8; 16],
    pub uri: String,
    pub file_len: u64,
    pub footer_crc32c: u32,
    pub digest_algorithm: u16,
    pub digest: Vec<u8>,
    pub row_count: u64,
    pub segment_count: u32,
    pub file_stats_ref: u32,
    pub file_exact_set_ref: u32,
    pub flags: u32,
}

impl CovmFileEntryV1 {
    pub fn encoded_len(&self) -> usize {
        16 + 2 + self.uri.len() + 8 + 4 + 2 + 2 + self.digest.len() + 8 + 4 + 4 + 4 + 4
    }

    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        if self.uri.len() > u16::MAX as usize {
            return Err(CoveError::BadSection(
                "COVM uri_len exceeds u16::MAX".into(),
            ));
        }
        if self.digest.len() > u16::MAX as usize {
            return Err(CoveError::BadSection(
                "COVM digest_len exceeds u16::MAX".into(),
            ));
        }
        let mut out = Vec::with_capacity(self.encoded_len());
        out.extend_from_slice(&self.file_id);
        out.extend_from_slice(&(self.uri.len() as u16).to_le_bytes());
        out.extend_from_slice(self.uri.as_bytes());
        out.extend_from_slice(&self.file_len.to_le_bytes());
        out.extend_from_slice(&self.footer_crc32c.to_le_bytes());
        out.extend_from_slice(&self.digest_algorithm.to_le_bytes());
        out.extend_from_slice(&(self.digest.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.digest);
        out.extend_from_slice(&self.row_count.to_le_bytes());
        out.extend_from_slice(&self.segment_count.to_le_bytes());
        out.extend_from_slice(&self.file_stats_ref.to_le_bytes());
        out.extend_from_slice(&self.file_exact_set_ref.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        Ok(out)
    }

    pub fn parse(bytes: &[u8]) -> Result<(Self, usize), CoveError> {
        // Fixed prefix up to and including uri_len.
        if bytes.len() < 16 + 2 {
            return Err(CoveError::BufferTooShort);
        }
        let mut file_id = [0u8; 16];
        file_id.copy_from_slice(&bytes[0..16]);
        let uri_len = u16::from_le_bytes(bytes[16..18].try_into().unwrap()) as usize;
        let mut pos = 18usize;

        let uri_end = pos.checked_add(uri_len).ok_or(CoveError::ArithOverflow)?;
        if uri_end > bytes.len() {
            return Err(CoveError::BufferTooShort);
        }
        let uri = std::str::from_utf8(&bytes[pos..uri_end])
            .map_err(|_| CoveError::BadSection("COVM uri is not UTF-8".into()))?
            .to_string();
        pos = uri_end;

        // file_len(8) + footer_crc32c(4) + digest_algorithm(2) + digest_len(2)
        if bytes.len() < pos + 8 + 4 + 2 + 2 {
            return Err(CoveError::BufferTooShort);
        }
        let file_len = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let footer_crc32c = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let digest_algorithm = u16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap());
        pos += 2;
        let digest_len = u16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;

        let digest_end = pos
            .checked_add(digest_len)
            .ok_or(CoveError::ArithOverflow)?;
        if digest_end > bytes.len() {
            return Err(CoveError::BufferTooShort);
        }
        let digest = bytes[pos..digest_end].to_vec();
        pos = digest_end;

        if bytes.len() < pos + 8 + 4 + 4 + 4 + 4 {
            return Err(CoveError::BufferTooShort);
        }
        let row_count = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let segment_count = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let file_stats_ref = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let file_exact_set_ref = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let flags = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;

        Ok((
            Self {
                file_id,
                uri,
                file_len,
                footer_crc32c,
                digest_algorithm,
                digest,
                row_count,
                segment_count,
                file_stats_ref,
                file_exact_set_ref,
                flags,
            },
            pos,
        ))
    }

    /// Verify this entry against a host COVE file's identity.
    /// Mismatch of `file_id`, `file_len`, `footer_crc32c`, or `digest`
    /// yields [`CoveError::SidecarStale`] (Spec §69 Rules).
    pub fn verify_against(
        &self,
        host_file_id: &[u8; 16],
        host_file_len: u64,
        host_footer_crc32c: u32,
        host_digest: &[u8],
    ) -> Result<(), CoveError> {
        if &self.file_id != host_file_id
            || self.file_len != host_file_len
            || self.footer_crc32c != host_footer_crc32c
            || self.digest.as_slice() != host_digest
        {
            Err(CoveError::SidecarStale)
        } else {
            Ok(())
        }
    }
}

// ── CovmPostscriptV1 ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmPostscriptV1 {
    pub header_offset: u64,
    pub header_len: u64,
    pub entries_offset: u64,
    pub entries_len: u64,
    pub file_len: u64,
    pub flags: u32,
    pub checksum: u32,
}

impl CovmPostscriptV1 {
    pub fn serialize(&self) -> [u8; COVM_POSTSCRIPT_LEN as usize] {
        let mut buf = [0u8; COVM_POSTSCRIPT_LEN as usize];
        buf[0..8].copy_from_slice(&self.header_offset.to_le_bytes());
        buf[8..16].copy_from_slice(&self.header_len.to_le_bytes());
        buf[16..24].copy_from_slice(&self.entries_offset.to_le_bytes());
        buf[24..32].copy_from_slice(&self.entries_len.to_le_bytes());
        buf[32..40].copy_from_slice(&self.file_len.to_le_bytes());
        buf[40..44].copy_from_slice(&self.flags.to_le_bytes());
        let crc = checksum::crc32c(&buf);
        buf[44..48].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn serialize_tail(&self) -> [u8; COVM_POSTSCRIPT_LEN as usize + COVM_POSTSCRIPT_TAIL_SIZE] {
        let mut tail = [0u8; COVM_POSTSCRIPT_LEN as usize + COVM_POSTSCRIPT_TAIL_SIZE];
        let payload = self.serialize();
        tail[..COVM_POSTSCRIPT_LEN as usize].copy_from_slice(&payload);
        let n = COVM_POSTSCRIPT_LEN as usize;
        tail[n..n + 2].copy_from_slice(&COVM_POSTSCRIPT_VERSION_V1.to_le_bytes());
        tail[n + 2..n + 4].copy_from_slice(&COVM_POSTSCRIPT_LEN.to_le_bytes());
        tail[n + 4..n + 8].copy_from_slice(&MAGIC_COVM);
        tail
    }

    pub fn parse_from_tail(file_data: &[u8]) -> Result<Self, CoveError> {
        let total = COVM_POSTSCRIPT_LEN as usize + COVM_POSTSCRIPT_TAIL_SIZE;
        if file_data.len() < total {
            return Err(CoveError::BufferTooShort);
        }
        let tail = &file_data[file_data.len() - total..];

        let n = COVM_POSTSCRIPT_LEN as usize;
        let version = u16::from_le_bytes(tail[n..n + 2].try_into().unwrap());
        let len = u16::from_le_bytes(tail[n + 2..n + 4].try_into().unwrap());
        let magic: [u8; 4] = tail[n + 4..n + 8].try_into().unwrap();

        if magic != MAGIC_COVM {
            return Err(CoveError::BadMagic);
        }
        if version != COVM_POSTSCRIPT_VERSION_V1 {
            return Err(CoveError::BadVersion);
        }
        if len != COVM_POSTSCRIPT_LEN {
            return Err(CoveError::BadSection(format!(
                "COVM postscript_len must be {COVM_POSTSCRIPT_LEN}, got {len}"
            )));
        }

        let payload: [u8; COVM_POSTSCRIPT_LEN as usize] = tail[..n].try_into().unwrap();
        let header_offset = u64::from_le_bytes(payload[0..8].try_into().unwrap());
        let header_len = u64::from_le_bytes(payload[8..16].try_into().unwrap());
        let entries_offset = u64::from_le_bytes(payload[16..24].try_into().unwrap());
        let entries_len = u64::from_le_bytes(payload[24..32].try_into().unwrap());
        let file_len = u64::from_le_bytes(payload[32..40].try_into().unwrap());
        let flags = u32::from_le_bytes(payload[40..44].try_into().unwrap());
        let checksum_field = u32::from_le_bytes(payload[44..48].try_into().unwrap());

        let mut for_crc = payload;
        for_crc[44..48].fill(0);
        if checksum::crc32c(&for_crc) != checksum_field {
            return Err(CoveError::ChecksumMismatch);
        }

        Ok(Self {
            header_offset,
            header_len,
            entries_offset,
            entries_len,
            file_len,
            flags,
            checksum: checksum_field,
        })
    }
}

// ── COVM delta-chain required extension ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmDeltaArtifactRefV1 {
    pub chain_ordinal: u32,
    pub flags: u32,
    pub artifact_id: [u8; 16],
    pub snapshot_id: [u8; 16],
    pub parent_snapshot_id: [u8; 16],
    pub file_len: u64,
    pub footer_crc32c: u32,
    pub digest_algorithm: u16,
    pub digest_len: u16,
    pub digest: [u8; 32],
    pub uri_ref: u32,
    pub checksum: u32,
}

impl CovmDeltaArtifactRefV1 {
    pub fn serialize(&self) -> [u8; COVM_DELTA_ARTIFACT_REF_LEN] {
        let mut buf = [0u8; COVM_DELTA_ARTIFACT_REF_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.chain_ordinal);
        put_u32(&mut buf, &mut pos, self.flags);
        put_bytes(&mut buf, &mut pos, &self.artifact_id);
        put_bytes(&mut buf, &mut pos, &self.snapshot_id);
        put_bytes(&mut buf, &mut pos, &self.parent_snapshot_id);
        put_u64(&mut buf, &mut pos, self.file_len);
        put_u32(&mut buf, &mut pos, self.footer_crc32c);
        put_u16(&mut buf, &mut pos, self.digest_algorithm);
        put_u16(&mut buf, &mut pos, self.digest_len);
        put_bytes(&mut buf, &mut pos, &self.digest);
        put_u32(&mut buf, &mut pos, self.uri_ref);
        let checksum_offset = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, COVM_DELTA_ARTIFACT_REF_LEN);

        let crc = checksum::crc32c(&buf);
        buf[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < COVM_DELTA_ARTIFACT_REF_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVM_DELTA_ARTIFACT_REF_LEN];
        let mut checksum_bytes = [0u8; COVM_DELTA_ARTIFACT_REF_LEN];
        checksum_bytes.copy_from_slice(bytes);
        checksum_bytes[COVM_DELTA_ARTIFACT_REF_LEN - 4..].fill(0);
        let checksum_field = u32::from_le_bytes(
            bytes[COVM_DELTA_ARTIFACT_REF_LEN - 4..COVM_DELTA_ARTIFACT_REF_LEN]
                .try_into()
                .unwrap(),
        );
        if checksum::crc32c(&checksum_bytes) != checksum_field {
            return Err(CoveError::ChecksumMismatch);
        }

        let mut pos = 0usize;
        let chain_ordinal = take_u32(bytes, &mut pos)?;
        let flags = take_u32(bytes, &mut pos)?;
        let artifact_id = take_array::<16>(bytes, &mut pos)?;
        let snapshot_id = take_array::<16>(bytes, &mut pos)?;
        let parent_snapshot_id = take_array::<16>(bytes, &mut pos)?;
        let file_len = take_u64(bytes, &mut pos)?;
        let footer_crc32c = take_u32(bytes, &mut pos)?;
        let digest_algorithm = take_u16(bytes, &mut pos)?;
        let digest_len = take_u16(bytes, &mut pos)?;
        let digest = take_array::<32>(bytes, &mut pos)?;
        let uri_ref = take_u32(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;

        let reference = Self {
            chain_ordinal,
            flags,
            artifact_id,
            snapshot_id,
            parent_snapshot_id,
            file_len,
            footer_crc32c,
            digest_algorithm,
            digest_len,
            digest,
            uri_ref,
            checksum,
        };
        reference.validate_mandatory_digest()?;
        Ok(reference)
    }

    fn validate_mandatory_digest(&self) -> Result<(), CoveError> {
        let algorithm = covm_delta_required_digest_algorithm(
            self.digest_algorithm,
            "COVM delta artifact ref digest_algorithm",
        )?;
        let expected_len = covm_delta_expected_digest_len(algorithm);
        if self.digest_len as usize != expected_len {
            return Err(CoveError::BadSection(format!(
                "COVM delta artifact ref digest_len must be {expected_len}, got {}",
                self.digest_len
            )));
        }
        Ok(())
    }

    fn append_digest_binding(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.chain_ordinal.to_le_bytes());
        out.extend_from_slice(&self.artifact_id);
        out.extend_from_slice(&self.snapshot_id);
        out.extend_from_slice(&self.parent_snapshot_id);
        out.extend_from_slice(&self.file_len.to_le_bytes());
        out.extend_from_slice(&self.footer_crc32c.to_le_bytes());
        out.extend_from_slice(&self.digest_algorithm.to_le_bytes());
        out.extend_from_slice(&self.digest_len.to_le_bytes());
        out.extend_from_slice(&self.digest);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmDeltaChainExtensionV1 {
    pub delta_chain_profile_id: u32,
    pub delta_chain_profile_version_major: u16,
    pub delta_chain_profile_version_minor: u16,
    pub required_delta_features: u64,
    pub optional_delta_features: u64,
    pub dataset_id: [u8; 16],
    pub base_snapshot_id: [u8; 16],
    pub result_snapshot_id: [u8; 16],
    pub base_artifact_ref: CovmDeltaArtifactRefV1,
    pub ordered_delta_artifact_refs: Vec<CovmDeltaArtifactRefV1>,
    pub chain_digest_algorithm: u16,
    pub chain_digest: Vec<u8>,
    pub chain_summary_kind: u16,
    pub chain_summary_ref: u32,
    pub chain_summary_offset: u64,
    pub chain_summary_length: u64,
    pub chain_summary_crc32c: u32,
    pub chain_summary_digest_algorithm: u16,
    pub chain_summary_digest: Vec<u8>,
    pub effective_schema_fingerprint_ref: u32,
    pub effective_object_catalog_fingerprint_ref: u32,
    pub effective_projection_fingerprint_ref: u32,
    pub effective_semantic_map_fingerprint_ref: u32,
    pub effective_visibility_fingerprint_ref: u32,
    pub effective_redaction_fingerprint_ref: u32,
    pub csn_min: u64,
    pub csn_max: u64,
    pub created_at_us: i64,
    pub checksum: u32,
}

impl CovmDeltaChainExtensionV1 {
    pub fn new(
        dataset_id: [u8; 16],
        base_snapshot_id: [u8; 16],
        result_snapshot_id: [u8; 16],
        base_artifact_ref: CovmDeltaArtifactRefV1,
        ordered_delta_artifact_refs: Vec<CovmDeltaArtifactRefV1>,
    ) -> Self {
        Self {
            delta_chain_profile_id: COVM_DELTA_CHAIN_PROFILE_ID_V1,
            delta_chain_profile_version_major: COVM_DELTA_CHAIN_PROFILE_VERSION_MAJOR_V1,
            delta_chain_profile_version_minor: COVM_DELTA_CHAIN_PROFILE_VERSION_MINOR_V1,
            required_delta_features: 0,
            optional_delta_features: 0,
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            base_artifact_ref,
            ordered_delta_artifact_refs,
            chain_digest_algorithm: DigestAlgorithm::Sha256 as u16,
            chain_digest: Vec::new(),
            chain_summary_kind: COVM_DELTA_CHAIN_SUMMARY_KIND_NONE,
            chain_summary_ref: 0,
            chain_summary_offset: 0,
            chain_summary_length: 0,
            chain_summary_crc32c: 0,
            chain_summary_digest_algorithm: DigestAlgorithm::None as u16,
            chain_summary_digest: Vec::new(),
            effective_schema_fingerprint_ref: 0,
            effective_object_catalog_fingerprint_ref: 0,
            effective_projection_fingerprint_ref: 0,
            effective_semantic_map_fingerprint_ref: 0,
            effective_visibility_fingerprint_ref: 0,
            effective_redaction_fingerprint_ref: 0,
            csn_min: 0,
            csn_max: 0,
            created_at_us: 0,
            checksum: 0,
        }
    }

    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        self.validate_structure(COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES)?;
        let chain_digest = self.computed_chain_digest()?;
        let chain_digest_algorithm = covm_delta_required_digest_algorithm(
            self.chain_digest_algorithm,
            "COVM delta chain_digest_algorithm",
        )?;
        if chain_digest.len() != covm_delta_expected_digest_len(chain_digest_algorithm) {
            return Err(CoveError::BadSection(
                "COVM delta chain digest has invalid length".into(),
            ));
        }
        let summary_digest_algorithm = covm_delta_optional_digest_algorithm(
            self.chain_summary_digest_algorithm,
            "COVM delta chain_summary_digest_algorithm",
        )?;
        if let Some(algorithm) = summary_digest_algorithm {
            let expected_len = covm_delta_expected_digest_len(algorithm);
            if self.chain_summary_digest.len() != expected_len {
                return Err(CoveError::BadSection(format!(
                    "COVM delta chain summary digest length must be {expected_len}, got {}",
                    self.chain_summary_digest.len()
                )));
            }
        } else if !self.chain_summary_digest.is_empty() {
            return Err(CoveError::BadSection(
                "COVM delta chain summary digest bytes require a digest algorithm".into(),
            ));
        }

        let ordered_refs_bytes = self
            .ordered_delta_artifact_refs
            .iter()
            .flat_map(|reference| reference.serialize())
            .collect::<Vec<_>>();
        let ordered_refs_offset = COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN as u64;
        let ordered_refs_length = ordered_refs_bytes.len() as u64;
        let chain_digest_ref = u32::try_from(ordered_refs_offset + ordered_refs_length)
            .map_err(|_| CoveError::OffsetRange)?;
        let chain_summary_digest_ref =
            u32::try_from(chain_digest_ref as u64 + chain_digest.len() as u64)
                .map_err(|_| CoveError::OffsetRange)?;

        let mut header = [0u8; COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN];
        let mut pos = 0usize;
        put_u32(&mut header, &mut pos, self.delta_chain_profile_id);
        put_u16(
            &mut header,
            &mut pos,
            self.delta_chain_profile_version_major,
        );
        put_u16(
            &mut header,
            &mut pos,
            self.delta_chain_profile_version_minor,
        );
        put_u64(&mut header, &mut pos, self.required_delta_features);
        put_u64(&mut header, &mut pos, self.optional_delta_features);
        put_bytes(&mut header, &mut pos, &self.dataset_id);
        put_bytes(&mut header, &mut pos, &self.base_snapshot_id);
        put_bytes(&mut header, &mut pos, &self.result_snapshot_id);
        put_bytes(&mut header, &mut pos, &self.base_artifact_ref.serialize());
        put_u32(
            &mut header,
            &mut pos,
            u32::try_from(self.ordered_delta_artifact_refs.len())
                .map_err(|_| CoveError::BadSection("too many COVM delta artifact refs".into()))?,
        );
        put_u64(&mut header, &mut pos, ordered_refs_offset);
        put_u64(&mut header, &mut pos, ordered_refs_length);
        put_u16(&mut header, &mut pos, self.chain_digest_algorithm);
        put_u16(
            &mut header,
            &mut pos,
            u16::try_from(chain_digest.len())
                .map_err(|_| CoveError::BadSection("COVM chain digest too long".into()))?,
        );
        put_u32(&mut header, &mut pos, chain_digest_ref);
        put_u16(&mut header, &mut pos, self.chain_summary_kind);
        put_u32(&mut header, &mut pos, self.chain_summary_ref);
        put_u64(&mut header, &mut pos, self.chain_summary_offset);
        put_u64(&mut header, &mut pos, self.chain_summary_length);
        put_u32(&mut header, &mut pos, self.chain_summary_crc32c);
        put_u16(&mut header, &mut pos, self.chain_summary_digest_algorithm);
        put_u16(
            &mut header,
            &mut pos,
            u16::try_from(self.chain_summary_digest.len())
                .map_err(|_| CoveError::BadSection("COVM chain summary digest too long".into()))?,
        );
        put_u32(&mut header, &mut pos, chain_summary_digest_ref);
        put_u32(&mut header, &mut pos, self.effective_schema_fingerprint_ref);
        put_u32(
            &mut header,
            &mut pos,
            self.effective_object_catalog_fingerprint_ref,
        );
        put_u32(
            &mut header,
            &mut pos,
            self.effective_projection_fingerprint_ref,
        );
        put_u32(
            &mut header,
            &mut pos,
            self.effective_semantic_map_fingerprint_ref,
        );
        put_u32(
            &mut header,
            &mut pos,
            self.effective_visibility_fingerprint_ref,
        );
        put_u32(
            &mut header,
            &mut pos,
            self.effective_redaction_fingerprint_ref,
        );
        put_u64(&mut header, &mut pos, self.csn_min);
        put_u64(&mut header, &mut pos, self.csn_max);
        put_i64(&mut header, &mut pos, self.created_at_us);
        let checksum_offset = pos;
        put_u32(&mut header, &mut pos, 0);
        debug_assert_eq!(pos, COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN);

        let crc = checksum::crc32c(&header);
        header[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());

        let file_len = COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN
            .checked_add(ordered_refs_bytes.len())
            .and_then(|len| len.checked_add(chain_digest.len()))
            .and_then(|len| len.checked_add(self.chain_summary_digest.len()))
            .ok_or(CoveError::ArithOverflow)?;
        let mut out = Vec::with_capacity(file_len);
        out.extend_from_slice(&header);
        out.extend_from_slice(&ordered_refs_bytes);
        out.extend_from_slice(&chain_digest);
        out.extend_from_slice(&self.chain_summary_digest);
        Ok(out)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        Self::parse_with_supported_required_delta_features(
            bytes,
            COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES,
        )
    }

    pub fn parse_with_supported_required_delta_features(
        bytes: &[u8],
        supported_required_delta_features: u64,
    ) -> Result<Self, CoveError> {
        if bytes.len() < COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let header = &bytes[..COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN];
        let mut checksum_bytes = [0u8; COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN];
        checksum_bytes.copy_from_slice(header);
        checksum_bytes[COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN - 4..].fill(0);
        let checksum_field = u32::from_le_bytes(
            header
                [COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN - 4..COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN]
                .try_into()
                .unwrap(),
        );
        if checksum::crc32c(&checksum_bytes) != checksum_field {
            return Err(CoveError::ChecksumMismatch);
        }

        let mut pos = 0usize;
        let delta_chain_profile_id = take_u32(header, &mut pos)?;
        let delta_chain_profile_version_major = take_u16(header, &mut pos)?;
        let delta_chain_profile_version_minor = take_u16(header, &mut pos)?;
        let required_delta_features = take_u64(header, &mut pos)?;
        let optional_delta_features = take_u64(header, &mut pos)?;
        let dataset_id = take_array::<16>(header, &mut pos)?;
        let base_snapshot_id = take_array::<16>(header, &mut pos)?;
        let result_snapshot_id = take_array::<16>(header, &mut pos)?;
        let base_artifact_ref = CovmDeltaArtifactRefV1::parse(take_bytes(
            header,
            &mut pos,
            COVM_DELTA_ARTIFACT_REF_LEN,
        )?)?;
        let ordered_delta_count = take_u32(header, &mut pos)?;
        let ordered_delta_artifact_refs_offset = take_u64(header, &mut pos)?;
        let ordered_delta_artifact_refs_length = take_u64(header, &mut pos)?;
        let chain_digest_algorithm = take_u16(header, &mut pos)?;
        let chain_digest_len = take_u16(header, &mut pos)?;
        let chain_digest_ref = take_u32(header, &mut pos)?;
        let chain_summary_kind = take_u16(header, &mut pos)?;
        let chain_summary_ref = take_u32(header, &mut pos)?;
        let chain_summary_offset = take_u64(header, &mut pos)?;
        let chain_summary_length = take_u64(header, &mut pos)?;
        let chain_summary_crc32c = take_u32(header, &mut pos)?;
        let chain_summary_digest_algorithm = take_u16(header, &mut pos)?;
        let chain_summary_digest_len = take_u16(header, &mut pos)?;
        let chain_summary_digest_ref = take_u32(header, &mut pos)?;
        let effective_schema_fingerprint_ref = take_u32(header, &mut pos)?;
        let effective_object_catalog_fingerprint_ref = take_u32(header, &mut pos)?;
        let effective_projection_fingerprint_ref = take_u32(header, &mut pos)?;
        let effective_semantic_map_fingerprint_ref = take_u32(header, &mut pos)?;
        let effective_visibility_fingerprint_ref = take_u32(header, &mut pos)?;
        let effective_redaction_fingerprint_ref = take_u32(header, &mut pos)?;
        let csn_min = take_u64(header, &mut pos)?;
        let csn_max = take_u64(header, &mut pos)?;
        let created_at_us = take_i64(header, &mut pos)?;
        let checksum = take_u32(header, &mut pos)?;
        debug_assert_eq!(pos, COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN);

        if delta_chain_profile_id != COVM_DELTA_CHAIN_PROFILE_ID_V1 {
            return Err(CoveError::BadExtension);
        }
        if delta_chain_profile_version_major != COVM_DELTA_CHAIN_PROFILE_VERSION_MAJOR_V1 {
            return Err(CoveError::BadVersion);
        }

        let ordered_refs_len = usize::try_from(ordered_delta_artifact_refs_length)
            .map_err(|_| CoveError::OffsetRange)?;
        let expected_refs_len = usize::try_from(ordered_delta_count)
            .map_err(|_| CoveError::OffsetRange)?
            .checked_mul(COVM_DELTA_ARTIFACT_REF_LEN)
            .ok_or(CoveError::ArithOverflow)?;
        if ordered_refs_len != expected_refs_len {
            return Err(CoveError::BadSection(
                "COVM ordered delta artifact refs length does not match count".into(),
            ));
        }
        let ordered_refs_offset = usize::try_from(ordered_delta_artifact_refs_offset)
            .map_err(|_| CoveError::OffsetRange)?;
        if ordered_refs_offset != COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN {
            return Err(CoveError::BadSection(
                "COVM ordered delta artifact refs must directly follow the extension header".into(),
            ));
        }
        let ordered_refs_end = ordered_refs_offset
            .checked_add(ordered_refs_len)
            .ok_or(CoveError::ArithOverflow)?;
        if ordered_refs_end > bytes.len() {
            return Err(CoveError::OffsetRange);
        }

        let chain_digest_ref =
            usize::try_from(chain_digest_ref).map_err(|_| CoveError::OffsetRange)?;
        if chain_digest_ref != ordered_refs_end {
            return Err(CoveError::BadSection(
                "COVM chain digest must directly follow ordered delta artifact refs".into(),
            ));
        }
        let chain_digest_len = chain_digest_len as usize;
        let chain_digest_algorithm_value = covm_delta_required_digest_algorithm(
            chain_digest_algorithm,
            "COVM delta chain_digest_algorithm",
        )?;
        let expected_chain_digest_len =
            covm_delta_expected_digest_len(chain_digest_algorithm_value);
        if chain_digest_len != expected_chain_digest_len {
            return Err(CoveError::BadSection(format!(
                "COVM chain digest length must be {expected_chain_digest_len}, got {chain_digest_len}"
            )));
        }
        let chain_digest_end = chain_digest_ref
            .checked_add(chain_digest_len)
            .ok_or(CoveError::ArithOverflow)?;
        if chain_digest_end > bytes.len() {
            return Err(CoveError::OffsetRange);
        }

        let chain_summary_digest_ref =
            usize::try_from(chain_summary_digest_ref).map_err(|_| CoveError::OffsetRange)?;
        if chain_summary_digest_ref != chain_digest_end {
            return Err(CoveError::BadSection(
                "COVM chain summary digest must directly follow chain digest".into(),
            ));
        }
        let chain_summary_digest_len = chain_summary_digest_len as usize;
        let summary_digest_algorithm = covm_delta_optional_digest_algorithm(
            chain_summary_digest_algorithm,
            "COVM delta chain_summary_digest_algorithm",
        )?;
        if let Some(algorithm) = summary_digest_algorithm {
            let expected_len = covm_delta_expected_digest_len(algorithm);
            if chain_summary_digest_len != expected_len {
                return Err(CoveError::BadSection(format!(
                    "COVM chain summary digest length must be {expected_len}, got {chain_summary_digest_len}"
                )));
            }
        } else if chain_summary_digest_len != 0 {
            return Err(CoveError::BadSection(
                "COVM chain summary digest length requires a digest algorithm".into(),
            ));
        }
        let chain_summary_digest_end = chain_summary_digest_ref
            .checked_add(chain_summary_digest_len)
            .ok_or(CoveError::ArithOverflow)?;
        if chain_summary_digest_end != bytes.len() {
            return Err(CoveError::BadSection(
                "COVM delta-chain extension has trailing or overlapping bytes".into(),
            ));
        }

        let mut ordered_delta_artifact_refs = Vec::with_capacity(ordered_delta_count as usize);
        let mut ref_pos = ordered_refs_offset;
        for _ in 0..ordered_delta_count {
            let ref_end = ref_pos
                .checked_add(COVM_DELTA_ARTIFACT_REF_LEN)
                .ok_or(CoveError::ArithOverflow)?;
            ordered_delta_artifact_refs
                .push(CovmDeltaArtifactRefV1::parse(&bytes[ref_pos..ref_end])?);
            ref_pos = ref_end;
        }

        let extension = Self {
            delta_chain_profile_id,
            delta_chain_profile_version_major,
            delta_chain_profile_version_minor,
            required_delta_features,
            optional_delta_features,
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            base_artifact_ref,
            ordered_delta_artifact_refs,
            chain_digest_algorithm,
            chain_digest: bytes[chain_digest_ref..chain_digest_end].to_vec(),
            chain_summary_kind,
            chain_summary_ref,
            chain_summary_offset,
            chain_summary_length,
            chain_summary_crc32c,
            chain_summary_digest_algorithm,
            chain_summary_digest: bytes[chain_summary_digest_ref..chain_summary_digest_end]
                .to_vec(),
            effective_schema_fingerprint_ref,
            effective_object_catalog_fingerprint_ref,
            effective_projection_fingerprint_ref,
            effective_semantic_map_fingerprint_ref,
            effective_visibility_fingerprint_ref,
            effective_redaction_fingerprint_ref,
            csn_min,
            csn_max,
            created_at_us,
            checksum,
        };
        extension
            .validate_with_supported_required_delta_features(supported_required_delta_features)?;
        Ok(extension)
    }

    pub fn validate_with_supported_required_delta_features(
        &self,
        supported_required_delta_features: u64,
    ) -> Result<(), CoveError> {
        self.validate_structure(supported_required_delta_features)?;
        let expected_digest = self.computed_chain_digest()?;
        if self.chain_digest != expected_digest {
            return Err(CoveError::DigestMismatch);
        }
        Ok(())
    }

    pub fn computed_chain_digest(&self) -> Result<Vec<u8>, CoveError> {
        let algorithm = covm_delta_required_digest_algorithm(
            self.chain_digest_algorithm,
            "COVM delta chain_digest_algorithm",
        )?;
        let mut material = Vec::new();
        material.extend_from_slice(b"COVM_DELTA_CHAIN_DIGEST_V1\0");
        material.extend_from_slice(&self.dataset_id);
        material.extend_from_slice(&self.base_snapshot_id);
        self.base_artifact_ref.append_digest_binding(&mut material);
        for reference in &self.ordered_delta_artifact_refs {
            reference.append_digest_binding(&mut material);
        }
        material.extend_from_slice(&self.result_snapshot_id);
        material.extend_from_slice(&self.required_delta_features.to_le_bytes());
        material.extend_from_slice(&self.effective_schema_fingerprint_ref.to_le_bytes());
        material.extend_from_slice(&self.effective_object_catalog_fingerprint_ref.to_le_bytes());
        material.extend_from_slice(&self.effective_projection_fingerprint_ref.to_le_bytes());
        material.extend_from_slice(&self.effective_semantic_map_fingerprint_ref.to_le_bytes());
        material.extend_from_slice(&self.effective_visibility_fingerprint_ref.to_le_bytes());
        material.extend_from_slice(&self.effective_redaction_fingerprint_ref.to_le_bytes());
        compute_digest(algorithm, &material)
    }

    fn validate_structure(&self, supported_required_delta_features: u64) -> Result<(), CoveError> {
        if self.delta_chain_profile_id != COVM_DELTA_CHAIN_PROFILE_ID_V1 {
            return Err(CoveError::BadExtension);
        }
        if self.delta_chain_profile_version_major != COVM_DELTA_CHAIN_PROFILE_VERSION_MAJOR_V1 {
            return Err(CoveError::BadVersion);
        }
        if self.delta_chain_profile_version_minor > COVM_DELTA_CHAIN_PROFILE_VERSION_MINOR_V1 {
            return Err(CoveError::BadVersion);
        }
        let unknown_required = self.required_delta_features & !supported_required_delta_features;
        if unknown_required != 0 {
            return Err(CoveError::UnknownRequiredFeature(unknown_required));
        }
        self.base_artifact_ref.validate_mandatory_digest()?;
        if self.base_artifact_ref.chain_ordinal != 0 {
            return Err(CoveError::BadSection(
                "COVM base artifact ref must have chain_ordinal 0".into(),
            ));
        }
        if self.base_artifact_ref.snapshot_id != self.base_snapshot_id {
            return Err(CoveError::BadSection(
                "COVM base artifact ref snapshot_id must match base_snapshot_id".into(),
            ));
        }
        if self.ordered_delta_artifact_refs.is_empty() {
            return Err(CoveError::BadSection(
                "COVM delta-chain extension requires at least one delta artifact ref".into(),
            ));
        }
        if self.csn_min > self.csn_max {
            return Err(CoveError::BadSection(
                "COVM delta-chain csn_min must be <= csn_max".into(),
            ));
        }

        let mut artifact_ids = BTreeSet::new();
        artifact_ids.insert(self.base_artifact_ref.artifact_id);
        let mut expected_parent_snapshot = self.base_snapshot_id;
        for (idx, reference) in self.ordered_delta_artifact_refs.iter().enumerate() {
            reference.validate_mandatory_digest()?;
            let expected_ordinal = u32::try_from(idx + 1).map_err(|_| CoveError::OffsetRange)?;
            if reference.chain_ordinal != expected_ordinal {
                return Err(CoveError::BadSection(format!(
                    "COVM delta artifact ref ordinal must be dense; expected {expected_ordinal}, got {}",
                    reference.chain_ordinal
                )));
            }
            if reference.parent_snapshot_id != expected_parent_snapshot {
                return Err(CoveError::BadSection(
                    "COVM delta artifact ref parent_snapshot_id does not match selected chain"
                        .into(),
                ));
            }
            if !artifact_ids.insert(reference.artifact_id) {
                return Err(CoveError::BadSection(
                    "COVM delta-chain artifact IDs must be unique".into(),
                ));
            }
            expected_parent_snapshot = reference.snapshot_id;
        }
        if expected_parent_snapshot != self.result_snapshot_id {
            return Err(CoveError::BadSection(
                "COVM ordered delta artifact refs do not end at result_snapshot_id".into(),
            ));
        }

        if self.chain_summary_kind == COVM_DELTA_CHAIN_SUMMARY_KIND_NONE {
            if self.chain_summary_ref != 0
                || self.chain_summary_offset != 0
                || self.chain_summary_length != 0
                || self.chain_summary_crc32c != 0
                || self.chain_summary_digest_algorithm != DigestAlgorithm::None as u16
                || !self.chain_summary_digest.is_empty()
            {
                return Err(CoveError::BadSection(
                    "COVM empty chain summary must not carry refs, ranges, CRC, or digest".into(),
                ));
            }
        } else {
            if self.chain_summary_kind != COVM_DELTA_CHAIN_SUMMARY_KIND_CDS1 {
                return Err(CoveError::BadSection(
                    "COVM unsupported delta chain summary kind".into(),
                ));
            }
            covm_delta_required_digest_algorithm(
                self.chain_summary_digest_algorithm,
                "COVM delta chain_summary_digest_algorithm",
            )?;
            if self.chain_summary_length == 0 || self.chain_summary_digest.is_empty() {
                return Err(CoveError::BadSection(
                    "COVM non-empty chain summary requires length and digest".into(),
                ));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaChainSummaryEntryV1 {
    pub chain_ordinal: u32,
    pub delta_artifact_ref: CovmDeltaArtifactRefV1,
    pub delta_artifact_id: [u8; 16],
    pub required_delta_features: u64,
    pub optional_delta_features: u64,
    pub csn_min: u64,
    pub csn_max: u64,
    pub commit_time_start_us: i64,
    pub commit_time_end_us: i64,
    pub artifact_created_at_us: i64,
    pub first_published_at_us: i64,
    pub selected_snapshot_published_at_us: i64,
    pub time_field_presence_flags: u32,
    pub time_summary_exactness_flags: u32,
    pub source_publish_range_start_us: i64,
    pub source_publish_range_end_us: i64,
    pub scope_summary_ref: u32,
    pub branch_summary_ref: u32,
    pub object_type_summary_ref: u32,
    pub goid_range_summary_ref: u32,
    pub touched_summary_ref: u32,
    pub tombstone_summary_ref: u32,
    pub property_summary_ref: u32,
    pub temporal_role_summary_ref: u32,
    pub delta_header_range_offset: u64,
    pub delta_header_range_length: u64,
    pub hot_summary_range_offset: u64,
    pub hot_summary_range_length: u64,
    pub checksum: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CovmDeltaPruneRequest {
    pub as_of_csn: Option<u64>,
    pub as_of_commit_timestamp_us: Option<i64>,
    pub as_of_valid_time_us: Option<i64>,
    pub source_publish_range_us: Option<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmDeltaPruneSkip {
    pub chain_ordinal: u32,
    pub reason: CovmDeltaPruneReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovmDeltaPruneReason {
    AsOfCsnBeforeDelta,
    AsOfCommitBeforeDelta,
    SourcePublishRangeOutsideDelta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmDeltaPruneDecision {
    pub selected_chain_ordinals: Vec<u32>,
    pub skipped: Vec<CovmDeltaPruneSkip>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CovmDeltaPruneMetrics {
    pub delta_chain_depth: usize,
    pub selected_delta_count: usize,
    pub skipped_delta_count: usize,
    pub delta_artifacts_planned_to_open: usize,
    pub delta_artifacts_skipped_before_open: usize,
    pub as_of_csn_prunes: usize,
    pub commit_time_range_prunes: usize,
    pub source_publish_range_prunes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CovmDeltaReadAmplificationMetrics {
    pub delta_chain_depth: usize,
    pub chain_summary_bytes: usize,
    pub chain_summary_range_requests: usize,
    pub selected_delta_count: usize,
    pub skipped_delta_count: usize,
    pub delta_artifacts_opened: usize,
    pub delta_artifacts_skipped_before_open: usize,
    pub base_ranges_requested: usize,
    pub delta_ranges_requested: usize,
    pub object_store_request_count: usize,
    pub bytes_returned: u64,
    pub touched_set_hits: usize,
    pub touched_set_misses: usize,
    pub tombstone_summary_hits: usize,
    pub source_publish_range_prunes: usize,
    pub commit_time_range_prunes: usize,
    pub valid_time_summary_prunes: usize,
    pub anchor_validations: usize,
    pub patch_rows_applied: usize,
    pub dictionary_alias_resolutions: usize,
    pub materialized_property_count: usize,
    pub base_file_bytes: u64,
    pub total_delta_bytes: u64,
    pub max_patch_rows_since_checkpoint: usize,
    pub point_lookup_artifacts_p95: usize,
    pub metadata_range_requests_before_data: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CovmDeltaReadAmplificationPolicy {
    pub warn_chain_depth: usize,
    pub hard_chain_depth: usize,
    pub checkpoint_patch_rows: usize,
    pub compaction_delta_bytes_percent: u64,
    pub point_lookup_artifacts_p95: usize,
    pub metadata_range_requests_before_data: usize,
    pub pack_small_delta_min_request_count: usize,
    pub pack_small_delta_max_bytes_per_request: u64,
}

impl Default for CovmDeltaReadAmplificationPolicy {
    fn default() -> Self {
        Self {
            warn_chain_depth: 16,
            hard_chain_depth: 64,
            checkpoint_patch_rows: 32,
            compaction_delta_bytes_percent: 20,
            point_lookup_artifacts_p95: 4,
            metadata_range_requests_before_data: 2,
            pack_small_delta_min_request_count: 4,
            pack_small_delta_max_bytes_per_request: 16 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovmDeltaReadAmplificationRecommendation {
    WarnChainDepth,
    RequireOverrideChainDepth,
    RecommendCheckpoint,
    RecommendCompaction,
    RecommendSnapshotLevelIndex,
    RecommendSummaryHoistingOrCompaction,
    RecommendPackingSmallDeltas,
}

impl CovmDeltaPruneDecision {
    pub fn selected_delta_count(&self) -> usize {
        self.selected_chain_ordinals.len()
    }

    pub fn skipped_delta_count(&self) -> usize {
        self.skipped.len()
    }

    pub fn delta_chain_depth(&self) -> usize {
        self.selected_delta_count() + self.skipped_delta_count()
    }

    pub fn skipped_count_by_reason(&self, reason: CovmDeltaPruneReason) -> usize {
        self.skipped
            .iter()
            .filter(|skip| skip.reason == reason)
            .count()
    }

    pub fn metrics(&self) -> CovmDeltaPruneMetrics {
        CovmDeltaPruneMetrics {
            delta_chain_depth: self.delta_chain_depth(),
            selected_delta_count: self.selected_delta_count(),
            skipped_delta_count: self.skipped_delta_count(),
            delta_artifacts_planned_to_open: self.selected_delta_count(),
            delta_artifacts_skipped_before_open: self.skipped_delta_count(),
            as_of_csn_prunes: self
                .skipped_count_by_reason(CovmDeltaPruneReason::AsOfCsnBeforeDelta),
            commit_time_range_prunes: self
                .skipped_count_by_reason(CovmDeltaPruneReason::AsOfCommitBeforeDelta),
            source_publish_range_prunes: self
                .skipped_count_by_reason(CovmDeltaPruneReason::SourcePublishRangeOutsideDelta),
        }
    }
}

impl CovmDeltaReadAmplificationMetrics {
    pub fn from_prune_decision(decision: &CovmDeltaPruneDecision) -> Self {
        let prune_metrics = decision.metrics();
        Self {
            delta_chain_depth: prune_metrics.delta_chain_depth,
            selected_delta_count: prune_metrics.selected_delta_count,
            skipped_delta_count: prune_metrics.skipped_delta_count,
            delta_artifacts_opened: prune_metrics.delta_artifacts_planned_to_open,
            delta_artifacts_skipped_before_open: prune_metrics.delta_artifacts_skipped_before_open,
            source_publish_range_prunes: prune_metrics.source_publish_range_prunes,
            commit_time_range_prunes: prune_metrics.commit_time_range_prunes,
            ..Self::default()
        }
    }

    pub fn recommendations(
        &self,
        policy: CovmDeltaReadAmplificationPolicy,
    ) -> Vec<CovmDeltaReadAmplificationRecommendation> {
        let mut recommendations = Vec::new();
        if self.delta_chain_depth > policy.hard_chain_depth {
            recommendations
                .push(CovmDeltaReadAmplificationRecommendation::RequireOverrideChainDepth);
        } else if self.delta_chain_depth > policy.warn_chain_depth {
            recommendations.push(CovmDeltaReadAmplificationRecommendation::WarnChainDepth);
        }
        if self.max_patch_rows_since_checkpoint > policy.checkpoint_patch_rows {
            recommendations.push(CovmDeltaReadAmplificationRecommendation::RecommendCheckpoint);
        }
        if self.base_file_bytes != 0
            && self.total_delta_bytes.saturating_mul(100)
                > self
                    .base_file_bytes
                    .saturating_mul(policy.compaction_delta_bytes_percent)
        {
            recommendations.push(CovmDeltaReadAmplificationRecommendation::RecommendCompaction);
        }
        if self.point_lookup_artifacts_p95 > policy.point_lookup_artifacts_p95 {
            recommendations
                .push(CovmDeltaReadAmplificationRecommendation::RecommendSnapshotLevelIndex);
        }
        if self.metadata_range_requests_before_data > policy.metadata_range_requests_before_data {
            recommendations.push(
                CovmDeltaReadAmplificationRecommendation::RecommendSummaryHoistingOrCompaction,
            );
        }
        if self.object_store_request_count >= policy.pack_small_delta_min_request_count
            && self.object_store_request_count != 0
            && self.bytes_returned
                <= policy
                    .pack_small_delta_max_bytes_per_request
                    .saturating_mul(self.object_store_request_count as u64)
        {
            recommendations
                .push(CovmDeltaReadAmplificationRecommendation::RecommendPackingSmallDeltas);
        }
        recommendations
    }
}

impl DeltaChainSummaryEntryV1 {
    pub fn serialize(&self) -> Result<[u8; COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN], CoveError> {
        self.validate(COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES)?;
        let mut buf = [0u8; COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN];
        let mut pos = 0usize;
        put_u32(&mut buf, &mut pos, self.chain_ordinal);
        put_bytes(&mut buf, &mut pos, &self.delta_artifact_ref.serialize());
        put_bytes(&mut buf, &mut pos, &self.delta_artifact_id);
        put_u64(&mut buf, &mut pos, self.required_delta_features);
        put_u64(&mut buf, &mut pos, self.optional_delta_features);
        put_u64(&mut buf, &mut pos, self.csn_min);
        put_u64(&mut buf, &mut pos, self.csn_max);
        put_i64(&mut buf, &mut pos, self.commit_time_start_us);
        put_i64(&mut buf, &mut pos, self.commit_time_end_us);
        put_i64(&mut buf, &mut pos, self.artifact_created_at_us);
        put_i64(&mut buf, &mut pos, self.first_published_at_us);
        put_i64(&mut buf, &mut pos, self.selected_snapshot_published_at_us);
        put_u32(&mut buf, &mut pos, self.time_field_presence_flags);
        put_u32(&mut buf, &mut pos, self.time_summary_exactness_flags);
        put_i64(&mut buf, &mut pos, self.source_publish_range_start_us);
        put_i64(&mut buf, &mut pos, self.source_publish_range_end_us);
        put_u32(&mut buf, &mut pos, self.scope_summary_ref);
        put_u32(&mut buf, &mut pos, self.branch_summary_ref);
        put_u32(&mut buf, &mut pos, self.object_type_summary_ref);
        put_u32(&mut buf, &mut pos, self.goid_range_summary_ref);
        put_u32(&mut buf, &mut pos, self.touched_summary_ref);
        put_u32(&mut buf, &mut pos, self.tombstone_summary_ref);
        put_u32(&mut buf, &mut pos, self.property_summary_ref);
        put_u32(&mut buf, &mut pos, self.temporal_role_summary_ref);
        put_u64(&mut buf, &mut pos, self.delta_header_range_offset);
        put_u64(&mut buf, &mut pos, self.delta_header_range_length);
        put_u64(&mut buf, &mut pos, self.hot_summary_range_offset);
        put_u64(&mut buf, &mut pos, self.hot_summary_range_length);
        let checksum_offset = pos;
        put_u32(&mut buf, &mut pos, 0);
        debug_assert_eq!(pos, COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN);

        let crc = checksum::crc32c(&buf);
        buf[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
        Ok(buf)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        Self::parse_with_supported_required_delta_features(
            bytes,
            COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES,
        )
    }

    pub fn parse_with_supported_required_delta_features(
        bytes: &[u8],
        supported_required_delta_features: u64,
    ) -> Result<Self, CoveError> {
        if bytes.len() < COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN {
            return Err(CoveError::BufferTooShort);
        }
        let bytes = &bytes[..COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN];
        let mut checksum_bytes = [0u8; COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN];
        checksum_bytes.copy_from_slice(bytes);
        checksum_bytes[COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN - 4..].fill(0);
        let checksum_field = u32::from_le_bytes(
            bytes[COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN - 4..COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN]
                .try_into()
                .unwrap(),
        );
        if checksum::crc32c(&checksum_bytes) != checksum_field {
            return Err(CoveError::ChecksumMismatch);
        }

        let mut pos = 0usize;
        let chain_ordinal = take_u32(bytes, &mut pos)?;
        let delta_artifact_ref = CovmDeltaArtifactRefV1::parse(take_bytes(
            bytes,
            &mut pos,
            COVM_DELTA_ARTIFACT_REF_LEN,
        )?)?;
        let delta_artifact_id = take_array::<16>(bytes, &mut pos)?;
        let required_delta_features = take_u64(bytes, &mut pos)?;
        let optional_delta_features = take_u64(bytes, &mut pos)?;
        let csn_min = take_u64(bytes, &mut pos)?;
        let csn_max = take_u64(bytes, &mut pos)?;
        let commit_time_start_us = take_i64(bytes, &mut pos)?;
        let commit_time_end_us = take_i64(bytes, &mut pos)?;
        let artifact_created_at_us = take_i64(bytes, &mut pos)?;
        let first_published_at_us = take_i64(bytes, &mut pos)?;
        let selected_snapshot_published_at_us = take_i64(bytes, &mut pos)?;
        let time_field_presence_flags = take_u32(bytes, &mut pos)?;
        let time_summary_exactness_flags = take_u32(bytes, &mut pos)?;
        let source_publish_range_start_us = take_i64(bytes, &mut pos)?;
        let source_publish_range_end_us = take_i64(bytes, &mut pos)?;
        let scope_summary_ref = take_u32(bytes, &mut pos)?;
        let branch_summary_ref = take_u32(bytes, &mut pos)?;
        let object_type_summary_ref = take_u32(bytes, &mut pos)?;
        let goid_range_summary_ref = take_u32(bytes, &mut pos)?;
        let touched_summary_ref = take_u32(bytes, &mut pos)?;
        let tombstone_summary_ref = take_u32(bytes, &mut pos)?;
        let property_summary_ref = take_u32(bytes, &mut pos)?;
        let temporal_role_summary_ref = take_u32(bytes, &mut pos)?;
        let delta_header_range_offset = take_u64(bytes, &mut pos)?;
        let delta_header_range_length = take_u64(bytes, &mut pos)?;
        let hot_summary_range_offset = take_u64(bytes, &mut pos)?;
        let hot_summary_range_length = take_u64(bytes, &mut pos)?;
        let checksum = take_u32(bytes, &mut pos)?;
        debug_assert_eq!(pos, COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN);

        let entry = Self {
            chain_ordinal,
            delta_artifact_ref,
            delta_artifact_id,
            required_delta_features,
            optional_delta_features,
            csn_min,
            csn_max,
            commit_time_start_us,
            commit_time_end_us,
            artifact_created_at_us,
            first_published_at_us,
            selected_snapshot_published_at_us,
            time_field_presence_flags,
            time_summary_exactness_flags,
            source_publish_range_start_us,
            source_publish_range_end_us,
            scope_summary_ref,
            branch_summary_ref,
            object_type_summary_ref,
            goid_range_summary_ref,
            touched_summary_ref,
            tombstone_summary_ref,
            property_summary_ref,
            temporal_role_summary_ref,
            delta_header_range_offset,
            delta_header_range_length,
            hot_summary_range_offset,
            hot_summary_range_length,
            checksum,
        };
        entry.validate(supported_required_delta_features)?;
        Ok(entry)
    }

    fn validate(&self, supported_required_delta_features: u64) -> Result<(), CoveError> {
        let unknown_required = self.required_delta_features & !supported_required_delta_features;
        if unknown_required != 0 {
            return Err(CoveError::UnknownRequiredFeature(unknown_required));
        }
        self.delta_artifact_ref.validate_mandatory_digest()?;
        if self.chain_ordinal == 0 || self.delta_artifact_ref.chain_ordinal != self.chain_ordinal {
            return Err(CoveError::BadSection(
                "COVM delta summary entry ordinal must match delta artifact ref".into(),
            ));
        }
        if self.delta_artifact_id != self.delta_artifact_ref.artifact_id {
            return Err(CoveError::BadSection(
                "COVM delta summary entry artifact ID disagrees with artifact ref".into(),
            ));
        }
        if self.csn_min > self.csn_max {
            return Err(CoveError::BadSection(
                "COVM delta summary csn_min must be <= csn_max".into(),
            ));
        }
        let known_time_flags = DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT
            | DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT;
        if self.time_field_presence_flags & !known_time_flags != 0 {
            return Err(CoveError::BadSection(
                "COVM delta summary contains unknown time presence flags".into(),
            ));
        }
        let known_exactness_flags = DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE
            | DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_EXACT;
        if self.time_summary_exactness_flags & !known_exactness_flags != 0 {
            return Err(CoveError::BadSection(
                "COVM delta summary contains unknown time exactness flags".into(),
            ));
        }
        if self.time_field_presence_flags & DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT != 0 {
            if self.commit_time_start_us > self.commit_time_end_us {
                return Err(CoveError::BadSection(
                    "COVM delta summary commit-time range is inverted".into(),
                ));
            }
        } else if self.commit_time_start_us != 0 || self.commit_time_end_us != 0 {
            return Err(CoveError::BadSection(
                "COVM delta summary commit-time fields require presence flag".into(),
            ));
        }
        if self.time_field_presence_flags & DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT != 0 {
            if self.source_publish_range_start_us > self.source_publish_range_end_us {
                return Err(CoveError::BadSection(
                    "COVM delta summary source-publish range is inverted".into(),
                ));
            }
        } else if self.source_publish_range_start_us != 0 || self.source_publish_range_end_us != 0 {
            return Err(CoveError::BadSection(
                "COVM delta summary source-publish fields require presence flag".into(),
            ));
        }
        if self.time_field_presence_flags & DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT == 0
            && self.time_summary_exactness_flags != 0
        {
            return Err(CoveError::BadSection(
                "COVM delta summary source-publish exactness requires source-publish fields".into(),
            ));
        }
        Ok(())
    }

    fn source_publish_summary_proves_absence(&self) -> bool {
        self.time_summary_exactness_flags
            & (DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE
                | DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_EXACT)
            != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmDeltaChainSummaryV1 {
    pub magic: [u8; 4],
    pub version_major: u16,
    pub version_minor: u16,
    pub header_len: u16,
    pub flags: u32,
    pub dataset_id: [u8; 16],
    pub result_snapshot_id: [u8; 16],
    pub chain_digest_algorithm: u16,
    pub chain_digest: Vec<u8>,
    pub chain_digest_ref: u32,
    pub delta_summary_count: u32,
    pub object_type_summary_count: u32,
    pub branch_summary_count: u32,
    pub temporal_role_summary_count: u32,
    pub delta_summaries_offset: u64,
    pub object_type_summaries_offset: u64,
    pub branch_summaries_offset: u64,
    pub temporal_role_summaries_offset: u64,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub delta_summaries: Vec<DeltaChainSummaryEntryV1>,
    pub object_type_summaries: Vec<u8>,
    pub branch_summaries: Vec<u8>,
    pub temporal_role_summaries: Vec<u8>,
    pub payload: Vec<u8>,
    pub checksum: u32,
}

impl CovmDeltaChainSummaryV1 {
    pub fn new(
        dataset_id: [u8; 16],
        result_snapshot_id: [u8; 16],
        chain_digest_algorithm: u16,
        chain_digest: Vec<u8>,
        delta_summaries: Vec<DeltaChainSummaryEntryV1>,
    ) -> Self {
        Self {
            magic: COVM_DELTA_CHAIN_SUMMARY_MAGIC,
            version_major: COVM_DELTA_CHAIN_SUMMARY_VERSION_MAJOR_V1,
            version_minor: COVM_DELTA_CHAIN_SUMMARY_VERSION_MINOR_V1,
            header_len: COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN,
            flags: 0,
            dataset_id,
            result_snapshot_id,
            chain_digest_algorithm,
            chain_digest,
            chain_digest_ref: 0,
            delta_summary_count: 0,
            object_type_summary_count: 0,
            branch_summary_count: 0,
            temporal_role_summary_count: 0,
            delta_summaries_offset: 0,
            object_type_summaries_offset: 0,
            branch_summaries_offset: 0,
            temporal_role_summaries_offset: 0,
            payload_offset: 0,
            payload_length: 0,
            delta_summaries,
            object_type_summaries: Vec::new(),
            branch_summaries: Vec::new(),
            temporal_role_summaries: Vec::new(),
            payload: Vec::new(),
            checksum: 0,
        }
    }

    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        self.validate_structure(COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES)?;
        let algorithm = covm_delta_required_digest_algorithm(
            self.chain_digest_algorithm,
            "COVM delta chain summary chain_digest_algorithm",
        )?;
        let expected_digest_len = covm_delta_expected_digest_len(algorithm);
        if self.chain_digest.len() != expected_digest_len {
            return Err(CoveError::BadSection(format!(
                "COVM delta chain summary chain digest length must be {expected_digest_len}, got {}",
                self.chain_digest.len()
            )));
        }

        let delta_summaries_bytes = self
            .delta_summaries
            .iter()
            .map(DeltaChainSummaryEntryV1::serialize)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        let chain_digest_ref = COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as u32;
        let delta_summaries_offset = u64::from(chain_digest_ref) + self.chain_digest.len() as u64;
        let object_type_summaries_offset =
            delta_summaries_offset + delta_summaries_bytes.len() as u64;
        let branch_summaries_offset =
            object_type_summaries_offset + self.object_type_summaries.len() as u64;
        let temporal_role_summaries_offset =
            branch_summaries_offset + self.branch_summaries.len() as u64;
        let payload_offset =
            temporal_role_summaries_offset + self.temporal_role_summaries.len() as u64;
        let payload_length = self.payload.len() as u64;

        let mut header = [0u8; COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize];
        let mut pos = 0usize;
        put_bytes(&mut header, &mut pos, &self.magic);
        put_u16(&mut header, &mut pos, self.version_major);
        put_u16(&mut header, &mut pos, self.version_minor);
        put_u16(&mut header, &mut pos, self.header_len);
        put_u32(&mut header, &mut pos, self.flags);
        put_bytes(&mut header, &mut pos, &self.dataset_id);
        put_bytes(&mut header, &mut pos, &self.result_snapshot_id);
        put_u16(&mut header, &mut pos, self.chain_digest_algorithm);
        put_u16(
            &mut header,
            &mut pos,
            u16::try_from(self.chain_digest.len())
                .map_err(|_| CoveError::BadSection("COVM summary digest too long".into()))?,
        );
        put_u32(&mut header, &mut pos, chain_digest_ref);
        put_u32(
            &mut header,
            &mut pos,
            u32::try_from(self.delta_summaries.len())
                .map_err(|_| CoveError::BadSection("too many COVM delta summary entries".into()))?,
        );
        put_u32(&mut header, &mut pos, self.object_type_summary_count);
        put_u32(&mut header, &mut pos, self.branch_summary_count);
        put_u32(&mut header, &mut pos, self.temporal_role_summary_count);
        put_u64(&mut header, &mut pos, delta_summaries_offset);
        put_u64(&mut header, &mut pos, object_type_summaries_offset);
        put_u64(&mut header, &mut pos, branch_summaries_offset);
        put_u64(&mut header, &mut pos, temporal_role_summaries_offset);
        put_u64(&mut header, &mut pos, payload_offset);
        put_u64(&mut header, &mut pos, payload_length);
        let checksum_offset = pos;
        put_u32(&mut header, &mut pos, 0);
        debug_assert_eq!(pos, COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize);
        let crc = checksum::crc32c(&header);
        header[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());

        let file_len = (payload_offset as usize)
            .checked_add(self.payload.len())
            .ok_or(CoveError::ArithOverflow)?;
        let mut out = Vec::with_capacity(file_len);
        out.extend_from_slice(&header);
        out.extend_from_slice(&self.chain_digest);
        out.extend_from_slice(&delta_summaries_bytes);
        out.extend_from_slice(&self.object_type_summaries);
        out.extend_from_slice(&self.branch_summaries);
        out.extend_from_slice(&self.temporal_role_summaries);
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        Self::parse_with_supported_required_delta_features(
            bytes,
            COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES,
        )
    }

    pub fn parse_with_supported_required_delta_features(
        bytes: &[u8],
        supported_required_delta_features: u64,
    ) -> Result<Self, CoveError> {
        if bytes.len() < COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize {
            return Err(CoveError::BufferTooShort);
        }
        let header = &bytes[..COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize];
        let mut checksum_bytes = [0u8; COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize];
        checksum_bytes.copy_from_slice(header);
        checksum_bytes[COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize - 4..].fill(0);
        let checksum_field = u32::from_le_bytes(
            header[COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize - 4
                ..COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize]
                .try_into()
                .unwrap(),
        );
        if checksum::crc32c(&checksum_bytes) != checksum_field {
            return Err(CoveError::ChecksumMismatch);
        }

        let mut pos = 0usize;
        let magic = take_array::<4>(header, &mut pos)?;
        let version_major = take_u16(header, &mut pos)?;
        let version_minor = take_u16(header, &mut pos)?;
        let header_len = take_u16(header, &mut pos)?;
        let flags = take_u32(header, &mut pos)?;
        let dataset_id = take_array::<16>(header, &mut pos)?;
        let result_snapshot_id = take_array::<16>(header, &mut pos)?;
        let chain_digest_algorithm = take_u16(header, &mut pos)?;
        let chain_digest_len = take_u16(header, &mut pos)?;
        let chain_digest_ref = take_u32(header, &mut pos)?;
        let delta_summary_count = take_u32(header, &mut pos)?;
        let object_type_summary_count = take_u32(header, &mut pos)?;
        let branch_summary_count = take_u32(header, &mut pos)?;
        let temporal_role_summary_count = take_u32(header, &mut pos)?;
        let delta_summaries_offset = take_u64(header, &mut pos)?;
        let object_type_summaries_offset = take_u64(header, &mut pos)?;
        let branch_summaries_offset = take_u64(header, &mut pos)?;
        let temporal_role_summaries_offset = take_u64(header, &mut pos)?;
        let payload_offset = take_u64(header, &mut pos)?;
        let payload_length = take_u64(header, &mut pos)?;
        let checksum = take_u32(header, &mut pos)?;
        debug_assert_eq!(pos, COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize);

        if magic != COVM_DELTA_CHAIN_SUMMARY_MAGIC {
            return Err(CoveError::BadMagic);
        }
        if version_major != COVM_DELTA_CHAIN_SUMMARY_VERSION_MAJOR_V1 {
            return Err(CoveError::BadVersion);
        }
        if version_minor > COVM_DELTA_CHAIN_SUMMARY_VERSION_MINOR_V1 {
            return Err(CoveError::BadVersion);
        }
        if header_len != COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN {
            return Err(CoveError::BadSection(format!(
                "COVM delta chain summary header_len must be {}, got {header_len}",
                COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN
            )));
        }

        let algorithm = covm_delta_required_digest_algorithm(
            chain_digest_algorithm,
            "COVM delta chain summary chain_digest_algorithm",
        )?;
        let chain_digest_len = chain_digest_len as usize;
        let expected_chain_digest_len = covm_delta_expected_digest_len(algorithm);
        if chain_digest_len != expected_chain_digest_len {
            return Err(CoveError::BadSection(format!(
                "COVM delta chain summary chain digest length must be {expected_chain_digest_len}, got {chain_digest_len}"
            )));
        }
        let chain_digest_ref_usize =
            usize::try_from(chain_digest_ref).map_err(|_| CoveError::OffsetRange)?;
        if chain_digest_ref_usize != COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize {
            return Err(CoveError::BadSection(
                "COVM delta chain summary digest must directly follow the header".into(),
            ));
        }
        let chain_digest_end = chain_digest_ref_usize
            .checked_add(chain_digest_len)
            .ok_or(CoveError::ArithOverflow)?;
        if chain_digest_end > bytes.len() {
            return Err(CoveError::OffsetRange);
        }

        let delta_summaries_offset_usize =
            usize::try_from(delta_summaries_offset).map_err(|_| CoveError::OffsetRange)?;
        if delta_summaries_offset_usize != chain_digest_end {
            return Err(CoveError::BadSection(
                "COVM delta summaries must directly follow the chain digest".into(),
            ));
        }
        let delta_summaries_len = usize::try_from(delta_summary_count)
            .map_err(|_| CoveError::OffsetRange)?
            .checked_mul(COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN)
            .ok_or(CoveError::ArithOverflow)?;
        let delta_summaries_end = delta_summaries_offset_usize
            .checked_add(delta_summaries_len)
            .ok_or(CoveError::ArithOverflow)?;
        if delta_summaries_end > bytes.len() {
            return Err(CoveError::OffsetRange);
        }

        let object_type_summaries_offset_usize =
            usize::try_from(object_type_summaries_offset).map_err(|_| CoveError::OffsetRange)?;
        let branch_summaries_offset_usize =
            usize::try_from(branch_summaries_offset).map_err(|_| CoveError::OffsetRange)?;
        let temporal_role_summaries_offset_usize =
            usize::try_from(temporal_role_summaries_offset).map_err(|_| CoveError::OffsetRange)?;
        let payload_offset_usize =
            usize::try_from(payload_offset).map_err(|_| CoveError::OffsetRange)?;
        let payload_length_usize =
            usize::try_from(payload_length).map_err(|_| CoveError::OffsetRange)?;
        if object_type_summaries_offset_usize != delta_summaries_end
            || branch_summaries_offset_usize < object_type_summaries_offset_usize
            || temporal_role_summaries_offset_usize < branch_summaries_offset_usize
            || payload_offset_usize < temporal_role_summaries_offset_usize
        {
            return Err(CoveError::BadSection(
                "COVM delta chain summary regions are not in canonical order".into(),
            ));
        }
        let payload_end = payload_offset_usize
            .checked_add(payload_length_usize)
            .ok_or(CoveError::ArithOverflow)?;
        if payload_end != bytes.len() {
            return Err(CoveError::BadSection(
                "COVM delta chain summary has trailing or overlapping bytes".into(),
            ));
        }

        let mut delta_summaries = Vec::with_capacity(delta_summary_count as usize);
        let mut entry_pos = delta_summaries_offset_usize;
        for _ in 0..delta_summary_count {
            let entry_end = entry_pos
                .checked_add(COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN)
                .ok_or(CoveError::ArithOverflow)?;
            delta_summaries.push(
                DeltaChainSummaryEntryV1::parse_with_supported_required_delta_features(
                    &bytes[entry_pos..entry_end],
                    supported_required_delta_features,
                )?,
            );
            entry_pos = entry_end;
        }

        let summary = Self {
            magic,
            version_major,
            version_minor,
            header_len,
            flags,
            dataset_id,
            result_snapshot_id,
            chain_digest_algorithm,
            chain_digest: bytes[chain_digest_ref_usize..chain_digest_end].to_vec(),
            chain_digest_ref,
            delta_summary_count,
            object_type_summary_count,
            branch_summary_count,
            temporal_role_summary_count,
            delta_summaries_offset,
            object_type_summaries_offset,
            branch_summaries_offset,
            temporal_role_summaries_offset,
            payload_offset,
            payload_length,
            delta_summaries,
            object_type_summaries: bytes
                [object_type_summaries_offset_usize..branch_summaries_offset_usize]
                .to_vec(),
            branch_summaries: bytes
                [branch_summaries_offset_usize..temporal_role_summaries_offset_usize]
                .to_vec(),
            temporal_role_summaries: bytes
                [temporal_role_summaries_offset_usize..payload_offset_usize]
                .to_vec(),
            payload: bytes[payload_offset_usize..payload_end].to_vec(),
            checksum,
        };
        summary.validate_structure(supported_required_delta_features)?;
        Ok(summary)
    }

    pub fn validate_against_delta_chain_extension(
        &self,
        extension: &CovmDeltaChainExtensionV1,
    ) -> Result<(), CoveError> {
        self.validate_structure(COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES)?;
        if self.dataset_id != extension.dataset_id
            || self.result_snapshot_id != extension.result_snapshot_id
            || self.chain_digest_algorithm != extension.chain_digest_algorithm
            || self.chain_digest != extension.chain_digest
            || self.delta_summaries.len() != extension.ordered_delta_artifact_refs.len()
        {
            return Err(CoveError::SidecarStale);
        }
        for (entry, reference) in self
            .delta_summaries
            .iter()
            .zip(extension.ordered_delta_artifact_refs.iter())
        {
            if entry.delta_artifact_ref != *reference {
                return Err(CoveError::SidecarStale);
            }
        }
        Ok(())
    }

    pub fn prune_delta_chain(
        &self,
        request: CovmDeltaPruneRequest,
    ) -> Result<CovmDeltaPruneDecision, CoveError> {
        self.validate_structure(COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES)?;
        if let Some((source_publish_start_us, source_publish_end_us)) =
            request.source_publish_range_us
        {
            if source_publish_start_us > source_publish_end_us {
                return Err(CoveError::BadSection(
                    "COVM source-publish pruning range is inverted".into(),
                ));
            }
        }
        let mut selected_chain_ordinals = Vec::new();
        let mut skipped = Vec::new();

        for entry in &self.delta_summaries {
            if let Some(as_of_csn) = request.as_of_csn {
                if as_of_csn < entry.csn_min {
                    skipped.push(CovmDeltaPruneSkip {
                        chain_ordinal: entry.chain_ordinal,
                        reason: CovmDeltaPruneReason::AsOfCsnBeforeDelta,
                    });
                    continue;
                }
            }

            if let Some(as_of_commit_timestamp_us) = request.as_of_commit_timestamp_us {
                if entry.time_field_presence_flags & DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT == 0 {
                    return Err(CoveError::BadSection(
                        "COVM commit-time pruning requires commit-time summary fields".into(),
                    ));
                }
                if as_of_commit_timestamp_us < entry.commit_time_start_us {
                    skipped.push(CovmDeltaPruneSkip {
                        chain_ordinal: entry.chain_ordinal,
                        reason: CovmDeltaPruneReason::AsOfCommitBeforeDelta,
                    });
                    continue;
                }
            }

            if let Some((source_publish_start_us, source_publish_end_us)) =
                request.source_publish_range_us
            {
                if entry.time_field_presence_flags & DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT
                    == 0
                {
                    return Err(CoveError::BadSection(
                        "COVM source-publish pruning requires source-publish summary fields".into(),
                    ));
                }
                if !entry.source_publish_summary_proves_absence() {
                    return Err(CoveError::BadSection(
                        "COVM source-publish pruning requires conservative source-publish summary exactness"
                            .into(),
                    ));
                }
                if source_publish_end_us < entry.source_publish_range_start_us
                    || source_publish_start_us > entry.source_publish_range_end_us
                {
                    skipped.push(CovmDeltaPruneSkip {
                        chain_ordinal: entry.chain_ordinal,
                        reason: CovmDeltaPruneReason::SourcePublishRangeOutsideDelta,
                    });
                    continue;
                }
            }

            selected_chain_ordinals.push(entry.chain_ordinal);
        }

        Ok(CovmDeltaPruneDecision {
            selected_chain_ordinals,
            skipped,
        })
    }

    pub fn read_amplification_metrics(
        &self,
        decision: &CovmDeltaPruneDecision,
    ) -> CovmDeltaReadAmplificationMetrics {
        let mut metrics = CovmDeltaReadAmplificationMetrics::from_prune_decision(decision);
        metrics.chain_summary_bytes = self.encoded_len();
        metrics.chain_summary_range_requests = usize::from(metrics.chain_summary_bytes != 0);
        metrics.base_ranges_requested = 1;
        metrics.delta_ranges_requested = metrics.selected_delta_count;
        metrics.object_store_request_count = metrics
            .chain_summary_range_requests
            .saturating_add(metrics.base_ranges_requested)
            .saturating_add(metrics.delta_ranges_requested);
        metrics
    }

    pub fn encoded_len(&self) -> usize {
        COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize
            + self.chain_digest.len()
            + self.delta_summaries.len() * COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN
            + self.object_type_summaries.len()
            + self.branch_summaries.len()
            + self.temporal_role_summaries.len()
            + self.payload.len()
    }

    fn validate_structure(&self, supported_required_delta_features: u64) -> Result<(), CoveError> {
        if self.magic != COVM_DELTA_CHAIN_SUMMARY_MAGIC {
            return Err(CoveError::BadMagic);
        }
        if self.version_major != COVM_DELTA_CHAIN_SUMMARY_VERSION_MAJOR_V1 {
            return Err(CoveError::BadVersion);
        }
        if self.version_minor > COVM_DELTA_CHAIN_SUMMARY_VERSION_MINOR_V1 {
            return Err(CoveError::BadVersion);
        }
        if self.header_len != COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN {
            return Err(CoveError::BadSection(format!(
                "COVM delta chain summary header_len must be {}, got {}",
                COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN, self.header_len
            )));
        }
        covm_delta_required_digest_algorithm(
            self.chain_digest_algorithm,
            "COVM delta chain summary chain_digest_algorithm",
        )?;
        if self.delta_summaries.is_empty() {
            return Err(CoveError::BadSection(
                "COVM delta chain summary requires at least one delta entry".into(),
            ));
        }
        let mut previous_csn_max = None;
        let mut previous_commit_time_start_us = None;
        let mut previous_commit_time_end_us = None;
        for (idx, entry) in self.delta_summaries.iter().enumerate() {
            entry.validate(supported_required_delta_features)?;
            let expected_ordinal = u32::try_from(idx + 1).map_err(|_| CoveError::OffsetRange)?;
            if entry.chain_ordinal != expected_ordinal {
                return Err(CoveError::BadSection(format!(
                    "COVM delta summary entries must be dense and sorted; expected {expected_ordinal}, got {}",
                    entry.chain_ordinal
                )));
            }
            if let Some(previous_csn_max) = previous_csn_max {
                if entry.csn_min <= previous_csn_max {
                    return Err(CoveError::BadSection(
                        "COVM delta summary CSN ranges must be strictly append-only".into(),
                    ));
                }
            }
            previous_csn_max = Some(entry.csn_max);
            if entry.time_field_presence_flags & DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT != 0 {
                if let (Some(previous_start), Some(previous_end)) =
                    (previous_commit_time_start_us, previous_commit_time_end_us)
                {
                    if entry.commit_time_start_us < previous_start
                        || entry.commit_time_end_us < previous_end
                    {
                        return Err(CoveError::BadSection(
                            "COVM delta summary commit-time ranges must be monotonic".into(),
                        ));
                    }
                }
                previous_commit_time_start_us = Some(entry.commit_time_start_us);
                previous_commit_time_end_us = Some(entry.commit_time_end_us);
            }
        }
        if self.object_type_summary_count == 0 && !self.object_type_summaries.is_empty()
            || self.branch_summary_count == 0 && !self.branch_summaries.is_empty()
            || self.temporal_role_summary_count == 0 && !self.temporal_role_summaries.is_empty()
        {
            return Err(CoveError::BadSection(
                "COVM delta summary raw summary bytes require a non-zero summary count".into(),
            ));
        }
        Ok(())
    }
}

pub fn validate_selected_delta_chain(
    extension: &CovmDeltaChainExtensionV1,
    summary: Option<&CovmDeltaChainSummaryV1>,
    ordered_delta_artifacts: &[&[u8]],
) -> Result<Vec<CoveDeltaFile>, CoveError> {
    extension.validate_with_supported_required_delta_features(
        COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES,
    )?;
    if extension.chain_summary_kind != COVM_DELTA_CHAIN_SUMMARY_KIND_NONE && summary.is_none() {
        return Err(CoveError::BadSection(
            "selected COVM delta chain requires declared chain summary bytes".into(),
        ));
    }
    if let Some(summary) = summary {
        summary.validate_against_delta_chain_extension(extension)?;
    }
    if ordered_delta_artifacts.len() != extension.ordered_delta_artifact_refs.len() {
        return Err(CoveError::BadSection(
            "selected COVM delta chain artifact count does not match extension".into(),
        ));
    }

    let mut parsed = Vec::with_capacity(ordered_delta_artifacts.len());
    let mut expected_parent_snapshot = extension.base_snapshot_id;
    for (idx, (bytes, reference)) in ordered_delta_artifacts
        .iter()
        .zip(extension.ordered_delta_artifact_refs.iter())
        .enumerate()
    {
        let file = CoveDeltaFile::parse(bytes)?;
        let expected_ordinal = u32::try_from(idx + 1).map_err(|_| CoveError::OffsetRange)?;
        if reference.chain_ordinal != expected_ordinal
            || file.header.chain_ordinal != expected_ordinal
            || file.header.dataset_id != extension.dataset_id
            || file.header.delta_artifact_id != reference.artifact_id
            || file.header.snapshot_id != reference.snapshot_id
            || file.header.parent_snapshot_id != reference.parent_snapshot_id
            || file.header.parent_snapshot_id != expected_parent_snapshot
            || file.footer.footer_crc32c != reference.footer_crc32c
            || file.header.required_delta_features & !extension.required_delta_features != 0
        {
            return Err(CoveError::SidecarStale);
        }

        verify_delta_artifact_ref_bytes(reference, bytes, "selected delta artifact")?;
        if let Some(summary) = summary {
            validate_delta_summary_entry_against_delta_file(&summary.delta_summaries[idx], &file)?;
        }

        expected_parent_snapshot = file.header.snapshot_id;
        parsed.push(file);
    }
    if expected_parent_snapshot != extension.result_snapshot_id {
        return Err(CoveError::SidecarStale);
    }
    validate_delta_lineage_parent_refs_against_selected_artifacts(extension, &parsed)?;
    Ok(parsed)
}

pub fn validate_selected_delta_chain_with_summary_bytes(
    extension: &CovmDeltaChainExtensionV1,
    summary_bytes: Option<&[u8]>,
    ordered_delta_artifacts: &[&[u8]],
) -> Result<Vec<CoveDeltaFile>, CoveError> {
    let summary = match summary_bytes {
        Some(bytes) => Some(validate_delta_chain_summary_bytes_against_extension(
            extension, bytes,
        )?),
        None => None,
    };
    validate_selected_delta_chain(extension, summary.as_ref(), ordered_delta_artifacts)
}

fn validate_delta_chain_summary_bytes_against_extension(
    extension: &CovmDeltaChainExtensionV1,
    summary_bytes: &[u8],
) -> Result<CovmDeltaChainSummaryV1, CoveError> {
    if extension.chain_summary_kind != COVM_DELTA_CHAIN_SUMMARY_KIND_NONE {
        if extension.chain_summary_length != summary_bytes.len() as u64 {
            return Err(CoveError::SidecarStale);
        }
        if checksum::crc32c(summary_bytes) != extension.chain_summary_crc32c {
            return Err(CoveError::ChecksumMismatch);
        }
        let algorithm = covm_delta_required_digest_algorithm(
            extension.chain_summary_digest_algorithm,
            "COVM delta chain_summary_digest_algorithm",
        )?;
        let expected_len = covm_delta_expected_digest_len(algorithm);
        let digest = compute_digest(algorithm, summary_bytes)?;
        if extension.chain_summary_digest.len() != expected_len
            || digest.as_slice() != extension.chain_summary_digest.as_slice()
        {
            return Err(CoveError::DigestMismatch);
        }
    }
    let summary = CovmDeltaChainSummaryV1::parse(summary_bytes)?;
    summary.validate_against_delta_chain_extension(extension)?;
    Ok(summary)
}

pub fn validate_selected_delta_chain_with_base(
    extension: &CovmDeltaChainExtensionV1,
    summary: Option<&CovmDeltaChainSummaryV1>,
    base_artifact: Option<&[u8]>,
    ordered_delta_artifacts: &[&[u8]],
) -> Result<Vec<CoveDeltaFile>, CoveError> {
    extension.validate_with_supported_required_delta_features(
        COVM_DELTA_CHAIN_SUPPORTED_REQUIRED_FEATURES,
    )?;
    let base_artifact = base_artifact.ok_or_else(|| {
        CoveError::BadSection("selected COVM delta chain requires base artifact bytes".into())
    })?;
    verify_delta_artifact_ref_bytes(&extension.base_artifact_ref, base_artifact, "base artifact")?;
    validate_selected_delta_chain(extension, summary, ordered_delta_artifacts)
}

fn verify_delta_artifact_ref_bytes(
    reference: &CovmDeltaArtifactRefV1,
    bytes: &[u8],
    label: &str,
) -> Result<(), CoveError> {
    reference.validate_mandatory_digest()?;
    if bytes.len() as u64 != reference.file_len {
        return Err(CoveError::SidecarStale);
    }
    let algorithm =
        covm_delta_required_digest_algorithm(reference.digest_algorithm, "COVM artifact ref")?;
    let expected_len = covm_delta_expected_digest_len(algorithm);
    let digest = compute_digest(algorithm, bytes)?;
    if digest.as_slice() != &reference.digest[..expected_len] {
        return Err(CoveError::DigestMismatch);
    }
    if reference.chain_ordinal == 0 && reference.parent_snapshot_id != [0; 16] {
        return Err(CoveError::BadSection(format!(
            "COVM {label} base ref must not carry a parent snapshot"
        )));
    }
    Ok(())
}

fn validate_delta_lineage_parent_refs_against_selected_artifacts(
    extension: &CovmDeltaChainExtensionV1,
    parsed_deltas: &[CoveDeltaFile],
) -> Result<(), CoveError> {
    for (idx, delta) in parsed_deltas.iter().enumerate() {
        let expected_parent_ref = if idx == 0 {
            &extension.base_artifact_ref
        } else {
            &extension.ordered_delta_artifact_refs[idx - 1]
        };
        let lineage_parent = delta
            .parent_refs
            .iter()
            .find(|parent| parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0)
            .ok_or_else(|| {
                CoveError::BadSection("selected delta lacks lineage parent ref".into())
            })?;
        validate_delta_lineage_parent_ref_against_artifact_ref(
            lineage_parent,
            expected_parent_ref,
        )?;
    }
    Ok(())
}

fn validate_delta_lineage_parent_ref_against_artifact_ref(
    parent: &DeltaParentRefV1,
    expected: &CovmDeltaArtifactRefV1,
) -> Result<(), CoveError> {
    if parent.digest_ref == DELTA_REF_NONE {
        return Err(CoveError::BadSection(
            "COVEDELTA lineage parent requires digest_ref".into(),
        ));
    }
    if parent.artifact_id != expected.artifact_id
        || parent.snapshot_id != expected.snapshot_id
        || parent.file_len != expected.file_len
        || parent.footer_crc32c != expected.footer_crc32c
        || parent.digest_algorithm != expected.digest_algorithm
        || parent.digest_len != expected.digest_len
        || parent.uri_ref != expected.uri_ref
    {
        return Err(CoveError::SidecarStale);
    }
    Ok(())
}

fn validate_delta_summary_entry_against_delta_file(
    entry: &DeltaChainSummaryEntryV1,
    file: &CoveDeltaFile,
) -> Result<(), CoveError> {
    if entry.required_delta_features != file.header.required_delta_features
        || entry.optional_delta_features != file.header.optional_delta_features
        || entry.csn_min > file.header.csn_min
        || entry.csn_max < file.header.csn_max
    {
        return Err(CoveError::SidecarStale);
    }
    if entry.time_field_presence_flags & DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT == 0
        || entry.commit_time_start_us > file.header.commit_time_range_start_us
        || entry.commit_time_end_us < file.header.commit_time_range_end_us
    {
        return Err(CoveError::SidecarStale);
    }
    if file.header.flags & DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT != 0
        && (entry.time_field_presence_flags & DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT == 0
            || !entry.source_publish_summary_proves_absence()
            || entry.source_publish_range_start_us > file.header.source_publish_range_start_us
            || entry.source_publish_range_end_us < file.header.source_publish_range_end_us)
    {
        return Err(CoveError::SidecarStale);
    }
    Ok(())
}

// ── Top-level COVM file ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovmFile {
    pub header: CovmHeaderV1,
    pub files: Vec<CovmFileEntryV1>,
    pub postscript: CovmPostscriptV1,
}

impl CovmFile {
    /// Parse a COVM as a non-delta-aware reader.
    ///
    /// This intentionally rejects manifests marked with
    /// [`COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED`] so callers cannot
    /// accidentally answer from base files only when the selected snapshot
    /// requires an ordered delta chain.
    pub fn parse(file_data: &[u8]) -> Result<Self, CoveError> {
        Self::parse_with_delta_awareness(file_data, false)
    }

    /// Parse a COVM for a caller that will validate the selected delta chain.
    pub fn parse_delta_aware(file_data: &[u8]) -> Result<Self, CoveError> {
        Self::parse_with_delta_awareness(file_data, true)
    }

    fn parse_with_delta_awareness(
        file_data: &[u8],
        allow_delta_chain_required: bool,
    ) -> Result<Self, CoveError> {
        let postscript = CovmPostscriptV1::parse_from_tail(file_data)?;
        if postscript.flags & !COVM_POSTSCRIPT_KNOWN_FLAGS != 0 {
            return Err(CoveError::BadSection(
                "COVM postscript contains unknown flags".into(),
            ));
        }
        if postscript.flags & COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED != 0
            && !allow_delta_chain_required
        {
            return Err(CoveError::UnknownRequiredFeature(
                COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED as u64,
            ));
        }

        if postscript.file_len != file_data.len() as u64 {
            return Err(CoveError::BadSection(format!(
                "COVM postscript file_len {} does not match actual file length {}",
                postscript.file_len,
                file_data.len()
            )));
        }

        let h_off =
            usize::try_from(postscript.header_offset).map_err(|_| CoveError::OffsetRange)?;
        let h_len = usize::try_from(postscript.header_len).map_err(|_| CoveError::OffsetRange)?;
        let h_end = h_off.checked_add(h_len).ok_or(CoveError::ArithOverflow)?;
        if h_end > file_data.len() {
            return Err(CoveError::OffsetRange);
        }
        let header = CovmHeaderV1::parse(&file_data[h_off..h_end])?;
        if postscript.header_len as u16 != header.header_len {
            return Err(CoveError::BadSection(
                "COVM postscript header_len disagrees with header".into(),
            ));
        }

        let e_off =
            usize::try_from(postscript.entries_offset).map_err(|_| CoveError::OffsetRange)?;
        let e_len = usize::try_from(postscript.entries_len).map_err(|_| CoveError::OffsetRange)?;
        let e_end = e_off.checked_add(e_len).ok_or(CoveError::ArithOverflow)?;
        if e_end > file_data.len() {
            return Err(CoveError::OffsetRange);
        }
        let region = &file_data[e_off..e_end];

        let mut files = Vec::with_capacity(header.file_count as usize);
        let mut pos = 0usize;
        for _ in 0..header.file_count {
            let (entry, used) = CovmFileEntryV1::parse(&region[pos..])?;
            pos = pos.checked_add(used).ok_or(CoveError::ArithOverflow)?;
            files.push(entry);
        }
        if pos != region.len() {
            return Err(CoveError::BadSection(
                "COVM file-entry region has trailing bytes".into(),
            ));
        }

        Ok(Self {
            header,
            files,
            postscript,
        })
    }

    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        let mut header = self.header.clone();
        header.file_count = u32::try_from(self.files.len())
            .map_err(|_| CoveError::BadSection("too many COVM file entries".into()))?;
        let header_bytes = header.serialize();

        let mut entries_bytes: Vec<u8> = Vec::new();
        for entry in &self.files {
            entries_bytes.extend_from_slice(&entry.serialize()?);
        }

        let header_offset = 0u64;
        let header_len_u64 = header_bytes.len() as u64;
        let entries_offset = header_len_u64;
        let entries_len = entries_bytes.len() as u64;
        let postscript_total = (COVM_POSTSCRIPT_LEN as u64) + (COVM_POSTSCRIPT_TAIL_SIZE as u64);
        let file_len = entries_offset + entries_len + postscript_total;

        let postscript = CovmPostscriptV1 {
            header_offset,
            header_len: header_len_u64,
            entries_offset,
            entries_len,
            file_len,
            flags: self.postscript.flags,
            checksum: 0,
        };
        let tail = postscript.serialize_tail();

        let mut out = Vec::with_capacity(file_len as usize);
        out.extend_from_slice(&header_bytes);
        out.extend_from_slice(&entries_bytes);
        out.extend_from_slice(&tail);
        debug_assert_eq!(out.len() as u64, file_len);
        Ok(out)
    }
}

fn put_u16(buf: &mut [u8], pos: &mut usize, value: u16) {
    buf[*pos..*pos + 2].copy_from_slice(&value.to_le_bytes());
    *pos += 2;
}

fn put_u32(buf: &mut [u8], pos: &mut usize, value: u32) {
    buf[*pos..*pos + 4].copy_from_slice(&value.to_le_bytes());
    *pos += 4;
}

fn put_u64(buf: &mut [u8], pos: &mut usize, value: u64) {
    buf[*pos..*pos + 8].copy_from_slice(&value.to_le_bytes());
    *pos += 8;
}

fn put_i64(buf: &mut [u8], pos: &mut usize, value: i64) {
    buf[*pos..*pos + 8].copy_from_slice(&value.to_le_bytes());
    *pos += 8;
}

fn put_bytes(buf: &mut [u8], pos: &mut usize, value: &[u8]) {
    let end = *pos + value.len();
    buf[*pos..end].copy_from_slice(value);
    *pos = end;
}

fn take_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, CoveError> {
    let end = (*pos).checked_add(2).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let value = u16::from_le_bytes(bytes[*pos..end].try_into().unwrap());
    *pos = end;
    Ok(value)
}

fn take_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, CoveError> {
    let end = (*pos).checked_add(4).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let value = u32::from_le_bytes(bytes[*pos..end].try_into().unwrap());
    *pos = end;
    Ok(value)
}

fn take_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, CoveError> {
    let end = (*pos).checked_add(8).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let value = u64::from_le_bytes(bytes[*pos..end].try_into().unwrap());
    *pos = end;
    Ok(value)
}

fn take_i64(bytes: &[u8], pos: &mut usize) -> Result<i64, CoveError> {
    let end = (*pos).checked_add(8).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let value = i64::from_le_bytes(bytes[*pos..end].try_into().unwrap());
    *pos = end;
    Ok(value)
}

fn take_bytes<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], CoveError> {
    let end = (*pos).checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let value = &bytes[*pos..end];
    *pos = end;
    Ok(value)
}

fn take_array<const N: usize>(bytes: &[u8], pos: &mut usize) -> Result<[u8; N], CoveError> {
    let slice = take_bytes(bytes, pos, N)?;
    Ok(slice.try_into().unwrap())
}

fn covm_delta_required_digest_algorithm(
    raw: u16,
    field: &str,
) -> Result<DigestAlgorithm, CoveError> {
    DigestAlgorithm::from_u16(raw)
        .filter(|algorithm| *algorithm != DigestAlgorithm::None)
        .ok_or_else(|| {
            CoveError::BadSection(format!("{field} must be SHA-256 or BLAKE3, got {raw}"))
        })
}

fn covm_delta_optional_digest_algorithm(
    raw: u16,
    field: &str,
) -> Result<Option<DigestAlgorithm>, CoveError> {
    match DigestAlgorithm::from_u16(raw) {
        Some(DigestAlgorithm::None) => Ok(None),
        Some(algorithm) => Ok(Some(algorithm)),
        None => Err(CoveError::BadSection(format!(
            "{field} must be none, SHA-256, or BLAKE3, got {raw}"
        ))),
    }
}

fn covm_delta_expected_digest_len(algorithm: DigestAlgorithm) -> usize {
    match algorithm {
        DigestAlgorithm::None => 0,
        DigestAlgorithm::Sha256 | DigestAlgorithm::Blake3 => 32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(file_id: u8, uri: &str, digest_byte: u8, digest_len: usize) -> CovmFileEntryV1 {
        CovmFileEntryV1 {
            file_id: [file_id; 16],
            uri: uri.to_string(),
            file_len: 8192,
            footer_crc32c: 0xDEADBEEF,
            digest_algorithm: 1,
            digest: vec![digest_byte; digest_len],
            row_count: 100_000,
            segment_count: 4,
            file_stats_ref: 0,
            file_exact_set_ref: 0,
            flags: 0,
        }
    }

    fn sample_file() -> CovmFile {
        CovmFile {
            header: CovmHeaderV1::new([0x55; 16], 1, 0, 1_700_000_000_000_000),
            files: vec![
                sample_entry(0x66, "s3://bucket/a.cove", 0x11, 32),
                sample_entry(0x77, "s3://bucket/b.cove", 0x22, 64),
            ],
            postscript: CovmPostscriptV1 {
                header_offset: 0,
                header_len: 0,
                entries_offset: 0,
                entries_len: 0,
                file_len: 0,
                flags: 0,
                checksum: 0,
            },
        }
    }

    fn sample_delta_artifact_ref(
        chain_ordinal: u32,
        artifact_byte: u8,
        snapshot_id: [u8; 16],
        parent_snapshot_id: [u8; 16],
    ) -> CovmDeltaArtifactRefV1 {
        CovmDeltaArtifactRefV1 {
            chain_ordinal,
            flags: 0,
            artifact_id: [artifact_byte; 16],
            snapshot_id,
            parent_snapshot_id,
            file_len: 4096 + u64::from(chain_ordinal),
            footer_crc32c: 0xCAFE_0000 | chain_ordinal,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest_len: 32,
            digest: [artifact_byte ^ 0x55; 32],
            uri_ref: chain_ordinal + 10,
            checksum: 0,
        }
    }

    fn sample_delta_chain_extension() -> CovmDeltaChainExtensionV1 {
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let mut extension = CovmDeltaChainExtensionV1::new(
            [0x10; 16],
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![sample_delta_artifact_ref(
                1,
                0x31,
                result_snapshot_id,
                base_snapshot_id,
            )],
        );
        extension.csn_min = 100;
        extension.csn_max = 101;
        extension.created_at_us = 1_700_000_000_000_000;
        extension
    }

    fn sample_validated_delta_chain_extension() -> CovmDeltaChainExtensionV1 {
        let extension = sample_delta_chain_extension();
        CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap()
    }

    fn sample_delta_summary_entry(reference: CovmDeltaArtifactRefV1) -> DeltaChainSummaryEntryV1 {
        DeltaChainSummaryEntryV1 {
            chain_ordinal: reference.chain_ordinal,
            delta_artifact_id: reference.artifact_id,
            delta_artifact_ref: reference,
            required_delta_features: 0,
            optional_delta_features: 0,
            csn_min: 100,
            csn_max: 101,
            commit_time_start_us: 1_700_000_000_000_000,
            commit_time_end_us: 1_700_000_000_000_010,
            artifact_created_at_us: 1_700_000_000_000_020,
            first_published_at_us: 1_700_000_000_000_030,
            selected_snapshot_published_at_us: 1_700_000_000_000_040,
            time_field_presence_flags: DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT
                | DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT,
            time_summary_exactness_flags: DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE,
            source_publish_range_start_us: 1_700_000_000_000_001,
            source_publish_range_end_us: 1_700_000_000_000_002,
            scope_summary_ref: 0,
            branch_summary_ref: 0,
            object_type_summary_ref: 0,
            goid_range_summary_ref: 0,
            touched_summary_ref: 0,
            tombstone_summary_ref: 0,
            property_summary_ref: 0,
            temporal_role_summary_ref: 0,
            delta_header_range_offset: 0,
            delta_header_range_length: 64,
            hot_summary_range_offset: 64,
            hot_summary_range_length: 32,
            checksum: 0,
        }
    }

    fn sample_delta_chain_summary(
        extension: &CovmDeltaChainExtensionV1,
    ) -> CovmDeltaChainSummaryV1 {
        CovmDeltaChainSummaryV1::new(
            extension.dataset_id,
            extension.result_snapshot_id,
            extension.chain_digest_algorithm,
            extension.chain_digest.clone(),
            vec![sample_delta_summary_entry(
                extension.ordered_delta_artifact_refs[0].clone(),
            )],
        )
    }

    fn sample_delta_chain_extension_with_declared_summary(
    ) -> (CovmDeltaChainExtensionV1, Vec<u8>, CovmDeltaChainSummaryV1) {
        let extension = sample_validated_delta_chain_extension();
        let summary = sample_delta_chain_summary(&extension);
        let summary_bytes = summary.serialize().unwrap();
        let mut extension_with_summary = extension;
        extension_with_summary.chain_summary_kind = COVM_DELTA_CHAIN_SUMMARY_KIND_CDS1;
        extension_with_summary.chain_summary_ref = 1;
        extension_with_summary.chain_summary_offset = 0;
        extension_with_summary.chain_summary_length = summary_bytes.len() as u64;
        extension_with_summary.chain_summary_crc32c = checksum::crc32c(&summary_bytes);
        extension_with_summary.chain_summary_digest_algorithm = DigestAlgorithm::Sha256 as u16;
        extension_with_summary.chain_summary_digest =
            compute_digest(DigestAlgorithm::Sha256, &summary_bytes).unwrap();
        let extension_with_summary =
            CovmDeltaChainExtensionV1::parse(&extension_with_summary.serialize().unwrap()).unwrap();
        let summary = CovmDeltaChainSummaryV1::parse(&summary_bytes).unwrap();
        (extension_with_summary, summary_bytes, summary)
    }

    fn sample_two_entry_delta_chain_summary() -> CovmDeltaChainSummaryV1 {
        let first = sample_delta_artifact_ref(1, 0x31, [0x21; 16], [0x20; 16]);
        let second = sample_delta_artifact_ref(2, 0x32, [0x22; 16], [0x21; 16]);
        let mut first_entry = sample_delta_summary_entry(first);
        first_entry.csn_min = 10;
        first_entry.csn_max = 20;
        first_entry.commit_time_start_us = 1_000;
        first_entry.commit_time_end_us = 1_100;
        first_entry.source_publish_range_start_us = 1_200;
        first_entry.source_publish_range_end_us = 1_300;
        let mut second_entry = sample_delta_summary_entry(second);
        second_entry.csn_min = 30;
        second_entry.csn_max = 40;
        second_entry.commit_time_start_us = 2_000;
        second_entry.commit_time_end_us = 2_100;
        second_entry.source_publish_range_start_us = 2_200;
        second_entry.source_publish_range_end_us = 2_300;
        CovmDeltaChainSummaryV1::new(
            [0x10; 16],
            [0x22; 16],
            DigestAlgorithm::Sha256 as u16,
            vec![0xAB; 32],
            vec![first_entry, second_entry],
        )
    }

    fn sample_covedelta_bytes(
        artifact_id: [u8; 16],
        dataset_id: [u8; 16],
        snapshot_id: [u8; 16],
        parent_snapshot_id: [u8; 16],
        chain_ordinal: u32,
    ) -> Vec<u8> {
        use super::super::covedelta::{
            CoveDeltaFile, CoveDeltaFooterV1, CoveDeltaHeaderV1, CoveDeltaPostscriptV1,
            CoveDeltaSection, CoveDeltaSectionDirectoryEntryV1, CoveDeltaSectionKind,
            DeltaParentRefV1, COVEDELTA_FOOTER_LEN, COVEDELTA_HEADER_LEN,
            DELTA_PARENT_REF_LINEAGE_PARENT,
        };

        let mut header =
            CoveDeltaHeaderV1::new(artifact_id, dataset_id, snapshot_id, parent_snapshot_id);
        header.chain_ordinal = chain_ordinal;
        header.chain_depth = chain_ordinal;
        header.csn_min = 100;
        header.csn_max = 101;
        header.commit_time_range_start_us = 1_700_000_000_000_000;
        header.commit_time_range_end_us = 1_700_000_000_000_010;
        CoveDeltaFile {
            header,
            parent_refs: vec![DeltaParentRefV1 {
                parent_ref: 0,
                parent_kind: 0,
                flags: DELTA_PARENT_REF_LINEAGE_PARENT,
                artifact_id: [0x30; 16],
                snapshot_id: parent_snapshot_id,
                file_len: 4096,
                footer_crc32c: 0xCAFE_0000,
                digest_algorithm: DigestAlgorithm::Sha256 as u16,
                digest_len: 32,
                digest_ref: 0,
                uri_ref: 10,
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
                payload: b"selected-chain-placeholder".to_vec(),
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
        .serialize()
        .unwrap()
    }

    fn delta_artifact_ref_from_bytes(bytes: &[u8]) -> CovmDeltaArtifactRefV1 {
        let file = CoveDeltaFile::parse(bytes).unwrap();
        let digest = compute_digest(DigestAlgorithm::Sha256, bytes).unwrap();
        let mut digest_array = [0u8; 32];
        digest_array.copy_from_slice(&digest);
        CovmDeltaArtifactRefV1 {
            chain_ordinal: file.header.chain_ordinal,
            flags: 0,
            artifact_id: file.header.delta_artifact_id,
            snapshot_id: file.header.snapshot_id,
            parent_snapshot_id: file.header.parent_snapshot_id,
            file_len: bytes.len() as u64,
            footer_crc32c: file.footer.footer_crc32c,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest_len: 32,
            digest: digest_array,
            uri_ref: 1,
            checksum: 0,
        }
    }

    fn base_artifact_ref_from_bytes(
        bytes: &[u8],
        artifact_id: [u8; 16],
        snapshot_id: [u8; 16],
    ) -> CovmDeltaArtifactRefV1 {
        let digest = compute_digest(DigestAlgorithm::Sha256, bytes).unwrap();
        let mut digest_array = [0u8; 32];
        digest_array.copy_from_slice(&digest);
        CovmDeltaArtifactRefV1 {
            chain_ordinal: 0,
            flags: 0,
            artifact_id,
            snapshot_id,
            parent_snapshot_id: [0; 16],
            file_len: bytes.len() as u64,
            footer_crc32c: 0,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest_len: 32,
            digest: digest_array,
            uri_ref: 1,
            checksum: 0,
        }
    }

    fn align_delta_lineage_parent_with_ref(
        delta: &mut CoveDeltaFile,
        expected_parent: &CovmDeltaArtifactRefV1,
    ) {
        let parent = delta
            .parent_refs
            .iter_mut()
            .find(|parent| parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0)
            .unwrap();
        parent.artifact_id = expected_parent.artifact_id;
        parent.snapshot_id = expected_parent.snapshot_id;
        parent.file_len = expected_parent.file_len;
        parent.footer_crc32c = expected_parent.footer_crc32c;
        parent.digest_algorithm = expected_parent.digest_algorithm;
        parent.digest_len = expected_parent.digest_len;
        parent.uri_ref = expected_parent.uri_ref;
    }

    fn sample_base_aware_selected_chain() -> (
        Vec<u8>,
        Vec<u8>,
        CovmDeltaChainExtensionV1,
        CovmDeltaChainSummaryV1,
    ) {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let base_bytes = b"base snapshot artifact bytes".to_vec();
        let base_ref = base_artifact_ref_from_bytes(&base_bytes, [0x30; 16], base_snapshot_id);
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let mut delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        align_delta_lineage_parent_with_ref(&mut delta, &base_ref);
        let delta_bytes = delta.serialize().unwrap();
        let delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            base_ref,
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
        let summary = sample_delta_chain_summary(&extension);
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        (base_bytes, delta_bytes, extension, summary)
    }

    #[test]
    fn header_roundtrip_and_checksum() {
        let h = CovmHeaderV1::new([0xCC; 16], 7, 13, 99);
        let bytes = h.serialize();
        let h2 = CovmHeaderV1::parse(&bytes).unwrap();
        assert_eq!(h2.dataset_id, [0xCC; 16]);
        assert_eq!(h2.table_count, 7);
        assert_eq!(h2.file_count, 13);
        assert_eq!(h2.created_at_us, 99);
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut bytes = CovmHeaderV1::new([0; 16], 0, 0, 0).serialize();
        bytes[0] = b'X';
        assert_eq!(CovmHeaderV1::parse(&bytes), Err(CoveError::BadMagic));
    }

    #[test]
    fn header_rejects_flipped_checksum() {
        let mut bytes = CovmHeaderV1::new([0; 16], 0, 0, 0).serialize();
        bytes[78] ^= 0xFF;
        assert_eq!(
            CovmHeaderV1::parse(&bytes),
            Err(CoveError::ChecksumMismatch)
        );
    }

    #[test]
    fn header_rejects_bad_version() {
        let mut bytes = CovmHeaderV1::new([0; 16], 0, 0, 0).serialize();
        bytes[6..8].copy_from_slice(&2u16.to_le_bytes());
        bytes[78..82].fill(0);
        let crc = checksum::crc32c(&bytes);
        bytes[78..82].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(CovmHeaderV1::parse(&bytes), Err(CoveError::BadVersion));
    }

    #[test]
    fn entry_roundtrip_with_uri_and_digest() {
        let e = sample_entry(0x42, "file:///x/y.cove", 0xCD, 48);
        let bytes = e.serialize().unwrap();
        let (e2, used) = CovmFileEntryV1::parse(&bytes).unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(e, e2);
    }

    #[test]
    fn file_roundtrip_two_entries() {
        let f = sample_file();
        let bytes = f.serialize().unwrap();
        let parsed = CovmFile::parse(&bytes).unwrap();
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].uri, "s3://bucket/a.cove");
        assert_eq!(parsed.files[1].digest.len(), 64);
        assert_eq!(parsed.postscript.file_len, bytes.len() as u64);
    }

    #[test]
    fn file_rejects_delta_required_manifest_without_delta_aware_parse() {
        let mut f = sample_file();
        f.postscript.flags = COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED;
        let bytes = f.serialize().unwrap();

        assert_eq!(
            CovmFile::parse(&bytes),
            Err(CoveError::UnknownRequiredFeature(
                COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED as u64
            ))
        );
        let parsed = CovmFile::parse_delta_aware(&bytes).unwrap();
        assert_eq!(
            parsed.postscript.flags & COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED,
            COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED
        );
    }

    #[test]
    fn file_rejects_unknown_postscript_flags() {
        let mut f = sample_file();
        f.postscript.flags = 0x8000_0000;
        let bytes = f.serialize().unwrap();

        assert!(matches!(
            CovmFile::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("unknown flags")
        ));
        assert!(matches!(
            CovmFile::parse_delta_aware(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("unknown flags")
        ));
    }

    #[test]
    fn file_rejects_flipped_tail_magic() {
        let f = sample_file();
        let mut bytes = f.serialize().unwrap();
        let n = bytes.len();
        bytes[n - 1] ^= 0xFF;
        assert_eq!(CovmFile::parse(&bytes), Err(CoveError::BadMagic));
    }

    #[test]
    fn file_rejects_postscript_checksum_corruption() {
        let f = sample_file();
        let mut bytes = f.serialize().unwrap();
        let n = bytes.len();
        let cksum_off = n - COVM_POSTSCRIPT_TAIL_SIZE - 4;
        bytes[cksum_off] ^= 0xFF;
        assert_eq!(CovmFile::parse(&bytes), Err(CoveError::ChecksumMismatch));
    }

    #[test]
    fn entry_verify_detects_stale_in_each_field() {
        let e = sample_entry(0x88, "x", 0x99, 32);
        let id = [0x88u8; 16];
        let dg = vec![0x99u8; 32];
        assert!(e.verify_against(&id, 8192, 0xDEADBEEF, &dg).is_ok());
        assert_eq!(
            e.verify_against(&[0; 16], 8192, 0xDEADBEEF, &dg),
            Err(CoveError::SidecarStale)
        );
        assert_eq!(
            e.verify_against(&id, 0, 0xDEADBEEF, &dg),
            Err(CoveError::SidecarStale)
        );
        assert_eq!(
            e.verify_against(&id, 8192, 0, &dg),
            Err(CoveError::SidecarStale)
        );
        assert_eq!(
            e.verify_against(&id, 8192, 0xDEADBEEF, &[0u8; 32]),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn delta_chain_extension_roundtrip_and_validates_digest() {
        let extension = sample_delta_chain_extension();
        let bytes = extension.serialize().unwrap();
        let parsed = CovmDeltaChainExtensionV1::parse(&bytes).unwrap();
        assert_eq!(parsed.dataset_id, [0x10; 16]);
        assert_eq!(parsed.base_snapshot_id, [0x20; 16]);
        assert_eq!(parsed.result_snapshot_id, [0x21; 16]);
        assert_eq!(parsed.ordered_delta_artifact_refs.len(), 1);
        assert_eq!(parsed.chain_digest, parsed.computed_chain_digest().unwrap());
    }

    #[test]
    fn delta_chain_extension_rejects_digest_mismatch() {
        let extension = sample_delta_chain_extension();
        let mut bytes = extension.serialize().unwrap();
        let digest_offset = COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN + COVM_DELTA_ARTIFACT_REF_LEN;
        bytes[digest_offset] ^= 0xFF;
        assert_eq!(
            CovmDeltaChainExtensionV1::parse(&bytes),
            Err(CoveError::DigestMismatch)
        );
    }

    #[test]
    fn delta_chain_extension_rejects_unsupported_required_features() {
        let mut extension = sample_delta_chain_extension();
        extension.required_delta_features = 1;
        assert_eq!(
            extension.serialize(),
            Err(CoveError::UnknownRequiredFeature(1))
        );
    }

    #[test]
    fn delta_chain_extension_rejects_reordered_or_sparse_deltas() {
        let base_snapshot_id = [0x20; 16];
        let mid_snapshot_id = [0x21; 16];
        let result_snapshot_id = [0x22; 16];
        let mut extension = CovmDeltaChainExtensionV1::new(
            [0x10; 16],
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![
                sample_delta_artifact_ref(2, 0x32, result_snapshot_id, mid_snapshot_id),
                sample_delta_artifact_ref(1, 0x31, mid_snapshot_id, base_snapshot_id),
            ],
        );
        extension.csn_max = 1;
        assert!(matches!(
            extension.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("ordinal must be dense")
        ));
    }

    #[test]
    fn delta_chain_extension_rejects_duplicate_artifact_ids() {
        let mut extension = sample_delta_chain_extension();
        extension.ordered_delta_artifact_refs[0].artifact_id =
            extension.base_artifact_ref.artifact_id;
        assert!(matches!(
            extension.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("artifact IDs must be unique")
        ));
    }

    #[test]
    fn delta_chain_summary_roundtrip_and_validates_against_extension() {
        let extension = sample_validated_delta_chain_extension();
        let summary = sample_delta_chain_summary(&extension);
        let bytes = summary.serialize().unwrap();
        let parsed = CovmDeltaChainSummaryV1::parse(&bytes).unwrap();
        assert_eq!(parsed.magic, COVM_DELTA_CHAIN_SUMMARY_MAGIC);
        assert_eq!(parsed.delta_summaries.len(), 1);
        parsed
            .validate_against_delta_chain_extension(&extension)
            .unwrap();
    }

    #[test]
    fn delta_chain_summary_rejects_stale_chain_digest_binding() {
        let extension = sample_validated_delta_chain_extension();
        let mut summary = sample_delta_chain_summary(&extension);
        summary.chain_digest[0] ^= 0xFF;
        assert_eq!(
            summary.validate_against_delta_chain_extension(&extension),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_missing_declared_chain_summary() {
        let (extension, _summary_bytes, _summary) =
            sample_delta_chain_extension_with_declared_summary();

        assert!(matches!(
            validate_selected_delta_chain(&extension, None, &[]),
            Err(CoveError::BadSection(message))
                if message.contains("requires declared chain summary")
        ));
    }

    #[test]
    fn selected_delta_chain_summary_bytes_reject_corrupt_declared_summary() {
        let (extension, mut summary_bytes, _summary) =
            sample_delta_chain_extension_with_declared_summary();
        let last = summary_bytes.len() - 1;
        summary_bytes[last] ^= 0xFF;

        assert_eq!(
            validate_selected_delta_chain_with_summary_bytes(&extension, Some(&summary_bytes), &[]),
            Err(CoveError::ChecksumMismatch)
        );
    }

    #[test]
    fn selected_delta_chain_summary_bytes_reject_stale_declared_summary_digest() {
        let (mut extension, summary_bytes, _summary) =
            sample_delta_chain_extension_with_declared_summary();
        extension.chain_summary_digest[0] ^= 0xFF;

        assert_eq!(
            validate_selected_delta_chain_with_summary_bytes(&extension, Some(&summary_bytes), &[]),
            Err(CoveError::DigestMismatch)
        );
    }

    #[test]
    fn delta_chain_summary_rejects_sparse_entry_ordinal() {
        let extension = sample_validated_delta_chain_extension();
        let summary = sample_delta_chain_summary(&extension);
        let mut bytes = summary.serialize().unwrap();
        let entry_offset =
            COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize + extension.chain_digest.len();
        bytes[entry_offset..entry_offset + 4].copy_from_slice(&2u32.to_le_bytes());
        let checksum_offset = entry_offset + COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN - 4;
        bytes[checksum_offset..checksum_offset + 4].fill(0);
        let crc = checksum::crc32c(
            &bytes[entry_offset..entry_offset + COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN],
        );
        bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(
            CovmDeltaChainSummaryV1::parse(&bytes),
            Err(CoveError::BadSection(message))
                if message.contains("ordinal")
        ));
    }

    #[test]
    fn delta_chain_summary_rejects_non_append_only_csn_ranges() {
        let mut summary = sample_two_entry_delta_chain_summary();
        summary.delta_summaries[1].csn_min = 20;
        assert!(matches!(
            summary.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("CSN ranges")
        ));
    }

    #[test]
    fn delta_chain_summary_rejects_decreasing_commit_ranges() {
        let mut summary = sample_two_entry_delta_chain_summary();
        summary.delta_summaries[1].commit_time_start_us = 900;
        assert!(matches!(
            summary.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("commit-time ranges")
        ));
    }

    #[test]
    fn delta_chain_summary_prunes_by_as_of_csn() {
        let summary = sample_two_entry_delta_chain_summary();
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        let decision = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: Some(25),
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            })
            .unwrap();
        assert_eq!(decision.selected_chain_ordinals, vec![1]);
        assert_eq!(decision.skipped_delta_count(), 1);
        assert_eq!(
            decision.skipped[0],
            CovmDeltaPruneSkip {
                chain_ordinal: 2,
                reason: CovmDeltaPruneReason::AsOfCsnBeforeDelta,
            }
        );
    }

    #[test]
    fn delta_chain_summary_prunes_as_of_csn_before_inside_and_after_ranges() {
        let summary = sample_two_entry_delta_chain_summary();
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();

        let before = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: Some(5),
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            })
            .unwrap();
        assert!(before.selected_chain_ordinals.is_empty());
        assert_eq!(before.skipped_delta_count(), 2);
        assert_eq!(
            before
                .skipped
                .iter()
                .map(|skip| skip.chain_ordinal)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(before
            .skipped
            .iter()
            .all(|skip| skip.reason == CovmDeltaPruneReason::AsOfCsnBeforeDelta));

        let inside_first = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: Some(15),
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            })
            .unwrap();
        assert_eq!(inside_first.selected_chain_ordinals, vec![1]);
        assert_eq!(
            inside_first.skipped,
            vec![CovmDeltaPruneSkip {
                chain_ordinal: 2,
                reason: CovmDeltaPruneReason::AsOfCsnBeforeDelta,
            }]
        );

        let after_all = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: Some(45),
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            })
            .unwrap();
        assert_eq!(after_all.selected_chain_ordinals, vec![1, 2]);
        assert!(after_all.skipped.is_empty());
    }

    #[test]
    fn delta_chain_summary_prunes_by_commit_time() {
        let summary = sample_two_entry_delta_chain_summary();
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        let decision = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: None,
                as_of_commit_timestamp_us: Some(1_500),
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            })
            .unwrap();
        assert_eq!(decision.selected_chain_ordinals, vec![1]);
        assert_eq!(decision.skipped_delta_count(), 1);
        assert_eq!(
            decision.skipped[0].reason,
            CovmDeltaPruneReason::AsOfCommitBeforeDelta
        );
    }

    #[test]
    fn delta_chain_summary_prune_metrics_track_reason_counts() {
        let summary = sample_two_entry_delta_chain_summary();
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        let decision = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: Some(25),
                as_of_commit_timestamp_us: Some(1_500),
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            })
            .unwrap();
        let metrics = decision.metrics();
        assert_eq!(metrics.delta_chain_depth, 2);
        assert_eq!(metrics.selected_delta_count, 1);
        assert_eq!(metrics.skipped_delta_count, 1);
        assert_eq!(metrics.delta_artifacts_planned_to_open, 1);
        assert_eq!(metrics.delta_artifacts_skipped_before_open, 1);
        assert_eq!(metrics.as_of_csn_prunes, 1);
        assert_eq!(metrics.commit_time_range_prunes, 0);
        assert_eq!(metrics.source_publish_range_prunes, 0);
    }

    #[test]
    fn delta_chain_summary_exposes_read_amplification_metrics() {
        let summary = sample_two_entry_delta_chain_summary();
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        let decision = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: Some(25),
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            })
            .unwrap();

        let metrics = summary.read_amplification_metrics(&decision);
        assert_eq!(metrics.delta_chain_depth, 2);
        assert_eq!(metrics.selected_delta_count, 1);
        assert_eq!(metrics.skipped_delta_count, 1);
        assert_eq!(metrics.delta_artifacts_opened, 1);
        assert_eq!(metrics.delta_artifacts_skipped_before_open, 1);
        assert_eq!(metrics.base_ranges_requested, 1);
        assert_eq!(metrics.delta_ranges_requested, 1);
        assert_eq!(metrics.chain_summary_range_requests, 1);
        assert_eq!(metrics.object_store_request_count, 3);
        assert_eq!(metrics.chain_summary_bytes, summary.encoded_len());
        assert!(metrics.chain_summary_bytes > COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize);
    }

    #[test]
    fn read_amplification_policy_recommends_checkpoint_compaction_and_indexes() {
        let metrics = CovmDeltaReadAmplificationMetrics {
            delta_chain_depth: 17,
            base_file_bytes: 100,
            total_delta_bytes: 21,
            max_patch_rows_since_checkpoint: 33,
            point_lookup_artifacts_p95: 5,
            metadata_range_requests_before_data: 3,
            ..CovmDeltaReadAmplificationMetrics::default()
        };

        let recommendations = metrics.recommendations(CovmDeltaReadAmplificationPolicy::default());
        assert_eq!(
            recommendations,
            vec![
                CovmDeltaReadAmplificationRecommendation::WarnChainDepth,
                CovmDeltaReadAmplificationRecommendation::RecommendCheckpoint,
                CovmDeltaReadAmplificationRecommendation::RecommendCompaction,
                CovmDeltaReadAmplificationRecommendation::RecommendSnapshotLevelIndex,
                CovmDeltaReadAmplificationRecommendation::RecommendSummaryHoistingOrCompaction,
            ]
        );
    }

    #[test]
    fn read_amplification_policy_requires_override_past_hard_chain_depth() {
        let metrics = CovmDeltaReadAmplificationMetrics {
            delta_chain_depth: 65,
            ..CovmDeltaReadAmplificationMetrics::default()
        };

        assert_eq!(
            metrics.recommendations(CovmDeltaReadAmplificationPolicy::default()),
            vec![CovmDeltaReadAmplificationRecommendation::RequireOverrideChainDepth]
        );
    }

    #[test]
    fn read_amplification_policy_recommends_packing_tiny_delta_requests() {
        let metrics = CovmDeltaReadAmplificationMetrics {
            object_store_request_count: 4,
            bytes_returned: 8 * 1024,
            ..CovmDeltaReadAmplificationMetrics::default()
        };

        assert_eq!(
            metrics.recommendations(CovmDeltaReadAmplificationPolicy::default()),
            vec![CovmDeltaReadAmplificationRecommendation::RecommendPackingSmallDeltas]
        );
    }

    #[test]
    fn delta_chain_summary_prunes_by_source_publish_range() {
        let summary = sample_two_entry_delta_chain_summary();
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        let decision = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: None,
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: Some((1_150, 1_350)),
            })
            .unwrap();
        assert_eq!(decision.selected_chain_ordinals, vec![1]);
        assert_eq!(decision.skipped_delta_count(), 1);
        assert_eq!(
            decision.skipped[0].reason,
            CovmDeltaPruneReason::SourcePublishRangeOutsideDelta
        );
        assert_eq!(decision.metrics().source_publish_range_prunes, 1);
    }

    #[test]
    fn delta_chain_summary_valid_time_without_temporal_summary_does_not_prune_by_csn() {
        let summary = sample_two_entry_delta_chain_summary();
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        let decision = summary
            .prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: None,
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: Some(5),
                source_publish_range_us: None,
            })
            .unwrap();

        assert_eq!(decision.selected_chain_ordinals, vec![1, 2]);
        assert!(decision.skipped.is_empty());
    }

    #[test]
    fn delta_chain_summary_source_publish_pruning_requires_source_fields() {
        let mut summary = sample_two_entry_delta_chain_summary();
        summary.delta_summaries[0].time_field_presence_flags &=
            !DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT;
        summary.delta_summaries[0].time_summary_exactness_flags = 0;
        summary.delta_summaries[0].source_publish_range_start_us = 0;
        summary.delta_summaries[0].source_publish_range_end_us = 0;
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        assert!(matches!(
            summary.prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: None,
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: Some((1_150, 1_350)),
            }),
            Err(CoveError::BadSection(message))
                if message.contains("source-publish pruning")
        ));
    }

    #[test]
    fn delta_chain_summary_source_publish_pruning_requires_source_exactness() {
        let mut summary = sample_two_entry_delta_chain_summary();
        summary.delta_summaries[0].time_summary_exactness_flags = 0;
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        assert!(matches!(
            summary.prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: None,
                as_of_commit_timestamp_us: None,
                as_of_valid_time_us: None,
                source_publish_range_us: Some((1_150, 1_350)),
            }),
            Err(CoveError::BadSection(message))
                if message.contains("source-publish summary exactness")
        ));
    }

    #[test]
    fn delta_chain_summary_rejects_source_exactness_without_source_fields() {
        let mut summary = sample_two_entry_delta_chain_summary();
        summary.delta_summaries[0].time_field_presence_flags &=
            !DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT;
        summary.delta_summaries[0].source_publish_range_start_us = 0;
        summary.delta_summaries[0].source_publish_range_end_us = 0;
        assert!(matches!(
            summary.serialize(),
            Err(CoveError::BadSection(message))
                if message.contains("exactness requires source-publish fields")
        ));
    }

    #[test]
    fn delta_chain_summary_commit_pruning_requires_commit_fields() {
        let mut summary = sample_two_entry_delta_chain_summary();
        summary.delta_summaries[0].time_field_presence_flags &=
            !DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT;
        summary.delta_summaries[0].commit_time_start_us = 0;
        summary.delta_summaries[0].commit_time_end_us = 0;
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();
        assert!(matches!(
            summary.prune_delta_chain(CovmDeltaPruneRequest {
                as_of_csn: None,
                as_of_commit_timestamp_us: Some(1_500),
                as_of_valid_time_us: None,
                source_publish_range_us: None,
            }),
            Err(CoveError::BadSection(message))
                if message.contains("commit-time pruning")
        ));
    }

    #[test]
    fn selected_delta_chain_validates_extension_summary_and_delta_bytes() {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
        let summary = sample_delta_chain_summary(&extension);
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();

        let parsed =
            validate_selected_delta_chain(&extension, Some(&summary), &[&delta_bytes]).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].header.snapshot_id, result_snapshot_id);
    }

    #[test]
    fn selected_delta_chain_with_base_validates_base_and_delta_bytes() {
        let (base_bytes, delta_bytes, extension, summary) = sample_base_aware_selected_chain();

        let parsed = validate_selected_delta_chain_with_base(
            &extension,
            Some(&summary),
            Some(&base_bytes),
            &[&delta_bytes],
        )
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].header.snapshot_id, extension.result_snapshot_id);
    }

    #[test]
    fn selected_delta_chain_with_base_rejects_missing_base() {
        let (_base_bytes, delta_bytes, extension, summary) = sample_base_aware_selected_chain();

        assert!(matches!(
            validate_selected_delta_chain_with_base(&extension, Some(&summary), None, &[&delta_bytes]),
            Err(CoveError::BadSection(message))
                if message.contains("requires base artifact bytes")
        ));
    }

    #[test]
    fn selected_delta_chain_with_base_rejects_stale_base_digest() {
        let (mut base_bytes, delta_bytes, extension, summary) = sample_base_aware_selected_chain();
        base_bytes[0] ^= 0xFF;

        assert_eq!(
            validate_selected_delta_chain_with_base(
                &extension,
                Some(&summary),
                Some(&base_bytes),
                &[&delta_bytes],
            ),
            Err(CoveError::DigestMismatch)
        );
    }

    #[test]
    fn selected_delta_chain_with_base_rejects_stale_lineage_parent_metadata() {
        let (base_bytes, delta_bytes, mut extension, _summary) = sample_base_aware_selected_chain();
        let mut delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        delta.parent_refs[0].artifact_id = [0xFE; 16];
        let delta_bytes = delta.serialize().unwrap();
        extension.ordered_delta_artifact_refs[0] = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
        let summary = sample_delta_chain_summary(&extension);
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain_with_base(
                &extension,
                Some(&summary),
                Some(&base_bytes),
                &[&delta_bytes],
            ),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_stale_lineage_parent_metadata_without_base_bytes() {
        let (_base_bytes, delta_bytes, mut extension, _summary) =
            sample_base_aware_selected_chain();
        let mut delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        delta.parent_refs[0].artifact_id = [0xFE; 16];
        let delta_bytes = delta.serialize().unwrap();
        extension.ordered_delta_artifact_refs[0] = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain(&extension, None, &[&delta_bytes]),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_underinclusive_summary_csn_range() {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
        let mut summary = sample_delta_chain_summary(&extension);
        summary.delta_summaries[0].csn_min = 101;
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain(&extension, Some(&summary), &[&delta_bytes]),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_underinclusive_summary_commit_range() {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
        let mut summary = sample_delta_chain_summary(&extension);
        summary.delta_summaries[0].commit_time_end_us = 1_700_000_000_000_005;
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain(&extension, Some(&summary), &[&delta_bytes]),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_underinclusive_summary_source_publish_range() {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let mut delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        delta.header.flags |= DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT;
        delta.header.source_publish_range_start_us = 1_700_000_000_000_000;
        delta.header.source_publish_range_end_us = 1_700_000_000_000_020;
        let delta_bytes = delta.serialize().unwrap();
        let delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
        let mut summary = sample_delta_chain_summary(&extension);
        summary.delta_summaries[0].source_publish_range_start_us = 1_700_000_000_000_005;
        summary.delta_summaries[0].source_publish_range_end_us = 1_700_000_000_000_015;
        let summary = CovmDeltaChainSummaryV1::parse(&summary.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain(&extension, Some(&summary), &[&delta_bytes]),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_digest_mismatch() {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let mut delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        delta_ref.digest[0] ^= 0xFF;
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain(&extension, None, &[&delta_bytes]),
            Err(CoveError::DigestMismatch)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_missing_or_extra_delta_bytes() {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_artifact_ref_from_bytes(&delta_bytes)],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

        assert!(matches!(
            validate_selected_delta_chain(&extension, None, &[]),
            Err(CoveError::BadSection(message))
                if message.contains("artifact count does not match")
        ));
        assert!(matches!(
            validate_selected_delta_chain(&extension, None, &[&delta_bytes, &delta_bytes]),
            Err(CoveError::BadSection(message))
                if message.contains("artifact count does not match")
        ));
    }

    #[test]
    fn selected_delta_chain_rejects_reordered_delta_bytes() {
        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let mid_snapshot_id = [0x21; 16];
        let result_snapshot_id = [0x22; 16];
        let delta_one_bytes =
            sample_covedelta_bytes([0x31; 16], dataset_id, mid_snapshot_id, base_snapshot_id, 1);
        let delta_one_ref = delta_artifact_ref_from_bytes(&delta_one_bytes);
        let mut delta_two = CoveDeltaFile::parse(&sample_covedelta_bytes(
            [0x32; 16],
            dataset_id,
            result_snapshot_id,
            mid_snapshot_id,
            2,
        ))
        .unwrap();
        align_delta_lineage_parent_with_ref(&mut delta_two, &delta_one_ref);
        let delta_two_bytes = delta_two.serialize().unwrap();
        let delta_two_ref = delta_artifact_ref_from_bytes(&delta_two_bytes);
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_one_ref, delta_two_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

        validate_selected_delta_chain(&extension, None, &[&delta_one_bytes, &delta_two_bytes])
            .unwrap();
        assert_eq!(
            validate_selected_delta_chain(&extension, None, &[&delta_two_bytes, &delta_one_bytes],),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_delta_required_feature_not_declared_by_extension() {
        use super::super::covedelta::DELTA_FEATURE_EXACT_TOUCHED_SET;

        let dataset_id = [0x10; 16];
        let base_snapshot_id = [0x20; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            base_snapshot_id,
            1,
        );
        let mut delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        delta.header.required_delta_features = DELTA_FEATURE_EXACT_TOUCHED_SET;
        delta.postscript.required_delta_features = DELTA_FEATURE_EXACT_TOUCHED_SET;
        let delta_bytes = delta.serialize().unwrap();
        let delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, base_snapshot_id, [0; 16]),
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain(&extension, None, &[&delta_bytes]),
            Err(CoveError::SidecarStale)
        );
    }

    #[test]
    fn selected_delta_chain_rejects_wrong_parent_snapshot_identity() {
        let dataset_id = [0x10; 16];
        let actual_base_snapshot_id = [0x20; 16];
        let declared_base_snapshot_id = [0x22; 16];
        let result_snapshot_id = [0x21; 16];
        let delta_bytes = sample_covedelta_bytes(
            [0x31; 16],
            dataset_id,
            result_snapshot_id,
            actual_base_snapshot_id,
            1,
        );
        let mut delta_ref = delta_artifact_ref_from_bytes(&delta_bytes);
        delta_ref.parent_snapshot_id = declared_base_snapshot_id;
        let extension = CovmDeltaChainExtensionV1::new(
            dataset_id,
            declared_base_snapshot_id,
            result_snapshot_id,
            sample_delta_artifact_ref(0, 0x30, declared_base_snapshot_id, [0; 16]),
            vec![delta_ref],
        );
        let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

        assert_eq!(
            validate_selected_delta_chain(&extension, None, &[&delta_bytes]),
            Err(CoveError::SidecarStale)
        );
    }
}
