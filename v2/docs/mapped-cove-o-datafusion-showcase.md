# Mapped COVE-O DataFusion Showcase

This repository now includes a full end-to-end showcase proving that COVE can:

1. ingest multiple tabular source tables in different file formats,
2. compact them into one mapped `COVE-O` object file,
3. preserve deterministic provenance and readback metadata, and
4. expose that object file back to SQL through the DataFusion adapter.

The goal is not just "convert a table into another file". The goal is:

**multiple source tables -> one canonical object surface -> SQL tables plus lineage**

That is the part worth showing off.

## What the showcase proves

The integration test
`mapped_cove_o_showcase_spans_multiple_sources_and_projections_in_datafusion`
in
[`v2/crates/cove-datafusion/tests/native_single_file.rs`](../crates/cove-datafusion/tests/native_single_file.rs)
builds a single mapped `COVE-O` file from three source tables:

- `crm.csv`
- `directory.parquet`
- `subscription.csv`

All three sources describe the same canonical `Person` objects. The mapping uses
deterministic identity rules and source-priority conflict handling so one
canonical name wins, while evidence rows still preserve where each input fact
came from.

The test then registers the resulting mapped object file in DataFusion with:

- `people_projection` — canonical person rows
- `evidence_projection` — provenance rows keyed back to canonical objects

That lets SQL query both the compacted semantic surface and its lineage from the
same persisted `COVE-O` file.

## Why this matters

This is the core value proposition behind mapped `COVE-O`:

- **Deduplicate semantics, not just bytes.** Multiple origin tables can collapse
  into one canonical object identity.
- **Keep deterministic lineage.** The system does not throw away where a value
  came from.
- **Read back as tables.** Engines like DataFusion can query the mapped file as
  normal SQL tables instead of needing a custom object API.
- **Expose provenance as first-class query data.** Canonical objects and origin
  evidence can be joined in SQL.

That combination is the interesting demo story:

> one object file that is lighter and more canonical than the original table
> sprawl, but still understandable and queryable as tables with provenance.

## What the test does

At a high level the showcase test does this:

```text
crm.csv + directory.parquet + subscription.csv
  -> cove-map materialization
  -> one mapped COVE-O file
  -> register_cove_o_projections(...)
  -> DataFusion SQL over canonical rows and provenance rows
```

The canonical query proves that one canonical person row comes back from the
mapped file:

```sql
SELECT name
FROM demo__people_projection
ORDER BY name;
```

Result:

```text
+-------+
| name  |
+-------+
| Ada   |
| Linus |
+-------+
```

The provenance query proves that the object file still knows which source tables
contributed evidence:

```sql
SELECT source_id, COUNT(DISTINCT source_row_identity) AS evidence_count
FROM demo__evidence_projection
GROUP BY source_id
ORDER BY source_id;
```

Result:

```text
+--------------+----------------+
| source_id    | evidence_count |
+--------------+----------------+
| crm          | 2              |
| directory    | 2              |
| subscription | 2              |
+--------------+----------------+
```

The join query is the real showcase piece:

```sql
SELECT DISTINCT p.name, e.source_id, e.source_row_identity
FROM demo__people_projection p
JOIN demo__evidence_projection e
  ON p.person_goid = e.output_object_id
ORDER BY p.name, e.source_id, e.source_row_identity;
```

Result:

```text
+-------+--------------+---------------------+
| name  | source_id    | source_row_identity |
+-------+--------------+---------------------+
| Ada   | crm          | crm:0               |
| Ada   | directory    | directory:0         |
| Ada   | subscription | subscription:0      |
| Linus | crm          | crm:1               |
| Linus | directory    | directory:1         |
| Linus | subscription | subscription:1      |
+-------+--------------+---------------------+
```

That is the key benefit in one screen:

- one canonical `Person` row,
- multiple origin tables,
- deterministic readback,
- lineage preserved and queryable in SQL.

## How to run it

From the `v2/` workspace:

```sh
cargo test -p cove-datafusion --test native_single_file \
  mapped_cove_o_showcase_spans_multiple_sources_and_projections_in_datafusion -- --exact
```

For the broader mapped-readback/DataFusion coverage:

```sh
cargo test -p cove-datafusion --test native_single_file
```

## Benchmark snapshot

The showcase now has a matching `m6` Criterion track that compares:

- mapped `COVE-O` queried through `register_cove_o_projection(...)`
- the same projection exported as `COVE-T`
- the same projection exported as Parquet

Run it from `v2/` with:

```sh
cargo bench -p cove-datafusion --features parquet-compare --bench m6 mapped_showcase \
  -- --sample-size 10 --measurement-time 1 --warm-up-time 0.5
```

Current local results:

| Track | mapped `COVE-O` | projected `COVE-T` | Parquet |
| --- | ---: | ---: | ---: |
| `mapped_showcase_people_projection` | 188.20-192.77 us | 124.29-131.22 us | 130.18-131.38 us |
| `mapped_showcase_evidence_aggregate` | 710.88-729.13 us | 588.17-600.80 us | 676.92-727.67 us |
| `mapped_showcase_people_evidence_join` | 887.42-928.92 us | 663.92-684.46 us | 870.47-886.91 us |

What this means today:

- the **mapped readback path is correct and benchmarkable**
- the current **projected `COVE-T` baseline is faster** than the mapped `COVE-O`
  path on these showcase queries
- Parquet is also still faster than the mapped `COVE-O` path on the current
  people scan, evidence aggregate, and join
- the mapped-path work **did** materially improve the broad showcase timings
  versus the original baseline: the people projection is now about **50%**
  faster, the evidence aggregate about **20-25%** faster, and the join about
  **25-30%** faster depending on which ends of the confidence intervals you
  compare
