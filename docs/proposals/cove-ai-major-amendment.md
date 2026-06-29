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

## COVE-AI Integration Contract

COVE-AI must use the existing COVE v2 scoping, sidecar, digest, redaction, and
operation-requiredness machinery. It is not a parallel format with different
validity rules.

1. COVE-AI is a set of optional profiles. COVE-Core, COVE-T, COVE-O, and
   materialized COVE-MAP truth remain readable without COVE-AI.
2. Embedded COVE-AI sections in `.cove` use the global COVE section-kind
   registry and the global feature-word model. Operation-required AI features
   MUST be scoped through section entries, profile capability matrices, or
   `SECTION_FEATURE_BINDING`, not through `.cove` header
   `required_features` word 0.
3. `CVA2` and `CVV2` artifacts MAY use an artifact-local COVE-AI feature
   namespace in their own headers, postscripts, and section entries. Any `.cove`
   reference to those artifacts must advertise the corresponding global optional
   presence or capability feature and bind requiredness to the selected
   operation.
4. COVE-VEC owns vector spaces, vector payloads, vector bindings, composition
   metadata, quantization metadata, and vector lineage. COVX/COVE-I-compatible
   rules own index proof, false-negative policy, index-only capability, sidecar
   validity, coverage/fallback semantics, and cross-file or dataset index
   publication.
5. Every payload-bearing AI section must bind to the source snapshot, schema,
   dictionary or canonical value identity, semantic slot or object path,
   model/tokenizer/chunker/vectorizer/template lineage, policy scope,
   visibility scope, redaction scope, and digest lineage needed to validate
   freshness.
6. CRC32C validates transport integrity. Cryptographic digest references are
   required for trust, payload-byte verification, reproducibility, replay, and
   auditability claims.
7. Direct readers of `CVA2` or `CVV2` artifacts MUST fail closed on policy until
   source binding, visibility scope, redaction scope, policy scope, and
   sensitivity summaries are validated or explicitly overridden by a trusted
   caller policy.

## Proposed Registry and Artifact Plan

This proposal keeps the full COVE-AI scope, but stages implementation through
stable profile boundaries. The registry details in this section are proposed
allocations for review. Once accepted, they should be copied into the feature
bit, section-kind, and artifact registries and backed by conformance fixtures.

### Feature Bits

COVE-AI uses two feature locations:

- global COVE feature words for embedded `.cove` sections, COVM references,
  profile capability matrices, and operation-required bindings;
- an artifact-local COVE-AI feature namespace inside `CVA2` and `CVV2`
  companion artifacts.

The global COVE-AI feature word is `TBD_AI_GLOBAL_FEATURE_WORD`. The bit numbers
below are local to that global word when used in `.cove` artifacts and local to
the `CVA2`/`CVV2` artifact namespace when used inside those sidecars. Embedded
`.cove` AI sections MUST NOT use artifact-local feature numbering.

| Bit | Feature | Scope | Required when |
| ---: | --- | --- | --- |
| 0 | `AI_FEATURE_MAP_AI_POLICY` | COVE-MAP-AI | AI ingestion or AI query planning uses slot policy. |
| 1 | `AI_FEATURE_CHUNK` | COVE-CHUNK | Stored chunk boundaries are required for the requested operation. |
| 2 | `AI_FEATURE_TOKEN` | COVE-TOK | Stored token IDs, masks, labels, or sequence packs are required. |
| 3 | `AI_FEATURE_VECTOR` | COVE-VEC | Stored vector payloads or vector bindings are required. |
| 4 | `AI_FEATURE_VECTOR_INDEX` | COVE-VEC | A vector index is required by the requested operation. |
| 5 | `AI_FEATURE_TENSOR_LAYOUT` | COVE-VEC / COVE-MMSEQ | Tensor layout or zero-copy export metadata is required. |
| 6 | `AI_FEATURE_ASSET_REF` | COVE-MMSEQ / COVE-TRAIN | Digest-bound asset references are required. |
| 7 | `AI_FEATURE_MMSEQ` | COVE-MMSEQ | Interleaved multimodal sequence reconstruction is required. |
| 8 | `AI_FEATURE_TRAIN` | COVE-TRAIN | Training sample indexes or split metadata are required. |
| 9 | `AI_FEATURE_GENERATOR_PROVENANCE` | COVE-TRAIN | Synthetic output, label, score, preference, or rewrite audit is required. |
| 10 | `AI_FEATURE_COVEQL_AI` | CoveQL-AI | A stored operation profile requires CoveQL-AI semantics. |
| 11 | `AI_FEATURE_CANONICAL_FIXED_POINT_VECTOR` | COVE-VEC | Canonical vector recomputation is claimed. |
| 12 | `AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED` | COVE-MMSEQ / COVE-TRAIN | Replayability depends on external asset digests. |
| 13 | `AI_FEATURE_PRIVACY_SUMMARY` | COVE-AI | Planners rely on sensitivity summaries before loading payloads. |
| 14 | `AI_FEATURE_VECTOR_SPACE_COMPATIBILITY` | COVE-VEC | Cross-vector-space comparison is allowed by descriptor. |
| 15 | `AI_FEATURE_MODEL_INPUT_IDENTITY` | COVE-VEC | Vector bindings prove exact model-input identity and descriptor-level vector deduplication. |

Rules:

- Writers MUST NOT place operation-only COVE-AI requirements in `.cove` header
  `required_features` word 0. Unknown bits there are file-required and reject
  before a reader can narrow the operation.
- Embedded `.cove` AI sections use `EXTENDED_FEATURE_SET` and
  `SECTION_FEATURE_BINDING` when an AI requirement is profile-, section-, page-,
  or operation-scoped.
- A global COVE-AI bit in `.cove` header `optional_features`, or in an
  extended optional feature word, advertises presence or optional capability.
  Unknown optional bits are ignored for ordinary COVE-T/O/MAP reads.
- A global COVE-AI bit in an extended required feature word without a narrower
  binding is `FileRequired`. Writers SHOULD avoid this for COVE-AI unless the
  entire file is intentionally unreadable without that AI feature.
- A COVE-AI bit in `CVA2`/`CVV2` `required_ai_features` is artifact-required:
  a reader that opens the sidecar but does not support the bit rejects the
  sidecar or the selected sidecar operation. It MUST NOT reject the referenced
  `.cove` file solely because the sidecar is unsupported.
- A COVE-AI bit in `CVA2`/`CVV2` `optional_ai_features` is sidecar-advisory or
  sidecar-optional. Unsupported readers ignore the corresponding metadata unless
  the requested AI operation requires it through a supported binding.
- Writers MUST bind required COVE-AI features to the narrowest possible scope:
  section, profile, artifact, or operation. They MUST NOT make an ordinary
  COVE-T scan fail because an AI sidecar is unsupported.
- Required COVE-AI features need accept and reject fixtures. Optional COVE-AI
  features need inspect/report coverage proving safe fallback.

| Location | Feature namespace | Requiredness rule |
| --- | --- | --- |
| `.cove` header word 0 | global COVE low word | File-required when in `required_features`; never use for operation-only AI requirements. |
| `.cove` extended feature words | global COVE feature words | Scoped by `SECTION_FEATURE_BINDING`, profile capability matrices, section entries, or operation bindings. |
| Embedded `.cove` AI section entries | global COVE feature words | Section-required unless narrowed by binary binding. |
| COVM references to AI sidecars | global COVE feature words plus digest-bound artifact refs | Requiredness is selected snapshot or operation scoped. |
| `CVA2`/`CVV2` headers/postscripts/sections | artifact-local COVE-AI feature namespace | Rejects sidecar use or selected AI operation, not ordinary `.cove` reads. |

### Profile and Operation IDs

COVE-AI profile IDs are proposed as continuations of the current v2 profile
registry. They intentionally match the standards-suite part numbers for this
amendment. If the registry later decouples profile IDs from part numbers, this
table must be updated before acceptance.

| Profile ID | Profile | Capability matrix name |
| ---: | --- | --- |
| 12-15 | Reserved | Reserved for current-suite continuation. |
| 16 | COVE-AI Shared | `COVE_AI_SHARED` |
| 17 | COVE-MAP-AI | `COVE_MAP_AI` |
| 18 | COVE-CHUNK | `COVE_CHUNK` |
| 19 | COVE-TOK | `COVE_TOK` |
| 20 | COVE-VEC | `COVE_VEC` |
| 21 | COVE-MMSEQ | `COVE_MMSEQ` |
| 22 | COVE-TRAIN | `COVE_TRAIN` |
| 23 | CoveQL-AI | `COVEQL_AI` |

Unknown profile IDs in optional sections MUST NOT cause ordinary COVE-Core,
COVE-T, COVE-O, or materialized COVE-MAP reads to reject. A reader that does
not recognize a section profile MUST treat the section as an unknown optional
section, preserve it for inspect/report when possible, and reject only if that
section, profile, or operation is required for the selected operation. The same
fallback rule applies to `profile_kind` inside `CVA2` and `CVV2` section
entries.

Proposed COVE-AI operation kinds extend `OperationKindV2`:

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

### Section Kinds

COVE-MAP-AI policy lives with COVE-MAP because it describes semantic intent.
Large derived AI payloads normally live in COVE-AI bundle artifacts, `.covev`
artifacts, COVX, or COVE-I-style sidecars. Embedded `.cove` sections are allowed
for small, file-local metadata and test vectors, but large vectors and token
blocks SHOULD use companion artifacts.

Proposed standard section-kind assignments:

| ID | Name | Profile | Payload |
| ---: | --- | --- | --- |
| 70 | `MAP_AI_PROFILE_CATALOG` | COVE-MAP-AI | `MapAiProfileV1`, `MapAiSlotPolicyV1`, and policy defaults. |
| 71 | `MAP_AI_TEMPLATE_CATALOG` | COVE-MAP-AI | `AiVectorTemplateV1` and template fingerprints. |
| 72 | `MAP_AI_TRAINING_POLICY_CATALOG` | COVE-MAP-AI | Slot-level sample, label, and split intent. |
| 99 | `AI_COMPANION_ARTIFACT_REF` | COVE-AI | Digest-bound references to `CVA2` and `CVV2` companion artifacts. |
| 100 | `AI_SOURCE_BINDING` | COVE-AI | Source file, COVM snapshot, schema, dictionary, mapping, policy, and digest bindings. |
| 101 | `AI_CHUNK_PROFILE` | COVE-CHUNK | `ChunkProfileV1` records. |
| 102 | `AI_TEXT_CHUNK_INDEX` | COVE-CHUNK | `TextChunkEntryV1` records and context links. |
| 103 | `AI_TOKENIZER_PROFILE` | COVE-TOK | `TokenizerProfileV1` records and tokenizer digest references. |
| 104 | `AI_TOKEN_BLOCK` | COVE-TOK | `TokenBlockHeaderV1` descriptor records; token bytes are referenced through `AI_PAYLOAD_REF_TABLE`. |
| 105 | `AI_TOKENIZED_SPAN` | COVE-TOK | `TokenizedSpanV1` records and byte alignment references. |
| 106 | `AI_TOKEN_SEQUENCE_PACK` | COVE-TOK | `TokenSequencePackV1` records, masks, labels, and positions. |
| 107 | `AI_VECTOR_SPACE` | COVE-VEC | `VectorSpaceDescriptorV1` and `VectorSpaceCompatibilityDescriptorV1` records. |
| 108 | `AI_VECTOR_BINDING` | COVE-VEC | FileCode, chunk, object, sample, asset, and sequence vector bindings. |
| 109 | `AI_VECTOR_PAYLOAD_BLOCK` | COVE-VEC | `VectorPayloadBlockHeaderV1` descriptor records; vector bytes are referenced through `AI_PAYLOAD_REF_TABLE`. |
| 110 | `AI_VECTOR_COMPOSITION` | COVE-VEC | Composition components and arithmetic profiles. |
| 111 | `AI_VECTOR_INDEX` | COVE-VEC | `VectorIndexDescriptorV1` and index payload references. |
| 112 | `AI_TENSOR_LAYOUT` | COVE-VEC / COVE-MMSEQ | Tensor layout and device-transfer descriptors. |
| 113 | `AI_ASSET_MANIFEST` | COVE-MMSEQ / COVE-TRAIN | `AiAssetRefV1` records and asset policy. |
| 114 | `AI_MULTIMODAL_SEQUENCE` | COVE-MMSEQ | Sequence packs and ordered sequence elements. |
| 115 | `AI_TRAINING_PROFILE` | COVE-TRAIN | `TrainingProfileV1` records. |
| 116 | `AI_TRAINING_SAMPLE_INDEX` | COVE-TRAIN | `TrainingSampleEntryV1` records. |
| 117 | `AI_TRAINING_SPLIT_DEDUP_EPOCH` | COVE-TRAIN | Splits, dedup groups, and epoch plans. |
| 118 | `AI_LABEL_PREFERENCE` | COVE-TRAIN | Labels, preference pairs, and human review links. |
| 119 | `AI_GENERATOR_PROVENANCE` | COVE-TRAIN | Generator, model actor, decoding, and review records. |
| 120 | `AI_REFERENCE_TABLES` | COVE-AI | String, digest, policy, payload, mask/label, source-span, transform, and extension reference tables. |
| 121 | `AI_PAYLOAD_INTEGRITY` | COVE-AI | Cryptographic digest and CRC records for payload-byte verification and replay claims. |
| 122 | `AI_PRIVACY_SUMMARY` | COVE-AI | Sensitivity summaries, disclosure bounds, retention/revocation status, and policy-load hints. |
| 123 | `AI_SECTION_FEATURE_BINDING` | COVE-AI | Artifact-local profile, section, and operation requiredness bindings. |
| 124 | `AI_VECTOR_DIRECTORY` | COVE-VEC | `VectorEntryV1` records resolving vector refs to payload blocks and byte ranges. |
| 125 | `AI_PAYLOAD_BYTES` | COVE-AI | Opaque payload byte ranges referenced by `AI_PAYLOAD_REF_TABLE`. |

Rules:

- `MAP_AI_*` payloads use the COVE-MAP payload discipline: canonical JSON or
  deterministic CBOR for reusable `.covemap` artifacts, with duplicate keys and
  undeclared extension fields rejected.
- `AI_*` payloads use length-delimited binary records by default. Canonical JSON
  or deterministic CBOR MAY be used for fixtures, inspectable policy metadata,
  or low-volume catalogs when the payload encoding declares it. `AI_PAYLOAD_BYTES`
  is the Phase 1 exception: it is an opaque byte-carrier section and is never
  parsed as a record array.
- All descriptor section payloads MUST be covered by section payload CRC32C
  before their records are used. Large payload-bearing sections MAY set section
  `payload_crc32c = 0` only when every used payload range is covered by
  validated block/range integrity records such as `AiPayloadIntegrityV1`. All
  section and payload ranges MUST still be bounds-checked before use.
- Unknown optional COVE-AI sections are skipped but remain visible to inspect
  and report tools. Unknown required COVE-AI sections reject only the selected
  AI operation.

### Companion Artifacts

COVE-AI supports two first-class companion artifact shapes:

```text
Extension: .coveai
Magic: CVA2
Profile: COVE-AI bundle
Purpose: mixed chunks, tokens, vectors, training samples, multimodal sequences,
         generator provenance, asset manifests, and tensor metadata.

Extension: .covev
Magic: CVV2
Profile: COVE-VEC optimized vector artifact
Purpose: large vector payloads, vector bindings, vector composition metadata,
         vector indexes, tensor layouts, and device-transfer hints.
```

`.coveai` is the general bundle for implementations that want one AI sidecar per
source snapshot. `.covev` is an optimized COVE-VEC carrier for large vector
payloads or vector indexes. A writer MAY store COVE-VEC data in `.coveai`,
`.covev`, COVX, or COVE-I-style artifacts, but every carrier MUST expose the
same logical section kinds and validation rules.

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
    magic: [u8; 4],        // "CVA2" or "CVV2"
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
// followed by:
//   CoveAiSectionEntryV1[section_count]

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
    artifact_kind: u8,       // 1=CVA2, 2=CVV2
    artifact_id: [u8; 16],
    uri_ref: u32,
    artifact_digest_ref: u32,
    source_binding_ref: u32,
    required_ai_features: u64,
    optional_ai_features: u64,
    flags: u32,
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

- All integer fields are little-endian. There is no byte-order negotiation.
- COVE-AI companion artifacts use tail discovery. The final bytes are
  postscript bytes, `postscript_version: u16`, `postscript_len: u16`, and magic
  `CVA2` or `CVV2`.
- For the first COVE-AI companion artifact profile, `postscript_version` MUST
  be 1, `version_major` MUST be 1, and `version_minor` MUST be 0. Readers MUST
  reject unsupported major versions and MAY accept supported minor versions
  according to declared feature bits.
- A `.cove`, `.covm`, or external catalog reference to a COVE-AI companion
  artifact SHOULD use `AI_COMPANION_ARTIFACT_REF` or an equivalent COVM
  extension carrying `AiCompanionArtifactRefV1`. The reference MUST be
  digest-bound before sidecar use is trusted.
- `AI_COMPANION_ARTIFACT_REF` is a descriptor section for `.cove`, COVM, or
  catalog-side references to AI sidecars. A `CVA2` or `CVV2` sidecar does not
  need to contain a self-reference unless a profile explicitly uses
  self-description.
- When `AI_COMPANION_ARTIFACT_REF` is embedded in `.cove`, its `uri_ref` and
  `artifact_digest_ref` resolve through an embedded `AI_REFERENCE_TABLES`
  section in the same `.cove` artifact. `source_binding_ref` resolves to an
  `AiSourceBindingV1` record in an embedded `AI_SOURCE_BINDING` section, or is
  zero when the sidecar supplies its own source binding. Other resolver sources
  require an explicit required extension.
- `postscript_len` is the byte length of `CoveAiPostscriptV1` only. It excludes
  `postscript_version`, `postscript_len`, and trailing magic. The first profile
  requires `postscript_len == 44`; readers MUST reject values below 44 or above
  4096 unless a required extension defines a longer postscript.
- The `required_ai_features` and `optional_ai_features` words in
  `CoveAiPostscriptV1` and `CoveAiHeaderV1` MUST match exactly. A mismatch is
  structural corruption and the artifact MUST be rejected. The postscript copy
  exists only for tail-discovery bootstrap and early rejection.
- `header_len` is the byte length of `CoveAiHeaderV1`; the first profile
  requires `header_len == 70`.
- `section_entry_len` is the byte length of `CoveAiSectionEntryV1`; the first
  profile requires `section_entry_len == 64`.
- `header_length` in the postscript MUST equal
  `header_len + section_count * section_entry_len`. The header region begins at
  `header_offset` and includes the fixed header plus section-entry array.
- `file_len` MUST equal the actual artifact byte length. `header_offset +
  header_length` MUST be within `file_len`.
- COVE-AI headers and records are packed wire records. Readers MUST parse fields
  explicitly and MUST NOT rely on native struct alignment or padding.
- `VectorPayloadBlockHeaderV1.compression_codec`,
  `TokenBlockHeaderV1.compression_codec`, and `CoveAiSectionEntryV1.compression`
  use `AiCompressionCodecV1`, whose Phase 1 values match the COVE section
  compression registry.
- `AI_PAYLOAD_BYTES` sections MUST declare
  `payload_encoding = AiPayloadEncodingV1::OpaqueBytes`.
- `offset` and `length` are absolute byte offsets from the start of the
  companion artifact.
- Section ranges MUST be within `file_len` and MUST NOT overlap unless a
  required extension explicitly permits overlap.
- `section_id` is a unique section instance ID within the companion artifact.
  Multiple sections may share one `section_kind`, but they MUST have distinct
  `section_id` values.
- `section_kind` uses the global COVE section-kind registry, widened to `u32` in
  `CVA2`/`CVV2` for future growth. Standard COVE-AI section kinds are the
  values listed above. Vendor or artifact-local section semantics require a
  registered extension and cannot silently reuse standard values.
- `source_binding_ref` binds every derived section to the source file, snapshot,
  dictionary, schema, mapping, visibility, and redaction context needed to check
  freshness.
- If `source_binding_ref` in the section entry is non-zero, all records in the
  section inherit that binding unless the section-specific schema permits a
  record-level source-binding override. If it is zero for a payload-bearing
  derived section, each record MUST carry or reference its own source binding.
- `requiredness_scope` uses `AiRequirednessScopeV1`. Section-local
  `required_ai_features` and `optional_ai_features` are interpreted under that
  scope. `feature_binding_ref` references an optional AI feature-binding record
  when section-local feature words are not sufficient for profile- or
  operation-scoped requiredness.
- `feature_binding_ref = 0` means no additional binding. A non-zero
  `feature_binding_ref` MUST reference exactly one validated
  `AiSectionFeatureBindingV1` record in `AI_SECTION_FEATURE_BINDING`.
- A stale source file digest, schema fingerprint, dictionary digest, mapping
  fingerprint, branch, CSN, visibility scope, or redaction scope makes the
  affected section unusable for the requested operation.
- `reserved0` and any future reserved bytes MUST be zero on write and validated
  as zero on read.
- `crc32c` fields use CRC32C. `CoveAiPostscriptV1.crc32c` covers the
  postscript struct with its `crc32c` field treated as zero.
  `CoveAiHeaderV1.crc32c` covers the header struct with its `crc32c` field
  treated as zero. `CoveAiHeaderV1.section_directory_crc32c` covers the
  `CoveAiSectionEntryV1[section_count]` bytes exactly as stored in the header
  region. `CoveAiSectionEntryV1.payload_crc32c` covers the decoded section
  payload bytes for descriptor sections and MAY be zero for large
  payload-bearing sections that use block/range integrity. Record-level
  `crc32c` fields cover the record bytes with the record checksum field treated
  as zero. CRC32C is not a trust, replayability, or cryptographic identity
  mechanism.
