# Encoded Arrays, Codecs, and Domains

## 20. Encoded Arrays

COVE stores pages as encoded arrays.
**An encoded array has:**
- logical length,
- logical type,
- physical kind,
- encoding tree,
- buffers,
- child arrays,
- statistics reference.

### 20.1 Encoding Kinds

```rust
enum CoveEncodingKind {
    Canonical = 0,

    Validity = 1,
    Constant = 2,

    FileCode = 3,
    NumCode = 4,

    LocalCodebook = 5,
    Rle = 6,
    RunEnd = 7,
    BitPacked = 8,
    Delta = 9,
    FrameOfReference = 10,
    PatchedBase = 11,
    Sparse = 12,
    Sequence = 13,

    PlainFixed = 14,
    PlainVarint = 15,
    VarBytes = 16,

    Lz4Block = 17,
    ZstdBlock = 18,

    RegisteredEncoding = 19,
}
```

### 20.2 Encoding Node Descriptor

```rust
struct CoveEncodingNodeV2 {
    node_id: u16,
    encoding_kind: u16,

    logical_type: u16,
    physical_kind: u8,
    flags: u8,

    logical_len: u32,

    child_count: u16,
    buffer_count: u16,

    params_offset: u32,
    params_length: u32,

    stats_id: u32,
    reserved: u16,       // MUST be 0
}
```

Encoded length: 30 bytes.

**Rules:**
- The root node describes the page payload.
- Child nodes and buffers MUST be bounds-checked.
- Each encoding MUST have a canonical decode path.
- Encoded predicate kernels are optional but SHOULD be implemented for common encodings.

### 20.3 Approved v2 Encoding Cascades

**For FileCode columns:**
Constant(FileCode)
Rle(FileCode)
RunEnd(FileCode)
LocalCodebook(BitPacked(local_index -> FileCode))
LocalCodebook(Rle(local_index -> FileCode))
Sparse(fill FileCode + patches)
PlainVarint(FileCode)
**For NumCode columns:**
Constant(NumCode)
Delta(NumCode)
FrameOfReference(NumCode)
PatchedBase(NumCode)
BitPacked(NumCode delta/range)
PlainFixed(NumCode)
PlainVarint(NumCode)
**For booleans:**
Constant(Boolean)
BitPacked(Boolean)
Rle(Boolean)
**For variable bytes:**
VarBytes
LocalCodebook(VarBytes) only if values are page-local and not globally dictionary encoded
Writers SHOULD prefer FileCode over VarBytes for repeated strings, categories, UUID-like dimensions, and low/medium-cardinality values.

### 20.4 LocalCodebook Payload

```rust
struct LocalCodebookPayloadV2 {
  child_encoding_kind: u16,   // Rle or BitPacked
  value_physical_kind: u16,   // FileCode, NumCode, Boolean, or VarBytes
  codebook_len: u32,
  child_payload_len: u32,
  codebook_values: [u8],
  child_payload: [u8],
}
```

**Codebook value layout:**

| value_physical_kind | Codebook entry wire layout |
| --- | --- |
| FileCode | `u32` little-endian FileCode |
| NumCode | `u64` little-endian raw NumCode bits |
| Boolean | `u8`, where `0=false` and `1=true` |
| VarBytes | `u32 byte_len` followed by `byte_len` bytes |

**Rules:**
- `child_encoding_kind` MUST be `Rle` or `BitPacked` and decodes local indexes.
- Each decoded local index MUST be less than `codebook_len`.
- Boolean codebook entries MUST be either 0 or 1.
- VarBytes codebook entries are page-local values and MUST NOT be interpreted as FileCode dictionary entries.
- Readers MUST reject unsupported `value_physical_kind` values and malformed codebook lengths.

### 20.5 Writer Encoding Selection Policy

