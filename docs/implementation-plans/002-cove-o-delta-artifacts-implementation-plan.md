# COVE-O Delta Artifacts Implementation Plan

Status: implementation plan

Derived from:

- [COVE-O Delta Artifacts](../proposals/cove-o-delta-artifacts.md)
- [COVE-MAP Entity Resolution Implementation Plan](./001-covemap-entity-resolution-implementation-plan.md)
- [Cross-Feature Implementation Sequence](./000-cross-feature-implementation-sequence.md)

## Objective

Add immutable COVE-O delta artifacts as an optional publication and efficiency
layer for selected object-temporal snapshots:

```text
base.cove + delta-0001.covedelta + delta-0002.covedelta + ...
```

Following this plan in numbered order must produce a delta-aware COVM snapshot
selection path, `.covedelta` binary envelope, chain summaries, temporal sparse
patches, anchors, touched/tombstone summaries, reconstruction, compaction,
validation, conformance fixtures, and second-tier/deferred extension boundaries.

## Implementation Contract

1. `.cove` files remain immutable and self-contained.
2. `.covedelta` files are immutable and complete only after footer,
   postscript, checksums, and digests validate.
3. Deltas are never selected by filename discovery. They are selected only by a
   COVM snapshot or an external catalog snapshot.
4. A selected delta chain is ordered. Reordering must change the selected
   snapshot and fail chain-digest validation.
5. Missing, corrupt, reordered, stale, or unsupported required deltas reject the
   selected snapshot or selected operation.
6. A reader that opens a delta-bearing selected snapshot must not silently
   return base-only object-temporal data.
7. Raw FileCodes remain artifact-local. Cross-artifact equality uses canonical
   values, canonical hashes with collision resolution, or digest-bound
   dictionary aliases.
8. Cross-artifact continuity uses logical continuation anchors, not cross-file
   COVE-O `prev_ref`.
9. The first profile is append-only in commit order. Historical commit-order
   insertions are a deferred required extension.
10. Branch identity across artifacts is canonical branch identity, not a raw
    artifact-local FileCode.

## Primary Code Surfaces

Inspect current code before editing. Expected primary surfaces:

- `crates/cove-core/src/artifact/covm.rs`
- `crates/cove-core/src/artifact/mod.rs`
- `crates/cove-core/src/constants.rs`
- `crates/cove-core/src/registry.rs`
- `crates/cove-core/src/footer.rs`
- `crates/cove-core/src/postscript.rs`
- `crates/cove-core/src/checksum.rs`
- `crates/cove-core/src/digest.rs`
- `crates/cove-core/src/dictionary.rs`
- `crates/cove-core/src/profile/cove_o.rs`
- `crates/cove-core/src/profile/cove_o/readback.rs`
- `crates/cove-core/src/trust_chain.rs`
- `crates/cove-core/src/redaction.rs`
- `crates/cove-reader/src/lib.rs`
- `crates/cove-writer/src/lib.rs`
- `crates/cove-validate/src/validate.rs`
- `crates/cove-dump/src/dump.rs`
- `crates/cove-conformance/src/main.rs`
- `crates/cove-conformance/src/gen_corpus_support.rs`
- `crates/cove-coverage/src/lib.rs`
- `crates/cove-index` if delta-local or snapshot-level COVE-I support is added
- `crates/cove-layout` if object-store layout hints are added
- `crates/cove-map` only for resolver-aware evidence/projection patches after
  plan 001 is stable

## Non-Goals

Do not implement these in the core MVP:

1. In-place append to finalized `.cove` files.
2. Mutable footer, section directory, dictionary, or page index.
3. Implicit transaction log by scanning directories.
4. Cross-file `prev_ref` in ordinary COVE-O temporal rows.
5. Relaxation of COVE-O self-containment for ordinary `.cove` files.
6. General ACID table protocol, concurrent writer protocol, or lakehouse
   catalog replacement.
7. Hidden comparison of raw FileCodes from different artifacts.
8. Base-only answer for a selected snapshot that declares required deltas.
9. Historical commit-order insertions.
10. Merge DAGs or multi-writer delta branches.

## Phase 0: Open Decisions And Registry Preparation

Resolve or explicitly defer each proposal open question before coding the
dependent phase:

1. Whether `.covedelta` reuses exact COVE section encodings for temporal
   segments or uses artifact-native segment grammar.
2. Whether parent dictionary aliases are tier 2 or wait until after
   snapshot-level indexing.
3. Whether the COVM delta-chain block is a core COVM section or a required
   extension block.
4. Whether checkpoint deltas remain ordinary `.covedelta` artifacts or get a
   distinct artifact kind.
5. Portable hard limit for chain depth.
6. How much schema evolution is allowed before requiring a new base `.cove`.
7. Whether snapshot-level COVE-I indexes are recommended after compaction only
   or also for long-lived chains.
8. Maximum chain-summary size before summary sidecars, delta packing,
   checkpointing, or compaction.
9. Whether source publication/ingest ranges live only in COVM or also become
   standardized queryable COVE-O operational metadata.

Initial defaults for implementation:

1. Use extension `.covedelta`.
2. Use magic `CVD2`.
3. Use a required COVM delta-chain extension unless the registry accepts a core
   COVM section before implementation starts.
4. Keep checkpoint deltas as ordinary deltas with Snapshot/Baseline-heavy
   payloads.
5. Parent dictionary aliases are a tier-2 surface and are implemented after the
   MVP parser/reconstruction path.
6. Defer historical commit-order insertion.

Resolved implementation decisions:

1. `.covedelta` object-temporal sections reuse the existing COVE-O
   `TemporalSegmentData` grammar for temporal payloads, with delta-native
   envelope, parent, descriptor, sparse patch, touched-set, tombstone-set, and
   state-hash sections layered around it.
2. Dictionary overlays support inline values, parent dictionary aliases, and
   non-materializing canonical hash hints with fail-closed validation for
   invalid refs, missing feature gates, and zero hash hints.
3. The COVM delta-chain selector is implemented as a required COVM extension
   profile rather than a core COVM section.
4. Checkpoints remain ordinary `.covedelta` artifacts using the checkpoint
   baseline feature and Snapshot/Baseline temporal rows.
5. `CovmDeltaReadAmplificationPolicy::default()` sets the portable operational
   limits: warn at chain depth 16, require override after depth 64, recommend
   checkpoints after 32 patch rows, and recommend compaction at 20% delta/base
   byte ratio.
6. Schema/catalog/projection/semantic-map/visibility/redaction evolution is
   represented by effective fingerprints. The MVP accepts additive catalog
   patches and otherwise requires a new compatible base snapshot.
