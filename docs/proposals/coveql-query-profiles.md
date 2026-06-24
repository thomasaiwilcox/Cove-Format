# CoveQL: Unified Cove Query Language Profiles

Status: implemented CoveQL 0.1 baseline

Related proposal: [CoveQL: Cove Object Query Language](./coveql-object-query-language.md)

Owning profiles: COVE-O / COVE-MAP / COVE-COVERAGE / COVE-I / COVE-E

## Summary

This follow-up broadens the object-native query work into **CoveQL**, short
for **Cove Query Language**.

The broader goal is to keep the object-native syntax and proof-safe execution
model from CoveQL/Object while defining CoveQL as a profiled query language
that can address object, graph, and table-shaped Cove data without pretending
every shape is SQL.

The proposed profiles are:

- **CoveQL/Object**: canonical objects, associations, evidence, temporal state,
  and deterministic COVE-MAP projections.
- **CoveQL/Graph**: nodes, edges, paths, traversals, graph evidence, temporal
  graph state, and graph projections.
- **CoveQL/Table**: table rows, columns, table projections, temporal table
  reads, evidence/lineage, coded predicates, aggregates, and SQL/DataFusion
  interop.

The language should have one shared core:

```text
root(...)
  .branch(...)
  .asOf(...)
  .includeTombstones(...)
  .history(...)
  .changes(...)
  .where(...)
  .groupBy(...)
  .select(...)
  .orderBy(...)
  .skip(...)
  .take(...)
  .explain(...)
```

Each profile defines what the root means, what one row/state/path represents,
how identity and ordering work, which relationships are available, how
evidence attaches, and where materialization boundaries occur.

CoveQL is read-oriented. Mutation, transactions, DDL, arbitrary scripting, and
general-purpose user-defined execution are outside CoveQL-Core 0.1.

## Naming Recommendation

Use **CoveQL** as the public language-family name.

```text
Formal name: Cove Query Language
Short name: CoveQL
Core crate/module: coveql
Profile crates/modules:
  coveql-object
  coveql-graph
  coveql-table
```

The current object-language proposal becomes the object profile:

```text
Object query surface -> CoveQL/Object
```

New specification text, crate names, examples, persisted queries, and
conformance suites should use CoveQL and the profile names. Avoid **CQL** as a
public abbreviation because it is too generic and likely to collide with other
query languages.

## Motivation

The CoveQL/Object design already points toward a root-surface language rather
than a single object-only language. It has object roots, association roots,
evidence roots, projection roots, temporal semantics, proof-safe planning,
policy-aware explain, fallback boundaries, and late materialization.

Those same mechanics can apply to graph and table data:

- object state maps naturally to graph nodes;
- associations map naturally to graph edges;
- projection rows and table rows share a row-grain execution model;
- evidence and lineage are valuable across all shapes;
- COVE dictionaries, encoded columns, coverage metadata, COVE-I, COVE-E, and
  COVE-L can accelerate all profiles when their invariants hold.

The strategic goal is not to replace SQL, Cypher, Gremlin, or Datalog. The goal
is to provide a Cove-native query surface where Cove semantics are first-class:
temporal state, evidence, file authority, proof-safe metadata, visibility,
redaction, coded execution, and deterministic materialization.

## Design Principle

CoveQL should be a profiled language, not a bag of special cases.

CoveQL-Core should be deliberately small and strict. It carries shared query
mechanics across profiles, but it must not define the meaning of object state,
table rows, graph paths, association identity, projection authority, or
evidence grain.

The core language owns:

- parsing and diagnostics;
- method-chain semantics;
- resolved AST and fingerprints;
- operation context and security context;
- predicate planning;
- coded/proof-safe physical planning;
- fallback and rejection behavior;
- output modes;
- explain JSON;
- resource budgets;
- DataFusion interop boundaries.

Each profile owns:

- root names and root authority;
- input grain;
- identity and canonical ordering;
- temporal rules;
- null, missing, and repeated-value rules;
- relationship semantics;
- aggregate semantics where they differ from the core;
- evidence attachment;
- projection dependencies;
- profile-specific conformance tests.

## CoveQL-Core Contract

CoveQL-Core should own only the machinery that can be applied without knowing
whether the query is object, graph, or table-shaped:

```text
CoveQLCoreContract {
  language_version,
  core_version,
  grammar_version,
  resolved_ast_version,
  explain_schema_version,
  method_chain_rules,
  diagnostic_schema,
  operation_context_schema,
  security_context_schema,
  resource_budget_schema,
  fingerprint_schema,
  fallback_matrix,
  output_modes,
  datafusion_boundary,
}
```

Core may parse, carry, validate, fingerprint, plan, explain, and enforce
profile declarations. It may not invent profile semantics. A core planner that
does not understand a profile contract must reject or residualize according to
that contract; it must not guess object, graph, or table behavior.

## Profile Contract

Every CoveQL profile should fill in the same contract:

```text
CoveQLProfileContract {
  profile_id,
  profile_version,
  supported_roots,
  root_authority,
  input_grain,
  identity_model,
  canonical_order,
  temporal_capabilities,
  branch_capabilities,
  tombstone_capabilities,
  evidence_targets,
  relationship_capabilities,
  profile_methods,
  profile_expressions,
  bridge_requirements,
  aggregate_rules,
  null_missing_nan_rules,
  security_barriers,
  materialization_boundaries,
  output_modes,
  explain_fields,
  conformance_tiers,
}
```

The profile contract is part of planning. It must appear in explain output
when policy allows, and it must be included in plan fingerprints so cached or
accelerated plans cannot cross profile-version boundaries by accident.

## Operation And Security Context

CoveQL-Core should carry a minimal execution context before profile resolution,
planning, or accelerator selection:

```text
OperationContext {
  dataset_or_file_snapshot,
  selected_operation,
  primary_profile,
  enabled_profiles,
  profile_contract_versions,
  bridge_contract_versions,
  temporal_mode,
  branch_mode,
  tombstone_mode,
  output_mode,
  capability_table,
}
```

Security policy is also a planning input, not an output-only filter:

```text
SecurityContext {
  principal_or_session,
  visibility_policy,
  redaction_policy,
  explain_policy,
  metadata_disclosure_policy,
  aggregate_disclosure_policy,
  zero_copy_permission,
  index_only_answer_permission,
}
```

The planner must decide whether it may consult metadata, reveal diagnostics,
return exact aggregates, use zero-copy buffers, or answer from indexes before
choosing a physical plan.

## Versioning And Conformance IDs

CoveQL should version the core and profiles independently:

```text
CoveQL language version: 0.1
CoveQL-Core version: 0.1
CoveQL/Object version: 0.1
CoveQL/Table version: 0.1
CoveQL/Graph version: 0.1
Explain JSON schema version: 0.1
Resolved AST schema version: 0.1
```

Persisted query strings should be able to declare their version and enabled
profiles:

```coveql
# coveql: 0.1
# profiles: object, table
```

Host APIs should also accept explicit version/profile inputs:

```rust
parse(query, CoveQlVersion::V0_1, &[Profile::Object])
```

Conformance suites should use profile-specific ids such as:

```text
coveql-core-0.1
coveql-object-0.1
coveql-table-0.1
coveql-graph-0.1
```

## CoveQL Grammar

The full grammar should live in the CoveQL-Core spec. This proposal uses the
following shape to make root selection, bindings, named arguments, directives,
profile methods, and evidence syntax deterministic.

