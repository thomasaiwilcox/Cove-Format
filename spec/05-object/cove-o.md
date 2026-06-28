# COVE-O Object Temporal Profile

## 55. COVE-O Object Temporal Profile

COVE-O is an optional object-temporal extension profile. It is not part of baseline COVE-Core/COVE-T/COVE-A/COVE-E conformance.
COVE-O is designed for committed object history workloads and may be implemented by any engine with compatible temporal object semantics. Harbor is one possible implementation, not a dependency.
**COVE-O supports:**
- object type catalog,
- scope identity,
- branch identity,
- GOIDs,
- record UUIDs,
- timestamps,
- CSNs,
- baselines,
- snapshots,
- deltas,
- tombstones,
- prev_ref chains,
- optional trust chains,
- optional temporal blooms,
- optional materialisation of COVE-MAP semantic assertions as object, property, link, association, evidence, or projection records.

---


## 56. COVE-O Object Type Catalog

```rust
struct ObjectTypeCatalogV2 {
    type_count: u32,
    flags: u32,

    types: [ObjectTypeEntryV2],
}
```

```rust
struct ObjectTypeEntryV2 {
    object_type_id: u32,

    type_name_len: u16,
    type_name: [u8],

    flags: u32,

    property_count: u16,

    properties: [PropertyEntryV2],
}
```

```rust
struct PropertyEntryV2 {
    property_id: u32,

    property_name_len: u16,
    property_name: [u8],

    logical_type: u16,
    physical_kind: u8,

    nullable: u8,

    collation_id: u16,

    flags: u32,
}
```

**Object type flags:**

| Bit | Name | Meaning |
| --- | --- | --- |
| 0x0000_0001 | OBJECT_TYPE_FLAG_ENTITY_OBJECT | Type is primarily an entity/object identity surface. |
| 0x0000_0002 | OBJECT_TYPE_FLAG_EVENT_OBJECT | Type is primarily an event/transaction object. |
| 0x0000_0004 | OBJECT_TYPE_FLAG_LINK_OBJECT | Type is a first-class connector/link object. |
| 0x0000_0008 | OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | Type materially represents an association between endpoint objects. |
| 0x0000_0010 | OBJECT_TYPE_FLAG_EVIDENCE_OBJECT | Type primarily carries evidence/provenance materialisation. |
| 0x0000_0020 | OBJECT_TYPE_FLAG_PROJECTION_OBJECT | Type is a materialised projection/read-surface helper rather than canonical object truth. |

**Property flags:**

| Bit | Name | Meaning |
| --- | --- | --- |
| 0x0000_0001 | PROPERTY_FLAG_ASSOCIATION_FROM_GOID | Property is the source/from endpoint GOID. |
| 0x0000_0002 | PROPERTY_FLAG_ASSOCIATION_TO_GOID | Property is the target/to endpoint GOID. |
| 0x0000_0004 | PROPERTY_FLAG_ASSOCIATION_TYPE | Property identifies the association type or role family. |
| 0x0000_0008 | PROPERTY_FLAG_ASSOCIATION_VALID_FROM | Property is the association validity start timestamp. |
| 0x0000_0010 | PROPERTY_FLAG_ASSOCIATION_VALID_TO | Property is the association validity end timestamp. |
| 0x0000_0020 | PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT | Property records observation/materialisation time. |
| 0x0000_0040 | PROPERTY_FLAG_EVIDENCE_REF | Property references evidence/provenance material. |
| 0x0000_0080 | PROPERTY_FLAG_MAPPING_RULE_REF | Property references the mapping rule or projection rule that produced the materialised value. |

**Rules:**
- object_type_id MUST be unique.
- property_id MUST be unique within object_type_id.
- Top-level property declarations MUST NOT use logical Null.
- A writer that claims association readback MUST set OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT or OBJECT_TYPE_FLAG_LINK_OBJECT on every materialised association type and MUST flag association endpoint and semantics properties with the corresponding PROPERTY_FLAG_* bits.
- Readers SHOULD use ObjectTypeEntryV2.flags and PropertyEntryV2.flags, not property names alone, as the authoritative cues for association, evidence, and projection readback. Property names such as `from_goid`, `to_goid`, `association_type`, `source_evidence_id`, and `mapping_rule_id` remain recommended conventions only.
- An object type flagged OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT SHOULD expose exactly one PROPERTY_FLAG_ASSOCIATION_FROM_GOID property and exactly one PROPERTY_FLAG_ASSOCIATION_TO_GOID property unless a required extension defines a multi-endpoint association form.
- OBJECT_TYPE_FLAG_LINK_OBJECT and OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT MAY be set together when a type is both a first-class object and an association carrier. Other combinations that materially change readback semantics SHOULD be documented by the profile or extension that emits them.

