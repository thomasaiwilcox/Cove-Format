use crate::{
    array::{CoveArrayValue, EncodedArray},
    canonical::CanonicalValue,
    constants::{CoveLogicalType, CovePhysicalKind, ValueTag},
    dictionary::{DictionaryValue, FileDictionary, FileDictionaryView},
    page_payload::{ColumnPagePayloadV1, PageBufferKind},
    page_validation::{
        materialize_stats_only_constant_page_payload, StatsOnlyPageMaterializationContext,
    },
    trust_chain,
    types::{
        numcode_as_date_days, numcode_as_decimal64, numcode_as_f32, numcode_as_f64, numcode_as_i16,
        numcode_as_i32, numcode_as_i64, numcode_as_i8, numcode_as_timestamp_micros,
        numcode_as_timestamp_nanos, numcode_as_u16, numcode_as_u32, numcode_as_u64, numcode_as_u8,
    },
    validity::ValidityBitmap,
    CoveError,
};

use super::{segment::TemporalSegmentData, TRUST_MANIFEST_ENTRY_LEN};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustManifestEntryV1 {
    pub segment_id: u32,
    pub row_index: u32,
    pub expected_hash: [u8; 32],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustManifest {
    pub entries: Vec<TrustManifestEntryV1>,
}

impl TrustManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, CoveError> {
        if bytes.len() < 4 {
            return Err(CoveError::BufferTooShort);
        }
        let entry_count = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let needed = 4usize
            .checked_add(
                entry_count
                    .checked_mul(TRUST_MANIFEST_ENTRY_LEN)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .ok_or(CoveError::ArithOverflow)?;
        if needed > bytes.len() {
            return Err(CoveError::BufferTooShort);
        }
        let mut entries = Vec::with_capacity(entry_count);
        let mut pos = 4usize;
        for _ in 0..entry_count {
            let mut expected_hash = [0u8; 32];
            expected_hash.copy_from_slice(&bytes[pos + 8..pos + 40]);
            entries.push(TrustManifestEntryV1 {
                segment_id: u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()),
                row_index: u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()),
                expected_hash,
            });
            pos += TRUST_MANIFEST_ENTRY_LEN;
        }
        Ok(Self { entries })
    }

    /// Inverse of [`Self::parse`]; produces canonical bytes that round-trip.
    pub fn serialize(&self) -> Result<Vec<u8>, CoveError> {
        let entry_count = u32::try_from(self.entries.len())
            .map_err(|_| CoveError::BadSchema("too many trust manifest entries".into()))?;
        let capacity = 4usize
            .checked_add(
                self.entries
                    .len()
                    .checked_mul(TRUST_MANIFEST_ENTRY_LEN)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .ok_or(CoveError::ArithOverflow)?;
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&entry_count.to_le_bytes());
        for e in &self.entries {
            out.extend_from_slice(&e.segment_id.to_le_bytes());
            out.extend_from_slice(&e.row_index.to_le_bytes());
            out.extend_from_slice(&e.expected_hash);
        }
        Ok(out)
    }

    pub fn verify_against(&self, segments: &[TemporalSegmentData]) -> Result<(), CoveError> {
        self.verify_against_with_dictionary::<FileDictionaryView<'_>>(segments, None, &[])
    }

    pub fn verify_against_with_dictionary<D: TrustDictionary + ?Sized>(
        &self,
        segments: &[TemporalSegmentData],
        dictionary: Option<&D>,
        zone_stats: &[crate::zone_stats::ZoneStatsEntry],
    ) -> Result<(), CoveError> {
        let mut prev = [0u8; 32];
        for entry in &self.entries {
            let segment = segments
                .iter()
                .find(|segment| segment.header.segment_id == entry.segment_id)
                .ok_or(CoveError::RefInvalid)?;
            let payload =
                temporal_row_trust_payload(segment, entry.row_index, dictionary, zone_stats)?;
            let computed = trust_chain::chain(&prev, &payload)?;
            if computed != entry.expected_hash {
                return Err(CoveError::DigestMismatch);
            }
            prev = computed;
        }
        Ok(())
    }
}

pub trait TrustDictionary {
    fn len(&self) -> u32;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn canonical_entry(&self, file_code: u32) -> Result<(ValueTag, Vec<u8>), CoveError>;
}

impl TrustDictionary for FileDictionary {
    fn len(&self) -> u32 {
        self.len()
    }

