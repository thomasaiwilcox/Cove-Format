# COVM Dataset Manifest and COVE-CACHE

## 69. COVM Dataset Manifest

COVM is an optional multi-file dataset manifest.
**COVM final bytes:**
[postscript bytes]
[postscript_version: u16]
[postscript_len: u16]
[magic: "CVM2"]

### 69.1 COVM Header

```rust
struct CovmHeaderV2 {
    magic: [u8; 4],          // "CVM2"

    header_len: u16,
    version_major: u16,
    version_minor: u16,

    flags: u32,

    dataset_id: [u8; 16],

    table_count: u32,
    file_count: u32,

    created_at_us: i64,

    reserved: [u8; 32],

    checksum: u32,
}
```

### 69.2 Manifest File Entry

```rust
struct CovmFileEntryV2 {
    file_id: [u8; 16],

    uri_len: u16,
    uri: [u8],

    file_len: u64,

    footer_crc32c: u32,

    digest_algorithm: u16,
    digest_len: u16,
    digest: [u8; digest_len],

    row_count: u64,
    segment_count: u32,

    file_stats_ref: u32,
    file_exact_set_ref: u32,

    flags: u32,
}
```

**COVM MAY contain:**
- table schema fingerprints,
- partition values,
- file-level min/max,
- file-level domain ranges,
- file-level exact sets,
- dictionary fingerprints,
- COVX references,
- COVE-MAP artifact references, including projection catalogs where present,
- COVE-I secondary index references, including root indexes and index validity,
- COVE-COVERAGE provider references and coverage set summaries,
- COVE-CACHE compatibility and invalidation hints when a runtime cache is permitted by policy,
- COVE-O delta-chain extensions, ordered `.covedelta` artifact references, and digest-bound chain summaries,
- object-store hints.
**Rules:**
- COVM MUST be ignored if stale.
- COVM MUST NOT change COVE semantics.
- Query planners MAY use COVM to prune files before opening COVE footers.

### 69.3 COVM Publication and Atomic Update Discipline

COVM describes a dataset state. Updating a dataset means publishing a new dataset state; it MUST NOT mutate the logical meaning of any referenced immutable COVE file.
**Preferred publication model:**
1. write a complete new COVM object or file;
2. validate its header, section directory, footer/postscript, checksums, and referenced COVE digests when present;
3. publish it by an atomic rename, catalog pointer update, compare-and-swap object metadata update, or other external atomic reference mechanism.
**Rules:**
- COVM readers MUST validate freshness using referenced file_id, file_len, footer_crc32c, and digest fields when digest_algorithm is not None before trusting manifest pruning.
- A stale, corrupt, partially written, or unsupported COVM MUST be ignored; readers MUST fall back to opening COVE files directly.
- The dual-root/header rotation pattern MAY be used only as an optional local-filesystem publication protocol for mutable COVM pointers. It MUST NOT be required for canonical .covm objects and MUST NOT be applied to immutable COVE data files.
- If a local mutable COVM pointer file uses dual roots, each root slot MUST include a generation counter, COVM location or footer section spec, file length, digest or CRC, and checksum. Readers MUST choose the highest-generation root that fully validates; if neither validates, the COVM pointer is ignored.
- Object-store deployments SHOULD prefer immutable COVM objects plus an atomic catalog/reference update over in-place 4 KiB header writes.


### 69.4 COVM References to COVE-MAP

COVM MAY reference COVE-MAP artifacts for lineage, planning, conversion replay, or explanation.

**A COVM mapping reference SHOULD include:**
- mapping artifact URI or logical reference,
- mapping artifact digest and digest algorithm,
- mapping_id and mapping_version,
- source-set identity or source-load digest,
- output COVE file IDs produced by the mapping run,
- mapping execution ID,
- conversion report reference.

**Rules:**
- COVM MUST NOT be the sole authority for semantic mapping rules unless a future required profile explicitly defines that behaviour.
- COVM MUST NOT change the logical meaning of referenced COVE files.
- If a COVM mapping reference is stale, corrupt, or unsupported, readers MUST still be able to read the referenced COVE files. Only mapping replay/explanation operations fail or degrade.
- A COVE-MAP converter SHOULD emit a COVM dataset manifest when it materialises more than one output COVE file or when source lineage must be preserved across a dataset.

#### 69.4.1 COVM Delta-Chain Extension

A COVM or equivalent external catalog snapshot selects a COVE-O base-plus-delta snapshot through a required delta-chain extension block.

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

The chain digest is mandatory. It MUST bind the dataset ID; base artifact ID, length, footer CRC, digest, and base snapshot ID; ordered delta artifact IDs, lengths, footer CRCs, digests, and ordinals; result snapshot ID; required delta feature bits; and effective schema, object catalog, projection, semantic-map, visibility, and redaction fingerprints.

