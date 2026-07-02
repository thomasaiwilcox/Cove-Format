# Crate Ownership

This table names the intended owner surface for each workspace crate. If a
change does not fit the stated purpose, add or use a typed API in the owning
crate instead of reaching across layers.

| Crate | Purpose | Primary audience |
| --- | --- | --- |
| `cove` | Canonical app facade for validation, inspection, conversion, query, explain, and engine registration. | App developers |
| `cove-reader` | Narrow read and validation facade. | Reader users |
| `cove-writer` | Narrow writer facade. | Writer users |
| `cove-engine` | Runtime and engine integration facade. | Engine integrators |
| `cove-core` | Wire format, validation, profiles, and core data model. | Format implementers |
| `coveql` | CoveQL parse, resolve, plan, execute, explain, and query contracts. | Query implementers |
| `cove-datafusion` | DataFusion adapter, registration, decoding, and pushdown. | Engine integrators |
| `cove-arrow` | Arrow and Parquet interop. | Interop maintainers |
| `cove-convert` | Shared source-to-COVE conversion facade. | Import/export users |
| `cove-convert-parquet` | CLI-facing conversion command compatibility. | CLI maintainers |
| `cove-map` | COVE-MAP build, replay, review, projection, and mapping APIs. | Mapping implementers |
| `cove-index` | COVI index build, parse, and execution helpers. | Acceleration implementers |
| `cove-layout` | Layout plans, scan splits, and zero-copy map metadata. | Runtime implementers |
| `cove-runtime` | Runtime compatibility hints and capability registries. | Runtime implementers |
| `cove-coverage` | Coverage proofs, coverage plans, and provider metadata. | Pruning implementers |
| `cove-cache` | Coverage cache diagnostics and metadata. | Runtime implementers |
| `cove-profile-validation` | Optional profile validation helpers. | Validator maintainers |
| `cove-ai-adapters` | COVE-AI import/export/archive adapters. | AI workflow integrators |
| `cove-harbor` | Harbor integration surface. | Harbor integrators |
| `cove-validate` | Validation command implementation and JSON reporting. | CLI/tool maintainers |
| `cove-inspect` | Detailed inspection command behavior. | CLI/tool maintainers |
| `cove-dump` | Low-level dump command behavior. | CLI/tool maintainers |
| `cove-cli` | User-facing terminal UX and command routing. | CLI maintainers |
| `cove-conformance` | Corpus runner and generated capability evidence. | Standards maintainers |
| `cove-fuzz` | Deterministic fuzz and mutation harness. | Assurance maintainers |
| `cove-bench` | Benchmark corpora and reports. | Performance maintainers |
| `cove-codec` | Registered codec descriptors and validation utility. | Codec implementers |

