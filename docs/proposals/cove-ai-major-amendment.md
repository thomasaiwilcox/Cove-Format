# COVE-AI Major Amendment

Status: draft proposal

Owning profiles: COVE-AI / COVE-MAP-AI / COVE-CHUNK / COVE-TOK / COVE-VEC / COVE-MMSEQ / COVE-TRAIN / CoveQL-AI

Feature scope: optional standards-suite extension

Primary goal: make Cove the natural archive format for AI data: queryable,
vector-ready, chunk-aware, token-aware, training-ready, multimodal,
reproducible, and proof-aware.

Compatibility statement: COVE-AI does not reduce or alter COVE-Core, COVE-T,
COVE-O, or COVE-MAP logical truth. Baseline readers may ignore COVE-AI
artifacts safely.

## Summary

COVE-AI adds an optional AI layer to the COVE standards suite for semantic
querying, RAG, training-data storage, synthetic-data provenance, and large-scale
AI corpus management.

The central design rule is that Cove should not blindly store row-level
embeddings. Cove should dictionary-encode semantics: each distinct canonical
value, semantic slot value, chunk, token span, object state, or training sample
is vectorized, tokenized, chunked, labeled, or referenced once, then reused
through COVE's existing identity, FileCode, CSN, mapping, digest, and evidence
model.

If `red` appears in a product table 100,000 times, Cove should not vectorize
`red` 100,000 times. It should store one slot-aware vector for:

```text
Product.color = "red"
```

Every row using that FileCode can then reference the same vector.

The golden rule is:

> COVE-AI artifacts are derivative. Canonical values, COVE-T rows, COVE-O object
> state, COVE-MAP evidence, digests, trust chains, visibility, and redaction
> remain the source of truth. AI artifacts may accelerate, enrich, index,
> tokenize, vectorize, sample, or explain that truth, but they must not silently
> redefine it.

This preserves the existing COVE authority model: the baseline `.cove` file is
engine-neutral, optional acceleration can help engines read the same truth more
cheaply, and acceleration must never change logical results.

## Motivation

The AI data stack is fragmented across raw archives, analytical storage,
semantic mapping jobs, RAG chunkers, model-specific tokenization, vector
databases, training shards, governance catalogs, prompt logs, and experiment
trackers.

Cove already has the right foundation for a unified AI archive:

- immutable archives;
- FileCodes and dictionaries;
- object history and CSNs;
- COVE-MAP evidence;
- digest manifests and trust chains;
- optional indexes and layout metadata;
- CoveQL.

COVE-AI makes those primitives directly useful for AI querying and
training-data storage. The goal is not to turn Cove into a vector database,
model runtime, or lakehouse transaction protocol. The goal is to make Cove the
format that can answer:

- What data did this AI system see?
- Which exact snapshot, branch, CSN, object state, and source rows?
- Which mapping rules, chunks, tokenizer, vectors, and vectorizer?
- Which examples were in train versus eval?
- Which model generated this label or preference judgment?
- Which values were redacted, forbidden, approximate, stale, or ignored?

## Non-Goals

COVE-AI does not:

- make vectors canonical logical truth;
- require every Cove reader to implement AI features;
- require any embedding model, tokenizer, vector database, GPU, or ML framework;
- replace Arrow, Parquet, PyTorch, TensorFlow, JAX, Hugging Face, or vector
  databases;
- guarantee deterministic model training;
- guarantee deterministic external API-generated synthetic data;
- solve access control or encryption by itself;
- make approximate ANN indexes exact;
- make runtime floating-point composition cryptographically reproducible by
  default.

COVE-AI should make the data, preprocessing, derived artifacts, and lineage
reproducible and auditable. It should not claim that stochastic model training
itself becomes deterministic merely because the input archive is verifiable.

## Design Principles

### AI Surfaces Are Derivative

Chunks, tokens, embeddings, ANN indexes, generated labels, quality scores, and
training samples are derivative AI surfaces unless another profile explicitly
declares them as canonical logical values.

A missing, stale, unsupported, corrupt, or policy-blocked AI sidecar must not
change ordinary COVE-T scans, COVE-O reconstruction, or COVE-MAP projection
readback.

### Vectorize Distinct Semantic Units Once

COVE-VEC is built around deduplicated vectorization of distinct canonical
values, slot-aware values, chunks, tokenized spans, object states, association
states, evidence spans, training samples, prompt contexts, multimodal assets,
and multimodal sequences.

### Semantic Slot Beats Raw Literal

The value `red` is not always semantically identical. COVE-AI must support:

```text
Raw value vector:
  "red"

Slot-aware vector:
  Product.color = "red"

Template-aware vector:
  "The product colour is red."
```

Default AI quality should prefer slot-aware or template-aware vectors.

### COVE-MAP Owns AI Intent

The decision to vectorize, tokenize, chunk, ignore, forbid, sample, or label a
field belongs in COVE-MAP-AI because COVE-MAP understands source semantics,
object properties, associations, evidence, projection fields, governance, and
semantic roles.

### CoveQL-AI Is a Profile

CoveQL-Core should remain small and strict. AI methods such as `.similar()`,
`.embedding()`, `.chunks()`, `.tokens()`, and `.trainingSamples()` belong in
CoveQL-AI, not CoveQL-Core.

### Reproducibility Must Be Classified

COVE-AI must distinguish these claims:

- we know what happened;
- we can select the same source snapshot;
- we can replay preprocessing;
- we can verify stored derived bytes;
- we can recompute derived bytes exactly;
- we can only audit an external model/API output.

### Floating-Point Composition Is Advisory Unless Made Deterministic

Runtime floating-point vector composition is useful but not bit-reproducible
across hardware by default. COVE-AI must explicitly classify dynamic composed
vectors as advisory unless they are materialized and digested or computed under
a strict deterministic arithmetic profile.

## Standards-Suite Map

Add the following optional standards:

| Part | Profile | Purpose |
| --- | --- | --- |
| 16 | COVE-AI Overview | Authority model, reproducibility classes, privacy boundary, conformance tiers, and relationship to existing COVE profiles. |
| 17 | COVE-MAP-AI | AI semantic slot roles, vectorization intent, chunk/token policy, sample policy, label policy, governance annotations, and composition participation. |
| 18 | COVE-CHUNK | Text/document chunk spans, byte/token offsets, hierarchy, context navigation, chunker lineage, source-value binding, and evidence binding. |
| 19 | COVE-TOK | Tokenizer profiles, token caches, token blocks, token-to-byte alignment, sequence packs, masks, labels, and token stream export. |
| 20 | COVE-VEC | Vector spaces, dictionary-encoded value vectors, chunk/object/sample vectors, vector payload blocks, composition profiles, arithmetic profiles, indexes, quantization, tensor layout, and device-transfer hints. |
| 21 | COVE-MMSEQ | Interleaved multimodal model-consumable sequences over text, tokens, image, audio, video, tensors, tools, labels, and control markers. |
| 22 | COVE-TRAIN | Training/evaluation sample indexes, deterministic splits, labels, preference pairs, generated outputs, weights, quality scores, dedup groups, epoch plans, and synthetic provenance. |
| 23 | CoveQL-AI | Query language profile for semantic search, embedding composition, RAG context, token/sample export, multimodal sequence reads, synthetic-data filtering, and AI explain output. |