```text
Query              := Directive* RootBinding MethodChain
Directive          := "#" Identifier ":" DirectiveValue

RootBinding        := Root Alias?
Alias              := "as" Identifier
Root               := ObjectRoot
                    | AssociationRoot
                    | NodeRoot
                    | EdgeRoot
                    | PathRoot
                    | TableRoot
                    | ProjectionRoot
                    | EvidenceRoot

ObjectRoot         := "object" "(" Identifier ")"
AssociationRoot    := "association" "(" Identifier RoleArg? ")"
NodeRoot           := "node" "(" Identifier ")"
EdgeRoot           := "edge" "(" Identifier RoleArg? ")"
PathRoot           := "path" "(" PathExpression ")"
TableRoot          := "table" "(" Identifier ")"
ProjectionRoot     := "projection" "(" Identifier ")"
EvidenceRoot       := "evidence" "(" EvidenceSpec? ")"

MethodChain        := Method*
Method             := "." CoreMethod | "." ProfileMethod
CoreMethod         := Branch
                    | AsOf
                    | IncludeTombstones
                    | History
                    | Changes
                    | Where
                    | GroupBy
                    | Select
                    | OrderBy
                    | Skip
                    | Take
                    | Explain

Branch             := "branch" "(" BranchSelector ")"
AsOf               := "asOf" "(" ("csn" ":" UInt | TimeBound) ")"
TimeBound          := TemporalRole ":" TimestampLiteral
TemporalRole       := "commit_time"
                    | "valid_time"
                    | "observed_time"
                    | "source_event_time"
                    | "association_valid_time"
IncludeTombstones  := "includeTombstones" "(" Boolean ")"
History            := "history" "(" HistoryArgs? ")"
HistoryArgs        := "mode" ":" HistoryMode
HistoryMode        := "records" | "states" | "records_and_states"
Changes            := "changes" "(" ChangeBound "," ChangeBound ChangeArgs? ")"
ChangeBound        := ("csn" ":" UInt) | TimeBound
ChangeArgs         := "," "mode" ":" ChangeMode
ChangeMode         := "records" | "state_transitions" | "property_diffs"
                    | "final_rows"
Where              := "where" "(" Predicate ")"
GroupBy            := "groupBy" "(" Expr ("," Expr)* ")"
Select             := "select" "(" SelectItem ("," SelectItem)* ")"
OrderBy            := "orderBy" "(" Expr OrderDirection? NullOrdering? ")"
Skip               := "skip" "(" UInt ")"
Take               := "take" "(" UInt ")"
Explain            := "explain" "(" ExplainMode? ")"
ExplainMode        := StringLiteral | Identifier

ProfileMethod      := Lookup | Traverse | ProfileMethodCall
Lookup             := "lookup" "(" RootBinding "," NamedArgList ")"
Traverse           := "traverse" "(" RelationshipExpr ("," NamedArg)* ")"
ProfileMethodCall  := Identifier "(" (ExprOrNamedArg ("," ExprOrNamedArg)*)? ")"

PathExpression     := NodeBinding ("." RelationshipExpr)+
NodeBinding        := NodeRoot Alias?
RelationshipExpr   := Direction "(" EdgeRoot Alias? ")" RelationshipTarget?
Direction          := "in" | "out" | "either"
RelationshipTarget := ".to" "(" NodeRoot Alias? ")"

Predicate          := OrExpr
OrExpr             := AndExpr ("||" AndExpr)*
AndExpr            := NotExpr ("&&" NotExpr)*
NotExpr            := "!" NotExpr | CompareExpr
CompareExpr        := Expr CompareOp Expr
                    | Expr "in" "[" Literal ("," Literal)* "]"
                    | Expr "." ("isNull" | "isNotNull") "(" ")"
                    | "exists" "(" ExistsTarget ("," NamedArgList)? ")"
                    | BoolFunctionCall
                    | "(" Predicate ")"
ExistsTarget       := RootBinding | RelationshipExpr

Expr               := Path
                    | Literal
                    | FunctionCall
                    | AggregateCall
                    | RelationshipExpr
                    | EvidenceExpr
                    | "(" Expr ")"
Path               := Identifier ("." Identifier)*
SelectItem         := Identifier ":" Expr | Expr
FunctionCall       := Identifier "(" (Expr ("," Expr)*)? ")"
BoolFunctionCall   := FunctionCall
AggregateCall      := AggregateName "(" ("*" | Expr)? ")"
EvidenceExpr       := "evidence" "(" EvidenceSpec? ")"

NamedArgList       := NamedArg ("," NamedArg)*
NamedArg           := Identifier ":" NamedArgValue
NamedArgValue      := Predicate | Expr | EnumLiteral
EnumLiteral        := Identifier
ExprOrNamedArg     := Expr | NamedArg
RoleArg            := "," ("role" | "from" | "to") ":" Identifier
EvidenceSpec       := EvidenceTarget ("," EvidenceOption)*
                    | EvidenceOption ("," EvidenceOption)*
EvidenceTarget     := Path | RootBinding | "self"
EvidenceOption     := "grain" ":" EvidenceGrain

BranchSelector     := Identifier | StringLiteral | UInt
OrderDirection     := "," ("asc" | "desc")
NullOrdering       := "," ("nulls_first" | "nulls_last")
CompareOp          := "==" | "!=" | "<" | "<=" | ">" | ">="
AggregateName      := "count" | "sum" | "avg" | "min" | "max"
                    | "distinct_count"
EvidenceGrain      := "object" | "property" | "row" | "column"
                    | "association" | "projection"
                    | "node" | "edge" | "path" | "source"
DirectiveValue     := DirectiveAtom ("," DirectiveAtom)*
DirectiveAtom      := Identifier | StringLiteral | VersionLiteral
Integer            := "-"? UInt
Decimal            := "-"? UInt "." UInt
Literal            := NullLiteral | Boolean | Integer | Decimal | StringLiteral
                    | TimestampToken
TimestampLiteral   := StringLiteral | TimestampToken
```

Lexical rules should be explicit before parser conformance:

- identifiers are case-sensitive and may be unquoted or quoted;
- unquoted identifiers use ASCII letters, digits, and `_`, and cannot start
  with a digit;
- quoted identifiers preserve case and punctuation, so
  `table("order-history")` and `o."customer id"` are valid forms;
- reserved words such as `object`, `table`, `where`, `select`, `as`, `in`,
  `out`, `either`, and method names require quoted identifiers when used as
  field names;
- string literals use double quotes and C-style escapes for quotes,
  backslashes, tabs, newlines, and Unicode scalar values;
- booleans are `true` and `false`; null is `null`;
- integer literals and decimal literals may be signed; CSNs, `skip`, and
  `take` remain unsigned;
- decimal literals must canonicalize with declared precision and scale before
  planning;
- timestamp string literals and timestamp tokens must canonicalize to COVE
  timestamp values before planning; invalid, ambiguous, or timezone-free
  values reject unless the selected profile declares a timezone policy;
- a line beginning with `#` is a directive only when it has `name: value`
  form; other `#` lines are comments and do not participate in directive
  fingerprints;
- version literals use dotted numeric form such as `0.1`.

The grammar is profile-extensible only through `ProfileMethod` and profile
contracts. Root forms, aliases, directives, expressions, diagnostics, and
method-chain conflict behavior remain CoveQL-Core responsibilities.

The parser should preserve named argument values as unresolved
named-argument atoms. The receiving method or expression contract resolves
each value as a predicate, expression, enum literal, or policy literal during
type resolution.

Enum-like named argument values are resolved by the receiving method or
expression contract before ordinary field lookup. If the declared argument
type is an enum, an unquoted identifier is resolved as an enum literal. If the
declared argument type is an expression, the identifier is resolved as a path
or binding. Unknown enum values reject with a profile-method diagnostic. For
example, `cardinality: one`, `unmatched: nulls`, and `mode: walk` are enum
literals only because their profile contracts declare those values.

Boolean-valued function calls may appear as predicates only when the resolver
proves the function returns a boolean under the selected profile's type rules.

Valid explain modes are `public`, `developer`, `proof`, and `forensic`.
`.explain()` defaults to `.explain("public")`. Higher-disclosure modes are
subject to the active diagnostic and metadata disclosure policies.

## Root Surface Model

The recommended explicit roots are:

```text
object(Person)
association(CustomerPlacedOrder)
node(Person)
edge(CustomerPlacedOrder)
path(...)
table(orders)
projection(people_projection)
evidence(...)
```

Canonical persisted CoveQL queries should use explicit roots. Bare object roots
may be accepted only by CoveQL/Object as a compatibility shorthand, or when the
host API has selected Object as the default profile.

```coveql
Person.where(status == "active")
```

In that context, the shorthand must resolve to the same resolved AST as:

```coveql
object(Person).where(status == "active")
```

The explicit root form is preferable in shared documents, mixed-profile
queries, and diagnostics:

```coveql
object(Person)
  .where(status == "active")
  .select(goid, name)
```

Bare identifiers are not portable across profiles because an identifier may be
an object type, table surface, graph label, projection name, function name, or
future root kind.

## Primary Profile Resolution

Resolvers should select one primary profile before type resolution:

1. If the host API provides a primary profile, use it.
2. Else if the root is profile-specific, infer that profile.
3. Else if directives declare compatible profiles, use the declared profile.
4. Else if the root is common, such as `projection(...)` or `evidence(...)`,
   require a host context or catalog-declared default profile.
5. If more than one profile remains possible, reject as ambiguous.

Examples:

| Query root | Primary profile |
| --- | --- |
| `object(Person)` | object |
| `table(orders)` | table |
| `node(Customer)` | graph |
| `projection(name)` | host or catalog default required |
| `evidence(Person)` | target, host, or catalog default required |

Ambiguous profile resolution should produce a structured diagnostic instead of
trying object, table, and graph resolution in sequence.

## Binding And Alias Model

CoveQL should use explicit `as` bindings whenever a query can expose more than
one root, relationship, table, node, edge, or path binding.

```coveql
table(orders) as o
  .lookup(table(customers) as c, on: o.customer_id == c.customer_id)
  .select(
    order_id: o.order_id,
    customer_name: c.name,
    total: o.total
  )
```

```coveql
node(Customer) as c
  .traverse(out(edge(CustomerPlacedOrder) as placed).to(node(Order) as o))
  .where(o.status == "shipped")
  .select(
    customer: c.goid,
    order: o.goid,
    total: o.total
  )
```

