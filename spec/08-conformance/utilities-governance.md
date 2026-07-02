# Utilities, Governance, and Design Summary

## 80. Utilities and Supporting Artifacts

The public COVE project SHOULD ship the following utilities and artifacts.

### 80.1 Reference Libraries

- **cove-core:** Format primitives, checksums, section directory, dictionary, encoded arrays, validation, collation, extension registry.
- **cove-reader:** Read COVE-Core and COVE-T files.
- **cove-writer:** Write COVE-Core and COVE-T files.
- **cove-arrow:** Export COVE data as Arrow arrays / record batches.
- **cove-engine:** COVE-E engine execution profile helpers.
- **cove-harbor:** Optional COVE-H Harbor mount profile implementation.
- **cove-convert:** Conversion library for Parquet/CSV/Arrow/ORC -> COVE-T.
- **cove-map:** Optional COVE-MAP library for deterministic source-row semantics, multi-source identity joins, evidence tracking, materialisation into COVE-O object/association outputs, and deterministic object/association-to-table projections.
- **cove-codec:** Optional COVE-CX library for registered codec descriptors, codec dispatch, fallback validation, and codec conformance tests.
- **cove-layout:** Optional COVE-L library for layout plans, scan splits, page cluster planning, fast metadata indexes, and zero-copy buffer maps.
- **cove-coverage:** Optional COVE-COVERAGE library for predicate normal forms, coverage providers, coverage sets, plan candidates, and proof validation.
- **cove-index:** Optional COVE-I library for building, validating, and querying `.covi` secondary indexes.
- **cove-cache:** Optional COVE-CACHE helpers for runtime/local predicate coverage caches and invalidation.
- **cove-runtime:** Optional COVE-R helpers for explicit reader sessions, registries, FFI adapter discovery, and engine compatibility hints.

### 80.2 CLI Tools

The public user-facing CLI is a single `cove` command with grouped subcommands:

- **cove validate:** Validate structure, CRCs, digests, schema, dictionaries, sections, indexes, profiles, extensions, and conformance.
- **cove inspect:** Print beginner-friendly artifact summaries by default, with detailed section inspection via `--sections`.
- **cove dump:** Dump selected rows, columns, pages, morsels, dictionary values, or encoded array structures.
- **cove query:** Execute CoveQL against queryable COVE artifacts and mappings.
- **cove optimize:** Build safe acceleration sidecars and a performance discovery manifest.
- **cove convert parquet|arrow|orc|csv:** Convert source files to COVE-T.
- **cove convert report:** Emit machine-readable conversion fidelity reports for source-to-COVE and COVE-to-source conversions.
- **cove map validate|preview|plan-keys|convert|build|doctor|suggest|parity|explain|diff|project|test:** Validate, preview, convert, build bundles, verify outputs, suggest starter mappings, compare projections, explain, diff, project, and test COVE-MAP artifacts.
- **cove export arrow:** Export COVE-T tables to Arrow-compatible batches.
- **cove perf explain-pruning|plan-cost:** Explain pruning decisions and estimate projected scan work.
- **cove sidecar inspect index|coverage|layout|cache|runtime:** Inspect COVE-I, COVE-COVERAGE, COVE-L, COVE-CACHE, and COVE-R artifacts.
- **cove sidecar build covi|covx|covm:** Build or refresh COVE-I, COVX, and COVM sidecars.
- **cove digest verify:** Verify cryptographic digests and Merkle roots.
- **cove canonicalise:** Verify canonical value encodings, collation ordering, domain-rank construction, and trust input canonicalisation.
- **cove profile:** Inspect or generate COVE-E engine profile metadata.

Developer-only tools such as **cove-bench**, **cove-fuzz**, **cove-conformance**, and **cove-codec-validate** may remain separate because they are not end-user data utilities.

### 80.3 Engine Integrations

**Recommended initial integrations:**

- **Arrow:** COVE -> Arrow arrays and record batches.
- **DataFusion:** COVE TableProvider.
- **DuckDB:** COVE scan extension / table function.
- **Polars:** COVE scan/read support.
- **Spark / Trino / Presto / ClickHouse:** Optional read-only adapters or table-format data-file adapters once COVE-T conformance vectors are stable.
- **Python:** cove.read_table(), cove.scan(), cove.to_arrow(), cove.to_polars().
- **Java / Scala:** Table-format and engine adapters where JVM ecosystem integration is required.
- **Go:** Lightweight validation, inspection, and service-side read bindings.
- **Rust:** cove-core, cove, cove-arrow, cove-datafusion, cove-engine.
- **WASM / embedded:** Optional lightweight COVE-Core/COVE-T validation and projection readers with optional profiles disabled by default.
- **Harbor:** Optional COVE-H direct leased-code mount support.

