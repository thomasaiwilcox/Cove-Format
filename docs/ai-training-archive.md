# COVE-AI Training Archive

COVE-AI can be used as a reproducible, policy-aware archive of record for SFT,
RAG, preference, pretraining, and multimodal training datasets. The intended
workflow is simple: keep COVE-AI as the governed source of authority, then
stream or export into the AI stack you already use.

Interop formats such as Hugging Face JSONL, Arrow, Parquet, WebDataset, and
PyTorch iterables are export targets. They are not the authority for freshness,
splits, generator provenance, withheld payload diagnostics, source binding, or
policy decisions.

## What This Adds

The adoption layer adds:

- `cove-ai-adapters`, a Rust crate for opening, verifying, importing,
  streaming, exporting, and diffing COVE-AI training archives;
- the `cove-ai` Python package with import name `cove_ai`;
- CLI workflows for `cove ai import`, `cove ai verify`, `cove ai stream`,
  `cove ai diff`, and `cove showcase ai-training`;
- trainer examples for Hugging Face datasets, PyTorch iterables, TRL, Axolotl,
  and WebDataset-style multimodal export;
- benchmark coverage for import, verify, stream, export, policy-withheld
  counts, payload bytes read, and AI context/vector latency report fields.

Baseline COVE behavior is unchanged. Ordinary COVE-T/O/MAP reads ignore AI
sidecars. AI sidecars are required only when an operation explicitly asks for
AI payloads, vectors, chunks, tokens, training samples, or multimodal records.

## Import Schemas

The current import layer supports these schema modes:

| Schema | Required fields | Optional fields | Target payload |
| --- | --- | --- | --- |
| `instruction` | `instruction`, `output` | `input`, `generator`, `policy`, `labels`, `source_refs` | `{ "output": ... }` |
| `chat` | `messages[]` | `generator`, `policy`, `labels`, `source_refs` | last assistant message |
| `pretrain` | `text` | `generator`, `policy`, `labels`, `source_refs` | `null` |
| `preference` | `prompt`, `chosen`, `rejected` | `generator`, `policy`, `labels`, `source_refs` | chosen/rejected pair |
| `rag` | `query`, `context[]`, `answer` | `source_refs`, `generator`, `policy`, `labels` | `{ "answer": ... }` |

Every row may provide `sample_id` or `id`. If neither exists, import assigns a
stable row-position id such as `sample-000000000000`. Duplicate sample ids are
rejected.

For chat imports, supported roles are `system`, `user`, `assistant`, and
`tool`. Malformed rows are imported with diagnostics so the archive can be
audited, but required structural failures such as duplicate ids still fail the
import.

## Splits

`--split-policy deterministic` is the only split policy in this adoption layer.
Without a split column, COVE hashes the sample id or canonical sample JSON into
default ratios:

- `train`: 98 percent
- `validation`: 1 percent
- `test`: 1 percent

Use `--split-column <name>` when the source already carries split labels. The
accepted labels are `train`, `validation`, `valid`, `val`, and `test`.

## CLI Workflows

Dry-run an import to check counts, splits, and diagnostics without writing an
archive:

```bash
cove ai import jsonl samples.jsonl \
  --schema instruction \
  --dry-run
```

Create a governed training archive:

```bash
cove ai import jsonl samples.jsonl \
  --out training.coveai \
  --schema instruction \
  --split-policy deterministic
```

Import from supported local sources:

```bash
cove ai import jsonl samples.jsonl --out training.coveai --schema instruction
cove ai import parquet samples.parquet --out training.coveai --schema chat
cove ai import hf local-hf-dataset-dir --out training.coveai --schema pretrain
```

`hf` import reads local `.jsonl` files from the supplied directory. It does not
download datasets.

Publish a digest-bound COVM manifest beside the sidecar:

```bash
cove ai import jsonl samples.jsonl \
  --out training.coveai \
  --schema instruction \
  --publish-covm
```

Verify before training:

```bash
cove ai verify training.coveai --policy-report --json
cove ai verify training.covm --dataset . --policy-report
```

Stream trainer-friendly records:

```bash
cove ai stream training.coveai \
  --format hf-jsonl \
  --split train \
  --include-payloads \
  --out train.hf.jsonl

cove ai stream training.coveai \
  --format webdataset \
  --split train \
  --include-payloads \
  --out train.tar
```

`jsonl` and `hf-jsonl` may stream to stdout when `--out` is omitted. `arrow`,
`parquet`, and `webdataset` require `--out`.

Diff archive revisions by stable sample key:

```bash
cove ai diff old.coveai new.coveai \
  --keys sample_id \
  --report training-diff.json
```

Payload-aware exports are available through both the adoption stream command
and the descriptor-family export command:

```bash
cove ai export training training.coveai \
  --format parquet \
  --include-payloads \
  --out training.parquet

cove ai export chunks training.coveai --format jsonl --policy-report
cove ai export tokens training.coveai --format jsonl --include-payloads
cove ai export multimodal training.coveai --format webdataset --out mm.tar
```

## Policy And Diagnostics

Payload exposure is fail-closed. Readers must obtain a validated AI payload
lease before text, token bytes, vectors, tensors, assets, or training payloads
are returned.

Common diagnostic codes include:

- `COVE_AI_IMPORT_MISSING_PAYLOAD_FIELD`: a schema-required field was absent;
- `COVE_AI_IMPORT_MALFORMED_CHAT`: chat import did not contain `messages[]`;
- `COVE_AI_IMPORT_MALFORMED_CHAT_ROLE`: chat role was not in the supported set;
- `COVE_AI_IMPORT_MISSING_CHAT_TARGET`: chat row had no assistant response;
- `COVE_AI_IMPORT_MISSING_RAG_CONTEXT`: RAG row did not contain `context[]`;
- `COVE_AI_IMPORT_POLICY_WITHHELD`: source row declared
  `policy.payload_permission=false`;
- `COVE_AI_PAYLOAD_POLICY_BLOCKED`: the sidecar does not allow payload access;
- `COVE_AI_PAYLOAD_DISCLOSURE_CHECK_FAILED`: a payload ref failed disclosure
  validation during verify.

Policy-withheld payloads are reported as diagnostic rows. They are not silently
dropped from split counts, diff reports, or verification reports.

When a COVM manifest is used, `cove ai verify` resolves the digest-bound AI
sidecar reference, validates the referenced sidecar length and digest, validates
the bound source member length and digest, and rejects stale references for
selected AI operations.

## Python Quickstart

Install the Python package from the repository with maturin:

```bash
cd bindings/python
maturin develop
```

Open, verify, and stream records:

```python
import cove_ai

archive = cove_ai.open("training.coveai")
print(archive.verify(policy_report=True))

for sample in archive.training_samples(split="train", include_payloads=True):
    print(sample)
    break
```

Open a COVM manifest or explicit sidecar:

```python
archive = cove_ai.open("training.covm", dataset_dir=".")
archive = cove_ai.open("source.cove", cove_ai="training.coveai")
```

Optional integrations are extras, so the base wheel does not depend on PyTorch,
Hugging Face, or WebDataset:

```bash
pip install "cove-ai[hf]"
pip install "cove-ai[torch]"
pip install "cove-ai[webdataset]"
```

```python
hf_dataset = archive.to_hf_dataset(split="train", streaming=True)
torch_dataset = archive.to_torch_iterable(split="train", batch_size=8)
archive.export(format="webdataset", out="train.tar", split="train", include_payloads=True)
```

The Python API returns owned Python dictionaries by default. Exceptions preserve
a stable `COVE_AI_ERROR:` prefix so data pipelines can separate archive
validation and policy failures from trainer failures.

### Python API Reference

| API | Purpose |
| --- | --- |
| `cove_ai.open(path, cove_ai=None, dataset_dir=None)` | Open a `.coveai`, `.covev`, source path plus explicit sidecar, or digest-bound `.covm`. |
| `archive.verify(policy_report=True)` | Return archive metadata, split counts, payload-access state, and diagnostics. |
| `archive.training_samples(split=None, include_payloads=False)` | Return training sample records, optionally filtered by split and payload-leased. |
| `archive.chunks(include_text=False)` | Return chunk descriptors; text is withheld unless explicitly requested and policy allows it. |
| `archive.tokens(include_payloads=False)` | Return token block descriptors and optional token payloads. |
| `archive.multimodal(include_payloads=False)` | Return multimodal sequence elements and optional payload-backed position/evidence streams. |
| `archive.to_hf_dataset(split=None, streaming=True)` | Build a Hugging Face dataset when `datasets` is installed. |
| `archive.to_torch_iterable(split=None, batch_size=None)` | Build a PyTorch `IterableDataset` when `torch` is installed. |
| `archive.export(format="jsonl", out=None, split=None, include_payloads=False)` | Export JSON, JSONL, HF-JSONL, Arrow, Parquet, or WebDataset-style output. |

## Showcase

Generate the deterministic flagship demo:

```bash
cove showcase ai-training \
  --profile quick \
  --out target/cove-ai-training \
  --force
```

The showcase writes:

- `training-source.jsonl`: deterministic interop source rows;
- `training.coveai`: authoritative COVE-AI training archive;
- `training.covm`: digest-bound manifest with an AI sidecar reference;
- `verification-report.json`: freshness and policy report;
- `training.hf.jsonl`, `training.parquet`, `training.tar`: export targets;
- `load_archive.py`: minimal Python loader.

Profiles:

- `quick`: tiny deterministic smoke/demo profile;
- `standard`: larger local demonstration profile;
- `publication`: largest deterministic public-report profile.

Trainer examples live in
[`examples/ai-training/`](../examples/ai-training/). They verify the archive,
surface withheld/policy diagnostics, and then hand records to Hugging Face,
PyTorch, TRL, Axolotl, or WebDataset-style workflows without requiring network
access by default.

## Rust Adapter APIs

Rust import/export/streaming logic lives in `cove-ai-adapters`, not only in the
CLI. The crate exposes stable option/report structs for opening archives,
verification, sample iteration, import, export, and withheld diagnostics:

- `AiArchiveOpenOptions`
- `AiVerifyOptions`
- `AiSampleIteratorOptions`
- `AiExportOptions`
- `AiImportOptions`
- `AiArchiveReport`
- `AiWithheldDiagnostic`

Wire parsing and policy-gated payload leases remain in `cove-core`; CLI
orchestration lives in `cove-cli`; Python calls into the adapter crate through
the `cove_ai._native` extension.

## Benchmarks

`cove-bench check` includes an `ai_training_archive_report` case. The report
emits:

- `ai_import_samples_per_sec`
- `ai_verify_samples_per_sec`
- `ai_stream_samples_per_sec`
- `ai_export_samples_per_sec`
- `ai_payload_bytes_read`
- `ai_policy_withheld_count`
- `ai_context_latency_ms`
- `ai_vector_search_latency_ms`
- `ai_export_format`

These are deterministic reference benchmark fields, not universal performance
claims across hardware or external trainer stacks.
