use super::*;

type FileCodeDomainStats = (u32, u32, u32, u32, bool);
type ScalarZoneStats = (StatKind, Vec<u8>, Vec<u8>, u32, u32, ZoneStatFlags);

pub(super) fn build_column_domains(
    columns: &[ConvertedColumn],
    dictionary: Option<&FileDictionary>,
) -> Result<Vec<ColumnDomain>, CoveError> {
    let Some(dictionary) = dictionary else {
        return Ok(Vec::new());
    };
    let mut domains = Vec::new();
    for column in columns {
        let MaterializedValues::FileCode(codes) = &column.values else {
            continue;
        };
        let mut sorted_codes = codes
            .iter()
            .enumerate()
            .filter_map(|(row, code)| (!column.is_null(row)).then_some(*code))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if sorted_codes.is_empty() {
            continue;
        }
        let mut sort_keys = BTreeMap::new();
        for code in &sorted_codes {
            sort_keys.insert(
                *code,
                dictionary_domain_sort_key(column.entry.logical, dictionary, *code)?,
            );
        }
        sorted_codes.sort_by(|left, right| sort_keys[left].cmp(&sort_keys[right]));
        let domain = ColumnDomain::from_sorted_present_codes(
            &sorted_codes,
            dictionary.len(),
            1,
            column.entry.column_id,
            column.entry.logical as u16,
            column.entry.collation_id,
            0,
        )?;
        domains.push(domain);
    }
    Ok(domains)
}

fn dictionary_domain_sort_key(
    logical: CoveLogicalType,
    dictionary: &FileDictionary,
    file_code: u32,
) -> Result<Vec<u8>, CoveError> {
    let entry = dictionary.get_entry(file_code)?;
    let bytes = match dictionary.decode_value(file_code)? {
        DictionaryValue::RawBytes(bytes) => bytes,
        DictionaryValue::RedactedPresent => return Err(CoveError::BadDomain),
        _ => return Err(CoveError::BadDomain),
    };
    match logical {
        CoveLogicalType::Utf8 | CoveLogicalType::Binary => {
            let (len, prefix_len) =
                wire::decode_u64_leb128(&bytes).map_err(|_| CoveError::BadDomain)?;
            let len = usize::try_from(len).map_err(|_| CoveError::BadDomain)?;
            let end = prefix_len
                .checked_add(len)
                .ok_or(CoveError::ArithOverflow)?;
            if end != bytes.len() {
                return Err(CoveError::BadDomain);
            }
            Ok(bytes[prefix_len..end].to_vec())
        }
        _ => Err(CoveError::BadDomain),
    }
    .and_then(|key| {
        let tag = ValueTag::from_u16(entry.value_tag).ok_or(CoveError::BadDomain)?;
        match (logical, tag) {
            (CoveLogicalType::Utf8, ValueTag::Utf8)
            | (CoveLogicalType::Binary, ValueTag::Binary) => Ok(key),
            _ => Err(CoveError::BadDomain),
        }
    })
}

pub(super) fn build_zone_stats(
    columns: &[ConvertedColumn],
    segments: &[SegmentLayout],
    morsel_row_count: u32,
) -> Result<Option<ZoneStatsSection>, CoveError> {
    let mut entries = Vec::new();
    for column in columns {
        for segment in segments {
            let row_end = segment
                .row_start
                .checked_add(segment.row_count)
                .ok_or(CoveError::ArithOverflow)?;
            let mut start = segment.row_start;
            let mut morsel_id = 0u32;
            while start < row_end {
                let len = (row_end - start).min(morsel_row_count as usize);
                if let Some(entry) =
                    build_zone_stats_entry(column, start, len, segment.segment_id, morsel_id)?
                {
                    entries.push(entry);
                }
                start = start.checked_add(len).ok_or(CoveError::ArithOverflow)?;
                morsel_id = morsel_id.checked_add(1).ok_or(CoveError::ArithOverflow)?;
            }
        }
    }
    if entries.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ZoneStatsSection { entries }))
    }
}

