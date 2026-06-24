# CoveQL/Object: Cove Object Query Language

Status: exploratory proposal

Owning profiles: COVE-O / COVE-MAP

Inspired by: COVE-COVERAGE, COVE-I, COVE-E, mapped COVE-O projection readback,
and object-native query planning

## Summary

**CoveQL/Object** is the Cove Object Query Language profile: a read-only,
object-native query surface for canonical COVE-O objects, associations,
evidence, temporal state, and deterministic COVE-MAP projections.

The language gives readers an object-native way to query canonical objects,
associations, evidence, and deterministic projections without starting from SQL
table names.

The goal is not to replace SQL. The goal is to expose the semantic structure
already present in COVE-O and COVE-MAP:

- objects and properties;
- associations between objects;
- temporal object state;
- source evidence and lineage;
- deterministic projected table surfaces;
- proof-safe predicate planning over COVE metadata;
- late materialization into Arrow, JSON, object rows, or SQL tables.

The core design is to parse a friendly object query into a logical plan, lower
that into a physical plan, preserve coded or proof-safe execution for as long
as correctness allows, and materialize values only at explicit boundaries.

CoveQL should start as a focused, immutable, read-only query language. It
should not include scripting, mutation, transactions, user-defined control
flow, or live object-store behaviors.

## Motivation

COVE currently has strong table and projection read paths. Mapped COVE-O files
can already expose deterministic DataFusion tables through projection metadata,
as shown in the [mapped COVE-O DataFusion showcase](../mapped-cove-o-datafusion-showcase.md).

That is useful for SQL engines, but it hides the semantic shape of the file.
Users often want to ask object questions directly:

```text
Give me active Person objects as of CSN 42.
Show the evidence rows that contributed to this object.
Find Customers that have placed Orders.
Project a canonical object surface with association counts.
Explain whether this predicate was answered through coverage/index metadata or
through decoded scan.
```

CoveQL would provide that object surface while still lowering to the same
validated COVE-O, COVE-MAP, COVE-COVERAGE, COVE-I, and DataFusion machinery.

## Goals

- Provide a compact read-only language for object, association, evidence, and
  projection queries.
- Preserve the authority of COVE files. Query acceleration must never change
  file truth.
- Make coded execution explicit and safe. FileCode equality, ExecutionCode
  remapping, collation, nulls, and dictionary scope must be visible to planning.
- Reuse existing COVE-O readback and COVE-MAP projection semantics where
  possible.
- Lower into a compact logical plan that can target multiple backends:
  DataFusion, object readback, direct Arrow output, or future coded physical
  execution.
- Optimize around the COVE-O physical layout: object type catalog, temporal
  segment index, segment-local object type, system columns, property columns,
  temporal blooms, coverage/index sidecars, and projection catalogs.
- Support `EXPLAIN` output that shows pruning proofs, index usage, decode
  boundaries, and fallback reasons.

## Non-Goals

- No mutations. COVE files are immutable archives.
- No transactions or live object-store operations.
- No arbitrary scripting surface such as `let`, `for`, `match`, or imports in
  this read-only query surface.
- No general UDF execution in the coded hot path.
- No hidden cross-file code comparison. Equal integers from unrelated code
  domains are not equal values.
- No promise that every valid CoveQL query is code-executable. Correct
  materialized fallback is allowed, but it must be explainable.

## Design Principles

CoveQL should be an object-first query surface that compiles into explicit
planning layers rather than treating object queries as strings wrapped around
SQL.

The core principles are:

- a fluent object and association syntax that mirrors user intent;
- a real logical plan before backend-specific execution choices;
- a physical plan that distinguishes indexed, coded, VM, and materialized
  execution;
- a focused coded predicate compiler that fails closed when an expression cannot
  be proven code-safe;
- boundary-only materialization, where strings and rich values are decoded for
  output or required semantic operations rather than by default;
- explainability for why a query stayed coded, used metadata, or fell back.

COVE should keep the object query language read-only, archive-aware, and
proof-aware. Broader scripting, mutation, transaction-oriented shapes, and live
object-store assumptions are outside this proposal.

## Existing Implementation Anchors

CoveQL should build on current repo surfaces instead of creating parallel
semantics:

- COVE-O object state readback in
  [`cove_o/readback.rs`](../../crates/cove-core/src/profile/cove_o/readback.rs)
- COVE-MAP projection expression and row emission support in
  [`cove-map/src/project.rs`](../../crates/cove-map/src/project.rs)
- DataFusion projection registration in
  [`projection_provider.rs`](../../crates/cove-datafusion/src/projection_provider.rs)
- Conservative DataFusion filter pushdown classification in
  [`projection_provider/filters.rs`](../../crates/cove-datafusion/src/projection_provider/filters.rs)
- Normative COVE-O and COVE-MAP semantics in
  [`spec.md`](../../spec.md)

## Spec Integration Contract

CoveQL should be an execution/profile surface layered on the existing COVE
standards suite. It must not create a parallel truth model. A conforming
implementation should treat the spec as the authority for bootstrap,
feature requiredness, object reconstruction, mapping/projection readback,
predicate proof, sidecar validity, execution-code mapping, layout planning,
visibility, redaction, and output interop.

Before planning a query, the implementation should build an operation context:

```text
selected operation:
  object reconstruction | association readback | projection readback |
  evidence readback | index-only answer | Arrow export | explain

selected dataset state:
  file digest/footer CRC/length, snapshot id, schema fingerprint,
  COVM state when present, semantic-map/projection version when used,
  visibility overlay, redaction policy, requested temporal cut

selected capabilities:
  COVE-O, COVE-MAP, COVE-COVERAGE, COVE-I/COVX, COVE-E,
  COVE-L, COVE-R, COVE-CACHE, COVE-CX, Arrow/zero-copy
```

The planner should then apply the spec requiredness model:

- unknown header `FileRequired` features reject before query planning;
- unknown section/page/profile/operation-required features reject only when the
  selected CoveQL operation needs that feature;
- unsupported advisory features are ignored;
- optional COVE-I, COVX, COVE-L, COVE-R, and COVE-CACHE metadata never changes
  correctness and must be validated before it can affect planning;
- COVE-MAP metadata is required only for projection, evidence, replay,
  explanation, or mapping-specific readback, not for ordinary materialized
  COVE-O object reconstruction;
- layout, runtime hints, cache state, zero-copy maps, and ExecutionCodes remain
  non-authoritative.

### Profile Integration Matrix

- COVE-Core: bootstrap, sections, dictionary, checksums, and feature scopes.
  Validate before all query planning.
- COVE-O: object catalog, temporal segments, system columns, and
  reconstruction. Object truth and temporal semantics come from COVE-O.
- COVE-MAP: projection, association semantics, evidence, and deterministic
  functions. Required only when those surfaces are requested.
- COVE-COVERAGE: predicate normal forms, coverage sets, and proof records. Use
  only validated conservative proof for pruning.
- COVE-I/COVX: value, path, association, projection, and index-only access.
  Validate snapshot, schema, semantic map, visibility, exactness, and proof.
- COVE-E: FileCode to ExecutionCode mapping. Acceleration only; enforce
  comparison scope and epoch.
- COVE-L: layout plans, scan splits, page clusters, and zero-copy maps.
  Scheduling and range planning only; not schema or proof.
- COVE-R: runtime/session capability hints. Optional capability discovery;
  never logical truth.
- COVE-CACHE: runtime/local coverage reuse. Snapshot-bound planning cache;
  never canonical truth.
- COVE-CX: registered codecs and kernel capabilities. Required only for
  selected pages or kernels; fallback must be exact.
- Interop: Arrow, DataFusion, JSON, and object output. Output is a boundary;
  preserve COVE null, dictionary, and visibility rules.

## Definitions

This proposal uses the following terms:

- Object record: one materialized COVE-O temporal record, including system
  columns, record kind, optional `prev_ref`, and property values.
- Object state: the reconstructed visible state of one object at a temporal
  cut after applying baselines, snapshots, deltas, tombstones, branch scope,
  visibility, and redaction.
- Association record: a COVE-O object record whose type and property flags
  declare association or link semantics.
- Association state: the reconstructed visible state of an association/link
  object at a temporal cut.
- Evidence row: one evidence or provenance assertion exposed by COVE-MAP
  evidence metadata or materialized evidence objects.
- Projection row: a deterministic row produced by a COVE-MAP projection rule
  from object, association, evidence, temporal, and function inputs.
- Root surface: the initial query source, such as an object type,
  `association(...)`, `evidence(...)`, or `projection(...)`.
- Temporal cut: the selected latest/as-of/history/changes mode and its CSN,
  timestamp, branch, and tombstone policy.
- Branch scope: the branch selector applied during object or association
  reconstruction.
- Visibility overlay: an external or embedded policy that hides rows, objects,
  associations, evidence, values, metadata, or index-only answers.
- Redaction policy: a policy that masks or suppresses values, metadata,
  diagnostics, aggregate answers, or zero-copy exposure.
- Materialization boundary: the point where coded, compressed, or file-native
  values are decoded into object rows, Arrow arrays, JSON, DataFusion batches,
  diagnostics, or another external representation.
- Code domain: the exact context in which a code has equality or ordering
  meaning.
- Execution code domain: an engine-local COVE-E code space with declared
  comparison scope, lifetime, epoch, and null policy.
- Residual predicate: a predicate fragment that could not be proven safe for
  coded, indexed, or coverage execution and must be evaluated later.
- Index-only answer: a result returned from validated metadata or sidecar
  state without scanning the underlying payload rows.
- Fallback boundary: a planned transition from an optimized path to a wider
  scan, decoded predicate, materialized output, or rejection.
- Security scope: the tenant, principal, policy, and disclosure context that
  controls whether values, metadata, diagnostics, aggregates, and buffers may
  be read or revealed.
- Canonical row identity: the stable tie-breaker tuple used to make unordered
  state, record, evidence, projection, and association output deterministic.
- Query fingerprint: a canonical digest of query text, AST, resolved AST,
  predicate form, projection dependency, logical plan, physical plan, or
  explain schema used for conformance, caching, and proof matching.

## Normative Language And Conformance Keywords

The proposal should use explicit conformance language where ambiguity would
otherwise create implementation-defined behavior:

- `MUST`: required for a conforming CoveQL implementation.
- `SHOULD`: expected unless a documented implementation constraint applies.
- `MAY`: optional behavior that must still preserve all visible semantics.
- `Rejected`: parsing, resolution, planning, or execution stops with a
  structured diagnostic and no partial result.
- `Residualized`: an expression remains semantically active but is evaluated
  later with decoded or materialized logical values.
