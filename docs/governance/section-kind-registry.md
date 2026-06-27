# COVE v2.0 Section-Kind Registry

The COVE v2.0 section-kind registry records every stable section identifier, owning profile, requiredness rules, payload encoding, checksum policy, and compatibility behavior. New section kinds must include a normative parser, validation fixture, inspect summary, dump or report behavior where applicable, and a release-gate command when the section is publication-critical.

Section identifiers are globally stable, but feature-scope rules still apply to optional semantics inside each section. Unknown optional sections are skipped through extension fallback. Unknown required sections fail validation with a structured error.

Current COVE-R embedded section assignment:

| ID | Name | Profile | Required feature |
| --- | --- | --- | --- |
| 48 | RUNTIME_COMPATIBILITY_HINTS | COVE-R | FEATURE_RUNTIME_COMPATIBILITY_HINTS |

Current COVE-MAP embedded section assignments:

| ID | Name | Profile | Required feature |
| --- | --- | --- | --- |
| 60 | MAP_SOURCE_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 61 | MAP_FUNCTION_REGISTRY | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 62 | MAP_IDENTITY_RULE_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 63 | MAP_ROW_SEMANTICS_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 64 | MAP_ASSERTION_LOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 65 | MAP_IDENTITY_EQUIVALENCE_INDEX | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 66 | MAP_EVIDENCE_INDEX | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 67 | MAP_CONVERSION_REPORT | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 68 | MAP_PROJECTION_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |
| 69 | MAP_RESOLUTION_CATALOG | COVE-MAP | FEATURE_SEMANTIC_MAP |

The registry is checked by the conformance suite through:

```sh
cargo run -p cove-conformance --bin gen-corpus -- --check
cargo run -p cove-conformance --bin gen-capability-matrix -- --check
cargo run -p cove-conformance --bin cove-conformance -- conformance/
```