All are optional. A baseline COVE reader may ignore them safely.

## Authority Model

### Authoritative Surfaces

COVE-AI does not redefine the existing source of truth. Authoritative surfaces
remain:

- COVE file header, footer, section directory, checksums, and feature
  declarations;
- COVE canonical logical values and dictionaries;
- COVE-T table data;
- COVE-O object reconstruction;
- COVE-MAP deterministic mapping semantics, evidence, and projection readback
  when requested;
- digest manifests;
- trust chains;
- redaction manifests;
- visibility and policy context;
- selected COVM publication state.

### Derivative AI Surfaces

Derivative AI surfaces include:

- chunk indexes;
- token caches and sequence packs;
- vector sidecars and vector indexes;
- composed row/object embeddings;
- ANN candidates and retrieval rankings;
- prompt-context assembly;
- training sample indexes and dataset splits;
- generated labels, model-judged preference scores, quality scores, and dedup
  similarity groups;
- multimodal sequence packs.

### Normative Rules

A COVE-AI reader MUST NOT treat a vector, token cache, chunk boundary, training
sample, generated label, or ANN result as canonical source truth unless another
COVE profile explicitly defines that field as a canonical logical value.

A missing, stale, corrupt, unsupported, or policy-blocked COVE-AI artifact MUST
NOT change COVE-T scan results, COVE-O reconstruction, or COVE-MAP projection
readback.

If an AI operation explicitly requires a COVE-AI artifact, the operation MUST
reject with structured diagnostics when the artifact is missing, stale,
unsupported, corrupt, or policy-blocked.

AI explain output MUST disclose, subject to policy, which AI artifacts were
used, what their freshness status was, whether the operation was exact or
approximate, and what fallback occurred.

## AI Reproducibility Classes

```rust
enum AiReproducibilityClassV1 {
    DescriptiveOnly = 0,
    SourceSnapshotReproducible = 1,
    PreprocessingReproducible = 2,
    PayloadByteReproducible = 3,
    CanonicalRecomputeReproducible = 4,
    ExternalAuditOnly = 5,
    Extension = 255,
}
```

| Class | Meaning |
| --- | --- |
| `DescriptiveOnly` | Metadata describes what happened but does not support replay. |
| `SourceSnapshotReproducible` | The same source COVE/COVM snapshot, CSN, branch, mapping version, and evidence can be selected again. |
| `PreprocessingReproducible` | Chunking, tokenization, filtering, sample selection, and splits can be replayed from deterministic profiles. |
| `PayloadByteReproducible` | Derived artifact bytes are stored and digest-verified. |
| `CanonicalRecomputeReproducible` | Independent implementations can recompute the same derived bytes under a strict canonical algorithm. |
| `ExternalAuditOnly` | External model/API/tool provenance is recorded, but deterministic regeneration is not claimed. |

COVE-AI metadata MUST NOT imply a stronger reproducibility class than its
declared lineage supports.

External model/API outputs SHOULD normally be `ExternalAuditOnly` unless the
model, weights, runtime, prompt, decoding, seed, toolchain, and deterministic
generation algorithm are sufficiently specified.

Runtime floating-point vector composition SHOULD normally be `RuntimeAdvisory`
and MUST NOT claim `PayloadByteReproducible` unless the result is materialized
and digested.

Canonical fixed-point vector composition MAY claim
`CanonicalRecomputeReproducible` when all arithmetic rules are declared.

## COVE-MAP-AI

COVE-MAP-AI tells AI ingestion and query engines which semantic slots matter.
It stores intent and policy. COVE-VEC stores vectors. COVE-CHUNK stores chunks.
COVE-TOK stores tokenization. COVE-TRAIN stores samples.

It answers:

- Should this field be vectorized, chunked, tokenized, or sampled?
- Should it participate in object embedding composition?
- Should it be ignored or forbidden?
- Is this a title, description, category, identifier, timestamp, label, prompt,
  completion, tool call, protected field, or safety label?

### Slot Roles

```rust
enum AiSlotRoleV1 {
    Unknown = 0,
    NaturalLanguageLong = 1,
    NaturalLanguageShort = 2,
    Title = 3,
    Summary = 4,
    Label = 5,
    Category = 6,
    Tag = 7,
    Name = 8,
    Identifier = 9,
    Code = 10,
    Boolean = 11,
    Timestamp = 12,
    NumericMeasure = 13,
    Ordinal = 14,
    Geo = 15,
    Url = 16,
    Email = 17,
    OpaqueJson = 18,
    Binary = 19,
    ImageRef = 20,
    AudioRef = 21,
    VideoRef = 22,
    DocumentRef = 23,
    Redacted = 24,
    PolicyProtected = 25,
    Prompt = 26,
    Completion = 27,
    Instruction = 28,
    ToolCall = 29,
    ToolResult = 30,
    SafetyLabel = 31,
    PreferenceLabel = 32,
    QualityScore = 33,
    Extension = 255,
}
```

Recommended defaults:

- usually vectorize natural language, titles, summaries, labels, categories,
  tags, and names when policy permits;
- usually chunk long natural language, document references, prompts, and
  completions;
- usually tokenize long natural language, prompts, completions, instructions,
  and chat transcripts;
- usually ignore identifiers, code, booleans, timestamps, numeric measures, and
  binary values;
- require explicit opt-in for email, URLs, opaque JSON, protected fields,
  redacted values, private notes, and sensitive free text;
- usually sample prompts, completions, instructions, tool calls, tool results,
  labels, preference labels, safety labels, and curated long natural language.

### Slot Decisions

```rust
enum AiSlotDecisionV1 {
    Ignore = 0,
    VectorizeDistinctValues = 1,
    VectorizeSlotValues = 2,
    ChunkThenVectorize = 3,
    Tokenize = 4,
    ChunkAndTokenize = 5,
    ComposeOnly = 6,
    LabelOnly = 7,
    SampleOnly = 8,
    VectorizeAndSample = 9,
    TokenizeAndSample = 10,
    PolicyDefined = 11,
    Forbidden = 12,
    Extension = 255,
}
```

`Forbidden` means conforming COVE-AI tooling MUST NOT vectorize, tokenize,
chunk, sample, export, or include the slot in AI sidecars unless a
higher-priority explicit policy authorizes it.

`Ignore` means the slot is not selected by the declared profile, but another
explicit profile MAY select it.

`PolicyDefined` means the slot's role and sensitivity are declared, but runtime
policy decides whether and how it can be used.

### Sensitivity

```rust
enum AiSensitivityV1 {
    Unknown = 0,
    Public = 1,
    Internal = 2,
    Confidential = 3,
    Sensitive = 4,
    PersonalData = 5,
    Secret = 6,
    Redacted = 7,
    PolicyProtected = 8,
    Forbidden = 9,
    Extension = 255,
}
```

