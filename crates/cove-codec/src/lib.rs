//! COVE-CX registered codec descriptors and envelopes for COVE v2.

use cove_core::{wire, CoveError};

pub use cove_core::codec::{
    stable_registered_codec_supported_masks, validate_descriptor_set, CodecExtensionDescriptorV2,
    CodecFallbackPolicyV2, CodecRequirementV2, CodecSpecificationStatusV2, LogicalPage,
    RegisteredEncodingEnvelopeV2, ABSENT_REF,
};

pub const FSST_UTF8_CODEC_IDENTITY: (&str, &str, u16, u16) =
    ("org.coveformat.codec", "fsst-utf8", 2, 0);
pub const ALP_FLOAT_CODEC_IDENTITY: (&str, &str, u16, u16) =
    ("org.coveformat.codec", "alp-float", 2, 0);
pub const FASTLANES_INTEGER_CODEC_IDENTITY: (&str, &str, u16, u16) =
    ("org.coveformat.codec", "fastlanes-integer", 2, 0);

mod sealed {
    pub trait Sealed {}

    impl Sealed for super::StableRegisteredCodec {}
}

/// Stable COVE-CX registered codec implementation supported by this facade.
///
/// This trait is sealed because it represents the stable registered codec set
/// defined by COVE-CX. It is not an extension point and does not replace
/// [`cove_core::codec::RegisteredCodecResolver`] for resolving codec descriptors
/// while reading COVE files.
pub trait RegisteredCodec: sealed::Sealed {
    /// Return the COVE-CX descriptor for this stable codec.
    fn descriptor(&self) -> CodecExtensionDescriptorV2;

    /// Encode a logical page using this stable registered codec.
    ///
    /// # Errors
    ///
    /// Returns a [`CoveError`] when the page cannot be represented by the
    /// selected codec or the registered codec envelope cannot be produced.
    fn encode(&self, page: &LogicalPage) -> Result<Vec<u8>, CoveError>;

    /// Decode a stable registered codec payload into a logical page.
    ///
    /// # Errors
    ///
    /// Returns a [`CoveError`] when the payload is malformed, unsupported for
    /// the selected codec, or fails COVE-CX fallback validation invariants.
    fn decode(&self, payload: &[u8]) -> Result<LogicalPage, CoveError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableRegisteredCodec {
    FsstUtf8,
    AlpFloat,
    FastLanesInteger,
}

impl StableRegisteredCodec {
    pub fn descriptor_identity(self) -> (&'static str, &'static str, u16, u16) {
        match self {
            Self::FsstUtf8 => FSST_UTF8_CODEC_IDENTITY,
            Self::AlpFloat => ALP_FLOAT_CODEC_IDENTITY,
            Self::FastLanesInteger => FASTLANES_INTEGER_CODEC_IDENTITY,
        }
    }

    pub fn codec_family(self) -> u16 {
        match self {
            Self::FsstUtf8 => 0,
            Self::AlpFloat => 1,
            Self::FastLanesInteger => 2,
        }
    }

    fn core_registered_type_masks(self) -> (u64, u64) {
        let (namespace, name, version_major, version_minor) = self.descriptor_identity();
        match stable_registered_codec_supported_masks(namespace, name, version_major, version_minor)
        {
            Some(masks) => masks,
            None => {
                debug_assert!(
                    false,
                    "stable codec identity missing from cove-core registry"
                );
                (0, 0)
            }
        }
    }
}

impl RegisteredCodec for StableRegisteredCodec {
    fn descriptor(&self) -> CodecExtensionDescriptorV2 {
        let (namespace, name, version_major, version_minor) = self.descriptor_identity();
        let (logical_type_mask, physical_kind_mask) = self.core_registered_type_masks();
        CodecExtensionDescriptorV2 {
            codec_id: match self {
                StableRegisteredCodec::FsstUtf8 => 1,
                StableRegisteredCodec::AlpFloat => 2,
                StableRegisteredCodec::FastLanesInteger => 3,
            },
            namespace: namespace.into(),
            name: name.into(),
            version_major,
            version_minor,
            codec_family: self.codec_family(),
            logical_type_mask,
            physical_kind_mask,
            requirement: CodecRequirementV2::OptionalWithFallback,
            fallback_policy: CodecFallbackPolicyV2::CoreEncodingPayloadPresent,
            parameter_schema_kind: 0,
            flags: 0,
            specification_status: CodecSpecificationStatusV2::StableRegistered,
            required_feature_bit: 0,
            optional_feature_bit: cove_core::constants::FEATURE_REGISTERED_ENCODINGS,
            spec_digest_algorithm: 1,
            spec_digest: stable_spec_digest(*self),
            conformance_vector_ref: ABSENT_REF,
            fallback_ref: 0,
            private_payload_ref: ABSENT_REF,
            checksum: 0,
        }
    }