fn build_zone_stats_entry(
    column: &ConvertedColumn,
    start: usize,
    len: usize,
    segment_id: u32,
    morsel_id: u32,
) -> Result<Option<ZoneStatsEntry>, CoveError> {
    if len == 0 {
        return Ok(None);
    }
    let null_count = column.null_count_range(start, len)?;
    if null_count == len {
        return Ok(Some(zone_entry(
            column,
            len,
            null_count,
            segment_id,
            morsel_id,
            0,
            0,
            ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: len as u64,
                null_count: null_count as u64,
                min: None,
                max: None,
                flags: ZoneStatFlags::empty(),
            },
            u32::MAX,
            u32::MAX,
        )));
    }
    if let Some((min_rank, max_rank, distinct_count, run_count, constant)) =
        filecode_domain_stats(column, start, len)?
    {
        let flags = ZoneStatFlags::HAS_DOMAIN_RANGE
            | ZoneStatFlags::DISTINCT_EXACT
            | if constant {
                ZoneStatFlags::CONSTANT
            } else {
                ZoneStatFlags::empty()
            };
        return Ok(Some(zone_entry(
            column,
            len,
            null_count,
            segment_id,
            morsel_id,
            distinct_count,
            run_count,
            ZoneStats {
                scope: ZoneScope::Morsel,
                row_count: len as u64,
                null_count: null_count as u64,
                min: None,
                max: None,
                flags,
            },
            min_rank,
            max_rank,
        )));
    }

    let Some((kind, min, max, distinct_count, run_count, mut flags)) =
        scalar_min_max_stats(column, start, len)?
    else {
        return Ok(None);
    };
    flags = flags | ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::DISTINCT_EXACT;
    if distinct_count == 1 {
        flags = flags | ZoneStatFlags::CONSTANT;
    }
    Ok(Some(zone_entry(
        column,
        len,
        null_count,
        segment_id,
        morsel_id,
        distinct_count,
        run_count,
        ZoneStats {
            scope: ZoneScope::Morsel,
            row_count: len as u64,
            null_count: null_count as u64,
            min: Some(StatScalar {
                kind,
                bytes: min,
                truncated: false,
            }),
            max: Some(StatScalar {
                kind,
                bytes: max,
                truncated: false,
            }),
            flags,
        },
        u32::MAX,
        u32::MAX,
    )))
}

#[allow(clippy::too_many_arguments)]
fn zone_entry(
    column: &ConvertedColumn,
    row_count: usize,
    null_count: usize,
    segment_id: u32,
    morsel_id: u32,
    distinct_count: u32,
    run_count: u32,
    stats: ZoneStats,
    min_domain_rank: u32,
    max_domain_rank: u32,
) -> ZoneStatsEntry {
    ZoneStatsEntry {
        table_id: 1,
        segment_id,
        morsel_id,
        column_id: column.entry.column_id,
        non_null_count: row_count.saturating_sub(null_count) as u32,
        distinct_count,
        run_count,
        stats,
        min_domain_rank,
        max_domain_rank,
        exact_set_ref: 0,
        bloom_ref: 0,
    }
}

fn filecode_domain_stats(
    column: &ConvertedColumn,
    start: usize,
    len: usize,
) -> Result<Option<FileCodeDomainStats>, CoveError> {
    let MaterializedValues::FileCode(values) = &column.values else {
        return Ok(None);
    };
    let all_codes = values
        .iter()
        .enumerate()
        .filter_map(|(row, code)| (!column.is_null(row)).then_some(*code))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let rows = column.non_null_indices(start, len)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let slice = rows.iter().map(|row| values[*row]).collect::<Vec<_>>();
    let min_code = *slice.iter().min().ok_or(CoveError::BadStats)?;
    let max_code = *slice.iter().max().ok_or(CoveError::BadStats)?;
    let min_rank = all_codes
        .binary_search(&min_code)
        .map_err(|_| CoveError::BadDomain)? as u32;
    let max_rank = all_codes
        .binary_search(&max_code)
        .map_err(|_| CoveError::BadDomain)? as u32;
    let distinct_count = u32::try_from(slice.iter().copied().collect::<BTreeSet<_>>().len())
        .map_err(|_| CoveError::ArithOverflow)?;
    let run_count = run_count_u32(slice.iter().copied())?;
    Ok(Some((
        min_rank,
        max_rank,
        distinct_count,
        run_count,
        distinct_count == 1,
    )))
}

fn scalar_min_max_stats(
    column: &ConvertedColumn,
    start: usize,
    len: usize,
) -> Result<Option<ScalarZoneStats>, CoveError> {
    let rows = column.non_null_indices(start, len)?;
    if rows.is_empty() {
        return Ok(None);
    }
    match (&column.values, column.entry.logical) {
        (MaterializedValues::Boolean(values), CoveLogicalType::Bool) => {
            let slice = rows.iter().map(|row| values[*row]).collect::<Vec<_>>();
            let min = u64::from(*slice.iter().min().ok_or(CoveError::BadStats)? != 0);
            let max = u64::from(*slice.iter().max().ok_or(CoveError::BadStats)? != 0);
            Ok(Some((
                StatKind::UInt64,
                min.to_le_bytes().to_vec(),
                max.to_le_bytes().to_vec(),
                u32::try_from(slice.iter().copied().collect::<BTreeSet<_>>().len())
                    .map_err(|_| CoveError::ArithOverflow)?,
                run_count_u32(slice.iter().copied())?,
                ZoneStatFlags::empty(),
            )))
        }
        (MaterializedValues::NumCode(values), logical) => {
            numcode_min_max_stats(values, logical, column.source_kind, &rows)
        }
        (MaterializedValues::FixedBytes { values, width: 16 }, CoveLogicalType::Decimal128) => {
            let mut decoded = Vec::with_capacity(rows.len());
            for row in &rows {
                let value = &values[*row];
                let raw: [u8; 16] = value.as_slice().try_into().map_err(|_| {
                    CoveError::BadSchema("decimal128 fixed value must be 16 bytes".into())
                })?;
                decoded.push(i128::from_le_bytes(raw));
            }
            let min = *decoded.iter().min().ok_or(CoveError::BadStats)?;
            let max = *decoded.iter().max().ok_or(CoveError::BadStats)?;
            Ok(Some((
                StatKind::Decimal128,
                min.to_le_bytes().to_vec(),
                max.to_le_bytes().to_vec(),
                u32::try_from(decoded.iter().copied().collect::<BTreeSet<_>>().len())
                    .map_err(|_| CoveError::ArithOverflow)?,
                run_count_u32(decoded.iter().copied())?,
                ZoneStatFlags::empty(),
            )))
        }
        _ => Ok(None),
    }
}

