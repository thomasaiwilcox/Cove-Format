#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelpTopic {
    Global,
    Query,
    Inspect,
    Optimize,
    Showcase,
    Sidecar,
    Delta,
    Convert,
    Map,
}

pub(crate) fn print_usage(topic: HelpTopic) {
    println!("{}", usage(topic));
}

pub(crate) fn usage(topic: HelpTopic) -> String {
    match topic {
        HelpTopic::Global => global_usage(),
        HelpTopic::Query => query_usage(),
        HelpTopic::Inspect => inspect_usage(),
        HelpTopic::Optimize => optimize_usage(),
        HelpTopic::Showcase => showcase_usage(),
        HelpTopic::Sidecar => sidecar_usage(),
        HelpTopic::Delta => delta_usage(),
        HelpTopic::Convert => convert_usage(),
        HelpTopic::Map => map_usage(),
    }
    .into()
}

fn global_usage() -> &'static str {
    r#"Usage:
  cove examples [--json]
  cove showcase customer360 --out <dir> [--profile quick|standard|publication] [--force] [--json]
  cove showcase proof-suite --out <dir> [--scenario customer360|claims|catalog|all] [--profile quick|standard|publication] [--force] [--json]
  cove doctor [--json] <file>
  cove inspect [--queries] [--performance] [--json] <file>
  cove inspect [--json] [--sections stats,dictionary,execution,indexes,optional] <file...>
  cove optimize <file> [--out-dir dir] [--full] [--json]
  cove query [--format table|json|jsonl|csv] [--take n] [--max-cell-width n] [--explain [public|developer|proof|coded|forensic]] [--engine auto|materialized|physical|compare|kernel] [--no-auto-sidecars] [--strict-performance] [--perf-report] [--batch-size n] [--external-table name=path.csv|json|jsonl] [--enable-graph-traversal] [--max-graph-depth n] [--max-graph-paths n] [--max-graph-fanout n] [--mapping file.covemap] [--member id=path] [--dataset dir] [--covi file] [--covx file] [--cove-e file] [file] '<coveql>'
  cove query [options] --query-file <path|-> [file]
  cove convert <parquet|arrow|orc|csv|report> ...
  cove validate ...
  cove dump ...
  cove map <validate|preview|plan-keys|candidates|review|aliases|replay|convert|build|publish|doctor|suggest|parity|explain|diff|project|test> ...
  cove export arrow ...
  cove perf <explain-pruning|plan-cost> ...
  cove sidecar inspect <index|coverage|layout|cache|runtime> <file>
  cove sidecar build <covi|covx|covm> ...
  cove delta <inspect|validate|dump|chain|publish|publish-atomic|reconstruct|compact|checkpoint> ...
  cove digest verify <file.cove> [--require]
  cove profile <inspect|generate|validate-section> ...
  cove canonicalise <validate-payload|encode-json|check-domain|check-trust> ...

Examples:
  cove examples
  cove showcase customer360 --profile quick --out examples/customer360 --force
  cove showcase proof-suite --scenario all --profile quick --out target/cove-proof-suite --force
  cove doctor people.cove
  cove inspect --queries --performance people.cove
  cove convert parquet source.parquet output.cove
  cove validate --semantic output.cove
  cove optimize output.cove
  cove query output.cove 'table(source).take(10)'
  cove query --format jsonl people.cove 'table(people).where(active == true)'
  cove map preview mapping.covemap
  cove map build --verify --publish-covm --out-dir bundle mapping.covemap source.csv
  cove sidecar build covi output.cove output.covi --all-columns
  cove sidecar build covi people.cove people.covi --object-properties
  cove delta inspect delta-0001.covedelta
  cove query --query-file query.coveql people.cove"#
}

