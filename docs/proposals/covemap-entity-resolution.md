# COVE-MAP Resolver Catalog and Entity Resolution

Status: draft proposal

Owning profiles: COVE-MAP / COVE-O / CoveQL

Related documents:

- [COVE-MAP JSON Schema v1](../covemap-json-schema-v1.md)
- [Customer 360 Data-Science Showcase](../customer360-showcase.md)
- [CoveQL: Unified Cove Query Language Profiles](./coveql-query-profiles.md)
- [COVE-O Delta Artifacts](./cove-o-delta-artifacts.md)

## Summary

This proposal extends COVE-MAP with deterministic, auditable entity-resolution
support for messy real-world identifiers and names.

The current COVE-MAP path can already merge rows from multiple source tables
into one COVE-O object when they share an auto-mergeable identity join key. That
is the right foundation, but practical data often arrives with labels such as
`Tesco`, `Tesco PLC`, and `tesco supermarket` rather than a shared company
number, LEI, tax identifier, or stable internal ID.

The proposed improvement is a single COVE-MAP resolver catalog:

- curated alias resolvers that map observed values to canonical entity keys;
- versioned normalization pipelines;
- candidate-match rules for non-authoritative fuzzy evidence;
- reviewed same-object and do-not-merge decisions;
- typed resolver and identity references;
- conversion reports and evidence rows that explain every resolution decision.

The hard rule remains: COVE must not silently guess object identity. Fuzzy or
heuristic matching may suggest candidate evidence, but COVE-O object merging
must only occur from authoritative, strong deterministic, or explicitly reviewed
evidence.

## Recommendation

Start with a small schema migration and then implement curated alias resolvers.

The highest-value first step is not fuzzy matching. It is making aliases such as
`Tesco`, `Tesco PLC`, and `tesco supermarket` replayable, digest-pinned,
evidence-backed inputs to the existing identity planner.

Candidate matching and reviewed equivalence closure should build on the same
resolver/evidence model later.

If this proposal is implemented alongside COVE-O delta artifacts, the resolver
catalog schema, digest model, row-level outcomes, and evidence metadata should
land first. Delta artifacts are not required for entity resolution; they are a
later publication mechanism for incremental resolver-derived object and
evidence changes.

## Motivation

Semantic archives are most valuable when they preserve the meaning of data, not
only its table shape. A user should be able to map several source tables into a
canonical object such as:

```text
Company: Tesco
```

while preserving source-specific observations:

```text
crm_accounts.name       = "Tesco"
supplier_master.name    = "Tesco PLC"
invoice_extract.vendor  = "tesco supermarket"
```

Without first-class resolution support, users must pre-normalize source tables
outside Cove or rely only on strict equality over existing join keys. That works
for clean identifiers, but it weakens Cove as a semantic archive for messy
enterprise, procurement, compliance, customer, and reference data.

Cove should make this workflow native:

1. Ingest source tables as they are.
2. Resolve observed values through deterministic resolvers.
3. Generate explainable candidate matches for non-authoritative signals.
4. Allow curated aliases or reviewed equivalences to authorize merges.
5. Materialize one canonical COVE-O object with source evidence retained.
6. Query objects, properties, aliases, and evidence through CoveQL.

## Goals

- Support deterministic alias-based identity resolution inside COVE-MAP.
- Preserve COVE-O object authority: merged objects must be reproducible from the
  mapping, source data, resolver catalogs, and reviewed decisions.
- Keep fuzzy or heuristic matching out of automatic merge paths by default.
- Record enough evidence to explain why two source rows did or did not share a
  GOID.
- Preserve raw source labels as evidence even when a canonical label is chosen.
- Provide a practical CLI workflow for candidate generation, review, alias
  import, build, and explain.
- Keep existing COVE-MAP files valid without requiring resolution metadata.
- Make the design useful for organizations, people, products, locations, and
  other entity-like object types.

## Non-Goals

- No silent fuzzy auto-merge.
- No live external identity service dependency during deterministic replay.
- No large language model decisions in the authoritative conversion path.
- No global claim that a name string uniquely identifies a real-world entity.
- No mutation of source files.
- No replacement for strong identifiers such as company numbers, LEIs, tax IDs,
  internal IDs, or stable source-system keys.

## Current Baseline

Current COVE-MAP identity planning already supports:

- multiple source files in one build;
- source-specific row semantic rules;
- object identity rules over canonical join-key tuples;
- union-find grouping of matching auto-mergeable identity keys;
- candidate identity rules that emit evidence without GOIDs;
- do-not-merge constraints;
- property conflict policies such as `reject_conflict` and
  `source_priority_wins`.

Current COVE-MAP embedded payloads use the COVE-MAP v2 JSON envelope:

```json
{
  "schema_id": "org.coveformat.covemap.v2",
  "section_id": 62,
  "mapping_id": "...",
  "mapping_version": "..."
}
```

The current parser validates section IDs and nested field sets strictly. This
proposal therefore requires an explicit schema and registry migration before
new resolver fields can be accepted.

## Registry Path

This proposal should add one new COVE-MAP section, not three independent
placeholder sections.

Requested registry entry:

```text
ID: 69
Name: MAP_RESOLUTION_CATALOG
Profile: COVE-MAP
Required feature: FEATURE_SEMANTIC_MAP
Payload encoding: COVE-MAP v2 JSON envelope
```

The embedded section still uses:

```json
"schema_id": "org.coveformat.covemap.v2",
"section_id": 69
```

The payload shape is the resolution-catalog schema for section 69. If the
registry does not allocate section 69, the fallback path should be a registered
extension or digest-pinned `.covemap` companion artifact.

