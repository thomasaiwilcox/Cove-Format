# Table Indexes and Scan Semantics

## 30. Exact Set Indexes

Exact sets represent exact values present in a segment or morsel.

```rust
struct ExactSetIndexHeaderV2 {
    table_id: u32,
    column_id: u32,

    granularity: u8,        // 0=segment, 1=morsel
    key_kind: u8,           // 0=FileCode, 1=NumCode, 2=CanonicalHash
    representation: u8,     // 0=sorted list, 1=bitset, 2=roaring-like
    flags: u8,

    entry_count: u32,

    data_offset: u64,
    data_length: u64,

    checksum: u32,
}
```

**Rules:**
- Exact sets are valid only after checksum validation.
- Corrupt exact sets MUST be ignored.
- Exact sets MUST NOT produce false negatives.
- Exact sets MAY prove DefinitelyNo or DefinitelyYes.
**Recommended writer policy:**
**Build exact sets for:**
  - low-cardinality columns,
  - medium-cardinality predicate columns,
  - equality-heavy dimensions,
  - columns used in IN predicates.

---


## 31. Bloom Indexes

Bloom filters provide conservative membership tests.

```rust
struct BloomIndexHeaderV2 {
    table_id: u32,
    column_id: u32,

    granularity: u8,       // 0=segment, 1=morsel
    hash_domain: u8,       // 0=FileCode, 1=NumCode, 2=CanonicalValueHash
    algorithm: u8,         // 0=split-block
    flags: u8,

    target_fpr_ppm: u32,

    filter_count: u32,

    data_offset: u64,
    data_length: u64,

    checksum: u32,
}
```

**Rules:**
- Bloom filters may produce false positives.
- Bloom filters MUST NOT produce false negatives.
- Corrupt bloom filters MUST be ignored.
- Bloom filters can prove DefinitelyNo.
- Bloom filters generally cannot prove DefinitelyYes.
**Recommended sizing:**
**target_fpr:**
  1% default

**bits_per_item:**
  approximately 10 for 1% FPR

**hash_count:**
  approximately 7 for 1% FPR

**minimum_filter_bits:**
  512

---


## 32. Inverted Morsel Indexes

Inverted morsel indexes map a value to candidate morsels.

```rust
struct InvertedMorselIndexHeaderV2 {
    table_id: u32,
    column_id: u32,

    key_kind: u8,       // 0=FileCode, 1=NumCode
    flags: u8,
    representation: u8,
    reserved: u8,

    entry_count: u32,

    entries_offset: u64,
    bitmap_data_offset: u64,

    checksum: u32,
}
```

```rust
struct InvertedMorselEntryV2 {
    key: u64,
    morsel_bitmap_offset: u64,
    morsel_bitmap_length: u32,

    row_bitmap_offset: u64,
    row_bitmap_length: u32,
}
```

**Rules:**
- Inverted indexes are optional.
- Corrupt inverted indexes MUST be ignored.
- Morsel-level bitmaps are preferred.
- Row-level bitmaps are optional.

---


## 33. Lookup Indexes

Lookup indexes support direct point access.
**Useful predicates:**
WHERE event_id = ?
WHERE patient_id = ?
WHERE order_id = ?
WHERE external_ref = ?

```rust
struct LookupIndexHeaderV2 {
    table_id: u32,
    column_id: u32,

    key_kind: u8,
    // 0=FileCode
    // 1=NumCode
    // 2=CanonicalHash
    // 3=FixedBytes

    index_kind: u8,
    // 0=Hash
    // 1=SparseSorted
    // 2=MinimalPerfectHash

    uniqueness: u8,
    // 0=unknown
    // 1=unique
    // 2=non_unique

    flags: u8,

    entry_count: u64,

    entries_offset: u64,
    entries_length: u64,

    rowref_offset: u64,
    rowref_length: u64,

    checksum: u32,
}
```

**For unique keys:**
key -> CoveTableRowRef
**For non-unique keys:**
key -> rowref list
**Rules:**
- Lookup indexes are optional.
- Lookup indexes MUST be ignored if stale or corrupt.
- Lookup indexes MAY be stored inside COVE or in COVX.
- Lookup indexes MUST NOT change query results.


### 33.1 COVE-I Secondary Index Artifact

COVE-I is an optional secondary index profile. A `.covi` artifact may contain global, dataset-level, file-level, object-level, path-level, row-range, or dimensional-bucket indexes. COVE-I exists because some indexes are too large, workload-specific, mutable-to-rebuild, or cross-file to belong in every `.cove` file.

**COVE-I final bytes:**
[postscript bytes]
[postscript_version: u16]
[postscript_len: u16]
[magic: "CVI2"]

A `.covi` artifact uses the same tail-discovery discipline as COVE: readers discover the postscript from the final bytes, validate the postscript, locate the header/section directory, and then validate only the index roots and payload sections needed by the requested operation.

```rust
struct CoviPostscriptV2 {
    required_features: u64,
    optional_features: u64,
    file_len: u64,
    header_offset: u64,
    header_length: u64,
    checksum: u32,
}
```

