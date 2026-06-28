# Predicates and Coverage

## 29. Neutral Predicate Semantics

**Predicate proof evaluation uses SQL WHERE semantics:**
**TRUE:**
  row is selected

**FALSE or UNKNOWN:**
  row is not selected
**A zone predicate proof returns:**
**DefinitelyNo:**
  no row in the zone can evaluate TRUE

**DefinitelyYes:**
  every row in the zone evaluates TRUE

**Unknown:**
  metadata cannot prove either

### 29.1 Composition Rules

**For A AND B:**
**if A=DefinitelyNo or B=DefinitelyNo:**
  DefinitelyNo

**else if A=DefinitelyYes and B=DefinitelyYes:**
  DefinitelyYes

**else:**
  Unknown
**For A OR B:**
**if A=DefinitelyYes or B=DefinitelyYes:**
  DefinitelyYes

**else if A=DefinitelyNo and B=DefinitelyNo:**
  DefinitelyNo

**else:**
  Unknown
**For NOT A:**
**if A=DefinitelyYes:**
  DefinitelyNo

**if A=DefinitelyNo:**
  DefinitelyYes only when SQL TRUE/FALSE/UNKNOWN semantics are safe

**otherwise:**
  Unknown
**Readers SHOULD be conservative with:**
- NOT,
- nullable columns,
- NaN-sensitive predicates,
- collation-dependent predicates,
- predicates that can evaluate UNKNOWN.

### 29.2 Examples

**For:**
age BETWEEN 18 AND 65
**A non-null zone with:**
min_age = 22
max_age = 51
**is:**
DefinitelyYes
**A zone with:**
max_age = 12
**is:**
DefinitelyNo
**A zone with:**
min_age = 10
max_age = 80
**is:**
Unknown
**For:**
status IN ('active', 'pending')
**A non-null exact set:**
`{active, pending}`
**is:**
DefinitelyYes
**A validated exact set:**
`{closed, cancelled}`
**is:**
DefinitelyNo


### 29.3 COVE-COVERAGE Query Coverage Semantics

COVE-COVERAGE defines the common vocabulary for conservative query coverage. A coverage set identifies the fragments that are sufficient to evaluate a predicate, answer an index-only query, route a lookup, or produce a conservative scan plan for a declared dataset snapshot.

```rust
enum CoverageGranularityV2 {
    Dataset = 0,
    Object = 1,
    File = 2,
    Segment = 3,
    RowGroup = 4,
    Page = 5,
    Morsel = 6,
    RowRange = 7,
    RowOrdinalSet = 8,
    MapNode = 9,
    DimensionalBucket = 10,
    ObjectPath = 11,
    Association = 12,
    ProjectionFragment = 13,
    ExternalFragment = 255,
}

enum CoverageProofKindV2 {
    MinMaxExclusion = 0,
    DictionaryMembership = 1,
    BloomMaybe = 2,
    ZoneMap = 3,
    ExactSet = 4,
    ValueToFragmentIndex = 5,
    RangeBucketLayout = 6,
    SemanticPathMapping = 7,
    ObjectDimensionMapping = 8,
    AggregateSynopsis = 9,
    LookupIndex = 10,
    CompositeZone = 11,
    EngineObservedCache = 12,
    ExternalIndex = 13,
    RuntimeHint = 14,
    VendorDefined = 255,
}

enum CoverageProofStrengthV2 {
    ExactTight = 0,
    ExactConservative = 1,
    ProbabilisticConservative = 2,
    AdvisoryOnly = 3,
    EngineLocal = 4,
    ApproximateMayUnderInclude = 5,
}

enum CoverageExactnessV2 {
    Exact = 0,
    ApproximateOverInclusiveOnly = 1,
    ApproximateMayUnderInclude = 2,
    Unknown = 255,
}
```

