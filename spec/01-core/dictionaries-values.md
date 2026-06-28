# Dictionaries, Values, and Physical Kinds

## 16. File Dictionary

The file dictionary maps dense file-local FileCodes to canonical logical values.
FileCode = zero-based ordinal into dictionary index
**Example:**
dictionary[0] = "active"
dictionary[1] = "pending"
dictionary[2] = "closed"

FileCode(0) = "active"
FileCode(1) = "pending"
FileCode(2) = "closed"

### 16.1 Dictionary Header

```rust
struct FileDictionaryHeaderV2 {
    entry_count: u32,
    flags: u32,

    index_entry_len: u16,
    value_hash_algorithm: u16,
    // 0=None
    // 1=xxh3_64
    // 2=sha256_truncated64

    payload_length: u64,

    reserved: [u8; 24],
}
```

### 16.2 Dictionary Index Entry

```rust
struct FileDictionaryIndexEntryV2 {
    value_tag: u16,
    storage_class: u8,
    flags: u8,

    inline_len: u8,
    reserved0: [u8; 3],

    inline_data: [u8; 16],

    payload_offset: u64,
    payload_length: u32,

    canonical_hash64: u64,

    reserved1: u32,
}
```

**Rules:**
- Any FileCode >= entry_count is invalid.
- payload_offset + payload_length MUST be within FILE_DICTIONARY_PAYLOAD.
- canonical_hash64 is an acceleration hint, not proof of equality.
- Equality is determined by value_tag and canonical value bytes.
- Redacted values MUST be marked with StorageClass::Redacted.

### 16.3 Value Tags

```rust
enum ValueTag {
    Null = 0,

    BoolFalse = 1,
    BoolTrue = 2,

    Int64 = 3,
    UInt64 = 4,

    Float32Bits = 5,
    Float64Bits = 6,

    Decimal64 = 7,
    Decimal128 = 8,

    DateDays = 9,
    TimestampMicros = 10,
    TimestampNanos = 11,

    Utf8 = 12,
    Binary = 13,
    Uuid = 14,
    Json = 15,

    List = 16,
    Struct = 17,
    Map = 18,
}
```

### 16.4 Storage Classes

```rust
enum StorageClass {
    Inline = 0,
    Payload = 1,
    Redacted = 2,
}
```

**Rules:**
- Redacted values are present values, not nulls.
- Redacted values MUST have redaction manifest entries.
- Readers MUST NOT silently treat redacted values as null.

---


## 17. Canonical Value Encoding

**Scalar canonical payloads:**
**BoolFalse / BoolTrue:**
  no payload

**Int64:**
  i64 little-endian

**UInt64:**
  u64 little-endian

**Float32Bits:**
  raw IEEE-754 bits as u32 little-endian

**Float64Bits:**
  raw IEEE-754 bits as u64 little-endian

**Decimal64:**
  i64 unscaled value

**Decimal128:**
  i128 two's-complement little-endian unscaled value

**DateDays:**
  i32 days since Unix epoch

**TimestampMicros:**
  i64 microseconds since Unix epoch

**TimestampNanos:**
  i64 nanoseconds since Unix epoch

**Uuid:**
  16 raw UUID bytes

**Utf8:**
  [len: varint][utf8 bytes]

**Binary:**
  [len: varint][bytes]

**Json:**
  [len: varint][utf8 JSON bytes]
**Nested canonical payloads:**
**List:**
  [element_count: varint]
**repeated:**
    [element_value_tag: varint]
    [element_payload]

**Struct:**
  [field_count: varint]
**repeated sorted by field_id ascending:**
    [field_id: varint]
    [field_value_tag: varint]
    [field_payload]

**Map:**
  [pair_count: varint]
**repeated sorted by canonical key bytes:**
    [key_value_tag: varint]
    [key_payload]
    [value_value_tag: varint]
    [value_payload]
**Map rules:**
- Map keys MUST be scalar.
- Map keys MUST NOT be List, Struct, or Map.
- Duplicate canonical keys are invalid.

---


## 18. Logical Types

```rust
enum CoveLogicalType {
    Null = 0,

    Bool = 1,

    Int8 = 2,
    Int16 = 3,
    Int32 = 4,
    Int64 = 5,

    UInt8 = 6,
    UInt16 = 7,
    UInt32 = 8,
    UInt64 = 9,

    Float32 = 10,
    Float64 = 11,

    Decimal64 = 12,
    Decimal128 = 13,

    DateDays = 14,
    TimestampMicros = 15,
    TimestampNanos = 16,

    Utf8 = 17,
    Binary = 18,
    Uuid = 19,
    Json = 20,

    List = 21,
    Struct = 22,
    Map = 23,
}
```

