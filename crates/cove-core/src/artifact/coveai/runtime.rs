use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiWritableSection {
    pub section_id: u32,
    pub section_kind: u32,
    pub profile_kind: u8,
    pub payload_encoding: AiPayloadEncodingV1,
    pub requiredness_scope: AiRequirednessScopeV1,
    pub source_binding_ref: u32,
    pub required_ai_features: u64,
    pub optional_ai_features: u64,
    pub feature_binding_ref: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveVecFileCodeVectorBuild {
    pub artifact_id: [u8; 16],
    pub created_at_us: i64,
    pub dimension_count: u32,
    pub file_codes: Vec<u32>,
    pub vector_payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoveVecFileCodeVectorBuildOptions {
    pub index_kind: Option<u8>,
    pub metric: u8,
    pub quantization_kind: u8,
}

impl Default for CoveVecFileCodeVectorBuildOptions {
    fn default() -> Self {
        Self {
            index_kind: None,
            metric: 1,
            quantization_kind: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiDescriptorBundleBuild {
    pub artifact_id: [u8; 16],
    pub created_at_us: i64,
    pub payload_sections: Vec<CoveAiWritableSection>,
    pub descriptor_tables: AiDescriptorTablesV1,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExactFlatFileCodeVectorSearchResult {
    pub file_code: u32,
    pub vector_ref: u64,
    pub vector_space_id: u32,
    /// Larger is better. For distance metrics this is the negative distance.
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileCodeEmbeddingV1 {
    pub file_code: u32,
    pub vector_ref: u64,
    pub vector_space_id: u32,
    pub dimension_count: u32,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiAccessContext {
    pub operation: String,
    pub allow_payloads: bool,
    pub require_privacy_summary: bool,
    pub allow_external_payloads: bool,
    pub visibility_scope_ref: u32,
    pub redaction_scope_ref: u32,
}

impl CoveAiAccessContext {
    pub fn for_operation(operation: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            allow_payloads: true,
            require_privacy_summary: true,
            allow_external_payloads: false,
            visibility_scope_ref: 0,
            redaction_scope_ref: 0,
        }
    }

    pub fn descriptor_only(operation: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            allow_payloads: false,
            require_privacy_summary: true,
            allow_external_payloads: false,
            visibility_scope_ref: 0,
            redaction_scope_ref: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiDisclosureDecision {
    Allowed,
    PayloadAccessDisabled,
    MissingPrivacySummary,
    PolicyProtected,
    Revoked,
    ExternalPayloadBlocked,
}

impl AiDisclosureDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::PayloadAccessDisabled => "payload_access_disabled",
            Self::MissingPrivacySummary => "missing_privacy_summary",
            Self::PolicyProtected => "policy_protected",
            Self::Revoked => "revoked",
            Self::ExternalPayloadBlocked => "external_payload_blocked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveAiPayloadLease<'a> {
    pub payload_ref: u32,
    pub media_type_ref: u32,
    pub decoded_length: u64,
    pub integrity_ref: u32,
    pub disclosure: AiDisclosureDecision,
    pub bytes: &'a [u8],
}

pub struct AiPayloadReader<'a> {
    artifact_bytes: &'a [u8],
    sidecar: &'a CoveAiFile,
    context: CoveAiAccessContext,
}

impl<'a> AiPayloadReader<'a> {
    pub fn new(
        artifact_bytes: &'a [u8],
        sidecar: &'a CoveAiFile,
        context: CoveAiAccessContext,
    ) -> Self {
        Self {
            artifact_bytes,
            sidecar,
            context,
        }
    }

    pub fn disclosure_for_payload_ref(
        &self,
        payload_ref: u32,
    ) -> Result<AiDisclosureDecision, CoveError> {
        if !self.context.allow_payloads {
            return Ok(AiDisclosureDecision::PayloadAccessDisabled);
        }
        if self.context.require_privacy_summary
            && self.sidecar.payload_access
                == AiPayloadAccessState::PolicyBlockedMissingPrivacySummary
        {
            return Ok(AiDisclosureDecision::MissingPrivacySummary);
        }
        let payload_ref = self
            .sidecar
            .descriptor_tables
            .payload_ref(payload_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!("AI payload_ref {payload_ref} is missing"))
            })?;
        if payload_ref.flags & AI_FLAG_REVOKED != 0 {
            return Ok(AiDisclosureDecision::Revoked);
        }
        if payload_ref.flags & AI_FLAG_POLICY_PROTECTED != 0 {
            return Ok(AiDisclosureDecision::PolicyProtected);
        }
        if payload_ref.storage_kind == AiStorageKindV1::ExternalUri as u8
            && !self.context.allow_external_payloads
        {
            return Ok(AiDisclosureDecision::ExternalPayloadBlocked);
        }
        Ok(AiDisclosureDecision::Allowed)
    }

    pub fn lease_payload_ref(&self, payload_ref: u32) -> Result<CoveAiPayloadLease<'a>, CoveError> {
        let disclosure = self.disclosure_for_payload_ref(payload_ref)?;
        if disclosure != AiDisclosureDecision::Allowed {
            return Err(CoveError::BadSection(format!(
                "AI payload_ref {payload_ref} disclosure is {}",
                disclosure.as_str()
            )));
        }
        let payload = self
            .sidecar
            .descriptor_tables
            .payload_ref(payload_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!("AI payload_ref {payload_ref} is missing"))
            })?;
        if payload.integrity_ref != 0 {
            exact_flat_verify_payload_integrity(
                self.artifact_bytes,
                self.sidecar,
                payload.integrity_ref,
            )?;
        }
        let bytes = exact_flat_payload_ref_bytes(self.artifact_bytes, self.sidecar, payload)?;
        Ok(CoveAiPayloadLease {
            payload_ref: payload.payload_ref,
            media_type_ref: payload.media_type_ref,
            decoded_length: payload.decoded_length,
            integrity_ref: payload.integrity_ref,
            disclosure,
            bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiEmbeddingRequest {
    pub file_code: Option<u32>,
    pub vector_ref: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiEmbeddingResult {
    pub target_kind: String,
    pub file_code: Option<u32>,
    pub vector_ref: u64,
    pub vector_space_id: u32,
    pub dimension_count: u32,
    pub element_type: u8,
    pub values: Vec<f32>,
    pub result_authority: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiVectorSearchTargetKind {
    All,
    FileCode,
    Chunk,
    ObjectState,
    AssociationState,
    TrainingSample,
    Asset,
    MultimodalSequence,
}

impl AiVectorSearchTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::FileCode => "file_code",
            Self::Chunk => "chunk",
            Self::ObjectState => "object_state",
            Self::AssociationState => "association_state",
            Self::TrainingSample => "training_sample",
            Self::Asset => "asset",
            Self::MultimodalSequence => "multimodal_sequence",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiVectorIndexSelection {
    Auto,
    ExactFlat,
    Hnsw,
    IvfFlat,
    IvfPq,
    DiskAnn,
    Vamana,
}

impl AiVectorIndexSelection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ExactFlat => "exact_flat",
            Self::Hnsw => "hnsw",
            Self::IvfFlat => "ivf_flat",
            Self::IvfPq => "ivf_pq",
            Self::DiskAnn => "diskann",
            Self::Vamana => "vamana",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiVectorSearchPlan {
    pub query_file_code: Option<u32>,
    pub query_vector_ref: Option<u64>,
    pub query_values: Option<Vec<f32>>,
    pub top_k: usize,
    pub target_kind: AiVectorSearchTargetKind,
    pub index: AiVectorIndexSelection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiVectorSearchResult {
    pub target_kind: String,
    pub binding_id: Option<u64>,
    pub file_code: Option<u32>,
    pub chunk_id: Option<u64>,
    pub object_type_id: Option<u32>,
    pub association_type_id: Option<u32>,
    pub sample_id: Option<u64>,
    pub asset_ref: Option<u64>,
    pub multimodal_sequence_pack_id: Option<u64>,
    pub vector_ref: u64,
    pub vector_space_id: u32,
    /// Larger is better. For distance metrics this is the negative distance.
    pub score: f32,
    pub exact: bool,
    pub selected_index: String,
    pub fallback_used: bool,
    pub result_authority: String,
}

pub(super) struct SelectedAiVectorIndex {
    name: String,
    index_kind: u8,
    fallback_used: bool,
    exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExportOptions {
    pub include_payloads: bool,
    pub format: String,
    pub profile_filter: Option<u32>,
    pub split_filter: Option<u32>,
    pub epoch_plan_filter: Option<u64>,
    pub policy_report: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExportReport {
    pub records_considered: usize,
    pub records_exported: usize,
    pub payloads_read: usize,
    pub payload_bytes_read: u64,
    pub policy_withheld: usize,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExplainReport {
    pub artifact_kind: CoveAiArtifactKind,
    pub artifact_id: [u8; 16],
    pub payload_access: AiPayloadAccessState,
    pub vector_space_count: usize,
    pub vector_index_count: usize,
    pub payload_ref_count: usize,
    pub privacy_summary_count: usize,
    pub stale_or_withheld: Vec<String>,
    pub supported_indexes: Vec<String>,
    pub vector_spaces: Vec<AiExplainVectorSpace>,
    pub vector_indexes: Vec<AiExplainVectorIndex>,
    pub fallback_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExplainVectorSpace {
    pub vector_space_id: u32,
    pub dimension_count: u32,
    pub element_type: u8,
    pub metric: u8,
    pub normalization_policy: u8,
    pub quantization_policy: u8,
    pub deterministic: u8,
    pub approximate: u8,
    pub reproducibility_class: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExplainVectorIndex {
    pub vector_index_id: u32,
    pub vector_space_id: u32,
    pub index_kind: String,
    pub exactness_kind: u8,
    pub false_negative_policy: u8,
    pub metric: u8,
    pub dimension_count: u32,
    pub indexed_binding_kind: u8,
    pub result_authority: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiDlpackDType {
    pub code: u8,
    pub bits: u8,
    pub lanes: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiDlpackDevice {
    pub device_type: u8,
    pub device_id: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiTensorZeroCopyView<'a> {
    pub tensor_layout_id: u32,
    pub payload_ref: u32,
    pub dtype: u8,
    pub dlpack_dtype: AiDlpackDType,
    pub dlpack_device: AiDlpackDevice,
    pub shape: Vec<i64>,
    pub strides: Option<Vec<i64>>,
    pub byte_offset: usize,
    pub data: &'a [u8],
    pub result_authority: &'static str,
}

pub fn ai_tensor_zero_copy_view<'a>(
    artifact_bytes: &'a [u8],
    tensor_layout_id: u32,
    payload_ref: u32,
) -> Result<AiTensorZeroCopyView<'a>, CoveError> {
    let sidecar = parse_ai_sidecar_for_operation_with_payload_access(
        artifact_bytes,
        OperationKindV2::AiMultimodalSequenceRead,
    )?;
    let layout = sidecar
        .descriptor_tables
        .tensor_layouts
        .iter()
        .find(|layout| layout.tensor_layout_id == tensor_layout_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!("AI tensor layout {tensor_layout_id} is missing"))
        })?;
    let access_context = CoveAiAccessContext::for_operation("tensor_zero_copy");
    let payload_reader = AiPayloadReader::new(artifact_bytes, &sidecar, access_context);
    let shape = ai_tensor_i64_vector_payload(
        &payload_reader,
        layout.shape_ref,
        usize::from(layout.rank),
        "tensor shape_ref",
    )?;
    let strides = if layout.stride_ref == 0 {
        None
    } else {
        Some(ai_tensor_i64_vector_payload(
            &payload_reader,
            layout.stride_ref,
            usize::from(layout.rank),
            "tensor stride_ref",
        )?)
    };
    payload_reader.lease_payload_ref(payload_ref)?;
    let data_payload = sidecar
        .descriptor_tables
        .payload_ref(payload_ref)
        .ok_or_else(|| CoveError::BadSection(format!("AI payload_ref {payload_ref} is missing")))?;
    let data = exact_flat_payload_ref_bytes(artifact_bytes, &sidecar, data_payload)?;
    ai_validate_tensor_zero_copy_layout(layout, &shape, strides.as_deref(), data)?;
    let dtype = ai_tensor_dlpack_dtype(layout.dtype)?;
    let byte_offset = ai_tensor_byte_offset(layout)?;
    Ok(AiTensorZeroCopyView {
        tensor_layout_id,
        payload_ref,
        dtype: layout.dtype,
        dlpack_dtype: dtype,
        dlpack_device: AiDlpackDevice {
            device_type: layout.device_affinity_hint,
            device_id: 0,
        },
        shape,
        strides,
        byte_offset,
        data,
        result_authority: "ValidatedAiPayloadLeaseTensorZeroCopy",
    })
}

pub fn ai_explain_report(sidecar: &CoveAiFile) -> AiExplainReport {
    let mut stale_or_withheld = Vec::new();
    let mut fallback_actions = Vec::new();
    if sidecar.payload_access == AiPayloadAccessState::PolicyBlockedMissingPrivacySummary {
        stale_or_withheld.push("payload_access_blocked_missing_privacy_summary".to_string());
    }
    let supported_indexes = sidecar
        .descriptor_tables
        .vector_indexes
        .iter()
        .map(|index| vector_index_kind_name(index.index_kind).to_string())
        .collect();
    let vector_spaces = sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .map(|space| AiExplainVectorSpace {
            vector_space_id: space.vector_space_id,
            dimension_count: space.dimension_count,
            element_type: space.element_type,
            metric: space.metric,
            normalization_policy: space.normalization_policy,
            quantization_policy: space.quantization_policy,
            deterministic: space.deterministic,
            approximate: space.approximate,
            reproducibility_class: space.reproducibility_class,
        })
        .collect::<Vec<_>>();
    let vector_indexes = sidecar
        .descriptor_tables
        .vector_indexes
        .iter()
        .map(|index| {
            let (result_authority, fallback_action) =
                if index.exactness_kind == 0 && index.false_negative_policy == 0 {
                    ("ExactOptimizedKernel", None)
                } else if index.index_kind == 0 {
                    (
                        "ExactFlatFallback",
                        Some(format!(
                            "{}_candidate_metadata_exact_flat_fallback",
                            vector_index_kind_name(index.index_kind)
                        )),
                    )
                } else {
                    ("ApproximateInternalAnn", None)
                };
            if let Some(action) = fallback_action {
                fallback_actions.push(action);
            }
            AiExplainVectorIndex {
                vector_index_id: index.vector_index_id,
                vector_space_id: index.vector_space_id,
                index_kind: vector_index_kind_name(index.index_kind).to_string(),
                exactness_kind: index.exactness_kind,
                false_negative_policy: index.false_negative_policy,
                metric: index.metric,
                dimension_count: index.dimension_count,
                indexed_binding_kind: index.indexed_binding_kind,
                result_authority: result_authority.to_string(),
            }
        })
        .collect::<Vec<_>>();
    AiExplainReport {
        artifact_kind: sidecar.artifact_kind,
        artifact_id: sidecar.header.artifact_id,
        payload_access: sidecar.payload_access,
        vector_space_count: sidecar.descriptor_tables.vector_spaces.len(),
        vector_index_count: sidecar.descriptor_tables.vector_indexes.len(),
        payload_ref_count: sidecar.descriptor_tables.payload_refs.len(),
        privacy_summary_count: sidecar.descriptor_tables.privacy_summaries.len(),
        stale_or_withheld,
        supported_indexes,
        vector_spaces,
        vector_indexes,
        fallback_actions,
    }
}

pub(super) fn ai_tensor_i64_vector_payload(
    payload_reader: &AiPayloadReader<'_>,
    payload_ref: u32,
    expected_len: usize,
    label: &str,
) -> Result<Vec<i64>, CoveError> {
    if payload_ref == 0 {
        return Err(CoveError::BadSection(format!(
            "{label} is required for tensor zero-copy"
        )));
    }
    let lease = payload_reader.lease_payload_ref(payload_ref)?;
    let expected_bytes = expected_len
        .checked_mul(8)
        .ok_or(CoveError::ArithOverflow)?;
    if lease.bytes.len() != expected_bytes {
        return Err(CoveError::BadSection(format!(
            "{label} payload length {} does not match rank byte length {expected_bytes}",
            lease.bytes.len()
        )));
    }
    let mut values = Vec::with_capacity(expected_len);
    for chunk in lease.bytes.chunks_exact(8) {
        values.push(read_i64(chunk, 0)?);
    }
    Ok(values)
}

pub(super) fn ai_validate_tensor_zero_copy_layout(
    layout: &TensorLayoutDescriptorV1,
    shape: &[i64],
    strides: Option<&[i64]>,
    data: &[u8],
) -> Result<(), CoveError> {
    let rank = usize::from(layout.rank);
    if shape.len() != rank || strides.is_some_and(|strides| strides.len() != rank) {
        return Err(CoveError::BadSection(
            "AI tensor shape/stride rank mismatch".into(),
        ));
    }
    if layout.byte_order == 2 {
        return Err(CoveError::BadSection(
            "AI tensor zero-copy does not expose byte-swapped big-endian payloads".into(),
        ));
    }
    if layout.quantization_profile_ref != 0 || layout.sparsity_profile_ref != 0 {
        return Err(CoveError::BadSection(
            "AI tensor zero-copy requires uncompressed dense tensor payloads".into(),
        ));
    }
    let element_width = ai_tensor_element_width(layout.dtype)?;
    let strides = match strides {
        Some(strides) => strides.to_vec(),
        None => ai_contiguous_strides(shape)?,
    };
    let storage_offset =
        usize::try_from(layout.storage_offset_elements).map_err(|_| CoveError::ArithOverflow)?;
    let max_element_offset = ai_tensor_max_element_offset(shape, &strides, storage_offset)?;
    let required_bytes = max_element_offset
        .checked_add(1)
        .and_then(|elements| elements.checked_mul(element_width))
        .ok_or(CoveError::ArithOverflow)?;
    if required_bytes > data.len() {
        return Err(CoveError::BadSection(format!(
            "AI tensor payload length {} is shorter than required zero-copy byte length {required_bytes}",
            data.len()
        )));
    }
    let byte_offset = storage_offset
        .checked_mul(element_width)
        .ok_or(CoveError::ArithOverflow)?;
    if byte_offset > data.len() {
        return Err(CoveError::BadSection(
            "AI tensor storage offset exceeds payload length".into(),
        ));
    }
    if layout.memory_alignment_bytes > 1 {
        let alignment =
            usize::try_from(layout.memory_alignment_bytes).map_err(|_| CoveError::ArithOverflow)?;
        let address = data.as_ptr().wrapping_add(byte_offset) as usize;
        if !address.is_multiple_of(alignment) {
            return Err(CoveError::BadSection(format!(
                "AI tensor payload is not aligned to {} bytes",
                layout.memory_alignment_bytes
            )));
        }
    }
    Ok(())
}

pub(super) fn ai_tensor_byte_offset(layout: &TensorLayoutDescriptorV1) -> Result<usize, CoveError> {
    let element_width = ai_tensor_element_width(layout.dtype)?;
    usize::try_from(layout.storage_offset_elements)
        .map_err(|_| CoveError::ArithOverflow)?
        .checked_mul(element_width)
        .ok_or(CoveError::ArithOverflow)
}

pub(super) fn ai_contiguous_strides(shape: &[i64]) -> Result<Vec<i64>, CoveError> {
    let mut strides = vec![0; shape.len()];
    let mut stride = 1i64;
    for index in (0..shape.len()).rev() {
        let dim = shape[index];
        if dim <= 0 {
            return Err(CoveError::BadSection(
                "AI tensor shape dimensions must be positive".into(),
            ));
        }
        strides[index] = stride;
        stride = stride.checked_mul(dim).ok_or(CoveError::ArithOverflow)?;
    }
    Ok(strides)
}

pub(super) fn ai_tensor_max_element_offset(
    shape: &[i64],
    strides: &[i64],
    storage_offset: usize,
) -> Result<usize, CoveError> {
    let mut max_offset = storage_offset;
    for (dim, stride) in shape.iter().zip(strides) {
        if *dim <= 0 {
            return Err(CoveError::BadSection(
                "AI tensor shape dimensions must be positive".into(),
            ));
        }
        if *stride < 0 {
            return Err(CoveError::BadSection(
                "AI tensor zero-copy does not expose negative strides".into(),
            ));
        }
        let dim_span = usize::try_from(dim - 1).map_err(|_| CoveError::ArithOverflow)?;
        let stride = usize::try_from(*stride).map_err(|_| CoveError::ArithOverflow)?;
        max_offset = max_offset
            .checked_add(
                dim_span
                    .checked_mul(stride)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok(max_offset)
}

pub(super) fn ai_tensor_element_width(dtype: u8) -> Result<usize, CoveError> {
    match dtype {
        0 => Ok(4),  // Float32
        1 => Ok(8),  // Float64
        2 => Ok(2),  // Float16
        3 => Ok(2),  // BFloat16
        4 => Ok(1),  // Int8
        5 => Ok(1),  // UInt8
        6 => Ok(2),  // Int16
        7 => Ok(2),  // UInt16
        8 => Ok(4),  // Int32
        9 => Ok(4),  // UInt32
        10 => Ok(8), // Int64
        11 => Ok(8), // UInt64
        12 => Ok(1), // Bool
        13 => Ok(2), // Fixed16
        14 => Ok(4), // Fixed32
        15 => Ok(8), // Fixed64
        _ => Err(CoveError::BadSection(format!(
            "AI tensor dtype {dtype} is not supported for zero-copy"
        ))),
    }
}

pub(super) fn ai_tensor_dlpack_dtype(dtype: u8) -> Result<AiDlpackDType, CoveError> {
    let (code, bits) = match dtype {
        0 => (2, 32),
        1 => (2, 64),
        2 => (2, 16),
        3 => (4, 16),
        4 => (0, 8),
        5 => (1, 8),
        6 => (0, 16),
        7 => (1, 16),
        8 => (0, 32),
        9 => (1, 32),
        10 => (0, 64),
        11 => (1, 64),
        12 => (1, 1),
        13 => (0, 16),
        14 => (0, 32),
        15 => (0, 64),
        _ => {
            return Err(CoveError::BadSection(format!(
                "AI tensor dtype {dtype} is not supported for DLPack"
            )));
        }
    };
    Ok(AiDlpackDType {
        code,
        bits,
        lanes: 1,
    })
}

pub fn exact_flat_filecode_vector_search(
    artifact_bytes: &[u8],
    query: &[f32],
    top_k: usize,
) -> Result<Vec<ExactFlatFileCodeVectorSearchResult>, CoveError> {
    if query.is_empty() {
        return Err(CoveError::BadSection(
            "exact flat vector search requires a non-empty query vector".into(),
        ));
    }
    for value in query {
        if !value.is_finite() {
            return Err(CoveError::BadSection(
                "exact flat vector search query contains non-finite values".into(),
            ));
        }
    }

    let sidecar = exact_flat_parse_covev_with_payload_access(artifact_bytes)?;

    let query_dimension = u32::try_from(query.len()).map_err(|_| CoveError::ArithOverflow)?;
    let matching_spaces = sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .filter(|space| space.dimension_count == query_dimension)
        .collect::<Vec<_>>();
    let vector_space = match matching_spaces.as_slice() {
        [space] => *space,
        [] => {
            return Err(CoveError::BadSection(format!(
                "no COVE-VEC vector space matches query dimension {query_dimension}"
            )));
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "multiple COVE-VEC vector spaces match query dimension {query_dimension}; exact flat search is ambiguous"
            )));
        }
    };
    exact_flat_validate_vector_space(vector_space)?;

    exact_flat_filecode_vector_search_in_space(artifact_bytes, &sidecar, vector_space, query, top_k)
}

pub fn exact_flat_filecode_vector_search_by_file_code(
    artifact_bytes: &[u8],
    query_file_code: u32,
    top_k: usize,
) -> Result<Vec<ExactFlatFileCodeVectorSearchResult>, CoveError> {
    let sidecar = exact_flat_parse_covev_with_payload_access(artifact_bytes)?;
    let (_query_binding, vector_space, query_entry) =
        exact_flat_filecode_binding_parts(&sidecar, query_file_code)?;
    let query = exact_flat_vector_entry_f32(artifact_bytes, &sidecar, vector_space, query_entry)?;
    exact_flat_filecode_vector_search_in_space(
        artifact_bytes,
        &sidecar,
        vector_space,
        &query,
        top_k,
    )
}

pub fn filecode_embedding(
    artifact_bytes: &[u8],
    file_code: u32,
) -> Result<FileCodeEmbeddingV1, CoveError> {
    let sidecar = exact_flat_parse_covev_with_payload_access(artifact_bytes)?;
    let (binding, vector_space, vector_entry) =
        exact_flat_filecode_binding_parts(&sidecar, file_code)?;
    let values = exact_flat_vector_entry_f32(artifact_bytes, &sidecar, vector_space, vector_entry)?;
    Ok(FileCodeEmbeddingV1 {
        file_code,
        vector_ref: binding.vector_ref,
        vector_space_id: vector_space.vector_space_id,
        dimension_count: vector_space.dimension_count,
        values,
    })
}

pub fn ai_embedding(
    artifact_bytes: &[u8],
    request: &AiEmbeddingRequest,
) -> Result<AiEmbeddingResult, CoveError> {
    let sidecar = parse_ai_sidecar_for_operation_with_payload_access(
        artifact_bytes,
        OperationKindV2::AiEmbedding,
    )?;
    let (target_kind, file_code, vector_space, vector_entry) =
        resolve_embedding_request(&sidecar, request)?;
    let values = vector_entry_values_as_f32(artifact_bytes, &sidecar, vector_space, vector_entry)?;
    Ok(AiEmbeddingResult {
        target_kind: target_kind.to_string(),
        file_code,
        vector_ref: vector_entry.vector_ref,
        vector_space_id: vector_space.vector_space_id,
        dimension_count: vector_space.dimension_count,
        element_type: vector_space.element_type,
        values,
        result_authority: "PersistedPayloadDigest".to_string(),
    })
}

pub fn ai_vector_search(
    artifact_bytes: &[u8],
    plan: &AiVectorSearchPlan,
) -> Result<Vec<AiVectorSearchResult>, CoveError> {
    if plan.top_k == 0 {
        return Ok(Vec::new());
    }
    let sidecar = parse_ai_sidecar_for_operation_with_payload_access(
        artifact_bytes,
        OperationKindV2::AiSemanticSearch,
    )?;
    let (query_space, query_values) = resolve_vector_query(artifact_bytes, &sidecar, plan)?;
    let selected_index = select_vector_index(&sidecar, query_space, plan.index);
    let loaded_candidates =
        load_vector_search_candidates(artifact_bytes, &sidecar, query_space, plan.target_kind)?;
    let selected_candidate_indices = if selected_index.exact || selected_index.fallback_used {
        (0..loaded_candidates.len()).collect::<Vec<_>>()
    } else {
        ann_candidate_indices(
            &loaded_candidates,
            query_space.metric,
            &query_values,
            plan.top_k,
            selected_index.index_kind,
        )?
    };
    let mut results = Vec::new();

    for candidate_index in selected_candidate_indices {
        let loaded = loaded_candidates.get(candidate_index).ok_or_else(|| {
            CoveError::BadSection("AI ANN candidate index is out of range".into())
        })?;
        let score = exact_flat_metric_score(query_space.metric, &query_values, &loaded.values)?;
        let candidate = &loaded.candidate;
        results.push(AiVectorSearchResult {
            target_kind: candidate.target_kind.to_string(),
            binding_id: candidate.binding_id,
            file_code: candidate.file_code,
            chunk_id: candidate.chunk_id,
            object_type_id: candidate.object_type_id,
            association_type_id: candidate.association_type_id,
            sample_id: candidate.sample_id,
            asset_ref: candidate.asset_ref,
            multimodal_sequence_pack_id: candidate.multimodal_sequence_pack_id,
            vector_ref: candidate.vector_ref,
            vector_space_id: query_space.vector_space_id,
            score,
            exact: selected_index.exact,
            selected_index: selected_index.name.clone(),
            fallback_used: selected_index.fallback_used,
            result_authority: if selected_index.fallback_used {
                "ExactFlatFallback".to_string()
            } else if selected_index.exact {
                "ExactOptimizedKernel".to_string()
            } else {
                "ApproximateInternalAnn".to_string()
            },
        });
    }

    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.target_kind.cmp(&right.target_kind))
            .then_with(|| left.vector_ref.cmp(&right.vector_ref))
    });
    results.truncate(plan.top_k.min(results.len()));
    Ok(results)
}

pub(super) fn parse_ai_sidecar_for_operation_with_payload_access(
    artifact_bytes: &[u8],
    operation: OperationKindV2,
) -> Result<CoveAiFile, CoveError> {
    let sidecar = CoveAiFile::parse_for_operation(artifact_bytes, operation)?;
    if sidecar.payload_access != AiPayloadAccessState::StructurallyAllowed {
        return Err(CoveError::BadSection(
            "direct AI payload access is policy-blocked: missing AI_PRIVACY_SUMMARY".into(),
        ));
    }
    Ok(sidecar)
}

pub(super) fn resolve_embedding_request<'a>(
    sidecar: &'a CoveAiFile,
    request: &AiEmbeddingRequest,
) -> Result<
    (
        &'static str,
        Option<u32>,
        &'a VectorSpaceDescriptorV1,
        &'a VectorEntryV1,
    ),
    CoveError,
> {
    match (request.file_code, request.vector_ref) {
        (Some(file_code), None) => {
            let (binding, vector_space, vector_entry) =
                runtime_filecode_binding_parts(sidecar, file_code)?;
            Ok((
                AiVectorSearchTargetKind::FileCode.as_str(),
                Some(binding.file_code),
                vector_space,
                vector_entry,
            ))
        }
        (None, Some(vector_ref)) => {
            let vector_entry = vector_entry_by_ref(sidecar, vector_ref)?;
            let vector_space = vector_space_for_entry(sidecar, vector_entry)?;
            validate_runtime_vector_space(vector_space)?;
            Ok(("vector_ref", None, vector_space, vector_entry))
        }
        (Some(_), Some(_)) => Err(CoveError::BadSection(
            "AI embedding request must use either file_code or vector_ref, not both".into(),
        )),
        (None, None) => Err(CoveError::BadSection(
            "AI embedding request requires file_code or vector_ref".into(),
        )),
    }
}

pub(super) fn resolve_vector_query<'a>(
    artifact_bytes: &[u8],
    sidecar: &'a CoveAiFile,
    plan: &AiVectorSearchPlan,
) -> Result<(&'a VectorSpaceDescriptorV1, Vec<f32>), CoveError> {
    let provided = usize::from(plan.query_file_code.is_some())
        + usize::from(plan.query_vector_ref.is_some())
        + usize::from(plan.query_values.is_some());
    if provided != 1 {
        return Err(CoveError::BadSection(
            "AI vector search requires exactly one query: file_code, vector_ref, or query_values"
                .into(),
        ));
    }
    if let Some(file_code) = plan.query_file_code {
        let (_binding, vector_space, vector_entry) =
            runtime_filecode_binding_parts(sidecar, file_code)?;
        let values =
            vector_entry_values_as_f32(artifact_bytes, sidecar, vector_space, vector_entry)?;
        return Ok((vector_space, values));
    }
    if let Some(vector_ref) = plan.query_vector_ref {
        let vector_entry = vector_entry_by_ref(sidecar, vector_ref)?;
        let vector_space = vector_space_for_entry(sidecar, vector_entry)?;
        validate_runtime_vector_space(vector_space)?;
        let values =
            vector_entry_values_as_f32(artifact_bytes, sidecar, vector_space, vector_entry)?;
        return Ok((vector_space, values));
    }
    let query_values = plan.query_values.clone().unwrap_or_default();
    if query_values.is_empty() {
        return Err(CoveError::BadSection(
            "AI vector search query_values must be non-empty".into(),
        ));
    }
    for value in &query_values {
        if !value.is_finite() {
            return Err(CoveError::BadSection(
                "AI vector search query contains non-finite values".into(),
            ));
        }
    }
    let query_dimension =
        u32::try_from(query_values.len()).map_err(|_| CoveError::ArithOverflow)?;
    let matching_spaces = sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .filter(|space| space.dimension_count == query_dimension)
        .collect::<Vec<_>>();
    let vector_space = match matching_spaces.as_slice() {
        [space] => *space,
        [] => {
            return Err(CoveError::BadSection(format!(
                "no COVE-AI vector space matches query dimension {query_dimension}"
            )));
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "multiple COVE-AI vector spaces match query dimension {query_dimension}; vector search is ambiguous"
            )));
        }
    };
    validate_runtime_vector_space(vector_space)?;
    Ok((vector_space, query_values))
}

#[derive(Debug, Clone)]
pub(super) struct VectorSearchCandidate {
    target_kind: &'static str,
    binding_id: Option<u64>,
    file_code: Option<u32>,
    chunk_id: Option<u64>,
    object_type_id: Option<u32>,
    association_type_id: Option<u32>,
    sample_id: Option<u64>,
    asset_ref: Option<u64>,
    multimodal_sequence_pack_id: Option<u64>,
    vector_ref: u64,
}

#[derive(Debug, Clone)]
pub(super) struct LoadedVectorSearchCandidate {
    pub(super) candidate: VectorSearchCandidate,
    pub(super) values: Vec<f32>,
}

pub(super) fn vector_search_candidates(
    sidecar: &CoveAiFile,
    vector_space: &VectorSpaceDescriptorV1,
    target_kind: AiVectorSearchTargetKind,
) -> Vec<VectorSearchCandidate> {
    let mut out = Vec::new();
    if matches!(
        target_kind,
        AiVectorSearchTargetKind::All | AiVectorSearchTargetKind::FileCode
    ) {
        out.extend(
            sidecar
                .descriptor_tables
                .filecode_vector_bindings
                .iter()
                .filter(|binding| binding.vector_space_id == vector_space.vector_space_id)
                .map(|binding| VectorSearchCandidate {
                    target_kind: AiVectorSearchTargetKind::FileCode.as_str(),
                    binding_id: Some(binding.binding_id),
                    file_code: Some(binding.file_code),
                    chunk_id: None,
                    object_type_id: None,
                    association_type_id: None,
                    sample_id: None,
                    asset_ref: None,
                    multimodal_sequence_pack_id: None,
                    vector_ref: binding.vector_ref,
                }),
        );
    }
    if matches!(
        target_kind,
        AiVectorSearchTargetKind::All | AiVectorSearchTargetKind::Chunk
    ) {
        out.extend(
            sidecar
                .descriptor_tables
                .chunk_vector_bindings
                .iter()
                .filter(|binding| binding.vector_space_id == vector_space.vector_space_id)
                .map(|binding| VectorSearchCandidate {
                    target_kind: AiVectorSearchTargetKind::Chunk.as_str(),
                    binding_id: Some(binding.binding_id),
                    file_code: None,
                    chunk_id: Some(binding.chunk_id),
                    object_type_id: None,
                    association_type_id: None,
                    sample_id: None,
                    asset_ref: None,
                    multimodal_sequence_pack_id: None,
                    vector_ref: binding.vector_ref,
                }),
        );
    }
    if matches!(
        target_kind,
        AiVectorSearchTargetKind::All | AiVectorSearchTargetKind::ObjectState
    ) {
        out.extend(
            sidecar
                .descriptor_tables
                .object_state_vector_bindings
                .iter()
                .filter(|binding| binding.vector_space_id == vector_space.vector_space_id)
                .map(|binding| VectorSearchCandidate {
                    target_kind: AiVectorSearchTargetKind::ObjectState.as_str(),
                    binding_id: Some(binding.binding_id),
                    file_code: None,
                    chunk_id: None,
                    object_type_id: Some(binding.object_type_id),
                    association_type_id: None,
                    sample_id: None,
                    asset_ref: None,
                    multimodal_sequence_pack_id: None,
                    vector_ref: binding.vector_ref,
                }),
        );
    }
    if matches!(
        target_kind,
        AiVectorSearchTargetKind::All | AiVectorSearchTargetKind::AssociationState
    ) {
        out.extend(
            sidecar
                .descriptor_tables
                .association_state_vector_bindings
                .iter()
                .filter(|binding| binding.vector_space_id == vector_space.vector_space_id)
                .map(|binding| VectorSearchCandidate {
                    target_kind: AiVectorSearchTargetKind::AssociationState.as_str(),
                    binding_id: Some(binding.binding_id),
                    file_code: None,
                    chunk_id: None,
                    object_type_id: None,
                    association_type_id: Some(binding.association_type_id),
                    sample_id: None,
                    asset_ref: None,
                    multimodal_sequence_pack_id: None,
                    vector_ref: binding.vector_ref,
                }),
        );
    }
    if matches!(
        target_kind,
        AiVectorSearchTargetKind::All | AiVectorSearchTargetKind::TrainingSample
    ) {
        out.extend(
            sidecar
                .descriptor_tables
                .training_sample_vector_bindings
                .iter()
                .filter(|binding| binding.vector_space_id == vector_space.vector_space_id)
                .map(|binding| VectorSearchCandidate {
                    target_kind: AiVectorSearchTargetKind::TrainingSample.as_str(),
                    binding_id: Some(binding.binding_id),
                    file_code: None,
                    chunk_id: None,
                    object_type_id: None,
                    association_type_id: None,
                    sample_id: Some(binding.sample_id),
                    asset_ref: None,
                    multimodal_sequence_pack_id: None,
                    vector_ref: binding.vector_ref,
                }),
        );
    }
    if matches!(
        target_kind,
        AiVectorSearchTargetKind::All | AiVectorSearchTargetKind::Asset
    ) {
        out.extend(
            sidecar
                .descriptor_tables
                .asset_vector_bindings
                .iter()
                .filter(|binding| binding.vector_space_id == vector_space.vector_space_id)
                .map(|binding| VectorSearchCandidate {
                    target_kind: AiVectorSearchTargetKind::Asset.as_str(),
                    binding_id: Some(binding.binding_id),
                    file_code: None,
                    chunk_id: None,
                    object_type_id: None,
                    association_type_id: None,
                    sample_id: None,
                    asset_ref: Some(binding.asset_ref),
                    multimodal_sequence_pack_id: None,
                    vector_ref: binding.vector_ref,
                }),
        );
    }
    if matches!(
        target_kind,
        AiVectorSearchTargetKind::All | AiVectorSearchTargetKind::MultimodalSequence
    ) {
        out.extend(
            sidecar
                .descriptor_tables
                .multimodal_sequence_vector_bindings
                .iter()
                .filter(|binding| binding.vector_space_id == vector_space.vector_space_id)
                .map(|binding| VectorSearchCandidate {
                    target_kind: AiVectorSearchTargetKind::MultimodalSequence.as_str(),
                    binding_id: Some(binding.binding_id),
                    file_code: None,
                    chunk_id: None,
                    object_type_id: None,
                    association_type_id: None,
                    sample_id: None,
                    asset_ref: None,
                    multimodal_sequence_pack_id: Some(binding.sequence_pack_id),
                    vector_ref: binding.vector_ref,
                }),
        );
    }
    out
}

pub(super) fn load_vector_search_candidates(
    artifact_bytes: &[u8],
    sidecar: &CoveAiFile,
    vector_space: &VectorSpaceDescriptorV1,
    target_kind: AiVectorSearchTargetKind,
) -> Result<Vec<LoadedVectorSearchCandidate>, CoveError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for candidate in vector_search_candidates(sidecar, vector_space, target_kind) {
        if !seen.insert((candidate.vector_ref, candidate.target_kind.to_string())) {
            continue;
        }
        let vector_entry = sidecar
            .descriptor_tables
            .vector_entries
            .iter()
            .find(|entry| entry.vector_ref == candidate.vector_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "AI vector binding references missing vector_ref {}",
                    candidate.vector_ref
                ))
            })?;
        let values =
            vector_entry_values_as_f32(artifact_bytes, sidecar, vector_space, vector_entry)?;
        out.push(LoadedVectorSearchCandidate { candidate, values });
    }
    Ok(out)
}

