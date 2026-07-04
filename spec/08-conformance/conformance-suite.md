# Conformance Requirements and Open Suite

## 78. Conformance Requirements

Conformance levels are cumulative implementation claims, not reductions in specification detail. A narrower conformance claim lets an implementation honestly state what it supports, but it does not make unsupported profiles underspecified or optional in the standards-suite sense.

**A conforming COVE-Core reader MUST:**
- validate header checksum,
- validate postscript,
- validate footer,
- parse section directory,
- reject unknown required features,
- bounds-check every used offset/length/count,
- validate CRCs for every used section,
- validate dictionary FileCode ranges,
- enforce null bitmap semantics,
- interpret NumCodes by declared logical type,
- ignore corrupt optional pushdown metadata,
- avoid unsafe min/max pruning,
- fail closed on structural corruption by default.
**A conforming COVE-T reader MUST additionally:**
- parse table catalog,
- validate segment/morsel/page row counts,
- support FileCode decode to dictionary values,
- support direct FileCode/NumCode scan paths,
- preserve correctness when pushdown metadata is missing,
- implement PredicateZoneOutcome conservatively.
**A conforming COVE-A reader SHOULD additionally:**
- use lookup indexes when valid,
- use aggregate synopses when exact and applicable,
- use COVM for file pruning,
- use COVX when valid and beneficial,
- ignore stale or corrupt acceleration artifacts.
**A conforming COVE-E reader MUST additionally:**
- parse engine profile registry when required,
- validate execution descriptors,
- validate scope and code-space descriptors,
- follow mount policy only when understood,
- ignore unknown optional engine profiles,
- reject unknown required engine profiles only when needed by the requested operation,
- never treat ExecutionCodes as COVE logical truth.
**A conforming COVE-H reader MUST additionally:**
- support FileCode -> Harbor EngineCode mapping,
- respect Harbor lease epoch and code-space policy,
- rebuild stale mount maps,
- never treat on-disk FileCodes as Harbor EngineCodes.
**A conforming COVE-O reader MUST additionally:**
- parse object catalog,
- validate temporal segment ordering,
- validate prev_ref targets,
- enforce reconstruction self-containment,
- verify trust chains when requested and present.
**A COVE-O delta-aware reader MUST additionally:**
- select delta-bearing snapshots only through COVM or an equivalent external catalog snapshot,
- validate `CovmDeltaChainExtensionV1`, ordered chain digest, and required chain summary before using deltas,
- parse and validate `.covedelta` `CVD2` framing, header, footer, section directory, parent refs, and required sections,
- reject unsupported required delta features for the selected operation,
- validate continuation anchors and required state hashes before applying existing-object patches,
- apply sparse patch rows, tombstones, redactions, and Baseline/Snapshot records according to COVE-O temporal semantics,
- use touched sets, tombstone sets, chain summaries, coverage, and indexes only when they are conservative for the selected chain,
- fail closed rather than returning base-only state for a selected delta-bearing snapshot.
**A COVE-CX-aware reader MUST additionally:**
- parse and validate codec extension descriptors when required,
- reject unsupported required registered codecs without valid fallback,
- use fallback payloads only after checksum and semantic validation,
- preserve exact logical values and null positions when decoding registered codecs,
- never use codec capability metadata as predicate proof.
**A COVE-L-aware reader MUST additionally:**
- validate layout nodes, scan splits, page clusters, zero-copy maps, and fast metadata references before use,
- ignore corrupt optional COVE-L sections,
- treat layout plans and split indexes as scheduling metadata, not logical truth,
- derive predicate pruning only from validated COVE proof metadata.
**A COVE-R-aware implementation MUST additionally:**
- keep runtime registries/session state outside COVE logical truth,
- ignore unknown optional runtime hints,
- reject unknown required runtime hints only for operations that explicitly require them.
**A COVE-COVERAGE-aware reader MUST additionally:**
- validate provider descriptors, predicate forms, coverage sets, proof strength, exactness, and snapshot validity before use,
- use only conservative coverage for pruning or index routing,
- fail open to wider coverage or full scan when coverage is unsupported, stale, corrupt, or advisory,
- never use coverage metrics or cost estimates as proof.
**A COVE-I-aware reader MUST additionally:**
- validate `.covi` header, referenced file fingerprints, index roots, capabilities, and snapshot validity before use,
- distinguish exact, approximate, and advisory index-only capabilities,
- apply external visibility overlays before returning rows or exact aggregates,
- ignore stale or corrupt secondary indexes for ordinary reads.
**A COVE-CACHE-aware implementation MUST additionally:**
- keep cache entries outside COVE logical truth,
- bind cache entries to snapshot, schema, mapping, visibility, and sidecar versions,
- invalidate stale entries,
- never use cache entries that may under-include data for pruning.
**A COVE-MAP-aware tool MUST additionally:**
- validate mapping artifacts and embedded mapping sections before use,
- compute identity join keys from canonical logical values,
- apply declared normalisation and canonicalisation function versions,
- preserve declared component order for multi-column join keys,
- keep candidate matches separate from canonical object identity unless explicitly promoted by deterministic mapping rules,
- enforce do-not-merge constraints before automatic object merge,
- materialise object-based destinations as valid COVE-O files when COVE-O output is requested,
- preserve evidence sufficient to explain source row -> object/property/association output when explanation is claimed,
- reject or report unresolved identity/property conflicts according to declared policy,
- never require Harbor for COVE-MAP conversion or COVE-O output.
**A COVE-MAP resolver-aware tool MUST additionally:**
- validate `MAP_RESOLUTION_CATALOG` payloads before resolver-backed conversion, replay, explanation, candidate generation, reviewed-decision handling, or resolver-aware projection,
- implement COVE canonical JSON v1 digesting for `catalog_digest`, `pipeline_digest`, and `resolver_digest`,
- require `resolver_digest` to include `pipeline_digest`,
- support the standard `alias_catalog` resolver kind before claiming resolver MVP support,
- enforce resolver hit/miss policies and ambiguity policy without escalating resolver outcomes,
- keep candidate match rules evidence-only with `merge_behavior = never`,
- validate reviewed same-object and do-not-merge decisions before materialisation,
- require canonical anchors for cross-rule or cross-resolver reviewed equivalences,
- preserve resolver evidence metadata sufficient for replay/explain according to governance policy,
- reject unpinned live external resolver state for deterministic replay.
**A conforming writer MUST:**
- never emit engine execution codes as authoritative logical data,
- write FileCodes densely into the file dictionary,
- emit valid null bitmaps,
- emit valid CRCs,
- publish by durable replace,
- mark optional indexes accurately,
- avoid false sort/min/max/domain claims,
- recompute safe stats during conversion unless source stats are proven compatible,
- mark required extensions and profiles accurately.