Logical type describes value semantics.

Logical type is independent of physical representation.

**Examples:**
**Utf8 may be physically:**
  FileCode or VarBytes

**Int64 may be physically:**
  NumCode or FileCode

**TimestampMicros may be physically:**
  NumCode

---


## 19. Physical Kinds

```rust
enum CovePhysicalKind {
    FileCode = 0,
    NumCode = 1,
    Boolean = 2,
    FixedBytes = 3,
    VarBytes = 4,
    List = 5,
    Struct = 6,
    Map = 7,
}
```

### 19.1 NumCode Compatibility

**Allowed logical types for NumCode:**
Bool if explicitly declared numeric
Int8/16/32/64
UInt8/16/32/64
Float32/Float64
Decimal64
DateDays
TimestampMicros
TimestampNanos
**Rules:**
- In v2, `Bool if explicitly declared numeric` is declared with the
  per-column/property numeric flag: `TableColumnEntryV2.flags bit 0`,
  `TableColumnDirectoryEntryV2.flags bit 0`, and `PropertyEntryV2.flags bit 8`.
  Catalog and segment declarations for the same column/property MUST agree.
- NumCode MUST be interpreted by declared logical_type.
- NumCode MUST NOT be dictionary-resolved.
- Numeric min/max statistics use logical ordering.
**Float rules:**
- Float values preserve raw IEEE bit patterns.
- NaN values are valid.
- Min/max statistics exclude NaN and set HAS_NAN.
- Readers MUST NOT use min/max to exclude NaN-sensitive predicates unless safe.


### 19.2 Portable NumCode Encoding Metadata

NumCode is portable only because its interpretation is declared by COVE metadata. A reader MUST NOT infer logical comparison safety from the raw numeric width or from an engine-specific representation.

```rust
struct NumCodeEncodingDescriptorV2 {
    descriptor_id: u32,
    logical_type: u16,
    physical_width_bits: u16,
    signedness: u8,          // 0=unsigned, 1=signed, 2=raw_bits, 3=decimal_scaled
    byte_order: u8,          // 1=little-endian in v2
    scale: i16,
    offset_kind: u8,         // 0=none, 1=signed_i64, 2=unsigned_u64, 3=decimal128
    flags: u8,
    min_logical_ref: u32,
    max_logical_ref: u32,
    overflow_policy: u8,     // 0=reject, 1=wrap_invalid, 2=saturate_invalid, 3=extension_defined
    null_representation: u8, // 0=null_bitmap_only in core v2
    reserved: u16,
    checksum: u32,
}
```

**Descriptor flags:**

| Bit | Name | Meaning |
| --- | --- | --- |
| 0x0001 | ORDER_PRESERVING | Physical order is identical to logical order for non-null values under the declared logical type. |
| 0x0002 | EQUALITY_PRESERVING | Physical equality is identical to logical equality for non-null values. |
| 0x0004 | RANGE_COMPARISON_SAFE | Range predicates may be evaluated over the encoded physical domain without logical decode. |
| 0x0008 | ENCODED_PREDICATE_SAFE | Declared encoded predicate kernels are equivalent to baseline logical evaluation. |
| 0x0010 | ADAPTIVE_WIDTH | Values may be stored in an adaptive width stream such as u8/u16/u32/u64 or i8/i16/i32/i64 under this descriptor. |
| 0x0020 | BITPACKED_WIDTH | Values may be bit-packed with a declared bit width. |
| 0x0040 | DELTA_OR_FOR | Values may use delta, frame-of-reference, or scaled integer transforms under declared codec rules. |
| 0x0080 | FLOAT_RAW_BITS | Float values preserve raw IEEE bits and require float-specific predicate safety rules. |

**Rules:**
- `null_representation` MUST be `null_bitmap_only` for COVE-Core/COVE-T v2. NumCode values are never null sentinels.
- Physical equality, ordering, and range comparison are usable only when the corresponding descriptor flags are set and the logical type, collation, NaN, signed-zero, decimal scale, timestamp unit, and overflow rules are understood.
- Encoded predicate kernels MUST NOT run on NumCode streams unless `ENCODED_PREDICATE_SAFE` or a codec-specific equivalent is declared and validated.
- A descriptor mismatch between catalog, page, codec, and kernel metadata is corruption for the affected operation.
- A reader MAY ignore NumCode encoding descriptors and decode through the baseline logical path.

---
