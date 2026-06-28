# COVE-O Delta Artifacts

Status: implemented for the supported reference-prototype delta contract;
retained as design context for future standardization and advanced tiers.

Owning profiles: COVE-O / COVM / COVE-COVERAGE / COVE-I / COVE-MAP

Related specification areas: COVE immutability, COVM publication, COVE-O
object-temporal reconstruction, COVE-I secondary indexes, COVE-COVERAGE proof
semantics, COVE-L object-store planning

Related proposals:

- [COVE-MAP Resolver Catalog and Entity Resolution](./covemap-entity-resolution.md)

## Summary

This proposal defines an optional **COVE-O delta artifact**: an immutable
overlay file that records object-temporal additions, patches, tombstones,
association changes, evidence additions, and additive catalog changes against a
published COVE-O snapshot.

The current reference implementation supports the core COVM-selected
base-plus-delta workflow, `cove delta` chain commands, delta-aware query/export,
snapshot-bound sidecars, COVE-MAP semantic delta generation, inline dictionary
overlays, parent dictionary aliases, reconstruction, compaction, and
checkpointing. Broader items such as delta-local COVE-I indexes, COVE-COVERAGE
patches, object-store layout hints, and merge DAGs remain future design work.

The intent is to make COVE-O logically appendable without making `.cove` files
mutable.

The base `.cove` file remains a complete immutable archive. A delta-aware
reader selects a dataset snapshot through COVM or an external catalog, validates
the base file and ordered delta chain, then reconstructs object state from:

```text
base.cove + delta-0001.covedelta + delta-0002.covedelta + ...
```

Generic COVE readers that do not understand this proposal continue to read
ordinary `.cove` files when the user explicitly opens those files. A reader
that opens a COVM or external-catalog snapshot declaring a required delta chain
must either support this profile or reject that selected snapshot for
object-temporal reads. It must not silently return base-only data as if it were
the selected snapshot.

A compactor can materialize a base plus deltas into a new self-contained
`.cove` file whenever broad compatibility or lower read amplification is more
important than append efficiency.

This proposal is a publication and storage layer. COVE-MAP resolver-catalog
support can be implemented first and can produce ordinary self-contained
`.cove` snapshots without delta artifacts. Delta support for resolver-derived
evidence, identity-equivalence updates, or mapping-rule changes should build on
the resolver catalog's stable section IDs, digests, evidence metadata, and
semantic-map fingerprints.

## Motivation

COVE is deliberately immutable. That is a good property for archive integrity,
object-store publication, reproducibility, checksums, trust chains, and
conformance testing. It also means that small updates to a large mapped COVE-O
archive currently require one of three approaches:

- rewrite a new full `.cove` file;
- publish an additional complete `.cove` file and let an external protocol
  define the merge semantics;
- represent change data as ordinary COVE-T rows and require application-specific
  interpretation.

Those are valid, but they leave efficiency on the table for COVE-O workloads.
COVE-O already has object identity, branches, CSNs, temporal records, deltas,
snapshots, baselines, tombstones, and trust surfaces. The missing layer is a
portable way to publish **small immutable object-temporal overlays** without
restating the parent archive's object catalog, mapping metadata, dictionaries,
temporal segments, indexes, or coverage proofs.

The target workloads are:

- mapped customer/product/account/entity archives with periodic corrections;
- append-heavy object history where most objects are unchanged between cuts;
- audit trails where new evidence and tombstones should be published quickly;
- object-store deployments where rewriting a large base object is expensive;
- edge/offline archives that need a compact update bundle and later compaction.

## Goals

- Preserve COVE's immutable-file model.
- Make COVE-O snapshots appendable at the dataset layer.
- Avoid rewriting data already covered by the base `.cove` or parent deltas.
- Store changed object-temporal facts, not whole objects, when a patch is
  sufficient.
- Reuse parent catalogs, semantic-map fingerprints, projection definitions,
  dictionaries, coverage providers, and index roots by validated reference.
- Keep FileCode domains file-local while allowing digest-bound dictionary
  aliases to avoid duplicating common value bytes.
- Make snapshot selection explicit through COVM or an external catalog.
- Make delta-bearing snapshots fail closed for readers that do not support the
  required delta-chain profile.
- Let blob/object-store readers prune irrelevant deltas from one manifest or
  summary read before issuing per-delta requests.
- Bound read amplification through compaction, checkpoints, delta-local indexes,
  touched-object summaries, and chain-depth policy.
- Provide enough validation to reject stale, missing, reordered, or conflicting
  deltas.

## Non-Goals

- No in-place append to a finalized `.cove` file.
- No mutable footer, mutable section directory, mutable dictionary, or mutable
  page index.
- No implicit transaction log formed by scanning a directory for deltas.
- No cross-file `prev_ref` in ordinary COVE-O temporal rows.
- No relaxation of COVE-O self-containment for ordinary `.cove` files.
- No general ACID table protocol, concurrent writer protocol, or lakehouse
  catalog replacement.
- No hidden comparison of raw FileCodes from different artifacts.
- No base-only answer for a selected dataset snapshot that declares required
  deltas.

## Design Principle

Delta artifacts should be **thin, explicit, and reconstructable**.

They should carry only the information that is new at the selected snapshot:

- new temporal records;
- changed property values;
- tombstones and branch changes;
- new evidence rows;
- additive object/property/catalog declarations;
- delta-local dictionaries or aliases to parent dictionary entries;
- small summaries needed to avoid unnecessary parent reads;
- optional delta-local coverage and index metadata.

Everything inherited from the parent snapshot should be referenced by digest,
fingerprint, stable identifier, and selected snapshot state. A delta must not
silently copy or redefine parent truth.

## Profile Shape

This proposal intentionally defines one object-store-aware COVE-O delta profile
with feature tiers, rather than splitting the first draft into separate
local-minimal and blob-optimized profiles.

The reason is practical: a delta-bearing snapshot that cannot cheaply identify
which delta artifacts matter will be correct but expensive in the deployment
model COVE already targets. Requiring a compact chain summary in the first
profile keeps implementations honest about blob request cost and avoids a
second compatibility surface.

Local filesystem implementations may store the required summary inline and may
choose not to optimize range placement. Object-store implementations should
place the summary so it can be fetched with the selected COVM snapshot metadata
or through one small additional request.

## Core Invariants

1. `.cove` files remain immutable and self-contained.
2. `.covedelta` files are also immutable. A valid delta is complete only after
   its footer/postscript/checksums validate.
3. A delta is not selected by filename discovery. It is selected only by a COVM
   snapshot or an external catalog snapshot.
4. A selected delta chain is ordered. Reordering deltas changes the snapshot and
   must be rejected by chain digest validation.
5. Every delta declares exactly which parent snapshot it extends.
6. Parent files and parent deltas are identified by stable artifact IDs, length,
   footer CRC, and mandatory cryptographic digest.
7. Object catalogs and projection catalogs are inherited by fingerprint unless
   the delta declares an additive catalog change.
8. Object type IDs, property IDs, branch identities, GOIDs, record IDs, CSNs,
   scopes, and temporal roles keep the meaning defined by COVE-O.
9. Raw FileCodes remain local to the artifact that stores them. Cross-artifact
   equality requires canonical values, canonical hashes with collision
   resolution, or digest-bound dictionary aliases.
10. A delta record that depends on parent state uses logical ordering and
    continuation anchors, not cross-file COVE-O `prev_ref`.
11. Missing or corrupt required deltas reject the selected snapshot. Optional
    delta indexes, layout plans, and coverage caches may fail open only when the
    requested operation does not require them.
