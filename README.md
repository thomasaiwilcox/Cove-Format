# Cove Format

COVE is an experimental immutable archive format for datasets that need to stay
queryable after the original pipeline, catalog, or application context has
gone away. It explores a narrow question: what should an offline data artifact
carry so a future reader can validate it, understand its schema and provenance,
respect redaction/visibility rules, and skip work only when the metadata proves
the answer is unchanged?

The baseline `.cove` file is engine-neutral. A reader can validate and decode
logical values without depending on DataFusion, Arrow IPC, Harbor, a lakehouse
catalog, or an object store. Optional acceleration can help engines read the
same truth more cheaply, but it must never change logical results.

COVE has two primary user-facing surfaces:

1. **COVE-T: tabular archives.** For ordinary table-shaped data that should
   stay table-shaped. COVE-T stores immutable tables with dictionaries,
   encodings, checksums, morsels, zone statistics, and metadata that can prove
   when data may be skipped safely.
2. **COVE-O / COVE-MAP: mapped archives.** For fragmented source tables where
   the archive should also preserve deterministic mappings, provenance,
   evidence, and projected table readback. In plain terms: source rows remain
   auditable, while repeated business entities such as customers, products, or
   accounts can be represented once as declared objects and associations.

Everything else in the COVE suite supports those two surfaces: CoveQL, Arrow
and DataFusion access, validation tooling, conformance tests, benchmarks,
sidecars, coverage proofs, layout plans, caches, indexes, and runtime planning
hints. The rule is simple: authoritative data and validated metadata define
truth.

The current reference CLI also covers two newer workflows around those
surfaces:

- **Delta snapshots:** immutable `.covedelta` artifacts are selected through a
  COVM dataset manifest, validated as an ordered base-plus-delta chain, and
  exposed through `cove delta`, `cove query`, `cove export arrow`, and
  snapshot-bound sidecar commands.
- **Deterministic entity resolution:** COVE-MAP can use digest-pinned resolver
  catalogs, curated aliases, reviewed decisions, candidate evidence, replay
  verification, and resolver-aware projections without allowing silent fuzzy
  auto-merge.
- **AI companion sidecars:** optional `.coveai` (`CVA2`) and `.covev`
  (`CVV2`) artifacts can carry chunk, token, vector, training, multimodal, and
  generator-provenance metadata for selected AI operations. Baseline COVE reads
  do not depend on those sidecars.

## Current Status

This repository is a working standards-suite prototype, not a production data
standard with external adoption.

- The root workspace is the active COVE standards-suite workspace. Its
  normative baseline is the split specification tree under [`spec/`](./spec/),
  with [`spec.md`](./spec.md) retained as the stable entrypoint.
- The reference implementation is Rust and includes readers, writers,
  validators, conversion tools, DataFusion access, CoveQL, mapping, sidecar
  tooling, benchmark harnesses, and conformance generators.
- The current implementation is staged and evidence-tracked rather than
  described by a single blanket compliance claim. The generated matrix in
  [`conformance/capability_matrix.md`](./conformance/capability_matrix.md)
  is the source of truth for which areas are modeled, parsed, validated,
  written, and exercised by corpus fixtures.
- Historical versioned workspaces have been removed. The repository now
  exposes one active COVE standards-suite workspace at the root.

The reference code is AI-assisted. Compatibility claims are based on executable
evidence, not generated prose: CI runs formatting, clippy with warnings denied,
locked dependency checks, cross-platform tests, documentation builds with
warnings denied, release gates, conformance corpus checks, capability-matrix
regeneration checks, and fuzz smoke tests.

## Why Not Parquet, Iceberg, Delta, Or Vortex?

COVE is not trying to replace Parquet as the default columnar file format, and
it is not a table format like Iceberg or Delta. Those projects are mature and
should remain the default answer for most lakehouse workloads.

COVE is aimed at a different experiment: immutable archive artifacts that carry
more of their read contract with them.

| System | What it is best at | How COVE differs |
| --- | --- | --- |
| Parquet / ORC | Mature columnar storage for analytics engines | COVE focuses on self-contained validation, proof-scoped metadata, archive semantics, and optional deterministic mapping/provenance. |
| Iceberg / Delta / Hudi | Mutable lakehouse table metadata, snapshots, deletes, and catalog integration | COVE files are immutable artifacts; sidecars can accelerate reads but do not define table transactions. |
| Avro / Arrow IPC | Row/message interchange or in-memory/IPC columnar exchange | COVE is a durable archive format with validation, conformance, and query-planning metadata. |
| Vortex and newer analytic formats | High-performance compressed analytics layouts | COVE is not only a physical encoding experiment; it also explores portable proof semantics, provenance, and mapped archive readback. |
| Semantic layers and catalogs | Centralized meaning, governance, and metric definitions | COVE-MAP stores deterministic mapping evidence inside portable artifacts; it is not a catalog service or AI schema matcher. |

