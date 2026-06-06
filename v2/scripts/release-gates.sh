#!/usr/bin/env sh
set -eu

usage() {
    cat <<'EOF'
Usage: scripts/release-gates.sh [--quick|--full|--publication|--help]

Modes:
  --quick        Short developer gate for CLI/reference-tooling changes.
  --full         Full release gate. This is also the default when no mode is set.
  --publication  Full gate plus deterministic public benchmark report generation.
  --help         Print this help without running any gate.
EOF
}

quick_gate() {
    cargo fmt --check
    sh scripts/check-v2-boundaries.sh
    cargo test -p cove-cli
    cargo run -p cove-conformance --bin gen-corpus -- --check
    cargo run -p cove-conformance --bin gen-capability-matrix -- --check
    cargo run -p cove-conformance --bin cove-conformance -- \
        conformance/ --manifest conformance/minimal-reader-manifest.jsonl
}

full_gate() {
    cargo fmt --check
    sh scripts/check-v2-boundaries.sh
    cargo test --workspace
    cargo run -p cove-codec --bin cove-codec-validate -- conformance/codecs/codec_descriptor_valid.bin > /dev/null
    cargo run -p cove-codec --bin cove-codec-validate -- conformance/codecs/registered_encoding_envelope_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect layout conformance/layout/layout_plan_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect layout conformance/layout/scan_split_index_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect layout conformance/zerocopy/zero_copy_map_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect runtime conformance/runtime/runtime_hint_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect coverage conformance/coverage/coverage_set_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect coverage conformance/coverage/provider_registry_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect coverage conformance/coverage/coverage_proof_record_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect coverage conformance/coverage/predicate_normal_form_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect coverage conformance/coverage/interval_predicate_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect coverage conformance/coverage/coverage_plan_candidate_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect index conformance/covi/empty_valid.covi > /dev/null
    cargo run -p cove-cli -- sidecar inspect index conformance/covi/single_section_valid.covi > /dev/null
    cargo run -p cove-cli -- sidecar inspect index conformance/covi/index_capability_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect index conformance/covi/index_only_capability_valid.bin > /dev/null
    cargo run -p cove-cli -- sidecar inspect cache conformance/cache/cache_valid.bin > /dev/null
    cargo run -p cove-cli -- profile inspect conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- profile validate-section conformance/accept/cove_e_execution_code_valid.bin --kind execution-code > /dev/null
    cargo run -p cove-cli -- profile validate-section conformance/accept/cove_e_execution_scope_valid.bin --kind execution-scope > /dev/null
    cargo run -p cove-cli -- profile validate-section conformance/accept/cove_e_code_space_valid.bin --kind code-space > /dev/null
    cargo run -p cove-cli -- profile validate-section conformance/accept/cove_e_mount_policy_valid.bin --kind mount-policy > /dev/null
    cargo run -p cove-cli -- profile generate --kind execution-code --out /tmp/cove-release-gate-execution-code.bin > /dev/null
    cargo run -p cove-cli -- profile validate-section /tmp/cove-release-gate-execution-code.bin --kind execution-code > /dev/null
    cargo run -p cove-cli -- profile generate --kind execution-scope --out /tmp/cove-release-gate-execution-scope.bin > /dev/null
    cargo run -p cove-cli -- profile validate-section /tmp/cove-release-gate-execution-scope.bin --kind execution-scope > /dev/null
    cargo run -p cove-cli -- profile generate --kind code-space --out /tmp/cove-release-gate-code-space.bin > /dev/null
    cargo run -p cove-cli -- profile validate-section /tmp/cove-release-gate-code-space.bin --kind code-space > /dev/null
    cargo run -p cove-cli -- profile generate --kind mount-policy --out /tmp/cove-release-gate-mount-policy.bin > /dev/null
    cargo run -p cove-cli -- profile validate-section /tmp/cove-release-gate-mount-policy.bin --kind mount-policy > /dev/null
    cargo run -p cove-cli -- profile generate --kind engine-registry --out /tmp/cove-release-gate-engine-registry.bin > /dev/null
    cargo run -p cove-cli -- profile validate-section /tmp/cove-release-gate-engine-registry.bin --kind engine-registry > /dev/null
    cargo run -p cove-cli -- canonicalise validate-payload --tag int64 --hex 2a00000000000000 > /dev/null
    cargo run -p cove-cli -- canonicalise encode-json --logical utf8 --value '"red"' > /dev/null
    cargo run -p cove-cli -- canonicalise check-domain conformance/accept/cove_t_zone_stats_valid.cove > /dev/null
    cargo run -p cove-cli -- canonicalise check-trust conformance/accept/cove_o_trust_manifest_valid.cove > /dev/null
    cargo run -p cove-cli -- digest verify conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- sidecar build covm /tmp/cove-release-gate.covm conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- sidecar build covx /tmp/cove-release-gate.covx conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- perf explain-pruning conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- perf plan-cost --execute conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- convert parquet conformance/accept/parquet_primitives_valid.parquet /tmp/cove-release-gate.cove --report /tmp/cove-release-gate-convert.json > /dev/null
    cargo run -p cove-cli -- convert report conformance/accept/parquet_primitives_valid.parquet > /dev/null
    cargo run -p cove-cli -- export arrow conformance/accept/cove_t_scan_table.cove /tmp/cove-release-gate.arrow --report /tmp/cove-release-gate-arrow-export.json > /dev/null
    cargo run -p cove-cli -- convert arrow /tmp/cove-release-gate.arrow /tmp/cove-release-gate-arrow.cove > /dev/null
    printf 'id,name\n1,Ada\n2,Linus\n' > /tmp/cove-release-gate.csv
    cargo run -p cove-cli -- convert csv /tmp/cove-release-gate.csv /tmp/cove-release-gate-csv.cove --report /tmp/cove-release-gate-csv-report.json > /dev/null
    cargo run -p cove-cli -- convert report --direction cove-to-source --target-format arrow --output /tmp/cove-release-gate-reverse.arrow conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- convert report --direction cove-to-source --target-format csv --output /tmp/cove-release-gate-reverse.csv conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- convert report --direction cove-to-source --target-format parquet --output /tmp/cove-release-gate-reverse.parquet conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- convert report --direction cove-to-source --target-format orc --output /tmp/cove-release-gate-reverse.orc conformance/accept/cove_t_scan_table.cove > /dev/null
    cargo run -p cove-cli -- convert orc /tmp/cove-release-gate-reverse.orc /tmp/cove-release-gate-reverse-orc.cove > /dev/null
    cargo run -p cove-cli -- convert orc --help > /dev/null 2>&1
    cargo run -p cove-cli -- map --help > /dev/null 2>&1
    cargo test -p cove-convert-parquet
    cargo run -p cove-bench --bin cove-bench -- check > /dev/null
    test -f docs/governance/semantic-versioning.md
    test -f docs/governance/feature-bit-registry.md
    test -f docs/governance/section-kind-registry.md
    test -f docs/governance/encoding-kind-registry.md
    test -f docs/governance/extension-proposal-process.md
    test -f docs/governance/conformance-levels.md
    test -f docs/governance/security-privacy-model.md
    test -f docs/governance/benchmark-methodology.md
    test -f docs/governance/name-trademark-guidance.md
    grep -R "COVE v2.0" docs/governance > /dev/null
    grep -R "feature-scope" docs/governance > /dev/null
    grep -R "extension fallback" docs/governance > /dev/null
    grep -R "cargo run -p cove-conformance --bin cove-conformance -- conformance/" docs/governance > /dev/null
    cargo run -p cove-fuzz --bin cove-fuzz -- smoke > /dev/null
    cargo run -p cove-conformance --bin gen-corpus -- --check
    cargo run -p cove-conformance --bin gen-capability-matrix -- --check
    cargo run -p cove-conformance --bin cove-conformance -- conformance/
}

publication_gate() {
    full_gate
    cargo run -p cove-bench --bin cove-bench -- gen \
        --profile standard \
        --out target/cove-bench/publication-standard
    cargo run -p cove-bench --bin cove-bench -- run \
        --corpus target/cove-bench/publication-standard \
        --report-json target/cove-bench/publication-standard/report.json \
        --report-md target/cove-bench/publication-standard/report.md
}

mode="${1:---full}"
if [ "$#" -gt 1 ]; then
    usage >&2
    exit 2
fi

case "$mode" in
    --help|-h)
        usage
        ;;
    --quick)
        quick_gate
        ;;
    --full)
        full_gate
        ;;
    --publication)
        publication_gate
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
