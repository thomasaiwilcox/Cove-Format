# COVE-AI Optional Extension

## 83. COVE-AI Optional Extension Profile

COVE-AI is an optional standards-suite extension for AI-oriented companion
metadata, chunks, token caches, vector payloads, vector indexes, training
records, generator provenance, multimodal assets, tensor layout metadata, and
CoveQL-AI query surfaces.

COVE-AI metadata is derivative. It MUST NOT redefine canonical values,
COVE-T rows, COVE-O object reconstruction, or COVE-MAP semantic mapping truth.
Baseline COVE-Core, COVE-T, COVE-O, and COVE-MAP readers MAY ignore optional
COVE-AI metadata and MUST NOT require COVE-AI sidecars unless the selected
profile, section, page, or operation explicitly requires the AI feature.

The COVE-AI profile family reserves primary profile IDs 16 through 23:

| Profile ID | Profile | Spec document |
| ---: | --- | --- |
| 16 | COVE-AI Shared | this document |
| 17 | COVE-MAP-AI | [`cove-map-ai.md`](cove-map-ai.md) |
| 18 | COVE-CHUNK | [`cove-chunk.md`](cove-chunk.md) |
| 19 | COVE-TOK | [`cove-tok.md`](cove-tok.md) |
| 20 | COVE-VEC | [`cove-vec.md`](cove-vec.md) |
| 21 | COVE-MMSEQ | [`cove-mmseq.md`](cove-mmseq.md) |
| 22 | COVE-TRAIN | [`cove-train.md`](cove-train.md) |
| 23 | CoveQL-AI | [`coveql-ai.md`](coveql-ai.md) |

All COVE-AI profiles are optional. Unknown optional COVE-AI sections or
profiles MUST be ignored safely for ordinary COVE-T/O/MAP reads and SHOULD be
reported by inspect tooling. Unknown required COVE-AI support rejects only the
sidecar, profile, section, page, or selected operation whose requiredness scope
intersects the operation.

### 83.1 Design Contract

COVE-AI uses the existing COVE v2 scoping, sidecar, digest, redaction, policy,
and operation-requiredness machinery. It is not a parallel format with different
validity rules.

Normative rules:

- Embedded COVE-AI metadata in `.cove` uses the global COVE section-kind
  registry and global feature-word model.
- Operation-only COVE-AI requirements MUST NOT be placed in `.cove`
  `required_features` word 0.
- Embedded AI requiredness MUST be scoped through section entries, profile
  capability matrices, or `SECTION_FEATURE_BINDING`.
- Standalone `CVA2` and `CVV2` artifacts use artifact-local AI feature words
  and `AI_SECTION_FEATURE_BINDING`.
- COVE-VEC owns vector spaces, vector payloads, vector bindings, composition
  metadata, quantization metadata, and vector lineage.
- COVX/COVE-I-compatible rules own proof semantics, false-negative policy,
  index-only capability, sidecar validity, coverage/fallback semantics, and
  dataset-level index publication.
- Every payload-bearing AI section MUST bind to the source snapshot, schema,
  dictionary or canonical value identity, semantic slot or object path,
  model/tokenizer/chunker/vectorizer/template lineage, policy scope,
  visibility scope, redaction scope, and digest lineage needed to validate
  freshness.
- CRC32C validates transport integrity. Cryptographic digests are required for
  trust, payload-byte verification, replay, reproducibility, and auditability
  claims.
- A direct reader of `CVA2` or `CVV2` MUST fail closed on payload access until
  source binding, visibility scope, redaction scope, policy scope, and
  sensitivity summaries validate, or until a trusted caller policy explicitly
  overrides the fail-closed default.

### 83.2 Feature Bits

COVE-AI uses extended global feature word `1` for embedded `.cove` metadata.
The same bit numbers are artifact-local AI feature bits inside `CVA2` and
`CVV2` sidecars.

