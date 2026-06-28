# CoveQL Quickstart

This quickstart uses the beginner `cove` command. It is a friendly umbrella CLI
for discovering and querying COVE artifacts without writing Rust.

## Choose A Path

COVE has two main read surfaces. If your data is already table-shaped and
should remain table-shaped, start with a COVE-T table such as
`examples/coveql/events.cove`. If repeated real-world entities, provenance,
associations, or deterministic projections matter, start with a mapped COVE-O
file such as `examples/coveql/people.cove` or the Customer 360 showcase.

The same `cove inspect`, `cove doctor`, and `cove query` workflow works for
both paths. Acceleration sidecars are optional: they may help readers skip work,
but materialized CoveQL remains the semantic authority.

## Run The CLI

If `cove` is installed:

```bash
cove --help
```

When developing from the repository, run the same commands through Cargo from
the repository root, for example `cargo run -p cove-cli -- --help`.

## Find A Starting Point

Ask the CLI for copy-paste examples:

```bash
cove examples
```

For one specific file, use `doctor`. It combines queryability, performance
sidecar status, suggested queries, and next-step commands:

```bash
cove doctor examples/coveql/people.cove
cove doctor --json examples/coveql/events.cove
```

## Inspect A File

Start with the checked-in samples:

```bash
cove inspect examples/coveql/people.cove --queries
cove inspect examples/coveql/events.cove --queries
```

`inspect` detects the artifact type, shows whether it is queryable, lists
objects/tables/projections/evidence surfaces, and prints copy-paste CoveQL
queries from the file metadata.

Sidecars are first-class too:

```bash
cove inspect examples/coveql/people.covemap --queries
```

That file is COVE-MAP metadata, not row data, so `cove` explains how to use it
with a related data file instead of pretending it can be scanned as rows.

## Build A Mapped Bundle

Use `cove map build` when you have a COVE-MAP file and source rows and want a
single adoption-ready output directory:

```bash
cove map build --out-dir target/people-map-build --force \
  examples/coveql/people.covemap examples/coveql/people.jsonl
```

The bundle writes a mapped COVE-O object, `map-build-report.json`,
`map-build-manifest.json`, and a README with suggested next commands. When the
mapping declares COVE-T projection output, the command also writes
`projections/*.cove` table baselines.

## Resolver-Backed Entity Resolution

COVE-MAP can resolve messy source labels through deterministic resolver
catalogs. The supported workflow is explicit: import curated aliases, generate
candidate evidence, review candidate matches, rebuild, and verify replay. Fuzzy
or candidate-only matches do not merge objects unless a reviewed decision or
authoritative resolver path permits that merge.

Generate candidate matches from a mapping and source rows:

```bash
cove map candidates --out target/company-candidates.json \
  company.covemap suppliers.jsonl invoices.jsonl crm.jsonl
```

Turn candidates into a review worklist, add reviewed decisions to the
`reviewed_decisions` array, then import those decisions into a new mapping:

```bash
cove map review --out target/company-reviewed.json \
  target/company-candidates.json

cove map review import company.covemap target/company-reviewed.json \
  --out company-reviewed.covemap
```

Curated alias catalogs can be imported from CSV and bound to a resolver ID:

```bash
cove map aliases import company-reviewed.covemap aliases.csv \
  --catalog-id company_aliases \
  --resolver-id company_name_resolver \
  --out company-with-resolution.covemap
```

After building, verify the conversion report against the mapping, source
fingerprints, resolver/catalog/pipeline digests, and reviewed-decision digests:

```bash
cove map build --out-dir target/company-build --force \
  company-with-resolution.covemap suppliers.jsonl invoices.jsonl crm.jsonl

cove map replay verify company-with-resolution.covemap \
  target/company-build/map-build-report.json
```

Resolution evidence is exposed through the ordinary COVE-O evidence and
projection surfaces, so `cove query` and DataFusion can read the materialized
object snapshot without needing a live resolver service.

## Query A COVE-T Table

`events.cove` is a tiny COVE-T table file:

```bash
cove query examples/coveql/events.cove \
  'table(events).where(score >= 20).select(id, score)'
```

By default, results are printed as a terminal table.

Export formats are available for scripts:

```bash
cove query examples/coveql/events.cove \
  --format jsonl \
  'table(events).select(id, score)'

cove query examples/coveql/events.cove \
  --format csv \
  'table(events).select(id, score)'
```

Use `--take` for quick sampling when the query has no explicit `.take(...)`:

```bash
cove query examples/coveql/events.cove \
  --take 2 \
  'table(events).select(id, score)'
```

For longer queries, keep the CoveQL text in a file:

```bash
printf 'table(events).where(score >= 20).select(id, score)\n' > /tmp/events.coveql
cove query --query-file /tmp/events.coveql examples/coveql/events.cove
```

Or pipe a query through stdin:

```bash
printf 'table(events).select(id, score)\n' | \
  cove query --query-file - examples/coveql/events.cove
```

If table cells are too wide or too narrow for your terminal, adjust the display
width:

```bash
cove query examples/coveql/events.cove \
  --max-cell-width 16 \
  'table(events).select(id, score)'
```

## Query A COVE-O Object File

`people.cove` is a mapped COVE-O object sample generated from
`people.jsonl` and `people.covemap`.

Object roots expose semantic object rows:

```bash
cove query examples/coveql/people.cove \
  'object(Person).take(5)'
```

For flat beginner-friendly columns, use the mapped table that `inspect` shows.
COVE-O projection output tables are exposed through the same `table(...)`
surface as COVE-T:

```bash
cove query examples/coveql/people.cove \
  'table(people).where(score >= 20).select(score, status, nickname)'
```

## SQL-Like CoveQL Methods

The same table-shaped CoveQL methods work for COVE-T tables and COVE-O mapped
tables:

```bash
cove query examples/coveql/events.cove \
  'table(events).select(rows: count(*), total: sum(score), average: avg(score))'

cove query examples/coveql/people.cove \
  'table(people).orderBy(score, desc).select(score, status).take(2)'

cove query examples/coveql/people.cove \
  'table(people).window(orderBy: score).select(score, rn: row_number()).take(3)'
```

CoveQL also supports relational methods such as `join`, `semiJoin`, `antiJoin`,
`union`, `intersect`, `except`, scoped `with(...)` bindings, finite
`withRecursive(...)`, grouping, aggregates, and window functions. These are
executed by the materialized CoveQL authority unless an optimized path proves
equivalence.

## Performance Sidecars

By default, `cove query` uses safe-auto execution: it looks for validated
acceleration sidecars and falls back to the materialized authority when proof is
missing. Generate sibling sidecars without rewriting the source file:

```bash
cove optimize examples/coveql/events.cove
cove inspect --performance examples/coveql/events.cove
cove query --perf-report examples/coveql/events.cove \
  'table(events).where(score >= 20).select(id, score)'
```

The optimizer writes a `.covperf.json` discovery manifest plus applicable
sidecars such as `.covi`, `.covx`, `.covee`, COVE-L split/layout files, and
structured skipped reasons for metadata that is not applicable to the source.
Sidecars are acceleration metadata, not portable logical truth.

The CLI can also run the physical/kernel planner explicitly:

```bash
cove query examples/coveql/events.cove \
  --engine physical \
  'table(events).where(score >= 20).select(id, score)'

cove query examples/coveql/people.cove \
  --engine compare \
  'table(people).where(score >= 20).select(score).take(2)'
```

Use `--engine compare` when you want optimized execution to compare against the
materialized authority. Use `--strict-performance` to reject instead of falling
back when no validated acceleration metadata is available. Physical sidecars can
be supplied with flags such as
`--covi`, `--covx`, `--coverage-plan`, `--coverage-proof`, `--coverage-set`,
`--layout-plan`, `--zero-copy-buffer-map`, and `--cove-e`.

## Delta Snapshots

Delta-bearing datasets are selected through COVM manifests. A selected snapshot
is an immutable base `.cove` plus an ordered `.covedelta` chain; the CLI never
discovers undeclared deltas by scanning a directory, and a delta-bearing
snapshot must not silently fall back to base-only results.

Inspect and validate the selected chain:

```bash
cove delta chain inspect dataset.covm
cove delta chain validate dataset.covm --dataset bundle
cove delta chain graph dataset.covm --dataset bundle --format text
```

