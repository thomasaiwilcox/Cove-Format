# CoveQL-AI

## 83.80 CoveQL-AI Query Profile

CoveQL-AI adds AI-native query methods over existing Cove roots, including
tables, objects, associations, evidence, projections, chunks, training samples,
multimodal sequences, and assets. CoveQL-AI is an operation profile, not a
baseline query requirement.

Stored CoveQL-AI operation profiles MAY be described by `AI_SOURCE_BINDING`
plus profile-specific metadata, but query text and runtime parameters are not
source truth unless a separate profile explicitly stores them as canonical
values.

### 83.80.1 Operation Context

Every CoveQL-AI operation context SHOULD record:

- selected source file, COVM snapshot, branch, CSN, schema fingerprint,
  dictionary digest, mapping fingerprint, visibility scope, and redaction
  scope;
- AI profile, vector space, chunk profile, tokenizer profile, training profile,
  sequence profile, and generator provenance records used;
- query vector source: explicit vector bytes, stored vector ref, deterministic
  vectorizer profile, or external runtime service;
- vector result authority and reproducibility class;
- exactness and approximation status for vector indexes, hybrid search,
  rerankers, dedup filters, and sampled outputs;
- fallback decisions, including ignored stale sidecars, recomputation, exact
  scan fallback, withheld metadata, and rejected operations.

### 83.80.2 Methods

| Method | Operation |
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
| `.generatorAudit(...)` | Project generator provenance, model actor, decoding, review, label, and preference metadata. |
| `.explain(ai)` | Explain AI sidecars, freshness, exactness, approximation, fallback, redaction, and lineage. |

### 83.80.3 Execution Rules

- `.embedding()` over FileCode, direct vector refs, row/object/chunk/sample
  bindings, association-state bindings, asset bindings, or
  multimodal-sequence bindings MUST validate vector-space identity,
  dictionary/schema/path binding, source freshness, privacy summary, and
  payload integrity before returning vector bytes.
- Runtime composed embeddings are `RuntimeAdvisory` unless persisted and
  digested or computed under a canonical fixed-point arithmetic profile.
- `.similar()` MUST label exactness. It MAY use exact flat scan, or a validated
  candidate index when approximate/incomplete status is reported correctly.
  Search targets include FileCode, chunk, object-state, association-state,
  training-sample, asset, and multimodal-sequence vector bindings.
  Requested ANN descriptors that are present and supported MAY execute through
  implementation-owned graph, IVF, quantized, or disk-graph-style candidate
  generation; if they cannot prove exactness, they MUST report approximate
  authority and the selected index name even when final candidate scores are
  exact. Unsupported, missing, stale, or policy-blocked ANN descriptors MUST
  report exact-flat fallback when exact vector payloads are available and policy
  permits scanning, or reject with diagnostics when fallback is not allowed.
- `.hybrid()` and `.rerank()` are advisory unless persisted authority and
  deterministic ranking lineage are available.
- `.chunks()` and `.context()` MUST validate source-value hashes and withhold
  neighboring/parent chunks when freshness or policy validation fails. Payload
  text exposure MUST reconstruct from visible source COVE-O values through the
  shared AI access context, not from descriptor records alone.
- `.tokens()` and `.pack()` MUST validate tokenizer profile and token payload
  refs before exposing token IDs, masks, labels, or positions.
- AI payload exposure MUST use the shared COVE-AI payload lease path. A failed
  lease produces structured withheld diagnostics rather than direct byte reads.
- Implementations MAY auto-discover sibling `.coveai` or `.covev` sidecars for
  selected COVE-AI operations, including COVM dataset inputs. COVM discovery
  MAY use a digest-bound AI-sidecar reference extension that binds the sidecar
  URI to a manifest member file id and sidecar digest. Discovery MUST validate
  the `CVA2`/`CVV2` envelope before use and MUST fail closed when no valid
  sidecar is available or when freshness/policy checks fail.
- `.trainingSamples().split().pack()` MUST preserve deterministic split and
  sample ordering when declared; policy-withheld samples produce diagnostics
  rather than silent skips.
- `.multimodal()` MUST validate sequence ordinals, referenced tokens, assets,
  tensors, vectors, labels, evidence, and policy before reconstructing a
  model-consumable sequence.
- `.generatorAudit()` MUST distinguish external audit records from deterministic
  regeneration claims and MUST support filters over model namespace/name/
  version, provider, endpoint, decoding profile, human-review status, and
  reproducibility class when those descriptor families are present.

### 83.80.4 AI Explain Output

AI explain output MUST include, subject to disclosure policy:

- sidecar artifact IDs and freshness status;
- vector-space identity, metric, dimension, normalization, model lineage, and
  result authority;
- chunk, tokenizer, training, sequence, and generator profiles used;
- exactness/approximation status for vector indexes and hybrid/rerank stages;
- fallback actions, including ignored stale sidecars, recomputation, exact scan
  fallback, or operation rejection;
- redaction, forbidden-slot, withheld-metadata, and policy decisions;
- generator provenance and reproducibility class for synthetic labels, scores,
  preferences, or generated outputs.

Explain output MUST NOT reveal protected values, protected token IDs, protected
vectors, protected prompt/context text, or sensitive metadata when disclosure
policy forbids it.
