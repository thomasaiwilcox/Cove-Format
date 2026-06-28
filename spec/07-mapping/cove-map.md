# COVE-MAP Deterministic Semantic Mapping Profile

## 70. COVE-MAP Deterministic Semantic Mapping Profile

COVE-MAP is an optional profile and companion artifact for deterministic semantic mapping from one or more external source datasets into object-and-association-based COVE. In COVE-MAP, objects and associations are a paired semantic model: objects carry identity and properties; associations carry durable meaning between objects. Projected tables are read surfaces over that pair, not a replacement for it.

COVE-MAP is not part of baseline COVE-Core, COVE-T, COVE-A, COVE-E, COVE-H, or COVE-O conformance. A general COVE reader MUST NOT require COVE-MAP support to read ordinary materialised COVE-T or COVE-O files.

COVE-MAP is used when a tool needs to validate, replay, explain, perform source-to-object/association conversion, or expose COVE-O through deterministic table projections.

**Typical flow:**

```text
source tables/files/streams
  + COVE-MAP source catalog
  + source-local row semantics
  + deterministic semantic join keys
  + identity/conflict/provenance rules
  -> paired object-association semantic assertions
  -> COVE-O object-temporal output with materialised association/link records
  -> optional COVE-T/Arrow/SQL projections as read surfaces
  -> optional COVM manifest and evidence indexes
```

**Destination and read-surface rule:**
When the destination is object-based COVE, the materialised output SHOULD be valid COVE-O and SHOULD preserve both objects and associations as the semantic truth surface. Optional table projections MAY be emitted as COVE-T, Arrow record batches, SQL-accessible views, or other table-shaped read surfaces for engines that want relational scans over the object-association output. These projections MUST NOT redefine object identity, association identity, temporal history, or canonical property truth.

### 70.1 Artifact Boundary

COVE-MAP is an optional v2 profile with a stable conceptual and conformance boundary. Artifact identifiers, artifact framing, validation boundary, identity rules, projection/evidence rules, standard `MAP_*` payload schemas, and operation-level fallback/rejection behaviour in this section are normative for v2. Non-normative implementation documents MAY provide examples, generated JSON Schema files, implementation notes, or extension payload schemas, but they are not required to implement the standard COVE-MAP v2 mapping model defined here. Registered required extensions MAY add new section kinds, encodings, functions, or expression operators, but they MUST NOT redefine the standard v2 semantics in this section.

A reusable mapping definition SHOULD be stored in a separate `.covemap` artifact. Embedded `MAP_*` sections inside a `.cove` file are typically file-local evidence, projection catalogs, conversion reports, identity-equivalence indexes, or embedded mapping snapshots tied to that file or dataset state. Unless a required profile or extension explicitly says otherwise, the `.covemap` artifact is the authoritative reusable mapping definition.

**COVEMAP final bytes:**
[postscript bytes]
[postscript_version: u16]
[postscript_len: u16]
[magic: "CMP2"]

`.covemap` uses the same tail-discovery pattern as COVE files. The postscript points to the CovemapHeaderV2 region rather than to a COVE footer.

```rust
struct CovemapPostscriptV2 {
  required_features: u64,
  optional_features: u64,
  file_len: u64,
  header_offset: u64,
  header_length: u64,
  checksum: u32,
}
```

```rust
struct CovemapHeaderV2 {
  magic: [u8; 4],          // "CMP2"

  header_len: u16,
  version_major: u16,
  version_minor: u16,

  flags: u32,

  mapping_id: [u8; 16],

  required_features: u64,
  optional_features: u64,

  section_count: u32,

  mapping_version_len: u16,
  reserved0: u16,

  created_at_us: i64,

  reserved: [u8; 32],

  checksum: u32,
}
// followed by:
//   mapping_version[mapping_version_len]
//   CovemapSectionEntryV2[section_count]
```

```rust
enum CovemapPayloadEncodingV2 {
  CoveMapJsonV2 = 1,       // UTF-8 canonical JSON payload using Section 70 schema
  CoveMapCborV2 = 2,       // deterministic CBOR representation of the same Section 70 schema
  Extension = 255,
}

struct CovemapSectionEntryV2 {
  section_id: u32,         // MAP_* or VENDOR_EXTENSION
  offset: u64,
  length: u64,
  uncompressed_length: u64,
  compression: u8,
  payload_encoding: u8,   // CovemapPayloadEncodingV2
  required: u8,
  reserved: u8,
  checksum: u32,
}
```

**Assigned mapping artifact identifiers:**

| Field | Value |
| --- | --- |
| Artifact magic | `CMP2` |
| Extension | `.covemap` |
| Primary role | Deterministic source-row to semantic-assertion mapping |
| Output role | Produce COVE-O object/association output and optional COVE-T/COVM/COVX/projection artifacts |

A `.covemap` artifact may be referenced by COVM or by output COVE metadata using digest-verified references. COVM may reference mappings for lineage and replay, but COVM MUST NOT be the sole authority for semantic interpretation unless a future required profile defines that behaviour.

COVE-MAP artifacts MUST be immutable for a declared mapping version. A new mapping version may produce different output, but the mapping version, source snapshot/load identity, deterministic functions, and conflict rules must make the difference explainable.

Generated JSON Schema files, reference-tool schema documents, and examples MAY be published for implementer convenience, but they are derived artifacts. The authority for standard COVE-MAP v2 `MAP_*` payload content is this section.

**Rules:**
- `magic` MUST be `CMP2`.
- `mapping_version` identifies the reusable mapping-definition version; a new version MUST produce a new immutable artifact.
- `.covemap` postscript discovery MUST use absolute byte offsets from the start of the artifact. `header_offset` and `header_length` in CovemapPostscriptV2 MUST be within `file_len` and MUST locate the CovemapHeaderV2 region for the artifact version being read.
- `section_id` SHOULD reference `MAP_*` section kinds or `VENDOR_EXTENSION`.
- `offset` and `length` in CovemapSectionEntryV2 are absolute byte offsets from the start of the `.covemap` artifact unless a future required extension defines otherwise.
- `compression` in CovemapSectionEntryV2 uses the Section 66 `CompressionCodec` registry.
- `payload_encoding` MUST identify the encoding of the uncompressed payload bytes. A COVE-MAP v2 artifact validator MUST support `CoveMapJsonV2` for all standard section kinds it claims. `CoveMapCborV2` and `Extension` are optional unless advertised by a required feature bit or claimed profile.
- If `compression` is `None`, `length` MUST equal `uncompressed_length`.
- If `compression` is not `None`, `uncompressed_length` MUST be the exact decoded byte length.
- If `length == 0`, `uncompressed_length` MUST also be zero.
- A `.covemap` artifact MUST be discoverable and integrity-checkable without consulting a COVE data file.
- The artifact framing and standard payload schema defined here are stable for v2. Payload bodies MUST conform to the Section 70 schema for their `section_id`, mapping version, and `payload_encoding`, or to a registered required extension when `section_id` or `payload_encoding` is extension-defined.
- An implementation MAY claim support for a subset of standard COVE-MAP section kinds, but it MUST NOT claim full COVE-MAP artifact validation unless it validates every standard section kind present in a required `.covemap` artifact.

#### 70.1.1 Standard COVE-MAP Payload Schema

