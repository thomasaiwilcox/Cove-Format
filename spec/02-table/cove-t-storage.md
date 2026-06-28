# COVE-T Storage

## 24. COVE-T Table Catalog

```rust
struct TableCatalogV2 {
    table_count: u32,
    flags: u32,

    tables: [TableEntryV2],
}
```

```rust
struct TableEntryV2 {
    table_id: u32,

    namespace_len: u16,
    namespace: [u8],

    table_name_len: u16,
    table_name: [u8],

    column_count: u32,
    row_count: u64,

    primary_sort_key_count: u16,
    clustering_key_count: u16,

    flags: u32,

    columns: [TableColumnEntryV2],
}
```

```rust
struct TableColumnEntryV2 {
    column_id: u32,

    column_name_len: u16,
    column_name: [u8],

    logical_type: u16,
    physical_kind: u8,
    nullable: u8,

    sort_order: u16,
    collation_id: u16,

    precision: u16,
    scale: i16,

    flags: u32,
}
```

**Rules:**
- table_id MUST be unique.
- column_id MUST be unique within a table.
- logical_type and physical_kind MUST be compatible.
- nullable=false means all corresponding null counts MUST be zero.

---


## 25. COVE-T Table Segments

A table segment is a contiguous row range for one table.
**Recommended writer targets:**
**segment target uncompressed size:**
  64 MiB to 256 MiB

**morsel row count:**
  4096 default
  8192 for very narrow tables

### 25.1 Table Segment Index Entry

```rust
struct TableSegmentIndexEntryV2 {
    table_id: u32,
    segment_id: u32,

    row_start: u64,
    row_count: u32,

    morsel_count: u32,
    morsel_row_count: u32,

    column_count: u32,

    offset: u64,
    length: u64,

    stats_ref: u32,

    flags: u32,

    checksum: u32,
}
```

### 25.2 Table Segment Header

```rust
struct TableSegmentHeaderV2 {
    table_id: u32,
    segment_id: u32,

    row_start: u64,
    row_count: u32,

    morsel_count: u32,
    morsel_row_count: u32,

    column_count: u32,

    morsel_directory_offset: u64,
    column_directory_offset: u64,
    page_index_offset: u64,
    data_offset: u64,

    flags: u32,

    checksum: u32,
}
```

**Rules:**
- segment_id MUST be unique within table_id.
- row_count MUST equal the sum of row counts in the segment's morsels.
- Last morsel MAY contain fewer rows.
- Segment checksum MUST validate before internal offsets are trusted.

---


## 26. COVE-T Row Morsels

```rust
struct RowMorselEntryV2 {
    morsel_id: u32,

    first_row_in_segment: u32,
    row_count: u32,

    flags: u32,

    stats_ref: u32,

    checksum: u32,
}
```

**Rules:**
- Morsels MUST be ordered by first_row_in_segment.
- Morsel row ranges MUST be contiguous and non-overlapping.
- All columns in a segment MUST use the same morsel boundaries.

---


## 27. COVE-T Column Directory and Pages

### 27.1 Column Directory Entry

```rust
struct TableColumnDirectoryEntryV2 {
    column_id: u32,

    logical_type: u16,
    physical_kind: u8,
    flags: u8,

    page_index_offset: u64,
    page_index_length: u64,

    data_offset: u64,
    data_length: u64,

    stats_ref: u32,
    domain_ref: u32,

    checksum: u32,
}
```

### 27.2 Column Page Index Entry

```rust
struct ColumnPageIndexEntryV2 {
    column_id: u32,
    morsel_id: u32,

    row_count: u32,
    non_null_count: u32,
    null_count: u32,

    encoding_root: u32,

    page_offset: u64,
    page_length: u64,

    uncompressed_length: u64,

    stats_ref: u32,

    flags: u32,

    checksum: u32,
}
```

**Rules:**
- One column page SHOULD exist per column per morsel.
- row_count MUST equal the referenced morsel row_count.
- null_count + non_null_count MUST equal row_count.
- For non-nullable columns, null_count MUST be zero.
- Page checksum covers page payload.
**Page flags:**