Rules:

- unqualified field references are allowed only when exactly one binding
  exposes that field;
- qualified references use `binding.field`;
- type names, table names, labels, and projection names are not bindings unless
  explicitly declared as aliases;
- aliases affect query scope and diagnostics, not data authority;
- repeated labels or repeated table surfaces require explicit aliases;
- aliases must be included in resolved AST fingerprints and explain output.

## Shared Method Semantics

The shared methods should retain the CoveQL/Object ordering model:

```text
Root
-> branch, tombstone, and temporal mode resolution
-> scan grain selection
-> pre-reconstruction filters
-> reconstruction, when state-producing
-> visibility and redaction barriers
-> post-reconstruction filters
-> relationship expansion, semi-joins, and anti-joins
-> grouping and aggregation
-> projection/select
-> sort
-> skip/take
-> output or explain
```

The meaning of "scan grain" is profile-specific:

| Profile | Root | Grain |
| --- | --- | --- |
| Object | `object(Person)` | one reconstructed object state |
| Object | `association(T)` | one reconstructed association state |
| Graph | `node(Person)` | one visible graph node state |
| Graph | `edge(T)` | one visible graph edge state |
| Graph | `path(...)` | one path binding |
| Table | `table(orders)` | one visible table row |
| Common | `projection(name)` | one deterministic projection row |
| Common | `evidence(...)` | one evidence row at declared grain |

## Source Order And Filter Placement

Source order defines binding scope. A method may reference only bindings that
exist at that point in the method chain.

Execution order is dependency-aware. Filters should be placed at the earliest
safe execution stage where all referenced bindings exist and the required
visibility, redaction, temporal, and reconstruction barriers have been applied.

Examples:

- `where(c.status == "active")` may run before traversal when `c` is the root
  binding.
- `where(o.status == "shipped")` must run after traversal or lookup
  introduces `o`.
- `where(c.status == "active" && o.status == "shipped")` may be split into
  independent fragments only when doing so preserves visible results,
  diagnostics class, profile grain, and policy behavior.

The planner may reorder filters only when the rewrite preserves temporal
semantics, visibility, redaction, aggregate disclosure, and profile grain.

## Method Conflict Rules

Duplicate and conflicting methods should resolve deterministically:

- multiple `where` methods are allowed and are equivalent to conjunction,
  subject to binding scope;
- multiple `select`, `groupBy`, `asOf`, `branch`, `includeTombstones`,
  `take`, `skip`, or `explain` methods reject unless a profile explicitly
  defines a different rule;
- `history`, `changes`, and `asOf` are mutually exclusive unless a profile
  defines a combined temporal mode;
- `explain` followed by additional methods rejects;
- profile methods that introduce bindings, such as `lookup` and `traverse`,
  may appear only before `groupBy` unless the method contract explicitly
  accepts aggregate grain;
- multiple `orderBy` methods reject; CoveQL 0.1 `orderBy` accepts one sort
  term with optional direction and null ordering.

`changes` bounds must use the same bound kind and temporal role unless the
selected profile declares an exact conversion rule. Mixed CSN/time bounds and
mixed temporal roles reject by default. The default interval is half-open:
`[from, to)`.

## Method Placement Matrix

Method placement is part of each method contract. The default core placement
rules are:

| Method | Before relationship expansion | After relationship expansion | After `groupBy` | Select aliases |
| --- | ---: | ---: | ---: | ---: |
| `where` | yes | yes | no | no |
| `select` | yes | yes | yes | n/a |
| `orderBy` | yes | yes | yes | yes, if after `select` |
| `history` | temporal phase only | no | no | no |
| `changes` | temporal phase only | no | no | no |
| `lookup` | yes | yes | no by default | no |
| `traverse` | yes | yes | no by default | no |
| `take`/`skip` | final only | final only | final only | yes |
| `explain` | final only | final only | final only | n/a |

`where` never references select aliases. `where` after `groupBy` rejects unless
a future `having` or post-aggregate filter method is introduced.

`history` and `changes` change the temporal grain. They must appear in the
temporal phase before relationship expansion unless a profile explicitly
supports history or change surfaces over expanded relationships.

## Grouped Select Legality

After `groupBy`, `select` may contain only:

- grouping expressions;
- aggregate expressions;
- deterministic expressions of grouping expressions;
- deterministic expressions of aggregate results;
- aliases of those expressions.

Ungrouped raw fields reject unless the profile contract declares a functional
dependency proving the field has one visible value per group.

## Profile Extension Methods

The core grammar should allow profile methods without making them core
semantics:

```text
Method := CoreMethod | ProfileMethod
```

Every profile method should declare:

```text
ProfileMethodContract {
  name,
  owning_profile,
  valid_input_grains,
  output_grain,
  placement_in_method_chain,
  may_change_cardinality,
  may_change_bindings,
  security_barrier_requirements,
  fallback_behavior,
  explain_fields,
}
```

Examples:

```text
lookup:
  profile: table
  input grain: table row
  output grain: table row with joined binding
  placement: relationship/join phase before groupBy
  cardinality: left-preserving by default
```

```text
traverse:
  profile: graph
  input grain: node or path binding
  output grain: path binding
  placement: relationship expansion before groupBy
  cardinality: expands
```

Profile methods must participate in security planning, resource budgets,
diagnostics, explain output, and plan fingerprints.

## Profile Extension Expressions

Not every profile extension is a method. Relationship expressions, evidence
targets, table `exists(..., on: ...)`, and aggregate inputs need contracts too.

```text
ProfileExpressionContract {
  name,
  owning_profile,
  expression_kind,
  valid_input_grains,
  output_type,
  may_change_bindings,
  required_bindings,
  security_barrier_requirements,
  fallback_behavior,
  explain_fields,
}
```

A profile expression may be used only when declared by the primary profile
contract or by an active bridge contract. Otherwise it rejects with
`E_UNSUPPORTED_PROFILE_METHOD`, `E_UNKNOWN_BRIDGE`, or the closest
profile-specific diagnostic.

`lookup` is a core method shape with profile-owned semantics. It may be
invoked when the primary profile contract declares it, or when an active bridge
contract exports it for the current input grain and target root.

## Shared Ordering And Pagination

`skip` and `take` without explicit `orderBy` use the primary profile's
`canonical_order`.

CoveQL 0.1 `orderBy` accepts one sort term with optional direction and null
ordering. Multiple sort terms should be represented in a later `SortSpec`
list only after the syntax and tie-breaker rules are specified.

If a profile cannot provide deterministic canonical ordering for the selected
grain, `skip` and `take` without `orderBy` must reject. Accelerated and
materialized execution must produce the same first `N` rows under the same
canonical order.

`orderBy` may reference:

- resolved input expressions still in scope;
- select aliases when `orderBy` appears after `select` in the method chain;
- aggregate aliases after `groupBy` and `select`.

The resolved AST must replace alias references with their defining expression
or with a stable output binding. Select aliases cannot shadow existing
bindings in the same scope unless a profile explicitly allows quoted shadowing;
the default behavior is rejection.

## Shared Aggregate Semantics

Aggregate input is the current visible grain after required visibility and
redaction barriers.

Rules:

- `count(*)` counts visible input rows, states, paths, or records.
- `count(expr)` counts inputs where `expr` is visible and not null.
- `count(evidence(...))` counts visible evidence rows at the declared grain.
- `count(out(edge(T)))` counts visible edge states by default.
- distinct target counts must use an explicit `distinct_count` or distinct
  target expression.
- `distinct_count(expr)` counts canonical logical values, not raw codes,
  unless code equality is proven for the selected collation and null policy.
- `sum` and `avg` ignore nulls; all-null groups return null.
- `min` and `max` over strings require a declared collation or materialized
  canonical comparison.
- aggregates are subject to aggregate disclosure policy before index-only or
  metadata-only answers are allowed.

Baseline aggregate output rules:

| Aggregate | Null behavior | Output type |
| --- | --- | --- |
| `count(*)` | Counts visible input rows, states, paths, or records. | Unsigned integer. |
| `count(expr)` | Counts visible non-null values. | Unsigned integer. |
| `sum(int)` | Ignores nulls; all-null returns null. | Widened integer or decimal; reject on overflow. |
| `sum(decimal)` | Ignores nulls; all-null returns null. | Decimal with declared precision and scale. |
| `avg(int/decimal)` | Ignores nulls; all-null returns null. | Decimal by default; float only if declared. |
| `min/max(string)` | Ignores nulls; all-null group returns null. | Same logical type; collation required. |
| `distinct_count(expr)` | Ignores nulls unless the profile declares null as a distinct value. | Unsigned integer. |

## Evidence Shorthand

Evidence shorthand is contextual and must resolve to a profile-specific
evidence target:

- in the object profile, `evidence()` means
  `evidence(current_object, grain: object)`;