If all you need is fast analytics over active lakehouse data, use the mature
formats first. COVE is for cases where the archive itself should remain
validatable, explainable, and queryable when the surrounding system is gone.

## Tiny Example

From the repository root, a skeptical first pass should be: generate a small
sample, validate it, inspect what query surfaces exist, run a query, and
inspect which acceleration is available.

```bash
cargo run -p cove-cli -- examples
cargo run -p cove-cli -- doctor examples/coveql/people.cove
cargo run -p cove-cli -- inspect --queries examples/coveql/people.cove
cargo run -p cove-cli -- query examples/coveql/people.cove \
  'table(people).select(score, status, nickname).take(5)'
cargo run -p cove-cli -- optimize examples/coveql/events.cove
cargo run -p cove-cli -- query examples/coveql/events.cove --perf-report \
  'table(events).where(score >= 20).select(id, score)'
```

## What Works Today

The standards suite is easiest to approach in layers rather than as one
mandatory feature pile:

- **Data surfaces:** COVE-Core, COVE-T, COVE-O, and COVE-MAP define the data,
  meaning, provenance, and deterministic projections.
- **Query and access:** the CLI, CoveQL, Arrow export, and DataFusion
  integration are how users and engines touch COVE archives.
- **Delta snapshots:** `.covedelta` artifacts, COVM delta-chain manifests,
  reconstruction, compaction, checkpointing, source-publish pruning, and
  delta-aware query/export paths support incremental COVE-O publication without
  mutating existing `.cove` files.
- **Resolver-backed mapping:** COVE-MAP resolver catalogs, alias import,
  candidate generation, reviewed equivalences, redacted evidence, and replay
  checks support deterministic entity-resolution workflows.
- **AI companion metadata:** COVE-AI validates optional `.coveai` and `.covev`
  sidecars for provider-free AI workflows such as FileCode embeddings, exact
  flat semantic search, chunk/token/training metadata, multimodal sequence
  descriptors, and generator audit records.
- **Acceleration and planning:** COVE-COVERAGE, COVE-I, COVX, COVE-L, COVE-E,
  COVE-CACHE, COVE-R, range planning, and zero-copy maps help readers skip work
  safely when their contracts prove equivalence.
- **Assurance:** validation, conformance fixtures, capability matrices, fuzzing,
  benchmark harnesses, and release gates prove the implementation is grounded
  rather than aspirational.

The format is shaped around query planning:

- table scans over COVE-T segments and morsels;
- file-local dictionaries and encoded arrays;
- zone statistics, exact sets, blooms, lookup indexes, aggregate synopses, and
  composite indexes;
- optional manifests, sidecars, secondary indexes, and layout metadata;
- COVM-selected base-plus-delta object snapshots;
- object, association, evidence, and projection metadata for semantic archives.

The important distinction is authority. COVE metadata can be used as a proof
only when the spec defines the proof semantics and the reader validates the
metadata under the relevant logical type, collation, null semantics, feature
scope, and snapshot. Advisory metadata can help planning, but it must not change
query results.

## Current Limitations

- COVE is experimental and has no independent ecosystem adoption yet.
- The repository intentionally contains both normative/spec work and reference
  implementation code, so some surfaces are standards-suite scaffolding rather
  than independently deep libraries.
- Some crates are thin stable facades over larger implementation crates.
- Performance numbers are local deterministic benchmark results, not universal
  file-format claims.
- Some conversion/export paths are reference-grade and report unsupported
  features rather than pretending to be complete.
- COVE-MAP entity resolution is deterministic, digest-pinned resolver and
  review machinery; it is not probabilistic auto-merge, ETL orchestration, or
  AI schema matching.

## CoveQL Companion Query Layer

CoveQL is a substantial but optional addition to the COVE ecosystem. The core
format remains useful without it: baseline readers can validate files, decode
values, scan COVE-T segments, export Arrow arrays, and use proof-safe metadata
without implementing a query language. CoveQL is for the next layer up: engines
and tools that want a portable way to ask semantic questions over mapped COVE
data.

The current `coveql` crate implements CoveQL/Core plus CoveQL/Object, with
profile contracts for Object, Table, and Graph. It can parse and resolve
queries over objects, associations, evidence, projections, projection-backed
tables, graph nodes/edges/paths, temporal cuts, history, changes, grouping,
aggregates, ordering, pagination, and explain modes. It also exposes builder
APIs, Arrow output, manifest-aware planning, and DataFusion table-provider
integration.

