# Validation Model

## 73. Validation Model

### 73.1 Bootstrap Validation

1. Read trailing magic.
2. Read postscript_len and postscript_version.
3. Read postscript.
4. Validate postscript checksum.
5. Validate file_len.
6. Locate footer.
7. Validate footer CRC via postscript section spec.
8. Parse footer and section directory.

### 73.2 Structural Validation

**For every used section:**
- validate offset,
- validate length,
- validate compression,
- validate feature bits,
- validate CRC,
- validate item counts,
- validate internal offsets,
- validate enum ranges,
- validate arithmetic overflow.

### 73.3 COVE-T Semantic Validation

- table IDs unique,
- column IDs unique within table,
- logical/physical pairs valid,
- segment row ranges valid,
- morsel ranges contiguous,
- page row_count matches morsel row_count,
- null_count + non_null_count = row_count,
- FileCodes < dictionary entry_count,
- ColumnDomain ranks valid,
- stats safe before pushdown,
- optional indexes checksum-valid before use.

### 73.4 COVE-E Semantic Validation

- engine profile namespace valid,
- execution descriptor valid,
- scope descriptor valid,
- code-space descriptor valid,
- mount policy valid,
- execution mapping optional or required according to requested operation,
- unknown required profiles rejected only when needed.

### 73.5 COVE-O Semantic Validation

- object_type_id exists,
- property_id exists,
- scope values valid if scope-scoped,
- rows sorted by required order,
- csn/timestamp monotonicity holds,
- prev_ref targets valid rows,
- prev_ref target kind matches,
- reconstruction self-containment holds.

**Delta-aware validation has two modes.**

Snapshot-selection validation checks enough metadata to plan and execute a specific query without opening irrelevant deltas:
- COVM delta-chain extension or equivalent external catalog snapshot;
- ordered chain digest;
- chain-summary CRC and cryptographic digest;
- artifact references;
- required and optional delta feature bits;
- effective schema, object catalog, semantic-map, projection, visibility, and redaction fingerprints;
- summary descriptors needed by the selected operation.

Full delta-chain validation additionally opens every selected delta and proves that chain summaries, touched sets, tombstone sets, anchors, temporal indexes, and temporal rows agree with payloads. Conformance validators and release gates SHOULD provide full delta-chain validation.

**Delta validation requirements:**
- header magic, version, lengths, checksums, and section directory validate;
- postscript/footer consistency and final `CVD2` magic validate;
- parent refs match selected snapshot and digests;
- exactly one parent ref is marked `DELTA_PARENT_REF_LINEAGE_PARENT` and its snapshot ID matches the header parent snapshot ID;
- COVM delta-chain extension binds the exact ordered chain digest;
- COVM chain summary exists, validates by CRC and cryptographic digest, and binds the same ordered chain digest;
- chain summary entries are dense, ordered, and match referenced deltas;
- chain summary time fields preserve commit time, source publication/ingest batch time, snapshot publication time, artifact creation time, and valid-time summary distinctions;
- chain ordinals are dense and ordered;
- artifact IDs are unique within the selected chain;
- schema/catalog/projection fingerprints are inherited or patched validly;
- catalog patches are additive;
- temporal rows are sorted;
- `csn` and commit timestamp monotonicity hold under the append-only profile;
- `prev_ref` is file-local when present;
- scope and branch identity are explicit or covered by a single-scope header invariant;
- continuation anchors target valid logical parent state and meet required anchor strength;
- dictionary aliases resolve through validated parent dictionary digests;
- dictionary aliases do not expose redacted or policy-protected equality unless permitted;
- touched sets do not under-include temporal records;
- tombstone summaries cover all delta tombstones;
- chain-summary pruning summaries do not under-include candidate deltas for the operations they claim to support in full delta-chain validation;
- trust continuation hashes match when required;
- required delta feature bits are supported for the requested operation.

### 73.6 COVE-CX Codec Validation

A COVE-CX-aware validator MUST validate codec descriptors before using registered codec pages.

**Validation requirements:**
- codec IDs unique within the file or artifact;
- namespace/name/version identifies a known exact codec contract or a supported required extension;
- `specification_status` is compatible with the claimed conformance level; candidate/provisional codecs are not required for broad COVE-T conformance;
- feature bits match required/optional codec usage;
- codec envelope row counts match page index row counts;
- params and payload ranges are within the page payload;
- fallback payloads, when present, decode to the same logical sequence as the registered payload;
- float codecs preserve exact declared IEEE semantics;
- string codecs preserve exact UTF-8 or Binary bytes;
- unsupported required codecs reject safely.

### 73.7 COVE-L Layout and Split Validation

A COVE-L-aware validator MUST validate that layout, cluster, split, zero-copy, and fast metadata sections reference existing authoritative metadata.

**Validation requirements:**
- layout node IDs unique;
- parent/child ranges valid and acyclic;
- referenced table, column, segment, morsel, page, section, stats, cluster, and split IDs exist;
- row ranges agree with table segment and morsel metadata;
- page cluster byte ranges do not contradict page index ranges;
- scan split row ranges are contiguous or explicitly declared non-contiguous according to profile rules;
- zero-copy targets do not claim incompatible null polarity, key width, alignment, offset width, endianness, compression state, dictionary semantics, nested layout, visibility overlay compatibility, or lifetime;
- corrupt optional COVE-L sections are ignored.