    fn canonical_entry(&self, file_code: u32) -> Result<(ValueTag, Vec<u8>), CoveError> {
        let entry = self.get_entry(file_code)?;
        let tag = ValueTag::from_u16(entry.value_tag).ok_or(CoveError::BadFileCode)?;
        match self.decode_value(file_code)? {
            DictionaryValue::RawBytes(bytes) => Ok((tag, bytes)),
            DictionaryValue::RedactedPresent => Err(CoveError::UnsupportedEncoding(
                "trust verification cannot hash redacted dictionary payload bytes".into(),
            )),
        }
    }
}

impl TrustDictionary for FileDictionaryView<'_> {
    fn len(&self) -> u32 {
        self.len()
    }

    fn canonical_entry(&self, file_code: u32) -> Result<(ValueTag, Vec<u8>), CoveError> {
        let entry = self.get_entry(file_code)?;
        let tag = ValueTag::from_u16(entry.value_tag).ok_or(CoveError::BadFileCode)?;
        match self.decode_value(file_code)? {
            DictionaryValue::RawBytes(bytes) => Ok((tag, bytes)),
            DictionaryValue::RedactedPresent => Err(CoveError::UnsupportedEncoding(
                "trust verification cannot hash redacted dictionary payload bytes".into(),
            )),
        }
    }
}

pub fn temporal_row_trust_payload<D: TrustDictionary + ?Sized>(
    segment: &TemporalSegmentData,
    row_index: u32,
    dictionary: Option<&D>,
    zone_stats: &[crate::zone_stats::ZoneStatsEntry],
) -> Result<Vec<u8>, CoveError> {
    let row = segment
        .rows
        .get(row_index as usize)
        .ok_or(CoveError::RefInvalid)?;
    let mut out = row.trust_payload();
    if segment.property_columns.is_empty() {
        return Ok(out);
    }

    out.extend_from_slice(b"COVE-O-TRUST-PROPS-V1\0");
    let column_count = u32::try_from(segment.property_columns.len())
        .map_err(|_| CoveError::BadSchema("too many temporal property columns".into()))?;
    out.extend_from_slice(&column_count.to_le_bytes());

    let mut columns = segment.property_columns.iter().collect::<Vec<_>>();
    columns.sort_by_key(|column| column.directory.column_id);
    for column in columns {
        let (tag, canonical) =
            property_value_for_row(segment, column, row_index, dictionary, zone_stats)?;
        out.extend_from_slice(&column.directory.column_id.to_le_bytes());
        out.extend_from_slice(&(column.directory.logical_type as u16).to_le_bytes());
        out.push(column.directory.physical_kind as u8);
        out.extend_from_slice(&(tag as u16).to_le_bytes());
        let len = u64::try_from(canonical.len()).map_err(|_| CoveError::ArithOverflow)?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&canonical);
    }
    Ok(out)
}

fn property_value_for_row<D: TrustDictionary + ?Sized>(
    segment: &TemporalSegmentData,
    column: &super::segment::TemporalPropertyColumn,
    row_index: u32,
    dictionary: Option<&D>,
    zone_stats: &[crate::zone_stats::ZoneStatsEntry],
) -> Result<(ValueTag, Vec<u8>), CoveError> {
    if segment.header.morsel_row_count == 0 {
        return Err(CoveError::SegmentCorrupt);
    }
    let morsel_id = row_index / segment.header.morsel_row_count;
    let first_row = morsel_id
        .checked_mul(segment.header.morsel_row_count)
        .ok_or(CoveError::ArithOverflow)?;
    let local_row = row_index
        .checked_sub(first_row)
        .ok_or(CoveError::SegmentCorrupt)?;
    let page = column
        .pages
        .iter()
        .find(|page| page.index_entry.morsel_id == morsel_id)
        .ok_or(CoveError::PageCorrupt)?;

    let materialized = if page.payload.is_none() {
        Some(materialize_stats_only_constant_page_payload(
            StatsOnlyPageMaterializationContext {
                table_id: None,
                segment_id: Some(segment.header.segment_id),
                column_id: column.directory.column_id,
                logical_type: column.directory.logical_type,
                physical_kind: column.directory.physical_kind,
                dictionary_len: dictionary.map(|dictionary| dictionary.len()),
                zone_stats,
            },
            &page.index_entry,
        )?)
    } else {
        None
    };
    let parsed_materialized = materialized
        .as_deref()
        .map(ColumnPagePayloadV1::parse)
        .transpose()?;
    let payload = page
        .payload
        .as_ref()
        .or(parsed_materialized.as_ref())
        .ok_or(CoveError::PageCorrupt)?;

    let root = payload.root_node()?;
    let null_bitmap = payload.buffer_bytes(PageBufferKind::NullBitmap)?;
    let validity =
        null_bitmap.map(|bytes| ValidityBitmap::new(bytes, u64::from(page.index_entry.row_count)));
    if let Some(ref validity) = validity {
        validity.validate_len(u64::from(page.index_entry.row_count))?;
    }
    let values = payload.buffer_bytes(PageBufferKind::Values)?.unwrap_or(&[]);
    let array = EncodedArray::new(
        column.directory.logical_type,
        column.directory.physical_kind,
        u64::from(page.index_entry.row_count),
        root.encoding_kind,
        validity,
        values,
        None,
    );
    canonical_for_array_value(
        column.directory.logical_type,
        column.directory.physical_kind,
        array.prepare()?.decode_row(u64::from(local_row))?,
        dictionary,
    )
}

