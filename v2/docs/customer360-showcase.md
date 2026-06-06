# Customer 360 Data-Science Showcase

The Customer 360 showcase is the main approachable semantic-archive demo for
CoveQL and the unified `cove` CLI. It generates deterministic CRM, support,
billing, and event sources, writes a reconciled canonical customer readback
source, and builds a queryable COVE-O archive with canonical customers,
evidence, projected COVE-T tables, and Parquet baselines.

This is the COVE-O / COVE-MAP path: the original data starts as ordinary tables,
but the durable meaning becomes customer objects, properties, associations,
evidence, and deterministic projections. The projected tables keep the workflow
familiar for SQL, pandas, Polars, Arrow, and DataFusion users.

The current runnable archive is materialized from `customers_360.jsonl` with
`customer360_readback.covemap` so CoveQL readback stays deterministic and
copy-pasteable. The CRM/support/billing files and `customer360.covemap` are
generated alongside it as the messy multi-source mapping surface and provenance
contract for the showcase.

## What This Shows

- Source tables remain visible provenance rather than disappearing behind the
  generated canonical customer readback source.
- Canonical customer rows provide the stable semantic surface for readback,
  aggregation, and joins.
- Evidence rows make lineage queryable, including which sources contributed to
  the canonical customer view.
- Projected COVE-T and Parquet outputs keep the data-science workflow practical.
- Optional acceleration sidecars can improve reads, but they remain subordinate
  to the mapped archive's logical truth.

## Why This Is A COVE-Shaped Problem

A normal table export can store reconciled customer rows. COVE adds a stronger
archive contract:

1. The canonical customer view, evidence surface, and projected tables are kept
   together.
2. The same archive can be inspected as semantic objects, provenance/evidence,
   or familiar table-shaped projections.
3. Optional sidecars can accelerate reads without becoming the source of truth.

The current showcase keeps the messy CRM/support/billing inputs visible and uses
a reconciled canonical readback source for deterministic CoveQL examples.

## Generate

From `v2/`:

```bash
cove showcase customer360 --profile quick --out examples/customer360 --force
```

For larger local runs, keep generated data under `target/`:

```bash
cove showcase customer360 --profile standard --out target/customer360-standard --force
```

The output includes:

- `crm.csv`, `support.jsonl`, and `billing.parquet` source data;
- `events.jsonl` and `events.cove` activity facts;
- `customer360.covemap` for the messy source mapping surface;
- `customers.cove`, the queryable COVE-O archive materialized from the
  reconciled canonical readback source;
- `customers_projection.cove` and `evidence_projection.cove` projected COVE-T baselines;
- matching Parquet projection baselines;
- `customer360-manifest.json` with paths, row counts, recommended queries, and benchmark IDs;
- `notebooks/customer360_analysis.py` for pandas/Polars-oriented exploration.

## Inspect And Query

```bash
cove doctor examples/customer360/customers.cove
cove inspect --queries --performance examples/customer360/customers.cove
```

Query canonical customer rows:

```bash
cove query examples/customer360/customers.cove \
  'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'
```

Inspect provenance/evidence rows:

```bash
cove query examples/customer360/customers.cove \
  'table(customer_evidence).select(source_id, source_row_identity, rule_id).take(10)'
```

Join canonical customers to generated activity facts through an external JSONL
table:

```bash
cove query examples/customer360/customers.cove \
  --external-table events=examples/customer360/events.jsonl \
  'table(customers) as c.join(table(events) as e, on: c.customer_id == e.customer_id).select(customer_id: c.customer_id, tier: c.tier, event_kind: e.event_kind, event_score: e.score).take(10)'
```

## Optimize And Compare

Generate acceleration sidecars for the event fact table:

```bash
cove optimize examples/customer360/events.cove
cove query --engine compare --perf-report examples/customer360/events.cove \
  'table(events).where(score >= 80).select(event_id, customer_id, event_kind, score)'
```

For the mapped customer archive, materialized CoveQL remains the authority. Use
`--engine compare --perf-report` to see whether available sidecars were used,
skipped, or fell back. Acceleration can make the generated showcase cheaper to
read, but it must not change the customer, evidence, or projection results.

## Notebook-Style Analysis

The generated script is intentionally a plain Python file so it can run in CI
and also be copied into a notebook:

```bash
python3 examples/customer360/notebooks/customer360_analysis.py --input-dir examples/customer360
```

It loads the generated JSONL sources, prints row-count and distribution
summaries, uses pandas when installed, and uses Polars when installed.

## Developer Setup

When the `cove` binary is not installed, run the same commands through Cargo:

```bash
cargo run -p cove-cli -- showcase customer360 --profile quick --out examples/customer360 --force
```
