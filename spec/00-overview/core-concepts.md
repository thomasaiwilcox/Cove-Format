# Core Concepts and Invariants

## 6. Core Concepts

### 6.1 FileCode

```rust
type FileCode = u32;
```

A FileCode is a dense file-local dictionary code.
FileCode(0) = dictionary entry 0
FileCode(1) = dictionary entry 1
FileCode(2) = dictionary entry 2
**Rules:**
- FileCode is local to exactly one COVE file.
- FileCode equality is meaningful only within the same COVE file.
- FileCode equality across files has no semantic meaning.
- FileCode MUST NOT be interpreted as an engine execution code.
- FileCode MUST NOT be used as canonical trust-chain input.
- FileCode(0) is a valid ordinary code.
- FileCode(0) MUST NOT be treated as null.
**Cross-file equality requires:**
- resolving FileCodes to canonical logical values, or
- mapping FileCodes to engine-local ExecutionCodes under a shared engine policy.

---

### 6.2 ExecutionCode

ExecutionCode is an implementation-local runtime code.
**Examples:**
**DuckDB:**
  dictionary vector code / internal categorical representation

**DataFusion / Arrow:**
  dictionary array key or implementation-local dictionary key

**Polars:**
  categorical code

**Custom engine:**
  symbol ID, interned value ID, dictionary key, catalog code, etc.

**COVE-H example:**
  leased Harbor EngineCode
COVE-Core does not define the meaning of an ExecutionCode.

COVE-E defines the universal mechanism for describing execution-code mappings.

**COVE-H defines Harbor’s implementation:**
FileCode -> Harbor EngineCode

---

### 6.3 Named Engine Example: Harbor EngineCode

This subsection is COVE-H specific. It is not part of COVE-Core, COVE-T, COVE-A, or generic COVE-E conformance.
**A Harbor EngineCode is:**
- Harbor-owned,
- tenant/code-space scoped,
- lease-policy governed,
- mount/import resolved,
- possibly epoch-dependent,
- not authoritative COVE file data.
COVE files MUST NOT persist Harbor EngineCodes as authoritative data.

---

### 6.4 NumCode

```rust
type NumCode = u64;
```

NumCode stores raw fixed-width numeric bits.

**Rules:**
- NumCode is interpreted by the declared logical type.
- NumCode MUST NOT be dictionary-resolved.
- NumCode(0) is an ordinary value.
- NumCode(0) MUST NOT be treated as null.

---

### 6.5 Scope

A scope describes the logical ownership or execution boundary associated with a file, profile, dictionary, or execution-code mapping.
**Examples:**
- tenant,
- account,
- organisation,
- workspace,
- catalog,
- dataset,
- engine-specific scope.
The core header uses producer_scope_id and producer_scope_kind.

**COVE-H example:**
producer_scope_kind = Tenant
producer_scope_id   = Harbor tenant UUID
**For other engines:**
producer_scope_kind = Workspace / Catalog / Dataset / EngineSpecific
producer_scope_id   = implementation-defined stable ID

---

### 6.6 Null

Null is structural.
**Null bitmap convention:**
bit = 1 means null
bit = 0 means non-null
**Rules:**
- FileCode values are never null sentinels.
- NumCode values are never null sentinels.
- Top-level column absence is represented only by the null bitmap.
- Dictionary ValueTag::Null is not a row-null sentinel.
ValueTag::Null is valid for nested canonical values, explicit JSON/list/map nulls, and canonical value representation.

**Bitmap layout:**
- Row i uses bit (i & 7) of byte (i >> 3).
- Bits are numbered least-significant-bit first within each byte.
- Unused high bits in the final byte MUST be zero.
- Implementations SHOULD name this structure null_bitmap or cove_null_bitmap, not validity_bitmap.

**Rationale:**
COVE stores a nullness bitmap rather than an Arrow-style validity bitmap because null is a structural exception and because all-zero freshly allocated bitmap memory represents the common all-non-null case. This convention also allows a null bitmap to be used directly as a null-rejection mask during predicate evaluation. The tradeoff is intentional: Arrow export requires inversion or materialisation of an Arrow validity bitmap, and conformance vectors MUST cover that conversion.