pub(super) fn ann_candidate_indices(
    candidates: &[LoadedVectorSearchCandidate],
    metric: u8,
    query: &[f32],
    top_k: usize,
    index_kind: u8,
) -> Result<Vec<usize>, CoveError> {
    if candidates.is_empty() || top_k == 0 {
        return Ok(Vec::new());
    }
    let limit = ann_candidate_limit(top_k, candidates.len());
    match index_kind {
        1 => graph_ann_candidate_indices(candidates, metric, query, top_k, 16, limit, 4),
        2 => ivf_flat_candidate_indices(candidates, metric, query, top_k, limit),
        3 => ivf_pq_candidate_indices(candidates, metric, query, top_k, limit),
        4 => graph_ann_candidate_indices(candidates, metric, query, top_k, 32, limit, 8),
        5 => graph_ann_candidate_indices(candidates, metric, query, top_k, 24, limit, 6),
        _ => Ok((0..candidates.len()).collect()),
    }
}

pub(super) fn ann_candidate_limit(top_k: usize, candidate_count: usize) -> usize {
    candidate_count.min(top_k.saturating_mul(8).max(32).max(top_k))
}

pub(super) fn graph_ann_candidate_indices(
    candidates: &[LoadedVectorSearchCandidate],
    metric: u8,
    query: &[f32],
    top_k: usize,
    degree: usize,
    limit: usize,
    entry_count: usize,
) -> Result<Vec<usize>, CoveError> {
    if candidates.len() <= limit {
        return Ok((0..candidates.len()).collect());
    }
    let query_scores = ann_query_scores(candidates, metric, query)?;
    let graph = ann_neighbor_graph(candidates, metric, degree)?;
    let mut entries = sampled_entry_points(&query_scores, entry_count.max(1));
    if entries.is_empty() {
        entries.push(0);
    }

    let ef = candidates
        .len()
        .min(limit.saturating_mul(2).max(top_k).max(degree));
    let mut visited = BTreeSet::new();
    let mut frontier = Vec::new();
    for entry in entries {
        if visited.insert(entry) {
            frontier.push(entry);
        }
    }
    let mut reached = Vec::new();
    while !frontier.is_empty() && reached.len() < ef {
        let best_pos = frontier
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| {
                query_scores[**left]
                    .total_cmp(&query_scores[**right])
                    .then_with(|| right.cmp(left))
            })
            .map(|(pos, _)| pos)
            .unwrap_or(0);
        let current = frontier.swap_remove(best_pos);
        reached.push(current);
        for neighbor in &graph[current] {
            if visited.insert(*neighbor) {
                frontier.push(*neighbor);
            }
        }
    }
    if reached.len() < top_k {
        widen_ann_candidates(&mut reached, &query_scores, top_k);
    }
    sort_and_truncate_indices(&mut reached, &query_scores, limit);
    Ok(reached)
}

