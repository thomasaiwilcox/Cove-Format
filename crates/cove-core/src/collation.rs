//! Cove Format (COVE) v2.0 — Collation registry (Spec §22).
//!
//! Every COVE v2 reader MUST recognise the six minimum collations defined by the
//! spec. Each collation has a stable name and a deterministic comparison rule.
//! Comparisons are total orders so they can drive ColumnDomain rank maps,
//! min/max statistics, and ordered indexes safely.

use crate::{
    wire::{read_u16_le_checked, read_u32_le_checked},
    CoveError,
};

/// Total order produced by comparing two values under a collation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Ordering3 {
    Less,
    Equal,
    Greater,
}

impl From<std::cmp::Ordering> for Ordering3 {
    fn from(o: std::cmp::Ordering) -> Self {
        match o {
            std::cmp::Ordering::Less => Self::Less,
            std::cmp::Ordering::Equal => Self::Equal,
            std::cmp::Ordering::Greater => Self::Greater,
        }
    }
}

/// One of the six v2 collations, plus `None` for unspecified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CollationKind {
    /// `none` — equality only, no ordering allowed.
    None,
    /// `utf8-bytewise` — byte-by-byte comparison of UTF-8 encoded text.
    Utf8Bytewise,
    /// `unsigned-fixed-bytes` — lexicographic on fixed-width unsigned bytes.
    UnsignedFixedBytes,
    /// `signed-numeric` — two's-complement signed integers.
    SignedNumeric,
    /// `unsigned-numeric` — unsigned integers.
    UnsignedNumeric,
    /// `timestamp-chronological` — chronological ordering on timestamps.
    TimestampChronological,
}

impl CollationKind {
    /// Minimum spec collation ID.
    pub const fn id(self) -> u16 {
        match self {
            CollationKind::None => 0,
            CollationKind::Utf8Bytewise => 1,
            CollationKind::UnsignedFixedBytes => 2,
            CollationKind::SignedNumeric => 3,
            CollationKind::UnsignedNumeric => 4,
            CollationKind::TimestampChronological => 5,
        }
    }

    /// Look up one of the minimum v2 collations by ID.
    pub const fn from_id(id: u16) -> Option<Self> {
        match id {
            0 => Some(CollationKind::None),
            1 => Some(CollationKind::Utf8Bytewise),
            2 => Some(CollationKind::UnsignedFixedBytes),
            3 => Some(CollationKind::SignedNumeric),
            4 => Some(CollationKind::UnsignedNumeric),
            5 => Some(CollationKind::TimestampChronological),
            _ => None,
        }
    }

    /// Stable spec name for this collation.
    pub const fn name(self) -> &'static str {
        match self {
            CollationKind::None => "none",
            CollationKind::Utf8Bytewise => "utf8-bytewise",
            CollationKind::UnsignedFixedBytes => "unsigned-fixed-bytes",
            CollationKind::SignedNumeric => "signed-numeric",
            CollationKind::UnsignedNumeric => "unsigned-numeric",
            CollationKind::TimestampChronological => "timestamp-chronological",
        }
    }

    /// Look up a collation by its spec name.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "none" => CollationKind::None,
            "utf8-bytewise" => CollationKind::Utf8Bytewise,
            "unsigned-fixed-bytes" => CollationKind::UnsignedFixedBytes,
            "signed-numeric" => CollationKind::SignedNumeric,
            "unsigned-numeric" => CollationKind::UnsignedNumeric,
            "timestamp-chronological" => CollationKind::TimestampChronological,
            _ => return None,
        })
    }

    /// Whether this collation defines a total order (i.e. supports min/max,
    /// ColumnDomain ranks, and ordered indexes). `None` does not.
    pub const fn supports_ordering(self) -> bool {
        !matches!(self, CollationKind::None)
    }

    /// Compare two values under this collation. Returns `BadStats` when the
    /// inputs do not match the expected width for the collation.
    pub fn compare(self, lhs: &[u8], rhs: &[u8]) -> Result<Ordering3, CoveError> {
        match self {
            CollationKind::None => {
                // Equality only; ordering is not defined.
                if lhs == rhs {
                    Ok(Ordering3::Equal)
                } else {
                    Err(CoveError::BadStats)
                }
            }
            CollationKind::Utf8Bytewise | CollationKind::UnsignedFixedBytes => {
                Ok(lhs.cmp(rhs).into())
            }
            CollationKind::UnsignedNumeric => unsigned_numeric(lhs, rhs),
            CollationKind::SignedNumeric => signed_numeric(lhs, rhs),
            CollationKind::TimestampChronological => signed_numeric(lhs, rhs),
        }
    }
}