---

### 6.7 Morsel

A morsel is COVE-T’s fundamental scan unit.
**A morsel is the unit of:**
- scheduling,
- predicate bitmap production,
- page pruning,
- late materialisation,
- FileCode -> ExecutionCode remapping,
- vector decode,
- row reference construction.
All columns in a table segment MUST share the same morsel boundaries.

**Default:**
morsel_row_count = 4096

**Motivation:**
A morsel is an execution and pruning grain, not merely a compression block. It is intentionally smaller and more regular than a large table segment so engines can schedule work, build predicate bitmaps, remap FileCodes, and late-materialise selected columns without opening unrelated data. The 4096-row default balances:
- low per-morsel metadata overhead,
- cache-friendly predicate bitmaps and row-selection masks,
- simple row references with u16 row offsets,
- vectorised execution in 1024-row or 2048-row engines,
- practical packing of narrow columns.

**Relationship to other batching concepts:**
- Table segments may contain many morsels.
- Column pages are encoded per column and align to morsel row ranges unless an extension explicitly defines a different safe layout.
- Arrow RecordBatches are an output representation chosen by the reader; a reader MAY expose one morsel per RecordBatch, combine adjacent morsels, or split a morsel for downstream limits, provided logical row order and row-reference semantics are preserved.
- Morsel boundaries are the default unit for zone statistics, exact sets, bloom membership summaries, lookup row references, and predicate proof bitmaps.

**Vector alignment:**
- morsel_row_count SHOULD be a power of two.
- morsel_row_count SHOULD be a whole multiple of any declared execution vector size hint.
- Engine profiles that promise direct engine-vector materialisation MUST declare their execution vector size, or declare that no fixed vector size is assumed.
- Except for the final morsel in a segment, writers SHOULD NOT emit partial execution vectors within a morsel.
- The default 4096-row morsel is intentionally compatible with 2048-row and 1024-row execution vectors.

---


### 6.8 Semantic Object Identity and COVE-MAP Join Keys

COVE-MAP introduces a portable distinction between source-local row identity and semantic object identity.

A **source row identity** identifies a row, record, event, or payload within a declared source snapshot or source load. It is provenance, not object identity.

A **semantic object identity** identifies the destination object that source evidence contributes to. In COVE-MAP, semantic identity is produced only by declared identity rules and deterministic join keys.

A **semantic join key** is an ordered tuple of one or more canonicalised source values used to assert that source rows describe the same destination object. A join-key definition may bind the same semantic roles to columns from different source schemas, but each join key tuple is computed per source row or source record using only that source's declared bindings. Cross-source matching occurs because different source-specific bindings map into the same ordered semantic roles, not because values from multiple sources are combined before identity resolution.

**Example:**

```text
Customer.name_email_key:
  object_type: Customer
  confidence_class: strong_deterministic
  auto_merge: true
  components:
    - semantic_role: Customer.Name
      source_columns:
        crm.customers.name
        support.requester_name
      normalisation: cove.fn.person_name.v2
    - semantic_role: Customer.Email
      source_columns:
        crm.customers.email
        orders.customer_email
        support.requester_email
      normalisation: cove.fn.email.v2
  null_policy: all_components_required
```

Under this rule, a CRM row and a Support row with the same canonical `Customer.Name` and `Customer.Email` values produce the same strong deterministic identity key and may be merged into one `Customer` object. The same name alone would not merge unless a separate rule explicitly allowed it.

