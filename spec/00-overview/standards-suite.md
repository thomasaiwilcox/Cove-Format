# COVE Standards Suite v2.0 — Full Specification
> **Specification status:** This is the full-detail combined v2.0 specification and the current normative baseline for implementation and conformance-vector development. It defines the active COVE standard in this repository and incorporates the Harbor Row Semantics material. It intentionally preserves the original COVE identity while adding optional registered codec, layout-planning, zero-copy, runtime-registry, conservative query-coverage, optional secondary-index, runtime/local coverage-cache, and richer deterministic row-semantics mechanisms.
>
> **Non-reduction rule:** This is not a micro-spec, summary, or reduced profile. The document remains a full specification. Implementation staging and future split documents are conformance and organisation tools only; they MUST NOT reduce the normative detail below the original.
>
> **Document model:** This combined specification is written as one document for review and implementation, but each major part is designed to split cleanly into a standalone full standard: COVE-Core, COVE-T, COVE-COVERAGE, COVE-A, COVE-I, COVE-E, COVE-H, COVE-O, COVE-MAP, COVE-CX, COVE-L, COVE-R, COVE-CACHE, COVE-Interop, and COVE-Conformance.

**COVE:** Canonical Offline Value Encoding
Cove Format is a Canonical Offline Value Encoding: an immutable,
queryable offline/archive format for portable logical values, encoded
arrays, proof-carrying predicate and coverage metadata, optional acceleration
artifacts, optional secondary indexes, and engine-local execution mappings.

| Field | Value |
| --- | --- |
| Format Name | Cove Format |
| Formal Expansion | Canonical Offline Value Encoding |
| Normative Acronym | COVE |
| Public Short Name | Cove |
| Primary Data File Magic | COV2 |
| Footer Magic | CV2F |
| Accelerator Sidecar Magic | CVX2 |
| Dataset Manifest Magic | CVM2 |
| Semantic Mapping Artifact Magic | CMP2 |
| Secondary Index Artifact Magic | CVI2 |
| Runtime Coverage Cache Artifact Magic | None normative in v2; implementation-defined if persisted locally |
| Legacy Draft Identifiers | Non-normative pre-COVE draft artifacts; not valid COVE v2 identifiers |
| Canonical Extension | .cove |
| Short Extension | None in v2; do not introduce .cov unless later required |
| Accelerator Sidecar Extension | .covx |
| Dataset Manifest Extension | .covm |
| Semantic Mapping Extension | .covemap |
| Secondary Index Extension | .covi |
| Runtime Coverage Cache Extension | None normative in v2; implementation-defined and non-canonical if persisted locally |
| MIME Type | application/vnd.cove-format |
| Version | 2.0 full-detail combined specification |
| Byte Order | Little-endian throughout; no byte-order negotiation in v2 |
| Mutability | Immutable / write-once-read-many |
| Primary Purpose | Engine-neutral queryable offline/archive format with optional engine execution profiles, optional semantic source-to-object/association conversion, registered lossless codec extensions, optional coverage proofs, optional secondary indexes, optional layout/split planning metadata, optional runtime/local coverage caches, and optional runtime registry interoperability hints |
| Compatibility Posture | COVE uses `COV2` magic and major version 2. Legacy pre-2 artifacts are outside this standard and are not part of the active reference implementation. |
| V2 Identity Rule | Catalog/schema, canonical logical values, predicate-proof metadata, validated coverage proofs used for pruning or metadata-only answers, COVE-O truth, COVE-MAP mapping/replay truth when requested, COVM publication state, and digest/trust/redaction surfaces remain authoritative. Codec, layout, zero-copy, COVX acceleration, writer cost metadata, and runtime-registry additions are non-authoritative unless explicitly required for decode or for the requested operation. |
| Standards Suite Rule | COVE-Core and COVE-T are the first public implementation target, but not a smaller spec. Other profiles, including COVE-COVERAGE, COVE-I, COVE-CACHE, COVE-MAP, COVE-CX, and COVE-L, are optional standards with explicit feature bits, fallback behaviour, validation boundaries, conformance claims, and full normative detail when defined. |

The generated capability matrix in `conformance/capability_matrix.md` is the implementation-status record for this workspace. It distinguishes fully gated conformance from partial, unit-only, and vector-family scoped implementation evidence.