```rust
struct CoviHeaderV2 {
    magic: [u8; 4],          // "CVI2"
    header_len: u16,         // fixed header length for this version
    version_major: u16,
    version_minor: u16,

    flags: u32,
    index_artifact_id: [u8; 16],
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],

    section_count: u32,
    referenced_file_count: u32,
    snapshot_validity_count: u32,
    index_root_count: u32,
    capability_count: u32,

    section_directory_offset: u64,
    section_directory_length: u64,
    referenced_files_offset: u64,
    snapshot_validity_offset: u64,
    index_roots_offset: u64,
    capabilities_offset: u64,
    string_table_section_ref: u32,

    created_at_us: i64,
    reserved: [u8; 24],
    checksum: u32,
}
```

```rust
enum CoviSectionKindV2 {
    ReferencedFiles = 0,
    SnapshotValidity = 1,
    StringTable = 2,
    IndexRoots = 3,
    IndexCapabilities = 4,
    KeyBlock = 5,
    EntryBlock = 6,
    PostingsBlock = 7,
    RowRangeBlock = 8,
    RowOrdinalSetBlock = 9,
    BitmapBlock = 10,
    AggregateAnswerBlock = 11,
    CoverageSetBlock = 12,
    DimensionalBucketBlock = 13,
    ObjectPathBlock = 14,
    ExtensionBlock = 255,
}

struct CoviSectionEntryV2 {
    section_id: u32,
    section_kind: u16,       // CoviSectionKindV2
    flags: u16,
    offset: u64,
    length: u64,
    uncompressed_length: u64,
    item_count: u64,
    compression: u8,         // CompressionCodec
    encryption: u8,          // 0=None in v2
    alignment_log2: u8,
    reserved0: u8,
    required_features: u64,
    optional_features: u64,
    crc32c: u32,
    checksum: u32,
}
```

```rust
struct CoviReferencedFileV2 {
    file_ref: u32,           // dense zero-based file reference used by postings
    flags: u32,
    file_id: [u8; 16],
    file_len: u64,
    footer_crc32c: u32,
    digest_algorithm: u16,
    digest_len: u16,
    digest_offset: u64,      // digest bytes in StringTable or binary payload block
    uri_ref: u32,            // optional URI string-table ref, u32::MAX when absent
    schema_fingerprint_ref: u32,
    checksum: u32,
}

struct CoviSnapshotValidityV2 {
    snapshot_validity_ref: u32,
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    schema_fingerprint_ref: u32,
    semantic_map_fingerprint_ref: u32,
    external_visibility_ref: u32,
    data_checksum_root_ref: u32,
    valid_from_us: i64,
    valid_until_us: i64,     // i64::MAX when open-ended for immutable snapshot ref
    flags: u32,
    checksum: u32,
}
```

```rust
enum CoviIndexedTargetKindV2 {
    TableColumn = 0,
    ObjectProperty = 1,
    ObjectPath = 2,
    AssociationEndpoint = 3,
    ProjectionColumn = 4,
    SemanticDimension = 5,
    DimensionalTuple = 6,
    ExternalTarget = 255,
}

enum CoviIndexKindV2 {
    Hash = 0,
    Sorted = 1,
    SparseSorted = 2,
    Trie = 3,
    RangeBucket = 4,
    Bitmap = 5,
    MinimalPerfectHash = 6,
    AggregateOnly = 7,
    Extension = 255,
}

struct CoviIndexRootV2 {
    index_root_id: u32,
    indexed_target_kind: u16,      // CoviIndexedTargetKindV2
    index_kind: u16,               // CoviIndexKindV2
    coverage_granularity: u8,      // CoverageGranularityV2
    proof_strength: u8,            // CoverageProofStrengthV2
    exactness: u8,                 // CoverageExactnessV2
    flags: u8,

    table_id: u32,
    column_id: u32,
    object_type_id: u32,
    property_id: u32,
    path_ref: u32,
    semantic_dimension_ref: u32,

    logical_type: u16,
    physical_kind: u8,
    key_encoding_kind: u8,
    comparator_kind: u16,
    collation_id: u16,
    null_semantics: u8,
    sort_order: u8,

    value_count: u64,
    distinct_count: u64,
    null_count: u64,

    min_key_ref: u32,
    max_key_ref: u32,

    key_block_section_id: u32,
    entry_block_section_id: u32,
    postings_block_section_id: u32,
    aggregate_block_section_id: u32,
    coverage_set_ref: u32,
    capability_ref: u32,
    snapshot_validity_ref: u32,
    checksum: u32,
}
```

### 33.1.0 COVE-I Block Containers and Reference Spaces

COVE-I uses local block containers so entries, postings, coverage references, and aggregate answers can be validated independently and resolved without JSON metadata. A `.covi` reader MUST resolve references through the binary block headers and arrays described here.