    fn encode(&self, page: &LogicalPage) -> Result<Vec<u8>, CoveError> {
        match self {
            StableRegisteredCodec::FsstUtf8 => {
                for value in page.values.iter().flatten() {
                    std::str::from_utf8(value).map_err(|_| CoveError::BadCodecExtension)?;
                }
                encode_row_bytes(b"CFS2", page)
            }
            StableRegisteredCodec::AlpFloat => encode_row_bytes(b"CAF2", page),
            StableRegisteredCodec::FastLanesInteger => encode_row_bytes(b"CFI2", page),
        }
    }

    fn decode(&self, payload: &[u8]) -> Result<LogicalPage, CoveError> {
        let expected_magic = match self {
            StableRegisteredCodec::FsstUtf8 => b"CFS2",
            StableRegisteredCodec::AlpFloat => b"CAF2",
            StableRegisteredCodec::FastLanesInteger => b"CFI2",
        };
        let page = decode_row_bytes(expected_magic, payload)?;
        if *self == StableRegisteredCodec::FsstUtf8 {
            for value in page.values.iter().flatten() {
                std::str::from_utf8(value).map_err(|_| CoveError::BadCodecExtension)?;
            }
        }
        Ok(page)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CodecRegistry {
    codecs: Vec<StableRegisteredCodec>,
}

impl CodecRegistry {
    pub fn stable_v2() -> Self {
        Self {
            codecs: vec![
                StableRegisteredCodec::FsstUtf8,
                StableRegisteredCodec::AlpFloat,
                StableRegisteredCodec::FastLanesInteger,
            ],
        }
    }

    pub fn codec_for_descriptor(
        &self,
        descriptor: &CodecExtensionDescriptorV2,
    ) -> Option<StableRegisteredCodec> {
        self.codecs.iter().copied().find(|codec| {
            let (namespace, name, major, minor) = codec.descriptor_identity();
            descriptor.namespace == namespace
                && descriptor.name == name
                && descriptor.version_major == major
                && descriptor.version_minor == minor
                && descriptor.codec_family == codec.codec_family()
                && descriptor.specification_status == CodecSpecificationStatusV2::StableRegistered
                && descriptor.spec_digest == stable_spec_digest(*codec)
        })
    }
}

impl cove_core::codec::RegisteredCodecResolver for CodecRegistry {
    fn decode_registered_page(
        &self,
        descriptor: &CodecExtensionDescriptorV2,
        envelope: &RegisteredEncodingEnvelopeV2,
        encoded_payload: &[u8],
    ) -> Result<LogicalPage, CoveError> {
        let codec = self
            .codec_for_descriptor(descriptor)
            .filter(|codec| {
                let (_namespace, _name, major, minor) = codec.descriptor_identity();
                envelope.codec_id == descriptor.codec_id
                    && envelope.codec_version_major == major
                    && envelope.codec_version_minor == minor
            })
            .ok_or(CoveError::CodecUnsupported)?;
        codec.decode(encoded_payload)
    }
}

/// Validate that a stable COVE-CX registered payload decodes to its fallback page.
///
/// This helper is intentionally limited to the sealed stable registered codecs
/// exposed by this crate. Resolver integration for file reading belongs to
/// [`cove_core::codec::RegisteredCodecResolver`].
///
/// # Errors
///
/// Returns a [`CoveError`] if the registered payload cannot be decoded or its
/// decoded page is not byte-for-byte equivalent to the fallback page.
pub fn validate_fallback_equivalence<C: RegisteredCodec>(
    codec: &C,
    registered_payload: &[u8],
    fallback: &LogicalPage,
) -> Result<(), CoveError> {
    let decoded = codec.decode(registered_payload)?;
    if &decoded == fallback {
        Ok(())
    } else {
        Err(CoveError::BadCodecExtension)
    }
}

fn stable_spec_digest(codec: StableRegisteredCodec) -> Vec<u8> {
    match codec {
        StableRegisteredCodec::FsstUtf8 => b"COVE-FSST-UTF8-V2-SPEC-DIGEST".to_vec(),
        StableRegisteredCodec::AlpFloat => b"COVE-ALP-FLOAT-V2-SPEC-DIGEST".to_vec(),
        StableRegisteredCodec::FastLanesInteger => b"COVE-FASTLANES-I-V2-SPEC-DIGEST".to_vec(),
    }
}

fn encode_row_bytes(magic: &[u8; 4], page: &LogicalPage) -> Result<Vec<u8>, CoveError> {
    let row_count = u32::try_from(page.values.len()).map_err(|_| CoveError::ArithOverflow)?;
    let null_bitmap_len = page.values.len().div_ceil(8);
    let offsets_len = page
        .values
        .len()
        .checked_add(1)
        .and_then(|count| count.checked_mul(4))
        .ok_or(CoveError::ArithOverflow)?;
    let mut payload = Vec::new();
    let mut offsets = Vec::with_capacity(page.values.len() + 1);
    offsets.push(0u32);
    for value in &page.values {
        let current_offset = offsets.last().copied().ok_or(CoveError::ArithOverflow)?;
        if let Some(bytes) = value {
            let next = current_offset
                .checked_add(u32::try_from(bytes.len()).map_err(|_| CoveError::ArithOverflow)?)
                .ok_or(CoveError::ArithOverflow)?;
            offsets.push(next);
            payload.extend_from_slice(bytes);
        } else {
            offsets.push(current_offset);
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(magic);
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&(null_bitmap_len as u32).to_le_bytes());
    out.extend_from_slice(&(offsets_len as u32).to_le_bytes());
    let mut null_bitmap = vec![0u8; null_bitmap_len];
    for (index, value) in page.values.iter().enumerate() {
        if value.is_none() {
            null_bitmap[index >> 3] |= 1 << (index & 7);
        }
    }
    out.extend_from_slice(&null_bitmap);
    for offset in offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&payload);
    Ok(out)
}

fn decode_row_bytes(expected_magic: &[u8; 4], bytes: &[u8]) -> Result<LogicalPage, CoveError> {
    if bytes.len() < 16 || &bytes[0..4] != expected_magic {
        return Err(CoveError::BadCodecExtension);
    }
    let row_count = wire::read_u32_le_checked(bytes, 4)? as usize;
    let null_bitmap_len = wire::read_u32_le_checked(bytes, 8)? as usize;
    let offsets_len = wire::read_u32_le_checked(bytes, 12)? as usize;
    if null_bitmap_len != row_count.div_ceil(8) || offsets_len != (row_count + 1) * 4 {
        return Err(CoveError::BadCodecExtension);
    }
    let bitmap_start = 16usize;
    let offsets_start = bitmap_start
        .checked_add(null_bitmap_len)
        .ok_or(CoveError::ArithOverflow)?;
    let payload_start = offsets_start
        .checked_add(offsets_len)
        .ok_or(CoveError::ArithOverflow)?;
    if payload_start > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let null_bitmap = &bytes[bitmap_start..offsets_start];
    let mut offsets = Vec::with_capacity(row_count + 1);
    for chunk in bytes[offsets_start..payload_start].chunks_exact(4) {
        offsets.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize);
    }
    if offsets.first() != Some(&0) {
        return Err(CoveError::BadCodecExtension);
    }
    let payload = &bytes[payload_start..];
    let mut values = Vec::with_capacity(row_count);
    for index in 0..row_count {
        let start = offsets[index];
        let end = offsets[index + 1];
        if start > end || end > payload.len() {
            return Err(CoveError::BadCodecExtension);
        }
        let is_null = (null_bitmap[index >> 3] & (1 << (index & 7))) != 0;
        values.push((!is_null).then(|| payload[start..end].to_vec()));
    }
    if offsets.last() != Some(&payload.len()) {
        return Err(CoveError::BadCodecExtension);
    }
    if !row_count.is_multiple_of(8) && !null_bitmap.is_empty() {
        let unused_mask = !((1u8 << (row_count % 8)) - 1);
        if null_bitmap[null_bitmap.len() - 1] & unused_mask != 0 {
            return Err(CoveError::BadCodecExtension);
        }
    }
    Ok(LogicalPage { values })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(codec_id: u32) -> CodecExtensionDescriptorV2 {
        CodecExtensionDescriptorV2 {
            codec_id,
            namespace: "org.cove".into(),
            name: format!("codec-{codec_id}"),
            version_major: 1,
            version_minor: 0,
            codec_family: 3,
            logical_type_mask: 1,
            physical_kind_mask: 1,
            requirement: CodecRequirementV2::OptionalWithFallback,
            fallback_policy: CodecFallbackPolicyV2::CoreEncodingPayloadPresent,
            parameter_schema_kind: 0,
            flags: 0,
            specification_status: CodecSpecificationStatusV2::Candidate,
            required_feature_bit: 0,
            optional_feature_bit: 0,
            spec_digest_algorithm: 1,
            spec_digest: vec![1, 2, 3, 4],
            conformance_vector_ref: ABSENT_REF,
            fallback_ref: 42,
            private_payload_ref: ABSENT_REF,
            checksum: 0,
        }
    }

    #[test]
    fn descriptor_round_trips_and_validates_set() {
        let first = descriptor(1).serialize().unwrap();
        let second = descriptor(2).serialize().unwrap();
        let mut bytes = first;
        bytes.extend_from_slice(&second);
        let parsed = CodecExtensionDescriptorV2::parse_many(&bytes).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].namespace, "org.cove");
    }