## Terminology

**Observed value**

A raw value found in a source row, such as `Tesco PLC`.

**Normalized value**

A deterministic transformation of an observed value, such as Unicode
normalization, case folding, legal suffix stripping, or whitespace collapse.

**Canonical entity key**

The stable value used in a resolver-backed identity join key after resolution,
such as `uk-company:tesco`. This key drives GOID generation.

**Canonical label**

The display label selected for a resolved object, such as `Tesco`. A label is
not a stable identity key.

**Alias**

A known observed or normalized value that resolves to a canonical entity key.

**Resolver**

A named data-backed resolution unit. A resolver owns a kind, object type,
normalization pipeline, catalog digest or external reference, hit/miss policies,
and evidence semantics.

**Candidate match**

A possible same-object relationship emitted as evidence only. Candidate matches
do not form GOID merge edges.

**Reviewed equivalence**

A durable decision that two typed identity references refer to the same object.
Reviewed equivalences may form GOID merge edges only when the identity rule
explicitly allows reviewed equivalence.

**Do-not-merge decision**

A durable decision that two typed identity references must not be merged, even
if another rule suggests a match.

## Design Principles

### Merge Only From Declared Authority

COVE-MAP should distinguish three classes of evidence:

- **authoritative merge evidence**: stable IDs, curated aliases, or reviewed
  equivalences that the mapping explicitly allows to merge;
- **strong deterministic evidence**: deterministic transforms that may merge
  only when `auto_merge` is explicitly enabled or defaulted by the confidence
  class;
- **candidate evidence**: fuzzy, approximate, or non-authoritative signals that
  never merge unless promoted by a reviewed decision.

### Alias Lookup Is Data-Backed Resolution

Normalization functions can be pure deterministic functions. Alias resolution
is different: it depends on catalog data. A resolver must therefore carry or
pin that data explicitly through embedded catalog entries, external references,
and digests.

### Evidence Must Survive Canonicalization

If `Tesco PLC` resolves to canonical key `uk-company:tesco`, the raw value
`Tesco PLC` must still be visible through evidence unless redaction policy
explicitly prevents it. Canonicalization must not erase the source observation.

### Resolution Must Be Replayable

Deterministic builds must not depend on a mutable external lookup service.
Resolver catalogs, reviewed decisions, normalizer versions, suffix tables,
match-rule versions, and candidate scoring rules must be embedded or referenced
by pinned digest.

### Candidate Generation Is Useful, But Not Truth

Candidate matching can use string similarity, token overlap, blocking keys, or
source-specific heuristics. Those outputs are useful review inputs, but they
enter COVE-O as candidate evidence unless a reviewed or authoritative policy
promotes them.

## MAP_RESOLUTION_CATALOG

`MAP_RESOLUTION_CATALOG` contains named normalization pipelines, resolvers,
candidate match rules, and reviewed decisions.

The catalog is input metadata. Candidate outputs still belong in
`MAP_CONVERSION_REPORT` and `MAP_EVIDENCE_INDEX`; materialized equivalence
output still belongs in `MAP_IDENTITY_EQUIVALENCE_INDEX`.

Example:

```json
{
  "schema_id": "org.coveformat.covemap.v2",
  "section_id": 69,
  "mapping_id": "company-map",
  "mapping_version": "2026.06",
  "normalization_pipelines": [
    {
      "pipeline_id": "company_name_gb.v1",
      "functions": [
        {
          "function_id": "unicode_nfkc",
          "version": "1"
        },
        {
          "function_id": "unicode_casefold",
          "version": "1"
        },
        {
          "function_id": "strip_punctuation",
          "version": "1"
        },
        {
          "function_id": "strip_legal_suffix",
          "version": "2026.06",
          "table_id": "gb_legal_suffixes.v1"
        },
        {
          "function_id": "collapse_whitespace",
          "version": "1"
        }
      ],
      "tables": [
        {
          "table_id": "gb_legal_suffixes.v1",
          "digest": "sha256:...",
          "values": [
            "plc",
            "ltd",
            "limited",
            "llp"
          ]
        }
      ]
    }
  ],
  "resolvers": [
    {
      "resolver_id": "uk_company_name_resolver",
      "kind": "alias_catalog",
      "object_type": "Company",
      "authority": "curated",
      "confidence_class": "authoritative",
      "normalization_pipeline_id": "company_name_gb.v1",
      "on_hit": "canonical_key",
      "on_miss": "candidate_only",
      "catalog_digest": "sha256:...",
      "pipeline_digest": "sha256:...",
      "resolver_digest": "sha256:...",
      "ambiguous_policy": "reject_auto_merge",
      "alias_catalog": {
        "alias_catalog_id": "uk_company_aliases",
        "entries": [
          {
            "alias_entry_id": "company:tesco",
            "canonical_key": "uk-company:tesco",
            "canonical_label": "Tesco",
            "aliases": [
              "Tesco",
              "Tesco PLC",
              "tesco supermarket"
            ],
            "metadata": {
              "jurisdiction": "GB",
              "source": "mapping-author"
            }
          }
        ]
      }
    }
  ],
  "match_rules": [],
  "reviewed_decisions": []
}
```

Rules:

- `resolver_id` must be unique within the mapping.
- `pipeline_id` must be unique within the mapping.
- Pipeline functions must match entries in `MAP_FUNCTION_REGISTRY` by
  `(function_id, version)`. Matched functions must be deterministic and must
  not depend on random, wall-clock, locale-default, network, or mutable external
  state.