```rust
struct CoverageProviderDescriptorV2 {
    provider_id: u32,
    provider_kind: u16,          // CoverageProofKindV2 or registered extension kind
    profile: u8,                 // COVE-T/COVE-A/COVX/COVE-I/COVM/COVE-MAP/COVE-CACHE/etc.
    granularity: u8,
    proof_strength: u8,
    exactness: u8,
    flags: u16,

    referenced_table_id: u32,
    referenced_column_id: u32,   // u32::MAX when path/object/dataset scoped
    referenced_path_ref: u32,    // 0 when not path scoped
    logical_type: u16,
    collation_id: u16,
    null_semantics: u8,
    snapshot_validity_ref: u32,
    predicate_form_ref: u32,
    producer_ref: u32,
    checksum: u32,
}

struct CoverageSetHeaderV2 {
    coverage_set_id: u32,
    provider_id: u32,
    granularity: u8,
    proof_strength: u8,
    exactness: u8,
    flags: u8,

    predicate_form_ref: u32,
    snapshot_validity_ref: u32,

    total_fragment_count: u64,
    covered_fragment_count: u64,
    required_fragment_count_estimate: u64,

    coverage_degree_ppm: u32,
    tightness_degree_ppm: u32,

    entries_offset: u64,
    entries_length: u64,
    checksum: u32,
}

struct CoverageSetEntryV2 {
    target_kind: u16,            // CoverageGranularityV2
    flags: u16,
    file_ref: u32,
    table_id: u32,
    segment_id: u32,
    morsel_id: u32,
    page_ref: u32,
    object_type_id: u32,
    path_ref: u32,
    dimensional_bucket_ref: u32,
    row_start: u64,
    row_count: u64,
    row_ordinal_bitmap_ref: u32,
    byte_range_ref: u32,
    checksum: u32,
}
```

**Definitions:**
- A **coverage set** is a set of fragments that is sufficient to contain every row, object, association, projection row, or value that can satisfy the declared predicate context for the declared snapshot.
- A **tight coverage set** is a conservative coverage set that contains only necessary fragments according to the declared proof model.
- **coverage_degree_ppm** estimates how much of the full search space is covered by the set, expressed in parts per million. Smaller values normally imply less data to read.
- **tightness_degree_ppm** estimates how close the coverage set is to the tight set under the declared model. Higher values normally imply less over-inclusion.
- **coverage_confidence** and cost estimates are planning metadata only. They are not proof.

**Rules:**
- A reader MAY skip fragments outside a validated conservative coverage set only when `proof_strength` is `ExactTight`, `ExactConservative`, or `ProbabilisticConservative` with a no-false-negative contract that is understood.
- A reader MUST NOT skip fragments based on `AdvisoryOnly`, `EngineLocal`, `ApproximateMayUnderInclude`, stale, corrupt, or unsupported coverage metadata.
- Bloom-derived coverage MAY be conservative for exclusion only if the Bloom implementation guarantees no false negatives for the declared hash domain and snapshot. Bloom membership positives are candidates, not proof of match.
- Coverage metrics and cost estimates MUST NOT be used as correctness proof.
- Coverage sets MUST declare snapshot validity and MUST NOT be reused across dataset snapshots, schema changes, semantic-map versions, sidecar versions, or external visibility overlays unless the validity descriptor explicitly proves compatibility.
- Coverage entries that reference COVE rows or pages MUST identify the target file by `file_id` plus file length, footer CRC, and digest where available.
- Coverage over object/association/projection data MUST be interpreted through the declared COVE-O/COVE-MAP identity, temporal, projection, and evidence rules.

### 29.3.1 Coverage Set Entry Grammar and Invariants

`CoverageSetEntryV2` is a tagged union encoded as one fixed-width structure. The `target_kind` field determines which identifier fields are meaningful. Unused identifier fields MUST use the absent sentinel below and MUST NOT be interpreted by readers.

