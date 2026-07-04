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
    Vec,
    Ai,
    Train,
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
        HelpTopic::Vec => vec_usage(),
        HelpTopic::Ai => ai_usage(),
        HelpTopic::Train => train_usage(),
    }
    .into()
}

fn global_usage() -> &'static str {
    r#"Usage:
  cove examples [--json]
  cove showcase customer360 --out <dir> [--profile quick|standard|publication] [--force] [--json]
  cove showcase proof-suite --out <dir> [--scenario customer360|claims|catalog|all] [--profile quick|standard|publication] [--force] [--json]
  cove showcase ai-training --out <dir> [--profile quick|standard|publication] [--force] [--json]
  cove doctor [--json] [--query-discovery] [--policy public|developer] [--audience name] <file>
  cove inspect [--queries] [--performance] [--ai] [--query-discovery|--agent] [--policy public|developer] [--audience name] [--json] <file>
  cove inspect [--json] [--sections stats,dictionary,execution,indexes,optional] <file...>
  cove optimize <file> [--out-dir dir] [--full] [--json]
  cove query [--format table|json|jsonl|csv] [--take n] [--max-cell-width n] [--explain [public|developer|proof|coded|ai|forensic]] [--engine auto|materialized|physical|compare|kernel] [--no-auto-sidecars] [--strict-performance] [--perf-report] [--batch-size n] [--external-table name=path.csv|json|jsonl] [--enable-graph-traversal] [--max-graph-depth n] [--max-graph-paths n] [--max-graph-fanout n] [--mapping file.covemap] [--member id=path] [--dataset dir] [--as-of-csn n|--as-of-commit-us n] [--delta-plan|--delta-plan-json] [--from-template id --param name=value...] [--covi file] [--covx file] [--cove-e file] [--cove-ai file.covev] [file] '<coveql>'
  cove query [options] --query-file <path|-> [file]
  cove convert <parquet|arrow|orc|csv|report> ...
  cove validate ...
  cove vec build --out <vectors.covev> --dimension <n> --file-code <u32>... (--deterministic | --payload <f32le.bin>) [--index exact|hnsw|ivf-flat|ivf-pq|diskann|vamana]
  cove ai import jsonl <input.jsonl> --out <training.coveai> --schema instruction|chat|pretrain|preference|rag [--dry-run] [--publish-covm]
  cove ai verify <training.coveai|manifest.covm> [--policy-report] [--json]
  cove ai stream <training.coveai|manifest.covm> --format jsonl|hf-jsonl|webdataset [--split train|validation|test] [--include-payloads]
  cove ai diff <old.coveai> <new.coveai> [--keys sample_id] [--report diff.json]
  cove ai export <chunks|tokens|vectors|training|multimodal|assets|tensors> <sidecar> [--include-payloads] [--format json|jsonl|hf-jsonl|arrow|parquet|webdataset]
  cove train export <training.coveai|training.covev> [--include-payloads] [--format json|jsonl|hf-jsonl|arrow|parquet|webdataset] [--out <path>] [--profile <id>] [--split <id>] [--epoch-plan <id>]
  cove dump ...
  cove map <validate|preview|plan-keys|candidates|review|aliases|replay|convert|build|delta|publish|doctor|suggest|parity|explain|diff|project|test> ...
  cove export arrow [--query '<coveql>'] ...
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
  cove showcase ai-training --profile quick --out target/cove-ai-training --force
  cove doctor people.cove
  cove inspect --queries --performance people.cove
  cove inspect --query-discovery --json people.cove
  cove inspect --ai vectors.covev
  cove vec build --out vectors.covev --dimension 3 --file-code 1 --file-code 2 --deterministic
  cove ai import jsonl samples.jsonl --out training.coveai --schema instruction --publish-covm
  cove ai verify training.coveai --policy-report
  cove ai export tokens vectors.covev --include-payloads --format jsonl
  cove train export training.coveai --format jsonl
  cove convert parquet source.parquet output.cove
  cove validate --semantic output.cove
  cove optimize output.cove
  cove query output.cove 'table(source).take(10)'
  cove query --format jsonl people.cove 'table(people).where(active == true)'
  cove query --from-template table_filter_select_take --param score=20 --param columns=id,score events.cove
  cove map preview mapping.covemap
  cove map build --verify --publish-covm --out-dir bundle mapping.covemap source.csv
  cove sidecar build covi output.cove output.covi --all-columns
  cove sidecar build covi people.cove people.covi --object-properties
  cove sidecar build covi --snapshot dataset.covm --dataset bundle --out snapshot.covi --object-properties
  cove delta inspect delta-0001.covedelta
  cove query --query-file query.coveql people.cove"#
}

