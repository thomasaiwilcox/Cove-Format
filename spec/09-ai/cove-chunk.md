# COVE-CHUNK

## 83.30 COVE-CHUNK Text Chunk Profile

COVE-CHUNK stores reusable chunk metadata so AI systems do not repeatedly split
the same text into incompatible spans. It supports sentence chunks, paragraph
chunks, section chunks, heading-aware chunks, fixed token windows, sliding
token windows, semantic spans, document pages, OCR spans, transcript segments,
parent/child context navigation, source-value binding, and evidence binding.

COVE-CHUNK payloads live in `AI_CHUNK_PROFILE` and `AI_TEXT_CHUNK_INDEX`
sections. They MAY be stored in `.coveai`, embedded `.cove` sections, or
another COVE-AI-compatible sidecar.

COVE-CHUNK records are reusable span metadata. They identify source byte/token
spans, navigation relationships, evidence, policies, and hashes; they MUST NOT
duplicate source text payload. Chunk text is reconstructed from the declared
source binding only after source freshness, visibility, redaction, and policy
checks pass.

### 83.30.1 Reader Obligations

- Validate every chunk byte span against the source value before exposing text.
- Validate UTF-8 boundary alignment for text chunks.
- Reject or withhold parent, child, previous, next, and sibling navigation when
  the referenced chunk is outside policy or fails validation.
- Expose chunk text only after redaction and visibility policy checks.
- Treat chunk hierarchy and neighboring-context expansion as retrieval
  structure, not source document truth.
- Recompute chunks only when the source value, deterministic chunk profile, and
  policy are all available.

### 83.30.2 Records

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

### 83.30.3 Validation Rules

- Byte spans over UTF-8 text MUST align to valid UTF-8 boundaries.
- Byte spans are authoritative for source binding; Unicode scalar offsets are
  advisory navigation metadata over the declared normalized text form.
- `unicode_scalar_start` and `unicode_scalar_length` count Unicode scalar
  values, not UTF-16 code units or grapheme clusters.
- A chunk entry MUST bind to a source value hash so stale chunk indexes can be
  detected after rewriting, remapping, redaction, or normalization changes.
- Chunk hierarchy is advisory for retrieval, but chunk boundaries MUST validate
  against the source value before use.
- Redaction policy applies before exposing chunk text, chunk hashes, token
  alignment, neighboring context, or parent/child navigation.
- Default context expansion MUST withhold neighboring or parent chunks when
  policy or freshness validation fails.
- Runtime chunk-text exposure MUST reconstruct text from the bound source
  value, not from `TextChunkEntryV1` itself. The operation MUST validate the
  source COVE-O value is visible, byte spans are in range and UTF-8 aligned,
  Unicode scalar offsets are consistent, `source_value_hash_ref` matches the
  full source value when present, and `chunk_text_hash_ref` matches the exposed
  byte span when present.

COVE-CHUNK MAY support matched-span context expansion over matched chunks,
parents, siblings, previous/next chunks, same heading, same object, same source
evidence, and declared before/after policies.