- in the object profile, `evidence(binding.property)` means property evidence;
- in the object profile, `evidence(association_binding)` means association
  evidence;
- in projection roots, `evidence(binding)` means projection-row evidence;
- in the table profile, `evidence(binding)` means current row evidence;
- in the table profile, `evidence(binding.column)` means column evidence;
- in the graph profile, `evidence(node_binding)` means node evidence;
- in the graph profile, `evidence(edge_binding)` means edge evidence;
- in the graph profile, `evidence(path_binding)` means path evidence.

Targetless `evidence()` is valid only when there is exactly one current grain.
If multiple bindings are in scope, the query must name the evidence target.

## Null, Missing, And NaN Conformance

CoveQL-Core defines the truth-table format. Each profile fills in the
concrete logical rules for its grain and value model.

Baseline rules:

| Case | Result |
| --- | --- |
| `null == null` | UNKNOWN |
| `null != value` | UNKNOWN |
| `null in [...]` | UNKNOWN |
| `value in [null, ...]` | TRUE if another item matches; otherwise UNKNOWN. |
| `isNull()` | TRUE for present null values. |
| `isNotNull()` | TRUE for present non-null values. |
| missing property | UNKNOWN in predicates unless the profile maps missing to null. |
| missing column | Reject for base tables; nullable projections may map to null. |
| `NaN == NaN` | FALSE unless the logical type declares canonical NaN equality. |
| NaN ordering | Reject unless a NaN sort policy is declared. |
| NaN grouping | Group by declared canonical NaN policy, or reject. |
| null join keys | Do not match unless `nulls_match: true` is declared. |
| null group keys | All nulls form one null group unless the profile says otherwise. |

## Nested And Repeated Values

Dotted paths traverse declared struct, object, row, node, edge, and projection
fields only. They do not implicitly iterate arrays, lists, maps, or repeated
values.

Predicates over repeated values require explicit functions such as `any`,
`all`, `contains`, or a profile-declared collection operator. Absent collection
elements are missing, not null, unless the selected profile maps them to null.

## Type Coercion

CoveQL performs no broad SQL-style implicit coercion by default.

Rules:

- comparisons require compatible logical types;
- safe numeric widening may be allowed when declared by the profile;
- string-to-number, string-to-timestamp, and boolean/string coercions require
  explicit cast functions;
- coded execution is allowed only after canonical logical coercion semantics
  are proven;
- SQL/DataFusion coercion rules do not override Cove logical rules unless a
  profile explicitly adopts them for a table surface.

## CoveQL/Object Profile

CoveQL/Object is the direct object query profile.

Example:

```coveql
object(Person)
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

Profile responsibilities:

- object type and property resolution through the COVE-O catalog;
- association resolution through COVE-MAP association/link metadata;
- object temporal reconstruction;
- tombstone and branch handling;
- object/property evidence;
- object projection dependencies;
- object canonical ordering.

This profile should keep the semantics already developed in the object-language
proposal.

## CoveQL/Graph Profile

The graph profile should treat objects and associations as graph-shaped data:
objects become nodes, and associations become edges. The profile should not
invent a separate graph truth model. Graph nodes and edges remain backed by
COVE-O object/association authority or by an explicit graph projection.

Basic node query:

```coveql
node(Customer) as c
  .where(status == "active")
  .select(c.goid, c.name)
```

Basic edge query:

```coveql
edge(CustomerPlacedOrder) as e
  .asOf(csn: 42)
  .select(e.source_goid, e.target_goid, e.valid_from, e.valid_to)
```

Object-relative graph query:

```coveql
node(Customer) as c
  .where(exists(out(edge(CustomerPlacedOrder))))
  .select(
    customer: c.goid,
    name: c.name,
    order_count: count(out(edge(CustomerPlacedOrder)))
  )
```

Traversal query:

```coveql
node(Customer) as c
  .traverse(out(edge(CustomerPlacedOrder) as placed).to(node(Order) as o))
  .where(o.status == "shipped")
  .select(
    customer: c.goid,
    order: o.goid,
    total: o.total
  )
```

Path query:

```coveql
path(
  node(Customer) as c
    .out(edge(CustomerPlacedOrder) as placed)
    .to(node(Order) as o)
)
  .where(o.total > 100)
  .select(c.goid, o.goid, o.total)
```

Graph profile responsibilities:

- define node identity, edge identity, endpoint roles, and path bindings;
- define direction: `in`, `out`, `either`;
- define path cardinality and whether duplicate paths are preserved;
- define maximum traversal depth and resource-budget behavior;
- define temporal alignment between nodes and edges;
- define hidden-endpoint and hidden-edge disclosure rules;
- define canonical path ordering;
- define path evidence and edge evidence grain.

Graph queries are where CoveQL can be much clearer than SQL. SQL can express
traversals through joins and recursive CTEs, but the user has to know the join
tables, endpoint columns, temporal filters, visibility barriers, and
deduplication rules. CoveQL/Graph can make those semantics explicit.

## Table Surface Authority

`table(name)` should resolve to a declared table surface rather than directly
to any one physical source kind.

```text
TableSurface {
  table_id,
  authority_kind:
    raw_table
    deterministic_projection
    materialized_projection
    external_registered_table,
  row_grain,
  row_identity,
  temporal_capabilities,
  temporal_authority:
    native_temporal_table
    recomputable_projection_at_temporal_cut
    materialized_snapshot_only
    external_snapshot_only
    none,
  evidence_capabilities,
  projection_dependency_contract?,
  datafusion_provider?,
}
```

### TableSurfaceContract Decision

Raw tables, materialized projections, and external registered tables become
CoveQL/Table surfaces only after they provide a `TableSurfaceContract`.
Until then, they remain available through lower-level readers or SQL/DataFusion
interop, but not through `table(name)`.

```text
TableSurfaceContract {
  table_id,
  table_name,
  contract_version,
  authority_kind,
  authority_fingerprint,
  schema_fingerprint,
  logical_column_map,
  row_grain,
  row_identity,
  canonical_order,
  visibility_authority,
  redaction_authority,
  temporal_authority,
  evidence_capabilities,
  null_missing_nan_policy,
  collation_policy,
  code_domain_contexts,
  code_domain_bridges,
  projection_dependency_contract?,
  datafusion_interop_contract?,
}
```

Required semantics:

- `row_identity` must identify one logical table row across the selected
  snapshot, branch, tenant, and temporal cut. If no non-redacted stable row
  identity exists, `table(name)` rejects.
- `canonical_order` must be total and deterministic. The preferred order is
  declared table order, then row identity. Manifest/file ordinal plus source row
  ordinal is allowed only when the manifest declares it as a stable fallback.
- `visibility_authority` and `redaction_authority` apply before filters, joins,
  grouping, evidence helpers, counts, and explain disclosures.
- `temporal_authority` must declare whether `asOf`, `history`, and `changes`
  are native, recomputable, snapshot-only, or unavailable. Snapshot-only tables
  reject temporal methods.
- `logical_column_map` maps user-facing column names to logical types, null
  policy, collation, COVE path/projection source, and optional code-domain
  metadata. Missing base columns reject; nullable projection outputs are allowed
  only when declared.
- Raw codes from table pages, dictionaries, projections, external engines, or
  manifest members are never comparable by integer value unless
  `code_domain_contexts` prove a shared domain or `code_domain_bridges` provide
  an exact remap. Otherwise planning must decode to canonical logical values or
  reject if policy forbids decode.
- Coded filters, grouping, distinct, ordering, lookup joins, semi-joins, and
  anti-joins require proof over logical values, not local physical codes.
- DataFusion pushdown is advisory until translated through the
  `datafusion_interop_contract` and proven equivalent under CoveQL null, type,
  collation, temporal, visibility, and redaction rules.

Fallback and rejection:

- Missing `TableSurfaceContract`: reject `table(name)` with
  `E_UNKNOWN_TABLE_SURFACE` or a redacted capability diagnostic.
- Missing row identity: reject.
- Missing canonical order: reject any query whose result order matters,
  including implicit default order, `skip`, `take`, and lookup expansion.
- Missing visibility/redaction authority: reject unless the operation context
  explicitly selects an internal trusted mode.
- Missing evidence capability: evidence helpers over the table reject; ordinary
  row scans may continue.
- Unsafe code domain, stale dictionary epoch, missing bridge, or unsafe
  collation: decode to canonical values when policy allows; otherwise reject.
- Coded execution is optional. Materialized execution remains the semantic
  authority until the exact table-surface proof exists for the operator.

Explain output for table surfaces must include the table-surface contract
version, authority kind, authority fingerprint, row identity class, canonical
order, visibility/redaction authorities, temporal authority, evidence
capabilities, code-domain bridge decisions, pushed/residual filters, pushed
columns, fallback/decode boundaries, and DataFusion interop decisions when
policy allows.

`projection(name)` remains the direct deterministic projection surface.
`table(name)` is the user-facing table surface chosen by the catalog. CoveQL
0.1 supports deterministic COVE-MAP projections and existing table readback
surfaces. Raw table sections are enabled after they expose the full
`TableSurfaceContract`.

Temporal rules:

- `asOf` is valid for `native_temporal_table` and
  `recomputable_projection_at_temporal_cut`.
- `history` and `changes` are valid only when the surface exposes temporal
  record grain or change grain.
- `asOf`, `history`, and `changes` reject for `materialized_snapshot_only`,
  `external_snapshot_only`, and `none`.

First CoveQL/Table conformance requires deterministic COVE-MAP
projections and existing table readback surfaces that declare row identity and
canonical order. Materialized projections, external registered tables, and raw
table sections can remain optional until they expose the full table-surface
contract.

## CoveQL/Table Profile

The table profile provides a Cove-native row query surface. It does not try to
replace SQL for all relational analytics. Instead, it makes
Cove-specific table reads safer, more explainable, and more directly
optimizable.

Basic table query:

```coveql
table(orders) as o
  .where(o.status == "shipped")
  .select(o.order_id, o.customer_id, o.total)
  .take(100)