---


## 57. COVE-O Temporal Segment Index

```rust
struct TemporalSegmentIndexEntryV2 {
    segment_id: u32,
    object_type_id: u32,

    time_range_start_us: i64,
    time_range_end_us: i64,

    csn_min: u64,
    csn_max: u64,

    row_count: u32,

    delta_count: u32,
    snapshot_count: u32,
    baseline_count: u32,
    tombstone_count: u32,

    min_goid: [u8; 16],
    max_goid: [u8; 16],

    offset: u64,
    length: u64,

    checksum: u32,
}
```

**Rules:**
- min_goid and max_goid are lexical min/max of full 16-byte GOIDs.
- GOIDs MUST NOT be truncated.
- Time ranges use commit/file-ordering timestamp.

---


## 58. COVE-O Temporal Segments

```rust
struct TemporalSegmentHeaderV2 {
    segment_id: u32,
    object_type_id: u32,

    time_range_start_us: i64,
    time_range_end_us: i64,

    csn_min: u64,
    csn_max: u64,

    row_count: u32,
    morsel_count: u32,
    morsel_row_count: u32,

    column_count: u32,

    row_directory_offset: u64,
    column_directory_offset: u64,
    page_index_offset: u64,
    data_offset: u64,

    flags: u32,

    checksum: u32,
}
```

**Rules:**
- A temporal segment contains exactly one object_type_id.
- For scope-scoped COVE-O use, producer_scope_kind and producer_scope_id SHOULD identify the scope that owns the object history.
- COVE-H/COVE-O Harbor deployments commonly use producer_scope_kind = Tenant and producer_scope_id = Harbor tenant UUID.
- Logical scope values MUST equal producer_scope_id when scope-scoped.
- Rows MUST be ordered by:
    (timestamp_us, csn, branch_key, goid, record_id)
- timestamp_us MUST be monotonic with csn inside a segment.
- prev_ref may point to earlier segments within the same file.
- prev_ref MUST NOT point outside the file.

---


## 59. COVE-O System Columns

Every temporal segment has these logical system columns.

| Column | Name | Physical Kind | Meaning |
| --- | --- | --- | --- |
| 0 | scope_id | implicit/fixed bytes | Producer scope UUID if scope-scoped. |
| 1 | branch_key | FileCode or FixedBytes | Logical branch identity. |
| 2 | goid | FixedBytes | 16-byte global object ID. |
| 3 | record_id | FixedBytes | 16-byte record UUID. |
| 4 | timestamp_us | NumCode | Commit/file-ordering timestamp. |
| 5 | csn | NumCode | Commit Sequence Number. |
| 6 | xmin | NumCode | Transaction provenance. |
| 7 | record_kind | u8/RLE | Delta/snapshot/baseline/tombstone. |
| 8 | prev_ref | nullable fixed struct | Previous chain reference. |

**Profile-specific scope interpretation:**
- Generic COVE-O readers treat scope_id as the declared producer/object scope.
- COVE-H/COVE-O Harbor deployments interpret scope_id as Harbor tenant_id.

### 59.1 Record Kind

```rust
enum RecordKind {
    Delta = 0,
    Snapshot = 1,
    ReservedLegacyMaterializedDelta = 2,
    Baseline = 3,
    Tombstone = 4,
}
```

Staging-only placeholders MUST NOT appear in COVE.

### 59.2 Object Record Reference

```rust
struct CoveRecordRefV2 {
    segment_id: u32,
    row_index: u32,
    target_kind: u8,      // 0=delta-like, 1=snapshot/baseline-like
}
```

**Rules:**
- prev_ref is file-local only.
- Readers MUST reject invalid segment_id or row_index.
- Readers MUST reject mismatched target_kind.

---


## 60. COVE-O Reconstruction Self-Containment