```rust
enum CoviBlockKindV2 {
    KeyBlock = 0,
    EntryBlock = 1,
    PostingsBlock = 2,
    RowOrdinalSetBlock = 3,
    AggregateAnswerBlock = 4,
    CoverageSetBlock = 5,
    Extension = 255,
}

struct CoviEntryBlockHeaderV2 {
    magic: [u8; 4],              // "CIE2"
    version_major: u16,
    version_minor: u16,
    header_len: u16,
    entry_len: u16,

    entry_block_id: u32,
    index_root_id: u32,
    entry_count: u32,            // maximum u32::MAX entries per block
    key_block_id: u32,
    postings_block_id: u32,      // u32::MAX when no postings block
    aggregate_block_id: u32,     // u32::MAX when no aggregate block

    entries_offset: u64,
    entries_length: u64,
    flags: u32,
    checksum: u32,
}

struct CoviPostingsBlockHeaderV2 {
    magic: [u8; 4],              // "CIP2"
    version_major: u16,
    version_minor: u16,
    header_len: u16,
    postings_header_len: u16,

    postings_block_id: u32,
    index_root_id: u32,
    postings_count: u32,
    row_ordinal_set_count: u32,

    postings_headers_offset: u64,
    row_ordinal_headers_offset: u64, // 0 when absent
    postings_payload_offset: u64,
    postings_payload_length: u64,

    flags: u32,
    checksum: u32,
}

struct CoviAggregateAnswerBlockHeaderV2 {
    magic: [u8; 4],              // "CIA2"
    version_major: u16,
    version_minor: u16,
    header_len: u16,
    aggregate_answer_len: u16,

    aggregate_block_id: u32,
    index_root_id: u32,
    aggregate_answer_count: u32,
    aggregate_answers_offset: u64,
    aggregate_payload_offset: u64,
    aggregate_payload_length: u64,

    flags: u32,
    checksum: u32,
}
```

**COVE-I reference spaces:**
- `key_block_id`, `entry_block_id`, `postings_block_id`, and `aggregate_block_id` are local to one `.covi` artifact and MUST identify blocks referenced by the owning `CoviIndexRootV2`.
- `CoviIndexEntryV2.entry_ref` is the dense local index of the entry within its `CoviEntryBlockHeaderV2` entry array. `entry_ref` MUST equal the array position of the entry.
- `postings_ref` is a dense local index into the `CoviPostingsHeaderV2` array of the postings block named by the owning index root. `u32::MAX` means absent.
- `aggregate_answer_ref` is a dense local index into the `CoviAggregateAnswerV2` array of the aggregate block named by the owning index root. `u32::MAX` means absent.
- `coverage_set_ref` references a `coverage_set_id` in the same `.covi` artifact's COVE-COVERAGE `COVERAGE_SET` section. External coverage sets MUST be copied into the `.covi` artifact or referenced through a registered extension payload with digest-verified validity.
- `next_duplicate_ref` is a dense local `entry_ref` in the same entry block. `u32::MAX` means absent. Duplicate chains MUST terminate and MUST NOT contain cycles.
- Payload offsets inside COVE-I block headers are relative to the start of that block payload, not the start of the `.covi` file.

**Block validation rules:**
- Block magic, version, lengths, counts, offsets, and checksums MUST validate before any local reference is resolved.
- Entry blocks MUST be sorted according to the owning root's comparator unless the root declares hash/minimal-perfect-hash ordering.
- Postings blocks MUST contain exactly `postings_count` posting headers; each `postings_ref` MUST identify exactly one header.
- Aggregate blocks MUST contain exactly `aggregate_answer_count` aggregate answer descriptors; exact answers MUST be snapshot-, overlay-, schema-, redaction-, and mapping-valid before use.
- A malformed local reference invalidates the entry or block. If the index is optional, readers MUST ignore the index and fall back.

### 33.1.1 COVE-I Key, Comparator, and Entry Grammar

Keys in COVE-I are deterministic byte strings or fixed-width scalar encodings. The comparator declared by the root determines equality and ordering. A COVE-I reader MUST NOT compare display bytes, source bytes, raw FileCodes from another file, or engine-local ExecutionCodes as a substitute for the declared key semantics.

```rust
enum CoviKeyEncodingKindV2 {
    FileCode = 0,             // only within the referenced file scope declared by postings
    NumCode = 1,
    CanonicalValueBytes = 2,
    CanonicalHash64 = 3,
    CanonicalHash128 = 4,
    FixedBytes = 5,
    Utf8BytewisePrefix = 6,
    IntervalTuple = 7,
    DimensionalTuple = 8,
    ObjectPathTuple = 9,
    Extension = 255,
}

enum CoviComparatorKindV2 {
    CanonicalEquality = 0,
    CanonicalOrdering = 1,
    DomainRankOrdering = 2,
    NumCodeLogicalOrdering = 3,
    Utf8BytewisePrefix = 4,
    IntervalOverlap = 5,
    DimensionalTupleLexicographic = 6,
    ObjectPathLexicographic = 7,
    ExtensionRequired = 255,
}

struct CoviKeyBlockHeaderV2 {
    magic: [u8; 4],              // "CIK2"
    version_major: u16,
    version_minor: u16,
    header_len: u16,
    reserved0: u16,

    key_block_id: u32,
    index_root_id: u32,
    key_count: u64,
    encoding_kind: u8,
    comparator_kind: u16,
    flags: u8,
    key_data_offset: u64,
    key_data_length: u64,
    checksum: u32,
}

struct CoviIndexEntryV2 {
    entry_ref: u32,             // dense local index in the owning entry block
    index_root_id: u32,
    entry_id: u64,              // stable/debug identifier; not the reference space
    key_kind: u8,
    comparator_kind: u16,
    flags: u8,
    key_offset: u64,          // into root key block
    key_length: u32,
    key_hash64: u64,          // hint only unless hash index declares collision policy
    postings_ref: u32,
    coverage_set_ref: u32,
    aggregate_answer_ref: u32,
    next_duplicate_ref: u32,  // u32::MAX when absent
    checksum: u32,
}
```

