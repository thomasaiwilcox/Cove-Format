# COVE-AI Companion Sidecars

COVE-AI is an optional extension for AI-oriented metadata around COVE
archives. It does not change baseline COVE truth: ordinary COVE-T scans,
COVE-O reconstruction, and COVE-MAP readback remain valid without COVE-AI
support and must ignore unsupported optional AI metadata.

COVE-AI adds two companion artifact forms:

- `.coveai` (`CVA2`) for mixed AI descriptor bundles such as chunks, tokens,
  training samples, multimodal sequences, assets, and generator provenance.
- `.covev` (`CVV2`) for vector-heavy sidecars such as FileCode embeddings and
  vector payload blocks.

The reference implementation currently supports the provider-free COVE-AI
surface. It validates sidecar framing, descriptor records, reference tables,
payload references, privacy summaries, payload integrity, MAP-AI slot policy,
policy-gated payload leases, source-bound chunk text reconstruction, COVM
AI-sidecar references, exact flat vector lookup/search, and Cove-owned
approximate candidate generation across FileCode, chunk, object-state,
association-state, training-sample, asset, and multimodal-sequence vector
bindings. It does not call network embedding, tokenizer, ANN service, or model
providers. Writer/test commands either consume supplied local payloads or
generate deterministic local vectors.

For training-data adoption, COVE-AI is positioned as the reproducible,
policy-aware training archive that exports cleanly into the AI stack teams
already use. See [COVE-AI Training Archive](./ai-training-archive.md) for the
Python, Hugging Face, PyTorch, WebDataset, import, verify, stream, diff, and
showcase workflows.

## Authority Model

COVE-AI sidecars are metadata and derived payload carriers. They can support AI
operations such as semantic search or prompt-context assembly only after the
selected operation validates the sidecar, source binding, visibility and
privacy policy, payload integrity, and relevant descriptor families.

Baseline readers should treat COVE-AI as optional:

- A COVE-T reader can scan tables without reading `.coveai` or `.covev`.
- A COVE-O reader can reconstruct objects without AI sidecars.
- A COVE-MAP reader can read mapping metadata without turning AI policy into
  baseline schema truth.
- Direct AI payload access fails closed unless source binding, privacy summary,
  redaction, and integrity checks allow the operation.

## What Is Implemented

The current reference implementation covers:

- profile and operation registration for COVE-AI, COVE-MAP-AI, COVE-CHUNK,
  COVE-TOK, COVE-VEC, COVE-MMSEQ, COVE-TRAIN, and CoveQL-AI;
- `.coveai` and `.covev` tail discovery, postscript/header/directory
  validation, section bounds, CRC32C, version checks, feature words, and
  record headers;
- `AI_REFERENCE_TABLES`, `AI_SOURCE_BINDING`, `AI_PRIVACY_SUMMARY`,
  `AI_PAYLOAD_INTEGRITY`, `AI_SECTION_FEATURE_BINDING`, and
  `AI_PAYLOAD_BYTES`;
- COVE-MAP-AI profile, template, slot-policy, and training-policy catalogs;
- chunk, tokenizer, token block/span/sequence-pack, vector-space, vector
  payload, vector binding, vector composition, vector index, training sample,
  split, dedup, epoch, label, preference, generator provenance, tensor, asset,
  and multimodal sequence descriptors;
- COVE-VEC V2 vector bindings with `AI_FEATURE_MODEL_INPUT_IDENTITY`, where a
  `ModelInputBytes` digest proves that repeated semantic bindings used the same
  exact model input and validates vector deduplication without reading
  `AI_PAYLOAD_BYTES`;
- shared runtime access APIs for `CoveAiAccessContext`,
  `AiPayloadReader`/payload leases, vector search plans/results, export
  reports, and AI explain summaries;
- exact flat vector scan and embedding lookup over validated `.covev` sidecars
  for FileCode, direct `vectorRef`, chunk, object-state, association-state,
  training-sample, asset, and multimodal-sequence vector bindings, with
  vector-index descriptor selection, exact/fallback labels, and internal HNSW,
  IVF-flat, IVF-PQ, DiskANN-style, and Vamana-style candidate generation for
  supported approximate ANN requests;