Encoding selection is writer policy; it does not change file semantics. Readers observe only the emitted encoding tree, buffers, checksums, and validated metadata.
Writers SHOULD choose encodings per column page, normally one column per morsel. Different morsels of the same column MAY use different approved encoding cascades.
**Recommended analysis pass:**
1. collect page-local facts: row_count, null_count, non_null_count, distinct_count estimate or exact count, run_count, min/max, domain range, sortedness, value width, and candidate encoded sizes;
2. test every approved encoding cascade that is applicable to the page's logical and physical type;
3. assign each candidate a deterministic score representing estimated stored bytes plus optional read-cost penalties for the writer's declared hot/cold policy;
4. choose the lowest-score candidate;
5. emit only the chosen encoding tree and its buffers.
**Rules:**
- A candidate encoder MUST NOT be selected unless it has a canonical decode path available to conforming readers for the chosen profile, or is guarded by a required extension feature bit.
- Writers SHOULD evaluate Constant first. If a page is all-null or all non-null values are equal, Constant or stats-only constant storage SHOULD win unless another representation is proven smaller and equally decodable.
- Writers SHOULD NOT apply a general block codec to an already compact page when the codec increases size or materially harms the declared hot-scan cost class.
- Adaptive selection metadata MAY be recorded in non-authoritative writer metadata for observability, but readers MUST NOT require it for decoding.


### 20.5.1 Page Reconstruction Authority

A page's decoded logical sequence is reconstructed from one of three authority classes:

```rust
enum PageReconstructionSource {
    Payload = 0,          // normal page payload and buffers
    ConstantParams = 1,   // explicit constant parameters in the encoding tree
    StatsConstant = 2,    // validated decode-required stats entry for all-null/all-non-null constant pages
}
```

**Rules:**
- `Payload` is the default.
- `ConstantParams` is canonical page data even when it is small enough to appear in an encoding parameter block.
- `StatsConstant` is allowed only under the strict page-elision rules in Section 27.2.
- When `StatsConstant` is used, the referenced stats entry is no longer optional pushdown metadata. It is decode-required canonical reconstruction data for that page.
- A reader MUST reject a `StatsConstant` page when the stats entry is missing, corrupt, unsafe, truncated, collation-incompatible, or unable to represent the exact logical value.
- Writers SHOULD prefer `ConstantParams` when exact value reconstruction from stats would be ambiguous, especially for floats, decimals, fixed bytes, redacted values, and extension logical types.

### 20.6 Constant and Payload-Elided Storage

Constant encoding is a first-class storage optimisation, not only a predicate-statistics hint.
**Rules:**
- Constant pages MAY omit value buffers when the value can be reconstructed from Constant parameters or, for stats-only all-non-null pages, from a validated page-level ZoneStatsEntry under the rules in 27.2.
- Stats-only constant pages are allowed only for all-null pages or all-non-null pages. Mixed null/non-null constant pages MAY elide the value stream but MUST retain enough null-position information to reconstruct logical row order.
- If the constant value is stored in Constant parameters, the page-level ZoneStatsEntry SHOULD still set IS_CONSTANT and SHOULD use matching min_value and max_value when min/max are valid.
- If the constant value is stored only in stats, the stats entry is decode-required canonical data for that page. It MUST be checksummed, bounds-checked, type-checked, and collation-checked before decoding.
- Readers MUST NOT use raw FileCode min/max as the logical constant for comparable FileCode columns. The constant must be a FileCode equality value or a canonical/domain-ranked value according to the column's declared physical kind and domain rules.

### 20.7 Registered Codec Extension Gate

COVE v2 replaces the v1 specialised-encoding placeholder with a formal COVE-CX codec-extension profile.

**Core rule:** specialised encodings are allowed only when their byte-level wire format, parameters, canonical decode algorithm, feature bits, fallback behaviour, and conformance vectors are defined by COVE v2 itself, by a companion COVE-CX codec specification, or by a registered required extension.

**High-priority candidate v2 codec registrations:**
- `org.coveformat.codec.fsst-utf8.v2` for lossless string/byte encodings where FileCode dictionary encoding is not better.
- `org.coveformat.codec.alp-float.v2` for lossless Float32/Float64 NumCode encodings that preserve exact IEEE bit patterns, including signed zero, infinities, NaN class, and any payload handling declared by the codec specification.
- `org.coveformat.codec.fastlanes-integer.v2` for lossless integer/date/timestamp/decimal NumCode encodings using bit packing, frame-of-reference, delta, patched-base, or related vectorised integer techniques.

These names are **candidate registration identifiers** until companion COVE-CX codec specifications define exact bitstream bytes, parameter schemas, offset bases, block termination rules, fallback equivalence, positive vectors, and negative vectors. A candidate registration MUST NOT be treated as broadly v2-supported merely because the identifier appears in this document.