- `Policy-defined`: behavior is selected by a named visibility, redaction,
  disclosure, or resource policy and must be reported in `explain` when
  policy allows.
- `Implementation-defined`: behavior outside the conformance surface. These
  cases should be minimized and explicitly named in diagnostics.

## Query Versioning And Fingerprints

Persisted queries, conformance cases, caches, and explain output should carry
explicit versions:

```text
CoveQlLanguageVersion: 0.1
CoveQlGrammarVersion: 0.1
ResolvedAstVersion: 0.1
ExplainJsonSchemaVersion: 0.1
```

String queries may declare the language version in a leading directive:

```coveo
# coveql:0.1
Person.where(status == "active")
```

APIs should also accept an explicit version parameter so callers do not depend
on a parser default.

Canonical fingerprints should be available for:

- `QueryTextFingerprint`;
- `ParsedAstFingerprint`;
- `ResolvedQueryFingerprint`;
- `PredicateAstFingerprint`;
- `PredicateCnfFingerprint`;
- `ProjectionDependencyFingerprint`;
- `LogicalPlanFingerprint`;
- `PhysicalPlanFingerprint`;
- `ExplainSchemaVersion`.

Fingerprints should ignore harmless whitespace and quoting differences while
preserving semantic changes. They should be used by explain JSON, conformance
tests, COVE-CACHE validation, and COVE-COVERAGE/COVE-I proof matching.

## Dataset Query Scope

A CoveQL query scope is one validated file snapshot unless a dataset
manifest explicitly defines a multi-file snapshot.

For manifest-scoped datasets, the operation context must resolve:

- dataset snapshot id and file membership;
- schema and semantic-map compatibility;
- cross-file object identity and association endpoint rules;
- projection and evidence dependencies across files;
- canonical cross-file ordering;
- code-domain remapping, decoding, or semantic bridges;
- security scope compatibility for every participating file.

Equal raw codes from different files are never equal values unless the plan
proves a shared code domain, remaps into a common execution code domain,
decodes to canonical logical values, or uses an approved semantic bridge.

## Performance Positioning

CoveQL should be designed as a storage-aware query language, not only as a
friendlier syntax for object readback. The parser is the least important part
of the performance story. The important contract is that every query lowers
into a plan that can exploit how COVE-O files are physically structured.

For object-based queries, the fastest path is usually:

```text
resolve object and property ids
  -> prune temporal segments
  -> prune pages/morsels/fragments
  -> scan system columns first
  -> evaluate coded predicates
  -> build a compact candidate row/object set
  -> reconstruct only required object states
  -> decode only selected output values
```

CoveQL should make this path the default. Materialized object records,
`serde_json::Value`, row maps, and string dictionaries are boundary tools, not
the primary execution representation.

## COVE-O Storage-Aware Planning

COVE-O gives the planner several layout facts that a high-performance query
language should exploit directly.

### Object Type Catalog

Object roots such as `Person` should resolve to `object_type_id` during
planning. Property paths should resolve to `property_id`, logical type,
physical kind, nullability, collation id, and property flags before execution.

This lets the planner:

- reject ambiguous paths before scanning;
- scan only temporal segments for the requested object type;
- request only predicate, join, association, grouping, sort, and projection
  property columns;
- use property flags rather than names for association endpoints, evidence
  references, mapping rule references, and validity fields;
- choose typed kernels for NumCode, FileCode, FixedBytes, and nested values.

### Temporal Segment Index

Temporal segment index entries carry `object_type_id`, timestamp range, CSN
range, row count, delta/snapshot/baseline/tombstone counts, GOID min/max, and
byte ranges. CoveQL planning should use those fields before reading segment
payloads.

Examples:

- `Person.asOf(csn: 42)` prunes segments where `csn_min > 42`.
- `Person.changes(from: A, to: B)` intersects the requested interval with
  segment CSN or timestamp ranges.
- `Person.where(goid == X)` prunes by object type and GOID min/max before
  consulting blooms or payload pages.
- Queries that do not need history can avoid reading segments that cannot
  contribute visible latest/as-of state once an exact latest-state sidecar is
  available.

Segment pruning should happen before property decoding, before projection
materialization, and before generic filter evaluation.

### Temporal Bloom Index

When present and valid, temporal blooms should be used for point or narrow
object lookups:

```coveo
Person.where(goid == "...")
Person.where(branch_key == "main" && goid == "...")
Person.asOf(csn: 100).where(goid in [...])
```

Blooms may over-include but must never exclude a possible match. Missing,
corrupt, stale, or unsupported blooms must be ignored.

### System Columns First

Every COVE-O temporal segment has fixed system columns such as branch, GOID,
record id, timestamp, CSN, record kind, and `prev_ref`. Queries should scan
these before property columns whenever possible.

This is the hot path for:

- temporal cuts;
- branch filters;
- GOID lookups;
- latest/as-of reconstruction;
- tombstone exclusion;
- history and changes queries;
- association/link object discovery by object type.

The planner should build a compact candidate set from system columns and only
then touch selected property columns.

### Property Columns

COVE-O property columns use COVE-T physical and encoded-array machinery.
Property values can therefore use the same page/morsel alignment, null bitmap,
FileCode, NumCode, stats-only, constant-page, zone-stat, and dictionary
contracts as table columns.

CoveQL predicates should compile against these columns directly:

- `status == "active"` resolves the literal to the relevant FileCode when the
  dictionary contract allows it;
- `age >= 18` uses a typed numeric comparison over the NumCode lane;
- `email.isNotNull()` uses the validity bitmap;
- `name.startsWith("A")` is dictionary-lifted only when exact dictionary or
  prefix metadata is available, otherwise it becomes a residual decoded
  predicate after more selective filters.

The planner should avoid decoding unreferenced properties. It should also avoid
decoding referenced properties until the candidate row/object set is as small
as possible.

### Association And Link Objects

COVE-O v2 materializes associations as declared association/link object types.
CoveQL association roots and traversals should plan against those object type
and property flags directly:

```coveo
association(CustomerPlacedOrder)
Customer.where(exists(association(CustomerPlacedOrder)))
```

The fast path is:

```text
resolve association object_type_id
  -> resolve from/to endpoint property ids by PROPERTY_FLAG_*
  -> prune temporal segments for the association type
  -> scan endpoint GOID columns as FixedBytes or coded lanes
  -> produce edge row ids, semi-join bitsets, or association aggregates
```

Name-based endpoint fallback is acceptable for compatibility diagnostics, but
it should not be the performance path.

### Projection And Evidence Catalogs

Projection roots should compile through the projection catalog, but the planner
must still push property, evidence, and object-type requirements down into the
COVE-O read. Evidence indexes should be loaded only for evidence roots,
evidence expressions, or projections that actually reference evidence fields.

For simple Arrow projections, the desired path is direct typed builders from
projected fields. Generic JSON row maps are a fallback path.

### Metadata That Unlocks The Fastest Plans

CoveQL should run correctly without optional accelerators, but writers can
make object queries dramatically faster by emitting the right validated
metadata. The planner should prefer these structures when present:

- temporal segment index entries with useful object type, CSN, timestamp, GOID,
  and byte-range bounds;
- temporal bloom indexes for scope, branch, GOID, and time-bucket lookups;
- property columns aligned with system columns by page and morsel;
- page/morsel stats, exact sets, and ColumnDomain metadata for typed
  predicates;
- predicate normal forms and coverage proof records for common object,
  association, evidence, and projection predicates;
- COVE-I roots for object paths, association endpoints, projection fragments,
  semantic dimensions, and high-selectivity property predicates;
- exact aggregate/index-only capabilities for counts, exists, min, max, and
  distinct counts where visibility and redaction allow them;
- COVE-L page clusters and scan split indexes for range-read coalescing and
  distributed scan scheduling;
- COVE-E descriptors when an engine can keep values in an ExecutionCode domain
  for grouping, joins, or output dictionaries;
- zero-copy buffer maps only when the target output can honor COVE layout,
  null polarity, dictionary semantics, visibility, and lifetime.

Each item is optional. Absence should widen the plan, not change results.

## Language Shape

CoveQL uses a focused fluent expression form. A query starts from one of four
root surfaces:

- an object type, such as `Person`;
- an association surface, such as `association(CustomerPlacedOrder)`;
- an evidence surface, such as `evidence(Person)`;
- a named projection, such as `projection(people_projection)`.

Example:

```coveo
Person
  .asOf(csn: 42)
  .where(status == "active" && email.isNotNull())
  .select(
    goid,
    name,
    email,
    evidence_count: count(evidence())
  )
  .orderBy(name)
  .take(50)
```

Association query:

```coveo
Customer
  .where(exists(association(CustomerPlacedOrder)))
  .select(
    customer_goid: goid,
    name,
    order_count: count(association(CustomerPlacedOrder))
  )
```

Association row query:

```coveo
association(CustomerPlacedOrder)
  .select(
    customer: source_goid,
    order: target_goid,
    valid_from,
    valid_to
  )
```

Evidence query:

```coveo
evidence(Person)
  .where(source_id in ["crm", "directory"])
  .select(
    output_object_id,
    source_id,
    source_row_identity
  )
```

Projection query:

```coveo
projection(people_projection)
  .where(name == "Ada")
  .select(person_goid, name)
```

## Language Specification

The public syntax should be precise enough for independent implementations.
The grammar below describes the intended full surface. Implementations may
stage support, but unsupported parsed constructs should fail with structured
diagnostics rather than ambiguous parsing behavior.