COVE-O v2 files MUST be reconstruction self-contained.
**For every represented object chain, the file MUST contain either:**
- the full chain back to the first record, or
- a Baseline/Snapshot sufficient to reconstruct state before dependent Delta records.
If a chain continues from outside the file, the writer MUST emit a Baseline or Snapshot anchor inside the file.
Mandatory cross-file prev_ref is not supported in v2.

A COVE-O delta-bearing dataset snapshot is self-contained at the selected snapshot level, not at the individual delta-artifact level. When COVM or an external catalog selects a base `.cove` plus ordered `.covedelta` chain, reconstruction MUST use only the selected base artifact, selected delta artifacts, selected sidecars, and digest-bound metadata for that snapshot.

**Delta self-containment rules:**
- Ordinary `.cove` files remain directly self-contained as described above.
- `.covedelta` artifacts MUST NOT contain mandatory cross-file `prev_ref` row pointers. Existing-object patches use `DeltaContinuationAnchorV1` logical anchors instead.
- The COVM delta-chain extension or equivalent external snapshot metadata is authoritative for the ordered parent chain. A delta header parent reference validates lineage, but it does not authorize discovery of additional deltas outside the selected snapshot.
- A delta may inherit effective schema, object catalog, semantic-map, projection, visibility, or redaction metadata by validated fingerprint from its parent. If a required inherited surface cannot be validated, the selected snapshot is invalid for operations that need it.
- A delta-aware reader MAY reconstruct only the objects and properties needed by a query, but it MUST behave as if applying the selected base and ordered deltas under the validation and visibility rules of this specification.

---


## 61. COVE-O Property Columns

Object property columns use the same physical and encoded-array machinery as COVE-T.
**Property values may be:**
**FileCode:**
  file-local dictionary value

**NumCode:**
  raw fixed-width numeric bit pattern

**FixedBytes / VarBytes / nested:**
  special or unsupported cases
**Rules:**
- Nulls are represented only by null bitmaps.
- FileCodes resolve through the file dictionary.
- NumCodes are interpreted by declared logical type.
- Stats-only property pages follow the same constant reconstruction rules as COVE-T pages in §27.2. All-non-null stats-only property pages MUST reference validated contextual page stats; readers MUST reject them if the stats entry is missing or does not match the property page metadata.
- Property columns SHOULD be page/morsel aligned with system columns.

---


### 61.1 COVE-MAP Association and Evidence Materialisation

COVE-O v2 does not require a dedicated native edge section. When COVE-MAP produces association assertions and the destination is object-based COVE, a writer MUST materialise those associations using declared COVE-O object types unless a future association-capable COVE-O extension is explicitly required.

**Recommended object-type pattern:**

```text
Object type: CustomerPlacedOrder
Required properties:
  association_type        Utf8 or registered enum
  from_goid              FixedBytes(16)
  to_goid                FixedBytes(16)
  valid_from_us          nullable Timestamp
  valid_to_us            nullable Timestamp
  observed_at_us         nullable Timestamp
  source_evidence_id     nullable FixedBytes or Utf8
  mapping_rule_id        nullable Utf8
```

The property names above are recommended conventions, not the only interoperable spelling. When association readback is claimed, ObjectTypeEntryV2.flags and PropertyEntryV2.flags are authoritative for identifying association objects, endpoint properties, validity fields, evidence references, and mapping-rule references.

Link objects such as `OrderLine`, `Membership`, `CustomerAddress`, or `AccountManagerAssignment` MAY carry additional properties and MAY create multiple association-like references through `from_goid`, `to_goid`, or named endpoint properties.

**Rules:**
- Association materialisation MUST be declared in the COVE-MAP row semantics or output profile.
- Association endpoint GOIDs MUST be produced by the same deterministic identity-resolution run as the objects they connect.
- Evidence fields SHOULD point to MAP_EVIDENCE_INDEX entries or to declared source row digests when explanation is required.
- A COVE-O reader that does not understand COVE-MAP may still read the materialised association/link objects as ordinary object records.

---


## 62. COVE-O Temporal Bloom Index

Temporal bloom filters are optional accelerators.
**They answer:**
Can this segment or time bucket contain rows for this scope/branch/goid?
**Recommended bloom key:**
hash(scope_id, branch_key_canonical_value, goid, time_bucket)
**Rules:**
- Single-scope or single-branch files MAY omit scope or branch only if declared.
- Bloom filters may produce false positives.
- Bloom filters MUST NOT produce false negatives.
- Corrupt or missing blooms MUST be ignored.