```

Temporal table query:

```coveql
table(orders) as o
  .asOf(csn: 42)
  .where(o.status == "shipped")
  .select(o.order_id, o.customer_id, o.total)
```

Aggregate query:

```coveql
table(orders) as o
  .where(o.status == "shipped")
  .groupBy(o.customer_id)
  .select(
    customer_id: o.customer_id,
    order_count: count(*),
    revenue: sum(o.total)
  )
  .orderBy(revenue, desc)
  .take(50)
```

Evidence-aware table query:

```coveql
table(customers) as c
  .where(c.email.isNotNull())
  .select(
    c.customer_id,
    c.email,
    email_evidence: count(evidence(c.email))
  )
```

Proof-aware table query:

```coveql
table(orders) as o
  .where(o.status == "shipped" && o.total > 100)
  .select(o.order_id, o.total)
  .explain("proof")
```

Profile responsibilities:

- table and column resolution;
- row identity and deterministic default ordering;
- table temporal state, when available;
- row evidence and column evidence;
- null, NaN, missing-column, and type-coercion behavior;
- grouping and aggregate semantics;
- table projection dependencies;
- SQL/DataFusion interop;
- coded predicate and coded aggregate correctness.

## Where CoveQL/Table Can Be Better Than SQL

CoveQL/Table can be better than SQL when the query depends on Cove semantics
that SQL normally hides or models indirectly.

### Temporal Reads

SQL can express temporal reads only when tables and views were modeled for it.
CoveQL can make temporal state part of the operation context:

```coveql
table(orders) as o
  .asOf(commit_time: "2026-01-01T00:00:00Z")
  .where(o.status == "shipped")
```

### Evidence And Lineage

SQL can join to lineage tables, but it does not know that lineage is a
first-class read concern. CoveQL can expose evidence without forcing users to
know internal evidence schemas:

```coveql
table(customers) as c
  .select(c.customer_id, c.email, evidence_count: count(evidence(c.email)))
```

### Proof-Aware Explain

SQL explains optimizer choices. CoveQL can explain proof and authority:

```text
trusted coverage
ignored stale sidecar
coded predicate
decoded residual predicate
visibility barrier
redacted aggregate
zero-copy rejected
```

### Coded Execution

For scan/filter/project/group queries, CoveQL can stay closer to COVE physical
data:

- dictionary-coded equality;
- typed numeric lanes;
- validity bitmaps;
- encoded temporal columns;
- COVE-COVERAGE proof records;
- COVE-I/COVX lookup;
- COVE-E ExecutionCode remaps;
- direct Arrow builders.

SQL engines can use some of these through a table provider, but CoveQL can make
the representation proof part of the query contract.

### Policy-Aware Aggregates

SQL usually treats row security, column security, and aggregate disclosure as
external rules. CoveQL can require the planner to decide whether exact counts,
index-only answers, thresholded aggregates, or redacted answers are allowed
before it chooses a physical plan.

## Where SQL Should Still Win

CoveQL/Table should not try to match SQL's full relational surface.

SQL remains the right tool for:

- arbitrary multi-way joins across unrelated tables;
- nested subqueries;
- common table expressions;
- window functions;
- recursive queries;
- set operators such as `UNION`, `INTERSECT`, and `EXCEPT`;
- broad type coercion and expression compatibility;
- mature cost-based join optimization;
- DDL, DML, transactions, and constraints;
- BI tool compatibility;
- ad hoc analyst workflows where SQL is already the lingua franca.

If CoveQL grows every SQL feature, it becomes a worse SQL. The better boundary
is:

```text
CoveQL/Table: semantic, temporal, evidence-aware, proof-aware reads over Cove.
SQL/DataFusion: general-purpose relational analytics and interoperability.
```

## Table Joins

CoveQL/Table should support a deliberately constrained join model rather than
copying SQL's full join grammar immediately.

Recommended first-class join forms:

```coveql
table(orders) as o
  .lookup(
    table(customers) as c,
    on: o.customer_id == c.customer_id,
    cardinality: one,
    unmatched: nulls
  )
  .select(o.order_id, c.name, o.total)
```

```coveql
table(customers) as c
  .where(exists(table(orders) as o, on: c.customer_id == o.customer_id))
```

```coveql
table(customers) as c
  .where(!exists(table(orders) as o, on: c.customer_id == o.customer_id))
```

These map cleanly to:

- lookup join;
- semi-join;
- anti-join.

The join contract requires:

- explicit join keys;
- declared null semantics;
- declared duplicate behavior;
- compatible logical types;
- compatible collation and code domains;
- policy approval on both sides;
- deterministic output ordering;
- materialized fallback when coded joins are unsafe.

Default lookup semantics:

- `lookup` is left-preserving enrichment by default;
- `cardinality: one` is the default;
- the right side must be unique for the join key unless `cardinality: many` is
  declared;
- `cardinality: many` expands the left row once per visible matching right
  row; output order is left canonical order, then right canonical order;
- unmatched rows produce null right-side fields unless `unmatched: reject` or
  `required: true` is declared;
- null join keys do not match unless `nulls_match: true` is declared;
- if `cardinality: one` is declared and more than one visible right-side match
  exists, the query rejects unless an explicit duplicate policy is declared;
- the default duplicate policy is `duplicate: reject`;
- future duplicate policies may include `first_by(canonical_order)`,
  `aggregate(...)`, or `many`;
- lookup may use coded execution only when both sides share compatible code
  domains or can remap to a common execution code domain;
- lookup, semi-join, and anti-join must not reveal hidden right-side existence
  through inclusion, exclusion, null-filled columns, counts, or explain output.

Visibility and redaction are applied independently to both sides before
lookup, semi-join, or anti-join semantics are evaluated. Hidden right-side rows
behave as absent for visible-query semantics.

`exists(table(...) as r, on: ...)` means at least one visible right-side row
matches the key. `!exists(table(...) as r, on: ...)` means no visible
right-side row matches the key. Explain output must not reveal whether hidden
rows existed, whether they were suppressed, or how many were suppressed.

More general joins can remain a DataFusion/SQL interop path until CoveQL has a
reason to own them directly.

## Table Profile Semantics

CoveQL/Table defines the following profile rules:

| Concern | Rule |
| --- | --- |
| Row grain | One visible row from the selected table, projection, or temporal table state. |
| Row identity | Declared primary row identity or canonical physical row identity. |
| Default order | Table id, branch key, row identity, temporal record id when relevant. |
| Nulls | SQL-style three-valued predicates unless a COVE logical type says otherwise. |
| Missing columns | Reject for base tables; nullable projection output only when declared. |
| NaN | Reject ordering unless a NaN sort policy is declared. |
| Grouping | Group by logical values, not raw codes unless equality is proven. |
| Distinct | Logical distinct, with coded execution only under domain proof. |
| Aggregates | Governed by aggregate disclosure policy. |
| Evidence | Row and column evidence available through `evidence(...)`. |
| Temporal | `asOf`, `history`, and `changes` only when table state carries temporal roles. |
| Joins | Lookup/semi/anti joins first; full joins through SQL/DataFusion interop. |

## Graph Profile Semantics

CoveQL/Graph defines the following profile rules:

| Concern | Rule |
| --- | --- |
| Node grain | One visible node state. |
| Edge grain | One visible edge state. |
| Path grain | One path binding with named node/edge bindings. |
| Node identity | Backing object identity or declared graph node id. |
| Edge identity | Backing association identity or declared graph edge id. |
| Direction | Explicit `in`, `out`, or `either`, with inference only when unambiguous. |
| Duplicate paths | Preserved unless a `distinct` path mode is selected. |
| Temporal | Node and edge state aligned to the same temporal cut by default. |
| Hidden endpoints | Must not leak through traversal, counts, paths, or explain. |
| Ordering | Canonical node/edge/path identity. |
| Resource limits | Maximum path length, fanout, paths emitted, and traversal time. |

## Graph Path Semantics

CoveQL/Graph uses explicit path terminology:

- walk: nodes and edges may repeat;
- trail: edges may not repeat;
- simple path: nodes may not repeat;
- path binding: named ordered node and edge bindings produced by traversal.

Defaults:

- one-hop traversal preserves duplicate visible edges;
- one-hop traversal defaults to `min: 1` and `max: 1`;
- variable-length traversal must declare `min` and `max` depth;
- unbounded traversal is invalid;
- traversal mode defaults to `walk`;
- every node and edge in a path must be visible at the selected temporal cut;
- hidden intermediate nodes or edges remove the path from results;
- traversal counts and explain output must not reveal hidden labels, fanout,
  endpoint ids, or pruned path counts.
- fanout, path count, path length, and traversal time budgets are mandatory
  operation-context settings. Explain should report effective budgets when
  policy allows.

Example:

```coveql
node(Customer) as c
  .traverse(
    out(edge(CustomerPlacedOrder) as placed).to(node(Order) as o),
    min: 1,
    max: 1,
    mode: walk
  )
  .where(o.status == "shipped")
  .select(c.goid, o.goid, o.total)
