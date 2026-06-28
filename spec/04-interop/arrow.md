# Arrow Interop Profile

## 49. Arrow Interop Profile

COVE-T SHOULD support Arrow-compatible output.
**Rules:**
- FileCode columns MAY be exposed as Arrow dictionary arrays.
- NumCode columns MAY be exposed as Arrow primitive arrays.
- Boolean columns MAY be exposed as Arrow boolean arrays.
- FixedBytes columns MAY be exposed as Arrow fixed-size binary arrays.
- VarBytes columns MAY be exposed as Arrow binary or UTF-8 arrays.
- Null bitmap MUST be convertible to Arrow validity bitmap.
- Nested List/Struct/Map layouts MUST have a defined Arrow mapping.
**Null bitmap conversion:**
**COVE:**
  1 = null
  0 = non-null

**Arrow:**
  1 = valid
  0 = null
Therefore, Arrow validity bitmaps require inversion unless a reader materialises a new validity bitmap.

### 49.1 Relationship to Arrow IPC

Arrow IPC is an interchange and transport format for Arrow record batches. COVE is a durable, immutable, query-planning-oriented storage format.
Use Arrow IPC when the primary requirement is to move or persist already-materialised Arrow arrays or RecordBatches with minimal semantic translation.
Use COVE when the primary requirement is offline/archive storage with file-level dictionaries, predicate-proof metadata, encoded arrays, optional lookup/synopsis indexes, digests, sidecars, and dataset manifests.
**Rules:**
- COVE readers MAY export Arrow arrays, RecordBatches, streams, or files.
- Arrow IPC is not a canonical serialisation of a COVE file. COVE section metadata, predicate proofs, FileCode domains, digests, and optional acceleration artifacts remain authoritative only in COVE/COVX/COVM.
- COVE writers MAY ingest Arrow IPC/Feather/RecordBatch streams as source data, but they SHOULD recompute COVE statistics, dictionaries, domains, and indexes from logical values rather than preserving Arrow batch boundaries as COVE morsel boundaries by default.
- A COVE-to-Arrow conversion MUST report or represent any COVE logical type, collation, extension type, or metadata guarantee that cannot be expressed exactly in Arrow.
**Zero-copy interop:**
- Zero-copy Arrow export is an implementation optimisation, not a COVE conformance requirement.
- A reader MAY expose COVE buffers to Arrow without copying only when the COVE physical layout, offsets, endianness, nullability representation, alignment, lifetime, and dictionary key width are compatible with the Arrow array being produced.
- When COVE null bitmaps, encoded pages, FileCode widths, nested offsets, or dictionary values do not match the target Arrow layout, the reader MUST materialise compatible Arrow buffers rather than exposing incompatible COVE bytes as Arrow memory.
- Writers SHOULD NOT weaken COVE encoding, statistics, or predicate metadata solely to maximise zero-copy Arrow export.

**FileCode to Arrow dictionary mapping:**
**Arrow dictionary keys:**
  MAY reuse FileCode values if key width permits.

**Arrow dictionary values:**
  decoded from COVE file dictionary.
**Rules:**
- Arrow interop MUST NOT require Harbor.
- Arrow interop MUST preserve COVE logical type semantics.
- If a COVE logical type cannot be represented exactly in Arrow, the reader MUST either expose an extension type or report a lossy conversion.


### 49.2 Arrow Export Profiles

COVE is Arrow-friendly but not Arrow-dependent. Arrow export profiles describe how a reader may expose COVE values to Arrow runtimes without making Arrow IPC or Arrow memory layout the canonical COVE representation.

| Profile | Meaning |
| --- | --- |
| COVE-Arrow-Owned | Reader materialises Arrow-compatible owned buffers. This is the safest universal export path. |
| COVE-Arrow-View | Reader exposes COVE buffers as Arrow-compatible views when lifetime, alignment, offset, null, and ownership rules are satisfied. |
| COVE-Arrow-Dictionary | Reader exposes FileCode or remapped dictionary data as Arrow dictionary arrays where key width and dictionary values are compatible. |

**Rules:**
- Arrow export MUST preserve COVE logical values, null positions, redaction policy, nested structure, and extension-type reporting.
- A reader MUST materialise owned Arrow buffers when zero-copy view requirements are not met.
- Arrow view export MUST NOT expose COVE null bitmaps as Arrow validity bitmaps unless the polarity is compatible or the target explicitly accepts COVE polarity.
- FileCode values MAY be reused as Arrow dictionary keys only when the key width, dictionary ordering, null representation, and lifetime are compatible.
- Arrow export profiles are interoperability surfaces. They MUST NOT become COVE schema authority.

---