COVE-MAP v2 defines a standard logical payload schema for the `MAP_*` section kinds listed in Section 14. This schema is normative in this document. Encodings such as `CoveMapJsonV2` and `CoveMapCborV2` are byte representations of the same logical schema; they are not separate mapping languages.

**Standard section payload model:**

| Section kind | Standard payload body |
| --- | --- |
| `MAP_SOURCE_CATALOG` | Mapping metadata plus an ordered array of source declarations described by Section 70.2. |
| `MAP_FUNCTION_REGISTRY` | Ordered function declarations described by Section 70.13. |
| `MAP_IDENTITY_RULE_CATALOG` | Identity rule declarations, semantic join-key components, confidence classes, merge policy, do-not-merge policy, and tie-breakers described by Section 70.5. |
| `MAP_ROW_SEMANTICS_CATALOG` | Source row semantics, operation semantics, dispatch/composite/key-value rules, temporal roles, and assertion kinds described by Sections 70.3 and 70.4. |
| `MAP_ASSERTION_LOG` | Optional ordered semantic assertion records described by Section 70.4, including deterministic assertion identity, source row identity, rule identity, payload, and conflict/candidate status where applicable. |
| `MAP_IDENTITY_EQUIVALENCE_INDEX` | Deterministic identity-key to GOID/equivalence-set records described by Sections 70.5 through 70.7. |
| `MAP_EVIDENCE_INDEX` | Evidence records described by Section 70.12. |
| `MAP_CONVERSION_REPORT` | Conversion diagnostics, rejected rows, unresolved conflicts, candidate matches, fidelity metrics, and policy outcomes described by Sections 70.8, 70.14, and 70.15. |
| `MAP_PROJECTION_CATALOG` | Object/association projection declarations and expression records described by Section 70.10. |
| `MAP_RESOLUTION_CATALOG` | Resolver catalogs, normalisation pipelines, candidate match rules, and reviewed decisions described by Section 70.5.1, with identity-planning integration in Sections 70.5 and 70.6. |

**Common payload rules:**
- A standard payload MUST identify `schema_id = "org.coveformat.covemap.v2"`, `section_id`, `mapping_id`, and `mapping_version`.
- All field names in standard JSON payloads are lowercase snake_case ASCII names from this specification. Duplicate object keys are invalid. Unknown fields are invalid unless they are nested under an `extensions` object whose namespace is declared by a required extension or under a vendor-specific optional extension that the reader is allowed to ignore.
- `CoveMapJsonV2` payloads MUST be UTF-8 JSON text with a top-level object, no duplicate object keys, no non-finite numbers, and no semantic dependence on object member order. When a writer advertises canonical-byte reproducibility for JSON payloads, object members MUST be emitted in lexicographic order by UTF-8 member-name bytes, insignificant whitespace MUST be omitted, and numbers MUST use the COVE canonical textual form for their logical type.
- `CoveMapCborV2` payloads MUST be a deterministic CBOR representation of the same logical schema: definite lengths, shortest integer encodings, deterministic map ordering, and COVE canonical logical-value semantics for typed values.
- Arrays whose order affects identity, conflict resolution, function dispatch, projection output, or evidence replay MUST be emitted in declared deterministic order and MUST be validated in that order.
- Logical values embedded in COVE-MAP payloads MUST use COVE canonical logical value semantics. Identity keys, hashes, and digests MUST be computed from canonical logical values or from the canonical tuple bytes defined in Section 70.5.
- IDs used for sources, functions, rules, projections, object types, association types, semantic roles, and dimensions MUST be stable within the mapping version. A payload MUST NOT rely on source file order, map-object iteration order, locale defaults, or runtime-generated names for semantic identity.
- A payload that omits a field marked as required by the relevant Section 70 rule is malformed. A payload that uses an undeclared function, source, identity rule, projection, object type, association type, semantic role, or extension namespace is malformed.

#### 70.1.2 COVE-MAP Canonical JSON Digests

Resolver catalogs and resolver evidence use COVE canonical JSON v1 for semantic digests.

**Canonical JSON rules:**
- The digest input is a UTF-8 JSON object payload.
- Duplicate object keys MUST be rejected before digesting.
- Object keys are sorted by bytewise UTF-8 member-name order.
- Arrays preserve declared order unless the schema declares the array semantically unordered.
- Insignificant whitespace is omitted.
- Strings use deterministic JSON escaping.
- Integers and decimal numbers use their parsed canonical JSON textual form.
- Fields explicitly marked `non_semantic_metadata` are excluded from semantic digests.
- All other metadata participates in the digest.

**Resolver digest fields:**

```text
catalog_digest =
  sha256(canonical-json(alias_catalog without catalog_digest fields))

resolver_digest =
  sha256(canonical-json({
    resolver_id,
    kind,
    object_type,
    authority,
    confidence_class,
    normalization_pipeline_id,
    pipeline_digest,
    on_hit,
    on_miss,
    miss_confidence_class,
    ambiguous_policy,
    catalog_digest
  }))

pipeline_digest =
  sha256(canonical-json(normalisation pipeline, referenced table IDs, and
  referenced table digests))
```

`catalog_digest` proves alias data. `resolver_digest` proves resolver behaviour, including pipeline digest, hit/miss policy, ambiguity policy, authority, and catalog digest. Evidence SHOULD carry `resolver_digest` when explaining or replaying resolver behaviour.

**Digest ordering rules:**
- Normalisation pipeline arrays preserve declared order because function order is semantic.
- Alias catalog entries are sorted by `alias_entry_id` by default.
- Aliases within an entry are sorted by normalised alias bytes, then raw alias bytes.
- Candidate outputs are sorted by declared deterministic output order.
- A resolver MAY declare `order_sensitive_catalog: true`; otherwise alias catalog order is not semantic.
- A `resolver_digest` that names a `normalization_pipeline_id` but does not include the resolved `pipeline_digest` is invalid for deterministic replay.

### 70.2 Source Catalog

A COVE-MAP source catalog describes the inputs that a mapping can consume.

**Supported source kinds may include:**
- COVE-T files,
- SQL tables or query snapshots,
- Parquet files,
- ORC files,
- CSV files,
- JSON/NDJSON exports,
- Arrow IPC/Feather data,
- application logs,
- event streams,
- API payload snapshots,
- other structured or semi-structured sources described by extension.

**A source entry SHOULD declare:**
- source_id,
- source_kind,
- source_uri or logical source reference,
- source_owner or producer,
- source_schema or schema fingerprint,
- source_load_id or snapshot identity,
- source_row_identity rule,
- source ordering rule when order-sensitive,
- source timestamp roles,
- source payload digest policy,
- source trust/sensitivity labels where applicable.

**Rules:**
- A mapping that claims replayability MUST identify source inputs by stable snapshot/load identity and digest or equivalent immutable source fingerprint.
- A source row number alone SHOULD NOT be the only source row identity unless it is paired with a source file digest and schema fingerprint.
- SQL/live source mappings MUST specify the snapshot, extraction query, transaction watermark, or export digest needed to reproduce the same rows.

### 70.3 Source-Local Row Semantics

Row semantics define what a source row means before identity resolution.

COVE-MAP row semantics are an engine-neutral inversion of operational row-semantics systems: source rows are interpreted into semantic assertions rather than live engine mutations.

**Row semantics kinds:**

