# COVE-AI Training Examples

These examples target a deterministic showcase archive generated with:

```bash
cove showcase ai-training --profile quick --out target/cove-ai-training --force
```

They do not download models or start training by default. The examples verify
the archive, stream records, and show where existing trainer stacks plug in
after COVE-AI has acted as the governed archive of record.

## Scripts

- `hf_sft.py`: verifies the archive, converts the train split to a Hugging Face
  dataset when `datasets` is installed, and prints a sample record.
- `torch_iterable.py`: verifies the archive and exposes the train split as a
  PyTorch `IterableDataset` when `torch` is installed.
- `trl_sft.py`: shows the TRL SFT handoff point without starting network model
  training by default.
- `axolotl.yml`: minimal Axolotl-style config showing COVE-AI generated
  HF-JSONL as the trainer input.
- `vlm_webdataset.py`: exports a WebDataset-style tar shard for multimodal or
  VLM-style data loading.

## Run

From the repository root:

```bash
cargo run -p cove-cli -- showcase ai-training \
  --profile quick \
  --out target/cove-ai-training \
  --force

cd bindings/python
maturin develop
cd ../..
python examples/ai-training/hf_sft.py target/cove-ai-training/training.coveai
```

Install optional extras only for the integrations you want to exercise. From
`bindings/python`, local development installs can use:

```bash
maturin develop --extras hf
maturin develop --extras torch
maturin develop --extras webdataset
```

Each script surfaces verification and withheld-policy diagnostics before records
are passed to trainer-facing adapters.
