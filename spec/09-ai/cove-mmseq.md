# COVE-MMSEQ

## 83.60 COVE-MMSEQ Multimodal Sequence Profile

COVE-MMSEQ preserves the order and semantics of model-consumable multimodal
sequences such as system text, user text, image, assistant text, audio clip,
tool call, tool result, assistant text, and label.

COVE-MMSEQ payloads live in `AI_MULTIMODAL_SEQUENCE`, `AI_ASSET_MANIFEST`,
`AI_TOKEN_SEQUENCE_PACK`, `AI_TENSOR_LAYOUT`, and, when needed,
`AI_GENERATOR_PROVENANCE` sections. The sequence pack is the model-consumable
ordering surface; referenced text, tokens, assets, tensors, labels, vectors,
and evidence remain separately validated surfaces.

### 83.60.1 Reader Obligations

- Validate that element ordinals are unique and form the declared sequence
  order.
- Validate every referenced token span, asset, tensor, vector, label, evidence,
  generator provenance record, and policy record before exposing an assembled
  sequence.
- Reject sequence reconstruction when an element is policy-blocked, stale,
  missing, or unsupported and no declared fallback exists.
- Apply loss masks, attention masks, labels, role markers, and position maps to
  unambiguous element or token ranges.
- Digest-bind external assets when replayability, auditability, or training
  reproducibility is claimed.
- Allow non-MMSEQ-aware readers to expose validated independent components, but
  not to claim model-consumable sequence reconstruction.

### 83.60.2 Records

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
sequence elements or token ranges unambiguously.

### 83.60.3 Reconstruction Policy

A reader MUST NOT expose a model-consumable sequence if any required element is
missing, stale, policy-blocked, or unsupported. A reader MAY expose independent
validated components, with diagnostics, when sequence reconstruction is
withheld. Policy-scoped reconstruction MUST apply redaction before prompt or
training material is emitted.