- `canonical_key` must be stable within the resolver catalog version.
- `canonical_label` is a display value, not an identity key.
- Alias lookup uses normalized aliases produced by the resolver's pipeline.
- One normalized alias must not map to multiple canonical keys unless the alias
  entry or resolver marks it ambiguous.
- Ambiguous aliases must not auto-merge.
- `catalog_digest` must be computed over the canonical alias catalog or
  external catalog content.
- `resolver_digest` must include `pipeline_digest`; a resolver digest that only
  names `normalization_pipeline_id` without the resolved pipeline digest is
  invalid.
- External catalogs must be referenced by stable URI plus digest, not by a
  mutable URL alone.

## Canonical Digests

Phase 0 must define digest inputs before resolver replay can be implemented.

This proposal uses COVE canonical JSON v1 for resolver digests:

- UTF-8 JSON object payload;
- duplicate keys rejected before digesting;
- object keys sorted by bytewise UTF-8 order;
- arrays preserved in declared order unless the schema declares the array
  semantically unordered;
- no insignificant whitespace;
- strings encoded with deterministic JSON escaping;
- integers and decimal numbers encoded in their parsed canonical JSON form;
- fields explicitly marked `non_semantic_metadata` excluded from semantic
  digests;
- all other metadata participates in the digest.

Digest fields:

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

`catalog_digest` proves the alias data. `resolver_digest` proves the behavior
that used the data, including hit/miss and ambiguity policy. Evidence should
usually carry `resolver_digest`, not just `catalog_digest`, because resolver
behavior is more than the alias table.

Digest ordering rules:

- normalization pipeline arrays preserve declared order because function order
  is semantic;
- alias catalog entries are sorted by `alias_entry_id` by default;
- aliases within an entry are sorted by normalized alias bytes, then raw alias
  bytes;
- candidate outputs are sorted by their declared deterministic output order;
- a resolver may declare `order_sensitive_catalog: true`, but that is not the
  default because alias catalog order normally does not change behavior.

## Identity Rule Extension

Existing identity rules remain valid. Resolver-backed identity rules add an
optional `resolution` object to each join-key component.

This is a schema migration because the current `MapIdentityRule` and
`MapJoinKeyComponent` parsers validate exact field sets. `MapIdentityRule`
currently accepts:

```text
rule_id, object_type, semantic_role, confidence_class, auto_merge,
candidate_only, property_conflicts_declared, function_ids, join_keys
```

The proposed identity-rule shape adds:

```text
allow_reviewed_equivalence?: bool = false
```

`MapJoinKeyComponent` currently accepts:

```text
role_id, source_column, logical_type, canonicalization, null_policy, ordering
```

The proposed shape is:

```text
role_id, source_column, logical_type, canonicalization, null_policy, ordering,
resolution?: MapResolutionBinding
```

Example:

```json
{
  "rule_id": "company_by_resolved_name",
  "object_type": "Company",
  "semantic_role": "subject",
  "confidence_class": "authoritative",
  "auto_merge": true,
  "candidate_only": false,
  "property_conflicts_declared": true,
  "function_ids": [
    "identity"
  ],
  "join_keys": [
    {
      "role_id": "company",
      "source_column": "supplier_name",
      "logical_type": "utf8",
      "canonicalization": "identity",
      "null_policy": "reject",
      "ordering": "declared",
      "resolution": {
        "resolver_id": "uk_company_name_resolver"
      }
    }
  ],
  "allow_reviewed_equivalence": true
}
```

### Evaluation Order

Resolver-backed join keys must use this exact order:

```text
raw source value
  -> null policy check
  -> resolver normalization pipeline
  -> resolver lookup or miss policy
  -> resolved identity value
  -> canonical COVE logical bytes for the join-key tuple
```

For resolver-backed join keys, `canonicalization` must be `identity` or `none`.
The resolver owns normalization. This avoids ambiguous double-normalization such
as applying `company_name_basic` on the identity rule and a second pipeline on
the resolver.

For non-resolver join keys, existing `canonicalization` behavior is unchanged.

### Hit and Miss Policies

Supported resolver policies:

- `on_hit: canonical_key`: use the alias entry's canonical key as the join-key
  value.
- `on_miss: reject`: fail conversion when no resolver match exists.
- `on_miss: normalized_value`: use the normalized value directly. This is
  deterministic but should usually be `strong_deterministic`, not
  `authoritative`.
- `on_miss: candidate_only`: emit candidate/resolution evidence and do not
  materialize an object row for that identity path.
- `on_miss: source_scoped`: produce a source-scoped key that does not merge
  across sources.

`on_hit: canonical_label` is invalid for identity keys because labels are not
stable keys.

`miss_confidence_class` is required when `on_miss` is `normalized_value`.
Allowed values are `strong_deterministic` and `weak_deterministic`.
`miss_confidence_class` must not be `authoritative`.

Example normalized-value miss policy:

```json
{
  "on_miss": "normalized_value",
  "miss_confidence_class": "weak_deterministic"
}
```

## Effective Merge Authority

Resolver-backed identity planning must compute merge authority per resolved
row, not only per identity rule.

The effective merge authority is the most restrictive authority from:

- identity rule confidence class and `auto_merge`;
- resolver confidence class;
- resolver hit or miss outcome;
- reviewed-decision confidence class when a reviewed decision contributes an
  edge;
- ambiguity state.

Authority outcomes:

| Outcome | Effective authority |
| --- | --- |
| alias hit with authoritative resolver | `authoritative` |
| alias hit with strong resolver | `strong_deterministic` |
| `on_miss: normalized_value` | `miss_confidence_class` |
| `on_miss: source_scoped` | `source_scoped` |
| `on_miss: candidate_only` | `candidate_only` |
| ambiguous alias with candidate policy | `candidate_only` |
| ambiguous alias with reject policy | conversion error |