### Vector Granularity

```rust
enum AiVectorGranularityV1 {
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
```

### Core Structures

```rust
struct MapAiProfileV1 {
    profile_id: u32,
    profile_name_ref: u32,
    default_decision: u8,
    default_granularity: u8,
    default_role: u8,
    default_sensitivity: u8,
    first_slot_ref: u32,
    slot_count: u32,
    first_template_ref: u32,
    template_count: u32,
    first_composition_ref: u32,
    composition_count: u32,
    first_training_policy_ref: u32,
    training_policy_count: u32,
    flags: u32,
    checksum: u32,
}

struct MapAiSlotPolicyV1 {
    slot_policy_id: u32,
    source_ref: u32,
    table_id: u32,
    column_id: u32,
    object_type_id: u32,
    property_id: u32,
    association_type_id: u32,
    path_ref: u32,
    role: u8,
    decision: u8,
    granularity: u8,
    sensitivity: u8,
    vector_space_ref: u32,
    template_ref: u32,
    chunk_profile_ref: u32,
    tokenizer_profile_ref: u32,
    training_policy_ref: u32,
    composition_weight_ppm: u32,
    min_distinct_count: u32,
    max_distinct_count: u32,
    max_value_bytes: u32,
    evidence_policy_ref: u32,
    license_policy_ref: u32,
    redaction_policy_ref: u32,
    flags: u32,
    checksum: u32,
}

struct AiVectorTemplateV1 {
    template_id: u32,
    template_kind: u8,
    template_text_ref: u32,
    locale_ref: u32,
    deterministic: u8,
    flags: u32,
    checksum: u32,
}
```

A template used for vectorization MUST be included in vector lineage. Changing
the template changes the vectorization profile fingerprint. Vectors generated
with different template fingerprints MUST NOT be compared unless a compatibility
descriptor explicitly permits it.

Example:

```yaml
ai_profiles:
  - id: product_ai_v1
    slots:
      - path: Product.name
        role: Title
        decision: VectorizeSlotValues
        template: "Product name: {value}"
        weight: 0.20
      - path: Product.description
        role: NaturalLanguageLong
        decision: ChunkThenVectorize
        chunk_profile: paragraph_or_256_tokens
        weight: 0.60
      - path: Product.category
        role: Category
        decision: VectorizeSlotValues
        template: "Product category: {value}"
        weight: 0.15
      - path: Product.color
        role: Category
        decision: VectorizeSlotValues
        template: "Product colour: {value}"
        weight: 0.05
      - path: Product.sku
        role: Identifier
        decision: Ignore
      - path: Customer.private_note
        role: PolicyProtected
        decision: Forbidden
```

## COVE-CHUNK

COVE-CHUNK stores reusable chunk metadata so AI systems do not repeatedly split
the same text into incompatible spans. It supports sentence chunks, paragraph
chunks, section chunks, heading-aware chunks, fixed token windows, sliding token
windows, semantic spans, document pages, OCR spans, transcript segments,
parent/child context navigation, source-value binding, and evidence binding.

```rust
struct ChunkProfileV1 {
    chunk_profile_id: u32,
    profile_name_ref: u32,
    chunker_namespace_ref: u32,
    chunker_name_ref: u32,
    chunker_version_major: u16,
    chunker_version_minor: u16,
    tokenizer_profile_ref: u32,
    boundary_kind: u8,
    overlap_policy: u8,
    parent_policy: u8,
    normalization_policy: u8,
    target_tokens: u32,
    min_tokens: u32,
    max_tokens: u32,
    overlap_tokens: u32,
    max_bytes: u32,
    locale_ref: u32,
    flags: u32,
    checksum: u32,
}

struct TextChunkEntryV1 {
    chunk_id: u64,
    source_ref: u32,
    table_id: u32,
    column_id: u32,
    object_type_id: u32,
    property_id: u32,
    association_type_id: u32,
    path_ref: u32,
    source_row_ref: u64,
    source_object_ref: u64,
    source_value_hash_ref: u32,
    byte_start: u64,
    byte_length: u64,
    char_start: u64,
    char_length: u64,
    token_start: u64,
    token_count: u32,
    parent_chunk_id: u64,
    first_child_ref: u32,
    child_count: u32,
    previous_chunk_id: u64,
    next_chunk_id: u64,
    chunk_text_hash_ref: u32,
    evidence_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}
```

Rules:

- byte spans over UTF-8 text MUST align to valid UTF-8 boundaries;
- a chunk entry MUST bind to a source value hash so stale chunk indexes can be
  detected after rewriting, remapping, redaction, or normalization changes;
- chunk hierarchy is advisory for retrieval, but chunk boundaries MUST validate
  against the source value before use;
- redaction policy applies before exposing chunk text, chunk hashes, token
  alignment, neighboring context, or parent/child navigation;
- a reader MUST NOT infer semantic document structure from raw text alone unless
  the chunk profile declares the structure and validation rules.

COVE-CHUNK should support matched-span context expansion over matched chunks,
parents, siblings, previous/next chunks, same heading, same object, same source
evidence, and context policies such as before/after N chunks or parent section.

## COVE-TOK

COVE-TOK stores tokenization outputs so training and retrieval pipelines do not
repeatedly tokenize the same source text. COVE-CHUNK is model-independent text
segmentation; COVE-TOK is tokenizer/model-specific tokenization and sequence
packing.

```rust
struct TokenizerProfileV1 {
    tokenizer_profile_id: u32,
    tokenizer_namespace_ref: u32,
    tokenizer_name_ref: u32,
    tokenizer_version_major: u16,
    tokenizer_version_minor: u16,
    vocab_digest_ref: u32,
    merges_digest_ref: u32,
    normalizer_digest_ref: u32,
    special_tokens_digest_ref: u32,
    token_id_width: u8,
    byte_alignment_available: u8,
    reversible: u8,
    deterministic: u8,
    bos_token_id: u32,
    eos_token_id: u32,
    pad_token_id: u32,
    unk_token_id: u32,
    flags: u32,
    checksum: u32,
}

struct TokenBlockHeaderV1 {
    token_block_id: u32,
    tokenizer_profile_id: u32,
    token_count: u64,
    token_id_width: u8,
    compression_codec: u8,
    layout_kind: u8,
    payload_offset: u64,
    payload_length: u64,
    checksum: u32,
}

struct TokenizedSpanV1 {
    tokenized_span_id: u64,
    chunk_id: u64,
    tokenizer_profile_id: u32,
    token_block_ref: u32,
    token_offset: u64,
    token_count: u32,
    byte_alignment_ref: u32,
    source_value_hash_ref: u32,
    flags: u32,
    checksum: u32,
}

struct TokenSequencePackV1 {
    sequence_pack_id: u64,
    tokenizer_profile_id: u32,
    training_profile_ref: u32,
    token_block_ref: u32,
    token_offset: u64,
    token_count: u32,
    source_span_count: u32,
    first_source_span_ref: u32,
    loss_mask_ref: u32,
    attention_mask_ref: u32,
    position_ids_ref: u32,
    labels_ref: u32,
    split_ref: u32,
    sample_weight_ppm: u32,
    flags: u32,
    checksum: u32,
}
```