```

### GraphTraversalContract Decision

Variable-length traversal becomes valid when the Graph profile exposes a
`GraphTraversalContract`. Without that contract, CoveQL 0.1 keeps the existing
fixed-hop behavior: omitted traversal bounds mean `min: 1`, `max: 1`,
`mode: walk`, and any other depth is rejected.

Accepted variable-length syntax:

```coveql
node(Person) as p
  .traverse(
    out(edge(Knows)).to(node(Person) as friend),
    min: 1,
    max: 3,
    mode: simple_path,
    distinct: none
  )
```

```text
GraphTraversalContract {
  contract_version,
  root_node_identity,
  edge_identity,
  path_identity,
  supported_modes,
  default_mode,
  min_depth_required_for_variable,
  max_depth_required_for_variable,
  max_depth_policy,
  fanout_budget_policy,
  path_count_budget_policy,
  frontier_budget_policy,
  traversal_time_budget_policy,
  duplicate_path_policy,
  canonical_path_order,
  visibility_authority,
  redaction_authority,
  temporal_alignment_policy,
  relationship_index_authority?,
  explain_disclosure_policy,
}
```

Required semantics:

- `min` and `max` are finite unsigned depths. `max` must be greater than or
  equal to `min`. `max` must be less than or equal to the operation-context
  maximum depth. Unbounded traversal is invalid.
- Omitting both `min` and `max` is the fixed-hop shorthand for `min: 1`,
  `max: 1`. Supplying one bound for variable-length traversal without the
  other rejects.
- `mode` is one of `walk`, `trail`, or `simple_path`.
- `walk` allows repeated nodes and edges.
- `trail` allows repeated nodes but not repeated edge identities in one path.
- `simple_path` allows neither repeated node identities nor repeated edge
  identities in one path.
- `distinct` is one of `none`, `path`, or `end_node`; default is `none`.
  `path` deduplicates by path identity. `end_node` deduplicates by start node,
  end node, depth, relationship expression, and temporal cut, preserving the
  first path under canonical path order.
- Path identity is the start node identity plus the ordered sequence of
  direction-qualified edge identities and node identities, the relationship
  expression fingerprint, mode, distinct policy, and temporal cut.
- Canonical path order is depth ascending, start node identity, then each hop's
  direction, edge identity, and target node identity. Implementations may use
  another order only if the contract declares it and explain reports it.
- Every node and edge in a path must be visible at the selected temporal cut.
  Hidden start nodes, hidden edges, hidden intermediate nodes, hidden end nodes,
  and redacted endpoints suppress the entire path.
- Counts, aggregates, negation, `exists`, and explain output must not reveal
  hidden labels, endpoint identities, fanout, suppressed path counts, or which
  budget was consumed by hidden data.
- `asOf` applies the same temporal cut to all nodes and edges unless an
  explicit temporal alignment policy declares otherwise. `history` and
  `changes` over variable-length paths require a separate path-history
  contract and otherwise reject.
- Relationship indexes may accelerate traversal only when they prove direction,
  endpoint role, temporal validity, visibility, redaction, and association type
  compatibility. Otherwise traversal falls back to the materialized graph
  oracle or rejects if the operation forbids fallback.

Resource limits:

- Planning rejects when static `max` exceeds the allowed maximum depth.
- Runtime must enforce maximum fanout per node, paths emitted, frontier size,
  and traversal time. If a budget is exceeded, the query fails with a resource
  diagnostic and emits no partial externally visible result.
- Explain may disclose configured budgets and consumed budgets only according
  to the explain disclosure policy. Protected graph structure must remain
  redacted.

Execution strategy:

- The semantic oracle is a materialized visible-graph traversal that applies
  temporal, visibility, redaction, mode, distinct, ordering, and budget rules.
- Native/indexed traversal may return results only when its output fingerprint
  is equivalent to the materialized oracle for the same snapshot and operation
  context. Debug/test mode should compare optimized path fingerprints against
  the materialized oracle.
- Coded execution may keep node and edge identities coded when those identities
  are protected identity tokens, but ordering and equality must use the graph
  identity contract rather than arbitrary dictionary or local shadow code order.

Explain output for variable-length traversal must include the traversal
contract version, min/max depth, mode, distinct policy, path identity class,
canonical path order, temporal alignment, visibility/redaction policy,
effective budgets, index/native acceleration decisions, residual/fallback
boundaries, and redacted resource-limit diagnostics.

## Mixed-Profile Query Rules

A CoveQL query has one primary profile. Common roots such as
`projection(...)` and `evidence(...)` may be used by all profiles when their
contracts allow it.

Cross-profile relationships require an explicit bridge contract. Examples that
need a bridge:

```coveql
object(Customer) as c
  .lookup(table(customer_scores) as s, on: c.goid == s.customer_goid)
```

```coveql
node(Customer) as c
  .where(exists(table(risk_flags) as r, on: c.goid == r.customer_goid))
```

```coveql
table(orders) as o
  .where(exists(edge(CustomerPlacedOrder) as e, on: o.order_id == e.target_goid))
```

Bridge contracts declare:

- source and target profiles;
- identity mapping;
- temporal alignment;
- null and missing-value behavior;
- code-domain or materialization requirements;
- visibility and redaction compatibility;
- explain fields and fallback behavior.

Without a bridge, mixed object/table/graph joins or traversals reject rather
than guessing semantics.

Formal bridge contract:

```text
CoveQLBridgeContract {
  bridge_id,
  bridge_version,
  source_profile,
  target_profile,
  source_grain,
  target_grain,
  source_identity_exprs,
  target_identity_exprs,
  cardinality,
  temporal_alignment,
  branch_alignment,
  tombstone_policy,
  null_missing_policy,
  code_domain_policy,
  materialization_requirement,
  visibility_compatibility,
  redaction_compatibility,
  aggregate_disclosure_policy,
  explain_fields,
  fallback_behavior,
}
```

Initial bridge requirements:

- object identity to graph node identity is required because CoveQL/Graph nodes
  are backed by object identity;
- association identity to graph edge identity is required because graph edges
  are backed by association identity;
- object/table bridges are optional unless a table surface declares row to
  object identity mapping;
- table/graph bridges are optional unless a declared bridge maps row identity
  to node or edge identity.

## Common Logical Plan

The shared logical plan should add profile-neutral scan nodes:

```text
ProfileScan {
  profile: object | graph | table | projection | evidence,
  root,
  grain,
  plan_context,
}

RelationshipExpand {
  input,
  relationship_kind,
  direction,
  target_profile,
}

LookupJoin {
  left,
  right,
  key_predicate,
  join_policy,
}

SemiJoin {
  left,
  right,
  key_or_relationship_predicate,
}

AntiJoin {
  left,
  right,
  key_or_relationship_predicate,
}