| Kind | Meaning |
| --- | --- |
| Object | Row contributes to one independent destination object. |
| EventObject | Row creates a point-in-time event or transaction object. |
| LinkObject | Row creates a first-class connector object between other objects. |
| AssociationOnly | Row creates an association assertion without a separate object, unless materialised as a link object for COVE-O v2. |
| Composite | Row contributes to multiple objects and associations. |
| Dispatched | A discriminator value selects one of several row semantics rules. |
| KeyValueFragment | Row is an entity-attribute-value or sparse-property fragment. |
| ProjectionOnly | Row is a read-only projection and does not create new semantic truth unless declared. |
| EvidenceOnly | Row provides source evidence for existing objects/properties/associations. |
| Tombstone | Row represents deletion, closure, revocation, or absence according to a declared policy. |

**Rules:**
- A row semantics rule MUST declare the assertion kinds it may produce.
- Object and association assertions are designed to be consumed together. A mapping that declares association output MUST NOT treat those associations merely as optional foreign-key hints; they are durable semantic facts subject to identity, temporal, evidence, governance, and projection rules.
- Composite and dispatched semantics MUST be deterministic for each input row.
- ProjectionOnly rows MUST NOT create canonical object identity unless an explicit identity rule says they do.
- Tombstone semantics MUST declare whether the tombstone applies to an object, property, association, source-local record, or evidence assertion.

#### 70.3.1 Harbor Row Semantics Translation Boundary

COVE-MAP adopts the useful two-axis structure from Harbor Row Semantics in an engine-neutral, offline form.

**Axis 1: core row meaning** describes what the row fundamentally contributes:

| Harbor core semantics | COVE-MAP row semantics | COVE-MAP meaning |
| --- | --- | --- |
| Object | Object | Row contributes to one independent destination object. |
| Transaction | EventObject | Row creates a point-in-time event or transaction object, often with frozen/stamped property values. |
| Link | LinkObject | Row creates a first-class connector object between endpoint objects and may also create association assertions. |
| Association | AssociationOnly | Row creates an association assertion without requiring a separate source object, although COVE-O v2 materialises it as a link/association object when object output is requested. |
| View | ProjectionOnly | Row is a read surface and does not create canonical object truth unless an explicit mapping rule says so. |

**Axis 2: derived meaning wrappers** describe how row meaning is selected or expanded:

| Harbor wrapper | COVE-MAP equivalent | COVE-MAP meaning |
| --- | --- | --- |
| Dispatched | Dispatched | A discriminator value selects one of several deterministic row-semantics rules. |
| Composite | Composite | One source row emits multiple object, property, association, temporal, tombstone, or evidence assertions. |
| KeyValueFragment | KeyValueFragment | Row is an entity-attribute-value or sparse-property fragment. |
| DerivedTable | ProjectionOnly / Projection rule | Row/table is a derived read surface, aggregate, or debug/export projection rather than canonical truth unless explicitly materialised with lineage. |

**Rules:**
- COVE-MAP MUST describe source-row meaning without requiring Harbor runtime mutation semantics.
- COVE-MAP row semantics are applied to immutable source snapshots, source files, source streams, or declared source loads.
- A COVE-MAP converter MAY materialise results as COVE-O object/association history, COVE-T projections, evidence indexes, conversion reports, or future profile outputs.
- Harbor-specific concepts such as SQL DML side effects, Harbor object graph mutation, Harbor tenant state, and Harbor leased codes remain COVE-H or implementation concerns.

#### 70.3.2 Source Operation Semantics

Some source rows describe operational changes rather than complete facts. COVE-MAP supports deterministic operation interpretation without making COVE files mutable.

```rust
enum SourceOperationKind {
    Fact = 0,                 // row asserts a complete or partial fact
    Insert = 1,               // row represents creation in the source
    Upsert = 2,               // row creates or updates according to source identity
    PatchProperty = 3,        // row modifies one or more properties
    ReplaceObjectState = 4,   // row replaces the mapped state for an object or association
    CloseAssociation = 5,     // row ends an association validity interval
    ExpireAndCreate = 6,      // row closes an old association/state and creates a replacement
    TombstoneObject = 7,      // row tombstones an object
    TombstoneProperty = 8,    // row clears/tombstones a property
    TombstoneAssociation = 9, // row tombstones or closes an association
    RedactEvidence = 10,      // row redacts evidence or protected payload
    EvidenceOnly = 11,        // row is retained for provenance but does not alter canonical truth
    Correction = 12,          // row corrects previous source evidence according to declared policy
}
```

**Rules:**
- Source operation semantics MUST be declared by mapping rule, source stream, source table, or row discriminator.
- Operation rows MUST still produce deterministic semantic assertions.
- Operation rows MUST NOT imply in-place mutation of an existing COVE file.
- When materialised as COVE-O, operation rows SHOULD produce deltas, snapshots, baselines, tombstones, association validity changes, or evidence records according to COVE-O rules.
- `PatchProperty` MUST declare null/missing semantics and whether null means unknown, no-op, clear, tombstone, or redacted.
- `ReplaceObjectState` MUST declare whether omitted properties are unchanged, cleared, tombstoned, or unknown.
- `CloseAssociation` and `ExpireAndCreate` MUST declare the temporal axis used: valid time, observed time, source transaction time, mapping execution time, or COVE-O commit/file-ordering time.
- `Correction` MUST declare whether the correction rewrites interpretation for a replayed mapping version, emits a new temporal correction fact, or records conflict evidence only.
- Operation interpretation MUST be replayable from the declared source snapshot/load, mapping version, function versions, and conflict policy.

#### 70.3.3 Stamped and Frozen Value Semantics

A source row may intentionally freeze a value copied or derived from another object at the time the source event occurred. This is common for order/customer, payment/account, admission/patient, and audit/event records.

**A stamped value rule SHOULD declare:**
- destination object or association type;
- stamped property ID/name;
- source or referenced object/property expression;
- temporal role of the stamp;
- whether the stamped value is immutable after creation;
- evidence source and rule ID;
- conflict behaviour if replay finds a different referenced current value.

**Rules:**
- A stamped value is a canonical property assertion of the event/link/association object that receives it.
- A stamped value MUST NOT be silently recomputed from current object state during readback unless the projection rule explicitly requests a derived current-state value.
- A stamped value SHOULD retain evidence identifying the source row and mapping rule that produced the frozen value.
- When materialised as COVE-O, stamped values SHOULD appear as ordinary properties of the event or link object with evidence references, not as hidden projection-only metadata.


### 70.4 Semantic Assertions

COVE-MAP applies row semantics and identity rules to produce semantic assertions.

**Assertion kinds:**
- object assertion,
- property assertion,
- association assertion,
- temporal assertion,
- identity-key assertion,
- identity-equivalence assertion,
- candidate-match assertion,
- tombstone assertion,
- evidence assertion,
- conflict assertion.

A semantic assertion is not necessarily the final COVE-O row. It is the deterministic intermediate meaning produced from source data. A COVE-MAP converter may materialise assertions as COVE-O object records, COVE-O link/association object records, COVE-T projections, evidence indexes, conversion reports, or future association sections.

**Rules:**
- Assertion identity MUST be deterministic for a given source row identity, mapping rule ID, mapping version, and assertion payload.
- Assertion canonical bytes MUST use COVE canonical value encoding for logical values where applicable.
- A materialiser MUST NOT discard conflicts, candidate matches, or rejected rows silently when the mapping claims auditability.

