# COVX Accelerator Sidecar

## 68. COVX Accelerator Sidecar

COVX is an optional sidecar containing rebuildable acceleration metadata.
**COVX final bytes:**
[postscript bytes]
[postscript_version: u16]
[postscript_len: u16]
[magic: "CVX2"]

### 68.1 COVX Header

```rust
struct CovxHeaderV2 {
    magic: [u8; 4],          // "CVX2"

    header_len: u16,
    version_major: u16,
    version_minor: u16,

    flags: u32,

    accelerator_id: [u8; 16],

    referenced_file_count: u32,

    created_at_us: i64,

    reserved: [u8; 40],

    checksum: u32,
}
```

### 68.2 Referenced File Entry

```rust
struct CovxReferencedFileV2 {
    file_id: [u8; 16],

    file_len: u64,
    footer_crc32c: u32,

    digest_algorithm: u16,
    digest_len: u16,
    digest: [u8; digest_len],
}
```

**COVX may contain:**
- lookup indexes,
- composite zone indexes,
- large histograms,
- full-text indexes,
- vector indexes,
- spatial indexes,
- learned/adaptive indexes,
- workload-specific synopses.
**Rules:**
- COVX MUST be ignored if referenced file digest does not match.
- COVX MUST be ignored if referenced file_id does not match.
- COVX MUST NOT change query semantics.
- COVX acceleration failures MUST fall back to COVE.
- COVX vector, ANN, spatial, learned, or workload-specific indexes MUST have a registered descriptor that declares proof capability, false-negative policy, metric/query class where relevant, and fallback behaviour.
- Approximate or candidate-generating COVX indexes MAY accelerate ranking or candidate selection, but MUST NOT advertise DefinitelyNo, DefinitelyYes, exact Top-N, or metadata-answerable semantics unless the index is exact for the declared query class.


### 68.3 COVX Kernel Capability Vocabulary

COVX may describe optional accelerated kernels. These kernels are implementation dispatch hints, not mandatory execution plans.

```rust
enum CoveKernelKindV2 {
    ScanEncodedEq = 0,
    ScanEncodedRange = 1,
    ScanEncodedInSet = 2,
    ScanEncodedNotNull = 3,
    SelectByBitmap = 4,
    ExtractRowRange = 5,
    ExpandByBitmap = 6,
    DecodeSelected = 7,
    DecompressBlock = 8,
    MaterialiseArrowView = 9,
    MaterialiseArrowOwned = 10,
    BuildCoverageSet = 11,
    IntersectCoverageSets = 12,
    UnionCoverageSets = 13,
    VendorDefined = 255,
}

struct CovxKernelDescriptorV2 {
    kernel_id: u32,
    kernel_kind: u16,
    input_encoding_kind: u16,
    input_codec_id: u32,
    output_form: u16,
    null_semantics: u8,
    comparison_semantics: u8,
    deterministic_equivalence_ref: u32,
    requires_alignment_log2: u8,
    optional_hardware: u8,       // 0=none, 1=simd, 2=gpu, 3=iaa_qpl, 4=arm_extension, 255=vendor
    reserved: u16,
    checksum: u32,
}
```

**Rules:**
- A COVX kernel descriptor MUST declare equivalence to baseline logical semantics or be marked advisory/non-semantic.
- Hardware acceleration is optional. A reader MUST NOT require a specific hardware accelerator to read ordinary COVE-Core/COVE-T values.
- A reader MAY ignore COVX kernels and use baseline decode.
- A kernel descriptor MUST NOT be used as predicate proof. Proof still comes from validated COVE predicate or coverage metadata.

### 68.4 Sidecar Validity

Every COVX sidecar MUST describe exactly which data snapshot, file set, schema, semantic map, and digest root it applies to.

```rust
struct SidecarValidityV2 {
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    file_id: [u8; 16],          // zero UUID when dataset-scoped
    schema_fingerprint_ref: u32,
    semantic_map_fingerprint_ref: u32,
    data_checksum_root_ref: u32,
    external_visibility_ref: u32,
    created_at_us: i64,
    producer_ref: u32,
    flags: u32,
    checksum: u32,
}
```

**Rules:**
- A reader MUST NOT use a sidecar whose declared validity does not match the selected data snapshot and requested operation.
- If `semantic_map_fingerprint_ref` is non-zero, the sidecar is valid only for that mapping/projection version unless a required extension proves compatibility.
- If an external visibility/delete overlay is active, a sidecar that is not overlay-aware may be used for conservative physical-file pruning but MUST NOT provide exact visible-table aggregate answers.
- Sidecar validity applies to COVX, COVE-I, COVM references, COVE-MAP references, runtime mapping artifacts, and coverage artifacts.

---