A tokenizer cache MUST bind to tokenizer profile identity and digest material.
It MUST NOT be reused across incompatible tokenizer profiles. If token IDs are
exposed, redaction and policy checks MUST be applied first because token IDs may
leak source text. Loss masks and labels MUST be unambiguously scoped to token
positions.

## COVE-VEC

COVE-VEC stores vectors once per distinct semantic unit and lets rows, objects,
chunks, samples, and multimodal assets reference them.

Recommended artifact:

```text
Extension: .covev
Magic: CVV2
Profile: COVE-VEC
```

COVE-VEC may also be embedded in COVX or COVE-I-style extension artifacts for
implementations that do not initially want a separate file type.

### Vector Space

```rust
struct VectorSpaceDescriptorV1 {
    vector_space_id: u32,
    vector_space_name_ref: u32,
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
```

Vectors from different `vector_space_id` values MUST NOT be compared unless a
compatibility descriptor explicitly permits it. Metric and normalization policy
are part of vector-space identity. If query-time embedding is performed through
an external service, the operation context MUST record service/model identity
and whether the result is reproducible or audit-only.

### Binding Kinds

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
```

### FileCode Vector Binding

This is the defining feature of COVE-VEC.

```rust
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
    file_code: u64,
    canonical_value_hash_ref: u32,
    vector_ref: u64,
    flags: u32,
    checksum: u32,
}
```

Inside one validated file/dictionary scope, `file_code` is the fast lookup key.
Across files, datasets, manifests, or rewritten artifacts, readers MUST use
dictionary digest, canonical value hash, schema/path binding, or an explicit
code-domain bridge.

A raw FileCode from another file MUST NOT be used as a vector key unless the
plan proves a shared code domain or validates an equivalent canonical binding.

A vector binding MUST include enough slot/profile/model lineage to distinguish
raw value vectors from slot-aware and template-aware vectors.

### Other Vector Bindings

```rust
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

### Vector Payloads

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
    payload_offset: u64,
    payload_length: u64,
    checksum: u32,
}
```

Recommended logical exports include `FixedSizeList<Float32>`,
`FixedSizeList<Float16>`, `FixedSizeBinary` for quantized vectors, Arrow
extension types for tensor/quantized/PQ layouts, and DLPack-compatible export
when tensor layout and lifetime allow.

### Vector Composition

```rust
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
```

Example:

```yaml
composition_profiles:
  - id: product_embedding_v1
    method: weighted_average
    result_authority: RuntimeAdvisory
    normalize_inputs: true
    normalize_output: true
    output: product_semantic_1536
    slots:
      - path: Product.name
        weight: 0.20
      - path: Product.category
        weight: 0.15
      - path: Product.description
        weight: 0.60
      - path: Product.color
        weight: 0.05
```

## Floating-Point Safety

Runtime composition over `f32`, `f16`, `bf16`, SIMD, GPU kernels, fused
multiply-add, and different addition orders can produce different least
significant bits across hardware. COVE-AI must not let ordinary runtime
floating-point math become a cryptographic truth claim.

```rust
enum VectorResultAuthorityV1 {
    RuntimeAdvisory = 0,
    PersistedPayloadDigest = 1,
    CanonicalFixedPointRecompute = 2,
    ExactExternalProof = 3,
    Extension = 255,
}
```

| Authority | Meaning |
| --- | --- |
| `RuntimeAdvisory` | The vector may be composed at query time using ordinary runtime math. It may be used for search, ranking, display, and approximate AI operations, but MUST NOT be treated as byte-reproducible across implementations. |
| `PersistedPayloadDigest` | The composed vector is stored as bytes in COVE-VEC. The digest covers stored vector payload bytes and dependency fingerprint. |
| `CanonicalFixedPointRecompute` | The vector may be recomputed bit-for-bit by independent implementations under a strict integer/fixed-point arithmetic profile. |
| `ExactExternalProof` | Reserved for a future extension that provides formal proof or certified deterministic computation. |

```rust
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

A dynamically composed vector produced with ordinary runtime floating-point
arithmetic has `RuntimeAdvisory` authority unless the vector is materialized and
digested or the composition profile declares a deterministic arithmetic profile.

`RuntimeAdvisory` composed vectors MUST NOT be included in digest manifests,
trust chains, conformance vectors, or reproducibility claims as expected vector
bytes.

If a composition profile claims `CanonicalFixedPointRecompute`, it MUST define
input quantization, component ordering, integer weight scale, accumulator width,
rounding mode, overflow behavior, normalization behavior, output quantization,
and exact tie-breaking rules.

Recommended defaults:

| Artifact | Default authority or reproducibility |
| --- | --- |
| Persisted value vectors | `PayloadByteReproducible` |
| Persisted chunk vectors | `PayloadByteReproducible` |
| Persisted object vectors | `PayloadByteReproducible` |
| Runtime-composed vectors | `RuntimeAdvisory` |
| Canonical fixed-point composed vectors | `CanonicalRecomputeReproducible` |
| ANN candidate rankings | `RuntimeAdvisory` unless exact index semantics are proven |

## Vector Indexes

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
    index_kind: u8,
    exactness_kind: u8,
    false_negative_policy: u8,
    metric: u8,
    dimension_count: u32,
    indexed_binding_kind: u8,
    temporal_scope_ref: u32,
    visibility_scope_ref: u32,
    redaction_scope_ref: u32,
    payload_ref: u32,
    checksum: u32,
}
```

Approximate vector indexes MAY return candidates. They MUST NOT claim complete
nearest-neighbor results unless their descriptor proves exactness for the
requested metric and query class. A semantic search operation MUST disclose
approximate/exact status in explain output. An index that may have false
negatives MUST NOT be used as proof that no matching vector, object, or chunk
exists unless the operation explicitly accepts approximate recall.

## Tensor and Vector Layouts

AI training systems often care about physical layout: dtype, shape, rank,
strides, storage offset, alignment, channels-first versus channels-last,
blocked/tiled layouts, sparse layouts, quantized layouts, and device-transfer
friendliness. COVE-AI should support these as validated layout/export metadata,
not as mandatory hardware dependencies.

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
policy, or source evidence. Zero-copy export MAY be used only after validating
payload bounds, dtype, shape, strides, alignment, compression state,
quantization profile, lifetime, visibility/redaction policy, and target runtime
compatibility. If validation fails, the reader MUST materialize a safe owned
output buffer or reject the operation with diagnostics.

## AI Assets

```rust
struct AiAssetRefV1 {
    asset_ref_id: u64,
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
    tensor_layout_ref: u32,
    license_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}