### 70.5 Identity Rules and Multi-Column Semantic Join Keys

Identity rules determine which source rows contribute to the same destination object.

An identity rule may define one or more semantic join keys. A join key is a deterministic ordered tuple of canonicalised components.
A join key tuple is computed per source row or source record. Cross-source matching occurs because different source-specific column bindings map into the same ordered semantic roles, not because values from multiple sources are combined before identity resolution.

**Identity rule classes:**

| Class | May auto-merge? | Typical use |
| --- | --- | --- |
| authoritative | Yes | Source primary key or governed master key. |
| strong_deterministic | Yes, if declared | Exact canonical match on a high-confidence tuple such as email + name, national ID + date of birth, or external ID + issuer. |
| weak_deterministic | Not by default | Name + postcode, phone-only, or other collision-prone deterministic tuples. |
| source_scoped | Only within source scope | Source-local ID with no cross-source merge authority. |
| candidate | No | Suggested possible match retained as evidence. |
| do_not_merge | Prevents merge | Explicit negative match, conflict rule, privacy boundary, or known collision. |

**Multi-column join-key requirements:**
- object_type,
- identity_rule_id,
- key_family or semantic key name,
- confidence_class,
- auto_merge flag,
- component_count,
- declared component order,
- logical type for each component,
- semantic role for each component,
- source column bindings for each source,
- normalisation/canonicalisation function for each component,
- null/missing policy,
- duplicate/collision policy,
- do-not-merge behaviour,
- tie-breaker policy,
- optional `allow_reviewed_equivalence` flag, default false,
- optional resolver-backed `resolution` binding on each join-key component.

**Canonical tuple construction:**

```text
join_key_tuple_bytes =
  version_marker
  || object_type_id
  || identity_rule_id
  || component_count
  || for each component in declared order:
       component_role_id
       logical_type_id
       null_marker or length-prefixed canonical_value_bytes
```

If hashed, the hash input MUST be the canonical tuple bytes. Implementations MUST NOT hash display strings, source bytes, FileCodes, or engine-local ExecutionCodes as a substitute.

Existing non-resolver identity rules remain valid. Resolver-backed identity rules add an optional `resolution` object to a join-key component. A `MapIdentityRule` that supports reviewed equivalence includes `allow_reviewed_equivalence?: bool = false`. A `MapJoinKeyComponent` that supports resolver lookup includes:

```text
role_id, source_column, logical_type, canonicalization, null_policy, ordering,
resolution?: MapResolutionBinding
```

The minimum standard `MapResolutionBinding` contains `resolver_id`. Required extensions MAY add fields, but a validator MUST reject unknown resolver-binding fields unless they are declared by a supported extension.

**Resolver-backed join-key evaluation order:**

```text
raw source value
  -> null policy check
  -> resolver normalisation pipeline
  -> resolver lookup or miss policy
  -> resolved identity value
  -> canonical COVE logical bytes for the join-key tuple
```

For resolver-backed join keys, `canonicalization` MUST be `identity` or `none`; the resolver owns normalisation. Non-resolver join keys keep the existing canonicalisation behaviour.

**Standard resolver hit and miss policies:**
- `on_hit: canonical_key` uses the alias entry's canonical key as the join-key value.
- `on_hit: canonical_label` is invalid for identity keys because labels are not stable keys.
- `on_miss: reject` fails conversion when no resolver match exists.
- `on_miss: normalized_value` uses the normalised value directly and requires `miss_confidence_class` of `strong_deterministic` or `weak_deterministic`; it MUST NOT produce authoritative merge evidence.
- `on_miss: candidate_only` emits candidate/resolution evidence and does not materialise an object row for that identity path.
- `on_miss: source_scoped` produces a source-scoped key that does not merge across sources.

#### 70.5.1 MAP_RESOLUTION_CATALOG

`MAP_RESOLUTION_CATALOG` contains named normalisation pipelines, resolvers, candidate match rules, and reviewed decisions. It is the standard COVE-MAP surface for deterministic alias-based entity resolution and reviewed equivalence.

**Resolver terminology:**
- Observed value: raw source value before resolver normalisation.
- Normalised value: observed value after the resolver's declared normalisation pipeline.
- Canonical key: stable resolver output used for identity keys.
- Canonical label: display value associated with a canonical key; not identity truth by itself.
- Alias: observed or normalised value that maps to a canonical key.
- Resolver: deterministic rule plus catalog and policies that maps observed values to canonical outcomes.
- Candidate: non-authoritative possible match retained as evidence.
- Reviewed equivalence: explicit reviewed same-object decision allowed only when the identity rule declares `allow_reviewed_equivalence`.
- Do-not-merge decision: explicit negative decision that prevents or rejects conflicting merge plans.

**Standard payload members:**
- `normalization_pipelines`: ordered, versioned deterministic function pipelines.
- `resolvers`: resolver declarations. The first standard resolver kind is `alias_catalog`.
- `match_rules`: candidate-generation rules that emit evidence only.
- `reviewed_decisions`: reviewed same-object or do-not-merge decisions.

**Alias catalog resolver rules:**
- `resolver_id` MUST be unique within the mapping.
- `kind` MUST be `alias_catalog` unless a required extension declares another supported resolver kind.
- The resolver MUST declare `object_type`, `authority`, `confidence_class`, `normalization_pipeline_id`, `on_hit`, `on_miss`, `ambiguous_policy`, `catalog_digest`, `pipeline_digest`, and `resolver_digest`.
- `canonical_key` MUST be stable within the resolver catalog version. `canonical_label` MAY change only according to mapping governance and MUST NOT be used as an identity key.
- Alias lookup uses normalised aliases produced by the resolver pipeline.
- One normalised alias MUST NOT map to multiple canonical keys unless the resolver or alias entry explicitly marks it ambiguous and routes it to candidate-only evidence or rejection.
- Ambiguous aliases MUST NOT auto-merge and MUST NOT fall through to `normalized_value` in a way that creates cross-source auto-merge.
- Embedded or external resolver catalogs MUST be digest-pinned. A resolver that references unpinned live external state is not replayable.
- Unsupported resolver kinds, unsupported resolver versions, or unsupported required normalisation functions reject before materialisation.

Candidate match rules live in `MAP_RESOLUTION_CATALOG.match_rules`. They emit candidate evidence and review inputs, not GOID merge edges. Candidate scoring rules MUST declare source-aware inputs, blocking, normalisation pipeline, scoring kind, threshold, score scale, rounding, pair/cluster ordering, duplicate suppression, limits, and limit behaviour. `merge_behavior` MUST be `never`. Unsupported candidate rules MUST emit skipped-rule diagnostics or reject when the mapping requires them. Conformance fixtures SHOULD use `on_limit: fail_closed`.

Reviewed decisions live in `MAP_RESOLUTION_CATALOG.reviewed_decisions`. They use typed identity references, not loose strings. Standard identity reference kinds are `identity_join_key`, `resolver_key`, `source_row`, `row_digest`, and `identity_alias` when it can be resolved to a typed form during validation. Durable `source_row` references SHOULD include source ID, source row identity, source snapshot digest, schema fingerprint, object type, and identity-rule context.

