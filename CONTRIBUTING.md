# Contributing to COVE

This repository is both a standards-suite prototype and a Rust reference
implementation. The safest workflow is to pick the right layer, run the
smallest relevant gate first, then run the broader gate before
publication-sensitive changes.

## Developer Entry Points

- Use the `cove` crate for application-level Rust code: validate, inspect,
  read, write, convert, query, and explain.
- Use `cove-reader`, `cove-writer`, and `cove-engine` when a narrower facade is
  clearer for the caller.
- Use `cove-core` only for wire format, validation, section parsing, and core
  data model work.
- Use `coveql`, `cove-datafusion`, `cove-map`, and sidecar crates directly only
  when changing those specialist domains.
- Keep `cove-cli` as terminal UX: argument parsing, help text, stdout/stderr
  formatting, and exit behavior.

## Common Commands

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
sh scripts/release-gates.sh --quick
```

Use the full gate before changes that affect format behavior, conformance,
query execution, conversion, or release evidence:

```sh
sh scripts/release-gates.sh --full
```

## Conformance and Generated Evidence

Run these after changing spec-covered behavior, corpus generation, validators,
or capability metadata:

```sh
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```

The minimal reader subset is useful for fast independent-reader checks:

```sh
cargo run -p cove-conformance --bin cove-conformance -- \
  conformance/ --manifest conformance/minimal-reader-manifest.jsonl
```

## Boundary Rules

- `cove-core` must not depend on Arrow, Parquet, DataFusion, or facade crates.
- DataFusion imports belong in `cove-datafusion` adapter modules.
- Reusable workflows should return typed results and diagnostics, not write to
  stdout or stderr.
- CLI modules may format terminal output, but domain crates should expose
  structured APIs that the CLI calls.

The boundary script enforces the highest-risk rules:

```sh
sh scripts/check-cove-boundaries.sh
```

Large modules are not automatically rejected, but contributors should check the
advisory report before adding more code to already-large files:

```sh
sh scripts/module-size-report.sh
```