An authoritative identity rule must not escalate a weaker resolver outcome into
a global merge. For example, an authoritative identity rule with
`on_miss: normalized_value` may only globally merge missed rows if the resolver
declares that miss outcome as strong enough and `auto_merge` rules allow it.

### Row-Level Merge Class

The current planner derives merge class from the identity rule. Resolver-backed
planning needs row-level merge class because the same rule can produce:

- a global authoritative merge for an alias hit;
- a source-scoped key for a miss;
- candidate-only evidence for an ambiguous value.

The preferred implementation is to add an effective row-level merge class to
planned identities, for example:

```text
PlannedIdentity.effective_merge_class
PlannedIdentity.resolution_outcome
```

Encoding source scope into the resolved identity string is allowed only as an
implementation detail if evidence still exposes the explicit effective merge
class. The logical model should treat this as planner state, not as a hidden
string convention.

## Evidence Shape

Resolution metadata should be carried in `MAP_EVIDENCE_INDEX` operation
metadata. The parser must explicitly allow these new keys:

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

Redacted evidence may omit raw values, but it must still preserve enough digest
or resolver-hit proof to support replay and explain for authorized readers.

Candidate evidence is pairwise or cluster-based. Candidate entries in
`MAP_CONVERSION_REPORT.candidate_matches` and optional evidence metadata should
include left/right source row references, raw values, normalized values, row
digests, blocking key, match rule ID, and score. Candidate rows remain evidence
only and do not enter GOID merge planning.

## Materialization Semantics

### Authoritative Alias Hit

If an authoritative resolver maps all observed variants to the same canonical
key:

```text
Tesco              -> uk-company:tesco
Tesco PLC          -> uk-company:tesco
tesco supermarket  -> uk-company:tesco
```

then an authoritative identity rule may merge all three rows into one COVE-O
GOID.

The object can expose:

```text
Company.goid
Company.name = "Tesco"
Company.canonical_key = "uk-company:tesco"
```

The source observations remain evidence.

### Alias Miss

If a source row contains `Tesco Express Stores` and the resolver does not match
it, behavior depends on `on_miss`:

- `reject`: fail conversion.
- `normalized_value`: produce a deterministic key from the normalized string.
- `candidate_only`: emit candidate evidence and skip automatic object merge.
- `source_scoped`: produce a source-scoped key.

### Ambiguous Alias

If the same normalized alias maps to multiple canonical keys, automatic merge is
invalid unless the resolver explicitly marks the alias ambiguous and routes it
to candidate-only evidence. Ambiguous aliases must not fall through to
`normalized_value` in a way that creates cross-source auto-merge.

## Reviewed Decisions

Reviewed decisions live inside `MAP_RESOLUTION_CATALOG.reviewed_decisions`.
They use typed identity references, not loose strings.

Example:

```json
{
  "decision_id": "review:000001",
  "decision": "same_object",
  "confidence_class": "reviewed_authoritative",
  "reviewed_by": "mapping-author",
  "reviewed_at": "2026-06-25T00:00:00Z",
  "reason": "Known trading alias for the same retailer",
  "left": {
    "kind": "resolver_key",
    "object_type": "Company",
    "resolver_id": "uk_company_name_resolver",
    "canonical_key": "uk-company:tesco"
  },
  "right": {
    "kind": "identity_join_key",
    "object_type": "Company",
    "identity_rule_id": "company_by_resolved_name",
    "join_key_sha256": "..."
  },
  "canonical_anchor": {
    "kind": "resolved_join_key",
    "object_type": "Company",
    "identity_rule_id": "company_by_resolved_name",
    "components": [
      {
        "role_id": "company",
        "logical_type": "utf8",
        "resolved_value": "uk-company:tesco"
      }
    ]
  }
}
```

Supported identity reference kinds:

- `identity_join_key`: object type, identity rule ID, and join-key SHA-256.
- `resolver_key`: object type, resolver ID, and canonical key.
- `source_row`: source ID, source row identity, source snapshot digest, schema
  fingerprint, and object/identity context.
- `row_digest`: row digest alias.
- `identity_alias`: legacy compact alias, allowed only when it can be resolved
  to a typed form during validation.

Snapshot-bound source row reference:

```json
{
  "kind": "source_row",
  "object_type": "Company",
  "identity_rule_id": "company_by_resolved_name",
  "source_id": "supplier_master",
  "source_row_identity": "supplier_master:42",
  "source_snapshot_digest": "sha256:...",
  "schema_fingerprint": "cove-map-schema-v1:..."
}
```

Durable reviewed decisions should not point at a bare row index without a source
snapshot digest, schema fingerprint, object type, and identity-rule context.

Rules:

- `same_object` decisions may form merge edges only when the identity rule
  declares `allow_reviewed_equivalence: true`.
- `do_not_merge` decisions are hard constraints and must reject conflicting
  merge plans.
- Reviewed decision validation must detect conflicts before materialization.
- Transitive closure must be deterministic.
- Reviewed decisions that bridge different identity-rule or resolver families
  must provide `canonical_anchor`.

### Deterministic Anchor Semantics

Reviewed equivalence edges can join identities that did not share the same join
key originally. GOID stability therefore depends on anchor selection.

For Phase 4, this proposal requires:

- same-rule, same-resolver reviewed decisions may use the current deterministic
  identity planner's anchor sort if all component keys share one identity rule;
- cross-rule or cross-resolver reviewed decisions must declare
  `canonical_anchor`;