- A companion artifact MUST be integrity-checkable without consulting external
  services. External assets and model services may be referenced, but replay or
  byte-reproducibility claims require digest-bound inputs.
- Any section or payload block that claims `StoredPayloadVerifiable`,
  trust-chain participation, replayability, or auditability MUST reference an
  `AiPayloadIntegrityV1` record or another profile-defined cryptographic digest
  record.

### CVA2/CVV2 Validation Order

A conforming COVE-AI sidecar reader SHOULD validate companion artifacts in this
order:

1. Read tail magic, `postscript_version`, and `postscript_len`.
2. Validate postscript length, `file_len`, `header_offset`, `header_length`, and
   postscript CRC32C.
3. Validate header bounds, `header_len`, `section_count`, `section_entry_len`,
   reserved fields, header CRC32C, and section-directory CRC32C.
4. Require postscript and header feature words to match exactly.
5. Validate section directory bounds, unique `section_id` values, non-overlap,
   compression declarations, section-entry lengths, and descriptor section
   payload CRC32C.
6. Reject unknown artifact-required header AI feature bits.
7. Validate `AI_SECTION_FEATURE_BINDING` before using scoped feature bindings.
8. Build the sidecar feature-scope table from header words, section-local words,
   and validated feature-binding records.
9. Select the requested AI operation.
10. Reject only unknown required AI features whose scope intersects the selected
    sidecar operation.
11. Validate `AI_SOURCE_BINDING`, `AI_REFERENCE_TABLES`, `AI_PRIVACY_SUMMARY`,
    and policy context before exposing payload-bearing sections.

`AI_SECTION_FEATURE_BINDING` MUST be parseable with COVE-AI Shared support
alone. A section entry that references a non-existent or invalid
`feature_binding_ref` is unsupported for the referenced scope.
`AI_SECTION_FEATURE_BINDING` cannot itself depend on a feature binding that has
not yet been validated. `ArtifactRequired` section-local bits reject sidecar use
or the selected sidecar operation; they MUST NOT reject the referenced `.cove`
file.

Descriptor sections MUST validate section payload CRC32C before records are
used. For operations that use large payload-bearing sections, readers MAY
validate the referenced descriptor records first and then validate only the
addressed payload block, vector range, or token range through
`AiPayloadIntegrityV1`, unless the operation requests full-section validation.
In Phase 1, `AI_VECTOR_PAYLOAD_BLOCK` and
`AI_TOKEN_BLOCK` contain descriptor records only. Raw vector or token bytes MUST
be referenced through `AI_PAYLOAD_REF_TABLE` and stored in `AI_PAYLOAD_BYTES`
or another explicitly permitted payload carrier.

`AI_PAYLOAD_BYTES` is never semantically meaningful on its own. Its bytes are an
opaque carrier; meaning comes only from payload refs, integrity records,
vector/token descriptors, source bindings, and policy context. Readers MUST NOT
parse `AI_PAYLOAD_BYTES` as `AiRecordHeaderV1` records.

`AI_PAYLOAD_BYTES` does not by itself authorize payload exposure. If an
`AI_PAYLOAD_BYTES` section has `source_binding_ref = 0`, each referenced payload
range inherits source binding, privacy, visibility, redaction, and policy
context from the descriptor record and payload-ref chain that gives the byte
range semantic meaning. If an `AI_PAYLOAD_BYTES` section has non-zero
`source_binding_ref`, that binding is a section-level upper bound only. Readers
MUST still validate the descriptor record, payload ref, integrity record, source
binding, privacy summary, and policy context before exposing any byte range.

Phase 1 descriptor sections:

- `AI_COMPANION_ARTIFACT_REF`;
- `AI_SOURCE_BINDING`;
- `AI_REFERENCE_TABLES`;
- `AI_PRIVACY_SUMMARY`;
- `AI_PAYLOAD_INTEGRITY`;
- `AI_SECTION_FEATURE_BINDING`;
- `AI_TOKEN_BLOCK`;
- `AI_VECTOR_SPACE`;
- `AI_VECTOR_BINDING`;
- `AI_VECTOR_PAYLOAD_BLOCK`;
- `AI_VECTOR_DIRECTORY`;
- `AI_VECTOR_COMPOSITION`;
- `MAP_AI_PROFILE_CATALOG`;
- `MAP_AI_TEMPLATE_CATALOG`;
- `MAP_AI_TRAINING_POLICY_CATALOG`.

Phase 1 large payload-bearing sections:

- `AI_PAYLOAD_BYTES`.

Only Phase 1 large payload-bearing sections may set
`CoveAiSectionEntryV1.payload_crc32c == 0` when every used payload range is
covered by validated `AiPayloadIntegrityV1` records.

### Reference Spaces, Integrity, and Binary Records

COVE-AI structs use local `_ref` fields. Unless a structure explicitly says
otherwise, `0` means absent and non-zero references point into a declared
COVE-AI reference space in `AI_REFERENCE_TABLES` or into the local record table
of the section that owns the structure.

For Phase 1, a `.cove`, `CVA2`, or `CVV2` artifact that carries standard
COVE-AI records MUST contain at most one `AI_REFERENCE_TABLES` section unless a
required extension defines reference-table partitioning. All `_ref` values in
standard Phase 1 records resolve through that single table or through the local
record table explicitly named by the field.

Within one `AI_REFERENCE_TABLES` section, IDs MUST be unique per reference
space. Duplicate `string_ref`, `digest_ref`, `payload_ref`, `policy_ref`,
`source_span_ref`, or `transform_ref` values are structural corruption unless a
required extension defines scoped duplicates.

Standard COVE-AI reference spaces:

| Space | Used by |
| --- | --- |
| `AI_STRING_TABLE` | Names, namespaces, versions, locale tags, text templates, endpoint names, prompt template IDs, and media types. |
| `AI_DIGEST_TABLE` | Source file digests, dictionary digests, schema fingerprints, model/checkpoint digests, tokenizer material digests, payload digests, and transform digests. |
| `AI_POLICY_TABLE` | Visibility, redaction, sensitivity, license, safety, retention, disclosure, and export policies. |
| `AI_PAYLOAD_REF_TABLE` | Payload byte ranges, embedded blobs, masks, labels, prompt text, target text, metadata, and external payload handles. |
| `AI_FUNCTION_OR_TEMPLATE_TABLE` | Chunkers, tokenizers, vectorizers, transforms, templates, normalization pipelines, scoring functions, and deterministic split functions. |
| `AI_MASK_LABEL_TABLE` | Loss masks, attention masks, labels, position IDs, preference records, and quality-score payloads. |
| `AI_SOURCE_SPAN_TABLE` | Source rows, object refs, source values, evidence spans, byte ranges, token ranges, and asset time ranges. |
| `AI_TRANSFORM_TABLE` | Asset preprocessing, vector transforms, quantization transforms, calibration transforms, OCR/caption/transcript transforms, and image/audio/video normalization profiles. |
| `AI_EXTENSION_TABLE` | Registered extension records and vendor payload references. |