**Key-block rules:**
- `CoviKeyBlockHeaderV2.magic` MUST be `"CIK2"`; unsupported major versions make the key block unsupported.
- `header_len` MUST cover the fixed header fields and `reserved0` MUST be zero.
- `key_data_offset` and `key_data_length` are relative to the start of the key-block payload and MUST lie within the block.
- The key block checksum covers the key-block header with the checksum field zeroed plus the key data bytes.

**Key rules:**
- Canonical value keys are `[value_tag: varint][canonical_value_payload]` or a length-delimited sequence of those components for tuple keys.
- `FileCode` keys are valid only for postings that are scoped to exactly one referenced COVE file and dictionary digest. Cross-file equality MUST use canonical value bytes or canonical hashes with collision resolution.
- `NumCode` keys are compared using the declared logical type and NumCode descriptor. Raw numeric bit comparison is allowed only when the descriptor declares it safe.
- `CanonicalHash64` and `CanonicalHash128` are lookup accelerators. A hash match is not equality unless the root declares collision-free construction or the entry stores canonical bytes for verification.
- Sorted indexes MUST sort entries by the declared comparator and then by canonical key bytes as a deterministic tie-breaker.
- Duplicate keys are allowed only when `next_duplicate_ref` chains or postings lists express all duplicate locations. `next_duplicate_ref` is a local entry-block reference and MUST NOT be interpreted as a file offset or global ID. Silent duplicate collapse is invalid unless the root declares aggregate-only semantics.

### 33.1.2 COVE-I Postings, Row Ranges, and Ordinal Sets

A posting maps one key to one or more candidate fragments. Postings may over-include candidates but MUST NOT under-include when the index advertises conservative coverage or exact-answer semantics.

```rust
enum CoviPostingRepresentationV2 {
    SortedFileRefs = 0,
    SortedSegmentRefs = 1,
    SortedPageRefs = 2,
    SortedMorselRefs = 3,
    RowRangeList = 4,
    RowOrdinalBitmap = 5,
    RowOrdinalDeltaVarint = 6,
    ByteRangeList = 7,
    ObjectPathRefs = 8,
    DimensionalBucketRefs = 9,
    CoverageSetRef = 10,
    Extension = 255,
}

struct CoviPostingsHeaderV2 {
    postings_ref: u32,
    index_root_id: u32,
    representation: u8,          // CoviPostingRepresentationV2
    target_granularity: u8,      // CoverageGranularityV2
    flags: u16,
    item_count: u64,
    payload_offset: u64,
    payload_length: u64,
    coverage_set_ref: u32,
    checksum: u32,
}

struct CoviFragmentRefV2 {
    file_ref: u32,
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,
    page_ref: u32,
    object_type_id: u32,
    path_ref: u32,
    dimensional_bucket_ref: u32,
    flags: u32,
    checksum: u32,
}

struct CoviRowRangePostingV2 {
    file_ref: u32,
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,          // u32::MAX when segment/global row range
    row_start: u64,
    row_count: u64,
    flags: u32,
    checksum: u32,
}

struct CoviFileRefPostingV2 {
    file_ref: u32,
    flags: u32,
    checksum: u32,
}

struct CoviSegmentRefPostingV2 {
    file_ref: u32,
    table_id: u32,
    segment_id: u32,
    flags: u32,
    checksum: u32,
}

struct CoviMorselRefPostingV2 {
    file_ref: u32,
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,
    flags: u32,
    checksum: u32,
}

struct CoviPageRefPostingV2 {
    file_ref: u32,
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,
    page_ref: u32,
    flags: u32,
    checksum: u32,
}

struct CoviByteRangePostingV2 {
    file_ref: u32,
    section_id: u32,
    offset: u64,
    length: u64,
    flags: u32,
    checksum: u32,
}

struct CoviObjectPathPostingV2 {
    file_ref: u32,
    object_type_id: u32,
    path_ref: u32,
    segment_id: u32,
    row_start: u64,
    row_count: u64,
    flags: u32,
    checksum: u32,
}

struct CoviDimensionalBucketPostingV2 {
    file_ref: u32,
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,
    dimensional_bucket_ref: u32,
    flags: u32,
    checksum: u32,
}
```

```rust
enum CoviBitmapKindV2 {
    DenseBitsetLsb0 = 0,
    SortedU32 = 1,
    SortedU64 = 2,
    DeltaVarintU32 = 3,
    RangeList = 4,
    RegisteredRoaring32 = 5,
    RegisteredRoaring64 = 6,
    Extension = 255,
}

struct CoviRowOrdinalSetHeaderV2 {
    row_ordinal_set_ref: u32,
    file_ref: u32,
    table_id: u32,
    segment_id: u32,          // u32::MAX when file/table scoped
    morsel_id: u32,           // u32::MAX when not morsel scoped
    bitmap_kind: u8,          // CoviBitmapKindV2
    flags: u8,
    reserved: u16,
    universe_row_count: u64,
    set_row_count: u64,
    payload_offset: u64,
    payload_length: u64,
    checksum: u32,
}
```

