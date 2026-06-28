# Redaction, Digests, and Compression

## 64. Redaction

A redacted value is present but inaccessible.
It is not null.

```rust
struct RedactionManifestEntryV2 {
    redaction_id: u64,

    section_id: u32,
    local_ref: u64,

    reason_code: u16,

    policy_id_len: u16,
    policy_id: [u8],

    audit_ref_len: u16,
    audit_ref: [u8],

    created_at_us: i64,
}
```

**Rules:**
- Readers MUST NOT silently expose redacted payload bytes.
- Readers MUST NOT silently treat redacted values as null.
- Query engines MAY compare redacted markers only according to policy.

### 64.1 Security and Privacy Boundary

COVE v2 provides corruption detection, optional cryptographic digests, redaction markers, and trust metadata. It does not define a complete access-control, key-management, or encrypted-storage protocol.

**Rules:**
- The encryption fields in v2 section specs and postscript specs MUST be 0. Encrypted sections, encrypted columns, authenticated encryption modes, key identifiers, key rotation, and associated-data rules require a future required extension or profile.
- Redaction is a logical/audit marker, not access control. If sensitive bytes are present unencrypted in a COVE file, COVE redaction metadata alone does not prevent disclosure.
- Column-level or row-level access control is external to COVE v2. Engines enforcing access policy MUST apply that policy before exposing decoded values, indexes, synopses, dictionaries, or metadata that could reveal protected data.
- Indexes, dictionaries, exact sets, blooms, histograms, Top-N summaries, and aggregate synopses may reveal value distributions. Writers handling sensitive datasets SHOULD omit or coarsen acceleration metadata according to policy.
- Differentially private, sampled, masked, or otherwise privacy-preserving statistics MUST be marked as approximate or policy-protected. They MUST NOT be used as exact aggregate synopses, exact value sets, or predicate-proof metadata unless the proof remains valid under the declared privacy transformation.

---


## 65. Digest Manifest

The digest manifest provides cryptographic integrity.

```rust
struct DigestManifestHeaderV2 {
    digest_algorithm: u16,   // 1=SHA-256, 2=BLAKE3
    digest_scope: u16,       // 0=file, 1=section, 2=page, 3=merkle

    entry_count: u32,

    entries_offset: u64,
    entries_length: u64,

    root_digest: [u8; 32],

    checksum: u32,
}
```

```rust
struct DigestEntryV2 {
    target_kind: u16,        // section/page/file/custom
    digest_len: u16,

    section_id: u32,
    local_id: u64,

    offset: u64,
    length: u64,

    digest: [u8; digest_len],
}
```

**Rules:**
- Digest manifests are optional.
- Public archive datasets SHOULD include them.
- COVX and COVM SHOULD reference COVE files by cryptographic digest.
- Digest validation failure MUST be reported.
- If digest validation is required by policy, failure MUST reject the file.

---


## 66. Compression

```rust
enum CompressionCodec {
    None = 0,
    Lz4 = 1,
    Zstd = 2,
}
```

**Rules:**
- Readers MUST support None.
- Readers SHOULD support LZ4.
- Zstd requires FEATURE_CODEC_ZSTD.
- Unknown required compression codecs cause rejection.
**Recommended policy:**
**Metadata:**
  None or LZ4

**Hot scan pages:**
  LZ4

**Cold archive pages:**
  Zstd

**Already compact bit-packed pages:**
  MAY be uncompressed

**Indexes:**
  None or LZ4

**Codec selection:**
- Compression codecs wrap already encoded byte buffers or sections; they are distinct from CoveEncodingKind array encodings.
- Writers SHOULD evaluate the codec choice after array encoding selection.
- Writers SHOULD leave already compact bit-packed, RLE, run-end, local-codebook, or stats-only constant pages uncompressed when a block codec does not reduce size.
- Writers MAY use Zstd for cold archive sections and LZ4 for hot scan sections, but MUST advertise any required codec feature bits.

---
