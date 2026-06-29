# COVE v2.0 Benchmark Methodology

COVE v2.0 public benchmarks use deterministic generated corpora for CI, standard, and publication profiles. The public corpus includes scan/filter workloads, conversion cost, ORC/Parquet comparison, indexes, coverage cache behavior, COVE-MAP semantics, canonicalisation vectors, negative corrupt vectors, and an offline object-store harness.

The object-store harness records object GETs, range GETs, bytes requested, bytes returned, cold/warm cache state, and coalescing decisions. It is hermetic object-store semantics, not live S3 or MinIO performance. Live object-store publication evidence can be added later without replacing this deterministic gate.

The COVE-O proof suite adds three adoption-shaped scenarios: Customer 360,
claims/events, and product/vendor catalog. Each scenario emits source tables,
a COVE-MAP file, a verified map-build bundle, COVE-T projections, COVE-I
sidecars, a COVM manifest, doctor JSON, projection parity JSON, Parquet
baselines, and a size-comparison report. Benchmark cases report build time,
validation/parity time, source bytes, source Parquet bytes, denormalized Parquet
bytes, COVE-O bytes, COVE-T bytes, COVE-I bytes, COVM bytes, total bundle bytes,
object/property/evidence counts, and duplication ratios. `cove-bench check`
fails if doctor or parity reports are not `ok` or if required proof metrics are
missing.

The overlap-scale benchmark is a synthetic maximum-overlap sweep. It generates
one, two, four, and eight source tables containing the same logical
object/property state, then measures the generated COVE-O object, complete
adoption bundle, source CSV bundle, source Parquet bundle, and unique Parquet
baseline. The benchmark is intended to show the crossover curve for repeated
multi-table state: COVE-O should improve relative to duplicate source-shaped
tables as table count rises, while the full bundle remains honest about
projection, index, manifest, report, and README overhead. It is not a claim that
COVE-O is always smaller than Parquet for low-overlap or single-table data.

The partial-overlap benchmark fixes the source-table count at eight and varies
the shared logical-entity fraction across 0%, 25%, 50%, 75%, and 100%. Rows
outside the shared fraction are source-specific entities. This isolates the
practical adoption question: how much entity overlap is needed before COVE-O's
object/property deduplication offsets semantic evidence and bundle overhead.
Current overlap result tables and interpretation are recorded in
[`../performance/cove-o-overlap-benchmark-results.md`](../performance/cove-o-overlap-benchmark-results.md).

COVE-AI benchmark reports SHOULD include vector dedup ratio, embedding cost
avoided, tokenization cost avoided, chunk reuse ratio, training stream
throughput, multimodal assembly latency, RAG retrieval latency, snapshot
verification cost, tensor zero-copy/materialization rates, storage overhead,
stale sidecar rejection, redaction leakage checks, split reproducibility, and
generator filtering correctness. The provider-free reference harness currently
generates a deterministic indexed `events-ai.covev` sidecar and reports vector
build latency, sidecar parse latency, exact vector-search latency, internal ANN
candidate-search latency, recall versus exact scan, exact-fallback rate,
filtered top-k completeness, payload bytes read, and policy-withheld counts.
It also includes an `ai-training-archive` group for the adoption workflow:
JSONL import throughput, archive verification throughput, sample streaming
throughput, export throughput, payload bytes read, policy-withheld counts,
context/vector latency placeholders where the archive contains those records,
and deterministic report schema checks. These benchmarks treat HF JSONL,
Parquet, Arrow, WebDataset, and PyTorch/Hugging Face loaders as interop/export
paths; COVE-AI remains the archive authority.
ANN benchmarks MUST report exactness, false-negative policy, quantization
score-space declarations, post-filtered top-k incompleteness, and
fallback-to-exact behavior when exact vectors are available and query policy
permits it.

Benchmark artifacts follow COVE v2.0 feature-scope rules and extension fallback policy. The conformance command set remains:

```sh
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```