12. Compaction produces a new `.cove` file. It never mutates the base or parent
    deltas.
13. A COVM or external-catalog snapshot that declares a required delta chain
    must fail closed for unsupported object-temporal reads.
14. The first implementation profile is append-only in commit order: delta CSNs
    and COVE-O `timestamp_us` values must advance beyond the selected parent
    high-water mark. Historical commit-order corrections require a later
    required extension.
15. Branch identity used across artifacts is canonical branch identity, not a
    raw artifact-local FileCode.

## Artifact Naming

Recommended extension:

```text
.covedelta
```

Provisional magic:

```text
CVD2
```

The magic intentionally differs from ordinary COVE magic so v2 `.cove` readers
cannot mistake a delta artifact for a self-contained COVE file.

`CVD2` means "COVE v2 delta artifact". The artifact envelope still carries
`version_major` and `version_minor`; the `V1` struct names below describe the
first proposed delta-artifact schema inside the COVE v2 family.

## COVM Delta-Chain Extension

A COVM or external catalog snapshot should describe the selected base and
ordered delta chain through a first-class required extension block:

```rust
struct CovmDeltaChainExtensionV1 {
    delta_chain_profile_id: u32,
    delta_chain_profile_version_major: u16,
    delta_chain_profile_version_minor: u16,
    required_delta_features: u64,
    optional_delta_features: u64,

    dataset_id: [u8; 16],
    base_snapshot_id: [u8; 16],
    result_snapshot_id: [u8; 16],

    base_artifact_ref: u32,
    ordered_delta_count: u32,
    ordered_delta_artifact_refs_offset: u64,
    ordered_delta_artifact_refs_length: u64,

    chain_digest_algorithm: u16,
    chain_digest_len: u16,
    chain_digest_ref: u32,

    chain_summary_kind: u8,       // inline, covm_section, external_ref
    reserved0: u8,
    chain_summary_ref: u32,
    chain_summary_offset: u64,
    chain_summary_length: u64,
    chain_summary_crc32c: u32,
    chain_summary_digest_algorithm: u16,
    chain_summary_digest_len: u16,
    chain_summary_digest_ref: u32,

    effective_schema_fingerprint_ref: u32,
    effective_object_catalog_fingerprint_ref: u32,
    effective_projection_fingerprint_ref: u32,
    effective_semantic_map_fingerprint_ref: u32,
    effective_visibility_fingerprint_ref: u32,
    effective_redaction_fingerprint_ref: u32,

    csn_min: u64,
    csn_max: u64,
    created_at_us: i64,
    checksum: u32,
}
```

The chain digest is mandatory. It should bind:

- dataset ID;
- base artifact ID, length, footer CRC, digest, and base snapshot ID;
- ordered delta artifact IDs, lengths, footer CRCs, digests, and ordinals;
- result snapshot ID;
- required delta feature bits;
- effective schema, object catalog, projection, semantic-map, visibility, and
  redaction fingerprints.

Rules:

- `ordered_delta_artifact_refs` is part of snapshot truth. A reader must not add
  newer deltas because they happen to exist next to the selected files.
- A reader that does not support `delta_chain_profile_id`,
  `delta_chain_profile_version_*`, or any required delta feature bit must reject
  the selected snapshot for object-temporal reads.
- A reader may open the base `.cove` directly only when the user/request selects
  the base artifact rather than the delta-bearing dataset snapshot.
- Snapshot-level COVE-I/COVX indexes, coverage providers, and caches must bind
  the exact `chain_digest`, not only the base file digest or latest timestamp.
- A selected chain with a mismatched chain digest, missing delta, extra delta,
  or reordered delta is invalid.
- `chain_summary_*` must identify a compact, checksum-validated summary that is
  sufficient to prune deltas before opening individual `.covedelta` objects for
  common object-temporal reads.
- CRC32C detects accidental transfer/storage corruption. The cryptographic
  `chain_summary_digest_*` fields bind the summary bytes as selected snapshot
  metadata. A reader must not use an external or separately addressable chain
  summary for pruning unless its cryptographic digest validates.
- Object-store-oriented COVM writers should place the chain summary in the same
  small COVM range as the selected snapshot metadata, or in one separately
  addressable summary object. A reader should not need one blob request per
  delta merely to discover that most deltas are irrelevant.

## Authoritative Surfaces

Several fields are deliberately duplicated so readers can plan without opening
every artifact. The precedence rule is:

- The COVM delta-chain extension is authoritative for selected snapshot
  identity.
- The chain digest is authoritative for the ordered base-plus-delta chain.
- The chain summary is authoritative only for pre-open pruning after its digest
  validates.
- The delta postscript/footer are authoritative for locating delta bytes.
- The delta header is authoritative for the delta artifact's declared lineage,
  feature bits, effective metadata fingerprints, scope policy, and commit-time
  range.
- Section directory entries are authoritative for section byte ranges.
- Section payloads are authoritative for their own local reference spaces after
  their checksums and required features validate.

Any duplicated field that disagrees across authoritative surfaces must reject
the selected snapshot or requested operation, except where the field is
explicitly declared an over-inclusive summary.

## Delta Chain Summary

The chain summary is the blob-cost control plane for a delta-bearing snapshot.
It is selected and validated through the COVM delta-chain extension, then used
to decide which delta artifacts must be opened for a requested object-temporal
operation.

```rust
struct CovmDeltaChainSummaryV1 {
    magic: [u8; 4],              // "CDS1"
    version_major: u16,
    version_minor: u16,
    header_len: u16,
    flags: u32,

    dataset_id: [u8; 16],
    result_snapshot_id: [u8; 16],
    chain_digest_algorithm: u16,
    chain_digest_len: u16,
    chain_digest_ref: u32,

    delta_summary_count: u32,
    object_type_summary_count: u32,
    branch_summary_count: u32,
    temporal_role_summary_count: u32,

    delta_summaries_offset: u64,
    object_type_summaries_offset: u64,
    branch_summaries_offset: u64,
    temporal_role_summaries_offset: u64,
    payload_offset: u64,
    payload_length: u64,
    checksum: u32,
}

struct DeltaChainSummaryEntryV1 {
    chain_ordinal: u32,
    delta_artifact_ref: u32,
    delta_artifact_id: [u8; 16],

    required_delta_features: u64,
    optional_delta_features: u64,

    csn_min: u64,
    csn_max: u64,
    commit_time_start_us: i64,
    commit_time_end_us: i64,

    artifact_created_at_us: i64,
    first_published_at_us: i64,
    selected_snapshot_published_at_us: i64,
    time_field_presence_flags: u32,
    time_summary_exactness_flags: u32,
    source_publish_range_start_us: i64,
    source_publish_range_end_us: i64,

    scope_summary_ref: u32,
    branch_summary_ref: u32,
    object_type_summary_ref: u32,
    goid_range_summary_ref: u32,
    touched_summary_ref: u32,
    tombstone_summary_ref: u32,
    property_summary_ref: u32,
    temporal_role_summary_ref: u32,

    delta_header_range_offset: u64,
    delta_header_range_length: u64,
    hot_summary_range_offset: u64,
    hot_summary_range_length: u64,
    checksum: u32,
}
```

Rules:

- The summary must bind the same ordered `chain_digest` as the selected COVM
  delta-chain extension.
- Summary entries must be dense and sorted by `chain_ordinal`.
- `commit_time_start_us/end_us` describe COVE-O commit/file-ordering
  `timestamp_us` range, not business valid time.