**Absent sentinels:**
- Identifier fields ending in `_id` use `u32::MAX` when absent.
- Reference fields ending in `_ref` use `u32::MAX` when absent unless the enclosing section explicitly defines `0` as the absent reference for that reference space.
- `row_start` MUST be `0` and `row_count` MUST be `0` when the entry is not row-range scoped.
- `checksum` covers the fixed entry with the checksum field zeroed.

| `target_kind` | Required fields | Optional fields | Required absent fields |
| --- | --- | --- | --- |
| `Dataset` | none beyond `snapshot_validity_ref` in the header | `byte_range_ref` for whole-dataset planning ranges | table/segment/morsel/page/object/path/bucket/row fields |
| `Object` | `object_type_id`, `path_ref` or object identity payload via `byte_range_ref` | `file_ref` when object rows are materialised in a file | segment/page/morsel row fields unless physically scoped |
| `File` | `file_ref` | `byte_range_ref` for file-level range hints | table/segment/morsel/page/row fields |
| `Segment` | `file_ref`, `table_id`, `segment_id` | `byte_range_ref` | morsel/page/row fields |
| `RowGroup` | `file_ref`, `table_id`, `segment_id`, `row_start`, `row_count` | `byte_range_ref` | morsel/page unless also declared by extension |
| `Page` | `file_ref`, `table_id`, `segment_id`, `page_ref` | `morsel_id`, `byte_range_ref` | row ordinal set unless explicitly page-row scoped |
| `Morsel` | `file_ref`, `table_id`, `segment_id`, `morsel_id` | `byte_range_ref` | page/row ordinal fields |
| `RowRange` | `file_ref`, `table_id`, `segment_id`, `row_start`, `row_count` | `morsel_id` if range is morsel-local | `row_ordinal_bitmap_ref` |
| `RowOrdinalSet` | `file_ref`, `table_id`, `row_ordinal_bitmap_ref` | `segment_id`, `morsel_id` | `row_count` unless the bitmap descriptor declares count |
| `MapNode` | `path_ref` | `file_ref`, `byte_range_ref` | table row fields unless map node is materialised as table rows |
| `DimensionalBucket` | `dimensional_bucket_ref` | `file_ref`, `table_id`, `segment_id`, `byte_range_ref` | row fields unless bucket maps to row ranges |
| `ObjectPath` | `object_type_id`, `path_ref` | `file_ref`, `byte_range_ref` | table row fields unless materialised |
| `Association` | `object_type_id` or `path_ref` identifying association type | endpoint references via extension payload | table row fields unless materialised |
| `ProjectionFragment` | `path_ref` or projection fragment ref | `file_ref`, `table_id`, `segment_id` | unused physical fields |
| `ExternalFragment` | `byte_range_ref` or extension payload | implementation-defined with required extension | all fields not declared by extension |

**Entry ordering and duplicate rules:**
- Entries in one `CoverageSetHeaderV2` payload MUST be sorted by `(target_kind, file_ref, table_id, segment_id, morsel_id, page_ref, object_type_id, path_ref, dimensional_bucket_ref, row_start, row_count)` after substituting absent sentinels.
- Exact duplicate entries are invalid.
- Row ranges for the same physical scope MUST be sorted by `row_start`, non-overlapping, and coalesced when adjacent unless the writer sets a diagnostic flag explaining why ranges are intentionally split.
- Row ordinal sets for the same physical scope MUST NOT overlap unless a required extension declares multiset semantics. COVE-Core/COVE-T coverage sets use mathematical set semantics, not bag semantics.
- A `CoverageSetHeaderV2` coverage set is the union of its entries.
- `total_fragment_count`, `covered_fragment_count`, `coverage_degree_ppm`, and `tightness_degree_ppm` are metrics; they MUST NOT be used to infer missing entries.

### 29.3.2 Coverage Set Algebra for Predicate Planning

Coverage set algebra is defined only for validated coverage sets over the same snapshot, schema fingerprint, semantic-map fingerprint when applicable, external visibility overlay state, predicate logical context, and compatible granularity.