```text
Query          := Root MethodChain
Root           := Identifier
                | "association" "(" Identifier RoleArg? ")"
                | EvidenceExpr
                | "projection" "(" Identifier ")"

MethodChain    := Method*
Method         := "." Where
                | "." Select
                | "." AsOf
                | "." Branch
                | "." Tombstones
                | "." History
                | "." Changes
                | "." OrderBy
                | "." Take
                | "." Skip
                | "." GroupBy
                | "." Explain

Where          := "where" "(" Predicate ")"
Select         := "select" "(" SelectItem ("," SelectItem)* ")"
SelectItem     := Identifier ":" Expr | Expr
AsOf           := "asOf" "(" ("csn" ":" UInt | TimeBound) ")"
Branch         := "branch" "(" BranchSelector ")"
Tombstones     := "includeTombstones" "(" Boolean ")"
History        := "history" "(" HistoryArgs? ")"
Changes        := "changes" "(" ChangeBound "," ChangeBound ChangeArgs? ")"
OrderBy        := "orderBy" "(" Expr OrderDirection? NullOrdering? ")"
Take           := "take" "(" UInt ")"
Skip           := "skip" "(" UInt ")"
GroupBy        := "groupBy" "(" Expr ("," Expr)* ")"
Explain        := "explain" "(" ExplainMode? ")"

Predicate      := OrExpr
OrExpr         := AndExpr ("||" AndExpr)*
AndExpr        := NotExpr ("&&" NotExpr)*
NotExpr        := "!" NotExpr | CompareExpr
CompareExpr    := Expr CompareOp Expr
                | Expr "in" "[" Literal ("," Literal)* "]"
                | Expr "." ("isNull" | "isNotNull") "(" ")"
                | "exists" "(" AssociationExpr ")"
                | "(" Predicate ")"

Expr           := Path
                | Literal
                | FunctionCall
                | AggregateCall
                | AssociationExpr
                | EvidenceExpr
                | ConditionalExpr
                | "(" Expr ")"

Path           := Identifier ("." Identifier)*
FunctionCall   := Identifier "(" (Expr ("," Expr)*)? ")"
AggregateCall  := AggregateName "(" ("*" | Expr)? ")"
AssociationExpr := "association" "(" Identifier RoleArg? ")"
                | AssociationDirection "(" "association" "(" Identifier RoleArg? ")" ")"
ConditionalExpr := "if" "(" Predicate "," Expr "," Expr ")"

CompareOp      := "==" | "!=" | "<" | "<=" | ">" | ">="
AggregateName  := "count" | "min" | "max" | "sum" | "avg" | "exists"
                | "distinct_count"
AssociationDirection := "in" | "out" | "either"
RoleArg        := "," ("role" | "from" | "to") ":" Identifier
EvidenceExpr   := "evidence" "(" EvidenceSpec? ")"
EvidenceSpec   := EvidenceTarget ("," EvidenceOption)*
                | EvidenceOption ("," EvidenceOption)*
EvidenceTarget := Path | AssociationExpr | ProjectionTarget | "self"
EvidenceOption := "grain" ":" EvidenceGrain
ProjectionTarget := "projection" "(" Identifier ")"
EvidenceGrain := "object" | "property" | "association" | "row"
                | "column" | "projection" | "node" | "edge" | "path"
                | "source"
BranchSelector := Identifier | StringLiteral | UInt
TimeBound      := TimeRole ":" Timestamp
TimeRole       := "time" | "commit_time" | "valid_time" | "observed_time"
                | "source_event_time" | "association_valid_time"
HistoryArgs    := "mode" ":" HistoryMode
HistoryMode    := "records" | "states" | "records_and_states"
ChangeBound    := ("csn" ":" UInt) | TimeBound
ChangeArgs     := "," "mode" ":" ChangeMode
ChangeMode     := "records" | "state_transitions" | "property_diffs"
                | "final_rows" | "final_objects"
OrderDirection := "," ("asc" | "desc")
NullOrdering   := "," ("nulls_first" | "nulls_last")
ExplainMode    := "public" | "developer" | "proof" | "forensic"
```

Lexical rules:

- Identifiers are case-sensitive unless the referenced catalog declares a
  case-folding rule.
- Reserved method and operator words cannot be unquoted identifiers.
- Quoted identifiers should be supported for object, property, projection,
  association, function, and evidence names that collide with reserved words or
  contain punctuation.
- String, decimal, timestamp, UUID, and binary literals must canonicalize to
  COVE logical values before predicate planning.
- Decimal literals must carry scale and precision through type resolution.
- Timestamp literals must resolve to a declared temporal role, timezone policy,
  and unit before they can be used for pruning or reconstruction.
- Boolean operators use SQL-style three-valued predicate semantics for
  filtering: only TRUE selects.
- Operator precedence is `!`, comparison/null/in/existence, `&&`, then `||`.
- Targetless evidence forms are valid as evidence roots and as contextual
  helpers inside an object, association, projection, or evidence expression
  context. Root-level `evidence()` scans evidence rows at the default object
  grain; contextual `evidence()` binds to the current row grain.

## Resolved AST And Type Resolution

The public AST should preserve user intent. A resolved AST should replace names
with catalog identifiers, types, collations, domains, temporal roles, policies,
and fingerprints.

```text
ResolvedQuery {
  root: ResolvedRoot,
  temporal_mode: TemporalMode,
  branch_mode: BranchMode,
  tombstone_mode: TombstoneMode,
  methods: [ResolvedMethod],
  output_mode: OutputMode,
  operation_context: OperationContext,
  fallback_policy: FallbackPolicy,
  diagnostic_policy: DiagnosticPolicy,
}

ResolvedPath {
  object_type_id: ObjectTypeId,
  property_id: PropertyId?,
  association_type_id: AssociationTypeId?,
  evidence_field_id: EvidenceFieldId?,
  projection_id: ProjectionId?,
  system_field: SystemField?,
  logical_type: LogicalType,
  physical_kind: PhysicalKind,
  null_policy: NullPolicy,
  collation_id: CollationId?,
  code_domain_id: CodeDomainId?,
  deterministic: Determinism,
}
```

Resolution rules:

- Object roots resolve through the COVE-O object type catalog.
- Association roots resolve through object type flags, association/link flags,
  and declared endpoint property flags.
- Projection roots resolve through the COVE-MAP projection catalog.
- Evidence roots resolve through COVE-MAP evidence metadata or materialized
  evidence object types.
- Property paths must resolve to exactly one property, system field,
  association endpoint, evidence field, temporal role, or projection-local
  binding.
- Aliases in `select` affect output names only; they do not create new
  authority for object, association, evidence, or projection truth.
- Function calls must resolve to deterministic, versioned COVE-MAP function
  registry entries before they are usable in persisted projection semantics or
  proof-safe predicate planning.
- Ambiguous paths reject with a structured diagnostic that lists the relevant
  root surface and candidate kinds without leaking protected metadata.

## Root Surfaces

### Object Surface

An object root scans canonical object states for one object type.

```coveo
Person.where(name == "Ada").select(goid, name)
```

The object surface should expose:

- `goid`;
- object type metadata;
- branch and temporal fields when requested;
- declared canonical properties;
- association traversal helpers;
- evidence helpers.

### Association Surface

An association root scans association records or association-like object/link
records declared by COVE-MAP.

```coveo
association(CustomerPlacedOrder).select(source_goid, target_goid)
```

The association surface should expose:

- association type;
- source endpoint;
- target endpoint;
- endpoint roles when declared;
- temporal fields;
- evidence and source provenance when available.

### Evidence Surface

An evidence root scans mapped evidence rows.

```coveo
evidence(Person).where(source_id == "crm")
```

The evidence surface is important because mapped COVE-O's value proposition is
not only canonical object readback. It is canonical object readback with
deterministic lineage.

### Projection Surface

A projection root uses existing COVE-MAP projection metadata.

```coveo
projection(people_projection).select(person_goid, name)
```

This surface is the compatibility bridge to SQL-shaped readback. It should use
the same projection catalog semantics as DataFusion registration.

## Temporal Semantics

CoveQL temporal behavior should be explicit and testable. Temporal
reconstruction is applied before predicates that depend on reconstructed
object state. Predicates over raw records, history rows, or changes operate at
the requested output grain.

- `Person`: latest committed state; default branch only if unique or declared;
  tombstones omitted by default; one row per visible object state.
- `Person.asOf(csn: N)`: records with `csn <= N`; same branch rule as the
  root; tombstones omitted by default; one row per visible object state.
- `Person.asOf(time: T)`: alias for commit/file-ordering time; same branch
  rule as the root; tombstones omitted by default; one row per visible object
  state.
- `Person.asOf(valid_time: T)`, `Person.asOf(observed_time: T)`, and related
  role-specific forms: use the declared temporal role and reject when the root
  or projection has no matching role.
- `Person.history(mode: states)`: all selected records; explicit or scoped
  branch set; tombstones included only when requested; record rows,
  reconstructed state rows, or both by mode.
- `Person.changes(from: A, to: B)`: half-open interval `[A, B)` unless
  explicitly closed; explicit or scoped branch set; configurable tombstone
  behavior; changed object, record, or property-diff rows by mode.

Temporal rules:

- `asOf(csn: N)` is inclusive: records with `csn <= N` are candidates.
- `asOf(time: T)` uses COVE-O commit/file-ordering timestamp by default.
  `commit_time`, `valid_time`, `observed_time`, `source_event_time`, and
  `association_valid_time` select explicit temporal roles.
- Rows with the same timestamp and CSN follow COVE-O segment ordering:
  `(timestamp_us, csn, branch_key, goid, record_id)`.
- When no branch is specified and more than one branch can affect the result,
  the query must either use a declared default branch policy or reject as
  ambiguous.
- Tombstoned objects and associations are omitted by default from state
  surfaces. They can be requested explicitly through `includeTombstones(true)`
  or a history/changes mode that declares tombstone inclusion.
- `history(mode: records)` returns raw records.
- `history(mode: states)` returns reconstructed state after each selected
  record.
- `history(mode: records_and_states)` returns both forms with explicit output
  grain tags.
- `history()` is an alias for `history(mode: states)`.
- `changes(from, to, mode: records)` returns record events.
- `changes(from, to, mode: state_transitions)` returns state transitions.
- `changes(from, to, mode: property_diffs)` returns property-level diffs.
- `changes(from, to, mode: final_rows)` returns final changed rows.
- `changes(from, to, mode: final_objects)` is accepted as a legacy alias for
  `final_rows` and canonicalizes to `final_rows`.
- `changes(from, to)` is an alias for `changes(from, to, mode: records)`.
- `changes` bounds must use the same bound kind. Mixed CSN/time bounds reject
  unless a projection declares an exact conversion rule.
- `prev_ref` validation and reconstruction self-containment are mandatory for
  every state-producing query. Optimized execution may not bypass them.
- Association validity intervals are distinct from COVE-O commit/file-ordering
  time. A query that filters on validity must state that temporal role
  explicitly.

## Association Semantics

Association semantics should not depend on property names alone. The planner
must resolve association types and endpoints from object type flags and
property flags whenever association readback is claimed.

Direction and role rules:

- `association(Type)` as a root scans association/link records of that type.
- Object-relative association expressions must specify direction or role when
  the association type is ambiguous for the current object type.
- `out(association(Type))` means the current object is the declared source or
  from endpoint.
- `in(association(Type))` means the current object is the declared target or
  to endpoint.
- `either(association(Type))` is allowed only when the query explicitly accepts
  either endpoint role.
- If an association type has exactly one valid endpoint role for the current
  root object type, the planner may infer that role and report the inference
  in `explain`.

Counting and existence rules:

- `exists(association(Type))` is a semi-join over reconstructed, visible
  association states at the query temporal cut.