**Reviewed decision rules:**
- `same_object` decisions may form merge edges only when the identity rule declares `allow_reviewed_equivalence: true`.
- `do_not_merge` decisions are hard constraints and MUST reject conflicting merge plans.
- Reviewed decision validation MUST detect conflicts before materialisation.
- Transitive closure MUST be deterministic.
- Same-rule, same-resolver reviewed decisions MAY use the deterministic identity planner's anchor sort when all component keys share one identity rule.
- Cross-rule or cross-resolver reviewed decisions MUST declare `canonical_anchor`.
- A `canonical_anchor` MUST define object type, identity rule ID, role components, logical types, and resolved values used to build the canonical join-key tuple.
- Changing the canonical anchor is a GOID-changing mapping edit and SHOULD be reported by validation, doctor, or explain tooling.

**Effective merge authority:**

| Resolver outcome | Effective merge authority |
| --- | --- |
| authoritative alias hit | `authoritative` |
| strong resolver alias hit | `strong_deterministic` |
| `on_miss: normalized_value` | declared `miss_confidence_class` only |
| `on_miss: source_scoped` | `source_scoped` |
| `on_miss: candidate_only` | `candidate_only` |
| ambiguous alias with candidate policy | `candidate_only` |
| ambiguous alias with reject policy | conversion error |
| candidate match rule output | evidence only |

A rule-level `authoritative` class MUST NOT escalate a weaker resolver outcome to authoritative. Row-level resolver outcome metadata SHOULD distinguish authoritative alias hits, strong deterministic hits, source-scoped misses, normalised-value misses, candidate-only misses, ambiguous aliases, reviewed same-object edges, do-not-merge constraints, and rejected rows.

The following non-resolver example remains valid for baseline multi-column identity rules and for resolver-aware mappings that also contain ordinary deterministic join keys.

**Example: Customer high-confidence match**

```yaml
identity_rules:
  - id: customer.name_email.v2
    object_type: Customer
    class: strong_deterministic
    auto_merge: true
    null_policy: all_components_required
    components:
      - role: Customer.Name
        logical_type: Utf8
        normalise: cove.fn.person_name.v2
        bindings:
          crm.customers: name
          support.tickets: requester_name
      - role: Customer.Email
        logical_type: Utf8
        normalise: cove.fn.email.v2
        bindings:
          crm.customers: email
          orders.orders: customer_email
          support.tickets: requester_email
    do_not_merge:
      - rule: customer.email_marked_shared_or_role_account
      - rule: source_policy_boundary_conflict
```

A CRM row and a Support row whose canonical `Customer.Name` and canonical `Customer.Email` components match create the same strong deterministic join key and may contribute to one `Customer` object. A row with the same name but different email does not match this key. A row with the same email but a do-not-merge marker is kept separate or rejected according to policy.

A single source row may emit more than one identity-key assertion for the same row-semantics object output, for example a governed source ID, an email key, and a name-plus-email key. Those keys are separate evidence items; they become co-referential only under the equivalence rules below.

### 70.6 Identity Resolution Algorithm

A COVE-MAP implementation that claims deterministic identity resolution MUST implement an equivalent deterministic algorithm.

**Recommended abstract algorithm:**
1. For each source row, compute source row identity and source evidence digest.
2. Apply row semantics to produce identity-key, object, property, association, temporal, and evidence assertions.
3. Compute every declared non-resolver join key using declared canonicalisation functions and null policies.
4. For resolver-backed join-key components, apply the resolver evaluation order from Section 70.5, validate resolver/catalog/pipeline digests, and record row-level resolver outcomes.
5. Partition keys by object type and identity rule scope.
6. Add merge edges only for authoritative or strong deterministic keys whose `auto_merge` policy is true and whose resolver outcome permits that authority.
7. Add reviewed same-object edges only when the target identity rule declares `allow_reviewed_equivalence: true` and the reviewed decision validates.
8. Add candidate edges only as candidate-match assertions. Candidate edges MUST NOT participate in GOID selection.
9. Apply do-not-merge constraints before forming final equivalence sets.
10. For each valid equivalence set, choose a canonical identity anchor using declared precedence: identity class, rule precedence, source priority, canonical key bytes, source row identity tie-breakers, and any required reviewed-decision `canonical_anchor`.
11. Generate the destination GOID from the canonical anchor or from a declared external authoritative key.
12. Emit identity-equivalence and evidence records linking all contributing keys, source rows, resolver outcomes, reviewed decisions, and mapping rules.

**Rules:**
- The algorithm MUST produce the same equivalence sets and GOIDs for the same source data, source order declarations, mapping version, function versions, and conflict policy.
- If input row order can affect output, the mapping MUST declare a deterministic row ordering or reject replayability claims.
- Do-not-merge constraints take precedence over auto-merge edges.
- Identity-key assertions emitted for the same source row and the same row-semantics object output MAY declare co-reference. Co-referenced keys participate in the same identity equivalence graph only when their rule classes permit merge and no do-not-merge constraint applies.
- Identity-equivalence indexes SHOULD NOT emit self-equivalence pairs where the left and right identity aliases are identical. The component/member list remains the authoritative compact representation for all contributing rows and keys.
- Candidate matches MUST NOT participate in GOID selection.
- Resolver-backed candidate-only outcomes MUST remain evidence unless a later reviewed decision explicitly authorizes merge under an identity rule that allows reviewed equivalence.
- A resolver-backed row that emits `source_scoped` identity MUST NOT merge across sources through that key.
- Reviewed decisions that bridge identity-rule or resolver families MUST provide a `canonical_anchor`; missing anchors reject before GOID generation.
- A mapping MAY declare that unresolved identity conflicts reject the conversion, keep source-scoped objects separate, or emit conflict evidence. The default safe behaviour is rejection for canonical object output.

### 70.7 GOID Generation for Mapped Objects

COVE-O GOIDs produced by COVE-MAP SHOULD be deterministic within a declared mapping namespace.

**Recommended GOID input:**
- mapping namespace UUID or dataset namespace,
- mapping_id,
- mapping_version or declared identity-stability version,
- object_type_id,
- canonical identity anchor kind,
- canonical identity anchor bytes,
- optional source scope when identity is source-scoped.

**Rules:**
- GOIDs MUST NOT be derived from FileCodes.
- GOIDs SHOULD NOT be derived from non-canonical display strings.
- GOIDs generated from personal data SHOULD use a keyed or governance-approved digest policy when raw key exposure is a concern.
- If a mapping version changes identity precedence or canonicalisation functions, generated GOIDs may change unless an explicit identity-stability policy or alias index is used.
- A converter SHOULD emit an identity-equivalence index when multiple source keys or join keys map to the same GOID.

### 70.8 Property Mapping and Conflict Rules

A row may contribute property assertions to destination objects.

**Property mapping SHOULD declare:**
- destination object type and property ID/name,
- source column binding,
- logical type and conversion policy,
- normalisation or derivation function,
- temporal role if the value is time-qualified,
- source priority,
- null/missing semantics,
- conflict handling,
- evidence retention.

**Conflict behaviours:**
- source priority wins,
- latest observed value wins with deterministic tie-breaker,
- valid-time precedence,
- reject on conflict,
- keep multi-valued property,
- keep source-specific facets,
- canonicalise equivalent values,
- retain non-winning values as evidence.