CoveQL follows the same authority model as the file format. Materialized
readback is the semantic oracle. Coded execution, projection pushdown,
DataFusion pushdown, graph traversal, index-only answers, and multi-file
bridges are allowed only when their contracts prove the result is equivalent;
otherwise they must fall back or reject with structured diagnostics and explain
output.

That makes CoveQL useful for higher-level workflows without making it mandatory
for basic interoperability:

- applications can use COVE as a compact immutable archive and ignore CoveQL;
- SQL engines can register COVE files directly through the DataFusion adapter;
- semantic applications can use CoveQL for object, association, evidence, and
  projection-aware reads;
- implementers can adopt individual CoveQL profiles as their COVE-O and
  COVE-MAP support matures.

Key paths:

- [`crates/coveql`](./crates/coveql): parser, resolver, planner,
  materialized/coded execution, explain output, Arrow output, and DataFusion
  integration.
- [`crates/cove-cli`](./crates/cove-cli): beginner-friendly `cove`
  terminal entry point for inspecting artifacts, discovering query surfaces, and
  running CoveQL without writing Rust, including relational table methods,
  graph methods, explain output, physical/kernel execution modes, and sidecar
  inputs.
- [`docs/coveql-quickstart.md`](./docs/coveql-quickstart.md): start here
  to inspect sample COVE files, run table/object/projection/evidence queries,
  export JSONL/CSV, work with resolver-backed mapping and delta snapshots, and
  read explain output.
- [`docs/customer360-showcase.md`](./docs/customer360-showcase.md): the
  data-science-oriented Customer 360 walkthrough for generating messy
  multi-source CRM/support/billing/event data, querying canonical customers and
  provenance with CoveQL, comparing optimized execution, and loading generated
  outputs from Python.
- [`examples/coveql`](./examples/coveql): tiny checked-in COVE-O,
  COVE-T, source, and COVE-MAP samples for the quickstart.
- [`examples/customer360`](./examples/customer360): checked-in quick
  Customer 360 sample generated by `cove showcase customer360`.
- [`docs/proposals/coveql-object-query-language.md`](./docs/proposals/coveql-object-query-language.md):
  CoveQL/Object proposal and conformance decisions.
- [`docs/proposals/coveql-query-profiles.md`](./docs/proposals/coveql-query-profiles.md):
  CoveQL-Core, Object, Table, and Graph profile contract RFC.
- [`docs/proposals/cove-o-delta-artifacts.md`](./docs/proposals/cove-o-delta-artifacts.md):
  design context for immutable COVE-O delta artifacts and COVM chain
  selection.
- [`docs/proposals/covemap-entity-resolution.md`](./docs/proposals/covemap-entity-resolution.md):
  resolver catalog and deterministic entity-resolution design context.
- [`docs/covemap-json-schema-v1.md`](./docs/covemap-json-schema-v1.md):
  reference companion schema for COVE-MAP JSON payloads, including resolver
  catalog and projection fields.

Try the beginner CLI from the repository root:

```bash
cove examples
cove showcase customer360 --profile quick --out examples/customer360 --force
cove inspect --queries --performance examples/customer360/customers.cove
cove query examples/customer360/customers.cove \
  'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'
cove map build --out-dir target/people-map-build --force \
  examples/coveql/people.covemap examples/coveql/people.jsonl
cove doctor examples/coveql/people.cove
cove inspect examples/coveql/people.cove --queries
cove optimize examples/coveql/events.cove
cove inspect examples/coveql/events.cove --performance
cove query examples/coveql/events.cove \
  'table(events).where(score >= 20).select(id, score)'
cove query examples/coveql/events.cove --perf-report \
  'table(events).where(score >= 20).select(id, score)'
cove query examples/coveql/people.cove \
  'table(people).select(score, status, nickname).take(5)'
cove query examples/coveql/events.cove --engine compare \
  'table(events).where(score >= 20).select(id, score)'
cove query --external-table people=/tmp/people.csv \
  'table(people).where(score >= 20).select(id, score)'
cove query examples/coveql/people.cove \
  'node(Person) as p.degree(kind: total).select(id: p.goid, degree).take(3)'
```

When running from source instead of an installed binary, use the same commands
through Cargo, for example `cargo run -p cove-cli -- examples`.

Longer CoveQL snippets can be supplied with `--query-file <path>` or
`--query-file -` for stdin, and terminal tables support `--max-cell-width`.
Use `cove examples` when you want copy-paste starting points, and `cove doctor
<file>` when you want queryability, performance, and next-step guidance in one
place.
`cove query` uses safe-auto execution by default: validated acceleration
sidecars are used when available, and materialized CoveQL remains the semantic
authority. Use `cove optimize` to create sibling `.covperf.json`, COVE-I/COVX,
COVE-E, and COVE-L sidecars without rewriting the source file. Use
`--engine physical`, `--engine compare`, `--force-kernel`,
`--strict-performance`, or `--perf-report` to inspect and control optimized
execution; use `--enable-graph-traversal` with bounded graph budgets for
variable-length traversals. Use `--external-table name=path` to mount CSV, JSON
array, or JSONL rows as file-backed `ExternalRegisteredTable` providers.

