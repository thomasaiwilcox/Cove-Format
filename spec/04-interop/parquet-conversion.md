# COVE-T Parquet Conversion Profile

## 51. COVE-T Parquet Conversion Profile

COVE-T is intended as a high-performance conversion target for Parquet data.
The converter SHOULD rewrite data into COVE-native scan layout, not copy Parquet’s physical layout.

### 51.1 Conversion Steps

1. Read Parquet schema and metadata.
2. Select target COVE table schema.
3. Select table segment boundaries.
4. Select morsel row count.
5. Decode Parquet pages as needed.
6. Build COVE file-local dictionary.
7. Assign dense FileCodes.
8. Convert repeated string/category/binary/uuid values to FileCode columns.
9. Convert numeric/date/timestamp values to NumCode columns where appropriate.
10. Build ColumnDomain sections for comparable FileCode columns.
11. Build segment and morsel zone stats.
12. Build exact sets for low/medium-cardinality columns.
13. Build bloom filters for high-cardinality equality columns.
14. Build lookup indexes for declared point-lookup columns.
15. Build aggregate synopses for useful low-cardinality and numeric columns.
16. Build composite zone indexes for declared clustering/filter combinations.
17. Build Top-N summaries for ordered hot columns where useful.
18. Analyze each column page/morsel and encode using COVE-approved encodings under the writer encoding selection policy.
19. Write section directory, footer, postscript, and CRCs.
20. Optionally write COVM/COVX companion artifacts.

### 51.2 Statistics Policy

Converters SHOULD recompute COVE statistics from decoded logical values.
**Converters SHOULD NOT blindly trust source statistics unless they validate:**
- logical type interpretation,
- collation semantics,
- null semantics,
- min/max truncation rules,
- source statistics completeness,
- timestamp/timezone interpretation,
- decimal scale/precision semantics.

### 51.3 Unsupported Nested Shapes

**Unsupported nested shapes MAY be encoded as:**
Json
or
Binary
but MUST be marked pushdown-limited.

### 51.4 Optional Physical Row Reordering

COVE-T writers MAY reorder rows within a table segment to improve compression, clustering, and pruning, but only when row order is not part of the dataset's logical contract.
**Rules:**
- Reordering MUST be opt-in writer behaviour.
- Reordering MUST NOT change the logical multiset of rows or any declared primary/lookup key semantics.
- Reordering MUST happen before morsel IDs, page indexes, zone stats, exact sets, bloom filters, lookup indexes, aggregate synopses, row references, and digest/trust inputs are generated.
- Writers MUST NOT apply physical row reordering to COVE-O temporal/object segments unless the profile explicitly proves that object history order, CSNs, baselines, deltas, tombstones, and trust chains remain semantically identical.
- If source row order is externally observable or needed for reproducibility, the writer SHOULD either disable reordering or materialise a source ordinal column before reordering.
- Writers SHOULD evaluate the benefit before committing a reorder. A reorder SHOULD be kept only when the estimated encoded size, pruning quality, or declared workload score improves enough to justify the additional write cost.
- Writer metadata MAY record the reorder policy and sort keys, but this metadata is descriptive and MUST NOT be required for logical decoding.
**Recommended heuristic:**
- Prefer stable clustering keys with low or medium cardinality and common predicate use.
- Avoid high-cardinality timestamp-only ordering unless time filtering is the dominant workload; coarse time buckets followed by other clustering keys are usually safer.
- Do not reorder nested, temporal, or trust-sensitive data unless the profile explicitly permits it.

### 51.5 Conversion Fidelity and Reporting

Converters are adoption-critical but are not allowed to redefine COVE semantics. A converter MUST NOT claim lossless conversion unless the declared conversion policy preserves logical values, nulls, schema semantics, decimal precision/scale, timestamp units/timezone interpretation, nested structure, map-key rules, and redaction/trust semantics for the supported source features.

**A converter SHOULD produce a machine-readable conversion report containing:**
- source format, source file identifiers, and source digests where available,
- source schema fingerprint and target COVE schema fingerprint,
- row count and column count,
- conversion policy version,
- unsupported or lossy source features,
- nested-shape fallbacks to Json or Binary,
- timestamp/timezone and decimal policies,
- collation and canonicalisation policies,
- row reordering policy, if any,
- generated COVE feature bits, section kinds, and acceleration artifacts,
- validation result for the produced COVE/COVX/COVM artifacts.

**Rules:**
- Source physical encodings, compression codecs, page boundaries, and statistics do not need to be preserved. COVE statistics and indexes SHOULD be recomputed from decoded logical values.
- Bidirectional tools such as Parquet <-> COVE, ORC <-> COVE, Arrow IPC <-> COVE, and CSV <-> COVE MUST distinguish logical round-trip fidelity from physical-layout preservation.
- If a source feature cannot be represented exactly in COVE-Core/COVE-T, the converter MUST either use a required extension, use a declared lossy fallback, or reject the conversion.

---