**Integration guidance:**
- Engine integrations SHOULD start read-only until COVE-Core/COVE-T conformance vectors pass.
- An engine integration MUST NOT reinterpret optional acceleration metadata as required table semantics.
- Table-format adapters MUST apply external catalog visibility and delete rules before returning rows.

### 80.3A Coverage-Centred Benchmark Reporting

Coverage-aware benchmark reports SHOULD include metrics that show *why* work was avoided, not only final wall-clock time.

**Coverage-level benchmark groups SHOULD include:**
- min/max coverage pruning;
- dictionary/FileCode coverage pruning;
- Bloom/no-false-negative exclusion;
- exact set and inverted-morsel coverage;
- COVE-I global index lookup;
- COVE-CACHE coverage-cache hit and miss planning;
- semantic/dimensional bucket coverage;
- index-only count/min/max/distinct/existence answers;
- object-store many-file coverage planning.

**Coverage metrics SHOULD include:**
- bytes read;
- object-store requests;
- fragments considered;
- fragments in coverage set;
- rows decoded;
- rows materialised;
- coverage degree;
- tightness degree;
- index lookup cost;
- cache hit rate;
- decode time;
- materialisation time;
- full-scan fallback frequency.

A benchmark MUST NOT claim format-level superiority when the result depends on optional COVE-I, COVX, COVE-CACHE, engine-native kernels, hardware acceleration, or zero-copy export unless that dependency is explicitly disclosed and separately measured.

### 80.4 Dataset and Benchmark Corpus

**Recommended corpora:**

- **synthetic-numeric:** numeric full scan and range predicates.
- **synthetic-categorical:** low/medium-cardinality FileCode workloads.
- **synthetic-wide:** hundreds/thousands of columns with small projections.
- **synthetic-point:** lookup-heavy high-cardinality IDs.
- **synthetic-composite:** multi-column predicates and composite pruning.
- **synthetic-coverage:** coverage-set, tightness, coverage-degree, and do-no-harm planning workloads.
- **synthetic-index-only:** exact metadata/index-only count, min, max, distinct-count, and existence checks.
- **synthetic-cache:** repeated predicate-containment workloads with cache hit, miss, and invalidation cases.
- **synthetic-dimensional:** object/dimensional path and bucket queries over sparse or nested data.
- **synthetic-archive:** multi-file object-store-style dataset with COVM.
- **parquet-tpch:** converted TPC-H-style tables.
- **parquet-tpcds:** converted TPC-DS-style tables.
- **parquet-medical-operational:** categorical, temporal, event, and object-history style data.
- **negative-corrupt:** malformed sections, invalid CRCs, bad offsets, invalid FileCodes.
- **canonicalisation:** UTF-8, decimal, timestamp, UUID, NaN, null, map/list/struct cases.
- **semantic-mapping:** CRM/orders/support style multi-source datasets where `Customer.Name` + `Customer.Email` produces a strong deterministic object match, with candidate-name-only and do-not-merge negative cases.
- **engine-profile:** FileCode -> ExecutionCode mapping tests for generic and Arrow profiles; Harbor vectors are required only for COVE-H claims.

**Benchmark reporting:**
- Public performance claims SHOULD publish dataset versions, query definitions, selected columns/predicates, hardware, storage medium, cold/warm cache state, thread count, engine version, COVE writer settings, comparator format settings, and reproducible scripts.
- Benchmarks SHOULD separate file-size, conversion cost, cold planning latency, warm planning latency, scan CPU, decompression CPU, materialisation time, object requests, bytes read, rows decoded, rows materialised, coverage degree, tightness degree, coverage-provider lookup cost, index build cost, cache hit rate, and end-to-end query latency.
- A benchmark MUST NOT claim format-level superiority when the result depends on a non-portable engine shortcut that is unavailable to the compared format, unless the shortcut is explicitly disclosed.

### 80.5 Governance Artifacts

**For open adoption, the project SHOULD publish:**
- formal binary specification,
- semantic versioning policy,
- feature bit registry,
- section kind registry,
- encoding kind registry,
- extension registry,
- engine profile registry,
- collation registry,
- COVE-MAP deterministic function registry,
- COVE-MAP identity confidence-class and row-semantics registry,
- COVE-COVERAGE proof-kind, proof-strength, coverage-granularity, and predicate-form registries,
- COVE-I index-kind and index-capability registries,
- COVE-CACHE compatibility and invalidation registry,
- test vector registry,
- implementation conformance levels,
- performance benchmark methodology,
- security model,
- trademark/name guidance,
- extension proposal process.