**Posting payload layouts:**

`CoviPostingsHeaderV2.payload_offset` and `CoviPostingsHeaderV2.payload_length` are relative to the `postings_payload_offset` base of the owning `CoviPostingsBlockHeaderV2`. `CoviRowOrdinalSetHeaderV2.payload_offset` and `payload_length` use the same base. The following representation payloads are normative for v2:

| Representation | Payload at `payload_offset` | Length and count rules |
| --- | --- | --- |
| `SortedFileRefs` | `CoviFileRefPostingV2[item_count]` | `payload_length == item_count * encoded_len(CoviFileRefPostingV2)`. Entries sorted by `file_ref`. |
| `SortedSegmentRefs` | `CoviSegmentRefPostingV2[item_count]` | Entries sorted by `(file_ref, table_id, segment_id)`. |
| `SortedMorselRefs` | `CoviMorselRefPostingV2[item_count]` | Entries sorted by `(file_ref, table_id, segment_id, morsel_id)`. |
| `SortedPageRefs` | `CoviPageRefPostingV2[item_count]` | Entries sorted by `(file_ref, table_id, segment_id, morsel_id, page_ref)`. |
| `RowRangeList` | `CoviRowRangePostingV2[item_count]` | Ranges sorted by `(file_ref, table_id, segment_id, morsel_id, row_start)`, non-overlapping, and coalesced where adjacent. |
| `RowOrdinalBitmap` | `u32 row_ordinal_set_ref[item_count]` | Each ref MUST identify a `CoviRowOrdinalSetHeaderV2` in the owning postings block with bitmap-compatible `bitmap_kind`. |
| `RowOrdinalDeltaVarint` | `u32 row_ordinal_set_ref[item_count]` | Each ref MUST identify a `CoviRowOrdinalSetHeaderV2` whose `bitmap_kind` is `DeltaVarintU32` or a compatible required extension. |
| `ByteRangeList` | `CoviByteRangePostingV2[item_count]` | Byte ranges sorted by `(file_ref, section_id, offset)`, non-overlapping, and within the validated referenced file/section. |
| `ObjectPathRefs` | `CoviObjectPathPostingV2[item_count]` | Entries sorted by `(file_ref, object_type_id, path_ref, segment_id, row_start)`. |
| `DimensionalBucketRefs` | `CoviDimensionalBucketPostingV2[item_count]` | Entries sorted by `(dimensional_bucket_ref, file_ref, table_id, segment_id, morsel_id)`. |
| `CoverageSetRef` | no payload bytes | `payload_length == 0`, `item_count == 1`, and `coverage_set_ref` MUST identify a validated coverage set. |
| `Extension` | extension-defined | Required extension defines payload layout, sorting, duplicate, false-negative, and validation rules. |

For every fixed-structure array listed in the table, `payload_length` MUST equal `item_count * encoded_len(payload_struct)` unless the representation explicitly states otherwise. `encoded_len(T)` means the fixed wire length of the named COVE-I posting structure as emitted field-by-field in little-endian order, including its checksum field. Readers MUST NOT infer payload layout from native struct size or padding.

**Posting rules:**
- Posting items MUST be sorted in deterministic target order and duplicates MUST be removed unless a required extension defines multiset postings.
- Row ranges MUST be sorted, non-overlapping, and coalesced when adjacent.
- `CoviPostingsHeaderV2.payload_length` MUST match the representation's fixed layout or registered extension layout exactly. Trailing bytes are invalid.
- `DenseBitsetLsb0` uses the same bit order as COVE null bitmaps: row ordinal `i` uses bit `(i & 7)` of byte `(i >> 3)`. Unused high bits in the final byte MUST be zero.
- `SortedU32`, `SortedU64`, and `DeltaVarintU32` payloads are exact lists of row ordinals in ascending order.
- `RegisteredRoaring32` and `RegisteredRoaring64` are reserved names until a companion COVE-I bitmap specification defines exact bytes and vectors. They MUST NOT be required for broad COVE-I conformance before that companion spec exists.
- A posting with `CoverageSetRef` MUST reference a validated COVE-COVERAGE set that obeys the same snapshot and overlay validity rules as the index root.

### 33.1.3 COVE-I Aggregate and Index-Only Payloads

```rust
struct CoviAggregateAnswerV2 {
    aggregate_answer_ref: u32,
    index_root_id: u32,
    aggregate_kind: u16,       // count, min, max, sum, avg, distinct_count, exists, membership
    exactness: u8,
    null_semantics: u8,
    flags: u16,
    row_count: u64,
    null_count: u64,
    non_null_count: u64,
    value_ref: u32,            // canonical scalar/list payload or extension payload
    predicate_form_ref: u32,   // u32::MAX when unfiltered
    snapshot_validity_ref: u32,
    checksum: u32,
}
```

**Aggregate/index-only rules:**
- Exact aggregate answers MUST be computed over the selected snapshot, schema, external visibility overlay, redaction policy, and COVE-MAP projection semantics when applicable.
- Approximate aggregate answers MUST carry approximate exactness and MUST NOT answer exact queries without explicit approximate query semantics.
- `sum` and `avg` payloads MUST declare decimal scale, overflow policy, NaN handling, and redaction policy through `value_ref` or a required extension payload.
- A COVE-I index-only answer MUST be rejected when the required visibility overlay, source projection version, or semantic-map fingerprint does not match the selected dataset state.

