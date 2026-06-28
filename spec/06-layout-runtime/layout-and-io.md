# I/O, Layout, and Runtime Planning

## 67. I/O and Mechanical Sympathy

**COVE writers SHOULD organise files for:**
- tail bootstrap,
- object-store range reads,
- metadata-first pruning,
- predicate-first scans,
- read coalescing,
- late materialisation,
- column projection,
- morsel-level scheduling.
**Optional I/O hints:**

```rust
struct CoveIoHintV2 {
    preferred_read_alignment: u32,
    preferred_coalesce_distance: u32,
    preferred_max_coalesced_read: u32,

    prefetch_group_id: u32,
    page_cluster_id: u32,

    flags: u32,
}
```

Hints are advisory only.

### 67.1 Small Page Packing

COVE does not require fixed-size allocation blocks. Writers therefore SHOULD NOT import a database-style block allocation model that wastes space for narrow columns or tiny morsel pages.
**Recommended writer policy:**
- Pack small column pages contiguously inside TABLE_SEGMENT_DATA rather than aligning every page to a large block boundary.
- Small pages from different columns and morsels MAY share a page cluster when each ColumnPageIndexEntry still identifies the exact page_offset, page_length, uncompressed_length, flags, and checksum.
- Writers SHOULD use a tunable target cluster size for read coalescing and object-store range requests. The target is a writer/I/O policy, not a required allocation unit.
- Large pages MAY be placed in dedicated aligned ranges when doing so improves direct reads or decompression.
- Packing MUST NOT merge checksums across independently addressable pages unless an additional enclosing checksum is provided; each page checksum remains authoritative for that page's bytes.
- Packing MUST preserve morsel and column page boundaries at the logical level even when physical bytes are adjacent.


### 67.2 Fast Metadata Index

COVE v2 MAY include a `FAST_METADATA_INDEX` section to make very wide schemas, large page indexes, and object-store planning cheaper to access. This index is an acceleration mirror over authoritative sections.

```rust
struct FastMetadataIndexHeaderV2 {
    entry_count: u32,
    entry_len: u16,
    index_kind: u8,
    flags: u8,
    entries_offset: u64,
    entries_length: u64,
    checksum: u32,
}

struct FastMetadataIndexEntryV2 {
    target_kind: u16,
    // 0=table
    // 1=column
    // 2=segment
    // 3=morsel
    // 4=page
    // 5=stats
    // 6=section
    // 7=layout_node

    flags: u16,

    table_id: u32,
    column_id: u32,
    segment_id: u32,
    morsel_id: u32,

    section_id: u32,
    local_id: u32,

    offset: u64,
    length: u64,

    checksum_or_crc32c: u32,
    reserved: u32,
}
```

**Rules:**
- Fast metadata entries MUST reference existing authoritative metadata.
- A mismatch between a fast metadata entry and the authoritative section invalidates the fast metadata entry.
- If the section is optional, corrupt fast metadata MUST be ignored and readers MUST fall back to the footer and profile sections.
- A writer MUST NOT rely on `FAST_METADATA_INDEX` as the only location for schema, page, or statistics metadata.
- Reference readers MAY parse fast metadata through the header section id for planning acceleration, but the footer section directory remains authoritative. Valid fast metadata may reduce metadata lookup work; it MUST NOT create or override a section, page, layout node, statistic, or proof record.

### 67.3 Page Cluster Directory

A page cluster groups nearby page payloads for efficient range reads and coalescing. It is a physical I/O hint, not a logical page boundary.

```rust
struct PageClusterDirectoryHeaderV2 {
    cluster_count: u32,
    flags: u32,
    checksum: u32,
}

struct PageClusterEntryV2 {
    cluster_id: u32,
    section_id: u32,

    offset: u64,
    length: u64,

    table_id: u32,
    segment_id: u32,
    first_morsel_id: u32,
    morsel_count: u32,

    first_page_ref: u32,
    page_count: u32,

    preferred_read_alignment: u32,
    preferred_coalesce_distance: u32,

    flags: u32,
    checksum: u32,
}
```

