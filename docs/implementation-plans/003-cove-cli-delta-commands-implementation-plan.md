# COVE CLI Delta Commands Implementation Plan

Status: implemented for the `cove delta` command surface, delta-aware
query/export materialization, direct COVE-O delta query execution for supported
object and graph roots, snapshot sidecars, map snapshot-bundle generation,
initial resolver-aware map semantic delta generation, and release-gate coverage.
Remaining architectural work is listed under
"Follow-Up Work".

Derived from:

- [COVE-O Delta Artifacts](../proposals/cove-o-delta-artifacts.md)
- [COVE-O Delta Artifacts Implementation Plan](./002-cove-o-delta-artifacts-implementation-plan.md)

## Implementation Status

Implemented in the CLI:

1. `cove delta inspect`, `validate`, and `dump`.
2. `cove delta chain inspect`, `validate`, `plan`, `graph`, and `extend`.
3. `cove delta reconstruct`, `compact`, and `checkpoint`.
4. `cove delta publish` and `publish-atomic`.
5. `.covedelta` routing through `cove inspect` and `cove validate
   --object-delta`.
6. Delta-aware `cove query` for COVM delta manifests, with chain validation,
   CSN/commit-time snapshot pruning, direct COVE-O object-surface execution for
   supported object-backed roots and graph traversal/algorithm node roots,
   materialized fallback for unsupported roots, and optional delta plan
   diagnostics.
7. Delta-aware `cove export arrow` for COVM delta manifests, including native
   table-style direct export over validated COVE-O projection surfaces when the
   selected snapshot exposes exactly one Arrow projection, materialized fallback
   for native-planner snapshots, and explicit `--query` Arrow export for
   supported COVE-O roots over the direct delta object surface.
8. Snapshot-bound `cove sidecar build covi|covm|covx --snapshot ...`.
9. `cove map delta build` for building COVE-MAP bundle artifacts from a
   materialized delta snapshot, plus `cove map delta build --base ... --mapping
   ... --out ... <source...>` for generating resolver-aware semantic
   `.covedelta` artifacts with additive object-catalog, evidence, and
   projection patches.
10. Snapshot-bound COVE-I sidecars carry the validated delta-chain digest in
    `CoviSnapshotValidityV2`, so COVE-I validation fails closed unless callers
    provide the matching chain digest.
11. Release gates include static delta inspect/validate checks plus the targeted
    CLI smoke path that publishes a chain and exercises query, export, sidecar,
    and map delta commands.

The current query/export implementation validates the full declared chain
before execution. `cove query` can execute supported COVE-O object-backed roots
and graph traversal/algorithm node roots directly against a validated
delta-composed object surface, falling back to a materialized snapshot when
planner metadata or unsupported roots require it. `cove export arrow --query`
can export supported CoveQL roots directly as Arrow batches over the same
validated object surface. Native table-style `cove export arrow` can export a
validated delta-composed COVE-O projection surface directly when the selected
snapshot exposes exactly one Arrow projection; ambiguous projection surfaces
fail closed and require explicit `--query`.
Source-publish range pruning is exposed for `cove delta chain plan`,
`cove query`, and explicit CoveQL Arrow export. Query/export treat
`--source-publish-range` as an artifact relevance predicate, not an
object-state temporal cut: execution is allowed only when the summary proves
exact source-publish ranges and the selected ordinals form a reconstructable
dense prefix. Missing source-publish summary metadata or non-prefix selections
fail closed.

## Objective

Add a complete command-line surface for COVE-O delta artifacts and COVM
delta-chain snapshots.

The CLI must make delta support usable without weakening the storage model:

```text
base.cove + delta-0001.covedelta + delta-0002.covedelta + ...
```

The base file and every `.covedelta` file remain immutable. A selected
base-plus-delta snapshot is selected by COVM or an external catalog, never by
implicit directory scanning. Normal user-facing commands must never silently
answer from base-only data when a selected snapshot declares required deltas.

## User Outcomes

The eventual CLI should let users answer these questions:

1. Is this `.covedelta` structurally and semantically valid?
2. What parent, snapshot, CSN range, feature bits, and section payloads does it
   declare?