Governance rules SHOULD ensure that required feature bits, section kinds, encoding IDs, and profile registrations are not controlled by a single proprietary engine or vendor-specific implementation. Named engine registrations are allowed, but they MUST remain optional unless a reader explicitly claims that named profile.
**Governance for new stable features SHOULD require:**
- an extension proposal or specification patch,
- assigned feature bits and registry entries where applicable,
- fallback and unknown-reader behaviour,
- security/privacy review for features that expose, hide, encrypt, redact, or approximate data,
- positive and negative conformance vectors,
- reference implementation support,
- interoperability evidence from at least one independent implementation before broad ecosystem conformance claims are made.

---


## 80.6 Preservation Checklist for Reviewers

This combined specification retains COVE's core concept while adding the current standards-suite mechanisms. Reviewers SHOULD verify that no future edit removes these preserved capabilities unless an explicit design decision records the replacement.

| Original value | Preserved in this draft |
| --- | --- |
| Immutable `.cove` files | COVE-Core invariants and durable replace rules. |
| Engine-neutral table scans | COVE-T baseline and starter interoperability subset. |
| File-local FileCodes | Core concepts, dictionary rules, and trust rules. |
| Engine-local ExecutionCodes | COVE-E and COVE-H, explicitly non-authoritative. |
| Null bitmap polarity `1 = null` | Core null semantics and Arrow conversion rules. |
| Morsel-aligned scans | COVE-T segments, morsels, predicate bitmaps, and late materialisation. |
| Predicate proof metadata | Zone stats, exact sets, blooms, composition rules, and conservative pushdown. |
| Archive acceleration | COVE-A, COVX, COVM, lookup indexes, synopses, composite zones, Top-N. |
| Object-temporal profile | COVE-O object catalogs, temporal segments, deltas, baselines, snapshots, tombstones, trust chains. |
| Semantic mapping | COVE-MAP artifact framing, source catalogs, row semantics, identity rules, associations, evidence, projections. |
| Harbor execution value | COVE-H named registration, separate from COVE-Core. |
| Arrow/lakehouse compatibility | COVE-Interop sections without making Arrow IPC or a table protocol authoritative. |
| Security/privacy boundaries | Redaction, digest, trust, privacy, sensitive index guidance, no v2 encryption claims. |
| Public conformance | Conformance levels, vectors, utilities, benchmark corpus, governance artifacts. |
| New v2 codec/layout/runtime value | COVE-CX, COVE-L, COVE-R added as subordinate optional standards. |
| Full-detail preservation | Split documents and starter subsets are implementation/conformance boundaries, not micro-spec replacements. |


## 81. Summary of v2 Design Decisions

**COVE v2 chooses:**

- **Neutral public name:** Cove Format, with Harbor represented only as an optional named COVE-H profile.
- **File-local FileCodes:** over persisted engine-owned codes.
- **ExecutionCode abstraction:** so non-Harbor engines can map FileCodes into their own runtime representations.
- **COVE-E universal engine execution profile:** over making any one engine's mount behaviour the generic extension mechanism.
- **COVE-H Harbor profile:** optional Harbor leased-code execution as one registered COVE-E implementation.
- **Scope descriptors:** over hard-coded tenant fields in the universal core.
- **Morsel-aligned pages:** over generic row-group-only scans.
- **Encoded arrays:** over flat codec-only compression.
- **Column domains:** over raw FileCode min/max.
- **Predicate proof outcomes:** over skip-only pruning.
- **Exact sets, blooms, lookup indexes, and aggregate synopses:** over statistics-only acceleration.
- **Composite zone indexes:** over single-column-only pruning.
- **COVX sidecars:** over mutable in-file workload indexes.
- **COVE-COVERAGE:** over vague pruning hints, so conservative coverage sets and proof strength are explicit.
- **COVE-I secondary indexes:** over mandatory global indexes, so value/path/dimensional indexes remain optional and snapshot-bound.
- **COVE-CACHE runtime coverage caches:** over persisted mutable file state, so predicate containment and coverage reuse remain engine-local and non-authoritative.
- **COVM manifests:** over opening every archive file for planning.
- **Extension registry:** so custom logical types, indexes, synopses, encodings, and engine profiles are safe, discoverable, and either ignorable or required.
- **Arrow interop:** so COVE-T is useful without Harbor.
- **Lakehouse compatibility:** so COVE files can live inside existing catalog/table ecosystems.
- **No COVE table protocol in v2:** over duplicating Iceberg/Delta/Hudi-style ACID catalog responsibilities inside the file spec.
- **External visibility overlays:** so delete vectors and table snapshots can be applied safely without changing immutable COVE file semantics.
- **Binary section directories:** over JSON-authoritative metadata.
- **Digest manifests:** over CRC-only archive integrity.
- **Self-contained object reconstruction:** over mandatory cross-file prev_ref.
- **WORM durable replace:** over in-place mutation.
- **Extension-gated vectors, tensors, semantic JSON, encryption, and advanced indexes:** over adding immature workload-specific semantics to COVE-Core v2.
- **COVE-MAP as an optional conversion/projection profile:** over embedding multi-source identity resolution, business-object mapping, source reconciliation, association readback, or object-to-table projection semantics into COVE-Core or COVE-T.
- **Deterministic multi-column semantic join keys:** over probabilistic or hidden matching for canonical object identity.

