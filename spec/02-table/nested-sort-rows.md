# Nested Columns, Sort Metadata, and Row References

## 52. Nested Columns

COVE-T stores native nested columns with two complementary contracts:

- a file-level `NESTED_SCHEMA` section carrying authoritative recursive child schema metadata for each top-level List/Struct/Map table column;
- page-local `ColumnPagePayloadV1` trees carrying the row-shape buffers and child value buffers for one page.

A COVE-T file that contains a native nested column MUST advertise `FEATURE_NESTED_COLUMNS`, MUST include a `NESTED_SCHEMA` section, and MUST provide exactly one nested schema entry for every top-level nested table column. Page-local shape metadata is not sufficient to infer child names, nullability, logical/physical types, decimal metadata, collation, or fixed-size-list assertions.

### 52.1 NESTED_SCHEMA Section

`NESTED_SCHEMA` has section kind `47`, profile `COVE-T`, and requires `FEATURE_NESTED_COLUMNS`.

```rust
struct NestedSchemaSectionV1 {
    magic: [u8; 4],          // "NSC1"
    version_major: u16,      // 1
    header_len: u16,         // 16
    entry_count: u32,
    reserved: u32,           // 0
    entries: NestedSchemaEntryV1[entry_count],
}

struct NestedSchemaEntryV1 {
    table_id: u32,
    column_id: u32,
    root: NestedSchemaNodeV1,
}

struct NestedSchemaNodeV1 {
    name: utf8_u16,
    logical_type: u16,
    physical_kind: u8,
    nullable: u8,            // 0 or 1
    precision: u16,
    scale: i16,
    collation_id: u16,
    child_count: u16,
    flags: u32,
    fixed_size_list_len: u32, // 0 unless this List represents a FixedSizeList
    children: NestedSchemaNodeV1[child_count],
}
```

**Rules:**
- Entries are keyed by `(table_id, column_id)` and MUST be unique.
- Every native top-level nested table column MUST have one matching entry.
- A `NestedSchemaEntryV1.root` MUST match the top-level table column's name, logical type, physical kind, nullability, precision, scale, collation, and flags.
- List nodes MUST have exactly one child.
- Struct nodes MUST have at least one child and child names MUST be unique within the struct.
- Map nodes MUST have exactly two children named `key` and `value`; keys MUST be scalar and non-null.
- Scalar nodes MUST NOT have children.
- `fixed_size_list_len` MUST be zero except on List nodes. Non-zero values assert a fixed element count per non-null parent list; the page still uses ordinary List offsets.

### 52.2 Page Payload Tree Interpretation

Native nested pages use the existing `ColumnPagePayloadV1` wire shape without adding fields.

**Tree rules:**
- The node list is pre-order depth-first.
- The root node MUST be the first node and MUST have `node_id == root_node_id`.
- Each node owns the next consecutive `buffer_count` page buffers in pre-order traversal.
- Each node owns the next consecutive `child_count` child subtrees in pre-order traversal.
- A container node's row-shape payload is stored in a `ChildLayout` buffer.
- Scalar child values use existing scalar encodings and `Values` buffers.
- A node null bitmap, when present, is stored in that node's `NullBitmap` buffer and uses COVE null polarity.

### 52.3 List

**List<T> page layout:**
  parent null bitmap
  `ChildLayout`: offsets `u32[row_count + 1]`
  one child subtree for T
**Rules:**
- offsets MUST be monotonic.
- offsets[0] MUST be 0.
- offsets[row_count] MUST equal the child node logical length.
- For fixed-size-list assertions, every non-null parent row MUST have the asserted element count.

### 52.4 Struct

**Struct page layout:**
  parent null bitmap
  `ChildLayout`: child row counts
  child subtrees, each with `row_count` rows
**Rules:**
- Struct children share parent row count.
- Parent null handling MUST be declared by the layout.

### 52.5 Map

**Map<K,V> page layout:**
  parent null bitmap
  `ChildLayout`: offsets `u32[row_count + 1]`, key/value child counts, duplicate-key policy data
  key child subtree
  value child subtree