```rust
enum CoverageSetOperationV2 {
    Union = 0,
    Intersection = 1,
    Difference = 2,
    Complement = 3,
    Coarsen = 4,
    Refine = 5,
}
```

**Rules:**
- For `A OR B`, a reader MAY use the union of the validated coverage sets for `A` and `B`.
- For `A AND B`, a reader MAY use the intersection of validated coverage sets only when both sets share compatible granularity and proof semantics. Otherwise it MUST use the narrower understood conservative set, a coarsened conservative set, or full scan fallback.
- For `NOT A`, a reader MUST NOT compute a complement coverage set unless the provider explicitly declares a complete universe, compatible null/UNKNOWN semantics, external visibility overlay compatibility, and exact complement proof. The default outcome for NOT is `Unknown`.
- `Difference` is allowed only for diagnostic or planner-estimation use unless the provider supplies exact set-difference proof under SQL three-valued semantics.
- `Coarsen` may convert row/page/morsel coverage to a broader granularity such as segment or file when all covered lower-level fragments map into the broader fragments. Coarsening is safe but may reduce tightness.
- `Refine` may split a broader fragment into narrower fragments only when a validated provider proves that no satisfying values exist outside the refined subset.
- If two coverage providers disagree, a reader MUST choose a conservative over-inclusive plan, ignore one provider, or scan. It MUST NOT use disagreement to under-include data.

### 29.3.3 Coverage Proof Records

A coverage set used for pruning or an index-only answer SHOULD be linked to an explicit proof record. A proof record binds the predicate form, provider, coverage set, snapshot validity, and proof semantics.

```rust
struct CoverageProofRecordV2 {
    proof_id: u32,
    provider_id: u32,
    coverage_set_id: u32,
    predicate_form_ref: u32,
    proof_kind: u16,
    proof_strength: u8,
    exactness: u8,
    granularity: u8,
    null_semantics: u8,
    flags: u16,
    snapshot_validity_ref: u32,
    coverage_set_checksum: u32,
    proof_payload_ref: u32,
    checksum: u32,
}
```

**Rules:**
- `coverage_set_checksum` MUST match the validated coverage set that is used.
- `proof_payload_ref` MAY reference provider-specific evidence such as min/max ranges, dictionary value sets, Bloom descriptor, index root, dimensional bucket definition, or COVE-MAP semantic path mapping.
- A proof record with unsupported `proof_kind`, unsupported collation, unsafe null semantics, stale validity, or checksum mismatch MUST NOT be used for pruning or exact answering.
- Approximate or may-under-include proof records MAY be used for advisory ranking or candidate generation only.

### 29.4 Predicate Normal Forms and Interval Predicates

Coverage providers need a stable predicate representation. COVE-COVERAGE defines several forms so readers can choose the weakest form that is sufficient and safe.

```rust
enum PredicateFormKindV2 {
    PredicateAst = 0,
    PredicateCnf = 1,
    IntervalPredicateForm = 2,
    EncodedPredicateForm = 3,
    EnginePrivate = 255,
}

struct PredicateNormalFormV2 {
    predicate_form_id: u32,
    form_kind: u16,
    flags: u16,
    logical_context_ref: u32,
    payload_offset: u64,
    payload_length: u64,
    checksum: u32,
}

struct IntervalPredicateV2 {
    column_or_path_ref: u32,
    logical_type: u16,
    collation_id: u16,
    null_policy: u8,          // 0=null_excluded, 1=null_included, 2=sql_unknown, 3=extension_defined
    bound_kind: u8,           // 0=lower_upper, 1=point, 2=open_range, 3=multi_interval_ref
    flags: u16,
    lower_inclusive: u8,
    upper_inclusive: u8,
    reserved: u16,
    lower_value_ref: u32,     // canonical value ref or u32::MAX for unbounded
    upper_value_ref: u32,     // canonical value ref or u32::MAX for unbounded
    checksum: u32,
}
```

