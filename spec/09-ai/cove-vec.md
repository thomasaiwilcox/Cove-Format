# COVE-VEC

## 83.50 COVE-VEC Vector Profile

COVE-VEC stores vectors once per distinct semantic unit and lets rows, objects,
chunks, samples, and multimodal assets reference them. The preferred optimized
artifact is `.covev` with magic `CVV2`, though the same logical sections MAY
appear in `.coveai`, COVX, or COVE-I-style sidecars.

### 83.50.1 Artifact Boundary

Required value-vector sections are `AI_REFERENCE_TABLES`, `AI_SOURCE_BINDING`,
`AI_PRIVACY_SUMMARY`, `AI_VECTOR_SPACE`, `AI_VECTOR_BINDING`,
`AI_VECTOR_PAYLOAD_BLOCK`, `AI_VECTOR_DIRECTORY`, `AI_PAYLOAD_BYTES` when
payload bytes are persisted, and `AI_PAYLOAD_INTEGRITY` when any payload block
claims stored-byte verification, replayability, auditability, or trust-chain
participation.

A sidecar without `AI_PRIVACY_SUMMARY` MAY validate structurally, but direct
payload access MUST remain policy-blocked unless a trusted caller policy
overrides the missing summary.

Reader obligations:

- validate source binding before reading vector bindings or payload bytes;
- validate vector-space dimension, element type, metric, normalization policy,
  quantization policy, model lineage, template fingerprint, and
  reproducibility class before comparing vectors;
- reject vector payload blocks whose byte length, stride, element type,
  dimension count, compression, or quantization metadata does not match the
  vector-space descriptor;
- use FileCode as a lookup key only inside the validated file and dictionary
  scope;
- use canonical value hash, schema/path binding, dictionary digest, or an
  explicit code-domain bridge across files, manifests, rewrites, or snapshots;
- treat approximate vector indexes as candidate generators unless exactness is
  proven for the requested metric, visibility scope, redaction scope, and query
  class;
- fall back from stale or unsupported vector indexes to exact vector scan when
  vector payloads are valid and the query permits it.

### 83.50.2 Vector Spaces

```rust
struct VectorSpaceDescriptorV1 {
    vector_space_id: u32,
    vector_space_name_ref: u32,
    vector_space_fingerprint_ref: u32,
    embedding_namespace_ref: u32,
    embedding_model_ref: u32,
    embedding_model_version_ref: u32,
    embedding_model_digest_ref: u32,
    embedding_pipeline_ref: u32,
    tokenizer_profile_ref: u32,
    chunk_profile_ref: u32,
    dimension_count: u32,
    element_type: u8,
    metric: u8,
    normalization_policy: u8,
    quantization_policy: u8,
    deterministic: u8,
    approximate: u8,
    reproducibility_class: u8,
    reserved: u8,
    flags: u32,
    checksum: u32,
}

struct VectorSpaceCompatibilityDescriptorV1 {
    compatibility_id: u32,
    left_vector_space_id: u32,
    right_vector_space_id: u32,
    compatibility_kind: u8,
    compatibility_authority: u8,
    metric: u8,
    normalization_policy: u8,
    transform_ref: u32,
    numeric_transform_error_ppm: u32,
    ranking_eval_ref: u32,
    calibration_dataset_ref: u32,
    evidence_ref: u32,
    flags: u32,
    checksum: u32,
}
```

Vectors from different `vector_space_id` values MUST NOT be compared unless a
compatibility descriptor explicitly permits it. Metric and normalization policy
are part of vector-space identity. Cross-artifact identity uses
`vector_space_fingerprint_ref`, not the local numeric ID.

### 83.50.3 Bindings

```rust
enum VectorBindingKindV1 {
    RawCanonicalValue = 0,
    SlotValue = 1,
    TextChunk = 2,
    TokenizedSpan = 3,
    RowProjection = 4,
    ObjectState = 5,
    AssociationState = 6,
    EvidenceSpan = 7,
    TrainingSample = 8,
    PromptContext = 9,
    MultimodalAsset = 10,
    MultimodalSequence = 11,
    Extension = 255,
}

struct FileCodeVectorBindingV1 {
    binding_id: u64,
    vector_space_id: u32,
    slot_policy_ref: u32,
    file_ref: u32,
    dictionary_digest_ref: u32,
    schema_fingerprint_ref: u32,
    table_id: u32,
    column_id: u32,
    object_type_id: u32,
    property_id: u32,
    association_type_id: u32,
    path_ref: u32,
    file_code: u32,
    reserved0: u32,
    canonical_value_hash_ref: u32,
    vector_ref: u64,
    flags: u32,
    checksum: u32,
}

struct ChunkVectorBindingV1 {
    binding_id: u64,
    vector_space_id: u32,
    chunk_id: u64,
    chunk_profile_id: u32,
    source_value_hash_ref: u32,
    chunk_text_hash_ref: u32,
    vector_ref: u64,
    flags: u32,
    checksum: u32,
}

struct ObjectStateVectorBindingV1 {
    binding_id: u64,
    vector_space_id: u32,
    composition_profile_ref: u32,
    file_ref: u32,
    object_type_id: u32,
    goid_ref: u32,
    branch_ref: u32,
    temporal_kind: u8,
    csn: u64,
    timestamp_us: i64,
    property_dependency_fingerprint_ref: u32,
    vector_ref: u64,
    flags: u32,
    checksum: u32,
}

struct TrainingSampleVectorBindingV1 {
    binding_id: u64,
    vector_space_id: u32,
    training_profile_ref: u32,
    sample_id: u64,
    source_snapshot_ref: u32,
    sample_fingerprint_ref: u32,
    vector_ref: u64,
    flags: u32,
    checksum: u32,
}
```