- `first_published_at_us` records when the delta artifact first became visible
  in any published snapshot known to the publisher.
- `selected_snapshot_published_at_us` records when this selected snapshot made
  the delta visible.
- `time_field_presence_flags` declares which optional time fields are present.
  A missing optional timestamp field is encoded as zero and must not be used for
  pruning.
- `time_summary_exactness_flags` declares whether advertised ranges are exact or
  conservative. Source publication/ingest ranges that are not exact may be used
  for discovery and cache refresh; they may prove absence only when the flag
  declares a conservative no-under-include range for the selected operation.
- `time_field_presence_flags` bit `0x0000_0001` means
  `source_publish_range_start_us/end_us` is present.
- `time_summary_exactness_flags` bit `0x0000_0001` means the source publish
  range is conservative and does not under-include source publication/ingest
  dates represented by the delta. Bit `0x0000_0002` means the range is exact.
- `source_publish_range_start_us/end_us` is a producer-declared source
  publication or ingest-batch date range. It is useful for operational
  discovery, replication, and cache refresh. It must not be used as COVE-O
  commit time or business valid time unless a required profile explicitly
  defines that mapping.
- Valid-time and other temporal-role pruning must use
  `temporal_role_summary_ref`; it must not be inferred from CSN, commit time,
  or source publication dates.
- `scope_summary_ref`, `branch_summary_ref`, `object_type_summary_ref`, and
  `goid_range_summary_ref` may over-include but must not under-include.
- `touched_summary_ref` and `tombstone_summary_ref` are exact in the MVP profile
  when used for latest-state point lookup skipping.
- A corrupt, stale, missing, or unsupported required chain summary makes the
  selected delta-bearing snapshot unsupported for object-temporal reads. It
  must not degrade into reading the base only.
- Optional large summaries may live inside the delta artifact, but the COVM
  chain summary must expose enough information to avoid opening deltas that are
  provably irrelevant.

Recommended summary representations:

- sorted object type IDs per delta;
- canonical branch identity refs per delta;
- scope refs or single-scope flags per delta;
- GOID min/max ranges per object type/branch/scope;
- exact touched-object set refs for small deltas;
- exact tombstone set refs for small deltas;
- no-false-negative blooms only when exact sets would be too large and the
  operation can tolerate false positives;
- property bitmaps by object type for projection pruning;
- temporal-role min/max summaries for valid-time pruning.

Blob-cost rule:

```text
COVM selected snapshot + chain summary should be enough to decide which delta
artifacts may need to be opened for common point, object-type, branch, CSN, and
commit-time reads.
```

If a reader must fetch every delta header before it can decide which deltas are
irrelevant, the snapshot is valid but not blob-cost efficient.

## Binary Envelope

A `.covedelta` artifact should use COVE-style tail discovery:

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

Rules:

- Delta envelope fields use the same binary discipline as COVE v2 unless this
  proposal says otherwise: little-endian integers, explicit lengths and offsets,
  no native struct padding, and checksums computed with the checksum field
  treated as zero.
- The final magic must be `CVD2`.
- `postscript_len` locates `CoveDeltaPostscriptV1`.
- The postscript locates the footer; the footer locates the header and section
  directory.
- Readers validate the postscript, footer, header, section directory, and only
  the sections needed by the requested operation.
- Section offsets and lengths are relative to the start of the `.covedelta`
  artifact.
- `section_kind` is the section-kind enum. `section_id` is a unique per-artifact
  section instance ID, allowing multiple sections of the same kind in later
  versions.
- Unknown required delta features reject only the selected operation that needs
  the feature. An unknown required feature in an optional index/layout section
  must not block ordinary object reconstruction if that section is not used.
  An unknown required feature in temporal segment semantics, sparse patch
  semantics, anchors, tombstones, or required summaries rejects
  object-temporal reads for the selected snapshot.

## Delta Header

The header identifies the artifact, selected snapshot lineage, effective
metadata fingerprints, scope policy, and append-only commit range.

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

Header rules:

- Header flag `0x0000_0001` is `DELTA_FLAG_SINGLE_SCOPE`.
- Header flag `0x0000_0002` is
  `DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT`. If the flag is clear,
  `source_publish_range_start_us/end_us` must be zero and ignored.
- `chain_ordinal` is dense within the selected snapshot chain.
- `chain_depth` includes the current delta and is used for policy limits.
- `csn_min..=csn_max` must be after the selected parent high-water mark for the
  same scope and branch identity in the initial append-only profile.
- COVE-O `timestamp_us` remains commit/file-ordering time. Business/effective
  time corrections must use declared temporal-role properties, not backdated
  commit timestamps.
- `created_at_us` records artifact creation time.
- `source_publish_range_start_us/end_us` records an optional producer-declared
  source publication, ingest, or update-batch date range covered by the delta.
  It is operational metadata, not COVE-O commit time and not business valid
  time. The header range is descriptive; pruning uses the COVM chain-summary
  presence and exactness flags.
- Historical commit-order insertion requires a later required extension.
- Fingerprints describe the **effective** metadata after applying this delta,
  not only the metadata physically stored inside the delta.
- A zero or absent fingerprint means the metadata surface is unchanged from the
  parent and inherited by parent reference.
- If `DELTA_FLAG_SINGLE_SCOPE` is set, every temporal record, anchor, touched
  set, and tombstone summary in the delta belongs to `scope_kind/scope_id`.
  Otherwise those structures must carry explicit scope fields.

## Parent References

Each delta declares the artifacts it depends on:

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

Rules:

- Parent flag `0x0000_0001` is `DELTA_PARENT_REF_LINEAGE_PARENT`.
- A parent reference must validate before any inherited metadata or value bytes
  are used.
- `digest_algorithm`, `digest_len`, and `digest_ref` are mandatory for base and
  parent-delta references.
- Each delta must contain exactly one lineage parent reference whose
  `snapshot_id` equals `CoveDeltaHeaderV1.parent_snapshot_id`.
- Other parent references are ancillary sidecar, index, mapping, validation, or
  evidence references. They do not define chain order.
- `parent_delta` references must form a single ordered chain for a simple
  snapshot. Future merge DAGs require a separate required extension.
- Parent URI is advisory. Digest and fingerprint validation are authoritative.
- A reader must reject a delta whose declared parent snapshot is not the
  selected parent snapshot.

## Sections

Recommended section kinds:

| Kind | Section | Meaning |
| ---: | --- | --- |
| 0 | `DELTA_PARENT_REFS` | Digest-bound base, parent-delta, and sidecar references. |
| 1 | `DELTA_CATALOG_PATCH` | Additive object/property/catalog declarations. |
| 2 | `DELTA_DICTIONARY_OVERLAY` | Local dictionary entries and parent aliases. |
| 3 | `DELTA_TEMPORAL_SEGMENT_INDEX` | Delta-local COVE-O temporal segment index. |
| 4 | `DELTA_TEMPORAL_SEGMENT_DATA` | Delta-local COVE-O temporal segment payloads. |
| 5 | `DELTA_CONTINUATION_ANCHORS` | Logical predecessor anchors for touched objects. |
| 6 | `DELTA_TOUCHED_OBJECT_SET` | Conservative changed object/property summary. |
| 7 | `DELTA_TOMBSTONE_SET` | Conservative tombstone summary. |
| 8 | `DELTA_PROPERTY_OPS` | Sparse property operation streams when not embedded in temporal pages. |
| 9 | `DELTA_EVIDENCE_PATCH` | Additional replay/explanation evidence metadata. |
| 10 | `DELTA_PROJECTION_PATCH` | Projection metadata or invalidation summaries. |
| 11 | `DELTA_COVERAGE_PATCH` | Delta-local COVE-COVERAGE providers and sets. |
| 12 | `DELTA_INDEX_HINTS` | Optional references to delta-local COVE-I/COVX artifacts. |
| 13 | `DELTA_LAYOUT_HINTS` | Optional byte-range and object-store planning hints. |
| 14 | `DELTA_TRUST_CONTINUATION` | Trust-chain continuation and state-hash metadata. |
| 15 | `DELTA_STRING_TABLE` | String and byte payload table for descriptors. |
| 16 | `DELTA_BRANCH_IDENTITY_TABLE` | Canonical branch identity descriptors. |
| 17 | `DELTA_SCOPE_TABLE` | Scope descriptors used by summaries and records. |
| 18 | `DELTA_TEMPORAL_ROLE_SUMMARY_TABLE` | Temporal-role range summaries. |
| 19 | `DELTA_TOUCHED_SUMMARY_TABLE` | Exact or conservative touched-object summaries. |
| 20 | `DELTA_TOMBSTONE_SUMMARY_TABLE` | Exact or conservative tombstone summaries. |
| 21 | `DELTA_STATE_HASH_TABLE` | Canonical state hash descriptors and payload refs. |
| 255 | `DELTA_EXTENSION` | Required or optional extension payload. |

Only the temporal segment sections and the metadata needed to validate them are
required for a minimal object delta. Index, coverage, layout, and projection
patches are optional.

## Descriptor Tables

Delta-local `*_ref` fields resolve through explicit descriptor tables, not
through ad hoc offsets or implicit ordering. Unless a section says otherwise,
descriptor refs are dense zero-based indexes into the matching descriptor table
in the same artifact or COVM chain-summary payload.

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

Rules:

- `branch_identity_ref` resolves through `DELTA_BRANCH_IDENTITY_TABLE`.
- `scope_summary_ref` resolves through `DELTA_SCOPE_TABLE` or a summary
  descriptor that names one or more scope refs.
- `temporal_role_summary_ref` resolves through
  `DELTA_TEMPORAL_ROLE_SUMMARY_TABLE`.
- `touched_summary_ref` resolves through `DELTA_TOUCHED_SUMMARY_TABLE`.
- `tombstone_summary_ref` resolves through `DELTA_TOMBSTONE_SUMMARY_TABLE`.
- `predecessor_state_hash_ref` resolves through `DELTA_STATE_HASH_TABLE`.
- Unsupported required descriptor kinds reject the selected operation that
  needs that descriptor.

## Delta Feature Bits

Delta feature bits should be split into required and optional surfaces:

| Bit | Feature | Meaning |
| ---: | --- | --- |
| 0 | `DELTA_FEATURE_SPARSE_PATCH_ROWS` | Delta temporal rows use sparse property operations. |
| 1 | `DELTA_FEATURE_OBJECT_TOMBSTONES` | Object tombstone records may appear. |
| 2 | `DELTA_FEATURE_PROPERTY_TOMBSTONES` | Property tombstone operations may appear. |
| 3 | `DELTA_FEATURE_ASSOCIATION_TOMBSTONES` | Association/link tombstones may appear. |
| 4 | `DELTA_FEATURE_CONTINUATION_ANCHORS` | Existing-object patches require logical anchors. |
| 5 | `DELTA_FEATURE_INLINE_DICTIONARY` | Delta-local dictionary values are inline. |
| 6 | `DELTA_FEATURE_PARENT_DICTIONARY_ALIASES` | Delta-local dictionary values may alias parent dictionaries. |
| 7 | `DELTA_FEATURE_EXACT_TOUCHED_SET` | Touched-object summaries are exact and required for skipping. |
| 8 | `DELTA_FEATURE_EXACT_TOMBSTONE_SET` | Tombstone summaries are exact and required for latest-state reads. |
| 9 | `DELTA_FEATURE_CHECKPOINT_BASELINES` | Delta may carry checkpoint Baseline/Snapshot records. |
| 10 | `DELTA_FEATURE_COVERAGE_PATCH` | Delta carries COVE-COVERAGE patch sections. |
| 11 | `DELTA_FEATURE_INDEX_HINTS` | Delta references COVE-I/COVX index artifacts. |
| 12 | `DELTA_FEATURE_MAP_EVIDENCE_PATCH` | Delta carries COVE-MAP evidence metadata. |
| 13 | `DELTA_FEATURE_PROJECTION_PATCH` | Delta carries projection metadata or invalidation summaries. |
| 14 | `DELTA_FEATURE_HISTORICAL_COMMIT_INSERT` | Delta may insert historical commit-order records. Deferred. |

Rules:

- The MVP should require bits 0, 1, 4, 5, 7, and 8.
- A reader that lacks a required feature rejects the selected snapshot for the
  affected operation.
- Optional feature corruption or unsupported optional feature payloads may fall
  back only when the requested operation does not require them.
- `DELTA_FEATURE_HISTORICAL_COMMIT_INSERT` is intentionally deferred and should
  not be required by the first profile.

## Catalog Patches

Most deltas should not repeat the parent object type catalog. They should
inherit it by fingerprint.

When catalog changes are needed, they should be additive:

- new object type;
- new property on an existing object type;
- new association/link/evidence/projection object type;
- new temporal role binding;
- new branch alias;
- new projection definition that depends only on declared object/property IDs.

Catalog patches must not rename, remove, or reinterpret parent declarations.
Breaking catalog changes should publish a new base `.cove` snapshot or use a
separate schema-generation branch.

```text
EffectiveCatalog(delta_n) =
  ApplyAdditivePatch(EffectiveCatalog(parent), delta_n.catalog_patch)
```

Readers reject:

- duplicate object type IDs;
- duplicate property IDs within one object type;
- changed logical type for an inherited property;
- changed collation for an inherited property;
- changed association endpoint flags for an inherited property;
- changed projection authority for an inherited projection.

## Dictionary Overlays

COVE FileCodes remain artifact-local. A delta may still avoid duplicating value
bytes by making its **local** dictionary entries aliases to parent dictionary
entries.

The first implementation tier should use inline delta dictionary values only.
Parent dictionary aliases are valuable but should be treated as a second-tier
optimization after sparse patches, exact touched sets, exact tombstone sets,
and compaction equivalence are stable.

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

Rules:

- Delta encoded pages use delta-local FileCodes.
- A local code may resolve to an inline value or to a validated parent alias.
- A parent alias is valid only if the parent artifact and parent dictionary
  digest match the selected snapshot.
- Parent aliases must include logical type and collation context.
- Parent aliases are an encoding optimization, not a cross-file code domain.
- V1 parent aliases must resolve directly to a parent inline dictionary value or
  ordinary parent COVE dictionary value. Alias-to-alias recursion is prohibited
  unless a later required extension defines bounded recursive resolution.
- `CanonicalHashHint` may support pruning or equality hints only. It is not
  sufficient to reconstruct a materialized output value unless canonical value
  bytes are recoverable from a validated parent or inline source.
- A delta must not alias, hash, or expose equality hints for a parent value that
  is redacted or policy-protected in the selected snapshot unless the selected
  disclosure policy explicitly permits that equality leakage.
- If a parent dictionary cannot be validated, the selected snapshot rejects
  when the aliased value is required.

This gives the efficiency benefit of not rewriting common strings, enums, and
low-cardinality values while preserving the existing FileCode domain rule.

## Branch Identity