```rust
enum CodecSpecificationStatusV2 {
    Candidate = 0,             // named here but not yet broad-conformance-ready
    ProvisionalRegistered = 1, // exact spec exists but interoperability evidence is incomplete
    StableRegistered = 2,      // exact spec, vectors, and conformance evidence exist
    Deprecated = 3,
    VendorPrivate = 255,
}
```

**Rules:**
- Writers MUST NOT emit FSST-style, ALP-style, FastLanes-style, Chimp/Patas-style, or similar specialised encodings as core `CoveEncodingKind` values unless the exact byte-level format is registered and gated.
- A `Candidate` or `ProvisionalRegistered` codec MUST NOT be required for broad COVE-Core/COVE-T conformance. It MAY be used only with a validated core fallback payload or inside explicitly experimental/vendor conformance levels.
- A `StableRegistered` encoding needed for decoding projected data MUST either provide a validated canonical fallback payload or set a required feature bit that causes unsupported readers to reject safely.
- Optional registered encodings MAY be used inside COVX or fallback-bearing pages for experimentation, but unsupported readers MUST still recover the same logical values through core COVE encodings.
- Codec names are not enough for interoperability. The registry entry MUST identify the exact codec specification version and conformance vector set.
- Lossy codecs are prohibited for COVE-Core/COVE-T decode unless a required logical extension explicitly defines lossy semantics and every affected column is marked accordingly. COVE v2 core specialised codecs are assumed lossless.

### 20.8 COVE-CX Codec Extension Descriptor

```rust
struct CodecExtensionDescriptorV2 {
    codec_id: u32,

    namespace_len: u16,
    namespace: [u8],

    name_len: u16,
    name: [u8],

    version_major: u16,
    version_minor: u16,

    codec_family: u16,
    // 0=string_symbol_table
    // 1=float_alp_like
    // 2=integer_fastlanes_like
    // 3=bitstream_transform
    // 4=vendor_defined

    logical_type_mask: u64,
    physical_kind_mask: u64,

    requirement: u8,
    // 0=optional_with_fallback
    // 1=required_for_decode
    // 2=sidecar_only

    fallback_policy: u8,
    // 0=no_fallback
    // 1=core_encoding_payload_present
    // 2=dictionary_or_canonical_decode_path
    // 3=external_required_extension

    parameter_schema_kind: u8,
    // 0=none
    // 1=cove_binary_params
    // 2=canonical_cbor
    // 3=json_descriptive_only

    flags: u8,

    specification_status: u8,   // CodecSpecificationStatusV2
    reserved0: [u8; 3],

    required_feature_bit: u64,
    optional_feature_bit: u64,

    spec_digest_algorithm: u16,
    spec_digest_len: u16,
    spec_digest: [u8; spec_digest_len],

    conformance_vector_ref: u32,
    fallback_ref: u32,
    private_payload_ref: u32,

    checksum: u32,
}
```

**Rules:**
- `namespace + name + version` MUST identify one exact codec definition when `specification_status` is `ProvisionalRegistered` or `StableRegistered`.
- `Candidate` descriptors are allowed only for experimental, sidecar-only, or fallback-bearing use and MUST NOT be required for ordinary COVE-Core/COVE-T decode.
- `spec_digest` SHOULD identify the exact codec specification or canonical bitstream definition used by the writer.
- `conformance_vector_ref` SHOULD reference positive and negative codec test vectors.
- `fallback_ref` MUST be valid when `fallback_policy` requires a fallback.
- A codec descriptor MUST declare whether it supports equality kernels, range kernels, selection decode, direct FileCode-to-ExecutionCode remap, or only full decode through `KERNEL_CAPABILITIES` or a codec-specific capability payload.
- A codec descriptor MUST declare any restrictions on null handling, value ordering, NaN handling, signed zero handling, byte ordering, padding bits, and final-block termination.
- A registered codec MUST be deterministic and side-effect-free.

The v2 reference suite includes three stable COVE-owned companion codec definitions:

| Descriptor identity | Companion spec |
| --- | --- |
| `org.coveformat.codec.fsst-utf8.v2` | `docs/codecs/fsst-utf8-v2.md` |
| `org.coveformat.codec.alp-float.v2` | `docs/codecs/alp-float-v2.md` |
| `org.coveformat.codec.fastlanes-integer.v2` | `docs/codecs/fastlanes-integer-v2.md` |