| Bit | Feature | Scope |
| ---: | --- | --- |
| 0 | `AI_FEATURE_MAP_AI_POLICY` | COVE-MAP-AI |
| 1 | `AI_FEATURE_CHUNK` | COVE-CHUNK |
| 2 | `AI_FEATURE_TOKEN` | COVE-TOK |
| 3 | `AI_FEATURE_VECTOR` | COVE-VEC |
| 4 | `AI_FEATURE_VECTOR_INDEX` | COVE-VEC |
| 5 | `AI_FEATURE_TENSOR_LAYOUT` | COVE-VEC / COVE-MMSEQ |
| 6 | `AI_FEATURE_ASSET_REF` | COVE-MMSEQ / COVE-TRAIN |
| 7 | `AI_FEATURE_MMSEQ` | COVE-MMSEQ |
| 8 | `AI_FEATURE_TRAIN` | COVE-TRAIN |
| 9 | `AI_FEATURE_GENERATOR_PROVENANCE` | COVE-TRAIN |
| 10 | `AI_FEATURE_COVEQL_AI` | CoveQL-AI |
| 11 | `AI_FEATURE_CANONICAL_FIXED_POINT_VECTOR` | COVE-VEC |
| 12 | `AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED` | COVE-MMSEQ / COVE-TRAIN |
| 13 | `AI_FEATURE_PRIVACY_SUMMARY` | COVE-AI Shared |
| 14 | `AI_FEATURE_VECTOR_SPACE_COMPATIBILITY` | COVE-VEC |
| 15 | `AI_FEATURE_MODEL_INPUT_IDENTITY` | COVE-VEC |

Unknown optional AI bits are ignored for ordinary reads. Unknown required AI
bits reject only the scope where they are required. Writers SHOULD bind
required COVE-AI features to the narrowest possible scope: section, profile,
artifact, page, or operation.

### 83.3 Operation Kinds

COVE-AI operation kinds extend `OperationKindV2`:

| Operation kind | Operation |
| ---: | --- |
| 128 | `AiInspect` |
| 129 | `AiChunkProjection` |
| 130 | `AiTokenProjection` |
| 131 | `AiEmbedding` |
| 132 | `AiSemanticSearch` |
| 133 | `AiRagContext` |
| 134 | `AiTrainingSampleExport` |
| 135 | `AiMultimodalSequenceRead` |
| 136 | `AiGeneratorAudit` |

### 83.4 Section Kinds

COVE-MAP-AI policy lives with COVE-MAP because it describes semantic intent.
Large derived AI payloads normally live in COVE-AI bundle artifacts, `.covev`
artifacts, COVX, or COVE-I-style sidecars.

| ID | Name | Owning profile |
| ---: | --- | --- |
| 70 | `MAP_AI_PROFILE_CATALOG` | COVE-MAP-AI |
| 71 | `MAP_AI_TEMPLATE_CATALOG` | COVE-MAP-AI |
| 72 | `MAP_AI_TRAINING_POLICY_CATALOG` | COVE-MAP-AI |
| 99 | `AI_COMPANION_ARTIFACT_REF` | COVE-AI Shared |
| 100 | `AI_SOURCE_BINDING` | COVE-AI Shared |
| 101 | `AI_CHUNK_PROFILE` | COVE-CHUNK |
| 102 | `AI_TEXT_CHUNK_INDEX` | COVE-CHUNK |
| 103 | `AI_TOKENIZER_PROFILE` | COVE-TOK |
| 104 | `AI_TOKEN_BLOCK` | COVE-TOK |
| 105 | `AI_TOKENIZED_SPAN` | COVE-TOK |
| 106 | `AI_TOKEN_SEQUENCE_PACK` | COVE-TOK |
| 107 | `AI_VECTOR_SPACE` | COVE-VEC |
| 108 | `AI_VECTOR_BINDING` | COVE-VEC |
| 109 | `AI_VECTOR_PAYLOAD_BLOCK` | COVE-VEC |
| 110 | `AI_VECTOR_COMPOSITION` | COVE-VEC |
| 111 | `AI_VECTOR_INDEX` | COVE-VEC |
| 112 | `AI_TENSOR_LAYOUT` | COVE-VEC / COVE-MMSEQ |
| 113 | `AI_ASSET_MANIFEST` | COVE-MMSEQ / COVE-TRAIN |
| 114 | `AI_MULTIMODAL_SEQUENCE` | COVE-MMSEQ |
| 115 | `AI_TRAINING_PROFILE` | COVE-TRAIN |
| 116 | `AI_TRAINING_SAMPLE_INDEX` | COVE-TRAIN |
| 117 | `AI_TRAINING_SPLIT_DEDUP_EPOCH` | COVE-TRAIN |
| 118 | `AI_LABEL_PREFERENCE` | COVE-TRAIN |
| 119 | `AI_GENERATOR_PROVENANCE` | COVE-TRAIN |
| 120 | `AI_REFERENCE_TABLES` | COVE-AI Shared |
| 121 | `AI_PAYLOAD_INTEGRITY` | COVE-AI Shared |
| 122 | `AI_PRIVACY_SUMMARY` | COVE-AI Shared |
| 123 | `AI_SECTION_FEATURE_BINDING` | COVE-AI Shared |
| 124 | `AI_VECTOR_DIRECTORY` | COVE-VEC |
| 125 | `AI_PAYLOAD_BYTES` | COVE-AI Shared |