**Supported index mappings include:**

| Mapping | Meaning |
| --- | --- |
| `value -> file_id` | Candidate files for equality, membership, or range predicates. |
| `value -> segment_id` | Candidate table segments or temporal segments. |
| `value -> page_id` | Candidate pages. |
| `value -> morsel_id` | Candidate morsels. |
| `value -> row_range` | Candidate physical row ranges. |
| `value -> row_ordinal_set` | Candidate row ordinal bitmap or compressed set. |
| `path -> object_path` | Candidate object/path fragments for COVE-O/COVE-MAP. |
| `dimension_tuple -> dimensional_bucket` | Candidate dimensional buckets for spatial, genomic, temporal, or object-dimensional layouts. |
| `association_endpoint -> association fragment` | Candidate association/link object records. |

**Rules:**
- COVE-I is optional. A conforming COVE-Core/COVE-T reader MUST NOT require `.covi` artifacts for ordinary logical decode.
- A COVE-I artifact MUST declare dataset, snapshot, file, schema, semantic-map, and digest validity sufficient for the requested operation.
- A stale, corrupt, unsupported, or mismatched COVE-I artifact MUST be ignored or cause rejection only when the requested operation explicitly requires that index.
- A COVE-I artifact MUST NOT change COVE logical values, COVE-O reconstruction, COVE-MAP identity, external visibility overlays, or table/catalog semantics.
- COVE-I index roots may advertise conservative coverage, exact answer, approximate answer, or advisory capabilities. Readers MUST interpret each capability under its declared proof strength and exactness.
- COVE-I global indexes SHOULD be referenced from COVM or an external catalog by digest and snapshot ID.

**Reference-code sidecar discovery:** The reference DataFusion adapter MAY discover local optional COVE-I sidecars next to a mounted file using `file.cove.covi` before `file.covi`. Discovery is an implementation convenience only. A discovered sidecar MUST pass the same referenced-file, snapshot, schema, semantic-map, visibility, and capability validation as an explicitly supplied artifact before it can affect planning or index-only answers. Invalid, stale, missing, or unsupported discovered sidecars MUST fall back to ordinary scans unless the caller explicitly requires the index operation.

### 33.2 Secondary Index Capabilities and Index-Only Access

A COVE-I or COVX index may declare operations it can answer or accelerate. Capability declarations are not enough for correctness; the index must also validate against the selected snapshot and proof semantics.

```rust
struct IndexCapabilityV2 {
    capability_id: u32,
    index_root_id: u32,
    flags: u32,

    supports_eq: u8,
    supports_range: u8,
    supports_membership: u8,
    supports_prefix: u8,
    supports_contains: u8,
    supports_count: u8,
    supports_min: u8,
    supports_max: u8,
    supports_sum: u8,
    supports_distinct_count: u8,
    supports_join_coverage: u8,
    supports_index_only: u8,

    exactness: u8,             // exact, approximate, advisory
    proof_strength: u8,
    null_semantics: u8,
    reserved: u8,

    snapshot_validity_ref: u32,
    coverage_provider_ref: u32,
    checksum: u32,
}

struct IndexOnlyCapabilityV2 {
    capability_id: u32,
    aggregate_kind: u16,       // count, min, max, sum, avg, distinct_count, exists, membership
    predicate_supported: u8,
    exactness: u8,
    null_semantics: u8,
    flags: u8,
    snapshot_validity_ref: u32,
    required_visibility_overlay_ref: u32,
    checksum: u32,
}
```

**Rules:**
- Exact index-only capabilities MAY be used for exact query answers only when the index, snapshot validity, null semantics, predicate form, collation, and external visibility overlay rules all validate.
- Approximate index-only capabilities MUST be surfaced as approximate and MUST NOT answer exact SQL queries unless the query explicitly requests approximate semantics.
- Index-only counts, min/max, distinct counts, and existence checks MUST account for nulls, redactions, external overlays, and COVE-MAP projection semantics according to declared policy.
- If a non-empty external delete or visibility overlay is active, physical-file index-only aggregate answers are invalid unless an overlay-aware correction or proof is declared and validated.
- Readers MAY use index-only capabilities to avoid opening `.cove` files only when the manifest or index artifact provides sufficient digest and snapshot validation.

---


## 34. Aggregate Synopsis Indexes

Aggregate synopses allow metadata-answerable queries and faster aggregation.

```rust
struct AggregateSynopsisEntryV2 {
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,       // u32::MAX for segment-level synopsis
    column_id: u32,

    synopsis_kind: u8,
    key_kind: u8,
    accuracy: u8,         // 0=exact, 1=approximate
    flags: u8,

    row_count: u32,
    null_count: u32,

    payload_offset: u64,
    payload_length: u64,

    checksum: u32,
}
```

```rust
enum SynopsisKind {
    Count = 0,
    MinMax = 1,
    Sum = 2,
    SumAndCount = 3,
    BoolTrueFalseCounts = 4,
    FileCodeHistogram = 5,
    NumCodeHistogram = 6,
    DistinctSketch = 7,
    QuantileSketch = 8,
    TopK = 9,
}
```

