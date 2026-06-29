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
and exact flat FileCode vector lookup/search. It does not call network
embedding, tokenizer, or model providers. Writer/test commands either consume
supplied local payloads or generate deterministic local vectors.

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
- exact flat FileCode vector scan and FileCode embedding lookup over validated
  `.covev` sidecars;
- CoveQL-AI methods over supplied sidecars:
  `.embedding()`, `.similar()`, `.chunks()`, `.tokens()`, `.context()`,
  `.asPromptContext()`, `.trainingSamples()`, `.split()`, `.pack()`,
  `.multimodal()`, `.hybrid()`, `.rerank()`, and `.generatorAudit()`.

Approximate nearest-neighbor index descriptors are parsed and validated, but
unsupported ANN payloads are treated as candidate metadata. Exact flat scan is
the supported search path in the current reference implementation.

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
  --deterministic
```

Query a COVE file with a supplied AI sidecar:

```bash
cove query --cove-ai target/vectors.covev examples/coveql/events.cove \
  '# profiles: table, ai
table(events).embedding(fileCode: 1)'

cove query --cove-ai target/vectors.covev examples/coveql/events.cove \
  '# profiles: table, ai
table(events).similar(fileCode: 1, k: 2)'
```

Export COVE-TRAIN descriptor metadata from a training sidecar:

```bash
cove train export training.coveai --format json
cove train export training.coveai --format jsonl --out samples.jsonl
```

The training export command validates the sidecar and emits descriptor
metadata. It does not bypass AI payload policy or read protected payload bytes
directly.

## CoveQL-AI

CoveQL-AI is an operation profile, not a baseline query requirement. Queries
must opt into the AI profile and provide an AI sidecar when the selected method
requires one:

```text
# profiles: table, ai
table(events).similar(fileCode: 1, k: 5)
```

Descriptor projection methods such as `.chunks()`, `.tokens()`,
`.trainingSamples()`, `.multimodal()`, and `.generatorAudit()` return validated
sidecar metadata with protected text/assets withheld unless a future
profile-specific operation validates direct exposure.

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