```

External assets MUST be fingerprinted by digest if replayability or training
reproducibility is claimed. A URI alone is not stable source identity. Derived
captions, OCR text, transcripts, embeddings, and labels SHOULD bind back to the
source asset digest and generator provenance where applicable.

## COVE-MMSEQ

COVE-MMSEQ preserves the order and semantics of model-consumable multimodal
sequences such as system text, user text, image, assistant text, audio clip,
tool call, tool result, assistant text, and label.

```rust
struct MultimodalSequencePackV1 {
    sequence_pack_id: u64,
    training_profile_id: u32,
    tokenizer_profile_id: u32,
    sequence_profile_ref: u32,
    element_count: u32,
    first_element_ref: u32,
    split_ref: u32,
    sample_weight_ppm: u32,
    loss_mask_ref: u32,
    attention_mask_ref: u32,
    position_map_ref: u32,
    label_ref: u32,
    source_snapshot_ref: u32,
    evidence_ref: u32,
    generator_provenance_ref: u64,
    flags: u32,
    checksum: u32,
}

struct MultimodalSequenceElementV1 {
    element_id: u64,
    sequence_pack_id: u64,
    ordinal: u32,
    element_kind: u8,
    modality: u8,
    role: u8,
    tokenized_span_ref: u64,
    token_sequence_pack_ref: u64,
    asset_ref: u64,
    tensor_ref: u64,
    vector_ref: u64,
    byte_start: u64,
    byte_length: u64,
    time_start_us: i64,
    time_duration_us: i64,
    position_stream_ref: u32,
    evidence_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}
```

The `ordinal` field is authoritative for sequence order. Asset references
inside a multimodal sequence MUST be digest-bound if replayability is claimed.
Loss masks, attention masks, labels, and role markers MUST be scoped to
sequence elements or token ranges unambiguously. A reader that does not support
COVE-MMSEQ MAY still expose referenced assets, text spans, token spans, and
evidence independently, but it MUST NOT claim to reconstruct the
model-consumable sequence.

Example:

```yaml
multimodal_sequence:
  id: support_visual_qa_001
  elements:
    - ordinal: 0
      role: system
      kind: text_span
      text: "You are a support assistant."
    - ordinal: 1
      role: user
      kind: text_span
      text: "Why is this device showing an error?"
    - ordinal: 2
      role: user
      kind: image_asset
      asset_ref: image_7
      transform: product_support_image_v1
    - ordinal: 3
      role: assistant
      kind: text_token_run
      tokenized_span_ref: answer_tokens_42
      loss_mask: train
```

## COVE-TRAIN

COVE-TRAIN makes Cove a portable archive for AI training and evaluation corpora.
It supports pretraining spans, instruction-response examples, chat transcripts,
tool traces, preference pairs, ranking lists, classification, extraction,
summarization, translation, code samples, multimodal examples, evaluation items,
safety/red-team items, generated labels, human-reviewed labels, and synthetic
teacher outputs.

```rust
struct TrainingProfileV1 {
    training_profile_id: u32,
    profile_name_ref: u32,
    task_family: u8,
    modality_mask: u32,
    source_snapshot_ref: u32,
    map_profile_ref: u32,
    chunk_profile_ref: u32,
    tokenizer_profile_ref: u32,
    vector_space_ref: u32,
    multimodal_sequence_profile_ref: u32,
    split_policy_ref: u32,
    sampling_policy_ref: u32,
    dedup_policy_ref: u32,
    quality_policy_ref: u32,
    license_policy_ref: u32,
    redaction_policy_ref: u32,
    default_generator_provenance_ref: u64,
    reproducibility_class: u8,
    flags: u32,
    checksum: u32,
}

enum TrainingExampleKindV1 {
    PretrainingSpan = 0,
    InstructionResponse = 1,
    ChatTranscript = 2,
    PreferencePair = 3,
    RankingList = 4,
    Classification = 5,
    Extraction = 6,
    Summarization = 7,
    Translation = 8,
    ToolTrace = 9,
    CodeSample = 10,
    MultimodalExample = 11,
    EvaluationItem = 12,
    SafetyItem = 13,
    SyntheticDistillation = 14,
    ModelJudgedPreference = 15,
    Extension = 255,
}

struct TrainingSampleEntryV1 {
    sample_id: u64,
    training_profile_id: u32,
    example_kind: u8,
    split_ref: u32,
    source_ref: u32,
    evidence_ref: u32,
    input_ref: u32,
    target_ref: u32,
    label_ref: u32,
    metadata_ref: u32,
    token_sequence_pack_ref: u32,
    multimodal_sequence_pack_ref: u64,
    vector_ref: u64,
    quality_score_ppm: u32,
    sample_weight_ppm: u32,
    dedup_group_ref: u32,
    license_ref: u32,
    policy_ref: u32,
    teacher_model_ref: u32,
    generator_provenance_ref: u64,
    judge_generator_provenance_ref: u64,
    label_generator_provenance_ref: u64,
    flags: u32,
    checksum: u32,
}
```

### Splits, Dedup, and Epoch Plans

```rust
struct DatasetSplitV1 {
    split_id: u32,
    split_name_ref: u32,
    split_method: u8,
    seed: u64,
    hash_function_ref: u32,
    stratification_path_ref: u32,
    grouping_ref: u32,
    sample_count: u64,
    first_sample_ref: u64,
    flags: u32,
    checksum: u32,
}

struct DedupGroupV1 {
    dedup_group_id: u64,
    dedup_policy_ref: u32,
    canonical_member_sample_id: u64,
    similarity_kind: u8,
    confidence_ppm: u32,
    first_member_ref: u32,
    member_count: u32,
    flags: u32,
    checksum: u32,
}

struct TrainingEpochPlanV1 {
    epoch_plan_id: u64,
    training_profile_id: u32,
    split_ref: u32,
    seed: u64,
    permutation_kind: u8,
    shard_count: u32,
    first_shard_ref: u32,
    shard_count_ref: u32,
    flags: u32,
    checksum: u32,
}
```

A split that claims reproducibility MUST declare its source snapshot, hash/seed
policy, filters, grouping, dedup behavior, and ordering. Training/evaluation
leakage controls SHOULD be represented through dedup groups, source grouping,
benchmark exclusion lists, and policy metadata.

### Labels and Preferences

```rust
struct TrainingLabelEntryV1 {
    label_id: u64,
    label_kind: u8,
    label_payload_ref: u32,
    generator_provenance_ref: u64,
    human_review_ref: u32,
    confidence_ppm: u32,
    evidence_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}

struct PreferencePairEntryV1 {
    preference_pair_id: u64,
    prompt_ref: u32,
    chosen_ref: u32,
    rejected_ref: u32,
    judge_generator_provenance_ref: u64,
    human_review_ref: u32,
    preference_strength_ppm: u32,
    confidence_ppm: u32,
    evidence_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}