---


## 79. Open Conformance Suite

A public interoperability release of COVE SHOULD NOT claim broad v2 readiness without a working reference reader, reference writer, and binary conformance suite. The wire format is defined by this specification, but adoption depends on reproducible test artifacts. An implementation SHOULD NOT claim COVE-Core or COVE-T conformance until it passes the applicable public vectors for that level.

**An open COVE release SHOULD include:**
1. Reference reader.
2. Reference writer.
3. Unified `cove validate` CLI.
4. Unified `cove inspect` and `cove dump` CLI.
5. Unified `cove convert parquet` CLI.
6. Binary conformance vectors.
7. Property-based fuzz tests.
8. Corruption/negative test corpus.
9. Canonicalisation/collation test corpus.
10. Parquet conversion corpus.
11. COVE-MAP multi-source conversion corpus when COVE-MAP tooling is claimed.
12. Benchmark suite.
**Benchmark categories SHOULD include:**
- full numeric scan,
- string/category scan,
- equality filter,
- IN filter,
- range filter,
- point lookup,
- Top-N,
- group-by low-cardinality FileCode column,
- count/min/max metadata-only query,
- object-store cold scan,
- warm mount-cache scan,
- Parquet-to-COVE conversion cost,
- COVE file-size overhead,
- COVX/COVM acceleration impact,
- COVE-O delta-chain read amplification, chain-summary pruning, checkpoint delta benefit, and compaction cost,
- ExecutionCode remap overhead,
- Harbor EngineCode remap overhead,
- COVE-MAP source-to-object conversion cost, resolver-catalog lookup cost, candidate-generation cost, and identity-resolution cost when COVE-MAP tooling is claimed,
- registered codec decode and predicate-kernel cost,
- fallback payload overhead,
- layout-plan and scan-split planning overhead,
- page-cluster range-read coalescing benefit,
- zero-copy export success/fallback rate,
- coverage degree and tightness degree for representative predicates,
- coverage-provider lookup cost versus scan cost,
- COVE-I index lookup and index-only answer latency,
- COVE-CACHE hit/miss/invalidation behaviour.