7. Snapshot-level COVE-I is optional and digest-bound to the exact chain
   digest. Delta-aware COVE-I correction is implemented for base tombstone
   overlays; stale chain digest fixtures fail closed.
8. Chain-summary size and read amplification are controlled by summary bytes,
   range-request metrics, checkpoint recommendations, compaction
   recommendations, and small-delta packing recommendations in
   `CovmDeltaReadAmplificationPolicy`.
9. Source publication ranges live in the delta header and COVM chain summary
   for operational pruning metrics; they are not standardized as ordinary
   queryable COVE-O object metadata in the MVP.

Acceptance:

1. Open decisions needed for Phase 1 through Phase 7 are documented.
2. Constants for `CVD2`, postscript versions, feature bits, and section kinds
   are registered or scaffolded behind explicit unsupported-feature errors.
3. Non-delta-aware readers reject selected delta-bearing COVM snapshots for
   object-temporal reads.

## Phase 1: COVM Delta-Chain Snapshot Selection

### 1.1 Implement Delta-Chain Extension

Add `CovmDeltaChainExtensionV1` with proposal fields:

```text
delta_chain_profile_id
delta_chain_profile_version_major
delta_chain_profile_version_minor
required_delta_features
optional_delta_features
dataset_id
base_snapshot_id
result_snapshot_id
base_artifact_ref
ordered_delta_count
ordered_delta_artifact_refs_offset
ordered_delta_artifact_refs_length
chain_digest_algorithm
chain_digest_len
chain_digest_ref
chain_summary_kind
chain_summary_ref
chain_summary_offset
chain_summary_length
chain_summary_crc32c
chain_summary_digest_algorithm
chain_summary_digest_len
chain_summary_digest_ref
effective_schema_fingerprint_ref
effective_object_catalog_fingerprint_ref
effective_projection_fingerprint_ref
effective_semantic_map_fingerprint_ref
effective_visibility_fingerprint_ref
effective_redaction_fingerprint_ref
csn_min
csn_max
created_at_us
checksum
```

### 1.2 Bind Chain Digest

The chain digest must bind:

1. dataset ID.
2. base artifact ID, length, footer CRC, digest, and base snapshot ID.
3. ordered delta artifact IDs, lengths, footer CRCs, digests, and ordinals.
4. result snapshot ID.
5. required delta feature bits.
6. effective schema, object catalog, projection, semantic-map, visibility, and
   redaction fingerprints.

Rules:

1. `ordered_delta_artifact_refs` is snapshot truth.
2. Readers must not add newer deltas found beside selected files.
3. Unsupported profile ID, version, or required feature bit rejects selected
   object-temporal reads.
4. Snapshot-level COVE-I/COVX indexes, coverage providers, and caches bind the
   exact `chain_digest`.

### 1.3 Implement Artifact Reference Validation

For base and deltas, validate:

1. artifact ID.
2. file length.
3. footer CRC.
4. mandatory cryptographic digest.
5. ordinal.
6. parent snapshot identity.

Acceptance:

1. Missing, extra, reordered, or digest-mismatched delta chains reject.
2. Opening the base `.cove` directly still has ordinary base-file behavior.
3. Selecting a COVM snapshot with required deltas fails closed for unsupported
   object-temporal reads.

## Phase 2: Chain Summary And Blob-Cost Control Plane

### 2.1 Implement `CovmDeltaChainSummaryV1`

The summary magic is `CDS1`. Encode and parse proposal fields:

```text
magic
version_major
version_minor
header_len
flags
dataset_id
result_snapshot_id
chain_digest_algorithm
chain_digest_len
chain_digest_ref
delta_summary_count
object_type_summary_count
branch_summary_count
temporal_role_summary_count
delta_summaries_offset
object_type_summaries_offset
branch_summaries_offset
temporal_role_summaries_offset
payload_offset
payload_length
checksum
```

### 2.2 Implement `DeltaChainSummaryEntryV1`

Encode and parse proposal fields:

```text
chain_ordinal
delta_artifact_ref
delta_artifact_id
required_delta_features
optional_delta_features
csn_min
csn_max
commit_time_start_us
commit_time_end_us
artifact_created_at_us
first_published_at_us
selected_snapshot_published_at_us
time_field_presence_flags
time_summary_exactness_flags
source_publish_range_start_us
source_publish_range_end_us
scope_summary_ref
branch_summary_ref
object_type_summary_ref
goid_range_summary_ref
touched_summary_ref
tombstone_summary_ref
property_summary_ref
temporal_role_summary_ref
delta_header_range_offset
delta_header_range_length
hot_summary_range_offset
hot_summary_range_length
checksum
```

### 2.3 Enforce Summary Rules

1. Summary binds the same ordered chain digest as the COVM extension.
2. Summary entries are dense and sorted by `chain_ordinal`.
3. Commit-time fields describe COVE-O commit/file-ordering `timestamp_us`.
4. Source publication/ingest ranges are operational metadata and not commit
   time, valid time, or business time.
5. Valid-time pruning uses `temporal_role_summary_ref`, not CSN or commit time.
6. Optional time fields are ignored unless their presence flag is set.
7. Exactness flags control proof-of-absence behavior.
8. `scope_summary_ref`, `branch_summary_ref`, `object_type_summary_ref`, and
   `goid_range_summary_ref` may over-include but must not under-include.
9. `touched_summary_ref` and `tombstone_summary_ref` are exact in the MVP when
   used for latest-state point lookup skipping.
10. Corrupt, stale, missing, or unsupported required summaries reject selected
    object-temporal reads.

### 2.4 Summary Representations

Support MVP representations:

1. sorted object type IDs per delta.
2. canonical branch identity refs per delta.
3. scope refs or single-scope flags per delta.
4. GOID min/max ranges per object type/branch/scope.
5. exact touched-object set refs for small deltas.
6. exact tombstone set refs for small deltas.
7. property bitmaps by object type when Phase 9 adds them.
8. temporal-role min/max summaries for valid-time pruning.

Defer probabilistic summaries until a separate required extension defines
proof-of-absence behavior.

Acceptance:

1. Selected COVM snapshot plus chain summary can decide which deltas may need
   opening for common point, object-type, branch, CSN, and commit-time reads.
2. A reader does not need one blob request per delta merely to discover most
   deltas are irrelevant.
3. Full validation rejects chain summaries that under-include affected deltas.

## Phase 3: `.covedelta` Binary Envelope

### 3.1 Implement Tail Discovery

Use:

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

Rules:

1. Little-endian integers.
2. Explicit lengths and offsets.
3. No native struct padding.
4. Checksums computed with checksum field zeroed.
5. Final magic must be `CVD2`.