```rust
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

Future optional reference-directory record:

```rust
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
```

`AiReferenceEntryV1` is an optional directory/index record over typed reference
records. It is not required for Phase 1 and has no standard record-kind
assignment in the MVP. Readers MUST use the typed reference records below for
standard semantics.

`AiRecordHeaderV1.crc32c`, when present on the containing record, covers the
record bytes. `AiPayloadIntegrityV1.payload_crc32c`, when non-zero or flagged
present, covers the referenced payload bytes in the declared
`digest_domain`. `digest_ref` is the cryptographic identity used for trust,
payload-byte verification, replay, and audit claims. Digest algorithms use the
same registry as COVE-Core digest manifests. `digest_algorithm`, `digest_len`,
and `digest_ref` MUST agree with the referenced `AiDigestEntryV1`; mismatch is
structural corruption.

A zero CRC32C field means "not present" only when the applicable flags or
section rules permit absence. If `AI_FLAG_PAYLOAD_CRC32C_PRESENT` is set, the
CRC field MUST be interpreted as present and validated even when its numeric
value is zero.

`digest_domain` declares exactly which bytes were digested. For compressed
sections this distinguishes stored compressed bytes from decoded section payload
bytes and canonical record bytes. Model-input and external-asset digest domains
MUST include the declared transform or preprocessing profile in lineage.

Digest payloads used to validate `AI_PAYLOAD_INTEGRITY` MUST be readable after
bounds and CRC validation without requiring the same integrity record they are
used to validate. Cyclic integrity dependencies are invalid. Digest payload
bytes are opaque digest bytes; they MUST NOT be transformed, decoded as text, or
normalized before comparison. `digest_len` MUST equal the actual digest byte
length.

`AI_SECTION_FEATURE_BINDING` is the artifact-local analogue of
`SECTION_FEATURE_BINDING` for `CVA2`/`CVV2`. It cannot narrow artifact-required
bits in `CoveAiHeaderV1.required_ai_features`; it can only bind section-local
optional or required AI feature words to profile-, section-, or
operation-scoped use inside the companion artifact.

Unless a structure explicitly states otherwise, `payload_offset` in COVE-AI
binary records is an absolute byte offset from the start of the containing
`CVA2`/`CVV2` artifact. Offsets into decoded section payloads MUST be named
`section_payload_offset`. The Phase 1 exception is
`VectorEntryV1.payload_offset`, which COVE-VEC defines as an offset relative to
the resolved block payload, not an artifact-absolute offset.

Payload records with artifact-absolute `payload_offset` point to stored
artifact bytes, not decoded bytes inside a compressed section. If payload bytes
are inside a decoded compressed section, the record MUST use an
`AiPayloadRefEntryV1` storage kind that declares section-relative decoded
coordinates, and the coordinate field MUST be named `section_payload_offset`.
For the Phase 1 MVP, raw vector and token bytes MUST be referenced through
`AI_PAYLOAD_REF_TABLE`; `AI_VECTOR_PAYLOAD_BLOCK` and `AI_TOKEN_BLOCK` do not
inline raw bytes after their descriptor record arrays. The ordinary carrier is
an uncompressed `AI_PAYLOAD_BYTES` section addressed by `ArtifactAbsolute`
payload refs. Compressed vector or token payloads MUST be addressed through
`AI_PAYLOAD_REF_TABLE` with an explicit storage kind and digest domain.

All fields named `checksum` in COVE-AI V1 records are CRC32C fields computed
with the checksum field treated as zero, unless a profile-specific rule
explicitly says otherwise. Future revisions SHOULD prefer the field name
`crc32c`.

Unless a record-specific flag registry says otherwise, all unassigned `flags`
bits MUST be zero on write and MUST cause rejection when non-zero in a required
record. Optional records with unknown non-zero flags MAY be skipped and
reported.

Common COVE-AI V1 flags:

| Bit | Flag | Meaning |
| ---: | --- | --- |
| 0 | `AI_FLAG_REQUIRED_RECORD` | Unknown support rejects the selected section/profile/operation. |
| 1 | `AI_FLAG_PAYLOAD_CRC32C_PRESENT` | `payload_crc32c` or an equivalent payload CRC field is present and must validate. |
| 2 | `AI_FLAG_POLICY_PROTECTED` | Payload or metadata requires policy validation before exposure. |
| 3 | `AI_FLAG_REVOKED` | Record or source binding is revoked for governed reads unless trusted policy overrides. |

Common COVE-AI V1 flags apply to `AiRecordHeaderV1.flags` unless a section
schema explicitly says they apply to a payload record's `flags` field. Payload
record `flags` fields are record-specific. Unknown non-zero payload flags in
required records reject unless the record schema declares them advisory.

Minimal standard reference records:

```rust
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
```

`AiPrivacySummaryEntryV1` is valid only for its declared
`source_binding_ref`, `policy_ref`, `visibility_scope_ref`, and
`redaction_scope_ref`. A mismatch between the privacy summary and the selected
source binding or policy context makes all payload-bearing records under that
source binding policy-blocked.

`AiPayloadRefEntryV1` storage-kind validation:

- If `storage_kind = ArtifactAbsolute`, `payload_offset` and `payload_length`
  are used; `section_id`, `section_payload_offset`, and `uri_ref` MUST be zero
  unless a required extension says otherwise.
- If `storage_kind = SectionDecodedRelative`, `section_id`,
  `section_payload_offset`, and `payload_length` are used; `payload_offset` and
  `uri_ref` MUST be zero.
- If `storage_kind = ExternalUri`, `uri_ref` and `payload_length` are used when
  known; artifact and section offsets MUST be zero.
- If `storage_kind = EmbeddedSection`, `section_id` is used and the payload is
  the entire decoded section unless offsets are explicitly allowed by the
  section schema.

For Phase 1 token and vector payloads, an `AiPayloadRefEntryV1` used by
`TokenBlockHeaderV1.payload_ref` or `VectorPayloadBlockHeaderV1.payload_ref`
MUST identify bytes contained in an `AI_PAYLOAD_BYTES` section, unless a
required extension declares another payload carrier. It MUST NOT identify bytes
inside `AI_TOKEN_BLOCK` or `AI_VECTOR_PAYLOAD_BLOCK`.

For Phase 1 token and vector payload bytes, an `AiPayloadRefEntryV1` with
`storage_kind = ArtifactAbsolute` MUST identify a byte range fully contained in
exactly one `AI_PAYLOAD_BYTES` section. The containing `AI_PAYLOAD_BYTES`
section MUST be uncompressed. If the range is outside all `AI_PAYLOAD_BYTES`
sections, overlaps more than one `AI_PAYLOAD_BYTES` section, overflows its
offset/length arithmetic, or points into a compressed `AI_PAYLOAD_BYTES`
section, the payload ref is invalid.

If a Phase 1 token or vector payload is stored in a compressed
`AI_PAYLOAD_BYTES` section, the payload ref MUST use
`storage_kind = SectionDecodedRelative`, `section_id` MUST identify that
`AI_PAYLOAD_BYTES` section, and the decoded-section range MUST fit within the
section's decoded payload length. `ArtifactAbsolute` refs into decoded bytes of
a compressed `AI_PAYLOAD_BYTES` section are invalid.

For Phase 1 token and vector payload blocks, `payload_ref` is the authoritative
payload-range reference. `TokenBlockHeaderV1.payload_offset`,
`TokenBlockHeaderV1.payload_length`,
`VectorPayloadBlockHeaderV1.payload_offset`, and
`VectorPayloadBlockHeaderV1.payload_length` are cached artifact-absolute
coordinates only for resolved `ArtifactAbsolute` payload refs. If
`payload_length != 0`, the cached range MUST exactly match the resolved
`AiPayloadRefEntryV1.payload_offset` and `payload_length`. If
`payload_length == 0`, cached `payload_offset` MUST also be zero and readers
MUST use the resolved payload ref. If the resolved payload ref uses any storage
kind other than `ArtifactAbsolute`, the cached block `payload_offset` and
`payload_length` fields MUST both be zero. A mismatch is structural corruption.

For `TokenBlockHeaderV1.integrity_ref` and
`VectorPayloadBlockHeaderV1.integrity_ref`, the referenced
`AiPayloadIntegrityV1.payload_ref` MUST equal the block's `payload_ref`, unless
a required extension defines a covering-range integrity scheme. For
`VectorEntryV1.integrity_ref != 0`, the referenced
`AiPayloadIntegrityV1.payload_ref` MUST identify exactly the vector entry's
resolved payload byte range, unless a required extension defines a
covering-range, Merkle-style, or profile-equivalent integrity scheme. A
per-vector integrity record that verifies a different byte range is structural
corruption.

Minimum standard record-kind assignments:

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
| `AI_VECTOR_SPACE` | 1 | `VectorSpaceDescriptorV1` |
| `AI_VECTOR_SPACE` | 2 | `VectorSpaceCompatibilityDescriptorV1` |
| `AI_VECTOR_BINDING` | 1 | `FileCodeVectorBindingV1` |
| `AI_VECTOR_BINDING` | 2 | `ChunkVectorBindingV1` |
| `AI_VECTOR_BINDING` | 3 | `ObjectStateVectorBindingV1` |
| `AI_VECTOR_BINDING` | 4 | `TrainingSampleVectorBindingV1` |
| `AI_VECTOR_PAYLOAD_BLOCK` | 1 | `VectorPayloadBlockHeaderV1` |
| `AI_VECTOR_DIRECTORY` | 1 | `VectorEntryV1` |
| `AI_VECTOR_COMPOSITION` | 1 | `VectorCompositionProfileV1` |
| `AI_VECTOR_COMPOSITION` | 2 | `VectorCompositionComponentV1` |
| `AI_VECTOR_COMPOSITION` | 3 | `VectorArithmeticProfileV1` |

`AI_PAYLOAD_BYTES` has no record-kind assignment in Phase 1. It is addressed
only through `AiPayloadRefEntryV1` records and the descriptors that reference
those payload refs.

Phase 1 and near-Phase 1 field-reference resolution for COVE-AI Shared,
COVE-TOK token blocks, and COVE-VEC value-vector records:

| Struct | Field | Resolves to |
| --- | --- | --- |
| `AiCompanionArtifactRefV1` | `uri_ref` | `AI_STRING_TABLE` URI string. |
| `AiCompanionArtifactRefV1` | `artifact_digest_ref` | `AI_DIGEST_TABLE`. |
| `AiCompanionArtifactRefV1` | `source_binding_ref` | `AiSourceBindingV1.source_binding_id` in `AI_SOURCE_BINDING`, or `0` when the sidecar supplies its own binding. |
| `AiSourceBindingV1` | `source_artifact_ref` | `AI_PAYLOAD_REF_TABLE` for an embedded/source artifact handle, or `AI_STRING_TABLE` URI when declared by source-kind rules. |
| `AiSourceBindingV1` | `source_file_digest_ref`, `schema_fingerprint_ref`, `dictionary_digest_ref`, `map_fingerprint_ref` | `AI_DIGEST_TABLE`. |
| `AiSourceBindingV1` | `policy_context_ref`, `visibility_scope_ref`, `redaction_scope_ref` | `AI_POLICY_TABLE`. |
| `AiSourceBindingV1` | `branch_ref` | `AI_STRING_TABLE`, or `0` when not branch-scoped. |
| `AiPayloadIntegrityV1` | `payload_ref` | `AI_PAYLOAD_REF_TABLE`. |
| `AiPayloadIntegrityV1` | `digest_ref` | `AI_DIGEST_TABLE`. |
| `VectorSpaceDescriptorV1` | `vector_space_name_ref`, `embedding_namespace_ref`, `embedding_model_ref`, `embedding_model_version_ref` | `AI_STRING_TABLE`. |
| `VectorSpaceDescriptorV1` | `vector_space_fingerprint_ref`, `embedding_model_digest_ref` | `AI_DIGEST_TABLE`. |
| `VectorSpaceDescriptorV1` | `embedding_pipeline_ref` | `AI_TRANSFORM_TABLE`; the transform entry MAY reference `AI_FUNCTION_OR_TEMPLATE_TABLE`. |
| `VectorSpaceDescriptorV1` | `tokenizer_profile_ref` | `TokenizerProfileV1.tokenizer_profile_id` in `AI_TOKENIZER_PROFILE`, or `0`. |
| `VectorSpaceDescriptorV1` | `chunk_profile_ref` | `ChunkProfileV1.chunk_profile_id` in `AI_CHUNK_PROFILE`, or `0`. |
| `TokenBlockHeaderV1` | `tokenizer_profile_id` | `TokenizerProfileV1.tokenizer_profile_id` in `AI_TOKENIZER_PROFILE`. |
| `TokenBlockHeaderV1` | `payload_ref` | `AI_PAYLOAD_REF_TABLE`. |
| `TokenBlockHeaderV1` | `integrity_ref` | `AiPayloadIntegrityV1.integrity_ref` in `AI_PAYLOAD_INTEGRITY`, or `0`. |
| `FileCodeVectorBindingV1` | `slot_policy_ref` | `MapAiSlotPolicyV1` record in `MAP_AI_PROFILE_CATALOG`, or `0`. |
| `FileCodeVectorBindingV1` | `file_ref` | `AiSourceBindingV1.source_binding_id`, or `0` to inherit the containing section source binding. |
| `FileCodeVectorBindingV1` | `dictionary_digest_ref`, `schema_fingerprint_ref`, `canonical_value_hash_ref` | `AI_DIGEST_TABLE`. |
| `FileCodeVectorBindingV1` | `path_ref` | `AI_STRING_TABLE` canonical COVE-T/COVE-MAP path string unless a required path-record extension is declared. |
| `FileCodeVectorBindingV1` | `vector_ref` | `VectorEntryV1.vector_ref` in `AI_VECTOR_DIRECTORY`. |
| `VectorPayloadBlockHeaderV1` | `payload_ref` | `AI_PAYLOAD_REF_TABLE`. |
| `VectorPayloadBlockHeaderV1` | `tensor_layout_ref` | `AI_TENSOR_LAYOUT` record, or `0`. |
| `VectorPayloadBlockHeaderV1` | `payload_stride_ref` | `AI_TENSOR_LAYOUT`, `AI_PAYLOAD_REF_TABLE`, or `0` for the dense-row-major default declared by COVE-VEC. |
| `VectorPayloadBlockHeaderV1` | `device_transfer_hint_ref` | `AI_TENSOR_LAYOUT` or profile-defined device-transfer descriptor, or `0`. |
| `VectorPayloadBlockHeaderV1` | `integrity_ref` | `AiPayloadIntegrityV1.integrity_ref` in `AI_PAYLOAD_INTEGRITY`, or `0`. |
| `VectorEntryV1` | `block_id` | `VectorPayloadBlockHeaderV1.block_id` in `AI_VECTOR_PAYLOAD_BLOCK`. |
| `VectorEntryV1` | `integrity_ref` | `AiPayloadIntegrityV1.integrity_ref` in `AI_PAYLOAD_INTEGRITY`, or `0` to inherit block integrity. |

Resolving a Phase 1 `_ref` field through any reference space other than the one
declared above is structural corruption, even if the numeric ID exists in the
wrong table. Extensions that change a field's reference space MUST be required
features and MUST define reject/fallback behavior.

Minimum enum assignments for Phase 1 interoperability:

```rust
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
```

Phase 1 vector element byte widths:

| Element type | Width |
| --- | ---: |
| `Float32` | 4 bytes |
| `Float16` | 2 bytes |
| `BFloat16` | 2 bytes |
| `Int8` | 1 byte |
| `UInt8` | 1 byte |
| `Binary` | Bit-packed; byte width MUST be declared by payload or layout metadata. |

Binary `AI_*` sections are arrays of length-delimited records.

```rust
struct AiRecordHeaderV1 {
    record_kind: u16,
    record_version: u16,
    record_len: u32,
    local_id: u64,
    flags: u32,
    crc32c: u32,
}
```

Rules:

- `record_len` includes the `AiRecordHeaderV1` bytes and the record payload.
- For Phase 1 standard COVE-AI records, `record_version` MUST be 1. Readers
  MUST reject unsupported record versions. A future minor-compatible record
  extension must be gated by feature bits or flags and must preserve the
  declared record length and required-field semantics.
- Records MUST be wholly contained in the section payload and MUST NOT overlap.
- `local_id` is unique within `(section_kind, record_kind)` unless the
  section-specific rules state that duplicate IDs are invalid across a wider
  scope.
- Unknown optional record kinds MAY be skipped after bounds and CRC validation.
- Unknown required record kinds reject only the section, profile, or operation
  selected by feature binding.
- All standard Phase 1 binary `AI_*` descriptor sections MUST use
  `AiRecordHeaderV1`. `AI_PAYLOAD_BYTES` is an opaque byte-carrier section and
  MUST NOT be parsed as records. Other headerless homogeneous fixed-array
  sections are reserved for a future required extension.

### Fallback and Operation Behavior

| Operation | Missing or unsupported optional AI artifact | Stale or corrupt artifact | Required artifact missing or unsupported |
| --- | --- | --- | --- |
| Ordinary COVE-T scan | Ignore and continue. | Ignore and continue. | Must not occur for ordinary scan unless a non-AI required feature is involved. |
| COVE-O reconstruction | Ignore and continue. | Ignore and continue. | Reject only if policy explicitly made the AI artifact part of the requested operation. |
| COVE-MAP projection readback | Ignore AI metadata and use normal COVE-MAP rules. | Ignore AI metadata and report it. | Reject only AI-enhanced projection features. |
| `.chunks()` | Recompute only if source text, chunk profile, and policy allow it; otherwise reject. | Reject stored chunks or recompute from source if allowed. | Reject with structured diagnostics. |
| `.tokens()` / `.pack()` | Retokenize only if tokenizer material and policy allow it; otherwise reject. | Reject stored token blocks or retokenize if allowed. | Reject with structured diagnostics. |
| `.embedding()` | Recompute only if vectorizer/profile is available and policy allows it; otherwise reject. | Reject stored vectors or recompute if allowed. | Reject with structured diagnostics. |
| `.similar()` | Fall back from index to exact vector scan when vector payloads are valid; otherwise reject. | Ignore stale index; reject stale vector payloads unless recompute is allowed. | Reject with structured diagnostics. |
| `.trainingSamples()` | Reject unless source data and deterministic policy can reconstruct samples. | Reject stored samples or reconstruct if allowed. | Reject with structured diagnostics. |
| `.multimodal()` | Expose individual components only; do not claim sequence reconstruction. | Reject sequence pack; expose validated independent components if requested. | Reject with structured diagnostics. |

All AI operation diagnostics SHOULD include the artifact ID, section kind,
feature bit, source binding, freshness check that failed, fallback attempted,
and policy reason if disclosure is allowed.

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
    StoredPayloadVerifiable = 3,
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
| `StoredPayloadVerifiable` | Derived artifact bytes are stored and digest-verified; no independent recomputation claim is implied. |
| `CanonicalRecomputeReproducible` | Independent implementations can recompute the same derived bytes under a strict canonical algorithm. |
| `ExternalAuditOnly` | External model/API/tool provenance is recorded, but deterministic regeneration is not claimed. |

COVE-AI metadata MUST NOT imply a stronger reproducibility class than its
declared lineage supports.

`AiReproducibilityClassV1` values are category identifiers, not a total numeric
ordering. `ExternalAuditOnly` is not stronger than
`CanonicalRecomputeReproducible`; it is a different claim class. Readers MUST
NOT compare these enum values numerically to decide trust or reproducibility.

External model/API outputs SHOULD normally be `ExternalAuditOnly` unless the
model, weights, runtime, prompt, decoding, seed, toolchain, and deterministic
generation algorithm are sufficiently specified.

Runtime floating-point vector composition SHOULD normally use
`VectorResultAuthorityV1::RuntimeAdvisory`. `RuntimeAdvisory` is not a
reproducibility class; it is the result-authority classification for a produced
or composed vector. Such a result MUST NOT claim `StoredPayloadVerifiable`
unless the result is materialized and digested.

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

### Artifact Placement

COVE-MAP-AI policy is stored as COVE-MAP payload, not as vector payload. The
authoritative reusable AI intent for a mapping version SHOULD live in the
`.covemap` artifact that owns the semantic mapping, using `MAP_AI_*` sections.
Embedded `MAP_AI_*` sections inside `.cove` outputs are file-local snapshots,
conversion evidence, or inspectable policy summaries tied to that output.

In embedded `.cove` section entries, `MAP_AI_*` sections use profile ID 17
(`COVE-MAP-AI`) and follow the COVE-MAP payload discipline. In `.covemap`
artifacts, `MAP_AI_*` section IDs 70-72 are COVE-MAP payload sections following
the `.covemap` artifact's own section-entry grammar. Readers that do not
recognize profile ID 17 MUST treat optional embedded `MAP_AI_*` sections as
ignorable optional sections and preserve their presence in inspect/report output
where possible. If an implementation validates profile IDs strictly and cannot
accept profile ID 17 yet, it MUST NOT claim COVE-MAP-AI support.

COVE-AI does not require full source-to-object mapping for plain COVE-T files.
A writer MAY store a minimal `.covemap` artifact whose only purpose is AI slot
policy over COVE-T table/column/path refs, or embed file-local optional
`MAP_AI_*` sections when no reusable mapping definition exists. Such use does
not imply support for COVE-MAP source-to-object conversion, identity resolution,
or projection readback.

Rules:

- `MAP_AI_PROFILE_CATALOG` declares `MapAiProfileV1` and
  `MapAiSlotPolicyV1` records.
- `MAP_AI_TEMPLATE_CATALOG` declares templates and template fingerprints used
  by vectorization, chunking, prompt context, or generated sample assembly.
- `MAP_AI_TRAINING_POLICY_CATALOG` declares slot-level sample, label, split,
  weighting, dedup, and quality intent.
- Reusable `.covemap` payloads MUST follow the COVE-MAP v2 payload discipline:
  declared schema ID, mapping ID, mapping version, canonical JSON or
  deterministic CBOR, stable IDs, no duplicate keys, and no undeclared
  extension fields.
- A COVE-MAP-aware tool that does not implement COVE-MAP-AI MAY ignore optional
  `MAP_AI_*` sections, but it MUST preserve their presence in inspect/report
  output where possible.
- A COVE-MAP-AI-aware tool MUST validate source refs, table/column refs,
  object/property refs, association refs, path refs, policy refs, template refs,
  vector-space refs, chunk-profile refs, tokenizer refs, and training-policy
  refs before using a slot policy.
- Within one active AI profile, duplicate slot policies for the same path are
  invalid unless an explicit precedence rule is declared. Across multiple
  active profiles, `Forbidden` fails closed unless the selected operation
  supplies a trusted policy override whose authority and audit record are
  declared.
- `Forbidden` and redaction decisions fail closed. If a later AI sidecar
  contains vectors, tokens, chunks, samples, sequence elements, or labels for a
  forbidden slot, the affected AI operation MUST reject.

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

COVE-CHUNK payloads live in `AI_CHUNK_PROFILE` and `AI_TEXT_CHUNK_INDEX`
sections. They MAY be stored in `.coveai`, embedded `.cove` sections, or
another COVE-AI-compatible sidecar. A stored chunk index is valid only for the
source value hash, normalization policy, chunk profile, tokenizer profile when
token windows are used, visibility scope, and redaction scope it declares.

Reader obligations:

- validate every chunk byte span against the source value before exposing text;
- validate UTF-8 boundary alignment for text chunks;
- reject or withhold parent, child, previous, next, and sibling navigation when
  the referenced chunk is outside policy or fails validation;
- expose chunk text only after redaction and visibility policy checks;
- treat chunk hierarchy and neighboring-context expansion as retrieval
  structure, not source document truth;
- recompute chunks only when the source value, deterministic chunk profile, and
  policy are all available.

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
    unicode_scalar_start: u64,
    unicode_scalar_length: u64,
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
- byte spans are authoritative for source binding; Unicode scalar offsets are
  advisory navigation metadata over the declared normalized text form;
- `unicode_scalar_start` and `unicode_scalar_length` count Unicode scalar
  values, not UTF-16 code units or grapheme clusters;
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

COVE-TOK metadata lives in `AI_TOKENIZER_PROFILE`, `AI_TOKEN_BLOCK`,
`AI_TOKENIZED_SPAN`, and `AI_TOKEN_SEQUENCE_PACK` sections. Persisted token
bytes live in `AI_PAYLOAD_BYTES` and are referenced through
`AI_PAYLOAD_REF_TABLE`. Token IDs are derived data and may leak source text, so
token payloads are policy-protected surfaces even when the source text is not
directly exposed.

Required `.coveai` sections for the token-cache MVP:

- `AI_REFERENCE_TABLES`;
- `AI_SOURCE_BINDING`;
- `AI_TOKENIZER_PROFILE`;
- `AI_TOKEN_BLOCK`;
- `AI_TOKENIZED_SPAN`;
- `AI_PAYLOAD_BYTES` when token payload bytes are persisted in the artifact;
- `AI_PAYLOAD_INTEGRITY` when any token block claims `StoredPayloadVerifiable`,
  replayability, auditability, or trust-chain participation.

Reader obligations:

- validate tokenizer namespace, name, version, vocabulary digest, merges digest,
  pre-tokenizer digest, normalizer digest, byte encoder/decoder digest,
  added-token digest, special-token digest, chat template, Unicode version,
  truncation/padding policy, token ID width, and reversibility flags before
  reusing a token block;
- support only declared `token_id_width` values of 1, 2, 4, or 8 bytes unless a
  required extension defines another width;
- bounds-check every token offset, token count, byte-alignment ref, mask ref,
  label ref, and position-id ref before export;
- reject loss masks, attention masks, labels, or position IDs that are shorter
  than their scoped token range or reference tokens outside the sequence pack;
- retokenize only when tokenizer material is available, deterministic, policy
  permits it, and the operation does not require stored token bytes;
- never reuse token IDs across tokenizer profiles unless a compatibility
  descriptor explicitly proves equivalence.

```rust
struct TokenizerProfileV1 {
    tokenizer_profile_id: u32,
    tokenizer_namespace_ref: u32,
    tokenizer_name_ref: u32,
    tokenizer_version_major: u16,
    tokenizer_version_minor: u16,
    vocab_digest_ref: u32,
    merges_digest_ref: u32,
    pre_tokenizer_digest_ref: u32,
    normalizer_digest_ref: u32,
    byte_encoder_digest_ref: u32,
    special_tokens_digest_ref: u32,
    added_tokens_digest_ref: u32,
    chat_template_ref: u32,
    unicode_version_ref: u32,
    truncation_policy_ref: u32,
    padding_policy_ref: u32,
    model_max_sequence_length: u32,
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
    payload_ref: u32,
    payload_offset: u64,
    payload_length: u64,
    integrity_ref: u32,
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
positions. A `TokenBlockHeaderV1` that claims `StoredPayloadVerifiable`,
replayability, auditability, or trust-chain participation MUST set
`integrity_ref` to a validated `AiPayloadIntegrityV1` record.
`TokenBlockHeaderV1.payload_ref` MUST resolve to an `AiPayloadRefEntryV1`.
For Phase 1 token payloads, that payload ref MUST point into an
`AI_PAYLOAD_BYTES` section; token bytes MUST NOT be appended to
`AI_TOKEN_BLOCK` after the descriptor records.

## COVE-VEC

COVE-VEC stores vectors once per distinct semantic unit and lets rows, objects,
chunks, samples, and multimodal assets reference them.

Recommended optimized artifact:

```text
Extension: .covev
Magic: CVV2
Profile: COVE-VEC
```

COVE-VEC may also be embedded in COVX or COVE-I-style extension artifacts for
implementations that do not initially want a separate file type.

### Artifact Boundary

`.covev` uses the `CoveAiPostscriptV1`, `CoveAiHeaderV1`,
`AiSourceBindingV1`, and `CoveAiSectionEntryV1` envelope with magic `CVV2`.
The same COVE-VEC logical sections MAY also appear in `.coveai` bundles, COVX,
or COVE-I-style index artifacts, but `.covev` is the preferred carrier for
large vector payloads and vector indexes.

Required `.covev` sections for value-vector MVP:

- `AI_REFERENCE_TABLES`;
- `AI_SOURCE_BINDING`;
- `AI_PRIVACY_SUMMARY`;
- `AI_VECTOR_SPACE`;
- `AI_VECTOR_BINDING`;
- `AI_VECTOR_PAYLOAD_BLOCK`;
- `AI_VECTOR_DIRECTORY`;
- `AI_PAYLOAD_BYTES` when vector payload bytes are persisted in the artifact;
- `AI_PAYLOAD_INTEGRITY` when any vector payload block claims
  `StoredPayloadVerifiable`, replayability, auditability, or trust-chain
  participation.

A sidecar without `AI_PRIVACY_SUMMARY` MAY validate structurally, but a direct
reader MUST treat payload-bearing sections as policy-blocked unless a trusted
caller policy overrides the missing summary.

Optional `.covev` sections:

- `AI_VECTOR_COMPOSITION`;
- `AI_VECTOR_INDEX`;
- `AI_TENSOR_LAYOUT`;
- `AI_ASSET_MANIFEST` when vectors bind to external assets.

Reader obligations:

- validate the source binding before reading vector bindings or payload bytes;
- validate vector-space dimension, element type, metric, normalization policy,
  quantization policy, model lineage, template fingerprint, and reproducibility
  class before comparing vectors;
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
- fall back from a stale or unsupported vector index to exact vector scan when
  vector payloads are valid and the query permits it.

### Vector Space

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
```

Vectors from different `vector_space_id` values MUST NOT be compared unless a
compatibility descriptor explicitly permits it. Metric and normalization policy
are part of vector-space identity. If query-time embedding is performed through
an external service, the operation context MUST record service/model identity
and whether the result is reproducible or audit-only.

`vector_space_id` is a local descriptor ID. Cross-artifact identity uses
`vector_space_fingerprint_ref`, which MUST digest the vector-space descriptor
fields that affect vector interpretation: model lineage, template lineage,
pipeline, tokenizer/chunker dependencies, dimension, element type, metric,
normalization, and quantization. Compatibility descriptors have their own
identity and are not included in the intrinsic vector-space fingerprint unless a
specific compatibility profile explicitly says otherwise. Reusing a numeric
`vector_space_id` with a different fingerprint is a validation error within one
artifact and a mismatch across artifacts.

```rust
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

A compatibility descriptor MAY permit comparison, projection, calibration, or
declared transform between vector spaces. It MUST NOT silently make two vector
spaces identical.

For deterministic numeric transforms, `numeric_transform_error_ppm` may bound
numeric error under the declared transform. For learned embedding compatibility,
implementations SHOULD normally use `ranking_eval_ref`,
`calibration_dataset_ref`, and an advisory or calibrated
`compatibility_authority` instead of claiming numeric equivalence. If
`transform_ref` is non-zero, the transform is required for the comparison and
must be supported or the operation rejects. Explain output MUST disclose whether
compatibility was exact, transformed, calibrated, evaluated, or advisory.

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
    file_code: u32,
    reserved0: u32,
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

`file_code` is the COVE v2 `FileCode` type and is therefore `u32`.
`reserved0` MUST be zero. Future wider dictionary-code schemes require a
required extension and MUST NOT be interpreted as ordinary v2 FileCodes.

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

Recommended logical exports include `FixedSizeList<Float32>`,
`FixedSizeList<Float16>`, `FixedSizeBinary` for quantized vectors, Arrow
extension types for tensor/quantized/PQ layouts, and DLPack-compatible export
when tensor layout and lifetime allow.

`payload_stride_ref` resolves through `AI_PAYLOAD_REF_TABLE`,
`AI_TENSOR_LAYOUT`, or a profile-defined tensor layout record. It MUST NOT be a
free-form implementation-local pointer.

`VectorPayloadBlockHeaderV1.payload_ref` MUST resolve to an
`AiPayloadRefEntryV1`. For Phase 1 vector payloads, that payload ref MUST
point into an `AI_PAYLOAD_BYTES` section; vector bytes MUST NOT be appended to
`AI_VECTOR_PAYLOAD_BLOCK` after the descriptor records.

For `AiLayoutKindV1::DenseRowMajor` fixed-size vectors,
`payload_stride_ref = 0` means tightly packed vectors with byte stride equal to
`dimension_count * element_width`. A non-zero `payload_stride_ref` MUST
reference a tensor/layout descriptor or payload descriptor that declares byte
stride.

A `VectorPayloadBlockHeaderV1` that claims `StoredPayloadVerifiable`,
replayability, auditability, or trust-chain participation MUST set
`integrity_ref` to a validated `AiPayloadIntegrityV1` record. CRC32C alone is
insufficient for those claims.

`vector_ref` values in vector bindings resolve through `AI_VECTOR_DIRECTORY`.
`VectorEntryV1.payload_offset` and `payload_length` identify the vector's byte
range within the resolved `VectorPayloadBlockHeaderV1.payload_ref` block
payload. The vector range MUST be fully contained in the resolved block payload
range. If `VectorEntryV1.payload_length != 0`, `payload_offset` is an explicit
byte offset from the start of the resolved block payload. If
`VectorEntryV1.payload_length == 0`, `payload_offset` MUST also be zero and the
reader MAY derive the vector byte range only for fixed-stride dense vectors from
`block_id`, `vector_ordinal`, vector element width, dimension count, and stride.
Variable-size, sparse, quantized, external, or extension vector payloads MUST
use explicit `payload_offset` and `payload_length`.

If `VectorEntryV1.integrity_ref == 0`, the entry inherits the block integrity
record from `VectorPayloadBlockHeaderV1.integrity_ref`. If
`VectorEntryV1.integrity_ref != 0`, that integrity record verifies only that
vector's resolved payload byte range under the common payload-integrity target
matching rule and MUST be consistent with the block-level digest domain and
vector-space descriptor. For fixed-size dense vectors,
`VectorEntryV1.payload_length`, when explicit, MUST equal the declared vector
byte width; it may be zero only under the derivation rule above.

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
| Persisted value vectors | `StoredPayloadVerifiable` |
| Persisted chunk vectors | `StoredPayloadVerifiable` |
| Persisted object vectors | `StoredPayloadVerifiable` |
| Runtime-composed vectors | `RuntimeAdvisory` |
| Canonical fixed-point composed vectors | `CanonicalRecomputeReproducible` |
| ANN candidate rankings | `RuntimeAdvisory` unless exact index semantics are proven |

## Vector Indexes

COVE-VEC defines vector index descriptors only to the extent needed to bind an
index to vector spaces, payloads, quantization, visibility, redaction, and
lineage. Generic proof semantics, false-negative policy, index-only capability,
sidecar validity, coverage/fallback behavior, and dataset-level publication use
the same obligations as COVX and COVE-I. A `.covev` artifact that contains ANN
or secondary-index structures MUST satisfy those COVX/COVE-I-compatible
obligations; it is not a second incompatible index standard.

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

`vector_space_id` is the logical query/result vector space advertised by the
index. `stored_vector_space_id` is the space of vectors physically indexed.
`search_vector_space_id` is the space in which distance or similarity scores
are computed after any declared transform, quantization, or dequantization.

Approximate vector indexes MAY return candidates. They MUST NOT claim complete
nearest-neighbor results unless their descriptor proves exactness for the
requested metric and query class. A semantic search operation MUST disclose
approximate/exact status in explain output. An index that may have false
negatives MUST NOT be used as proof that no matching vector, object, or chunk
exists unless the operation explicitly accepts approximate recall.

Quantized or compressed indexes MUST declare whether scores are computed in the
stored vector space, dequantized vector space, product-quantized approximate
space, binary/Hamming space, or device-native compressed space. Exactness claims
are valid only for the declared `search_vector_space_id`, metric,
`score_space_authority`, dequantization profile, quantization error profile, and
query class.

If predicate, temporal, visibility, or redaction filters are applied after
candidate generation, filtered top-k results MUST be marked possibly incomplete
unless the candidate generator proves coverage for the filtered universe or the
operation uses exact scan over the filtered vector set.

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

External assets MUST be fingerprinted by digest if replayability or training
reproducibility is claimed. A URI alone is not stable source identity. Derived
captions, OCR text, transcripts, embeddings, and labels SHOULD bind back to the
source asset digest and generator provenance where applicable.

Asset preprocessing that affects model input MUST be declared. This includes
EXIF orientation, color management, resize/crop/pad policy, video frame
extraction, audio resampling, normalization, OCR, captioning, transcription, and
model-specific image/audio/video preprocessing. Derived assets SHOULD set
`parent_asset_ref`, `transform_profile_ref`, and `transform_digest_ref` so the
derived-asset chain is auditable.

`preprocessing_profile_ref` describes the model-input preprocessing applied when
the asset is fed to a tokenizer, encoder, embedding model, or training sample.
`transform_profile_ref` describes the derived-asset lineage from
`parent_asset_ref` to this asset. They may reference the same transform only
when the stored derived asset is exactly the model input.

## COVE-MMSEQ

COVE-MMSEQ preserves the order and semantics of model-consumable multimodal
sequences such as system text, user text, image, assistant text, audio clip,
tool call, tool result, assistant text, and label.

COVE-MMSEQ payloads live in `AI_MULTIMODAL_SEQUENCE`, `AI_ASSET_MANIFEST`,
`AI_TOKEN_SEQUENCE_PACK`, `AI_TENSOR_LAYOUT`, and, when needed,
`AI_GENERATOR_PROVENANCE` sections. The sequence pack is the model-consumable
ordering surface; referenced text, tokens, assets, tensors, labels, and
evidence remain separately validated surfaces.

Reader obligations:

- validate that element ordinals are unique and form the declared sequence
  order;
- validate every referenced token span, asset, tensor, vector, label, evidence,
  and policy record before exposing the assembled sequence;
- reject sequence reconstruction when an element is policy-blocked, stale,
  missing, or unsupported and no declared fallback exists;
- apply loss masks, attention masks, labels, role markers, and position maps to
  unambiguous element or token ranges;
- digest-bind external assets when replayability, auditability, or training
  reproducibility is claimed;
- allow non-MMSEQ-aware readers to expose validated independent components, but
  not to claim model-consumable sequence reconstruction.

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

COVE-TRAIN payloads live in `AI_TRAINING_PROFILE`,
`AI_TRAINING_SAMPLE_INDEX`, `AI_TRAINING_SPLIT_DEDUP_EPOCH`,
`AI_LABEL_PREFERENCE`, `AI_GENERATOR_PROVENANCE`, and, for multimodal corpora,
`AI_MULTIMODAL_SEQUENCE` sections.

Reader obligations:

- validate the source snapshot, mapping profile, chunk profile, tokenizer
  profile, vector space, split policy, sampling policy, dedup policy, quality
  policy, license policy, and redaction policy before exporting samples;
- reject a training split that claims reproducibility but omits source snapshot,
  hash function, seed, filters, grouping, ordering, or dedup behavior needed to
  replay the split;
- reject evaluation or holdout exports when a dedup group, source grouping, or
  benchmark exclusion rule proves contamination under the declared policy;
- validate sample weights, quality scores, labels, loss masks, attention masks,
  preference pairs, and generator provenance before export;
- preserve deterministic sample order when an epoch plan declares one;
- export policy-withheld samples as rejected/withheld diagnostics, not as
  silently skipped rows, when reproducibility or auditability is claimed.

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
    token_sequence_pack_ref: u64,
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
    source_snapshot_ref: u32,
    filter_policy_ref: u32,
    seed: u64,
    hash_function_ref: u32,
    stratification_path_ref: u32,
    grouping_ref: u32,
    ordering_policy_ref: u32,
    dedup_policy_ref: u32,
    sample_count: u64,
    first_sample_ref: u64,
    flags: u32,
    checksum: u32,
}