**Rules:**
- Page clusters MAY contain pages from multiple columns and morsels only when every page remains independently addressable by its `ColumnPageIndexEntryV2`.
- Page cluster checksums MAY provide an enclosing integrity check, but each page checksum remains authoritative for page bytes.
- Page clusters MUST NOT change row order, page row_count, null counts, or page reconstruction rules.
- Reference readers MAY use validated page clusters as advisory range-coalescing bounds. A reader MUST preserve the original page slicing, MUST NOT read outside the validated cluster byte range, and MUST fall back to ordinary range coalescing when cluster metadata is absent, corrupt, unsupported, or inconsistent with authoritative table, segment, morsel, page, or section metadata.

### 67.4 Zero-Copy Buffer Map

A zero-copy buffer map describes when COVE page buffers can be exposed directly to an output format or engine runtime. It is optional compatibility metadata.

```rust
struct ZeroCopyBufferMapHeaderV2 {
    map_count: u32,
    target_count: u32,
    flags: u32,
    checksum: u32,
}

struct ZeroCopyTargetV2 {
    target_id: u32,

    namespace_len: u16,
    namespace: [u8],

    target_name_len: u16,
    target_name: [u8],

    version_major: u16,
    version_minor: u16,

    flags: u32,
}

enum ZeroCopyNullBitmapPolarityV2 {
    OneMeansNull = 0,
    OneMeansValid = 1,
    NoNullBitmap = 2,
    TargetDefines = 255,
}

enum ZeroCopyLifetimeScopeV2 {
    Page = 0,
    Segment = 1,
    FileMapping = 2,
    ReaderSession = 3,
    ExternalOwner = 4,
    InvalidAfterDecode = 5,
}

enum ZeroCopyDictionarySemanticsV2 {
    NoDictionary = 0,
    FileCodeDictionary = 1,
    ArrowDictionaryValues = 2,
    EngineDictionary = 3,
    RequiresRemap = 4,
    Incompatible = 255,
}

enum ZeroCopyNestedLayoutKindV2 {
    NotNested = 0,
    ArrowListOffsets32 = 1,
    ArrowLargeListOffsets64 = 2,
    ArrowStructChildren = 3,
    ArrowMapOffsets32 = 4,
    CoveNativeNested = 5,
    Extension = 255,
}

enum ZeroCopyTargetBufferRoleV2 {
    Values = 0,
    ValidityBitmap = 1,
    NullBitmap = 2,
    Offsets32 = 3,
    Offsets64 = 4,
    TypeIds = 5,
    DictionaryKeys = 6,
    DictionaryValues = 7,
    ChildData = 8,
    SelectionBitmap = 9,
    RunEnds = 10,
    Extension = 255,
}

enum ZeroCopySourceBufferRoleV2 {
    CoveValues = 0,
    CoveNullBitmap = 1,
    CoveOffsets = 2,
    CoveChildLayout = 3,
    CoveDictionaryCodes = 4,
    CoveDictionaryPayload = 5,
    CoveEncodedPayload = 6,
    CoveSelectionBitmap = 7,
    CoveRunEnds = 8,
    Extension = 255,
}

struct ZeroCopyBufferMapEntryV2 {
    target_id: u32,
    table_id: u32,
    column_id: u32,
    segment_id: u32,
    morsel_id: u32,

    page_ref: u32,
    buffer_id: u16,
    buffer_kind: u16,

    logical_type: u16,
    physical_kind: u8,
    source_endianness: u8,

    required_alignment_log2: u8,
    null_bitmap_polarity: u8,      // ZeroCopyNullBitmapPolarityV2
    source_offset_width_bits: u16,
    target_offset_width_bits: u16,
    dictionary_key_width_bits: u16,

    dictionary_semantics: u8,      // ZeroCopyDictionarySemanticsV2
    lifetime_scope: u8,            // ZeroCopyLifetimeScopeV2
    nested_layout_kind: u8,        // ZeroCopyNestedLayoutKindV2
    compression_required_none: u8,

    target_buffer_role: u16,       // ZeroCopyTargetBufferRoleV2
    source_buffer_role: u16,       // ZeroCopySourceBufferRoleV2
    target_type_ref: u32,
    dictionary_values_ref: u32,
    child_layout_ref: u32,
    owner_lifetime_ref: u32,

    flags: u32,
    checksum: u32,
}
```