---


## 0. Standards Suite Scope, Detail Preservation, and Split Plan

COVE v2 is a **standards suite** for immutable, canonical, queryable archive data. The primary `.cove` file stores portable logical values and validated physical encodings. Companion artifacts may describe acceleration, manifests, semantic mappings, codec registrations, layout plans, and runtime compatibility. Only explicitly authoritative surfaces define logical truth. Optional surfaces are ignorable unless required by feature bit or by the requested operation.

This combined specification is intentionally one document for design review and implementation. It SHOULD later be split into the following standalone standards without changing the meaning of the combined specification. A split standard MUST remain a full specification for its scope, not a micro-spec, overview, or thin adapter note. Each split standard MUST carry its own normative structures, validation rules, feature bits, fallback behaviour, failure behaviour, conformance requirements, and test-vector obligations.

| Part | Future standard | Scope |
| --- | --- | --- |
| Part 0 | COVE-Overview | Positioning, identity, terminology, conformance tiers, and the standards-suite map. |
| Part 1 | COVE-Core | File layout, primitives, canonical values, feature model, dictionary, extension registry, checksums, digests, validation, and error model. |
| Part 2 | COVE-T | Table catalog, segments, morsels, pages, encoded arrays, null semantics, ColumnDomains, predicate proofs, and table scans. |
| Part 3 | COVE-COVERAGE | Formal conservative query coverage semantics: coverage sets, tightness, coverage degree, proof strength, provider metadata, interval forms, and do-no-harm planning. |
| Part 4 | COVE-A | Archive/query acceleration: exact sets, blooms, inverted indexes, lookup indexes, synopses, composite zones, Top-N summaries, COVX, and COVM planning. |
| Part 5 | COVE-I | Optional secondary index artifacts, root indexes, value-to-fragment mappings, index-only capabilities, and snapshot validity. |
| Part 6 | COVE-E | Generic FileCode-to-ExecutionCode execution mapping for engines. |
| Part 7 | COVE-H | Harbor leased-code registration under COVE-E. |
| Part 8 | COVE-O | Object-temporal profile: object catalogs, temporal segments, deltas, baselines, snapshots, branches, tombstones, and trust chains. |
| Part 9 | COVE-MAP | Deterministic semantic mapping from source rows into object/property/association/evidence assertions, dimensional coverage maps, and projections. |
| Part 10 | COVE-CX | Registered lossless codec extension framework, codec descriptors, registered encoding envelopes, fallback payloads, and codec conformance vectors. |
| Part 11 | COVE-L | Layout planning, scan splits, page clusters, fast metadata indexes, zero-copy buffer maps, and object-store range planning. |
| Part 12 | COVE-R | Runtime/session registry guidance and optional runtime compatibility hints. |
| Part 13 | COVE-CACHE | Optional mutable runtime/local predicate coverage cache with snapshot-bound validity and explicit non-authority. |
| Part 14 | COVE-Interop | Arrow, Parquet/ORC/CSV/Arrow IPC conversion, lakehouse integration, external visibility overlays, and publication rules. |
| Part 15 | COVE-Conformance | Reader/writer levels, conformance vectors, negative corpora, registries, governance, and benchmark methodology. |

### 0.1 Full-Detail Specification Rule

COVE v2 MUST NOT become a micro-spec. The combined specification and any future split standards MUST remain detailed enough for independent implementation without private knowledge.

**Rules:**
- A normative profile MUST define its binary structures, field meanings, enum values, validation rules, required and optional feature bits, fallback behaviour, failure behaviour, and conformance requirements.
- A split document such as `COVE-MAP`, `COVE-CX`, or `COVE-L` MUST be a full standard for that domain, not a summary of the combined specification.
- Implementation staging, starter subsets, and recommended first targets are adoption tools only. They MUST NOT remove detail from the standard.
- Extension schema specifications MAY define additional extension payload grammars where the main document intentionally reserves an extension point, but they MUST NOT replace or narrow normative profile grammars defined in this combined specification. COVE-MAP v2 mapping payloads are defined by this document, not by a separate required schema.
- A feature that is not specified in enough detail for independent implementation MUST remain explicitly provisional, experimental, or registry-reserved, and MUST NOT be required for broad conformance.
- No future editorial split may delete a normative structure, validation rule, fallback rule, failure rule, or conformance requirement merely because it is optional to implement.

