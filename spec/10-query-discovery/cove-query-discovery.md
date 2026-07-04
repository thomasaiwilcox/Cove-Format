# COVE-QD Query Discovery Profile

## 90. COVE-QD Optional Query Discovery Profile

COVE-QD defines optional advisory metadata that helps readers, tools, LLMs, and
AI agents discover CoveQL query surfaces. COVE-QD metadata is not archive truth.
Readers may ignore it. Query planners MUST resolve and validate all generated
CoveQL against canonical COVE metadata and active policy.

CoveQL remains the executable query language. COVE-QD guides query generation;
canonical COVE metadata, policy, sidecar validation, and CoveQL planning decide.

The COVE-QD profile reserves:

| Registry | Value | Name |
| --- | ---: | --- |
| Primary profile | 12 | COVE-QD / Query Discovery |
| Section kind | 90 | `QUERY_DISCOVERY_MANIFEST` |
| Low-word feature bit | `0x0000_0200_0000_0000` | `FEATURE_QUERY_DISCOVERY_METADATA` |

**Profile rules:**

- COVE-QD is optional.
- Ordinary COVE-T, COVE-O, COVE-MAP, COVE-AI, and CoveQL readers MAY ignore it.
- COVE-QD MUST NOT be required for ordinary table scans, object reconstruction,
  mapping readback, evidence readback, or AI sidecar validation.
- COVE-QD owns only discovery metadata. It does not own root semantics, table
  semantics, object semantics, evidence semantics, AI payload semantics, sidecar
  validity, or query execution authority.
- COVE-QD may describe query roots and capabilities owned by other profiles, but
  the owning CoveQL profile and canonical COVE metadata remain authoritative.

## 91. QUERY_DISCOVERY_MANIFEST Section

`QUERY_DISCOVERY_MANIFEST` stores a canonical UTF-8 JSON payload using schema
`cove.query_discovery.v1`.

**Embedding rules:**

- The section is optional.
- Writers embedding this section in ordinary `.cove` data artifacts MUST set
  `FEATURE_QUERY_DISCOVERY_METADATA` in `optional_features`.
- Writers MUST NOT set `FEATURE_QUERY_DISCOVERY_METADATA` in file-level
  `required_features` for ordinary data artifacts.
- A writer MAY make query discovery required only for a dedicated discovery-only
  or tooling-contract artifact whose declared purpose is not ordinary data
  reading.
- For COVM or other manifest-style artifacts, equivalent requiredness signaling
  MUST follow that artifact's own extension/requiredness rules and MUST preserve
  ordinary dataset readability when query discovery is optional.
- A reader that does not implement COVE-QD MUST be able to skip
  `QUERY_DISCOVERY_MANIFEST` using ordinary section-directory and
  feature-requiredness rules.

**Payload rules:**

- The payload MUST be canonical UTF-8 JSON using RFC 8785 JSON Canonicalization
  Scheme (JCS).
- Duplicate object keys are invalid.
- The payload MUST contain `schema`, `canonicalization`, `authority`,
  `source_binding`, `coveql`, `surfaces`, and `policy`.
- The payload MUST NOT contain protected values, copied source payloads, token
  IDs, vectors, embeddings, prompt text, prompt context, hidden sidecar payloads,
  or hidden metadata.
- The payload MUST NOT define feature requiredness. Binary feature words,
  section entries, profile capability matrices, and feature-binding sections
  remain authoritative for requiredness.

## 92. Manifest Schema

The top-level manifest object has this shape:

```json
{
  "schema": "cove.query_discovery.v1",
  "canonicalization": "rfc8785-jcs",
  "manifest_id": "optional-stable-id",
  "generated_at": "2026-07-03T00:00:00Z",
  "generated_by": {},
  "manifest_features": [],
  "authority": "advisory_discovery_not_archive_truth",
  "source_binding": {},
  "coveql": {},
  "surfaces": {},
  "alias_bindings": [],
  "relationships": [],
  "property_glossary": [],
  "templates": [],
  "examples": [],
  "ai": {},
  "policy": {},
  "resource_budgets": {},
  "diagnostics": []
}
```

Required fields:

- `schema`
- `canonicalization`
- `authority`
- `source_binding`
- `coveql`
- `surfaces`
- `policy`

`authority` MUST be `advisory_discovery_not_archive_truth`.

The structural JSON Schema for this version is
`spec/10-query-discovery/cove.query_discovery.v1.schema.json`. The schema is a
validation aid; it does not replace duplicate-key rejection, RFC 8785 JCS
canonicalization checks, source-binding freshness checks, policy compatibility
checks, sidecar validation, or ordinary CoveQL parse/resolve/plan/execute
validation.

