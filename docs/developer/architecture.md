# COVE Developer Architecture

COVE is organized as layers. New application code should start high and move
down only when it needs a specialist contract.

## Layer 1: Facades

- `cove`: canonical app-facing facade for validation, inspection, reading,
  writing, conversion, query, explain, and engine registration.
- `cove-reader`: narrow read/validate facade.
- `cove-writer`: narrow writer facade.
- `cove-engine`: runtime and DataFusion-facing facade.

These crates should expose typed options, typed reports, and stable errors.
They are the preferred import surface for examples and downstream users.
See [`crate-ownership.md`](./crate-ownership.md) for the crate-by-crate owner
map.

## Layer 2: Domain Libraries

- `cove-core`: wire format, section parsing, validation, profiles, and core
  model types.
- `coveql`: parser, resolver, planning, execution, explain output, and query
  contracts.
- `cove-datafusion`: DataFusion registration, planning, decoding, pushdown, and
  physical execution integration.
- `cove-map`: COVE-MAP build, verification, replay, projection, and mapping
  APIs.
- `cove-arrow`, `cove-convert`, `cove-index`, `cove-layout`, `cove-runtime`,
  `cove-coverage`, and `cove-cache`: focused interop or sidecar domains.

Domain crates own reusable behavior. They should not depend on CLI formatting
or terminal output.

## Layer 3: Tools and Evidence

- `cove-cli`: terminal entry point for users and release gates.
- `cove-conformance`: corpus runner and generated capability evidence.
- `cove-fuzz`: deterministic fuzz and mutation harness.
- `cove-bench`: benchmark corpus and reporting tools.

Tools can print to stdout/stderr. When tool behavior becomes useful to other
software, extract a typed workflow API into the owning facade or domain crate
and leave the CLI as an adapter.

## Authority Model

Authoritative data and validated metadata define truth. Optimizations, indexes,
sidecars, zero-copy paths, and engine pushdown may accelerate reads only when
their contracts prove equivalence to the semantic baseline. Otherwise they must
fall back or reject with structured diagnostics.

## CLI Boundary

The CLI should:

- parse flags and arguments;
- choose terminal-friendly defaults;
- call typed library workflows;
- format stdout/stderr and exit codes.

The CLI should not be the only place where query, explain, conversion, sidecar,
delta, or inspection workflows can be used. Reusable logic belongs in `cove` or
the capability crate that owns the behavior.
