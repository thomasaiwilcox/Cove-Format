# COVE-O Overlap Benchmark Results

Status: current `cove-bench` CI-profile synthetic overlap results

Audience: maintainers and evaluators asking when mapped COVE-O storage becomes
compelling versus source-shaped Parquet bundles.

## Summary

The current benchmark evidence is mixed in the useful way:

- small low-overlap proof fixtures remain overhead-heavy;
- the COVE-O object gets smaller as repeated logical entities increase;
- the full adoption bundle only beats source-shaped Parquet at high overlap,
  because the bundle also includes COVE-T projections, COVE-I sidecars, COVM,
  reports, manifests, and README artifacts;
- COVE-O should not be described as "always smaller than Parquet."

The strongest current claim is narrower and defensible:

> COVE-O is compelling when many source-shaped tables repeat the same logical
> object/property state and users also need deterministic mapping evidence,
> projection readback, validation, and provenance.

## Regenerate

From the repository root:

```sh
cargo run -p cove-bench -- check
```

The generated report is written to:

```text
target/cove-bench/ci/report.json
target/cove-bench/ci/report.md
```

## What Is Compared

The overlap benchmarks compare:

- **Source Parquet bundle**: duplicate source-shaped Parquet files, one per
  source table.
- **Unique Parquet baseline**: one Parquet file containing each logical entity
  once, used as a lower-bound reference for deduplicated table storage.
- **COVE-O**: the generated mapped object archive.
- **Full bundle**: COVE-O plus projections, indexes, COVM, manifest, report,
  README, and related adoption artifacts.

This is intentionally not a claim about all Parquet layouts. A production
lakehouse could also deduplicate or remodel data into normalized tables. The
benchmark isolates COVE-O's object/property deduplication behavior against
duplicated source-shaped tables.

## Partial Overlap

Eight source tables, 512 rows per table. Rows outside the shared fraction are
source-specific entities.

| Overlap | Unique objects | Dedupe ratio | Source Parquet | COVE-O | Full bundle | COVE-O / Parquet | Bundle / Parquet |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0% | 4,096 | 1.00x | 2,943,092 | 1,406,082 | 10,656,724 | 0.478x | 3.621x |
| 25% | 3,200 | 1.28x | 2,939,309 | 1,139,634 | 8,408,851 | 0.388x | 2.861x |
| 50% | 2,304 | 1.78x | 2,938,408 | 860,965 | 6,149,791 | 0.293x | 2.093x |
| 75% | 1,408 | 2.91x | 2,939,528 | 582,543 | 3,888,915 | 0.198x | 1.323x |
| 100% | 512 | 8.00x | 2,942,728 | 289,487 | 1,558,769 | 0.098x | 0.530x |

Interpretation:

- The COVE-O object improves smoothly as overlap rises.
- On this wide/string-heavy synthetic fixture, the COVE-O object is smaller
  than the duplicated source Parquet bundle even at 0% overlap.
- The full adoption bundle remains larger than source Parquet until very high
  overlap, crossing below source Parquet between 75% and 100% overlap.
- Bundle overhead is therefore the key target if the goal is a stronger
  storage-size story, especially for moderate overlap.

## Maximum Overlap Scale

All source tables contain the same logical object/property state. This is a
best-case deduplication curve, not a general workload claim.

| Case | Tables | Rows | Source Parquet | COVE-O | Full bundle | COVE-O / Parquet | Bundle / Parquet |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 table | 1 | 512 | 367,841 | 176,452 | 1,437,605 | 0.480x | 3.908x |
| 2 tables | 2 | 512 | 735,682 | 216,518 | 1,479,503 | 0.294x | 2.011x |
| 4 tables | 4 | 512 | 1,471,364 | 245,701 | 1,510,774 | 0.167x | 1.027x |
| 8 tables | 8 | 512 | 2,942,728 | 289,487 | 1,558,729 | 0.098x | 0.530x |
| 8 tables large | 8 | 2,048 | 11,656,016 | 1,098,691 | 5,890,199 | 0.094x | 0.505x |

Interpretation:

- The object archive scales with unique logical state much more than with the
  number of duplicate source tables.
- The full bundle has a mostly fixed overhead component plus projection/index
  overhead, so it needs enough duplicated source state before it wins on total
  bytes.
- At eight fully overlapping source tables, both the COVE-O object and the full
  bundle are smaller than the duplicated source Parquet bundle.

## Practical Takeaway

COVE-O's current size story is strongest for:

- customer/account/product/entity-360 archives;
- audit and provenance-heavy datasets;
- many source tables with repeated IDs and repeated wide string attributes;
- cases where users need deterministic replay, evidence, and projection
  readback rather than only compact columnar analytics.

The current size story is weakest for:

- single-table data;
- low-overlap multi-table data;
- small fixtures where fixed metadata dominates;
- workflows that only need ordinary analytics and no mapping/provenance bundle.

The next engineering target is to reduce full-bundle overhead or make bundle
components easier to publish separately, while preserving COVE-O's useful
semantic evidence and validation guarantees.