---


## 63. COVE-O Trust Chain

Trust chains are optional and gated by FEATURE_TRUST_CHAIN.
**Trust columns:**

| Name | Type | Meaning |
| --- | --- | --- |
| trust_hash | nullable [u8; 32] | Hash of canonical delta content. |
| prev_trust_hash | nullable [u8; 32] | Previous trust hash. |
| state_hash | nullable [u8; 32] | Hash of materialised state for baseline/snapshot. |

**Rules:**
- Trust hashes MUST be computed over canonical logical values, not FileCodes.
- Equivalent logical files with different FileCode assignments SHOULD verify to the same logical trust state.
- CRC32C is not a substitute for trust hashes.
**Recommended trust input:**
- scope_id if scope-scoped,
- branch canonical identity,
- object_type_id,
- goid,
- record_id,
- timestamp_us,
- csn,
- record_kind,
- property_id,
- property logical type,
- canonical property value bytes,
- previous trust hash where applicable.

---

### 63.1 COVE-O Delta Artifact Profile

COVE-O delta artifacts are immutable `.covedelta` files selected by COVM or by an external catalog snapshot. They represent object-temporal changes after a self-contained base `.cove` snapshot without mutating the base file. The first interoperable profile is append-only in COVE-O commit order: historical commit-order insertion requires `DELTA_FEATURE_HISTORICAL_COMMIT_INSERT` and is deferred.

#### 63.1.1 Binary Envelope

A `.covedelta` artifact uses COVE-style tail discovery:

```text
[header bytes]
[section payload bytes]
[section directory bytes]
[footer bytes]
[postscript bytes]
[postscript_version: u16]
[postscript_len: u16]
[magic: "CVD2"]
```

```rust
struct CoveDeltaPostscriptV1 {
    required_delta_features: u64,
    optional_delta_features: u64,
    file_len: u64,
    footer_offset: u64,
    footer_length: u64,
    checksum: u32,
}

struct CoveDeltaFooterV1 {
    header_offset: u64,
    header_length: u64,
    section_directory_offset: u64,
    section_directory_length: u64,
    section_count: u32,
    parent_ref_count: u32,
    footer_crc32c: u32,
    checksum: u32,
}

struct CoveDeltaSectionDirectoryEntryV1 {
    section_id: u32,
    section_kind: u16,
    flags: u16,
    offset: u64,
    length: u64,
    uncompressed_length: u64,
    item_count: u64,
    compression: u8,
    encryption: u8,             // 0=None in v1
    alignment_log2: u8,
    reserved0: u8,
    required_delta_features: u64,
    optional_delta_features: u64,
    crc32c: u32,
    checksum: u32,
}
```

**Rules:**
- Delta envelope fields use the same binary discipline as COVE v2 unless this section says otherwise: little-endian integers, explicit lengths and offsets, no native struct padding, and checksums computed with the checksum field treated as zero.
- The final magic MUST be `CVD2`.
- `postscript_len` locates `CoveDeltaPostscriptV1`; the postscript locates the footer; the footer locates the header and section directory.
- Section offsets and lengths are relative to the start of the `.covedelta` artifact.
- Readers MUST validate the postscript, footer, header, section directory, and every section needed by the requested operation before using those bytes.
- `encryption` MUST be `0` in v1 unless a required extension defines an encrypted delta payload profile.
- Unknown required delta features reject according to Section 11.1.

#### 63.1.2 Delta Header

```rust
struct CoveDeltaHeaderV1 {
    magic: [u8; 4],              // "CVD2"
    version_major: u16,
    version_minor: u16,
    header_len: u16,
    flags: u32,
    required_delta_features: u64,
    optional_delta_features: u64,

    delta_artifact_id: [u8; 16],
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    parent_snapshot_id: [u8; 16],

    chain_ordinal: u32,
    chain_depth: u32,
    parent_ref_count: u32,
    section_count: u32,

    csn_min: u64,
    csn_max: u64,
    commit_time_range_start_us: i64,
    commit_time_range_end_us: i64,

    scope_kind: u16,             // valid when DELTA_FLAG_SINGLE_SCOPE is set
    reserved0: u16,
    scope_id: [u8; 16],          // zero when multi-scope

    object_catalog_fingerprint_ref: u32,
    schema_fingerprint_ref: u32,
    semantic_map_fingerprint_ref: u32,
    projection_fingerprint_ref: u32,

    section_directory_offset: u64,
    section_directory_length: u64,
    parent_refs_offset: u64,
    parent_refs_length: u64,

    created_at_us: i64,
    source_publish_range_start_us: i64,
    source_publish_range_end_us: i64,
    checksum: u32,
}
```