3. Does this COVM select a delta chain?
4. Are the base artifact and ordered deltas complete, fresh, and digest-bound?
5. Which deltas matter for an as-of or source-publish-scoped operation?
6. What read amplification will this chain cause?
7. Should the dataset be checkpointed, compacted, packed, or indexed?
8. Can this selected snapshot be materialized into a self-contained `.cove`
   file?
9. Can a new delta-bearing COVM be published safely, with the delta artifact
   written before the manifest?

## Design Principles

1. Delta commands reinforce explicit snapshot selection. They must not scan a
   directory for undeclared deltas.
2. `.cove` files remain self-contained. Delta commands may produce a new
   `.cove` through compaction or reconstruction, but must not mutate an
   existing `.cove`.
3. Unsupported required delta features fail closed.
4. The CLI should expose digest, parent, chain-order, and read-amplification
   diagnostics directly enough for CI and object-store operators.
5. Low-level artifact commands should be deterministic and scriptable.
6. High-level query, inspect, and export commands should eventually understand
   delta-bearing COVM inputs without requiring users to manually invoke the
   low-level commands first.
7. COVE-MAP should own semantic production of resolver-aware object deltas.
   The `cove delta` namespace should own validation, planning, publication,
   reconstruction, compaction, and operational maintenance.

## Non-Goals

1. No command that appends in place to a finalized `.cove` file.
2. No implicit transaction log discovered from filenames.
3. No base-only query fallback for a selected delta-bearing snapshot.
4. No hidden raw FileCode equality across artifacts.
5. No general multi-writer ACID table protocol.
6. No broad `cove delta create` command until COVE-O and COVE-MAP writer
   semantics are explicit enough to avoid creating invalid logical deltas.

## Final Command Surface

The target top-level help should eventually include:

```text
cove delta inspect <delta.covedelta> [--json]
cove delta validate <delta.covedelta> [--object-delta] [--json]
cove delta dump <delta.covedelta> (--section <id|kind> | --parent-refs | --summary) [--max-bytes n]

cove delta chain inspect <manifest.covm> [--json]
cove delta chain validate <manifest.covm> --dataset <dir> [--summary <file>] [--json]
cove delta chain plan <manifest.covm> --dataset <dir> [--as-of-csn n] [--as-of-commit-us n] [--source-publish-range start:end] [--json]
cove delta chain graph <manifest.covm> --dataset <dir> [--format text|json|dot]
cove delta chain extend --manifest <manifest.covm> --delta <delta.covedelta> --out <manifest.covm> [--summary-out <file>] [--force]

cove delta reconstruct <manifest.covm> --dataset <dir> --out <snapshot.cove> [--json]
cove delta compact <manifest.covm> --dataset <dir> --out <snapshot.cove> [--publish-covm <manifest.covm>] [--json]
cove delta checkpoint <manifest.covm> --dataset <dir> --out <checkpoint.covedelta> [--summary-out <file>] [--json]
cove delta publish --base <base.cove> --delta <delta.covedelta>... --out <manifest.covm> [--summary <file>|--summary-out <file>] [--json]
cove delta publish-atomic --delta <delta.covedelta> --manifest <manifest.covm> [--json]

cove query <manifest.covm> --dataset <dir> [--as-of-csn n|--as-of-commit-us n] [--delta-plan|--delta-plan-json] '<coveql>'
cove export arrow <manifest.covm> <output.arrow|output.json> --dataset <dir> [--as-of-csn n|--as-of-commit-us n] [--delta-plan-json]
cove export arrow --query '<coveql>' <manifest.covm> <output.arrow|output.json> --dataset <dir> [--as-of-csn n|--as-of-commit-us n] [--delta-plan|--delta-plan-json]
cove sidecar build covi --snapshot <manifest.covm> --dataset <dir> --out <snapshot.covi> [--as-of-csn n|--as-of-commit-us n] [covi options]
cove sidecar build covm --snapshot <manifest.covm> --dataset <dir> --out <snapshot.covm> [--as-of-csn n|--as-of-commit-us n]
cove sidecar build covx --snapshot <manifest.covm> --dataset <dir> --out <snapshot.covx> [--as-of-csn n|--as-of-commit-us n]
cove map delta build <manifest.covm> --dataset <dir> --out-dir <dir> [--as-of-csn n|--as-of-commit-us n] [--projection-output cove-t|none] [--publish-covm] [--verify]
cove map delta build --base <manifest.covm> --dataset <dir> --mapping <mapping.covemap> --out <delta.covedelta> [--source-publish-range start:end] <source...>
```

