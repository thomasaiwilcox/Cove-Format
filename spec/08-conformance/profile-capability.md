# Profile Capability Matrix

## 71. Profile Capability Matrix

A public COVE implementation SHOULD declare which profile tier it supports.

| Feature | COVE-Core Reader | COVE-T Scan Reader | COVE-A Archive Reader | COVE-E Reader | COVE-H Harbor Reader | COVE-MAP Tool |
| --- | --- | --- | --- | --- | --- | --- |
| Validate header/footer/sections | Required | Required | Required | Required | Required | Required for COVE outputs |
| Decode FileCode to values | Required | Required | Required | Required | Required | Required when reading COVE sources/outputs |
| Decode NumCode columns | Required | Required | Required | Required | Required | Required when reading COVE sources/outputs |
| Arrow-compatible output | Recommended | Recommended | Recommended | Optional | Optional | Recommended for previews/projections |
| FileCode -> ExecutionCode | Optional | Recommended | Recommended | Required | Required as Harbor EngineCode | Optional; never identity truth |
| Engine profile registry | Optional | Optional | Optional | Required | Required | Optional |
| Morsel-aligned scanning | Optional | Required | Required | Optional | Required | Optional |
| Zone stats | Optional | Required | Required | Optional | Required | Optional |
| Predicate proof outcomes | Optional | Required | Required | Optional | Required | Optional |
| Exact sets | Optional | Recommended | Recommended | Optional | Recommended | Optional |
| Bloom filters | Optional | Recommended | Recommended | Optional | Recommended | Optional |
| Inverted morsel indexes | Optional | Optional | Recommended | Optional | Recommended | Optional |
| Lookup indexes | Optional | Optional | Recommended | Optional | Recommended | Optional |
| Aggregate synopses | Optional | Optional | Recommended | Optional | Recommended | Optional |
| Composite zone indexes | Optional | Optional | Recommended | Optional | Recommended | Optional |
| Top-N summaries | Optional | Optional | Recommended | Optional | Recommended | Optional |
| COVX sidecars | Optional | Optional | Optional | Optional | Optional | Optional |
| COVM manifests | Optional | Optional | Recommended | Optional | Recommended | Recommended for multi-file outputs |
| COVE-O object profile | Optional | Optional | Optional | Optional | Optional unless object-temporal semantics are requested | Required when destination is object-based COVE |
| COVE-O association readback | Optional | Not required | Optional | Optional | Recommended for object-association semantics | Required when association readback is claimed |
| COVE-MAP projection readback | Not required | Optional | Optional | Optional | Optional unless table projection is requested | Required when mapping-defined projection support is claimed |
| COVE-MAP semantic mapping | Not required | Not required | Not required | Not required | Not required unless mapping explanation is requested | Required |
| COVE-CX codec registry | Optional | Optional unless required codec pages are projected | Optional | Optional | Optional | Optional |
| Registered codec decode | Not required unless feature is required | Required when projected pages use required registered codecs | Required when projected pages use required registered codecs | Optional | Optional | Optional |
| COVE-L layout plan | Not required | Optional | Recommended for large archive planning | Optional | Optional | Optional |
| Scan split index | Not required | Optional | Recommended | Optional | Optional | Optional |
| Zero-copy buffer map | Optional | Optional | Optional | Optional | Optional | Optional |
| COVE-R runtime registry hints | Optional | Optional | Optional | Optional | Optional | Optional |
| COVE-H Harbor mount profile | Not required | Not required | Not required | Not required | Required only for COVE-H | Not required |

---