- `count(association(Type))` counts visible association states by default, not
  distinct targets, unless a `distinct_count` or distinct target expression is
  requested.
- Duplicate association records remain distinct unless the association
  identity, temporal mode, and projection rule define a deduplication policy.
- Association tombstones are excluded from state counts by default.
- Visibility and redaction can suppress association existence, target identity,
  endpoint fields, or aggregate answers. The planner must not reveal hidden
  target existence through association traversal, `exists`, counts, or
  diagnostics.

## Evidence Semantics

Evidence queries should be explicit about grain. Evidence is not a single
universal count; it may describe object identity, property values,
associations, source rows, mapping assertions, projection rows, conflicts, or
rules.

The proposed evidence grains are:

- object evidence: evidence that contributed to object identity or object
  existence;
- property evidence: evidence that contributed to a specific property value or
  property assertion;
- association evidence: evidence that contributed to an association/link
  state;
- projection evidence: evidence surfaced by a projection row or projection
  rule;
- source evidence: source row or source assertion provenance.

Examples:

```coveo
evidence(Person, grain: object)
evidence(Person.email, grain: property)
evidence(association(CustomerPlacedOrder), grain: association)
evidence(projection(people_projection), grain: row)
```

Rules:

- Evidence roots return evidence rows at the declared grain.
- `count(evidence(...))` counts evidence rows at the declared grain unless a
  distinct source, source system, assertion, or mapping-rule expression is
  requested.
- Contextual `evidence()` is a shorthand, not a separate evidence grain:
  inside an object query it means `evidence(self, grain: object)`;
  inside an association query it means `evidence(self, grain: association)`;
  inside a projection root it means evidence for the current projection row;
  inside an evidence root it means the current evidence row.
- `evidence(property_name)` inside an object query means
  `evidence(self.property_name, grain: property)`.
- Targetless `evidence(grain: object)` is valid only when the current root
  supplies a contextual target. Otherwise it rejects.
- One source row may produce multiple evidence rows, and one evidence row may
  support multiple objects or properties when the mapping says so.
- Redacted evidence fields must remain hidden in output and diagnostics.
- Evidence existence itself may be protected; when policy hides existence, the
  planner must not expose it through counts, `exists`, or explain metadata.

## Projection Dependency Contract

Every projection root should produce a dependency contract before execution:

```text
ProjectionDependency {
  projection_id,
  projection_version,
  row_grain,
  temporal_mode,
  source_object_types,
  source_association_types,
  source_properties,
  source_evidence_fields,
  deterministic_functions,
  visibility_policy,
  redaction_policy,
  pushdown_safe_fields,
  residual_required_fields,
}
```

`explain` should report:

- projection dependencies loaded;
- projection filters pushed into CoveQL predicates;
- projection filters left as residual predicates;
- projection expressions requiring materialization;
- evidence dependencies loaded;
- deterministic function versions used;
- projection row grain and temporal mode.

Projection rows are read surfaces. They must not redefine object identity,
association identity, temporal history, tombstone state, canonical property
truth, or evidence lineage.

## Core Methods

The core method set is deliberately finite so each method can have a precise
logical and physical contract.

| Method | Applies To | Meaning |
| --- | --- | --- |
| `.where(predicate)` | all roots | Filter rows or object states. |
| `.select(exprs...)` | all roots | Produce an output shape. |
| `.asOf(csn: N)` | object, association, projection | Reconstruct state as of a commit sequence number. |
| `.asOf(time: T)` | object, association, projection | Reconstruct state as of commit/file-ordering time. |
| `.asOf(valid_time: T)` | object, association, projection | Reconstruct state as of an explicit temporal role. |
| `.branch(selector)` | object, association, projection | Select branch scope for reconstruction. |
| `.includeTombstones(bool)` | object, association, history, changes | Select tombstone visibility, subject to policy. |
| `.history()` | object, association | Return state/version history instead of only latest state. |
| `.changes(from: A, to: B)` | object, association | Return changes over a temporal interval. |
| `.orderBy(expr [, asc|desc])` | all roots | Sort output by a valid sort expression. |
| `.take(n)` | all roots | Limit output. |
| `.skip(n)` | all roots | Offset output. |
| `.groupBy(exprs...)` | all roots | Group rows or states. |
| `.explain([mode])` | all roots | Return plan, proof, and materialization diagnostics. |

## Method Chain Semantics

Method chains preserve user order for diagnostics, but planning should resolve
them into a canonical semantic order:

```text
Root
-> branch, tombstone, and temporal mode resolution
-> scan grain selection: state, record, change, projection, or evidence
-> pre-reconstruction filters
-> reconstruction, when state-producing
-> visibility and redaction barriers
-> post-reconstruction filters
-> association/evidence expansion, semi-joins, and anti-joins
-> grouping and aggregation
-> projection/select
-> sort
-> skip/take
-> output or explain
```

Duplicate and conflicting methods must be deterministic:

- multiple `where` clauses are equivalent to one `where` with `&&` in source
  order;
- multiple `select`, `asOf`, `history`, `changes`, `branch`, or
  `includeTombstones` methods reject unless a future profile declares
  replacement semantics;
- `asOf` with `history` or `changes` rejects unless the history/change mode
  explicitly defines a nested temporal cut;
- multiple `orderBy` methods reject; callers should provide one ordered list;
- multiple `take` or `skip` methods reject rather than using last-wins
  behavior;
- `where` before and after `groupBy` is interpreted by canonical order:
  ordinary `where` filters input rows before grouping. Post-aggregate filters
  require a future `having`-style method or an explicit aggregate filter.

The planner may rewrite filters, projections, and ordering only when the
rewritten plan returns the same visible rows, order, diagnostics class, and
policy behavior.

## Result Ordering And Pagination

`take` and `skip` require deterministic ordering. When no `orderBy` is
provided, CoveQL applies a canonical default order so materialized and
accelerated plans return the same page.

Default orders:

- object state surfaces: `(object_type_id, branch_key, goid)`;
- history and record surfaces:
  `(object_type_id, branch_key, goid, timestamp_us, csn, record_id)`;
- association state surfaces:
  `(association_type_id, branch_key, source_goid, target_goid,
  association_goid, csn, record_id)`;
- evidence surfaces:
  `(target_id, grain, source_system, source_row_identity, evidence_id)`;
- projection surfaces: the projection's declared row identity, followed by the
  source canonical row identity when needed.

`orderBy` defaults to ascending order. Null ordering defaults to `nulls_last`
for ascending and `nulls_first` for descending unless a collation policy says
otherwise. Sorts should be stable by appending canonical row identity as a
tie-breaker.

String ordering uses the property's declared collation. If none is declared,
the default is the COVE canonical binary/codepoint collation. Raw FileCode
integer order is never a valid ordering unless the encoding contract declares
it order-preserving for the selected collation and null policy.

## Expression Model

The expression subset should be intentionally finite:

- property references: `name`, `email`, `status`;
- system references: `goid`, `object_type`, `valid_from`, `valid_to`;
- literals: strings, integers, decimals, booleans, timestamps, null;
- comparisons: `==`, `!=`, `<`, `<=`, `>`, `>=`;
- boolean operators: `&&`, `||`, `!`;
- null checks: `isNull()`, `isNotNull()`;
- set membership: `in [...]`;
- association existence: `exists(association(Type))`;
- association aggregate: `count(association(Type))`;
- evidence aggregate: `count(evidence())`;
- deterministic conditionals: `if(condition, then_expr, else_expr)`;
- deterministic functions already defined by COVE-MAP projection semantics.

The expression model should lower to the existing COVE-MAP projection
expression model wherever possible.

## Grouping And Aggregation Semantics

`groupBy` groups the current canonical input grain: object state, association
state, evidence row, projection row, history record, or change event.

After `groupBy`, `select` may contain:

- grouping expressions;
- aggregate expressions;
- deterministic expressions of grouping expressions;
- aliases of the above.

Ungrouped raw property references reject. Aggregate disclosure policy is
checked before index-only, metadata-only, or materialized aggregate results are
returned.

Aggregate rules:

- `count(*)` counts visible input rows.
- `count(expr)` counts visible rows where `expr` is not null.
- `exists(expr)` is true when at least one visible row or association match
  exists and policy allows that existence to be disclosed.
- `sum` and `avg` ignore nulls; all-null groups return null.
- `min` and `max` over strings require a declared collation or materialized
  canonical comparison.
- `distinct_count` counts distinct logical values, not raw FileCodes, unless
  code equality is proven to match logical equality for the selected
  collation, null semantics, and security scope.
- Integer overflow, decimal precision/scale propagation, and `avg` output type
  are resolved by COVE logical type rules and must be reported in diagnostics
  when they affect planning.

Post-aggregate filters are not part of the current method set. A future
`having`-style method should be added rather than overloading `where`.

## Deterministic Function Profile

Every function available to CoveQL should carry metadata:

```text
FunctionContract {
  name,
  version,
  signature,
  return_type_rule,
  null_behavior,
  collation_behavior,
  determinism_class,
  code_safe_class,
  dictionary_liftable,
  materialization_required,
  timezone_or_locale_dependency,
  security_sensitivity,
}
```

The first conformance profile should include:

- `isNull` and `isNotNull`;
- `coalesce` when all alternatives resolve to one compatible logical type;
- safe casts declared by the COVE-MAP function registry;
- `lower` only when Unicode and collation versions are declared;
- `startsWith` as residual materialized evaluation unless an exact accelerator
  exists;
- `length` only after materialization unless encoded metadata proves exact
  logical length semantics.

No deterministic COVE-MAP function should be accepted for proof-safe planning
unless its contract states null, collation, locale/timezone, code-domain, and
materialization behavior.

## Nested, Repeated, Null, And Missing Values

Dotted paths traverse declared object/property bindings and declared nested
struct fields. They do not implicitly iterate arrays or maps. Repeated values
and collections require explicit functions such as `any`, `all`, or
`contains`; otherwise the path rejects as cardinality-ambiguous.

Missing fields and null values are distinct:

| Expression | Result |
| --- | --- |
| `null == null` | UNKNOWN |
| `null != value` | UNKNOWN |
| `value in [null, ...]` | TRUE only if another item matches; otherwise UNKNOWN if null is present. |
| `null in [...]` | UNKNOWN |
| `x.isNull()` | TRUE for present null values. |
| `x.isNotNull()` | TRUE for present non-null values. |
| missing property path | UNKNOWN in predicates; null in nullable projections only when declared. |
| `NaN == NaN` | FALSE unless the logical type declares canonical NaN equality. |
| `NaN` ordering | Requires a declared NaN sort policy; otherwise ordering rejects. |
| decimal overflow | Rejects unless a declared cast or arithmetic rule provides exact behavior. |
| timestamp parse failure | Rejects during parsing or literal canonicalization. |