## Command Details

### `cove delta inspect`

Purpose: human and machine-readable overview of one `.covedelta` artifact.

Expected output fields:

1. artifact kind and version.
2. delta artifact ID.
3. dataset ID.
4. snapshot ID and parent snapshot ID.
5. chain ordinal.
6. CSN range.
7. commit-time range.
8. optional source-publish range.
9. required and optional delta feature bit names.
10. parent refs, including the lineage parent.
11. section directory summary by section ID and section kind.
12. object-delta summary when semantic validation succeeds.

Implementation notes:

- Use `CoveDeltaFile::parse`.
- Use `CoveDeltaFile::validate_object_delta` when enough sections are present
  and return object-delta diagnostics separately from structural diagnostics.
- Add stable JSON output before relying on the command in release gates.

### `cove delta validate`

Purpose: validate a single delta artifact.

Default behavior validates:

1. `CVD2` magic and version.
2. postscript and footer.
3. file length.
4. checksums.
5. parent-ref count and exactly one lineage parent.
6. canonical section payload ordering.
7. section CRCs and known section kinds.

With `--object-delta`, also validate:

1. supported required feature bits.
2. header and postscript feature agreement.
3. CSN and time-range monotonicity inside the delta.
4. parent refs used by object-delta sections.
5. dictionary overlay feature requirements.
6. sparse patch, continuation anchor, touched set, tombstone set, evidence
   patch, projection patch, coverage patch, index hint, and layout hint rules.

Acceptance:

1. `cove validate <file.covedelta>` routes to equivalent validation.
2. JSON output includes `ok`, `artifact`, `error_code`, and `error`.

### `cove delta dump`

Purpose: forensic byte-level access to delta payloads without pretending they
are ordinary `.cove` sections.

Initial selectors:

1. `--parent-refs`
2. `--section <id>`
3. `--section <kind>`
4. `--summary`

This command is intentionally low-level and should mirror the existing
`cove dump` style.

### `cove delta chain inspect`

Purpose: inspect COVM delta-chain metadata without opening all data artifacts.

Expected output fields:

1. whether `COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED` is set.
2. dataset ID.
3. base artifact ref.
4. result snapshot ID.
5. ordered delta count.
6. ordered delta refs.
7. chain digest algorithm and digest.
8. chain summary kind, length, CRC, and digest.
9. effective schema, object catalog, projection, semantic-map, visibility, and
   redaction fingerprints.
10. CSN range and created timestamp.
11. required and optional delta features.

Implementation notes:

- Use `CovmFile::parse_delta_aware` for delta-bearing manifests.
- Parse and report the delta-chain extension when present.
- Non-delta COVM inputs should produce a clear "no delta chain declared"
  result, not an error.

### `cove delta chain validate`

Purpose: validate the selected base-plus-delta snapshot.

Inputs:

```text
cove delta chain validate <manifest.covm> --dataset <dir> [--summary <file>] [--json]
```

Validation rules:

1. Parse the COVM in delta-aware mode.
2. Load the declared base artifact by COVM URI relative to `--dataset`.
3. Load each declared delta artifact by COVM URI relative to `--dataset`.
4. Validate base artifact length, footer CRC, and digest.
5. Validate every delta artifact length, footer CRC, digest, artifact ID, chain
   ordinal, dataset ID, snapshot ID, and parent snapshot ID.
6. Validate ordered parent-snapshot continuity.
7. Validate lineage parent refs against selected artifacts.
8. Validate chain summary bytes when declared or passed through `--summary`.
9. Reject missing, extra, stale, reordered, digest-mismatched, or
   unsupported-feature deltas.

Implementation notes:

- Use `validate_selected_delta_chain_with_base` and
  `validate_selected_delta_chain_with_summary_bytes`.
- Resolve artifact paths only from declared COVM URIs and explicit CLI flags.

### `cove delta chain plan`

Purpose: explain which deltas must be opened for a scoped operation and what
cost that implies.

Inputs:

```text
cove delta chain plan <manifest.covm> --dataset <dir> \
  [--as-of-csn n] \
  [--as-of-commit-us n] \
  [--source-publish-range start:end] \
  [--json]
```

