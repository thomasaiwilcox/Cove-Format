# COVE-MAP Entity Resolution Implementation Plan

Status: implementation plan

Derived from:

- [COVE-MAP Resolver Catalog and Entity Resolution](../proposals/covemap-entity-resolution.md)
- [Cross-Feature Implementation Sequence](./000-cross-feature-implementation-sequence.md)

## Objective

Add deterministic, auditable entity-resolution support to COVE-MAP while
preserving COVE-O object authority. Following this plan in numbered order must
produce resolver-backed identity planning, materialized COVE-O output,
resolution evidence, replay/explain support, candidate matching, reviewed
equivalences, and governance/redaction support.

This plan intentionally implements entity resolution before resolver-aware
COVE-O delta patches. Delta artifacts can later carry the materialized object
records and evidence produced here, but this feature must work with ordinary
self-contained `.cove` output first.

## Implementation Contract

1. Existing COVE-MAP files remain valid.
2. COVE must not silently guess object identity.
3. Fuzzy, approximate, or candidate-only evidence must never form GOID merge
   edges unless promoted by an explicitly reviewed or authoritative decision.
4. Resolver replay must be deterministic from source bytes, schema
   fingerprints, COVE-MAP bytes, resolver catalog content or digests, reviewed
   decision content or digests, normalizer versions, table digests, match-rule
   versions, and implementation versions for non-trivial scoring.
5. Object reconstruction from already materialized COVE-O rows must not require
   the resolver catalog unless the selected read surface asks for deterministic
   replay, resolution-aware explain, candidate/review surfaces, or
   resolution-specific evidence readback.
6. Unknown resolver-backed fields must fail closed before materialization.

## Primary Code Surfaces

Inspect current code before editing. Expected primary surfaces:

- `crates/cove-core/src/constants.rs`
- `crates/cove-core/src/registry.rs`
- `crates/cove-core/src/profile/cove_map.rs`
- `crates/cove-core/src/profile/cove_map/embedded.rs`
- `crates/cove-map/src/identity.rs`
- `crates/cove-map/src/build.rs`
- `crates/cove-map/src/emit.rs`
- `crates/cove-map/src/api.rs`
- `crates/cove-map/src/cli.rs`
- `crates/cove-map/src/verify.rs`
- `crates/cove-map/src/project.rs`
- `crates/cove-map/src/suggest.rs`
- `crates/cove-conformance/src/gen_corpus_cove_map_support.rs`
- `crates/cove-conformance/src/main.rs`
- `crates/cove-validate/src/validate.rs`
- `crates/cove-dump/src/dump.rs`
- `crates/coveql/src/resolver.rs`
- `crates/coveql/src/evidence_opt.rs`
- `crates/coveql/src/logical_plan.rs`
- `crates/coveql/src/parser.rs`
- `docs/governance/section-kind-registry.md`
- `docs/governance/feature-bit-registry.md`

## Non-Goals

Do not implement these while following this plan unless a later phase explicitly
adds them:

1. Silent fuzzy auto-merge.
2. Live external identity service dependency during deterministic replay.
3. Large language model decisions in authoritative conversion.
4. A global claim that a name string uniquely identifies a real-world entity.
5. Source-file mutation.
6. Replacement of strong identifiers such as company numbers, LEIs, tax IDs,
   internal IDs, or stable source-system keys.
7. COVE-O delta-specific resolver patches. Those belong to plan 002 after this
   plan establishes stable resolver semantics.

## Phase 0: Schema, Registry, Digests, And Validation

### 0.1 Register The Resolution Catalog

1. Add section kind `MAP_RESOLUTION_CATALOG`.
2. Use proposed ID `69` if accepted by the section registry.
3. If ID `69` is unavailable, implement a registered extension or
   digest-pinned `.covemap` companion artifact before continuing.
4. Bind the section to COVE-MAP and `FEATURE_SEMANTIC_MAP`.
5. Preserve the embedded COVE-MAP v2 JSON envelope:

```json
{
  "schema_id": "org.coveformat.covemap.v2",
  "section_id": 69
}
```

Acceptance:

1. `MAP_RESOLUTION_CATALOG` appears in Rust section-kind constants and registry
   metadata.
2. `docs/governance/section-kind-registry.md` documents the chosen ID or
   extension fallback.
3. Validators reject payloads whose envelope `section_id` does not match the
   registered resolution-catalog section kind.

### 0.2 Add Parser Data Structures

Add strict parser structs for the resolution catalog. At minimum:

1. `MapResolutionCatalog`.
2. `MapNormalizationPipeline`.
3. `MapNormalizationFunction`.
4. `MapNormalizationTable`.
5. `MapResolver`.
6. `MapAliasCatalog`.
7. `MapAliasEntry`.
8. `MapCandidateMatchRule`.
9. `MapReviewedDecision`.
10. `MapTypedIdentityReference`.
11. `MapCanonicalAnchor`.
12. `MapResolutionBinding`.
13. `MapResolutionOutcome`.
14. `MapEffectiveMergeAuthority`.