pub(super) fn ann_neighbor_graph(
    candidates: &[LoadedVectorSearchCandidate],
    metric: u8,
    degree: usize,
) -> Result<Vec<Vec<usize>>, CoveError> {
    let degree = degree.max(1).min(candidates.len().saturating_sub(1).max(1));
    let mut graph = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        let mut neighbors = Vec::with_capacity(candidates.len().saturating_sub(1));
        for (other_index, other) in candidates.iter().enumerate() {
            if index == other_index {
                continue;
            }
            let score = exact_flat_metric_score(metric, &candidate.values, &other.values)?;
            neighbors.push((other_index, score));
        }
        neighbors.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        graph.push(
            neighbors
                .into_iter()
                .take(degree)
                .map(|(neighbor, _)| neighbor)
                .collect(),
        );
    }
    Ok(graph)
}

pub(super) fn sampled_entry_points(query_scores: &[f32], entry_count: usize) -> Vec<usize> {
    let mut sampled = Vec::new();
    let stride = query_scores.len().saturating_div(entry_count.max(1)).max(1);
    let mut index = 0usize;
    while index < query_scores.len() {
        sampled.push((index, query_scores[index]));
        index = index.saturating_add(stride);
    }
    if sampled.last().map(|(idx, _)| *idx) != Some(query_scores.len().saturating_sub(1)) {
        let last = query_scores.len().saturating_sub(1);
        sampled.push((last, query_scores[last]));
    }
    sampled.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    sampled
        .into_iter()
        .take(entry_count)
        .map(|(index, _)| index)
        .collect()
}

