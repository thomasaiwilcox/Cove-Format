# Failure Behavior, Errors, and Compatibility

## 74. Recovery and Failure Behavior

| Condition | Default Behavior |
| --- | --- |
| Bad header magic | Reject file |
| Bad trailing magic | Reject file |
| Unsupported version | Reject file |
| Unknown required feature | Reject file |
| Unknown optional feature | Ignore if not needed |
| Header checksum mismatch | Reject file |
| Postscript checksum mismatch | Reject file |
| Footer CRC mismatch | Reject file |
| Required section CRC mismatch | Reject file |
| Optional index CRC mismatch | Ignore index and scan |
| Bloom corruption | Ignore bloom and scan |
| Exact set corruption | Ignore exact set and scan |
| Inverted index corruption | Ignore index and scan |
| Lookup index corruption | Ignore index and scan |
| Aggregate synopsis corruption | Ignore synopsis unless required by query-only plan |
| Composite zone corruption | Ignore composite zone and scan |
| Top-N summary corruption | Ignore summary and scan |
| COVE-E optional profile corrupt | Ignore profile |
| COVE-E required profile corrupt | Reject if needed |
| COVX stale/corrupt | Ignore COVX |
| COVM stale/corrupt | Ignore COVM |
| COVE-MAP artifact stale/corrupt | Ignore for ordinary reads; reject mapping replay/explanation/conversion if required |
| COVE-MAP identity conflict | Apply declared conflict behaviour; reject if no safe declared behaviour exists |
| COVE-MAP resolver catalog missing or unsupported | Reject conversion, replay, explanation, or resolver-aware projection that requires it |
| COVE-MAP resolver/catalog/pipeline digest mismatch | Reject resolver-aware operation |
| COVE-MAP alias ambiguous under merge policy | Reject conversion unless resolver routes ambiguity to candidate-only evidence |
| COVE-MAP reviewed decision conflict | Reject before materialisation |
| COVE-COVERAGE corrupt/stale/unsupported | Ignore coverage artifact and use wider conservative plan or full scan unless operation requires it |
| COVE-I stale/corrupt | Ignore index and scan unless operation explicitly requires the index |
| COVE-CACHE stale/corrupt | Ignore cache and plan from validated metadata or full scan |
| `.covedelta` malformed | Reject selected delta-bearing snapshot |
| Unsupported required delta feature | Reject selected snapshot or requested operation that needs the feature |
| Delta chain digest mismatch | Reject selected delta-bearing snapshot |
| Missing/corrupt required delta chain summary | Reject selected delta-bearing snapshot |
| Delta parent mismatch | Reject selected delta-bearing snapshot |
| Delta continuation anchor invalid | Reject selected delta-bearing snapshot |
| Delta state hash mismatch | Reject selected delta-bearing snapshot |
| Delta touched/tombstone summary under-includes | Reject full validation; do not use summary for pruning |
| Delta-bearing snapshot read by base-only path | Reject selected dataset snapshot rather than returning base-only state |
| Index-only exactness unsupported | Do not answer from index; scan or reject if index-only was explicitly required |
| Segment checksum mismatch | Reject segment; fail read unless explicit best-effort mode |
| Page checksum mismatch | Reject page; fail read unless explicit best-effort mode |
| Invalid FileCode | Treat as corruption |
| Invalid NumCode/logical type pairing | Schema error |
| Invalid prev_ref | Reject COVE-O file |
| Unsafe min/max | Do not use for skipping |

Best-effort mode MAY skip corrupt segments only when explicitly requested by recovery/export tooling.
Normal readers fail closed for structural corruption.

---


## 75. Durable Replace Protocol

Writers MUST publish COVE files by durable replace.
**Required protocol:**
1. Write complete candidate file to a temporary path in the target directory.
2. fsync/fdatasync the temporary file after all bytes are written.
3. Optionally reopen and validate header/footer/section CRCs.
4. Atomically rename the temporary file over the destination path.
5. fsync the parent directory to persist the rename.
6. Only after step 5 may the new COVE file be considered durable.
**Rules:**
- Writers MUST NOT claim durability on rename alone.
- If any step fails, the old file remains authoritative.
- Temporary files MUST be ignored, deleted, or quarantined.
- COVE files MUST NOT be modified in place.

---


## 76. Error Codes