- a `canonical_anchor` must define the object type, identity rule ID, role
  components, logical types, and resolved values used to build the canonical
  join-key tuple;
- changing the canonical anchor is a GOID-changing mapping edit and must be
  reported by `doctor` or `explain`.

## Candidate Match Rules

Candidate match rules live in `MAP_RESOLUTION_CATALOG.match_rules`. They emit
candidate evidence and review inputs, not GOID merge edges.

Example:

```json
{
  "match_rule_id": "company_name_similarity",
  "object_type": "Company",
  "inputs": [
    {
      "source_id": "supplier_master",
      "column": "supplier_name"
    },
    {
      "source_id": "crm_accounts",
      "column": "account_name"
    },
    {
      "source_id": "invoice_extract",
      "column": "vendor_name"
    }
  ],
  "blocking": {
    "kind": "normalized_prefix",
    "length": 4
  },
  "normalization_pipeline_id": "company_name_gb.v1",
  "scoring": {
    "kind": "token_jaccard",
    "candidate_threshold": 0.82,
    "merge_behavior": "never",
    "score_scale": 1000000,
    "rounding": "floor"
  },
  "limits": {
    "max_pairs_per_block": 10000,
    "max_pairs_total": 1000000,
    "on_limit": "fail_closed"
  },
  "output": {
    "assertion_kinds": [
      "candidate_match",
      "evidence"
    ]
  }
}
```

Deterministic output contract:

- tokenization must be specified by the scoring kind;
- Unicode normalization and case folding must come from the named pipeline;
- punctuation handling must come from the named pipeline;
- empty-token behavior must be specified;
- score precision and rounding must be specified;
- pair ordering must be stable by source ID, row index, normalized value, then
  row digest;
- cluster ordering must be stable by minimum member sort key;
- maximum candidate limits must be explicit and reported when reached;
- conformance fixtures should use `on_limit: fail_closed`;
- `on_limit: emit_diagnostic_and_truncate` may be allowed only when diagnostics
  make truncation explicit and deterministic;
- ties must have deterministic ordering;
- duplicate candidate pairs must be suppressed deterministically;
- unsupported match rules must emit skipped-rule diagnostics.

Candidate rule inputs are source-aware because column names are source-local.
Source wildcards may be supported explicitly:

```json
{
  "source_id": "*",
  "column": "company_name"
}
```

`merge_behavior` must be `never` for candidate match rules. Validators should
reject any field or value that attempts to make candidate scoring form merge
edges.

The existing non-authoritative suggest path is the right model: suggestions are
authoring hints, not semantic truth.

## Normalization Functions

The current deterministic runner already supports a small finite function set:
identity, trim, lower/casefold variants, Unicode NFC/NFKC, concatenation,
parse helpers, and SHA-256. Entity resolution can add more functions, but they
must be finite, versioned, and replayable.

Recommended primitive functions:

- `collapse_whitespace`
- `strip_punctuation`
- `strip_legal_suffix`
- `sort_tokens`

Recommended named pipelines:

- `company_name_basic.v1`
- `company_name_gb.v1`

Legal suffix stripping should be table-driven:

```json
{
  "function_id": "strip_legal_suffix",
  "version": "2026.06",
  "table_id": "gb_legal_suffixes.v1",
  "suffix_table_digest": "sha256:..."
}
```

`strip_trading_words` is intentionally not recommended for authoritative
identity keys. Removing words such as `supermarket`, `stores`, or `trading` can
collapse genuinely different organizations. If included, it should be
candidate-only or weak-deterministic unless a curated alias or reviewed decision
promotes the result.

## Property Expressions

Identity resolution chooses which source rows share a GOID. Property merging
still follows existing COVE-MAP property conflict policies.

The current `value_expression` grammar supports direct source columns and
`source.<column>`. Resolver-backed properties require a real expression
extension. Avoid implicit global `resolution.*` bindings because one row may
emit multiple identities or resolver-backed join keys.

Recommended explicit expressions:

```text
identity(company_by_resolved_name).resolution(company).canonical_key
identity(company_by_resolved_name).resolution(company).canonical_label
identity(company_by_resolved_name).resolution(company).normalized_value
identity(company_by_resolved_name).resolution(company).raw_observed_value
```

where `company_by_resolved_name` is the identity rule ID and `company` is the
join-key role ID.

Example property binding:

```json
{
  "assertion_id": "company_name",
  "property_id": "name",
  "property_name": "name",
  "source_column": "supplier_name",
  "logical_type": "utf8",
  "value_expression": "identity(company_by_resolved_name).resolution(company).canonical_label",
  "missing_policy": "reject",
  "conflict_policy": "source_priority_wins"
}
```

Rules:

- resolution expressions must fail closed when there is no resolver hit unless
  a fallback is declared;
- fallback behavior must be explicit, for example `?? source.supplier_name` if
  such syntax is later adopted;
- raw observed labels should remain evidence even when canonical labels become
  object properties.

## Association Endpoint Resolution

Association bindings should be able to use resolver-backed identity rules
without special endpoint syntax. If an endpoint expression resolves through
`identity(<identity_rule_id>)`, the endpoint GOID must use the same resolver
hit/miss semantics and evidence fields as object materialization.

Required tests should cover association endpoints where the target identity is
resolved through an alias resolver.

## CLI Workflow

### Candidate Discovery

```bash
cove map candidates company.covemap suppliers.csv invoices.csv crm.csv \
  --out target/company-candidates.json
```

Outputs:

- candidate pairs or clusters;
- match scores;
- normalized values;
- blocking keys;
- suggested canonical labels;
- skipped-rule diagnostics.

### Review

```bash
cove map review target/company-candidates.json \
  --out target/company-reviewed-equivalences.json
```