`file_code` is the COVE v2 `FileCode` type and is therefore `u32`.
`reserved0` MUST be zero. A raw FileCode from another file MUST NOT be used as
a vector key unless the plan proves a shared code domain or validates an
equivalent canonical binding.

Chunk, object-state, association-state, training-sample, prompt-context,
asset, and multimodal-sequence bindings MUST reference `VectorEntryV1` through
`vector_ref` and MUST carry enough source, slot, temporal, and model lineage to
validate freshness.

### 83.50.4 Payload Blocks and Directory

```rust
struct VectorPayloadBlockHeaderV1 {
    block_id: u32,
    vector_space_id: u32,
    vector_count: u64,
    dimension_count: u32,
    element_type: u8,
    compression_codec: u8,
    quantization_kind: u8,
    layout_kind: u8,
    tensor_layout_ref: u32,
    memory_alignment_bytes: u32,
    payload_stride_ref: u32,
    device_transfer_hint_ref: u32,
    payload_ref: u32,
    payload_offset: u64,
    payload_length: u64,
    integrity_ref: u32,
    checksum: u32,
}

struct VectorEntryV1 {
    vector_ref: u64,
    block_id: u32,
    vector_ordinal: u64,
    payload_offset: u64,
    payload_length: u32,
    integrity_ref: u32,
    flags: u32,
    checksum: u32,
}
```

`VectorPayloadBlockHeaderV1.payload_ref` MUST resolve to an
`AiPayloadRefEntryV1`. For provider-free COVE-VEC payloads, that payload ref
MUST point into an `AI_PAYLOAD_BYTES` section; vector bytes MUST NOT be
appended to `AI_VECTOR_PAYLOAD_BLOCK` after descriptor records.

`VectorEntryV1.payload_offset` and `payload_length` identify a vector byte
range within the resolved block payload. The range MUST be fully contained in
the block payload. If `payload_length == 0`, offset derivation is allowed only
for fixed-stride dense vectors.

### 83.50.5 Composition and Arithmetic

```rust
enum VectorResultAuthorityV1 {
    RuntimeAdvisory = 0,
    PersistedPayloadDigest = 1,
    CanonicalFixedPointRecompute = 2,
    ExactExternalProof = 3,
    Extension = 255,
}

struct VectorCompositionProfileV1 {
    composition_profile_id: u32,
    composition_name_ref: u32,
    output_vector_space_id: u32,
    arithmetic_profile_ref: u32,
    method: u8,
    missing_policy: u8,
    normalize_inputs: u8,
    normalize_output: u8,
    result_authority: u8,
    reproducibility_class: u8,
    first_component_ref: u32,
    component_count: u32,
    template_ref: u32,
    flags: u32,
    checksum: u32,
}

struct VectorCompositionComponentV1 {
    component_id: u32,
    slot_policy_ref: u32,
    source_vector_space_id: u32,
    weight_ppm: u32,
    required: u8,
    redaction_behavior: u8,
    missing_behavior: u8,
    reserved: u8,
    flags: u32,
    checksum: u32,
}

struct VectorArithmeticProfileV1 {
    arithmetic_profile_id: u32,
    profile_name_ref: u32,
    arithmetic_kind: u8,
    input_quantization_kind: u8,
    accumulator_kind: u8,
    rounding_mode: u8,
    overflow_policy: u8,
    component_order: u8,
    weight_scale: u32,
    output_quantization_kind: u8,
    output_element_type: u8,
    normalization_policy: u8,
    flags: u32,
    checksum: u32,
}
```

Runtime composition over ordinary floating-point math has `RuntimeAdvisory`
authority unless materialized and digested or computed under a canonical
fixed-point arithmetic profile. `RuntimeAdvisory` vectors MUST NOT be used as
byte-reproducible conformance vectors or trust-chain payloads.