`payload_offset` is section-relative. For canonical writers, all entries are
written first, then non-empty payloads are written in entry order immediately
after the entry table. `Count` entries MUST have `payload_length = 0`.

Payload-bearing entries use this common payload header:

```rust
struct AggregatePayloadHeaderV2 {
    magic: [u8; 4],       // "AGS2"
    synopsis_kind: u8,    // MUST match AggregateSynopsisEntryV2.synopsis_kind
    version: u8,          // 1
    flags: u16,
    item_count: u32,
    aux0: u32,            // kind-specific parameter
    aux1: u32,            // kind-specific parameter
    data_length: u32,
    checksum: u32,        // CRC-32C of header with checksum zeroed plus data
}
```

**Payload encodings:**
- `MinMax`: `min` then `max` as optional tagged canonical values. Each value is
  `tag:u16`, `reserved:u16`, `length:u32`, `payload:[u8; length]`; absent is
  encoded with `tag = u16::MAX` and `length = 0`. Both values are absent only
  when `row_count == null_count`.
- `Sum`: `aux0` is `NumericAggregateOverflowPolicy` (`0=checked_exact`,
  `1=saturating`, `2=wrapping`, `3=decimal_widened`), followed by one tagged
  canonical numeric sum. Exact SQL consumers MUST require `checked_exact`.
- `SumAndCount`: same `aux0` policy as `Sum`; data starts with
  `non_null_count:u64`, followed by one tagged canonical numeric sum.
- `BoolTrueFalseCounts`: `true_count:u64`, `false_count:u64`; the two counts
  MUST sum to `row_count - null_count`.
- `FileCodeHistogram` and `NumCodeHistogram`: sorted `(key:u64, count:u64)`
  records. Keys MUST be strictly ascending and counts MUST be non-zero. For
  exact synopses, counts MUST sum to `row_count - null_count`.
- `TopK`: `aux0` stores `k`; data is `(key:u64, count:u64)` records sorted by
  descending count and then ascending key. `item_count <= k`.
- `DistinctSketch`: HyperLogLog registers. `aux0` stores precision `p`; default
  is `p = 14`. Register count MUST be `2^p`. COVE hashes canonical value bytes
  with its deterministic 64-bit sketch hash before HLL update.
- `QuantileSketch`: KLL compactors. `aux0` stores `k`; default is `k = 200`.
  Data starts with `value_tag:u16`, `reserved:u16`, `level_count:u32`, followed
  by monotonic `level_offsets:u32[level_count]`, then length-prefixed canonical
  values. The final level offset MUST equal the value count. Compaction uses
  deterministic tie-breaking so fixtures are reproducible.

**Rules:**
- Exact synopses MAY be used for exact query results only when visibility is
  all-visible and no redaction policy can affect the answer.
- Approximate synopses MUST be marked approximate.
- Approximate synopses MUST NOT be used for exact answers.
- Payload kind MUST match `synopsis_kind`; readers MUST validate checksum,
  payload bounds, sorted keys, duplicate keys, count totals, canonical value
  tags/payloads, and `accuracy`.
- Sum payloads MUST declare overflow and decimal handling rules.
- Redacted values MUST follow declared redaction aggregation policy.

**Important use cases:**
SELECT count(*) FROM table;

SELECT min(created_at), max(created_at) FROM table;

SELECT status, count(*)
FROM admissions
GROUP BY status;

SELECT count(*)
FROM admissions
WHERE status = 'active';
For low-cardinality FileCode columns, FileCodeHistogram is especially important.

---


## 35. Composite Zone Indexes

Composite zone indexes support multi-column pruning.
**Example:**
WHERE scope = 'A'
  AND event_date = '2026-05-01'
  AND status = 'failed'

```rust
struct CompositeZoneIndexHeaderV2 {
    table_id: u32,

    key_column_count: u16,
    transform_kind: u8,
    // 0=tuple
    // 1=z_order
    // 2=hilbert
    // 3=writer_defined

    flags: u8,

    zone_count: u32,

    key_columns_offset: u64,
    entries_offset: u64,
    entries_length: u64,

    checksum: u32,
}
```

**Each composite zone entry MAY contain:**
- composite min key,
- composite max key,
- optional exact composite set,
- optional composite bloom,
- covered segment/morsel range.
**Rules:**
- Tuple transform uses lexicographic tuple ordering.
- Z-order and Hilbert transforms MUST declare exact encoding rules.
- writer_defined transforms require a required feature bit.
- Unknown composite transforms MUST NOT be used for pruning.
**Recommended use:**
scope + date
site + status
customer + event_type
branch + object_type
partition columns + time

---


## 36. Top-N Zone Summaries

Top-N summaries accelerate ordered limit queries.
**Example:**
SELECT *
FROM events
ORDER BY risk_score DESC
LIMIT 100;

```rust
struct TopNZoneSummaryV2 {
    table_id: u32,
    column_id: u32,
    segment_id: u32,
    morsel_id: u32,

    direction: u8,       // 0=top, 1=bottom
    value_count: u16,
    flags: u8,

    payload_offset: u64,
    payload_length: u64,

    checksum: u32,
}
```

**Rules:**
- Top-N summaries are optional.
- Corrupt summaries MUST be ignored.
- Readers MAY skip zones whose bounds cannot beat the current Top-N threshold.
- Readers MUST preserve stable query semantics, tie handling, and null ordering.