**Rules:**
- Conflict rules MUST be declared when multiple sources may write the same canonical property.
- Time-based conflict rules MUST declare the temporal axis used and tie-breakers for equal timestamps.
- Null MUST NOT overwrite a non-null value unless the mapping explicitly defines null as clearing, tombstoning, or unknown.
- Non-winning values SHOULD be retained as evidence when auditability is claimed.

### 70.9 Association Mapping

COVE-MAP associations describe durable relationships between destination objects. Associations are first-class semantic outputs paired with objects. They SHOULD be inspectable, explainable, and projectable in the same way as objects.

**Association mapping SHOULD declare:**
- association type,
- endpoint object types,
- endpoint identity rules or aliases,
- direction and cardinality,
- association properties,
- temporal validity fields,
- duplicate handling,
- source evidence,
- materialisation strategy.

For COVE-O v2 destinations, association assertions SHOULD be materialised as link/association object types as described in Section 61.1 unless a future association-specific extension is required. A reader that exposes COVE-O as an object-association surface SHOULD present these materialised records as associations even though their v2 storage form is object records.

**Rules:**
- Association endpoints MUST resolve through deterministic identity resolution.
- Association duplicate handling MUST be deterministic.
- Association validity time MUST NOT be confused with COVE-O commit/file-ordering timestamp.
- Association readback MUST preserve declared direction, endpoint roles, association type, materialised association/link GOID where present, temporal validity, and evidence linkage.

### 70.10 Object-Association Read Surfaces and Projection Rules

COVE-MAP supports two complementary directions:

1. **Source-to-object/association mapping:** external rows become deterministic semantic assertions and are materialised as COVE-O objects, link/association records, temporal facts, and evidence.
2. **Object/association-to-table projection:** existing COVE-O object-association data is exposed as deterministic table-shaped read views for SQL, BI, Arrow, dataframe, debugging, or export workflows.

A projection rule defines a read-time or materialised view over the object-association semantic surface. It does not create a new source of truth unless the projected output is explicitly materialised as COVE-T with lineage back to the COVE-O source and projection definition.

COVE-MAP v2 standard projection expressions use the following normative expression model. Surface syntaxes such as JSON strings, JSON expression objects, or deterministic CBOR expression objects MUST map exactly to this model.

```text
projection_expr =
    path_ref
  | literal
  | function_call
  | aggregate_call
  | association_traversal
  | identity_resolution_ref
  | conditional_expr

path_ref =
  identifier("." identifier)*

identity_resolution_ref =
  "identity(" identity_rule_id ").resolution(" component_role_id ")."
  ("canonical_key" | "canonical_label" | "normalized_value" | "raw_observed_value")

association_traversal =
  "association(" association_type_ref ["," endpoint_role] ")" ["." path_ref]

aggregate_call =
  ("count" | "min" | "max" | "sum" | "avg" | "exists" | "distinct_count")
  "(" [projection_expr] ")"

function_call =
  function_id "(" [projection_expr ("," projection_expr)*] ")"

conditional_expr =
  "if" "(" predicate_ref "," projection_expr "," projection_expr ")"
```

**Projection expression rules:**
- `identifier`, `association_type_ref`, `endpoint_role`, `function_id`, and `predicate_ref` MUST resolve through the mapping artifact's source, object, association, function, predicate, or projection catalogs.
- A path reference MUST resolve to exactly one declared object property, association property, endpoint role, evidence field, temporal role, or projection-local binding.
- An `identity_resolution_ref` MUST identify one identity rule and one resolver-backed join-key role emitted for the current row or projection anchor.
- Resolution expressions MUST fail closed when there is no resolver hit unless the projection or property binding declares an explicit fallback.
- Raw observed labels SHOULD remain evidence even when canonical labels become object properties.
- Function calls MUST reference declared deterministic functions from `MAP_FUNCTION_REGISTRY`.
- Aggregate calls MUST declare their null policy, empty-set policy, cardinality policy, and temporal cut when those policies are not implied by the projection rule.
- Association traversals MUST declare how zero, one, and many matching associations affect the output row: null, empty list, row explosion, aggregation, rejection, or deterministic first/last according to a declared ordering.
- An implementation MAY reject an expression operator it does not support unless the projection is optional and can be ignored for the requested operation.

**Reader surfaces:**

| Surface | Exposes | Required for baseline COVE-O? |
| --- | --- | --- |
| Object surface | Objects, properties, temporal history, GOIDs, tombstones | Yes for COVE-O readers. |
| Association surface | Associations/link records, endpoint roles, direction, cardinality, validity, evidence | Recommended for COVE-MAP-derived COVE-O; required when association readback is claimed. |
| Projection surface | Deterministic rows derived from objects and associations | Optional; required only when mapping-defined projection support is claimed. |
| Evidence surface | Source rows, mapping rules, assertion IDs, conflicts, and provenance | Optional; required only when explanation/audit support is claimed. |

**A projection rule SHOULD declare:**
- projection_id,
- output table or view name,
- output schema,
- row grain,
- anchor object type or association type,
- selected properties,
- association traversals,
- temporal mode or point-in-time cut,
- conflict/value selection policy,
- null and missing-value policy,
- cardinality explosion policy,
- duplicate handling,
- ordering policy,
- evidence inclusion policy,
- whether the projection is read-only, materialised, exportable as COVE-T, or exportable as Arrow/SQL rows.

**Recommended row grains:**

| Row grain | Meaning |
| --- | --- |
| `one_row_per_object` | One row per object of the anchor type. |
| `one_row_per_association` | One row per association of the anchor type. |
| `one_row_per_link_object` | One row per materialised link/association object. |
| `one_row_per_property_version` | One row per historical property value/version. |
| `one_row_per_event_object` | One row per event or transaction object. |
| `one_row_per_object_as_of_time` | One row per object at a declared temporal cut. |
| `one_row_per_evidence_assertion` | One row per source evidence or mapping assertion. |

**Example: object summary projection**

```yaml
projections:
  - id: customer_summary.v2
    output_table: customer_summary
    row_grain: one_row_per_object
    anchor:
      object_type: Customer
    temporal_mode:
      as_of: latest_committed
    columns:
      - name: customer_goid
        value: Customer.goid
      - name: display_name
        value: Customer.display_name
        conflict_policy: canonical_value
      - name: email
        value: Customer.email
        conflict_policy: canonical_value
      - name: order_count
        value: count(association(CustomerPlacedOrder))
      - name: latest_ticket_opened_at
        value: max(association(CustomerOpenedSupportTicket).SupportTicket.opened_at)
```

**Example: association edge projection**

```yaml
projections:
  - id: customer_order_edges.v2
    output_table: customer_order_edges
    row_grain: one_row_per_association
    anchor:
      association_type: CustomerPlacedOrder
    columns:
      - name: customer_goid
        value: association.source_goid
      - name: order_goid
        value: association.target_goid
      - name: association_goid
        value: association.goid
      - name: order_date
        value: Order.order_date
      - name: evidence_source
        value: evidence.source_id
```