| Bits | Name | Meaning |
| --- | --- | --- |
| 0x0000_00FF | PAGE_FLAG_COMPRESSION_CODEC | Page-level `CompressionCodec` value from Section 66. |
| 0x0000_0100 | PAGE_FLAG_STATS_ONLY_CONSTANT | No page payload exists; the page is reconstructed from page index counts and, for all-non-null pages, a validated page-level ZoneStatsEntry. Requires FEATURE_PAGE_PAYLOAD_ELISION. |
| 0x0000_0200 | PAGE_FLAG_ALL_NULL | Every row in the page is null. This fact flag does not by itself require FEATURE_PAGE_PAYLOAD_ELISION. |
| 0x0000_0400 | PAGE_FLAG_ALL_NON_NULL | Every row in the page is non-null. This fact flag does not by itself require FEATURE_PAGE_PAYLOAD_ELISION. |
| 0x0000_0800 | PAGE_FLAG_VALUE_STREAM_ELIDED | The non-null value stream is elided because the non-null value is constant. A null bitmap may still be present unless ALL_NULL or ALL_NON_NULL is set. Requires FEATURE_PAGE_PAYLOAD_ELISION. |
| 0xFFFF_F000 | reserved | Reserved for future required page extensions; MUST be zero in v2 unless a required extension defines the bit and the reader supports that extension. |

**Page codec rules:**
- PAGE_FLAG_COMPRESSION_CODEC applies only to the page payload bytes referenced by `page_offset` and `page_length`.
- Codec `None` requires `page_length == uncompressed_length`.
- LZ4 and Zstd page payloads use the same block codec definitions as Section 66 and require `uncompressed_length` to be the exact decoded byte length.
- If `page_length == 0`, `uncompressed_length` MUST also be zero.
- If `page_length > 0` and the page codec is not `None`, `uncompressed_length` MUST be non-zero.
- Writers that use LZ4 or Zstd page codecs MUST advertise the corresponding `FEATURE_CODEC_LZ4` or `FEATURE_CODEC_ZSTD` bit.
- Readers MUST reject unknown page codec values and any non-zero reserved page flag bits unless a required extension defines the bit and the reader supports that extension.

**Page flag consistency:**
- PAGE_FLAG_ALL_NULL and PAGE_FLAG_ALL_NON_NULL are exact page facts. They are not feature-gated when the ordinary page payload still carries all decode-required data.
- Payload-elision flags are decode-affecting metadata. Writers that use PAGE_FLAG_STATS_ONLY_CONSTANT or PAGE_FLAG_VALUE_STREAM_ELIDED MUST set FEATURE_PAGE_PAYLOAD_ELISION in required_features. A reader that does not support FEATURE_PAGE_PAYLOAD_ELISION MUST reject the file before decoding those pages.
- PAGE_FLAG_ALL_NULL and PAGE_FLAG_ALL_NON_NULL are mutually exclusive.
- PAGE_FLAG_ALL_NULL requires null_count == row_count and non_null_count == 0. The null bitmap MAY be omitted only when PAGE_FLAG_STATS_ONLY_CONSTANT is set and FEATURE_PAGE_PAYLOAD_ELISION is required; any present null bitmap MUST contain only null bits for rows in the page with unused final-byte bits zeroed.
- PAGE_FLAG_ALL_NON_NULL requires null_count == 0 and non_null_count == row_count. The null bitmap MAY be omitted because every row is non-null; any present null bitmap MUST contain only zero bits with unused final-byte bits zeroed.
- If neither PAGE_FLAG_ALL_NULL nor PAGE_FLAG_ALL_NON_NULL is set, the counts still determine how much null-position information is required. A mixed null/non-null page MUST include a validated null-position representation; a page with null_count == 0 MAY omit the null bitmap.
- Page flags MUST be internally consistent with row_count, null_count, non_null_count, page_length, uncompressed_length, encoding_root, checksum, and any referenced stats_ref. A mismatch is page corruption; flags are not hints and MUST NOT override the counts or validated payload metadata.
- PAGE_FLAG_VALUE_STREAM_ELIDED requires the non-null value to be reconstructable from Constant encoding parameters or, only when PAGE_FLAG_STATS_ONLY_CONSTANT is also set, from the validated page-level ZoneStatsEntry rules below.