These codecs are inspired by FSST, ALP, and FastLanes families, but they are COVE-owned exact bitstreams and do not claim byte compatibility with external library formats. Broad conformance requires registered-payload/fallback equivalence for decoded logical values and null positions.


### 20.8.1 Registered Encoding Dispatch

Registered codec payloads MUST be reachable through an explicit encoding node. A page whose root value stream is encoded by a registered codec MUST use `CoveEncodingKind::RegisteredEncoding` at the appropriate encoding node and MUST provide a `RegisteredEncodingEnvelopeV2` in that node's parameter payload or in a referenced page buffer.

**Rules:**
- `RegisteredEncoding` is not itself a codec; it is the COVE dispatch envelope for an exact registered codec descriptor.
- A reader MUST validate the codec descriptor and envelope before touching codec-specific bytes.
- A reader MUST NOT dispatch solely on runtime registry names, implementation class names, or vendor strings.
- If the codec is unsupported and a valid fallback payload exists, the reader MAY use the fallback.
- If the codec is unsupported and no valid fallback exists, the reader MUST reject only the operation that needs the encoded page.

### 20.9 Registered Encoding Page Envelope

Registered codecs use a common page-level envelope so readers can reject, fall back, or dispatch safely before touching codec-specific bytes.

```rust
struct RegisteredEncodingEnvelopeV2 {
    codec_id: u32,
    codec_version_major: u16,
    codec_version_minor: u16,

    logical_len: u32,
    non_null_count: u32,

    params_offset: u32,
    params_length: u32,

    encoded_payload_offset: u64,
    encoded_payload_length: u64,

    fallback_payload_offset: u64,  // 0 when absent
    fallback_payload_length: u64,  // 0 when absent

    decoded_uncompressed_length: u64,

    flags: u32,
    checksum: u32,
}
```

**Rules:**
- The envelope is part of the page payload and is covered by the page checksum.
- `logical_len` MUST match the page index row_count and root encoding node logical length.
- If a fallback payload is present, the fallback MUST decode to exactly the same logical sequence and null positions as the registered codec payload.
- If the registered codec is unsupported and a valid fallback payload is present, a reader MAY use the fallback.
- If the registered codec is unsupported and no valid fallback exists, a reader MUST reject any operation that needs the page.
- A reader MUST NOT choose an optional registered payload over a core fallback unless it supports the exact codec version.


### 20.10 Codec Pipeline Classification and Acceleration Neutrality

COVE-CX distinguishes logical encodings, lightweight physical encodings, compression, integrity transforms, and acceleration-only transforms. This prevents a writer from treating a hardware path or runtime plugin as the wire-format definition.

```rust
enum CodecTransformClassV2 {
    LogicalEncoding = 0,
    PhysicalLightweightEncoding = 1,
    BlockCompression = 2,
    ChecksumIntegrityTransform = 3,
    AccelerationOnlyTransform = 4,
    VendorDefined = 255,
}

struct CodecPipelineStageV2 {
    stage_id: u16,
    transform_class: u8,
    codec_id: u32,
    input_physical_kind: u16,
    output_physical_kind: u16,
    independent_decode_unit_rows: u32,
    preferred_block_size_bytes: u32,
    supports_random_access: u8,
    supports_encoded_scan: u8,
    supports_partial_decode: u8,
    supports_selective_decode: u8,
    canonical_decoder_required: u8,
    optional_accelerated_decoder: u8,
    fallback_decoder_ref: u32,
    conformance_vector_ref: u32,
    checksum: u32,
}
```

**Rules:**
- A COVE file MUST define the canonical byte-level decode path independently of any SIMD, GPU, Intel IAA/QPL, ARM extension, FPGA, or other hardware accelerator.
- Implementations MAY use hardware acceleration when it is semantically equivalent to the canonical decoder and all alignment, lifetime, page, null, checksum, and fallback requirements are satisfied.
- A hardware-specific decoder MUST NOT be required for baseline COVE-Core/COVE-T decode unless a non-portable required extension explicitly declares that dependency and unsupported readers reject safely.
- Codec pipeline metadata is advisory unless the registered codec itself is required for projected data. Unsupported advisory pipeline stages MUST be ignored.
- `supports_encoded_scan`, `supports_partial_decode`, and `supports_selective_decode` are capability claims. They MUST NOT be used as predicate-proof metadata.

---


## 21. Kernel Capability Metadata

COVE-T MAY declare encoding kernel capabilities.

