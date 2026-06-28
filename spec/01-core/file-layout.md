# File Layout and Sections

## 9. Top-Level COVE File Layout

```text
┌─────────────────────────────────────────────────────────────┐
│ Header                                                      │
├─────────────────────────────────────────────────────────────┤
│ Data and metadata sections                                  │
│   - file dictionary index                                   │
│   - file dictionary payload                                 │
│   - collation registry                                      │
│   - extension registry                                      │
│   - engine profile registry                                 │
│   - table catalog                                           │
│   - table segment index                                     │
│   - table segment data                                      │
│   - object catalog                                          │
│   - temporal segment index                                  │
│   - temporal segment data                                   │
│   - zone statistics                                         │
│   - exact sets                                              │
│   - bloom filters                                           │
│   - inverted indexes                                        │
│   - lookup indexes                                          │
│   - aggregate synopses                                      │
│   - composite zone indexes                                  │
│   - Top-N summaries                                         │
│   - digest manifests                                        │
│   - trust/redaction manifests                               │
├─────────────────────────────────────────────────────────────┤
│ Footer                                                      │
│   - binary section directory                                │
│   - optional descriptive JSON metadata                      │
├─────────────────────────────────────────────────────────────┤
│ Postscript                                                  │
│ Postscript version                                          │
│ Postscript length                                           │
│ Magic "COV2"                                                │
└─────────────────────────────────────────────────────────────┘
```

The postscript is discovered by reading the tail of the file.
The postscript points to the footer.
The footer contains the authoritative binary section directory.

---


## 10. Header

Rust-style declarations in this document are descriptive pseudocode only.
Readers and writers MUST parse and emit fields explicitly.
Readers and writers MUST NOT transmute or memory-map unvalidated bytes into native structs.

COVE v2 uses a new primary magic and a widened header. The widened header gives readers a bootstrap pointer to optional extended feature and metadata-index sections without making those sections mandatory for baseline COVE-Core/COVE-T reads.

```rust
struct CoveHeaderV2 {
    magic: [u8; 4],              // "COV2"

    header_len: u16,             // 160 for v2
    version_major: u16,          // 2
    version_minor: u16,          // 0 for v2.0

    primary_profile: u8,
    // 0=mixed/unknown
    // 1=COVE-O object temporal
    // 2=COVE-T table scan
    // 3=COVE-A archive acceleration
    // 4=COVE-E engine execution
    // 5=COVE-H Harbor registered execution profile
    // 6=COVE-MAP evidence/projection carrier inside a .cove file
    // 7=COVE-CX codec extension carrier
    // 8=COVE-L layout/split planning carrier
    // 9=COVE-R runtime compatibility carrier
    // 10=COVE-COVERAGE coverage metadata carrier
    // 11=COVE-I secondary index carrier

    endianness: u8,              // 1=little-endian

    flags: u32,

    required_features: u64,      // low feature word
    optional_features: u64,      // low feature word

    file_id: [u8; 16],

    producer_scope_id: [u8; 16],
    producer_scope_kind: u16,

    reserved_scope_flags: u16,

    created_at_us: i64,

    feature_set_section_id: u32,        // 0 if no EXTENDED_FEATURE_SET section
    profile_capability_section_id: u32, // 0 if no PROFILE_CAPABILITY_MATRIX section
    fast_metadata_section_id: u32,      // 0 if no FAST_METADATA_INDEX section
    v2_flags: u32,

    reserved: [u8; 64],          // MUST be zero

    checksum: u32,               // CRC32C of header with this field zeroed
}
```

**Scope kind:**

```rust
enum ProducerScopeKind {
    None = 0,
    Tenant = 1,
    Account = 2,
    Organisation = 3,
    Workspace = 4,
    Catalog = 5,
    Dataset = 6,
    EngineSpecific = 255,
}
```

**Header rules:**
- magic MUST be `"COV2"` for COVE v2 files.
- Any primary-file magic other than `"COV2"` is invalid for this standard.
- header_len MUST be 160 for v2.0.
- version_major MUST be 2.
- endianness MUST be 1.
- reserved bytes MUST be zero.
- checksum MUST validate before any other header field is trusted.
- Header `required_features` are **always file-required**. An unknown bit in the header low-word `required_features` MUST cause rejection during bootstrap. There is no scoped-requiredness escape hatch for header-required bits.
- Operation-, profile-, section-, and page-scoped requiredness MUST be expressed through section entries, page flags/envelopes, profile descriptors, `PROFILE_CAPABILITY_MATRIX`, or `SECTION_FEATURE_BINDING`, not by placing unknown or operation-only bits in the header `required_features`.
- Writers SHOULD place optional profile-presence bits such as COVE-MAP, COVE-H, COVE-L, COVE-R, COVX, COVE-I, and COVE-CACHE references in `optional_features` unless ordinary baseline file parsing or selected logical decode truly requires them.
- Writers MUST NOT place operation-only requirements, such as mapping replay, trust-chain verification, Harbor mount, projection readback, runtime adapter selection, index-only answering, or zero-copy export, in header `required_features`. Doing so makes the whole file unreadable to readers that do not know the bit.
- if `feature_set_section_id != 0`, the referenced `EXTENDED_FEATURE_SET` section MUST be validated before any extended required feature is acted on.
- `feature_set_section_id`, `profile_capability_section_id`, and `fast_metadata_section_id` are bootstrap **section identifiers**, not byte offsets and not replacements for the footer section directory. A reader still discovers authoritative section offsets through the postscript and footer. If a referenced optional section is absent, corrupt, or unsupported, a reader MUST fall back to ordinary footer/section parsing unless the section is marked required for the requested operation.
- Header fields MUST NOT be used to override footer section directory entries or profile-specific catalog metadata.

---


## 11. Feature Bits

**Feature bits are divided into:**
**required_features:**
  reader must understand these to correctly read required data

**optional_features:**
  reader may ignore these if the associated section is not needed
**Assigned v2 feature bits:**