    #[test]
    fn descriptor_rejects_bad_checksum() {
        let mut bytes = descriptor(1).serialize().unwrap();
        bytes[6] ^= 1;
        assert!(matches!(
            CodecExtensionDescriptorV2::parse_one(&bytes),
            Err(CoveError::ChecksumMismatch)
        ));
    }

    #[test]
    fn candidate_required_without_fallback_is_rejected() {
        let mut item = descriptor(1);
        item.requirement = CodecRequirementV2::RequiredForDecode;
        item.fallback_policy = CodecFallbackPolicyV2::NoFallback;
        item.fallback_ref = ABSENT_REF;
        assert!(matches!(
            item.serialize(),
            Err(CoveError::BadCodecExtension)
        ));
    }

    #[test]
    fn stable_registry_rejects_wrong_codec_family() {
        let mut descriptor = StableRegisteredCodec::FsstUtf8.descriptor();
        descriptor.codec_family = StableRegisteredCodec::AlpFloat.codec_family();
        assert!(CodecRegistry::stable_v2()
            .codec_for_descriptor(&descriptor)
            .is_none());
        assert!(matches!(
            descriptor.validate(),
            Err(CoveError::BadCodecExtension)
        ));
    }

    #[test]
    fn stable_descriptors_advertise_registered_type_domains() {
        for codec in [
            StableRegisteredCodec::FsstUtf8,
            StableRegisteredCodec::AlpFloat,
            StableRegisteredCodec::FastLanesInteger,
        ] {
            let descriptor = codec.descriptor();
            let (namespace, name, version_major, version_minor) = codec.descriptor_identity();
            assert_eq!(
                (descriptor.logical_type_mask, descriptor.physical_kind_mask),
                stable_registered_codec_supported_masks(
                    namespace,
                    name,
                    version_major,
                    version_minor
                )
                .expect("stable codec identity is registered in cove-core")
            );
        }

        let fsst = StableRegisteredCodec::FsstUtf8.descriptor();
        assert_eq!(
            fsst.logical_type_mask,
            1u64 << (cove_core::constants::CoveLogicalType::Utf8 as u32)
        );
        assert_eq!(
            fsst.physical_kind_mask,
            1u64 << (cove_core::constants::CovePhysicalKind::VarBytes as u32)
        );

        let alp = StableRegisteredCodec::AlpFloat.descriptor();
        assert_ne!(
            alp.logical_type_mask & (1u64 << (cove_core::constants::CoveLogicalType::Int64 as u32)),
            1u64 << (cove_core::constants::CoveLogicalType::Int64 as u32)
        );
        assert_eq!(
            alp.physical_kind_mask,
            1u64 << (cove_core::constants::CovePhysicalKind::NumCode as u32)
        );
    }