Expected output fields:

1. selected chain ordinals.
2. skipped chain ordinals.
3. skip reasons.
4. delta chain depth.
5. selected and skipped delta counts.
6. planned artifact opens.
7. estimated object-store requests.
8. chain summary bytes.
9. delta/base byte ratio when base size is known.
10. read-amplification recommendations.

Implementation notes:

- Use `CovmDeltaChainSummaryV1::prune_delta_chain`.
- Use `CovmDeltaChainSummaryV1::read_amplification_metrics`.
- Use `CovmDeltaReadAmplificationPolicy::default()` initially, with explicit
  flags only after there is a real operator need.

### `cove delta chain graph`

Purpose: show lineage in a form suitable for humans, CI logs, or documentation.

Formats:

1. `text`
2. `json`
3. `dot`

The graph must be generated from selected COVM refs, not from filesystem
discovery.

### `cove delta reconstruct`

Purpose: materialize the selected base-plus-delta snapshot into a normal
self-contained `.cove` file.

Rules:

1. Validate the selected chain first.
2. Reconstruct object state through the COVE-O readback path.
3. Emit an ordinary `.cove` artifact.
4. The output must not retain hidden dependencies on the source deltas.

This is a compatibility command. It lets non-delta-aware readers consume the
selected snapshot after materialization.

### `cove delta compact`

Purpose: maintenance command for reducing read amplification.

Behavior:

1. Validate the selected chain.
2. Materialize a new self-contained `.cove`.
3. Optionally publish a new non-delta COVM pointing at the compacted file.
4. Emit before/after metrics: chain depth, delta bytes, request estimate, and
   recommendation status.

`compact` may initially call the same implementation as `reconstruct` but
should remain a separate command because its user intent is operational
maintenance rather than compatibility export.

### `cove delta checkpoint`

Purpose: create a checkpoint delta when a full base rewrite is not desired.

Rules:

1. Validate the selected chain.
2. Produce an ordinary `.covedelta` artifact using checkpoint-baseline feature
   semantics.
3. Bind the checkpoint to the selected parent snapshot.
4. Emit or update chain-summary metadata.

Implementation note:

- The CLI reconstructs the selected live object state, emits checkpoint rows as
  `Snapshot` records with a new append-only CSN/time range, binds the lineage
  parent to the selected result snapshot, and can emit CDS1 chain-summary bytes
  for a future manifest extension.

### `cove delta publish`

Purpose: build a delta-aware COVM snapshot selector.

Inputs:

```text
cove delta publish \
  --base base.cove \
  --delta delta-0001.covedelta \
  --delta delta-0002.covedelta \
  --out dataset.covm
```

Rules:

1. Validate the base artifact.
2. Validate every delta artifact.
3. Verify ordered snapshot continuity.
4. Compute and bind artifact refs.
5. Compute and bind the chain digest.
6. Include or emit a chain summary.
7. Set the COVM fail-closed delta-chain-required postscript flag.
8. Refuse to publish when any required feature bit is unsupported or
   undeclared.

### `cove delta chain extend`

Purpose: safely create the next COVM snapshot by adding one or more deltas to
an existing delta-bearing manifest.

Rules:

1. Validate the existing selected chain.
2. Validate that the new delta extends the current result snapshot.
3. Produce a new COVM. Do not mutate the old manifest.
4. Recompute the chain digest and summary binding.

### `cove delta publish-atomic`

Purpose: publish a delta and manifest in the durable order required for
object-store style publication.

Rules:

1. Write or replace the delta artifact first.
2. Write or replace the manifest last.
3. Use the durable helper in `crates/cove-core/src/durable.rs`.
4. Reject identical delta and manifest paths.

## Integration With Existing Commands

### `cove validate`

`cove validate` should recognize:

1. `.cove`
2. `.covemap`
3. `.covm`
4. `.covedelta`
5. delta-chain summary and extension fixtures only when explicitly selected by
   a low-level kind flag, if needed.

For `.covedelta`, default validation should be equivalent to
`cove delta validate`. Add an option equivalent to `--object-delta` if the
existing validate flag model can support it without confusion.

### `cove inspect`

`cove inspect <manifest.covm>` should report:

1. whether the manifest is delta-bearing.
2. whether object-temporal reads require delta support.
3. the selected chain depth.
4. useful next commands:

```text
cove delta chain validate <manifest.covm> --dataset <dir>
cove delta chain plan <manifest.covm> --dataset <dir>
```

`cove inspect <delta.covedelta>` should route to a beginner-friendly subset of
`cove delta inspect`.

### `cove query`

`cove query <manifest.covm> --dataset <dir>` should eventually support
delta-bearing selected snapshots.

Rules:

1. Delta-bearing COVM inputs must parse in delta-aware mode.
2. The selected chain must be validated before object-temporal data is read.
3. Base-only fallback is forbidden for selected delta-bearing snapshots.
4. `--perf-report` should include delta-chain metrics when applicable.
5. `--strict-performance` should fail when chain metrics violate hard policy.

Potential query flags:

```text
--as-of-csn n
--as-of-commit-us n
--source-publish-range start:end
--delta-plan
--delta-plan-json
```

Add these only when they can be wired through the execution and planning model
without duplicating `cove delta chain plan`.

### `cove export arrow`

`cove export arrow <manifest.covm> --dataset <dir> <out.arrow>` should
eventually export the selected delta-aware snapshot with the same fail-closed
rules as query.

Implemented direct mode:

```text
cove export arrow --query '<coveql>' <manifest.covm> <out.arrow> --dataset <dir>
```

This mode validates the declared chain, reads the selected COVE-O object surface
directly, and fails closed if selected deltas require materialized planner
metadata. Native table-style export remains materialized until table/export
semantics over a delta object surface are specified.

### `cove sidecar`

Delta-aware sidecar generation should be explicit:

```text
cove sidecar build covi --snapshot <manifest.covm> --dataset <dir> --out <snapshot.covi>
cove sidecar build covx --snapshot <manifest.covm> --dataset <dir> --out <snapshot.covx>
```

Sidecar inspect should report:

1. result snapshot ID.
2. bound chain digest.
3. base artifact ref.
4. stale or fresh status when a manifest is supplied.

### `cove map`

COVE-MAP should own semantic delta production:

```text
cove map build --delta-from <manifest.covm> --dataset <dir> --out-dir <dir> ...
```

or:

```text
cove map delta build --base <manifest.covm> --dataset <dir> --out-dir <dir> ...
```

The exact spelling should be chosen when implementation starts. The important
boundary is that `cove map` understands resolver catalogs, evidence patches,
projection patches, reviewed decisions, and semantic-map fingerprints, while
`cove delta` validates, plans, publishes, and maintains delta chains.

## Primary Code Surfaces

Inspect current code before editing. Expected surfaces:

- `crates/cove-cli/src/lib.rs`
- `crates/cove-cli/src/help.rs`
- `crates/cove-cli/src/sidecar.rs`
- `crates/cove-cli/src/output.rs`
- `crates/cove-core/src/artifact/covedelta.rs`
- `crates/cove-core/src/artifact/covm.rs`
- `crates/cove-core/src/profile/cove_o/readback.rs`
- `crates/cove-core/src/utility.rs`
- `crates/cove-core/src/durable.rs`
- `crates/cove-validate/src/validate.rs`
- `crates/cove-dump/src/dump.rs`
- `crates/coveql/src/lib.rs`
- `crates/cove-datafusion/src/*`
- `crates/cove-map/src/cli.rs`
- `crates/cove-map/src/build.rs`
- `crates/cove-conformance/src/main.rs`
- `crates/cove-conformance/src/bin/gen-corpus.rs`
- `crates/cove-cli/tests/smoke.rs`
- `conformance/accept/*delta*`
- `conformance/reject/*delta*`

Prefer adding a dedicated `crates/cove-cli/src/delta.rs` module once the first
delta command is implemented.

## Implementation Phases

### Phase 0: CLI Shape And Help

Add the `cove delta` namespace, help topic, and parser skeleton.

Acceptance:

1. `cove delta --help` lists MVP commands.
2. `cove --help` lists the delta namespace.
3. Unknown delta subcommands produce useful errors.
4. No command performs filesystem discovery of undeclared deltas.

### Phase 1: Single-Artifact Inspect And Validate

Implement:

```text
cove delta inspect <delta.covedelta> [--json]
cove delta validate <delta.covedelta> [--object-delta] [--json]
```