**Rules:**
- COVE-MAP join keys MUST be computed from canonical logical values, not FileCodes, source display bytes, locale defaults, or engine-local ExecutionCodes.
- Multi-column join keys MUST preserve the declared component order and MUST use length-delimited canonical component bytes before hashing or comparison.
- A join key that permits automatic object merge MUST declare its object type, component list, normalisation functions, null policy, confidence class, merge policy, and conflict policy.
- A confidence class in COVE-MAP is a declared deterministic rule class, not a probability. It MUST NOT be produced by hidden probabilistic or AI matching unless the mapping labels the result as candidate-only evidence.
- Candidate join keys MAY be emitted as evidence, but candidate keys MUST NOT change canonical object identity unless promoted by an explicit deterministic mapping rule in the declared mapping version.
- If two join keys would merge objects in violation of a declared do-not-merge rule, the mapper MUST apply the declared conflict behaviour: reject, keep separate, or emit conflict evidence.


## 7. Core Invariants

### 7.1 COVE is immutable

COVE files are write-once-read-many.
- No in-place mutation.
- No append mutation in v2.
- No in-file delete overlays.
- No mutable visibility maps.
- No mutable execution-code maps.
- No mutable lease maps.
Compaction, import, export, and conversion produce new COVE files.

**Write finalisation:**
- A writer MAY stream input records into temporary builder state, temporary files, or uncommitted segment buffers.
- A .cove object is valid only after the complete section directory, footer, postscript, and covered checksums have been written and validated.
- COVE v2 does not define partially visible incremental writes, append-in-place, or reader recovery from an unfinished .cove object.
- Streaming or incremental dataset publication MAY be built above COVE using new immutable COVE files plus COVM or an external catalog, but readers MUST NOT infer visibility from partially written COVE data.

Future versions MAY define appendable or streaming containers, but such containers MUST use new magic, feature bits, or profile rules so v2 readers cannot mistake them for immutable v2 COVE files.
See Section 50.4 for the v2 append, streaming, CDC, and compaction boundary when COVE files are used inside a dataset or external table system.

---

### 7.2 COVE is engine-neutral at the core

COVE-Core and COVE-T MUST be readable without Harbor.
**A non-Harbor reader may choose one of two paths:**
**Portable decode path:**
  FileCode -> dictionary value -> normal engine value / Arrow array

**Native execution path:**
  FileCode -> engine-local ExecutionCode -> native vector
COVE-H is Harbor-specific, but it is registered through the universal COVE-E mechanism.

**Specification style rule:**
Generic COVE-Core, COVE-T, COVE-A, and COVE-E text SHOULD avoid Harbor terminology except when contrasting a generic rule with the COVE-H registration. This keeps the portable format boundary clear for non-Harbor implementers.

---

### 7.3 Engine profiles do not define logical truth

**INVARIANT:**
  Engine profiles accelerate or adapt execution; they do not define COVE logical truth.
**A COVE file’s logical values are determined by:**
- COVE-Core,
- file dictionary,
- logical types,
- physical streams,
- encoded arrays,
- validated sections,
- canonical value encoding.
**Engine profiles MAY define how those values are mapped into:**
- engine-local runtime codes,
- native vectors,
- caches,
- mount state,
- dictionary arrays.
Engine profiles MUST NOT be required to recover the logical values of a COVE-T file.

---

### 7.3.1 Semantic mappings do not redefine materialised truth

COVE-MAP definitions describe how external source data is converted into COVE outputs. They do not redefine the logical values already present in a materialised COVE-Core, COVE-T, or COVE-O file.

**Rules:**
- A COVE-T reader MUST NOT need COVE-MAP to decode table values.
- A COVE-O reader MUST NOT need COVE-MAP to reconstruct object records that have already been materialised.
- COVE-MAP may be required for mapping replay, mapping explanation, source-to-object conversion, or audit of source evidence.
- Mapping identity comparisons MUST use canonical logical values and declared mapping functions. They MUST NOT compare FileCodes across files.

---

### 7.4 Pushdown is conservative

A reader MUST NOT skip data unless validated metadata proves no matching row can exist.
**Rules:**
- Missing optional pushdown metadata fails open to scan.
- Corrupt optional pushdown metadata fails open to scan.
- Unknown optional pushdown metadata fails open to scan.
- Structural corruption fails closed by default.
- Bloom filters may produce false positives but MUST NOT produce false negatives.
- Unsafe min/max metadata MUST NOT be used for exclusion.


### 7.4.1 Coverage is conservative

