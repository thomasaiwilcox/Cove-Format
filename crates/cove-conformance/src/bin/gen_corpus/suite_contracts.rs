use super::*;

pub(super) fn write_suite_contract_fixtures(writer: &mut CorpusWriter<'_>) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    for (path, value) in [
        (
            "accept/suite_manifest_contract.json",
            json!({
                "op": "manifest_sections_present",
                "sections": ["§8", "§10", "§12", "§20", "§37", "§45", "§46", "§47", "§51", "§61", "§62", "§70.2", "§70.3", "§70.5", "§70.6", "§70.8", "§70.9", "§70.10", "§70.12", "§70.13", "§70.14", "§72.8", "§74", "§75", "§76", "§77", "§78", "§79", "§80"],
                "minimum_accept": 1,
                "minimum_reject": 1,
            }),
        ),
        (
            "accept/suite_release_gates_contract.json",
            json!({
                "op": "release_gate_contains",
                "needles": [
                    "cargo fmt --check",
                    "cargo test --workspace",
                    "cargo test -p cove-convert-parquet",
                    "cargo run -p cove-cli -- profile inspect conformance/accept/cove_t_scan_table.cove > /dev/null",
                    "cargo run -p cove-cli -- profile generate --kind engine-registry --out /tmp/cove-release-gate-engine-registry.bin > /dev/null",
                    "cargo run -p cove-cli -- profile validate-section /tmp/cove-release-gate-engine-registry.bin --kind engine-registry > /dev/null",
                    "cargo run -p cove-cli -- canonicalise validate-payload --tag int64 --hex 2a00000000000000 > /dev/null",
                    "cargo run -p cove-cli -- digest verify conformance/accept/cove_t_scan_table.cove > /dev/null",
                    "cargo run -p cove-cli -- delta inspect conformance/accept/covedelta_valid.covedelta --json > /dev/null",
                    "cargo run -p cove-cli -- delta validate conformance/accept/covedelta_object_delta_valid.covedelta --object-delta > /dev/null",
                    "cargo test -p cove-cli delta_cli_commands_validate_plan_and_publish_delta_chains",
                    "cargo run -p cove-cli -- sidecar build covm /tmp/cove-release-gate.covm conformance/accept/cove_t_scan_table.cove > /dev/null",
                    "cargo run -p cove-cli -- sidecar build covx /tmp/cove-release-gate.covx conformance/accept/cove_t_scan_table.cove > /dev/null",
                    "cargo run -p cove-cli -- perf explain-pruning conformance/accept/cove_t_scan_table.cove > /dev/null",
                    "cargo run -p cove-cli -- perf plan-cost --execute conformance/accept/cove_t_scan_table.cove > /dev/null",
                    "cargo run -p cove-cli -- export arrow conformance/accept/cove_t_scan_table.cove /tmp/cove-release-gate.arrow --report /tmp/cove-release-gate-arrow-export.json > /dev/null",
                    "cargo run -p cove-cli -- convert report conformance/accept/parquet_primitives_valid.parquet > /dev/null",
                    "cargo run -p cove-cli -- convert arrow /tmp/cove-release-gate.arrow /tmp/cove-release-gate-arrow.cove > /dev/null",
                    "cargo run -p cove-cli -- convert report --direction cove-to-source --target-format orc --output /tmp/cove-release-gate-reverse.orc conformance/accept/cove_t_scan_table.cove > /dev/null",
                    "cargo run -p cove-cli -- convert orc /tmp/cove-release-gate-reverse.orc /tmp/cove-release-gate-reverse-orc.cove > /dev/null",
                    "cargo run -p cove-cli -- convert orc --help > /dev/null 2>&1",
                    "cargo run -p cove-cli -- map --help > /dev/null 2>&1",
                    "cargo run -p cove-bench --bin cove-bench -- check > /dev/null",
                    "grep -R \"COVE v2.0\" docs/governance > /dev/null",
                    "grep -R \"feature-scope\" docs/governance > /dev/null",
                    "grep -R \"extension fallback\" docs/governance > /dev/null",
                    "cargo run -p cove-fuzz --bin cove-fuzz -- smoke > /dev/null",
                    "cargo run -p cove-conformance --bin gen-corpus -- --check",
                    "cargo run -p cove-conformance --bin gen-capability-matrix -- --check",
                    "cargo run -p cove-conformance --bin cove-conformance -- conformance/"
                ],
            }),
        ),
        (
            "accept/suite_governance_contract.json",
            json!({
                "op": "governance_docs_present",
                "docs": [
                    "docs/governance/semantic-versioning.md",
                    "docs/governance/feature-bit-registry.md",
                    "docs/governance/section-kind-registry.md",
                    "docs/governance/encoding-kind-registry.md",
                    "docs/governance/extension-proposal-process.md",
                    "docs/governance/conformance-levels.md",
                    "docs/governance/security-privacy-model.md",
                    "docs/governance/benchmark-methodology.md",
                    "docs/governance/name-trademark-guidance.md"
                ],
                "needles": [
                    "COVE v2.0",
                    "feature-scope",
                    "extension fallback",
                    "cargo run -p cove-conformance --bin cove-conformance -- conformance/"
                ],
            }),
        ),
        (
            "accept/suite_workspace_contract.json",
            json!({
                "op": "workspace_members_present",
                "members": [
                    "crates/cove-core",
                    "crates/cove-validate",
                    "crates/cove-inspect",
                    "crates/cove-dump",
                    "crates/cove-convert-parquet",
                    "crates/cove-conformance",
                    "crates/cove-map",
                    "crates/cove-bench",
                    "crates/cove-fuzz"
                ],
            }),
        ),
    ] {
        write_fixture(
            root,
            entries,
            fixture(
                path,
                "suite_contract_case",
                "accept",
                None,
                &["§78", "§79", "§80.2", "§80.3A"],
            ),
            suite_contract_fixture_bytes(value),
        );
    }

    let suite_rejects: Vec<(&str, Vec<&str>, Value)> = vec![
        (
            "reject/suite_manifest_missing_section.json",
            vec!["§78", "§76"],
            json!({
                "op": "manifest_sections_present",
                "sections": ["§999"],
                "minimum_accept": 1,
                "minimum_reject": 1,
            }),
        ),
        (
            "reject/suite_release_gate_missing_command.json",
            vec!["§79", "§76"],
            json!({
                "op": "release_gate_contains",
                "needles": ["definitely-missing-release-command"],
            }),
        ),
        (
            "reject/suite_cli_missing_command.json",
            vec!["§80.2", "§76"],
            json!({
                "op": "release_gate_contains",
                "needles": ["cargo run -p missing-cli --bin missing-cli"],
            }),
        ),
        (
            "reject/suite_coverage_report_missing_command.json",
            vec!["§80.3A", "§76"],
            json!({
                "op": "release_gate_contains",
                "needles": ["cove perf plan-cost --definitely-missing-coverage-report-mode"],
            }),
        ),
    ];
    for (path, sections, value) in suite_rejects {
        write_fixture(
            root,
            entries,
            fixture(
                path,
                "suite_contract_case",
                "reject",
                Some("COVE_E_BAD_SECTION"),
                &sections,
            ),
            suite_contract_fixture_bytes(value),
        );
    }
}