Fields that may exceed the interoperable JSON safe-integer range
(`-(2^53 - 1)` through `2^53 - 1`) MUST be encoded as decimal strings or typed
string-valued objects. This includes CSNs, byte lengths, row counts, object
identifiers, file offsets, section lengths, and timestamp counters.

Generators SHOULD emit arrays in deterministic order, such as canonical root
order followed by `query_identifier` order, unless a profile defines a stronger
canonical ordering.

`manifest_features` describe optional interpretation rules or blocks beyond the
baseline `cove.query_discovery.v1` schema. They MUST NOT describe COVE dataset
capabilities; dataset capabilities belong in `coveql.capabilities` or
profile-specific surface records. Unknown `manifest_features` MUST be rejected
for automated query generation unless the caller explicitly selects human-only
best-effort inspection.

Best-effort inspection MAY display unknown or partially validated manifest
content to a human diagnostic surface. It MUST NOT guide automated query
generation, template expansion, AI sidecar selection, or policy decisions.

## 93. Source Binding and Freshness

The manifest MUST bind to a selected COVE file, COVM dataset snapshot, or
catalog-published source snapshot. Source binding SHOULD include enough
digest/fingerprint information to detect stale file content, footer state,
schema, dictionary, COVE-MAP metadata, COVM snapshot state, delta-chain state,
branch/CSN selection, visibility scope, redaction scope, policy fingerprint,
principal class, and audience.

For embedded `QUERY_DISCOVERY_MANIFEST` sections, source binding MUST use a
non-self-referential source identity, such as:

- a canonical source snapshot digest;
- a digest over the containing artifact excluding the query-discovery section;
- explicit schema, dictionary, map, footer, policy, and COVM snapshot
  fingerprints.

An embedded manifest MUST NOT require a digest that includes its own bytes unless
an external envelope defines the signing procedure.

If source binding is stale, policy-incompatible, audience-incompatible, or
principal-incompatible, strict discovery MUST ignore the manifest or report it as
stale. A stale manifest MUST NOT guide generated queries in strict mode.

URI hints such as `source_uri_hint` and sidecar `uri_hint` are untrusted location
hints. Agent runtimes and tools MUST NOT fetch, open, or resolve URI hints
automatically unless the host application resolves them through an explicit
allowlist, sandbox, catalog policy, or user-approved file selection.

## 94. CoveQL Contract

The `coveql` block declares the CoveQL language version, core version, available
profiles, roots, capabilities, profile contract versions, and allowed explain
modes for the active policy context.

CoveQL profiles are versioned language contracts such as `table`, `object`,
`graph`, and `ai`. Roots are query entry points exposed by those profiles, such
as `table(...)`, `object(...)`, `association(...)`, `projection(...)`, and
`evidence(...)`. Evidence is a root or capability, not a standalone profile.

Agents and tools SHOULD emit CoveQL profile directives when multiple profiles
are enabled. Tools MUST reject generated queries that request unavailable
profiles, roots, or capabilities unless an explicitly selected exploratory mode
allows ordinary CoveQL diagnostics to surface the failure.

Public manifests MUST NOT advertise developer explain modes or privileged
diagnostics unless the active policy explicitly allows them.

## 95. Surfaces, Identifiers, and Aliases

Surface records describe roots an agent may use, including tables, objects,
projections, evidence roots, and other profile-defined query surfaces.

Surface records that expose names SHOULD include:

- `name`: compact compatibility field;
- `query_name`: resolver-recognized logical name;
- `query_identifier`: exact CoveQL-safe identifier atom to emit;
- `display_name`: human-facing label.

Agents MUST emit `query_identifier`, not `display_name` or raw `query_name`, when
constructing CoveQL text. If a name requires quoting, `query_identifier` MUST
include the CoveQL quoting form, for example `"\"order-history\""`, and `root`
MUST use that same identifier, as in `table("order-history")`.

When a complete `root` string is provided, agents SHOULD prefer the validated
root string over reconstructing the root from individual fields unless they are
using a structured template parameter that requires the root kind and identifier
separately.

Evidence surfaces MAY expose a complete validated `root` string instead of
separate query identifiers when the evidence root is derived from another
surface. Agents SHOULD prefer the complete root and MUST NOT reconstruct
evidence roots from display labels or target strings.