### 0.2 First Public Implementation Target, Not Specification Reduction

The first public implementation and interoperability target for COVE v2 MAY be staged so implementers can ship a correct reader/writer before every optional profile is implemented. This staging is **not** a reduction of the specification. COVE v2 remains a full-detail standards suite, and optional profiles remain fully specified when this document defines them.

A first implementation target SHOULD prioritise:

- COVE-Core structural validation;
- COVE-T table scan reading and writing;
- FileCode and NumCode decode;
- structural null bitmap handling;
- page checksums and section validation;
- safe predicate metadata interpretation;
- morsel-level zone statistics for common primitive and comparable FileCode columns;
- Arrow-compatible export for supported logical types;
- a reproducible binary conformance vector set.

COVE-A, COVE-E, COVE-H, COVE-O, COVE-MAP, COVE-CX, COVE-L, COVE-R, COVX, and COVM remain optional implementation/conformance claims unless an implementation explicitly claims those standards or a requested operation requires them. Their optionality does not make them lesser, sketch-level, or non-normative. Where the combined specification defines their wire structures and behaviour, they MUST be specified at full detail.

### 0.3 Authoritative and Advisory Surfaces

COVE v2 preserves the original COVE principle that portable logical truth must not depend on an engine, a sidecar, a layout tree, or a runtime plugin registry.

**Authoritative surfaces include:**

- the COVE file header, postscript, footer, and binary section directory;
- required feature declarations and validated required sections;
- table and object catalogs;
- canonical logical values and canonical value encodings;
- file-local FileCode dictionaries and ColumnDomain ordering;
- NumCode interpretation by declared logical type;
- structural null bitmaps and page reconstruction rules;
- predicate-proof and coverage-proof metadata when it is used to skip, include, or answer data;
- COVE-O object-temporal reconstruction rules;
- COVE-MAP mapping artifact semantics when mapping conversion, replay, explanation, or projection readback is requested;
- digest manifests, trust chains, redaction manifests, and COVM publication state when those policies are requested or required.

**Advisory or non-authoritative surfaces include, unless explicitly required for decode or for the requested operation:**

- COVX acceleration indexes and workload-specific sidecars;
- COVE-I secondary index artifacts unless their exactness, snapshot validity, and proof contract are validated for the requested operation;
- COVE-CACHE runtime/local coverage caches;
- COVM planning hints other than the selected publication state itself;
- COVE-L layout-plan nodes;
- scan split indexes;
- page-cluster directories;
- zero-copy buffer maps;
- runtime compatibility hints and runtime registry bindings;
- engine execution profiles and ExecutionCodes;
- writer cost-model metadata;
- advisory statistics, coverage estimates, and cost estimates not marked proof-safe or not validated.

A reader MUST NOT use advisory metadata to change query results. A reader MAY use advisory metadata for planning, performance, diagnostics, or runtime dispatch after validation.

### 0.4 Value Preservation Rule

This combined specification MUST preserve COVE's core design value:

- immutable write-once-read-many `.cove` files;
- engine-neutral COVE-Core and COVE-T readability;
- file-local FileCodes, with FileCode(0) as an ordinary value and never a null sentinel;
- ExecutionCodes as engine-local runtime values, never portable logical truth;
- structural nulls represented by a null bitmap where `1 = null`;
- morsel-aligned scanning, pruning, late materialisation, and execution remap;
- ColumnDomain-based logical ordering for FileCode columns;
- conservative predicate-proof pushdown with `DefinitelyNo`, `DefinitelyYes`, and `Unknown`;
- optional exact sets, blooms, lookup indexes, synopses, composite zones, Top-N summaries, COVX, and COVM;
- COVE-O self-contained object reconstruction;
- COVE-MAP deterministic identity, object/association semantics, evidence, and projection readback;
- Arrow and lakehouse interoperability without making Arrow IPC or a table protocol the COVE identity;
- durable replace publication and rejection of partially written COVE files;
- public conformance vectors and negative validation corpus.

### 0.5 Maturity Rule