fn query_usage() -> &'static str {
    "Usage:\n  cove query [options] [file.cove|manifest.covm] '<coveql>'\n  cove query [options] --query-file <path|-> [file.cove|manifest.covm]\n  cove query [options] --from-template <id> --param name=value... <file.cove>\n\nOutput options:\n  --format table|json|jsonl|csv\n  --take n\n  --max-cell-width n\n  --json-diagnostics\n\nExecution options:\n  --engine auto|materialized|physical|compare|kernel\n  --explain [public|developer|proof|coded|ai|forensic]\n  --perf-report\n  --strict-performance\n  --no-auto-sidecars\n  --batch-size n\n  --from-template id\n  --param name=value\n\nDelta snapshot options:\n  --dataset dir\n  --as-of-csn n\n  --as-of-commit-us n\n  --delta-plan\n  --delta-plan-json\n\nInputs and sidecars:\n  --mapping file.covemap\n  --external-table name=path.csv|json|jsonl\n  --member manifest-uri=path\n  --covi file.covi\n  --covx file.covx\n  --coverage-plan file\n  --coverage-proof file\n  --coverage-set file\n  --layout-plan file\n  --scan-split-index file\n  --page-cluster-directory file\n  --zero-copy-buffer-map file\n  --coverage-cache file\n  --cove-e file\n  --cove-ai file.covev\n\nGraph traversal:\n  --enable-graph-traversal\n  --max-graph-depth n\n  --max-graph-paths n\n  --max-graph-fanout n\n\nAuthority model:\n  Materialized CoveQL readback is the semantic authority. Auto, physical, kernel,\n  and sidecar-backed execution may accelerate a query only when validated metadata\n  proves equivalence; otherwise the CLI falls back or rejects when strict mode is set.\n  Query-discovery templates are rendered to CoveQL from manifest operator chains,\n  then parsed, resolved, planned, and executed through the same path as direct\n  CoveQL. With auto sidecars enabled, sibling .coveai/.covev files are discovered\n  for selected COVE-AI operations.\n\nExamples:\n  cove query events.cove 'table(events).where(score >= 20).select(id, score)'\n  cove query --format jsonl people.cove 'table(people).select(score, status).take(5)'\n  cove query --from-template table_filter_select_take --param score=20 --param columns=id,score events.cove\n  cove query dataset.covm --dataset bundle --as-of-csn 100 --delta-plan 'object(Thing).take(10)'\n  cove query --engine compare --perf-report events.cove 'table(events).where(score >= 20).select(id, score)'\n  cove query --external-table weights=weights.jsonl events.cove 'table(events) as e.join(table(weights) as w, on: e.id == w.id).select(id: e.id, score: e.score, weight: w.weight)'\n  cove query --cove-ai vectors.covev events.cove '# profiles: table, ai\\ntable(events).similar(fileCode: 10, k: 5)'\n  printf 'table(events).take(5)' | cove query --query-file - events.cove"
}

fn inspect_usage() -> &'static str {
    "Usage:\n  cove inspect [--queries] [--performance] [--ai] [--query-discovery|--agent] [--policy public|developer] [--audience name] [--json] <file>\n  cove inspect [--json] [--sections stats,dictionary,execution,indexes,optional] <file...>\n\nModes:\n  Beginner inspect detects query surfaces, artifact type, guidance, diagnostics,\n  and optional performance-sidecar status.\n  Query discovery emits canonical COVE-QD JSON for CoveQL tooling; --agent is an alias for --query-discovery --json. Query-discovery policy binding also accepts --principal-class and --policy-fingerprint.\n  AI inspect validates .coveai/.covev sidecars or reports embedded AI sections in .cove files.\n  Detailed inspect delegates to the lower-level inspector when --sections is used\n  or when multiple files are supplied.\n\nExamples:\n  cove inspect --queries people.cove\n  cove inspect --query-discovery --json --policy public --audience public-demo people.cove\n  cove inspect --agent people.cove\n  cove inspect --queries --performance events.cove\n  cove inspect --ai vectors.covev\n  cove inspect --json --sections stats,dictionary events.cove"
}

