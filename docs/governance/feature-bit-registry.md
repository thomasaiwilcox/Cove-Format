# COVE v2.0 Feature-Bit Registry

The COVE v2.0 feature-bit registry assigns bits by feature scope: core file features, scan/table features, object/profile features, COVE-MAP features, and sidecar features each maintain their own namespace. Feature-scope isolation prevents a bit allocated for one section family from changing the interpretation of another family.

Registry updates must document the bit name, scope, required/optional behavior, validation impact, reader fallback, and conformance vectors. Required feature bits need accept and reject fixtures. Optional feature bits need at least inspect/report coverage proving extension fallback behavior for readers that do not implement the feature.

Current v2 publication gates require the conformance command set:

## COVE-AI Feature Word

COVE-AI uses extended global feature word `1` for embedded `.cove` AI
metadata. The same bit numbers are used as artifact-local AI feature words
inside `CVA2` (`.coveai`) and `CVV2` (`.covev`) companion artifacts.

Writers MUST NOT place operation-only COVE-AI requirements in `.cove`
`required_features` word 0. Embedded AI requirements must be scoped through
`EXTENDED_FEATURE_SET`, `SECTION_FEATURE_BINDING`, profile capability matrices,
section entries, or operation bindings. Unsupported optional AI bits must not
change ordinary COVE-T/O/MAP logical results.

| Bit | Name | Scope |
| ---: | --- | --- |
| 0 | AI_FEATURE_MAP_AI_POLICY | COVE-MAP-AI |
| 1 | AI_FEATURE_CHUNK | COVE-CHUNK |
| 2 | AI_FEATURE_TOKEN | COVE-TOK |
| 3 | AI_FEATURE_VECTOR | COVE-VEC |
| 4 | AI_FEATURE_VECTOR_INDEX | COVE-VEC |
| 5 | AI_FEATURE_TENSOR_LAYOUT | COVE-VEC / COVE-MMSEQ |
| 6 | AI_FEATURE_ASSET_REF | COVE-MMSEQ / COVE-TRAIN |
| 7 | AI_FEATURE_MMSEQ | COVE-MMSEQ |
| 8 | AI_FEATURE_TRAIN | COVE-TRAIN |
| 9 | AI_FEATURE_GENERATOR_PROVENANCE | COVE-TRAIN |
| 10 | AI_FEATURE_COVEQL_AI | CoveQL-AI |
| 11 | AI_FEATURE_CANONICAL_FIXED_POINT_VECTOR | COVE-VEC |
| 12 | AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED | COVE-MMSEQ / COVE-TRAIN |
| 13 | AI_FEATURE_PRIVACY_SUMMARY | COVE-AI |
| 14 | AI_FEATURE_VECTOR_SPACE_COMPATIBILITY | COVE-VEC |

```sh
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```
