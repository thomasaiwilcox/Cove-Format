#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelpTopic {
    Global,
    Query,
    Inspect,
    Optimize,
    Sidecar,
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
        HelpTopic::Sidecar => sidecar_usage(),
        HelpTopic::Convert => convert_usage(),
        HelpTopic::Map => map_usage(),
    }
    .into()
}

fn global_usage() -> &'static str {
    "Usage:\n  cove examples [--json]\n  cove doctor [--json] <file>\n  cove inspect [--queries] [--performance] [--json] <file>\n  cove inspect [--json] [--sections stats,dictionary,execution,indexes,optional] <file...>\n  cove optimize <file> [--out-dir dir] [--full] [--json]\n  cove query [--format table|json|jsonl|csv] [--take n] [--max-cell-width n] [--explain [public|developer|proof|coded|forensic]] [--engine auto|materialized|physical|compare|kernel] [--no-auto-sidecars] [--strict-performance] [--perf-report] [--batch-size n] [--external-table name=path.csv|json|jsonl] [--enable-graph-traversal] [--max-graph-depth n] [--max-graph-paths n] [--max-graph-fanout n] [--mapping file.covemap] [--member id=path] [--dataset dir] [--covi file] [--covx file] [--cove-e file] [file] '<coveql>'\n  cove query [options] --query-file <path|-> [file]\n  cove convert <parquet|arrow|orc|csv|report> ...\n  cove validate ...\n  cove dump ...\n  cove map <validate|preview|plan-keys|convert|explain|diff|project|test> ...\n  cove export arrow ...\n  cove perf <explain-pruning|plan-cost> ...\n  cove sidecar inspect <index|coverage|layout|cache|runtime> <file>\n  cove sidecar build <covi|covx|covm> ...\n  cove digest verify <file.cove> [--require]\n  cove profile <inspect|generate|validate-section> ...\n  cove canonicalise <validate-payload|encode-json|check-domain|check-trust> ...\n\nExamples:\n  cove examples\n  cove doctor people.cove\n  cove inspect --queries --performance people.cove\n  cove convert parquet source.parquet output.cove\n  cove validate --semantic output.cove\n  cove optimize output.cove\n  cove query output.cove 'table(source).take(10)'\n  cove query --format jsonl people.cove 'table(people).where(active == true)'\n  cove map preview mapping.covemap\n  cove sidecar build covi output.cove output.covi --all-columns\n  cove query --query-file query.coveql people.cove"
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

fn sidecar_usage() -> &'static str {
    "Usage:\n  cove sidecar inspect <index|coverage|layout|cache|runtime> <file>\n  cove sidecar build covi <input.cove> <output.covi> [--table-id id] [--column-id id ... | --all-columns]\n  cove sidecar build covx <output.covx> <input.cove>...\n  cove sidecar build covm <output.covm> <input.cove>...\n\nExamples:\n  cove sidecar inspect index events.covi\n  cove sidecar build covi events.cove events.covi --all-columns --index-only-counts\n  cove sidecar build covm dataset.covm shard-1.cove shard-2.cove"
}

fn convert_usage() -> &'static str {
    "Usage:\n  cove convert parquet <source.parquet> <output.cove> [options]\n  cove convert arrow <source.arrow> <output.cove> [options]\n  cove convert orc <source.orc> <output.cove> [options]\n  cove convert csv <source.csv> <output.cove> [options]\n  cove convert report ...\n\nExamples:\n  cove convert parquet source.parquet output.cove --report report.json\n  cove convert csv source.csv output.cove --report -\n  cove convert report --direction cove-to-source --target-format csv --output output.csv input.cove"
}

fn map_usage() -> &'static str {
    "Usage:\n  cove map validate <mapping.covemap>\n  cove map preview <mapping.covemap>\n  cove map plan-keys <mapping.covemap> <source...>\n  cove map convert [--format json|cove-o] [-o output] <mapping.covemap> <source...>\n  cove map project [-o output] [--format json|arrow|cove-t|sql] <mapping.covemap> <source...>\n  cove map project-cove-o [--mapping mapping.covemap] [-o output] <object.cove>\n  cove map explain <mapping.covemap> <goid|assertion-id>\n  cove map diff <left.covemap> <right.covemap>\n  cove map test <fixture.json>\n\nExamples:\n  cove map validate people.covemap\n  cove map preview people.covemap\n  cove map convert --format cove-o -o people.cove people.covemap people.jsonl\n  cove map project --format json people.covemap people.jsonl"
}