**Rules:**
- Zero-copy maps MUST be ignored when the target format, alignment, null bitmap polarity, key width, offset width, endianness, lifetime, dictionary semantics, nested layout, target/source buffer role, compression state, redaction state, or external visibility policy is incompatible.
- `target_buffer_role` and `source_buffer_role` MUST use `ZeroCopyTargetBufferRoleV2` and `ZeroCopySourceBufferRoleV2`. Unknown role values make the map entry unsupported unless a required extension defines the role and the reader supports that extension.
- COVE null bitmap polarity remains `1 = null`. A target requiring `1 = valid` needs inversion unless the target explicitly accepts COVE polarity. A map entry with `OneMeansValid` cannot directly expose a COVE null bitmap; it can only describe a target-native buffer already materialised or supplied by an extension.
- Direct exposure is permitted only after the page, buffer descriptor, section CRC, and any required digest have validated.
- Compressed, encrypted, encoded, transformed, or value-stream-elided buffers MUST NOT be exposed as target logical buffers unless the target explicitly expects that encoded representation and the export profile declares it as a native encoded view.
- Dictionary buffers may be exposed directly only when dictionary values, key width, null policy, ordering expectations, and dictionary lifetime match the target. FileCode values MUST NOT be exposed as Arrow dictionary keys when the key width or dictionary value order is incompatible.
- Nested offsets may be exposed only when offset width, offset origin, monotonicity, final offset, child length, parent null semantics, and child layout match the target.
- If an external delete/visibility overlay or selection bitmap is active, a zero-copy value buffer may be exposed only together with a target-compatible selection/filter representation; otherwise the reader MUST materialise the visible rows.
- `lifetime_scope` MUST be at least as long as the target consumer's access. Readers MUST materialise owned buffers when memory mapping, ref-counting, or external owner lifetime cannot be guaranteed.
- A reader MUST materialise compatible buffers rather than exposing incompatible COVE bytes.
- Zero-copy compatibility MUST NOT influence writer choices if doing so would weaken COVE encoding, predicate-proof metadata, digest coverage, or logical type fidelity.

### 67.4.1 Arrow Zero-Copy Compatibility Checklist

For Arrow export, a reader MAY expose a COVE buffer as an Arrow buffer without copying only when all of the following hold:

1. the COVE buffer contains the exact Arrow physical buffer role being requested;
2. endianness is little-endian and matches Arrow's physical layout;
3. the buffer is uncompressed and not wrapped by a block codec;
4. the buffer is not an encoded stream unless the Arrow output is an explicitly encoded extension view;
5. null bitmap polarity is compatible or no null bitmap is required;
6. offset buffers are 32-bit or 64-bit as required by the Arrow type;
7. dictionary keys and dictionary values match Arrow dictionary semantics;
8. nested child buffers and parent offsets satisfy Arrow layout invariants;
9. the selected row set is contiguous or represented by a target-compatible selection vector;
10. memory lifetime extends beyond the Arrow array consumer's use;
11. redaction, privacy, and external visibility policies permit exposing the bytes;
12. COVE checksums and required digests validate before exposure.

Failure of any checklist item requires materialised Arrow-owned output.

### 67.5 COVE-L Layout Plan Profile

COVE-L is the v2 layout-plan and scan-split profile. It borrows the useful idea of hierarchical lazy read planning but keeps COVE's explicit catalog, segment, morsel, page, and proof metadata authoritative.

