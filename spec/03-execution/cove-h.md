# COVE-H Harbor Registration

## 44. COVE-H Harbor Registration

COVE-H is the Harbor registered implementation of COVE-E.
**Registration:**
**namespace:**
  "io.harbor"

**profile_name:**
  "harbor-leased-code"

**ExecutionCode:**
  u64 leased Harbor EngineCode

**Scope:**
  Tenant

**CodeSpace:**
  Harbor dictionary/code-space

**Mapping:**
  FileCode -> Harbor EngineCode

**Lifetime:**
  LeaseEpoch

**Stale policy:**
  Rebuild

**Cache:**
  External Harbor mount cache

### 44.1 Harbor Mount Hints

```rust
struct HarborMountHintsV2 {
    harbor_profile_version_major: u16;
    harbor_profile_version_minor: u16;

    tenant_scope_ref: u32;
    code_space_ref: u32;

    lease_epoch: u64;

    dictionary_digest_ref: u32;
    catalog_digest_ref: u32;

    mount_cache_policy: u8;
    reserved: [u8; 7];

    private_payload_ref: u32;

    checksum: u32;
}
```

**Rules:**
- HarborMountHints are optional outside Harbor.
- Generic readers MUST ignore HarborMountHints.
- Harbor readers MAY use them to build or validate mount caches.
- HarborMountHints MUST NOT be required to decode COVE-T values.
- Harbor readers MUST NOT treat on-disk FileCodes as Harbor EngineCodes.

### 44.2 Harbor Mount Steps

1. Validate COVE structure and required sections.
2. Read table catalog.
3. Read file dictionary.
4. Resolve or lease Harbor EngineCodes for required dictionary values.
5. Build FileCode -> Harbor EngineCode map.
6. Build reverse lookup:
     query literal -> FileCode where possible.
7. Read ColumnDomain and scan index metadata.
8. Validate optional COVX/COVM if present.
9. Expose tables to Harbor query planner.

### 44.3 Harbor Mount Code Map

```rust
type HarborEngineCode = u64;
```

```rust
struct HarborMountCodeMap {
    file_id: [u8; 16],
    table_id: u32,

    dictionary_crc32c: u32,
    lease_epoch: u64,

    filecode_to_enginecode: Vec<HarborEngineCode>,
}
```

**Rules:**
- HarborMountCodeMap is external Harbor metadata.
- HarborMountCodeMap is not authoritative COVE data.
- If missing or stale, it MUST be rebuilt.
- Harbor EngineCodes are not required for offline COVE readability.

---