```

## Synthetic Teacher and Generator Provenance

COVE-TRAIN must track which model generated an answer, which model judged a
preference pair, which model generated a label, which prompt/template and
decoding parameters were used, what source context was included, whether output
was human-reviewed, and how to filter out samples generated by a deprecated
model.

```rust
struct GeneratorProvenanceV1 {
    generator_provenance_id: u64,
    generator_kind: u8,
    model_actor_ref: u32,
    prompt_template_ref: u32,
    decoding_profile_ref: u32,
    toolchain_ref: u32,
    source_input_ref: u32,
    source_context_ref: u32,
    source_sample_ref: u64,
    parent_generator_provenance_ref: u64,
    generation_time_us: i64,
    confidence_ppm: u32,
    human_review_ref: u32,
    policy_ref: u32,
    reproducibility_class: u8,
    flags: u32,
    checksum: u32,
}

struct ModelActorDescriptorV1 {
    model_actor_id: u32,
    model_namespace_ref: u32,
    model_name_ref: u32,
    model_version_ref: u32,
    model_checkpoint_digest_ref: u32,
    provider_ref: u32,
    endpoint_ref: u32,
    endpoint_version_ref: u32,
    model_family_ref: u32,
    modality_mask: u32,
    license_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}

struct GenerationDecodingProfileV1 {
    decoding_profile_id: u32,
    temperature_micros: u32,
    top_p_micros: u32,
    top_k: u32,
    seed: u64,
    max_output_tokens: u32,
    stop_sequence_ref: u32,
    safety_policy_ref: u32,
    deterministic_claim: u8,
    flags: u32,
    checksum: u32,
}

struct HumanReviewEntryV1 {
    human_review_id: u32,
    review_kind: u8,
    reviewer_role_ref: u32,
    review_time_us: i64,
    rating_ppm: u32,
    notes_ref: u32,
    policy_ref: u32,
    flags: u32,
    checksum: u32,
}
```

If a dataset claims synthetic-data auditability, `GeneratorProvenanceV1` is
REQUIRED for every synthetic output, label, score, preference, or rewrite.
External model APIs may be non-reproducible. In that case, the provenance record
supports audit and filtering, but MUST NOT claim deterministic regeneration.

## CoveQL-AI

CoveQL-AI adds AI-native query methods over existing Cove roots, including
`table(...)`, object roots, `association(...)`, `evidence(...)`,
`projection(...)`, `chunk(...)`, `trainingSamples(...)`,
`multimodalSequences(...)`, and `assets(...)`.

Proposed methods:

| Method | Purpose |
| --- | --- |
| `.similar(...)` | Semantic/vector similarity search over rows, objects, chunks, samples, sequences, or assets. |
| `.embedding(...)` | Return or compose an embedding for the current row/object/chunk/sample. |
| `.chunks(...)` | Project chunk entries for selected text/document slots. |
| `.tokens(...)` | Project tokenized spans or sequence packs. |
| `.context(...)` | Expand matched chunks to parent/sibling/object/document context. |
| `.hybrid(...)` | Combine predicate, lexical, vector, and metadata search. |
| `.trainingSamples(...)` | Select training/evaluation examples from COVE-TRAIN. |
| `.split(...)` | Select train/validation/test/eval/holdout split. |
| `.pack(...)` | Return token sequence packs for training. |
| `.multimodal(...)` | Return interleaved multimodal sequence packs. |
| `.asPromptContext(...)` | Emit structured RAG/prompt context with evidence and redaction applied. |
| `.rerank(...)` | Optional runtime reranking step, never archive authority. |
| `.explain(ai)` | Explain vector spaces, chunk profiles, tokenizer profiles, sidecars, exactness, approximation, fallback, redaction, generator provenance, and lineage. |

### Composed Embeddings

```text
table(products)
  .asOf(csn: 1420)
  .where(status == "active")
  .embedding(using: product_embedding_v1)
  .select(product_id, name, embedding)
```

Execution:

1. Validate file and selected snapshot.
2. Resolve `product_embedding_v1`.
3. Resolve participating slots.
4. Read FileCode lanes where possible.
5. For each FileCode, look up value/slot vector once.
6. Compose row/object embedding according to authority class.
7. Return Arrow-compatible vector output.

### Semantic Search

```text
Product
  .asOf(csn: 1420)
  .similar(
    to: "waterproof red hiking jacket",
    using: product_embedding_v1,
    limit: 20
  )
  .select(goid, name, category, score, evidence())
  .explain(ai)
```

If `to` is a string, the query vectorizer is a runtime dependency and must be
declared in operation context. For reproducible tests, callers SHOULD pass an
explicit query vector or a pinned vectorizer profile. Approximate vector results
MUST be labeled approximate. Ordinary predicates, temporal scope, visibility
filters, and redaction filters MUST still be applied.

### RAG Context

```text
Document
  .similar(to: "What are the termination rights?", using: legal_chunk_v1)
  .context(parent: section, before: 1, after: 1)
  .select(document_id, section_title, chunk_text, context_text, evidence())
```

### Training Stream

```text
trainingSamples(profile: "support_sft_v2")
  .split("train")
  .where(quality_score >= 900000)
  .pack(tokenizer: "llama_compatible_v1", sequence_length: 4096)
  .select(sample_id, input_tokens, labels, loss_mask, evidence())
```

### Synthetic-Data Filtering

```text
trainingSamples(profile: "sft_v4")
  .where(generator.model_name != "legacy_model_v1")
  .where(generator.kind == "model")
  .where(human_review.status == "approved")
  .select(sample_id, input, target, generator.model_name, generator.prompt_template)
```

### Multimodal Sequence Read

```text
multimodalSequences(profile: "visual_support_v1")
  .split("train")
  .where(generator.model_family != "deprecated_teacher")
  .select(sequence_id, elements, loss_mask, evidence())