- CoveQL-AI methods over supplied sidecars:
  `.embedding()`, `.similar()`, `.chunks()`, `.tokens()`, `.context()`,
  `.asPromptContext()`, `.trainingSamples()`, `.split()`, `.pack()`,
  `.multimodal()`, `.hybrid()`, `.rerank()`, and `.generatorAudit()`;
- training archive adoption workflows through `cove-ai-adapters`,
  `cove ai import`, `cove ai verify`, `cove ai stream`, `cove ai diff`,
  `cove showcase ai-training`, and the `cove-ai` Python package.

Approximate nearest-neighbor index descriptors are parsed, validated, and
executed by Cove-owned in-process candidate generators. Approximate ANN results
are exact-scored over the generated candidate set and reported with
`ApproximateInternalAnn` authority; they do not claim complete nearest-neighbor
exactness unless the descriptor proves it. Missing, stale, unsupported, or
policy-blocked requested indexes fall back to exact flat scan when valid
vectors exist and policy allows the query. The provider-free implementation
does not call an external ANN service.

## CLI Workflows

Validate or inspect a sidecar:

```bash
cove validate vectors.covev
cove inspect --ai vectors.covev
cove inspect --ai training.coveai
```

Build a deterministic local vector sidecar:

```bash
cove vec build \
  --out target/vectors.covev \
  --dimension 3 \
  --file-code 1 \
  --file-code 2 \
  --deterministic \
  --index hnsw \
  --metric dot
```

`cove vec build` consumes deterministic or supplied little-endian f32 source
vectors. With `--quantization none` it stores dense Float32 payloads; with
`--quantization int8`, `uint8`, or `pq` it stores local compact code bytes and
emits matching COVE-VEC element-type and quantization descriptors. The selected
metric is written into both the vector-space and vector-index descriptors.
`--seed` makes deterministic vectors reproducible, and
`--integrity-report <path>` writes a JSON summary of payload integrity,
descriptor choices, and supplied index/sharding parameters.

Query a COVE file with a supplied AI sidecar:

```bash
cove query --cove-ai target/vectors.covev examples/coveql/events.cove \
  '# profiles: table, ai
table(events).embedding(fileCode: 1)'

cove query --cove-ai target/vectors.covev examples/coveql/events.cove \
  '# profiles: table, ai
table(events).similar(fileCode: 1, k: 2)'

cove query --cove-ai target/vectors.covev \
  --explain ai \
  examples/coveql/events.cove \
  '# profiles: table, ai
table(events).similar(fileCode: 1, k: 2)'
```

When auto sidecars are enabled, `cove query` also looks for validated sibling
AI sidecars using `.covev`, `.coveai`, `-ai.covev`, `-ai.coveai`, `.ai.covev`,
and `.ai.coveai` names beside the input file or supplied dataset directory.
For COVM manifests, `cove query` first honors optional `CAI1` AI-sidecar
reference blocks in the manifest extension region. Those references bind the
sidecar URI to a source member file id plus sidecar length and digest; selected
AI operations reject stale referenced sidecars, while ordinary non-AI COVM
queries keep ignoring the optional block. Use `--no-auto-sidecars` to disable
discovery. A discovered sidecar is used only after the `CVA2`/`CVV2` envelope
parses successfully; selected AI operations still fail closed when no valid
sidecar is available.

Export COVE-TRAIN descriptor metadata from a training sidecar:

```bash
cove train export training.coveai --format json
cove train export training.coveai --include-payloads --format jsonl --out samples.jsonl
cove train export training.coveai --format arrow --out samples.arrow
cove train export training.coveai --format parquet --out samples.parquet
cove train export training.coveai --format webdataset --out samples.tar
cove ai export tokens training.coveai --include-payloads --format jsonl
cove ai export vectors vectors.covev --format parquet --out vectors.parquet
cove ai export training training.coveai --format webdataset --out training.tar
cove ai export multimodal training.coveai --format json --policy-report
```