Header flag `0x0000_0001` is `DELTA_FLAG_SINGLE_SCOPE`. Header flag `0x0000_0002` is `DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT`.

**Rules:**
- `magic` MUST be `CVD2`.
- `chain_ordinal` MUST be dense within the selected snapshot chain.
- `chain_depth` includes the current delta and MAY be used for read-amplification and retention policy limits.
- `csn_min..=csn_max` MUST advance beyond the selected parent high-water mark for the same scope and branch identity in the initial append-only profile.
- COVE-O `timestamp_us` remains commit/file-ordering time. Business, effective, source publication, or valid-time corrections MUST use declared temporal-role properties and summaries, not backdated commit timestamps.
- If `DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT` is clear, `source_publish_range_start_us` and `source_publish_range_end_us` MUST be zero and ignored.
- `source_publish_range_start_us/end_us` is operational metadata for producer publication, ingest, or update-batch ranges. It is not COVE-O commit time and not business valid time.
- Fingerprint fields describe the effective metadata after applying this delta. A zero fingerprint reference means the surface is unchanged from the parent and inherited by validated parent reference.
- If `DELTA_FLAG_SINGLE_SCOPE` is set, every temporal record, anchor, touched set, and tombstone summary in the delta belongs to `scope_kind/scope_id`. Otherwise those structures MUST carry explicit scope fields.

#### 63.1.3 Parent References

```rust
struct DeltaParentRefV1 {
    parent_ref: u32,
    parent_kind: u8,            // base_cove, parent_delta, covi, covmap, other
    flags: u32,

    artifact_id: [u8; 16],
    snapshot_id: [u8; 16],
    file_len: u64,
    footer_crc32c: u32,
    digest_algorithm: u16,
    digest_len: u16,
    digest_ref: u32,
    uri_ref: u32,

    schema_fingerprint_ref: u32,
    object_catalog_fingerprint_ref: u32,
    semantic_map_fingerprint_ref: u32,
    projection_fingerprint_ref: u32,

    checksum: u32,
}
```

Parent flag `0x0000_0001` is `DELTA_PARENT_REF_LINEAGE_PARENT`.

**Rules:**
- Exactly one parent reference in a `.covedelta` MUST be marked `DELTA_PARENT_REF_LINEAGE_PARENT`.
- The lineage parent's `snapshot_id` MUST equal `CoveDeltaHeaderV1.parent_snapshot_id`.
- Parent references MUST validate length, footer CRC, cryptographic digest, artifact ID, and snapshot ID before inherited metadata or value bytes are used.
- `uri_ref` is advisory location metadata. It MUST NOT replace COVM or external catalog snapshot selection.
- Merge DAG lineage is not part of v1. Multiple non-lineage parent refs may describe sidecars or mapping artifacts, but they do not define multiple lineage parents unless a later required extension does so.

#### 63.1.4 Descriptor Tables

Delta-local `*_ref` fields resolve through explicit descriptor tables, not through implicit offsets or ad hoc ordering. Unless a section says otherwise, descriptor refs are dense zero-based indexes into the matching descriptor table in the same artifact or validated COVM chain-summary payload.

```rust
struct DeltaScopeDescriptorV1 {
    scope_ref: u32,
    scope_kind: u16,
    flags: u16,
    scope_id: [u8; 16],
    checksum: u32,
}

enum DeltaSummaryDescriptorKindV1 {
    ExactSortedSet = 0,
    ExactRangeSet = 1,
    ConservativeRange = 2,
    NoFalseNegativeBloom = 3,
    PropertyBitmap = 4,
    TemporalRoleRange = 5,
    Extension = 255,
}

struct DeltaSummaryDescriptorV1 {
    summary_ref: u32,
    summary_kind: u8,
    flags: u32,
    payload_ref: u32,
    item_count: u64,
    checksum: u32,
}

enum DeltaStateHashKindV1 {
    CoveObjectDeltaStateHashV1 = 0,
    CoveOTrustHash = 1,
    Extension = 255,
}

struct DeltaStateHashDescriptorV1 {
    state_hash_ref: u32,
    state_hash_kind: u8,
    hash_algorithm: u16,
    hash_len: u16,
    hash_payload_ref: u32,
    flags: u32,
    checksum: u32,
}
```