```rust
struct LayoutPlanHeaderV2 {
    layout_id: u32,
    node_count: u32,
    root_node_id: u32,
    flags: u32,
    checksum: u32,
}

struct LayoutPlanNodeV2 {
    node_id: u32,
    parent_node_id: u32,       // u32::MAX for root

    node_kind: u16,
    // 0=root
    // 1=table
    // 2=segment_group
    // 3=segment
    // 4=morsel_range
    // 5=column_group
    // 6=page_cluster
    // 7=section_range
    // 255=vendor_hint

    flags: u16,

    table_id: u32,
    column_id: u32,            // u32::MAX when not column-specific
    segment_id: u32,           // u32::MAX when not segment-specific
    first_morsel_id: u32,
    morsel_count: u32,

    row_start: u64,
    row_count: u64,

    section_id: u32,
    cluster_id: u32,

    first_child_index: u32,
    child_count: u32,

    stats_ref: u32,
    split_ref: u32,

    checksum: u32,
}
```

**Rules:**
- COVE-L is optional. A COVE-T reader MUST be able to scan a valid COVE-T file without COVE-L.
- Layout nodes MUST reference existing COVE-T/COVE-O sections, segments, morsels, pages, statistics, or page clusters.
- A layout node MUST NOT introduce a new table, column, row order, predicate proof, or logical value.
- If a COVE-L layout node disagrees with authoritative COVE metadata, the node is invalid and MUST be ignored or rejected according to whether the layout profile is optional or required for the requested operation.
- Predicate pruning through COVE-L is valid only when the node references validated COVE proof metadata. The layout node itself is not proof.
- A writer SHOULD use COVE-L to describe large-scale read planning, not to smuggle arbitrary runtime-specific layouts into the file.


### 67.5.1 Layout Unit Taxonomy

COVE-L distinguishes logical, physical, predicate, decode, compression, I/O, and scheduling units. A writer SHOULD NOT force these units to be identical unless doing so is beneficial for the workload and does not weaken validation.

```rust
enum LayoutUnitKindV2 {
    LogicalPage = 0,
    PhysicalPage = 1,
    PredicateStatsUnit = 2,
    DecodeUnit = 3,
    CompressionUnit = 4,
    IoRangeUnit = 5,
    ObjectStoreSplitUnit = 6,
    Morsel = 7,
    PageCluster = 8,
    DimensionalBucket = 9,
    ObjectPathFragment = 10,
    VendorDefined = 255,
}

struct LayoutUnitDescriptorV2 {
    unit_id: u32,
    unit_kind: u16,
    flags: u16,
    table_id: u32,
    column_id: u32,
    segment_id: u32,
    first_morsel_id: u32,
    morsel_count: u32,
    row_start: u64,
    row_count: u64,
    section_id: u32,
    byte_offset: u64,
    byte_length: u64,
    decode_dependency_ref: u32,
    compression_dependency_ref: u32,
    predicate_stats_ref: u32,
    coverage_set_ref: u32,
    preferred_read_size: u32,
    object_store_alignment: u32,
    checksum: u32,
}
```

**Rules:**
- A logical page is the row/value unit described by COVE page reconstruction rules.
- A physical page is the byte range containing one page payload after any page-level compression rules.
- A predicate statistics unit is the granularity at which proof metadata is valid.
- A decode unit is the smallest independently decodable value unit.
- A compression unit is the smallest independently decompressible byte unit.
- An I/O range unit is a suggested byte range for object-store or file-system reads.
- An object-store split unit is a scheduling unit for distributed readers.
- A layout unit MUST reference authoritative COVE metadata and MUST NOT introduce a new schema, row order, logical value, or predicate proof.

### 67.6 Scan Split Index

A scan split is a planner-ready unit of work that may group one or more table segments, morsel ranges, column groups, and page clusters.

```rust
struct ScanSplitIndexHeaderV2 {
    split_count: u32,
    flags: u32,
    checksum: u32,
}

struct ScanSplitEntryV2 {
    split_id: u32,
    table_id: u32,

    row_start: u64,
    row_count: u64,

    first_segment_id: u32,
    segment_count: u32,

    first_morsel_id: u32,
    morsel_count: u32,

    first_cluster_id: u32,
    cluster_count: u32,

    stats_ref: u32,
    estimated_uncompressed_bytes: u64,
    estimated_encoded_bytes: u64,

    flags: u32,
    checksum: u32,
}
```