### 79.1 Minimum Binary Test Vector Contract

**Each public conformance vector SHOULD include:**
- one or more binary .cove/.covx/.covm files,
- a machine-readable expected logical result set or expected validation error,
- expected `cove inspect`/`cove dump` metadata summaries,
- declared conformance level and required feature bits,
- producer version and vector version,
- checksum and digest expectations where applicable.

Negative vectors SHOULD name the expected error class rather than depending on exact implementation wording. Optional-profile vectors MUST state which profile is being tested; COVE-H and COVE-O vectors are optional unless an implementation claims those profiles.

**Conformance vectors SHOULD cover:**
- header/footer/postscript validation,
- dictionary FileCode resolution,
- null bitmap semantics, including bit order, final-byte padding, all-null/all-non-null flags, and Arrow validity inversion,
- NumCode interpretation,
- ColumnDomain ordering,
- predicate proof outcomes,
- exact set pruning,
- bloom false-positive safety,
- aggregate synopsis exactness,
- lookup index row references,
- extension registry fallback,
- engine profile descriptor validation,
- ExecutionCode scope/comparison rules,
- Harbor COVE-H lease mapping,
- temporal prev_ref validation,
- trust hash canonicalisation,
- redaction handling,
- digest verification,
- Arrow interop mapping,
- Arrow IPC conversion boundaries,
- lakehouse/COVM manifest freshness and visibility rules,
- external delete/visibility overlay safety for pruning, lookup, and aggregate synopses,
- row-reference file fingerprint validation,
- conversion fidelity reports and lossy conversion rejection,
- FixedSizeList/vector/tensor extension fallback behaviour,
- approximate COVX index proof-capability restrictions,
- Json opaque semantics and semantic-JSON extension behaviour,
- security/privacy boundary cases including redaction, omitted sensitive indexes, and approximate/private statistics,
- streaming-writer finalisation and partially written file rejection,
- COVE-MAP source catalog validation, deterministic function registry validation, multi-column join-key canonicalisation, candidate-vs-canonical identity separation, do-not-merge enforcement, source evidence traceability, object-and-association-based COVE-O output validation, association readback validation, and projection-rule validation,
- COVE-MAP resolver vectors: valid authoritative alias hit, `on_miss: reject`, `on_miss: candidate_only`, `on_miss: source_scoped`, ambiguous alias rejection, ambiguous alias candidate-only routing, alias-entry reorder preserving `catalog_digest`, changed suffix table changing `pipeline_digest` and `resolver_digest`, resolver digest missing `pipeline_digest` rejection, candidate rule `merge_behavior` rejection, candidate limit fail-closed behaviour, reviewed same-object allowed only with `allow_reviewed_equivalence`, do-not-merge violation rejection, canonical anchor required for cross-rule/cross-resolver review, redacted alias evidence proving a resolver hit without raw alias exposure, and resolver expression fail-closed behaviour,
- COVE-O delta vectors: non-delta-aware COVM reader rejects a delta-bearing snapshot, direct base `.cove` open still succeeds, minimal base plus one delta with one new object, sparse property patch, `SetValue`/`SetNull`/`Clear`/`Redact`/`Tombstone`/omitted-unchanged property semantics, object tombstone hiding parent latest state, association/link update, evidence addition with inherited COVE-MAP fingerprint, additive object catalog patch, invalid parent digest rejection, wrong `parent_snapshot_id` rejection, multiple or missing lineage parent refs rejection, chain reorder rejection by chain digest, missing/corrupt required chain summary rejection, wrong chain-summary digest rejection, source publication range not altering `as_of_csn` or valid-time semantics, multi-scope GOID collision isolation, raw parent FileCode branch key rejection as cross-artifact branch identity, duplicate record ID rejection, missing or weak continuation anchor rejection, state-hash mismatch rejection, touched-set under-inclusion rejection, tombstone-summary under-inclusion rejection, corrupt optional delta-local index fallback, `as_of_csn` cuts before/inside/after delta range, and valid-time pruning only through validated temporal-role summaries,
- COVE-COVERAGE provider validation, predicate normal-form validation, interval predicate canonicalisation, conservative coverage proof validation, coverage/tightness metric reporting, and stale coverage rejection,
- COVE-I secondary index root validation, value-to-fragment lookup, path/dimensional-bucket lookup, exact index-only count/min/max/existence vectors, approximate answer rejection for exact queries, and stale index rejection,
- COVE-CACHE predicate containment, snapshot-bound cache reuse, invalidation triggers, and full-scan fallback,
- COVE-QD query-discovery validation: canonical UTF-8 JCS JSON, duplicate-key rejection, unsafe-number rejection, quoted query identifiers, template-injection rejection, ordinary-read ignore behaviour, stale embedded self-binding rejection, no-auto-fetch URI hints, public diagnostic redaction, and COVE-QD not creating roots absent from canonical metadata,
- feature-scope rejection vectors: unknown header `FileRequired` feature rejects, unknown section `SectionRequired` feature rejects only when used, unknown `OperationRequired` feature rejects only the requested operation, global extended feature-word numbering is honoured in `SECTION_FEATURE_BINDING`, and ordinary COVE-T scan succeeds when unsupported optional COVE-MAP/COVE-I/COVE-L/COVE-R/COVE-QD metadata is present,
- registered codec vectors: unsupported required codec without fallback rejects selected page decode, unsupported codec with valid fallback decodes identically, malformed fallback rejects, and candidate/provisional codecs cannot be required for broad COVE-T conformance,
- coverage false-negative prevention vectors: corrupt, stale, under-inclusive, mismatched-snapshot, mismatched-overlay, and unsupported coverage proofs are ignored or rejected rather than used for skipping,
- COVE-I binary grammar vectors: block-container header validation, `CIK2` key-block validation, local reference-space validation, exact posting payload-layout validation for every v2 representation, sorted-key validation, duplicate-key handling, postings ordering, row-range coalescing, row-ordinal bitset bit order, hash-collision verification, aggregate-answer reference validation, coverage-set reference validation, and stale referenced-file digest rejection,
- predicate grammar vectors: canonical operand-ref validation, mirror-field mismatch rejection, operator arity validation for `LiteralValue`, `ColumnRef`, `Between`, `InSet`, n-ary `And`/`Or`, `Not`, `FunctionCall`, malformed operand ordering, literal-list sorting, and unsupported extension operators evaluating to `Unknown` for pruning,
- zero-copy incompatibility vectors: unknown target/source buffer roles, null-polarity mismatch, offset-width mismatch, dictionary-key mismatch, compressed-buffer mismatch, insufficient lifetime, nested-layout mismatch, and active visibility overlay all force materialised output,
- external visibility overlay vectors for coverage, COVE-I index-only answers, lookup indexes, aggregate synopses, and zero-copy export.

