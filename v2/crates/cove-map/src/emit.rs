use cove_core::{
    compression::encode_payload_for_codec,
    constants::FEATURE_CODEC_ZSTD,
    profile::cove_map::{compact_evidence_index_bytes, MapEvidenceIndex},
};

use super::*;

const SECTION_COMPRESSION_MIN_BYTES: usize = 4 * 1024;

#[cfg(test)]
pub(crate) fn build_cove_o(file: &CovemapFile, rows: &[SourceRow]) -> Result<Vec<u8>, String> {
    build_cove_o_with_source_states(file, rows, &[])
}

pub(crate) fn build_cove_o_with_source_states(
    file: &CovemapFile,
    rows: &[SourceRow],
    source_states: &[ObservedSourceState],
) -> Result<Vec<u8>, String> {
    let materialized = materialize_with_source_states(file, rows, source_states)?;
    build_cove_o_from_materialized(file, &materialized)
}

pub(crate) fn build_cove_o_from_materialized(
    file: &CovemapFile,
    materialized: &MaterializedModel,
) -> Result<Vec<u8>, String> {
    build_cove_o_from_materialized_with_evidence_encoding(
        file,
        materialized,
        MapEvidenceEncoding::Compact,
    )
}

pub(crate) fn build_cove_o_from_materialized_with_evidence_encoding(
    file: &CovemapFile,
    materialized: &MaterializedModel,
    evidence_encoding: MapEvidenceEncoding,
) -> Result<Vec<u8>, String> {
    Ok(build_cove_o_from_materialized_with_options(
        file,
        materialized,
        evidence_encoding,
        MapBuildSectionCompression::None,
    )?
    .bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoveOBuildOutput {
    pub bytes: Vec<u8>,
    pub compression_summary: CoveOCompressionSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoveOCompressionSummary {
    pub mode: MapBuildSectionCompression,
    pub threshold_bytes: usize,
    pub uncompressed_section_bytes: u64,
    pub emitted_section_bytes: u64,
    pub saved_bytes: u64,
    pub compressed_section_count: usize,
    pub sections: Vec<CoveOSectionCompressionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoveOSectionCompressionSummary {
    pub section_kind: u16,
    pub section_name: String,
    pub profile: String,
    pub compression: String,
    pub uncompressed_bytes: usize,
    pub emitted_bytes: usize,
    pub saved_bytes: usize,
}

pub(crate) fn build_cove_o_from_materialized_with_options(
    file: &CovemapFile,
    materialized: &MaterializedModel,
    evidence_encoding: MapEvidenceEncoding,
    section_compression: MapBuildSectionCompression,
) -> Result<CoveOBuildOutput, String> {
    let nested_shapes = nested_shapes_for_model(file, materialized)?;
    let file_dictionary = file_dictionary_for_model(materialized, &nested_shapes)?;
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: materialized.object_types.clone(),
    };
    let segments =
        build_temporal_segments(&materialized, &nested_shapes, file_dictionary.as_ref())?;
    let segment_index = temporal_segment_index(&segments)?;
    let trust_manifest = trust_manifest(&segments, file_dictionary.as_ref())?;

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_TRUST_CHAIN;
    writer.optional_features = FEATURE_SEMANTIC_MAP;
    for section in map_passthrough_sections(file, materialized)? {
        writer.sections.push(section);
    }
    writer.sections.push(object_section(
        SectionKind::ObjectTypeCatalog,
        catalog.types.len() as u64,
        0,
        catalog.serialize().map_err(|err| err.to_string())?,
    ));
    if let Some(dictionary) = &file_dictionary {
        writer.required_features |= FEATURE_FILE_DICTIONARY;
        writer.sections.push(dictionary_section(
            SectionKind::FileDictionaryIndex,
            dictionary.dictionary.len() as u64,
            file_dictionary_index_bytes(&dictionary.dictionary),
        ));
        if !dictionary.dictionary.payload.is_empty() {
            writer.sections.push(dictionary_section(
                SectionKind::FileDictionaryPayload,
                dictionary.dictionary.len() as u64,
                dictionary.dictionary.payload.clone(),
            ));
        }
    }
    writer.sections.push(object_section(
        SectionKind::TemporalSegmentIndex,
        segments.len() as u64,
        materialized.rows.len() as u64,
        segment_index.serialize().map_err(|err| err.to_string())?,
    ));
    for segment in &segments {
        writer.sections.push(object_section(
            SectionKind::TemporalSegmentData,
            1,
            segment.rows.len() as u64,
            segment.payload.clone(),
        ));
    }
    writer.sections.push(object_section(
        SectionKind::TrustManifest,
        trust_manifest.entries.len() as u64,
        0,
        trust_manifest.serialize().map_err(|err| err.to_string())?,
    ));
    writer.sections.push(map_section(
        SectionKind::MapAssertionLog,
        materialized.assertions.len() as u64,
        serde_json::to_vec_pretty(&materialized.assertion_log).map_err(|err| err.to_string())?,
    ));
    writer.sections.push(map_section(
        SectionKind::MapIdentityEquivalenceIndex,
        materialized
            .identity_equivalence_index
            .get("equivalences")
            .and_then(Value::as_array)
            .map(|values| values.len() as u64)
            .unwrap_or(0),
        serde_json::to_vec_pretty(&materialized.identity_equivalence_index)
            .map_err(|err| err.to_string())?,
    ));
    writer.sections.push(map_section(
        SectionKind::MapEvidenceIndex,
        materialized.evidence_entries.len() as u64,
        evidence_index_section_bytes(materialized, evidence_encoding)?,
    ));
    writer.sections.push(map_section(
        SectionKind::MapConversionReport,
        1,
        serde_json::to_vec_pretty(&materialized.conversion_report)
            .map_err(|err| err.to_string())?,
    ));
    let compression_summary = apply_section_compression(&mut writer, section_compression)?;
    let bytes = writer.write().map_err(|err| err.to_string())?;
    validate_bytes_with_options(
        &bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .map_err(|err| err.to_string())?;
    Ok(CoveOBuildOutput {
        bytes,
        compression_summary,
    })
}

fn apply_section_compression(
    writer: &mut MinimalCoveWriter,
    mode: MapBuildSectionCompression,
) -> Result<CoveOCompressionSummary, String> {
    let mut sections = Vec::with_capacity(writer.sections.len());
    let mut uncompressed_section_bytes = 0u64;
    let mut emitted_section_bytes = 0u64;
    let mut compressed_section_count = 0usize;
    let mut required_zstd = false;
    let mut optional_zstd = false;

    for section in &mut writer.sections {
        let uncompressed_bytes = section.data.len();
        let mut emitted_bytes = uncompressed_bytes;
        let mut compression = CompressionCodec::None;

        if mode == MapBuildSectionCompression::Zstd
            && uncompressed_bytes >= SECTION_COMPRESSION_MIN_BYTES
        {
            let compressed = encode_payload_for_codec(&section.data, CompressionCodec::Zstd as u8)
                .map_err(|err| format!("cannot zstd-compress generated COVE-O section: {err}"))?;
            if compressed.len() < uncompressed_bytes {
                section.compression = CompressionCodec::Zstd as u8;
                emitted_bytes = compressed.len();
                compression = CompressionCodec::Zstd;
                compressed_section_count += 1;
                if section.profile == PrimaryProfile::SemanticMapping as u8 {
                    section.optional_features |= FEATURE_CODEC_ZSTD;
                    optional_zstd = true;
                } else {
                    section.required_features |= FEATURE_CODEC_ZSTD;
                    required_zstd = true;
                }
            }
        }

        uncompressed_section_bytes += uncompressed_bytes as u64;
        emitted_section_bytes += emitted_bytes as u64;
        sections.push(CoveOSectionCompressionSummary {
            section_kind: section.section_kind,
            section_name: section_kind_label(section.section_kind),
            profile: profile_label(section.profile),
            compression: compression_label(compression),
            uncompressed_bytes,
            emitted_bytes,
            saved_bytes: uncompressed_bytes.saturating_sub(emitted_bytes),
        });
    }

    if required_zstd {
        writer.required_features |= FEATURE_CODEC_ZSTD;
    } else if optional_zstd {
        writer.optional_features |= FEATURE_CODEC_ZSTD;
    }

    Ok(CoveOCompressionSummary {
        mode,
        threshold_bytes: SECTION_COMPRESSION_MIN_BYTES,
        uncompressed_section_bytes,
        emitted_section_bytes,
        saved_bytes: uncompressed_section_bytes.saturating_sub(emitted_section_bytes),
        compressed_section_count,
        sections,
    })
}

fn section_kind_label(section_kind: u16) -> String {
    SectionKind::from_u16(section_kind)
        .map(|kind| format!("{kind:?}"))
        .unwrap_or_else(|| format!("unknown:{section_kind}"))
}

fn profile_label(profile: u8) -> String {
    PrimaryProfile::from_u8(profile)
        .map(|profile| format!("{profile:?}"))
        .unwrap_or_else(|| format!("unknown:{profile}"))
}

fn compression_label(codec: CompressionCodec) -> String {
    match codec {
        CompressionCodec::None => "none",
        CompressionCodec::Lz4 => "lz4",
        CompressionCodec::Zstd => "zstd",
        _ => "unknown",
    }
    .into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceEncodingSummary {
    pub encoding: MapEvidenceEncoding,
    pub logical_entry_count: usize,
    pub expanded_json_bytes: usize,
    pub emitted_index_bytes: usize,
    pub compact_binary_bytes: Option<usize>,
    pub estimated_saved_bytes: i64,
}

pub(crate) fn evidence_encoding_summary(
    materialized: &MaterializedModel,
    encoding: MapEvidenceEncoding,
) -> Result<EvidenceEncodingSummary, String> {
    let expanded = expanded_evidence_index_bytes(materialized)?;
    let compact = compact_evidence_index_bytes(&evidence_index_from_materialized(materialized)?)
        .map_err(|err| format!("cannot compact MAP_EVIDENCE_INDEX: {err}"))?;
    let emitted_index_bytes = match encoding {
        MapEvidenceEncoding::Compact | MapEvidenceEncoding::Both => compact.len(),
        MapEvidenceEncoding::Expanded => expanded.len(),
    };
    let compact_binary_bytes = match encoding {
        MapEvidenceEncoding::Compact | MapEvidenceEncoding::Both => Some(compact.len()),
        MapEvidenceEncoding::Expanded => None,
    };
    Ok(EvidenceEncodingSummary {
        encoding,
        logical_entry_count: materialized.evidence_entries.len(),
        expanded_json_bytes: expanded.len(),
        emitted_index_bytes,
        compact_binary_bytes,
        estimated_saved_bytes: expanded.len() as i64 - emitted_index_bytes as i64,
    })
}

fn evidence_index_section_bytes(
    materialized: &MaterializedModel,
    encoding: MapEvidenceEncoding,
) -> Result<Vec<u8>, String> {
    match encoding {
        MapEvidenceEncoding::Expanded => expanded_evidence_index_bytes(materialized),
        MapEvidenceEncoding::Compact | MapEvidenceEncoding::Both => {
            compact_evidence_index_bytes(&evidence_index_from_materialized(materialized)?)
                .map_err(|err| format!("cannot compact MAP_EVIDENCE_INDEX: {err}"))
        }
    }
}

fn expanded_evidence_index_bytes(materialized: &MaterializedModel) -> Result<Vec<u8>, String> {
    let data =
        serde_json::to_vec_pretty(&materialized.evidence_index).map_err(|err| err.to_string())?;
    Ok(ensure_covemap_payload_envelope(
        SectionKind::MapEvidenceIndex,
        data,
    ))
}

fn evidence_index_from_materialized(
    materialized: &MaterializedModel,
) -> Result<MapEvidenceIndex, String> {
    let bytes = expanded_evidence_index_bytes(materialized)?;
    MapEvidenceIndex::parse(&bytes)
        .map_err(|err| format!("expanded MAP_EVIDENCE_INDEX invalid: {err}"))
}