pub(super) fn ivf_flat_candidate_indices(
    candidates: &[LoadedVectorSearchCandidate],
    metric: u8,
    query: &[f32],
    top_k: usize,
    limit: usize,
) -> Result<Vec<usize>, CoveError> {
    if candidates.len() <= limit {
        return Ok((0..candidates.len()).collect());
    }
    let (centroids, clusters) = ivf_build_clusters(candidates, metric)?;
    let mut centroid_scores = centroids
        .iter()
        .enumerate()
        .map(|(index, centroid)| (index, ann_metric_score(metric, query, centroid)))
        .collect::<Vec<_>>();
    centroid_scores.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut selected = Vec::new();
    let min_probe_count = floor_sqrt_usize(centroids.len()).max(1);
    for (probe_index, (cluster_index, _)) in centroid_scores.iter().enumerate() {
        if probe_index >= min_probe_count && selected.len() >= top_k.max(1) {
            break;
        }
        selected.extend(clusters[*cluster_index].iter().copied());
        if selected.len() >= limit {
            break;
        }
    }
    let query_scores = ann_query_scores(candidates, metric, query)?;
    if selected.len() < top_k {
        widen_ann_candidates(&mut selected, &query_scores, top_k);
    }
    sort_and_truncate_indices(&mut selected, &query_scores, limit);
    Ok(selected)
}