fn check_same_len(lhs: &[u8], rhs: &[u8]) -> Result<(), CoveError> {
    if lhs.len() != rhs.len() || !matches!(lhs.len(), 1 | 2 | 4 | 8 | 16) {
        Err(CoveError::BadStats)
    } else {
        Ok(())
    }
}

fn unsigned_numeric(lhs: &[u8], rhs: &[u8]) -> Result<Ordering3, CoveError> {
    check_same_len(lhs, rhs)?;
    Ok(read_unsigned(lhs).cmp(&read_unsigned(rhs)).into())
}

fn signed_numeric(lhs: &[u8], rhs: &[u8]) -> Result<Ordering3, CoveError> {
    check_same_len(lhs, rhs)?;
    Ok(read_signed(lhs).cmp(&read_signed(rhs)).into())
}

fn read_unsigned(b: &[u8]) -> u128 {
    let mut buf = [0u8; 16];
    let n = b.len();
    buf[..n].copy_from_slice(b);
    u128::from_le_bytes(buf)
}

fn read_signed(b: &[u8]) -> i128 {
    match b.len() {
        1 => i8::from_le_bytes([b[0]]) as i128,
        2 => {
            let mut bytes = [0u8; 2];
            bytes.copy_from_slice(b);
            i16::from_le_bytes(bytes) as i128
        }
        4 => {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(b);
            i32::from_le_bytes(bytes) as i128
        }
        8 => {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(b);
            i64::from_le_bytes(bytes) as i128
        }
        16 => {
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(b);
            i128::from_le_bytes(bytes)
        }
        _ => 0,
    }
}

/// The six v1 collations enumerated in spec order.
pub const V1_COLLATIONS: &[CollationKind] = &[
    CollationKind::None,
    CollationKind::Utf8Bytewise,
    CollationKind::UnsignedFixedBytes,
    CollationKind::SignedNumeric,
    CollationKind::UnsignedNumeric,
    CollationKind::TimestampChronological,
];

/// A collation entry, mapping a column or domain to a named collation.
#[derive(Debug, Clone)]
pub struct CollationEntry {
    /// File-local collation ID.
    pub collation_id: u16,
    /// Collation name (e.g. "utf8-bytewise").
    pub name: String,
    /// Collation version string.
    pub version: String,
    /// Entry flags.
    pub flags: u32,
    /// Resolved kind, if it matches a v1 collation.
    pub kind: Option<CollationKind>,
}

/// A parsed collation registry section.
#[derive(Debug, Clone, Default)]
pub struct CollationRegistry {
    pub entries: Vec<CollationEntry>,
}

impl CollationRegistry {
    /// Parse a collation registry section.
    ///
    /// Wire format: `CollationRegistryHeaderV2`, then entries of:
    /// `u16 collation_id`, `u16 name_len`, name bytes, `u16 version_len`,
    /// version bytes, `u32 flags`.
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < 8 {
            return Err(CoveError::BufferTooShort);
        }
        let entry_count = read_u32_le_checked(bytes, 0)?;
        let flags = read_u32_le_checked(bytes, 4)?;
        if flags != 0 {
            return Err(CoveError::BadSection(
                "collation registry reserved flags must be zero".into(),
            ));
        }
        let mut pos = 8usize;
        let mut entries = Vec::with_capacity(entry_count as usize);
        let mut seen_ids = std::collections::BTreeSet::new();

