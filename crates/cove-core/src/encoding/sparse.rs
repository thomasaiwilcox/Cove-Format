//! Spec §20.3 — Sparse encoding.
//!
//! A page is mostly a single "fill" value, with a small set of override
//! positions. Wire layout (LE):
//! ```text
//! u32 row_count
//! i64 fill_value
//! u32 override_count
//! repeat override_count: u32 position | i64 value
//! ```
//! Spec §20.3.8 requires override positions to be strictly increasing and
//! distinct from any other override.

use crate::{wire, CoveError};

use super::Encoding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparsePayload {
    pub row_count: u32,
    pub fill: i64,
    pub overrides: Vec<(u32, i64)>,
}

impl SparsePayload {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < 16 {
            return Err(CoveError::BufferTooShort);
        }
        let row_count = wire::read_u32_le_checked(bytes, 0)?;
        let fill = wire::read_i64_le_checked(bytes, 4)?;
        let oc = wire::read_u32_le_checked(bytes, 12)? as usize;
        let need = oc
            .checked_mul(12)
            .and_then(|bytes| bytes.checked_add(16))
            .ok_or(CoveError::ArithOverflow)?;
        if bytes.len() < need {
            return Err(CoveError::BufferTooShort);
        }
        let mut overrides = Vec::with_capacity(oc);
        let mut prev: Option<u32> = None;
        for i in 0..oc {
            let off = 16 + i * 12;
            let p = wire::read_u32_le_checked(bytes, off)?;
            let v = wire::read_i64_le_checked(bytes, off + 4)?;
            if let Some(prev_pos) = prev {
                if p <= prev_pos {
                    return Err(CoveError::PageCorrupt);
                }
            }
            if p >= row_count {
                return Err(CoveError::PageCorrupt);
            }
            prev = Some(p);
            overrides.push((p, v));
        }
        Ok(Self {
            row_count,
            fill,
            overrides,
        })
    }
}

pub struct Sparse;

impl Encoding for Sparse {
    type Payload = SparsePayload;

    fn canonical_decode(payload: &Self::Payload) -> Result<Vec<i64>, CoveError> {
        let mut out = vec![payload.fill; payload.row_count as usize];
        for (p, v) in &payload.overrides {
            out[*p as usize] = *v;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_with_overrides() {
        let p = SparsePayload {
            row_count: 5,
            fill: 0,
            overrides: vec![(1, 42), (4, -7)],
        };
        assert_eq!(Sparse::canonical_decode(&p).unwrap(), vec![0, 42, 0, 0, -7]);
    }
}