Acceptance:

1. Valid conformance `.covedelta` fixtures inspect successfully.
2. Reject fixtures fail with stable error codes.
3. JSON output is stable enough for tests.
4. `cove validate <file.covedelta>` succeeds or fails consistently with
   `cove delta validate`.

### Phase 2: Delta Dump

Implement:

```text
cove delta dump <delta.covedelta> (--section <id|kind> | --parent-refs | --summary) [--max-bytes n]
```

Acceptance:

1. Parent refs can be displayed without parsing object-delta semantics.
2. Known section kinds can be selected by name.
3. Unknown section names produce a useful error.
4. Output is bounded by `--max-bytes`.

### Phase 3: Chain Inspect, Validate, And Plan

Implement:

```text
cove delta chain inspect <manifest.covm> [--json]
cove delta chain validate <manifest.covm> --dataset <dir> [--summary <file>] [--json]
cove delta chain plan <manifest.covm> --dataset <dir> [time/pruning options] [--json]
cove delta chain graph <manifest.covm> --dataset <dir> [--format text|json|dot]
```

Acceptance:

1. Non-delta COVM inspect reports no declared delta chain.
2. Delta-bearing COVM inspect reports fail-closed state and chain metadata.
3. Valid selected-chain fixtures pass validation.
4. Missing, extra, stale, reordered, and digest-mismatched deltas fail.
5. Plan output matches pruning conformance expectations for CSN,
   commit-time, and source-publish cases.
6. Read-amplification recommendations are visible in text and JSON output.

### Phase 4: Beginner Command Integration

Integrate delta awareness into:

```text
cove inspect
cove doctor
cove query
cove export arrow
```

Acceptance:

1. Inspect and doctor explain delta-bearing COVM state and next commands.
2. Query rejects unsupported delta-bearing selected snapshots instead of
   falling back to base-only reads.
3. Query succeeds once delta-aware readback is wired through the execution path.
4. Perf reports include delta-chain metrics when applicable.
5. Export follows the same fail-closed behavior as query.

### Phase 5: Reconstruction And Compaction

Implement:

```text
cove delta reconstruct <manifest.covm> --dataset <dir> --out <snapshot.cove> [--json]
cove delta compact <manifest.covm> --dataset <dir> --out <snapshot.cove> [--publish-covm <manifest.covm>] [--json]
```

Acceptance:

1. Selected chain validation runs before writing output.
2. Reconstructed output validates as an ordinary self-contained `.cove`.
3. Compacted output is semantically equivalent to base-plus-deltas for the
   covered fixtures.
4. Optional COVM publication points at the compacted `.cove` and does not
   declare required deltas.

### Phase 6: Publishing And Extension

Implement:

```text
cove delta publish --base <base.cove> --delta <delta.covedelta>... --out <manifest.covm> [--summary <file>|--summary-out <file>] [--json]
cove delta chain extend --manifest <manifest.covm> --delta <delta.covedelta> --out <manifest.covm> [--summary-out <file>] [--force]
cove delta publish-atomic --delta <delta.covedelta> --manifest <manifest.covm> [--json]
```

Acceptance:

1. Published COVM parses in delta-aware mode.
2. Non-delta-aware COVM parse fails closed for required delta chains.
3. Published chain validation passes using only declared artifact refs.
4. `extend` refuses deltas that do not extend the current result snapshot.
5. Atomic publish writes the delta before the manifest and rejects identical
   output paths.

### Phase 7: Checkpoints And Delta-Aware Sidecars

Implemented checkpoint generation and initial snapshot-bound sidecar
generation.

Acceptance:

1. Checkpoint deltas validate as ordinary `.covedelta` artifacts.
2. Chain plan recommendations change after checkpointing.
3. Snapshot-level COVE-I, COVM, and COVX sidecars are built from the validated
   materialized snapshot.
4. Snapshot-level COVE-I sidecars carry explicit delta-chain binding metadata.
   Snapshot COVM and COVX sidecars remain bound to the materialized snapshot
   file identity because those artifact formats do not yet expose equivalent
   delta-chain digest fields.

### Phase 8: COVE-MAP Delta Production

Initial `cove map delta build` support builds COVE-MAP bundle artifacts from a
validated materialized delta snapshot.

