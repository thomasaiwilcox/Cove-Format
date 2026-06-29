# COVE-MAP-AI

## 83.20 COVE-MAP-AI Slot Policy Profile

COVE-MAP-AI tells AI ingestion and query engines which semantic slots matter.
It stores intent and policy. COVE-VEC stores vectors, COVE-CHUNK stores chunks,
COVE-TOK stores tokenization, and COVE-TRAIN stores samples.

COVE-MAP-AI answers whether a field should be vectorized, chunked, tokenized,
sampled, composed, ignored, or forbidden, and whether it represents a title,
description, category, identifier, prompt, completion, tool call, protected
field, safety label, preference label, or quality score.

### 83.20.1 Artifact Placement

COVE-MAP-AI policy is stored as COVE-MAP payload. Authoritative reusable AI
intent for a mapping version SHOULD live in the `.covemap` artifact that owns
the semantic mapping, using `MAP_AI_*` sections. Embedded `MAP_AI_*` sections
inside `.cove` outputs are file-local snapshots, conversion evidence, or
inspectable policy summaries tied to that output.

`MAP_AI_PROFILE_CATALOG` declares `MapAiProfileV1` and
`MapAiSlotPolicyV1`. `MAP_AI_TEMPLATE_CATALOG` declares templates and template
fingerprints used by vectorization, chunking, prompt context, or generated
sample assembly. `MAP_AI_TRAINING_POLICY_CATALOG` declares slot-level sample,
label, split, weighting, dedup, and quality intent.

COVE-AI does not require full source-to-object mapping for plain COVE-T files.
A writer MAY store a minimal `.covemap` artifact whose only purpose is AI slot
policy over COVE-T table/column/path refs.

### 83.20.2 Validation Rules

- Reusable `.covemap` payloads MUST follow the COVE-MAP v2 payload discipline.
- A COVE-MAP-AI-aware tool MUST validate source refs, table/column refs,
  object/property refs, association refs, path refs, policy refs, template
  refs, vector-space refs, chunk-profile refs, tokenizer refs, and
  training-policy refs before using a slot policy.
- Within one active AI profile, duplicate slot policies for the same path are
  invalid unless an explicit precedence rule is declared.
- Across multiple active profiles, `Forbidden` fails closed unless a trusted
  policy override is declared and audited.
- If a later AI sidecar contains vectors, tokens, chunks, samples, sequence
  elements, or labels for a forbidden slot, the affected AI operation MUST
  reject.

### 83.20.3 Slot Roles and Decisions

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

`Forbidden` means conforming tooling MUST NOT vectorize, tokenize, chunk,
sample, export, or include the slot in AI sidecars unless a higher-priority
explicit policy authorizes it. `Ignore` means the slot is not selected by the
declared profile, but another explicit profile MAY select it. `PolicyDefined`
means runtime policy decides whether and how the slot can be used.

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

### 83.20.4 Core Records

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

A template used for vectorization MUST participate in vector lineage. Changing
the template changes the vectorization profile fingerprint. Vectors generated
with different template fingerprints MUST NOT be compared unless a
compatibility descriptor explicitly permits it.