`MAP_AI_*` payloads follow the COVE-MAP payload discipline. `AI_*` descriptor
payloads use length-delimited `AiRecordHeaderV1` binary records unless a
section explicitly declares canonical JSON, deterministic CBOR, or
`AI_PAYLOAD_BYTES` opaque storage. `AI_PAYLOAD_BYTES` is never parsed as record
metadata.

### 83.5 Companion Artifacts

COVE-AI defines two companion artifact shapes:

```text
Extension: .coveai
Magic: CVA2
Profile: COVE-AI bundle
Purpose: mixed chunks, tokens, vectors, training samples, multimodal sequences,
         generator provenance, assets, and tensor metadata.

Extension: .covev
Magic: CVV2
Profile: COVE-VEC optimized vector artifact
Purpose: large vector payloads, vector bindings, vector composition metadata,
         vector indexes, tensor layouts, and device-transfer hints.
```

```rust
struct CoveAiPostscriptV1 {
    required_ai_features: u64,
    optional_ai_features: u64,
    file_len: u64,
    header_offset: u64,
    header_length: u64,
    crc32c: u32,
}

struct CoveAiHeaderV1 {
    magic: [u8; 4],
    header_len: u16,
    version_major: u16,
    version_minor: u16,
    flags: u32,
    artifact_id: [u8; 16],
    required_ai_features: u64,
    optional_ai_features: u64,
    section_count: u32,
    section_entry_len: u16,
    reserved0: u16,
    created_at_us: i64,
    section_directory_crc32c: u32,
    crc32c: u32,
}

struct CoveAiSectionEntryV1 {
    section_id: u32,
    section_kind: u32,
    offset: u64,
    length: u64,
    uncompressed_length: u64,
    compression: u8,
    payload_encoding: u8,
    requiredness_scope: u8,
    profile_kind: u8,
    source_binding_ref: u32,
    required_ai_features: u64,
    optional_ai_features: u64,
    feature_binding_ref: u32,
    payload_crc32c: u32,
}

struct AiSourceBindingV1 {
    source_binding_id: u32,
    source_kind: u8,
    source_artifact_ref: u32,
    source_file_digest_ref: u32,
    covm_snapshot_ref: u32,
    schema_fingerprint_ref: u32,
    dictionary_digest_ref: u32,
    map_fingerprint_ref: u32,
    policy_context_ref: u32,
    visibility_scope_ref: u32,
    redaction_scope_ref: u32,
    branch_ref: u32,
    as_of_csn: u64,
    flags: u32,
    crc32c: u32,
}

struct AiCompanionArtifactRefV1 {
    artifact_ref: u32,
    artifact_kind: u8,
    artifact_id: [u8; 16],
    uri_ref: u32,
    artifact_digest_ref: u32,
    source_binding_ref: u32,
    required_ai_features: u64,
    optional_ai_features: u64,
    flags: u32,
    crc32c: u32,
}

enum AiPayloadEncodingV1 {
    BinaryRecords = 1,
    CanonicalJson = 2,
    DeterministicCbor = 3,
    OpaqueBytes = 4,
    Extension = 255,
}

enum AiRequirednessScopeV1 {
    ArtifactRequired = 0,
    SectionRequired = 1,
    ProfileRequired = 2,
    OperationRequired = 3,
    AdvisoryOnly = 4,
    Extension = 255,
}
```

Artifact rules:

- All integers are little-endian.
- Tail discovery uses final postscript bytes, `postscript_version: u16`,
  `postscript_len: u16`, and magic `CVA2` or `CVV2`.
- `postscript_version`, `version_major`, `header_len`, and
  `section_entry_len` MUST match the supported profile version.
- Header and postscript feature words MUST match exactly.
- Section ranges MUST be within `file_len` and MUST NOT overlap unless a
  required extension permits overlap.
- `section_id` values are unique within the companion artifact.
- `offset` and `length` are artifact-absolute byte offsets.
- Descriptor sections validate decoded payload CRC32C before records are used.
- Large payload-bearing sections MAY use block/range integrity rather than a
  whole-section payload CRC only when section rules explicitly allow it.
- A companion artifact MUST be integrity-checkable without consulting external
  services.
- `.cove`, COVM, or catalog references to a COVE-AI companion artifact SHOULD
  use `AI_COMPANION_ARTIFACT_REF` or an equivalent COVM extension carrying
  `AiCompanionArtifactRefV1`; the sidecar reference MUST be digest-bound before
  sidecar use is trusted. The reference implementation's COVM `CAI1`
  extension binds sidecar URI, source member file id, sidecar byte length, and
  sidecar digest; ordinary non-AI COVM readers ignore the extension, while
  selected AI operations reject stale referenced sidecars.