Only TRUE predicates select rows. FALSE and UNKNOWN do not select rows, but
they remain distinguishable in diagnostics and aggregate expressions where the
logical type requires it.

## Logical Plan

CoveQL should lower into a compact logical plan before choosing an execution
backend.

Every logical plan should carry:

```text
PlanContext {
  snapshot_id,
  semantic_map_version,
  temporal_mode,
  branch_mode,
  visibility_mode,
  redaction_mode,
  output_mode,
  diagnostic_policy,
  fallback_policy,
}
```

Every resolved expression should carry:

```text
ExprContext {
  logical_type,
  physical_kind,
  collation_id,
  null_semantics,
  code_domain_id,
  determinism,
  materialization_requirement,
}
```

```text
ObjectScan {
  object_type,
  plan_context,
}

AssociationScan {
  association_type,
  endpoint_role,
  direction,
  plan_context,
}

EvidenceScan {
  evidence_grain,
  target,
  plan_context,
}

ProjectionScan {
  projection_name,
  dependency_contract,
  plan_context,
}

Filter {
  input,
  predicate,
  placement: pre_reconstruction | post_reconstruction | visibility |
    redaction | residual_materialized,
}

Project {
  input,
  expressions,
}

ExpandAssociation {
  input,
  association_type,
  endpoint_role,
}

SemiAssociation {
  input,
  association_type,
  predicate,
}

Aggregate {
  input,
  group_exprs,
  aggregate_exprs,
}

Sort {
  input,
  sort_exprs,
}

Limit {
  input,
  offset,
  limit,
}

ReconstructState {
  input,
  temporal_mode,
  branch_mode,
  tombstone_mode,
}

ApplyVisibility {
  input,
  visibility_mode,
}

ApplyRedaction {
  input,
  redaction_mode,
}

IndexOnlyAnswer {
  input,
  answer_kind,
  proof_requirements,
}

FallbackBoundary {
  input,
  fallback_reason,
  target_mode,
}

Explain {
  input,
  mode,
}
```

This logical plan should be independent from DataFusion. DataFusion can remain
one backend target, but the object query semantics should not be defined by SQL
translation alone.

## Physical Plan

The physical plan should make representation choices explicit.

```text
ValidateFeatureScopes
BuildOperationContext
ResolveObjectType
ResolvePropertyIds
BuildPredicateNormalForms
ReadObjectCatalog
SelectTemporalSegments
TemporalBloomProbe
ValidateCoverageProofs
CoveragePrune
ValidateCoviOrCovx
CoviLookup
PlanLayoutRanges
RangeReadCoalesce
ReadSystemColumns
ReadPropertyColumns
MorselBitmapEval
FileCodePredicate
ExecutionCodePredicate
NumericPredicate
DictionaryLiftedPredicate
ReconstructObjectState
AssociationLinkScan
AssociationSemiJoin
AssociationAggregate
EvidenceRead
ApplyVisibilityAndRedaction
ZeroCopyArrowProjection
ArrowProjection
JsonProjection
MaterializedFilter
MaterializedSort
```

Every predicate and expression should be classified before execution:

| Class | Meaning |
| --- | --- |
| Code-pure | Can execute directly over codes with no lookup. |
| FileCode-literal | Literal can be resolved into the same file dictionary and compared safely. |
| ExecutionCode-remapped | File-local codes are remapped into an engine-owned execution code space. |
| Numeric-coded | Numeric/date/time encoding preserves the required comparison semantics. |
| Dictionary-lifted | Operation is evaluated once per distinct dictionary value with exact semantics. |
| Coverage-only | Metadata proves inclusion/exclusion or exact answer without touching payload. |
| Decode-boundary | Requires materialized logical values. |
| Unsupported | Query is rejected or requires a backend that supports the feature. |

### Physical Operator Contracts

Physical operators should have explicit contracts, not only names. Each
operator definition should state:

- inputs and outputs;
- preconditions and postconditions;
- whether it can change cardinality;
- whether it can change ordering;
- whether it may inspect protected metadata;
- whether it may run before visibility/redaction;
- whether it can produce an index-only answer;
- fallback and rejection behavior;
- required `explain` fields.

Example contract:

```text
FileCodePredicate
  input:
    property code lane, null bitmap, resolved literal code or code set
  output:
    candidate bitmap or selection vector
  preconditions:
    literal resolved in the same CodeDomainId, or remapped to a proven common
    ExecutionCodeDomainId; null bitmap semantics known; equality semantics
    match the logical type and collation
  postconditions:
    selected rows are exactly rows where the predicate is TRUE, or the operator
    returns a residual decoded predicate
  cardinality:
    narrows candidates
  ordering:
    preserves input order
  fallback:
    unresolved literal, unsafe domain, unsupported collation, or missing null
    semantics becomes residual decoded predicate
  explain:
    code_domain_id, literal_resolution_status, null_policy,
    residual_status
```

Example contract:

```text
IndexOnlyAnswer
  input:
    validated exact COVE-I/COVX capability, predicate form, visibility and
    redaction context
  output:
    scalar answer or rejection/fallback
  preconditions:
    exactness, snapshot, schema, semantic-map version, predicate form,
    collation, null semantics, visibility overlay, and redaction policy all
    validate
  fallback:
    materialized filtered execution or rejection when policy forbids disclosure
  explain:
    index_root, capability, exactness, proof_strength, visibility_status,
    fallback_reason
```

## Hot Path Execution Model

The physical executor should use columnar, compact intermediates. A logical
object query should not imply an allocation-heavy object-per-row runtime.

Preferred hot-path data structures:

- sorted or coalesced segment byte ranges;
- morsel-level bitmaps for candidate rows;
- selection vectors for sparse candidates;
- fixed-width slices for system columns;
- FileCode and NumCode slices for property predicates;
- validity bitmaps for null checks;
- segment-local code bitsets for small `IN` and equality predicates;
- dense row ids and object-state slots;
- compact `(object_type_id, branch_key, goid)` keys for reconstruction;
- direct Arrow builders for final output.

Avoid these in hot loops:

- `serde_json::Value`;
- `BTreeMap` or string-keyed maps per row;
- property-name comparisons;
- dictionary lookups per row;
- virtual dispatch for predicate evaluation;
- heap allocation while scanning rows;
- cloning property values before the final projection boundary.

The query compiler should specialize common plan shapes into small static
kernels. Examples:

```text
ObjectType + asOf(csn) + FileCode equality + direct Arrow projection
ObjectType + GOID lookup + latest state
AssociationType + endpoint GOID semi-join
Evidence source filter + direct evidence Arrow projection
```

Generic expression evaluation should remain available, but it should sit behind
specialized kernels in the plan-choice order.

## End-To-End Execution Pipeline

A fully integrated implementation should use this pipeline:

1. Bootstrap-validate the file or dataset manifest.
2. Build the feature-scope table and select the CoveQL operation.
3. Validate only the required profile, section, page, and operation features
   for that query.
4. Build the snapshot, visibility, redaction, semantic-map, and temporal-cut
   context.
5. Resolve object types, association types, property ids, projection ids,
   evidence fields, collations, logical types, and physical kinds.
6. Lower surface predicates into predicate normal forms: AST first, then CNF,
   interval, or encoded forms when useful and safe.
7. Validate coverage providers, coverage sets, and proof records against the
   selected snapshot and predicate form.
8. Validate optional COVE-I/COVX sidecars, exact/index-only capabilities, and
   sidecar freshness before using them.
9. Validate COVE-L layout, scan split, page cluster, and zero-copy metadata
   only for scheduling, coalescing, or compatible output.
10. Build a conservative physical plan from segment, morsel, page, row-range,
    object, association, path, projection-fragment, and byte-range candidates.
11. Execute system-column and coded property kernels into bitmaps or selection
    vectors.
12. Reconstruct only the object states needed for the selected temporal mode.
13. Apply visibility and redaction before returning rows, exact aggregates,
    index-only answers, or zero-copy views.
14. Materialize output values only at the requested object, Arrow, DataFusion,
    JSON, or diagnostic boundary.

This order matters. Optional metadata can narrow work only after it validates
for the selected operation; it cannot change the logical decode or
reconstruction rules.

## Planner Rewrites For Performance

CoveQL should allow users to write object-oriented queries while the planner
applies relational and storage-aware rewrites.

Safe rewrites include:

- push `.where(...)` below `.select(...)` when expressions permit;
- split predicates into system-column, property-column, association, evidence,
  and residual components;
- run object-type, branch, temporal, GOID, and tombstone filters before
  property filters;
- push `take(n)` into execution only when ordering and temporal semantics make
  early stop correct;
- rewrite `exists(association(T))` into an association semi-join;
- rewrite `count(association(T))` into an association aggregate over endpoint
  keys;
- collapse repeated property references into one decoded or coded access;
- use projection dependency analysis to load only required object types,
  properties, evidence keys, and association object types;
- use COVE-I, COVX, coverage, temporal blooms, and layout metadata only when
  their validity records match the requested file snapshot and operation.

Unsafe or invalid rewrites must be forbidden:

- do not use raw FileCode integer order for `orderBy`;
- do not compare FileCodes across files, properties, dictionaries, tenants, or
  collations without an explicit bridge;
- do not skip `prev_ref` chain validation to speed up reconstruction;
- do not apply `limit` before a filter, association traversal, aggregate, or
  sort unless the logical result is unchanged;
- do not treat projected table rows as more authoritative than object,
  association, temporal, tombstone, or evidence truth.

## Coded Execution Rules

CoveQL must follow the same representation discipline as COVE itself.

Every coded predicate, join, grouping key, distinct key, and ordering key
should carry an explicit domain descriptor.

```text
CodeDomainId {
  file_digest,
  snapshot_id,
  dictionary_id,
  object_type_id?,
  property_id?,
  logical_type,
  physical_kind,
  collation_id?,
  encoding_node_id?,
  semantic_domain_id?,
  dictionary_epoch?,
}

SecurityScopeId {
  tenant_id?,
  principal_scope?,
  visibility_policy,
  redaction_policy,
  metadata_disclosure_policy,
}

ExecutionCodeDomainId {
  engine_profile_id,
  code_space_id,
  comparison_scope,
  lifetime,
  epoch,
  null_code_policy,
  semantic_domain_id?,
}
```

Every coded comparison must prove one of:

- both operands share the same `CodeDomainId`;
- both operands have been remapped to a common compatible
  `ExecutionCodeDomainId`;
