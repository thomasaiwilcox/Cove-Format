# COVE v2.0 Section-Kind Registry

The COVE v2.0 section-kind registry records every stable section identifier, owning profile, requiredness rules, payload encoding, checksum policy, and compatibility behavior. New section kinds must include a normative parser, validation fixture, inspect summary, dump or report behavior where applicable, and a release-gate command when the section is publication-critical.

Section identifiers are globally stable, but feature-scope rules still apply to optional semantics inside each section. Unknown optional sections are skipped through extension fallback. Unknown required sections fail validation with a structured error.

Current COVE-R embedded section assignment:

| ID | Name | Profile | Required feature |
| --- | --- | --- | --- |
| 48 | RUNTIME_COMPATIBILITY_HINTS | COVE-R | FEATURE_RUNTIME_COMPATIBILITY_HINTS |

Current COVE-MAP embedded section assignments:

| ID | Name | Profile | Required feature |
| --- | --- | --- | --- |
| 60 | MAP_SOURCE_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 61 | MAP_FUNCTION_REGISTRY | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 62 | MAP_IDENTITY_RULE_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 63 | MAP_ROW_SEMANTICS_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 64 | MAP_ASSERTION_LOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 65 | MAP_IDENTITY_EQUIVALENCE_INDEX | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 66 | MAP_EVIDENCE_INDEX | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 67 | MAP_CONVERSION_REPORT | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 68 | MAP_PROJECTION_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 69 | MAP_RESOLUTION_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |

Current COVE-AI embedded and companion section assignments:

| ID | Name | Profile | Required feature |
| --- | --- | --- | --- |
| 70 | MAP_AI_PROFILE_CATALOG | COVE-MAP-AI | Extended COVE-AI feature word 1, scoped |
| 71 | MAP_AI_TEMPLATE_CATALOG | COVE-MAP-AI | Extended COVE-AI feature word 1, scoped |
| 72 | MAP_AI_TRAINING_POLICY_CATALOG | COVE-MAP-AI | Extended COVE-AI feature word 1, scoped |
| 99 | AI_COMPANION_ARTIFACT_REF | COVE-AI | Extended COVE-AI feature word 1, scoped |
| 100 | AI_SOURCE_BINDING | COVE-AI | Extended COVE-AI feature word 1, scoped |
| 101 | AI_CHUNK_PROFILE | COVE-CHUNK | Extended COVE-AI feature word 1, scoped |
| 102 | AI_TEXT_CHUNK_INDEX | COVE-CHUNK | Extended COVE-AI feature word 1, scoped |
| 103 | AI_TOKENIZER_PROFILE | COVE-TOK | Extended COVE-AI feature word 1, scoped |
| 104 | AI_TOKEN_BLOCK | COVE-TOK | Extended COVE-AI feature word 1, scoped |
| 105 | AI_TOKENIZED_SPAN | COVE-TOK | Extended COVE-AI feature word 1, scoped |
| 106 | AI_TOKEN_SEQUENCE_PACK | COVE-TOK | Extended COVE-AI feature word 1, scoped |
| 107 | AI_VECTOR_SPACE | COVE-VEC | Extended COVE-AI feature word 1, scoped |
| 108 | AI_VECTOR_BINDING | COVE-VEC | Extended COVE-AI feature word 1, scoped |
| 109 | AI_VECTOR_PAYLOAD_BLOCK | COVE-VEC | Extended COVE-AI feature word 1, scoped |
| 110 | AI_VECTOR_COMPOSITION | COVE-VEC | Extended COVE-AI feature word 1, scoped |
| 111 | AI_VECTOR_INDEX | COVE-VEC | Extended COVE-AI feature word 1, scoped |
| 112 | AI_TENSOR_LAYOUT | COVE-VEC / COVE-MMSEQ | Extended COVE-AI feature word 1, scoped |
| 113 | AI_ASSET_MANIFEST | COVE-MMSEQ / COVE-TRAIN | Extended COVE-AI feature word 1, scoped |
| 114 | AI_MULTIMODAL_SEQUENCE | COVE-MMSEQ | Extended COVE-AI feature word 1, scoped |
| 115 | AI_TRAINING_PROFILE | COVE-TRAIN | Extended COVE-AI feature word 1, scoped |
| 116 | AI_TRAINING_SAMPLE_INDEX | COVE-TRAIN | Extended COVE-AI feature word 1, scoped |
| 117 | AI_TRAINING_SPLIT_DEDUP_EPOCH | COVE-TRAIN | Extended COVE-AI feature word 1, scoped |
| 118 | AI_LABEL_PREFERENCE | COVE-TRAIN | Extended COVE-AI feature word 1, scoped |
| 119 | AI_GENERATOR_PROVENANCE | COVE-TRAIN | Extended COVE-AI feature word 1, scoped |
| 120 | AI_REFERENCE_TABLES | COVE-AI | Extended COVE-AI feature word 1, scoped |
| 121 | AI_PAYLOAD_INTEGRITY | COVE-AI | Extended COVE-AI feature word 1, scoped |
| 122 | AI_PRIVACY_SUMMARY | COVE-AI | Extended COVE-AI feature word 1, scoped |
| 123 | AI_SECTION_FEATURE_BINDING | COVE-AI | Extended COVE-AI feature word 1, scoped |
| 124 | AI_VECTOR_DIRECTORY | COVE-VEC | Extended COVE-AI feature word 1, scoped |
| 125 | AI_PAYLOAD_BYTES | COVE-AI | Extended COVE-AI feature word 1, scoped |

COVE-AI sections MUST NOT make ordinary COVE-T/O/MAP reads fail unless a
selected profile, section, page, or operation binding makes the AI feature
required for that selected operation.

The registry is checked by the conformance suite through:

```sh
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```