### 29.4.1 Canonical Predicate Payload Grammar

`PredicateNormalFormV2.payload_offset` and `payload_length` identify one of the following payload grammars according to `form_kind`. All offsets are relative to the containing `PREDICATE_NORMAL_FORM` section payload unless the section kind explicitly says otherwise. Every payload is length-delimited and checksummed by `PredicateNormalFormV2.checksum`; nested payload records with their own checksum cover their own fixed fields with the checksum field zeroed.

```rust
enum PredicateOpV2 {
    TrueLiteral = 0,
    FalseLiteral = 1,
    IsNull = 2,
    IsNotNull = 3,
    Eq = 4,
    NotEq = 5,
    Lt = 6,
    LtEq = 7,
    Gt = 8,
    GtEq = 9,
    Between = 10,
    InSet = 11,
    And = 12,
    Or = 13,
    Not = 14,
    LikePrefix = 15,
    Contains = 16,
    IsNaN = 17,
    IsFinite = 18,
    FunctionCall = 19,
    LiteralValue = 20,
    ColumnRef = 21,
    Extension = 255,
}

enum PredicateNullPolicyV2 {
    SqlWhere = 0,          // TRUE selects; FALSE/UNKNOWN do not select
    NullExcluded = 1,
    NullIncluded = 2,
    NullOnly = 3,
    NullRejected = 4,
    ExtensionDefined = 255,
}

enum PredicateOperandKindV2 {
    Node = 0,
    Literal = 1,
    LiteralList = 2,
    ColumnOrPath = 3,
    Function = 4,
    IntervalSet = 5,
    Extension = 255,
}

struct PredicateAstPayloadHeaderV2 {
    root_node_id: u32,
    node_count: u32,
    literal_count: u32,
    literal_list_count: u32,
    function_count: u32,
    operand_ref_count: u32,

    node_offset: u64,
    literal_offset: u64,
    literal_list_offset: u64,
    function_offset: u64,
    operand_ref_offset: u64,

    flags: u32,
    checksum: u32,
}

struct PredicateAstOperandRefV2 {
    parent_node_id: u32,
    ordinal: u16,
    operand_kind: u8,       // PredicateOperandKindV2
    flags: u8,
    ref_id: u32,            // node_id, literal_id, literal_list_id, column_or_path_ref, function_ref, interval_set_id, or extension ref
    checksum: u32,
}

struct PredicateAstNodeV2 {
    node_id: u32,
    op: u16,                    // PredicateOpV2
    flags: u16,
    result_logical_type: u16,
    collation_id: u16,
    null_policy: u8,
    reserved0: u8,

    operand_count: u16,
    first_operand_index: u32,    // index into PredicateAstOperandRefV2 array, or u32::MAX

    column_or_path_ref: u32,     // fast-path mirror; u32::MAX when unused
    literal_ref: u32,            // fast-path mirror; u32::MAX when unused
    function_ref: u32,           // fast-path mirror; u32::MAX when unused
    aux_ref: u32,                // literal list, interval set, extension payload, or u32::MAX

    checksum: u32,
}

struct PredicateLiteralV2 {
    literal_id: u32,
    value_tag: u16,
    logical_type: u16,
    flags: u32,
    canonical_value_offset: u64,
    canonical_value_length: u32,
    checksum: u32,
}

struct PredicateLiteralListV2 {
    literal_list_id: u32,
    first_literal_index: u32,
    literal_count: u32,
    flags: u32,
    checksum: u32,
}

struct PredicateFunctionRefV2 {
    function_ref: u32,
    namespace_len: u16,
    namespace: [u8],
    name_len: u16,
    name: [u8],
    version_major: u16,
    version_minor: u16,
    deterministic: u8,
    flags: u8,
    required_extension_ref: u32,
    checksum: u32,
}
```