- both operands have been decoded to canonical logical values;
- a declared semantic-domain bridge proves equivalence for the selected
  snapshot, collation, and null semantics.

Every coded comparison must also prove that the active `SecurityScopeId`
permits consulting the required dictionaries, maps, sidecars, or bridges and
permits disclosing the resulting answer or diagnostic.

Equality over dictionary codes is valid only when both sides share the same
canonical code domain, or after an explicit remap. A planner may compare
`status_file_code == 7` only if `7` was resolved from the relevant dictionary
for that property, file, segment, and snapshot.

Ordering cannot use arbitrary FileCode integer order. `orderBy(name)` requires
materialized values, valid collation ranks, or a sidecar that defines the
required order.

Case-insensitive equality requires folded canonical metadata or an explicit
equivalence map. Raw dictionary identity is not enough unless the dictionary is
declared to use that folded identity.

String functions such as prefix, substring, normalization, and regex require
materialization unless COVE metadata provides an exact accelerator with the same
logical semantics.

Nulls must remain separate from ordinary code values. A null sentinel may be
used internally only when the encoding contract makes collisions impossible and
the logical null semantics remain visible.

Cross-file comparison requires canonical values, ExecutionCode mapping, or a
declared semantic domain bridge. Equal integers from different files are not
equal values.

## Predicate Planning

Predicate planning should proceed in layers:

1. Normalize the predicate into a small internal form.
2. Identify property paths, system fields, association tests, and evidence
   fields.
3. Resolve literal values against the relevant logical type and dictionary
   domain.
4. Ask COVE-COVERAGE, COVE-I, COVX, runtime caches, and segment metadata for
   proof-safe pruning opportunities.
5. Compile any remaining safe parts into coded predicates.
6. Leave unsupported parts as materialized residual predicates.
7. Emit an explainable plan showing which parts ran in which class.

CoveQL's internal predicate form should map onto COVE-COVERAGE predicate
normal forms:

- use `PredicateAst` as the complete canonical predicate representation;
- use `PredicateCnf` for proof composition over `AND` and `OR`;
- use `IntervalPredicateForm` for ranges, `IN`, dimensional buckets, and
  coverage-cache containment;
- use `EncodedPredicateForm` only when the encoding, codec, logical type,
  collation, null semantics, and kernel capability declare exact equivalence
  or conservative no-false-negative behavior;
- store literals as canonical logical values, never display strings, raw
  FileCodes, source bytes, or engine-local ExecutionCodes.

Coverage integration should follow the spec's conservative proof model:

- use coverage sets only when proof strength is `ExactTight`,
  `ExactConservative`, or an understood no-false-negative
  `ProbabilisticConservative`;
- require coverage proof records for pruning or index-only answers when
  available, and verify their predicate form, provider, coverage set checksum,
  proof kind, proof strength, exactness, null semantics, and snapshot validity;
- treat advisory, engine-local, approximate-may-under-include, stale, corrupt,
  unsupported, or mismatched coverage as a planning hint at most;
- for `A AND B`, intersect coverage only when granularity and proof semantics
  are compatible;
- for `A OR B`, union compatible conservative coverage sets;
- for `NOT`, nullable predicates, NaN-sensitive predicates, or unknown
  functions, default to `Unknown` for pruning unless the provider proves safe
  complement semantics.

Example:

```coveo
Person.where(status == "active" && name.startsWith("A"))
```

Possible plan:

```text
ObjectScan(Person)
  -> CoveragePrune(status == "active")
  -> FileCodePredicate(status == active_code)
  -> DictionaryLiftedPredicate(name.startsWith("A")) if exact prefix sidecar exists
  -> MaterializedFilter(name.startsWith("A")) otherwise
```

## Output Modes

CoveQL should support several output modes:

- object rows, preserving object identity and typed property values;
- association rows;
- evidence rows;
- Arrow `RecordBatch` output;
- JSON rows for diagnostics and tests;
- DataFusion table provider output for SQL interop.

The output mode is a boundary. It is acceptable and expected for values to
materialize there.

Arrow output should use direct typed builders where possible. Zero-copy Arrow
output is allowed only when the COVE-L zero-copy map and target Arrow layout
agree on buffer role, endianness, compression state, offset width, dictionary
semantics, null bitmap polarity, lifetime, visibility, and redaction. Otherwise
the implementation must materialize Arrow-owned buffers.

DataFusion output should expose COVE catalog and projection authority through a
table provider, not replace it. Pushed filters should lower into CoveQL
predicate forms and then follow the same proof and residual-predicate rules as
native CoveQL queries.

## Streaming And Cancellation

Large result sets should use explicit streaming semantics rather than implicit
partial output.

Streaming rules:

- batch size and output mode are operation-context fields;
- ordering guarantees apply across batches, not only within a batch;
- visibility, redaction, feature-scope validation, and sidecar validation must
  complete before the first batch is emitted;
- partial results are forbidden unless the caller selected an explicit
  streaming mode that marks partial output as non-final;
- cancellation must stop execution at a batch or operator boundary, release
  scratch buffers, write any required audit event, and avoid returning a
  misleading final count;
- `explain` may be emitted before execution only when it is marked as a plan
  explanation, not as proof that the execution completed.

## Explain Output

`explain()` should be a first-class feature, not a debugging afterthought.

Example:

```coveo
Person
  .where(status == "active")
  .select(goid, name)
  .explain("proof")
```

Possible output:

```text
logical:
  Project(goid, name)
  Filter(status == "active")
  ObjectScan(Person, latest)

physical:
  ValidateFeatureScopes(operation = object_reconstruction)
  BuildOperationContext(snapshot, visibility, redaction, temporal = latest)
  ResolveObjectType(Person -> 17)
  ResolvePropertyIds(status -> 4, name -> 9)
  BuildPredicateNormalForms(PredicateAst, PredicateCnf, EncodedPredicateForm?)
  ReadObjectCatalog
  SelectTemporalSegments(object_type = 17, latest)
  ReadSystemColumns(branch_key, goid, timestamp_us, csn, record_kind, prev_ref)
  ValidateCoverageProofs(predicate_form = 12)
  CoveragePrune(status == "active") -> Unknown
  ReadPropertyColumns(status, name)
  FileCodePredicate(status == 3, dictionary = file:12/property:4)
  ReconstructObjectState
  ApplyVisibilityAndRedaction
  ArrowProjection(goid, name)

decode boundaries:
  name materialized for output

residual predicates:
  none

optional metadata:
  COVE-I: absent
  COVE-L page clusters: used for range coalescing
  COVE-E ExecutionCode: not requested
```

`explain("proof")` should additionally report which metadata was trusted,
which metadata was ignored, and why.

Explain modes should be policy-aware:

- `public`: safe for users without metadata disclosure privileges.
- `developer`: includes resolved ids, fallback reasons, and broad metadata
  categories.
- `proof`: includes predicate forms, proof records, proof strength, exactness,
  and validation outcomes when policy allows.
- `forensic`: includes maximum diagnostics for trusted operators, still subject
  to redaction and protected-metadata policy.

The stable conformance target should be structured JSON. Text explain output is
a rendering of the JSON form.

```json
{
  "schema_version": "0.1",
  "mode": "proof",
  "fingerprints": {
    "query_text": "<digest>",
    "resolved_query": "<digest>",
    "logical_plan": "<digest>",
    "physical_plan": "<digest>"
  },
  "operation_context": {
    "operation": "object_reconstruction",
    "snapshot_id": "<redacted-or-id>",
    "temporal_mode": "latest",
    "visibility_applied": true,
    "redaction_applied": true
  },
  "logical_plan": [],
  "physical_plan": [],
  "resolved_dependencies": [],
  "predicate_forms": [],
  "trusted_metadata": [],
  "ignored_metadata": [],
  "fallbacks": [],
  "decode_boundaries": [],
  "residual_predicates": [],
  "visibility": {
    "applied": true,
    "policy_id_redacted": true
  },
  "redactions": [],
  "warnings": []
}
```

## Diagnostic Schema

Every parse, resolution, planning, execution, fallback, and rejection
diagnostic should use a redaction-aware structured form:

```json
{
  "code": "E_AMBIGUOUS_PATH",
  "severity": "error",
  "message": "Path is ambiguous for the selected root",
  "span": {"start": 14, "end": 20},
  "phase": "resolution",
  "safe_details": {},
  "redacted": true
}
```

Initial diagnostic codes:

- `E_PARSE`;
- `E_UNSUPPORTED_CONSTRUCT`;
- `E_DUPLICATE_METHOD`;
- `E_METHOD_CONFLICT`;
- `E_AMBIGUOUS_PATH`;
- `E_UNKNOWN_OBJECT_TYPE`;
- `E_UNKNOWN_PROPERTY`;
- `E_UNKNOWN_PROJECTION`;
- `E_UNKNOWN_EVIDENCE_GRAIN`;
- `E_AMBIGUOUS_BRANCH`;
- `E_UNSUPPORTED_TEMPORAL_ROLE`;
- `E_UNSUPPORTED_HISTORY_MODE`;
- `E_UNSUPPORTED_CHANGE_MODE`;
- `E_UNSAFE_CODE_DOMAIN`;
- `E_STALE_SIDECAR`;
- `E_CORRUPT_PROOF`;
- `E_RESOURCE_BUDGET_EXCEEDED`;
- `E_SECURITY_DISCLOSURE_FORBIDDEN`;
- `E_AGGREGATE_DISCLOSURE_FORBIDDEN`;
- `E_INDEX_ONLY_FORBIDDEN`;
- `E_ZERO_COPY_FORBIDDEN`;
- `E_DATAFUSION_PUSH_FILTER_UNSAFE`.

Diagnostics must not reveal protected object names, paths, dictionary values,
sidecar existence, row counts, evidence existence, or policy identifiers unless
the active explain and metadata-disclosure policy allows it.

## Relationship To SQL And DataFusion

CoveQL should not be defined as syntax sugar for SQL, but it should be able
to target SQL-shaped projection providers.

Near-term lowering path:

```text
CoveQL
  -> CoveOLogicalPlan
  -> existing COVE-MAP projection/readback options
  -> DataFusion projection provider or direct Arrow output
```

Longer-term lowering path:

```text
CoveQL
  -> CoveOLogicalPlan
  -> CoveOPhysicalPlan
  -> coded/proof-safe operators
  -> Arrow/object/materialized boundary
```

This lets the language become useful before the full coded physical path is
implemented.

### DataFusion Pushdown Contract

DataFusion integration is a backend path, not the semantic authority.

Rules:

- DataFusion filters are advisory until translated into CoveQL predicate
  forms.
- Unsupported filters remain residual DataFusion filters or residual CoveQL
  materialized predicates.
