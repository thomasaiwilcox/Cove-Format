# cove-ai Python bindings

`cove-ai` opens governed COVE-AI training archives from Python while keeping
COVE as the archive of record. The base package has no hard dependency on
PyTorch, Hugging Face datasets, or WebDataset.

Install from this repository with maturin:

```bash
maturin develop
```

Install local optional integrations the same way:

```bash
maturin develop --extras hf
maturin develop --extras torch
maturin develop --extras webdataset
```

Or build a wheel:

```bash
maturin build
pip install target/wheels/cove_ai-*.whl
```

## Quickstart

```python
import cove_ai

archive = cove_ai.open("training.coveai")
print(archive.verify(policy_report=True))

for sample in archive.training_samples(split="train", include_payloads=True):
    print(sample)
    break
```

You can also open a digest-bound COVM manifest or provide an explicit AI
sidecar for a source file:

```python
archive = cove_ai.open("training.covm", dataset_dir=".")
archive = cove_ai.open("source.cove", cove_ai="training.coveai")
```

## API

- `cove_ai.open(path, cove_ai=None, dataset_dir=None)` opens `.coveai`,
  `.covev`, `.covm`, or a source path plus explicit AI sidecar.
- `archive.verify(policy_report=True)` returns artifact metadata, split counts,
  payload-access state, and withheld diagnostics.
- `archive.training_samples(split=None, include_payloads=False)` returns owned
  Python dictionaries for training samples.
- `archive.chunks(include_text=False)` returns chunk descriptors and optional
  policy-gated text payloads.
- `archive.tokens(include_payloads=False)` returns token block descriptors and
  optional token payloads.
- `archive.multimodal(include_payloads=False)` returns multimodal sequence
  elements and optional payload-backed position/evidence streams.
- `archive.export(format="jsonl", out=None, split=None, include_payloads=False)`
  writes or returns `json`, `jsonl`, `hf-jsonl`, `arrow`, `parquet`, or
  `webdataset` output.

Optional integrations are intentionally extras. In a published-package install,
they use the normal extras syntax:

- `cove-ai[hf]` enables `archive.to_hf_dataset()`.
- `cove-ai[torch]` enables `archive.to_torch_iterable()`.
- `cove-ai[webdataset]` keeps WebDataset export helpers available without making it a base dependency.

```python
hf_dataset = archive.to_hf_dataset(split="train", streaming=True)
torch_dataset = archive.to_torch_iterable(split="train", batch_size=8)
archive.export_webdataset("train.tar", split="train", include_payloads=True)
```

Payloads remain fail-closed. Python methods expose payload bytes only when the
underlying COVE-AI payload lease validates source binding, privacy summaries,
integrity, and policy state. Exceptions preserve a `COVE_AI_ERROR:` prefix so
pipeline code can route archive or policy failures separately from trainer
failures.
