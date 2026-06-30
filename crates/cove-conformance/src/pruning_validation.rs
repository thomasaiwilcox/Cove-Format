use super::*;

#[derive(Debug)]
struct PruningColumnFixture {
    column_id: u32,
    zone_stats: Option<ZoneStatsEntry>,
    domain: Option<ColumnDomain>,
    exact_set: Option<ExactSetIndex>,
    bloom: Option<BloomFilterIndex>,
    bloom_fail_open: bool,
    inverted: Option<InvertedMorselIndex>,
    inverted_fail_open: bool,
    lookup: Option<LookupIndex>,
    lookup_fail_open: bool,
    composite: Option<CompositeIndex>,
    composite_fail_open: bool,
    composite_matches_bindings: bool,
    aggregate: Option<AggregateSynopsis>,
    aggregate_fail_open: bool,
    aggregate_proves_no_match: bool,
}

/// Spec §10 — wire-format primitives (varint LEB128, ZigZag, strict bool).
///
/// Fixture shape:
/// ```json
/// { "op": "varint_round_trip",   "value": <u64>,  "expect_bytes": [u8...] }
/// { "op": "varint_decode_reject", "input": [u8...], "reason": "..." }
/// { "op": "zigzag_round_trip",   "value": <i64>,  "expect_zigzag": <u64> }
/// { "op": "bool_strict",         "byte": <u8>, "expect": <bool> }
/// { "op": "bool_strict_reject",  "byte": <u8> }
/// ```
pub(super) fn validate_wire_primitive_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CoveError::BadSection(format!("invalid wire fixture json: {err}")))?;
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("wire fixture missing op".into()))?;
    match op {
        "varint_round_trip" => {
            let n = value
                .get("value")
                .and_then(Value::as_u64)
                .ok_or_else(|| CoveError::BadSection("varint fixture missing value".into()))?;
            let expected = parse_fixture_byte_vector(value.get("expect_bytes"), "expect_bytes")?;
            let actual = encode_u64_leb128(n);
            if actual != expected {
                return Err(CoveError::BadSection(format!(
                    "varint encode mismatch for {n}: expected {:?}, got {:?}",
                    expected, actual
                )));
            }
            let (decoded, used) = decode_u64_leb128(&actual)?;
            if decoded != n || used != actual.len() {
                return Err(CoveError::BadSection(format!(
                    "varint round-trip mismatch for {n}: decoded={decoded}, used={used}, len={}",
                    actual.len()
                )));
            }
            Ok(())
        }
        "varint_decode_reject" => {
            let input = parse_fixture_byte_vector(value.get("input"), "input")?;
            if decode_u64_leb128(&input).is_ok() {
                return Err(CoveError::BadSection(
                    "varint_decode_reject input was accepted".into(),
                ));
            }
            Ok(())
        }
        "zigzag_round_trip" => {
            let n = value
                .get("value")
                .and_then(Value::as_i64)
                .ok_or_else(|| CoveError::BadSection("zigzag fixture missing value".into()))?;
            let expected = value
                .get("expect_zigzag")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    CoveError::BadSection("zigzag fixture missing expect_zigzag".into())
                })?;
            let encoded = zigzag_encode_i64(n);
            if encoded != expected {
                return Err(CoveError::BadSection(format!(
                    "zigzag encode mismatch for {n}: expected {expected}, got {encoded}"
                )));
            }
            if zigzag_decode_i64(encoded) != n {
                return Err(CoveError::BadSection(format!(
                    "zigzag decode mismatch for {n}: got {}",
                    zigzag_decode_i64(encoded)
                )));
            }
            Ok(())
        }
        "bool_strict" => {
            let byte =
                value.get("byte").and_then(Value::as_u64).ok_or_else(|| {
                    CoveError::BadSection("bool_strict fixture missing byte".into())
                })? as u8;
            let expected = value
                .get("expect")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    CoveError::BadSection("bool_strict fixture missing expect".into())
                })?;
            let actual = parse_bool_strict(byte)?;
            if actual != expected {
                return Err(CoveError::BadSection(format!(
                    "bool_strict mismatch: expected {expected}, got {actual}"
                )));
            }
            Ok(())
        }
        "bool_strict_reject" => {
            let byte = value.get("byte").and_then(Value::as_u64).ok_or_else(|| {
                CoveError::BadSection("bool_strict_reject fixture missing byte".into())
            })? as u8;
            if parse_bool_strict(byte).is_ok() {
                return Err(CoveError::BadSection(format!(
                    "bool_strict_reject byte {byte} was accepted"
                )));
            }
            Ok(())
        }
        other => Err(CoveError::BadSection(format!(
            "wire_primitive_case unknown op {other:?}"
        ))),
    }
}