**Rules:**
- Projection support is optional. A COVE-O reader MAY expose only the object surface unless it claims association, projection, or evidence readback support.
- A projection rule MUST be deterministic for a given COVE-O dataset state, mapping/projection version, temporal cut, and function registry.
- A projection rule MUST declare how multi-valued associations are handled: explode rows, aggregate, choose deterministic first/last, reject, or emit nested/list values where the target format supports them.
- A projection rule MUST declare whether it uses latest values, full history, valid-time state, observed-time state, or COVE-O commit/file-ordering state.
- A projected table view MUST NOT change object identity, association identity, canonical property truth, tombstone semantics, or evidence lineage.
- If a projected view is materialised as COVE-T, the COVE-T output SHOULD include lineage to the source COVE-O files, COVM dataset state where applicable, projection_id, projection_version, mapping/projection artifact digest, and temporal cut.
- A projection catalog MAY include per-column optimizer lineage. For direct scalar object-property columns this MAY identify `source = "object_property"`, stable object type/property IDs, stable projection table/column IDs, the original expression, `transform = "identity"`, and `filter_pushdown = "projection_covi_prefilter"`. This lineage is a proof surface for optional acceleration only; it MUST NOT redefine projected values.
- A COVE-I projection-column sidecar MAY index materialised projection row ordinals by projection table/column ID. Readers MAY use such sidecars to prefilter candidate projection rows only after validating the sidecar against the COVE-O snapshot and proving the pushed predicate matches the declared lineage. Readers MUST still apply the logical projection and residual predicate semantics to the candidate rows. Missing, stale, unsupported, ambiguous, or non-equivalent projection sidecars MUST fall back to materialised projection readback unless the caller explicitly requested strict accelerated execution.


### 70.10.1 Semantic Dimensions and Object/Dimensional Coverage Maps

COVE-MAP may describe logical dimensions over object, association, nested, or projected data. A semantic dimension is a named logical axis that can be mapped to physical fragments, layout buckets, COVE-I index entries, COVX acceleration structures, or COVE-COVERAGE coverage sets.

```rust
struct SemanticDimensionV2 {
    dimension_id: u32,
    name_ref: u32,
    dimension_kind: u16,       // categorical, integer, decimal, timestamp, spatial, genomic, object_path, association_role, extension
    logical_type: u16,
    collation_id: u16,
    path_ref: u32,
    object_type_id: u32,
    association_type_ref: u32,
    bucket_policy_ref: u32,
    flags: u32,
    checksum: u32,
}

struct DimensionalCoverageLayoutV2 {
    layout_id: u32,
    dimension_count: u16,
    coverage_function_kind: u16, // tuple, range_bucket, z_order, hilbert, semantic_path, extension
    flags: u32,
    dimensions_ref: u32,
    maps_to_granularity: u8,     // file, segment, morsel, page, object, projection fragment, etc.
    complete_coverage: u8,
    tight_when_predicate_matches_layout: u8,
    reserved: u8,
    coverage_provider_ref: u32,
    checksum: u32,
}
```

**Example dimensions:**

```text
semantic_dimension chromosome:
  kind: categorical
  path: /variant/chromosome

semantic_dimension position:
  kind: integer
  path: /variant/position
  bucket_width: 100000
```

**Rules:**
- Semantic dimensions MUST be derived from canonical logical values and declared COVE-MAP functions, not source display bytes or engine-local codes.
- A dimensional coverage layout MUST declare whether it provides complete conservative coverage, tight coverage for matching predicates, or advisory layout hints only.
- A dimensional coverage layout MUST NOT redefine object identity, association identity, temporal truth, or projected-table semantics.
- Dimensional bucket maps may be used for object/dimensional query planning only when their coverage proof and snapshot validity are validated.
- Unknown semantic dimensions MUST be ignored for ordinary object/table reads. Operations requesting dimensional coverage or projection planning MAY reject if required dimensions are unsupported.

### 70.11 Temporal Roles

Source time fields must declare their temporal role.

**Temporal roles:**
- source event time,
- valid-from time,
- valid-to time,
- observed-at time,
- ingested-at time,
- source transaction time,
- mapping execution time,
- COVE-O commit/file-ordering timestamp.

Only a field explicitly mapped to COVE-O commit/file-ordering timestamp may populate COVE-O `timestamp_us`. Other temporal roles must be represented as properties, association validity fields, evidence fields, or future temporal-axis extensions.

### 70.12 Provenance and Evidence

COVE-MAP SHOULD preserve evidence linking output objects, properties, associations, identity decisions, conflicts, and tombstones back to source data.

**Minimum evidence for explainable output SHOULD include:**
- source_id,
- source_kind,
- source schema fingerprint,
- source load/snapshot identity,
- source row identity,
- source row digest or payload digest,
- mapping_id,
- mapping_version,
- mapping rule ID,
- mapping execution ID,
- output assertion ID,
- output object GOID or association/link GOID where materialised.

**Resolver and candidate evidence metadata keys:**

```text
resolution_kind
resolver_id
resolver_digest
catalog_digest
pipeline_digest
normalization_pipeline_id
raw_observed_value
normalized_value
resolved_identity_value
canonical_key
canonical_label
alias_catalog_id
alias_entry_id
alias_hit
alias_miss
alias_ambiguous
miss_policy
candidate_match_id
candidate_score
left_source_id
left_source_row_identity
left_raw_observed_value
left_normalized_value
left_row_digest
right_source_id
right_source_row_identity
right_raw_observed_value
right_normalized_value
right_row_digest
blocking_key
match_rule_id
review_decision_id
redacted_resolution_evidence
```

Authoritative alias-hit evidence SHOULD include source ID, source row identity, rule ID, assertion ID, output object ID, identity rule ID, object type, join-key SHA-256, resolver ID, resolver digest, normalisation pipeline ID, raw observed value, normalised value, canonical key, canonical label, alias catalog ID, alias entry ID, `resolution_kind = "alias_catalog"`, and `alias_hit = true`.

Candidate evidence is pairwise or cluster-based. Candidate entries in `MAP_CONVERSION_REPORT.candidate_matches` and optional evidence metadata SHOULD include left/right source row references, raw values, normalised values, row digests, blocking key, match rule ID, and score. Candidate rows remain evidence only and do not enter GOID merge planning.

**Rules:**
- Evidence entries MUST be deterministic for a given mapping run.
- Evidence visibility MUST respect source governance/redaction policy.
- If evidence cannot be retained because of privacy/security policy, the mapping SHOULD retain a redacted evidence stub with digest and policy reference where allowed.
- Redacted resolver evidence MAY omit raw values, but it MUST preserve enough digest, resolver-hit proof, or policy-approved commitment to support replay and explain for authorized readers.
- Resolver evidence used for deterministic replay MUST carry `resolver_digest` or enough digest-pinned resolver/catalog/pipeline references to recompute the same value.
- `MAP_EVIDENCE_INDEX` defines the logical evidence table. Implementations MAY encode this section as either the expanded COVE-MAP JSON payload or a compact binary payload that is a deterministic, lossless encoding of the same fields.
- Compact evidence encodings MUST preserve logical entry order, mapping identity, source row identity, output assertion/object references, observed source fingerprints, snapshot digests, and operation metadata. Readers that support the compact encoding MUST expose the same expanded logical evidence records as expanded JSON readback.
- A compact evidence encoding MUST be self-identifying and integrity checked. Unknown, stale, corrupt, or unsupported compact evidence payloads MUST fail validation or fall back to a valid expanded representation; they MUST NOT silently change explain, projection, parity, or object readback results.
- Build tools MAY default to compact evidence for generated COVE-O when evidence fan-out would otherwise dominate bundle size, but SHOULD provide an expanded or diagnostic output mode for auditability and interoperability.

