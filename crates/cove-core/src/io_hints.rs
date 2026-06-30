//! Cove Format (COVE) v2.0 — I/O hints (Spec §8.8, §67).
//!
//! Optional descriptive metadata that nudges the reader toward efficient
//! object-store / NVMe access patterns. The reader is free to ignore hints.

use crate::{wire::read_u32_le_checked, CoveError};

/// Spec §67 `CoveIoHintV1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoHints {
    pub preferred_read_alignment: u32,
    pub preferred_coalesce_distance: u32,
    pub preferred_max_coalesced_read: u32,
    pub prefetch_group_id: u32,
    pub page_cluster_id: u32,
    pub flags: u32,
}

impl IoHints {
    pub const ENCODED_LEN: usize = 24;

    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < Self::ENCODED_LEN {
            return Err(CoveError::BufferTooShort);
        }
        if bytes.len() != Self::ENCODED_LEN {
            return Err(CoveError::BadSection(
                "I/O hints payload must be exactly 24 bytes".into(),
            ));
        }
        Ok(Self {
            preferred_read_alignment: read_u32_le_checked(bytes, 0)?,
            preferred_coalesce_distance: read_u32_le_checked(bytes, 4)?,
            preferred_max_coalesced_read: read_u32_le_checked(bytes, 8)?,
            prefetch_group_id: read_u32_le_checked(bytes, 12)?,
            page_cluster_id: read_u32_le_checked(bytes, 16)?,
            flags: read_u32_le_checked(bytes, 20)?,
        })
    }

    pub fn encode(&self) -> [u8; Self::ENCODED_LEN] {
        let mut out = [0u8; Self::ENCODED_LEN];
        out[0..4].copy_from_slice(&self.preferred_read_alignment.to_le_bytes());
        out[4..8].copy_from_slice(&self.preferred_coalesce_distance.to_le_bytes());
        out[8..12].copy_from_slice(&self.preferred_max_coalesced_read.to_le_bytes());
        out[12..16].copy_from_slice(&self.prefetch_group_id.to_le_bytes());
        out[16..20].copy_from_slice(&self.page_cluster_id.to_le_bytes());
        out[20..24].copy_from_slice(&self.flags.to_le_bytes());
        out
    }
}

/// Default Spec §67 hints suitable for cloud object stores.
pub const fn defaults_object_store() -> IoHints {
    IoHints {
        preferred_read_alignment: 1 << 20,
        preferred_coalesce_distance: 1 << 20,
        preferred_max_coalesced_read: 4 * (1 << 20),
        prefetch_group_id: 0,
        page_cluster_id: 0,
        flags: 0,
    }
}

/// Default hints suitable for local NVMe.
pub const fn defaults_nvme() -> IoHints {
    IoHints {
        preferred_read_alignment: 1 << 12,
        preferred_coalesce_distance: 1 << 16,
        preferred_max_coalesced_read: 1 << 20,
        prefetch_group_id: 0,
        page_cluster_id: 0,
        flags: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let h = defaults_object_store();
        let bytes = h.encode();
        assert_eq!(IoHints::parse(&bytes).unwrap(), h);
    }
}
