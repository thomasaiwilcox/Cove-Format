# COVE v2.0 Split Specification

This directory contains the split COVE v2.0 standards-suite specification. The
split is an editorial organization of the root `spec.md` baseline: section
numbers, heading titles, and normative meaning are preserved.

Start here by task:

- Implementing a minimal reader/writer: read `00-overview/`, `01-core/`, and
  `02-table/`.
- Working with engine code mappings: read `03-execution/`.
- Exporting or converting data: read `04-interop/`.
- Reading object-temporal data: read `05-object/`.
- Using layout, runtime, sidecars, or manifests: read `06-layout-runtime/`.
- Mapping source data into objects and associations: read `07-mapping/`.
- Validating conformance or writer claims: read `08-conformance/`.
- Using optional AI companion metadata: read `09-ai/`.
- Publishing query-discovery metadata for CoveQL tooling and agents: read
  `10-query-discovery/`.

## Document Map

| Area | Documents |
| --- | --- |
| Overview | [`standards-suite.md`](00-overview/standards-suite.md), [`core-concepts.md`](00-overview/core-concepts.md) |
| Core | [`wire-rules.md`](01-core/wire-rules.md), [`file-layout.md`](01-core/file-layout.md), [`dictionaries-values.md`](01-core/dictionaries-values.md), [`encoded-arrays-codecs.md`](01-core/encoded-arrays-codecs.md), [`extensions.md`](01-core/extensions.md), [`redaction-digests.md`](01-core/redaction-digests.md), [`failure-compatibility.md`](01-core/failure-compatibility.md) |
| Table | [`cove-t-storage.md`](02-table/cove-t-storage.md), [`predicates-coverage.md`](02-table/predicates-coverage.md), [`table-indexes.md`](02-table/table-indexes.md), [`nested-sort-rows.md`](02-table/nested-sort-rows.md) |
| Execution | [`cove-e.md`](03-execution/cove-e.md), [`cove-h.md`](03-execution/cove-h.md), [`mount-read-protocol.md`](03-execution/mount-read-protocol.md) |
| Interop | [`arrow.md`](04-interop/arrow.md), [`lakehouse.md`](04-interop/lakehouse.md), [`parquet-conversion.md`](04-interop/parquet-conversion.md) |
| Object | [`cove-o.md`](05-object/cove-o.md) |
| Layout and runtime | [`layout-and-io.md`](06-layout-runtime/layout-and-io.md), [`covx.md`](06-layout-runtime/covx.md), [`covm-cache.md`](06-layout-runtime/covm-cache.md) |
| Mapping | [`cove-map.md`](07-mapping/cove-map.md) |
| Conformance | [`profile-capability.md`](08-conformance/profile-capability.md), [`writer-profiles.md`](08-conformance/writer-profiles.md), [`validation-model.md`](08-conformance/validation-model.md), [`conformance-suite.md`](08-conformance/conformance-suite.md), [`utilities-governance.md`](08-conformance/utilities-governance.md) |
| AI extension | [`cove-ai.md`](09-ai/cove-ai.md), [`cove-map-ai.md`](09-ai/cove-map-ai.md), [`cove-chunk.md`](09-ai/cove-chunk.md), [`cove-tok.md`](09-ai/cove-tok.md), [`cove-vec.md`](09-ai/cove-vec.md), [`cove-mmseq.md`](09-ai/cove-mmseq.md), [`cove-train.md`](09-ai/cove-train.md), [`coveql-ai.md`](09-ai/coveql-ai.md) |
| Query discovery | [`cove-query-discovery.md`](10-query-discovery/cove-query-discovery.md) |