Initial semantic delta production is also implemented through:

```text
cove map delta build --base <manifest.covm> --dataset <dir> --mapping <mapping.covemap> --out <delta.covedelta> <source...>
```

This mode validates the parent manifest chain, reads the selected parent object
surface, materializes COVE-MAP source rows, rewrites generated rows as
full-value temporal `Delta` records, emits additive object-catalog patches,
emits additive evidence patches, emits projection patch upserts when projection
IDs are new or their definitions changed, binds deterministic
semantic/object/projection fingerprint refs in the COVEDELTA header, emits
strong continuation anchors with state-hash descriptors for full-value rows
that update existing parent objects, emits exact touched-object ranges and
exact tombstone ranges when tombstones are generated, and validates the
generated bytes with `CoveDeltaFile::validate_object_delta`. The CLI smoke
extends a published chain with generated map deltas for new and existing
objects, validates the extended chains, queries the resulting mapped object
state, and asserts touched-object ranges are present. Focused COVE-MAP tests
also cover reviewed-decision and alias-catalog semantic changes by validating
that generated semantic deltas carry changed semantic-map fingerprints and
compose to the same logical object state as a full rebuild for additive
new-object deltas. Existing-parent reviewed-decision and alias-catalog remaps
that merge prior live parent objects now synthesize parent-object tombstone rows,
emit exact tombstone ranges and continuation anchors, and compose to the same
logical object state as a full rebuilt snapshot. Delta evidence patches now
upsert by source evidence identity, so remaps replace stale parent
`output_object_id` targets instead of leaving old evidence targets visible.
Semantic delta builds emit sparse property-op sections for delta rows whose
changed properties can be represented as explicit `SetNull` or `SetValue`
operations with real inline value refs. FileCode-backed semantic deltas now
emit delta-local inline value tables plus inline dictionary-overlay entries,
and delta readback materializes those values for requested FileCode
properties. Parent dictionary aliases materialize through the selected parent
dictionary registry for base and previously applied delta dictionaries, with
COVM-aware callers passing the selected base artifact identity into readback and
redacted parent aliases preserving the existing FileCode redaction policy.

Current full-support implementation status: complete for the supported delta
contract. Non-additive object catalog migrations are intentionally not delta
operations in the proposal: catalog patches are additive-only, and
renames/removals/reinterpretive schema changes must publish a new base `.cove`
snapshot or use a separate schema-generation branch. The implementation fails
closed for reinterpretive patches and treats absence from a patch as no removal.

Acceptance:

1. Map delta builds bind resolver catalog identity, semantic-map fingerprint,
   evidence patches, projection patches, and object-catalog changes correctly.
2. Reviewed decisions and alias-catalog changes that affect identity produce a
   new effective semantic-map fingerprint.
3. Generated deltas pass `cove delta validate --object-delta`.
4. Published map delta chains pass `cove delta chain validate`.
5. Parity tests prove equivalence against a full rebuilt snapshot.

## Follow-Up Work

1. Completed for CoveQL object-surface query roots: `cove query` can execute
   supported COVE-O object-backed roots and graph traversal/algorithm node roots
   over a validated delta-composed object surface without first writing
   reconstructed snapshot bytes. Completed for explicit CoveQL Arrow export:
   `cove export arrow --query ...` emits Arrow IPC/JSON from the same direct
   object surface. Completed for native table-style export where the selected
   COVE-O delta snapshot has exactly one Arrow projection:
   `cove export arrow <manifest.covm> <out>` projects directly from the
   validated object surface and reports `direct_projection_surface`. Future
   extension point: add any future query/export roots that still require
   materialized planner metadata.
2. Completed initial safe source-publish scoped query/export semantics:
   `--source-publish-range` prunes by exact source-publish summary metadata and
   executes only when the selected chain remains a dense-prefix snapshot.
   Missing exact summaries and non-prefix selections fail closed. Future product
   question: decide whether a non-prefix "changes for source publish range"
   result model is needed separately from snapshot query/export.
3. Completed for generated sidecars at the current format level:
   snapshot COVE-I carries the validated delta-chain digest, while snapshot
   COVM and COVX bind the materialized snapshot by referenced file ID, file
   length, footer CRC, and digest. Future format question: decide whether
   future COVM/COVX versions need logical delta-chain digest fields in addition
   to materialized snapshot identity.