**Predicate AST reference spaces:**
- `root_node_id` MUST identify exactly one `PredicateAstNodeV2` unless the payload flag explicitly declares a fragment list.
- `node_id`, `literal_id`, `literal_list_id`, and `function_ref` are local to one predicate payload and MUST be unique within their own tables.
- If `operand_count == 0`, `first_operand_index` MUST be `u32::MAX`. If `operand_count > 0`, `first_operand_index` MUST NOT be `u32::MAX` and `first_operand_index + operand_count` MUST lie within the operand-ref array.
- Operand references for one node MUST be contiguous, sorted by `ordinal`, and have ordinals `0..operand_count-1` without gaps.
- The operand-ref table is the canonical predicate encoding. `column_or_path_ref`, `literal_ref`, `function_ref`, and `aux_ref` are redundant fast-path mirrors only. A mirror field MUST be `u32::MAX` or MUST exactly match the corresponding canonical operand. A mirror field MUST NOT satisfy an operator's arity requirement by itself.
- A reader MUST validate and interpret predicate semantics from operand references. If a non-`u32::MAX` mirror disagrees with the operand-ref table, the predicate payload is malformed and MUST NOT be used for pruning or exact answering.

**Operator arity and operand binding:**

| Operator | Required operands | Binding rules |
| --- | --- | --- |
| `TrueLiteral`, `FalseLiteral` | 0 | No column, literal, function, or interval operands. |
| `LiteralValue` | 1 literal operand | Produces the canonical literal value for expression-to-expression predicates. `literal_ref` MAY mirror the operand but is not canonical. |
| `ColumnRef` | 1 column/path operand | Produces a column/path value; range use still requires declared collation/order semantics. `column_or_path_ref` MAY mirror the operand but is not canonical. |
| `IsNull`, `IsNotNull`, `IsNaN`, `IsFinite` | 1 column/path or node | Operand 0 is the value being tested. Null and NaN semantics MUST be explicit. |
| `Eq`, `NotEq`, `Lt`, `LtEq`, `Gt`, `GtEq`, `LikePrefix`, `Contains` | 2 | Operand 0 is normally a column/path or expression node; operand 1 is normally a literal, literal-value node, or expression node. Simple column-literal atoms SHOULD mirror operands through `column_or_path_ref` and `literal_ref`. |
| `Between` | 3 | Operand 0 is column/path or expression; operand 1 is lower literal/expression; operand 2 is upper literal/expression. Flags bit 0 means lower inclusive; bit 1 means upper inclusive. Missing bounds MUST use `IntervalPredicateV2`, not an omitted AST operand. |
| `InSet` | 2 | Operand 0 is column/path or expression; operand 1 is `LiteralList`. Literal-list values MUST be canonical, sorted by declared equality/collation where applicable, and duplicate-free unless an extension defines multiset semantics. |
| `And`, `Or` | 2 or more node operands | N-ary logical operators are canonical. Writers SHOULD flatten nested same-op nodes and sort proof-safe atoms deterministically when doing so preserves semantics. |
| `Not` | 1 node operand | Readers MUST be conservative under SQL UNKNOWN semantics. `NOT` over nullable or NaN-sensitive expressions often remains `Unknown` for pruning. |
| `FunctionCall` | 1 function operand plus zero or more argument operands | Operand 0 MUST be a `Function` operand identifying the function. Argument operands follow the function operand by ordinal unless the function payload defines a different order. `function_ref` MAY mirror operand 0 but is not canonical. Functions used for pruning MUST be deterministic and fully versioned. |
| `Extension` | extension-defined | The required extension MUST define arity, operand kinds, null semantics, and proof safety. Unsupported extension nodes evaluate to `Unknown` for pruning. |