### 3.2 Implement Postscript, Footer, Section Directory

Add `CoveDeltaPostscriptV1`:

```text
required_delta_features
optional_delta_features
file_len
footer_offset
footer_length
checksum
```

Add `CoveDeltaFooterV1`:

```text
header_offset
header_length
section_directory_offset
section_directory_length
section_count
parent_ref_count
footer_crc32c
checksum
```

Add `CoveDeltaSectionDirectoryEntryV1`:

```text
section_id
section_kind
flags
offset
length
uncompressed_length
item_count
compression
encryption
alignment_log2
reserved0
required_delta_features
optional_delta_features
crc32c
checksum
```

Rules:

1. Section offsets and lengths are relative to the start of the `.covedelta`
   artifact.
2. `section_kind` is the section-kind enum.
3. `section_id` is a unique per-artifact section instance ID.
4. Unknown required features reject only operations that need them, except
   unknown required semantics for temporal rows, sparse patches, anchors,
   tombstones, or required summaries reject object-temporal reads.

Acceptance:

1. Corrupt postscript, footer, header, or section directory rejects.
2. Optional corrupted index/layout/coverage sections fall back when the
   requested operation does not require them.

## Phase 4: Delta Header, Parent References, Descriptors, And Feature Bits

### 4.1 Implement `CoveDeltaHeaderV1`

Encode and parse proposal fields:

```text
magic
version_major
version_minor
header_len
flags
required_delta_features
optional_delta_features
delta_artifact_id
dataset_id
snapshot_id
parent_snapshot_id
chain_ordinal
chain_depth
parent_ref_count
section_count
csn_min
csn_max
commit_time_range_start_us
commit_time_range_end_us
scope_kind
reserved0
scope_id
object_catalog_fingerprint_ref
schema_fingerprint_ref
semantic_map_fingerprint_ref
projection_fingerprint_ref
section_directory_offset
section_directory_length
parent_refs_offset
parent_refs_length
created_at_us
source_publish_range_start_us
source_publish_range_end_us
checksum
```

Header flags:

1. `0x0000_0001` = `DELTA_FLAG_SINGLE_SCOPE`.
2. `0x0000_0002` = `DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT`.

Rules:

1. `chain_ordinal` is dense in the selected snapshot chain.
2. `chain_depth` includes the current delta.
3. CSNs and commit timestamp range must advance beyond selected parent
   high-water mark under the append-only profile.
4. Effective fingerprints describe metadata after applying the delta.
5. Zero or absent fingerprint inherits parent metadata.
6. Single-scope deltas force all temporal records, anchors, touched sets, and
   tombstone summaries into the declared scope.

### 4.2 Implement Parent References

Add `DeltaParentRefV1`:

```text
parent_ref
parent_kind
flags
artifact_id
snapshot_id
file_len
footer_crc32c
digest_algorithm
digest_len
digest_ref
uri_ref
schema_fingerprint_ref
object_catalog_fingerprint_ref
semantic_map_fingerprint_ref
projection_fingerprint_ref
checksum
```

Rules:

1. Parent flag `0x0000_0001` = `DELTA_PARENT_REF_LINEAGE_PARENT`.
2. Exactly one parent ref is lineage parent and matches
   `parent_snapshot_id`.
3. Base and parent-delta refs require digest algorithm, digest length, and
   digest ref.
4. Parent URI is advisory; digest and fingerprints are authoritative.
5. Future merge DAGs require a separate required extension.

### 4.3 Implement Section Kinds

Register or model these delta section kinds:

| Kind | Section |
| ---: | --- |
| 0 | `DELTA_PARENT_REFS` |
| 1 | `DELTA_CATALOG_PATCH` |
| 2 | `DELTA_DICTIONARY_OVERLAY` |
| 3 | `DELTA_TEMPORAL_SEGMENT_INDEX` |
| 4 | `DELTA_TEMPORAL_SEGMENT_DATA` |
| 5 | `DELTA_CONTINUATION_ANCHORS` |
| 6 | `DELTA_TOUCHED_OBJECT_SET` |
| 7 | `DELTA_TOMBSTONE_SET` |
| 8 | `DELTA_PROPERTY_OPS` |
| 9 | `DELTA_EVIDENCE_PATCH` |
| 10 | `DELTA_PROJECTION_PATCH` |
| 11 | `DELTA_COVERAGE_PATCH` |
| 12 | `DELTA_INDEX_HINTS` |
| 13 | `DELTA_LAYOUT_HINTS` |
| 14 | `DELTA_TRUST_CONTINUATION` |
| 15 | `DELTA_STRING_TABLE` |
| 16 | `DELTA_BRANCH_IDENTITY_TABLE` |
| 17 | `DELTA_SCOPE_TABLE` |
| 18 | `DELTA_TEMPORAL_ROLE_SUMMARY_TABLE` |
| 19 | `DELTA_TOUCHED_SUMMARY_TABLE` |
| 20 | `DELTA_TOMBSTONE_SUMMARY_TABLE` |
| 21 | `DELTA_STATE_HASH_TABLE` |
| 255 | `DELTA_EXTENSION` |

Only temporal segment sections plus metadata required to validate them are
required for a minimal object delta. Index, coverage, layout, and projection
patches are optional unless a requested operation requires them.

### 4.4 Implement Descriptor Tables

Add descriptor support:

1. `DeltaScopeDescriptorV1`.
2. `DeltaSummaryDescriptorV1`.
3. `DeltaStateHashDescriptorV1`.

Descriptor kinds:

1. `ExactSortedSet`.
2. `ExactRangeSet`.
3. `ConservativeRange`.
4. `NoFalseNegativeBloom`.
5. `PropertyBitmap`.
6. `TemporalRoleRange`.
7. `Extension`.

State-hash kinds:

1. `CoveObjectDeltaStateHashV1`.
2. `CoveOTrustHash`.
3. `Extension`.

Rules:

1. Descriptor refs are dense zero-based indexes into matching descriptor tables
   unless a section says otherwise.
2. Unsupported required descriptor kinds reject operations that need them.

### 4.5 Implement Feature Bits

Register feature bits:

| Bit | Feature |
| ---: | --- |
| 0 | `DELTA_FEATURE_SPARSE_PATCH_ROWS` |
| 1 | `DELTA_FEATURE_OBJECT_TOMBSTONES` |
| 2 | `DELTA_FEATURE_PROPERTY_TOMBSTONES` |
| 3 | `DELTA_FEATURE_ASSOCIATION_TOMBSTONES` |
| 4 | `DELTA_FEATURE_CONTINUATION_ANCHORS` |
| 5 | `DELTA_FEATURE_INLINE_DICTIONARY` |
| 6 | `DELTA_FEATURE_PARENT_DICTIONARY_ALIASES` |
| 7 | `DELTA_FEATURE_EXACT_TOUCHED_SET` |
| 8 | `DELTA_FEATURE_EXACT_TOMBSTONE_SET` |
| 9 | `DELTA_FEATURE_CHECKPOINT_BASELINES` |
| 10 | `DELTA_FEATURE_COVERAGE_PATCH` |
| 11 | `DELTA_FEATURE_INDEX_HINTS` |
| 12 | `DELTA_FEATURE_MAP_EVIDENCE_PATCH` |
| 13 | `DELTA_FEATURE_PROJECTION_PATCH` |
| 14 | `DELTA_FEATURE_HISTORICAL_COMMIT_INSERT` |

MVP required bits:

1. sparse patch rows.
2. object tombstones.
3. continuation anchors.
4. inline dictionary.
5. exact touched set.
6. exact tombstone set.

Historical commit insert is deferred and must not be required by the first
profile.

Acceptance:

1. Required feature support is enforced by selected operation.
2. Optional unsupported features fall back when safe.
3. Required unsupported semantics reject object-temporal reads.

## Phase 5: Catalog Patches, Dictionaries, Branch Identity, And Temporal Rows

### 5.1 Implement Additive Catalog Patches

Effective catalog:

```text
EffectiveCatalog(delta_n) =
  ApplyAdditivePatch(EffectiveCatalog(parent), delta_n.catalog_patch)
```

Allowed additions:

1. new object type.
2. new property on existing object type.
3. new association/link/evidence/projection object type.
4. new temporal role binding.
5. new branch alias.
6. new projection definition that depends only on declared IDs.

Reject:

1. duplicate object type IDs.
2. duplicate property IDs within one object type.
3. changed logical type for inherited property.
4. changed collation for inherited property.
5. changed association endpoint flags for inherited property.
6. changed projection authority for inherited projection.

### 5.2 Implement Dictionary Overlays

The implementation supports `InlineValue`, digest-bound
`ParentDictionaryAlias`, and hint-only `CanonicalHashHint` entries.

Rules:

1. Inline entries require the inline-dictionary feature bit.
2. Parent aliases require the parent-dictionary-alias feature bit, resolve to a
   declared parent ref, and require parent dictionary digest refs.
3. Canonical hash hints are non-materializing equality/pruning hints only; they
   must not be required for materialization and must carry a non-zero hash.

Additional rules:

1. Delta pages use delta-local FileCodes.
2. Parent aliases are an optimization only, not cross-file code domains.
3. Hash hints cannot reconstruct values unless bytes are recoverable from a
   validated source.
4. Redaction-sensitive equality leakage remains outside materialization; hash
   hints do not expose value bytes.

### 5.3 Implement Canonical Branch Identity

Add `DeltaBranchIdentityV1`:

```text
branch_identity_ref
branch_identity_kind
flags
branch_value_ref
branch_hash128
branch_catalog_fingerprint_ref
checksum
```

Rules:

1. Delta temporal pages may use local physical branch representation.
2. Cross-artifact anchors, touched sets, tombstone sets, and summaries use
   `branch_identity_ref`.
3. Raw parent FileCode branch keys must not be accepted across artifacts.
4. Hash-only branch identity is an accelerator unless canonical bytes or a
   collision-free construction verifies it.

### 5.4 Implement Temporal Records

Delta temporal segment payloads reuse COVE-O record semantics:

1. one object type per temporal segment.
2. rows ordered by `(timestamp_us, csn, branch identity, goid, record_id)`.
3. record kinds: Delta, Snapshot, Baseline, Tombstone.
4. property columns use COVE encoded-array machinery.
5. null, missing, redaction, clear, and tombstone semantics stay explicit.
6. `prev_ref` remains file-local.
7. Cross-artifact continuity uses continuation anchors.

Append-only profile:

1. `timestamp_us` remains COVE-O commit/file-ordering time.
2. `csn_min..=csn_max` advances beyond selected parent high-water mark for the
   same scope and branch identity.
3. Historical business/effective-time corrections use declared temporal-role
   properties.
4. Historical commit-order insertion is deferred.

Acceptance:

1. Additive catalog patches validate.
2. Inline delta dictionaries reconstruct values.
3. Raw cross-artifact FileCode comparisons reject.
4. Temporal rows sort and validate under append-only policy.

## Phase 6: Sparse Patches, Anchors, State Hashes, Touched Sets, Tombstones

### 6.1 Implement Continuation Anchors

Add `DeltaContinuationAnchorV1`:

```text
scope_kind
scope_id
object_type_id
branch_identity_ref
goid
parent_ref
predecessor_csn
predecessor_timestamp_us
predecessor_record_id
predecessor_state_hash_ref
predecessor_trust_hash_ref
anchor_strength
flags
checksum
```

Anchor strengths:

1. `KeyOnly`.
2. `KeyAndRecordId`.
3. `KeyRecordAndStateHash`.
4. `KeyRecordStateAndTrustHash`.

MVP rule:

1. Require `KeyRecordAndStateHash` for patching existing objects.
2. Anchors are required for first patch/tombstone of an existing object unless
   the record is a full Baseline/Snapshot anchor.
3. Brand-new objects with first Baseline/Snapshot may omit anchors.

### 6.2 Implement Delta State Hash V1

`CoveObjectDeltaStateHashV1` input includes:

1. scope kind and ID.
2. canonical branch identity.
3. object type ID.
4. GOID.
5. predecessor record ID.
6. predecessor CSN.
7. predecessor commit timestamp.
8. record kind and tombstone state.
9. sorted property IDs in logical state.
10. property logical type and collation.
11. null, clear, tombstone, and redaction markers.
12. canonical logical value bytes when visible.
13. redaction marker plus redaction commitment when redacted.
14. optional hidden-value commitment only if policy permits.

It excludes:

1. artifact-local FileCodes.
2. dictionary IDs.
3. physical page order.
4. compression.
5. offsets.
6. row ordinals.
7. writer-local layout choices.
8. advisory summaries or indexes.

### 6.3 Implement Sparse Patch Operations

Patch record key:

```text
scope_id, branch_identity, object_type_id, goid, record_id, timestamp_us, csn
```

Patch body:

```text
record_kind
changed_property_count
changed_property_ids
changed_property_ops
changed_property_value_refs
```

Operations:

1. `SetValue`.
2. `SetNull`.
3. `Clear`.
4. `Tombstone`.
5. `Redact`.