### 70.13 Deterministic Function Registry

COVE-MAP may reference deterministic functions for normalisation, canonicalisation, hashing, type coercion, and simple derivation.

**Function declarations SHOULD include:**
- function_id,
- function_version,
- input logical types,
- output logical type,
- null policy,
- Unicode normalisation policy,
- locale/collation policy,
- timezone policy when applicable,
- hash/digest algorithm when applicable,
- deterministic failure behaviour.

**Rules:**
- Functions used for identity MUST be declared and versioned.
- Functions used for identity MUST NOT depend on undeclared locale defaults, mutable external services, random values, network calls, wall-clock time, or implementation-defined ordering.
- A mapper MUST reject conversion if it cannot execute a required identity or property function exactly as declared.
- Entity-resolution pipelines MAY use the recommended primitive functions `collapse_whitespace`, `strip_punctuation`, `strip_legal_suffix`, and `sort_tokens` when those functions are declared, versioned, and deterministic.
- Legal suffix stripping MUST be table-driven when used for identity; the function declaration MUST include a stable `table_id` and suffix-table digest.
- `strip_trading_words` and similar broad semantic removals SHOULD NOT be used for authoritative identity keys unless a curated alias catalog or reviewed decision promotes the result. Without that authority they SHOULD be candidate-only or weak-deterministic.
- Recommended named pipelines such as `company_name_basic.v1` and `company_name_gb.v1` are conventions only; their behaviour is authoritative only when the mapping declares exact function versions and table digests.

### 70.14 Security, Governance, and Privacy

Semantic mapping can combine sources and reveal relationships not obvious in any single source.

**Rules:**
- A mapper MUST NOT silently weaken source access boundaries.
- If mapped output combines sources with different sensitivity labels, the output MUST preserve the most restrictive applicable policy metadata, emit declared governance reconciliation metadata, or reject conversion.
- Evidence indexes, identity-equivalence indexes, dictionaries, join-key digests, and conversion reports may leak sensitive information and must be governed like data.
- Join keys derived from personal or regulated data SHOULD use digest/redaction policies that avoid exposing raw identity components to unauthorised readers.
- Resolver catalogs, alias lists, candidate pairs, reviewed decisions, normalised values, and canonical keys may reveal sensitive relationships and MUST be governed at least as strictly as the source data they derive from.
- Published artifacts MAY use redacted aliases, digest-pinned private resolver catalogs, or evidence policies that prove resolver hits without revealing protected aliases, provided replay/explain claims are scoped to authorized readers or digest verification.
- Do-not-merge decisions may encode sensitive negative knowledge. Tools SHOULD expose governance metadata and redaction controls for reviewed decisions and candidate queues.
- COVE-MAP is not an access-control system. Readers and platforms remain responsible for enforcing policy.

### 70.15 Conversion Tool Contract

A COVE-MAP converter that targets object-and-association-based COVE SHOULD implement the following pipeline:

1. Validate mapping artifact and deterministic function registry.
2. Validate `MAP_RESOLUTION_CATALOG` when any identity rule, projection, evidence replay, or candidate/review surface references a resolver.
3. Validate source snapshots, schema fingerprints, and source digests.
4. Read source rows using declared source row identity and ordering.
5. Apply source-local row semantics.
6. Compute source evidence digests.
7. Evaluate resolver-backed join-key components, including normalisation pipelines, alias lookup, miss policies, ambiguity policy, resolver/catalog/pipeline digest validation, and row-level resolver outcomes.
8. Compute semantic join keys.
9. Resolve deterministic identity, reviewed equivalence, do-not-merge constraints, and GOIDs/equivalence sets.
10. Apply property and association conflict rules.
11. Produce semantic assertions and conversion diagnostics.
12. Materialise COVE-O object records and link/association object records.
13. Validate object-association readback semantics for the materialised output when association readback is claimed.
14. Optionally materialise or register COVE-MAP projection rules for COVE-T/Arrow/SQL relational query engines.
15. Emit evidence indexes and conversion report when auditability is claimed.
16. Optionally emit COVE-I secondary-index sidecars for COVE-O object-property, object-path, association, projection-fragment, or semantic-dimension lookup when the sidecar can be validated against the generated object artifact.
17. Emit COVM manifest references when a dataset has multiple output files or lineage artifacts.
18. Validate the produced COVE outputs independently of the mapping artifact.

`cove map build` is the reference CLI orchestration command for this pipeline. It validates a reusable `.covemap` artifact, reads one or more declared source tables, materialises COVE-O object/association output, optionally materialises COVE-T projections, emits standard optional COVE-I acceleration sidecars when supported, and writes implementation reports plus a bundle manifest for adoption workflows. The `map-build-manifest.json` bundle manifest is not a normative COVM manifest. Implementations that need dataset publication semantics SHOULD emit a separate `.covm` artifact, for example through `cove map build --publish-covm` or `cove map publish --bundle-dir <dir> --out <dataset.covm>`. Generated COVE-I and COVM artifacts are optional companion artifacts: they MUST validate against the generated COVE snapshot before use, stale or corrupt artifacts MUST be ignored unless the caller explicitly requires them, and they MUST NOT change object readback, projection, parity, or validation results.

Reference build tooling MAY use ordinary COVE section compression for generated COVE-O artifacts, including COVE-MAP metadata sections and temporal object sections, provided the file and section feature bits advertise the codec as required by Section 66. Such compression is a physical encoding choice only: it MUST NOT change logical object readback, projection readback, validation, parity, identity, evidence, or COVE-I sidecar semantics. Tooling SHOULD expose a compatibility option equivalent to `--section-compression none` when uncompressed sections are required by a downstream reader.

Reference COVE-MAP tooling MAY place object-property, object-path, association-endpoint, evidence-lookup, and projection-fragment COVE-I roots in a command-owned `indexes/` directory. Query and projection engines MAY discover those roots by bundle convention or explicit sidecar path, but materialised COVE-O/COVE-T readback remains the semantic authority. Where projection descriptors do not expose stable object-property lineage, implementations SHOULD validate and report sidecar readiness while falling back to ordinary projection materialisation instead of guessing a pruning mapping.

**Recommended tools:**
- `cove map validate`,
- `cove map preview`,
- `cove map plan-keys`,
- `cove map convert`,
- `cove map build`,
- `cove map publish`,
- `cove map doctor`,
- `cove map suggest`,
- `cove map parity`,
- `cove map explain`,
- `cove map diff`,
- `cove map project`,
- `cove map test`.

### 70.16 Non-Goals

COVE-MAP v2 deliberately does not define:
- probabilistic entity resolution as canonical identity,
- AI-based automatic mapping as canonical identity,
- a general ETL orchestration system,
- a master-data-management workflow,
- a business glossary standard,
- mutable catalog transactions,
- live database writes,
- a mandatory Harbor dependency,
- treating projected tables as more authoritative than the underlying object-association model,
- silently fuzzy auto-merge as canonical identity,
- live external resolver lookup as required replay state,
- LLM- or AI-produced matches as authoritative identity without declared deterministic review or alias authority,
- assuming global name uniqueness across sources,
- using candidate-match rules to create GOID merge edges.

Future extensions may support candidate suggestions, interactive approval workflows, or external resolver integrations, but such features MUST NOT silently change deterministic object identity in a COVE-MAP output.

---