**Rules for payload-elided pages:**
- page_length MAY be zero only when PAGE_FLAG_STATS_ONLY_CONSTANT is set.
- If PAGE_FLAG_STATS_ONLY_CONSTANT is set, PAGE_FLAG_COMPRESSION_CODEC MUST be `CompressionCodec::None`, page_offset and uncompressed_length MUST be zero, encoding_root MUST be u32::MAX, and checksum MUST be CRC32C of the empty byte string.
- PAGE_FLAG_STATS_ONLY_CONSTANT requires either PAGE_FLAG_ALL_NULL or PAGE_FLAG_ALL_NON_NULL. Mixed null/non-null constant pages still need a null-position representation and therefore MUST NOT be stats-only.
- For all-null stats-only pages, null_count MUST equal row_count and non_null_count MUST be zero.
- For all-non-null stats-only pages, non_null_count MUST equal row_count, null_count MUST be zero, and stats_ref MUST reference a validated page-level ZoneStatsEntry with IS_CONSTANT and min_value == max_value under the declared logical type and collation rules.
- For Float32 stats-only constant pages, the stats entry MUST preserve the exact raw IEEE value bits needed for reconstruction. Because v2 has no `StatKind::Float32Bits`, decode-required Float32 stats-only constants MUST use `StatKind::FixedBytes` with exactly 4 little-endian raw IEEE bytes; ordinary advisory Float32 pruning min/max may remain approximate or normalized where permitted by the statistics rules, but those advisory values MUST NOT be used as the reconstruction source for a stats-only constant page. If exact Float32 bits are not represented, including NaN payloads or signed-zero distinctions, the constant value MUST be stored in Constant parameters instead of stats-only storage.
- For Float64 stats-only constant pages, `StatKind::Float64Bits` may be used only for non-NaN values because ZoneStats Float64 min/max scalars MUST NOT contain NaN. Float64 NaN constants, including payload-preserving NaNs, MUST use a payload-backed Constant or value-stream representation rather than stats-only reconstruction.
- When PAGE_FLAG_STATS_ONLY_CONSTANT is set on an all-non-null page, the referenced stats entry is decode-required canonical data for that page, not optional pushdown metadata. A reader that cannot validate it MUST reject the page rather than fail open.
- A stats-only constant page MUST declare or imply `PageReconstructionSource::StatsConstant` and MUST NOT be treated as a normal optional statistics optimisation.
- A stats-only constant page MUST NOT use truncated `StatScalar` values for reconstruction. Truncated min/max may remain advisory pruning metadata, but they cannot be the only source of a decoded constant value.
- If the page contains redacted values, the reconstruction source MUST preserve the redaction marker and policy reference. A redacted constant MUST NOT be reconstructed as null or as the unredacted value.

### 27.3 Page Payload

**A column page payload contains:**
[column page header]
[encoding node descriptors]
[buffer directory]
[buffers]

```rust
struct ColumnPagePayloadHeaderV2 {
    magic: [u8; 4],          // "CPG2"
    version_major: u16,      // 2
    header_len: u16,         // 36
    flags: u16,              // reserved, MUST be 0
    root_node_id: u16,
    node_count: u16,
    buffer_count: u16,
    row_count: u32,
    nodes_offset: u32,
    buffer_directory_offset: u32,
    buffers_offset: u32,
    reserved: u32,           // MUST be 0
}

enum PageBufferKind {
    NullBitmap = 0,
    Values = 1,
    Offsets = 2,
    ChildLayout = 3,
    Other = 255,
}

struct PageBufferDescriptorV2 {
    buffer_id: u16,           // dense 0..buffer_count-1
    kind: u16,                // PageBufferKind
    flags: u32,               // reserved, MUST be 0
    offset: u64,              // byte offset within this page payload
    length: u64,
    checksum: u32,            // CRC32C of this buffer
    reserved: u32,            // MUST be 0
}
```