fn query_usage() -> &'static str {
    "Usage:\n  cove query [options] [file.cove|manifest.covm] '<coveql>'\n  cove query [options] --query-file <path|-> [file.cove|manifest.covm]\n\nOutput options:\n  --format table|json|jsonl|csv\n  --take n\n  --max-cell-width n\n  --json-diagnostics\n\nExecution options:\n  --engine auto|materialized|physical|compare|kernel\n  --explain [public|developer|proof|coded|forensic]\n  --perf-report\n  --strict-performance\n  --no-auto-sidecars\n  --batch-size n\n\nInputs and sidecars:\n  --mapping file.covemap\n  --external-table name=path.csv|json|jsonl\n  --member manifest-uri=path\n  --dataset dir\n  --covi file.covi\n  --covx file.covx\n  --coverage-plan file\n  --coverage-proof file\n  --coverage-set file\n  --layout-plan file\n  --scan-split-index file\n  --page-cluster-directory file\n  --zero-copy-buffer-map file\n  --coverage-cache file\n  --cove-e file\n\nGraph traversal:\n  --enable-graph-traversal\n  --max-graph-depth n\n  --max-graph-paths n\n  --max-graph-fanout n\n\nAuthority model:\n  Materialized CoveQL readback is the semantic authority. Auto, physical, kernel,\n  and sidecar-backed execution may accelerate a query only when validated metadata\n  proves equivalence; otherwise the CLI falls back or rejects when strict mode is set.\n\nExamples:\n  cove query events.cove 'table(events).where(score >= 20).select(id, score)'\n  cove query --format jsonl people.cove 'table(people).select(score, status).take(5)'\n  cove query --engine compare --perf-report events.cove 'table(events).where(score >= 20).select(id, score)'\n  cove query --external-table weights=weights.jsonl events.cove 'table(events) as e.join(table(weights) as w, on: e.id == w.id).select(id: e.id, score: e.score, weight: w.weight)'\n  printf 'table(events).take(5)' | cove query --query-file - events.cove"
}

fn inspect_usage() -> &'static str {
    "Usage:\n  cove inspect [--queries] [--performance] [--json] <file>\n  cove inspect [--json] [--sections stats,dictionary,execution,indexes,optional] <file...>\n\nModes:\n  Beginner inspect detects query surfaces, artifact type, guidance, diagnostics,\n  and optional performance-sidecar status.\n  Detailed inspect delegates to the lower-level inspector when --sections is used\n  or when multiple files are supplied.\n\nExamples:\n  cove inspect --queries people.cove\n  cove inspect --queries --performance events.cove\n  cove inspect --json --sections stats,dictionary events.cove"
}

fn optimize_usage() -> &'static str {
    "Usage:\n  cove optimize <file.cove> [--out-dir dir] [--full] [--json]\n\nBehavior:\n  Writes a sibling .covperf.json discovery manifest plus applicable acceleration\n  sidecars such as COVE-I, COVX, COVE-E, and COVE-L artifacts. Source files are\n  not rewritten. Generated sidecars are acceleration metadata, not portable\n  logical truth; query results remain governed by materialized readback unless\n  validated sidecars prove an optimized path equivalent.\n\nExamples:\n  cove optimize examples/coveql/events.cove\n  cove inspect --performance examples/coveql/events.cove\n  cove query --engine compare --perf-report examples/coveql/events.cove 'table(events).where(score >= 20).select(id, score)'"
}

fn showcase_usage() -> &'static str {
    "Usage:\n  cove showcase customer360 --out <dir> [--profile quick|standard|publication] [--force] [--json]\n  cove showcase proof-suite --out <dir> [--scenario customer360|claims|catalog|all] [--profile quick|standard|publication] [--force] [--json]\n\nBehavior:\n  Generates deterministic showcase data. Customer 360 remains the approachable\n  data-science demo and now includes a true messy-source map-build proof bundle.\n  The proof suite generates Customer 360, claims/events, and catalog/vendor\n  scenarios with source tables, COVE-MAP files, verified COVE-O bundles,\n  COVE-T projections, COVE-I sidecars, COVM manifests, parity reports, and\n  Parquet comparison baselines.\n\nProfiles:\n  quick        Tiny checked-in/demo-sized data.\n  standard     Larger local benchmark data written under target/.\n  publication  Largest deterministic public-report profile.\n\nExamples:\n  cove showcase customer360 --profile quick --out examples/customer360 --force\n  cove showcase proof-suite --scenario all --profile quick --out target/cove-proof-suite --force\n  cove showcase customer360 --profile standard --out target/customer360-standard --force\n  cove inspect --queries --performance target/customer360-standard/customers.cove"
}