The export commands support JSON, JSONL, HF-style JSONL, Arrow IPC, and Parquet
record streams, plus WebDataset-style tar shards containing metadata and JSON
record members. Arrow and Parquet exports use native table artifacts with
stable record columns plus the full record payload as canonical JSON. The Rust
API exposes `ai_tensor_zero_copy_view` for lifetime-checked, DLPack-style
borrowed tensor views after dtype, shape, stride, alignment, dense-layout,
payload-lease, and policy validation; the CLI still rejects direct DLPack file
output. All payload bytes are exposed only through AI payload leases. Missing
privacy summaries, revoked payload refs, protected payload refs, external
payload refs, and stale policy state produce structured withheld diagnostics
rather than best-effort disclosure.

Import and stream governed training archives:

```bash
cove ai import jsonl samples.jsonl \
  --out training.coveai \
  --schema instruction \
  --split-policy deterministic \
  --publish-covm

cove ai verify training.coveai --policy-report --json

cove ai stream training.coveai \
  --format hf-jsonl \
  --split train \
  --include-payloads \
  --out train.hf.jsonl

cove showcase ai-training --profile quick --out target/cove-ai-training --force
```

The adoption APIs live in the `cove-ai-adapters` Rust crate and the
`bindings/python` `cove-ai` package. They can open `.coveai`, `.covev`, and
digest-bound `.covm` sidecar references; verify freshness and policy state;
iterate training samples, chunks, tokens, and multimodal records; and export
JSONL, HF-JSONL, Arrow, Parquet, and WebDataset-style shards.

The detailed training archive guide is
[`docs/ai-training-archive.md`](./ai-training-archive.md). It documents import
schemas, split behavior, COVM source freshness checks, Python APIs, optional
Hugging Face/PyTorch/WebDataset integrations, trainer examples, and benchmark
report fields.

## CoveQL-AI

CoveQL-AI is an operation profile, not a baseline query requirement. Queries
must opt into the AI profile and provide an AI sidecar when the selected method
requires one:

```text
# profiles: table, ai
table(events).similar(fileCode: 1, k: 5)
```

Projection methods such as `.tokens()`, `.trainingSamples()`,
`.multimodal()`, and `.generatorAudit()` return validated sidecar records and
lease-backed payload fields when the selected operation is allowed to expose
payloads. `.generatorAudit()` supports filters over model namespace/name/
version, provider, endpoint, decoding profile, human-review status, and
reproducibility class. `.chunks()` and RAG context validate chunk spans,
source-value hashes, chunk-text hashes, UTF-8 boundaries, and source COVE-O
redaction state before reconstructing text from the source value. Parent,
sibling, and neighboring chunk expansion remains withheld unless each expansion
path has separately valid freshness and policy proof.

COVE-CHUNK remains span-only: chunk records store source spans, hashes,
navigation, evidence, and policy refs, not a second copy of chunk text.

Use `cove query --explain ai` to return structured AI explain output for the
selected operation, including sidecar freshness, vector-space identity, selected
index, exactness or approximation, redaction decisions, fallback decisions, and
withheld metadata.

## Conformance

COVE-AI coverage is represented in the generated capability matrix and corpus.
The corpus includes `.coveai` and `.covev` accept/reject fixtures, while the
more granular structural, policy, vector, token, chunk, training, generator,
tensor, and multimodal reject cases are covered by Rust tests.

Run the relevant gates with:

```bash
cargo test -p cove-core artifact::coveai::tests
cargo test -p cove-core profile::cove_map::tests::map_ai
cargo test -p coveql coveql_ai
cargo test -p cove-cli --test smoke
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```

Normative details live in [`spec/09-ai/`](../spec/09-ai/). The design proposal
in [`docs/proposals/cove-ai-major-amendment.md`](./proposals/cove-ai-major-amendment.md)
is retained as background and rationale, not as the user-facing guide.