| Bit | Name | Meaning |
| --- | --- | --- |
| 0x0000_0000_0000_0001 | FEATURE_OBJECT_PROFILE | File contains COVE-O sections. |
| 0x0000_0000_0000_0002 | FEATURE_TABLE_PROFILE | File contains COVE-T sections. |
| 0x0000_0000_0000_0004 | FEATURE_ARCHIVE_PROFILE | File contains COVE-A sections. |
| 0x0000_0000_0000_0008 | FEATURE_ENGINE_PROFILE | File contains COVE-E sections. |
| 0x0000_0000_0000_0010 | FEATURE_HARBOR_PROFILE | File contains COVE-H Harbor-specific metadata. |
| 0x0000_0000_0000_0020 | FEATURE_FILE_DICTIONARY | File uses FileCode dictionary. |
| 0x0000_0000_0000_0040 | FEATURE_NUMCODES | File contains NumCode columns. |
| 0x0000_0000_0000_0080 | FEATURE_COLUMN_DOMAINS | File contains ColumnDomain sections. |
| 0x0000_0000_0000_0100 | FEATURE_EXACT_SETS | File contains exact set indexes. |
| 0x0000_0000_0000_0200 | FEATURE_BLOOM_FILTERS | File contains bloom indexes. |
| 0x0000_0000_0000_0400 | FEATURE_INVERTED_INDEXES | File contains inverted morsel indexes. |
| 0x0000_0000_0000_0800 | FEATURE_LOOKUP_INDEXES | File contains point lookup indexes. |
| 0x0000_0000_0000_1000 | FEATURE_AGGREGATE_SYNOPSES | File contains aggregate synopsis sections. |
| 0x0000_0000_0000_2000 | FEATURE_COMPOSITE_ZONES | File contains composite zone indexes. |
| 0x0000_0000_0000_4000 | FEATURE_TOPN_SUMMARIES | File contains Top-N zone summaries. |
| 0x0000_0000_0000_8000 | FEATURE_TRUST_CHAIN | File contains trust-chain data. |
| 0x0000_0000_0001_0000 | FEATURE_REDACTIONS | File contains redacted values/audit references. |
| 0x0000_0000_0002_0000 | FEATURE_NESTED_COLUMNS | File contains list/struct/map columns. |
| 0x0000_0000_0004_0000 | FEATURE_DIGEST_MANIFEST | File contains cryptographic digest manifest. |
| 0x0000_0000_0008_0000 | FEATURE_ARROW_INTEROP_HINTS | File contains Arrow mapping hints. |
| 0x0000_0000_0010_0000 | FEATURE_LAKEHOUSE_HINTS | File contains lakehouse integration hints. |
| 0x0000_0000_0020_0000 | FEATURE_EXTENSION_REGISTRY | File contains extension registry. |
| 0x0000_0000_0040_0000 | FEATURE_CODEC_LZ4 | File uses LZ4-compressed payloads. |
| 0x0000_0000_0080_0000 | FEATURE_CODEC_ZSTD | File uses Zstd-compressed payloads. |
| 0x0000_0000_0100_0000 | FEATURE_SEMANTIC_MAP | File or companion artifact contains COVE-MAP mapping, mapping evidence, identity-equivalence, source-conversion metadata, or object-association projection definitions. |
| 0x0000_0000_0200_0000 | FEATURE_PAGE_PAYLOAD_ELISION | File may contain stats-only constant pages or value-stream-elided pages whose reconstruction depends on page flags and validated page-level stats. |
| 0x0000_0000_0400_0000 | FEATURE_CODEC_EXTENSION_REGISTRY | File contains COVE-CX codec extension descriptors. |
| 0x0000_0000_0800_0000 | FEATURE_REGISTERED_ENCODINGS | File contains pages encoded with registered COVE-CX encodings. Required when any projected page cannot be decoded through core encodings alone. |
| 0x0000_0000_1000_0000 | FEATURE_LAYOUT_PLAN | File contains COVE-L layout-plan metadata. |
| 0x0000_0000_2000_0000 | FEATURE_SCAN_SPLIT_INDEX | File contains precomputed scan split metadata. |
| 0x0000_0000_4000_0000 | FEATURE_PAGE_CLUSTER_DIRECTORY | File contains page cluster metadata for range-read coalescing. |
| 0x0000_0000_8000_0000 | FEATURE_ZERO_COPY_BUFFER_MAP | File contains optional zero-copy/export buffer compatibility metadata. |
| 0x0000_0001_0000_0000 | FEATURE_FAST_METADATA_INDEX | File contains optional wide-schema/random-access metadata index. |
| 0x0000_0002_0000_0000 | FEATURE_RUNTIME_COMPATIBILITY_HINTS | File contains COVE-R runtime compatibility hints. |
| 0x0000_0004_0000_0000 | FEATURE_EXTENDED_FEATURE_SET | File contains feature words beyond the low 64-bit header fields. |
| 0x0000_0008_0000_0000 | FEATURE_CODEC_FALLBACK_PAYLOADS | File contains explicit fallback payloads for optional registered encodings. |
| 0x0000_0010_0000_0000 | FEATURE_COVERAGE_METADATA | File or companion artifact contains COVE-COVERAGE coverage sets, proofs, provider descriptors, or coverage plan candidates. |
| 0x0000_0020_0000_0000 | FEATURE_COVERAGE_PLAN_CANDIDATES | File or companion artifact contains costed coverage plan candidates and do-no-harm fallback metadata. |
| 0x0000_0040_0000_0000 | FEATURE_SECONDARY_INDEX_ARTIFACT | Dataset or file references a COVE-I `.covi` secondary index artifact. |
| 0x0000_0080_0000_0000 | FEATURE_INDEX_ONLY_CAPABILITY | File or companion artifact declares exact or approximate index-only query-answer capabilities. |
| 0x0000_0100_0000_0000 | FEATURE_COVERAGE_CACHE_HINTS | File or manifest may reference a runtime/local COVE-CACHE compatibility or invalidation surface. |