enum DedupAuthorityV1 {
    Exact = 0,
    Conservative = 1,
    Advisory = 2,
    Approximate = 3,
    ModelScored = 4,
    PolicyWithheld = 5,
    Extension = 255,
}

struct DedupGroupV1 {
    dedup_group_id: u64,
    dedup_policy_ref: u32,
    canonical_member_sample_id: u64,
    similarity_kind: u8,
    dedup_authority: u8,
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
    rng_algorithm_ref: u32,
    permutation_function_ref: u32,
    shard_count: u32,
    first_shard_ref: u32,
    shard_ref_count: u32,
    flags: u32,
    checksum: u32,
}
```

A split that claims reproducibility MUST declare its source snapshot, hash/seed
policy, filters, grouping, dedup behavior, and ordering. Training/evaluation
leakage controls SHOULD be represented through dedup groups, source grouping,
benchmark exclusion lists, and policy metadata.

A split or epoch plan seed is not sufficient on its own. Reproducible splits and
epoch plans MUST declare the hash function, RNG algorithm, ordering policy,
filter policy, dedup policy, grouping policy, and permutation function needed to
produce the same sample membership and order.

If `DatasetSplitV1.first_sample_ref` and `sample_count` are used as a contiguous
range, the training sample index MUST be sorted by split and sample ordinal for
that profile. Otherwise the split MUST use a split-membership payload referenced
through `AI_SOURCE_SPAN_TABLE` or `AI_PAYLOAD_REF_TABLE`.

Dedup groups may be exact, conservative, advisory, approximate, or model-scored.
An approximate or advisory dedup group MAY guide sampling quality, but it MUST
NOT be used as proof that evaluation contamination is impossible.

### Labels and Preferences

```rust
enum TrainingLabelAuthorityV1 {
    SourceCanonical = 0,
    HumanAnnotation = 1,
    HumanReviewedModelOutput = 2,
    ModelGenerated = 3,
    HeuristicGenerated = 4,
    ExternalBenchmark = 5,
    PolicyWithheld = 6,
    Extension = 255,
}