**Rules:**
- Scan splits are scheduling hints. They MUST NOT change logical row order or returned rows.
- A reader MAY ignore scan splits and derive splits from table segment and morsel metadata.
- A corrupt optional split index MUST be ignored.
- Split estimates are advisory and MUST NOT be used as predicate proof.


### 67.6.1 COVE-R Standards Boundary

COVE-R is an implementation-guidance standard plus a small optional metadata surface. It SHOULD NOT be treated as a prerequisite for ordinary COVE-Core or COVE-T interoperability.

**Rules:**
- A COVE-Core/COVE-T reader MAY ignore all COVE-R metadata and remain conforming.
- Runtime registries, sessions, FFI adapters, language bindings, and engine adapters are not COVE logical data.
- Only explicitly encoded `RuntimeCompatibilityHintV2` and `RuntimeRegistryBindingV2` records are wire artifacts, and those records are advisory unless required by a requested runtime operation.
- A file MUST NOT depend on an unversioned process-global registry to define decode semantics.
- A required registered codec, mapping function, extension type, or engine profile MUST have a portable descriptor and conformance contract in the file, companion artifact, or registry specification; a runtime binding alone is insufficient.

### 67.7 COVE-R Runtime Registry and Session Model

COVE-R is primarily implementation guidance. It describes how readers SHOULD organise extensible runtime state without making that state part of file semantics.

**Recommended implementation model:**

```rust
struct CoveReaderSession {
    codec_registry;
    layout_registry;
    extension_type_registry;
    predicate_kernel_registry;
    synopsis_registry;
    engine_profile_registry;
    mapping_function_registry;
    ffi_adapter_registry;
    memory_and_io_policy;
}
```

**Rules:**
- A reader SHOULD instantiate codecs, kernels, mapping functions, and engine adapters through an explicit session or equivalent context rather than process-global mutable state.
- A COVE file MUST NOT depend on an unversioned global runtime registry to define required semantics.
- Runtime compatibility hints MAY help select adapters, but COVE-Core/COVE-T logical decode MUST remain possible without COVE-R unless a required codec/profile is explicitly needed.
- FFI and language bindings are ecosystem surfaces, not COVE logical data.
- Session caches MAY store decoded dictionaries, FileCode-to-ExecutionCode maps, layout plans, or range-read plans, but caches are not authoritative and MUST be rebuildable.

### 67.8 Runtime Compatibility Hints

```rust
struct RuntimeCompatibilityHintV2 {
    hint_id: u32,
    hint_kind: u16,
    // 0=codec_registry
    // 1=layout_registry
    // 2=predicate_kernel
    // 3=engine_adapter
    // 4=ffi_surface
    // 5=language_binding
    // 6=wasm_or_external_kernel_package

    required: u8,
    flags: u8,

    namespace_len: u16,
    namespace: [u8],

    name_len: u16,
    name: [u8],

    version_major: u16,
    version_minor: u16,

    payload_ref: u32,
    checksum: u32,
}
```

**Rules:**
- Runtime compatibility hints are optional unless the requested operation explicitly requires the hinted runtime surface.
- External kernel packages MUST NOT be required for baseline COVE-Core/COVE-T decode unless a required feature bit and extension contract explicitly say so.
- A runtime hint MUST NOT override a codec, extension, table schema, COVE-MAP function, or engine profile definition.

### 67.9 Non-Normative Vortex Interoperability Boundary

COVE v2 may be implemented using Vortex-inspired or Vortex-backed libraries, adapters, encodings, or benchmarks. Such implementation choices are non-normative.

**Rules:**
- A valid COVE v2 file is not a Vortex file and does not contain a Vortex layout tree as its authoritative data model.
- A COVE reader MAY map COVE pages into Vortex arrays or layouts internally, but that mapping MUST be derived from validated COVE metadata.
- A COVE writer MUST NOT make Vortex dtype/schema identity, runtime layout identifiers, or plugin registry IDs the only way to decode COVE logical values.
- A Vortex-backed adapter MUST preserve COVE table catalog authority, FileCode semantics, null bitmap polarity, predicate-proof safety, COVE-O reconstruction, and COVE-MAP provenance rules.

---