**Rules:**
- Readers MUST reject unknown header `required_features` bits unconditionally during bootstrap.
- Readers MUST reject unknown section-, page-, profile-, or operation-required feature bits only when the requested operation needs the section, page, profile, or operation carrying those bits.
- Readers MAY ignore unknown optional feature bits.
- FEATURE_SEMANTIC_MAP indicates the presence of COVE-MAP-related metadata. Whether that metadata is required depends on the requested operation and any required embedded profile or extension rules. Ordinary COVE-T or COVE-O reads MAY ignore optional mapping evidence, identity-equivalence, or projection metadata when mapping replay, explanation, conversion, or projection readback is not requested.
- Readers MUST NOT use unknown optional metadata for skipping.
- COVE-CX registered encodings that are needed to decode projected data MUST be represented by required feature bits unless an independently validated canonical fallback payload is present and selected.
- COVE-L layout, scan-split, page-cluster, zero-copy, and runtime compatibility metadata MUST be optional unless the requested operation explicitly asks for that metadata.
- COVE-COVERAGE metadata MUST be validated before use. Unknown optional coverage metadata MUST be ignored and MUST NOT be used for pruning.
- COVE-I secondary index artifacts and COVE-CACHE coverage caches MUST be optional and snapshot-bound. Unsupported, stale, or corrupt index/cache metadata MUST be ignored for ordinary reads.
- If `FEATURE_EXTENDED_FEATURE_SET` is set, readers MUST validate the extended feature set before accepting or rejecting unknown extended required features.

### 11.1 COVE-O Delta Artifact Feature Bits

COVE-O `.covedelta` artifacts use a local 64-bit delta feature namespace. These bits appear in `CoveDeltaPostscriptV1`, `CoveDeltaHeaderV1`, `CoveDeltaSectionDirectoryEntryV1`, `CovmDeltaChainExtensionV1`, and chain-summary entries. They are not global COVE header feature bits and MUST NOT be placed in `.cove` `required_features` or `optional_features`.

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `DELTA_FEATURE_SPARSE_PATCH_ROWS` | Delta temporal rows use sparse property operations. |
| 1 | `DELTA_FEATURE_OBJECT_TOMBSTONES` | Object tombstone records may appear. |
| 2 | `DELTA_FEATURE_PROPERTY_TOMBSTONES` | Property tombstone operations may appear. |
| 3 | `DELTA_FEATURE_ASSOCIATION_TOMBSTONES` | Association or link tombstone records may appear. |
| 4 | `DELTA_FEATURE_CONTINUATION_ANCHORS` | Existing-object patches require logical continuation anchors. |
| 5 | `DELTA_FEATURE_INLINE_DICTIONARY` | Delta-local dictionary values are inline. |
| 6 | `DELTA_FEATURE_PARENT_DICTIONARY_ALIASES` | Delta-local dictionary entries may alias validated parent dictionaries. |
| 7 | `DELTA_FEATURE_EXACT_TOUCHED_SET` | Touched-object summaries are exact and may be required for skipping. |
| 8 | `DELTA_FEATURE_EXACT_TOMBSTONE_SET` | Tombstone summaries are exact and required for latest-state reads. |
| 9 | `DELTA_FEATURE_CHECKPOINT_BASELINES` | Delta may carry checkpoint Baseline or Snapshot records. |
| 10 | `DELTA_FEATURE_COVERAGE_PATCH` | Delta carries COVE-COVERAGE patch sections. |
| 11 | `DELTA_FEATURE_INDEX_HINTS` | Delta references COVE-I or COVX index artifacts. |
| 12 | `DELTA_FEATURE_MAP_EVIDENCE_PATCH` | Delta carries COVE-MAP evidence metadata. |
| 13 | `DELTA_FEATURE_PROJECTION_PATCH` | Delta carries projection metadata or invalidation summaries. |
| 14 | `DELTA_FEATURE_HISTORICAL_COMMIT_INSERT` | Delta may insert historical commit-order records. Deferred for the first profile. |

**Rules:**
- The first interoperable COVE-O delta profile requires bits 0, 1, 4, 5, 7, and 8 for ordinary sparse object-temporal reconstruction.
- A reader that does not support a required delta feature bit MUST reject the selected snapshot or requested operation that needs the feature.
- Unknown optional delta feature bits MAY be ignored, but readers MUST NOT use unknown optional metadata for pruning, reconstruction, trust validation, projection readback, or evidence replay.
- Required feature bits in temporal segment semantics, sparse patch semantics, anchors, tombstones, exact touched sets, exact tombstone sets, or required chain summaries reject COVE-O object-temporal reads for the selected delta-bearing snapshot when unsupported.
- Required feature bits in optional index, layout, coverage, projection, or evidence sections reject only operations that select those sections.
- `DELTA_FEATURE_HISTORICAL_COMMIT_INSERT` MUST NOT be required by the first profile. Historical commit-order insertion requires a later required extension.


### 11.2 Extended Feature Set

The low 64-bit header feature words cover bootstrap features. COVE v2 also allows an `EXTENDED_FEATURE_SET` section for future feature banks.

Feature words are globally numbered. Global feature word 0 is the low 64-bit header/postscript word. Global feature word `N` contains feature bits `64*N` through `64*N + 63`. This global numbering is used by the `EXTENDED_FEATURE_SET`, `SECTION_FEATURE_BINDING`, profile capability matrices, and companion artifacts. Local arrays may store only the words needed by a binding, but every local word range MUST declare the global word number it represents.

```rust
struct ExtendedFeatureSetHeaderV2 {
    word_count: u32,
    required_word_count: u32,
    optional_word_count: u32,
    flags: u32,
    checksum: u32,
}
// followed by:
//   required_feature_words: u64[required_word_count]
//   optional_feature_words: u64[optional_word_count]
```