fn numcode_min_max_stats(
    values: &[u64],
    logical: CoveLogicalType,
    source_kind: SourceColumnKind,
    rows: &[usize],
) -> Result<Option<ScalarZoneStats>, CoveError> {
    let slice = rows.iter().map(|row| values[*row]).collect::<Vec<_>>();
    match logical {
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => {
            let decoded = slice
                .iter()
                .map(|value| signed_numcode(logical, *value))
                .collect::<Vec<_>>();
            let min = *decoded.iter().min().ok_or(CoveError::BadStats)?;
            let max = *decoded.iter().max().ok_or(CoveError::BadStats)?;
            Ok(Some((
                StatKind::Int64,
                min.to_le_bytes().to_vec(),
                max.to_le_bytes().to_vec(),
                distinct_len(&decoded)?,
                run_count_u32(decoded.iter().copied())?,
                ZoneStatFlags::empty(),
            )))
        }
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => {
            let min = *slice.iter().min().ok_or(CoveError::BadStats)?;
            let max = *slice.iter().max().ok_or(CoveError::BadStats)?;
            Ok(Some((
                StatKind::UInt64,
                min.to_le_bytes().to_vec(),
                max.to_le_bytes().to_vec(),
                distinct_len(&slice)?,
                run_count_u32(slice.iter().copied())?,
                ZoneStatFlags::empty(),
            )))
        }
        CoveLogicalType::Float32 | CoveLogicalType::Float64 => {
            let mut decoded = Vec::new();
            let mut has_nan = false;
            for value in &slice {
                let value = if source_kind == SourceColumnKind::Float32 {
                    f32::from_bits(*value as u32) as f64
                } else {
                    f64::from_bits(*value)
                };
                if value.is_nan() {
                    has_nan = true;
                } else {
                    decoded.push(value);
                }
            }
            if decoded.is_empty() {
                return Ok(None);
            }
            decoded.sort_by(f64::total_cmp);
            let min = decoded[0];
            let max = decoded[decoded.len() - 1];
            let flags = if has_nan {
                ZoneStatFlags::HAS_NAN
            } else {
                ZoneStatFlags::empty()
            };
            Ok(Some((
                StatKind::Float64Bits,
                min.to_bits().to_le_bytes().to_vec(),
                max.to_bits().to_le_bytes().to_vec(),
                u32::try_from(decoded.len()).map_err(|_| CoveError::ArithOverflow)?,
                u32::try_from(slice.len()).map_err(|_| CoveError::ArithOverflow)?,
                flags,
            )))
        }
        CoveLogicalType::DateDays => {
            let decoded = slice
                .iter()
                .map(|value| types::numcode_as_date_days(*value))
                .collect::<Vec<_>>();
            let min = *decoded.iter().min().ok_or(CoveError::BadStats)?;
            let max = *decoded.iter().max().ok_or(CoveError::BadStats)?;
            Ok(Some((
                StatKind::DateDays,
                min.to_le_bytes().to_vec(),
                max.to_le_bytes().to_vec(),
                distinct_len(&decoded)?,
                run_count_u32(decoded.iter().copied())?,
                ZoneStatFlags::empty(),
            )))
        }
        CoveLogicalType::TimestampMicros | CoveLogicalType::TimestampNanos => {
            let decoded = slice.iter().map(|value| *value as i64).collect::<Vec<_>>();
            let min = *decoded.iter().min().ok_or(CoveError::BadStats)?;
            let max = *decoded.iter().max().ok_or(CoveError::BadStats)?;
            Ok(Some((
                if logical == CoveLogicalType::TimestampMicros {
                    StatKind::TimestampMicros
                } else {
                    StatKind::TimestampNanos
                },
                min.to_le_bytes().to_vec(),
                max.to_le_bytes().to_vec(),
                distinct_len(&decoded)?,
                run_count_u32(decoded.iter().copied())?,
                ZoneStatFlags::empty(),
            )))
        }
        _ => Ok(None),
    }
}