fn canonical_for_array_value<D: TrustDictionary + ?Sized>(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    value: CoveArrayValue<'_>,
    dictionary: Option<&D>,
) -> Result<(ValueTag, Vec<u8>), CoveError> {
    match value {
        CoveArrayValue::Null => Ok((ValueTag::Null, Vec::new())),
        CoveArrayValue::FileCode(code) => dictionary
            .ok_or(CoveError::BadFileCode)?
            .canonical_entry(code),
        CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
            Ok((value_tag_for_logical(logical)?, bytes))
        }
        CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => {
            Err(CoveError::UnsupportedEncoding(
                "trust verification cannot hash redacted dictionary payload bytes".into(),
            ))
        }
        CoveArrayValue::Boolean(value) | CoveArrayValue::ValidityBit(value) => Ok(bool_tag(value)),
        CoveArrayValue::NumCode(value) | CoveArrayValue::Varint(value) => {
            canonical_for_u64(logical, physical, value)
        }
        CoveArrayValue::Int64(value) => canonical_for_i64(logical, value),
        CoveArrayValue::Bytes(bytes) => canonical_for_raw_bytes(logical, physical, bytes),
        CoveArrayValue::OwnedBytes(bytes) => canonical_for_raw_bytes(logical, physical, &bytes),
    }
}

fn bool_tag(value: bool) -> (ValueTag, Vec<u8>) {
    (
        if value {
            ValueTag::BoolTrue
        } else {
            ValueTag::BoolFalse
        },
        Vec::new(),
    )
}

fn canonical_for_u64(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    value: u64,
) -> Result<(ValueTag, Vec<u8>), CoveError> {
    if physical == CovePhysicalKind::Boolean || logical == CoveLogicalType::Bool {
        return match value {
            0 => Ok(bool_tag(false)),
            1 => Ok(bool_tag(true)),
            _ => Err(CoveError::PageCorrupt),
        };
    }
    match logical {
        CoveLogicalType::Int8 => canonical_for_i64(logical, i64::from(numcode_as_i8(value))),
        CoveLogicalType::Int16 => canonical_for_i64(logical, i64::from(numcode_as_i16(value))),
        CoveLogicalType::Int32 => canonical_for_i64(logical, i64::from(numcode_as_i32(value))),
        CoveLogicalType::Int64 => canonical_for_i64(logical, numcode_as_i64(value)),
        CoveLogicalType::UInt8 => Ok((
            ValueTag::UInt64,
            u64::from(numcode_as_u8(value)).to_le_bytes().to_vec(),
        )),
        CoveLogicalType::UInt16 => Ok((
            ValueTag::UInt64,
            u64::from(numcode_as_u16(value)).to_le_bytes().to_vec(),
        )),
        CoveLogicalType::UInt32 => Ok((
            ValueTag::UInt64,
            u64::from(numcode_as_u32(value)).to_le_bytes().to_vec(),
        )),
        CoveLogicalType::UInt64 => Ok((
            ValueTag::UInt64,
            numcode_as_u64(value).to_le_bytes().to_vec(),
        )),
        CoveLogicalType::Float32 => Ok((
            ValueTag::Float32Bits,
            numcode_as_f32(value).to_bits().to_le_bytes().to_vec(),
        )),
        CoveLogicalType::Float64 => Ok((
            ValueTag::Float64Bits,
            numcode_as_f64(value).to_bits().to_le_bytes().to_vec(),
        )),
        CoveLogicalType::Decimal64 => canonical_for_i64(logical, numcode_as_decimal64(value)),
        CoveLogicalType::DateDays => Ok((
            ValueTag::DateDays,
            numcode_as_date_days(value).to_le_bytes().to_vec(),
        )),
        CoveLogicalType::TimestampMicros => {
            canonical_for_i64(logical, numcode_as_timestamp_micros(value))
        }
        CoveLogicalType::TimestampNanos => {
            canonical_for_i64(logical, numcode_as_timestamp_nanos(value))
        }
        _ => Err(CoveError::UnsupportedEncoding(format!(
            "cannot canonicalize numeric trust value for {logical:?}"
        ))),
    }
}