- `source_binding_ref` binds every derived section to the source file,
  snapshot, dictionary, schema, mapping, visibility, and redaction context
  needed to check freshness.
- A stale source file digest, schema fingerprint, dictionary digest, mapping
  fingerprint, branch, CSN, visibility scope, or redaction scope makes the
  affected section unusable for the requested AI operation.
- `CVA2` and `CVV2` headers and records are packed wire records. Readers MUST
  parse fields explicitly and MUST NOT rely on native struct alignment or
  padding.

### 83.6 Validation Order

A conforming COVE-AI sidecar reader SHOULD validate in this order:

1. Read tail magic, `postscript_version`, and `postscript_len`.
2. Validate postscript length, file length, header bounds, and postscript CRC.
3. Validate header fields, header CRC, and section-directory CRC.
4. Require matching postscript/header feature words.
5. Validate section bounds, unique `section_id` values, non-overlap,
   compression declarations, and descriptor payload CRCs.
6. Reject unknown artifact-required header AI feature bits.
7. Validate `AI_SECTION_FEATURE_BINDING`.
8. Build the sidecar feature-scope table.
9. Select the requested AI operation.
10. Reject unknown required AI features whose scope intersects that operation.
11. Validate `AI_SOURCE_BINDING`, `AI_REFERENCE_TABLES`, `AI_PRIVACY_SUMMARY`,
    and policy context before exposing payload-bearing sections.

### 83.7 Reference Spaces and Integrity

COVE-AI records use local `_ref` fields. Unless a structure says otherwise,
`0` means absent and non-zero values resolve through the single standard
`AI_REFERENCE_TABLES` section or through the local record table named by the
field.

Standard reference spaces are:

| Space | Used by |
| --- | --- |
| `AI_STRING_TABLE` | Names, namespaces, versions, locale tags, endpoint names, templates, prompt-template IDs, and media types. |
| `AI_DIGEST_TABLE` | Source file digests, dictionary digests, schema fingerprints, model/checkpoint digests, tokenizer material digests, payload digests, and transform digests. |
| `AI_POLICY_TABLE` | Visibility, redaction, sensitivity, license, safety, retention, disclosure, and export policies. |
| `AI_PAYLOAD_REF_TABLE` | Payload byte ranges, embedded blobs, masks, labels, prompt text, target text, metadata, and external payload handles. |
| `AI_FUNCTION_OR_TEMPLATE_TABLE` | Chunkers, tokenizers, vectorizers, transforms, templates, normalization pipelines, scoring functions, and deterministic split functions. |
| `AI_MASK_LABEL_TABLE` | Loss masks, attention masks, labels, position IDs, preference records, and quality-score payloads. |
| `AI_SOURCE_SPAN_TABLE` | Source rows, object refs, source values, evidence spans, byte ranges, token ranges, and asset time ranges. |
| `AI_TRANSFORM_TABLE` | Asset preprocessing, vector transforms, quantization transforms, calibration transforms, OCR/caption/transcript transforms, and image/audio/video normalization profiles. |
| `AI_EXTENSION_TABLE` | Registered extension records and vendor payload references. |

IDs MUST be unique per reference space. Duplicate IDs are structural corruption
unless a required extension defines scoped duplicates.