pub(super) fn ivf_pq_candidate_indices(
    candidates: &[LoadedVectorSearchCandidate],
    metric: u8,
    query: &[f32],
    top_k: usize,
    limit: usize,
) -> Result<Vec<usize>, CoveError> {
    let coarse_limit = candidates.len().min(limit.saturating_mul(2).max(top_k));
    let mut coarse = ivf_flat_candidate_indices(candidates, metric, query, top_k, coarse_limit)?;
    let ranges = per_dimension_ranges(candidates, query);
    let mut approx_scores = coarse
        .iter()
        .map(|index| {
            (
                *index,
                product_quantized_metric_score(metric, query, &candidates[*index].values, &ranges),
            )
        })
        .collect::<Vec<_>>();
    approx_scores.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    coarse = approx_scores
        .into_iter()
        .take(limit.max(top_k))
        .map(|(index, _)| index)
        .collect();
    if coarse.len() < top_k {
        let exact_scores = ann_query_scores(candidates, metric, query)?;
        widen_ann_candidates(&mut coarse, &exact_scores, top_k);
    }
    Ok(coarse)
}

type IvfClusterBuild = (Vec<Vec<f32>>, Vec<Vec<usize>>);

pub(super) fn ivf_build_clusters(
    candidates: &[LoadedVectorSearchCandidate],
    metric: u8,
) -> Result<IvfClusterBuild, CoveError> {
    let cluster_count = floor_sqrt_usize(candidates.len()).clamp(1, 64);
    let dimension = candidates
        .first()
        .map(|candidate| candidate.values.len())
        .unwrap_or(0);
    let mut centroids = (0..cluster_count)
        .map(|cluster| {
            let index = cluster
                .saturating_mul(candidates.len())
                .checked_div(cluster_count)
                .unwrap_or(0)
                .min(candidates.len().saturating_sub(1));
            candidates[index].values.clone()
        })
        .collect::<Vec<_>>();
    let mut assignments = vec![0usize; candidates.len()];
    for _ in 0..2 {
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            assignments[candidate_index] = nearest_centroid(metric, &candidate.values, &centroids)?;
        }
        let mut sums = vec![vec![0.0f64; dimension]; cluster_count];
        let mut counts = vec![0usize; cluster_count];
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            let cluster = assignments[candidate_index];
            counts[cluster] += 1;
            for (lane, value) in candidate.values.iter().enumerate() {
                sums[cluster][lane] += f64::from(*value);
            }
        }
        for cluster in 0..cluster_count {
            if counts[cluster] == 0 {
                continue;
            }
            let divisor = counts[cluster] as f64;
            for lane in 0..dimension {
                centroids[cluster][lane] = (sums[cluster][lane] / divisor) as f32;
            }
        }
    }
    let mut clusters = vec![Vec::new(); cluster_count];
    for (candidate_index, cluster) in assignments.into_iter().enumerate() {
        clusters[cluster].push(candidate_index);
    }
    Ok((centroids, clusters))
}