| Code | Meaning |
| --- | --- |
| COVE_E_BAD_MAGIC | Missing or invalid magic. |
| COVE_E_BAD_VERSION | Unsupported COVE version. |
| COVE_E_UNKNOWN_REQUIRED_FEATURE | Unknown required feature bit set. |
| COVE_E_CHECKSUM_MISMATCH | Header, postscript, footer, section, segment, or page checksum mismatch. |
| COVE_E_DIGEST_MISMATCH | Cryptographic digest mismatch. |
| COVE_E_OFFSET_RANGE | Offset/length/count exceeds file bounds. |
| COVE_E_ARITH_OVERFLOW | Offset/count/size arithmetic overflow. |
| COVE_E_BAD_SECTION | Section malformed or invalid. |
| COVE_E_BAD_SCHEMA | Catalog/schema malformed. |
| COVE_E_BAD_LOGICAL_PHYSICAL_PAIR | Logical type incompatible with physical kind. |
| COVE_E_DICT_MISS | FileCode missing from dictionary. |
| COVE_E_BAD_FILECODE | FileCode outside dictionary range. |
| COVE_E_BAD_NUMCODE | NumCode invalid for declared logical type. |
| COVE_E_BAD_DOMAIN | ColumnDomain invalid. |
| COVE_E_BAD_STATS | Statistics invalid or unsafe. |
| COVE_E_BAD_INDEX | Optional index invalid or corrupt. |
| COVE_E_BAD_EXTENSION | Extension invalid or required extension unsupported. |
| COVE_E_BAD_CODEC_EXTENSION | COVE-CX codec descriptor, envelope, payload, fallback, or feature-bit contract is invalid. |
| COVE_E_CODEC_UNSUPPORTED | A required registered codec is not supported and no valid fallback exists. |
| COVE_E_BAD_LAYOUT_PLAN | COVE-L layout plan, split index, page cluster, zero-copy map, or fast metadata index is invalid. |
| COVE_E_RUNTIME_HINT_UNSUPPORTED | A required COVE-R runtime compatibility hint is unsupported for the requested runtime operation. |
| COVE_E_BAD_ENGINE_PROFILE | Engine profile invalid or unsupported when required. |
| COVE_E_EXECUTION_CODE_MAP | Engine-local code mapping failed. |
| COVE_E_HARBOR_MOUNT_LEASE | Harbor code lease resolution failed. |
| COVE_E_REF_INVALID | COVE-O prev_ref invalid. |
| COVE_E_NOT_SELF_CONTAINED | COVE-O chain lacks baseline/snapshot/full chain. |
| COVE_E_SEGMENT_CORRUPT | Segment structure invalid. |
| COVE_E_PAGE_CORRUPT | Page structure invalid. |
| COVE_E_REDACTION_POLICY | Redacted value cannot be surfaced under current policy. |
| COVE_E_SIDECAR_STALE | COVX/COVM sidecar does not match referenced COVE. |
| COVE_E_MAP_INVALID | COVE-MAP mapping artifact or embedded mapping section is malformed. |
| COVE_E_MAP_FUNCTION_UNDECLARED | Mapping references an undeclared or unsupported deterministic function. |
| COVE_E_MAP_IDENTITY_CONFLICT | Declared identity rules produce an unresolved merge/do-not-merge conflict. |
| COVE_E_MAP_SOURCE_STALE | Source snapshot, schema fingerprint, or source digest does not match the mapping run. |
| COVE_E_MAP_EVIDENCE_INVALID | Mapping evidence references a missing source, rule, row, assertion, or output object. |
| COVE_E_MAP_RESOLUTION_CATALOG_MISSING | Identity rule, projection, evidence replay, or candidate/review operation references a missing resolver catalog. |
| COVE_E_MAP_RESOLVER_UNSUPPORTED | Resolver kind, version, required function, or required policy is unsupported. |
| COVE_E_MAP_RESOLVER_DIGEST_MISMATCH | Embedded or external resolver digest does not match the declared digest. |
| COVE_E_MAP_CATALOG_DIGEST_MISMATCH | Embedded or external alias catalog digest does not match the declared digest. |
| COVE_E_MAP_PIPELINE_DIGEST_MISMATCH | Normalization pipeline or referenced table digest does not match the declared digest. |
| COVE_E_MAP_ALIAS_AMBIGUOUS | One normalised alias maps to multiple canonical keys without a valid ambiguity policy. |
| COVE_E_MAP_ALIAS_MISS | Alias lookup failed under `on_miss: reject`. |
| COVE_E_MAP_REVIEW_DECISION_CONFLICT | Reviewed same-object or do-not-merge decisions conflict. |
| COVE_E_MAP_DO_NOT_MERGE_VIOLATION | Merge plan violates a reviewed or declared do-not-merge constraint. |
| COVE_E_MAP_CANONICAL_ANCHOR_REQUIRED | Reviewed equivalence requires an explicit canonical anchor. |
| COVE_E_MAP_CANDIDATE_RULE_UNSUPPORTED | Candidate rule cannot be executed or validated deterministically. |
| COVE_E_MAP_RESOLUTION_NOT_REPLAYABLE | Resolver references unpinned external state or mutable resolver inputs. |
| COVE_E_BAD_COVERAGE | Coverage provider, predicate form, coverage set, proof strength, or coverage entry is invalid. |
| COVE_E_COVERAGE_STALE | Coverage artifact does not match the selected snapshot, schema, semantic map, file digest, or visibility overlay. |
| COVE_E_BAD_COVI | COVE-I secondary index artifact is malformed, stale, corrupt, or unsupported for the requested operation. |
| COVE_E_INDEX_ONLY_UNSAFE | Requested metadata/index-only answer is not exact or not valid for the selected snapshot/overlay. |
| COVE_E_CACHE_STALE | COVE-CACHE entry is stale, corrupt, approximate-may-under-include, or incompatible with the current runtime operation. |
| COVE_E_BAD_COVEDELTA | `.covedelta` artifact is malformed, corrupt, or has invalid `CVD2` framing. |
| COVE_E_DELTA_PROFILE_UNSUPPORTED | Delta-chain profile ID or version is unsupported. |
| COVE_E_DELTA_REQUIRED_FEATURE_UNSUPPORTED | Required delta feature bit is unsupported for the requested operation. |
| COVE_E_DELTA_CHAIN_DIGEST_MISMATCH | Selected base-plus-delta chain digest does not match COVM or catalog snapshot truth. |
| COVE_E_DELTA_CHAIN_SUMMARY_INVALID | Delta chain summary is missing, malformed, corrupt, digest-mismatched, or unsupported when required. |
| COVE_E_DELTA_PARENT_MISMATCH | Delta parent reference does not match selected parent snapshot, digest, artifact ID, or lineage rule. |
| COVE_E_DELTA_ANCHOR_INVALID | Continuation anchor is missing, too weak, or does not validate against logical parent state. |
| COVE_E_DELTA_STATE_HASH_MISMATCH | Continuation state hash or trust continuation hash does not match canonical logical state. |
| COVE_E_DELTA_SUMMARY_UNDER_INCLUDES | Touched, tombstone, or chain summary omits an affected object, property, tombstone, or delta. |
| COVE_E_DELTA_BASE_ONLY_SELECTED | Reader attempted to satisfy a selected delta-bearing snapshot by returning base-only state. |