**The final shape is:**

- **COVE-Core:** immutable binary foundation.
- **COVE-T:** engine-neutral table scan format.
- **COVE-A:** queryable archive acceleration profile.
- **COVE-COVERAGE:** conservative coverage semantics and proof vocabulary.
- **COVE-I:** optional secondary index artifact profile.
- **COVE-CACHE:** optional runtime/local predicate coverage cache guidance, not canonical artifact truth.
- **COVE-E:** universal engine execution/mount profile.
- **COVE-H:** optional Harbor leased-code implementation of COVE-E.
- **COVE-O:** optional object-temporal extension profile.
- **COVE-MAP:** optional deterministic semantic mapping and multi-source object-conversion profile.
- **COVX:** optional rebuildable accelerator sidecar.
- **COVM:** optional multi-file dataset manifest.
This gives Cove Format a neutral public identity, a strict portable decode path, rich queryable archive acceleration, a universal execution-profile mechanism, and an optional path from fragmented source data into object-based COVE while allowing named engine fast paths such as COVE-H without making them dependencies of the core format.



### 81.1 Coverage-Centred v2 Philosophy

COVE v2 makes coverage the conceptual centre of acceleration. Compression, codecs, indexes, sidecars, layout plans, zero-copy export, late materialisation, runtime registries, semantic maps, and hardware-neutral kernels are all valuable, but they cohere only when the reader can tell which fragments are sufficient for a predicate and why.

**Final design principle:**

COVE stores immutable logical values in explicitly declared physical encodings. Optional metadata and sidecars may conservatively prove which fragments are sufficient for a predicate, describe how to evaluate predicates against encoded data, and expose acceleration paths. All such acceleration is advisory unless explicitly required by an extension or requested operation, and ignoring it must preserve logical correctness.

**Public positioning rule:**
COVE MUST NOT claim universal superiority over Parquet, ORC, Arrow IPC, Iceberg, Delta, Hudi, DuckLake, or any particular engine. The credible claim is narrower and stronger: COVE makes acceleration explicit, portable, optional, coverage-aware, and safe for selective, dimensional, indexed, cached, late-materialised, or object/archive workloads.


## 82. Summary of Current Design

COVE keeps its archive-format identity while adding only the mechanisms that improve performance and implementation maturity without turning COVE into a layout-tree clone.

**COVE defines:**
- `COV2`, `CV2F`, `CVX2`, `CVM2`, and `CMP2` major-version artifact identifiers;
- widened header with bootstrap pointers for extended features, profile capabilities, and fast metadata;
- extended feature set section;
- COVE-CX registered codec-extension profile;
- registered encoding page envelope;
- codec/kernel capability binding;
- COVE-L layout-plan profile;
- scan split index;
- page cluster directory;
- zero-copy buffer map;
- fast metadata index;
- COVE-R explicit runtime registry/session guidance;
- COVE-COVERAGE coverage provider, coverage set, predicate normal-form, and plan-candidate structures;
- COVE-I `.covi` secondary index artifact framing, index roots, and index-only capabilities;
- COVE-CACHE runtime/local cache guidance and invalidation rules;
- stronger registry discipline for extensions and runtime bindings;
- conformance vectors for registered codecs, fallback payloads, layout plans, split indexes, zero-copy maps, fast metadata, and runtime hint behaviour.

**COVE preserves:**
- immutable write-once-read-many files;
- explicit table/object catalog authority;
- file-local FileCode semantics;
- portable canonical logical values;
- structural null bitmaps;
- predicate-proof pruning rules;
- COVE-A acceleration as optional and semantics-preserving;
- COVE-E and COVE-H as execution mappings, not logical truth;
- COVE-O self-contained reconstruction;
- COVE-MAP deterministic object/association/provenance semantics;
- COVX and COVM as optional sidecar/manifest artifacts;
- ignorable-or-required extension discipline.

**COVE explicitly does not adopt:**
- dtype-only schema authority;
- generic runtime layout trees as the file's logical data model;
- unchecked plugin IDs as wire-format semantics;
- advisory statistics as proof;
- mandatory dependency on any single engine, library, runtime, or vendor;
- lossy specialised encodings as core COVE semantics.

The design therefore lets COVE use registered codecs, lazy planning, zero-copy metadata, and implementation registries while remaining a canonical, immutable, queryable archive format with portable values, explicit schema, proof-carrying metadata, and optional engine-local execution mappings.