Object properties and table columns SHOULD use rich records with
`query_identifier`, logical type, nullability, and allowed operations.
Plain-string property or column lists are allowed only when every string is
already a resolver-recognized unquoted CoveQL identifier and no display aliasing
or redaction is involved.

Alias bindings are optional. Agents MAY emit an alias only by resolving it
through `alias_bindings` to a `query_identifier`. Agents MUST NOT infer aliases
from display names, examples, descriptions, or glossary text.

`property_glossary` entries MAY describe table columns, object properties, and
projection columns. Glossary entries MUST use query-safe paths when paths are
provided, MUST NOT include source values by default, and MUST treat
descriptions, lineage labels, and policy notes as untrusted metadata.

`relationships` entries MAY be derived from object-catalog association flags or
from embedded COVE-MAP row-semantics association bindings. Relationship records
are advisory. They SHOULD include a validated `association_root`, bounded
example, authority label, and any canonical endpoint metadata available under
the active disclosure policy. Missing `from_root` or `to_root` means the
generator could not safely resolve the endpoint root from metadata already in
hand; agents MUST NOT infer hidden endpoints from names or descriptions.

`root_index`, when present, SHOULD list canonical rendered root strings in
deterministic order and is an index over emitted surfaces, not a grant of query
authority.

The resolver still validates every root, identifier, path, relationship, and
capability against canonical metadata and active policy.

## 96. Templates and Examples

Templates are the primary automated query-generation bridge. A template MUST use
typed parameters and a structured `operator_chain` or equivalent method-chain
builder. `template_display` is illustrative only; it MUST NOT be the binding
authority.

Template expansion MUST bind structured values or parsed AST fragments. It MUST
NOT perform raw string substitution into CoveQL. A parameter kind that accepts
arbitrary CoveQL text is invalid for a safe agent manifest.

Query discovery operator chains support `root`, `where`, `select`, `orderBy`,
`groupBy`, `take`, `skip`, `explain`, and profile methods explicitly declared by
the template. Profile-specific methods MUST declare the required profiles, roots,
capabilities, sidecars, and resource budgets. A method such as `similar` MUST
declare the `ai` profile, the similarity capability, and its sidecar
requirements.

Fragment parameters MUST declare allowed CoveQL-safe identifiers, semantic
operators, literal types, and complexity limits. `allowed_operators` are semantic
operator names, not raw CoveQL source tokens; renderers map them to canonical
CoveQL syntax. `allowed_literal_types` is a coarse bound; the renderer or
resolver MUST still enforce field-specific type compatibility before execution.

Identifier constraints SHOULD be scoped to the selected root when a template
accepts more than one root. Keys in root-scoped identifier maps MUST be canonical
CoveQL root strings.

The final rendered CoveQL text MUST be parsed again, or equivalently validated
through the same parser/resolver path, so template rendering cannot bypass
ordinary CoveQL diagnostics and policy enforcement.

Example queries are recommendations, not conformance requirements. Trusted
tooling SHOULD parse, resolve, and perform no-payload planning dry-runs for
emitted examples when policy and budget allow. If an example cannot be validated
without reading protected payloads or exceeding budget, it MUST be marked with a
diagnostic and MUST NOT be used for automated query generation in strict mode.

Template records SHOULD carry `template_validation` when emitted by trusted
tooling. Allowed values are `operator_chain_validated`,
`parameters_validated`, `representative_planned_dry_run`,
`not_validated_policy_limited`, `not_validated_budget_limited`, and
`not_validated`.

Example records SHOULD carry `query_validation` when emitted by trusted
tooling. Allowed values are `parsed_and_resolved`, `parsed_only`,
`planned_dry_run`, `not_validated_policy_limited`,
`not_validated_budget_limited`, and `not_validated`. Public-mode generators
SHOULD prefer `planned_dry_run` when policy and budget allow. Generators that
cannot invoke the CoveQL parser/resolver/planner from their implementation layer
MUST mark emitted examples as `not_validated` and SHOULD add a diagnostic such
as `QD_EXAMPLE_VALIDATION_NOT_PERFORMED`.

Example and template validation SHOULD prefer parse checks, root resolution,
identifier resolution, profile/capability checks, and no-payload planning
dry-runs. It MUST NOT read payload pages, expose values, materialize rows, or
lease AI payloads unless the caller explicitly requests query execution.

## 97. Policy, Diagnostics, and Resource Budgets

COVE-QD manifests are generated under a metadata-disclosure policy. A manifest
generated under a broader disclosure scope MUST NOT be reused under a narrower
scope unless a trusted tool revalidates and filters it.