`branch_identity_ref` resolves through `DELTA_BRANCH_IDENTITY_TABLE`; `scope_summary_ref` resolves through `DELTA_SCOPE_TABLE` or a summary descriptor naming one or more scopes; `temporal_role_summary_ref`, `touched_summary_ref`, `tombstone_summary_ref`, and `predecessor_state_hash_ref` resolve through their named delta tables. Unsupported required descriptor kinds reject the operation that needs them.

#### 63.1.5 Catalog Patches and Dictionary Overlays

Catalog patches are inherited by fingerprint and MUST be additive. Allowed patch operations include new object types, new properties on existing object types, new association/link/evidence/projection object types, new temporal-role bindings, new branch aliases, and new projection definitions that depend only on declared object/property IDs.

```text
EffectiveCatalog(delta_n) =
  ApplyAdditivePatch(EffectiveCatalog(parent), delta_n.catalog_patch)
```

Readers MUST reject duplicate object type IDs, duplicate property IDs within one object type, changed logical type or collation for an inherited property, changed association endpoint flags for an inherited property, or changed projection authority for an inherited projection. Catalog patches MUST NOT rename, remove, or reinterpret parent declarations. Breaking catalog changes require a new base `.cove` snapshot or a separate schema-generation branch.

COVE FileCodes remain artifact-local. A delta may avoid duplicating value bytes by making local dictionary entries aliases to validated parent dictionary entries, but the first profile requires inline delta dictionary values for ordinary reconstruction.

```rust
enum DeltaDictionaryEntryKindV1 {
    InlineValue = 0,
    ParentDictionaryAlias = 1,
    CanonicalHashHint = 2,
}

struct DeltaDictionaryEntryV1 {
    local_dictionary_id: u32,
    local_code: u32,
    logical_type: u16,
    collation_id: u16,
    entry_kind: u8,
    flags: u32,

    inline_value_ref: u32,       // when InlineValue

    parent_ref: u32,             // when ParentDictionaryAlias
    parent_dictionary_id: u32,
    parent_code: u32,
    parent_dictionary_digest_ref: u32,

    canonical_hash128: [u8; 16],
    checksum: u32,
}
```

**Dictionary rules:**
- Delta encoded pages use delta-local FileCodes.
- A local code may resolve to an inline value or to a validated parent alias.
- Parent aliases are encoding optimizations, not cross-file code domains.
- A parent alias is valid only if the parent artifact and parent dictionary digest match the selected snapshot and the alias includes logical type and collation context.
- V1 parent aliases MUST resolve directly to a parent inline dictionary value or ordinary parent COVE dictionary value. Alias-to-alias recursion is prohibited unless a later required extension defines bounded recursive resolution.
- `CanonicalHashHint` supports pruning or equality hints only. It is not sufficient to reconstruct a materialised output value unless canonical value bytes are recoverable from a validated parent or inline source.
- A delta MUST NOT alias, hash, or expose equality hints for a parent value that is redacted or policy-protected in the selected snapshot unless the selected disclosure policy explicitly permits that equality leakage.

#### 63.1.6 Branch Identity, Temporal Records, and Patch Operations

Raw FileCodes cannot cross artifact boundaries. Delta metadata that names a branch MUST use canonical branch identity metadata.

```rust
struct DeltaBranchIdentityV1 {
    branch_identity_ref: u32,
    branch_identity_kind: u8,     // canonical_bytes, catalog_branch_id, hash
    flags: u32,
    branch_value_ref: u32,
    branch_hash128: [u8; 16],
    branch_catalog_fingerprint_ref: u32,
    checksum: u32,
}
```

Delta temporal pages may encode branch values in their local physical representation. Cross-artifact anchors, touched sets, tombstone sets, and summaries MUST use `branch_identity_ref`, not a raw FileCode.

Delta records follow COVE-O temporal semantics. The first profile applies records in selected delta-chain order and COVE-O commit order. Within each delta, temporal rows MUST be sorted by the same ordering required for COVE-O temporal segment data. A delta whose records do not advance beyond the selected parent high-water mark for the same scope and branch identity is invalid unless `DELTA_FEATURE_HISTORICAL_COMMIT_INSERT` is required and supported.