The public utility surface is grouped under the single `cove` binary:

- `cove convert parquet|arrow|orc|csv|report ...` converts between COVE and
  common columnar or row-oriented source formats.
- `cove validate`, `cove inspect`, and `cove dump` cover structural
  validation, readable summaries, and lower-level metadata inspection.
- `cove map validate|preview|plan-keys|convert|build|explain|diff|project|test ...`
  works with COVE-MAP mapping definitions, build bundles, and projection
  workflows.
- `cove map candidates|review|aliases import|replay verify ...` covers
  resolver-backed entity-resolution review, alias import, and replay checks.
- `cove map delta build ...` emits either a delta-built mapped bundle from a
  selected COVM snapshot or a direct semantic `.covedelta` from source rows.
- `cove delta inspect|validate|dump|chain|publish|publish-atomic|reconstruct|compact|checkpoint ...`
  covers delta artifact inspection, COVM chain planning, publication,
  maintenance, and compatibility materialization.
- `cove sidecar inspect ...` and `cove sidecar build ...` expose expert COVE-I,
  COVX, COVM, COVE-COVERAGE, COVE-L, COVE-CACHE, and COVE-R sidecar tooling.
- `cove inspect --ai`, `cove vec build`, `cove train export`, and
  `cove query --cove-ai ...` expose the optional COVE-AI companion sidecar
  workflow.
- `cove export arrow`, `cove perf explain-pruning`, `cove perf plan-cost`,
  `cove digest verify`, `cove profile`, and `cove canonicalise` provide
  integration, planning, integrity, profile, and canonical-value utilities.

## How It Works

### FileCodes and ExecutionCodes

Repeated values such as strings, categories, identifiers, and other canonical
values can be stored as dense file-local integer codes called `FileCode`s. A
file dictionary maps each `FileCode` back to the canonical logical value.

`FileCode` equality is meaningful inside one COVE file. Cross-file equality
requires resolving to canonical values or mapping into an engine-owned execution
code space. COVE-E defines metadata for this kind of engine mapping, but
execution codes remain runtime-local. They are never the portable truth stored
by COVE.

This separation lets an engine mount a file, map file-local values into its own
native dictionary or symbol space, and then run equality, grouping, filtering,
and joins with integer operations. The current DataFusion adapter has opt-in
FileCode dictionary output for integration testing, but benchmark results do
not support treating that path as the default performance win yet.

### Morsels and Proof-Safe Pruning

COVE-T data is organized into table segments subdivided into morsels. The
default morsel size in the spec and reference writer is 4,096 rows. A morsel is
the unit for scheduling, predicate bitmap production, page pruning, late
materialization, row references, and FileCode-to-ExecutionCode remapping. All
columns in a segment share the same morsel boundaries.

Predicate metadata can prove outcomes such as `DefinitelyNo`,
`DefinitelyYes`, or `Unknown` in the spec vocabulary. A reader may skip data
only when the proof is valid for the requested operation. If metadata is absent,
unsupported, stale, corrupt, or not strong enough, the safe behavior is to scan
the candidate data rather than prune it. Structural corruption fails closed.

COVE formalizes this into a coverage model: a coverage provider may
over-include data, but it must not under-include data when it is used for
pruning, metadata-only answers, lookup routing, or index-only access.

### Object and Association Semantics

COVE works as a table archive without any object layer. For organizations that
need richer semantics over fragmented sources, COVE also includes optional
COVE-O and COVE-MAP profiles. These profiles are the path from ordinary source
tables toward canonical objects and associations with deterministic table
readback.

COVE-MAP describes deterministic conversion from source rows into semantic
objects, properties, associations, evidence, and projection readback metadata.
It separates source-row identity from semantic object identity. Source rows are
provenance; destination object identity is produced by declared deterministic
identity rules and semantic join keys.

Resolver-backed mapping extends that identity path with explicit resolution
inputs. A mapping may declare normalization pipelines, `alias_catalog`
resolvers, candidate-match rules, and reviewed decisions. Curated aliases and
reviewed equivalences can authorize GOID merge edges when the identity rule
permits them; fuzzy or candidate-only matches remain evidence and review input,
not object truth. Replay verification binds the mapping, source fingerprints,
resolver/catalog/pipeline digests, reviewed-decision digests, and generated
evidence so a future reader can explain why rows did or did not merge.

The mapping layer is intended to be:

- deterministic: the same declared sources, mapping rules, and function
  versions produce the same semantic assertions;