- those wins line up with the current implementation changes: the DataFusion
  path now reuses the already-loaded projection catalog, avoids redundant
  projection-row cloning/sorting for common temporal modes, and reconstructs
  latest/as-of projection rows directly from temporally ordered `ProjectionRow`
  values instead of round-tripping through `CoveObjectState`; the latest pass
  also keeps mapped evidence entries structured longer, can build Arrow
  evidence batches directly for simple `evidence.*` projections, and now skips
  rebuilding per-cell `Value` wrappers for direct evidence UTF-8/UUID columns;
  the newest pass does the same for simple one-row-per-object Arrow projections
  like the showcase people side
- that means the remaining dominant costs now look less like
  catalog/state/map-section reconstruction and more like downstream execution
  over the emitted evidence batches, although the join has continued to come
  down as both the evidence and people projection builders get cheaper

That matches the current implementation shape: mapped `COVE-O` DataFusion
execution now runs through a dedicated `CoveProjectionExec` that range-reads
header/footer/metadata/temporal sections, rebuilds a smaller valid in-memory
COVE file, emits `RecordBatch` output directly, and can push exact scalar
filters into mapped projection materialization. The provider now also trims
temporal sections by requested object types and recursively pulls in only the
extra segments needed to satisfy `prev_ref` self-containment. On top of that,
the projection layer now avoids a second projection-catalog parse,
reconstructs latest/as-of rows directly from sorted projection rows, and can
skip generic row-map assembly for simple Arrow evidence projections. The latest
builder passes also emit direct UTF-8/UUID evidence arrays without first
reconstructing generic JSON cells and can batch simple object projections
straight from `ProjectionRow` values. So these numbers measure a meaningfully
improved mapped path, but still not the ceiling for deeper selective readback
and lower-cost downstream join/aggregate execution on evidence-heavy queries.

## Blob/object-store read snapshot

The repository also now emits a projected-table object-store comparison through
`cove-bench`:

```sh
cargo run -p cove-bench -- check
```

The generated CI corpus report (`target/cove-bench/ci/report.json`) includes
`semantic_projection_object_store_compare`, which now compares the simple public
semantic-mapping corpus as:

- mapped `COVE-O`
- projected `COVE-T`
- Parquet

Current local report values:

| Metric | mapped `COVE-O` | projected `COVE-T` | Parquet |
| --- | ---: | ---: | ---: |
| file size | 837,512 bytes | 16,550 bytes | 16,622 bytes |
| cold bytes requested | 16,384 | 16,467 | 16,503 |
| cold range GETs | 3 | 2 | 2 |
| warm bytes requested | 16,384 | 16,467 | 16,503 |
| warm range GETs | 3 | 2 | 2 |

So on this small deterministic semantic-mapping corpus:

- projected `COVE-T` is still the compact table-shaped result: **72 bytes
  smaller** than Parquet
- the mapped `COVE-O` artifact is **much larger** on this simple single-source
  fixture, because it carries the richer object/lineage representation rather
  than just the projected table
- the offline harness touched **3** coalesced cold/warm ranges for mapped
  `COVE-O` versus **2** for projected `COVE-T` and Parquet

This is still a **corpus-artifact** result, not a live-cloud claim. The
deterministic harness samples fixed byte ranges rather than executing a mapped
query plan against an object store, so the small byte-request deltas here are
far less important than the representation-size difference. On this fixture, the
storage-friendly readback story is clearly the projected table output, not the
raw mapped object.

The repository now also includes a **richer multi-source showcase bundle**
benchmark derived from the real demo semantics: one mapped `COVE-O` file versus
the bundled projected outputs for:

- `people_projection`
- `evidence_projection`

Current local report values for
`semantic_showcase_bundle_object_store_compare`:

| Metric | mapped `COVE-O` | projected `COVE-T` bundle | Parquet bundle |
| --- | ---: | ---: | ---: |
| total file size | 20,948 bytes | 3,347 bytes | 2,187 bytes |
| cold bytes requested | 16,384 | 3,347 | 2,187 |
| cold range GETs | 3 | 2 | 2 |
| warm bytes requested | 16,384 | 3,347 | 2,187 |
| warm range GETs | 3 | 2 | 2 |

That is a fairer comparison than the simple one-table corpus because the mapped
file is now carrying both canonical rows and lineage/evidence readback surfaces.
But on this tiny two-person showcase, mapped `COVE-O` is **still materially
larger** than the projected-table bundles. So the current evidence says the
showcase semantics alone are not yet enough to overcome object-model overhead at
small scale.

## What this demo is, and what it is not

This showcase proves a real and important milestone:

- mapped `COVE-O` can be produced from multiple source tables,
- registered directly in DataFusion,
- and queried as canonical rows plus provenance tables.

It is intentionally focused on **correctness and demo clarity**. The current
DataFusion projection provider is now a real execution plan with range-reader
metrics, but it still rebuilds the requested projection by materializing
`cove-map` Arrow output inside execution rather than doing deeper pushdown-aware
scan planning. That is an optimization target, not a blocker for the showcase.

## Recommended demo framing

If you want the short pitch:

> COVE can compact multiple source tables into one canonical object file, then
> let DataFusion query both the canonical table view and the original lineage
> back out of that same file.

If you want the slightly longer pitch:

> Instead of keeping the same repeated entities copied across CRM, directory,
> subscriptions, and similar source tables, mapped `COVE-O` lets us store one
> canonical object identity with deterministic mapping and preserved evidence.
> Then DataFusion can query that file as SQL tables again — not only the
> canonical table, but also the provenance surface that explains where each
> object came from.