### 73.8 COVE-R Runtime Compatibility Validation

Runtime compatibility hints MUST be validated only when the requested operation uses them. Unknown optional runtime hints are ignored. Required runtime hints cause rejection only for the runtime operation that requires them. Runtime hints MUST NOT affect baseline logical decode.

### 73.8.1 COVE-COVERAGE Validation

A COVE-COVERAGE-aware validator MUST validate coverage providers, predicate normal forms, coverage sets, and plan candidates before using them for pruning, metadata-only answers, or index routing.

**Validation requirements:**
- provider IDs unique within the file/artifact;
- granularity, proof kind, proof strength, and exactness are known or registered;
- predicate normal forms parse under the declared AST, CNF/DNF, interval, or encoded-predicate grammar and reference declared columns, object paths, dimensions, logical types, collations, and null semantics;
- coverage entries obey per-granularity required fields, absent sentinels, ordering, duplicate, row-range, and row-ordinal-set invariants;
- coverage entries reference existing files, segments, pages, morsels, row ranges, objects, paths, projection fragments, or external fragments;
- snapshot validity matches the selected dataset state, semantic-map version, sidecar versions, and external visibility overlay;
- coverage metrics and costs are not used as proof;
- advisory, approximate-may-under-include, stale, corrupt, or unsupported coverage artifacts are not used for skipping.

### 73.8.2 COVE-I and COVE-CACHE Validation

COVE-I artifacts and COVE-CACHE entries MUST be validated only when the requested operation uses them.

**Validation requirements:**
- referenced file IDs, file lengths, footer CRCs, and digests match the selected files;
- COVE-I postscript, header, section directory, referenced-file table, snapshot validity records, index roots, key blocks, entry blocks, postings blocks, row ranges, row ordinal sets, and aggregate-answer blocks validate before use;
- index root logical types, physical kinds, key encoding, comparators, collations, null semantics, and path/dimension references match the query context;
- sorted keys, duplicate chains, hash-collision policy, postings ordering, row-range coalescing, and row-ordinal bitmap bit order validate;
- index-only capabilities declare exactness and overlay-awareness before answering exact queries;
- cache entries match dataset snapshot, predicate form, semantic-map version, schema fingerprint, and sidecar versions;
- when the selected snapshot is delta-bearing, COVE-I roots, COVX artifacts, coverage providers, and COVE-CACHE entries bind the selected `chain_digest` and effective fingerprints, not only the base file digest or timestamp;
- stale or corrupt indexes/caches fail open to a wider conservative plan or full scan.

### 73.9 COVE-MAP Semantic Validation

A COVE-MAP-aware validator MUST validate the mapping artifact and any embedded mapping sections before using them for conversion, replay, or explanation.

**Validation requirements:**
- source IDs unique within the mapping artifact,
- source schema fingerprints present when source replay is claimed,
- source row identity rules deterministic and non-empty,
- mapping_id and mapping_version present,
- mapping function IDs declared with explicit versions,
- no undeclared random, wall-clock, locale-default, network, or mutable external dependency,
- identity rules reference existing object types and semantic roles,
- multi-column join-key components have declared logical types, canonicalisation, null policy, and ordering,
- resolver-backed join-key components reference an existing `MAP_RESOLUTION_CATALOG` resolver,
- resolver-backed join-key components use `canonicalization = identity` or `none`,
- resolver normalisation pipelines resolve to declared deterministic functions in order,
- `catalog_digest`, `pipeline_digest`, and `resolver_digest` validate under Section 70.1.2 canonical JSON rules,
- `resolver_digest` includes `pipeline_digest`,
- external resolver catalogs, suffix tables, and reviewed-decision inputs are digest-pinned or rejected as not replayable,
- alias catalogs do not map one normalised alias to multiple canonical keys unless ambiguity is declared and routed to candidate-only evidence or rejection,
- unsupported resolver kinds reject before materialisation,
- resolver miss policies are valid and cannot escalate to authoritative merge evidence,
- auto-merge rules use authoritative or deterministic confidence classes only,
- candidate rules do not alter canonical object identity,
- candidate match rules declare deterministic blocking, scoring, rounding, ordering, duplicate suppression, limits, and `merge_behavior = never`,
- do-not-merge constraints are checked before equivalence classes are materialised,
- reviewed `same_object` decisions form merge edges only when the identity rule allows reviewed equivalence,
- reviewed decisions use typed identity references and required canonical anchors,
- reviewed decisions and do-not-merge decisions are conflict-checked before materialisation,
- property conflict rules are declared for multi-source canonical properties,
- association endpoints resolve to deterministic object identities,
- output COVE-O object records satisfy COVE-O validation,
- evidence entries refer to valid source IDs, source row identities or digests, mapping rule IDs, resolver IDs, resolver digests, reviewed decisions, and output assertion IDs.

---