ProfileBridge {
  source_profile,
  target_profile,
  bridge_contract,
}
```

Profile-specific logical nodes can lower into these shared forms when possible.
For example:

```text
object association exists -> SemiJoin
graph out edge traversal -> RelationshipExpand + SemiJoin or path binding
table lookup -> LookupJoin
mixed profile bridge -> ProfileBridge + profile-specific join or traversal
```

## Dataset And Multi-File Scope

A CoveQL query runs against one validated file snapshot or one validated
dataset manifest.

For manifest-scoped queries, the operation context must include:

- dataset snapshot id;
- file membership fingerprint;
- schema compatibility proof;
- semantic-map compatibility proof;
- profile contract versions;
- bridge contract versions;
- cross-file code-domain policy;
- security-scope compatibility.

Codes from different files are not comparable unless decoded to canonical
logical values, remapped into a common `ExecutionCodeDomain`, or bridged by
declared semantic-domain metadata.

## Shared Physical Planning

CoveQL should keep the CoveQL/Object representation discipline:

- codes are representations, not values;
- dictionary equality is valid only in a proven code domain;
- arbitrary dictionary code order is not logical order;
- local shadow codes are never comparable across segments without remap;
- nulls are separate from ordinary codes;
- visibility and redaction apply before exact aggregates, index-only answers,
  zero-copy output, or explain disclosure.

The same physical operators can serve multiple profiles:

- temporal segment pruning;
- coverage pruning;
- COVE-I/COVX lookup;
- FileCode predicates;
- ExecutionCode remapping;
- numeric/date/time predicate lanes;
- dictionary-lifted functions;
- selection bitmaps and vectors;
- direct Arrow projection;
- materialized residual filters;
- policy-aware aggregate operators.

## SQL And DataFusion Interop

SQL should remain a first-class interop target, especially for the table
profile.

Recommended boundaries:

- CoveQL defines Cove semantics.
- SQL/DataFusion can execute projection/table plans when semantics are
  representable.
- DataFusion filters are advisory until translated into CoveQL predicate
  forms.
- Unsupported SQL filters remain residual.
- SQL null/collation/type coercion rules do not override Cove logical rules
  unless the table profile explicitly adopts them.
- Full relational joins, CTEs, window functions, and recursive queries remain
  SQL/DataFusion territory until CoveQL has a profile-native contract for them.

## Shared Explain Fields

Every explain JSON document should report the profile contract that shaped the
query, subject to policy redaction.

Example table explain fields:

```json
{
  "coveql_version": "0.1",
  "core_version": "0.1",
  "profiles": ["table"],
  "primary_profile": "table",
  "root": "table(orders)",
  "root_authority": "deterministic_projection",
  "grain": "visible_table_row",
  "operation": "table_scan",
  "identity_model": "declared_primary_row_identity",
  "canonical_order": ["table_id", "branch_key", "row_identity"],
  "temporal_mode": "latest",
  "logical_plan": [],
  "profile_methods": [],
  "security_barriers": [],
  "fallbacks": [],
  "security": {
    "visibility_applied": true,
    "redaction_applied": true
  },
  "diagnostics": []
}
```

Example graph explain fields:

```json
{
  "profiles": ["graph"],
  "root": "path(...)",
  "grain": "path_binding",
  "path_mode": "walk",
  "max_depth": 1,
  "hidden_endpoint_policy": "suppress_path"
}
```

Explain output must not reveal protected table names, graph labels, endpoint
ids, key domains, fanout, row counts, or pruned path counts unless the active
metadata disclosure policy allows it.

Mandatory CoveQL 0.1 explain fields:

- `coveql_version`;
- `core_version`;
- `primary_profile`;
- `profiles`;
- `root`;
- `grain`;
- `operation`;
- `temporal_mode`;
- `canonical_order` status;
- logical plan summary;
- fallback list;
- visibility and redaction applied flags;
- diagnostics.

Policy-optional fields include resolved ids, field names, table names, graph
labels, sidecar names, coverage details, row counts, fanout, hidden/pruned
counts, dictionary literals, and code-domain internals.

## Diagnostic Schema

CoveQL diagnostics should be structured, redaction-aware, and stable enough for
conformance tests.

```json
{
  "code": "E_AMBIGUOUS_PROFILE",
  "severity": "error",
  "phase": "resolution",
  "message": "Root can resolve through multiple profiles.",
  "span": {"start": 0, "end": 29},
  "profile": null,
  "safe_details": {
    "candidate_profiles": ["object", "table"]
  },
  "redacted": false
}
```

Initial diagnostic codes:

- `E_PARSE`;
- `E_UNSUPPORTED_PROFILE`;
- `E_AMBIGUOUS_PROFILE`;
- `E_UNKNOWN_ROOT`;
- `E_UNKNOWN_BINDING`;
- `E_AMBIGUOUS_FIELD`;
- `E_BINDING_OUT_OF_SCOPE`;
- `E_DUPLICATE_METHOD`;
- `E_INVALID_METHOD_PLACEMENT`;
- `E_CONFLICTING_TEMPORAL_METHODS`;
- `E_UNSUPPORTED_PROFILE_METHOD`;
- `E_UNKNOWN_ENUM_LITERAL`;
- `E_INVALID_CHANGES_BOUNDS`;
- `E_NON_BOOLEAN_PREDICATE`;
- `E_UNKNOWN_TABLE_SURFACE`;
- `E_UNKNOWN_GRAPH_LABEL`;
- `E_UNKNOWN_BRIDGE`;
- `E_UNSAFE_CODE_DOMAIN`;
- `E_SECURITY_DISCLOSURE_FORBIDDEN`;
- `E_RESOURCE_BUDGET_EXCEEDED`;
- `E_DATAFUSION_RESIDUAL_REQUIRED`.

Diagnostics must not reveal protected profile names, table names, graph labels,
field names, key domains, hidden row existence, hidden endpoint existence, or
sidecar metadata unless the active disclosure policy allows it.

## Fingerprints

CoveQL defines canonical fingerprints for:

- `QueryTextFingerprint`;
- `DirectiveFingerprint`;
- `ParsedAstFingerprint`;
- `ResolvedAstFingerprint`;
- `ProfileContractFingerprint`;
- `BridgeContractFingerprint`;
- `PredicateAstFingerprint`;
- `PredicateNormalFormFingerprint`;
- `LogicalPlanFingerprint`;
- `PhysicalPlanFingerprint`;
- `SecurityContextFingerprint`;
- `ExplainSchemaFingerprint`.

A cached, accelerated, or index-only plan is reusable only when the relevant
query, profile, bridge, security, snapshot, schema, semantic-map, and
sidecar/proof fingerprints match.

## Fallback And Rejection Matrix

Profile-specific fallback behavior should extend the core fallback matrix:

| Condition | Behavior |
| --- | --- |
| Unknown profile root | Reject. |
| Root profile disabled by host API | Reject. |
| Common root ambiguous across profiles | Reject. |
| Unknown profile method | Reject with `E_UNSUPPORTED_PROFILE_METHOD`. |
| Profile method used after `groupBy` without aggregate-grain support | Reject. |
| `lookup` without bridge from current profile | Reject. |
| `table(name)` resolves to a raw/external surface without `TableSurfaceContract` | Reject. |
| Table surface lacks row identity | Reject table root. |
| Table surface lacks canonical order | Reject any query whose result order matters, including default output order, `skip`, `take`, and lookup expansion. |
| Table surface lacks visibility/redaction authority | Reject unless a trusted internal mode is explicitly selected. |
| Table evidence helper used without table evidence capability | Reject. |
| `asOf` on materialized snapshot table | Reject. |
| `history` or `changes` on a surface without temporal capability | Reject. |
| Mixed `changes` bound kinds or temporal roles | Reject unless the profile declares exact conversion. |
| Variable-length graph traversal without `GraphTraversalContract` | Reject. |
| Variable-length graph traversal omits `min` or `max` | Reject. |
| Graph traversal exceeds max depth | Reject at planning if known; fail safely at execution otherwise with no partial externally visible results. |
| Graph traversal exceeds fanout, path-count, frontier, or time budget | Fail safely with a resource diagnostic and no partial externally visible results. |
| Relationship expression used outside a relationship-capable profile | Reject. |
| Hidden graph endpoint | Suppress path. |
| Hidden right-side table row | Treat as absent. |
| Duplicate lookup match with `cardinality: one` | Reject unless explicit duplicate policy. |
| Unknown enum named-argument value | Reject with profile-method diagnostic. |
| Boolean function used as predicate but returns non-boolean | Reject. |
| Evidence shorthand ambiguous across bindings | Reject. |
| `orderBy` uses alias not yet in scope | Reject. |
| Cross-file code-domain mismatch | Decode, remap, bridge, or reject. |
| External streaming requested in CoveQL 0.1 | Reject. |

## Resource Budgets

CoveQL-Core defines concrete parse, plan, execution, output, join, and
traversal budgets:

```text
max_query_bytes
max_ast_depth
max_bindings
max_output_columns
max_in_list_size
max_group_count
max_join_build_rows
max_lookup_matches_per_left_row
max_path_length
max_path_count
max_fanout_per_step
max_decode_bytes
max_scan_bytes
max_range_requests
max_planning_time_ms
max_execution_time_ms
```

Failure behavior:

- parse-time budget exceeded: reject;
- plan-time budget exceeded: reject with diagnostic;
- execution-time budget exceeded: cancel safely;
- partial results are forbidden unless streaming mode explicitly allows them.

CoveQL-Core 0.1 conforming implementations should accept conformance queries
at least up to:

```text
max_query_bytes >= 16 KiB
max_ast_depth >= 64
max_bindings >= 16
max_output_columns >= 64
max_in_list_size >= 1024
max_path_length >= 3 for graph conformance
```

For group count, path count, fanout, scan bytes, execution time, and join build
rows, fixtures should declare required budgets. Implementations should either
satisfy them or report `E_RESOURCE_BUDGET_EXCEEDED`. Effective runtime limits
should appear in explain output when policy allows.

## Streaming And Partial Results

Externally visible streaming output is not part of CoveQL 0.1 semantics.
Implementations may stream internally between operators, but a partial batch is
not a valid CoveQL result.

Before any future external streaming mode is accepted, it must define:

- batch ordering across the full canonical order;
- visibility and redaction validation before first batch emission;
- cancellation behavior;
- whether explain may be emitted before execution;
- whether partial batches are observable;
- how aggregate disclosure and hidden-row policies apply across batches.

## Conformance Profiles

CoveQL conformance is split by profile and shared layers:

```text
CoveQL-Core:
  parsing, method chains, diagnostics, security context, explain JSON,
  resource budgets, fingerprints, fallback matrix