```rust
struct AiRecordHeaderV1 {
    record_kind: u16,
    record_version: u16,
    record_len: u32,
    local_id: u64,
    flags: u32,
    crc32c: u32,
}

enum AiReferenceSpaceKindV1 {
    String = 1,
    Digest = 2,
    Policy = 3,
    Payload = 4,
    FunctionOrTemplate = 5,
    MaskLabel = 6,
    SourceSpan = 7,
    Transform = 8,
    Extension = 255,
}

enum AiTargetKindV1 {
    Unknown = 0,
    Utf8String = 1,
    DigestBytes = 2,
    EmbeddedPayloadBytes = 3,
    ExternalUri = 4,
    PolicyPayload = 5,
    SourceSpan = 6,
    TransformPayload = 7,
    Extension = 255,
}

enum AiStorageKindV1 {
    ArtifactAbsolute = 0,
    SectionDecodedRelative = 1,
    ExternalUri = 2,
    EmbeddedSection = 3,
    Extension = 255,
}

enum AiPolicyKindV1 {
    Visibility = 0,
    Redaction = 1,
    Sensitivity = 2,
    License = 3,
    Retention = 4,
    Disclosure = 5,
    Safety = 6,
    Extension = 255,
}

enum AiSourceKindV1 {
    CoveFile = 0,
    CovmSnapshot = 1,
    CovemapArtifact = 2,
    ExternalAsset = 3,
    ExternalDataset = 4,
    Extension = 255,
}

enum AiTransformKindV1 {
    None = 0,
    TextNormalization = 1,
    Tokenizer = 2,
    Chunker = 3,
    Vectorizer = 4,
    Quantization = 5,
    ImagePreprocess = 6,
    AudioPreprocess = 7,
    VideoFrameExtraction = 8,
    Ocr = 9,
    Caption = 10,
    Transcript = 11,
    Extension = 255,
}

enum AiCompressionCodecV1 {
    None = 0,
    Lz4 = 1,
    Zstd = 2,
    Extension = 255,
}

enum AiVectorElementTypeV1 {
    Float32 = 0,
    Float16 = 1,
    BFloat16 = 2,
    Int8 = 3,
    UInt8 = 4,
    Binary = 5,
    Extension = 255,
}

enum AiVectorMetricV1 {
    Cosine = 0,
    Dot = 1,
    L2 = 2,
    L1 = 3,
    Hamming = 4,
    Extension = 255,
}

enum AiNormalizationPolicyV1 {
    None = 0,
    UnitL2 = 1,
    MeanCentered = 2,
    ModelDefined = 3,
    Extension = 255,
}

enum AiQuantizationKindV1 {
    None = 0,
    ScalarInt8 = 1,
    Binary = 2,
    ProductQuantized = 3,
    Extension = 255,
}

enum AiLayoutKindV1 {
    DenseRowMajor = 0,
    DenseColumnMajor = 1,
    SparseCsr = 2,
    PackedBinary = 3,
    Extension = 255,
}

enum AiAssetKindV1 {
    Uri = 0,
    EmbeddedBytes = 1,
    Image = 2,
    Audio = 3,
    Video = 4,
    Document = 5,
    Tensor = 6,
    Extension = 255,
}

enum AiModalityV1 {
    Text = 0,
    Token = 1,
    Image = 2,
    Audio = 3,
    Video = 4,
    Tensor = 5,
    Tool = 6,
    Control = 7,
    Extension = 255,
}

enum AiRoleV1 {
    Unknown = 0,
    System = 1,
    User = 2,
    Assistant = 3,
    Tool = 4,
    Label = 5,
    Control = 6,
    Extension = 255,
}

enum GeneratorKindV1 {
    Human = 0,
    Model = 1,
    Tool = 2,
    Heuristic = 3,
    ExternalBenchmark = 4,
    Extension = 255,
}

enum TrainingLabelKindV1 {
    Class = 0,
    Text = 1,
    NumericScore = 2,
    Preference = 3,
    Ranking = 4,
    Span = 5,
    Safety = 6,
    Extension = 255,
}

enum AiDigestDomainV1 {
    StoredCompressedBytes = 0,
    DecodedSectionPayloadBytes = 1,
    RecordPayloadBytes = 2,
    CanonicalRecordBytes = 3,
    ModelInputBytes = 4,
    ExternalAssetBytes = 5,
    VectorPayloadBytes = 6,
    TokenPayloadBytes = 7,
    Extension = 255,
}

struct AiStringEntryV1 {
    string_ref: u32,
    utf8_byte_length: u32,
    payload_ref: u32,
    flags: u32,
    crc32c: u32,
}

struct AiDigestEntryV1 {
    digest_ref: u32,
    digest_algorithm: u16,
    digest_len: u16,
    digest_payload_ref: u32,
    domain_hint: u8,
    flags: u32,
    crc32c: u32,
}

struct AiPayloadRefEntryV1 {
    payload_ref: u32,
    storage_kind: u8,
    media_type_ref: u32,
    section_id: u32,
    uri_ref: u32,
    payload_offset: u64,
    section_payload_offset: u64,
    payload_length: u64,
    decoded_length: u64,
    integrity_ref: u32,
    flags: u32,
    crc32c: u32,
}

struct AiPolicyRefEntryV1 {
    policy_ref: u32,
    policy_kind: u8,
    authority_ref: u32,
    payload_ref: u32,
    digest_ref: u32,
    flags: u32,
    crc32c: u32,
}

struct AiSourceSpanEntryV1 {
    source_span_ref: u32,
    source_binding_ref: u32,
    source_kind: u8,
    source_row_ref: u64,
    source_object_ref: u64,
    byte_start: u64,
    byte_length: u64,
    token_start: u64,
    token_count: u32,
    evidence_ref: u32,
    flags: u32,
    crc32c: u32,
}

struct AiTransformEntryV1 {
    transform_ref: u32,
    transform_kind: u8,
    function_or_template_ref: u32,
    input_digest_ref: u32,
    output_digest_ref: u32,
    parameter_payload_ref: u32,
    transform_digest_ref: u32,
    flags: u32,
    crc32c: u32,
}

struct AiPrivacySummaryEntryV1 {
    privacy_summary_ref: u32,
    source_binding_ref: u32,
    sensitivity_mask: u32,
    sensitivity_bits_ref: u32,
    policy_ref: u32,
    visibility_scope_ref: u32,
    redaction_scope_ref: u32,
    retention_state: u8,
    disclosure_state: u8,
    flags: u32,
    crc32c: u32,
}

struct AiReferenceEntryV1 {
    ref_id: u32,
    ref_space: u8,
    target_kind: u16,
    reserved0: u8,
    payload_ref: u32,
    digest_ref: u32,
    flags: u32,
    crc32c: u32,
}

struct AiPayloadIntegrityV1 {
    integrity_ref: u32,
    payload_ref: u32,
    digest_domain: u8,
    reserved0: u8,
    digest_algorithm: u16,
    digest_len: u16,
    digest_ref: u32,
    payload_crc32c: u32,
    flags: u32,
}

struct AiSectionFeatureBindingV1 {
    binding_ref: u32,
    section_id: u32,
    scope: u8,
    profile_kind: u8,
    operation_kind: u16,
    required_ai_features: u64,
    optional_ai_features: u64,
    target_local_ref: u64,
    flags: u32,
    crc32c: u32,
}
```