### 83.50.6 Vector Indexes

```rust
enum VectorIndexKindV1 {
    ExactFlat = 0,
    Hnsw = 1,
    IvfFlat = 2,
    IvfPq = 3,
    DiskAnn = 4,
    Vamana = 5,
    ScannLike = 6,
    ProductQuantized = 7,
    BinaryFlat = 8,
    Extension = 255,
}

struct VectorIndexDescriptorV1 {
    vector_index_id: u32,
    vector_space_id: u32,
    stored_vector_space_id: u32,
    search_vector_space_id: u32,
    index_kind: u8,
    exactness_kind: u8,
    false_negative_policy: u8,
    metric: u8,
    score_space_authority: u8,
    dimension_count: u32,
    indexed_binding_kind: u8,
    temporal_scope_ref: u32,
    visibility_scope_ref: u32,
    redaction_scope_ref: u32,
    dequantization_profile_ref: u32,
    quantization_error_profile_ref: u32,
    payload_ref: u32,
    checksum: u32,
}
```

Approximate indexes MAY return candidates. They MUST NOT claim complete
nearest-neighbor results unless their descriptor proves exactness for the
requested metric and query class. If temporal, visibility, or redaction filters
are applied after candidate generation, filtered top-k results MUST be marked
possibly incomplete unless coverage for the filtered universe is proven.

### 83.50.7 Tensor Layouts and Assets

Tensor layout descriptors define dtype, rank, shape, stride, alignment,
storage offset, layout kind, quantization, sparsity, and device-transfer hints.
Zero-copy export MAY be used only after validating payload bounds, dtype,
shape, strides, alignment, compression state, quantization profile, lifetime,
visibility/redaction policy, and target runtime compatibility.

```rust
struct TensorLayoutDescriptorV1 {
    tensor_layout_id: u32,
    layout_name_ref: u32,
    rank: u8,
    dtype: u8,
    byte_order: u8,
    shape_ref: u32,
    stride_ref: u32,
    storage_offset_elements: i64,
    layout_kind: u8,
    memory_alignment_bytes: u32,
    preferred_page_alignment_bytes: u32,
    tile_shape_ref: u32,
    block_shape_ref: u32,
    quantization_profile_ref: u32,
    sparsity_profile_ref: u32,
    framework_compatibility_ref: u32,
    device_affinity_hint: u8,
    flags: u32,
    checksum: u32,
}

struct DeviceTransferHintV1 {
    transfer_hint_id: u32,
    target_kind: u8,
    preferred_alignment_bytes: u32,
    preferred_chunk_bytes: u32,
    pinned_memory_required: u8,
    contiguous_required: u8,
    zero_copy_possible: u8,
    runtime_registry_binding_ref: u32,
    flags: u32,
    checksum: u32,
}
```

Tensor layout descriptors are physical layout and interoperability metadata.
They MUST NOT override canonical logical values, visibility policy, redaction
policy, or source evidence. If zero-copy validation fails, a reader MUST
materialize a safe owned output buffer or reject the operation with
diagnostics.

`AiAssetRefV1` records describe embedded or external image, audio, video,
document, and derived assets. External assets MUST be digest-bound when
replayability or training reproducibility is claimed. Preprocessing that
affects model input, including EXIF orientation, color management, resize,
crop, resampling, frame extraction, OCR, captioning, transcription, and
model-specific transforms, MUST be declared.

```rust
struct AiAssetRefV1 {
    asset_ref_id: u64,
    parent_asset_ref: u64,
    asset_kind: u8,
    uri_ref: u32,
    embedded_section_ref: u32,
    media_type_ref: u32,
    byte_length: u64,
    digest_ref: u32,
    width: u32,
    height: u32,
    duration_us: u64,
    sample_rate_hz: u32,
    channel_count: u16,
    decode_profile_ref: u32,
    preprocessing_profile_ref: u32,
    transform_profile_ref: u32,
    transform_digest_ref: u32,
    tensor_layout_ref: u32,
    license_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}
```

A URI alone is not stable asset identity. Derived captions, OCR text,
transcripts, embeddings, and labels SHOULD bind back to the source asset
digest and generator provenance when applicable. Derived assets SHOULD set
`parent_asset_ref`, `transform_profile_ref`, and `transform_digest_ref` so the
lineage chain is auditable.

`preprocessing_profile_ref` describes model-input preprocessing applied when an
asset is fed to a tokenizer, encoder, embedding model, or training sample.
`transform_profile_ref` describes derived-asset lineage from
`parent_asset_ref` to this asset. They may reference the same transform only
when the stored derived asset is exactly the model input.