```

## AI Explain Output

AI operations must be explainable. Example:

```json
{
  "ai_profile": "CoveQL-AI 0.1",
  "snapshot": {
    "file_digest": "...",
    "branch": "main",
    "csn": 1420
  },
  "vector": {
    "vector_space": "product_embedding_v1",
    "model": "example_embedding_model",
    "dimension": 1536,
    "metric": "cosine",
    "normalization": "unit_l2",
    "sidecar_status": "validated",
    "index_kind": "hnsw",
    "exactness": "approximate_candidates",
    "result_authority": "RuntimeAdvisory"
  },
  "composition": {
    "profile": "product_embedding_v1",
    "arithmetic_profile": "runtime_float_advisory",
    "slots": [
      {"path": "Product.name", "weight": 0.20},
      {"path": "Product.description", "weight": 0.60},
      {"path": "Product.category", "weight": 0.15},
      {"path": "Product.color", "weight": 0.05}
    ]
  },
  "chunking": {
    "chunk_profile": "paragraph_or_256_tokens",
    "tokenizer_profile": "example_tokenizer_v1"
  },
  "training": {
    "split": "train",
    "split_method": "stratified_hash",
    "epoch_plan": "seed_1234_block_shuffle"
  },
  "generator": {
    "kind": "model",
    "model_name": "teacher_model_v3",
    "prompt_template": "support_answer_v2",
    "reproducibility_class": "ExternalAuditOnly"
  },
  "fallback": {
    "used": false
  },
  "policy": {
    "redaction_applied": true,
    "forbidden_slots_skipped": ["Customer.private_note"],
    "withheld_metadata": ["private_note_vector_status"]
  }
}
```

Explain output MUST include sidecar freshness, vector-space identity,
chunk/token profiles, exactness/approximation status, result authority,
fallback, and policy decisions when policy allows disclosure. It MUST NOT reveal
protected values, protected tokens, protected vectors, or sensitive metadata
when disclosure policy forbids it.

## Security, Privacy, and Governance

COVE-AI must acknowledge AI-specific leakage risks:

- vectors can leak information about source values;
- token IDs can leak source text;
- chunk boundaries can reveal document structure;
- embedding neighborhoods can reveal similarity relationships;
- dedup groups can reveal copied or near-copied content;
- generated labels and quality scores can reveal protected inferences;
- ANN indexes may leak distributional information;
- prompt-context assembly can accidentally include redacted neighboring text.

Rules:

- a redacted value MUST NOT be exposed through chunk text, token IDs, vector
  payloads, nearest-neighbor metadata, dedup metadata, multimodal sequence
  elements, or training sample exports unless policy explicitly allows it;
- a forbidden slot MUST NOT be vectorized, tokenized, chunked, sampled,
  exported, or included in AI sidecars by conforming tooling;
- AI sidecars SHOULD carry sensitivity summaries so planners can avoid loading
  protected metadata unnecessarily;
- AI explain output MUST respect metadata disclosure policy;
- approximate, sampled, masked, or privacy-transformed AI metadata MUST be
  marked as approximate or policy-protected and MUST NOT be used as exact proof
  unless the proof remains valid under the declared transformation.

## Interoperability

COVE-AI should export to Arrow as:

- vectors: `FixedSizeList<Float32/Float16>`, `FixedSizeBinary`, or Arrow
  extension types;
- tokens: `List<Int32>`, `LargeList<Int32>`, `FixedSizeList<Int32>`, or packed
  binary;
- chunks: struct arrays with byte ranges, token ranges, parent IDs, and evidence
  refs;
- training samples: struct arrays with `input_ids`, labels, loss masks,
  attention masks, and metadata;
- multimodal sequences: struct/list arrays with ordered elements, modality
  tags, role tags, and asset refs;
- assets: URI/digest structs, binary views when embedded, or tensor views when
  safe;
- tensors: Arrow tensor-compatible output and DLPack-compatible views when
  validated.

COVE-AI should support export to Parquet/lakehouse tables for chunks, vectors,
tokens, samples, multimodal elements, generator provenance, labels, evidence,
and asset manifests.

ML framework adapters may target PyTorch, TensorFlow, JAX/NumPy, DLPack, Arrow
streams, WebDataset-like shards, JSONL, and Parquet. These adapters MUST
preserve redaction, visibility, split, sample order, mask, label, and evidence
policy. Zero-copy export MUST validate tensor layout, lifetime, compression,
alignment, and policy before exposing COVE buffers.

## Conformance Tiers

| Tier | Name | Requirements |
| --- | --- | --- |
| COVE-AI-L0 | Ignore safely | Reader recognizes unknown optional AI profiles and ignores them safely. |
| COVE-AI-L1 | Metadata | Reader validates COVE-MAP-AI slot policies and exposes them in inspect/explain. |
| COVE-AI-L2 | Chunks | Reader validates COVE-CHUNK indexes and returns chunk/context projections. |
| COVE-AI-L3 | Tokens | Reader validates tokenizer profiles, token blocks, tokenized spans, and sequence packs. |
| COVE-AI-L4 | Value vectors | Reader validates COVE-VEC vector spaces, FileCode vector bindings, and vector payload blocks; supports `.embedding()` for distinct value/slot vectors. |
| COVE-AI-L5 | Semantic search | Reader supports `.similar()` with exact flat scan or validated candidate indexes, with correct temporal/visibility/redaction behavior. |
| COVE-AI-L6 | Training data | Reader validates COVE-TRAIN samples, splits, labels, loss masks, sample weights, dedup groups, generator provenance, and epoch plans. |
| COVE-AI-L7 | Multimodal | Reader validates COVE-MMSEQ interleaved multimodal sequences and asset refs. |
| COVE-AI-L8 | Full AI archive | Reader supports chunks, tokens, vectors, training samples, multimodal sequences, synthetic provenance, tensor layout, AI explain, deterministic split/epoch plans, and negative conformance vectors. |

## Negative Conformance Corpus

The conformance suite should include reject/fallback tests for:

- stale `.covev` file digest;
- wrong dictionary digest, schema fingerprint, vector dimension, metric, or
  normalization policy;
- template fingerprint mismatch;
- tokenizer digest mismatch;
- chunk span not UTF-8 aligned;
- chunk source hash mismatch;
- tokenized span source hash mismatch;
- FileCode reused across files without dictionary proof;
- forbidden slot vectorized or tokenized;
- redacted slot exposed through vector metadata;
- redacted neighboring chunk included in context;
- approximate index claimed as exact;
- ANN index with false negatives used for proof exclusion;
- training split missing seed/hash policy;
- evaluation split contaminated by dedup group overlap;
- external asset URI without digest when replayability is claimed;
- generated label missing generator provenance when auditability is claimed;
- teacher model name present but version/provenance missing under strict policy;
- runtime floating-point composed vector claimed as digest-reproducible;
- tensor layout claims zero-copy but alignment/stride is invalid;
- multimodal sequence element ordinal duplicate;
- loss mask references invalid token range.

## Benchmarks

COVE-AI benchmark reporting should include:

- vector dedup ratio: raw occurrences / distinct vectorized values;
- embedding cost avoided: estimated duplicate value embeddings skipped;
- tokenization cost avoided: repeated text/chunks served from COVE-TOK;
- chunk reuse ratio: repeated chunks / distinct chunks;
- training stream throughput: samples/sec, tokens/sec, sequence packs/sec;
- multimodal stream throughput: sequences/sec, assets/sec, token plus asset
  assembly latency;
- random sample access: p50/p95 sample fetch latency;
- RAG retrieval: candidate generation latency, context assembly latency, and
  exact/approx status;
- snapshot verification: time to validate file digest, sidecar freshness, trust
  chain, and map profile;
- tensor export: zero-copy success rate, materialization fallback rate, and
  device-transfer bytes/sec;
- storage overhead: AI metadata, vector payload, token cache, and training index
  bytes relative to source bytes;
- correctness: stale sidecar rejection, redaction leakage tests, split
  reproducibility, and generator filtering correctness.

## Implementation Roadmap

### Phase 1: AI Slot Metadata and Dictionary Vectors

Implement COVE-MAP-AI slot policy metadata, COVE-VEC vector space descriptors,
`FileCodeVectorBindingV1`, `VectorPayloadBlockHeaderV1`,
`VectorCompositionProfileV1`, `VectorResultAuthorityV1`, `cove inspect --ai`,
`cove vec build`, and CoveQL-AI `.embedding()`.

Deliverable: distinct logical values are vectorized once per semantic slot and
reused by FileCode.

### Phase 2: Chunking and RAG

Implement COVE-CHUNK profiles, `TextChunkEntryV1`, `ChunkVectorBindingV1`,
CoveQL-AI `.similar()` over chunks, `.context()`, `.asPromptContext()`, and AI
explain output.

Deliverable: Cove becomes a self-verifying RAG archive with source-bound chunks
and evidence-aware context.

### Phase 3: Tokenization and Training Data

Implement `TokenizerProfileV1`, `TokenBlockHeaderV1`, `TokenizedSpanV1`,
`TokenSequencePackV1`, `TrainingProfileV1`, `TrainingSampleEntryV1`,
`DatasetSplitV1`, `cove train export`, and
`.trainingSamples().split().pack()`.

Deliverable: Cove stores reproducible tokenized training streams and sample
splits.

### Phase 4: Synthetic Provenance

Implement `GeneratorProvenanceV1`, `ModelActorDescriptorV1`,
`GenerationDecodingProfileV1`, `HumanReviewEntryV1`,
`PreferencePairEntryV1`, and CoveQL generator filters.

Deliverable: labs can filter, audit, and scrub synthetic data by teacher model,
prompt, judge, and review status.

### Phase 5: Multimodal and Tensor Layout

Implement `AiAssetRefV1`, COVE-MMSEQ sequence packs,
`MultimodalSequenceElementV1`, `TensorLayoutDescriptorV1`,
`DeviceTransferHintV1`, and DLPack/Arrow tensor export.

Deliverable: Cove supports interleaved multimodal model-consumable sequences
and hardware-aware tensor export.

### Phase 6: Vector Indexes and Scale

Implement exact flat vector scan, optional HNSW/IVF/DiskANN/Vamana indexes,
approx/exact explain, large corpus sharding through COVM, and distributed
training/sample scan.

Deliverable: Cove supports large-scale semantic search and training-data
streaming while preserving archive authority.

## README Positioning

Recommended headline:

```text
Cove is a proof-aware archive format for the AI era.
```

Recommended technical pitch:

> COVE-AI turns AI data from loose preprocessing artifacts into a
> self-verifying semantic archive. Distinct values are embedded once, long text
> is chunked once, tokenization can be cached once, multimodal model streams can
> be preserved in order, synthetic teacher provenance can be audited, and
> training samples can be reconstructed against the same immutable source truth.

Comparison:

| Need | Ordinary AI data pipeline | Cove with COVE-AI |
| --- | --- | --- |
| Repeated values | Embedded/tokenized repeatedly | Distinct values vectorized/tokenized once per slot/profile |
| Long text | Re-chunked by every RAG/training job | Chunk spans, hierarchy, tokenizer lineage, evidence binding |
| Vectors | External DB/index with separate lifecycle | Optional `.covev` sidecar bound to file digest, dictionary, slot, and model lineage |
| Training data | JSONL/Arrow shards plus undocumented scripts | Samples, splits, token packs, labels, weights, evidence, and policies |
| Multimodal data | External asset links and ad hoc manifests | Interleaved multimodal sequence packs with digest-bound assets |
| Synthetic data | Prompt logs and side tables | Teacher/generator provenance, model actor descriptors, decoding profiles |
| Reproducibility | Pipeline-dependent | Snapshot, CSN, mapping profile, chunker, tokenizer, vectorizer, split, and digest lineage |
| Querying | SQL plus vector/RAG tools | CoveQL-AI over tables, objects, chunks, vectors, samples, assets, and evidence |
| Governance | External catalogs and notebooks | Redaction, policy, evidence, source lineage, forbidden slots, and AI explain |

Avoid claiming:

- Cove replaces Parquet.
- Cove replaces all vector databases.
- Cove eliminates all ETL.
- Cove guarantees deterministic model training.
- Cove makes external model APIs reproducible.
- Cove makes redaction equivalent to encryption.

Prefer:

- Cove provides a self-verifying archive foundation for AI data.
- Cove can export to existing ML, Arrow, Parquet, lakehouse, and vector systems.
- Cove preserves the lineage and policy context those systems usually lose.

## Normative Amendment Opening

The amendment should open with:

> COVE-AI extends the COVE standards suite with optional semantic AI profiles for
> chunking, tokenization, vectorization, multimodal sequencing, training samples,
> teacher provenance, hardware-aware tensor layout, and AI query planning.
> COVE-AI does not alter COVE-Core logical truth. AI artifacts are derivative
> unless another COVE profile explicitly defines them as canonical logical
> values.
>
> A COVE-AI implementation MUST bind derivative AI artifacts to the source file,
> snapshot, schema, dictionary, semantic slot, mapping profile, model/tokenizer/
> chunker/vectorizer lineage, arithmetic profile, generator provenance, tensor
> layout, and policy context needed to validate freshness and replay intent.
>
> Dynamically composed floating-point vectors are RuntimeAdvisory by default and
> MUST NOT be treated as byte-reproducible cryptographic truth. Composed vectors
> may become digest-verifiable only when materialized as payload bytes or
> produced under a strict canonical deterministic arithmetic profile.
>
> A conforming implementation MUST fail closed or fall back safely when an AI
> artifact is stale, unsupported, corrupt, policy-blocked, hardware-incompatible,
> or outside its declared vector/token/chunk/training/multimodal profile.
>
> The purpose of COVE-AI is to make AI data reusable, explainable, and
> reproducible: distinct values are embedded once, long text is chunked once,
> tokenization can be cached once, multimodal model streams can be preserved in
> order, synthetic teacher provenance can be audited, training samples can be
> reconstructed, and semantic query plans can be explained against the same
> immutable archive truth.

## Final Summary

COVE-AI makes Cove AI-ready without compromising Cove's database and archive
mechanics.

It makes AI artifacts:

- selective: COVE-MAP-AI chooses useful semantic slots and forbids dangerous
  ones;
- deduplicated: COVE-VEC stores vectors once per distinct canonical value, slot,
  chunk, object state, or training sample;
- reusable: chunks, tokens, vectors, samples, and multimodal sequences can be
  shared across RAG, search, training, evaluation, and export;
- auditable: source snapshots, CSNs, mapping profiles, evidence, digests, trust
  chains, and generator provenance travel with the archive;
- safe: redaction, policy, forbidden slots, approximate-index limits, and
  runtime floating-point boundaries are explicit;
- performant: FileCodes, vector payload blocks, token packs, tensor layouts,
  zero-copy hints, and optional vector indexes support high-throughput AI
  workloads;
- queryable: CoveQL-AI gives users one profile for semantic search, RAG context,
  embeddings, training samples, multimodal streams, and AI explain output.

Market message:

> Cove is not just a data file. Cove is a self-verifying semantic archive for AI
> systems: values, objects, evidence, chunks, tokens, vectors, labels, samples,
> synthetic provenance, and multimodal sequences bound together under one
> immutable, queryable, proof-aware format.