fn canonical_for_i64(
    logical: CoveLogicalType,
    value: i64,
) -> Result<(ValueTag, Vec<u8>), CoveError> {
    match logical {
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => Ok((ValueTag::Int64, value.to_le_bytes().to_vec())),
        CoveLogicalType::TimestampMicros => {
            Ok((ValueTag::TimestampMicros, value.to_le_bytes().to_vec()))
        }
        CoveLogicalType::TimestampNanos => {
            Ok((ValueTag::TimestampNanos, value.to_le_bytes().to_vec()))
        }
        CoveLogicalType::Decimal64 => Ok((ValueTag::Decimal64, value.to_le_bytes().to_vec())),
        CoveLogicalType::DateDays => {
            let value = i32::try_from(value).map_err(|_| CoveError::PageCorrupt)?;
            Ok((ValueTag::DateDays, value.to_le_bytes().to_vec()))
        }
        _ => Err(CoveError::UnsupportedEncoding(format!(
            "cannot canonicalize signed trust value for {logical:?}"
        ))),
    }
}

fn canonical_for_raw_bytes(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    bytes: &[u8],
) -> Result<(ValueTag, Vec<u8>), CoveError> {
    if physical == CovePhysicalKind::Boolean || logical == CoveLogicalType::Bool {
        return match bytes {
            [0] => Ok(bool_tag(false)),
            [1] => Ok(bool_tag(true)),
            _ => Err(CoveError::PageCorrupt),
        };
    }
    match logical {
        CoveLogicalType::Int8 => Ok((
            ValueTag::Int64,
            i64::from(i8::from_le_bytes(read_fixed(bytes)?))
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::Int16 => Ok((
            ValueTag::Int64,
            i64::from(i16::from_le_bytes(read_fixed(bytes)?))
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::Int32 => Ok((
            ValueTag::Int64,
            i64::from(i32::from_le_bytes(read_fixed(bytes)?))
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::Int64 => Ok((
            ValueTag::Int64,
            i64::from_le_bytes(read_fixed(bytes)?)
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::UInt8 => Ok((
            ValueTag::UInt64,
            u64::from(u8::from_le_bytes(read_fixed(bytes)?))
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::UInt16 => Ok((
            ValueTag::UInt64,
            u64::from(u16::from_le_bytes(read_fixed(bytes)?))
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::UInt32 => Ok((
            ValueTag::UInt64,
            u64::from(u32::from_le_bytes(read_fixed(bytes)?))
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::UInt64 => Ok((
            ValueTag::UInt64,
            u64::from_le_bytes(read_fixed(bytes)?)
                .to_le_bytes()
                .to_vec(),
        )),
        CoveLogicalType::Bool => match bytes {
            [0] => Ok(bool_tag(false)),
            [1] => Ok(bool_tag(true)),
            _ => Err(CoveError::PageCorrupt),
        },
        CoveLogicalType::Float32 => Ok((
            ValueTag::Float32Bits,
            <[u8; 4]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::Float64 => Ok((
            ValueTag::Float64Bits,
            <[u8; 8]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::Decimal64 => Ok((
            ValueTag::Decimal64,
            <[u8; 8]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::Decimal128 => Ok((
            ValueTag::Decimal128,
            <[u8; 16]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::DateDays => Ok((
            ValueTag::DateDays,
            <[u8; 4]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::TimestampMicros => Ok((
            ValueTag::TimestampMicros,
            <[u8; 8]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::TimestampNanos => Ok((
            ValueTag::TimestampNanos,
            <[u8; 8]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::Uuid => Ok((
            ValueTag::Uuid,
            <[u8; 16]>::try_from(bytes)
                .map_err(|_| CoveError::PageCorrupt)?
                .to_vec(),
        )),
        CoveLogicalType::Utf8 => Ok((
            ValueTag::Utf8,
            CanonicalValue::Utf8(std::str::from_utf8(bytes).map_err(|_| CoveError::PageCorrupt)?)
                .encode()?,
        )),
        CoveLogicalType::Binary => Ok((ValueTag::Binary, CanonicalValue::Bytes(bytes).encode()?)),
        CoveLogicalType::Json => Ok((
            ValueTag::Json,
            CanonicalValue::Json(std::str::from_utf8(bytes).map_err(|_| CoveError::PageCorrupt)?)
                .encode()?,
        )),
        CoveLogicalType::Null => Ok((ValueTag::Null, Vec::new())),
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            Ok((value_tag_for_logical(logical)?, bytes.to_vec()))
        }
    }
}

fn value_tag_for_logical(logical: CoveLogicalType) -> Result<ValueTag, CoveError> {
    match logical {
        CoveLogicalType::Null => Ok(ValueTag::Null),
        CoveLogicalType::Bool => Err(CoveError::BadFileCode),
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => Ok(ValueTag::Int64),
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => Ok(ValueTag::UInt64),
        CoveLogicalType::Float32 => Ok(ValueTag::Float32Bits),
        CoveLogicalType::Float64 => Ok(ValueTag::Float64Bits),
        CoveLogicalType::Decimal64 => Ok(ValueTag::Decimal64),
        CoveLogicalType::Decimal128 => Ok(ValueTag::Decimal128),
        CoveLogicalType::DateDays => Ok(ValueTag::DateDays),
        CoveLogicalType::TimestampMicros => Ok(ValueTag::TimestampMicros),
        CoveLogicalType::TimestampNanos => Ok(ValueTag::TimestampNanos),
        CoveLogicalType::Utf8 => Ok(ValueTag::Utf8),
        CoveLogicalType::Binary => Ok(ValueTag::Binary),
        CoveLogicalType::Uuid => Ok(ValueTag::Uuid),
        CoveLogicalType::Json => Ok(ValueTag::Json),
        CoveLogicalType::List => Ok(ValueTag::List),
        CoveLogicalType::Struct => Ok(ValueTag::Struct),
        CoveLogicalType::Map => Ok(ValueTag::Map),
    }
}

fn read_fixed<const N: usize>(bytes: &[u8]) -> Result<[u8; N], CoveError> {
    bytes.try_into().map_err(|_| CoveError::PageCorrupt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_numcode_canonicalization_uses_declared_logical_type() {
        assert_eq!(
            canonical_for_u64(CoveLogicalType::Int8, CovePhysicalKind::NumCode, 0xff).unwrap(),
            (ValueTag::Int64, (-1i64).to_le_bytes().to_vec())
        );
        assert_eq!(
            canonical_for_u64(CoveLogicalType::UInt8, CovePhysicalKind::NumCode, 0x1ff).unwrap(),
            (ValueTag::UInt64, 255u64.to_le_bytes().to_vec())
        );
        assert_eq!(
            canonical_for_u64(
                CoveLogicalType::DateDays,
                CovePhysicalKind::NumCode,
                0xffff_ffff,
            )
            .unwrap(),
            (ValueTag::DateDays, (-1i32).to_le_bytes().to_vec())
        );

        let f32_bits = 0x7fc0_0042u32;
        assert_eq!(
            canonical_for_u64(
                CoveLogicalType::Float32,
                CovePhysicalKind::NumCode,
                u64::from(f32_bits),
            )
            .unwrap(),
            (ValueTag::Float32Bits, f32_bits.to_le_bytes().to_vec())
        );

        let f64_bits = 0x7ff8_0000_0000_0042u64;
        assert_eq!(
            canonical_for_u64(
                CoveLogicalType::Float64,
                CovePhysicalKind::NumCode,
                f64_bits
            )
            .unwrap(),
            (ValueTag::Float64Bits, f64_bits.to_le_bytes().to_vec())
        );
    }
}