pub(super) fn nearest_centroid(
    metric: u8,
    values: &[f32],
    centroids: &[Vec<f32>],
) -> Result<usize, CoveError> {
    centroids
        .iter()
        .enumerate()
        .map(|(index, centroid)| Ok((index, exact_flat_metric_score(metric, values, centroid)?)))
        .collect::<Result<Vec<_>, CoveError>>()?
        .into_iter()
        .max_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|(index, _)| index)
        .ok_or_else(|| CoveError::BadSection("AI IVF index has no centroids".into()))
}

pub(super) fn ann_query_scores(
    candidates: &[LoadedVectorSearchCandidate],
    metric: u8,
    query: &[f32],
) -> Result<Vec<f32>, CoveError> {
    candidates
        .iter()
        .map(|candidate| exact_flat_metric_score(metric, query, &candidate.values))
        .collect()
}

pub(super) fn widen_ann_candidates(indices: &mut Vec<usize>, scores: &[f32], target_len: usize) {
    let mut seen = indices.iter().copied().collect::<BTreeSet<_>>();
    let mut remaining = scores
        .iter()
        .enumerate()
        .filter(|(index, _)| !seen.contains(index))
        .map(|(index, score)| (index, *score))
        .collect::<Vec<_>>();
    remaining.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    for (index, _) in remaining {
        if indices.len() >= target_len {
            break;
        }
        if seen.insert(index) {
            indices.push(index);
        }
    }
}