```rust
struct EncodingKernelCapabilityV2 {
    encoding_kind: u16,

    supports_eq: u8,
    supports_in: u8,
    supports_range: u8,
    supports_is_null: u8,

    supports_count: u8,
    supports_min_max: u8,
    supports_selection_decode: u8,
    supports_direct_executioncode_remap: u8,

    decode_cost_class: u8,
    predicate_cost_class: u8,

    reserved: [u8; 6],
}
```

**Rules:**
- Kernel capabilities are advisory.
- A reader MAY ignore them.
- A reader MUST NOT trust capability metadata to skip data without validated stats/index proof.
- supports_direct_executioncode_remap means the page can decode FileCodes directly into engine-local ExecutionCode vectors.
**COVE-H refines this to:**
**supports_direct_enginecode_remap:**
  page can decode FileCodes directly into Harbor EngineCode vectors.


### 21.1 V2 Codec and Kernel Capability Binding

COVE-CX codec descriptors and `KERNEL_CAPABILITIES` MAY be linked so engines can decide whether to evaluate predicates on encoded data, decode into canonical values, or materialise engine-local vectors.

```rust
struct CodecKernelCapabilityV2 {
    codec_id: u32,
    encoding_kind: u16,

    supports_eq: u8,
    supports_in: u8,
    supports_range: u8,
    supports_is_null: u8,
    supports_like_or_prefix: u8,
    supports_selection_decode: u8,
    supports_direct_executioncode_remap: u8,
    supports_zero_copy_export: u8,

    decode_cost_class: u8,
    predicate_cost_class: u8,
    random_access_cost_class: u8,
    reserved0: u8,

    min_reader_version_major: u16,
    min_reader_version_minor: u16,

    checksum: u32,
}
```

**Rules:**
- Capability metadata is advisory and MUST NOT be trusted as proof for skipping rows.
- Predicate skipping still requires validated COVE predicate-proof metadata.
- A false capability declaration is a writer/tooling error but MUST NOT change query results; readers MAY ignore capability metadata.
- Zero-copy export capability means only that the codec/page/buffer layout may be exposed without copy when all other nullability, alignment, lifetime, and target format rules also hold.

---


## 22. Collation and Canonicalisation Registry

Collation metadata defines safe ordering semantics.
Range pushdown is allowed only when query collation and stored collation agree.

```rust
struct CollationRegistryHeaderV2 {
    entry_count: u32,
    flags: u32,
}
```

```rust
struct CollationRegistryEntryV2 {
    collation_id: u16,

    name_len: u16,
    name: [u8],

    version_len: u16,
    version: [u8],

    flags: u32,
}
```

**Minimum v2 collations:**

| ID | Name | Meaning |
| --- | --- | --- |
| 0 | none | Unordered; range pushdown unavailable. |
| 1 | utf8-bytewise | Bytewise UTF-8 ordering. |
| 2 | unsigned-fixed-bytes | Unsigned bytewise fixed bytes. |
| 3 | signed-numeric | Signed numeric logical order. |
| 4 | unsigned-numeric | Unsigned numeric logical order. |
| 5 | timestamp-chronological | Timestamp chronological order. |

**Rules:**
- String min/max MUST NOT be used for range exclusion unless collation is known and compatible.
- ColumnDomain sections MUST reference a valid collation_id.
- Test vectors MUST cover UTF-8 edge cases, decimals, timestamps, UUIDs, floats, NaN, and nulls.

---


## 23. Column Domains

A ColumnDomain defines logical ordering for FileCode columns.
Raw FileCode numeric order has no semantic meaning.

```rust
struct ColumnDomainHeaderV2 {
    table_or_object_id: u32,
    column_or_property_id: u32,

    logical_type: u16,
    collation_id: u16,

    domain_count: u32,

    sorted_file_codes_offset: u64,
    file_code_to_rank_offset: u64,

    flags: u32,

    checksum: u32,
}
```

**Payload:**
**sorted_file_codes:**
  FileCode[domain_count]

**file_code_to_rank:**
  u32[dictionary_entry_count] or compressed sparse map
**Rules:**
- sorted_file_codes MUST be sorted by logical value order.
- file_code_to_rank maps FileCode -> domain rank.
- Values absent from the column MAY map to INVALID_RANK.
- Readers MUST validate ranks before using domain min/max.
- If no safe ordering exists, range pushdown MUST be disabled.

---