fn sidecar_usage() -> &'static str {
    "Usage:\n  cove sidecar inspect <index|coverage|layout|cache|runtime> <file>\n  cove sidecar build covi <input.cove> <output.covi> [--table-id id] [--column-id id ... | --all-columns | --object-properties]\n  cove sidecar build covx <output.covx> <input.cove>...\n  cove sidecar build covm <output.covm> <input.cove>...\n\nExamples:\n  cove sidecar inspect index events.covi\n  cove sidecar build covi events.cove events.covi --all-columns --index-only-counts\n  cove sidecar build covi people.cove people-object-properties.covi --object-properties\n  cove sidecar build covm dataset.covm shard-1.cove shard-2.cove"
}

fn delta_usage() -> &'static str {
    crate::delta::usage()
}

fn convert_usage() -> &'static str {
    "Usage:\n  cove convert parquet <source.parquet> <output.cove> [options]\n  cove convert arrow <source.arrow> <output.cove> [options]\n  cove convert orc <source.orc> <output.cove> [options]\n  cove convert csv <source.csv> <output.cove> [options]\n  cove convert report ...\n\nExamples:\n  cove convert parquet source.parquet output.cove --report report.json\n  cove convert csv source.csv output.cove --report -\n  cove convert report --direction cove-to-source --target-format csv --output output.csv input.cove"
}

fn map_usage() -> &'static str {
    r#"Usage:
  cove map validate <mapping.covemap>
  cove map preview <mapping.covemap>
  cove map plan-keys <mapping.covemap> <source...>
  cove map candidates [--out candidates.json] <mapping.covemap> <source...>
  cove map review [--out reviewed.json] <candidate-matches.json>
  cove map review export <mapping.covemap> [--out reviewed.json]
  cove map review import <mapping.covemap> <reviewed.json> --out <mapping.covemap> [--replace]
  cove map aliases import <mapping.covemap> <aliases.csv> --catalog-id <id> --resolver-id <id> --out <mapping.covemap>
  cove map replay verify <mapping.covemap> <conversion-report.json>
  cove map convert [--format json|cove-o] [-o output] <mapping.covemap> <source...>
  cove map build --out-dir <dir> [--verify] [--publish-covm] [--force] [--json] [--object-name name.cove] [--projection-output cove-t|none] [--evidence-encoding compact|expanded|both] [--section-compression zstd|none] <mapping.covemap> <source...>
  cove map publish --bundle-dir <dir> --out <dataset.covm> [--force] [--json]
  cove map doctor [--json] [--strict] --bundle-dir <dir>
  cove map doctor [--json] [--strict] <mapping.covemap> <source...>
  cove map suggest [--json] [--out suggestions.json] <source...>
  cove map parity [--json] --projection-id <id> --expected <table> [--expected-query <coveql>] [--key col[,col...]] <mapping.covemap> <source...>
  cove map parity-cove-o [--json] --projection-id <id> --expected <table> [--expected-query <coveql>] [--key col[,col...]] <object.cove>
  cove map project [-o output] [--format json|arrow|cove-t|sql] <mapping.covemap> <source...>
  cove map project-cove-o [--mapping mapping.covemap] [-o output] <object.cove>
  cove map explain <mapping.covemap> <goid|assertion-id>
  cove map diff <left.covemap> <right.covemap>
  cove map test <fixture.json>

Behavior:
  cove map build emits COVE-O, COVE-T projections when enabled, reports, a bundle manifest, and COVE-I acceleration roots and optional normative COVM publication.

Examples:
  cove map validate people.covemap
  cove map preview people.covemap
  cove map convert --format cove-o -o people.cove people.covemap people.jsonl
  cove map build --verify --publish-covm --out-dir people-bundle people.covemap people.jsonl
  cove map publish --bundle-dir people-bundle --out people.covm --force
  cove map candidates --out candidates.json people.covemap people.jsonl
  cove map review --out reviewed.json candidates.json
  cove map review export people-reviewed.covemap --out reviewed-export.json
  cove map review import people.covemap reviewed.json --out people-reviewed.covemap
  cove map aliases import people.covemap aliases.csv --catalog-id company_aliases --resolver-id company_name_resolver --out people-with-aliases.covemap
  cove map replay verify people-reviewed.covemap conversion-report.json
  cove map doctor --bundle-dir people-bundle
  cove map suggest people.csv people.jsonl
  cove map parity --projection-id people.v1 --expected expected.csv --key id people.covemap people.jsonl
  cove map project --format json people.covemap people.jsonl"#
}