- versioned and auditable: mapping artifacts carry source catalogs, replay
  fingerprints, function declarations, rule references, and evidence;
- projection-aware: object/association results can be read back through
  deterministic projected table shapes when the mapping declares that behavior.

COVE-MAP is not a probabilistic entity-resolution system, an ETL orchestrator,
or AI-based schema matching. Those systems may produce inputs, but COVE-MAP's
portable contract is deterministic replay, explanation, evidence, and
projection semantics.

CoveQL is the optional read/query layer over that semantic surface. It is
described above because it is a companion to COVE-O and COVE-MAP rather than a
requirement for basic COVE file interoperability.

### AI Companion Sidecars

COVE-AI is an optional sidecar layer for AI-oriented metadata. It adds `.coveai`
and `.covev` companion artifacts for chunk boundaries, tokenization metadata,
FileCode vectors, exact flat vector search, training sample descriptors,
multimodal sequence descriptors, tensor/asset references, and generator
provenance.

COVE-AI does not alter baseline COVE truth. Ordinary COVE-T scans, COVE-O
object reconstruction, and COVE-MAP readback must continue to work without AI
support. A selected AI operation can require a sidecar, but unsupported AI
metadata is optional for ordinary reads.

The current reference implementation is provider-free: it validates supplied
sidecars and can build deterministic local `.covev` vectors, but it does not
call network embedding, tokenizer, or model providers. Exact flat FileCode
vector scan is implemented; unsupported ANN payloads are treated as descriptor
metadata unless a future implementation adds them behind explicit support.

Start with [`docs/cove-ai.md`](./docs/cove-ai.md) for the public guide and
[`spec/09-ai/`](./spec/09-ai/) for normative details.

### Delta Snapshots

COVE files remain immutable. Incremental COVE-O publication uses an immutable
base `.cove`, immutable `.covedelta` artifacts, and an explicit COVM snapshot
that selects the ordered chain. Readers do not scan a directory for undeclared
deltas, and a delta-bearing snapshot must not silently answer from base-only
data.

The current CLI can inspect and validate individual deltas, validate and plan a
selected chain, query or export the selected snapshot, build snapshot-bound
sidecars, reconstruct or compact to a self-contained `.cove`, and publish or
extend COVM manifests:

```bash
cove delta inspect delta-0001.covedelta
cove delta chain validate dataset.covm --dataset bundle
cove delta chain plan dataset.covm --dataset bundle --as-of-csn 100 --json
cove query dataset.covm --dataset bundle --as-of-csn 100 --delta-plan \
  'object(Thing).take(10)'
cove export arrow dataset.covm snapshot.arrow --dataset bundle --delta-plan-json
cove sidecar build covi --snapshot dataset.covm --dataset bundle \
  --out snapshot.covi --object-properties
cove delta reconstruct dataset.covm --dataset bundle --out snapshot.cove
cove delta compact dataset.covm --dataset bundle --out compacted.cove \
  --publish-covm compacted.covm
```

COVE-MAP owns semantic production of resolver-aware object deltas:

```bash
cove map delta build --base dataset.covm --dataset bundle \
  --mapping mapping.covemap --out delta-0002.covedelta sources.jsonl
```

Delta object catalogs are additive for the supported contract. Breaking catalog
changes, object type reinterpretation, or resolver behavior changes require a
new effective semantic-map fingerprint and normally a new base or schema branch
rather than being hidden inside an ordinary additive delta.

### Long-Term Direction

COVE's long-term goal is to make archived tabular data physically efficient
and easier to understand across fragmented sources. Many datasets repeat the
same real-world values and entities: company names, products, customers,
locations, instruments, accounts, and other business objects. COVE's object
and mapping profiles are intended to let those repeated facts be represented
as declared objects and associations while preserving deterministic metadata
that can project the data back into table-shaped views compatible with the
original sources.

In that model, the original tables remain readable, but they are no longer the
only structure available to readers. Source rows become provenance. Objects,
properties, associations, evidence, and projection rules describe the declared
meaning. This can reduce duplicated storage and repeated read work, but the
larger goal is that archived data remains explainable without requiring the
original application stack.

### Object Storage and Cheaper Reads

On object storage, every range request has latency and often a per-request cost.
COVE is designed so readers can make fewer requests when metadata proves that
payload pages are irrelevant, or when layout metadata lets nearby reads be
coalesced.

The current DataFusion adapter includes byte-range readers, mmap-backed local
reads, range coalescing, layout-aware page-cluster coalescing, optional COVI
candidate pruning, and metrics for requested/coalesced ranges. COVE also
defines I/O hints, COVE-L layout plans, scan split indexes, page cluster
directories, and object-store range planning.