COVE v2 adds modern mechanisms only when they are subordinate to COVE logical truth:

- COVE-CX registered codecs MAY improve compression and scan performance, but codec names and plugin IDs are not sufficient wire semantics.
- COVE-L layout plans MAY improve object-store and lazy-read planning, but layout nodes are not schema authority or predicate proof.
- Zero-copy maps MAY reduce export cost, but target compatibility must be proven before exposing COVE buffers directly.
- COVE-R runtime/session guidance MAY help implementations instantiate codecs, kernels, functions, and adapters, but process-global runtime state MUST NOT define on-disk semantics.
- COVE-MAP MAY use Harbor-inspired row semantics, but Harbor runtime write behaviour remains a named implementation influence, not a COVE-Core requirement.

### 0.6 Standards Boundary for Harbor Row Semantics

The Harbor Row Semantics model is valuable because it clearly separates what a source row **is** from how meaning is derived from it. COVE-MAP adopts the engine-neutral version of that idea for offline deterministic mapping.

**Boundary rule:**

- Harbor Row Semantics answers: *what should a SQL mutation do inside Harbor now?*
- COVE-MAP answers: *what deterministic semantic assertions does this source row produce for archive materialisation, replay, explanation, or projection?*

COVE-MAP MUST NOT require Harbor software, Harbor tenancy, Harbor leases, or Harbor object graph runtime behaviour. Harbor may implement COVE-MAP and COVE-O efficiently through COVE-H, but that is a named profile registration, not a core dependency.


### 0.6A Accepted and Constrained Additions from Coverage Review

The coverage-centred additions are accepted with constraints so they strengthen COVE without weakening the original format.

**Accepted into the v2 suite:** COVE-COVERAGE, predicate normal forms, interval predicate forms, balanced coverage plan candidates, sidecar validity, index-only capability declarations, COVE-I secondary indexes, COVX kernel descriptors, dimensional coverage maps, late materialisation/export capabilities, and stronger benchmark metrics.

**Accepted but constrained:** COVE-CACHE is useful as runtime/local snapshot-bound state, but it is not a canonical COVE artifact and MUST NOT be required for logical correctness. Hardware acceleration descriptors are useful, but they are optional capability metadata and MUST NOT make a file vendor-hardware-dependent unless a non-portable required extension explicitly says so.

**Not adopted as core requirements:** mandatory global secondary indexes, mandatory hardware acceleration, a table/lakehouse transaction protocol, mutable in-file caches, a universal query optimiser encoded in bytes, and any claim that COVE is generally faster than Parquet or replaces Iceberg/Delta/Hudi.

### 0.7 Coverage-Aware v2 Identity

COVE v2 SHOULD be understood as a **coverage-aware value/archive format**, not merely as a columnar layout with optional statistics. A coverage-aware format describes not only how values are stored, but which validated fragments are sufficient to answer or evaluate a predicate without reading the whole dataset.

**Coverage principle:**

A COVE coverage artifact may over-include data, but it MUST NOT under-include data when it is used for pruning, metadata-only answers, lookup routing, or index-only access. An artifact that may under-include data is approximate or advisory and MUST NOT be used to skip candidate data unless a required extension explicitly defines a bounded-loss query semantics and the query requests that semantics.

**Coverage-aware identity surfaces:**

- COVE-T zone stats, exact sets, blooms, inverted morsel indexes, lookup indexes, aggregate synopses, and composite zones may act as coverage providers when their proof semantics are validated.
- COVE-A and COVX may carry rebuildable acceleration providers, but they remain semantics-preserving.
- COVE-I may carry optional secondary indexes that map values, intervals, object paths, or dimensional buckets to files, segments, pages, morsels, row ranges, row ordinals, objects, or projection fragments.
- COVE-MAP may define object, association, semantic path, and dimensional mappings that allow coverage over non-flat or object-derived data.
- COVE-L may describe how coverage fragments correspond to byte ranges, page clusters, scan splits, and object-store requests.
- COVE-CACHE may remember previously validated coverage sets for a dataset snapshot, but it is mutable runtime/local state and never canonical truth.