Binary `AI_*` descriptor sections are arrays of length-delimited records using
`AiRecordHeaderV1`. `record_len` includes the header and payload bytes.
`record_version` MUST be 1 for V1 records. COVE-VEC vector-binding records
`AI_VECTOR_BINDING` kinds 1 through 4 MAY use `record_version = 2` only for the
append-only model-input identity layouts defined by COVE-VEC. Records MUST be
wholly contained in the section payload and MUST NOT overlap. `local_id` is unique within
`(section_kind, record_kind)` unless a section-specific rule uses a wider
duplicate-ID scope. Unknown optional record kinds MAY be skipped after bounds
and CRC validation. Unknown required record kinds reject only the selected
section, profile, or operation whose requiredness scope intersects use.
Known record kinds with unsupported `record_version` values MUST reject.

`AI_PAYLOAD_BYTES` has no standard record-kind assignment. It is an opaque byte
carrier addressed only through `AiPayloadRefEntryV1` records and descriptors
that reference those payload refs.

`AiReferenceEntryV1` is an optional future directory record over typed
reference records. It is not required for the provider-free reference surface
and has no standard V1 record-kind assignment. Readers MUST use the typed
reference records above for standard COVE-AI semantics unless a required
extension defines and gates the directory record.

Common COVE-AI V1 flags are:

| Bit | Flag | Meaning |
| ---: | --- | --- |
| 0 | `AI_FLAG_REQUIRED_RECORD` | Unknown support rejects the selected section/profile/operation. |
| 1 | `AI_FLAG_PAYLOAD_CRC32C_PRESENT` | Payload CRC is present and MUST validate, even when the numeric CRC value is zero. |
| 2 | `AI_FLAG_POLICY_PROTECTED` | Payload or metadata requires policy validation before exposure. |
| 3 | `AI_FLAG_REVOKED` | Record or source binding is revoked for governed reads unless trusted policy overrides. |

Unassigned `flags` bits MUST be zero on write and MUST cause rejection when
non-zero in a required record, unless the record-specific flag registry defines
the bit. Optional records with unknown non-zero flags MAY be skipped and
reported.

Digest payloads used to validate `AI_PAYLOAD_INTEGRITY` MUST be readable after
bounds and CRC validation without depending on the same integrity record.
Cyclic integrity dependencies are invalid. Digest payload bytes are opaque
digest bytes and MUST NOT be text-normalized before comparison.

`AiPayloadRefEntryV1` storage-kind validation:

- `ArtifactAbsolute` uses `payload_offset` and `payload_length`; `section_id`,
  `section_payload_offset`, and `uri_ref` MUST be zero unless a required
  extension says otherwise.
- `SectionDecodedRelative` uses `section_id`, `section_payload_offset`, and
  `payload_length`; `payload_offset` and `uri_ref` MUST be zero.
- `ExternalUri` uses `uri_ref` and `payload_length` when known; artifact and
  section offsets MUST be zero.
- `EmbeddedSection` uses `section_id`, and the payload is the entire decoded
  section unless offsets are explicitly allowed by the section schema.

For provider-free token and vector payloads, a payload ref used by
`TokenBlockHeaderV1.payload_ref` or `VectorPayloadBlockHeaderV1.payload_ref`
MUST identify bytes contained in an `AI_PAYLOAD_BYTES` section unless a
required extension declares another payload carrier. It MUST NOT identify
bytes appended inside `AI_TOKEN_BLOCK` or `AI_VECTOR_PAYLOAD_BLOCK`.