/// Spec §66 / §27 — exercise page-level compression and validation.
///
/// Fixture shape:
/// ```json
/// {
///   "codec": "none" | "lz4" | "zstd",
///   "payload": "<utf-8 string used as the uncompressed page bytes>",
///   "expect": "round_trip" | "parse_reject" | "decode_reject",
///   // optional overrides applied before serializing the entry:
///   "page_length_override":         <u64?>,
///   "uncompressed_length_override": <u64?>,
///   "flags_override":               <u32?>,
///   "row_count_override":           <u32?>,
///   "non_null_count_override":      <u32?>,
///   "null_count_override":          <u32?>,
///   "encoding_root_override":       <u32?>,
///   "page_offset_override":         <u64?>,
///   // optional wire-byte mutation applied before column_page_payload:
///   "truncate_wire_bytes":          <usize?>
/// }
/// ```
///
/// `round_trip`     — encode payload, parse the entry, decode wire bytes,
///                    assert decoded == payload.
/// `parse_reject`   — apply overrides, expect `ColumnPageIndexEntryV1::parse`
///                    to reject (Spec §27.2 invariants + §66 codec rules).
/// `decode_reject`  — entry parses cleanly but `column_page_payload` rejects
///                    the wire bytes (Spec §66 robustness against truncation
///                    or length mismatch).
pub(super) fn validate_page_codec_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CoveError::BadSection(format!("invalid page_codec fixture json: {err}")))?;
    let codec = match value.get("codec").and_then(Value::as_str) {
        Some("none") => CompressionCodec::None,
        Some("lz4") => CompressionCodec::Lz4,
        Some("zstd") => CompressionCodec::Zstd,
        other => {
            return Err(CoveError::BadSection(format!(
                "page_codec fixture has unknown codec {other:?}"
            )));
        }
    };
    let payload = value
        .get("payload")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("page_codec fixture missing payload".into()))?
        .as_bytes()
        .to_vec();
    let expect = value
        .get("expect")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("page_codec fixture missing expect".into()))?;

    let wire = encode_page_payload(&payload, codec)?;
    let page_length = value
        .get("page_length_override")
        .and_then(Value::as_u64)
        .unwrap_or(wire.len() as u64);
    let uncompressed_length = value
        .get("uncompressed_length_override")
        .and_then(Value::as_u64)
        .unwrap_or(payload.len() as u64);
    let flags = value
        .get("flags_override")
        .and_then(Value::as_u64)
        .map(|raw| raw as u32)
        .unwrap_or(codec as u32);
    let row_count = value
        .get("row_count_override")
        .and_then(Value::as_u64)
        .unwrap_or(1) as u32;
    let non_null_count = value
        .get("non_null_count_override")
        .and_then(Value::as_u64)
        .unwrap_or(row_count as u64) as u32;
    let null_count = value
        .get("null_count_override")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let encoding_root = value
        .get("encoding_root_override")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let page_offset = value
        .get("page_offset_override")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let entry = ColumnPageIndexEntryV1 {
        column_id: 1,
        morsel_id: 0,
        row_count,
        non_null_count,
        null_count,
        encoding_root,
        page_offset,
        page_length,
        uncompressed_length,
        stats_ref: 0,
        flags,
        checksum: checksum::crc32c(&wire),
    };
    let serialized = entry.serialize();
    let parsed = ColumnPageIndexEntryV1::parse(&serialized);

    match expect {
        "parse_reject" => {
            if parsed.is_ok() {
                return Err(CoveError::BadSection(
                    "page_codec parse_reject fixture parsed successfully".into(),
                ));
            }
            Ok(())
        }
        "round_trip" => {
            let parsed = parsed?;
            let decoded = column_page_payload(&wire, &parsed)?;
            if &*decoded != payload.as_slice() {
                return Err(CoveError::BadSection(
                    "page_codec round_trip decoded payload mismatch".into(),
                ));
            }
            Ok(())
        }
        "decode_reject" => {
            let parsed = parsed?;
            let mut wire = wire.clone();
            if let Some(truncate_to) = value.get("truncate_wire_bytes").and_then(Value::as_u64) {
                wire.truncate(truncate_to as usize);
            }
            // Re-stamp page_length to match the (possibly truncated) wire so
            // that the §66 codec dispatch is what surfaces the rejection,
            // not the surface-length check.
            let mut entry = parsed;
            entry.page_length = wire.len() as u64;
            if column_page_payload(&wire, &entry).is_ok() {
                return Err(CoveError::BadSection(
                    "page_codec decode_reject fixture decoded successfully".into(),
                ));
            }
            Ok(())
        }
        other => Err(CoveError::BadSection(format!(
            "page_codec fixture unknown expect kind {other:?}"
        ))),
    }
}