Tombstone kinds:

1. `Object`.
2. `Property`.
3. `Association`.
4. `Evidence`.
5. `ProjectionRow`.

Rules:

1. Omitted property means unchanged.
2. Ordinary null bitmap must not mean unchanged.
3. `SetNull` requires nullable property.
4. `Clear`, `Tombstone`, and `Redact` follow declared policy.
5. Snapshot/Baseline records may carry full state.

### 6.4 Implement Touched And Tombstone Sets

Add `DeltaTouchedObjectRangeV1`:

```text
scope_kind
scope_id
object_type_id
branch_identity_ref
min_goid
max_goid
touched_count
property_bitmap_ref
object_set_ref
checksum
```

Rules:

1. Touched summaries used for skipping may over-include but must not
   under-include.
2. Exact touched sets skip deltas for untouched point lookups.
3. Property bitmaps skip unaffected projections when supported.
4. Latest-state queries check tombstone summaries before returning parent
   state.
5. Probabilistic representations may prove absence only when a later required
   extension declares a no-false-negative construction.
6. MVP requires exact touched-object and exact tombstone summaries.

Acceptance:

1. Missing or weak anchors reject.
2. State hash recomputation matches stored state hash.
3. Sparse property operations produce expected latest state.
4. Touched/tombstone summaries reject under-inclusion in full validation.

## Phase 7: Reconstruction, Query Planning, Failure Semantics

### 7.1 Implement Reconstruction

Logical reconstruction:

```text
state = parent_state_at_cut(base, parent_deltas, query_cut)
for delta in ordered_deltas_needed_by_cut:
    validate delta parent and continuation anchors
    apply records in COVE-O temporal order
return state after visibility, redaction, branch, tombstone, and projection rules
```

Efficient read path:

1. Validate selected snapshot, delta-chain extension, and chain summary.
2. Use query root, object type, branch, temporal cut, selected properties, and
   predicates to choose candidate components.
3. Use base temporal indexes, chain summaries, delta temporal indexes, touched
   sets, tombstone sets, temporal blooms, and COVE-I/COVX sidecars to prune.
4. Fetch only delta headers or hot summary ranges after chain-summary pruning.
5. For untouched objects/properties, read only newest component that proves the
   requested state.
6. For touched objects, read nearest required anchor plus later delta records.
7. Apply sparse patches in CSN order into an in-memory state table keyed by
   `(scope_id, branch_identity, object_type_id, goid)`.
8. Materialize only requested output fields.

### 7.2 Implement Query-Pruning Rules

Support:

1. skip delta when `as_of_csn` is before `delta.csn_min`.
2. skip delta when `as_of_commit_timestamp_us` is before
   `commit_time_range_start_us` and monotonicity validates.
3. source publication/ingest range filters for operational delta selection
   only.
4. valid-time pruning only through validated temporal-role summaries.
5. object-type pruning.
6. branch pruning.
7. exact touched-set point lookup skip.
8. projection property bitmap skip when implemented.
9. latest-state tombstone summary check.
10. optional coverage/index metadata under normal proof rules.

Coverage composition:

1. `DefinitelyNo` requires every selected component to prove no matching
   visible result.
2. Parent `DefinitelyYes` does not prove visible snapshot yes when deltas may
   tombstone, change, or redact affected rows.
3. Exact aggregate/index-only answers require component-wise exact answer plus
   exact overlay correction or a snapshot-level index for selected snapshot.
4. Approximate summaries remain approximate.

### 7.3 Implement Failure Semantics

Reject:

1. unsupported required COVM delta-chain profile.
2. unsupported required delta feature for affected operation.
3. missing required base file.
4. missing required delta.
5. corrupt required delta.
6. stale parent fingerprint.
7. invalid continuation anchor.
8. invalid dictionary alias used by selected rows.

Fallback only when safe:

1. missing optional delta-local index.
2. corrupt optional coverage/index/layout metadata when requested operation
   does not require it.

Acceptance:

1. Base-only answers never appear for delta-bearing selected snapshots.
2. Point lookups skip irrelevant exact untouched deltas.
3. Latest-state reads consult tombstone summaries before returning parent
   state.
4. Coverage proof behavior is conservative.

## Phase 8: Publication, Compaction, Checkpoints, And Operations

### 8.1 Implement Publication Protocol

Local filesystem writer:

1. Read and validate parent snapshot state.
2. Build delta in temporary storage.
3. Finalize footer, postscript, checksums, digests, and trust data.
4. Build or update COVM chain summary.
5. Flush delta bytes and `fsync`/`fdatasync` by platform policy.
6. Publish new COVM snapshot referencing complete delta and summary.
7. Flush manifest and containing directory.

Object storage:

1. Upload delta under immutable content-addressed or versioned object name.
2. Verify object length and digest.
3. Upload or embed updated chain summary.
4. Publish COVM snapshot or external catalog commit last.
5. Never infer visibility from partially uploaded or unreferenced objects.

### 8.2 Implement Compaction

Compaction:

```text
compact(base.cove, deltas...) -> compacted-base.cove
```

Compaction must:

1. preserve COVE-O object state, history, branches, tombstones, trust hashes,
   and evidence required by selected policy.
2. assign new file-local dictionaries and FileCodes.
3. rebuild COVE-O temporal segment indexes.
4. rebuild COVE-COVERAGE and COVE-I/COVX sidecars when requested.
5. emit object catalog section with logical fingerprint equal to selected
   effective object catalog fingerprint.
6. preserve parent catalog fingerprint when logical catalog is unchanged.
7. publish new COVM snapshot referencing the compacted file.
8. leave old base and deltas immutable.

Recommended compaction triggers:

1. chain depth exceeds configured limit.
2. delta bytes exceed percentage of base bytes.
3. point lookup read amplification exceeds target.
4. latest-state reconstruction touches too many deltas.
5. schema/catalog patches accumulate past threshold.
6. object-store range requests exceed budget.

### 8.3 Implement Checkpoint Deltas

Checkpoint deltas remain `.covedelta` artifacts carrying Baseline/Snapshot
records for declared object subsets.

Use cases:

1. hot objects updated many times.
2. branch tips.
3. frequently queried object types.
4. offline bundles with bounded replay memory.

Checkpoint deltas do not replace full compaction.

### 8.4 Expose Read-Amplification Metrics

Expose:

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

Default policies:

1. warn when chain depth exceeds 16.
2. recommend checkpointing when one object has more than 32 patches since last
   Snapshot/Baseline.
3. recommend compaction when delta bytes exceed 20 percent of base bytes.
4. recommend snapshot-level COVE-I when latest-state point lookups touch more
   than 4 artifacts at p95.