**Rules:**
- Map keys MUST be scalar.
- Map keys MUST be non-null.
- Duplicate keys within one map value are invalid unless schema/layout flags allow duplicates.
**Pushdown:**
- Struct child fields MAY support full pushdown.
- List/Map element bloom indexes MAY be provided.
- Whole-list/whole-map min/max is usually unsupported.

### 52.6 Fixed-Size Lists, Vectors, Tensors, and Embeddings

COVE-Core v2 does not define Vector, Tensor, or Embedding as additional scalar logical types. Dense vectors SHOULD be represented by existing nested or extension mechanisms rather than by adding ad hoc core scalar types.

**Recommended representation:**
- For maximum generic compatibility, store a dense fixed-length vector as List<Float32> or List<Float64> with ordinary List offsets and a schema-level fixed-length assertion.
- For space- and scan-optimised storage, a FixedSizeList or Tensor extension MAY elide offsets or use a specialised physical layout only when it declares a required feature bit or a safe List/Binary fallback.

**A FixedSizeList, Tensor, or Embedding extension MUST declare:**
- element logical type,
- dimension count and shape,
- row-major/column-major or other layout order,
- nullable element policy,
- whether vector length is fixed or variable,
- distance/similarity metrics if indexes depend on them,
- normalisation policy if cosine/dot-product semantics depend on it,
- Arrow base type or Arrow extension mapping where exported.

Approximate nearest-neighbour, vector, spatial, learned, or similarity indexes MUST be optional COVX or registered extension indexes. They MAY return candidates, but they MUST NOT be used for predicate exclusion, nearest-neighbour completeness, or metadata-only answers unless their descriptor proves exactness and a no-false-negative policy for the declared metric/query class.

### 52.5 Semi-Structured and Document Values

COVE Json is an opaque UTF-8 JSON payload unless a required extension declares stronger semantics. Core COVE readers MUST NOT assume semantic JSON equality, object-key ordering, numeric normalisation, path typing, or JSON path pushdown from the Json logical type alone.

**Rules:**
- JSON/path indexes MAY be stored as optional COVX or registered extension indexes. They MUST NOT change the logical Json payload.
- A semantic JSON/document extension MUST define canonicalisation, duplicate-key policy, missing-vs-null semantics, numeric normalisation, path type rules, and safe predicate outcomes.
- Without such an extension, Json columns are pushdown-limited to nullness, byte-level equality if declared safe, and indexes that explicitly state their proof semantics.
- COVE-O object-temporal semantics MUST NOT be used as an implicit replacement for general JSON/document semantics.

---


## 53. Sort and Clustering Metadata

```rust
struct SortKeyEntryV2 {
    column_id: u32,
    direction: u8,       // 0=asc, 1=desc
    null_order: u8,      // 0=nulls first, 1=nulls last
    collation_id: u16,
}
```

```rust
struct ClusteringKeyEntryV2 {
    column_id: u32,
    clustering_strength: u8, // 0=unknown, 255=perfect
    reserved: [u8; 3],
}
```

**Rules:**
- Declared sort keys are mandatory claims.
- False sort declarations are format errors.
- Clustering strength is advisory.

---


## 54. Row References

### 54.1 Table Row Reference

```rust
struct CoveTableRowRefV2 {
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,
    row_in_morsel: u16,
}
```

**Use cases:**
- lookup indexes,
- late materialisation,
- row selections,
- diagnostics,
- deferred joins,
- external visibility overlays.
**Rules:**
- CoveTableRowRefV2 identifies a physical row position inside one immutable COVE file.
- External catalogs, delete vectors, lookup overlays, or audit systems that persist row references SHOULD pair the row reference with file_id and a validating file fingerprint such as file_len, footer_crc32c, or cryptographic digest.
- Row references are not stable across conversion, row reordering, compaction, or file rewrite unless an external protocol explicitly maps old references to new references.
- Readers MUST NOT apply row references from one file to another file solely because schemas or paths match.
If future morsels exceed u16::MAX rows, v2 must widen this field or introduce a new row reference type.

---
