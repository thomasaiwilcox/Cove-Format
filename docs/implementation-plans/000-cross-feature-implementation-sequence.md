# Cross-Feature Implementation Sequence

Status: implementation plan

Derived from:

- [COVE-MAP Resolver Catalog and Entity Resolution](../proposals/covemap-entity-resolution.md)
- [COVE-O Delta Artifacts](../proposals/cove-o-delta-artifacts.md)

## Purpose

This document defines the numbered order for adding both proposed features
without losing the dependency boundary between COVE-MAP resolver semantics and
COVE-O delta publication.

The implementation plans are intentionally separate because the features have
different correctness centers:

- COVE-MAP entity resolution defines semantic truth: resolver catalogs,
  canonical digests, row-level identity authority, reviewed decisions, evidence,
  replay, and explain surfaces.
- COVE-O delta artifacts define publication mechanics: immutable overlays,
  COVM snapshot selection, chain validation, sparse temporal patches,
  compaction, and read-amplification control.

## Numbered Plan Order

Follow these plans in numeric filename order for a single AI agent or a single
implementation stream:

1. [COVE-MAP Entity Resolution Implementation Plan](./001-covemap-entity-resolution-implementation-plan.md)
2. [COVE-O Delta Artifacts Implementation Plan](./002-cove-o-delta-artifacts-implementation-plan.md)

The conservative order is deliberate. Resolver-aware deltas must bind stable
resolver section IDs, digest formulas, evidence metadata keys, row-level
outcomes, and semantic-map fingerprints. Implementing the resolver plan first
prevents delta code from inventing temporary resolver metadata that later needs
to be broken.

## Dependency Rules

1. COVE-MAP entity resolution does not require COVE-O delta artifacts.
2. The COVE-O delta MVP does not require resolver execution.
3. Resolver-aware delta evidence/projection patches must not be implemented
   until COVE-MAP entity resolution Phase 0 and Phase 1 are complete.
4. Ordinary COVE-O object truth must remain materialized COVE-O temporal
   records. Resolver metadata and delta evidence patches can support replay,
   explain, evidence readback, and planning, but cannot be the only source of
   ordinary object truth unless a later required profile explicitly grants that
   authority.
5. Any change to mapping rules, resolver behavior, alias catalog content,
   normalization pipeline versions, reviewed decisions that contribute merge
   edges, or identity-rule semantics must expose a new effective semantic-map
   fingerprint.

## Allowed Parallelism

If multiple engineers or agents are working in parallel, the only safe
parallelism before full entity-resolution completion is:

1. Complete COVE-MAP Phase 0 and Phase 1 first.
2. Then implement the core COVE-O delta MVP without resolver-specific patch
   sections.
3. Continue COVE-MAP Phases 2 through 5 in parallel only if the delta work uses
   resolver metadata as opaque semantic-map fingerprints and does not implement
   `DELTA_FEATURE_MAP_EVIDENCE_PATCH`, `DELTA_EVIDENCE_PATCH`, or resolver-aware
   projection patches yet.

For a single ordered implementation, do not use this shortcut. Finish plan 001,
then plan 002.

## Completion Gates

Both features are complete only when all of the following are true:

1. Every numbered phase in plan 001 has its acceptance criteria satisfied.
2. Every numbered phase in plan 002 has its acceptance criteria satisfied.
3. Each plan's proposal-coverage matrix has an implementation evidence entry
   for every proposal heading.
4. All conformance fixtures listed in the plans exist and are exercised by the
   conformance runner or an equivalent test command.
5. All open proposal decisions that block implementation have been resolved in
   docs or code comments before the dependent phase starts.
6. Unsupported or deferred proposal items are represented as explicit
   fail-closed validation, unsupported-feature errors, or documented deferred
   required extensions.

## Repository Surfaces

The plans intentionally reference current modules but do not freeze the final
file layout. Before each phase, inspect the current code and prefer existing
patterns.

Primary surfaces expected from the current tree:

- `crates/cove-core/src/constants.rs`
- `crates/cove-core/src/registry.rs`
- `crates/cove-core/src/profile/cove_map.rs`
- `crates/cove-core/src/profile/cove_map/embedded.rs`
- `crates/cove-core/src/artifact/covm.rs`
- `crates/cove-core/src/artifact/mod.rs`
- `crates/cove-map/src/identity.rs`
- `crates/cove-map/src/build.rs`
- `crates/cove-map/src/emit.rs`
- `crates/cove-map/src/api.rs`
- `crates/cove-map/src/cli.rs`
- `crates/cove-map/src/verify.rs`
- `crates/cove-map/src/project.rs`
- `crates/cove-conformance/src/gen_corpus_cove_map_support.rs`
- `crates/cove-conformance/src/main.rs`
- `crates/cove-validate/src/validate.rs`
- `crates/cove-dump/src/dump.rs`
- `crates/coveql/src/*`
- `docs/governance/section-kind-registry.md`
- `docs/governance/feature-bit-registry.md`
- `docs/governance/conformance-levels.md`

Do not edit unrelated dirty worktree files while following these plans.
