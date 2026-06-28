# COVE Standards Suite v2.0 Specification

Status: split normative specification index

The COVE v2.0 specification is now organized as a foldered standards-suite tree
under [`spec/`](./spec/). This root document remains the stable entrypoint for
existing links and tooling. The split is editorial only: section numbers,
heading titles, and normative meaning are preserved in the split documents.

Start with [`spec/README.md`](./spec/README.md) for a beginner-oriented reading
map.

## Normative Specification Documents

| Area | Documents |
| --- | --- |
| Overview | [`00-overview/standards-suite.md`](./spec/00-overview/standards-suite.md), [`00-overview/core-concepts.md`](./spec/00-overview/core-concepts.md) |
| Core | [`01-core/wire-rules.md`](./spec/01-core/wire-rules.md), [`01-core/file-layout.md`](./spec/01-core/file-layout.md), [`01-core/dictionaries-values.md`](./spec/01-core/dictionaries-values.md), [`01-core/encoded-arrays-codecs.md`](./spec/01-core/encoded-arrays-codecs.md), [`01-core/extensions.md`](./spec/01-core/extensions.md), [`01-core/redaction-digests.md`](./spec/01-core/redaction-digests.md), [`01-core/failure-compatibility.md`](./spec/01-core/failure-compatibility.md) |
| Table | [`02-table/cove-t-storage.md`](./spec/02-table/cove-t-storage.md), [`02-table/predicates-coverage.md`](./spec/02-table/predicates-coverage.md), [`02-table/table-indexes.md`](./spec/02-table/table-indexes.md), [`02-table/nested-sort-rows.md`](./spec/02-table/nested-sort-rows.md) |
| Execution | [`03-execution/cove-e.md`](./spec/03-execution/cove-e.md), [`03-execution/cove-h.md`](./spec/03-execution/cove-h.md), [`03-execution/mount-read-protocol.md`](./spec/03-execution/mount-read-protocol.md) |
| Interop | [`04-interop/arrow.md`](./spec/04-interop/arrow.md), [`04-interop/lakehouse.md`](./spec/04-interop/lakehouse.md), [`04-interop/parquet-conversion.md`](./spec/04-interop/parquet-conversion.md) |
| Object | [`05-object/cove-o.md`](./spec/05-object/cove-o.md) |
| Layout and runtime | [`06-layout-runtime/layout-and-io.md`](./spec/06-layout-runtime/layout-and-io.md), [`06-layout-runtime/covx.md`](./spec/06-layout-runtime/covx.md), [`06-layout-runtime/covm-cache.md`](./spec/06-layout-runtime/covm-cache.md) |
| Mapping | [`07-mapping/cove-map.md`](./spec/07-mapping/cove-map.md) |
| Conformance | [`08-conformance/profile-capability.md`](./spec/08-conformance/profile-capability.md), [`08-conformance/writer-profiles.md`](./spec/08-conformance/writer-profiles.md), [`08-conformance/validation-model.md`](./spec/08-conformance/validation-model.md), [`08-conformance/conformance-suite.md`](./spec/08-conformance/conformance-suite.md), [`08-conformance/utilities-governance.md`](./spec/08-conformance/utilities-governance.md) |

## Compatibility Notes

Existing references to numbered sections such as `§24`, `§73`, and `§79.1`
remain valid. The section text now lives in the split documents above, and
conformance tooling scans the split specification tree when validating section
references.