Plan an as-of read, then query the selected snapshot directly:

```bash
cove delta chain plan dataset.covm --dataset bundle \
  --as-of-csn 100 --json

cove query dataset.covm --dataset bundle --as-of-csn 100 --delta-plan \
  'object(Thing).take(10)'
```

The same selected snapshot can be exported, indexed, reconstructed, or
compacted:

```bash
cove export arrow --query 'object(Thing).take(10)' \
  --dataset bundle --as-of-csn 100 --delta-plan-json \
  dataset.covm snapshot.arrow

cove sidecar build covi --snapshot dataset.covm --dataset bundle \
  --out snapshot.covi --object-properties

cove delta reconstruct dataset.covm --dataset bundle --out snapshot.cove
cove delta compact dataset.covm --dataset bundle --out compacted.cove \
  --publish-covm compacted.covm
```

COVE-MAP can also produce semantic object deltas from source rows:

```bash
cove map delta build --base dataset.covm --dataset bundle \
  --mapping mapping.covemap --out delta-0002.covedelta sources.jsonl
```

Ordinary supported catalog patches are additive. Breaking catalog changes,
object type reinterpretation, or resolver behavior changes should publish a new
effective semantic-map fingerprint and usually a new base or schema branch.

## Query External Tables

For local data that is not inside a COVE file yet, register a file-backed
external table. The CLI accepts CSV, JSON arrays, and JSONL/NDJSON rows:

```bash
printf 'id,score\n1,10\n2,20\n3,30\n' > /tmp/people.csv
cove query \
  --external-table people=/tmp/people.csv \
  'table(people).where(score >= 20).select(id, score)'
```

External tables can also join with COVE-T or mapped COVE-O tables:

```bash
cove query examples/coveql/events.cove \
  --external-table weights=/tmp/weights.jsonl \
  'table(events) as e.join(table(weights) as w, on: e.id == w.id).select(id: e.id, score: e.score, weight: w.weight)'
```

This is the CLI form of CoveQL `ExternalRegisteredTable`: it gives terminal
users a file-backed provider without requiring custom Rust provider code.

Graph roots and graph algorithms work over COVE-O graph-shaped object and
association surfaces:

```bash
cove query examples/coveql/people.cove \
  'node(Person) as p.connectedComponents().degree(kind: total).select(id: p.goid, component_id, degree).take(3)'
```

Variable-length traversal is budgeted and must be enabled explicitly:

```bash
cove query graph.cove \
  --enable-graph-traversal --max-graph-depth 4 --max-graph-paths 1000 \
  'node(Person) as p.traverse(out(edge(Knows)), min: 1, max: 4, distinct: path).select(p.goid)'
```

Evidence rows are available when the file includes COVE-MAP evidence metadata:

```bash
cove query examples/coveql/people.cove \
  'evidence(Person, grain: object).select(source_id, source_row_identity).take(5)'
```

## Explain A Query

Ask CoveQL why a query planned the way it did:

```bash
cove query examples/coveql/events.cove \
  --explain coded \
  'table(events).where(score >= 20).select(id, score)'
```

Explain output reports the mode, operation, fingerprints, pushdown status,
fallbacks, redactions, and diagnostics. The CLI keeps explain disclosure public
unless you explicitly request a higher mode such as `coded`.

## Common Errors

- `E_UNKNOWN_TABLE_SURFACE`: the table name is not registered in this file.
  Run `inspect --queries` and copy the suggested `table(...)` query.
- `E_UNKNOWN_OBJECT_TYPE`: the object type is not present in this COVE-O file.
  Run `inspect` to see available object types.
- Sidecar guidance: files like COVE-MAP, COVE-I, COVE-L, and COVE-COVERAGE are
  metadata. Query the related row data file and pass sidecars with the relevant
  option, such as `--mapping`.
- Unsupported COVE-T values: beginner table readback decodes primitive safe
  values first. Nested or encoded values that need additional contracts fail
  closed with a diagnostic rather than exposing unsafe data.

## Regenerate The Samples

The checked-in samples are generated by a Cargo example:

```bash
cargo run -p cove-cli --example generate_beginner_samples -- examples/coveql
```