**Rules:**
- Coverage metadata MUST declare its granularity, proof strength, exactness, snapshot validity, referenced logical context, and fallback behaviour.
- Coverage metadata MUST be checksummed and bounds-checked before use.
- Coverage metadata MUST be interpreted under the declared logical type, collation, null semantics, canonicalisation rules, and feature/profile version.
- A coverage provider MUST NOT silently substitute physical-code comparisons for logical comparisons unless the encoding explicitly declares them safe.
- Ignoring coverage metadata MUST preserve logical correctness; it may only reduce performance.
- COVE-Core and COVE-T remain decodable without COVE-I, COVX, COVM, COVE-MAP, or any runtime-local COVE-CACHE state unless a requested operation explicitly requires one of those optional surfaces.


## 1. Specification Status

This document defines Cove Format v2.0, hereafter COVE.
COVE means Canonical Offline Value Encoding.
**COVE defines the following profiles and companion artifacts:**

- **COVE-Core:** Common immutable file structure, section directory, dictionary, logical/physical types, encoded arrays, checksums, validation, collation, canonical values, and extension rules.
- **COVE-T:** Engine-neutral table-scan profile.
- **COVE-COVERAGE:** Optional formal coverage-semantics profile for conservative predicate coverage sets, tightness/coverage metrics, proof strength, interval forms, provider metadata, and do-no-harm planning. COVE-COVERAGE is the common proof vocabulary used by COVE-T, COVE-A, COVX, COVE-I, COVM, COVE-MAP, and COVE-CACHE when those profiles expose coverage.
- **COVE-A:** Archive acceleration profile for synopses, lookup indexes, composite pruning, manifests, and sidecar acceleration.
- **COVE-E:** Universal engine execution profile for mapping FileCodes into implementation-local ExecutionCodes.
- **COVE-H:** Optional named Harbor registration under COVE-E. Defines Harbor leased-code execution: FileCode -> Harbor EngineCode. COVE-H is not required for generic COVE conformance.
- **COVE-O:** Optional object-temporal extension profile for committed object history, deltas, branches, CSNs, baselines, snapshots, tombstones, and trust chains. COVE-O is not required for generic COVE conformance.
- **COVE-MAP:** Optional deterministic semantic mapping profile and companion `.covemap` artifact for converting one or more external source tables/files/streams into paired object-and-association semantic assertions, properties, temporal facts, and evidence that may be materialised as COVE-O and exposed through optional COVE-T/Arrow/SQL table projections. COVE-MAP is not required for generic COVE conformance.
- **COVE-CX:** Optional registered codec-extension profile for lossless specialised encodings, codec capability descriptors, canonical fallback rules, and conformance vectors. COVE-CX is the v2 path for FSST-style string encoding, ALP-style floating-point encoding, FastLanes-style integer packing/frame-of-reference/delta encoding, and future codecs.
- **COVE-L:** Optional layout-plan and scan-split profile. COVE-L describes lazy read planning, page clusters, split generation, and object-store range grouping without replacing the COVE table catalog, segment index, page index, or predicate-proof metadata.
- **COVE-R:** Optional runtime registry/session interoperability guidance and artifacts. COVE-R describes how implementations advertise supported codec, profile, index, kernel, FFI, and engine-adapter capabilities without making runtime state part of COVE logical truth.
- **COVE-I:** Optional secondary index artifact profile and `.covi` artifact for value-to-fragment, path-to-fragment, dimensional-bucket, row-range, and index-only access metadata.
- **COVE-CACHE:** Optional mutable runtime/local coverage-cache profile for snapshot-bound predicate containment and coverage reuse. COVE-CACHE is never canonical file truth.
- **COVX:** Optional accelerator sidecar.
- **COVM:** Optional dataset manifest.
A conforming COVE reader MUST be able to validate and read COVE files without COVX, COVM, COVE-I, COVE-CACHE, or COVE-MAP.
COVX, COVM, and COVE-I are optional acceleration, planning, index, or manifest artifacts. COVE-CACHE is optional runtime/local state, not canonical file truth. None of these surfaces may change the logical meaning of referenced COVE files. COVE-MAP artifacts MUST NOT change the logical meaning of already materialised COVE files; they define how source data is converted, replayed, explained, or re-materialised into new COVE outputs.

### 1.1 Profile Maturity and Conformance Surface