The resolution catalog must include these top-level arrays:

1. `normalization_pipelines`.
2. `resolvers`.
3. `match_rules`.
4. `reviewed_decisions`.

Validation rules:

1. `resolver_id` is unique within the mapping.
2. `pipeline_id` is unique within the mapping.
3. Pipeline functions match `MAP_FUNCTION_REGISTRY` by `(function_id, version)`.
4. Matched functions must be deterministic and must not depend on random,
   wall-clock, locale-default, network, or mutable external state.
5. `canonical_key` is stable within the resolver catalog version.
6. `canonical_label` is display data and never an identity key.
7. Alias lookup uses normalized aliases produced by the resolver pipeline.
8. One normalized alias must not map to multiple canonical keys unless the
   resolver or alias entry marks it ambiguous.
9. Ambiguous aliases must not auto-merge.
10. External catalogs use stable URI plus digest, never a mutable URL alone.
11. Unknown fields reject unless explicitly listed as non-semantic metadata.

Acceptance:

1. Valid minimal resolution catalogs parse.
2. Duplicate IDs reject.
3. Unknown nested fields reject.
4. Unsupported resolver kinds reject before materialization.

### 0.3 Extend Existing COVE-MAP Schemas

Extend strict parsers without weakening old validation:

1. Add `allow_reviewed_equivalence: bool = false` to `MapIdentityRule`.
2. Add `resolution: Option<MapResolutionBinding>` to `MapJoinKeyComponent`.
3. Extend the identity-rule catalog allowlist for
   `allow_reviewed_equivalence`.
4. Extend the join-key allowlist for `resolution`.
5. Preserve existing non-resolver join-key behavior.

For resolver-backed join keys:

1. `canonicalization` must be `identity` or `none`.
2. The resolver owns normalization.
3. Double-normalization must reject.

Acceptance:

1. Existing identity-rule fixtures still parse.
2. Resolver-backed identity-rule fixtures parse.
3. Resolver-backed join keys with non-identity canonicalization reject.
4. Missing resolver references reject with a stable diagnostic.

### 0.4 Define Canonical JSON Digest Inputs

Implement COVE canonical JSON v1 for resolver digests:

1. UTF-8 JSON object payload.
2. Duplicate keys rejected before digesting.
3. Object keys sorted by bytewise UTF-8 order.
4. Arrays preserved in declared order unless the schema declares the array
   semantically unordered.
5. No insignificant whitespace.
6. Strings encoded with deterministic JSON escaping.
7. Integers and decimal numbers encoded in parsed canonical JSON form.
8. Fields marked `non_semantic_metadata` excluded from semantic digests.
9. All other metadata participates in the digest.

Digest formulas:

```text
catalog_digest =
  sha256(canonical-json(alias_catalog without catalog_digest fields))

resolver_digest =
  sha256(canonical-json({
    resolver_id,
    kind,
    object_type,
    authority,
    confidence_class,
    normalization_pipeline_id,
    pipeline_digest,
    on_hit,
    on_miss,
    miss_confidence_class,
    ambiguous_policy,
    catalog_digest
  }))

pipeline_digest =
  sha256(canonical-json(normalization pipeline, referenced table IDs, and
  referenced table digests))
```

Ordering rules:

1. Normalization pipeline arrays preserve declared order.
2. Alias catalog entries sort by `alias_entry_id` by default.
3. Aliases within an entry sort by normalized alias bytes, then raw alias bytes.
4. Candidate outputs sort by deterministic output order.
5. `order_sensitive_catalog: true` may opt into catalog-order sensitivity, but
   the default is unordered alias semantics.

Acceptance:

1. Changing alias order without semantic change keeps `catalog_digest` stable.
2. Changing a pipeline function version changes `pipeline_digest` and
   `resolver_digest`.
3. A `resolver_digest` that does not include `pipeline_digest` rejects.
4. Embedded and external catalog digest mismatches reject.

### 0.5 Extend Evidence Metadata Allowlist

Permit resolution metadata in `MAP_EVIDENCE_INDEX.operation_metadata`.

Required keys:

```text
resolution_kind
resolver_id
resolver_digest
catalog_digest
pipeline_digest
normalization_pipeline_id
raw_observed_value
normalized_value
resolved_identity_value
canonical_key
canonical_label
alias_catalog_id
alias_entry_id
alias_hit
alias_miss
alias_ambiguous
miss_policy
candidate_match_id
candidate_score
left_source_id
left_source_row_identity
left_raw_observed_value
left_normalized_value
left_row_digest
right_source_id
right_source_row_identity
right_raw_observed_value
right_normalized_value
right_row_digest
blocking_key
match_rule_id
review_decision_id
redacted_resolution_evidence
```

Acceptance:

1. Evidence parser accepts the listed keys.
2. Evidence parser still rejects unknown keys when strict validation is active.
3. Filtered metadata readback can request any listed key without exposing other
   metadata.

### 0.6 Add Stable Diagnostics

Add or map stable diagnostics:

1. `MAP_RESOLUTION_CATALOG_MISSING`.
2. `MAP_RESOLVER_UNSUPPORTED`.
3. `MAP_RESOLVER_DIGEST_MISMATCH`.
4. `MAP_CATALOG_DIGEST_MISMATCH`.
5. `MAP_PIPELINE_DIGEST_MISMATCH`.
6. `MAP_ALIAS_AMBIGUOUS`.
7. `MAP_ALIAS_MISS`.
8. `MAP_REVIEW_DECISION_CONFLICT`.
9. `MAP_DO_NOT_MERGE_VIOLATION`.
10. `MAP_CANONICAL_ANCHOR_REQUIRED`.
11. `MAP_CANDIDATE_RULE_UNSUPPORTED`.
12. `MAP_RESOLUTION_NOT_REPLAYABLE`.

Each diagnostic should include source ID, row identity, rule ID, resolver ID,
and enough context to reproduce the failure when available.

### 0.7 Phase 0 Gate

Phase 0 is complete only when:

1. The section registry decision is implemented.
2. Parser structs and strict schema validation exist.
3. Identity-rule and join-key extensions parse and validate.
4. Evidence metadata allowlists include resolver and candidate-pair fields.
5. Canonical digest formulas and ordering rules are implemented and tested.
6. Pipeline functions validate by `(function_id, version)`.
7. `miss_confidence_class` exists and is required for `on_miss:
   normalized_value`.
8. Effective merge authority and row-level resolver outcome enums exist.
9. Candidate limit fields and fail-closed behavior are represented in schema.
10. The first supported resolver kind is limited to `alias_catalog`.
11. Initial supported normalization functions are declared.
12. Valid and invalid conformance fixtures exist.

## Phase 1: Curated Alias Resolver

### 1.1 Implement Normalization Pipelines

Initial executable support may start with:

1. `identity`.
2. `trim`.
3. `unicode_nfkc`.
4. `unicode_casefold`.

Before the `company_name_gb.v1` conformance fixture is accepted, add:

1. `strip_punctuation`.
2. table-driven `strip_legal_suffix`.
3. `collapse_whitespace`.

Recommended later primitive:

1. `sort_tokens`.

Legal suffix stripping must be table-driven and digest-pinned:

```json
{
  "function_id": "strip_legal_suffix",
  "version": "2026.06",
  "table_id": "gb_legal_suffixes.v1",
  "suffix_table_digest": "sha256:..."
}
```

Do not use locale defaults or mutable process state.

### 1.2 Implement Alias Lookup

Resolver-backed join-key evaluation order:

```text
raw source value
  -> null policy check
  -> resolver normalization pipeline
  -> resolver lookup or miss policy
  -> resolved identity value
  -> canonical COVE logical bytes for the join-key tuple
```

Supported resolver policies:

1. `on_hit: canonical_key`.
2. `on_miss: reject`.
3. `on_miss: normalized_value`.
4. `on_miss: candidate_only`.
5. `on_miss: source_scoped`.

Invalid policy:

1. `on_hit: canonical_label` for identity keys.

`miss_confidence_class` is required for `on_miss: normalized_value` and may be
only:

1. `strong_deterministic`.
2. `weak_deterministic`.

It must not be `authoritative`.

### 1.3 Compute Row-Level Authority

Effective merge authority is per row, not only per identity rule. Add planner
state such as:

1. `PlannedIdentity.effective_merge_class`.
2. `PlannedIdentity.resolution_outcome`.
3. Resolver hit/miss/ambiguous metadata.

Authority matrix:

| Outcome | Effective authority |
| --- | --- |
| alias hit with authoritative resolver | `authoritative` |
| alias hit with strong resolver | `strong_deterministic` |
| `on_miss: normalized_value` | `miss_confidence_class` |
| `on_miss: source_scoped` | `source_scoped` |
| `on_miss: candidate_only` | `candidate_only` |
| ambiguous alias with candidate policy | `candidate_only` |
| ambiguous alias with reject policy | conversion error |

Rules:

1. An authoritative identity rule must not escalate a weaker resolver outcome
   into a global merge.
2. Source-scoped keys do not merge across sources.
3. Candidate-only outcomes do not materialize object merge edges.
4. Encoding source scope into a string is allowed only as an implementation
   detail; evidence must expose the explicit merge class.

### 1.4 Materialize Alias-Hit Output

For authoritative alias hits such as:

```text
Tesco              -> uk-company:tesco
Tesco PLC          -> uk-company:tesco
tesco supermarket  -> uk-company:tesco
```

the identity planner may merge all rows into one COVE-O GOID when the identity
rule allows authoritative auto-merge.

The object can expose:

```text
Company.goid
Company.name = "Tesco"
Company.canonical_key = "uk-company:tesco"
```

Raw observed labels must remain evidence unless redaction policy prevents it.

### 1.5 Emit Resolution Evidence

Authoritative alias-hit evidence should include:

```text
source_id
source_row_identity
rule_id
assertion_id
output_object_id
identity_rule_id
object_type
join_key_sha256
resolver_id
resolver_digest
normalization_pipeline_id
raw_observed_value
normalized_value
canonical_key
canonical_label
alias_catalog_id
alias_entry_id
resolution_kind = "alias_catalog"
alias_hit = true
```

Alias misses and ambiguous aliases must emit miss or ambiguity evidence where
policy allows.

### 1.6 Update Reports And Public APIs

Build reports must include:

1. resolver hit count.
2. resolver miss count by policy.
3. ambiguous alias count.
4. candidate count.
5. reviewed same-object count.
6. do-not-merge count.
7. merge rejection diagnostics.
8. resolver catalog digests.
9. normalizer version and suffix-table digests.

### 1.7 Phase 1 Gate

Phase 1 is complete only when:

1. Alias catalog resolver evaluation is implemented.
2. Miss and ambiguity policies are implemented.
3. Digest validation is applied to embedded and external catalogs.
4. Row-level effective merge class drives union-find grouping.
5. Resolution evidence is emitted.
6. Tesco-style alias merge fixture produces one Company GOID.
7. Ambiguous alias rejection and candidate-only routing are tested.

## Phase 2: Resolution Property Expressions And Explain

### 2.1 Implement Explicit Resolution Expressions

Extend `value_expression` with explicit identity/resolution references:

```text
identity(company_by_resolved_name).resolution(company).canonical_key
identity(company_by_resolved_name).resolution(company).canonical_label
identity(company_by_resolved_name).resolution(company).normalized_value
identity(company_by_resolved_name).resolution(company).raw_observed_value
```

Rules:

1. Do not add implicit global `resolution.*` bindings.
2. One row may emit multiple identities or resolver-backed join keys.
3. The identity rule ID and join-key role ID select the intended resolution
   context.
4. Resolution expressions fail closed when there is no resolver hit unless an
   explicit fallback is declared.
5. Raw observed labels remain evidence even when canonical labels become object
   properties.

### 2.2 Add Explain Support

`cove map explain <mapping.covemap> <goid>` should show, subject to policy:

1. source rows in the identity component.
2. raw observed labels.
3. normalized values.
4. resolver entries used.
5. reviewed equivalence decisions used.
6. canonical anchor.
7. property conflicts and suppressed values.

### 2.3 Add Readback Surfaces

Expose resolution evidence through existing evidence roots and projections.
CoveQL support may stay under `object(...)`, projections, and `evidence(...)`.
A first-class `identity(...)` root is optional future syntax.

Examples that should work through available surfaces:

```text
object(Company)
  .where(name == "Tesco")
  .select(goid, name, canonical_key)
```

```text
evidence(Company, grain: object)
  .where(resolver_id == "uk_company_name_resolver")
  .select(source_id, source_row_identity, raw_observed_value, canonical_label)
```

### 2.4 Phase 2 Gate

Phase 2 is complete only when:

1. Resolution property expressions are parsed and evaluated.
2. Missing resolver-hit behavior fails closed unless fallback is explicit.
3. Explain output includes resolver-backed identity details.
4. A row with two resolver-backed identities proves expression syntax selects
   the intended identity rule and role.
5. Association endpoint resolution works when the endpoint identity rule uses
   an alias resolver.

## Phase 3: Candidate Match Rules

### 3.1 Implement Candidate Rule Schema

Candidate match rules live in `MAP_RESOLUTION_CATALOG.match_rules`. They emit
candidate evidence and review inputs only.

Required semantics:

1. `merge_behavior` must be `never`.
2. Validators reject any field or value that attempts to make candidate scoring
   form merge edges.
3. Inputs are source-aware and may support explicit `source_id: "*"`
   wildcards.
4. Candidate outputs belong in `MAP_CONVERSION_REPORT.candidate_matches` and
   optional evidence metadata.

### 3.2 Implement Deterministic Scoring

Initial scoring should support the proposal's useful minimum, such as
`token_jaccard`, with exact deterministic contracts:

1. tokenization specified by scoring kind.
2. Unicode normalization and case folding from named pipeline.
3. punctuation handling from named pipeline.
4. empty-token behavior specified.
5. score precision and rounding specified.
6. pair ordering stable by source ID, row index, normalized value, then row
   digest.
7. cluster ordering stable by minimum member sort key.
8. duplicate candidate pairs suppressed deterministically.
9. ties ordered deterministically.
10. skipped unsupported rules emit diagnostics.

Limits:

1. `max_pairs_per_block`.
2. `max_pairs_total`.
3. `on_limit: fail_closed`.
4. Optional `on_limit: emit_diagnostic_and_truncate` only when diagnostics make
   truncation explicit and deterministic.

### 3.3 Add Candidate CLI

Add:

```bash
cove map candidates company.covemap suppliers.csv invoices.csv crm.csv \
  --out target/company-candidates.json
```

Output:

1. candidate pairs or clusters.
2. match scores.
3. normalized values.
4. blocking keys.
5. suggested canonical labels.
6. skipped-rule diagnostics.

