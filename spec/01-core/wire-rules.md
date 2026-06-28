# Primitive Wire Rules

## 8. Primitive Wire Rules

### 8.1 Endianness

All multi-byte integers are little-endian.
COVE v2 deliberately chooses one canonical byte order instead of storing a byte-order marker or negotiating host endianness. This keeps section parsing, memory-mapped fixed-width fields, checksum coverage, and conformance vectors deterministic.
**Rules:**
- Writers MUST emit little-endian values.
- Readers on big-endian hosts MUST byte-swap multi-byte scalar fields into host order before interpretation.
- Encoded byte streams whose algorithm defines its own byte order MUST follow that algorithm's registered COVE encoding definition.
- Future formats that want byte-order negotiation MUST use new magic, a new major version, or an explicitly incompatible required feature bit.

### 8.2 Boolean

Boolean fields are encoded as u8.
0 = false
1 = true
Other values are invalid unless explicitly assigned by an enum.

### 8.3 Varint

Unsigned varints use LEB128-style base-128 encoding.
Signed varints use ZigZag encoding before unsigned varint encoding.
zigzag_i64(x) = (x << 1) ^ (x >> 63)

### 8.4 UUID

UUIDs are stored as raw 16-byte canonical UUID byte order.
UUID values MUST NOT be truncated.

### 8.5 Strings

Strings are UTF-8 byte sequences.
Unless a specific collation is declared, string equality is byte equality.

### 8.6 Checksums

**CRC algorithm:**
CRC32C / Castagnoli
CRC fields are computed over the covered byte range with the CRC field itself set to zero if the covered structure contains its own CRC field.
CRC32C is for corruption detection, not cryptographic trust.

**Checksum coverage discipline:**

| Structure | Coverage rule |
| --- | --- |
| Header | CRC32C over the header bytes with the header checksum field zeroed. |
| Postscript | CRC32C over the postscript bytes with the postscript checksum field zeroed. |
| Footer section spec | CRC32C over the stored footer bytes after section-level decompression rules are applied only when the spec says the CRC covers decoded bytes; otherwise over stored bytes. COVE-Core v2 default is stored bytes. |
| Section entry CRC | CRC32C over the stored section payload bytes unless the section kind explicitly declares decoded-byte coverage. |
| Page checksum | CRC32C over the stored page payload bytes referenced by the page index after page-level compression wrapping; buffer descriptor checksums cover individual decoded page buffers when present. |
| Page buffer checksum | CRC32C over the exact buffer bytes described by the buffer descriptor. |
| Codec envelope checksum | CRC32C over the registered encoding envelope with its checksum field zeroed. |
| Optional enclosing cluster checksum | CRC32C over the stored cluster byte range; individual page checksums remain authoritative. |

**Rules:**
- A structure with an embedded checksum MUST define whether the checksum field is zeroed during computation.
- Writers MUST NOT mix stored-byte and decoded-byte CRC coverage for the same structure kind unless a required extension defines the distinction.
- Cryptographic digests MAY cover file, section, page, decoded logical value, or Merkle scopes, but the digest manifest MUST declare the exact scope.


### 8.7 Cryptographic Digests

COVE MAY include cryptographic digests.
**Supported digest algorithms in v2:**

```rust
enum DigestAlgorithm {
    None = 0,
    Sha256 = 1,
    Blake3 = 2,
}
```

**Rules:**
- CRC32C is mandatory for structural corruption detection.
- Cryptographic digests are optional but recommended for public archives.
- Digest manifests MAY cover file, section, page, or Merkle scopes.

### 8.8 Alignment

Writers SHOULD align major sections to at least 8 bytes.
Writers SHOULD align scan payloads to 64 bytes where practical.
Object-store profiles MAY use larger alignment.

---