- COVE null, collation, canonicalization, temporal, visibility, and redaction
  semantics win over SQL-engine assumptions.
- COVE-O reconstruction happens before any SQL-level filter or projection that
  depends on object state.
- Projection rows remain deterministic read surfaces, not canonical object
  truth.
- A pushed filter must produce identical visible rows to the same filter left
  unpushed.
- `EXPLAIN` must identify filters received from DataFusion, filters converted
  to CoveQL predicate forms, filters trusted for proof/coded execution, and
  filters left residual.

## Compatibility And Fallback

CoveQL should be an execution profile or library surface, not required
baseline COVE-Core behavior.

A reader that does not support CoveQL can still read the underlying COVE file
through normal COVE-O, COVE-MAP, or table/projection surfaces.

A reader that supports CoveQL but not a specific optional accelerator must
fall back to scanning or materialized evaluation when doing so preserves
semantics. It must fail closed when required metadata is corrupt, stale, or
claims unsafe proof authority.

### Fallback And Rejection Matrix

| Condition | Correct behavior |
| --- | --- |
| Missing optional temporal bloom | Ignore and scan the wider candidate set. |
| Corrupt optional temporal bloom | Ignore, report diagnostic, and scan wider. |
| Stale COVX or COVE-I sidecar | Ignore for ordinary reads; reject only if required by the operation. |
| Unsupported optional COVE-L layout | Ignore for scheduling and use ordinary range planning. |
| Unsupported required codec for selected page | Reject selected operation. |
| Predicate not code-safe but materializable | Keep as residual decoded predicate. |
| Predicate not materializable under security policy | Reject selected operation. |
| Redaction incompatible with index-only count | Materialize filtered rows or reject if disclosure remains unsafe. |
| Unknown operation-required feature used by selected query | Reject selected operation. |
| Unknown advisory feature | Ignore. |
| Coverage proof exactness claimed but snapshot mismatches | Ignore or reject as corrupt; never trust. |
| DataFusion pushed filter cannot be represented safely | Treat as residual outside COVE pushdown. |
| Zero-copy map incompatible with target output | Materialize compatible owned buffers. |
| COVE-CACHE entry stale or engine-incompatible | Ignore cache and replan from validated metadata. |
| ExecutionCode map stale or wrong comparison scope | Rebuild, decode, or reject according to mount policy. |
| Ambiguous branch with no default branch policy | Reject. |
| Duplicate temporal methods, such as two `asOf` calls | Reject. |
| `history()` mode omitted | Use `mode: states` unless a profile disables the default. |
| `changes()` mode omitted | Use `mode: records` unless a profile disables the default. |
| Mixed CSN/time change bounds | Reject unless conversion semantics are declared. |
| Unsupported temporal role | Reject unless equivalent commit-time semantics are proven. |
| Missing required COVE-MAP metadata for projection root | Reject projection query. |
| Missing evidence catalog for evidence root | Reject unless materialized evidence objects are authoritative. |
| Unsupported deterministic function | Reject or residualize only when materialized semantics are available. |
| Unsupported aggregate under redaction policy | Reject or return policy-redacted aggregate. |
| Explain mode exceeds principal privilege | Downgrade only if requested policy allows; otherwise reject. |
| Resource budget exceeded during parse or planning | Reject with structured diagnostic. |
| Resource budget exceeded during execution | Cancel safely; partial results require explicit streaming mode. |

## Security And Privacy Notes

Query planning metadata can reveal information. Dictionaries, coverage maps,
semantic domains, and execution-code maps may leak value existence or equality
relationships.

Security policy should be part of planning, not only output filtering. The
operation context should include:

```text
SecurityContext {
  principal_or_session,
  visibility_policy,
  redaction_policy,
  explain_policy,
  aggregate_disclosure_policy,
  metadata_disclosure_policy,
  resource_budget_policy,
  zero_copy_permission,
  index_only_answer_permission,
}
```

Before consulting or disclosing metadata, the planner should ask:

- may this principal consult this metadata?
- may the plan reveal that this metadata exists?
- may this query return an index-only answer?
- may this query return exact counts or aggregates?
- may this query expose zero-copy buffers?
- may diagnostics include dictionary literals, path names, sidecar names, or
  coverage degrees?

CoveQL should inherit COVE's visibility and redaction rules:

- do not expose redacted dictionary values through explain output;
- do not reveal hidden evidence rows through association/object convenience
  methods;
- respect visibility overlays when interpreting coverage or index-only answers;
- keep ExecutionCode maps runtime-local unless an explicit export policy allows
  otherwise;
- do not return exact aggregate, count, exists, or index-only answers from
  optional metadata unless visibility and redaction policies are proven
  compatible with the selected snapshot;
- do not expose zero-copy buffers when an active visibility overlay or redaction
  policy requires materialized filtering or masking;
- make `explain` output redact dictionary values, predicate literals, path
  names, or sidecar details when policy marks them protected.

Aggregate disclosure policy should have explicit planner outcomes:

| Outcome | Meaning |
| --- | --- |
| `AllowExact` | Exact aggregate, count, `exists`, or distinct answer may be returned. |
| `AllowMaterializedOnly` | Metadata-only answers are forbidden; visible rows must be evaluated. |
| `AllowThresholded` | Answer may be returned only when a threshold or bucket rule is satisfied. |
| `AllowRedacted` | Return a policy marker instead of the value. |
| `Reject` | Do not execute the aggregate expression for this principal/context. |

This policy applies to association counts, evidence counts, grouped
aggregates, `exists`, `distinct_count`, index-only answers, and explain output.
Falling back from an index-only count to materialized counting is allowed only
when the materialized visible-row result is itself safe to disclose.

## Resource Budgets

CoveQL should be bounded before it is exposed to untrusted or multi-tenant
query input.

The operation context should carry limits for:

- maximum query bytes;
- maximum AST depth;
- maximum method count;
- maximum `IN` list size;
- maximum disjunction count;
- maximum output columns;
- maximum groups;
- maximum rows returned without explicit `take`;
- maximum decode bytes;
- maximum range requests;
- maximum planning time;
- maximum execution time.

Budget failures during parsing or planning reject with diagnostics. Budget
failures during execution cancel the query, release resources, and forbid
partial results unless an explicit streaming mode allows them.

## Implementation Plan

The implementation sequence is an ordering rule, not a scope limit. Public
interfaces should stay shaped for the complete language surface while each
phase proves one layer of correctness before the next layer adds more
execution authority.

Roadmap invariants:

- every query builds an operation context and security context before
  consulting optional metadata;
- every phase preserves the canonical materialized result from prior phases;
- optional metadata may reduce work only after validation, never change visible
  rows;
- every coded, indexed, cached, zero-copy, or metadata-only path has an
  explicit fallback boundary or rejection reason;
- `explain` JSON is updated in the same phase that introduces a new planning
  concept or physical operator;
- DataFusion remains a backend integration point, not the semantic authority;
- performance work measures the same logical and physical contracts that
  conformance tests assert.

### Phase 0: Operation Context And Spec Validation

Implement the spec-facing operation layer before the parser becomes large.

Deliverables:

- bootstrap validation and profile capability table construction;
- selected operation kinds for object, association, projection, evidence,
  index-only, Arrow, and explain queries;
- snapshot, semantic-map, branch, tombstone, visibility, redaction, output,
  explain, fallback, and temporal context structs;
- `SecurityContext` construction for visibility, redaction, metadata
  disclosure, aggregate disclosure, index-only answers, and zero-copy
  eligibility;
- resource budget policy for parse, planning, execution, decoding, grouping,
  output, and range requests;
- optional metadata validation hooks for COVE-COVERAGE, COVE-I/COVX, COVE-L,
  COVE-E, COVE-R, COVE-CACHE, and COVE-CX;
- operation-scoped fallback and rejection reporting aligned with the
  fallback/rejection matrix.

### Phase 1: Formal Language And Resolved AST

Add a dedicated parser and resolver crate or module for CoveQL:

```text
coveql
```

Deliverables:

- lexical rules, grammar conformance tests, parser, source spans, and public
  AST;
- language, grammar, resolved AST, and explain schema version fields;
- roots: object, association, evidence, projection;
- methods: `where`, `select`, `asOf`, `history`, `changes`, `orderBy`,
  `take`, `skip`, `groupBy`, `explain`;
- expressions: paths, literals, comparisons, boolean operators, `in`,
  null checks, association/evidence helpers, simple aggregates;
- name, type, function, association-role, evidence-grain, and projection
  dependency resolution;
- literal canonicalizer for string, numeric, boolean, timestamp, decimal, and
  null values;
- resolved AST with ids, logical types, physical kinds, collations,
  `CodeDomainId` placeholders, temporal roles, branch/tombstone modes,
  visibility/redaction references, and diagnostics.
- method-chain conflict detection, default ordering rules, and canonical
  fingerprints for parsed and resolved queries.

### Phase 2: Canonical Logical Plan

Introduce `CoveOLogicalPlan` before relying on any backend execution path.

Deliverables:

- `PlanContext` and `ExprContext` construction from the resolved AST;
- object type, property, association, evidence, and projection dependency
  extraction;
- temporal, branch, tombstone, visibility, and redaction placement;
- logical predicate normalization and source-preserving predicate diagnostics;
- grouping, aggregation, ordering, pagination, null, NaN, missing-value, and
  nested-path semantics;
- pre-reconstruction, post-reconstruction, visibility, redaction, and residual
  filter classification;
- logical nodes for scan, reconstruct, visibility, redaction, association,
  evidence, projection, aggregate, sort, limit, explain, index-only, and
  fallback boundaries;
- projection dependency contracts and deterministic function fingerprints;
- logical plan validation plus text and JSON debug printers.

### Phase 3: Correct Materialized Execution

Lower object/projection/evidence queries to current COVE-O and COVE-MAP
readback options through the logical plan.

Deliverables:

- canonical materialized execution for object, projection, evidence, and
  association roots where the existing readback path can provide them;
- temporal, branch, tombstone, association-role, evidence-grain, and
  projection dependency semantics enforced before optimization;
- direct Arrow output;
- JSON diagnostic output;
- DataFusion registration helper that consumes CoveQL plans rather than
  defining them;
- conformance-style golden outputs for mapped files;
- fallback-invariance tests with optional metadata absent, present, stale,
  corrupt, and unsupported.

### Phase 4: Stable Explain JSON

Make `EXPLAIN` output independent of DataFusion and stable enough for tests.

Deliverables:

- stable JSON explain schema for `public`, `developer`, `proof`, and
  `forensic` policy modes;