COVE v2 is profile-scoped. Implementers MUST NOT treat the existence of an optional profile in this document as a requirement for baseline COVE conformance.
**Baseline v2 interoperability target:**
- COVE-Core structural validation and typed logical decode,
- COVE-T table scan reading,
- safe predicate metadata interpretation,
- Arrow-compatible export for supported logical types,
- a reproducible binary conformance vector set.
**Optional v2 profiles and artifacts:**
- COVE-A archive acceleration,
- COVE-COVERAGE coverage proof vocabulary,
- COVE-I secondary index artifacts,
- COVE-E engine execution-code mapping,
- COVX accelerator sidecars,
- COVM dataset manifests,
- COVE-MAP semantic mapping artifacts when mapping tooling is claimed,
- COVE-CX registered codec extensions,
- COVE-L layout plans, split indexes, page cluster directories, and zero-copy maps,
- COVE-R runtime compatibility manifests and session/registry hints,
- COVE-CACHE runtime/local predicate coverage caches.
**Named engine registrations:**
- COVE-H is a Harbor-specific COVE-E registration. It demonstrates and standardises one engine profile; it is not a dependency of COVE-Core, COVE-T, COVE-A, or generic COVE-E.
**Optional extension profiles:**
- COVE-O is an optional object-temporal profile. It MAY be implemented by temporal-object engines, but general table readers SHOULD ignore COVE-O sections unless the requested operation explicitly requires object-temporal semantics.
- COVE-MAP is an optional v2 profile with a stable conceptual and conformance boundary: artifact magic, feature bit, validation boundary, identity model, operation-level rules, reusable `.covemap` artifact framing, and standard `MAP_*` payload schemas are part of this specification. General COVE readers SHOULD ignore COVE-MAP artifacts or sections unless the requested operation explicitly requires mapping validation, mapping replay, mapping explanation, source-to-object/association conversion, or mapping-defined projection readback.

A file that contains optional profile sections MUST advertise the corresponding feature bits. A reader that does not implement an advertised optional profile MUST either ignore the profile when it is not required for the requested operation, or reject the requested operation with a profile-not-supported error.
Implementations that claim COVE-MAP support SHOULD state which standard `MAP_*` section kinds and registered extension payload encodings they support. A COVE-MAP v2 artifact validator MUST support the standard payload schema defined in Section 70 for the section kinds it claims.

**Standards-suite conformance note:** An implementation SHOULD state conformance at the narrowest honest level, for example `COVE-Core v2 reader`, `COVE-T starter reader`, `COVE-T scan writer`, `COVE-CX-aware reader`, `COVE-MAP artifact validator`, or `COVE-H Harbor registration`. A product MUST NOT claim broad COVE v2 support merely because it can parse the header or use one named engine profile. A narrow conformance claim is not a narrow specification; it is an honest implementation boundary.

### 1.2 Named Engine and Product-Specific Terms

COVE is an engine-neutral format. Product-specific names are allowed only in named profiles, examples, registries, or non-normative implementation guidance.

Harbor is a named engine/profile registration that supplied the initial leased-code execution use case. Generic COVE text SHOULD use engine-neutral terms such as engine, scope, ExecutionCode, code-space, mapping, and profile. Harbor-specific concepts such as Harbor tenant UUID, Harbor EngineCode, Harbor lease, and Harbor mount cache apply only to COVE-H or to examples explicitly labelled as Harbor examples.

A COVE-Core, COVE-T, COVE-A, or generic COVE-E implementation MUST NOT require Harbor software, Harbor identity, Harbor tenancy, Harbor leases, or Harbor code spaces.

### 1.3 Standards Boundary

This specification admits only features that define portable wire semantics, validation behaviour, interoperability obligations, conformance levels, or extension contracts. Ecosystem tasks such as engine plugins, UI viewers, orchestration hooks, benchmark dashboards, and language bindings are valuable, but they do not belong in the normative core unless they introduce a stable artifact or reader/writer obligation.

**Rules:**
- COVE-Core and COVE-T MUST remain implementable without a lakehouse catalog, named engine profile, accelerator sidecar, object-temporal engine, or product-specific integration.
- New stable profiles MUST define feature bits, fallback behaviour, failure behaviour, security/privacy impact where relevant, and conformance vectors.
- Optional acceleration and ecosystem integration metadata MUST remain ignorable unless explicitly required by a feature bit or by the requested operation.