COVE-O allows the physical `branch_key` column to use FileCode or FixedBytes.
Raw FileCodes cannot cross artifact boundaries. Delta metadata that names a
branch must therefore use a canonical branch identity:

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

Rules:

- Delta temporal pages may still encode branch values in their local physical
  representation.
- Cross-artifact anchors, touched sets, tombstone sets, and summaries must use
  `branch_identity_ref`, not a raw FileCode.
- If `branch_identity_kind` is `catalog_branch_id`, the selected branch catalog
  fingerprint binds the ID to its canonical branch value.
- Hash-only branch identity is a lookup accelerator unless collision-free
  construction or canonical branch bytes are available for verification.

## Temporal Records

Delta temporal segment payloads should reuse COVE-O record semantics:

- one object type per temporal segment;
- rows ordered by COVE-O logical order:
  `(timestamp_us, csn, branch identity, goid, record_id)`;
- record kinds are Delta, Snapshot, Baseline, or Tombstone;
- property columns use COVE encoded-array machinery;
- null, missing, redaction, and clear/tombstone semantics stay explicit.

The delta format differs from ordinary COVE-O in one important way:

```text
Delta records may depend on parent state, but they must not use cross-file
COVE-O prev_ref.
```

Inside one delta artifact, `prev_ref` remains file-local. Across artifacts,
logical continuity is expressed through continuation anchors.

The initial profile is append-only in commit order:

- delta `timestamp_us` remains COVE-O commit/file-ordering time;
- `csn_min..=csn_max` must advance beyond the selected parent high-water mark
  for the same scope and branch identity;
- historical business/effective-time corrections must be represented through
  declared temporal-role properties;
- historical commit-order insertion is deferred to a required extension.

## Continuation Anchors

A continuation anchor says which parent object state a delta expects to extend.
It is not a physical row pointer.

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

Rules:

- Anchors are required for the first patch/tombstone of an existing object in a
  delta unless the record is a full Baseline/Snapshot anchor.
- Anchors bind the delta to logical parent state by key, CSN, record ID, and
  hash where available.
- The first implementation should require `KeyRecordAndStateHash` for patching
  existing objects. If the parent snapshot lacks a stored state hash, the reader
  may compute it from canonical logical state or reject when the delta requires
  stored hashes.
- A reader that cannot validate a required anchor must reject the selected
  snapshot.
- Anchors may be omitted for brand-new objects whose first record is a Baseline
  or Snapshot in the delta.

This avoids cross-file row references while preserving corruption and conflict
detection.

## Delta State Hash V1

`DeltaStateHashKindV1::CoveObjectDeltaStateHashV1` is the required state-hash
input for MVP continuation anchors.

The hash input is computed over the canonical logical latest object state at
the predecessor record. It includes:

- scope kind and scope ID;
- canonical branch identity;
- object type ID;
- GOID;
- predecessor record ID;
- predecessor CSN;
- predecessor commit timestamp;
- record kind and tombstone state;
- sorted property IDs present in the logical state;
- each property's logical type, collation, null/clear/tombstone/redaction
  marker, and canonical logical value bytes when visible;
- redaction marker plus redaction commitment when the value is redacted;
- optional hidden-value commitment only when the selected disclosure policy
  permits that commitment to participate in equality/trust checks.

It excludes:

- artifact-local FileCodes;
- dictionary IDs;
- physical page order;
- compression;
- section offsets;
- row ordinals;
- writer-local layout choices;
- advisory summaries or indexes.

Rules:

- Equivalent logical states with different FileCodes or physical layout should
  produce the same state hash.
- A redacted value must not be hashed as hidden plaintext unless policy
  explicitly permits that disclosure.
- If the parent snapshot lacks a stored compatible state hash, a reader may
  compute it from canonical logical state or reject the delta when the selected
  operation requires anchor validation and recomputation is unsupported.

## Patch Encoding

Delta records should default to sparse patches.

For each object record:

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

Semantics:

- omitted property means unchanged;
- `SetValue` assigns a value from the delta-local encoded value stream;
- `SetNull` sets the property to null if the property allows null;
- `Clear` explicitly clears the property under the declared COVE-O policy;
- `Tombstone` tombstones an object, property, association, evidence assertion,
  or projection row according to `DeltaTombstoneKindV1`;
- `Redact` marks a present value inaccessible and must bind matching redaction
  metadata;
- Snapshot/Baseline records may carry full state when faster reads or
  self-anchoring justify the extra bytes.

The ordinary COVE null bitmap must not be overloaded to mean "unchanged" in a
sparse patch row. "Unchanged" is represented by the absence of a property ID
from the sparse patch operation list.

The writer should choose:

- Delta rows for small changes;
- Snapshot rows after many patches to the same object;
- Baseline rows when the delta is intended to be self-anchoring for a subset;
- Tombstone rows for object, property, association, or evidence removal.

## Touched Object Set

Every delta should include a compact summary of what it can affect:

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

Representations may include sorted GOIDs, GOID prefix ranges, bitmaps over a
manifest-provided dense object ordinal map, or a bloom with declared false
positive behavior.

Rules:

- A touched-object summary used for skipping must be conservative: it may
  over-include touched objects/properties but must not exclude any
  object/property affected by the delta.
- Exact touched sets allow a reader to skip deltas for untouched point lookups.
- Property bitmaps allow a projection query to skip deltas that cannot affect
  selected properties.
- Tombstone sets must be consulted before returning parent state for latest
  queries.
- If the representation is probabilistic, it may prove absence only when its
  descriptor declares a structurally validated no-false-negative construction.
- If a touched set is corrupt or unsupported, it must not be used for skipping.
- The MVP should require exact touched-object and exact tombstone summaries.

Tombstone summaries should carry the same `scope_kind`, `scope_id`,
`object_type_id`, and `branch_identity_ref` identity fields so latest-state
readers never apply a tombstone across scopes or branches.

## Reconstruction

For a selected snapshot, object state reconstruction is:

```text
state = parent_state_at_cut(base, parent_deltas, query_cut)
for delta in ordered_deltas_needed_by_cut:
    validate delta parent and continuation anchors
    apply records in COVE-O temporal order
return state after visibility, redaction, branch, tombstone, and projection rules
```

An implementation should not literally reconstruct the whole dataset for every
query. The efficient read path is:

1. Validate the selected snapshot, delta-chain extension, and chain summary.
2. Use query root, object type, branch, temporal cut, selected properties, and
   predicates to choose candidate components from the chain summary before
   opening individual delta artifacts.
3. Use base COVE-O temporal segment index, chain-summary entries, delta
   temporal indexes, touched sets, tombstone sets, temporal blooms, and
   COVE-I/COVX sidecars to prune.
4. Fetch only the delta headers or hot summary ranges for deltas that remain
   candidates after chain-summary pruning.
5. For untouched objects/properties, read only the newest component that can
   prove the requested state.
6. For touched objects, read the nearest required anchor plus later delta
   records.
7. Apply sparse patches in CSN order into a dense in-memory state table keyed by
   `(scope_id, branch_identity, object_type_id, goid)`.
8. Materialize only requested output fields.

## Query Planning

A delta-aware planner should use these pruning rules:

- An `as_of_csn` cut before `delta.csn_min` can skip that delta.
- An `as_of_commit_timestamp_us` cut before
  `delta.commit_time_range_start_us` can skip that delta when commit timestamp
  monotonicity validates.
- A query filtered by source publication/ingest batch time may use
  `source_publish_range_start_us/end_us` for operational delta selection, but
  must not treat that range as object commit time or valid time.
