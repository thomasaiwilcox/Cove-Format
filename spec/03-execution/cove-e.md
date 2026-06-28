# COVE-E Engine Execution Profile

## 38. COVE-E Engine Execution Profile

COVE-E allows an engine to map file-local physical values into implementation-local execution values without changing COVE logical semantics.
**Universal behaviour:**
FileCode -> ExecutionCode
**Examples:**
FileCode -> Arrow dictionary key
FileCode -> DataFusion dictionary array key
FileCode -> DuckDB dictionary vector code
FileCode -> Polars categorical code
FileCode -> custom engine symbol ID
FileCode -> Harbor EngineCode under the optional COVE-H registration
**Rules:**
- COVE-E is optional.
- COVE-E MUST NOT be required to decode COVE-T logical values.
- COVE-E MAY accelerate scans, output materialisation, joins, grouping, and dictionary execution.
- Unknown required COVE-E profiles cause rejection only when the reader needs that profile.
- Unknown optional COVE-E profiles MUST be ignored.

---


## 39. Engine Profile Registry

```rust
struct EngineProfileRegistryHeaderV2 {
    profile_count: u32,
    flags: u32,
}
```

```rust
struct EngineProfileEntryV2 {
    profile_id: u32,

    namespace_len: u16,
    namespace: [u8],
    // Examples:
    //   "org.coveformat.core"
    //   "io.harbor"
    //   "org.duckdb"
    //   "org.apache.arrow"
    //   "org.apache.datafusion"

    profile_name_len: u16,
    profile_name: [u8],
    // Examples:
    //   "harbor-leased-code"
    //   "arrow-dictionary"
    //   "engine-dictionary-code"

    version_major: u16,
    version_minor: u16,

    required_features: u64,
    optional_features: u64,

    execution_descriptor_ref: u32,
    mount_policy_ref: u32,
    private_payload_ref: u32,

    checksum: u32,
}
```

**Rules:**
- namespace MUST be globally unique.
- Unknown required engine profiles MUST cause rejection only if that profile is required for the requested operation.
- Unknown optional engine profiles MUST be ignored.
- Engine profiles MUST NOT change COVE logical values.
- Engine profiles MAY define faster ways to materialise or compare those values.

---


## 40. ExecutionCode Descriptor

```rust
struct ExecutionCodeDescriptorV2 {
    descriptor_id: u32,

    code_kind: u8,
    code_width_bits: u16,
    byte_order: u8,

    lifetime: u8,
    comparison_scope: u8,
    canonicality: u8,
    null_code_policy: u8,

    flags: u32,

    scope_ref: u32,
    code_space_ref: u32,

    checksum: u32,
}
```

```rust
enum ExecutionCodeKind {
    UnsignedInteger = 0,
    SignedInteger = 1,
    OpaqueBytes = 2,
    DictionaryKey = 3,
    EnginePrivate = 255,
}
```

```rust
enum ExecutionCodeLifetime {
    Query = 0,
    Scan = 1,
    Session = 2,
    Mount = 3,
    LeaseEpoch = 4,
    PersistentEngineLocal = 5,
}
```

```rust
enum ExecutionCodeComparisonScope {
    NotComparable = 0,
    File = 1,
    Dataset = 2,
    Catalog = 3,
    Scope = 4,
    EngineGlobal = 5,
}
```

```rust
enum ExecutionCodeCanonicality {
    Transient = 0,
    Leased = 1,
    CanonicalWithinScope = 2,
    EnginePrivate = 255,
}
```

```rust
enum NullCodePolicy {
    NoNullCode = 0,
    EngineDefinesNullCode = 1,
    NullBitmapOnly = 2,
}
```

**Rules:**
- COVE logical nulls remain structural regardless of execution null-code policy.
- If an engine uses a runtime null code, that code is not a COVE FileCode null sentinel.
- ExecutionCode comparison is valid only within the declared comparison_scope.

---


## 41. Execution Scope Descriptor

```rust
struct ExecutionScopeDescriptorV2 {
    scope_id: u32;

    scope_kind: u16;
    flags: u16;

    stable_id_len: u16;
    stable_id: [u8];

    display_name_len: u16;
    display_name: [u8];

    private_payload_ref: u32;
}
```

```rust
enum ExecutionScopeKind {
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

**Examples:**
**Generic lakehouse engine:**
  scope_kind = Catalog
  stable_id  = catalog/table namespace ID

**Single-file reader:**
  scope_kind = None

**COVE-H example:**
  scope_kind = Tenant
  stable_id  = Harbor tenant UUID

---


## 42. Code Space Descriptor

```rust
struct CodeSpaceDescriptorV2 {
    code_space_id: u32;

    namespace_len: u16;
    namespace: [u8];

    stable_id_len: u16;
    stable_id: [u8];

    epoch: u64;

    flags: u32;

    private_payload_ref: u32;
}
```

**Examples:**
**Arrow dictionary output:**
  namespace = "org.apache.arrow"
  stable_id = dictionary batch or schema identifier
  epoch = 0 or batch/session epoch

**Custom engine:**
  namespace = globally unique engine namespace
  stable_id = implementation-specific code-space ID

**COVE-H example:**
  namespace = "io.harbor"
  stable_id = Harbor code-space UUID
  epoch = Harbor lease/code-space epoch
**Rules:**
- Code spaces are implementation-local.
- Code-space metadata MUST NOT be required to recover COVE logical values.
- Code-space epoch MAY be used to invalidate stale execution maps.

---


## 43. Engine Mount Policy

```rust
struct EngineMountPolicyV2 {
    policy_id: u32;

    filecode_mapping_kind: u8;
    missing_value_policy: u8;
    stale_mapping_policy: u8;
    reverse_lookup_policy: u8;

    flags: u32;

    dictionary_digest_ref: u32;
    code_space_ref: u32;
    cache_key_ref: u32;

    private_payload_ref: u32;

    checksum: u32;
}
```

```rust
enum FileCodeMappingKind {
    DecodeToValue = 0,
    MapToExecutionCode = 1,
    MapToArrowDictionary = 2,
    EnginePrivate = 255,
}
```

```rust
enum MissingValuePolicy {
    Error = 0,
    DecodeValueOnly = 1,
    RequestLeaseOrIntern = 2,
    ReturnUnmapped = 3,
}
```

```rust
enum StaleMappingPolicy {
    Rebuild = 0,
    Reject = 1,
    IgnoreIfOptional = 2,
}
```

```rust
enum ReverseLookupPolicy {
    NotAvailable = 0,
    BuildFromDictionary = 1,
    EngineProvided = 2,
    CachedExternal = 3,
}
```

**Examples:**
**Generic Arrow reader:**
  filecode_mapping_kind = MapToArrowDictionary
  missing_value_policy = DecodeValueOnly
  stale_mapping_policy = IgnoreIfOptional

**COVE-H example:**
  filecode_mapping_kind = MapToExecutionCode
  missing_value_policy = RequestLeaseOrIntern
  stale_mapping_policy = Rebuild

---