The policy block SHOULD describe visibility scope, redaction scope, metadata
disclosure labels, aggregate disclosure labels, policy fingerprint, policy
version, principal class, audience, and whether forbidden surfaces were withheld.
Disclosure values are local policy labels unless another policy profile defines
a fixed enum.

Resource budgets SHOULD include default and maximum `take` limits, graph depth,
path counts, prompt-context chunk limits, export-row limits, and operations that
require explicit budgets. Agents SHOULD preserve or tighten budgets and MUST NOT
remove `.take(...)`, traversal budgets, context budgets, or policy filters from
templates.

Diagnostics MAY explain that data was withheld, stale, unsupported, invalid, or
budget-limited, but policy remains authoritative. Diagnostic text is untrusted
data. Allowed severities are `info`, `warning`, and `error`.

Diagnostic records SHOULD contain `code`, `severity`, and `message`, and MAY
contain `target_kind`, `target`, and `withheld`. Diagnostic codes SHOULD be
stable, upper-case identifiers such as `QD_POLICY_FILTERED_FIELD`.

Diagnostic `target`, when present, SHOULD be a JSON Pointer into the manifest.
Public diagnostics MUST NOT reveal hidden field names, source identifiers,
internal plan details, privileged policy decisions, hidden metadata paths, or
omitted hidden fields. Public diagnostic targets SHOULD point only to nodes
present in the emitted manifest, or to a safe parent node.

Validation reports are external to the canonical manifest payload unless carried
by an explicit envelope. A manifest MUST NOT assert its own current validity,
because validity depends on selected source, policy, principal, audience, and
sidecar context at validation time.

The canonical manifest object MUST NOT include top-level `validation_status` or
`validation_flags` members. External envelopes MAY place validation metadata
beside the manifest object.

Allowed validation status values are `valid`, `stale`, and `invalid`. Allowed
validation flags include `policy_filtered`, `diagnostics_withheld`,
`examples_limited`, and `ai_limited`.

## 98. COVE-AI Capability Advertising

The manifest MAY advertise AI operations that are available when the caller opts
into the CoveQL `ai` profile and validates the required sidecars.

AI capability records are hints. COVE-AI validation remains authoritative. The
manifest MUST NOT contain vectors, embeddings, token IDs, protected chunk text,
prompt context text, copied training samples, hidden sidecar payloads, or
protected source payloads.

Stale, missing, unsupported, or policy-blocked AI sidecars MUST produce ordinary
CoveQL-AI diagnostics. Query discovery MUST NOT make AI metadata baseline truth.

## 99. Validation Order and Conformance

A conforming COVE-QD reader SHOULD validate in this order:

1. Parse UTF-8 JSON and reject duplicate object keys.
2. Validate RFC 8785 JCS canonicalization for embedded
   `QUERY_DISCOVERY_MANIFEST` sections and whenever strict canonical mode,
   digest validation, or signature validation is requested.
3. Validate `schema`, `canonicalization`, and advisory `authority`.
4. Validate `manifest_features`.
5. Validate source binding against the selected file, COVM snapshot, catalog
   snapshot, or embedded non-self-referential source identity.
6. Validate policy fingerprint, policy version, principal class, audience,
   visibility scope, redaction scope, and duplicated source/policy fields.
7. Validate CoveQL version, profile names, roots, and capabilities.
8. Parse surface root strings as CoveQL roots.
9. Validate resolver-recognized query names and alias bindings.
10. Validate template parameter declarations, identifiers, semantic operators,
    literal types, complexity limits, and operator chains.
11. Parse/resolve examples and perform no-payload planning dry-runs when policy
    and budget allow.
12. Validate AI sidecar references only when AI diagnostics or AI methods are
    selected.

Conformance vectors for COVE-QD MUST cover:

- valid minimal manifests;
- duplicate JSON keys;
- unsafe large JSON numbers;
- quoted identifiers and reserved words;
- template injection attempts;
- root-scoped identifier constraints;
- stale source bindings and stale embedded self-bindings;
- `FEATURE_QUERY_DISCOVERY_METADATA` optional-bit behavior;
- rejection of query-discovery-required features for ordinary data artifacts;
- acceptance of query-discovery-required features only for declared
  discovery-only/tooling artifacts;
- ordinary reads ignoring optional COVE-QD sections;
- public diagnostic redaction;
- URI hints not being fetched automatically;
- COVE-QD not creating roots absent from canonical metadata;
- lying manifests being rejected by ordinary CoveQL resolution;
- stale, missing, unsupported, or policy-blocked AI sidecars failing closed for
  AI operations.
