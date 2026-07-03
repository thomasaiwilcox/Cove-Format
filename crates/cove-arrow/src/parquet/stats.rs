use super::*;

type FileCodeDomainStats = (u32, u32, u32, u32, bool);

#[derive(Debug, Clone)]
struct FileCodeDomainArtifacts {
    sorted_codes: Vec<u32>,
    ranks: BTreeMap<u32, u32>,
}

#[derive(Debug, Clone)]
struct ScalarZoneStats {
    min: Option<StatScalar>,
    max: Option<StatScalar>,
    distinct_count: u32,
    run_count: u32,
    flags: ZoneStatFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FloatStatKey {
    Number(u64),
    Nan,
}

pub(super) fn build_column_domains(
    columns: &[ConvertedColumn],
    dictionary: Option<&FileDictionary>,
) -> Result<Vec<ColumnDomain>, CoveError> {
    let Some(dictionary) = dictionary else {
        return Ok(Vec::new());
    };
    let mut domains = Vec::new();
    for column in columns {
        if !matches!(column.values, MaterializedValues::FileCode(_)) {
            continue;
        }
        let artifacts = filecode_domain_artifacts(column, dictionary)?;
        if artifacts.sorted_codes.is_empty() {
            continue;
        }
        let domain = ColumnDomain::from_sorted_present_codes(
            &artifacts.sorted_codes,
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

fn filecode_domain_artifacts(
    column: &ConvertedColumn,
    dictionary: &FileDictionary,
) -> Result<FileCodeDomainArtifacts, CoveError> {
    let MaterializedValues::FileCode(codes) = &column.values else {
        return Ok(FileCodeDomainArtifacts {
            sorted_codes: Vec::new(),
            ranks: BTreeMap::new(),
        });
    };
    let mut sorted_codes = codes
        .iter()
        .enumerate()
        .filter_map(|(row, code)| (!column.is_null(row)).then_some(*code))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut sort_keys = BTreeMap::new();
    for code in &sorted_codes {
        sort_keys.insert(
            *code,
            dictionary_domain_sort_key(column.entry.logical, dictionary, *code)?,
        );
    }
    sorted_codes.sort_by(|left, right| sort_keys[left].cmp(&sort_keys[right]));
    let ranks = sorted_codes
        .iter()
        .enumerate()
        .map(|(rank, code)| {
            u32::try_from(rank)
                .map(|rank| (*code, rank))
                .map_err(|_| CoveError::ArithOverflow)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(FileCodeDomainArtifacts {
        sorted_codes,
        ranks,
    })
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
    dictionary: Option<&FileDictionary>,
    segments: &[SegmentLayout],
    morsel_row_count: u32,
) -> Result<Option<ZoneStatsSection>, CoveError> {
    let mut entries = Vec::new();
    for column in columns {
        let filecode_artifacts = if let (MaterializedValues::FileCode(_), Some(dictionary)) =
            (&column.values, dictionary)
        {
            Some(filecode_domain_artifacts(column, dictionary)?)
        } else {
            None
        };
        for segment in segments {
            let row_end = segment
                .row_start
                .checked_add(segment.row_count)
                .ok_or(CoveError::ArithOverflow)?;
            let mut start = segment.row_start;
            let mut morsel_id = 0u32;
            while start < row_end {
                let len = (row_end - start).min(morsel_row_count as usize);
                if let Some(entry) = build_zone_stats_entry(
                    column,
                    filecode_artifacts.as_ref(),
                    start,
                    len,
                    segment.segment_id,
                    morsel_id,
                )? {
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
    filecode_artifacts: Option<&FileCodeDomainArtifacts>,
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
        filecode_domain_stats(column, filecode_artifacts, start, len)?
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

    let Some(scalar_stats) = scalar_min_max_stats(column, start, len)? else {
        return Ok(None);
    };
    let mut flags = scalar_stats.flags | ZoneStatFlags::DISTINCT_EXACT;
    if scalar_stats.min.is_some() {
        flags = flags | ZoneStatFlags::HAS_MIN_MAX;
    }
    if scalar_stats.distinct_count == 1 && !flags.contains(ZoneStatFlags::HAS_NAN) {
        flags = flags | ZoneStatFlags::CONSTANT;
    }
    Ok(Some(zone_entry(
        column,
        len,
        null_count,
        segment_id,
        morsel_id,
        scalar_stats.distinct_count,
        scalar_stats.run_count,
        ZoneStats {
            scope: ZoneScope::Morsel,
            row_count: len as u64,
            null_count: null_count as u64,
            min: scalar_stats.min,
            max: scalar_stats.max,
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
    artifacts: Option<&FileCodeDomainArtifacts>,
    start: usize,
    len: usize,
) -> Result<Option<FileCodeDomainStats>, CoveError> {
    let MaterializedValues::FileCode(values) = &column.values else {
        return Ok(None);
    };
    let Some(artifacts) = artifacts else {
        return Ok(None);
    };
    let rows = column.non_null_indices(start, len)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let slice = rows.iter().map(|row| values[*row]).collect::<Vec<_>>();
    let mut min_rank = u32::MAX;
    let mut max_rank = 0u32;
    for code in &slice {
        let rank = *artifacts.ranks.get(code).ok_or(CoveError::BadDomain)?;
        min_rank = min_rank.min(rank);
        max_rank = max_rank.max(rank);
    }
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
            Ok(Some(scalar_stats_with_min_max(
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
            Ok(Some(scalar_stats_with_min_max(
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

fn scalar_stats_with_min_max(
    kind: StatKind,
    min: Vec<u8>,
    max: Vec<u8>,
    distinct_count: u32,
    run_count: u32,
    flags: ZoneStatFlags,
) -> ScalarZoneStats {
    ScalarZoneStats {
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
        distinct_count,
        run_count,
        flags,
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
            Ok(Some(scalar_stats_with_min_max(
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
            Ok(Some(scalar_stats_with_min_max(
                StatKind::UInt64,
                min.to_le_bytes().to_vec(),
                max.to_le_bytes().to_vec(),
                distinct_len(&slice)?,
                run_count_u32(slice.iter().copied())?,
                ZoneStatFlags::empty(),
            )))
        }
        CoveLogicalType::Float32 | CoveLogicalType::Float64 => {
            let mut ordered_values = Vec::new();
            let mut distinct_keys = BTreeSet::new();
            let mut run_keys = Vec::with_capacity(slice.len());
            let mut has_nan = false;
            for value in &slice {
                let decoded = if source_kind == SourceColumnKind::Float32 {
                    let bits = *value as u32;
                    f32::from_bits(bits) as f64
                } else {
                    let bits = *value;
                    f64::from_bits(bits)
                };
                let key = float_stat_key(decoded);
                distinct_keys.insert(key);
                run_keys.push(key);
                if decoded.is_nan() {
                    has_nan = true;
                } else {
                    ordered_values.push(normalize_float_zero(decoded));
                }
            }
            let flags = if has_nan {
                ZoneStatFlags::HAS_NAN
            } else {
                ZoneStatFlags::empty()
            };
            let distinct_count =
                u32::try_from(distinct_keys.len()).map_err(|_| CoveError::ArithOverflow)?;
            let run_count = run_count_u32(run_keys.into_iter())?;
            if ordered_values.is_empty() {
                return Ok(Some(ScalarZoneStats {
                    min: None,
                    max: None,
                    distinct_count,
                    run_count,
                    flags,
                }));
            }
            ordered_values.sort_by(f64::total_cmp);
            let min = ordered_values[0];
            let max = ordered_values[ordered_values.len() - 1];
            Ok(Some(scalar_stats_with_min_max(
                StatKind::Float64Bits,
                min.to_bits().to_le_bytes().to_vec(),
                max.to_bits().to_le_bytes().to_vec(),
                distinct_count,
                run_count,
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
            Ok(Some(scalar_stats_with_min_max(
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
            Ok(Some(scalar_stats_with_min_max(
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

fn float_stat_key(value: f64) -> FloatStatKey {
    if value.is_nan() {
        FloatStatKey::Nan
    } else {
        FloatStatKey::Number(normalize_float_zero(value).to_bits())
    }
}

fn normalize_float_zero(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::StorageClass,
        dictionary::{FileDictionaryHeaderV1, FileDictionaryIndexEntryV1},
    };

    #[test]
    fn filecode_domain_artifacts_rank_by_logical_order_not_filecode_order() {
        let dictionary = dictionary_with_utf8_entries_in_filecode_order(&["b", "aa"]);
        let column = filecode_column(vec![0, 1]);
        let artifacts = filecode_domain_artifacts(&column, &dictionary).unwrap();

        assert_eq!(artifacts.sorted_codes, vec![1, 0]);
        assert_eq!(artifacts.ranks.get(&0), Some(&1));
        assert_eq!(artifacts.ranks.get(&1), Some(&0));

        let first_morsel = filecode_domain_stats(&column, Some(&artifacts), 0, 1)
            .unwrap()
            .unwrap();
        let second_morsel = filecode_domain_stats(&column, Some(&artifacts), 1, 1)
            .unwrap()
            .unwrap();
        assert_eq!((first_morsel.0, first_morsel.1), (1, 1));
        assert_eq!((second_morsel.0, second_morsel.1), (0, 0));
    }

    fn dictionary_with_utf8_entries_in_filecode_order(values: &[&str]) -> FileDictionary {
        FileDictionary {
            header: FileDictionaryHeaderV1 {
                entry_count: values.len() as u32,
                flags: 0,
                index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
                value_hash_algorithm: 0,
                payload_length: 0,
                reserved: [0; 24],
            },
            entries: values
                .iter()
                .map(|value| inline_utf8_entry(value))
                .collect(),
            payload: Vec::new(),
        }
    }

    fn inline_utf8_entry(value: &str) -> FileDictionaryIndexEntryV1 {
        let mut canonical = wire::encode_u64_leb128(value.len() as u64);
        canonical.extend_from_slice(value.as_bytes());
        let mut inline_data = [0u8; 16];
        inline_data[..canonical.len()].copy_from_slice(&canonical);
        FileDictionaryIndexEntryV1 {
            value_tag: ValueTag::Utf8 as u16,
            storage_class: StorageClass::Inline as u8,
            flags: 0,
            inline_len: canonical.len() as u8,
            reserved0: [0; 3],
            inline_data,
            payload_offset: 0,
            payload_length: 0,
            canonical_hash64: 0,
            reserved1: 0,
        }
    }

    fn filecode_column(codes: Vec<u32>) -> ConvertedColumn {
        ConvertedColumn {
            entry: ColumnEntry {
                column_id: 7,
                name: "city".into(),
                logical: CoveLogicalType::Utf8,
                physical: CovePhysicalKind::FileCode,
                nullable: false,
                sort_order: 0,
                collation_id: 1,
                precision: 0,
                scale: 0,
                flags: 0,
            },
            source_kind: SourceColumnKind::Utf8,
            source_type: "Utf8".into(),
            encoding: CoveEncodingKind::FileCode,
            fallback: None,
            pushdown_limited: false,
            notes: Vec::new(),
            nulls: vec![false; codes.len()],
            values: MaterializedValues::FileCode(codes),
            nested: None,
        }
    }
}