For each sparse patch row:

```text
record key:
  scope_id, branch_identity, object_type_id, goid, record_id, timestamp_us, csn

record body:
  record_kind
  changed_property_count
  changed_property_ids
  changed_property_ops
  changed_property_value_refs
```

```rust
enum DeltaPropertyOpV1 {
    SetValue = 0,
    SetNull = 1,
    Clear = 2,
    Tombstone = 3,
    Redact = 4,
}

enum DeltaTombstoneKindV1 {
    Object = 0,
    Property = 1,
    Association = 2,
    Evidence = 3,
    ProjectionRow = 4,
}
```

Omitted property means unchanged. `SetValue` assigns a value from the delta-local encoded value stream. `SetNull` sets the property to null if allowed. `Clear` explicitly clears the property under the declared COVE-O policy. `Tombstone` tombstones an object, property, association, evidence assertion, or projection row according to `DeltaTombstoneKindV1`. `Redact` marks a present value inaccessible and MUST bind matching redaction metadata. The ordinary COVE null bitmap MUST NOT mean "unchanged" in a sparse patch row.

#### 63.1.7 Continuation Anchors and State Hashes

A continuation anchor identifies the logical parent object state a delta expects to extend. It is not a physical row pointer.

```rust
enum DeltaAnchorStrengthV1 {
    KeyOnly = 0,
    KeyAndRecordId = 1,
    KeyRecordAndStateHash = 2,
    KeyRecordStateAndTrustHash = 3,
}

struct DeltaContinuationAnchorV1 {
    scope_kind: u16,
    scope_id: [u8; 16],
    object_type_id: u32,
    branch_identity_ref: u32,
    goid: [u8; 16],

    parent_ref: u32,
    predecessor_csn: u64,
    predecessor_timestamp_us: i64,
    predecessor_record_id: [u8; 16],

    predecessor_state_hash_ref: u32,
    predecessor_trust_hash_ref: u32,
    anchor_strength: u8,
    flags: u32,
    checksum: u32,
}
```

Anchors are required for the first patch or tombstone of an existing object in a delta unless the record is a full Baseline or Snapshot anchor. The first profile SHOULD require `KeyRecordAndStateHash` for patching existing objects. If the parent snapshot lacks a stored state hash, a reader MAY compute it from canonical logical state or reject when the selected operation requires anchor validation and recomputation is unsupported. Brand-new objects whose first delta record is a Baseline or Snapshot do not require a continuation anchor.

`DeltaStateHashKindV1::CoveObjectDeltaStateHashV1` is the required state-hash input for MVP continuation anchors. The hash input is the canonical logical latest object state at the predecessor record and includes scope kind and ID, canonical branch identity, object type ID, GOID, predecessor record ID, predecessor CSN, predecessor commit timestamp, record kind and tombstone state, sorted property IDs present in logical state, each property's logical type/collation/null/clear/tombstone/redaction marker/canonical visible value bytes, redaction commitments, and hidden-value commitments only when the selected disclosure policy permits them to participate.

The state hash excludes artifact-local FileCodes, dictionary IDs, physical page order, compression, section offsets, row ordinals, writer-local layout choices, advisory summaries, and indexes. Equivalent logical states with different FileCodes or layouts SHOULD produce the same state hash. Redacted values MUST NOT be hashed as hidden plaintext unless policy explicitly permits that disclosure.

#### 63.1.8 Touched Sets, Tombstone Sets, and Reconstruction

Every delta SHOULD include a compact summary of what it can affect:

```rust
struct DeltaTouchedObjectRangeV1 {
    scope_kind: u16,
    scope_id: [u8; 16],
    object_type_id: u32,
    branch_identity_ref: u32,
    min_goid: [u8; 16],
    max_goid: [u8; 16],
    touched_count: u64,
    property_bitmap_ref: u32,
    object_set_ref: u32,
    checksum: u32,
}
```

Touched-set representations MAY include sorted GOIDs, GOID prefix ranges, bitmaps over a manifest-provided dense object ordinal map, or a no-false-negative probabilistic representation. A touched-object summary used for skipping MUST be conservative: it may over-include touched objects/properties but MUST NOT exclude any object/property affected by the delta. Tombstone summaries MUST carry the same scope, object type, and branch identity fields so latest-state readers never apply a tombstone across scopes or branches. The first profile requires exact touched-object and exact tombstone summaries for ordinary latest-state and point-lookup skipping.