- explain schema version and plan/query fingerprints;
- text rendering of JSON explain;
- resolved dependency reporting;
- trusted and ignored metadata reporting;
- fallback and rejection reporting;
- decode-boundary and residual-predicate reporting;
- policy redaction for path names, dictionary literals, sidecar identities,
  coverage degrees, row counts, and protected evidence metadata;
- structured diagnostics with safe details and redaction status;
- storage-aware plan diagnostics for selected object types, property ids,
  temporal cuts, segment filters, and loaded columns.

### Phase 5: Conservative Pushdown

Add storage-aware pruning and pushdown that preserves identical visible rows
with and without optional metadata.

Deliverables:

- temporal segment index pruning;
- temporal bloom probing for point and narrow GOID lookups;
- system-column-first scan kernels;
- property-column pruning;
- validity-bitmap null checks;
- numeric/date/time predicate paths;
- candidate bitmap and selection-vector propagation through materialized
  reconstruction;
- residual predicate handling;
- decode-boundary reporting;
- conformance checks proving that every pushed predicate has the same result as
  the residual materialized predicate.

### Phase 6: Coded And Proof-Safe Physical Planning

Introduce `CoveOPhysicalPlan` with explicit coded, indexed, coverage,
ExecutionCode, and materialized operators.

Deliverables:

- COVE-COVERAGE-compatible predicate normal-form generation;
- coverage proof record validation;
- coverage/index metadata consultation;
- COVE-I/COVX sidecar freshness and capability validation;
- `CodeDomainId` and `ExecutionCodeDomainId` validation;
- FileCode literal resolution and dictionary-lifted predicate planning;
- ExecutionCode remapping hooks with stale-map rejection;
- COVE-L range planning and page-cluster coalescing;
- COVE-CACHE and COVE-CX compatibility checks;
- index-only answer validation under visibility and redaction policy;
- zero-copy eligibility validation with materialized fallback.

### Phase 7: Mechanical-Sympathy Execution Kernels

Add specialized kernels for the dominant object query shapes.

Deliverables:

- direct Arrow builders for simple object, association, and evidence
  projections;
- selection bitmap and selection-vector execution;
- segment-local FileCode predicate bitsets;
- dense latest/as-of reconstruction state keyed by compact object identifiers;
- allocation-free property predicate loops;
- reusable per-query scratch buffers;
- hot/cold split between validated fast paths and generic fallback paths;
- no `serde_json::Value`, dynamic expression dispatch, allocation, formatting,
  or dictionary lookup inside tight coded scan loops unless the operator is a
  declared materialization boundary;
- benchmark and codegen checks for allocation count, bytes touched,
  branch misses, cache misses, local shadow width, remap cost, and final
  materialization cost.

### Phase 8: Association And Evidence Optimization

Optimize association and evidence queries after the base object path is stable.

Deliverables:

- endpoint-role and direction-specific association scan planning;
- association semi-join and anti-join planning;
- association count, distinct target, and validity-interval fast paths;
- endpoint-aware coverage, COVE-I/COVX, and temporal pruning;
- evidence grain indexes for object, property, association, projection, and
  source evidence;
- evidence existence and count fast paths only when disclosure policy permits;
- lineage-aware projection output and projection/evidence dependency reuse;
- tests proving hidden endpoints, hidden evidence, and redacted association
  targets do not leak through traversal, counts, or explain output.

## Test Plan

Conformance should be organized into tiers that keep correctness and
optimization accountability separate without reducing the intended language
scope.

Tier 0: semantic correctness without accelerators.

- Use object, projection, evidence, association, temporal, and explain queries
  without trusting optional accelerators.
- Results must match canonical COVE-O/COVE-MAP readback.
- Output modes should include JSON diagnostics and Arrow batches.

Tier 1: fallback invariance.

- Run the same queries with valid optional metadata, missing optional metadata,
  stale metadata, corrupt metadata, and unsupported metadata.
- Visible result rows must remain identical.
- `explain` output may differ and should report the trusted, ignored, or
  rejected metadata.

Tier 2: acceleration proof.

- Verify temporal segment pruning, blooms, coverage pruning, COVE-I/COVX
  lookup, index-only answers, coded predicates, ExecutionCode remapping,
  COVE-L range planning, zero-copy eligibility, and fallback boundaries.
- Verify each accelerated answer against the Tier 0 materialized baseline.

Correctness tests should cover:

- feature-scope validation for header, section, page, profile, and operation
  requiredness;
- latest, as-of, history, and changes semantics;
- tombstones and branch selection;
- prev_ref target validation and reconstruction self-containment;
- object properties with nulls;
- association existence and counts;
- evidence filtering and projection;
- projection rule determinism, row grain, and temporal mode;
- predicate normal-form validation, malformed arity, and canonical literal
  handling;
- coverage proof records, coverage set algebra, stale coverage rejection, and
  no-false-negative behavior;
- COVE-I root validation, exact index-only answers, approximate-answer
  rejection for exact queries, and stale sidecar rejection;
- COVE-L layout, page-cluster, scan-split, and zero-copy compatibility
  fallback;
- COVE-E ExecutionCode comparison scope, code-space epoch, and stale mapping
  handling;
- COVE-CACHE invalidation and full-scan fallback;
- dictionary equality inside one file;
- rejected or remapped cross-file code comparisons;
- order by dictionary-coded strings;
- case-sensitive and case-insensitive comparisons;
- visibility overlays and redaction;
- fallback when coverage or index metadata is absent;
- fail-closed behavior for stale or corrupt proof metadata;
- duplicate methods, method conflicts, unsupported temporal roles, resource
  budget failures, and diagnostic redaction.

Metamorphic tests should cover:

- `where(a).where(b)` equals `where(a && b)`;
- valid optional metadata returns the same visible rows as absent optional
  metadata;
- valid accelerator plus residual predicate equals full materialized predicate;
- DataFusion pushed filter equals DataFusion residual filter;
- dictionary-coded equality equals decoded equality;
- ExecutionCode remap equality equals decoded equality;
- materialized `orderBy` equals collation-sidecar `orderBy`;
- `includeTombstones(false)` equals the default state surface;
- hidden associations and hidden evidence do not leak through `not exists`,
  counts, or explain output;
- parse, print, and parse preserves the AST fingerprint;
- resolved plan fingerprints stay stable across harmless whitespace and
  quoting changes.

Performance tests should report:

- feature and sidecar validation cost;
- predicate normal-form generation cost;
- coverage provider lookup cost and coverage-set size;
- COVE-I lookup and index-only answer latency;
- rows/states scanned;
- rows/states pruned;
- temporal segments read and skipped;
- morsels/pages read and skipped;
- metadata-only answers;
- percentage of predicates evaluated coded;
- local shadow width and local-to-global translation cost when used;
- residual materialized predicate cost;
- dictionary lookup count and dictionary-lift amortization;
- allocations per output row and allocations per scanned row;
- bytes touched per candidate and per emitted row;
- branch-miss and cache-miss counters for hot kernels where practical;
- bytes read and range requests for object storage;
- range coalescing effectiveness;
- zero-copy success/fallback rate;
- ExecutionCode remap overhead;
- cache hit/miss/invalidation behavior;
- final materialization cost.

## Resolved Conformance Decisions

The first conformance profile closes the previously open implementation
defaults:

- Mandatory `history()` modes are `records`, `states`, and
  `records_and_states`.
- Mandatory `changes()` modes are `records`, `state_transitions`,
  `property_diffs`, and `final_rows`; `final_objects` remains a legacy input
  alias.
- Mandatory deterministic functions are `isNull`, `isNotNull`, `coalesce`,
  safe COVE-MAP-declared casts, `lower`, `upper`, `trim`, `length`,
  `startsWith`, and `identity`.
- Projection roots order by declared projection ordering first, then canonical
  row identity, then manifest/file ordinal plus source row ordinal as a
  reported fallback.
- String syntax and builder APIs both accept object, association, evidence,
  projection, temporal, grouping, aggregation, ordering, pagination, and
  explain forms, including `EXPLAIN CODED` and `.explain("coded")`.
- Accepted evidence shorthand forms are `evidence()`, `evidence(self)`,
  `evidence(path)`, `evidence(association(...))`, and
  `evidence(projection(...))`.

## Initial Acceptance Criteria

A first implementation should be considered successful when:

1. A query such as the following returns identical visible rows in canonical
   default order with no optional metadata, valid optional metadata, stale
   optional metadata, corrupt optional metadata, and unsupported optional
   metadata:

```coveo
Person
  .asOf(csn: 42)
  .where(status == "active" && email.isNotNull())
  .select(goid, name, email)
  .take(50)
```

2. `explain()` reports resolved object type ids, property ids, temporal mode,
   selected segments, loaded system columns, loaded property columns, trusted
   metadata, ignored metadata, coded predicates, residual predicates, decode
   boundaries, visibility/redaction application, and fallback reasons.
3. Optional metadata can reduce work but cannot change rows.
4. Visibility and redaction are applied before index-only answers, zero-copy
   output, exact aggregate disclosure, or explain disclosure.
5. The same logical query can target JSON diagnostics and Arrow output.
6. DataFusion integration, when present, is demonstrably a backend path rather
   than the source of semantics.

## Initial Integration Slice

The first implementation slice should exercise the complete architecture on
the highest-value query path. It is not a language scope limit: parser,
resolved AST, logical plan, security context, and explain contracts should be
shaped for every root surface and core method from the start.

1. Implement operation-context construction and feature-scope validation.
2. Implement the grammar, public AST, resolved AST, and diagnostics for all
   root surfaces and core methods.
3. Execute object, projection, evidence, and association reads through the
   canonical materialized path where the current readback layer can provide
   them.
4. Support `.where`, `.select`, `.asOf`, `.take`, and `.explain` as the first
   end-to-end exercised chain.
5. Resolve object type ids, property ids, association roles, evidence grains,
   projection dependencies, temporal cuts, branch modes, tombstone modes, and
   security context during planning.
6. Lower predicates to CoveQL logical predicate forms and record
   COVE-COVERAGE-compatible forms for diagnostics and later proof validation.
7. Produce direct Arrow output for simple object/evidence/association
   projections and keep JSON as a debug/fallback output.
8. Add explain diagnostics that identify feature requiredness, segment
   pruning, loaded properties, predicate forms, proof records, sidecar usage,
   coded predicates, residual predicates, decode boundaries, security policy,
   and materialized fallback.
9. Add coded predicate, index-only, cache, and zero-copy execution authority
   only after the logical semantics, fallback invariance, and explain schema
   are stable.

This creates a useful user-facing object query language quickly while keeping
the complete coded physical execution path aligned with the same semantics.