The first implementation can emit a JSON review file and leave UI/editor
integration for later.

### Alias Import

```bash
cove map aliases import company.covemap aliases.csv \
  --catalog-id uk_company_aliases \
  --resolver-id uk_company_name_resolver \
  --out company-with-resolution.covemap
```

CSV columns:

```text
canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
```

### Build

```bash
cove map build --out-dir target/company-map-build --verify \
  company-with-resolution.covemap suppliers.csv invoices.csv crm.csv
```

The build report should include:

- resolver hit count;
- resolver miss count by policy;
- ambiguous alias count;
- candidate count;
- reviewed same-object count;
- do-not-merge count;
- merge rejection diagnostics;
- resolver catalog digests;
- normalizer version and suffix-table digests.

### Explain

```bash
cove map explain company-with-resolution.covemap <goid>
```

Explain output should show:

- source rows in the identity component;
- raw observed labels, subject to policy;
- normalized values;
- resolver entries used;
- reviewed equivalence decisions used;
- canonical anchor;
- property conflicts and suppressed values.

## CoveQL Read Surfaces

CoveQL should expose resolution evidence through existing evidence roots and
possibly a specialized identity surface.

Examples:

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

```text
identity(Company)
  .where(canonical_key == "uk-company:tesco")
  .select(goid, canonical_key, aliases: count(evidence()))
```

The `identity(...)` root is optional future syntax. The baseline can use
`object(...)`, projections, and `evidence(...)`.

## Governance and Security

Entity resolution can increase data sensitivity because it links records across
systems. The conversion report should preserve governance metadata for:

- source sensitivity;
- resolver catalog sensitivity;
- reviewer identity;
- decision source;
- effective policy after merge;
- mixed-sensitivity rejection if configured.

Alias catalogs may contain personal data or commercially sensitive identifiers.
Mappings should support:

- redacted aliases in public artifacts;
- digest-pinned private resolver catalogs;
- evidence policies that preserve proof without revealing protected aliases;
- policy-aware query errors when evidence is not visible to the caller.

## Determinism and Replay

The following inputs must be part of deterministic replay:

- source file bytes and schema fingerprints;
- COVE-MAP bytes;
- MAP_RESOLUTION_CATALOG bytes;
- resolver catalog content or external catalog digest;
- reviewed decision catalog content or digest;
- normalization pipeline IDs and function versions;
- suffix table IDs and digests;
- match-rule IDs and scoring versions;
- implementation version for any non-trivial candidate scoring.

Authoritative object identity must not depend on non-deterministic ordering,
locale-dependent comparisons, wall-clock time, or mutable external services.

## Compatibility

Existing COVE-MAP files remain valid.

Readers that do not understand `MAP_RESOLUTION_CATALOG` can still read the
materialized COVE-O output. They may not be able to explain resolution-specific
evidence.

The registry entry can belong to `FEATURE_SEMANTIC_MAP` without making resolver
metadata mandatory for ordinary materialized COVE-O object reconstruction. The
resolver catalog is required for operations that need deterministic replay,
resolution-aware explain, candidate/review surfaces, or resolution-specific
evidence readback. Object reconstruction from already materialized COVE-O rows
should not require the resolver catalog unless the selected read surface asks
for mapping-specific resolution metadata.

Writers should only mark the new section required when the COVE-O artifact
cannot be faithfully interpreted without it. Resolver catalogs used only during
conversion can be optional if resolution evidence is materialized into the
output artifact, but deterministic replay tooling should still require the
resolver catalog or digest.

## Interaction With COVE-O Delta Artifacts

This proposal does not require COVE-O delta artifacts. A resolver-aware
COVE-MAP build can materialize a complete `.cove` snapshot with ordinary
COVE-O object records and evidence rows. In that form, ordinary object
reconstruction does not require a delta-aware reader or resolver execution.

Delta artifacts become relevant when a publisher wants to append new
resolver-derived facts without rewriting the full base `.cove`. A future delta
may carry new source-row evidence, resolver-run metadata, identity-equivalence
assertions, reviewed decision outputs, projection invalidations, or object
temporal records produced by resolver-backed identity rules.

The authority boundary remains the same:

- object identity and object state that affect ordinary COVE-O reconstruction
  must be materialized as COVE-O temporal records;
- resolver metadata in a delta may support replay, explain, evidence readback,
  and planning, but must not become the only source of ordinary object truth
  unless a later required profile explicitly grants that authority;
- a delta that changes resolver behavior, alias catalog content, normalizer
  versions, resolver digests, reviewed decisions that contribute merge edges,
  or identity-rule semantics must expose a new effective semantic-map
  fingerprint;
- a delta that only adds rows resolved under inherited mapping and resolver
  semantics may inherit the parent semantic-map fingerprint while adding new
  materialized object/evidence records.

Implementation order with delta artifacts:

1. Implement this proposal's Phase 0 schema and registry work first, including
   section 69 or its extension fallback, canonical digest inputs, evidence
   metadata allowlists, and row-level resolver outcomes.
2. Implement Phase 1 curated alias resolvers and materialize resolver-derived
   output into ordinary full `.cove` snapshots.
3. Implement the core COVE-O delta MVP independently, binding effective
   semantic-map fingerprints but not requiring resolver-specific patch
   sections.
4. Add resolver-aware delta evidence/projection patches only after
   `resolver_digest`, `catalog_digest`, `pipeline_digest`, and resolution
   evidence semantics are stable enough for conformance fixtures.

## Error Handling

Recommended errors:

- `MAP_RESOLUTION_CATALOG_MISSING`: identity rule references a missing resolver.
- `MAP_RESOLVER_UNSUPPORTED`: resolver kind or version is not implemented.
- `MAP_RESOLVER_DIGEST_MISMATCH`: embedded or external resolver digest does not
  match.
- `MAP_CATALOG_DIGEST_MISMATCH`: embedded or external alias catalog digest does
  not match.
- `MAP_PIPELINE_DIGEST_MISMATCH`: normalization pipeline or suffix-table digest
  does not match.
- `MAP_ALIAS_AMBIGUOUS`: one normalized alias maps to multiple canonical keys.
- `MAP_ALIAS_MISS`: alias lookup failed under `on_miss: reject`.
- `MAP_REVIEW_DECISION_CONFLICT`: reviewed decisions conflict.
- `MAP_DO_NOT_MERGE_VIOLATION`: merge plan violates a do-not-merge decision.
- `MAP_CANONICAL_ANCHOR_REQUIRED`: reviewed equivalence requires an explicit
  anchor.
- `MAP_CANDIDATE_RULE_UNSUPPORTED`: candidate rule cannot be executed.
- `MAP_RESOLUTION_NOT_REPLAYABLE`: resolver references unpinned external state.

All errors should include source ID, row identity, rule ID, resolver ID, and
enough context to reproduce the failure.

These named diagnostics require error-code registry work in Phase 0. Until then,
implementations may map them to broad COVE-MAP errors with structured detail,
but publication-quality support should expose stable error codes.

## Implementation Plan

### Phase 0: Schema and Registry Integration

- Add `MapResolutionCatalog = 69` to the section registry if accepted.
- Add stable error-code registry entries for resolution diagnostics.
- Add parser and validation structs for `MAP_RESOLUTION_CATALOG`.
- Extend the COVE-MAP root-key allowlist for the new section.
- Extend `MapIdentityRule` with `allow_reviewed_equivalence: bool = false`.
- Extend `MapJoinKeyComponent` with `resolution: Option<MapResolutionBinding>`.
- Extend the `MAP_IDENTITY_RULE_CATALOG` parser allowlist for
  `allow_reviewed_equivalence` and join-key `resolution`.
- Extend `MAP_EVIDENCE_INDEX` operation metadata allowlists for resolver and
  candidate-pair fields.
- Define canonical JSON digest inputs for `catalog_digest`, `resolver_digest`,
  and `pipeline_digest`.
- Require `resolver_digest` to include `pipeline_digest`.
- Define semantically unordered alias-catalog digest ordering.
- Validate pipeline functions against `MAP_FUNCTION_REGISTRY` by
  `(function_id, version)`.
- Define `miss_confidence_class` and require it for
  `on_miss: normalized_value`.
- Define the effective merge authority matrix.
- Define row-level resolver outcomes for global, source-scoped, singleton, and
  candidate-only planning.
- Define candidate rule limit fields and fail-closed conformance behavior.
- Limit the first supported resolver kind to `alias_catalog`.
- Declare the initial supported normalization functions.
- Reject unknown resolver-backed fields before materialization.
- Add conformance fixtures for valid and invalid resolution catalogs.

Phase 0 acceptance criteria:

1. Section 69 is accepted or an extension fallback is chosen.
2. `MapResolutionCatalog` structs and parser schema are defined.
3. `MapIdentityRule` includes `allow_reviewed_equivalence`.
4. `MapJoinKeyComponent` includes `resolution`.
5. Evidence operation metadata allowlists include resolver and candidate-pair
   fields.
6. `catalog_digest`, `resolver_digest`, and `pipeline_digest` canonical inputs
   are defined.
7. `resolver_digest` includes `pipeline_digest`.
8. Alias catalog digest sorting rules are defined.
9. Pipeline functions validate by `(function_id, version)`.
10. `miss_confidence_class` is defined for normalized-value misses.
11. Effective merge authority matrix is defined.
12. Row-level source-scoped and candidate-only resolver outcomes are defined.
13. Candidate limits and `on_limit` behavior are defined.
14. Initial resolver kind is limited to `alias_catalog`.
15. Initial supported normalization functions are declared.

### Phase 1: Curated Alias Resolver

- Implement resolver normalization pipeline evaluation. Phase 1 may start with
  current built-ins (`unicode_nfkc`, `unicode_casefold`, and `trim`) for the
  first executable fixture, but the `company_name_gb.v1` pipeline requires
  `strip_punctuation`, table-driven `strip_legal_suffix`, and
  `collapse_whitespace` before it can be a conformance fixture.
- Implement alias lookup and miss policies.
- Add digest validation for embedded and external catalogs.
- Add `resolver_digest`, `catalog_digest`, and `pipeline_digest` to evidence
  where applicable.
- Add row-level effective merge class to planned identities.
- Add evidence fields for resolver hit, miss, ambiguity, raw value, normalized
  value, canonical key, and canonical label.
- Add deterministic tests for the Tesco-style merge case.
- Add ambiguous alias rejection tests.

### Phase 2: Resolution Property Expressions and Explain

- Extend `value_expression` with explicit identity/resolution expressions.
- Add projections for resolver/evidence readback.
- Add explain output for resolver-backed GOIDs.
- Add failure behavior when a resolution expression has no resolver hit.

### Phase 3: Candidate Match Rules

- Implement `MAP_RESOLUTION_CATALOG.match_rules`.
- Add `cove map candidates`.
- Emit candidate evidence without object materialization.
- Add deterministic ordering, score rounding, duplicate suppression, and
  skipped-rule diagnostics.
- Add source-aware candidate inputs and pairwise candidate evidence fields.