pub(super) fn validate_pruning_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CoveError::BadSection(format!("invalid pruning fixture json: {err}")))?;
    let columns = parse_pruning_columns(value.get("columns"))?;
    let predicate = value
        .get("predicate")
        .ok_or_else(|| CoveError::BadSection("pruning fixture missing predicate".into()))?;
    let explanation = evaluate_pruning_predicate(predicate, &columns)?;

    let expected_outcome =
        parse_expected_outcome(value.get("expect_outcome").ok_or_else(|| {
            CoveError::BadSection("pruning fixture missing expect_outcome".into())
        })?)?;
    if explanation.final_outcome != expected_outcome {
        return Err(CoveError::BadSection(format!(
            "pruning outcome mismatch: expected {:?}, got {:?}",
            expected_outcome, explanation.final_outcome
        )));
    }

    if let Some(expected) = value.get("expect_evidence") {
        let expected = expected
            .as_array()
            .ok_or_else(|| {
                CoveError::BadSection("expect_evidence must be an array of strings".into())
            })?
            .iter()
            .map(|item| {
                item.as_str().ok_or_else(|| {
                    CoveError::BadSection("expect_evidence entries must be strings".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let actual = explanation
            .steps
            .iter()
            .map(|step| pruning_evidence_name(step.evidence))
            .collect::<Vec<_>>();
        if actual != expected {
            return Err(CoveError::BadSection(format!(
                "pruning evidence mismatch: expected {:?}, got {:?}",
                expected, actual
            )));
        }
    }

    Ok(())
}

fn parse_pruning_columns(value: Option<&Value>) -> Result<Vec<PruningColumnFixture>, CoveError> {
    let Some(columns) = value else {
        return Ok(Vec::new());
    };
    let columns = columns
        .as_array()
        .ok_or_else(|| CoveError::BadSection("pruning fixture columns must be an array".into()))?;
    columns
        .iter()
        .map(parse_pruning_column)
        .collect::<Result<Vec<_>, _>>()
}

fn parse_pruning_column(value: &Value) -> Result<PruningColumnFixture, CoveError> {
    let column_id = value
        .get("column_id")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| CoveError::BadSection("pruning column missing column_id".into()))?;

    Ok(PruningColumnFixture {
        column_id,
        zone_stats: value
            .get("zone_stats")
            .map(|zone_stats| parse_pruning_zone_stats(zone_stats, column_id))
            .transpose()?,
        domain: value
            .get("column_domain")
            .map(|domain| parse_pruning_domain(domain, column_id))
            .transpose()?,
        exact_set: value
            .get("exact_set")
            .map(|exact_set| parse_pruning_exact_set(exact_set, column_id))
            .transpose()?,
        bloom: value
            .get("bloom")
            .map(|bloom| parse_pruning_bloom(bloom, column_id))
            .transpose()?,
        bloom_fail_open: value
            .get("bloom")
            .and_then(|bloom| bloom.get("fail_open"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        inverted: value
            .get("inverted")
            .map(|inverted| parse_pruning_inverted(inverted, column_id))
            .transpose()?,
        inverted_fail_open: value
            .get("inverted")
            .and_then(|inverted| inverted.get("fail_open"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        lookup: value
            .get("lookup")
            .map(|lookup| parse_pruning_lookup(lookup, column_id))
            .transpose()?,
        lookup_fail_open: value
            .get("lookup")
            .and_then(|lookup| lookup.get("fail_open"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        composite: value
            .get("composite")
            .map(|_| composite_index_stub(column_id)),
        composite_fail_open: value
            .get("composite")
            .and_then(|composite| composite.get("fail_open"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        composite_matches_bindings: value
            .get("composite")
            .and_then(|composite| composite.get("matches_bindings"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        aggregate: value.get("aggregate").map(|_| AggregateSynopsis::default()),
        aggregate_fail_open: value
            .get("aggregate")
            .and_then(|aggregate| aggregate.get("fail_open"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        aggregate_proves_no_match: value
            .get("aggregate")
            .and_then(|aggregate| aggregate.get("proves_no_match"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_pruning_zone_stats(value: &Value, column_id: u32) -> Result<ZoneStatsEntry, CoveError> {
    let row_count = value
        .get("row_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| CoveError::BadSection("zone_stats missing row_count".into()))?;
    let null_count = value
        .get("null_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| CoveError::BadSection("zone_stats missing null_count".into()))?;
    let min_domain_rank = value
        .get("min_domain_rank")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let max_domain_rank = value
        .get("max_domain_rank")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let flags = parse_zone_stat_flags(value.get("flags"))?;
    let mut min = value
        .get("min")
        .map(|scalar| parse_pruning_stat_scalar(scalar, "zone_stats min"))
        .transpose()?;
    let mut max = value
        .get("max")
        .map(|scalar| parse_pruning_stat_scalar(scalar, "zone_stats max"))
        .transpose()?;
    if flags.contains(ZoneStatFlags::MINMAX_TRUNCATED) {
        if let Some(min) = min.as_mut() {
            min.truncated = true;
        }
        if let Some(max) = max.as_mut() {
            max.truncated = true;
        }
    }

    let entry = ZoneStatsEntry {
        table_id: 1,
        segment_id: 0,
        morsel_id: u32::MAX,
        column_id,
        non_null_count: u32::try_from(row_count.checked_sub(null_count).ok_or_else(|| {
            CoveError::BadSection("zone_stats null_count exceeds row_count".into())
        })?)
        .map_err(|_| CoveError::BadSection("zone_stats non_null_count overflows u32".into()))?,
        distinct_count: 0,
        run_count: 0,
        stats: ZoneStats {
            scope: ZoneScope::Segment,
            row_count,
            null_count,
            min,
            max,
            flags,
        },
        min_domain_rank,
        max_domain_rank,
        exact_set_ref: 0,
        bloom_ref: 0,
    };
    entry.validate()?;
    Ok(entry)
}

fn parse_zone_stat_flags(value: Option<&Value>) -> Result<ZoneStatFlags, CoveError> {
    let mut flags = ZoneStatFlags::empty();
    let Some(value) = value else {
        return Ok(flags);
    };
    let items = value.as_array().ok_or_else(|| {
        CoveError::BadSection("zone_stats flags must be an array of strings".into())
    })?;
    for item in items {
        match item.as_str().ok_or_else(|| {
            CoveError::BadSection("zone_stats flags entries must be strings".into())
        })? {
            "has_min_max" => flags = flags | ZoneStatFlags::HAS_MIN_MAX,
            "has_domain_range" => flags = flags | ZoneStatFlags::HAS_DOMAIN_RANGE,
            "constant" => flags = flags | ZoneStatFlags::CONSTANT,
            "has_nan" => flags = flags | ZoneStatFlags::HAS_NAN,
            "minmax_truncated" => flags = flags | ZoneStatFlags::MINMAX_TRUNCATED,
            other => {
                return Err(CoveError::BadSection(format!(
                    "unsupported pruning zone_stats flag {other}"
                )));
            }
        }
    }
    Ok(flags)
}

fn parse_pruning_domain(value: &Value, column_id: u32) -> Result<ColumnDomain, CoveError> {
    let sorted_file_codes = value
        .get("sorted_file_codes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("column_domain missing sorted_file_codes array".into())
        })?
        .iter()
        .map(|item| {
            item.as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection(
                        "column_domain sorted_file_codes entries must be u32 values".into(),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let dictionary_entry_count = value
        .get("dictionary_entry_count")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_else(|| {
            sorted_file_codes
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .saturating_add(1)
        });
    let safe = value.get("safe").and_then(Value::as_bool).unwrap_or(true);

    let mut domain = ColumnDomain::from_sorted_present_codes(
        &sorted_file_codes,
        dictionary_entry_count,
        1,
        column_id,
        0,
        0,
        0,
    )?;
    if !safe && !domain.sorted_file_codes.is_empty() {
        let first_code = domain.sorted_file_codes[0] as usize;
        let replacement = domain.sorted_file_codes.len() as u32 - 1;
        domain.file_code_to_rank[first_code] = replacement;
    }
    Ok(domain)
}

fn parse_pruning_exact_set(value: &Value, column_id: u32) -> Result<ExactSetIndex, CoveError> {
    let keys = value
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("exact_set missing keys array".into()))?
        .iter()
        .map(|item| {
            item.as_u64().ok_or_else(|| {
                CoveError::BadSection("exact_set keys entries must be u64 values".into())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExactSetIndex {
        header: ExactSetIndexHeaderV1 {
            table_id: 1,
            column_id,
            granularity: ExactSetGranularity::Segment,
            key_kind: ExactSetKeyKind::FileCode,
            representation: ExactSetRepresentation::SortedList,
            flags: 0,
            entry_count: keys.len() as u32,
            data_offset: 0,
            data_length: 0,
            checksum: 0,
        },
        keys,
        data: Vec::new(),
    })
}

fn parse_pruning_bloom(value: &Value, column_id: u32) -> Result<BloomFilterIndex, CoveError> {
    use cove_core::index::bloom::{
        BloomAlgorithm, BloomGranularity, BloomHashDomain, BloomIndexHeaderV1,
        BLOOM_INDEX_HEADER_LEN,
    };
    let bit_count = value
        .get("bit_count")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(64);
    let mut bloom = BloomFilterIndex {
        header: BloomIndexHeaderV1 {
            table_id: 1,
            column_id,
            granularity: BloomGranularity::Segment,
            hash_domain: BloomHashDomain::CanonicalValueHash,
            algorithm: BloomAlgorithm::SplitBlock,
            flags: 0,
            target_fpr_ppm: 10_000,
            filter_count: 1,
            data_offset: BLOOM_INDEX_HEADER_LEN as u64,
            data_length: bit_count as u64,
            checksum: 0,
        },
        hash_count: 4,
        bits: vec![0u8; bit_count],
    };
    if let Some(values) = value.get("values").and_then(Value::as_array) {
        for entry in values {
            let bytes = parse_pruning_byte_string(entry, "bloom values entry")?;
            bloom.insert(&bytes);
        }
    }
    Ok(bloom)
}

fn parse_pruning_inverted(value: &Value, column_id: u32) -> Result<InvertedMorselIndex, CoveError> {
    use cove_core::index::inverted::{
        InvertedEntry, InvertedKeyKind, InvertedMorselIndexHeaderV1,
        INVERTED_MORSEL_INDEX_HEADER_LEN,
    };
    let mut keys: Vec<u64> = value
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("inverted missing keys array".into()))?
        .iter()
        .map(|item| {
            item.as_u64().ok_or_else(|| {
                CoveError::BadSection("inverted keys entries must be u64 values".into())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    keys.sort_unstable();
    keys.dedup();
    Ok(InvertedMorselIndex {
        header: InvertedMorselIndexHeaderV1 {
            table_id: 1,
            column_id,
            key_kind: InvertedKeyKind::FileCode,
            flags: 0,
            representation: 0,
            reserved: 0,
            entry_count: keys.len() as u32,
            entries_offset: INVERTED_MORSEL_INDEX_HEADER_LEN as u64,
            bitmap_data_offset: INVERTED_MORSEL_INDEX_HEADER_LEN as u64,
            checksum: 0,
        },
        entries: keys
            .into_iter()
            .map(|key| InvertedEntry {
                key,
                morsel_bitmap_offset: 0,
                morsel_bitmap_length: 0,
                row_bitmap_offset: 0,
                row_bitmap_length: 0,
            })
            .collect(),
        bitmap_data: Vec::new(),
    })
}

fn parse_pruning_lookup(value: &Value, column_id: u32) -> Result<LookupIndex, CoveError> {
    use cove_core::index::lookup::{
        LookupEntry, LookupIndexHeaderV1, LookupIndexKind, LookupKeyKind, LookupUniqueness,
    };
    let mut keys: Vec<u64> = value
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("lookup missing keys array".into()))?
        .iter()
        .map(|item| {
            item.as_u64().ok_or_else(|| {
                CoveError::BadSection("lookup keys entries must be u64 values".into())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    keys.sort_unstable();
    keys.dedup();
    Ok(LookupIndex {
        header: LookupIndexHeaderV1 {
            table_id: 1,
            column_id,
            key_kind: LookupKeyKind::FileCode,
            index_kind: LookupIndexKind::SparseSorted,
            uniqueness: LookupUniqueness::Unique,
            flags: 0,
            entry_count: keys.len() as u64,
            entries_offset: 0,
            entries_length: 0,
            rowref_offset: 0,
            rowref_length: 0,
            checksum: 0,
        },
        entries: keys
            .into_iter()
            .map(|key| LookupEntry {
                key,
                rows: vec![RowRef {
                    table_id: 1,
                    segment_id: 0,
                    morsel_id: 0,
                    row_in_morsel: 0,
                }],
            })
            .collect(),
    })
}

fn composite_index_stub(column_id: u32) -> CompositeIndex {
    use cove_core::index::composite::{
        CompositeTransformKind, CompositeZoneIndexHeaderV1, COMPOSITE_ZONE_INDEX_HEADER_LEN,
    };
    CompositeIndex {
        header: CompositeZoneIndexHeaderV1 {
            table_id: 1,
            key_column_count: 1,
            transform_kind: CompositeTransformKind::Tuple,
            flags: 0,
            zone_count: 1,
            key_columns_offset: COMPOSITE_ZONE_INDEX_HEADER_LEN as u64,
            entries_offset: (COMPOSITE_ZONE_INDEX_HEADER_LEN + 4) as u64,
            entries_length: 0,
            checksum: 0,
        },
        key_columns: vec![column_id],
        entries: Vec::new(),
    }
}

fn parse_pruning_byte_string(value: &Value, field: &str) -> Result<Vec<u8>, CoveError> {
    if let Some(text) = value.as_str() {
        return Ok(text.as_bytes().to_vec());
    }
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .map(|item| {
                item.as_u64()
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or_else(|| {
                        CoveError::BadSection(format!("{field} byte array entries must be u8"))
                    })
            })
            .collect();
    }
    Err(CoveError::BadSection(format!(
        "{field} must be a string or u8 array"
    )))
}

fn parse_pruning_stat_scalar(value: &Value, field: &str) -> Result<StatScalar, CoveError> {
    let kind_name = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection(format!("{field} missing kind")))?;
    let kind = parse_pruning_stat_kind(kind_name)?;
    let raw_value = value
        .get("value")
        .ok_or_else(|| CoveError::BadSection(format!("{field} missing value")))?;
    let bytes = match kind {
        StatKind::Int64 => parse_json_i64(raw_value, field)?.to_le_bytes().to_vec(),
        StatKind::UInt64 => parse_json_u64(raw_value, field)?.to_le_bytes().to_vec(),
        StatKind::Float64Bits => parse_json_f64(raw_value, field)?
            .to_bits()
            .to_le_bytes()
            .to_vec(),
        StatKind::Decimal128 => parse_json_i128(raw_value, field)?.to_le_bytes().to_vec(),
        StatKind::TimestampMicros => parse_json_i64(raw_value, field)?.to_le_bytes().to_vec(),
        StatKind::TimestampNanos => parse_json_i64(raw_value, field)?.to_le_bytes().to_vec(),
        StatKind::DateDays => parse_json_i32(raw_value, field)?.to_le_bytes().to_vec(),
        StatKind::None | StatKind::FixedBytes => {
            return Err(CoveError::BadSection(format!(
                "{field} uses unsupported pruning stat kind {kind_name}"
            )));
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "{field} uses future pruning stat kind {kind_name}"
            )));
        }
    };

    Ok(StatScalar {
        kind,
        bytes,
        truncated: value
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_pruning_numeric_bound(value: &Value, field: &str) -> Result<NumericStatValue, CoveError> {
    parse_pruning_stat_scalar(value, field)?
        .numeric_value()
        .ok_or_else(|| {
            CoveError::BadSection(format!("{field} must decode to a numeric stat value"))
        })
}

fn parse_pruning_stat_kind(kind: &str) -> Result<StatKind, CoveError> {
    match kind {
        "int64" => Ok(StatKind::Int64),
        "uint64" => Ok(StatKind::UInt64),
        "float64" | "float64_bits" => Ok(StatKind::Float64Bits),
        "decimal128" => Ok(StatKind::Decimal128),
        "timestamp_micros" => Ok(StatKind::TimestampMicros),
        "timestamp_nanos" => Ok(StatKind::TimestampNanos),
        "date_days" => Ok(StatKind::DateDays),
        other => Err(CoveError::BadSection(format!(
            "unsupported pruning stat kind {other}"
        ))),
    }
}

fn parse_json_i32(value: &Value, field: &str) -> Result<i32, CoveError> {
    let parsed = parse_json_i64(value, field)?;
    i32::try_from(parsed).map_err(|_| CoveError::BadSection(format!("{field} must fit in i32")))
}

fn parse_json_i64(value: &Value, field: &str) -> Result<i64, CoveError> {
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    if let Some(value) = value.as_u64() {
        return i64::try_from(value)
            .map_err(|_| CoveError::BadSection(format!("{field} must fit in i64")));
    }
    if let Some(value) = value.as_str() {
        return value.parse::<i64>().map_err(|_| {
            CoveError::BadSection(format!("{field} must be an i64-compatible value"))
        });
    }
    Err(CoveError::BadSection(format!(
        "{field} must be an integer value"
    )))
}

fn parse_json_u64(value: &Value, field: &str) -> Result<u64, CoveError> {
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }
    if let Some(value) = value.as_str() {
        return value
            .parse::<u64>()
            .map_err(|_| CoveError::BadSection(format!("{field} must be a u64-compatible value")));
    }
    Err(CoveError::BadSection(format!(
        "{field} must be an unsigned integer value"
    )))
}

fn parse_json_i128(value: &Value, field: &str) -> Result<i128, CoveError> {
    if let Some(value) = value.as_i64() {
        return Ok(value as i128);
    }
    if let Some(value) = value.as_u64() {
        return Ok(value as i128);
    }
    if let Some(value) = value.as_str() {
        return value.parse::<i128>().map_err(|_| {
            CoveError::BadSection(format!("{field} must be an i128-compatible value"))
        });
    }
    Err(CoveError::BadSection(format!(
        "{field} must be an integer value"
    )))
}

fn parse_json_f64(value: &Value, field: &str) -> Result<f64, CoveError> {
    if let Some(value) = value.as_f64() {
        return Ok(value);
    }
    if let Some(value) = value.as_str() {
        return value.parse::<f64>().map_err(|_| {
            CoveError::BadSection(format!("{field} must be an f64-compatible value"))
        });
    }
    Err(CoveError::BadSection(format!(
        "{field} must be a numeric value"
    )))
}

fn evaluate_pruning_predicate(
    predicate: &Value,
    columns: &[PruningColumnFixture],
) -> Result<PruningExplanation, CoveError> {
    let op = predicate
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("pruning predicate missing op".into()))?;
    match op {
        "is_null" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            Ok(explain_is_null(
                column.and_then(|column| column.zone_stats.as_ref()),
            ))
        }
        "is_not_null" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            Ok(explain_is_not_null(
                column.and_then(|column| column.zone_stats.as_ref()),
            ))
        }
        "file_code_eq" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            let file_code = predicate
                .get("file_code")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| CoveError::BadSection("file_code_eq missing file_code".into()))?;
            Ok(explain_file_code_equality(
                file_code,
                column.and_then(|column| column.zone_stats.as_ref()),
                column.and_then(|column| column.domain.as_ref()),
                column.and_then(|column| column.exact_set.as_ref()),
            ))
        }
        "domain_rank_range" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            let min_rank = predicate
                .get("min_rank")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection("domain_rank_range missing min_rank".into())
                })?;
            let max_rank = predicate
                .get("max_rank")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection("domain_rank_range missing max_rank".into())
                })?;
            Ok(explain_resolved_domain_rank_range(
                min_rank,
                max_rank,
                column.and_then(|column| column.zone_stats.as_ref()),
                column.and_then(|column| column.domain.as_ref()),
            ))
        }
        "numcode_range" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            let lower_bound = predicate
                .get("lower")
                .map(|value| parse_pruning_numeric_bound(value, "numcode_range lower"))
                .transpose()?;
            let upper_bound = predicate
                .get("upper")
                .map(|value| parse_pruning_numeric_bound(value, "numcode_range upper"))
                .transpose()?;
            if lower_bound.is_none() && upper_bound.is_none() {
                return Err(CoveError::BadSection(
                    "numcode_range must declare at least one bound".into(),
                ));
            }
            Ok(explain_numcode_range(
                lower_bound,
                predicate
                    .get("lower_inclusive")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                upper_bound,
                predicate
                    .get("upper_inclusive")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                column.and_then(|column| column.zone_stats.as_ref()),
            ))
        }
        "and" => fold_pruning_operands(predicate, columns, |left, right| left.and(right)),
        "or" => fold_pruning_operands(predicate, columns, |left, right| left.or(right)),
        "not" => {
            let operand = predicate
                .get("operand")
                .ok_or_else(|| CoveError::BadSection("not predicate missing operand".into()))?;
            Ok(!evaluate_pruning_predicate(operand, columns)?)
        }
        "bloom_membership" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            let value = predicate
                .get("value")
                .ok_or_else(|| CoveError::BadSection("bloom_membership missing value".into()))?;
            let bytes = parse_pruning_byte_string(value, "bloom_membership value")?;
            Ok(explain_bloom_membership(
                &bytes,
                column.and_then(|column| column.bloom.as_ref()),
                column.map(|column| column.bloom_fail_open).unwrap_or(false),
            ))
        }
        "inverted_lookup" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            let key = predicate
                .get("key")
                .and_then(Value::as_u64)
                .ok_or_else(|| CoveError::BadSection("inverted_lookup missing key".into()))?;
            Ok(explain_inverted_morsel_lookup(
                key,
                column.and_then(|column| column.inverted.as_ref()),
                column
                    .map(|column| column.inverted_fail_open)
                    .unwrap_or(false),
            ))
        }
        "lookup_point" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            let key = predicate
                .get("key")
                .and_then(Value::as_u64)
                .ok_or_else(|| CoveError::BadSection("lookup_point missing key".into()))?;
            Ok(explain_lookup_index_point(
                key,
                column.and_then(|column| column.lookup.as_ref()),
                column
                    .map(|column| column.lookup_fail_open)
                    .unwrap_or(false),
            ))
        }
        "composite_zone" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            Ok(explain_composite_zone(
                column.and_then(|column| column.composite.as_ref()),
                column
                    .map(|column| column.composite_fail_open)
                    .unwrap_or(false),
                column
                    .map(|column| column.composite_matches_bindings)
                    .unwrap_or(false),
            ))
        }
        "aggregate_synopsis" => {
            let column = pruning_column(columns, predicate_column_id(predicate)?);
            Ok(explain_aggregate_synopsis(
                column.and_then(|column| column.aggregate.as_ref()),
                column
                    .map(|column| column.aggregate_fail_open)
                    .unwrap_or(false),
                column
                    .map(|column| column.aggregate_proves_no_match)
                    .unwrap_or(false),
            ))
        }
        "reorder_invariant_and" => evaluate_reorder_invariant(predicate, columns, |a, b| a.and(b)),
        "reorder_invariant_or" => evaluate_reorder_invariant(predicate, columns, |a, b| a.or(b)),
        other => Err(CoveError::BadSection(format!(
            "unsupported pruning predicate op {other}"
        ))),
    }
}

fn fold_pruning_operands<F>(
    predicate: &Value,
    columns: &[PruningColumnFixture],
    combine: F,
) -> Result<PruningExplanation, CoveError>
where
    F: Fn(PruningExplanation, PruningExplanation) -> PruningExplanation,
{
    let operands = predicate
        .get("operands")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("compound pruning predicate missing operands".into())
        })?;
    let mut operands = operands.iter();
    let first = operands.next().ok_or_else(|| {
        CoveError::BadSection("compound pruning predicate must have at least one operand".into())
    })?;
    let mut explanation = evaluate_pruning_predicate(first, columns)?;
    for operand in operands {
        explanation = combine(explanation, evaluate_pruning_predicate(operand, columns)?);
    }
    Ok(explanation)
}

/// Spec §37.5: prove that AND/OR predicates are commutative under reordering.
///
/// Evaluate the operand list in the declared order to produce the canonical
/// explanation, then re-evaluate every other permutation and assert each
/// yields the same `final_outcome`. The runner returns the canonical
/// explanation so the caller can still assert outcome and evidence trace.
fn evaluate_reorder_invariant<F>(
    predicate: &Value,
    columns: &[PruningColumnFixture],
    combine: F,
) -> Result<PruningExplanation, CoveError>
where
    F: Fn(PruningExplanation, PruningExplanation) -> PruningExplanation,
{
    let operand_values: Vec<&Value> = predicate
        .get("operands")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("reorder_invariant predicate missing operands".into())
        })?
        .iter()
        .collect();
    if operand_values.is_empty() {
        return Err(CoveError::BadSection(
            "reorder_invariant predicate must have at least one operand".into(),
        ));
    }
    let canonical = fold_in_order(&operand_values, columns, &combine)?;
    let mut indices: Vec<usize> = (0..operand_values.len()).collect();
    let mut permutation = indices.clone();
    while next_permutation(&mut permutation) {
        let permuted: Vec<&Value> = permutation.iter().map(|i| operand_values[*i]).collect();
        let alternative = fold_in_order(&permuted, columns, &combine)?;
        if alternative.final_outcome != canonical.final_outcome {
            return Err(CoveError::BadSection(format!(
                "reorder_invariant outcome diverged under permutation {:?}: expected {:?}, got {:?}",
                permutation, canonical.final_outcome, alternative.final_outcome
            )));
        }
        indices.clone_from(&permutation);
    }
    let _ = indices;
    Ok(canonical)
}

fn fold_in_order<F>(
    operands: &[&Value],
    columns: &[PruningColumnFixture],
    combine: &F,
) -> Result<PruningExplanation, CoveError>
where
    F: Fn(PruningExplanation, PruningExplanation) -> PruningExplanation,
{
    let mut iter = operands.iter();
    let first = iter.next().ok_or_else(|| {
        CoveError::BadSection("fold_in_order requires at least one operand".into())
    })?;
    let mut explanation = evaluate_pruning_predicate(first, columns)?;
    for operand in iter {
        explanation = combine(explanation, evaluate_pruning_predicate(operand, columns)?);
    }
    Ok(explanation)
}

/// Lexicographic next-permutation; returns false when no further permutation
/// exists (the slice has been left in the smallest order).
fn next_permutation(slice: &mut [usize]) -> bool {
    if slice.len() < 2 {
        return false;
    }
    let mut i = slice.len() - 1;
    while i > 0 && slice[i - 1] >= slice[i] {
        i -= 1;
    }
    if i == 0 {
        slice.reverse();
        return false;
    }
    let pivot = i - 1;
    let mut j = slice.len() - 1;
    while slice[j] <= slice[pivot] {
        j -= 1;
    }
    slice.swap(pivot, j);
    slice[i..].reverse();
    true
}

fn predicate_column_id(predicate: &Value) -> Result<u32, CoveError> {
    predicate
        .get("column_id")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| CoveError::BadSection("pruning predicate missing column_id".into()))
}

fn pruning_column(
    columns: &[PruningColumnFixture],
    column_id: u32,
) -> Option<&PruningColumnFixture> {
    columns.iter().find(|column| column.column_id == column_id)
}

fn parse_expected_outcome(
    value: &Value,
) -> Result<cove_core::predicate::PredicateZoneOutcome, CoveError> {
    match value
        .as_str()
        .ok_or_else(|| CoveError::BadSection("expect_outcome must be a string".into()))?
    {
        "all_match" => Ok(cove_core::predicate::PredicateZoneOutcome::AllMatch),
        "no_match" => Ok(cove_core::predicate::PredicateZoneOutcome::NoMatch),
        "some_match" => Ok(cove_core::predicate::PredicateZoneOutcome::SomeMatch),
        "unknown" => Ok(cove_core::predicate::PredicateZoneOutcome::Unknown),
        other => Err(CoveError::BadSection(format!(
            "unsupported pruning expect_outcome {other}"
        ))),
    }
}

fn pruning_evidence_name(evidence: PruningEvidence) -> &'static str {
    match evidence {
        PruningEvidence::NoMetadata => "NoMetadata",
        PruningEvidence::ZoneStats => "ZoneStats",
        PruningEvidence::ColumnDomain => "ColumnDomain",
        PruningEvidence::ExactSet => "ExactSet",
        PruningEvidence::BloomFilter => "BloomFilter",
        PruningEvidence::InvertedIndex => "InvertedIndex",
        PruningEvidence::CompositeIndex => "CompositeIndex",
        PruningEvidence::AggregateSynopsis => "AggregateSynopsis",
        PruningEvidence::TopNSummary => "TopNSummary",
        PruningEvidence::FallbackToScan => "FallbackToScan",
        _ => "Future",
    }
}