CoveQL-Object:
  object roots, association roots, object evidence, temporal object state

CoveQL-Graph:
  node roots, edge roots, path roots, traversals, graph evidence

CoveQL-Table:
  table roots, column predicates, grouping, aggregates, lookup/semi/anti joins,
  row/column evidence, SQL/DataFusion interop
```

First CoveQL/Table conformance requires deterministic COVE-MAP projections and
existing table readback surfaces with declared row identity and canonical
ordering. Materialized projections, external registered tables, and raw table
sections are optional until they satisfy the table-surface contract.

First mixed-profile conformance requires object to graph node identity and
association to graph edge identity bridges. Object/table and table/graph
bridges are optional unless the fixture declares explicit bridge metadata.

Minimum CoveQL 0.1 parser conformance covers:

- directives;
- explicit roots and aliases;
- `where`, `select`, `groupBy`, `orderBy`, `skip`, and `take`;
- `asOf(csn)` and `asOf(commit_time)`;
- `history`, `changes`, and `includeTombstones`;
- `explain`;
- `lookup` syntax and `exists(table(...), on: ...)`;
- node, edge, path, relationship, and `traverse` syntax;
- `evidence(...)`;
- literals, comparisons, `&&`, `||`, `!`, `in`, `isNull`, and `isNotNull`;
- `count`, `sum`, `avg`, `min`, `max`, and `distinct_count`.

CoveQL 0.1 requires the following extension shapes at parser level:

```text
methods:
  lookup
  traverse

expressions:
  in(edge(...))
  out(edge(...))
  either(edge(...))
  exists(root, on: ...)
  exists(relationship_expr)
  evidence(...)
```

Profile semantics may reject these constructs when the active profile or bridge
contract does not declare support.

`history` and `changes` parser conformance covers all declared modes:

```text
history(mode: records)
history(mode: states)
history(mode: records_and_states)

changes(..., mode: records)
changes(..., mode: state_transitions)
changes(..., mode: property_diffs)
changes(..., mode: final_rows)
```

First semantic conformance requires:

- CoveQL/Object: `history(mode: states)` and
  `changes(..., mode: final_rows)`;
- CoveQL/Table: `changes(..., mode: final_rows)` only for temporal table
  fixtures, and `history(mode: records)` only where the table surface exposes
  temporal record grain;
- CoveQL/Graph: `asOf` over node, edge, and path state first; `history` and
  `changes` only for graph fixtures that declare temporal graph record or
  change grain.

Semantic conformance can still reject unsupported profile features with
structured diagnostics. Parser conformance keeps the grammar stable even
when execution support is staged.

Parser-positive fixture examples:

```coveql
object(Person).where(status == "active")

table(orders) as o
  .lookup(table(customers) as c, on: o.customer_id == c.customer_id)

node(Customer) as c
  .traverse(out(edge(CustomerPlacedOrder)).to(node(Order)))

table(orders).asOf(commit_time: "2026-01-01T00:00:00Z")

object(Person).changes(csn: 1, csn: 10, mode: final_rows)
```

Parser-negative fixture examples:

```coveql
path(table(orders).out(edge(X)))
```

Semantic-negative fixture examples:

```coveql
object(Person).asOf(csn: 1).asOf(csn: 2)

table(orders)
  .changes(csn: 1, commit_time: "2026-01-01T00:00:00Z")

table(orders)
  .where(revenue > 10)
  .groupBy(customer_id)

node(Customer)
  .where(exists(out(edge(X)), unsupported_arg: y))
```

Each profile should run the same three conformance tiers:

- semantic correctness without accelerators;
- fallback invariance with valid, missing, stale, corrupt, and unsupported
  metadata;
- acceleration proof for coverage, COVE-I/COVX, coded predicates,
  ExecutionCode, zero-copy, cache, and index-only paths.

## Implementation Sequence

The practical sequence is:

1. Rename the language family to CoveQL in docs and module names.
2. Define the CoveQL-Core and CoveQL profile contracts.
3. Classify the existing object-language proposal as CoveQL/Object and
   preserve its acceptance criteria.
4. Extract CoveQL-Core from the object proposal:
   parser, method chain, diagnostics, operation context, explain JSON,
   fingerprints, fallback matrix, security context, resource budgets, and
   profile contract loading.
5. Build coded/proof-safe physical-plan interfaces: operator structs,
   profile/bridge fingerprints, explain fields, fallback boundaries, and
   materialized-oracle comparison hooks for accelerated paths.
6. Add explicit root tags: `object`, `association`, `node`, `edge`, `path`,
   `table`, `projection`, and `evidence`.
7. Implement the CoveQL/Object compatibility path first so the rename does not
   weaken existing object semantics.
8. Implement CoveQL/Table over deterministic COVE-MAP projection/table
   readback: `table`, `where`, `select`, `groupBy`, `orderBy`, `take`, and
   `explain`.
9. Add table temporal reads where the underlying table/projection declares
   recomputable temporal authority.
10. Add row/column evidence for table roots.
11. Add constrained table joins: lookup, semi-join, anti-join.
12. Add CoveQL/Graph roots by mapping COVE-O objects and associations into
   nodes and edges.
13. Add path bindings, chained fixed-hop traversal, and explicit rejection for
   variable-length traversal until a graph traversal contract declares it.
14. Add `TableSurfaceContract` registration and validation for raw tables,
    materialized projections, and external registered tables. Keep
    deterministic projection-backed tables as the first authority.
15. Add `GraphTraversalContract` registration and validation for finite
    variable-length traversal, then implement the materialized oracle before
    indexed/native acceleration.
16. Enable shared coded/proof-safe physical execution across profiles after
   the corresponding semantic and fallback tests pass.

## Settled Milestone Decisions

The CoveQL 0.1 baseline uses one public crate, `coveql`. CoveQL/Object is
semantically implemented. CoveQL/Table is implemented for deterministic
COVE-MAP projection-backed table surfaces, row/column evidence targets,
grouping/aggregation, DataFusion scan interop, lookup joins, semi-joins, and
anti-joins. CoveQL/Graph is implemented for COVE-O object-backed nodes,
association-backed edges, fixed-hop path bindings, chained `traverse`, graph
relationship helpers, and node/edge/path evidence target contracts. Advanced
graph algorithms, variable-length traversal, raw table sections without a
table-surface contract, and broad SQL features remain outside CoveQL 0.1.

## Completion Decisions For Post-0.1 Implementation

The next implementation work can proceed without further semantic decisions if
it follows these contracts:

1. `TableSurfaceContract` is the gate for raw table sections, materialized
   projections, and external registered tables. Implement validation first,
   materialized authority second, then coded/DataFusion pushdown only after
   exact equivalence tests pass.
2. `GraphTraversalContract` is the gate for variable-length traversal.
   Implement the materialized visible-graph oracle first, including path
   identity, ordering, visibility, redaction, temporal alignment, dedupe, and
   budgets. Add indexed/native traversal only after debug/test comparison
   proves equivalence.
3. The materialized CoveQL path remains authoritative for both contracts until
   each optimized operator reports a proof and fallback boundary in explain
   output.
4. Security-sensitive metadata remains gated: table code-domain contexts,
   bridge proofs, dictionary epochs, graph fanout, hidden endpoints, suppressed
   paths, and budget consumption must be redacted unless policy explicitly
   allows disclosure.

## Recommendation

Proceed with **CoveQL** as the language family name.

Keep the current object query design, but classify it as **CoveQL/Object**.
Add **CoveQL/Graph** because objects and associations naturally map to nodes
and edges. Add **CoveQL/Table** for Cove-native table reads where temporal
state, evidence, proof-safe execution, and policy-aware explain make CoveQL
meaningfully better than SQL.

Do not position CoveQL/Table as a SQL replacement. Position it as the
semantic, temporal, evidence-aware, proof-aware read surface for Cove data,
with SQL/DataFusion kept as the general-purpose relational backend and
interoperability layer.
