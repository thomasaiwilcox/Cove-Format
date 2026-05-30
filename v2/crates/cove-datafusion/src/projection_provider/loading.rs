use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::Range,
    path::Path,
};

use async_trait::async_trait;
use cove_core::{
    artifact::covemap::CovemapFile,
    checksum, compression,
    constants::{SectionKind, FOOTER_VERSION_V1, MAGIC_COVE_FOOTER, SECTION_ENTRY_LEN},
    footer::{CoveFooter, CoveFooterHeaderV1, CoveSectionEntryV1},
    header::{CoveHeaderV1, HEADER_SIZE},
    postscript::{CovePostscriptV1, CoveSectionSpecV1, POSTSCRIPT_TOTAL_SIZE},
    profile::{
        cove_map::{parse_embedded_section, EmbeddedMapSection, MapProjectionCatalog},
        cove_o::{
            ObjectTypeCatalog, TemporalSegmentData, TemporalSegmentHeaderV1, TemporalSegmentIndex,
            TemporalSegmentIndexEntryV1, OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            OBJECT_TYPE_FLAG_LINK_OBJECT,
        },
    },
};
use cove_map::{
    projection_read_requirements_for_catalog, ProjectionBatchOptions, ProjectionReadRequirements,
};
use datafusion::common::{DataFusionError, Result};
use futures::executor::block_on;

use crate::{
    adapter_v53::metrics::CoveFileMetrics,
    range_reader::{CoveRangeReader, RangeReadKind},
};

#[derive(Debug, Clone)]
pub(super) struct InstrumentedProjectionRangeReader<R> {
    inner: R,
    metrics: CoveFileMetrics,
}

pub(super) struct LoadedProjectionBytes {
    pub(super) bytes: Vec<u8>,
    pub(super) projection_catalog: MapProjectionCatalog,
}

impl<R> InstrumentedProjectionRangeReader<R> {
    pub(super) fn new(inner: R, metrics: CoveFileMetrics) -> Self {
        Self { inner, metrics }
    }
}

#[async_trait]
impl<R> CoveRangeReader for InstrumentedProjectionRangeReader<R>
where
    R: CoveRangeReader,
{
    async fn read_ranges(
        &self,
        ranges: &[Range<u64>],
        kind: RangeReadKind,
    ) -> std::result::Result<Vec<Vec<u8>>, cove_core::CoveError> {
        self.metrics.range_requests.add(ranges.len());
        let chunks = self.inner.read_ranges(ranges, kind).await?;
        let mut bytes_read = 0usize;
        for chunk in &chunks {
            bytes_read = bytes_read
                .checked_add(chunk.len())
                .ok_or(cove_core::CoveError::ArithOverflow)?;
        }
        match kind {
            RangeReadKind::Metadata => self.metrics.metadata_bytes_read.add(bytes_read),
            RangeReadKind::Data => self.metrics.data_bytes_read.add(bytes_read),
        }
        Ok(chunks)
    }

    fn record_coalescing(&self, original_ranges: usize, coalesced_ranges: usize) {
        if original_ranges > coalesced_ranges {
            self.metrics.coalesced_range_requests.add(coalesced_ranges);
        }
        self.inner
            .record_coalescing(original_ranges, coalesced_ranges);
    }
}