        for _ in 0..entry_count {
            if pos.checked_add(4).ok_or(CoveError::ArithOverflow)? > bytes.len() {
                return Err(CoveError::BufferTooShort);
            }
            let collation_id = read_u16_le_checked(bytes, pos)?;
            pos += 2;
            let name_len = read_u16_le_checked(bytes, pos)? as usize;
            pos += 2;
            let name_end = pos.checked_add(name_len).ok_or(CoveError::ArithOverflow)?;
            if name_end > bytes.len() {
                return Err(CoveError::BufferTooShort);
            }
            let name = std::str::from_utf8(&bytes[pos..name_end])
                .map_err(|_| CoveError::BadSection("collation name is not valid UTF-8".into()))?
                .to_string();
            pos = name_end;

            if pos.checked_add(2).ok_or(CoveError::ArithOverflow)? > bytes.len() {
                return Err(CoveError::BufferTooShort);
            }
            let version_len = read_u16_le_checked(bytes, pos)? as usize;
            pos += 2;
            let version_end = pos
                .checked_add(version_len)
                .ok_or(CoveError::ArithOverflow)?;
            if version_end > bytes.len() {
                return Err(CoveError::BufferTooShort);
            }
            let version = std::str::from_utf8(&bytes[pos..version_end])
                .map_err(|_| CoveError::BadSection("collation version is not valid UTF-8".into()))?
                .to_string();
            pos = version_end;

            if pos.checked_add(4).ok_or(CoveError::ArithOverflow)? > bytes.len() {
                return Err(CoveError::BufferTooShort);
            }
            let flags = read_u32_le_checked(bytes, pos)?;
            pos += 4;

            if !seen_ids.insert(collation_id) {
                return Err(CoveError::BadSection(
                    "collation registry has duplicate collation_id".into(),
                ));
            }
            let kind = CollationKind::from_name(&name);
            if let Some(minimum_kind) = CollationKind::from_id(collation_id) {
                if Some(minimum_kind) != kind {
                    return Err(CoveError::BadSection(
                        "minimum collation IDs must use their spec-defined names".into(),
                    ));
                }
            }
            entries.push(CollationEntry {
                collation_id,
                name,
                version,
                flags,
                kind,
            });
        }
        if pos != bytes.len() {
            return Err(CoveError::BadSection(
                "collation registry has trailing bytes".into(),
            ));
        }
        Ok(Self { entries })
    }

    /// Inverse of [`Self::parse`]; produces canonical bytes that round-trip.
    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        let mut out = Vec::with_capacity(4 + self.entries.len() * 8);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for entry in &self.entries {
            let name_bytes = entry.name.as_bytes();
            let version_bytes = entry.version.as_bytes();
            let name_len = u16::try_from(name_bytes.len()).map_err(|_| {
                CoveError::BadSection("collation name exceeds u16 length limit".into())
            })?;
            let version_len = u16::try_from(version_bytes.len()).map_err(|_| {
                CoveError::BadSection("collation version exceeds u16 length limit".into())
            })?;
            out.extend_from_slice(&entry.collation_id.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(&version_len.to_le_bytes());
            out.extend_from_slice(version_bytes);
            out.extend_from_slice(&entry.flags.to_le_bytes());
        }
        Ok(out)
    }

    /// Returns true if every named collation is one of the six v2 collations.
    pub fn all_known(&self) -> bool {
        self.entries.iter().all(|e| e.kind.is_some())
    }

    /// Whether a given collation name is recognised by this v2 reader.
    pub fn is_known_collation(name: &str) -> bool {
        CollationKind::from_name(name).is_some()
    }

    /// Whether an ID names one of the minimum collations and supports ordering.
    pub fn is_ordering_collation_id(id: u16) -> bool {
        CollationKind::from_id(id)
            .map(CollationKind::supports_ordering)
            .unwrap_or(false)
    }

    /// Resolve an ID to a supported collation kind. Minimum spec IDs are fixed
    /// and cannot be overridden by registry entries.
    pub fn kind_for_id(&self, id: u16) -> Option<CollationKind> {
        CollationKind::from_id(id).or_else(|| {
            self.entries
                .iter()
                .find(|entry| entry.collation_id == id)
                .and_then(|entry| entry.kind)
        })
    }
}

#[cfg(test)]
mod serialize_tests {
    use super::*;

    #[test]
    fn serialize_round_trip() {
        let reg = CollationRegistry {
            entries: vec![
                CollationEntry {
                    collation_id: 1,
                    name: "utf8-bytewise".into(),
                    version: "v2".into(),
                    flags: 0,
                    kind: Some(CollationKind::Utf8Bytewise),
                },
                CollationEntry {
                    collation_id: 100,
                    name: "vendor-x".into(),
                    version: "2026".into(),
                    flags: 0,
                    kind: None,
                },
            ],
        };
        let bytes = reg.serialize().unwrap();
        let back = CollationRegistry::parse(&bytes).unwrap();
        assert_eq!(back.entries.len(), 2);
        assert_eq!(back.entries[0].name, "utf8-bytewise");
        assert_eq!(back.entries[1].collation_id, 100);
        assert_eq!(back.entries[1].version, "2026");
    }

    #[test]
    fn serialize_empty() {
        let reg = CollationRegistry::default();
        let bytes = reg.serialize().unwrap();
        assert_eq!(bytes, vec![0u8; 8]);
        assert!(CollationRegistry::parse(&bytes).unwrap().entries.is_empty());
    }

