# COVE v2 Implementer Guide

This guide is a practical starting point for independent COVE v2
implementations. It does not reduce the normative specification in
`spec.md`; it names the smallest useful reader target before optional profiles
such as COVE-O, COVE-MAP, COVE-I, COVE-L, COVE-CX, COVE-R, COVX, and COVM.

## Reader Kernel

A useful first reader should implement COVE-Core plus the COVE-T scan profile:

- validate the primary file magic, version, header, postscript, footer, and
  section directory;
- reject malformed required sections, overlapping ranges, checksum failures,
  bad section bounds, and unsupported required features that affect the
  requested operation;
- ignore unknown optional features while preserving enough diagnostics for an
  inspect/report tool;
- parse table catalogs, segment indexes, row morsel directories, column page
  indexes, and page payload references;
- decode structural null bitmaps where `1 = null`;
- decode plain fixed-width values, plain varints, constant pages, and
  file-local FileCode dictionaries for common scalar table columns;
- enforce logical/physical compatibility, including declared Bool-as-NumCode
  and FileCode dictionary bounds;
- validate ColumnDomain and zone-stat metadata before using either for pruning
  or planning;
- export supported primitive, UTF-8, binary, decimal, temporal, UUID, and
  dictionary-backed values to a host representation without changing logical
  values.

Anything outside that list should initially be treated as one of three cases:

- unsupported optional metadata: expose it to diagnostics and continue with a
  scan or decoded fallback;
- unsupported required metadata for the requested operation: reject with a
  stable error;
- corrupt authoritative metadata: reject, even when an optimization could have
  been skipped.

## Writer Kernel

A first writer should stay narrower than the reader:

- emit one table catalog with one or more COVE-T segments;
- use 4,096-row morsels unless there is a documented reason to choose a
  different morsel size;
- emit deterministic section ordering, aligned section ranges, page indexes,
  null bitmaps, page checksums, footer checksums, and postscript pointers;
- use simple encodings first: plain fixed-width, plain varint, constant pages,
  and FileCode dictionaries;
- include zone statistics only when the writer can prove they match the logical
  type, collation, null semantics, and page or morsel range they describe;
- avoid optional profiles in baseline fixtures until the core read/write loop
  passes the conformance subset.

## Conformance Starting Point

The full generated corpus remains the publication gate:

```sh
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```

For a smaller first target, run the minimal reader subset:

```sh
cargo run -p cove-conformance --bin cove-conformance -- \
  conformance/ --manifest conformance/minimal-reader-manifest.jsonl
```

That subset exercises structural file validation, COVE-T scan fixtures, feature
fallback, dictionaries, ColumnDomain, page indexes, zone stats, and common
reject paths. Passing it is not a conformance claim; it is a useful milestone
before attempting the full corpus.

## Implementation Order

1. Parse little-endian wire primitives and fixed-size headers without panics on
   short input.
2. Validate the primary envelope: magic, version, postscript, footer pointer,
   footer checksum, and section directory.
3. Parse COVE-T table, segment, morsel, page-index, and page-payload metadata.
4. Decode null bitmaps and simple scalar pages.
5. Add FileCode dictionaries and ColumnDomain validation.
6. Add zone stats and proof-safe pruning only after decoded scans are correct.
7. Add Arrow, Parquet, ORC, DataFusion, COVI, COVX, COVM, COVE-O, or COVE-MAP
   surfaces as explicit profile claims, not as hidden baseline dependencies.

## Error Behavior

COVE implementations should make failure behavior boring and deterministic:

- use stable error codes when comparing against conformance vectors;
- fail closed for structural corruption, bad checksums, bad required sections,
  and logically impossible authoritative metadata;
- fail open for optional acceleration, advisory layout, unsupported optional
  codecs with valid fallback, sidecar freshness misses, and optimization
  metadata that is not strong enough to prove a result;
- never use advisory metadata to change query results.

## Non-Goals For A First Reader

A first independent implementation does not need to implement semantic mapping,
object-temporal reconstruction, secondary index artifacts, runtime coverage
caches, object-store planning, zero-copy Arrow views, registered codec plugins,
Harbor integration, or DataFusion integration. Those are valuable profile
claims after the reader kernel is correct.