struct TrainingLabelEntryV1 {
    label_id: u64,
    label_kind: u8,
    label_authority: u8,
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

A label may be non-authoritative relative to source COVE truth while still being
the declared target label for a specific `TrainingProfileV1`.
`label_authority` states why the training profile may use the label and how it
should be audited.

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

When auditability is claimed, prompt templates, rendered prompts, source
context, tool calls, tool outputs, decoding profiles, model actor descriptors,
and generated outputs MUST be digest-bound through `AI_PAYLOAD_INTEGRITY`,
`AI_DIGEST_TABLE`, or a profile-defined digest record. A model name or endpoint
name without digest-bound prompt, context, tool-output, and decoding lineage is
not sufficient for synthetic-data auditability.

## CoveQL-AI

CoveQL-AI adds AI-native query methods over existing Cove roots, including
`table(...)`, object roots, `association(...)`, `evidence(...)`,
`projection(...)`, `chunk(...)`, `trainingSamples(...)`,
`multimodalSequences(...)`, and `assets(...)`.

CoveQL-AI is an operation profile, not a baseline query requirement. Stored
CoveQL-AI operation profiles MAY be described by `AI_SOURCE_BINDING` plus
profile-specific metadata, but query text and runtime parameters are not source
truth unless a separate profile explicitly stores them as canonical values.

Every CoveQL-AI operation context SHOULD record:

- selected source file, COVM snapshot, branch, CSN, schema fingerprint,
  dictionary digest, mapping fingerprint, visibility scope, and redaction scope;
- AI profile, vector space, chunk profile, tokenizer profile, training profile,
  sequence profile, and generator provenance records used;
- query vector source: explicit vector bytes, stored vector ref, deterministic
  vectorizer profile, or external runtime service;
- vector result authority and reproducibility class;
- exactness and approximation status for vector indexes, hybrid search,
  rerankers, dedup filters, and sampled outputs;
- fallback decisions, including ignored stale sidecars, recomputation, exact
  scan fallback, withheld metadata, and rejected operations.

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

- a reader opening `CVA2` or `CVV2` directly MUST treat payload-bearing AI
  sections as policy-blocked until `AI_SOURCE_BINDING`, visibility scope,
  redaction scope, policy scope, and sensitivity summaries are validated or a
  trusted caller policy explicitly overrides that fail-closed default;
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

Cove artifacts are immutable. If a source redaction, visibility rule,
retention rule, consent state, or policy decision changes, old AI sidecars may
remain physically present but become logically revoked for governed reads.
COVM, external catalogs, governance manifests, or policy layers SHOULD be able
to mark `CVA2`/`CVV2` artifacts and individual AI source bindings as revoked,
superseded, expired, or quarantined. A reader that sees a revoked AI sidecar
MUST NOT use it for AI operations unless an explicit trusted policy permits
historical forensic access.

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
| COVE-AI-L0 | Ignore safely | Reader recognizes optional AI profile presence, `AI_COMPANION_ARTIFACT_REF` sidecar references, and unknown optional AI sections, and ignores them safely for ordinary COVE-T/O/MAP reads. |
| COVE-AI-L1 | Metadata | Reader validates `CVA2`/`CVV2` framing and validation order when opened, `AI_SOURCE_BINDING`, `AI_REFERENCE_TABLES`, `AI_PRIVACY_SUMMARY`, COVE-MAP-AI slot policies, and exposes them in inspect/explain. |
| COVE-AI-L2 | Chunks | Reader validates COVE-CHUNK indexes and returns chunk/context projections. |
| COVE-AI-L3 | Tokens | Reader validates tokenizer profiles, token blocks, tokenized spans, and sequence packs. |
| COVE-AI-L4 | Value vectors | Reader validates COVE-VEC vector spaces, FileCode vector bindings, and vector payload blocks; supports `.embedding()` for distinct value/slot vectors. |
| COVE-AI-L5 | Semantic search | Reader supports `.similar()` with exact flat scan or validated candidate indexes, with correct temporal/visibility/redaction behavior. |
| COVE-AI-L6 | Training data | Reader validates COVE-TRAIN samples, splits, labels, loss masks, sample weights, dedup groups, generator provenance, and epoch plans. |
| COVE-AI-L7 | Multimodal | Reader validates COVE-MMSEQ interleaved multimodal sequences and asset refs. |
| COVE-AI-L8 | Full AI archive | Reader supports chunks, tokens, vectors, training samples, multimodal sequences, synthetic provenance, tensor layout, AI explain, deterministic split/epoch plans, and negative conformance vectors. |

## Negative Conformance Corpus

The conformance suite should include reject/fallback tests for:

- writer or AI-profile validator accepts an operation-only AI feature bit in
  `.cove` header `required_features` word 0 instead of scoped requiredness;
- unknown optional profile ID in an optional embedded `.cove` AI section causes
  ordinary COVE-T/O/MAP reads to reject;
- unsupported `postscript_version`, `version_major`, or incompatible
  `version_minor` is accepted without a feature-gated compatibility rule;
- unknown optional COVE-AI section reported by inspect and ignored by ordinary
  COVE-T scan;
- unknown required COVE-AI section rejecting only the selected AI operation;
- duplicate `section_id` values in one `CVA2`/`CVV2` artifact;
- section-entry array mutation changes `section_kind`, `source_binding_ref`,
  feature scope, offset, or length without changing section payload CRC;
- `CoveAiPostscriptV1.required_ai_features` or `optional_ai_features` differs
  from the corresponding `CoveAiHeaderV1` word;
- postscript `header_length` does not equal
  `header_len + section_count * section_entry_len`;
- `file_len` does not match actual artifact length, or a section range extends
  outside `file_len`;
- `AI_SECTION_FEATURE_BINDING` depends on an unvalidated feature binding;
- sidecar section-local feature bits are treated as `.cove` global feature
  bits or vice versa;
- `.cove` or COVM sidecar reference lacks digest-bound
  `AI_COMPANION_ARTIFACT_REF` metadata under a strict sidecar policy;
- embedded `.cove` `AI_COMPANION_ARTIFACT_REF` resolves refs without an embedded
  `AI_REFERENCE_TABLES` section or required resolver extension;
- malformed `CVA2` or `CVV2` postscript, header, section bounds, or checksum;
- missing `AI_SOURCE_BINDING` for a derived payload section;
- `_ref` field points outside its declared COVE-AI reference space;
- Phase 1 `_ref` field resolves through the wrong reference space but happens
  to find the same numeric ID;
- mixed-record binary section omits `AiRecordHeaderV1`;
- unknown required record kind is skipped instead of rejecting the selected
  section/profile/operation;
- `CVA2` or `CVV2` sidecar with valid CRC32C but missing cryptographic digest
  while claiming `StoredPayloadVerifiable`;
- payload integrity digest computed over stored compressed bytes but declared as
  decoded section payload bytes;
- `AiPayloadIntegrityV1.payload_crc32c` computed over the integrity record
  instead of the declared payload domain;
- digest payload needed to validate `AI_PAYLOAD_INTEGRITY` depends cyclically on
  the same integrity record;
- reference-table enum value outside the standard range is accepted without a
  required extension;
- non-zero unknown `flags` in a required record are ignored instead of rejected;
- AI compression enum uses `Zstd = 1` and `Lz4 = 2` instead of the COVE section
  compression ordering;
- payload offset interpreted as section-relative when the record declares the
  default artifact-absolute coordinate space;
- absolute vector/token payload offset points into decoded bytes of a compressed
  section instead of stored artifact bytes;
- `AiPayloadRefEntryV1` uses fields that are invalid for its `storage_kind`;
- `TokenBlockHeaderV1.payload_ref` resolves to one range, but the block
  header's non-zero cached `payload_offset` or `payload_length` describes a
  different range;
- `VectorPayloadBlockHeaderV1.payload_ref` resolves to one range, but the
  block header's non-zero cached `payload_offset` or `payload_length` describes
  a different range;
- Phase 1 vector or token payload ref uses `ArtifactAbsolute` but is not fully
  contained in exactly one `AI_PAYLOAD_BYTES` section;
- Phase 1 vector or token payload ref uses `ArtifactAbsolute` into a compressed
  `AI_PAYLOAD_BYTES` section instead of `SectionDecodedRelative`;
- `TokenBlockHeaderV1.integrity_ref` or
  `VectorPayloadBlockHeaderV1.integrity_ref` points to an
  `AiPayloadIntegrityV1` record whose `payload_ref` does not match the block
  payload ref;
- `VectorEntryV1.integrity_ref` points to an integrity record that verifies a
  different byte range than the resolved vector entry payload range;
- standard Phase 1 binary section omits `AiRecordHeaderV1`;
- raw vector or token bytes are appended to `AI_VECTOR_PAYLOAD_BLOCK` or
  `AI_TOKEN_BLOCK` and parsed as records because no `AI_PAYLOAD_BYTES` carrier
  or payload ref is declared;
- `AI_PAYLOAD_BYTES` is parsed as `AiRecordHeaderV1` records instead of treated
  as an opaque payload carrier;
- `AI_PAYLOAD_BYTES` declares `payload_encoding = BinaryRecords` instead of
  `OpaqueBytes`;
- duplicate reference IDs appear in one `AI_REFERENCE_TABLES` reference space;
- direct `.covev` open exposes payload before validating policy scope;
- direct `.covev` open exposes payload-bearing sections when
  `AI_PRIVACY_SUMMARY` is absent and no trusted caller override exists;
- stale `.covev` file digest;
- source file digest matches but redaction scope, visibility scope, policy
  scope, or sensitivity summary changed;
- wrong dictionary digest, schema fingerprint, vector dimension, metric, or
  normalization policy;
- `FileCodeVectorBindingV1.file_code` exceeds `u32::MAX` through an unsupported
  wider-code extension or non-zero reserved widening field;
- vector-space ID collision: same numeric `vector_space_id` but different
  model, template, metric, normalization, quantization, or pipeline descriptor;
- vector binding references a `vector_ref` with no matching `AI_VECTOR_DIRECTORY`
  entry;
- vector entry inherits block integrity incorrectly or declares per-vector
  integrity inconsistent with the vector payload byte range;
- template fingerprint mismatch;
- tokenizer digest mismatch;
- tokenizer profile matches vocab but not pre-tokenizer, byte encoder, added
  token ordering, chat template, Unicode version, truncation, or padding policy;
- chunk span not UTF-8 aligned;
- chunk `unicode_scalar_start` valid only under UTF-16 code-unit semantics, not
  declared Unicode scalar semantics;
- chunk source hash mismatch;
- tokenized span source hash mismatch;
- FileCode reused across files without dictionary proof;
- forbidden slot vectorized or tokenized;
- redacted slot exposed through vector metadata;
- redacted neighboring chunk included in context;
- approximate index claimed as exact;
- ANN index with false negatives used for proof exclusion;
- `.similar()` claims exact filtered top-k after post-filtering approximate
  global candidates;
- quantized vector index claims exact cosine results without declaring
  dequantization profile, quantization error profile, and score space authority;
- training split missing seed/hash policy;
- split reproducibility claimed while sample ordering depends on map/hash
  iteration order;
- evaluation split contaminated by dedup group overlap;
- dedup group used as evaluation-contamination proof while dedup authority is
  approximate, advisory, or model-scored;
- external asset URI without digest when replayability is claimed;
- asset digest binds compressed image bytes but the model input used undeclared
  EXIF orientation, resize, crop, color, frame extraction, resampling, OCR,
  caption, transcript, or model preprocessing transform;
- generated label missing generator provenance when auditability is claimed;
- model-generated label has model name but missing prompt, context, tool-output,
  or decoding digest under an auditability claim;
- teacher model name present but version/provenance missing under strict policy;
- runtime floating-point composed vector claimed as digest-reproducible;
- `ExternalAuditOnly` numerically compared as stronger than
  `CanonicalRecomputeReproducible`;
- tensor layout claims zero-copy but alignment/stride is invalid;
- multimodal sequence element ordinal duplicate;
- loss mask references invalid token range.

## Positive Conformance Corpus

Each COVE-AI profile should have at least one minimal accept fixture, one
inspect/report fixture, and one operation fixture before an implementation
claims the corresponding tier.

| Profile | Minimal accept fixture | Operation fixture |
| --- | --- | --- |
| COVE-MAP-AI | `.covemap` with `MAP_AI_PROFILE_CATALOG` and one slot policy. | Inspect slot decisions and reject a forbidden-slot sidecar. |
| COVE-AI Shared | `.cove` or COVM reference with `AI_COMPANION_ARTIFACT_REF`, one `AI_REFERENCE_TABLES` section, `AI_SOURCE_BINDING`, `AI_PRIVACY_SUMMARY`, `record_version = 1` records, and valid `CVA2`/`CVV2` validation order. | Ordinary COVE-T scan ignores optional AI sidecar; `cove inspect --ai` reports sidecar freshness. |
| COVE-CHUNK | `.coveai` with one `ChunkProfileV1` and UTF-8-valid `TextChunkEntryV1`. | `.chunks()` returns source-bound chunks with redaction applied. |
| COVE-TOK | `.coveai` with one tokenizer profile, token block descriptor, tokenized span, reference table, `AI_PAYLOAD_BYTES`, and payload integrity record. | `.tokens()` returns token IDs and validates masks/labels. |
| COVE-VEC | `.covev` with source binding, privacy summary, vector space, FileCode vector binding, payload block descriptor, vector directory, one reference table, `AI_PAYLOAD_BYTES`, payload integrity record, and `record_version = 1` records. | `.embedding()` resolves one FileCode vector and rejects wrong dictionary digest. |
| COVE-VEC index | `.covev` with exact flat index metadata. | `.similar()` labels exactness and falls back from stale approximate index. |
| COVE-TRAIN | `.coveai` with training profile, deterministic split, sample, label, and epoch plan. | `.trainingSamples().split().pack()` preserves order, masks, labels, and evidence. |
| COVE-MMSEQ | `.coveai` with asset manifest, sequence pack, ordered elements, and policy refs. | `.multimodal()` reconstructs element order and rejects duplicate ordinals. |
| Generator provenance | Synthetic label or preference pair with model actor and decoding profile. | Filter samples by generator model/version and human-review status. |

Release gates:

```sh
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```

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

Implement the COVE-AI feature namespace, proposed section-kind registry entries,
`CVA2`/`CVV2` companion artifact framing, `AI_SOURCE_BINDING`,
`AI_REFERENCE_TABLES`, `AI_PAYLOAD_INTEGRITY`, `AI_PAYLOAD_BYTES`,
COVE-MAP-AI slot policy
metadata, COVE-VEC vector space descriptors, `FileCodeVectorBindingV1`,
`VectorPayloadBlockHeaderV1`, `VectorEntryV1`,
`VectorCompositionProfileV1`, `VectorResultAuthorityV1`, `cove inspect --ai`,
`cove vec build`, and CoveQL-AI `.embedding()`.

Deliverable: distinct logical values are vectorized once per semantic slot and
reused by FileCode, with stale sidecars rejected and ordinary COVE-T scans
unaffected by unsupported AI metadata.

### Phase 2: Chunking and RAG

Implement COVE-CHUNK profiles, `TextChunkEntryV1`, `ChunkVectorBindingV1`,
CoveQL-AI `.similar()` over chunks, `.context()`, `.asPromptContext()`, and AI
explain output.

Deliverable: Cove becomes a self-verifying RAG archive with source-bound chunks
and evidence-aware context, including redaction-safe context expansion.

### Phase 3: Tokenization and Training Data

Implement `TokenizerProfileV1`, `TokenBlockHeaderV1`, `TokenizedSpanV1`,
`TokenSequencePackV1`, `TrainingProfileV1`, `TrainingSampleEntryV1`,
`DatasetSplitV1`, `cove train export`, and
`.trainingSamples().split().pack()`.

Deliverable: Cove stores reproducible tokenized training streams and sample
splits with validated masks, labels, order, and evidence.

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
> Dynamically composed floating-point vectors have
> `VectorResultAuthorityV1::RuntimeAdvisory` by default and MUST NOT be treated
> as byte-reproducible cryptographic truth. Composed vectors may become
> digest-verifiable only when materialized as payload bytes or produced under a
> strict canonical deterministic arithmetic profile.
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