`AiPrivacySummaryEntryV1` is valid only for its declared
`source_binding_ref`, `policy_ref`, `visibility_scope_ref`, and
`redaction_scope_ref`. A mismatch between the privacy summary and the selected
source binding or policy context makes all payload-bearing records under that
source binding policy-blocked.

Minimum standard record-kind assignments are:

| Section | Record kind | Meaning |
| --- | ---: | --- |
| `AI_REFERENCE_TABLES` | 1 | `AiStringEntryV1` |
| `AI_REFERENCE_TABLES` | 2 | `AiDigestEntryV1` |
| `AI_REFERENCE_TABLES` | 3 | `AiPayloadRefEntryV1` |
| `AI_REFERENCE_TABLES` | 4 | `AiPolicyRefEntryV1` |
| `AI_REFERENCE_TABLES` | 5 | `AiSourceSpanEntryV1` |
| `AI_REFERENCE_TABLES` | 6 | `AiTransformEntryV1` |
| `AI_COMPANION_ARTIFACT_REF` | 1 | `AiCompanionArtifactRefV1` |
| `AI_SOURCE_BINDING` | 1 | `AiSourceBindingV1` |
| `AI_PRIVACY_SUMMARY` | 1 | `AiPrivacySummaryEntryV1` |
| `AI_PAYLOAD_INTEGRITY` | 1 | `AiPayloadIntegrityV1` |
| `AI_SECTION_FEATURE_BINDING` | 1 | `AiSectionFeatureBindingV1` |
| `AI_CHUNK_PROFILE` | 1 | `ChunkProfileV1` |
| `AI_TEXT_CHUNK_INDEX` | 1 | `TextChunkEntryV1` |
| `AI_TOKENIZER_PROFILE` | 1 | `TokenizerProfileV1` |
| `AI_TOKEN_BLOCK` | 1 | `TokenBlockHeaderV1` |
| `AI_TOKENIZED_SPAN` | 1 | `TokenizedSpanV1` |
| `AI_TOKEN_SEQUENCE_PACK` | 1 | `TokenSequencePackV1` |
| `AI_VECTOR_SPACE` | 1 | `VectorSpaceDescriptorV1` |
| `AI_VECTOR_SPACE` | 2 | `VectorSpaceCompatibilityDescriptorV1` |
| `AI_VECTOR_BINDING` | 1 | `FileCodeVectorBindingV1` or `FileCodeVectorBindingV2` |
| `AI_VECTOR_BINDING` | 2 | `ChunkVectorBindingV1` or `ChunkVectorBindingV2` |
| `AI_VECTOR_BINDING` | 3 | `ObjectStateVectorBindingV1` or `ObjectStateVectorBindingV2` |
| `AI_VECTOR_BINDING` | 4 | `TrainingSampleVectorBindingV1` or `TrainingSampleVectorBindingV2` |
| `AI_VECTOR_PAYLOAD_BLOCK` | 1 | `VectorPayloadBlockHeaderV1` |
| `AI_VECTOR_DIRECTORY` | 1 | `VectorEntryV1` |
| `AI_VECTOR_COMPOSITION` | 1 | `VectorCompositionProfileV1` |
| `AI_VECTOR_COMPOSITION` | 2 | `VectorCompositionComponentV1` |
| `AI_VECTOR_COMPOSITION` | 3 | `VectorArithmeticProfileV1` |
| `AI_VECTOR_INDEX` | 1 | `VectorIndexDescriptorV1` |
| `AI_TENSOR_LAYOUT` | 1 | `TensorLayoutDescriptorV1` |
| `AI_TENSOR_LAYOUT` | 2 | `DeviceTransferHintV1` |
| `AI_ASSET_MANIFEST` | 1 | `AiAssetRefV1` |
| `AI_MULTIMODAL_SEQUENCE` | 1 | `MultimodalSequencePackV1` |
| `AI_MULTIMODAL_SEQUENCE` | 2 | `MultimodalSequenceElementV1` |
| `AI_TRAINING_PROFILE` | 1 | `TrainingProfileV1` |
| `AI_TRAINING_SAMPLE_INDEX` | 1 | `TrainingSampleEntryV1` |
| `AI_TRAINING_SPLIT_DEDUP_EPOCH` | 1 | `DatasetSplitV1` |
| `AI_TRAINING_SPLIT_DEDUP_EPOCH` | 2 | `DedupGroupV1` |
| `AI_TRAINING_SPLIT_DEDUP_EPOCH` | 3 | `TrainingEpochPlanV1` |
| `AI_LABEL_PREFERENCE` | 1 | `TrainingLabelEntryV1` |
| `AI_LABEL_PREFERENCE` | 2 | `PreferencePairEntryV1` |
| `AI_GENERATOR_PROVENANCE` | 1 | `GeneratorProvenanceV1` |
| `AI_GENERATOR_PROVENANCE` | 2 | `ModelActorDescriptorV1` |
| `AI_GENERATOR_PROVENANCE` | 3 | `GenerationDecodingProfileV1` |
| `AI_GENERATOR_PROVENANCE` | 4 | `HumanReviewEntryV1` |