- A valid-time or other temporal-role cut may skip a delta only through
  validated summaries for that temporal role.
- An `as_of_csn` cut after `delta.csn_max` may use the delta's final summaries
  for latest-state pruning.
- An object type not present in the delta temporal index can skip the delta.
- A branch not present in the delta branch summary can skip the delta.
- A point lookup for a GOID not present in an exact touched set can skip the
  delta.
- A projection that reads only properties outside a delta property bitmap can
  skip that delta unless tombstones or projection invalidations apply.
- A latest-state query must check tombstone summaries before returning parent
  state.
- Optional delta coverage/index metadata can prune only under the normal
  COVE-COVERAGE/COVE-I proof rules.
- Chain-summary pruning may over-include deltas. It must not under-include any
  delta that could affect the selected object-temporal result.

Coverage composition must be conservative:

- `DefinitelyNo` for a snapshot requires every selected component to prove
  no matching visible result.
- `DefinitelyYes` over parent data does not prove visible snapshot yes if
  deltas may tombstone, change, or redact the affected rows.
- Exact aggregate/index-only answers require a component-wise exact answer plus
  exact overlay correction, or a snapshot-level index built for the selected
  snapshot.
- Approximate summaries remain approximate and must not answer exact queries.

For the initial append-only profile, records are applied in selected delta-chain
order and COVE-O commit order. A delta whose records do not advance beyond the
selected parent high-water mark for the same scope and branch identity is
invalid unless a future required historical-commit extension is enabled.

## Publication Protocol

For a local filesystem writer:

1. Read and validate the parent snapshot state.
2. Build the delta in temporary storage.
3. Finalize the delta footer, postscript, checksums, digests, and trust data.
4. Build or update the COVM chain summary for the resulting snapshot.
5. Flush the delta bytes and `fsync`/`fdatasync` according to platform policy.
6. Publish a new COVM snapshot that references the complete delta and summary.
7. Flush the manifest and containing directory.

For object storage:

1. Upload the delta under an immutable content-addressed or versioned object
   name.
2. Verify object length and digest.
3. Upload or embed the updated chain summary.
4. Publish the COVM snapshot or external catalog commit last.
5. Never infer visibility from a partially uploaded or unreferenced object.

Readers select snapshots through COVM or the external catalog. They do not race
the writer's temporary files.

## Compaction

Compaction materializes a selected snapshot into a new self-contained `.cove`
file:

```text
compact(base.cove, deltas...) -> compacted-base.cove
```

Compaction should:

- preserve COVE-O object state, history, branches, tombstones, trust hashes, and
  evidence required by the selected policy;
- assign new file-local dictionaries and FileCodes;
- rebuild COVE-O temporal segment indexes;
- rebuild COVE-COVERAGE and COVE-I/COVX sidecars when requested;
- emit an object catalog section whose logical fingerprint equals the selected
  effective object catalog fingerprint, preserving the parent catalog
  fingerprint when the logical catalog is unchanged;
- publish a new COVM snapshot that references the compacted file;
- leave old base and delta artifacts immutable.

Recommended compaction triggers:

- chain depth exceeds a configured limit;
- delta bytes exceed a percentage of base bytes;
- point lookup read amplification exceeds a target;
- latest-state reconstruction touches too many deltas;
- schema/catalog patches accumulate beyond an implementation threshold;
- object-store range requests exceed the planned budget.

## Checkpoint Deltas

Between full compactions, a writer may publish a checkpoint delta. A checkpoint
delta is still a `.covedelta`, but it carries Baseline/Snapshot records for a
declared object subset.

Use cases:

- hot objects updated many times;
- branch tips;
- object types whose latest state is queried frequently;
- bounded offline update bundles where the reader may not have enough memory to
  replay a long patch chain at once.

Checkpoint deltas trade bytes for lower read amplification. They do not replace
full compaction because ordinary COVE-O readers still need a self-contained
`.cove` file.

## Interaction With COVE-MAP

COVE-MAP definitions should be inherited by fingerprint unless mapping rules
change. This creates an asymmetric dependency with the COVE-MAP resolver-catalog
proposal: resolver catalog support can materialize ordinary COVE-O snapshots
without deltas, but deltas that carry resolver-derived changes must preserve the
resolver proposal's replay, evidence, digest, and fingerprint semantics.

When the parent snapshot was produced through resolver-backed identity rules,
ordinary object truth remains the materialized COVE-O temporal records. A
delta-aware reader reconstructing ordinary objects does not need to execute
resolver logic unless the selected read surface asks for deterministic replay,
resolution-aware explain, candidate/review surfaces, or resolution-specific
evidence readback.

A delta may add:

- evidence rows;
- source-row digest references;
- mapping-run metadata;
- resolver-run metadata and digest proofs;
- conflict-resolution outcomes;
- identity-equivalence assertions;
- projection rows or projection invalidations;
- additive projection definitions.

Association, link, and evidence facts that affect ordinary COVE-O object
reconstruction must be materialized as COVE-O temporal records using declared
object types and property flags. `DELTA_EVIDENCE_PATCH` and
`DELTA_PROJECTION_PATCH` may provide replay, explanation, projection, or
planning metadata, but they must not be the only source of ordinary COVE-O
object truth unless a required extension defines that authority.

A delta must not silently change parent mapping semantics. If a mapping rule is
reinterpreted, the delta must declare a new semantic-map fingerprint and the
selected snapshot must expose that change.

The same rule applies to resolver behavior. A delta that changes alias catalog
content, resolver hit/miss policy, resolver digests, normalization pipeline
versions, reviewed decisions that contribute merge edges, or candidate/review
semantics must expose a new effective semantic-map fingerprint. A delta that
only adds new source rows resolved under the inherited mapping and resolver
catalog may inherit the parent semantic-map fingerprint while adding the
materialized COVE-O records and evidence metadata for those rows.

Projection readback follows the selected effective projection catalog. A
projection query may use parent projection rows only when no selected delta can
affect the projected object/property set, or when an overlay-aware projection
patch proves the corrected result.

Implementation ordering:

1. The core delta MVP should not depend on COVE-MAP resolver support. It only
   needs to bind the effective semantic-map fingerprint and preserve ordinary
   COVE-O object reconstruction.
2. COVE-MAP entity-resolution Phase 0 and Phase 1 should land before
   resolver-specific delta evidence/projection patches, so section 69,
   `resolver_digest`, `catalog_digest`, `pipeline_digest`, row-level resolver
   outcomes, and evidence metadata are stable.
3. `DELTA_FEATURE_MAP_EVIDENCE_PATCH`, `DELTA_EVIDENCE_PATCH`, and
   resolver-aware projection patches belong after the resolver catalog MVP.
   Until then, resolver-derived object truth should be represented as ordinary
   COVE-O temporal records in a full snapshot or in the core delta temporal
   sections, with mapping-specific replay/explain metadata omitted or treated
   as unsupported.

## Interaction With COVE-I

COVE-I indexes may be built at three levels:

- base-only indexes for the base `.cove`;
- delta-only indexes for one `.covedelta`;
- snapshot-level indexes for a selected base-plus-delta chain.

Delta-only indexes are cheap to build and should be preferred for frequent
updates. Snapshot-level indexes are more expensive but can answer latest-state
queries without consulting every delta.

Rules:

- COVE-I validity records must include the selected snapshot ID and delta chain
  digest when the index answers snapshot-level queries.
- A snapshot-level index is invalid unless it binds the exact ordered
  `chain_digest`, not merely the base file digest or latest snapshot timestamp.