---


## 37. COVE-T Scan Semantics

**A COVE-T scan SHOULD proceed as:**
1. Validate header, postscript, footer, and section directory.
2. Validate table catalog and required dictionary/domain sections.
3. Resolve query literals to FileCodes or NumCodes where possible.
4. Generate scan splits from table segment index.
5. Use COVM file-level pruning if available.
6. Apply segment-level zone stats.
7. Apply morsel-level zone stats.
8. Evaluate predicate proof outcomes.
9. Use exact sets.
10. Use bloom filters.
11. Use inverted morsel indexes.
12. Use lookup indexes for point predicates where available.
13. Use composite zone indexes for multi-column predicates.
14. Use aggregate synopses for metadata-answerable queries.
15. Decode predicate pages for Unknown surviving zones.
16. Build row selection bitmaps.
17. Late-materialise projected columns.
18. Remap FileCodes to ExecutionCodes if supported by the engine.
19. Return engine-native vectors or Arrow-compatible arrays.

### 37.1 Equality on FileCode Column

**For:**
WHERE status = 'active'
**Planner:**
1. Resolve 'active' in COVE dictionary.
2. If absent, predicate is DefinitelyNo for the file.
3. If present, obtain FileCode.
4. Use exact sets, blooms, inverted indexes, and lookup indexes.
5. Decode surviving pages.
6. Remap FileCode -> ExecutionCode for output/execution if supported.

### 37.2 Range on FileCode Column

**For:**
WHERE surname >= 'M' AND surname < 'T'
**Planner:**
1. Use ColumnDomain collation.
2. Convert bounds to domain-rank interval.
3. Compare zone min_domain_rank/max_domain_rank.
4. Skip DefinitelyNo zones.
5. Accept DefinitelyYes zones when safe.
6. Decode Unknown zones.
If no safe ColumnDomain exists, range pushdown MUST fall back to scan.

### 37.3 Numeric Predicate

**For:**
WHERE age BETWEEN 18 AND 65
**Planner:**
1. Use typed NumCode min/max.
2. Evaluate DefinitelyNo/DefinitelyYes/Unknown.
3. Decode Unknown zones.

### 37.4 Null Predicate

WHERE col IS NULL
**Skip zone if:**
null_count = 0
**Accept whole zone if:**
null_count = row_count
WHERE col IS NOT NULL
**Skip zone if:**
null_count = row_count
**Accept whole zone if:**
null_count = 0

### 37.5 Predicate Reordering

Readers MAY reorder conjunctive predicates when reordering preserves semantics.
**Writers SHOULD expose selectivity hints:**
- null_count,
- distinct_count,
- exact set cardinality,
- run_count,
- constant flag,
- sortedness,
- encoded size,
- histogram synopsis.


### 37.6 Encoded Predicate Evaluation and Late Materialisation

COVE readers SHOULD avoid logical materialisation until it is necessary for projection, export, or an unsupported predicate. This is a performance rule only; it MUST NOT change logical results.

**Recommended scan shape:**
1. use COVM, COVE-COVERAGE, COVE-I, COVX, zone stats, exact sets, blooms, and ColumnDomains to derive a conservative candidate fragment set;
2. evaluate safe predicates against encoded FileCode, NumCode, local-codebook, RLE, bit-packed, or registered codec streams when an equivalent encoded predicate kernel is declared;
3. produce a selection bitmap or row-id vector;
4. decode only selected rows for projected columns;
5. preserve dictionary/code vectors where the output engine can use them safely;
6. materialise Arrow-owned arrays, Arrow-view arrays, COVE-native views, or engine-native vectors according to export capability.

```rust
enum ExportCapabilityKindV2 {
    ArrowOwnedArray = 0,
    ArrowViewArray = 1,
    ArrowDictionaryArray = 2,
    CoveNativeView = 3,
    SelectionBitmap = 4,
    RowIdVector = 5,
    EngineNativeVector = 6,
}

struct ExportCapabilityV2 {
    capability_id: u32,
    target_kind: u16,
    table_id: u32,
    column_id: u32,
    logical_type: u16,
    physical_kind: u8,
    flags: u8,
    requires_owned_buffers: u8,
    supports_zero_copy: u8,
    supports_late_materialisation: u8,
    supports_dictionary_preservation: u8,
    null_bitmap_polarity: u8,
    dictionary_key_width_bits: u16,
    lifetime_policy: u8,
    reserved: [u8; 3],
    checksum: u32,
}
```

**Rules:**
- Encoded predicate evaluation is allowed only when the physical encoding, NumCode descriptor, codec descriptor, null semantics, collation, NaN/signed-zero rules, and predicate form declare equivalence to baseline logical evaluation.
- A reader MUST fall back to logical decode for unsupported or unsafe encoded predicates.
- Selection bitmaps and row-id vectors are intermediate execution artifacts. They MUST NOT be persisted as canonical COVE truth unless a future required extension defines such a section.
- Late materialisation MUST preserve row order, null positions, redaction policy, dictionary semantics, and projection semantics.
- A reader MAY expose zero-copy views only when the target format's alignment, lifetime, null polarity, key width, offset width, dictionary semantics, and ownership rules are satisfied.

---