This should be treated as a cost model, not a universal benchmark claim. The
release-gated benchmark uses a deterministic offline object-store harness that
records object GETs, range GETs, bytes requested/returned, cold/warm cache
state, and coalescing decisions without requiring S3 or MinIO. Live service
performance remains environment-specific and depends on dataset layout,
predicate selectivity, projected columns, object-store behavior, and whether
the optional layout/index metadata is present and valid.

## Standards Suite Highlights

COVE is a standards suite, not a single mandatory feature pile. Baseline
interoperability is COVE-Core plus COVE-T table scan reading, safe predicate
metadata interpretation, Arrow-compatible export for supported logical types,
and conformance vectors. Optional profiles are implemented or claimed
independently. You do not need the optional profiles below to understand or
implement the baseline tabular archive path.

- **COVE-Core and COVE-T**: file layout, sections, dictionaries, encoded
  arrays, table catalogs, segments, morsels, page indexes, checksums,
  validation, and table scans.
- **COVE-COVERAGE**: formal coverage providers and sets for conservative
  predicate and index planning.
- **COVE-A / COVX / COVM**: acceleration indexes, sidecars, delta-chain
  snapshot selection, and dataset manifests that must preserve file truth.
- **COVE-I**: optional `.covi` secondary index artifacts, including artifact
  framing, index roots, referenced-file/snapshot validity records, local block
  containers, postings, row ordinal sets, aggregate answers, and capability
  records.
- **COVE-E and COVE-H**: generic engine execution-code mapping and a named
  Harbor registration. Generic COVE readers do not require Harbor.
- **COVE-O**: optional object-temporal profile for object catalogs, temporal
  segments, deltas, branches, tombstones, and trust surfaces.
- **COVE-MAP**: optional semantic mapping from source rows into objects,
  associations, resolver-backed identity evidence, reviewed equivalences, and
  deterministic projection readback.
- **CoveQL**: optional semantic query layer over COVE-O/COVE-MAP and
  projection-backed table/graph profiles, with materialized readback as the
  authority and proof-gated optimized execution.
- **COVE-AI**: optional `.coveai` and `.covev` companion artifacts for
  validated AI metadata, FileCode vectors, exact flat semantic search,
  chunk/token/training/multimodal descriptors, and generator audit records.
- **COVE-CX**: registered codec-extension framework with stable COVE-owned v2
  bitstream identities for FSST-style UTF-8, ALP-style floats, and
  FastLanes-style integers, plus mandatory fallback-equivalence validation.
- **COVE-L**: layout planning, scan splits, page clusters, fast metadata
  indexes, and zero-copy maps as optional planning aids, not schema authority.
- **COVE-R and COVE-CACHE**: runtime compatibility hints and runtime/local
  coverage caches. They are not canonical file truth.
- **Feature scopes**: COVE distinguishes file, section, page, profile,
  operation, and advisory requiredness so ordinary table reads do not fail just
  because unrelated optional profile metadata is unsupported.

## Repository Layout

Important paths:

- [`spec.md`](./spec.md): stable entrypoint for the split COVE specification
  tree in [`spec/`](./spec/), which is the current normative baseline for
  implementation and conformance-vector development.
- [`IMPLEMENTERS.md`](./IMPLEMENTERS.md): practical COVE-Core plus
  COVE-T reader/writer starting point for independent implementations.
- [`crates/cove-core`](./crates/cove-core): core file structures,
  validation, dictionaries, encodings, indexes, writers, readers, and profiles.
- [`crates/cove-arrow`](./crates/cove-arrow): Arrow export/import and
  Parquet conversion support layered on `cove-core`.
- [`crates/cove-datafusion`](./crates/cove-datafusion): DataFusion table
  provider, file format integration, pruning, range reads, COVE-L planning
  consumption, optional COVI candidate/index-only use, metrics, COVM/COVX
  bootstrap paths, and benchmarks.
- [`crates/cove-map`](./crates/cove-map): reference COVE-MAP execution,
  materialization, resolver-backed identity planning, evidence, replay, and
  projection helpers.
- [`crates/coveql`](./crates/coveql): CoveQL parser, builder API,
  resolver, dependency contracts, materialized and coded execution, stable
  explain output, Arrow output, manifest-aware planning, and DataFusion table
  provider integration for COVE-O reads.
- [`docs/mapped-cove-o-datafusion-showcase.md`](./docs/mapped-cove-o-datafusion-showcase.md):
  end-to-end multi-source mapped COVE-O showcase through DataFusion SQL.
- [`docs/proposals/cove-o-delta-artifacts.md`](./docs/proposals/cove-o-delta-artifacts.md):
  design context for immutable COVE-O delta artifacts, COVM chain selection,
  pruning, compaction, and checkpointing.
- [`docs/proposals/covemap-entity-resolution.md`](./docs/proposals/covemap-entity-resolution.md):
  resolver catalog and deterministic entity-resolution design context.
