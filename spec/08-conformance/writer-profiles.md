# Writer Profiles

## 72. Writer Profiles

### 72.1 COVE-Core Minimal Profile

**MUST emit:**
- valid header,
- valid postscript,
- valid footer,
- section directory,
- file dictionary if FileCode columns exist,
- valid checksums,
- valid logical/physical typing,
- valid null bitmaps, unless nullness is fully determined by valid page flags in a COVE-T stats-only constant page.

### 72.2 COVE-T Minimal Table Profile

**MUST emit:**
- all COVE-Core requirements,
- table catalog,
- table segment index,
- table segment data,
- column page indexes,
- page checksums,
- null counts,
- segment/morsel row counts.

This profile MUST NOT require COVE-A, COVE-E, COVE-H, COVE-O, COVX, COVM, or any required custom extension.

#### 72.2.1 COVE-T Starter Interoperability Subset

This subset is an implementation rollout tier, not a reduced COVE-T specification. A first public reader/writer SHOULD target this subset before claiming broader COVE ecosystem readiness, while the full COVE-T standard remains defined by all applicable COVE-T sections:
- COVE-Core plus COVE-T Minimal Table Profile,
- primitive Bool/Int/UInt/Float/Decimal/Date/Timestamp types,
- Utf8/Binary/Uuid through FileCode or VarBytes,
- ordinary List/Struct/Map only when Arrow-compatible mappings are implemented,
- uncompressed and LZ4 payloads,
- valid null bitmaps and all-null/all-non-null page flags,
- morsel_row_count = 4096 unless explicitly declared otherwise,
- morsel-level zone stats for numeric and comparable FileCode columns,
- Arrow-compatible export,
- no required COVE-A, COVE-E, COVE-H, COVE-O, COVX, or COVM dependencies.

Writers producing starter-subset files SHOULD avoid required extensions and exotic encodings. Readers implementing the starter subset MUST still reject unknown required feature bits and MUST remain correct when optional acceleration metadata is absent.

### 72.3 COVE-T Scan Profile

**Recommended default:**
- all COVE-T Minimal requirements,
- FileCode columns for repeated strings/categories,
- NumCode columns for numeric/timestamp data,
- morsel_row_count = 4096,
- ColumnDomain for comparable FileCode columns,
- morsel-level zone stats,
- predicate proof support,
- exact sets for low/medium-cardinality columns,
- bloom filters for high-cardinality equality columns,
- local codebook encoding for FileCode pages,
- frame-of-reference or delta encoding for NumCode pages,
- adaptive per-page encoding selection,
- stats-only constant pages for all-null and all-non-null constant pages where supported,
- small page packing inside table segment data,
- LZ4 for hot scan pages.

### 72.4 COVE-A Archive Acceleration Profile

**Recommended for fast offline archives:**
- all COVE-T Scan Profile features,
- COVM manifest,
- digest manifest,
- FileCode histograms,
- lookup indexes,
- composite zone indexes,
- Top-N summaries for ordered hot columns,
- optional COVX sidecar,
- safe COVM publication using immutable manifests or an external atomic reference update,
- Zstd for cold page payloads where scan latency permits.

### 72.5 COVE-E Engine Execution Profile

**Recommended for engines with dictionary/coded execution:**
- engine profile registry,
- execution code descriptor,
- execution scope descriptor,
- code-space descriptor,
- engine mount policy,
- FileCode -> ExecutionCode mapping strategy,
- optional execution-code cache metadata,
- reverse lookup policy.

### 72.6 COVE-H Harbor Profile

**Recommended for Harbor:**
- all COVE-T Scan Profile features,
- COVE-E engine execution profile,
- FileCode -> Harbor EngineCode mount map,
- Harbor lease epoch tracking,
- Harbor code-space descriptor,
- Harbor mount cache key,
- direct Harbor vector materialisation,
- optional COVE-O object-temporal support.

### 72.7 COVE-O Object Checkpoint Profile

**Recommended for object state:**
- object type catalog,
- temporal segment index,
- self-contained baselines/snapshots,
- FileCode/NumCode property columns,
- temporal blooms,
- trust chain if compliance requires,
- redaction manifest if redactions are present.


### 72.8 COVE-MAP Object Conversion Profile

**Recommended for deterministic multi-source conversion into object-and-association-based COVE:**
- COVE-MAP mapping artifact or embedded mapping sections,
- source catalog with source identity, source kind, schema fingerprint, source load/snapshot identity, and source row identity rules,
- deterministic function registry with function IDs and versions,
- row semantics catalog defining whether rows produce objects, event objects, link objects, associations, composite records, dispatch records, key/value fragments, projections, tombstones, or evidence-only assertions,
- identity rule catalog with authoritative, strong deterministic, weak deterministic, source-scoped, candidate, and do-not-merge rules,
- multi-column semantic join keys for high-confidence cross-source object matching,
- deterministic conflict rules for property values and identity collisions,
- evidence index linking output objects/properties/associations to source rows and mapping rule IDs,
- COVE-O materialisation when the destination is object-based COVE, including materialised link/association object records when associations are produced,
- optional object-association readback metadata for readers that expose associations as a first-class surface,
- optional projection catalog for deterministic object/association-to-table readback,
- optional COVE-T projections for query compatibility,
- optional COVM manifest referencing mapping artifact, source set, conversion report, and output files.