---


## 77. Compatibility

### 77.1 Versioning

**COVE v2 readers support:**
version_major = 2

**Compatibility note:**
- Legacy pre-2 artifacts are outside this standard.
- A writer MUST NOT emit COVE magic for a file unless it intentionally opts
  into COVE conformance and validation rules.

**Rules:**
- Readers MUST reject unsupported major versions.
- Readers MAY accept newer minor versions if no unknown required features are set.

### 77.2 Required vs Optional Features

Required features are needed for correctness.
Optional features are accelerators or metadata.
**Examples:**
**Required:**
  - codec needed to decode projected data,
  - nested column support when projected,
  - trust-chain support when verification is requested,
  - engine profile required by requested output mode,
  - COVE-MAP artifact required by requested mapping replay, source-to-object conversion, or mapping explanation operation,
  - COVE-MAP resolver catalog required by resolver-backed identity conversion, deterministic replay, or resolver-aware projection,
  - COVE-O delta feature required by the selected delta-bearing snapshot operation.

**Optional:**
  - bloom filters,
  - exact sets,
  - lookup indexes,
  - aggregate synopses,
  - Top-N summaries,
  - COVX sidecars,
  - COVM manifests,
  - optional engine profile mappings,
  - COVE-MAP mapping artifacts and evidence when ordinary table/object reading does not request mapping replay or explanation,
  - COVE-O delta evidence, projection, coverage, index, or layout hints when ordinary object reconstruction does not select those optional sections.

---