- [`docs/covemap-json-schema-v1.md`](./docs/covemap-json-schema-v1.md):
  reference companion schema for COVE-MAP JSON payloads, including resolver
  catalog and projection fields.
- [`docs/cove-ai.md`](./docs/cove-ai.md): public guide for optional COVE-AI
  `.coveai` and `.covev` sidecars, CLI commands, CoveQL-AI methods, and
  conformance gates.
- [`docs/proposals/coveql-object-query-language.md`](./docs/proposals/coveql-object-query-language.md):
  CoveQL/Object proposal and conformance decisions.
- [`docs/proposals/coveql-query-profiles.md`](./docs/proposals/coveql-query-profiles.md):
  CoveQL-Core, Object, Table, and Graph profile contract RFC.
- [`crates/cove-codec`](./crates/cove-codec): COVE-CX descriptor and
  registered-envelope validation.
- [`crates/cove-coverage`](./crates/cove-coverage): COVE-COVERAGE
  provider and coverage-set parsing/inspection.
- [`crates/cove-layout`](./crates/cove-layout): COVE-L layout, fast
  metadata, page-cluster, and scan split metadata helpers.
- [`crates/cove-index`](./crates/cove-index): COVE-I artifact framing,
  validation, safe lookup, index-only answer, and inspection/build helpers.
- [`crates/cove-runtime`](./crates/cove-runtime): COVE-R runtime
  compatibility hints.
- [`crates/cove-cache`](./crates/cove-cache): runtime/local coverage-cache
  artifact helpers.
- [`crates/cove-validate`](./crates/cove-validate): validation logic used
  by `cove validate`.
- [`crates/cove-inspect`](./crates/cove-inspect): readable file layout
  inspection used by `cove inspect --sections`.
- [`crates/cove-dump`](./crates/cove-dump): metadata and section byte
  dumping used by `cove dump`.
- [`crates/cove-convert-parquet`](./crates/cove-convert-parquet):
  Parquet, Arrow IPC, ORC, and CSV conversion logic used by `cove convert`.
- [`crates/cove-conformance`](./crates/cove-conformance): conformance
  runner and generated capability matrix support.
- [`conformance`](./conformance): generated accept/reject corpus,
  manifest, and capability matrix.
- [`docs/performance/datafusion-benchmark-report.md`](./docs/performance/datafusion-benchmark-report.md):
  current local DataFusion benchmark methodology and results.
- [`docs/performance/cove-o-overlap-benchmark-results.md`](./docs/performance/cove-o-overlap-benchmark-results.md):
  current COVE-O overlap and scale results showing where mapped object storage
  does and does not beat duplicated source-shaped Parquet bundles.

## Benchmark Snapshot

The current benchmark report compares COVE through this repository's DataFusion
adapter with DataFusion's native Parquet path. These are local synthetic
fixtures, not universal file-format claims and not object-store measurements.
Ratio is `COVE / Parquet`; lower than `1.00x` means COVE was faster in that
run.

Selected M7 execute-only results:

| Query | COVE | Parquet | Ratio |
| --- | ---: | ---: | ---: |
| `operational_point_lookup` | 288.69 us | 1.565 ms | 0.18x |
| `operational_zero_match` | 22.93 us | 120.17 us | 0.19x |
| `join_selective_dimensions` | 900.47 us | 4.467 ms | 0.20x |
| `join_left_stocked_products` | 455.84 us | 710.74 us | 0.64x |
| `olap_top_customers` | 1.460 ms | 3.027 ms | 0.48x |

Selected M6 COVE-vs-Parquet results:

| Track | COVE | Parquet | Ratio |
| --- | ---: | ---: | ---: |
| `parquet_compare_full_scan` | 227.88 us | 273.39 us | 0.83x |
| `parquet_compare_projection_scan` | 280.25 us | 272.97 us | 1.03x |
| `parquet_compare_low_cardinality_filter` | 234.88 us | 628.14 us | 0.37x |
| `parquet_compare_numeric_range_filter` | 294.33 us | 605.80 us | 0.49x |
| `parquet_compare_wide_projection_filter` | 247.02 us | 607.07 us | 0.41x |

The same report also records cases where Parquet is faster, including the M6
scan-heavy projection path and some OLAP grouping/top-customer full-query
tracks. Planning overhead is visible in the full-query numbers. FileCode
dictionary output is currently opt-in and should not be assumed to be faster
than decoded output.