fn optimize_usage() -> &'static str {
    "Usage:\n  cove optimize <file.cove> [--out-dir dir] [--full] [--json]\n\nBehavior:\n  Writes a sibling .covperf.json discovery manifest plus applicable acceleration\n  sidecars such as COVE-I, COVX, COVE-E, and COVE-L artifacts. Source files are\n  not rewritten. Generated sidecars are acceleration metadata, not portable\n  logical truth; query results remain governed by materialized readback unless\n  validated sidecars prove an optimized path equivalent.\n\nExamples:\n  cove optimize examples/coveql/events.cove\n  cove inspect --performance examples/coveql/events.cove\n  cove query --engine compare --perf-report examples/coveql/events.cove 'table(events).where(score >= 20).select(id, score)'"
}

fn showcase_usage() -> &'static str {
    "Usage:\n  cove showcase customer360 --out <dir> [--profile quick|standard|publication] [--force] [--json]\n  cove showcase proof-suite --out <dir> [--scenario customer360|claims|catalog|all] [--profile quick|standard|publication] [--force] [--json]\n  cove showcase ai-training --out <dir> [--profile quick|standard|publication] [--force] [--json]\n\nBehavior:\n  Generates deterministic showcase data. Customer 360 remains the approachable\n  data-science demo and now includes a true messy-source map-build proof bundle.\n  The proof suite generates Customer 360, claims/events, and catalog/vendor\n  scenarios with source tables, COVE-MAP files, verified COVE-O bundles,\n  COVE-T projections, COVE-I sidecars, COVM manifests, parity reports, and\n  Parquet comparison baselines. The AI training showcase generates a governed\n  COVE-AI archive, COVM manifest, verification report, and HF/Parquet/WebDataset\n  exports for trainer integration examples.\n\nProfiles:\n  quick        Tiny checked-in/demo-sized data.\n  standard     Larger local benchmark data written under target/.\n  publication  Largest deterministic public-report profile.\n\nExamples:\n  cove showcase customer360 --profile quick --out examples/customer360 --force\n  cove showcase proof-suite --scenario all --profile quick --out target/cove-proof-suite --force\n  cove showcase ai-training --profile quick --out target/cove-ai-training --force\n  cove showcase customer360 --profile standard --out target/customer360-standard --force\n  cove inspect --queries --performance target/customer360-standard/customers.cove"
}

fn sidecar_usage() -> &'static str {
    "Usage:\n  cove sidecar inspect <index|coverage|layout|cache|runtime> <file>\n  cove sidecar build covi <input.cove> <output.covi> [--table-id id] [--column-id id ... | --all-columns | --object-properties]\n  cove sidecar build covi --snapshot <manifest.covm> --dataset <dir> --out <output.covi> [--as-of-csn n|--as-of-commit-us n] [covi options]\n  cove sidecar build covx <output.covx> <input.cove>...\n  cove sidecar build covx --snapshot <manifest.covm> --dataset <dir> --out <output.covx> [--as-of-csn n|--as-of-commit-us n]\n  cove sidecar build covm <output.covm> <input.cove>...\n  cove sidecar build covm --snapshot <manifest.covm> --dataset <dir> --out <output.covm> [--as-of-csn n|--as-of-commit-us n]\n\nExamples:\n  cove sidecar inspect index events.covi\n  cove sidecar build covi events.cove events.covi --all-columns --index-only-counts\n  cove sidecar build covi people.cove people-object-properties.covi --object-properties\n  cove sidecar build covi --snapshot dataset.covm --dataset bundle --out snapshot.covi --object-properties\n  cove sidecar build covm dataset.covm shard-1.cove shard-2.cove"
}

fn delta_usage() -> &'static str {
    crate::delta::usage()
}

fn convert_usage() -> &'static str {
    "Usage:\n  cove convert parquet <source.parquet> <output.cove> [options]\n  cove convert arrow <source.arrow> <output.cove> [options]\n  cove convert orc <source.orc> <output.cove> [options]\n  cove convert csv <source.csv> <output.cove> [options]\n  cove convert report ...\n\nExamples:\n  cove convert parquet source.parquet output.cove --report report.json\n  cove convert csv source.csv output.cove --report -\n  cove convert report --direction cove-to-source --target-format csv --output output.csv input.cove"
}