5. recommend summary hoisting or compaction when point lookups require more
   than 2 metadata range requests before data.
6. recommend packing small deltas when request cost dominates bytes returned.
7. reject or require override when chain depth exceeds hard limit.

Acceptance:

1. Local and object-store publication order prevents partially published
   snapshots.
2. Compaction equivalence test passes.
3. Checkpoint deltas lower replay cost without changing logical state.
4. Metrics are visible in explain/diagnostics or benchmark output.

## Phase 9: Interactions, Security, Trust, And Second-Tier Features

### 9.1 COVE-MAP Interaction

Rules:

1. Inherit COVE-MAP definitions by fingerprint unless mapping rules change.
2. Deltas may add evidence rows, source-row digest references, mapping-run
   metadata, resolver-run metadata and digest proofs, conflict outcomes,
   identity-equivalence assertions, projection rows, projection invalidations,
   and additive projection definitions.
3. Association, link, and evidence facts that affect ordinary COVE-O
   reconstruction must be materialized as COVE-O temporal records.
4. `DELTA_EVIDENCE_PATCH` and `DELTA_PROJECTION_PATCH` may support replay,
   explanation, projection, or planning metadata but are not ordinary object
   truth unless a required extension grants authority.
5. Resolver-aware delta patches require plan 001 stable section IDs, digests,
   row-level outcomes, and evidence metadata.

### 9.2 COVE-I Interaction

Support index levels:

1. base-only indexes.
2. delta-only indexes.
3. snapshot-level indexes for selected base-plus-delta chain.

Rules:

1. Snapshot-level index validity records include selected snapshot ID and
   delta chain digest.
2. Snapshot-level index is invalid unless it binds exact ordered chain digest.
3. Base-only index results are corrected for tombstones, redactions, and
   property changes.
4. Delta-only indexes may over-include but must not under-include when they
   advertise conservative coverage.
5. Missing optional indexes fall back; required indexes reject only operations
   that require them.

### 9.3 COVE-L Interaction

Delta-aware layout hints are advisory and focus on object-store requests:

1. co-locate small temporal segments by object type and branch.
2. store touched summaries near header or in small first range.
3. group hot checkpoint records into contiguous page clusters.
4. expose scan splits that schedule base and delta reads.
5. never require layout plan to discover authoritative temporal records.

### 9.4 Trust And Digest Continuity

Compute trust data over canonical logical values, not local FileCodes.

Include:

1. parent snapshot digest.
2. parent effective catalog fingerprint.
3. parent semantic-map fingerprint where relevant.
4. predecessor state hash for touched objects where available.
5. canonical hash of each delta temporal record.
6. final state hash for objects with Snapshot/Baseline records.
7. chain digest.

Reject:

1. changed parent bytes.
2. changed parent dictionary entry behind alias.
3. reordered deltas.
4. missing predecessor anchor.
5. mismatched predecessor state hash.
6. duplicate record ID in selected object/branch scope.
7. unexpected CSN gap when policy requires contiguous CSNs.

### 9.5 Security And Governance

Rules:

1. Redaction manifests are inherited by fingerprint or patched explicitly.
2. Delta must not reveal parent redacted values through dictionary aliases,
   trust hashes, explain output, or error messages.
3. Redaction/tombstone delta changes selected snapshot semantics but does not
   remove parent bytes.
4. If old values must stop being distributed, compact into redacted
   self-contained `.cove` and stop publishing old chain.
5. Visibility overlays remain external unless a required profile defines them.
6. Evidence additions bind mapping/source fingerprints when explanation or
   audit readback is requested.
7. Diagnostics identify artifact IDs and section kinds, not protected object
   names or values unless policy permits.

### 9.6 Second-Tier Features

Implemented after MVP acceptance:

1. Parent dictionary aliases.
2. Hash/equality dictionary hints as non-materializing hints that do not expose
   value bytes.
3. Property-level touched bitmap refs with descriptor validation.
4. Checkpoint deltas.
5. COVE-MAP evidence patches.
6. COVE-MAP projection patches as non-authoritative projection metadata.
7. Snapshot-level COVE-I chain-digest binding.
8. Small-delta packing recommendations when request cost dominates bytes
   returned.
9. Delta-local COVE-I/COVX index hints, COVE-COVERAGE patch hints, and
   object-store layout hints validate on request while corrupt optional
   metadata still falls back for ordinary object replay.

Deferred required extensions:

1. Historical commit-order inserts or corrections.
2. Probabilistic touched summaries for proof-of-absence.
3. Projection patches authoritative for ordinary object truth.
4. Merge DAGs or multi-writer delta branches.

Acceptance:

1. Each optional feature has feature-bit gating.
2. Unsupported optional feature corruption falls back only when safe.
3. Deferred features reject fail-closed if encountered as required features.

## Phase 10: Validation, Conformance, Benchmarks, And Release Gates

### 10.1 Implement Validation Modes

Snapshot-selection validation checks enough metadata to plan and execute a
specific query without opening irrelevant deltas:

1. COVM delta-chain extension.
2. chain digest.
3. chain-summary digest.
4. artifact references.
5. feature bits.
6. effective fingerprints.
7. summary descriptors needed by selected operation.

Full delta-chain validation opens every selected delta and proves:

1. chain summaries agree with payloads.
2. touched sets agree with temporal records.
3. tombstone sets agree with tombstones.
4. anchors target valid logical parent state.
5. temporal indexes agree with temporal rows.
6. summaries do not under-include candidate deltas.

### 10.2 Validator Checklist

Validator must check:

1. header magic, versions, lengths, checksums, and section directory.
2. postscript/footer consistency and final `CVD2` magic.
3. parent refs match selected snapshot and digests.
4. exactly one lineage parent ref.
5. COVM extension binds exact ordered chain digest.
6. COVM summary validates by CRC and cryptographic digest.
7. summary entries dense, ordered, and matching referenced deltas.
8. time fields preserve commit/source/snapshot/valid-time distinctions.
9. artifact IDs unique within selected chain.
10. schema/catalog/projection fingerprints inherited or patched validly.
11. catalog patches additive.
12. temporal rows sorted.
13. CSN and commit timestamp monotonicity under append-only policy.
14. `prev_ref` file-local.
15. scope and branch identity explicit or single-scope invariant holds.
16. anchors target valid logical parent state and meet strength.
17. dictionary aliases resolve through parent dictionary digests.
18. aliases do not expose redacted equality unless permitted.
19. touched sets and tombstone summaries do not under-include.
20. trust continuation hashes match when required.
21. required feature bits supported for requested operation.