**Rules:**
- Feature word 0 MUST equal the low feature words in the header and postscript.
- `required_feature_words[i]` and `optional_feature_words[i]` represent global feature word `i`; missing words beyond the declared counts are interpreted as zero.
- `word_count` is the declared logical feature-word horizon for this artifact: valid global feature-word indexes are `0` through `word_count - 1`. It MUST be greater than or equal to both `required_word_count` and `optional_word_count`. If `word_count` is greater than either array count, the missing trailing words for that array are zero. Writers SHOULD set `word_count` to the smallest value that covers every non-zero required or optional feature word and every globally numbered feature word referenced by a section binding, profile capability matrix, or companion artifact reference.
- Readers MUST reject an extended feature set when `word_count == 0`, when a non-zero bit appears outside the declared horizon, or when a `SECTION_FEATURE_BINDING` references a global feature word greater than or equal to `word_count`.
- Unknown required bits in global feature word 0 are header-required and MUST reject unconditionally during bootstrap.
- Unknown required bits in global feature words greater than 0 MUST cause rejection according to their declared scope. If no narrower binary binding exists, the default scope is `FileRequired`.
- Unknown optional bits MAY be ignored.
- A later `SECTION_FEATURE_BINDING` MAY scope extended feature words greater than 0, but it MUST NOT reinterpret, narrow, or defer unknown header-required bits in global word 0.
- The extended feature set MUST NOT be represented only in JSON metadata.
- A writer SHOULD use the low feature word for commonly required bootstrap features and extended words for profile-specific, vendor, or future features.

### 11.3 Feature Scope and Requiredness Model

COVE v2 distinguishes the scope of requiredness. This avoids the failure mode where an ordinary table reader rejects a valid table scan because the file also contains required metadata for a different operation such as mapping replay, trust verification, Harbor mount, index-only answering, zero-copy export, or projection readback.

```rust
enum FeatureScope {
    FileRequired = 0,      // required for bootstrap, baseline parse, and selected logical decode
    SectionRequired = 1,   // required only when the section is used
    PageRequired = 2,      // required only when the page is projected, filtered, or reconstructed
    ProfileRequired = 3,   // required only when the profile is claimed or requested
    OperationRequired = 4, // required only for a named operation such as mapping replay or trust verification
    AdvisoryOnly = 5,      // never required for correctness; unsupported readers ignore
}
```

**Default binding of feature words:**

| Location | Default scope | Meaning |
| --- | --- | --- |
| Header `required_features` word 0 | `FileRequired` | Required for bootstrap or ordinary logical decode of the file. Unknown bits always reject during bootstrap and cannot be narrowed by later bindings. |
| Header `optional_features` word 0 | `AdvisoryOnly` or scoped by section/profile | Presence or optional capability advertisement. Unknown bits are ignored. |
| `EXTENDED_FEATURE_SET.required_feature_words` without narrower binding | `FileRequired` | Required for the file or artifact as a whole. Unknown bits reject before use. |
| `CoveSectionEntryV2.required_features` | `SectionRequired` | Required only to use that section. Unknown bits reject use of that section, not unrelated operations. |
| `ColumnPageIndexEntryV2` page flags / registered codec envelope | `PageRequired` when decode-affecting | Required only to decode, predicate-evaluate, reconstruct, or validate that page. |
| `PROFILE_CAPABILITY_MATRIX` or profile descriptor | `ProfileRequired` | Required only when the profile is explicitly requested or claimed. |
| `SectionFeatureBindingV2.scope = OperationRequired` | `OperationRequired` | Required only for the named operation or capability referenced by the binding payload/profile matrix. |

**Precedence rules:**
1. Header `required_features` have the highest bootstrap precedence. If a writer puts an unknown bit there, a generic reader MUST reject. Header-required bits cannot be narrowed by `PROFILE_CAPABILITY_MATRIX`, `SectionFeatureBindingV2`, section entries, or page envelopes.
2. Section and page requiredness cannot make an otherwise undecodable file safe to parse; it can only scope rejection for optional sections/pages that are not needed by the selected operation.
3. `SectionFeatureBindingV2` can narrow or extend section-level requiredness for extended feature banks, but it cannot override, reinterpret, or defer any header `required_features` bit.
4. Profile and operation requiredness applies only after the reader has selected an operation, output mode, requested profile, or advertised conformance claim.
5. Advisory features MUST NOT cause rejection and MUST NOT be used for pruning, index-only answers, or decode unless independently validated by a supported proof or codec contract.

**Rules:**
- A `FileRequired` unknown feature MUST cause rejection before logical decode.
- A `SectionRequired` unknown feature MUST cause rejection only when the reader needs that section.
- A `PageRequired` unknown feature MUST cause rejection only when the reader needs that page for projection, predicate evaluation, reconstruction, or validation.
- A `ProfileRequired` unknown feature MUST cause rejection only when the reader claims or requests that profile.
- An `OperationRequired` unknown feature MUST cause rejection only for the operation that requires it.
- Ordinary COVE-T reads MUST NOT fail solely because optional COVE-MAP, COVE-H, COVE-L, COVE-R, COVX, COVE-I, COVM, or COVE-CACHE metadata is unsupported, stale, corrupt, or missing.
- If a registered codec is required to decode a projected page and no valid fallback exists, that codec is `PageRequired` and the page operation MUST reject when unsupported.
- If COVE-MAP metadata is required only for replay, conversion, explanation, or projection readback, ordinary COVE-T/COVE-O decoding MUST remain possible without it.
- If a trust-chain, redaction, digest, COVE-I index-only answer, COVX kernel, COVE-L zero-copy map, COVE-R runtime adapter, or COVE-CACHE entry is requested by operation or policy, unsupported required features reject that operation only.
- A writer that wants a file to be broadly readable SHOULD advertise optional profiles in header `optional_features`, then express their requiredness in section entries, profile matrices, or operation-specific bindings.

### 11.3.1 Requiredness Validation Order

A conforming reader SHOULD evaluate requiredness in this order:

1. Validate header magic, length, version, endianness, reserved bytes, and header checksum.
2. Reject unknown header `required_features` bits unconditionally.
3. Discover and validate the postscript and footer section directory.
4. Validate `EXTENDED_FEATURE_SET` if advertised or referenced.
5. Build the feature-scope table from header words, footer section entries, `PROFILE_CAPABILITY_MATRIX`, and `SectionFeatureBindingV2` records.
6. Select the requested operation: ordinary table scan, object reconstruction, mapping replay, projection readback, index-only answer, trust verification, Harbor mount, Arrow zero-copy export, etc.
7. Reject only the unknown required features whose scope intersects the selected operation.
8. Ignore unsupported advisory features and unsupported optional sections.

A reader MAY implement a stricter policy for safety, but such a policy MUST be reported as an implementation policy rather than a COVE wire-format requirement.

### 11.3.2 Profile Capability Matrix