**Container rules:**
- `nodes_offset` MUST equal `header_len`.
- `buffer_directory_offset` MUST equal `nodes_offset + node_count * 30`.
- `buffers_offset` MUST equal `buffer_directory_offset + buffer_count * 32`.
- `root_node_id` MUST identify exactly one `CoveEncodingNodeV2`, and that node's `logical_len` MUST equal the page row count.
- Buffer descriptors MUST be dense by `buffer_id`, in ascending non-overlapping offset order, and every buffer MUST lie inside the page payload.
- A non-elided page payload MUST be fully consumed by its buffer descriptors; trailing bytes are invalid.
- A buffer descriptor checksum mismatch is a page checksum failure.

**Logical row reconstruction:**
1. if PAGE_FLAG_STATS_ONLY_CONSTANT is set, reconstruct all rows from page index counts and, for all-non-null pages, the validated page-level stats entry;
2. otherwise read the null bitmap if present,
3. decode the non-null value stream,
4. re-expand values into logical row order.

---


## 28. COVE-T Zone Statistics

**Zone statistics exist at:**
- file/table level,
- segment level,
- morsel level,
- page level.
Morsel-level statistics are the default pruning unit.

```rust
struct ZoneStatsEntryV2 {
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,       // u32::MAX for segment-level stats
    column_id: u32,

    row_count: u32,
    null_count: u32,
    non_null_count: u32,

    distinct_count: u32,
    run_count: u32,

    flags: u32,

    min_value: StatScalarV2,
    max_value: StatScalarV2,

    min_domain_rank: u32,
    max_domain_rank: u32,

    exact_set_ref: u32,
    bloom_ref: u32,
}
```

```rust
struct StatScalarV2 {
    stat_kind: u8,
    flags: u8,
    length: u16,
    data: [u8; 16],
}
```

```rust
enum StatKind {
    None = 0,
    Int64 = 1,
    UInt64 = 2,
    Float64Bits = 3,
    Decimal128 = 4,
    TimestampMicros = 5,
    TimestampNanos = 6,
    DateDays = 7,
    FixedBytes = 8,
}
```

**StatScalar flags:**

| Bit | Name | Meaning |
| --- | --- | --- |
| 0 | STAT_SCALAR_TRUNCATED | This scalar is a truncated bound. |
| 1-7 | reserved | MUST be zero in v2. |

**StatScalar length rules:**

| StatKind | Required length |
| --- | --- |
| None | 0 |
| Int64 | 8 |
| UInt64 | 8 |
| Float64Bits | 8 |
| Decimal128 | 16 |
| TimestampMicros | 8 |
| TimestampNanos | 8 |
| DateDays | 4 |
| FixedBytes | 0..16 |

`MINMAX_TRUNCATED` MUST be set if and only if `STAT_SCALAR_TRUNCATED` is set on either the min or max scalar.

Float64 min/max scalars MUST NOT contain NaN. If a zone contains NaN values, writers MUST set `HAS_NAN` and exclude NaN from min/max.

**Stats flags:**

| Flag | Meaning |
| --- | --- |
| HAS_MIN_MAX | min/max are valid for conservative pruning. |
| HAS_DOMAIN_RANGE | min_domain_rank/max_domain_rank are valid. |
| DISTINCT_EXACT | distinct_count is exact. |
| IS_CONSTANT | all non-null values are equal. |
| IS_SORTED_ASC | non-null values sorted ascending. |
| IS_SORTED_DESC | non-null values sorted descending. |
| HAS_NAN | float data contains NaN. |
| HAS_REDACTED | zone contains redacted values. |
| MINMAX_TRUNCATED | bounds are truncated and require caution. |
| HAS_TOP_N_SUMMARY | top summary available. |
| HAS_BOTTOM_N_SUMMARY | bottom summary available. |

**Rules:**
- `ZoneStatsEntryV2` has no explicit scope field. Segment-level entries are encoded by `morsel_id == u32::MAX`; morsel-level entries use the concrete morsel id. Page-level use is contextual: a page-level stats entry is proven only when a referencing page selects it by `stats_ref` and the entry matches the page's table, segment, morsel, column, row_count, null_count, and non_null_count metadata.
- For NumCode columns, min/max are interpreted by logical_type.
- For FileCode columns, range stats use domain ranks.
- Raw FileCode min/max MUST NOT be used for logical range pruning.
- Unsafe or truncated bounds MUST NOT be used for exclusion unless rules prove safety.

---