**Predicate AST rules:**
- AST nodes MUST form a finite acyclic graph with exactly one root unless the payload is explicitly a list of predicate fragments.
- Literal values MUST be COVE canonical value bytes. Display strings, raw source bytes, raw FileCodes, and engine-local ExecutionCodes are not valid predicate literals.
- `And` and `Or` nodes are n-ary and MUST use node operands. Binary logical trees MAY be normalised to n-ary form.
- `LiteralValue`, `ColumnRef`, `Between`, `InSet`, n-ary `And`/`Or`, `Not`, and `FunctionCall` MUST follow the arity table above through canonical operand references; malformed arity is a predicate-payload validation error.
- A `FunctionCall` used for pruning MUST reference a deterministic, versioned function with declared null, collation, timezone, and failure behaviour.
- Unknown predicate operations, unknown deterministic functions, malformed arity, and unsupported extension nodes MUST evaluate to `Unknown` for pruning.

### 29.4.2 CNF/DNF Payload Grammar

```rust
enum PredicateNormalisationKindV2 {
    Cnf = 0,
    Dnf = 1,
    FlatConjunction = 2,
    FlatDisjunction = 3,
}

struct PredicateNormalisedPayloadHeaderV2 {
    normalisation_kind: u8,
    flags: u8,
    reserved: u16,
    clause_count: u32,
    term_count: u32,
    clause_offset: u64,
    term_offset: u64,
    checksum: u32,
}

struct PredicateClauseEntryV2 {
    clause_id: u32,
    first_term_index: u32,
    term_count: u32,
    flags: u32,
    checksum: u32,
}

struct PredicateTermV2 {
    term_id: u32,
    ast_node_ref: u32,
    negated: u8,
    null_policy: u8,
    proof_safe: u8,
    reserved: u8,
    checksum: u32,
}
```

**CNF/DNF rules:**
- CNF and DNF payloads MUST reference AST atom nodes through `ast_node_ref`; they MUST NOT invent different literal semantics.
- Terms within a clause SHOULD be sorted by `(column_or_path_ref, op, literal canonical bytes)` for deterministic equality.
- Duplicate terms SHOULD be removed by writers and MAY be ignored by readers.
- A term marked `proof_safe = 0` MUST NOT be used for coverage exclusion or inclusion, but MAY remain in the predicate form for full evaluation.

### 29.4.3 Multi-Interval Predicate Payload Grammar

`IntervalPredicateV2.bound_kind = multi_interval_ref` references an `IntervalPredicateSetV2` payload. Multi-interval sets are used for `IN`, disjoint ranges, dimensional buckets, and predicate-containment caches.

```rust
struct IntervalPredicateSetV2 {
    interval_set_id: u32,
    column_or_path_ref: u32,
    logical_type: u16,
    collation_id: u16,
    null_policy: u8,
    flags: u8,
    interval_count: u32,
    intervals_offset: u64,
    checksum: u32,
}

struct IntervalBoundPairV2 {
    lower_value_ref: u32,       // u32::MAX for unbounded
    upper_value_ref: u32,       // u32::MAX for unbounded
    lower_inclusive: u8,
    upper_inclusive: u8,
    flags: u16,
    checksum: u32,
}
```

**Interval rules:**
- Intervals in one set MUST be sorted by lower bound under the declared collation and logical type.
- Intervals MUST be non-overlapping. Adjacent intervals SHOULD be coalesced when inclusivity makes them equivalent to a single range.
- `u32::MAX` unbounded sentinels are allowed only where the bound direction permits unbounded range semantics.
- Float intervals MUST declare NaN and signed-zero behaviour. Min/max or interval exclusion MUST NOT be used for NaN-sensitive predicates unless safe rules are declared.
- String intervals require a known compatible collation. Bytewise UTF-8 range rules are not a substitute for locale collation unless the column declares bytewise collation.

### 29.4.4 Encoded Predicate Form Payload Grammar

