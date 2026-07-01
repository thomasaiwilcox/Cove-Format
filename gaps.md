# COVE Spec Gap Audit

Audit date: 2026-07-01

Status: no open verified spec gaps remain.

Scope: `spec.md` and every markdown file under `spec/`.

Resolution summary:

- COVE-AI scoped required feature handling now defers non-artifact unknown
  required feature bits and required record kinds until the requested AI
  operation is known.
- Section-required COVE-AI checks now use operation-specific needed sections
  instead of treating every section in the same AI profile as intersecting.
- Unknown optional future COVE-AI record kinds are skipped after bounds and CRC
  validation, while required records with unassigned common flag bits reject.
- Duplicate `local_id` checks no longer over-apply to unknown records that are
  skippable or deferred by requiredness scope, while bad CRCs still reject
  before skip.
- `AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED` now rejects affected external
  assets and asset-vector bindings without digest references.
- The generated conformance corpus includes regression fixtures for the COVE-AI
  scoped-feature, scoped-record-kind, optional-record, required-record-flag, and
  external-asset digest cases.
- `cove-fuzz` now includes deterministic COVE-AI record-stream coverage for
  arbitrary unknown optional/required record kinds, CRC-before-skip rejection,
  duplicate unknown local IDs, and operation-scoped requiredness.
- `spec/09-ai/cove-ai.md` now matches COVE-VEC and the implementation by
  documenting V2 vector-binding records for current binding kinds 1 through 7.

Verification:

- `cargo test -p cove-core coveai --lib`: 122 passed.
- `cargo check --workspace`: passed.
- `cargo run -p cove-conformance --bin gen-corpus -- --check`: 679 fixtures up
  to date.
- `cargo run -p cove-conformance --bin gen-capability-matrix -- --check`: 95/95
  fully gated capabilities up to date.
- `cargo run -p cove-conformance --bin cove-conformance -- conformance/`:
  679/679 fixtures passed.
- `cargo test -p cove-fuzz`: 9 passed.
- `cargo run -p cove-fuzz --bin cove-fuzz -- smoke --mutations 8`: 534 cases,
  299 rejects, 136 accepted mutations, 0 skipped, 0 panics.
- `cargo run -p cove-fuzz --bin cove-fuzz -- parsers --mutations 16`: 1227
  cases, 781 rejects, 294 accepted mutations, 0 skipped, 0 panics.
- `cargo run -p cove-fuzz --bin cove-fuzz -- corpus conformance/manifest.jsonl --mutations 2`:
  980 cases, 800 rejects, 40 accepted mutations, 357 skipped, 0 panics.