**Rules:**
- `ordered_delta_artifact_refs` is selected snapshot truth. A reader MUST NOT add newer deltas because they exist next to the selected files.
- A reader that does not support `delta_chain_profile_id`, `delta_chain_profile_version_*`, or any required delta feature bit MUST reject the selected snapshot for object-temporal reads.
- A reader MAY open the base `.cove` directly only when the user/request selects the base artifact rather than the delta-bearing dataset snapshot.
- Snapshot-level COVE-I/COVX indexes, coverage providers, zero-copy/runtime hints, and COVE-CACHE entries MUST bind the exact `chain_digest`, not only the base file digest or latest timestamp.
- A selected chain with a mismatched chain digest, missing delta, extra delta, reordered delta, mismatched effective fingerprint, or unsupported required feature is invalid.
- `chain_summary_*` MUST identify a compact, checksum-validated summary sufficient to prune deltas before opening individual `.covedelta` objects for common COVE-O object-temporal reads.
- CRC32C detects accidental transfer/storage corruption. The cryptographic `chain_summary_digest_*` fields bind summary bytes as selected snapshot metadata. A reader MUST NOT use an external or separately addressable chain summary for pruning unless its cryptographic digest validates.
- Object-store-oriented COVM writers SHOULD place the chain summary in the same small COVM range as selected snapshot metadata or in one separately addressable summary object. A reader SHOULD NOT need one blob request per delta merely to discover that most deltas are irrelevant.

The authoritative precedence for a delta-bearing COVM snapshot is: COVM delta-chain extension for selected snapshot identity; chain digest for ordered base-plus-delta chain; chain summary for pre-open pruning after its digest validates; delta postscript/footer for locating delta bytes; delta header for declared lineage, feature bits, effective metadata fingerprints, scope policy, and commit-time range; section directory entries for section byte ranges; and section payloads for their local reference spaces after checksum and required-feature validation. Any duplicated field that disagrees across authoritative surfaces MUST reject the selected snapshot or requested operation, except where the field is explicitly declared over-inclusive summary metadata.

#### 69.4.2 COVM Delta Chain Summary

The chain summary is the blob-cost control plane for a delta-bearing snapshot. It is selected and validated through `CovmDeltaChainExtensionV1`, then used to decide which delta artifacts must be opened for a requested object-temporal operation.

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

**Rules:**
- The summary MUST bind the same ordered `chain_digest` as the selected COVM delta-chain extension.
- Summary entries MUST be dense and sorted by `chain_ordinal`.
- Time fields MUST preserve the distinction between COVE-O commit time, source publication/ingest batch time, artifact creation time, selected snapshot publication time, and valid-time or other temporal-role summaries.
- `time_field_presence_flags` declares which non-commit time fields are meaningful. Missing optional time fields MUST NOT be inferred from commit time.
- `time_summary_exactness_flags` declares whether a summary is exact, conservative over-inclusive, or unavailable for each advertised time dimension. Unknown exactness values make that summary unusable for pruning.
- Scope, branch, object type, GOID, touched, tombstone, property, and temporal-role summary references MUST resolve to validated summary descriptors. A corrupt or unsupported summary MUST NOT be used for pruning.
- Chain-summary pruning may over-include deltas. It MUST NOT under-include any delta that could affect the selected object-temporal result.
- Full delta-chain validation MUST prove that chain-summary entries agree with the referenced delta headers and payload summaries for every pruning claim the summary makes.

---

### 69.5 COVE-CACHE Runtime Coverage Cache

COVE-CACHE is an optional mutable runtime/local cache for predicate coverage sets. It is intentionally not part of immutable `.cove` logical truth and SHOULD NOT be stored inside a `.cove` file.

**Local persistence boundary:**
COVE-CACHE does not define a normative `.cove`-family artifact, magic value, or durable publication protocol in v2. An implementation MAY persist cache entries in a local store, but that store is engine-owned, mutable, revocable, and outside COVE logical truth. The structure below is a recommended diagnostic/interop record shape for implementations that expose cache state; it is not a canonical artifact header.

```rust
struct CoveCoverageCacheHeaderV2 {
    cache_format_namespace_ref: u32,
    cache_format_version_major: u16,
    cache_format_version_minor: u16,
    flags: u32,
    cache_id: [u8; 16],
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    entry_count: u32,
    created_at_us: i64,
    producer_engine_ref: u32,
    reserved: [u8; 32],
    checksum: u32,
}

struct CoverageCacheEntryV2 {
    entry_id: u64,
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    predicate_normal_form_ref: u32,
    interval_normal_form_ref: u32,
    coverage_set_ref: u32,
    coverage_granularity: u8,
    proof_strength: u8,
    exactness: u8,
    flags: u8,
    actual_coverage_size_bytes: u64,
    actual_read_cost_ns: u64,
    created_at_us: i64,
    valid_until_snapshot_ref: u32,
    producer_engine_ref: u32,
    checksum: u32,
}
```

**Invalidation triggers:**

A COVE-CACHE entry MUST be invalidated when any of the following changes unless a required extension proves the cached coverage remains conservative:

- selected dataset snapshot;
- COVM publication state;
- referenced `.cove` file list, file length, footer CRC, or digest;
- selected `.covedelta` chain, chain digest, chain summary digest, or effective delta-chain fingerprints;
- schema fingerprint;
- external delete or visibility overlay;
- COVE-MAP mapping/projection version;
- COVE-I or COVX sidecar version used to build the entry;
- semantic dimension definition;
- collation or deterministic function version;
- policy governing redaction or protected metadata.

**Rules:**
- COVE-CACHE may improve planning, but it is never canonical truth.
- A reader MUST be able to ignore COVE-CACHE and still read correct logical values from `.cove` files and validated required artifacts.
- COVE-CACHE entries MAY be used only for the dataset snapshot and predicate context for which they are valid.
- COVE-CACHE entries that are stale, corrupt, unsupported, engine-local to another incompatible engine, or approximate-may-under-include MUST NOT be used for pruning.
- COVE-CACHE may store predicate containment relationships, interval normal forms, and actual observed coverage costs, but these are planning hints unless the stored coverage set itself is validated as conservative.

---