`PROFILE_CAPABILITY_MATRIX` is a shared v2 section that scopes required and optional feature words to a profile, operation, section, or target-local reference. It is used when a file advertises optional profiles whose requiredness should not affect ordinary reads.

```rust
struct ProfileCapabilityMatrixHeaderV2 {
    magic: [u8; 4],              // "PCM2"
    version_major: u16,          // 2
    header_len: u16,
    entry_len: u16,
    reserved: u16,               // 0
    entry_count: u32,
    flags: u32,
    entries_offset: u64,
    entries_length: u64,
    checksum: u32,
}

struct ProfileCapabilityEntryV2 {
    profile: u8,
    scope: u8,                   // FeatureScope
    operation_kind: u16,          // OperationKindV2 or None
    global_feature_word_index: u32,
    required_mask: u64,
    optional_mask: u64,
    section_id: u32,              // 0 when not section-scoped
    target_local_ref: u64,        // u64::MAX when absent
    flags: u32,
    reserved: u32,                // 0
    checksum: u32,
}
```

Entries MUST be sorted by `(profile, scope, operation_kind, global_feature_word_index, section_id, target_local_ref)` and duplicate keys are invalid. `operation_kind` MUST be `None` unless `scope = OperationRequired`. `global_feature_word_index` MUST be less than the `EXTENDED_FEATURE_SET.word_count` when an extended set is present.

### 11.4 Section-Level Extended Feature Binding

The low 64-bit feature words in `CoveSectionEntryV2` are sufficient for common bootstrap and section features. When section-, profile-, page-, or operation-scoped requiredness uses extended feature words, a `SECTION_FEATURE_BINDING` section provides the binary-authoritative binding. The binding section is not a way to make an unknown header-required feature safe; it applies only after header validation has succeeded.

A `SECTION_FEATURE_BINDING` payload has one header, one binding array, an optional local payload-reference array, and a feature-word data area. All offsets in this subsection are byte offsets relative to the start of the `SECTION_FEATURE_BINDING` section payload unless explicitly stated otherwise.

```rust
struct SectionFeatureBindingSectionHeaderV2 {
    magic: [u8; 4],              // "SFB2"
    version_major: u16,          // 2
    version_minor: u16,          // 0
    header_len: u16,
    entry_len: u16,

    binding_count: u32,
    payload_ref_count: u32,
    feature_word_count: u32,

    bindings_offset: u64,
    payload_refs_offset: u64,    // 0 when payload_ref_count == 0
    feature_words_offset: u64,   // 0 when feature_word_count == 0
    payload_data_offset: u64,    // 0 when there is no local payload data
    payload_data_length: u64,

    flags: u32,
    checksum: u32,
}
```

```rust
enum SectionFeatureBindingPayloadKindV2 {
    None = 0,
    ProfileRequirement = 1,
    OperationRequirement = 2,
    PageRequirement = 3,
    ExtensionRequirement = 4,
    CodecRequirement = 5,
    CoverageRequirement = 6,
    IndexRequirement = 7,
    RuntimeRequirement = 8,
    VendorDefined = 255,
}

struct SectionFeatureBindingPayloadRefV2 {
    binding_payload_ref: u32,     // dense 1..payload_ref_count; 0 is absent
    payload_kind: u16,            // SectionFeatureBindingPayloadKindV2
    operation_kind: u16,          // OperationKindV2 or None
    profile: u8,                  // section/profile id using CoveSectionEntryV2.profile values
    flags: u8,
    reserved: u16,
    payload_offset: u64,          // into payload_data area
    payload_length: u64,
    checksum: u32,
}
```

```rust
struct SectionFeatureBindingV2 {
    binding_id: u32,              // dense 0..binding_count-1
    section_id: u32,              // 0 when binding applies to a profile/artifact rather than one section
    scope: u8,                    // FeatureScope
    profile: u8,                  // 0=shared or CoveSectionEntryV2.profile value
    operation_kind: u16,           // OperationKindV2; must be None unless scope=OperationRequired

    required_word_count: u32,
    optional_word_count: u32,
    required_feature_word_index: u32, // index into local feature-word array, or u32::MAX
    optional_feature_word_index: u32, // index into local feature-word array, or u32::MAX
    required_first_feature_word_number: u32, // global feature word number, or u32::MAX
    optional_first_feature_word_number: u32, // global feature word number, or u32::MAX

    binding_payload_ref: u32,      // 0 when absent; local SectionFeatureBindingPayloadRefV2 reference
    target_local_ref: u64,         // page_id, profile_id, codec_id, index_root_id, etc.; u64::MAX when not applicable
    flags: u32,
    checksum: u32,
}
```

```rust
enum OperationKindV2 {
    None = 0,
    OrdinaryTableScan = 1,
    ObjectReconstruction = 2,
    MappingReplay = 3,
    MappingExplanation = 4,
    ProjectionReadback = 5,
    TrustVerification = 6,
    RedactionPolicyEvaluation = 7,
    HarborMount = 8,
    EngineExecutionMapping = 9,
    IndexOnlyAnswer = 10,
    CoveragePlanning = 11,
    ZeroCopyExport = 12,
    RuntimeAdapterSelection = 13,
    VendorDefined = 255,
}
```