- Base-only index results must be corrected for delta tombstones, redactions,
  and property changes before being returned for latest-state queries.
- Delta-only indexes may over-include candidates but must not under-include
  when they advertise conservative coverage.
- Missing optional indexes fall back to scan/reconstruction. Required indexes
  reject only the requested operation that requires them.

## Interaction With COVE-L

COVE-L layout metadata remains advisory. Delta-aware layout hints should focus
on reducing object-store request count:

- co-locate small delta temporal segments by object type and branch;
- store touched-object summaries near the header or in a small first range;
- group hot object checkpoint records into contiguous page clusters;
- expose scan splits that keep base reads and delta reads schedulable;
- avoid requiring a layout plan to discover authoritative temporal records.

## Trust And Digest Continuity

Delta trust data should be computed over canonical logical values, not local
FileCodes. A delta should include:

- parent snapshot digest;
- parent effective catalog fingerprint;
- parent semantic-map fingerprint where relevant;
- predecessor state hash for touched objects where available;
- canonical hash of each delta temporal record;
- final state hash for objects with Snapshot/Baseline records;
- chain digest for the ordered delta sequence.

Trust verification should reject:

- changed parent bytes;
- changed parent dictionary entry behind an alias;
- reordered deltas;
- missing predecessor anchor;
- mismatched predecessor state hash;
- duplicate record ID in the selected object/branch scope;
- unexpected CSN gap when the snapshot policy requires contiguous CSNs.

## Efficiency Model

The proposal optimizes for this common case:

```text
large base COVE-O archive
small update batch
small fraction of objects touched
small fraction of properties changed
many values repeat parent dictionary values
queries mostly latest-state and point/range object reads
periodic compaction
```

Efficiency mechanisms:

- No base temporal segments are rewritten.
- Parent catalogs are inherited by fingerprint.
- Parent mapping/projection metadata is inherited by fingerprint.
- The COVM chain summary lets readers prune deltas before issuing per-delta
  blob requests.
- Delta publication and source-publish date ranges support operational sync and
  cache-refresh workflows without scanning every delta object.
- Delta rows store sparse property patches.
- Omitted properties mean unchanged.
- Delta dictionaries store only new values or aliases to parent dictionary
  entries.
- Touched-object sets skip unrelated deltas.
- Property bitmaps skip deltas for unaffected projections.
- Tombstone summaries avoid returning stale parent state.
- Delta temporal indexes prune by object type, branch, CSN, timestamp, and GOID
  range.
- Delta-local COVE-I indexes handle high-selectivity lookups in recent changes.
- Snapshot-level indexes are optional and can be rebuilt asynchronously.
- Checkpoint deltas bound replay cost for hot objects.
- Full compaction resets chain depth and restores self-contained COVE-O reads.

Blob-read efficiency depends on keeping the hot summary path small:

- the selected COVM snapshot and chain summary should fit in a small number of
  range requests;
- per-delta hot summary ranges should be contiguous and optional after COVM
  pruning;
- large exact touched sets may live in deltas or sidecars, but the chain summary
  should still expose enough object type, branch, scope, CSN, commit-time,
  source-publish, and coarse GOID information to avoid opening unrelated delta
  objects;
- compaction or checkpointing should be triggered before ordinary point lookups
  require many delta-object requests.

## Read Amplification Budget

Implementations should expose planning metrics:

```text
delta_chain_depth
chain_summary_bytes
chain_summary_range_requests
selected_delta_count
skipped_delta_count
delta_artifacts_opened
delta_artifacts_skipped_before_open
base_ranges_requested
delta_ranges_requested
touched_set_hits
touched_set_misses
tombstone_summary_hits
source_publish_range_prunes
commit_time_range_prunes
valid_time_summary_prunes
anchor_validations
patch_rows_applied
dictionary_alias_resolutions
materialized_property_count
```

Recommended default policies:

- warn when chain depth exceeds 16;
- recommend checkpointing when one object has more than 32 patches since its
  last Snapshot/Baseline;
- recommend compaction when total delta bytes exceed 20 percent of selected
  base bytes;
- recommend snapshot-level COVE-I when latest-state point lookups touch more
  than 4 artifacts at p95;
- recommend summary hoisting or compaction when point lookups require more than
  2 metadata range requests before any data range is read;
- recommend packing small deltas when the request cost of opening deltas
  dominates bytes returned;
- reject or require explicit override when chain depth exceeds an
  implementation-defined hard limit.

These numbers are starting points, not normative performance claims.

## Failure Semantics

For a selected snapshot:

- unsupported required COVM delta-chain profile: reject;
- unsupported required delta feature: reject the affected selected snapshot
  operation;
- missing required base file: reject;
- missing required delta: reject;
- corrupt required delta: reject;
- stale parent fingerprint: reject;
- invalid continuation anchor: reject;
- invalid dictionary alias used by selected rows: reject;
- missing optional delta-local index: fall back;
- corrupt optional coverage/index/layout metadata: ignore or reject only if the
  requested operation requires it;
- base `.cove` opened directly without selecting the delta-bearing snapshot:
  ordinary base-file behavior applies.

Readers may still open an older COVM snapshot that does not reference the
missing/corrupt delta.

## Security And Governance

Delta artifacts do not make redaction, access control, or visibility mutable
inside a `.cove` file. They only contribute new immutable facts to a selected
snapshot.

Rules:

- Redaction manifests must be inherited by fingerprint or patched explicitly.
- A delta must not reveal parent redacted values through dictionary aliases,
  trust hashes, explain output, or error messages.
- A redaction or tombstone delta changes selected snapshot semantics, but it
  does not remove bytes from the parent artifact. If old values must no longer
  be distributed, publishers must compact into a redacted self-contained `.cove`
  and stop publishing the old base/delta chain to unauthorized readers.
- Visibility overlays remain external unless a required profile defines them.
- Evidence additions must bind to mapping/source fingerprints when explanation
  or audit readback is requested.
- Diagnostics should identify artifact IDs and section kinds, not protected
  object names or values, unless policy allows disclosure.

## Validation Requirements

Delta-aware validation has two modes.

Snapshot-selection validation checks enough metadata to plan and execute a
specific query without opening irrelevant deltas:

- COVM delta-chain extension;
- chain digest;
- chain-summary digest;
- artifact references;
- feature bits;
- effective fingerprints;
- summary descriptors needed by the selected operation.

Query planners may rely on a digest-bound, snapshot-selected chain summary for
pre-open pruning. They are not required to prove the summary against every delta
payload for every query.

Full delta-chain validation additionally opens every selected delta and proves
that chain summaries, touched sets, tombstone sets, anchors, temporal indexes,
and temporal rows agree with payloads. Conformance validators and release gates
should provide this mode.

A delta-aware validator should check:

- header magic, version, lengths, checksums, and section directory;
- postscript/footer consistency and final `CVD2` magic;
- parent refs match selected snapshot and digests;
- exactly one parent ref is marked `DELTA_PARENT_REF_LINEAGE_PARENT` and its
  snapshot ID matches the header parent snapshot ID;
- COVM delta-chain extension binds the exact ordered chain digest;
- COVM chain summary exists, validates by CRC and cryptographic digest, and
  binds the same ordered chain digest;
- chain summary entries are dense, ordered, and match the referenced deltas;
- chain summary time fields preserve the distinction between commit time,
  source publication/ingest batch time, snapshot publication time, and valid
  time summaries;