### 1.4 V2 Delta and Identity Guardrails

COVE v2 adds a narrow set of next-generation mechanisms that improve performance, implementation ergonomics, and ecosystem integration while preserving COVE's identity as a canonical offline value encoding.

**V2 additions are deliberately scoped:**
- **Registered codec extensions** replace vague specialised-encoding aspirations with a concrete envelope for lossless codecs, feature bits, fallback rules, and test vectors.
- **Layout-plan metadata** gives readers a hierarchical planning surface for lazy object-store reads and scan split generation, but it is not an authoritative data model.
- **Fast metadata indexes and page-cluster directories** improve wide-schema and range-read behaviour, but they mirror validated COVE sections rather than replacing them.
- **Zero-copy buffer maps** allow Arrow/engine-friendly buffer exposure when safe, but they do not weaken null, dictionary, canonical-value, or checksum semantics.
- **Runtime registry/session guidance** gives implementations a clean way to manage codecs, indexes, kernels, functions, FFI adapters, and engine integrations without global state or engine-specific leakage into COVE-Core.

**The following COVE surfaces remain authoritative in v2:**
- the COVE file header, postscript, footer, and binary section directory;
- the table catalog and object catalog;
- canonical logical values and canonical value encodings;
- file-local FileCode dictionaries and ColumnDomain ordering;
- null bitmaps and page reconstruction rules;
- predicate-proof metadata and safe `PredicateZoneOutcome` rules;
- COVE-O object-temporal reconstruction rules;
- COVE-MAP deterministic identity, association, projection, and evidence rules;
- digest manifests, trust chains, redaction manifests, and COVM publication state.

**The following v2 surfaces are explicitly non-authoritative unless a required decode feature bit says otherwise:**
- COVE-L layout-plan nodes;
- scan split indexes;
- page-cluster directories;
- zero-copy buffer maps;
- runtime compatibility hints;
- COVX acceleration indexes;
- engine execution profiles and ExecutionCodes;
- writer cost-model metadata;
- registry/session implementation state.

**Anti-clone boundary:** COVE v2 MUST NOT become a generic layout-tree file format, MUST NOT replace the table catalog with a dtype-only schema model, MUST NOT make runtime registry string IDs the primary compatibility mechanism, MUST NOT require any particular external columnar library, and MUST NOT treat advisory layout/statistics metadata as a substitute for COVE's proof-carrying pruning semantics.

### 1.5 V2 Non-Goals

COVE v2 deliberately does not define:
- a mutable append-in-place file;
- a transaction log or table protocol;
- a mandatory Arrow IPC replacement;
- a mandatory Vortex, Parquet, ORC, Arrow, DuckDB, DataFusion, Harbor, Spark, or Trino dependency;
- a schema-less or dtype-only replacement for COVE-T's table catalog;
- a general plugin system whose unknown runtime identifiers are enough to decode required file data;
- lossy float or string encodings in the core format;
- probabilistic identity resolution as canonical COVE-MAP identity;
- advisory statistics that can skip data without conservative proof.

---


## 2. Normative Language

| Term | Meaning |
| --- | --- |
| MUST | Required for conformance. |
| MUST NOT | Prohibited for conformance. |
| SHOULD | Recommended default; deviations must be deliberate. |
| SHOULD NOT | Not recommended. |
| MAY | Optional. |
| REQUIRED | Same as MUST. |
| OPTIONAL | Same as MAY. |

---


## 3. Purpose