**Reference spaces:**
- `binding_id` is local to one `SECTION_FEATURE_BINDING` section and MUST be dense.
- `binding_payload_ref` is local to the `payload_refs` array of the same `SECTION_FEATURE_BINDING` section. `0` means absent. Non-zero values MUST be in `1..payload_ref_count` and MUST identify exactly one `SectionFeatureBindingPayloadRefV2`.
- `required_feature_word_index` and `optional_feature_word_index` are indexes into the local `u64[feature_word_count]` array beginning at `feature_words_offset`. The binding uses the contiguous local ranges `[index, index + word_count)`. `u32::MAX` is valid only when the corresponding word count is zero.
- `required_first_feature_word_number` and `optional_first_feature_word_number` identify the global feature-word number represented by the first word in the corresponding local range. The local word at `feature_word_index + i` represents global feature word `first_feature_word_number + i`. `u32::MAX` is valid only when the corresponding word count is zero.
- A `SECTION_FEATURE_BINDING` MUST NOT bind global feature word 0. Low-word section scoping is expressed by `CoveSectionEntryV2.required_features`, `CoveSectionEntryV2.optional_features`, page flags, codec envelopes, or other low-word fields. Unknown header-required bits in global word 0 always reject before bindings are interpreted.
- If multiple bindings for the same target and scope mention the same global feature-word number, the effective word is the bitwise OR of those validated bindings. Bindings MUST NOT rely on local array position as semantic feature-bank identity.
- `section_id` references a `CoveSectionEntryV2.section_id` in the same `.cove` artifact. `section_id = 0` is allowed only for profile-, artifact-, or operation-wide bindings where the payload reference identifies the target.
- `target_local_ref` is interpreted only by the `payload_kind` and `scope`. For example, it may be a page reference, codec ID, index root ID, coverage provider ID, profile ID, or runtime hint ID. If the required interpretation is unknown, the binding MUST be treated as unsupported for that scoped operation.
- For reference-code COVE-T/COVE-O column pages whose page index entry does not carry an explicit `page_id`, `target_local_ref` is the deterministic synthetic page reference `(column_id as u64) << 32 | morsel_id as u64`. Page indexes that carry explicit `page_id` values use that `page_id` instead.

**Rules:**
- Section-level extended feature bindings MUST be checksummed and bounds-checked before use.
- `magic` MUST be `"SFB2"`; unsupported major versions make the binding section unsupported.
- The binding array, payload-ref array, feature-word array, and payload-data area MUST be non-overlapping and within the section payload.
- A section with unsupported required extended bits MUST be rejected only at the scope declared by the binding.
- `operation_kind` MUST be `None` unless `scope == OperationRequired`.
- If `required_word_count > 0`, then `required_feature_word_index` and `required_first_feature_word_number` MUST NOT be `u32::MAX`, the local word range MUST be in bounds, and `required_first_feature_word_number` MUST be greater than 0.
- If `optional_word_count > 0`, then `optional_feature_word_index` and `optional_first_feature_word_number` MUST NOT be `u32::MAX`, the local word range MUST be in bounds, and `optional_first_feature_word_number` MUST be greater than 0.
- If a word count is zero, the corresponding local index and global first-word number MUST both be `u32::MAX`.
- `binding_payload_ref`, when non-zero, MUST reference a validated binary payload that defines the operation, profile, page, codec, index, runtime adapter, or extension contract. JSON metadata MUST NOT define requiredness.
- A writer SHOULD use this binding only when low-word section features are insufficient or when extended features need operation-, profile-, page-, or artifact-specific scope.
- `SectionFeatureBindingV2` MUST NOT narrow, override, reinterpret, or defer any unknown bit in header `required_features`.
- The extended feature set MUST remain binary-authoritative; JSON metadata MUST NOT define requiredness.

---


## 12. Postscript

**The final bytes of every COVE file are:**
[postscript bytes]
[postscript_version: u16]
[postscript_len: u16]
[magic: "COV2"]
**Rules:**
- postscript_len excludes postscript_version, postscript_len, and trailing magic.
- postscript_len MUST be <= 65535.
- Readers SHOULD be able to discover the footer by reading the final 64 KiB.

```rust
struct CovePostscriptV2 {
    required_features: u64,
    optional_features: u64,

    file_len: u64,

    footer: CoveSectionSpecV2,

    checksum: u32,
}
```

```rust
struct CoveSectionSpecV2 {
    offset: u64,
    length: u64,
    uncompressed_length: u64,

    compression: u8,        // 0=None, 1=LZ4, 2=Zstd
    encryption: u8,         // 0=None in v2
    alignment_log2: u8,
    flags: u8,

    crc32c: u32,
    reserved: u32,          // MUST be zero
}
```

**Postscript validation:**
- file_len MUST equal actual file length.
- footer offset/length MUST be within file_len.
- footer CRC32C MUST validate before footer contents are trusted.
- encryption MUST be 0 in v2.

---


## 13. Footer and Section Directory

The footer contains the authoritative section directory.

```rust
struct CoveFooterHeaderV2 {
    footer_magic: [u8; 4],       // "CV2F"

    footer_version: u16,         // 2
    header_len: u16,

    section_count: u32,
    section_entry_len: u16,
    flags: u16,

    metadata_len: u32,           // <= 1 MiB

    reserved: [u8; 24],          // MUST be zero
}
```

// followed by:
//   CoveSectionEntryV2[section_count]
//   metadata_json[metadata_len]

```rust
struct CoveSectionEntryV2 {
    section_id: u32,
    section_kind: u16,

    profile: u8,
    // 0=shared
    // 1=COVE-O
    // 2=COVE-T
    // 3=COVE-A
    // 4=COVE-E
    // 5=COVE-H
    // 6=COVE-MAP
    // 7=COVE-CX
    // 8=COVE-L
    // 9=COVE-R
    // 10=COVE-COVERAGE
    // 11=COVE-I

    flags: u8,

    offset: u64,
    length: u64,
    uncompressed_length: u64,

    item_count: u64,
    row_count: u64,

    compression: u8,
    encryption: u8,
    alignment_log2: u8,
    reserved0: u8,

    required_features: u64,
    optional_features: u64,

    crc32c: u32,
    reserved1: u32,
}
```

**Rules:**
- The binary section directory is authoritative.
- Section offsets and lengths MUST be bounds-checked.
- Every used section MUST have its CRC validated before use.
- Section ranges MUST NOT overlap unless explicitly permitted by section kind.
- Arithmetic overflow MUST be checked.
- JSON metadata MUST NOT override binary metadata.
**Directory granularity and lazy metadata:**
- The footer section directory SHOULD remain coarse-grained. Writers SHOULD NOT create one section entry per page or per morsel when a table segment index, column directory, or page index can describe the same data.
- Detailed table, segment, column, page, and morsel metadata SHOULD be stored in ordered arrays inside their profile sections.
- Readers MAY load the footer and top-level section directory eagerly, then lazily materialise table segment, column, and page metadata only for referenced tables, projected columns, and candidate morsels.
- Segment, morsel, and page lookup arrays SHOULD be ordered by table_id, segment_id, column_id, and morsel_id as applicable, so readers can use binary search without tree-shaped metadata structures.
- Lazy loading MUST NOT weaken validation. Any section or subsection used for pruning, decoding, or planning MUST be bounds-checked and checksum-validated before use.