### 10.3 Required Conformance Vectors

Add fixtures for:

1. non-delta-aware COVM reader rejects delta-bearing snapshot.
2. opening base `.cove` directly still succeeds.
3. minimal base plus one delta with one new object.
4. sparse property patch against existing object.
5. `SetValue`, `SetNull`, `Clear`, `Redact`, `Tombstone`, and omitted
   unchanged property.
6. object tombstone hiding parent latest state.
7. association/link object update.
8. evidence addition with inherited COVE-MAP fingerprint.
9. additive object catalog patch.
10. invalid parent digest rejection.
11. correct artifact digest but wrong `parent_snapshot_id` rejection.
12. multiple or missing lineage parent refs rejected.
13. delta chain reorder rejected by chain digest.
14. missing or corrupt required chain summary rejects selected snapshot.
15. wrong chain digest in summary rejected.
16. summary under-includes affected delta and validator rejects.
17. source publication range prunes operationally but does not alter `as_of_csn`
    or valid-time semantics.
18. multi-scope GOID collision does not cross-apply anchors or tombstones.
19. raw parent FileCode branch key not accepted cross-artifact.
20. duplicate record ID across selected scope/object/branch rejected.
21. missing continuation anchor rejected.
22. continuation anchor below required strength rejected.
23. touched set under-includes patched object and validator rejects.
24. tombstone summary under-includes tombstone and validator rejects.
25. corrupt optional delta-local index fallback.
26. `as_of_csn` cut before, inside, and after delta CSN range.
27. `as_of_valid_time` does not use `csn_min` pruning unless valid-time summary
    exists.
28. compaction equivalence: `base + deltas` equals compacted `.cove`.
29. COVE-I base-only result corrected by delta tombstone.
30. snapshot-level index with stale chain digest rejected.
31. touched-set exact skip for point lookup.
32. property bitmap skip for projection readback.
33. chain summary digest mismatch rejected.
34. anchor state hash recomputation matches stored state hash.
35. unsupported required delta feature rejects selected snapshot.

Second-tier fixtures:

1. parent dictionary alias for repeated string value.
2. invalid dictionary alias rejection.
3. parent dictionary alias to redacted value rejected or policy-gated.
4. compaction reassigns FileCodes but preserves canonical trust state.
5. corrupt optional layout/index/coverage section falls back.

### 10.4 Benchmark Plan

Compare:

1. full rewrite per update batch.
2. additional full `.cove` files plus manifest merge.
3. delta artifacts plus periodic compaction.

Measure:

1. bytes written per update.
2. total bytes stored.
3. writer finalization latency.
4. publication latency on local FS and object-store harness.
5. latest-state point lookup p50/p95/p99.
6. object history query cost.
7. projection readback cost.
8. object-store range request count.
9. COVM/chain-summary range request count.
10. delta artifacts skipped before opening.
11. delta artifacts opened per point lookup.
12. source publication range pruning effectiveness.
13. dictionary alias resolution cost.
14. compaction throughput and output size.
15. index rebuild cost.
16. recovery/validation time for selected snapshots.

Matrix:

1. base object count.
2. touched object percentage.
3. changed property percentage.
4. delta chain depth.
5. dictionary value reuse rate.
6. tombstone rate.
7. query selectivity.
8. object type count.
9. branch count.

### 10.5 Release Gate

The feature is complete only when:

1. MVP items in Phase 1 through Phase 8 pass.
2. Full validator and snapshot-selection validator exist.
3. Required conformance vectors pass.
4. Compaction equivalence passes.
5. Unsupported required extensions fail closed.
6. Optional sections fall back only when the requested operation does not
   require them.
7. Benchmarks report the required metrics.

## Recommended First Implementation Checklist

The smallest useful implementation supports:

1. COVM required extension block for ordered base-plus-delta chain.
2. Mandatory COVM chain summary with chain digest, per-delta CSN range, commit
   time range, source publication/ingest range, object type summary, branch
   summary, scope summary, touched summary refs, and tombstone summary refs.
3. Delta header, postscript/footer, section directory, and checksums.
4. Mandatory cryptographic digest for base and delta references.
5. Effective catalog, schema, projection, map, visibility, and redaction
   fingerprints.
6. Inline-only delta dictionaries.
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

## Proposal Coverage Matrix