pub(super) fn load_projection_bytes_via_ranges<R: CoveRangeReader>(
    reader: &R,
    object_len: u64,
    mapping_path: Option<&Path>,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<LoadedProjectionBytes> {
    let header = read_cove_header(reader)?;
    let postscript = read_cove_postscript(reader, object_len)?;
    let footer = read_cove_footer(reader, &postscript.footer)?;
    let metadata_sections = load_metadata_sections(reader, &footer)?;
    let projection_catalog =
        projection_catalog_from_sections_or_mapping(&metadata_sections, mapping_path)?;
    let requirements =
        projection_read_requirements_for_catalog(&projection_catalog, projection_id, options)
            .map_err(|err| {
                DataFusionError::Execution(format!(
            "cannot compile mapped read requirements for projection '{projection_id}': {err}"
        ))
            })?;
    let object_catalog = object_catalog_from_sections(&metadata_sections)?;
    let temporal_sections = load_selected_temporal_sections(
        reader,
        &footer,
        &object_catalog,
        &metadata_sections,
        &requirements,
    )?;
    let bytes =
        repack_selected_cove_sections(&header, &footer, metadata_sections, temporal_sections)?;
    Ok(LoadedProjectionBytes {
        bytes,
        projection_catalog,
    })
}

fn read_cove_header<R: CoveRangeReader>(reader: &R) -> Result<CoveHeaderV1> {
    let bytes = block_on(reader.read_range(0..HEADER_SIZE as u64, RangeReadKind::Metadata))
        .map_err(|err| DataFusionError::Execution(format!("cannot read COVE header: {err}")))?;
    CoveHeaderV1::parse(&bytes)
        .map_err(|err| DataFusionError::Execution(format!("cannot parse COVE header: {err}")))
}

fn read_cove_postscript<R: CoveRangeReader>(
    reader: &R,
    object_len: u64,
) -> Result<CovePostscriptV1> {
    if object_len < POSTSCRIPT_TOTAL_SIZE as u64 {
        return Err(DataFusionError::Execution(format!(
            "mapped COVE-O length {object_len} is shorter than the postscript"
        )));
    }
    let start = object_len - POSTSCRIPT_TOTAL_SIZE as u64;
    let bytes = block_on(reader.read_range(start..object_len, RangeReadKind::Metadata))
        .map_err(|err| DataFusionError::Execution(format!("cannot read COVE postscript: {err}")))?;
    let postscript = CovePostscriptV1::parse_from_tail_bytes(&bytes).map_err(|err| {
        DataFusionError::Execution(format!("cannot parse COVE postscript tail: {err}"))
    })?;
    if postscript.file_len != object_len {
        return Err(DataFusionError::Execution(format!(
            "mapped COVE-O postscript file_len {} does not match actual file length {}",
            postscript.file_len, object_len
        )));
    }
    Ok(postscript)
}

fn read_cove_footer<R: CoveRangeReader>(
    reader: &R,
    footer_spec: &CoveSectionSpecV1,
) -> Result<CoveFooter> {
    let footer_end = footer_spec.end_offset().map_err(|err| {
        DataFusionError::Execution(format!("invalid COVE footer range in postscript: {err}"))
    })?;
    let raw = block_on(reader.read_range(footer_spec.offset..footer_end, RangeReadKind::Metadata))
        .map_err(|err| DataFusionError::Execution(format!("cannot read COVE footer: {err}")))?;
    let payload = compression::section_payload_from_raw(
        &raw,
        footer_spec.length,
        footer_spec.uncompressed_length,
        footer_spec.compression,
        footer_spec.crc32c,
    )
    .map_err(|err| DataFusionError::Execution(format!("cannot decode COVE footer: {err}")))?;
    CoveFooter::parse(payload.as_ref())
        .map_err(|err| DataFusionError::Execution(format!("cannot parse COVE footer: {err}")))
}

fn load_metadata_sections<R: CoveRangeReader>(
    reader: &R,
    footer: &CoveFooter,
) -> Result<Vec<(CoveSectionEntryV1, Vec<u8>)>> {
    let metadata_entries = footer
        .sections
        .iter()
        .filter(|entry| {
            entry.section_kind != SectionKind::TemporalSegmentData as u16
                && entry.section_kind != SectionKind::TrustManifest as u16
        })
        .cloned()
        .collect::<Vec<_>>();
    load_section_raws(reader, &metadata_entries, RangeReadKind::Metadata)
}

fn load_section_raws<R: CoveRangeReader>(
    reader: &R,
    entries: &[CoveSectionEntryV1],
    kind: RangeReadKind,
) -> Result<Vec<(CoveSectionEntryV1, Vec<u8>)>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let ranges = entries
        .iter()
        .map(|entry| {
            entry
                .end_offset()
                .map(|end| entry.offset..end)
                .map_err(|err| DataFusionError::Execution(format!("invalid section range: {err}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let raws = block_on(reader.read_ranges(&ranges, kind)).map_err(|err| {
        DataFusionError::Execution(format!("cannot read COVE section ranges: {err}"))
    })?;
    Ok(entries.iter().cloned().zip(raws).collect())
}

fn projection_catalog_from_sections_or_mapping(
    sections: &[(CoveSectionEntryV1, Vec<u8>)],
    mapping_path: Option<&Path>,
) -> Result<MapProjectionCatalog> {
    if let Some(catalog) = projection_catalog_from_sections(sections)? {
        return Ok(catalog);
    }
    let mapping_path = mapping_path.ok_or_else(|| {
        DataFusionError::Execution(
            "mapped COVE-O does not contain an embedded projection catalog and no .covemap path was supplied"
                .into(),
        )
    })?;
    let bytes = std::fs::read(mapping_path).map_err(|err| {
        DataFusionError::Execution(format!(
            "cannot read mapping file {}: {err}",
            mapping_path.display()
        ))
    })?;
    let covemap = CovemapFile::parse_validated(&bytes).map_err(|err| {
        DataFusionError::Execution(format!(
            "cannot parse mapping file {}: {err}",
            mapping_path.display()
        ))
    })?;
    covemap
        .sections
        .iter()
        .find_map(|section| {
            let Ok(section_id) = u16::try_from(section.entry.section_id) else {
                return None;
            };
            let kind = SectionKind::from_u16(section_id)?;
            match parse_embedded_section(kind, section.payload.as_slice()) {
                Ok(EmbeddedMapSection::ProjectionCatalog(catalog)) => Some(Ok(catalog)),
                Ok(_) => None,
                Err(err) => Some(Err(DataFusionError::Execution(format!(
                    "cannot parse projection catalog in {}: {err}",
                    mapping_path.display()
                )))),
            }
        })
        .transpose()?
        .ok_or_else(|| {
            DataFusionError::Execution(format!(
                "mapping file {} does not contain a projection catalog",
                mapping_path.display()
            ))
        })
}

fn projection_catalog_from_sections(
    sections: &[(CoveSectionEntryV1, Vec<u8>)],
) -> Result<Option<MapProjectionCatalog>> {
    for (entry, raw) in sections {
        if entry.section_kind != SectionKind::MapProjectionCatalog as u16 {
            continue;
        }
        let payload = decode_section_payload(entry, raw)?;
        let section = parse_embedded_section(SectionKind::MapProjectionCatalog, payload.as_ref())
            .map_err(|err| {
            DataFusionError::Execution(format!("cannot parse embedded projection catalog: {err}"))
        })?;
        if let EmbeddedMapSection::ProjectionCatalog(catalog) = section {
            return Ok(Some(catalog));
        }
    }
    Ok(None)
}

fn object_catalog_from_sections(
    sections: &[(CoveSectionEntryV1, Vec<u8>)],
) -> Result<ObjectTypeCatalog> {
    for (entry, raw) in sections {
        if entry.section_kind != SectionKind::ObjectTypeCatalog as u16 {
            continue;
        }
        let payload = decode_section_payload(entry, raw)?;
        return ObjectTypeCatalog::parse(payload.as_ref()).map_err(|err| {
            DataFusionError::Execution(format!("cannot parse object type catalog: {err}"))
        });
    }
    Err(DataFusionError::Execution(
        "mapped COVE-O is missing an object type catalog".into(),
    ))
}

fn temporal_index_from_sections(
    sections: &[(CoveSectionEntryV1, Vec<u8>)],
) -> Result<TemporalSegmentIndex> {
    for (entry, raw) in sections {
        if entry.section_kind != SectionKind::TemporalSegmentIndex as u16 {
            continue;
        }
        let payload = decode_section_payload(entry, raw)?;
        return TemporalSegmentIndex::parse(payload.as_ref()).map_err(|err| {
            DataFusionError::Execution(format!("cannot parse temporal segment index: {err}"))
        });
    }
    Err(DataFusionError::Execution(
        "mapped COVE-O is missing a temporal segment index".into(),
    ))
}

fn load_selected_temporal_sections<R: CoveRangeReader>(
    reader: &R,
    footer: &CoveFooter,
    object_catalog: &ObjectTypeCatalog,
    metadata_sections: &[(CoveSectionEntryV1, Vec<u8>)],
    requirements: &ProjectionReadRequirements,
) -> Result<Vec<(CoveSectionEntryV1, Vec<u8>)>> {
    if !requirements.include_records {
        return Ok(Vec::new());
    }
    let temporal_index = temporal_index_from_sections(metadata_sections)?;
    let object_types_by_id = object_catalog
        .types
        .iter()
        .map(|object_type| (object_type.object_type_id, object_type))
        .collect::<BTreeMap<_, _>>();
    let temporal_entries = footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::TemporalSegmentData as u16)
        .cloned()
        .collect::<Vec<_>>();
    let mut segment_entries_by_id = BTreeMap::new();
    let mut preloaded_raws = BTreeMap::new();
    for entry in &temporal_entries {
        let (header, maybe_raw) = probe_temporal_segment_header(reader, entry)?;
        segment_entries_by_id.insert(header.segment_id, entry.clone());
        if let Some(raw) = maybe_raw {
            preloaded_raws.insert(header.segment_id, raw);
        }
    }
    let seed_segment_ids = temporal_index
        .entries
        .iter()
        .filter(|index_entry| {
            let Some(object_type) = object_types_by_id.get(&index_entry.object_type_id) else {
                return true;
            };
            requirements.requested_object_type_names.is_empty()
                || requirements
                    .requested_object_type_names
                    .iter()
                    .any(|name| name == &object_type.type_name)
                || (requirements.include_association_object_types
                    && object_type_is_association_like(object_type))
        })
        .map(|index_entry| index_entry.segment_id)
        .collect::<BTreeSet<_>>();
    let mut queue = seed_segment_ids.iter().copied().collect::<VecDeque<_>>();
    let mut selected_segment_ids = BTreeSet::new();
    let mut selected_sections = BTreeMap::new();
    while let Some(segment_id) = queue.pop_front() {
        if !selected_segment_ids.insert(segment_id) {
            continue;
        }
        let entry = segment_entries_by_id.get(&segment_id).ok_or_else(|| {
            DataFusionError::Execution(format!(
                "temporal segment dependency {} is missing from the temporal index",
                segment_id
            ))
        })?;
        let raw = if let Some(raw) = preloaded_raws.remove(&segment_id) {
            raw
        } else {
            block_on(reader.read_range(
                entry.offset..entry.end_offset().map_err(|err| {
                    DataFusionError::Execution(format!("invalid temporal segment range: {err}"))
                })?,
                RangeReadKind::Data,
            ))
            .map_err(|err| {
                DataFusionError::Execution(format!("cannot read temporal segment: {err}"))
            })?
        };
        let payload = decode_section_payload(entry, &raw)?;
        let segment = TemporalSegmentData::parse(payload.as_ref()).map_err(|err| {
            DataFusionError::Execution(format!("cannot parse temporal segment: {err}"))
        })?;
        for prev_ref in segment.rows.iter().filter_map(|row| row.prev_ref) {
            if !selected_segment_ids.contains(&prev_ref.segment_id) {
                queue.push_back(prev_ref.segment_id);
            }
        }
        selected_sections.insert(segment_id, (entry.clone(), raw));
    }
    Ok(selected_sections.into_values().collect::<Vec<_>>())
}

fn probe_temporal_segment_header<R: CoveRangeReader>(
    reader: &R,
    entry: &CoveSectionEntryV1,
) -> Result<(TemporalSegmentHeaderV1, Option<Vec<u8>>)> {
    if entry.compression == 0 && entry.length >= std::mem::size_of::<[u8; 96]>() as u64 {
        let header_bytes =
            block_on(reader.read_range(entry.offset..entry.offset + 96, RangeReadKind::Data))
                .map_err(|err| {
                    DataFusionError::Execution(format!(
                        "cannot read temporal segment header: {err}"
                    ))
                })?;
        let header = TemporalSegmentHeaderV1::parse(&header_bytes).map_err(|err| {
            DataFusionError::Execution(format!("cannot parse temporal segment header: {err}"))
        })?;
        return Ok((header, None));
    }
    let raw = block_on(reader.read_range(
        entry.offset..entry.end_offset().map_err(|err| {
            DataFusionError::Execution(format!("invalid temporal segment range: {err}"))
        })?,
        RangeReadKind::Data,
    ))
    .map_err(|err| DataFusionError::Execution(format!("cannot read temporal segment: {err}")))?;
    let payload = decode_section_payload(entry, &raw)?;
    let header = TemporalSegmentHeaderV1::parse(payload.as_ref()).map_err(|err| {
        DataFusionError::Execution(format!("cannot parse temporal segment header: {err}"))
    })?;
    Ok((header, Some(raw)))
}

fn object_type_is_association_like(
    object_type: &cove_core::profile::cove_o::ObjectTypeEntryV1,
) -> bool {
    object_type.flags & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT) != 0
}

fn decode_section_payload<'a>(
    entry: &'a CoveSectionEntryV1,
    raw: &'a [u8],
) -> Result<std::borrow::Cow<'a, [u8]>> {
    compression::section_payload_from_raw(
        raw,
        entry.length,
        entry.uncompressed_length,
        entry.compression,
        entry.crc32c,
    )
    .map_err(|err| {
        DataFusionError::Execution(format!("cannot decode section {}: {err}", entry.section_id))
    })
}

