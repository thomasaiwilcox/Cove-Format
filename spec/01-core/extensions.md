# Extension Registry and Custom Extensions

## 45. Extension Registry

The extension registry allows custom logical types, physical types, encodings, indexes, synopses, predicate kernels, engine profiles, policies, and vendor metadata.

```rust
struct ExtensionRegistryHeaderV2 {
    extension_count: u32;
    flags: u32;
}
```

```rust
struct ExtensionEntryV2 {
    extension_id: u32;

    namespace_len: u16;
    namespace: [u8];

    name_len: u16;
    name: [u8];

    version_major: u16;
    version_minor: u16;

    extension_kind: u16;
    required_feature_bit: u64;
    optional_feature_bit: u64;

    fallback_kind: u16;
    fallback_ref: u32;

    payload_ref: u32;

    checksum: u32;
}
```

```rust
enum ExtensionKind {
    LogicalType = 0,
    PhysicalKind = 1,
    Encoding = 2,
    CompressionCodec = 3,
    Index = 4,
    AggregateSynopsis = 5,
    PredicateKernel = 6,
    EngineProfile = 7,
    RedactionPolicy = 8,
    TrustPolicy = 9,
    SemanticMapping = 10,
    MappingFunction = 11,
    SourceConnector = 12,
    VendorMetadata = 255,
}
```

**Rules:**
- Unknown required extensions MUST cause rejection when needed.
- Unknown optional extensions MUST be ignored.
- Extension payloads MUST be length-delimited and checksummed.
- Extensions MUST NOT change the meaning of COVE-Core values unless the extension is required.
- Any custom physical encoding MUST provide a canonical fallback or require a feature bit.
- Any custom logical type SHOULD provide a base logical type or Arrow extension mapping.


### 45.1 V2 Registry Discipline

COVE v2 strengthens the extension registry so that extension identifiers are portable and conformance-testable rather than purely runtime-local.

**Rules:**
- Extension namespace, name, version, and kind MUST identify a stable public contract, not merely an implementation class name.
- Required extensions MUST declare exact fallback and failure behaviour.
- Extension payloads MUST be length-delimited, checksummed, and bound to a feature bit or requested operation.
- Runtime registry/session identifiers MAY be used to instantiate implementation code, but they MUST NOT be the only authority for on-disk semantics.
- A vendor extension MAY be optional and ignorable; if it is required to decode projected data or preserve semantics, it MUST be a required extension and MUST provide conformance vectors.

### 45.2 Runtime Registry Name Binding

```rust
struct RuntimeRegistryBindingV2 {
    extension_id: u32,

    registry_kind: u16,
    // 0=codec
    // 1=layout
    // 2=index
    // 3=synopsis
    // 4=predicate_kernel
    // 5=engine_profile
    // 6=mapping_function
    // 7=ffi_adapter

    runtime_namespace_len: u16,
    runtime_namespace: [u8],

    runtime_name_len: u16,
    runtime_name: [u8],

    runtime_version_major: u16,
    runtime_version_minor: u16,

    required: u8,
    flags: u8,
    reserved: u16,

    checksum: u32,
}
```

`RuntimeRegistryBindingV2` is optional COVE-R metadata. It helps implementations map portable extension definitions to local registries, but it is not a substitute for the extension definition itself.

**Rules:**
- Unknown optional runtime bindings MUST be ignored.
- Unknown required runtime bindings cause rejection only for operations that explicitly request that runtime integration.
- Runtime bindings MUST NOT change canonical decode, predicate-proof semantics, COVE-MAP identity, or COVE-O reconstruction.

---


## 46. Custom Logical Types

```rust
struct ExtensionLogicalTypeV2 {
    extension_id: u32;

    base_logical_type: u16;
    canonical_value_tag: u16;

    collation_id: u16;
    flags: u16;

    arrow_extension_name_len: u16;
    arrow_extension_name: [u8];

    metadata_payload_ref: u32;
}
```

**Rules:**
- If base_logical_type is known, generic readers MAY expose the value as the base type.
- If no safe base type exists and the type is required, unknown readers MUST reject.
- Range pushdown requires known collation/order semantics.
- Custom logical types SHOULD preserve a portable decode path.
**Example:**
**Custom PatientId:**
  namespace = "io.example.health"
  name = "patient-id"
  base_logical_type = Utf8
  canonical_value_tag = Utf8
Generic readers can decode it as UTF-8.

---


## 47. Custom Indexes and Synopses

```rust
struct ExtensionIndexDescriptorV2 {
    extension_id: u32;

    index_kind: u16;
    key_column_count: u16;

    proof_capability: u8;
    // 0=none
    // 1=DefinitelyNo
    // 2=DefinitelyNo+DefinitelyYes

    false_negative_policy: u8;
    // 0=must-not-have-false-negatives
    // 1=may-have-false-negatives, cannot be used for skipping

    flags: u32;

    payload_ref: u32;
}
```

**Rules:**
- An index that may have false negatives MUST NOT be used to skip data.
- Unknown custom indexes MUST be ignored.
- Custom indexes may live in COVE or COVX.
- Custom indexes MUST NOT change query results.

---