### 83.8 Authority and Reproducibility

Authoritative surfaces remain COVE file structure, canonical values,
dictionaries, COVE-T rows, COVE-O reconstruction, COVE-MAP deterministic
mapping/projection truth, digest manifests, trust chains, redaction manifests,
visibility/policy context, and selected COVM publication state.

Derivative AI surfaces include chunks, token caches, vectors, vector indexes,
composed embeddings, prompt contexts, training samples, generated labels,
dedup groups, multimodal sequence packs, and ANN candidates.

```rust
enum AiReproducibilityClassV1 {
    DescriptiveOnly = 0,
    SourceSnapshotReproducible = 1,
    PreprocessingReproducible = 2,
    StoredPayloadVerifiable = 3,
    CanonicalRecomputeReproducible = 4,
    ExternalAuditOnly = 5,
    Extension = 255,
}
```

External model/API outputs SHOULD normally be `ExternalAuditOnly` unless the
model, weights, runtime, prompt, decoding, seed, toolchain, and deterministic
generation algorithm are sufficiently specified. Runtime floating-point vector
composition is advisory unless the result is materialized and digested or a
canonical fixed-point arithmetic profile is declared.

### 83.9 Fallback and Security

A missing, stale, corrupt, unsupported, or policy-blocked COVE-AI artifact MUST
NOT change COVE-T scan results, COVE-O reconstruction, or COVE-MAP projection
readback. If an AI operation explicitly requires COVE-AI metadata, the
operation MUST reject with structured diagnostics when the metadata is missing,
stale, unsupported, corrupt, or policy-blocked.

AI-specific leakage risks include vector leakage, token leakage, chunk-boundary
leakage, neighborhood leakage, dedup leakage, generated-label inferences,
ANN-distribution leakage, and prompt-context redaction leakage. A redacted or
forbidden value MUST NOT be exposed through chunk text, token IDs, vector
payloads, nearest-neighbor metadata, dedup metadata, multimodal elements, or
training exports unless a trusted policy explicitly allows it.

### 83.9.1 Export Interoperability

COVE-AI export adapters MAY emit JSON, JSONL, HF-style JSONL, Arrow IPC,
Parquet, WebDataset-like shards, DLPack views, or other model/runtime-facing
artifacts. These exports are interoperability surfaces, not COVE truth
authority. Every payload-aware export MUST read bytes through the shared
COVE-AI payload lease path and MUST preserve withheld-policy diagnostics as
records or report metadata.

Arrow IPC and Parquet exports MUST carry stable record identity, record kind,
payload-access summary, and enough serialized record metadata to audit each
row back to the validated `.coveai` or `.covev` sidecar. WebDataset exports
additionally MUST validate shard lifetime and policy scope. DLPack-style APIs
MUST validate tensor lifetime, dtype, shape, stride, alignment, dense
uncompressed layout, and policy scope before exposing bytes or zero-copy views;
CLIs MAY defer direct DLPack file emission until they can preserve those
lifetime guarantees.

### 83.10 Conformance and Benchmarks

COVE-AI conformance tiers are defined in
[`docs/governance/conformance-levels.md`](../../docs/governance/conformance-levels.md).
The generated capability matrix is the implementation-status record.

Required release gates for the provider-free reference surface are:

```sh
cargo test --workspace
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```

AI benchmark reports SHOULD include vector dedup ratio, embedding cost avoided,
tokenization cost avoided, chunk reuse ratio, training stream throughput,
multimodal assembly latency, RAG retrieval latency, snapshot verification cost,
tensor zero-copy/materialization rates, storage overhead, stale sidecar
rejection, redaction leakage checks, split reproducibility, and generator
filtering correctness.

The provider-free reference benchmark harness MUST include at least one
deterministic COVE-AI vector sidecar case that reports vector build latency,
sidecar parse latency, exact vector-search latency, internal ANN candidate
search or exact-fallback latency, recall versus exact scan when an approximate
implementation is enabled, fallback rate, filtered top-k completeness, payload
bytes read, and policy-withheld counts.