4. Resolver-aware semantic delta production is implemented for additive
   object-catalog/evidence patches, projection patch upserts, full-value
   temporal delta rows, and continuation-anchor/state-hash descriptors for
   full-value updates to existing parent objects, plus exact touched-object
   ranges and exact tombstone ranges when tombstone rows are generated. Initial
   reviewed-decision and alias-catalog parity fixtures validate semantic
   fingerprint changes and full-rebuild equivalence for additive new-object
   deltas. Existing-parent reviewed-decision and alias-catalog remap fixtures
   now validate object-state parity through generated parent-object tombstones.
   Delta evidence patches replace stale evidence targets by source evidence
   identity during readback.
   Sparse property-op sections are emitted for explicit `SetNull` and
   `SetValue` updates with real inline value refs. FileCode-backed semantic
   deltas emit inline dictionary overlays and readback materializes those
   delta-local FileCode values. Parent dictionary alias overlays materialize
   through validated selected-parent dictionary identities and preserve redaction
   policy. Non-additive object catalog migrations are intentionally outside the
   delta patch contract and require a new base snapshot or schema-generation
   branch.
5. Completed: release-gate conformance coverage now requires delta
   inspect/validate commands and the targeted CLI delta smoke path covering
   query/export/sidecar/map delta commands.

### Phase 9: Conformance, Docs, And Release Gates

Add release-gate coverage and user-facing examples.

Status: release-gate coverage is implemented through
`conformance/accept/suite_release_gates_contract.json` and
`scripts/release-gates.sh`. The end-to-end implementer example is included
below.

Acceptance:

1. Conformance fixtures cover the new CLI contract.
2. `conformance/accept/suite_release_gates_contract.json` includes at least:

```text
cargo run -p cove-cli -- delta inspect conformance/accept/covedelta_valid.covedelta > /dev/null
cargo run -p cove-cli -- delta validate conformance/accept/covedelta_object_delta_valid.covedelta --object-delta > /dev/null
cargo run -p cove-cli -- delta chain inspect <delta-chain-manifest-fixture> > /dev/null
cargo run -p cove-cli -- delta chain plan <delta-chain-manifest-fixture> --dataset <fixture-dir> > /dev/null
```

3. `cove --help`, `cove delta --help`, and relevant command help pages stay in
   sync.
4. README or implementer docs include one end-to-end example:

```text
cove delta chain validate dataset.covm --dataset bundle
cove delta chain plan dataset.covm --dataset bundle --as-of-csn 100
cove query dataset.covm --dataset bundle 'table(objects).take(10)'
cove delta compact dataset.covm --dataset bundle --out compacted.cove
```

## Open Decisions

Resolved and remaining decisions:

1. Resolved: `cove delta chain inspect` parses extension bytes embedded in
   COVM only, or also accept a raw extension fixture through `--extension`.
2. Resolved: `--summary` means external summary bytes when accepted, while
   inline summary bytes remain the default.
3. Resolved: query time-selection flags are accepted on `cove query` for
   delta-bearing COVM inputs.
4. Resolved: the COVE-MAP delta command spelling is
   `cove map delta build`.
5. Remaining: Whether `--summary` should ever
   override a COVM-declared inline summary.
6. Remaining: Whether `reconstruct` and `compact` should be separate implementations or
   one implementation with different output reporting.
7. Remaining: Whether `publish` should accept only paths or also a JSON manifest of
   artifact refs for object-store deployments.
8. Remaining: How to represent external catalog snapshots once COVM is not the selector.
9. Remaining: Which JSON schemas become stability commitments for downstream automation.

## Recommended Initial Cut

Implement this first:

```text
cove delta inspect <delta.covedelta> [--json]
cove delta validate <delta.covedelta> [--object-delta] [--json]
cove delta chain inspect <manifest.covm> [--json]
cove delta chain validate <manifest.covm> --dataset <dir> [--summary <file>] [--json]
cove delta chain plan <manifest.covm> --dataset <dir> [--as-of-csn n] [--json]
```

This gives users and CI a real delta surface using APIs that already exist in
core, while deferring writer-heavy commands until reconstruction, compaction,
checkpoint, and COVE-MAP production semantics are ready.