fn repack_selected_cove_sections(
    header: &CoveHeaderV1,
    footer: &CoveFooter,
    metadata_sections: Vec<(CoveSectionEntryV1, Vec<u8>)>,
    temporal_sections: Vec<(CoveSectionEntryV1, Vec<u8>)>,
) -> Result<Vec<u8>> {
    let temporal_segment_keys = temporal_sections
        .iter()
        .map(|(entry, raw)| {
            let payload = decode_section_payload(entry, raw)?;
            let segment = TemporalSegmentHeaderV1::parse(payload.as_ref()).map_err(|err| {
                DataFusionError::Execution(format!("cannot parse temporal segment header: {err}"))
            })?;
            Ok((
                (segment.object_type_id, segment.segment_id),
                entry.section_id,
            ))
        })
        .collect::<Result<BTreeMap<(u32, u32), u32>>>()?;
    let temporal_index_rewrite = metadata_sections
        .iter()
        .find(|(entry, _)| entry.section_kind == SectionKind::TemporalSegmentIndex as u16)
        .map(|(entry, raw)| {
            let payload = decode_section_payload(entry, raw)?;
            let index = TemporalSegmentIndex::parse(payload.as_ref()).map_err(|err| {
                DataFusionError::Execution(format!("cannot parse temporal segment index: {err}"))
            })?;
            let entries = index
                .entries
                .into_iter()
                .filter(|index_entry| {
                    temporal_segment_keys
                        .contains_key(&(index_entry.object_type_id, index_entry.segment_id))
                })
                .collect::<Vec<_>>();
            Ok::<(u32, u32, Vec<TemporalSegmentIndexEntryV1>), DataFusionError>((
                entry.section_id,
                index.flags,
                entries,
            ))
        })
        .transpose()?;
    let mut raw_by_section_id = metadata_sections
        .into_iter()
        .chain(temporal_sections)
        .map(|(entry, raw)| (entry.section_id, (entry, raw)))
        .collect::<BTreeMap<_, _>>();

    let mut bytes = vec![0u8; HEADER_SIZE];
    let mut rebuilt_entries = Vec::new();
    let mut temporal_index_placeholder: Option<(
        usize,
        usize,
        u32,
        u32,
        Vec<TemporalSegmentIndexEntryV1>,
    )> = None;
    for original_entry in &footer.sections {
        let Some((mut entry, raw)) = raw_by_section_id.remove(&original_entry.section_id) else {
            continue;
        };
        entry.offset = bytes.len() as u64;
        if let Some((section_id, flags, index_entries)) = &temporal_index_rewrite {
            if *section_id == entry.section_id {
                let placeholder = TemporalSegmentIndex {
                    flags: *flags,
                    entries: index_entries.clone(),
                }
                .serialize()
                .map_err(|err| {
                    DataFusionError::Execution(format!(
                        "cannot serialize filtered temporal segment index: {err}"
                    ))
                })?;
                let start = bytes.len();
                bytes.extend_from_slice(&placeholder);
                entry.length = placeholder.len() as u64;
                entry.uncompressed_length = placeholder.len() as u64;
                entry.item_count = index_entries.len() as u64;
                entry.row_count = index_entries
                    .iter()
                    .map(|segment| segment.row_count as u64)
                    .sum();
                entry.compression = 0;
                entry.crc32c = 0;
                temporal_index_placeholder = Some((
                    rebuilt_entries.len(),
                    start,
                    *flags,
                    entry.section_id,
                    index_entries.clone(),
                ));
                rebuilt_entries.push(entry);
                continue;
            }
        }
        bytes.extend_from_slice(&raw);
        rebuilt_entries.push(entry);
    }
    if let Some((entry_index, start, flags, _, index_entries)) = temporal_index_placeholder {
        let section_offsets = rebuilt_entries
            .iter()
            .map(|entry| (entry.section_id, (entry.offset, entry.length)))
            .collect::<BTreeMap<_, _>>();
        let rewritten_entries = index_entries
            .into_iter()
            .map(|mut index_entry| {
                let section_id = temporal_segment_keys
                    .get(&(index_entry.object_type_id, index_entry.segment_id))
                    .ok_or_else(|| {
                        DataFusionError::Execution(format!(
                            "missing selected temporal segment {} for object type {}",
                            index_entry.segment_id, index_entry.object_type_id
                        ))
                    })?;
                let (offset, length) = section_offsets.get(section_id).ok_or_else(|| {
                    DataFusionError::Execution(format!(
                        "missing rebuilt temporal segment section {}",
                        section_id
                    ))
                })?;
                index_entry.offset = *offset;
                index_entry.length = *length;
                Ok(index_entry)
            })
            .collect::<Result<Vec<_>>>()?;
        let rewritten_index = TemporalSegmentIndex {
            flags,
            entries: rewritten_entries,
        }
        .serialize()
        .map_err(|err| {
            DataFusionError::Execution(format!("cannot serialize temporal segment index: {err}"))
        })?;
        let end = start + rewritten_index.len();
        bytes[start..end].copy_from_slice(&rewritten_index);
        rebuilt_entries[entry_index].crc32c = checksum::crc32c(&rewritten_index);
    }

    let footer_header = CoveFooterHeaderV1 {
        footer_magic: MAGIC_COVE_FOOTER,
        footer_version: FOOTER_VERSION_V1,
        header_len: footer.header.header_len,
        flags: footer.header.flags,
        section_count: rebuilt_entries.len() as u32,
        section_entry_len: SECTION_ENTRY_LEN,
        metadata_len: footer.metadata_json.len() as u32,
        reserved: footer.header.reserved,
    };
    let rebuilt_footer = CoveFooter {
        header: footer_header,
        sections: rebuilt_entries,
        metadata_json: footer.metadata_json.clone(),
    };
    let footer_bytes = rebuilt_footer.serialize();
    let footer_offset = bytes.len() as u64;
    let footer_crc32c = checksum::crc32c(&footer_bytes);
    bytes.extend_from_slice(&footer_bytes);

    let postscript = CovePostscriptV1 {
        required_features: header.required_features,
        optional_features: header.optional_features,
        file_len: bytes.len() as u64 + POSTSCRIPT_TOTAL_SIZE as u64,
        footer: CoveSectionSpecV1 {
            offset: footer_offset,
            length: footer_bytes.len() as u64,
            uncompressed_length: footer_bytes.len() as u64,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            flags: 0,
            crc32c: footer_crc32c,
            reserved: 0,
        },
        checksum: 0,
    };
    bytes.extend_from_slice(&postscript.serialize_tail());
    bytes[..HEADER_SIZE].copy_from_slice(&header.serialize());
    Ok(bytes)
}