---


## 14. Section Kinds

| ID | Name | Profile | Purpose |
| --- | --- | --- | --- |
| 1 | FILE_DICTIONARY_INDEX | shared | Fixed dictionary index entries. |
| 2 | FILE_DICTIONARY_PAYLOAD | shared | Variable/large value payloads. |
| 3 | COLLATION_REGISTRY | shared | Collation/canonicalisation registry. |
| 4 | DIGEST_MANIFEST | shared | Cryptographic digests. |
| 5 | REDACTION_MANIFEST | shared | Redaction audit metadata. |
| 6 | ARROW_INTEROP_HINTS | shared | Arrow mapping hints. |
| 7 | LAKEHOUSE_HINTS | shared | Iceberg/Delta/Hudi/catalog hints. |
| 8 | EXTENSION_REGISTRY | shared | Registered custom extensions. |
| 9 | PROFILE_CAPABILITY_MATRIX | shared | Declared profile support. |
| 10 | TABLE_CATALOG | COVE-T | Table schemas. |
| 11 | TABLE_SEGMENT_INDEX | COVE-T | Segment locators and row ranges. |
| 12 | TABLE_SEGMENT_DATA | COVE-T | Table segment payloads. |
| 13 | COLUMN_DOMAIN | COVE-T | Logical ordering for FileCodes. |
| 14 | ZONE_STATS | COVE-T | Segment/morsel/page stats. |
| 15 | EXACT_SET_INDEX | COVE-T/COVE-A | Exact value-set indexes. |
| 16 | BLOOM_INDEX | COVE-T/COVE-A | Bloom filters. |
| 17 | INVERTED_MORSEL_INDEX | COVE-T/COVE-A | Value-to-morsel indexes. |
| 18 | LOOKUP_INDEX | COVE-A | Point lookup indexes. |
| 19 | AGGREGATE_SYNOPSIS | COVE-A | Counts, histograms, sketches. |
| 20 | COMPOSITE_ZONE_INDEX | COVE-A | Multi-column pruning metadata. |
| 21 | TOPN_ZONE_SUMMARY | COVE-A | Top/bottom zone summaries. |
| 22 | KERNEL_CAPABILITIES | COVE-T/COVE-A | Encoded-kernel capability metadata. |
| 23 | EXTENDED_FEATURE_SET | shared | Feature words beyond the low 64-bit header/postscript fields. |
| 24 | CODEC_EXTENSION_REGISTRY | COVE-CX | Registered lossless codec descriptors, fallback contracts, and codec conformance references. |
| 25 | LAYOUT_PLAN | COVE-L | Optional hierarchical logical read-planning nodes. |
| 26 | SCAN_SPLIT_INDEX | COVE-L | Optional precomputed scan split descriptors. |
| 27 | PAGE_CLUSTER_DIRECTORY | COVE-L | Optional physical page clustering and range-read coalescing metadata. |
| 28 | ZERO_COPY_BUFFER_MAP | COVE-L/shared | Optional Arrow/engine buffer export compatibility metadata. |
| 29 | FAST_METADATA_INDEX | shared | Optional random-access metadata index for wide schemas and large page directories. |
| 35 | COVERAGE_PROVIDER_REGISTRY | COVE-COVERAGE | Coverage providers, proof kinds, proof strength, exactness, and validity declarations. |
| 36 | COVERAGE_SET | COVE-COVERAGE | Coverage set entries over files, segments, pages, morsels, row ranges, objects, paths, or dimensional buckets. |
| 37 | COVERAGE_PLAN_CANDIDATE | COVE-COVERAGE | Optional costed candidate plans for safe do-no-harm coverage planning. |
| 38 | PREDICATE_NORMAL_FORM | COVE-COVERAGE | Canonical predicate AST/CNF/interval/encoded forms used by coverage proofs and caches. |
| 39 | INDEX_ONLY_CAPABILITY | COVE-I/COVE-A | Declarations for metadata/index-only exact or approximate query answers. |
| 45 | SECTION_FEATURE_BINDING | shared | Section/profile/operation-scoped extended feature requiredness bindings. |
| 46 | COVERAGE_PROOF_RECORD | COVE-COVERAGE | Proof records binding predicate forms, providers, coverage sets, validity, and proof semantics. |
| 47 | NESTED_SCHEMA | COVE-T | Authoritative recursive child schema metadata for native List/Struct/Map table columns. |
| 48 | RUNTIME_COMPATIBILITY_HINTS | COVE-R | Optional runtime/session adapter selection hints. |
| 30 | ENGINE_PROFILE_REGISTRY | COVE-E | Registered engine execution profiles. |
| 31 | EXECUTION_CODE_DESCRIPTOR | COVE-E | ExecutionCode description. |
| 32 | EXECUTION_SCOPE_DESCRIPTOR | COVE-E | Execution scope metadata. |
| 33 | CODE_SPACE_DESCRIPTOR | COVE-E | Code-space metadata. |
| 34 | ENGINE_MOUNT_POLICY | COVE-E | Generic mount/execution mapping policy. |
| 40 | OBJECT_TYPE_CATALOG | COVE-O | Object/property catalog. |
| 41 | TEMPORAL_SEGMENT_INDEX | COVE-O | Temporal segment locators. |
| 42 | TEMPORAL_SEGMENT_DATA | COVE-O | Temporal segment payloads. |
| 43 | TEMPORAL_BLOOM_INDEX | COVE-O | Scope/branch/GOID/time bloom filters. |
| 44 | TRUST_MANIFEST | COVE-O | Trust-chain metadata. |
| 50 | HARBOR_MOUNT_HINTS | COVE-H | Harbor-specific lease/mount hints. |
| 60 | MAP_SOURCE_CATALOG | COVE-MAP | Source system/file/table/stream declarations and source-load fingerprints. |
| 61 | MAP_FUNCTION_REGISTRY | COVE-MAP | Declared deterministic normalisation, canonicalisation, hashing, and derivation functions. |
| 62 | MAP_IDENTITY_RULE_CATALOG | COVE-MAP | Object identity, multi-column join-key, confidence-class, merge, and do-not-merge rules. |
| 63 | MAP_ROW_SEMANTICS_CATALOG | COVE-MAP | Source row semantics: object, event, link, association, composite, dispatch, key/value fragment, projection, and evidence-only rules. |
| 64 | MAP_ASSERTION_LOG | COVE-MAP | Optional canonical semantic assertion stream produced by applying mapping rules. |
| 65 | MAP_IDENTITY_EQUIVALENCE_INDEX | COVE-MAP | Deterministic identity-key to destination-GOID/equivalence-set index. |
| 66 | MAP_EVIDENCE_INDEX | COVE-MAP | Source row, rule, digest, and output assertion evidence. |
| 67 | MAP_CONVERSION_REPORT | COVE-MAP | Conversion diagnostics, conflicts, candidate matches, rejected rows, and fidelity report. |
| 68 | MAP_PROJECTION_CATALOG | COVE-MAP | Object-and-association to table projection definitions and read-surface declarations. |
| 69 | MAP_RESOLUTION_CATALOG | COVE-MAP | Resolver catalogs, normalisation pipelines, candidate rules, and reviewed resolution decisions. |
| 255 | VENDOR_EXTENSION | shared | Reserved extension section. |

