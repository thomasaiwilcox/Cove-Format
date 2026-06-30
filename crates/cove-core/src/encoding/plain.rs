//! Spec §20.3 — Plain encodings.
//!
//! * [`PlainFixed`] — fixed-width little-endian `i64` values.
//! * [`PlainVarint`] — unsigned LEB128 values.

use crate::wire;
use crate::CoveError;

use super::Encoding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlainFixedPayload {
    pub values: Vec<i64>,
}

impl PlainFixedPayload {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if !bytes.len().is_multiple_of(8) {
            return Err(CoveError::OffsetRange);
        }
        let n = bytes.len() / 8;
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            let off = i * 8;
            values.push(wire::read_i64_le_checked(bytes, off)?);
        }
        Ok(Self { values })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.values.len() * 8);
        for v in &self.values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}

pub struct PlainFixed;

impl Encoding for PlainFixed {
    type Payload = PlainFixedPayload;

    fn canonical_decode(payload: &Self::Payload) -> Result<Vec<i64>, CoveError> {
        Ok(payload.values.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlainVarintPayload {
    pub values: Vec<u64>,
}

impl PlainVarintPayload {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        let mut pos = 0usize;
        let mut values = Vec::new();
        while pos < bytes.len() {
            let (z, used) = wire::decode_u64_leb128(&bytes[pos..])?;
            pos += used;
            values.push(z);
        }
        Ok(Self { values })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for v in &self.values {
            wire::append_u64_leb128(&mut out, *v);
        }
        out
    }
}

pub struct PlainVarint;

impl Encoding for PlainVarint {
    type Payload = PlainVarintPayload;

    fn canonical_decode(payload: &Self::Payload) -> Result<Vec<i64>, CoveError> {
        payload
            .values
            .iter()
            .map(|value| i64::try_from(*value).map_err(|_| CoveError::ArithOverflow))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::assert_parity;

    #[test]
    fn plain_fixed_round_trip() {
        let p = PlainFixedPayload {
            values: vec![1, -2, 3, -4],
        };
        let bytes = p.encode();
        assert_eq!(PlainFixedPayload::parse(&bytes).unwrap(), p);
        assert!(assert_parity::<PlainFixed>(&p).is_ok());
    }

    #[test]
    fn plain_varint_round_trip() {
        let p = PlainVarintPayload {
            values: vec![0, 1, 2, 127, 128, i64::MAX as u64],
        };
        let bytes = p.encode();
        assert_eq!(PlainVarintPayload::parse(&bytes).unwrap(), p);
        assert!(assert_parity::<PlainVarint>(&p).is_ok());
    }

    #[test]
    fn plain_varint_payload_preserves_high_u64_values() {
        let p = PlainVarintPayload {
            values: vec![0, i64::MAX as u64 + 1, u64::MAX],
        };
        let bytes = p.encode();
        assert_eq!(PlainVarintPayload::parse(&bytes).unwrap(), p);
        assert_eq!(
            PlainVarint::canonical_decode(&p),
            Err(CoveError::ArithOverflow)
        );
    }
}