fn vec_usage() -> &'static str {
    "Usage:\n  cove vec build --out <vectors.covev> --dimension <n> --file-code <u32>... (--deterministic | --payload <f32le-source.bin>) [--index exact|hnsw|ivf-flat|ivf-pq|diskann|vamana] [--metric cosine|dot|l2|l1] [--quantization none|int8|uint8|pq] [--seed <n>] [--ef <n>] [--ef-search <n>] [--ef-construction <n>] [--probes <n>] [--lists <n>] [--shard-count <n>] [--integrity-report <path>] [--artifact-id <32-hex>] [--created-at-us <n>]\n\nBehavior:\n  Builds a CVV2 sidecar from deterministic or supplied f32 little-endian source vectors.\n  The stored vector payload is dense Float32 for --quantization none, or local Int8,\n  UInt8, or PQ-code bytes for quantized builds. The sidecar includes FileCode bindings,\n  payload refs, payload integrity, privacy summary, vector-space, vector-block,\n  vector-directory, and optional vector-index records. Build/index parameters are\n  recorded in the optional integrity report. Core does not call network embedding\n  providers.\n\nExamples:\n  cove vec build --out vectors.covev --dimension 3 --file-code 1 --file-code 2 --deterministic\n  cove vec build --out vectors.covev --dimension 384 --file-code 10 --payload embeddings.f32le --index hnsw --metric dot"
}

fn ai_usage() -> &'static str {
    "Usage:\n  cove ai import jsonl <input.jsonl> --out <training.coveai> --schema instruction|chat|pretrain|preference|rag [--split-policy deterministic] [--split-column name] [--mapping mapping.json] [--dry-run] [--publish-covm]\n  cove ai import parquet <input.parquet> --out <training.coveai> --schema instruction|chat|pretrain|preference|rag [import options]\n  cove ai import hf <local-dataset-dir> --out <training.coveai> --schema instruction|chat|pretrain|preference|rag [import options]\n  cove ai verify <sidecar|manifest.covm> [--dataset dir] [--policy-report] [--strict-training] [--json]\n  cove ai stream <sidecar|manifest.covm> --format jsonl|hf-jsonl|arrow|parquet|webdataset [--split train|validation|test] [--include-payloads] [--out <path>]\n  cove ai diff <old.coveai> <new.coveai> [--keys sample_id] [--report diff.json]\n  cove ai export <chunks|tokens|vectors|training|multimodal|assets|tensors> <sidecar> [--include-payloads] [--format json|jsonl|hf-jsonl|arrow|parquet|webdataset] [--out <path>] [--policy-report]\n\nBehavior:\n  Import workflows turn JSONL, local Hugging Face JSONL directories, and Parquet\n  datasets into self-contained COVE-AI training archives with deterministic\n  splits, policy diagnostics, optional mapping-driven COVE-TRAIN metadata, and\n  optional digest-bound COVM publication. Strict verification rejects advisory\n  training archives that lack replayable split/epoch metadata or provenance.\n  Verify and stream validate archives before exposing payloads through policy-gated\n  AI payload leases. Existing descriptor export remains available for chunks,\n  tokens, vectors, training, multimodal, assets, and tensors.\n\nExamples:\n  cove ai import jsonl samples.jsonl --out training.coveai --schema instruction --publish-covm\n  cove ai import jsonl samples.jsonl --schema instruction --dry-run\n  cove ai verify training.coveai --policy-report --json\n  cove ai stream training.coveai --format hf-jsonl --split train --include-payloads\n  cove ai diff old.coveai new.coveai --keys sample_id --report diff.json\n  cove ai export vectors vectors.covev --format parquet --out vectors.parquet"
}

fn train_usage() -> &'static str {
    "Usage:\n  cove train export <training.coveai|training.covev> [--include-payloads] [--format json|jsonl|hf-jsonl|arrow|parquet|webdataset] [--out <path>] [--profile <id>] [--split <id>] [--epoch-plan <id>] [--policy-report] [--strict-training]\n\nBehavior:\n  Validates the COVE-AI/COVE-VEC sidecar and exports COVE-TRAIN records. Payload access remains policy-gated and withheld samples/payloads are diagnostic rows rather than silent skips. Arrow and Parquet write native table artifacts; WebDataset writes a tar shard with metadata and JSON sample members. With --strict-training, advisory archives are rejected before export.\n\nExamples:\n  cove train export training.coveai\n  cove train export training.coveai --include-payloads --format jsonl --out samples.jsonl\n  cove train export training.coveai --format arrow --out samples.arrow\n  cove train export training.coveai --format webdataset --out samples.tar\n  cove train export training.coveai --profile 1 --split 2 --epoch-plan 7"
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
  cove map delta build <manifest.covm> --dataset <dir> --out-dir <dir> [--as-of-csn n|--as-of-commit-us n] [--force] [--json] [--publish-covm] [--verify] [--projection-output cove-t|none] [--object-name name.cove]
  cove map delta build --base <manifest.covm> --dataset <dir> --mapping <mapping.covemap> --out <delta.covedelta> [--source-publish-range start:end] [--force] [--json] <source...>
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
  cove map delta build dataset.covm --dataset bundle --out-dir delta-bundle --projection-output none
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