For a selected snapshot, object state reconstruction is logically:

```text
state = parent_state_at_cut(base, parent_deltas, query_cut)
for delta in ordered_deltas_needed_by_cut:
    validate delta parent and continuation anchors
    apply records in COVE-O temporal order
return state after visibility, redaction, branch, tombstone, and projection rules
```

A delta-aware planner SHOULD first validate the selected snapshot, delta-chain extension, and chain summary; use query root, object type, branch, temporal cut, selected properties, and predicates to choose candidate components; use the base temporal index, chain-summary entries, delta temporal indexes, touched sets, tombstone sets, temporal blooms, and COVE-I/COVX sidecars to prune; fetch only required delta headers or hot summary ranges; read the nearest required anchor plus later delta records for touched objects; apply sparse patches in CSN order into a dense in-memory state table keyed by `(scope_id, branch_identity, object_type_id, goid)`; and materialise only requested output fields.

Pruning MUST be conservative. `as_of_csn` cuts before `delta.csn_min` may skip that delta. Commit-time cuts may use `commit_time_range_start_us/end_us` only when commit timestamp monotonicity validates. Source publication or ingest batch filters may use `source_publish_range_start_us/end_us` operationally, but not as object commit time or valid time. Valid-time or temporal-role cuts may skip a delta only through validated temporal-role summaries. Latest-state queries MUST check tombstone summaries before returning parent state. Optional coverage/index metadata may prune only under the COVE-COVERAGE/COVE-I proof rules.

#### 63.1.9 Publication, Compaction, and Cross-Feature Ordering

Writers publish deltas by finalizing the delta footer, postscript, checksums, digests, and trust data, then publishing the COVM or external catalog snapshot that references the complete delta and chain summary. Object-store writers MUST publish the COVM snapshot or external catalog commit last and MUST NOT infer visibility from partially uploaded or unreferenced objects.

Compaction materialises a selected snapshot into a new self-contained `.cove` file:

```text
compact(base.cove, deltas...) -> compacted-base.cove
```

Compaction MUST preserve object state, history, branches, tombstones, trust hashes, evidence required by policy, and selected effective fingerprints; assign new file-local dictionaries and FileCodes; rebuild temporal indexes and requested sidecars; publish a new COVM or external-catalog state; and leave old base and delta artifacts immutable. Checkpoint deltas remain `.covedelta` artifacts carrying Baseline or Snapshot records for a declared object subset; they reduce read amplification but do not replace full compaction.

COVE-MAP definitions are inherited by fingerprint unless mapping rules change. Resolver catalog support can materialise ordinary COVE-O snapshots without deltas, but resolver-specific delta evidence/projection patches depend on stable COVE-MAP resolver semantics. The core delta MVP MUST NOT require executing COVE-MAP resolver logic for ordinary object reconstruction; it only binds effective semantic-map fingerprints and reconstructs materialised COVE-O temporal records.

**Implementation order requirements:**
- COVE-MAP entity-resolution Phase 0 and Phase 1 SHOULD land before resolver-specific delta evidence/projection patches, so `MAP_RESOLUTION_CATALOG`, `resolver_digest`, `catalog_digest`, `pipeline_digest`, row-level resolver outcomes, and evidence metadata are stable.
- The core COVE-O delta MVP MAY be implemented before resolver execution if it treats semantic-map fingerprints opaquely and does not claim support for `DELTA_FEATURE_MAP_EVIDENCE_PATCH`, `DELTA_EVIDENCE_PATCH`, or resolver-aware projection patches.
- A delta that changes mapping rules, alias catalog content, resolver hit/miss policy, resolver digests, normalisation pipeline versions, reviewed decisions that contribute merge edges, candidate/review semantics, or projection behaviour MUST expose a new effective semantic-map or projection fingerprint.
- Association, link, and evidence facts that affect ordinary COVE-O reconstruction MUST be materialised as COVE-O temporal records using declared object types and property flags. `DELTA_EVIDENCE_PATCH` and `DELTA_PROJECTION_PATCH` may provide replay, explanation, projection, or planning metadata, but MUST NOT be the only source of ordinary object truth unless a required extension defines that authority.

---