| Proposal section | Plan coverage | Implementation evidence |
| --- | --- | --- |
| Summary | Objective, Implementation Contract | `.covedelta` envelope, COVM chain selection, reconstruction, compaction checks, and conformance fixtures are implemented in `crates/cove-core/src/artifact/covedelta.rs`, `crates/cove-core/src/artifact/covm.rs`, and `crates/cove-core/src/profile/cove_o/readback.rs`. |
| Motivation | Objective, Publication and Compaction phases | Generated fixtures cover base-plus-delta reconstruction, compacted equivalence, COVM chain selection, and read-amplification metrics. |
| Goals | Implementation Contract, MVP checklist | MVP parser, validator, reconstruction, pruning, publication, and conformance paths pass in the workspace test/check commands and 661-fixture conformance corpus. |
| Non-Goals | Non-Goals | In-place mutation, filename discovery, raw cross-artifact FileCode equality, historical inserts, and merge DAGs are rejected or deferred behind unsupported required feature bits. |
| Design Principle | Implementation Contract, all phases | Chain digest, parent ref, feature-bit, section-directory, and effective-fingerprint validation live in `covm.rs` and `covedelta.rs`. |
| Profile Shape | Phase 0, Phase 2 | The implementation uses `CVD2`, `.covedelta`, a required COVM delta-chain extension, and `CDS1` chain summaries. |
| Core Invariants | Implementation Contract | Validators enforce final magic/footer/postscript consistency, exact ordered chain selection, lineage parent refs, append-only CSN/commit ranges, and no base-only selected delta reads. |
| Artifact Naming | Phase 0, Phase 3 | `MAGIC_COVEDELTA` is `CVD2`; `crates/cove-core/src/artifact/mod.rs` exposes the `covedelta` artifact module. |
| COVM Delta-Chain Extension | Phase 1 | `CovmDeltaChainExtensionV1`, artifact refs, chain digests, and selected-chain validators are implemented in `covm.rs`. |
| Authoritative Surfaces | Phase 1, Phase 3, Phase 10 | Selected COVM chains, declared summaries, and full delta validation are the authoritative inputs; conformance rejects missing/extra/reordered/stale deltas. |
| Delta Chain Summary | Phase 2 | `CovmDeltaChainSummaryV1` and `DeltaChainSummaryEntryV1` parse, validate, prune, and expose metrics in `covm.rs`. |
| Binary Envelope | Phase 3 | `CoveDeltaFile` serializes/parses header, parent refs, section directory, footer, postscript, CRCs, and final `CVD2` magic. |
| Delta Header | Phase 4.1 | `CoveDeltaHeaderV1` carries artifact/snapshot identity, features, ranges, scope, and effective fingerprint refs. |
| Parent References | Phase 4.2 | `DeltaParentRefV1` validates lineage parent count, parent snapshot identity, file length, footer CRC, digest, and fingerprint refs. |
| Sections | Phase 4.3 | `CoveDeltaSectionKind` covers catalog patches, temporal indexes/data, sparse property ops, anchors, touched/tombstone sets, descriptors, dictionary overlays, evidence patches, and optional hints. |
| Descriptor Tables | Phase 4.4 | Scope, summary, temporal-role, touched, tombstone, and state-hash descriptors are parsed and cross-checked in `covedelta.rs`. |
| Delta Feature Bits | Phase 4.5 | Required and optional delta feature bits gate sparse patches, tombstones, anchors, inline dictionaries, exact sets, checkpoints, evidence patches, coverage, indexes, projection patches, and deferred historical inserts. |
| Catalog Patches | Phase 5.1 | Additive object catalog patch sections validate item counts and reject parent reinterpretation; reconstruction fixtures cover catalog patch output. |
| Dictionary Overlays | Phase 5.2, Phase 9.6 | Inline dictionary overlays, parent dictionary aliases, and non-materializing canonical hash hints are supported with feature gates, parent-ref validation, and conformance fixtures. |
| Branch Identity | Phase 5.3 | `DeltaBranchIdentityV1` validates canonical/hash-only branch identity and rejects raw parent FileCode identity across artifacts. |
| Temporal Records | Phase 5.4 | Delta temporal sections reuse `TemporalSegmentData` parsing and enforce sorted rows, CSN bounds, record IDs, branch/scope invariants, and file-local `prev_ref`. |
| Continuation Anchors | Phase 6.1 | `DeltaContinuationAnchorV1` validates scope, branch, predecessor CSN/record ID, anchor strength, and state-hash refs. |
| Delta State Hash V1 | Phase 6.2 | `CoveObjectDeltaStateHashV1` and `DeltaStateHashDescriptorV1` materialize and validate stored state-hash material. |
| Patch Encoding | Phase 6.3 | `DeltaSparsePatchRecordV1` supports `SetValue`, `SetNull`, `Clear`, `Tombstone`, and `Redact`, with reject fixtures for malformed op combinations. |
| Touched Object Set | Phase 6.4, Phase 9.6 | Exact touched and tombstone range sections validate coverage, bind property bitmap refs to property-bitmap descriptors, and support point/projection skip decisions. |
| Reconstruction | Phase 7.1 | `readback.rs` reconstructs latest object states from base plus deltas, including sparse patches, associations, catalog patches, and tombstones. |
| Query Planning | Phase 7.2 | `CovmDeltaChainSummaryV1::prune_delta_chain`, read-amplification metrics, exact touched sets, property skip checks, and COVE-I tombstone correction support planning. |
| Publication Protocol | Phase 8.1 | `durable_publish_delta_then_manifest` writes delta before manifest, and conformance covers manifest ordering. |
| Compaction | Phase 8.2 | Compaction equivalence fixtures compare reconstructed base-plus-delta state with compacted self-contained COVE-O output. |
| Checkpoint Deltas | Phase 8.3, Phase 9.6 | Checkpoint baseline features parse and require Snapshot/Baseline rows; checkpoint policy metrics recommend when to checkpoint. |
| Interaction With COVE-MAP | Phase 9.1, Phase 9.6 | Delta effective semantic-map/projection fingerprints inherit/override through header and parent refs; evidence patch sections require `DELTA_FEATURE_MAP_EVIDENCE_PATCH`, and projection patch sections require `DELTA_FEATURE_PROJECTION_PATCH` plus a projection fingerprint. |
| Interaction With COVE-I | Phase 9.2, Phase 9.6 | COVE-I delta-chain digests, tombstone overlay correction, stale digest fixtures, and requested delta-local index hint validation enforce index compatibility. |
| Interaction With COVE-L | Phase 9.3, Phase 9.6 | Object-store layout/index/coverage corruption remains optional-fallback behavior unless required by the selected operation; requested layout/index/coverage hint validators reject malformed or misbound sidecar metadata. |
| Trust And Digest Continuity | Phase 9.4 | Parent refs, chain digests, summary digests, state hashes, footer CRCs, and artifact digest checks bind selected snapshots. |
| Efficiency Model | Phase 2, Phase 7, Phase 8 | Summary pruning, exact touched/tombstone sets, read-amplification metrics, checkpoint recommendations, compaction recommendations, and packing recommendations are implemented. |
| Read Amplification Budget | Phase 8.4 | `CovmDeltaReadAmplificationMetrics` and `CovmDeltaReadAmplificationPolicy` expose chain depth, skipped/opened deltas, range requests, bytes returned, and policy recommendations. |
| Failure Semantics | Phase 7.3 | Missing, corrupt, stale, reordered, unsupported-required, under-inclusive, and unsafe optional sections fail closed or fall back only when safe. |
| Security And Governance | Phase 9.5 | Effective visibility/redaction fingerprints are bound in headers/parents/chain extensions; parent aliases and hash hints are validated without exposing protected parent values. |
| Validation Requirements | Phase 10.1, Phase 10.2 | Snapshot-selection validators and full object-delta validators are implemented in `covm.rs` and `covedelta.rs`; conformance invokes both. |
| Conformance Tests | Phase 10.3 | Generated fixtures cover the required MVP vectors, including sparse patches, anchors, tombstones, compaction, COVE-I correction, stale digests, optional fallback, and unsupported required features. |
| Benchmark Plan | Phase 10.4 | Benchmark-facing metrics and policy recommendations are implemented; full benchmark execution remains a release measurement step outside conformance. |
| Open Questions | Phase 0 | Resolved implementation decisions are recorded above and reflected in feature gates, validators, and fixtures. |
| Recommended First Implementation | Recommended First Implementation Checklist | The MVP checklist maps to implemented COVM extension, chain summary, `CVD2` envelope, digests, fingerprints, inline dictionaries, temporal/sparse sections, tombstones, exact sets, anchors, append-only policy, and compaction tests. |
| Positioning | Objective, Non-Goals, Release Gate | The implementation is an optional publication/efficiency layer; ordinary `.cove` files remain immutable and self-contained, with unsupported second-tier features fail-closed when required. |