**COVE is an immutable, queryable, encoded archive format designed for:**
- high-performance offline/archive table scans,
- Parquet/ORC/CSV/Arrow-to-COVE conversion,
- object-store-friendly query planning,
- predicate-heavy workloads,
- point lookup and rare-key access,
- metadata-answerable queries,
- engine-local dictionary/code execution,
- Arrow-compatible decoding,
- optional engine-specific execution mappings through COVE-E,
- optional named engine execution registrations such as COVE-H,
- optional object-temporal history through COVE-O,
- optional deterministic multi-source semantic mapping into object-based COVE through COVE-MAP,
- optional registered lossless codec extensions through COVE-CX,
- optional layout/split planning metadata through COVE-L,
- optional explicit runtime registry/session compatibility through COVE-R.
**COVE is not:**
- a WAL,
- a mutable database file,
- an in-flight transaction recovery log,
- a lakehouse catalog replacement,
- a lakehouse/table transaction protocol,
- a row-level delete or visibility protocol,
- an access-control system or encryption standard in v2,
- an Arrow IPC replacement,
- a generic Parquet clone,
- a Vortex clone or wrapper,
- a format whose schema authority is a layout tree or dtype-only model,
- a format that persists engine-local ExecutionCodes as authoritative logical data,
- a mandatory ETL orchestrator, master-data-management system, probabilistic entity-resolution system, or AI-based schema matching system.
**COVE’s guiding principle is:**
Store portable logical values and engine-shaped physical data.
Let each engine own its own execution identity at read or mount time.
Let specialised codecs, coverage proofs, indexes, caches, and layout plans accelerate access without changing portable logical truth.

---


## 4. Public Positioning

**Cove Format should be positioned as:** A Canonical Offline Value Encoding for immutable, queryable archives: encoded arrays, canonical logical values, proof-carrying predicate and coverage metadata, optional accelerator/index sidecars, and direct support for engine-local dictionary execution.

**It should not be positioned as:** A universal Parquet replacement.

**Recommended positioning:**

- **Parquet / ORC:** universal lakehouse interchange and mature analytical columnar storage.
- **COVE:** high-performance queryable archive and converted-table format for engines that can exploit encoded execution, proof-carrying coverage metadata, lookup indexes, aggregate synopses, optional sidecars, direct dictionary/code-vector execution, registered codec extensions, optional secondary indexes, and optional layout/split planning metadata.
- **COVE-MAP:** optional deterministic source-row semantics for organisations that want to convert fragmented source tables, files, and streams into portable object-and-association COVE, and optionally expose that object-association truth through deterministic projected tables, without adopting a named runtime engine.

---


## 5. Profile Overview

| Profile | Name | Audience | Purpose |
| --- | --- | --- | --- |
| COVE-Core | Core Format | All readers/writers | File layout, sections, dictionary, encodings, checksums, validation. |
| COVE-T | Table Scan Profile | General engines | Engine-neutral columnar table scan profile. |
| COVE-COVERAGE | Coverage Semantics Profile | Query planners/archive engines | Conservative query coverage vocabulary, proof strength, tightness/coverage metrics, interval predicate forms, and do-no-harm planning metadata. |
| COVE-A | Archive Acceleration Profile | Archive/query engines | Synopses, lookup indexes, manifests, composite pruning, sidecars. |
| COVE-E | Engine Execution Profile | All engines | Universal mapping from FileCodes to engine-local ExecutionCodes. |
| COVE-H | Harbor Execution Registration | Harbor implementations | Optional Harbor leased-code implementation of COVE-E. |
| COVE-O | Object Temporal Profile | Temporal-object engines | Optional object history, deltas, branches, CSNs, trust chains. |
| COVE-MAP | Semantic Mapping Profile | Conversion/governance/object/projection tools | Optional deterministic multi-source row semantics, identity joins, evidence, materialisation into object-and-association COVE, and deterministic readback as projected tables. |
| COVE-CX | Codec Extension Profile | Reader/writer/engine implementers | Registered lossless specialised encodings with feature bits, fallback rules, capability descriptors, and conformance vectors. |
| COVE-L | Layout Plan Profile | Query planners/object-store readers | Optional hierarchical layout plans, scan splits, page clusters, and zero-copy metadata that never replace catalog/page/index authority. |
| COVE-R | Runtime Registry Guidance | Library and adapter implementers | Explicit session/registry model for codecs, kernels, profiles, engine adapters, FFI, and capability discovery. |
| COVE-I | Secondary Index Profile | Archive/query/index builders | Optional secondary index artifacts, root indexes, value/path/dimensional mappings, and index-only access declarations. |
| COVE-CACHE | Runtime Coverage Cache Profile | Engine/runtime implementers | Optional snapshot-bound mutable predicate coverage cache for local planning reuse; never canonical truth. |

---