### 79.2 New v2 Surface Hardening Requirements

Before a public release claims broad v2 readiness, every newly introduced v2 surface MUST have both positive and negative vectors at the same mechanical precision as COVE-Core/COVE-T.

**Required hardening vector families:**
- `feature-scope/`: header, section, page, profile, and operation requiredness combinations, including global extended feature-word numbering and invalid local word-bank bindings;
- `coverage/`: predicate AST/CNF/interval payload parsing, canonical operand references, mirror-field validation, operator arity, set algebra, false-negative prevention, and snapshot/overlay invalidation;
- `covi/`: `.covi` postscript/header/section validation, block-container validation, `CIK2` key-block validation, local reference-space validation, key grammar, comparator semantics, exact posting payload layouts, row ordinal sets, index-only answers, and stale file references;
- `codecs/`: descriptor status, exact bitstream versions, fallback equivalence, negative malformed payloads, and unsupported required codecs;
- `zerocopy/`: compatible Arrow-view exports and every mandatory materialisation case;
- `sidecars/`: COVX/COVM/COVE-I digest mismatch, schema mismatch, semantic-map mismatch, and sidecar freshness;
- `visibility/`: delete/visibility overlay interactions with pruning, metadata-only answers, indexes, coverage, and zero-copy.

A profile whose new binary grammar lacks these vectors SHOULD remain provisional or implementation-specific rather than broad-conformance-ready.

---