COVE coverage metadata generalises predicate pruning from a single zone decision to an explicit set of fragments that is guaranteed to contain every possible matching value or row for a declared predicate context.

**INVARIANT:**
A coverage set used for correctness-sensitive pruning, routing, index-only answers, metadata-only answers, or lookup narrowing MUST be conservative for the declared snapshot and predicate context.

**Rules:**
- A conservative coverage set MAY contain false positives: fragments that are read even though they contain no matching row.
- A conservative coverage set MUST NOT contain false negatives: fragments outside the set that could contain matching rows.
- A tight coverage set is a conservative coverage set that contains only necessary fragments under the declared proof model.
- A coverage artifact with approximate, advisory, engine-local, stale, or unvalidated proof strength MUST NOT be used to skip data.
- Coverage metadata that is corrupt, unsupported, stale, or mismatched to the selected snapshot MUST fail open to a wider conservative plan or full scan.
- Coverage metadata MUST NOT override structural validation, page reconstruction rules, external visibility overlays, or COVE-MAP/COVE-O semantic truth.

---

### 7.5 Pushdown may prove exclusion or inclusion

**COVE-T pushdown returns:**

```rust
enum PredicateZoneOutcome {
    DefinitelyNo = 0,
    DefinitelyYes = 1,
    Unknown = 2,
}
```

**Meaning:**
**DefinitelyNo:**
  no row in the zone can satisfy the predicate.

**DefinitelyYes:**
  every row in the zone satisfies the predicate.

**Unknown:**
  metadata cannot prove exclusion or inclusion.
**Rules:**
- Readers MAY skip zones with DefinitelyNo.
- Readers MAY skip predicate-column decoding for zones with DefinitelyYes.
- Readers MUST evaluate Unknown zones normally.

---

### 7.6 Extensions must be ignorable or required

**INVARIANT:**
  Extension data must be either ignorable or required.
**Rules:**
- If an extension is optional, readers that do not understand it MUST be able
  to ignore it without changing query results.
- If an extension is required to decode projected data or preserve semantics,
  the file MUST set the corresponding required feature bit.

---

### 7.7 JSON is descriptive only

Binary metadata is authoritative.
**JSON metadata MUST NOT be the sole authority for:**
- section offsets,
- section lengths,
- checksums,
- schema,
- column layout,
- dictionary identity,
- pushdown statistics,
- required features,
- execution-code mappings.


### 7.8 COVE v2 layout and codec additions are subordinate to logical truth

COVE-CX codecs and COVE-L layout plans are performance and implementation mechanisms. They MUST NOT redefine COVE logical values.

**Rules:**
- A registered codec MAY change how a page is encoded; it MUST NOT change the decoded logical sequence, null positions, canonical value bytes, collation semantics, FileCode dictionary meaning, NumCode interpretation, or trust/digest inputs.
- A layout-plan node MAY describe how to group reads, generate scan splits, or traverse page clusters; it MUST NOT replace the table catalog, object catalog, segment indexes, page indexes, or row-reference rules.
- A runtime registry/session MAY decide how to instantiate codecs, kernels, and engine adapters; it MUST NOT become part of COVE logical truth.
- If a registered codec or layout-plan section is corrupt and optional, a reader MUST ignore it and fall back. If the codec is required to decode selected data, the reader MUST reject safely.

### 7.9 Catalog and schema authority

COVE v2 keeps explicit schema authority. A table-shaped COVE file is defined by COVE-T table catalog entries and column IDs, not by a dtype-only tree, runtime layout node, or engine adapter schema.

**Rules:**
- A COVE-T reader MUST resolve table and column identity from the table catalog.
- COVE-L layout nodes MUST reference existing table IDs, column IDs, segments, morsels, pages, or sections.
- A layout node that references a missing or mismatched catalog entry is invalid and MUST NOT be used.
- Engine-facing schemas exported to Arrow, SQL, DataFusion, DuckDB, Polars, Spark, Trino, or another runtime are projections of COVE catalog/schema authority, not replacements for it.

---