MAP_* section payloads are COVE-MAP profile payloads whose standard schema is defined by Section 70. The authoritative reusable mapping definition normally lives in a `.covemap` artifact. MAP_* sections embedded in a `.cove` file are intended for mapping evidence, projection catalogs, conversion reports, identity-equivalence indexes, or embedded mapping snapshots tied to that file or dataset state; they MUST NOT silently override an explicitly referenced reusable mapping definition unless a required profile or extension defines that authority rule. A writer MUST NOT place MAP_* sections in an ordinary COVE file unless it advertises FEATURE_SEMANTIC_MAP and the payload conforms to the COVE-MAP v2 schema or to a registered required extension. General COVE readers MUST ignore optional MAP_* sections for ordinary COVE-T or COVE-O reads. COVE-MAP-aware tools MUST validate MAP_* payload schemas, source fingerprints, function registries, and evidence references before using them for conversion, replay, projection, or explanation.

`.covedelta` artifacts use an artifact-local `section_kind` namespace in `CoveDeltaSectionDirectoryEntryV1`. These values are not global `.cove` section-kind IDs. A delta `section_id` is a unique section instance ID within the `.covedelta` artifact; `section_kind` identifies the local delta payload schema.

| Delta kind | Name | Purpose |
| ---: | --- | --- |
| 0 | `DELTA_PARENT_REFS` | Digest-bound base, parent-delta, and sidecar references. |
| 1 | `DELTA_CATALOG_PATCH` | Additive object, property, catalog, temporal-role, branch, or projection declarations. |
| 2 | `DELTA_DICTIONARY_OVERLAY` | Delta-local dictionary entries and validated parent aliases. |
| 3 | `DELTA_TEMPORAL_SEGMENT_INDEX` | Delta-local COVE-O temporal segment index. |
| 4 | `DELTA_TEMPORAL_SEGMENT_DATA` | Delta-local COVE-O temporal segment payloads. |
| 5 | `DELTA_CONTINUATION_ANCHORS` | Logical predecessor anchors for touched existing objects. |
| 6 | `DELTA_TOUCHED_OBJECT_SET` | Conservative or exact changed object/property summary. |
| 7 | `DELTA_TOMBSTONE_SET` | Conservative or exact tombstone summary. |
| 8 | `DELTA_PROPERTY_OPS` | Sparse property operation streams when not embedded in temporal pages. |
| 9 | `DELTA_EVIDENCE_PATCH` | Additional replay or explanation evidence metadata. |
| 10 | `DELTA_PROJECTION_PATCH` | Projection metadata or invalidation summaries. |
| 11 | `DELTA_COVERAGE_PATCH` | Delta-local COVE-COVERAGE providers and sets. |
| 12 | `DELTA_INDEX_HINTS` | Optional references to delta-local COVE-I or COVX artifacts. |
| 13 | `DELTA_LAYOUT_HINTS` | Optional byte-range and object-store planning hints. |
| 14 | `DELTA_TRUST_CONTINUATION` | Trust-chain continuation and state-hash metadata. |
| 15 | `DELTA_STRING_TABLE` | String and byte payload table for descriptors. |
| 16 | `DELTA_BRANCH_IDENTITY_TABLE` | Canonical branch identity descriptors. |
| 17 | `DELTA_SCOPE_TABLE` | Scope descriptors used by summaries and records. |
| 18 | `DELTA_TEMPORAL_ROLE_SUMMARY_TABLE` | Temporal-role range summaries. |
| 19 | `DELTA_TOUCHED_SUMMARY_TABLE` | Exact or conservative touched-object summaries. |
| 20 | `DELTA_TOMBSTONE_SUMMARY_TABLE` | Exact or conservative tombstone summaries. |
| 21 | `DELTA_STATE_HASH_TABLE` | Canonical state-hash descriptors and payload references. |
| 255 | `DELTA_EXTENSION` | Required or optional extension payload. |

Only the delta temporal segment sections plus metadata needed to validate them are required for a minimal object delta. Delta-local index, coverage, layout, evidence, and projection patches are optional unless selected by a required delta feature or by the requested operation.

---


## 15. Metadata JSON

Footer metadata JSON is optional, descriptive, and non-authoritative.
**Example:**

```json
{
  "format_version": "1.0",
  "format_name": "Cove Format",
  "created_by": "cove-writer/<version>",
  "created_at_us": 0,
  "primary_profile": "COVE-T",
  "source": {
    "format": "parquet",
    "schema_fingerprint": "",
    "conversion_policy": ""
  },
  "writer": {
    "morsel_row_count": 4096,
    "segment_target_uncompressed_bytes": 134217728
  },
  "notes": {}
}
```

**Rules:**
- metadata_len MUST be <= 1 MiB.
- Readers MUST ignore unknown metadata keys.
- Metadata JSON MUST NOT be required for correctness.

---