    fn envelope() -> RegisteredEncodingEnvelopeV2 {
        RegisteredEncodingEnvelopeV2 {
            codec_id: 1,
            codec_version_major: 1,
            codec_version_minor: 0,
            logical_len: 4,
            non_null_count: 3,
            params_offset: 72,
            params_length: 8,
            encoded_payload_offset: 80,
            encoded_payload_length: 32,
            fallback_payload_offset: 112,
            fallback_payload_length: 16,
            decoded_uncompressed_length: 64,
            flags: 0,
            checksum: 0,
        }
    }

    #[test]
    fn registered_encoding_envelope_round_trips() {
        let first = envelope().serialize().unwrap();
        let mut second_item = envelope();
        second_item.codec_id = 2;
        let second = second_item.serialize().unwrap();
        let mut bytes = first.to_vec();
        bytes.extend_from_slice(&second);
        let parsed = RegisteredEncodingEnvelopeV2::parse_many(&bytes).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].decoded_uncompressed_length, 64);
    }

    #[test]
    fn registered_encoding_envelope_rejects_malformed_fallback() {
        let mut item = envelope();
        item.fallback_payload_offset = 0;
        item.fallback_payload_length = 16;
        assert!(matches!(
            item.serialize(),
            Err(CoveError::BadCodecExtension)
        ));
    }

    #[test]
    fn registered_encoding_envelope_rejects_bad_checksum() {
        let mut bytes = envelope().serialize().unwrap();
        bytes[8] ^= 1;
        assert!(matches!(
            RegisteredEncodingEnvelopeV2::parse(&bytes),
            Err(CoveError::ChecksumMismatch)
        ));
    }

    #[test]
    fn stable_payload_rejects_trailing_bytes() {
        let page = LogicalPage {
            values: vec![Some(b"alpha".to_vec()), None, Some(b"omega".to_vec())],
        };
        let mut bytes = StableRegisteredCodec::FsstUtf8.encode(&page).unwrap();
        bytes.push(0);
        assert!(matches!(
            StableRegisteredCodec::FsstUtf8.decode(&bytes),
            Err(CoveError::BadCodecExtension)
        ));
    }
}
