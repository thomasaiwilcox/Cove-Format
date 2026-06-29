# COVE-TOK

## 83.40 COVE-TOK Token Cache Profile

COVE-TOK stores tokenization outputs so training and retrieval pipelines do not
repeatedly tokenize the same source text. COVE-CHUNK is model-independent text
segmentation; COVE-TOK is tokenizer/model-specific tokenization and sequence
packing.

COVE-TOK metadata lives in `AI_TOKENIZER_PROFILE`, `AI_TOKEN_BLOCK`,
`AI_TOKENIZED_SPAN`, and `AI_TOKEN_SEQUENCE_PACK` sections. Persisted token
bytes live in `AI_PAYLOAD_BYTES` and are referenced through
`AI_PAYLOAD_REF_TABLE`. Token IDs are derived data and may leak source text, so
token payloads are policy-protected even when source text is not exposed.

### 83.40.1 Reader Obligations

- Validate tokenizer namespace, name, version, vocabulary digest, merges
  digest, pre-tokenizer digest, normalizer digest, byte encoder/decoder digest,
  added-token digest, special-token digest, chat template, Unicode version,
  truncation/padding policy, token ID width, and reversibility flags before
  reusing a token block.
- Support only declared token widths `1`, `2`, `4`, and `8` bytes unless a
  required extension defines another width.
- Bounds-check every token offset, token count, byte-alignment ref, mask ref,
  label ref, and position-id ref before export.
- Reject loss masks, attention masks, labels, or position IDs that are shorter
  than their scoped token range or reference tokens outside the sequence pack.
- Retokenize only when tokenizer material is available, deterministic, policy
  permits it, and the operation does not require stored token bytes.
- Never reuse token IDs across tokenizer profiles unless a compatibility
  descriptor explicitly proves equivalence.

### 83.40.2 Records

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

### 83.40.3 Payload and Integrity Rules

`TokenBlockHeaderV1.payload_ref` MUST resolve to an `AiPayloadRefEntryV1`.
For the provider-free reference surface, that payload ref MUST point into an
`AI_PAYLOAD_BYTES` section. Token bytes MUST NOT be appended to
`AI_TOKEN_BLOCK` after descriptor records.

A token block that claims `StoredPayloadVerifiable`, replayability,
auditability, or trust-chain participation MUST set `integrity_ref` to a
validated `AiPayloadIntegrityV1` record. Loss masks and labels MUST be
unambiguously scoped to token positions.