The mapped COVE-O overlap benchmarks show a separate storage result. On the
current CI-profile synthetic sweep, the COVE-O object becomes smaller as
logical entity overlap rises; the full adoption bundle beats duplicated
source-shaped Parquet only at high overlap. In the 8-table partial-overlap
sweep, the full bundle is still larger at 75% overlap (`1.323x` Parquet) but
smaller at 100% overlap (`0.530x` Parquet). See
[`docs/performance/cove-o-overlap-benchmark-results.md`](./docs/performance/cove-o-overlap-benchmark-results.md)
for the full table and caveats.

## Getting Started

Run commands from the repository root:

```sh
cargo test --workspace
```

Run the release gate:

```sh
sh scripts/release-gates.sh
```

Run the conformance corpus directly:

```sh
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```

Run the CoveQL suite:

```sh
cargo test -p coveql --all-features
```

Run the smaller implementer-kernel subset:

```sh
cargo run -p cove-conformance --bin cove-conformance -- \
  conformance/ --manifest conformance/minimal-reader-manifest.jsonl
```

Register a local `.cove` file with DataFusion:

```rust
use cove_datafusion::register::register_cove_file;
use datafusion::prelude::SessionContext;

let ctx = SessionContext::new();
register_cove_file(&ctx, "orders", "orders.cove")?;

let df = ctx
    .sql("SELECT * FROM orders WHERE status = 'active'")
    .await?;
df.show().await?;
```

Register mapped `COVE-O` projections as SQL tables with DataFusion:

```rust
use cove_datafusion::register::register_cove_o_projections;
use datafusion::prelude::SessionContext;

let ctx = SessionContext::new();
let registered = register_cove_o_projections(
    &ctx,
    "people.cove",
    Some(std::path::Path::new("people-map.covemap")),
    Some("demo"),
)?;

assert!(registered.iter().any(|table| table.table_name == "demo__people"));

let df = ctx
    .sql("SELECT person_id, full_name FROM demo__people ORDER BY person_id")
    .await?;
df.show().await?;
```

If the mapped `COVE-O` file already embeds its projection catalog, the mapping
path can be omitted. Projection tables register from the declared
`output_table` name when present, otherwise from the projection id; an optional
prefix produces deterministic names like `<prefix>__people`.

Execute a CoveQL query directly against mapped `COVE-O` bytes:

```rust
use cove_core::reader::ValidationOptions;
use coveql::{
    parse_resolve_plan_and_execute_query, ExecutionOptions, ParseOptions,
    PlanOptions, ResolveOptions,
};

let bytes = std::fs::read("people.cove")?;
let executed = parse_resolve_plan_and_execute_query(
    &bytes,
    r#"object(Person).where(active == true).select(person_id, full_name).explain("coded")"#,
    ParseOptions::default(),
    ResolveOptions::default(),
    PlanOptions::default(),
    ExecutionOptions::default(),
    ValidationOptions::default(),
)?;

println!("{}", executed.explain_text());
```

For the full multi-source showcase, including canonical-object SQL joined back
to provenance rows from one mapped `COVE-O` file, see
[`docs/mapped-cove-o-datafusion-showcase.md`](./docs/mapped-cove-o-datafusion-showcase.md).

Run the benchmark suites:

```sh
cargo bench -p cove-datafusion --features parquet-compare --bench m6 -- --noplot
cargo bench -p cove-datafusion --features parquet-compare --bench m7_sql_mix -- --noplot
```

For a faster compile-and-smoke pass:

```sh
cargo bench -p cove-datafusion --features parquet-compare --bench m6 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.1
```

## Design Principles

- **Immutable files**: COVE files are write-once-read-many artifacts, not
  mutable database pages.
- **Portable logical truth**: canonical values, nulls, dictionaries, schemas,
  sections, checksums, and validated authoritative metadata define the file.
- **Proofs before pruning**: metadata used to skip data must be conservative,
  validated, and scoped to the requested operation.
- **Fail open for optimization, fail closed for corruption**: unsupported or
  insufficient acceleration falls back to scanning; structural corruption
  rejects.
- **Engine-local execution**: engines may map FileCodes into native runtime
  codes, but those codes are not persisted as portable COVE truth.
- **Subordinate acceleration**: sidecars, manifests, caches, layout plans,
  secondary indexes, and runtime hints can improve reads but must not change
  logical results.
- **Profile-scoped adoption**: readers should reject only the unsupported
  required features that intersect the operation they are actually performing.

## What COVE Is Not

COVE is not a universal Parquet replacement, a WAL, a mutable database file, a
row-level delete protocol, a lakehouse catalog, a lakehouse transaction layer, an
access-control system, an encryption standard, an Arrow IPC replacement, or a
mandatory semantic mapping system.

Parquet and ORC remain mature general-purpose lakehouse formats. COVE is aimed
at immutable archives, converted datasets, object-store planning, predicate-heavy
reads, metadata-answerable queries, deterministic semantic mapping, and engines
that can exploit encoded execution and proof-safe pruning.