- chain ordinals are dense and ordered;
- artifact IDs are unique within the selected chain;
- schema/catalog/projection fingerprints are inherited or patched validly;
- catalog patches are additive;
- temporal rows are sorted;
- `csn` and commit timestamp monotonicity hold under the append-only policy;
- `prev_ref` is file-local when present;
- scope and branch identity are explicit or covered by a single-scope header
  invariant;
- continuation anchors target valid logical parent state;
- continuation anchors meet the required anchor strength;
- dictionary aliases resolve through validated parent dictionary digests;
- dictionary aliases do not expose redacted or policy-protected equality unless
  permitted;
- touched sets do not under-include temporal records;
- tombstone summaries cover all delta tombstones;
- chain-summary pruning summaries do not under-include candidate deltas for the
  operations they claim to support in full delta-chain validation;
- trust continuation hashes match when required;
- required feature bits are supported for the requested operation.

## Conformance Tests

Initial conformance vectors should include:

- non-delta-aware COVM reader rejects a delta-bearing snapshot;
- opening the base `.cove` directly still succeeds;
- minimal base plus one delta with one new object;
- sparse property patch against an existing object;
- property `SetValue`, `SetNull`, `Clear`, `Redact`, `Tombstone`, and omitted
  unchanged property;
- object tombstone hiding parent latest state;
- association/link object update;
- evidence addition with inherited COVE-MAP fingerprint;
- additive object catalog patch;
- invalid parent digest rejection;
- correct artifact digest but wrong `parent_snapshot_id` rejection;
- multiple or missing lineage parent refs rejected;
- delta chain reorder rejected by chain digest;
- missing or corrupt required chain summary rejects the delta-bearing snapshot;
- chain summary with wrong chain digest rejected;
- chain summary under-includes an affected delta and validator rejects;
- source publication range filter prunes operationally but does not alter
  `as_of_csn` or valid-time semantics;
- multi-scope GOID collision does not cross-apply anchors or tombstones;
- raw parent FileCode branch key is not accepted as cross-artifact branch
  identity;
- duplicate record ID across selected scope/object/branch rejected;
- missing continuation anchor rejection;
- continuation anchor below required strength rejected;
- touched set under-includes one patched object and validator rejects;
- tombstone summary under-includes one tombstone and validator rejects;
- corrupt optional delta-local index fallback;
- `as_of_csn` cut before, inside, and after delta CSN range;
- `as_of_valid_time` does not use `csn_min` pruning unless a valid-time summary
  exists;
- compaction equivalence: `base + deltas` equals compacted `.cove`;
- COVE-I base-only result corrected by delta tombstone;
- snapshot-level index with stale chain digest rejected;
- touched-set exact skip for point lookup;
- property bitmap skip for projection readback;
- chain summary digest mismatch rejected;
- continuation anchor state hash recomputation matches stored state hash;
- unsupported required delta feature rejects the selected snapshot.

Second-tier conformance vectors should add:

- parent dictionary alias for a repeated string value;
- invalid dictionary alias rejection;
- parent dictionary alias to redacted value rejected or policy-gated;
- compaction reassigns FileCodes but preserves canonical trust state;
- corrupt optional layout/index/coverage section falls back.

## Benchmark Plan

Benchmarks should compare:

- full rewrite per update batch;
- additional full `.cove` files plus manifest merge;
- COVE-O delta artifacts plus periodic compaction.

Measure:

- bytes written per update;
- total bytes stored;
- writer finalization latency;
- publication latency on local FS and object-store harness;
- latest-state point lookup p50/p95/p99;
- object history query cost;
- projection readback cost;
- object-store range request count;
- COVM/chain-summary range request count;
- delta artifacts skipped before opening;
- delta artifacts opened per point lookup;
- source publication range pruning effectiveness;
- dictionary alias resolution cost;
- compaction throughput and output size;
- index rebuild cost;
- recovery/validation time for selected snapshots.

The benchmark matrix should vary:

- base object count;
- touched object percentage;
- changed property percentage;
- delta chain depth;
- dictionary value reuse rate;
- tombstone rate;
- query selectivity;
- object type count;
- branch count.

## Open Questions

- Should `.covedelta` reuse exact COVE section encodings for temporal segments,
  or use a smaller artifact-native segment grammar?
- Should the COVM delta-chain block become a core COVM section or remain an
  extension block with required profile bits?
- Should checkpoint deltas have a distinct artifact kind, or remain ordinary
  deltas with Snapshot/Baseline-heavy payloads?
- What is the right portable hard limit for chain depth?
- How much schema evolution should be allowed before a new base `.cove` is
  required?
- Should snapshot-level COVE-I indexes be recommended after compaction only, or
  also for long-lived chains?
- What maximum chain-summary size should this object-store-aware profile
  recommend
  before forcing summary sidecars, delta packing, checkpointing, or compaction?
- Should source publication/ingest date ranges be represented only in COVM, or
  also standardized as queryable COVE-O operational metadata?

Resolved in the reference prototype:

- Parent dictionary aliases are supported for selected base and prior-delta
  dictionaries. Redacted parent aliases preserve the configured redaction
  policy and do not expose protected values as ordinary decoded values.

## Recommended First Implementation

The smallest useful implementation should support:

1. COVM required extension block for an ordered base-plus-delta chain.
2. Mandatory COVM chain summary with chain digest, per-delta CSN range, commit
   time range, source publication/ingest range, object type summary, branch
   summary, scope summary, touched summary refs, and tombstone summary refs.
3. Delta header, postscript/footer, section directory, and checksums.
4. Mandatory cryptographic digest for base and delta references.
5. Effective catalog, schema, projection, map, visibility, and redaction
   fingerprints.
6. Inline delta dictionary overlays and parent dictionary aliases.
7. Delta-local temporal segment index and temporal segment data.
8. Sparse `SetValue`, `SetNull`, `Clear`, `Redact`, and `Tombstone` property
   operations.
9. Object tombstones.
10. Exact touched-object set.
11. Exact tombstone set.
12. Continuation anchors with scope, branch identity, predecessor CSN, record
    ID, and state hash.
13. Append-only CSN/commit-time policy.
14. Compaction equivalence test to a new self-contained `.cove`.

The next implementation tier should add:

1. Hash/equality dictionary hints with redaction policy gates.
2. Property-level touched bitmaps.
3. Delta-local COVE-I indexes.
4. Delta coverage patches.
5. Additional checkpoint-delta planning heuristics.
6. COVE-MAP evidence/projection patches, after the COVE-MAP resolver catalog
   MVP has stable section IDs, digest semantics, and evidence metadata.
7. Object-store layout hints.
8. Snapshot-level COVE-I indexes.
9. Small-delta packing when request cost dominates bytes returned.

Deferred until separate required extensions:

1. Historical commit-order inserts or corrections.
2. Probabilistic touched summaries for proof-of-absence.
3. Projection patches that are authoritative for ordinary object truth.
4. Merge DAGs or multi-writer delta branches.

This sequence keeps the first version correct and useful without requiring the
most aggressive storage optimizations on day one.

## Positioning

COVE-O delta artifacts are not a replacement for `.cove` files. They are a
publication and efficiency layer for selected object-temporal snapshots.

The compatibility story remains simple:

```text
Need broad reader compatibility:
  compact to a self-contained .cove

Need efficient incremental publication:
  publish .covedelta artifacts and a new COVM snapshot

Need current generic COVE-Core/COVE-T behavior:
  ignore .covedelta
```

This preserves the central COVE promise: immutable artifacts define validated
truth, and optional companion artifacts improve planning or publication only
when their contracts are explicitly selected and validated.