### 3.4 Candidate Evidence

Candidate evidence should include:

1. left/right source row references.
2. raw values.
3. normalized values.
4. row digests.
5. blocking key.
6. match rule ID.
7. score.

Candidate rows remain evidence only and do not enter GOID merge planning.

### 3.5 Phase 3 Gate

Phase 3 is complete only when:

1. Candidate rules parse and validate.
2. Candidate scoring is deterministic.
3. Candidate limits fail closed in conformance fixtures.
4. Candidate CLI emits stable JSON.
5. Candidate evidence never produces merge edges.

## Phase 4: Reviewed Equivalences

### 4.1 Implement Typed Identity References

Supported identity reference kinds:

1. `identity_join_key`.
2. `resolver_key`.
3. `source_row`.
4. `row_digest`.
5. `identity_alias` only when resolvable to a typed form during validation.

Snapshot-bound `source_row` references require:

1. source ID.
2. source row identity.
3. source snapshot digest.
4. schema fingerprint.
5. object type.
6. identity-rule context.

### 4.2 Implement Reviewed Decisions

Reviewed decisions live in `MAP_RESOLUTION_CATALOG.reviewed_decisions`.

Supported decisions:

1. `same_object`.
2. `do_not_merge`.

Rules:

1. `same_object` decisions may form merge edges only when the identity rule
   declares `allow_reviewed_equivalence: true`.
2. `do_not_merge` decisions are hard constraints.
3. Validation detects conflicts before materialization.
4. Transitive closure is deterministic.
5. Decisions that bridge identity-rule or resolver families must provide
   `canonical_anchor`.

### 4.3 Implement Canonical Anchor Semantics

Rules:

1. Same-rule, same-resolver reviewed decisions may use the current deterministic
   planner anchor sort if all component keys share one identity rule.
2. Cross-rule or cross-resolver decisions require `canonical_anchor`.
3. `canonical_anchor` defines object type, identity rule ID, role components,
   logical types, and resolved values used to build the canonical join-key
   tuple.
4. Changing the canonical anchor is a GOID-changing mapping edit and must be
   reported by `doctor` or `explain`.

### 4.4 Add Review CLI

Add:

```bash
cove map review target/company-candidates.json \
  --out target/company-reviewed-equivalences.json
```

The first implementation may emit and import JSON review files. UI/editor
integration is not required.

### 4.5 Phase 4 Gate

Phase 4 is complete only when:

1. Reviewed same-object decisions contribute merge edges only when allowed.
2. Do-not-merge conflicts reject deterministically.
3. Transitive closure is deterministic.
4. Canonical anchors are required where the proposal requires them.
5. Review-file import/export CLI exists.

## Phase 5: Governance, Redaction, Replay, And Compatibility

### 5.1 Add Governance Metadata

Preserve governance metadata for:

1. source sensitivity.
2. resolver catalog sensitivity.
3. reviewer identity.
4. decision source.
5. effective policy after merge.
6. mixed-sensitivity rejection when configured.

### 5.2 Support Redacted Resolver Evidence

Mappings should support:

1. redacted aliases in public artifacts.
2. digest-pinned private resolver catalogs.
3. evidence policies that prove resolver hits without revealing protected
   aliases.
4. policy-aware query errors when evidence is not visible to the caller.

Rules:

1. Redacted evidence may omit raw values.
2. Redacted evidence must preserve enough digest or resolver-hit proof to
   support replay and explain for authorized readers.
3. Error messages must not leak protected aliases.

### 5.3 Verify Replay Inputs

Replay verification must bind:

1. source file bytes and schema fingerprints.
2. COVE-MAP bytes.
3. `MAP_RESOLUTION_CATALOG` bytes.
4. resolver catalog content or external catalog digest.
5. reviewed decision catalog content or digest.
6. normalization pipeline IDs and function versions.
7. suffix table IDs and digests.
8. match-rule IDs and scoring versions.
9. implementation version for non-trivial candidate scoring.

### 5.4 Compatibility Rules

1. Existing COVE-MAP files remain valid.
2. Readers that do not understand `MAP_RESOLUTION_CATALOG` can still read
   materialized COVE-O output.
3. Resolver metadata is not mandatory for ordinary materialized COVE-O object
   reconstruction.
4. Writers mark the new section required only when the artifact cannot be
   faithfully interpreted without it.
5. Deterministic replay tooling requires the resolver catalog or digest.

### 5.5 Phase 5 Gate

Phase 5 is complete only when:

1. Governance metadata is parsed and preserved.
2. Redacted alias evidence can prove a resolver hit without exposing raw alias
   text.
3. Replay verification fails on changed resolver catalog, normalizer version,
   suffix table digest, or reviewed decision content.
4. Compatibility tests show old COVE-MAP files still pass.

## CLI Surface Checklist

Implement or update:

1. `cove map aliases import company.covemap aliases.csv --catalog-id ... --resolver-id ... --out ...`
2. `cove map build --out-dir target/company-map-build --verify company-with-resolution.covemap ...`
3. `cove map explain company-with-resolution.covemap <goid>`
4. `cove map candidates company.covemap ... --out target/company-candidates.json`
5. `cove map review target/company-candidates.json --out target/company-reviewed-equivalences.json`

CSV alias import columns:

```text
canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
```

## Required Test And Conformance Fixtures

Add unit, integration, and conformance fixtures for:

1. `Tesco`, `Tesco PLC`, and `tesco supermarket` resolve to one Company GOID
   through an authoritative alias resolver.
2. Raw observed values remain visible in evidence.
3. Missing alias with `on_miss: reject` fails conversion.
4. Missing alias with `on_miss: candidate_only` emits candidate evidence and
   does not materialize an object row for that identity path.
5. Missing alias with `on_miss: source_scoped` does not merge across sources.
6. Authoritative identity rule with weaker resolver miss outcome does not
   escalate the miss into a global merge.
7. Same normalized alias in two alias entries fails unless explicitly
   ambiguous.
8. Ambiguous alias never auto-merges, including with `on_miss:
   normalized_value`.
9. Resolver catalog digest changes and replay/explain validation fails.
10. Reordering alias entries or alias values without semantic change does not
    change `catalog_digest`.
11. Changing pipeline function version or suffix-table digest changes
    `pipeline_digest` and `resolver_digest`.
12. Normalizer version or suffix-table digest changes and GOID impact is
    reported.
13. `on_miss: normalized_value` without `miss_confidence_class` rejects.
14. `miss_confidence_class: authoritative` rejects.
15. Reviewed `source_row` references require source snapshot digest, schema
    fingerprint, object type, and identity-rule context.
16. Same row has both a strong ID and an alias ID; canonical anchor selection is
    stable.
17. Reviewed equivalence transitive closure: A=B and B=C produces one
    component.
18. Reviewed conflict: A=B, A do-not-merge C, and B=C rejects
    deterministically.
19. Candidate score tie produces stable ordering.
20. Candidate limit overflow fails closed for conformance fixtures.
21. Redacted alias evidence proves a resolver hit without revealing raw alias.
22. Resolution property expression fails closed when no resolver hit exists
    unless fallback is declared.
23. Row with two resolver-backed identities selects intended identity rule and
    role in expression syntax.
24. Association endpoint resolution works with an alias-backed endpoint
    identity rule.
25. Property conflicts after identity merge still honor `reject_conflict` and
    `source_priority_wins`.

## Open Decisions Before Coding

Resolve before or during the phase that needs the answer:

1. Whether section 69 is allocated as `MAP_RESOLUTION_CATALOG` or the first
   implementation uses a registered extension.
2. Whether external resolver catalogs are allowed in published archives or only
   in build-time `.covemap` inputs with materialized evidence.
3. Minimum candidate scoring set that is useful without large dependencies.
4. Whether CoveQL adds a first-class `identity(...)` root or keeps resolution
   queries under existing object, projection, and evidence surfaces.
5. How private alias catalogs expose proof to unauthorized readers without
   leaking raw aliases.

Resolved implementation decisions:

1. Section ID `69` is allocated as `MAP_RESOLUTION_CATALOG`; the Rust constants,
   registry metadata, embedded COVE-MAP parser, and section-kind registry doc
   all use that core section ID.
2. Published resolver catalogs are embedded and digest-pinned in `.covemap`
   inputs. Live mutable external resolver lookups are not part of deterministic
   replay; alias CSV import materializes catalog content into the mapping.
3. Candidate scoring uses deterministic token/Jaccard matching with stable
   blocking, tie ordering, and fail-closed pair limits.
4. CoveQL keeps resolution access under existing object, projection, and
   evidence surfaces. COVE-MAP projection expressions expose explicit
   `identity(...).resolution(...).field` selectors instead of adding a new
   first-class CoveQL `identity(...)` root.
5. Private alias evidence uses the resolver `evidence_policy` redaction path:
   raw and normalized aliases are omitted while resolver, catalog, and pipeline
   digests remain available as replay/explain proof.

## Proposal Coverage Matrix

Every proposal heading must map to implementation work and evidence:

| Proposal section | Plan coverage | Implementation evidence |
| --- | --- | --- |
| Summary | Objective, Implementation Contract | Resolver-backed planning, materialization, reports, and replay are implemented in `crates/cove-map/src/lib.rs`, `crates/cove-map/src/identity.rs`, and `crates/cove-map/src/replay.rs`; conformance includes `accept/cove_map_resolution_alias_case.json` and `accept/cove_map_replay_verify_case.json`. |
| Recommendation | Phase 0, Phase 1, Cross-feature ordering | `MAP_RESOLUTION_CATALOG` is registered in `crates/cove-core/src/constants.rs`, `crates/cove-core/src/registry.rs`, and `docs/governance/section-kind-registry.md`; resolver-aware delta patches are gated in plan 002. |
| Motivation | Objective, CLI Surface Checklist | CLI surfaces in `crates/cove-map/src/cli.rs` cover `build`, `explain`, `candidates`, `review`, `aliases import`, and `replay verify`; fixtures exercise the Tesco alias workflow. |
| Goals | Implementation Contract, all phase gates | The resolver, candidate, review, projection, and replay suites pass in `cargo test -p cove-map` and the generated conformance corpus. |
| Non-Goals | Non-Goals | Fuzzy/candidate evidence stays non-authoritative unless reviewed; tests such as `alias_catalog_candidate_only_miss_emits_candidate_without_goid` and candidate fixtures prove this fail-closed behavior. |
| Current Baseline | Primary Code Surfaces, Phase 0.3 | Existing COVE-MAP parsing remains in `crates/cove-core/src/profile/cove_map/embedded.rs`; legacy fixtures continue to pass in the full conformance run. |
| Registry Path | Phase 0.1 | Section kind 69 is present in Rust constants, registry metadata, and `docs/governance/section-kind-registry.md`. |
| Terminology | Phase 0.2 data structures, Phase 1 outcome model | Strict structs and enums live in `crates/cove-core/src/profile/cove_map.rs`; row-level outcomes and authority handling live in `crates/cove-map/src/identity.rs`. |
| Design Principles | Implementation Contract, Phase 1 authority rules | Authority ranking, merge classes, do-not-merge checks, and candidate-only paths are enforced in `crates/cove-map/src/identity.rs` and unit-tested in `crates/cove-map/src/lib.rs`. |
| Merge Only From Declared Authority | Phase 1.3, Phase 3, Phase 4 | Candidate-only misses and ambiguous aliases do not form GOIDs; reviewed decisions merge only when identity rules permit reviewed equivalence. |
| Alias Lookup Is Data-Backed Resolution | Phase 0.4, Phase 1.2 | Alias catalogs, pipeline digests, catalog digests, and resolver digests are parsed in `embedded.rs` and evaluated in `identity.rs`; fixtures cover hit, miss, ambiguity, and stale digest cases. |
| Evidence Must Survive Canonicalization | Phase 1.5, Phase 5.2 | Evidence entries carry raw/normalized/resolved values or redacted digest proof through `crates/cove-map/src/ui.rs`; conformance checks raw evidence and redacted evidence. |
| Resolution Must Be Replayable | Phase 0.4, Phase 5.3 | Conversion reports bind source snapshots, resolver/catalog/pipeline digests, and reviewed decision digests; `cove map replay verify` and replay fixtures reject stale bindings. |
| Candidate Generation Is Useful, But Not Truth | Phase 3 | `crates/cove-map/src/candidates.rs` emits deterministic candidate matches and review worklists without GOID authority; candidate conformance fixtures cover scoring, ties, and limits. |
| MAP_RESOLUTION_CATALOG | Phase 0.1, Phase 0.2 | Core section support is in constants, registry, `MapResolutionCatalog`, and the embedded parser; accept/reject resolution catalog fixtures validate it. |
| Canonical Digests | Phase 0.4 | Canonical JSON and digest formulas are enforced by `embedded.rs` and alias import code; tests cover order-insensitive aliases and stale pipeline/suffix-table digests. |
| Identity Rule Extension | Phase 0.3 | `allow_reviewed_equivalence` and join-key `resolution` parsing are implemented in `embedded.rs` and exercised by resolver/review fixtures. |
| Evaluation Order | Phase 1.2 | `identity.rs` evaluates source value, resolver pipeline, alias lookup, miss policy, then authority classification. |
| Hit and Miss Policies | Phase 1.2 | `authoritative`, `candidate_only`, `source_scoped`, `normalized_value`, and reject policies are implemented and covered by generated fixtures. |
| Effective Merge Authority | Phase 1.3 | Effective authority ranking is implemented in `identity.rs`; candidate and miss outcomes cannot escalate authoritative identity rules. |
| Row-Level Merge Class | Phase 1.3 | Merge classes are encoded in planned identities and candidates, with source-scoped and candidate-only behavior verified by tests and fixtures. |
| Evidence Shape | Phase 0.5, Phase 1.5 | Evidence metadata allowlists in `embedded.rs` include resolution and candidate fields; `ui.rs` emits stable evidence/explain JSON. |
| Materialization Semantics | Phase 1.4 | `materialize_with_source_states` in `lib.rs` emits ordinary COVE-O rows and evidence while leaving candidate-only identities out of object truth. |
| Authoritative Alias Hit | Phase 1.4 | Tesco alias fixtures and `alias_catalog_resolver_merges_alias_hits_and_emits_evidence` prove one GOID for authoritative hits. |
| Alias Miss | Phase 1.2, tests | Generated conformance covers reject, candidate-only, source-scoped, and normalized-value miss policies. |
| Ambiguous Alias | Phase 1.2, tests | Generated fixtures reject or route ambiguous aliases to candidates according to policy; unit tests cover both branches. |
| Reviewed Decisions | Phase 4 | `crates/cove-map/src/review.rs` imports/exports reviewed decisions; conversion and replay bind the reviewed decision catalog digest. |
| Deterministic Anchor Semantics | Phase 4.3 | Cross-rule/cross-resolver reviewed decisions require canonical anchors and are tested by cross-rule reviewed fixtures and unit tests. |
| Candidate Match Rules | Phase 3 | Candidate rule schemas parse in `embedded.rs`; deterministic scoring and evidence are implemented in `candidates.rs` and `lib.rs`. |
| Normalization Functions | Phase 1.1 | Built-in lower/trim/Unicode normalization and suffix stripping live in `identity.rs`; suffix table digest fixtures validate replay sensitivity. |
| Property Expressions | Phase 2.1 | Projection expression parsing supports `identity(...).resolution(...).field` selectors in `crates/cove-map/src/project.rs`; missing-hit fixtures fail closed. |
| Association Endpoint Resolution | Phase 2.4 | Alias-backed association endpoint resolution is implemented in projection/materialization paths and covered by unit/conformance tests. |
| CLI Workflow | CLI Surface Checklist, Phases 2-4 | `crates/cove-map/src/cli.rs` and `crates/cove-map/src/ui.rs` expose the planned workflow and usage text. |
| Candidate Discovery | Phase 3.3 | `cove map candidates` calls `candidate_matches` and can write stable JSON for review. |
| Review | Phase 4.4 | `cove map review`, `review export`, and `review import` are implemented in CLI and `review.rs`. |
| Alias Import | CLI Surface Checklist | `crates/cove-map/src/alias_import.rs` imports CSV aliases, updates catalog digests, and fail-closes ambiguous imports. |
| Build | Phase 1.6, CLI Surface Checklist | Build reports include resolver hit/miss counts, GOID impact, resolver digests, reviewed decision counts, and replay digests. |
| Explain | Phase 2.2 | `crates/cove-map/src/ui.rs` returns resolver/evidence details for GOID or assertion IDs. |
| CoveQL Read Surfaces | Phase 2.3 | Resolution data is exposed through materialized COVE-O projection/evidence surfaces and semantic-map fingerprints; no new root is required. |
| Governance and Security | Phase 5.1, Phase 5.2 | Governance policy and redacted resolver evidence paths are enforced in `lib.rs`, `identity.rs`, and generated redaction fixtures. |
| Determinism and Replay | Phase 0.4, Phase 5.3 | `replay.rs` verifies source, resolver, catalog, pipeline, mapping, and reviewed-decision bindings; conformance includes stale resolver, review, and source cases. |
| Compatibility | Phase 5.4 | Existing mappings remain valid; resolver requirements fail closed before materialization; legacy and resolver fixtures pass together. |
| Interaction With COVE-O Delta Artifacts | Cross-feature ordering, plan 002 integration boundary | Plan 002 uses effective semantic-map fingerprints and gates resolver-aware evidence patches behind `DELTA_FEATURE_MAP_EVIDENCE_PATCH`. |
| Error Handling | Phase 0.6 | Stable `MAP_RESOLUTION_*`, `MAP_REPLAY_*`, and map validation diagnostics are emitted by parser, materializer, and replay verifier. |
| Implementation Plan | Phases 0-5 | Phase work is implemented across `cove-core`, `cove-map`, CLI, and conformance surfaces listed above. |
| Phase 0 | Phase 0 | Registry, schemas, digest formulas, allowlists, and diagnostics are implemented and tested. |
| Phase 0: Schema and Registry Integration | Phase 0 | `MapResolutionCatalog` parses under section 69 with strict unknown-field validation. |
| Phase 1 | Phase 1 | Alias resolver evaluation, evidence, reports, and materialized COVE-O output are implemented. |
| Phase 2 | Phase 2 | Projection expressions and explain output expose resolver fields with fail-closed missing-hit behavior. |
| Phase 2: Resolution Property Expressions and Explain | Phase 2 | `project.rs` and `ui.rs` implement the expression and explain surfaces; fixtures cover role selection and missing hits. |
| Phase 3 | Phase 3 | Candidate rules, deterministic scoring, CLI output, and candidate evidence are implemented. |
| Phase 4 | Phase 4 | Reviewed decisions, canonical anchors, import/export, replay binding, and conflict rejection are implemented. |
| Phase 5 | Phase 5 | Governance metadata, redacted proof, replay verification, and compatibility behavior are implemented. |
| Phase 5: Governance and Redaction | Phase 5 | Redacted alias evidence conformance proves digest-backed proof without raw alias disclosure. |
| Test Plan | Required Test And Conformance Fixtures | The generated corpus includes COVE-MAP resolver, candidate, review, projection, redaction, and replay fixtures; `cargo test -p cove-map` covers unit behavior. |
| Example End-to-End Mapping Sketch | Phase 1 Tesco fixture, CLI Surface Checklist | The Tesco/company alias fixtures and CLI commands implement the end-to-end sketch. |
| Open Questions | Open Decisions Before Coding | Resolved decisions are recorded above and reflected in code/fixtures. |
