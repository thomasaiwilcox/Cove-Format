# COVE v2.0 Conformance Levels

COVE v2.0 conformance levels describe how much of the format an implementation supports:

- Level 0 validates the core envelope and rejects malformed required sections.
- Level 1 reads COVE-T scan profile files and reports optional metadata through extension fallback.
- Level 2 adds write support, conversion reports, indexes, coverage metadata, and COVE-O/COVE-MAP surfaces.
- Level 3 is publication-grade reference behavior, including release gates, benchmark manifests, and governance checks.

Feature-scope rules are part of every level. Unknown required local features fail. Unknown optional local features remain visible through inspect/report fallback.

## COVE-AI Tiers

COVE-AI conformance is optional and layered on top of the base levels:

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

Each tier requires positive and reject fixtures for the profile behavior it
claims before an implementation may advertise support for that tier.

The reference implementation exposes the provider-free COVE-AI reference
surface through release-gated fixtures: registries, scoped requiredness
integration, `.coveai`/`.covev` structural validation, common descriptor
parsing, strict `MAP_AI_*` slot-policy validation, `inspect --ai` summaries,
COVE-VEC FileCode vector sidecar writing, exact flat FileCode vector search,
FileCode embedding lookup, descriptor-bundle writing for all modeled AI
descriptor families, CoveQL-AI `.embedding(fileCode)`, `.similar(fileCode, k)`,
advisory `.hybrid(fileCode, k)` / `.rerank(fileCode, k)`, and
descriptor-backed `.chunks()`, `.tokens()`, `.context()`, `.asPromptContext()`,
`.trainingSamples()`, `.split()`, `.pack()`, `.multimodal()`, and
`.generatorAudit()` projections against supplied sidecars. Descriptor metadata
is exact; protected payload text/assets remain withheld unless a
profile-specific operation validates direct exposure.

Network embedding, tokenizer, or model providers and executable ANN index
payloads are separate extension claims, not part of the provider-free reference
surface.

Level claims do not imply production lakehouse catalog integration. The reference implementation's lakehouse evidence is fixture-backed visibility and overlay validation; Iceberg, Delta, Hudi, Hive, Unity, or other catalog adapters must be named as separate engine/table-format claims.

Conversion claims should identify fallback behavior. The reference converter emits native COVE nested layouts for supported List, Struct, and Map shapes, while unsupported nested child shapes are converted through explicit JSON fallback reports rather than claimed as native nested coverage.

Required conformance command set:

```sh
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```
