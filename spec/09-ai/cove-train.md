# COVE-TRAIN

## 83.70 COVE-TRAIN Training Corpus Profile

COVE-TRAIN makes Cove a portable archive for AI training and evaluation
corpora. It supports pretraining spans, instruction-response examples, chat
transcripts, tool traces, preference pairs, ranking lists, classification,
extraction, summarization, translation, code samples, multimodal examples,
evaluation items, safety/red-team items, generated labels, human-reviewed
labels, and synthetic teacher outputs.

COVE-TRAIN payloads live in `AI_TRAINING_PROFILE`,
`AI_TRAINING_SAMPLE_INDEX`, `AI_TRAINING_SPLIT_DEDUP_EPOCH`,
`AI_LABEL_PREFERENCE`, `AI_GENERATOR_PROVENANCE`, and, for multimodal corpora,
`AI_MULTIMODAL_SEQUENCE` sections.

### 83.70.1 Reader Obligations

- Validate source snapshot, mapping profile, chunk profile, tokenizer profile,
  vector space, split policy, sampling policy, dedup policy, quality policy,
  license policy, and redaction policy before exporting samples.
- Reject a training split that claims reproducibility but omits source
  snapshot, hash function, seed, filters, grouping, ordering, or dedup behavior
  needed to replay the split.
- Reject evaluation or holdout exports when a dedup group, source grouping, or
  benchmark exclusion rule proves contamination under the declared policy.
- Validate sample weights, quality scores, labels, loss masks, attention masks,
  preference pairs, and generator provenance before export.
- Preserve deterministic sample order when an epoch plan declares one.
- Export policy-withheld samples as rejected/withheld diagnostics, not as
  silently skipped rows, when reproducibility or auditability is claimed.
- JSON, JSONL, HF-style JSONL, Arrow IPC, Parquet, and WebDataset-style shard
  exports MUST preserve the same sample ordering and policy-withheld
  diagnostics. Arrow, Parquet, and WebDataset exports are interoperability
  artifacts; they MUST NOT become sample truth authority and MUST carry enough
  record metadata for audit back to the COVE-AI sidecar.

### 83.70.2 Profiles and Samples

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

### 83.70.3 Splits, Dedup, and Epoch Plans

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
    ExactHash = 0,
    CanonicalIdentity = 1,
    NearDuplicateHash = 2,
    EmbeddingSimilarity = 3,
    ModelScored = 4,
    HumanReviewed = 5,
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

A split or epoch plan seed is not sufficient on its own. Reproducible splits
and epoch plans MUST declare hash function, RNG algorithm, ordering policy,
filter policy, dedup policy, grouping policy, and permutation function.

Approximate or advisory dedup groups MAY guide sampling quality, but MUST NOT
be used as proof that evaluation contamination is impossible.

### 83.70.4 Labels, Preferences, and Generator Provenance

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
External model APIs may be non-reproducible; in that case provenance supports
audit and filtering but MUST NOT claim deterministic regeneration.

Prompt templates, rendered prompts, source context, tool calls, tool outputs,
decoding profiles, model actor descriptors, and generated outputs MUST be
digest-bound when auditability is claimed.

CoveQL-AI generator filters over model namespace, name, version, provider,
endpoint, decoding profile, human review status, and reproducibility class MUST
read from `AI_GENERATOR_PROVENANCE` records and MUST respect disclosure policy.