    #[test]
    fn serialize_rejects_name_longer_than_u16() {
        let reg = CollationRegistry {
            entries: vec![CollationEntry {
                collation_id: 1,
                name: "a".repeat(usize::from(u16::MAX) + 1),
                version: String::new(),
                flags: 0,
                kind: None,
            }],
        };

        assert!(matches!(reg.serialize(), Err(CoveError::BadSection(_))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry_bytes(entries: &[(u16, &str, &str)]) -> Vec<u8> {
        let mut out = (entries.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&0u32.to_le_bytes());
        for (id, name, version) in entries {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(version.len() as u16).to_le_bytes());
            out.extend_from_slice(version.as_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out
    }

    #[test]
    fn empty_registry_parses() {
        let reg = CollationRegistry::parse(&make_registry_bytes(&[])).unwrap();
        assert_eq!(reg.entries.len(), 0);
        assert!(reg.all_known());
    }

    #[test]
    fn spec_22_v1_collations_all_resolve() {
        for c in V1_COLLATIONS {
            assert_eq!(CollationKind::from_name(c.name()), Some(*c));
            assert_eq!(CollationKind::from_id(c.id()), Some(*c));
        }
    }

    #[test]
    fn spec_22_none_supports_only_equality() {
        let none = CollationKind::None;
        assert!(!none.supports_ordering());
        assert_eq!(none.compare(b"a", b"a"), Ok(Ordering3::Equal));
        assert!(matches!(none.compare(b"a", b"b"), Err(CoveError::BadStats)));
    }

    #[test]
    fn utf8_bytewise_orders_strings_lexicographically() {
        let c = CollationKind::Utf8Bytewise;
        assert_eq!(c.compare(b"abc", b"abd"), Ok(Ordering3::Less));
        assert_eq!(c.compare(b"abc", b"abc"), Ok(Ordering3::Equal));
    }

    #[test]
    fn unsigned_fixed_bytes_orders_by_byte_lex() {
        let c = CollationKind::UnsignedFixedBytes;
        assert_eq!(
            c.compare(&[0x01, 0x00], &[0x00, 0xff]),
            Ok(Ordering3::Greater)
        );
    }

    #[test]
    fn signed_numeric_handles_negative_values() {
        let c = CollationKind::SignedNumeric;
        // -1i32 vs 0i32: -1 < 0
        let neg_one = (-1i32).to_le_bytes();
        let zero = 0i32.to_le_bytes();
        assert_eq!(c.compare(&neg_one, &zero), Ok(Ordering3::Less));
    }

    #[test]
    fn unsigned_numeric_orders_by_value_not_bytes() {
        let c = CollationKind::UnsignedNumeric;
        // 0x0001 < 0x0100 numerically but byte-lex says 0x01 0x00 > 0x00 0x01
        let small = 1u16.to_le_bytes();
        let big = 256u16.to_le_bytes();
        assert_eq!(c.compare(&small, &big), Ok(Ordering3::Less));
    }

    #[test]
    fn timestamp_chronological_uses_signed_compare() {
        let c = CollationKind::TimestampChronological;
        let earlier = (-100i64).to_le_bytes();
        let later = 100i64.to_le_bytes();
        assert_eq!(c.compare(&earlier, &later), Ok(Ordering3::Less));
    }

    #[test]
    fn known_collation_check() {
        assert!(CollationRegistry::is_known_collation("utf8-bytewise"));
        assert!(!CollationRegistry::is_known_collation("utf8-icu"));
    }

    #[test]
    fn registry_with_v1_entries_resolves_all() {
        let bytes = make_registry_bytes(&[(1, "utf8-bytewise", ""), (3, "signed-numeric", "")]);
        let reg = CollationRegistry::parse(&bytes).unwrap();
        assert_eq!(reg.entries.len(), 2);
        assert!(reg.all_known());
    }

    #[test]
    fn registry_with_unknown_entry_is_not_all_known() {
        let bytes = make_registry_bytes(&[(100, "vendor-magic", "1")]);
        let reg = CollationRegistry::parse(&bytes).unwrap();
        assert!(!reg.all_known());
    }

    #[test]
    fn registry_rejects_trailing_bytes() {
        let mut bytes = make_registry_bytes(&[(1, "utf8-bytewise", "")]);
        bytes.push(0);
        assert!(matches!(
            CollationRegistry::parse(&bytes),
            Err(CoveError::BadSection(_))
        ));
    }

    #[test]
    fn registry_rejects_duplicate_collation_id() {
        let bytes = make_registry_bytes(&[(100, "utf8-bytewise", ""), (100, "utf8-bytewise", "")]);
        assert!(matches!(
            CollationRegistry::parse(&bytes),
            Err(CoveError::BadSection(_))
        ));
    }

    #[test]
    fn registry_rejects_minimum_id_override() {
        let bytes = make_registry_bytes(&[(1, "signed-numeric", "")]);
        assert!(matches!(
            CollationRegistry::parse(&bytes),
            Err(CoveError::BadSection(_))
        ));
    }
}