pub(super) fn sort_and_truncate_indices(indices: &mut Vec<usize>, scores: &[f32], limit: usize) {
    let mut seen = BTreeSet::new();
    indices.retain(|index| seen.insert(*index));
    indices.sort_by(|left, right| {
        scores[*right]
            .total_cmp(&scores[*left])
            .then_with(|| left.cmp(right))
    });
    indices.truncate(limit.min(indices.len()));
}

pub(super) fn per_dimension_ranges(
    candidates: &[LoadedVectorSearchCandidate],
    query: &[f32],
) -> Vec<(f32, f32)> {
    let mut ranges = query
        .iter()
        .map(|value| (*value, *value))
        .collect::<Vec<_>>();
    for candidate in candidates {
        for (lane, value) in candidate.values.iter().enumerate() {
            ranges[lane].0 = ranges[lane].0.min(*value);
            ranges[lane].1 = ranges[lane].1.max(*value);
        }
    }
    ranges
}

pub(super) fn product_quantized_metric_score(
    metric: u8,
    query: &[f32],
    vector: &[f32],
    ranges: &[(f32, f32)],
) -> f32 {
    let quantized_query = quantized_reconstruction(query, ranges);
    let quantized_vector = quantized_reconstruction(vector, ranges);
    ann_metric_score(metric, &quantized_query, &quantized_vector)
}