### Phase 4: Reviewed Equivalences

- Implement typed identity references.
- Add reviewed same-object and do-not-merge validation.
- Add deterministic transitive closure.
- Require canonical anchors for cross-rule or cross-resolver equivalences.
- Add review-file import/export CLI.

### Phase 5: Governance and Redaction

- Add resolution-specific governance metadata.
- Add redacted resolver evidence support.
- Extend doctor and verify to check replayability and policy invariants.

## Test Plan

Required unit and conformance cases:

- `Tesco`, `Tesco PLC`, and `tesco supermarket` resolve to one Company GOID
  through an authoritative alias resolver.
- Raw observed values remain visible in evidence.
- A missing alias with `on_miss: reject` fails conversion.
- A missing alias with `on_miss: candidate_only` emits candidate evidence and
  does not materialize an object row for that identity path.
- A missing alias with `on_miss: source_scoped` does not merge across sources.
- An authoritative identity rule with a weaker resolver miss outcome does not
  escalate the miss into a global merge.
- Same normalized alias in two alias entries fails unless explicitly ambiguous.
- Ambiguous alias never auto-merges, including with `on_miss: normalized_value`.
- Resolver catalog digest changes and replay/explain validation fails.
- Reordering alias entries or alias values without changing normalized alias
  semantics does not change `catalog_digest`.
- Changing a pipeline function version or suffix-table digest changes
  `pipeline_digest` and `resolver_digest`.
- Normalizer version or suffix-table digest changes and GOID impact is reported.
- `on_miss: normalized_value` without `miss_confidence_class` rejects.
- `miss_confidence_class: authoritative` rejects.
- Reviewed `source_row` references require source snapshot digest and schema
  fingerprint, object type, and identity-rule context.
- Same row has both a strong ID and an alias ID; canonical anchor selection is
  stable.
- Reviewed equivalence transitive closure: A=B and B=C produces one component.
- Reviewed conflict: A=B, A do-not-merge C, and B=C rejects deterministically.
- Candidate score tie produces stable ordering.
- Candidate limit overflow fails closed for conformance fixtures.
- Redacted alias evidence can prove a resolver hit without revealing the raw
  alias.
- Resolution property expression fails closed when no resolver hit exists,
  unless a fallback is declared.
- A row with two resolver-backed identities proves explicit property expression
  syntax selects the intended identity rule and role.
- Association endpoint resolution works when the endpoint identity rule uses an
  alias resolver.
- Property conflicts after identity merge still honor `reject_conflict` and
  `source_priority_wins`.

## Example End-to-End Mapping Sketch

```json
{
  "mapping_id": "retailer-company-map",
  "mapping_version": "2026.06",
  "sources": [
    {
      "source_id": "supplier_master",
      "row_identity_rules": [
        "company_by_resolved_name"
      ],
      "source_priority": 10
    },
    {
      "source_id": "invoice_extract",
      "row_identity_rules": [
        "company_by_resolved_name"
      ],
      "source_priority": 20
    },
    {
      "source_id": "crm_accounts",
      "row_identity_rules": [
        "company_by_resolved_name"
      ],
      "source_priority": 30
    }
  ],
  "identity_rules": [
    {
      "rule_id": "company_by_resolved_name",
      "object_type": "Company",
      "semantic_role": "subject",
      "confidence_class": "authoritative",
      "auto_merge": true,
      "candidate_only": false,
      "property_conflicts_declared": true,
      "function_ids": [
        "identity"
      ],
      "join_keys": [
        {
          "role_id": "company",
          "source_column": "company_name",
          "logical_type": "utf8",
          "canonicalization": "identity",
          "null_policy": "reject",
          "ordering": "declared",
          "resolution": {
            "resolver_id": "uk_company_name_resolver"
          }
        }
      ],
      "allow_reviewed_equivalence": true
    }
  ],
  "resolution_catalog": {
    "section_id": 69,
    "normalization_pipelines": [
      {
        "pipeline_id": "company_name_gb.v1",
        "functions": [
          {
            "function_id": "unicode_nfkc",
            "version": "1"
          },
          {
            "function_id": "unicode_casefold",
            "version": "1"
          },
          {
            "function_id": "trim",
            "version": "1"
          }
        ]
      }
    ],
    "resolvers": [
      {
        "resolver_id": "uk_company_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Company",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "company_name_gb.v1",
        "on_hit": "canonical_key",
        "on_miss": "candidate_only",
        "catalog_digest": "sha256:...",
        "alias_catalog": {
          "alias_catalog_id": "uk_company_aliases",
          "entries": [
            {
              "alias_entry_id": "company:tesco",
              "canonical_key": "uk-company:tesco",
              "canonical_label": "Tesco",
              "aliases": [
                "Tesco",
                "Tesco PLC",
                "tesco supermarket"
              ]
            }
          ]
        }
      }
    ]
  }
}
```

This sketch is illustrative. In an actual `.covemap` artifact,
`resolution_catalog` is the payload of section 69, while source, function,
identity, row semantics, evidence, report, and projection data remain in their
existing MAP sections.

## Open Questions

- Should section 69 be allocated as `MAP_RESOLUTION_CATALOG`, or should the
  first implementation use a registered extension with fallback?
- Should external resolver catalogs be allowed in published archives, or only in
  build-time `.covemap` inputs with materialized evidence snapshots?
- What is the minimum candidate scoring set that is useful without pulling in
  large dependencies?
- Should CoveQL add a first-class `identity(...)` root, or keep resolution
  queries under `object(...)`, projections, and `evidence(...)`?
- How should private alias catalogs expose proof to unauthorized readers without
  leaking raw alias values?