A COVE-MAP writer that claims object-conversion conformance MUST produce COVE-O output that is valid without requiring the mapping artifact for ordinary object reconstruction. If the writer claims association readback, it MUST preserve sufficient metadata for associations/link records to be exposed as associations rather than only as generic objects. The mapping artifact may be required for replay, explanation, conflict audit, projection readback, or source-row traceability.


### 72.9 COVE-CX Registered Codec Profile

**Recommended for writers that want v2 specialised encodings:**
- emit `CODEC_EXTENSION_REGISTRY` with exact codec descriptors;
- set `FEATURE_CODEC_EXTENSION_REGISTRY`;
- set `FEATURE_REGISTERED_ENCODINGS` as required when projected pages need a registered codec and no valid fallback is present;
- provide fallback payloads or reject unsupported readers safely;
- include codec conformance vector references where available;
- preserve exact logical values, null positions, FileCode/NumCode semantics, collation, and trust inputs.

**Recommended first codecs:**
- FSST-style Utf8/VarBytes codec only where FileCode dictionary encoding is not better;
- ALP-style Float32/Float64 NumCode codec only when exact IEEE semantics are preserved;
- FastLanes-style integer/date/timestamp/decimal NumCode codec where frame-of-reference, delta, patched-base, or bit-packing improves scan or storage cost.

### 72.10 COVE-L Layout/Split Planning Profile

**Recommended for object-store and large archive datasets:**
- emit page cluster directory for range-read coalescing;
- emit scan split index for scheduling;
- emit layout plan nodes that reference existing tables, segments, morsels, columns, pages, statistics, and clusters;
- emit fast metadata index for very wide schemas or very large page directories;
- never rely on layout plans as the only schema, page, or predicate-proof authority.

### 72.11 COVE-R Runtime Registry Profile

**Recommended for reference implementations and engine adapters:**
- use explicit sessions/registries for codecs, layout plans, kernels, mapping functions, engine profiles, and FFI adapters;
- expose capability discovery without requiring global mutable state;
- keep runtime compatibility hints optional and rebuildable;
- make engine adapters read-only until COVE-Core/COVE-T/COVE-CX/COVE-L vectors pass.

### 72.12 COVE-COVERAGE Coverage Metadata Profile

**Recommended for writers that expose conservative coverage:**
- emit coverage provider descriptors for proof-carrying stats, indexes, maps, dimensions, or sidecars;
- emit predicate normal forms when a coverage provider depends on a normalised predicate representation;
- declare coverage granularity, proof kind, proof strength, exactness, snapshot validity, collation, logical type context, and null semantics;
- emit coverage degree and tightness degree as planning metrics only;
- emit coverage plan candidates and fallback policy when lookup cost matters;
- never use approximate-may-under-include artifacts for correctness-sensitive pruning.

### 72.13 COVE-I Secondary Index Profile

**Recommended for cross-file and high-selectivity archive workloads:**
- emit `.covi` artifacts with `CVI2` magic;
- reference COVE files by file_id, file length, footer CRC, and digest;
- emit index roots for indexed columns, object paths, associations, projection fragments, or semantic dimensions;
- declare exactness, coverage granularity, null semantics, collation, and index-only capabilities;
- reference `.covi` artifacts from COVM or an external catalog;
- ensure indexes are rebuildable and optional for ordinary reads.

### 72.14 COVE-CACHE Runtime Coverage Cache Profile

**Recommended for engines that repeatedly query the same immutable snapshot:**
- store cache entries outside `.cove` files;
- bind entries to dataset_id, snapshot_id, schema fingerprint, semantic-map version, visibility overlay, and sidecar versions;
- cache predicate normal forms, interval forms, conservative coverage sets, and observed costs;
- invalidate entries on snapshot, manifest, schema, semantic-map, index, sidecar, visibility, or policy changes;
- never treat cache entries as canonical truth.

### 72.15 COVE-QD Query Discovery Profile

**Recommended for writers, catalogs, SDKs, UIs, and agent-facing tooling that expose CoveQL discovery:**
- emit `QUERY_DISCOVERY_MANIFEST` payloads as canonical UTF-8 JCS JSON using schema `cove.query_discovery.v1`;
- set `FEATURE_QUERY_DISCOVERY_METADATA` as optional when embedding query-discovery metadata in ordinary data artifacts;
- never set `FEATURE_QUERY_DISCOVERY_METADATA` in file-level `required_features` for ordinary data artifacts;
- bind manifests to source, schema, dictionary, mapping, policy, principal, audience, and COVM snapshot state;
- use non-self-referential source identity for embedded manifests;
- expose query-safe `query_identifier` values rather than relying on display names;
- use structured template operator chains and typed parameters rather than raw CoveQL string substitution;
- validate examples and templates through parse, resolution, and no-payload planning dry-runs when policy and budget allow;
- keep COVE-QD advisory: generated CoveQL still resolves against canonical metadata and policy.

---
