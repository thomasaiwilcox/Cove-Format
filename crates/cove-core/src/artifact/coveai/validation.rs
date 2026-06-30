use std::collections::BTreeSet;

use crate::{checksum, CoveError};

use super::{read_u32, AiPayloadRefEntryV1, AiStorageKindV1, CoveAiSection, TokenBlockHeaderV1};

pub(super) fn validate_cached_payload_range(
    label: &str,
    id: u32,
    payload_ref: &AiPayloadRefEntryV1,
    cached_offset: u64,
    cached_length: u64,
) -> Result<(), CoveError> {
    match AiStorageKindV1::from_u8(payload_ref.storage_kind)
        .ok_or_else(|| CoveError::BadSection("unknown AI payload storage_kind".into()))?
    {
        AiStorageKindV1::ArtifactAbsolute => {
            if cached_length != 0 {
                if cached_offset != payload_ref.payload_offset
                    || cached_length != payload_ref.payload_length
                {
                    return Err(CoveError::BadSection(format!(
                        "{label} {id} cached payload range does not match payload_ref"
                    )));
                }
            } else if cached_offset != 0 {
                return Err(CoveError::BadSection(format!(
                    "{label} {id} has zero cached payload length but non-zero offset"
                )));
            }
        }
        _ => {
            if cached_offset != 0 || cached_length != 0 {
                return Err(CoveError::BadSection(format!(
                    "{label} {id} must not cache artifact offsets for non-ArtifactAbsolute payload refs"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_unique<I>(values: I, label: &str) -> Result<(), CoveError>
where
    I: IntoIterator<Item = u32>,
{
    let mut seen = BTreeSet::new();
    for value in values {
        if value == 0 {
            return Err(CoveError::BadSection(format!("{label} must be non-zero")));
        }
        if !seen.insert(value) {
            return Err(CoveError::BadSection(format!("duplicate {label} {value}")));
        }
    }
    Ok(())
}

pub(super) fn validate_unique_u64<I>(values: I, label: &str) -> Result<(), CoveError>
where
    I: IntoIterator<Item = u64>,
{
    let mut seen = BTreeSet::new();
    for value in values {
        if value == 0 {
            return Err(CoveError::BadSection(format!("{label} must be non-zero")));
        }
        if !seen.insert(value) {
            return Err(CoveError::BadSection(format!("duplicate {label} {value}")));
        }
    }
    Ok(())
}

pub(super) fn section_by_id(sections: &[CoveAiSection], section_id: u32) -> Option<&CoveAiSection> {
    sections
        .iter()
        .find(|section| section.entry.section_id == section_id)
}

pub(super) fn element_width_bytes(element_type: u8) -> Option<u64> {
    match element_type {
        0 => Some(4),
        1 | 2 => Some(2),
        3 | 4 => Some(1),
        6 => Some(8),
        5 => None,
        _ => None,
    }
}

pub(super) fn validate_ai_vector_element_type(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 6 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn vector_index_kind_name(value: u8) -> &'static str {
    match value {
        0 => "exact_flat",
        1 => "hnsw",
        2 => "ivf_flat",
        3 => "ivf_pq",
        4 => "diskann",
        5 => "vamana",
        6 => "pq",
        7 => "scalar_quantized",
        8 => "extension_candidate",
        255 => "extension",
        _ => "unknown",
    }
}

pub(super) fn validate_ai_vector_metric(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_index_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 8 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_index_exactness(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_false_negative_policy(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_score_space_authority(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_indexed_binding_kind(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_compatibility_kind(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_compatibility_authority(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_normalization_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_quantization_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_compression_codec(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 2 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_layout_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_chunk_boundary_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_chunk_overlap_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_chunk_parent_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_digest_domain(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_privacy_state(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_reproducibility_class(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_result_authority(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_composition_method(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_composition_missing_policy(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_redaction_behavior(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_component_missing_behavior(
    value: u8,
    label: &str,
) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_arithmetic_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_accumulator_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_rounding_mode(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_overflow_policy(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_vector_component_order(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 3 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_tensor_dtype(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_byte_order(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 2 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_device_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_asset_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 6 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_modality(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 7 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_role(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 6 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_multimodal_element_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_generator_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 4 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_review_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_split_method(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_similarity_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_dedup_authority(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 5 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_ai_permutation_kind(value: u8, label: &str) -> Result<(), CoveError> {
    if value <= 15 || value == 255 {
        return Ok(());
    }
    Err(CoveError::BadSection(format!(
        "{label} has unsupported value {value}"
    )))
}

pub(super) fn validate_bool_byte(value: u8, label: &str) -> Result<(), CoveError> {
    if value > 1 {
        return Err(CoveError::BadSection(format!(
            "{label} must be 0 or 1, got {value}"
        )));
    }
    Ok(())
}

pub(super) fn validate_power_of_two_alignment(value: u32, label: &str) -> Result<(), CoveError> {
    if value != 0 && !value.is_power_of_two() {
        return Err(CoveError::BadSection(format!(
            "{label} must be zero or a power of two, got {value}"
        )));
    }
    Ok(())
}

pub(super) fn validate_token_range(
    label: &str,
    id: u64,
    token_offset: u64,
    token_count: u32,
    block: &TokenBlockHeaderV1,
) -> Result<(), CoveError> {
    let end = token_offset
        .checked_add(u64::from(token_count))
        .ok_or(CoveError::ArithOverflow)?;
    if end > block.token_count {
        return Err(CoveError::BadSection(format!(
            "{label} {id} token range exceeds token_block_ref {} token_count",
            block.token_block_id
        )));
    }
    Ok(())
}

pub(super) fn verify_payload_crc(payload: &[u8], checksum_offset: usize) -> Result<(), CoveError> {
    if checksum_offset + 4 > payload.len() {
        return Err(CoveError::BufferTooShort);
    }
    let expected = read_u32(payload, checksum_offset)?;
    verify_crc32c(payload, checksum_offset, expected)
}

pub(super) fn verify_crc32c(
    bytes: &[u8],
    checksum_offset: usize,
    expected: u32,
) -> Result<(), CoveError> {
    if checksum_offset + 4 > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let mut for_crc = bytes.to_vec();
    for_crc[checksum_offset..checksum_offset + 4].fill(0);
    if checksum::crc32c(&for_crc) != expected {
        return Err(CoveError::ChecksumMismatch);
    }
    Ok(())
}

pub(super) fn checked_slice(data: &[u8], offset: u64, length: u64) -> Result<&[u8], CoveError> {
    let end = checked_range(offset, length, data.len() as u64)?;
    Ok(&data[offset as usize..end as usize])
}

pub(super) fn checked_range(offset: u64, length: u64, bound: u64) -> Result<u64, CoveError> {
    let end = offset.checked_add(length).ok_or(CoveError::ArithOverflow)?;
    if end > bound {
        return Err(CoveError::OffsetRange);
    }
    Ok(end)
}

pub(super) fn range_contains(
    outer_offset: u64,
    outer_len: u64,
    inner_offset: u64,
    inner_len: u64,
) -> bool {
    let Some(outer_end) = outer_offset.checked_add(outer_len) else {
        return false;
    };
    let Some(inner_end) = inner_offset.checked_add(inner_len) else {
        return false;
    };
    inner_offset >= outer_offset && inner_end <= outer_end
}