```rust
struct EncodedPredicateFormV2 {
    encoded_predicate_id: u32,
    baseline_predicate_ref: u32,
    table_id: u32,
    column_id: u32,
    logical_type: u16,
    physical_kind: u8,
    encoding_kind: u16,
    codec_id: u32,              // 0 when core encoding only
    flags: u32,
    equivalence_kind: u8,        // 0=exact_logical_equivalence, 1=conservative_no_false_negative, 2=advisory_only
    null_semantics: u8,
    collation_id: u16,
    params_offset: u64,
    params_length: u64,
    checksum: u32,
}
```

**Encoded predicate rules:**
- Encoded predicate evaluation is allowed only when the page encoding, NumCode descriptor, codec descriptor, and kernel capability all declare equivalence to baseline logical evaluation or conservative no-false-negative behaviour for the specific predicate class.
- `advisory_only` encoded predicate forms MUST NOT be used for pruning.
- Encoded predicates MUST preserve COVE structural null semantics and SQL TRUE/FALSE/UNKNOWN selection rules.
- FileCode encoded predicates may compare raw FileCodes for equality only after query literals are resolved through the same file dictionary. Range predicates over FileCode columns require ColumnDomain/domain-rank semantics.

**Rules:**
- `PredicateAst` is the general canonical predicate form.
- `PredicateCnf` is a normalised conjunction/disjunction form suitable for proof composition.
- `IntervalPredicateForm` is the range-compatible subset used by range pruning, dimensional buckets, coverage caches, and range indexes.
- `EncodedPredicateForm` may be used only when the underlying physical encoding declares the predicate physically safe and equivalent to baseline logical evaluation.
- Interval predicates MUST use canonical logical values, declared collation, declared null semantics, and length-delimited canonical bytes. They MUST NOT compare source display bytes, raw FileCodes, or engine-local ExecutionCodes.
- A predicate form with unknown functions, unknown collation, unsupported extension logical types, or unsafe null/NaN semantics MUST evaluate as `Unknown` for pruning unless a required extension defines safe behaviour.

### 29.5 Coverage Plan Candidates and Do-No-Harm Fallback

A tight coverage set is not always the best plan if computing it is more expensive than scanning a broader set. COVE therefore exposes costed coverage plan candidates without mandating a planner algorithm.

```rust
struct CoveragePlanCandidateV2 {
    candidate_id: u32,
    predicate_fragment_ref: u32,
    provider_id: u32,
    provider_type: u16,
    flags: u16,

    estimated_lookup_cost_ns: u64,
    estimated_coverage_size_bytes: u64,
    estimated_read_cost_ns: u64,
    estimated_decode_cost_ns: u64,
    estimated_materialisation_cost_ns: u64,
    baseline_scan_cost_estimate_ns: u64,

    max_allowed_estimated_cost_ns: u64,
    confidence_ppm: u32,
    calibration_epoch: u64,
    observed_error_bounds_ref: u32,
    fallback_policy: u8,
    reserved: [u8; 3],
    checksum: u32,
}

enum CoverageFallbackPolicyV2 {
    AdvisoryOnly = 0,
    FallbackRequired = 1,
    FullScanFallback = 2,
    WiderCoverageFallback = 3,
    RejectIfRequired = 4,
}
```

**Rules:**
- Coverage plan candidates are planning hints, not proof.
- A reader MAY ignore all plan candidates and derive a plan from ordinary COVE-T/COVE-A metadata.
- A reader SHOULD prefer plans whose estimated combined lookup, read, decode, and materialisation cost is lower than the baseline scan cost, but it is not required to use the writer's cost model.
- A reader MUST fall back to a wider conservative plan or full scan when a selected coverage provider is unavailable, stale, corrupt, too expensive under local policy, or unsupported.
- A cost estimate error MUST NOT change query results. It may only affect performance.
- If a plan candidate requires correctness trust in a sidecar, index, or cache, that sidecar/index/cache MUST validate under the selected snapshot before the plan is used.

---