pub(super) fn quantized_reconstruction(values: &[f32], ranges: &[(f32, f32)]) -> Vec<f32> {
    values
        .iter()
        .zip(ranges)
        .map(|(value, (min, max))| {
            let span = max - min;
            if span <= f32::EPSILON {
                return *value;
            }
            let bucket = (((*value - *min) / span) * 15.0).round().clamp(0.0, 15.0);
            *min + (bucket / 15.0) * span
        })
        .collect()
}

pub(super) fn ann_metric_score(metric: u8, left: &[f32], right: &[f32]) -> f32 {
    exact_flat_metric_score(metric, left, right).unwrap_or(f32::NEG_INFINITY)
}

pub(super) fn floor_sqrt_usize(value: usize) -> usize {
    if value <= 1 {
        return value;
    }
    let mut root = 1usize;
    while root
        .saturating_add(1)
        .saturating_mul(root.saturating_add(1))
        <= value
    {
        root += 1;
    }
    root
}

pub(super) fn select_vector_index(
    sidecar: &CoveAiFile,
    vector_space: &VectorSpaceDescriptorV1,
    requested: AiVectorIndexSelection,
) -> SelectedAiVectorIndex {
    if requested == AiVectorIndexSelection::ExactFlat {
        return SelectedAiVectorIndex {
            name: requested.as_str().to_string(),
            index_kind: 0,
            fallback_used: false,
            exact: true,
        };
    }
    let requested_name = requested.as_str();
    let matching = sidecar
        .descriptor_tables
        .vector_indexes
        .iter()
        .find(|index| {
            index.vector_space_id == vector_space.vector_space_id
                && (requested == AiVectorIndexSelection::Auto
                    || vector_index_kind_name(index.index_kind) == requested_name)
        });
    match matching {
        Some(index) if index.exactness_kind == 0 && index.false_negative_policy == 0 => {
            SelectedAiVectorIndex {
                name: vector_index_kind_name(index.index_kind).to_string(),
                index_kind: index.index_kind,
                fallback_used: false,
                exact: true,
            }
        }
        Some(index) if index.index_kind != 0 => SelectedAiVectorIndex {
            name: vector_index_kind_name(index.index_kind).to_string(),
            index_kind: index.index_kind,
            fallback_used: false,
            exact: false,
        },
        Some(index) => SelectedAiVectorIndex {
            name: format!(
                "{}_candidate_metadata_exact_flat_fallback",
                vector_index_kind_name(index.index_kind)
            ),
            index_kind: 0,
            fallback_used: true,
            exact: true,
        },
        None if requested == AiVectorIndexSelection::Auto => SelectedAiVectorIndex {
            name: AiVectorIndexSelection::ExactFlat.as_str().to_string(),
            index_kind: 0,
            fallback_used: false,
            exact: true,
        },
        None => SelectedAiVectorIndex {
            name: format!("{requested_name}_unavailable_exact_flat_fallback"),
            index_kind: 0,
            fallback_used: true,
            exact: true,
        },
    }
}

pub(super) fn exact_flat_filecode_binding_parts(
    sidecar: &CoveAiFile,
    file_code: u32,
) -> Result<
    (
        &FileCodeVectorBindingV1,
        &VectorSpaceDescriptorV1,
        &VectorEntryV1,
    ),
    CoveError,
> {
    let matching_bindings = sidecar
        .descriptor_tables
        .filecode_vector_bindings
        .iter()
        .filter(|binding| binding.file_code == file_code)
        .collect::<Vec<_>>();
    let binding = match matching_bindings.as_slice() {
        [binding] => *binding,
        [] => {
            return Err(CoveError::BadSection(format!(
                "query FileCode {file_code} is not present in COVE-VEC FileCode vector bindings"
            )));
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "query FileCode {file_code} has multiple COVE-VEC FileCode vector bindings"
            )));
        }
    };
    let vector_space = sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .find(|space| space.vector_space_id == binding.vector_space_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_space_id {}",
                binding.binding_id, binding.vector_space_id
            ))
        })?;
    exact_flat_validate_vector_space(vector_space)?;
    let vector_entry = sidecar
        .descriptor_tables
        .vector_entries
        .iter()
        .find(|entry| entry.vector_ref == binding.vector_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_ref {}",
                binding.binding_id, binding.vector_ref
            ))
        })?;
    Ok((binding, vector_space, vector_entry))
}

pub(super) fn runtime_filecode_binding_parts(
    sidecar: &CoveAiFile,
    file_code: u32,
) -> Result<
    (
        &FileCodeVectorBindingV1,
        &VectorSpaceDescriptorV1,
        &VectorEntryV1,
    ),
    CoveError,
> {
    let matching_bindings = sidecar
        .descriptor_tables
        .filecode_vector_bindings
        .iter()
        .filter(|binding| binding.file_code == file_code)
        .collect::<Vec<_>>();
    let binding = match matching_bindings.as_slice() {
        [binding] => *binding,
        [] => {
            return Err(CoveError::BadSection(format!(
                "query FileCode {file_code} is not present in COVE-AI FileCode vector bindings"
            )));
        }
        _ => {
            return Err(CoveError::BadSection(format!(
                "query FileCode {file_code} has multiple COVE-AI FileCode vector bindings"
            )));
        }
    };
    let vector_space = sidecar
        .descriptor_tables
        .vector_spaces
        .iter()
        .find(|space| space.vector_space_id == binding.vector_space_id)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_space_id {}",
                binding.binding_id, binding.vector_space_id
            ))
        })?;
    validate_runtime_vector_space(vector_space)?;
    let vector_entry = sidecar
        .descriptor_tables
        .vector_entries
        .iter()
        .find(|entry| entry.vector_ref == binding.vector_ref)
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "FileCodeVectorBinding {} references missing vector_ref {}",
                binding.binding_id, binding.vector_ref
            ))
        })?;
    Ok((binding, vector_space, vector_entry))
}

pub(super) fn exact_flat_filecode_vector_search_in_space(
    artifact_bytes: &[u8],
    sidecar: &CoveAiFile,
    vector_space: &VectorSpaceDescriptorV1,
    query: &[f32],
    top_k: usize,
) -> Result<Vec<ExactFlatFileCodeVectorSearchResult>, CoveError> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    let mut seen_file_codes = BTreeSet::new();
    let mut results = Vec::new();
    for binding in &sidecar.descriptor_tables.filecode_vector_bindings {
        if binding.vector_space_id != vector_space.vector_space_id {
            continue;
        }
        if !seen_file_codes.insert(binding.file_code) {
            return Err(CoveError::BadSection(format!(
                "duplicate FileCode {} in FileCode vector bindings",
                binding.file_code
            )));
        }
        let vector_entry = sidecar
            .descriptor_tables
            .vector_entries
            .iter()
            .find(|entry| entry.vector_ref == binding.vector_ref)
            .ok_or_else(|| {
                CoveError::BadSection(format!(
                    "FileCodeVectorBinding {} references missing vector_ref {}",
                    binding.binding_id, binding.vector_ref
                ))
            })?;
        let vector =
            exact_flat_vector_entry_f32(artifact_bytes, sidecar, vector_space, vector_entry)?;
        let score = exact_flat_metric_score(vector_space.metric, query, &vector)?;
        results.push(ExactFlatFileCodeVectorSearchResult {
            file_code: binding.file_code,
            vector_ref: binding.vector_ref,
            vector_space_id: vector_space.vector_space_id,
            score,
        });
    }

    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.file_code.cmp(&right.file_code))
            .then_with(|| left.vector_ref.cmp(&right.vector_ref))
    });
    results.truncate(top_k.min(results.len()));
    Ok(results)
}
