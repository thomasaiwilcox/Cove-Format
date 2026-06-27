use std::{path::Path, sync::Arc};

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use arrow_ipc::writer::FileWriter as IpcFileWriter;
use orc_rust::ArrowWriterBuilder as OrcWriterBuilder;
use parquet::arrow::ArrowWriter;

use cove_core::constants::DigestAlgorithm;
use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1,
        CovemapSection, CovemapSectionEntryV1,
    },
    constants::{PrimaryProfile, SectionKind, FEATURE_SEMANTIC_MAP},
    digest::compute_digest,
    writer::SectionPayload,
};
use serde_json::{json, Value};

use super::{
    covemap_payload_value, fixture, map_payload_bytes, semantic_profile_cove_file,
    suite_contract_fixture_bytes, write_auxiliary_file, write_fixture,
};

const RESOLUTION_CATALOG_SECTIONS: &[&str] = &["§70.5.1", "§73.6"];
const RESOLUTION_CATALOG_REJECT_SECTIONS: &[&str] = &["§70.5.1", "§73.6", "§76"];
const RESOLUTION_ALIAS_SECTIONS: &[&str] = &["§70.3", "§70.5.1", "§70.6", "§73.6"];
const RESOLUTION_ALIAS_REJECT_SECTIONS: &[&str] = &["§70.3", "§70.5.1", "§70.6", "§73.6", "§76"];
const CANDIDATE_RULE_SECTIONS: &[&str] = &["§70.4", "§70.5.1", "§73.6"];
const CANDIDATE_RULE_REJECT_SECTIONS: &[&str] = &["§70.4", "§70.5.1", "§76"];

pub(crate) fn write_cove_map_execution_cases(root: &Path, entries: &mut Vec<Value>) {
    let map_path = "accept/cove_map_execution.covemap";
    let source_path = "accept/people.csv";
    let parquet_source_path = "accept/people.parquet";
    let orc_source_path = "accept/people.orc";
    let arrow_source_path = "accept/people.arrow";
    write_fixture(
        root,
        entries,
        fixture(
            map_path,
            "covemap",
            "accept",
            None,
            &[
                "§70.2", "§70.3", "§70.5", "§70.6", "§70.9", "§70.10", "§70.12", "§70.13", "§72.8",
                "§73.6",
            ],
        ),
        cove_map_execution_file(),
    );
    write_auxiliary_file(root, source_path, cove_map_execution_source_bytes());
    write_auxiliary_file(
        root,
        parquet_source_path,
        &cove_map_execution_parquet_source_bytes(),
    );
    write_auxiliary_file(
        root,
        orc_source_path,
        &cove_map_execution_orc_source_bytes(),
    );
    write_auxiliary_file(
        root,
        arrow_source_path,
        &cove_map_execution_arrow_source_bytes(),
    );

    let candidate_map_path = "accept/cove_map_candidate_identity.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            candidate_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.4", "§70.6", "§72.8", "§73.6"],
        ),
        cove_map_candidate_identity_file(),
    );

    let candidate_map = root.join(candidate_map_path);
    let candidate_sources = vec![root.join(source_path)];
    let candidate_summary =
        cove_map::conversion_summary_from_paths(&candidate_map, &candidate_sources).unwrap();
    let candidate_report = candidate_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_candidate_identity_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.4", "§70.6", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": candidate_map_path,
            "sources": [source_path],
            "expected_conversion": {
                "object_count": candidate_report["object_count"],
                "association_count": candidate_report["association_count"],
                "candidate_match_count": candidate_report["candidate_match_count"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": candidate_summary["materialized_row_count"],
                "evidence_entry_count": candidate_summary["evidence_entry_count"],
                "assertion_count": candidate_summary["assertion_count"],
            }
        })),
    );

    let association_only_map_path = "accept/cove_map_association_only.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            association_only_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.9", "§72.8", "§73.6"],
        ),
        cove_map_association_only_file(),
    );
    let association_only_summary = cove_map::conversion_summary_from_paths(
        &root.join(association_only_map_path),
        &[root.join(source_path)],
    )
    .unwrap();
    let association_only_report = association_only_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_association_only_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.3", "§70.9", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": association_only_map_path,
            "sources": [source_path],
            "expected_conversion": {
                "object_count": association_only_report["object_count"],
                "association_count": association_only_report["association_count"],
            },
            "expect_cove_o_valid": true,
            "expect_association_readback_flags": true,
        })),
    );

    let alias_association_map_path = "accept/cove_map_alias_backed_association.covemap";
    let alias_association_memberships_path = "accept/cove_map_alias_memberships.csv";
    let alias_association_teams_path = "accept/cove_map_alias_teams.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_association_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.9", "§72.8", "§73.6"],
        ),
        cove_map_alias_backed_association_file(),
    );
    write_auxiliary_file(
        root,
        alias_association_memberships_path,
        b"person_id,team_name,valid_from,valid_to\np1,Alpha Team Ltd,2026-01-01,2026-12-31\n",
    );
    write_auxiliary_file(
        root,
        alias_association_teams_path,
        b"team_name\nTeam Alpha\n",
    );
    let alias_association_summary = cove_map::conversion_summary_from_paths(
        &root.join(alias_association_map_path),
        &[
            root.join(alias_association_memberships_path),
            root.join(alias_association_teams_path),
        ],
    )
    .unwrap();
    let alias_association_report = alias_association_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_alias_backed_association_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.3", "§70.9", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_association_map_path,
            "sources": [alias_association_memberships_path, alias_association_teams_path],
            "expected_conversion": {
                "object_count": alias_association_report["object_count"],
                "association_count": alias_association_report["association_count"],
                "resolver_hit_count": alias_association_report["resolver_hit_count"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": alias_association_summary["materialized_row_count"],
                "evidence_entry_count": alias_association_summary["evidence_entry_count"],
                "assertion_count": alias_association_summary["assertion_count"],
            },
            "expected_evidence_entries": [{
                "contains": {
                    "source_id": "cove_map_alias_teams",
                    "rule_id": "team_row",
                    "alias_hit": true,
                    "canonical_key": "team:alpha"
                },
                "present": ["resolver_digest", "catalog_digest", "pipeline_digest"]
            }],
            "expect_cove_o_valid": true,
            "expect_association_readback_flags": true,
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_composite_row_semantics.covemap",
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.9", "§72.8", "§73.6"],
        ),
        cove_map_composite_row_semantics_file(),
    );

    let tombstone_map_path = "accept/cove_map_tombstone_row_semantics.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            tombstone_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§72.8", "§73.6"],
        ),
        cove_map_tombstone_row_semantics_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_tombstone_row_semantics_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.3", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": tombstone_map_path,
            "sources": [source_path],
            "expected_conversion": {
                "object_count": 2,
                "association_count": 0,
            },
            "expect_cove_o_valid": true,
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_invalid_row_semantics.covemap",
            "covemap",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.3", "§76"],
        ),
        cove_map_invalid_row_semantics_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_association_bad_endpoint.covemap",
            "covemap",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.9", "§76"],
        ),
        cove_map_association_bad_endpoint_file(),
    );

    let missing_policy_map_path = "reject/cove_map_projection_missing_policy.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            missing_policy_map_path,
            "covemap",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.10", "§76"],
        ),
        cove_map_projection_missing_policy_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_catalog.covemap",
            "covemap",
            "accept",
            None,
            RESOLUTION_CATALOG_SECTIONS,
        ),
        cove_map_resolution_catalog_file(false),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_catalog_bad_digest.covemap",
            "covemap",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            RESOLUTION_CATALOG_REJECT_SECTIONS,
        ),
        cove_map_resolution_catalog_file(true),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_alias_entry_reorder_digest.covemap",
            "covemap",
            "accept",
            None,
            RESOLUTION_CATALOG_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x95,
            &["company_aliases"],
            company_resolution_catalog_value_with_reordered_alias_entries(),
        ),
    );

    let resolution_alias_map_path = "accept/cove_map_resolution_alias.covemap";
    let resolution_alias_source_path = "accept/company_aliases.csv";
    write_fixture(
        root,
        entries,
        fixture(
            resolution_alias_map_path,
            "covemap",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        cove_map_resolution_alias_file(),
    );
    write_auxiliary_file(
        root,
        resolution_alias_source_path,
        b"company_name\nTesco\nTesco PLC\ntesco supermarket\n",
    );
    let resolution_summary = cove_map::conversion_summary_from_paths(
        &root.join(resolution_alias_map_path),
        &[root.join(resolution_alias_source_path)],
    )
    .unwrap();
    let resolution_report = resolution_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_alias_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": resolution_alias_map_path,
            "sources": [resolution_alias_source_path],
            "expected_conversion": {
                "candidate_match_count": 0,
                "resolver_hit_count": resolution_report["resolver_hit_count"],
                "resolver_miss_count": resolution_report["resolver_miss_count"],
                "ambiguous_alias_count": resolution_report["ambiguous_alias_count"],
                "resolver_goid_impact": resolution_report["resolver_goid_impact"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": resolution_summary["materialized_row_count"],
                "evidence_entry_count": resolution_summary["evidence_entry_count"],
            },
            "expect_cove_o_valid": true,
        })),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_replay_verify_case.json",
            "cove_map_replay_case",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": resolution_alias_map_path,
            "sources": [resolution_alias_source_path],
            "expected_replay": {
                "ok": true,
                "resolver_catalog_digest_count": 1,
                "reviewed_decision_count": 0,
            },
        })),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_replay_verify_stale_resolver_case.json",
            "cove_map_replay_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": resolution_alias_map_path,
            "sources": [resolution_alias_source_path],
            "mutate_report": "resolver_digest",
            "expected_error_contains": "MAP_REPLAY_STALE_RESOLVER",
        })),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_replay_verify_stale_review_case.json",
            "cove_map_replay_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": resolution_alias_map_path,
            "sources": [resolution_alias_source_path],
            "mutate_report": "reviewed_decision_digest",
            "expected_error_contains": "MAP_REPLAY_STALE_REVIEW",
        })),
    );

    let resolution_projection_map_path = "accept/cove_map_resolution_projection.covemap";
    let resolution_projection_source_path = "accept/company_resolution_projection.csv";
    write_fixture(
        root,
        entries,
        fixture(
            resolution_projection_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.5.1", "§70.10", "§73.6"],
        ),
        cove_map_resolution_projection_file(
            0x8e,
            "company_resolution_projection",
            company_resolution_catalog_value(),
            false,
            json!([
                {"name": "canonical_key", "value": "identity(company_by_resolved_name).resolution(company).canonical_key"},
                {"name": "canonical_label", "value": "identity(company_by_resolved_name).resolution(company).canonical_label"},
                {"name": "normalized_value", "value": "identity(company_by_resolved_name).resolution(company).normalized_value"},
                {"name": "raw_observed_value", "value": "identity(company_by_resolved_name).resolution(company).raw_observed_value"}
            ]),
        ),
    );
    write_auxiliary_file(
        root,
        resolution_projection_source_path,
        b"company_name\nTesco PLC\n",
    );
    let resolution_projected = cove_map::projected_rows_from_paths(
        &root.join(resolution_projection_map_path),
        &[root.join(resolution_projection_source_path)],
    )
    .unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_projection_case.json",
            "cove_map_project_case",
            "accept",
            None,
            &["§70.3", "§70.5.1", "§70.10", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": resolution_projection_map_path,
            "sources": [resolution_projection_source_path],
            "expected_projection": {
                "format": resolution_projected["format"],
                "mapping_id": resolution_projected["mapping_id"],
                "mapping_version": resolution_projected["mapping_version"],
            },
            "expected_projected_rows": resolution_projected["rows"],
            "expect_persisted_projection_rows": true,
        })),
    );

    let resolution_role_map_path = "accept/cove_map_resolution_projection_roles.covemap";
    let resolution_role_source_path = "accept/company_resolution_projection_roles.csv";
    write_fixture(
        root,
        entries,
        fixture(
            resolution_role_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.5.1", "§70.10", "§73.6"],
        ),
        cove_map_resolution_projection_file(
            0x8f,
            "company_resolution_projection_roles",
            company_resolution_catalog_value(),
            true,
            json!([
                {"name": "company_raw", "value": "identity(company_by_resolved_name).resolution(company).raw_observed_value"},
                {"name": "parent_raw", "value": "identity(company_by_resolved_name).resolution(parent_company).raw_observed_value"}
            ]),
        ),
    );
    write_auxiliary_file(
        root,
        resolution_role_source_path,
        b"company_name,parent_company_name\nTesco,Tesco PLC\n",
    );
    let resolution_role_projected = cove_map::projected_rows_from_paths(
        &root.join(resolution_role_map_path),
        &[root.join(resolution_role_source_path)],
    )
    .unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_projection_roles_case.json",
            "cove_map_project_case",
            "accept",
            None,
            &["§70.3", "§70.5.1", "§70.10", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": resolution_role_map_path,
            "sources": [resolution_role_source_path],
            "expected_projection": {
                "format": resolution_role_projected["format"],
                "mapping_id": resolution_role_projected["mapping_id"],
                "mapping_version": resolution_role_projected["mapping_version"],
            },
            "expected_projected_rows": resolution_role_projected["rows"],
        })),
    );

    let resolution_fail_closed_map_path =
        "accept/cove_map_resolution_projection_missing_hit.covemap";
    let resolution_fail_closed_source_path = "accept/company_resolution_projection_missing_hit.csv";
    write_fixture(
        root,
        entries,
        fixture(
            resolution_fail_closed_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.5.1", "§70.10", "§73.6"],
        ),
        cove_map_resolution_projection_file(
            0x90,
            "company_resolution_projection_missing_hit",
            company_resolution_catalog_value_with_policy(
                "normalized_value",
                Some("weak_deterministic"),
                "reject_auto_merge",
                false,
            ),
            false,
            json!([
                {"name": "canonical_key", "value": "identity(company_by_resolved_name).resolution(company).canonical_key"}
            ]),
        ),
    );
    write_auxiliary_file(
        root,
        resolution_fail_closed_source_path,
        b"company_name\nUnknown Stores\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_projection_missing_hit_case.json",
            "cove_map_project_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.3", "§70.5.1", "§70.10", "§73.6", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": resolution_fail_closed_map_path,
            "sources": [resolution_fail_closed_source_path],
        })),
    );

    let redacted_alias_map_path = "accept/cove_map_resolution_alias_redacted.covemap";
    let redacted_alias_source_path = "accept/company_alias_redacted.csv";
    write_fixture(
        root,
        entries,
        fixture(
            redacted_alias_map_path,
            "covemap",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x7f,
            &["company_alias_redacted"],
            company_resolution_catalog_value_with_policy_and_evidence(
                "candidate_only",
                None,
                "reject_auto_merge",
                false,
                "redact_raw",
            ),
        ),
    );
    write_auxiliary_file(
        root,
        redacted_alias_source_path,
        b"company_name\nTesco PLC\n",
    );
    let redacted_alias_summary = cove_map::conversion_summary_from_paths(
        &root.join(redacted_alias_map_path),
        &[root.join(redacted_alias_source_path)],
    )
    .unwrap();
    let redacted_alias_report = redacted_alias_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_alias_redacted_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": redacted_alias_map_path,
            "sources": [redacted_alias_source_path],
            "expected_conversion": {
                "object_count": redacted_alias_report["object_count"],
                "candidate_match_count": 0,
                "resolver_hit_count": 1,
                "resolver_miss_count": 0,
                "ambiguous_alias_count": 0,
            },
            "expected_conversion_summary": {
                "materialized_row_count": redacted_alias_summary["materialized_row_count"],
                "evidence_entry_count": redacted_alias_summary["evidence_entry_count"],
                "assertion_count": redacted_alias_summary["assertion_count"],
            },
            "expected_evidence_entries": [{
                "contains": {
                    "resolver_id": "uk_company_name_resolver",
                    "alias_hit": true,
                    "canonical_key": "uk-company:tesco",
                    "canonical_label": "Tesco",
                    "evidence_policy": "redact_raw",
                    "redacted_resolution_evidence": true,
                    "redacted": true,
                    "redaction_scope": "resolver_evidence"
                },
                "present": ["resolver_digest", "catalog_digest", "pipeline_digest"],
                "absent": ["raw_observed_value", "normalized_value"]
            }],
            "expect_cove_o_valid": true,
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_normalized_missing_confidence.covemap",
            "covemap",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x83,
            &["company_alias_normalized_missing_confidence"],
            company_resolution_catalog_value_with_policy(
                "normalized_value",
                None,
                "reject_auto_merge",
                false,
            ),
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_authoritative_miss_confidence.covemap",
            "covemap",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x84,
            &["company_alias_authoritative_miss_confidence"],
            company_resolution_catalog_value_with_policy(
                "normalized_value",
                Some("authoritative"),
                "reject_auto_merge",
                false,
            ),
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_stale_pipeline_version.covemap",
            "covemap",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x85,
            &["company_alias_stale_pipeline_version"],
            company_resolution_catalog_value_with_stale_pipeline_version(),
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_stale_suffix_table_digest.covemap",
            "covemap",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x86,
            &["company_alias_stale_suffix_table_digest"],
            company_resolution_catalog_value_with_stale_suffix_table_digest(),
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_stale_resolver_digest.covemap",
            "covemap",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x87,
            &["company_alias_stale_resolver_digest"],
            company_resolution_catalog_value_with_stale_resolver_digest(),
        ),
    );

    let alias_miss_reject_map_path = "accept/cove_map_resolution_alias_miss_reject.covemap";
    let alias_miss_reject_source_path = "accept/company_alias_miss_reject.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_miss_reject_map_path,
            "covemap",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x7a,
            &["company_alias_miss_reject"],
            company_resolution_catalog_value_with_policy(
                "reject",
                None,
                "reject_auto_merge",
                false,
            ),
        ),
    );
    write_auxiliary_file(
        root,
        alias_miss_reject_source_path,
        b"company_name\nUnknown Stores\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_miss_reject_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_miss_reject_map_path,
            "sources": [alias_miss_reject_source_path],
        })),
    );

    let alias_miss_candidate_map_path = "accept/cove_map_resolution_alias_miss_candidate.covemap";
    let alias_miss_candidate_source_path = "accept/company_alias_miss_candidate.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_miss_candidate_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.4", "§70.5.1", "§73.6"],
        ),
        cove_map_resolution_alias_policy_file(
            0x7b,
            &["company_alias_miss_candidate"],
            company_resolution_catalog_value_with_policy(
                "candidate_only",
                None,
                "reject_auto_merge",
                false,
            ),
        ),
    );
    write_auxiliary_file(
        root,
        alias_miss_candidate_source_path,
        b"company_name\nUnknown Stores\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_alias_miss_candidate_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.3", "§70.4", "§70.5.1", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_miss_candidate_map_path,
            "sources": [alias_miss_candidate_source_path],
            "expected_conversion": {
                "source_count": 1,
                "row_count": 1,
                "object_count": 0,
                "candidate_match_count": 1,
                "resolver_hit_count": 0,
                "resolver_miss_count": 1,
                "ambiguous_alias_count": 0,
            },
            "expected_conversion_summary": {
                "materialized_row_count": 0,
                "evidence_entry_count": 1,
                "assertion_count": 1,
            }
        })),
    );

    let alias_miss_source_scoped_map_path =
        "accept/cove_map_resolution_alias_miss_source_scoped.covemap";
    let alias_miss_source_scoped_left_path = "accept/company_alias_source_a.csv";
    let alias_miss_source_scoped_right_path = "accept/company_alias_source_b.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_miss_source_scoped_map_path,
            "covemap",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x7c,
            &["company_alias_source_a", "company_alias_source_b"],
            company_resolution_catalog_value_with_policy(
                "source_scoped",
                None,
                "reject_auto_merge",
                false,
            ),
        ),
    );
    write_auxiliary_file(
        root,
        alias_miss_source_scoped_left_path,
        b"company_name\nUnknown Stores\n",
    );
    write_auxiliary_file(
        root,
        alias_miss_source_scoped_right_path,
        b"company_name\nUnknown Stores\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_alias_miss_source_scoped_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_miss_source_scoped_map_path,
            "sources": [
                alias_miss_source_scoped_left_path,
                alias_miss_source_scoped_right_path
            ],
            "expected_conversion": {
                "source_count": 2,
                "row_count": 2,
                "object_count": 2,
                "candidate_match_count": 0,
                "resolver_hit_count": 0,
                "resolver_miss_count": 2,
                "ambiguous_alias_count": 0,
            },
            "expected_conversion_summary": {
                "materialized_row_count": 2,
                "evidence_entry_count": 2,
                "assertion_count": 2,
            },
            "expect_cove_o_valid": true,
        })),
    );

    let alias_ambiguous_reject_map_path =
        "accept/cove_map_resolution_alias_ambiguous_reject.covemap";
    let alias_ambiguous_reject_source_path = "accept/company_alias_ambiguous_reject.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_ambiguous_reject_map_path,
            "covemap",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x7d,
            &["company_alias_ambiguous_reject"],
            company_resolution_catalog_value_with_policy(
                "candidate_only",
                None,
                "reject_auto_merge",
                true,
            ),
        ),
    );
    write_auxiliary_file(
        root,
        alias_ambiguous_reject_source_path,
        b"company_name\nTesco\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_ambiguous_reject_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_ambiguous_reject_map_path,
            "sources": [alias_ambiguous_reject_source_path],
        })),
    );

    let alias_ambiguous_candidate_map_path =
        "accept/cove_map_resolution_alias_ambiguous_candidate.covemap";
    let alias_ambiguous_candidate_source_path = "accept/company_alias_ambiguous_candidate.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_ambiguous_candidate_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.4", "§70.5.1", "§73.6"],
        ),
        cove_map_resolution_alias_policy_file(
            0x7e,
            &["company_alias_ambiguous_candidate"],
            company_resolution_catalog_value_with_policy(
                "candidate_only",
                None,
                "candidate_only",
                true,
            ),
        ),
    );
    write_auxiliary_file(
        root,
        alias_ambiguous_candidate_source_path,
        b"company_name\nTesco\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_alias_ambiguous_candidate_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.3", "§70.4", "§70.5.1", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_ambiguous_candidate_map_path,
            "sources": [alias_ambiguous_candidate_source_path],
            "expected_conversion": {
                "source_count": 1,
                "row_count": 1,
                "object_count": 0,
                "candidate_match_count": 1,
                "resolver_hit_count": 0,
                "resolver_miss_count": 0,
                "ambiguous_alias_count": 1,
            },
            "expected_conversion_summary": {
                "materialized_row_count": 0,
                "evidence_entry_count": 1,
                "assertion_count": 1,
            }
        })),
    );

    let alias_normalized_collision_reject_map_path =
        "accept/cove_map_resolution_alias_normalized_collision_reject.covemap";
    let alias_normalized_collision_reject_source_path =
        "accept/company_alias_normalized_collision_reject.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_normalized_collision_reject_map_path,
            "covemap",
            "accept",
            None,
            RESOLUTION_ALIAS_SECTIONS,
        ),
        cove_map_resolution_alias_policy_file(
            0x8c,
            &["company_alias_normalized_collision_reject"],
            company_resolution_catalog_value_with_normalized_alias_collision(
                "reject_auto_merge",
                false,
            ),
        ),
    );
    write_auxiliary_file(
        root,
        alias_normalized_collision_reject_source_path,
        b"company_name\nTESCO PLC\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_resolution_alias_normalized_collision_reject_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            RESOLUTION_ALIAS_REJECT_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_normalized_collision_reject_map_path,
            "sources": [alias_normalized_collision_reject_source_path],
        })),
    );

    let alias_normalized_collision_candidate_map_path =
        "accept/cove_map_resolution_alias_normalized_collision_candidate.covemap";
    let alias_normalized_collision_candidate_source_path =
        "accept/company_alias_normalized_collision_candidate.csv";
    write_fixture(
        root,
        entries,
        fixture(
            alias_normalized_collision_candidate_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.4", "§70.5.1", "§73.6"],
        ),
        cove_map_resolution_alias_policy_file(
            0x8d,
            &["company_alias_normalized_collision_candidate"],
            company_resolution_catalog_value_with_normalized_alias_collision(
                "candidate_only",
                true,
            ),
        ),
    );
    write_auxiliary_file(
        root,
        alias_normalized_collision_candidate_source_path,
        b"company_name\nTESCO PLC\n",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_resolution_alias_normalized_collision_candidate_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.3", "§70.4", "§70.5.1", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_normalized_collision_candidate_map_path,
            "sources": [alias_normalized_collision_candidate_source_path],
            "expected_conversion": {
                "source_count": 1,
                "row_count": 1,
                "object_count": 0,
                "candidate_match_count": 1,
                "resolver_hit_count": 0,
                "resolver_miss_count": 0,
                "ambiguous_alias_count": 1,
            },
            "expected_conversion_summary": {
                "materialized_row_count": 0,
                "evidence_entry_count": 1,
                "assertion_count": 1,
            },
            "expected_evidence_entries": [{
                "contains": {
                    "candidate": true,
                    "alias_ambiguous": true,
                    "alias_catalog_id": "company_aliases"
                },
                "present": ["resolver_digest", "catalog_digest", "pipeline_digest"]
            }]
        })),
    );

    let candidate_rules_map_path = "accept/cove_map_candidate_rules.covemap";
    let candidate_rules_source_path = "accept/company_candidates.csv";
    write_fixture(
        root,
        entries,
        fixture(
            candidate_rules_map_path,
            "covemap",
            "accept",
            None,
            CANDIDATE_RULE_SECTIONS,
        ),
        cove_map_candidate_rules_file(0x6f, 10),
    );
    write_auxiliary_file(
        root,
        candidate_rules_source_path,
        b"company_name\nTesco PLC\nTesco supermarket\n",
    );
    let candidate_rules_map = root.join(candidate_rules_map_path);
    let candidate_rules_sources = vec![root.join(candidate_rules_source_path)];
    let candidate_output =
        cove_map::candidate_matches_from_paths(&candidate_rules_map, &candidate_rules_sources)
            .unwrap();
    let candidate_summary =
        cove_map::conversion_summary_from_paths(&candidate_rules_map, &candidate_rules_sources)
            .unwrap();
    let candidate_report = candidate_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_candidate_rules_case.json",
            "cove_map_candidates_case",
            "accept",
            None,
            CANDIDATE_RULE_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": candidate_rules_map_path,
            "sources": [candidate_rules_source_path],
            "expected_candidates": {
                "candidate_match_count": candidate_output["candidate_matches"].as_array().unwrap().len(),
                "diagnostic_count": candidate_output["diagnostics"].as_array().unwrap().len(),
                "first_match": {
                    "match_rule_id": "company_name_similarity",
                    "object_type": "Company",
                    "candidate_score": 333333,
                    "score_scale": 1000000,
                    "blocking_key": "tesc",
                    "merge_behavior": "never"
                },
                "first_left": {
                    "source_id": "company_candidates",
                    "row_index": 0,
                    "column": "company_name",
                    "raw_value": "Tesco PLC",
                    "normalized_value": "tesco plc"
                },
                "first_right": {
                    "source_id": "company_candidates",
                    "row_index": 1,
                    "column": "company_name",
                    "raw_value": "Tesco supermarket",
                    "normalized_value": "tesco supermarket"
                }
            }
        })),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_candidate_rules_convert_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.4", "§70.5.1", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": candidate_rules_map_path,
            "sources": [candidate_rules_source_path],
            "expected_conversion": {
                "object_count": candidate_report["object_count"],
                "candidate_match_count": candidate_report["candidate_match_count"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": candidate_summary["materialized_row_count"],
                "evidence_entry_count": candidate_summary["evidence_entry_count"],
                "assertion_count": candidate_summary["assertion_count"],
            },
            "expect_cove_o_valid": true,
        })),
    );

    let candidate_tie_map_path = "accept/cove_map_candidate_rules_tie.covemap";
    let candidate_tie_source_path = "accept/company_candidates_tie.csv";
    write_fixture(
        root,
        entries,
        fixture(
            candidate_tie_map_path,
            "covemap",
            "accept",
            None,
            CANDIDATE_RULE_SECTIONS,
        ),
        cove_map_candidate_rules_file_for_source(0x91, 10, "company_candidates_tie"),
    );
    write_auxiliary_file(
        root,
        candidate_tie_source_path,
        b"company_name\nAlpha Beta\nAlpha Gamma\nAlpha Delta\n",
    );
    let candidate_tie_output = cove_map::candidate_matches_from_paths(
        &root.join(candidate_tie_map_path),
        &[root.join(candidate_tie_source_path)],
    )
    .unwrap();
    let candidate_tie_order = candidate_tie_output["candidate_matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| {
            json!({
                "match": {
                    "match_rule_id": candidate["match_rule_id"],
                    "candidate_score": candidate["candidate_score"],
                    "score_scale": candidate["score_scale"],
                    "blocking_key": candidate["blocking_key"],
                    "merge_behavior": candidate["merge_behavior"]
                },
                "left": {
                    "source_id": candidate["left"]["source_id"],
                    "row_index": candidate["left"]["row_index"],
                    "normalized_value": candidate["left"]["normalized_value"]
                },
                "right": {
                    "source_id": candidate["right"]["source_id"],
                    "row_index": candidate["right"]["row_index"],
                    "normalized_value": candidate["right"]["normalized_value"]
                }
            })
        })
        .collect::<Vec<_>>();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_candidate_rules_tie_case.json",
            "cove_map_candidates_case",
            "accept",
            None,
            CANDIDATE_RULE_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": candidate_tie_map_path,
            "sources": [candidate_tie_source_path],
            "expected_candidates": {
                "candidate_match_count": 3,
                "diagnostic_count": 0,
                "match_order": candidate_tie_order
            }
        })),
    );

    let candidate_rules_limit_map_path = "accept/cove_map_candidate_rules_limit.covemap";
    write_auxiliary_file(
        root,
        candidate_rules_limit_map_path,
        &cove_map_candidate_rules_file(0x70, 0),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_candidate_rules_limit_case.json",
            "cove_map_candidates_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            CANDIDATE_RULE_REJECT_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": candidate_rules_limit_map_path,
            "sources": [candidate_rules_source_path],
        })),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_candidate_rules_limit_convert_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            CANDIDATE_RULE_REJECT_SECTIONS,
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": candidate_rules_limit_map_path,
            "sources": [candidate_rules_source_path],
        })),
    );

    let reviewed_map_path = "accept/cove_map_reviewed_equivalence.covemap";
    let reviewed_crm_path = "accept/reviewed_crm.csv";
    let reviewed_support_path = "accept/reviewed_support.csv";
    let reviewed_ops_path = "accept/reviewed_ops.csv";
    write_fixture(
        root,
        entries,
        fixture(
            reviewed_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.6", "§73.6"],
        ),
        cove_map_reviewed_equivalence_file(0x71, true, false),
    );
    write_auxiliary_file(root, reviewed_crm_path, b"id\n1\n");
    write_auxiliary_file(root, reviewed_support_path, b"id\n2\n");
    write_auxiliary_file(root, reviewed_ops_path, b"id\n3\n");
    let reviewed_summary = cove_map::conversion_summary_from_paths(
        &root.join(reviewed_map_path),
        &[
            root.join(reviewed_crm_path),
            root.join(reviewed_support_path),
        ],
    )
    .unwrap();
    let reviewed_report = reviewed_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_reviewed_equivalence_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.6", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": reviewed_map_path,
            "sources": [reviewed_crm_path, reviewed_support_path],
            "expected_conversion": {
                "object_count": reviewed_report["object_count"],
                "candidate_match_count": reviewed_report["candidate_match_count"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": reviewed_summary["materialized_row_count"],
                "evidence_entry_count": reviewed_summary["evidence_entry_count"],
                "assertion_count": reviewed_summary["assertion_count"],
            },
            "expect_cove_o_valid": true,
        })),
    );

    let reviewed_transitive_map_path = "accept/cove_map_reviewed_equivalence_transitive.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            reviewed_transitive_map_path,
            "covemap",
            "accept",
            None,
            &["§70.6", "§72.8", "§73.6"],
        ),
        cove_map_reviewed_transitive_equivalence_file(0x92, false),
    );
    let reviewed_transitive_summary = cove_map::conversion_summary_from_paths(
        &root.join(reviewed_transitive_map_path),
        &[
            root.join(reviewed_crm_path),
            root.join(reviewed_support_path),
            root.join(reviewed_ops_path),
        ],
    )
    .unwrap();
    let reviewed_transitive_report = reviewed_transitive_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_reviewed_equivalence_transitive_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.6", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": reviewed_transitive_map_path,
            "sources": [reviewed_crm_path, reviewed_support_path, reviewed_ops_path],
            "expected_conversion": {
                "object_count": reviewed_transitive_report["object_count"],
                "candidate_match_count": reviewed_transitive_report["candidate_match_count"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": reviewed_transitive_summary["materialized_row_count"],
                "evidence_entry_count": reviewed_transitive_summary["evidence_entry_count"],
                "assertion_count": reviewed_transitive_summary["assertion_count"],
            },
            "expect_cove_o_valid": true,
        })),
    );

    let reviewed_cross_rule_map_path =
        "accept/cove_map_reviewed_cross_rule_canonical_anchor.covemap";
    let reviewed_cross_rule_source_path = "accept/reviewed_cross_rule.csv";
    let reviewed_cross_rule_source = reviewed_cross_rule_source_bytes();
    write_auxiliary_file(
        root,
        reviewed_cross_rule_source_path,
        reviewed_cross_rule_source,
    );
    write_fixture(
        root,
        entries,
        fixture(
            reviewed_cross_rule_map_path,
            "covemap",
            "accept",
            None,
            &["§70.6", "§72.8", "§73.6"],
        ),
        cove_map_reviewed_cross_rule_anchor_file(0x94, reviewed_cross_rule_source),
    );
    let reviewed_cross_rule_summary = cove_map::conversion_summary_from_paths(
        &root.join(reviewed_cross_rule_map_path),
        &[root.join(reviewed_cross_rule_source_path)],
    )
    .unwrap();
    let reviewed_cross_rule_report = reviewed_cross_rule_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_reviewed_cross_rule_canonical_anchor_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.6", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": reviewed_cross_rule_map_path,
            "sources": [reviewed_cross_rule_source_path],
            "expected_conversion": {
                "object_count": reviewed_cross_rule_report["object_count"],
                "candidate_match_count": reviewed_cross_rule_report["candidate_match_count"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": reviewed_cross_rule_summary["materialized_row_count"],
                "evidence_entry_count": reviewed_cross_rule_summary["evidence_entry_count"],
                "assertion_count": reviewed_cross_rule_summary["assertion_count"],
                "identity_equivalence_index": reviewed_cross_rule_summary["identity_equivalence_index"],
            },
            "expect_cove_o_valid": true,
        })),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_replay_verify_stale_source_case.json",
            "cove_map_replay_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.6", "§72.8", "§73.6", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": reviewed_cross_rule_map_path,
            "sources": [reviewed_cross_rule_source_path],
            "mutate_report": "source_snapshot_digest",
            "expected_error_contains": "MAP_REPLAY_SOURCE_STALE",
        })),
    );

    let reviewed_disallowed_map_path = "accept/cove_map_reviewed_equivalence_disallowed.covemap";
    write_auxiliary_file(
        root,
        reviewed_disallowed_map_path,
        &cove_map_reviewed_equivalence_file(0x72, false, false),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_reviewed_equivalence_disallowed_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.6", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": reviewed_disallowed_map_path,
            "sources": [reviewed_crm_path, reviewed_support_path],
        })),
    );

    let reviewed_conflict_map_path = "accept/cove_map_reviewed_equivalence_conflict.covemap";
    write_auxiliary_file(
        root,
        reviewed_conflict_map_path,
        &cove_map_reviewed_equivalence_file(0x73, true, true),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_reviewed_equivalence_conflict_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.6", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": reviewed_conflict_map_path,
            "sources": [reviewed_crm_path, reviewed_support_path],
        })),
    );

    let reviewed_transitive_conflict_map_path =
        "accept/cove_map_reviewed_equivalence_transitive_conflict.covemap";
    write_auxiliary_file(
        root,
        reviewed_transitive_conflict_map_path,
        &cove_map_reviewed_transitive_equivalence_file(0x93, true),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_reviewed_equivalence_transitive_conflict_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.6", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": reviewed_transitive_conflict_map_path,
            "sources": [reviewed_crm_path, reviewed_support_path, reviewed_ops_path],
        })),
    );

    for (path, file_id_byte, missing_field) in [
        (
            "reject/cove_map_reviewed_source_row_missing_snapshot.covemap",
            0x88,
            "source_snapshot_digest",
        ),
        (
            "reject/cove_map_reviewed_source_row_missing_schema.covemap",
            0x89,
            "schema_fingerprint",
        ),
        (
            "reject/cove_map_reviewed_source_row_missing_object_type.covemap",
            0x8a,
            "object_type",
        ),
        (
            "reject/cove_map_reviewed_source_row_missing_identity_rule.covemap",
            0x8b,
            "identity_rule_id",
        ),
    ] {
        write_fixture(
            root,
            entries,
            fixture(
                path,
                "covemap",
                "reject",
                Some("COVE_E_MAP_INVALID"),
                &["§70.6", "§73.6", "§76"],
            ),
            cove_map_reviewed_source_row_missing_field_file(file_id_byte, missing_field),
        );
    }

    write_cove_map_case_pair(
        root,
        entries,
        map_path,
        &[source_path],
        "accept/cove_map_convert_case.json",
        "accept/cove_map_project_case.json",
    );
    write_cove_map_case_pair(
        root,
        entries,
        map_path,
        &[parquet_source_path],
        "accept/cove_map_convert_parquet_case.json",
        "accept/cove_map_project_parquet_case.json",
    );
    write_cove_map_case_pair(
        root,
        entries,
        map_path,
        &[orc_source_path],
        "accept/cove_map_convert_orc_case.json",
        "accept/cove_map_project_orc_case.json",
    );
    write_cove_map_case_pair(
        root,
        entries,
        map_path,
        &[arrow_source_path],
        "accept/cove_map_convert_arrow_case.json",
        "accept/cove_map_project_arrow_case.json",
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_missing_source.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.2", "§73.6", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": map_path,
            "sources": [],
        })),
    );

    let undeclared_projection_function_map_path =
        "accept/cove_map_projection_undeclared_function.covemap";
    write_auxiliary_file(
        root,
        undeclared_projection_function_map_path,
        &cove_map_projection_undeclared_function_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_projection_undeclared_function_case.json",
            "cove_map_project_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.10", "§70.13", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": undeclared_projection_function_map_path,
            "sources": [source_path],
        })),
    );

    let aggregate_without_policy_map_path = "accept/cove_map_projection_aggregate_policy.covemap";
    write_auxiliary_file(
        root,
        aggregate_without_policy_map_path,
        &cove_map_projection_aggregate_policy_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_projection_aggregate_policy_case.json",
            "cove_map_project_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.10", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": aggregate_without_policy_map_path,
            "sources": [source_path],
        })),
    );

    let crm_path = "accept/cove_map_crm.csv";
    let support_path = "accept/cove_map_support.csv";
    let crm_parquet_path = "accept/cove_map_crm.parquet";
    let support_orc_path = "accept/cove_map_support.orc";
    write_auxiliary_file(root, crm_path, cove_map_crm_source_bytes());
    write_auxiliary_file(root, support_path, cove_map_support_source_bytes());
    write_auxiliary_file(root, crm_parquet_path, &cove_map_crm_parquet_source_bytes());
    write_auxiliary_file(root, support_orc_path, &cove_map_support_orc_source_bytes());

    let priority_map_path = "accept/cove_map_source_priority.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            priority_map_path,
            "covemap",
            "accept",
            None,
            &["§70.8", "§70.14", "§72.8", "§73.6"],
        ),
        cove_map_conflict_file("source_priority_wins", "emit_effective_policy"),
    );
    let priority_map = root.join(priority_map_path);
    let priority_sources = vec![root.join(crm_path), root.join(support_path)];
    let priority_summary =
        cove_map::conversion_summary_from_paths(&priority_map, &priority_sources).unwrap();
    let priority_report = priority_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_source_priority_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.8", "§70.14", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": priority_map_path,
            "sources": [crm_path, support_path],
            "expected_conversion": {
                "mapping_id": priority_report["mapping_id"],
                "mapping_version": priority_report["mapping_version"],
                "property_value_count": priority_report["property_value_count"],
                "governance": priority_report["governance"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": priority_summary["materialized_row_count"],
                "evidence_entry_count": priority_summary["evidence_entry_count"],
            },
            "expect_cove_o_valid": true,
        })),
    );

    let mixed_priority_map_path = "accept/cove_map_source_priority_projectable.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            mixed_priority_map_path,
            "covemap",
            "accept",
            None,
            &["§70.8", "§70.10", "§70.14", "§72.8", "§73.6"],
        ),
        cove_map_projectable_conflict_file("source_priority_wins", "emit_effective_policy"),
    );
    let mixed_priority_map = root.join(mixed_priority_map_path);
    let mixed_priority_sources = vec![root.join(crm_parquet_path), root.join(support_orc_path)];
    let mixed_priority_summary =
        cove_map::conversion_summary_from_paths(&mixed_priority_map, &mixed_priority_sources)
            .unwrap();
    let mixed_priority_report = mixed_priority_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    let mixed_priority_projected =
        cove_map::projected_rows_from_paths(&mixed_priority_map, &mixed_priority_sources).unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_source_priority_mixed_format_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.8", "§70.10", "§70.14", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": mixed_priority_map_path,
            "sources": [crm_parquet_path, support_orc_path],
            "expected_conversion": {
                "mapping_id": mixed_priority_report["mapping_id"],
                "mapping_version": mixed_priority_report["mapping_version"],
                "source_count": mixed_priority_report["source_count"],
                "row_count": mixed_priority_report["row_count"],
                "object_count": mixed_priority_report["object_count"],
                "property_value_count": mixed_priority_report["property_value_count"],
                "governance": mixed_priority_report["governance"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": mixed_priority_summary["materialized_row_count"],
                "evidence_entry_count": mixed_priority_summary["evidence_entry_count"],
                "assertion_count": mixed_priority_summary["assertion_count"],
            },
            "expect_cove_o_valid": true,
            "expect_semantic_map_optional": true,
        })),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_source_priority_mixed_format_project_case.json",
            "cove_map_project_case",
            "accept",
            None,
            &["§70.8", "§70.10", "§70.14", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": mixed_priority_map_path,
            "sources": [crm_parquet_path, support_orc_path],
            "expected_projection": {
                "format": mixed_priority_projected["format"],
                "mapping_id": mixed_priority_projected["mapping_id"],
                "mapping_version": mixed_priority_projected["mapping_version"],
            },
            "expected_projection_outputs": [
                {"format": "arrow", "projection_id": "person_projection"},
                {"format": "cove-t", "projection_id": "person_projection"}
            ],
            "expect_persisted_projection_rows": true,
            "expected_projected_rows": mixed_priority_projected["rows"],
        })),
    );

    let conflict_map_path = "accept/cove_map_property_conflict.covemap";
    write_auxiliary_file(
        root,
        conflict_map_path,
        &cove_map_conflict_file("reject_conflict", "emit_effective_policy"),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_property_conflict_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.8", "§72.8", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": conflict_map_path,
            "sources": [crm_path, support_path],
        })),
    );

    let alias_conflict_crm_path = "accept/company_alias_conflict_crm.csv";
    let alias_conflict_support_path = "accept/company_alias_conflict_support.csv";
    write_auxiliary_file(
        root,
        alias_conflict_crm_path,
        b"company_name,display_name\nAlpha Ltd,CRM Alpha\n",
    );
    write_auxiliary_file(
        root,
        alias_conflict_support_path,
        b"company_name,display_name\nAlpha Limited,Support Alpha\n",
    );

    let alias_conflict_reject_map_path = "accept/cove_map_alias_property_conflict.covemap";
    write_auxiliary_file(
        root,
        alias_conflict_reject_map_path,
        &cove_map_alias_property_conflict_file("reject_conflict"),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_alias_property_conflict_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.3", "§70.8", "§72.8", "§73.6", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_conflict_reject_map_path,
            "sources": [alias_conflict_crm_path, alias_conflict_support_path],
        })),
    );

    let alias_priority_map_path = "accept/cove_map_alias_property_source_priority.covemap";
    write_fixture(
        root,
        entries,
        fixture(
            alias_priority_map_path,
            "covemap",
            "accept",
            None,
            &["§70.3", "§70.8", "§70.14", "§72.8", "§73.6"],
        ),
        cove_map_alias_property_conflict_file("source_priority_wins"),
    );
    let alias_priority_summary = cove_map::conversion_summary_from_paths(
        &root.join(alias_priority_map_path),
        &[
            root.join(alias_conflict_crm_path),
            root.join(alias_conflict_support_path),
        ],
    )
    .unwrap();
    let alias_priority_report = alias_priority_summary
        .get("report")
        .cloned()
        .unwrap_or(Value::Null);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_alias_property_source_priority_case.json",
            "cove_map_convert_case",
            "accept",
            None,
            &["§70.3", "§70.8", "§70.14", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": alias_priority_map_path,
            "sources": [alias_conflict_crm_path, alias_conflict_support_path],
            "expected_conversion": {
                "mapping_id": alias_priority_report["mapping_id"],
                "mapping_version": alias_priority_report["mapping_version"],
                "object_count": alias_priority_report["object_count"],
                "property_value_count": alias_priority_report["property_value_count"],
                "resolver_hit_count": alias_priority_report["resolver_hit_count"],
                "governance": alias_priority_report["governance"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": alias_priority_summary["materialized_row_count"],
                "evidence_entry_count": alias_priority_summary["evidence_entry_count"],
                "assertion_count": alias_priority_summary["assertion_count"],
            },
            "expected_evidence_entries": [{
                "contains": {
                    "rule_id": "property_conflict_resolution",
                    "property_name": "display_name",
                    "suppressed": true,
                    "suppressed_reason": "source_priority_wins",
                    "suppressed_value": "CRM Alpha"
                }
            }],
            "expect_cove_o_valid": true,
        })),
    );

    let governance_reject_map_path = "accept/cove_map_mixed_governance_reject.covemap";
    write_auxiliary_file(
        root,
        governance_reject_map_path,
        &cove_map_conflict_file("source_priority_wins", "reject_on_mixed_sensitivity"),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_mixed_governance_case.json",
            "cove_map_convert_case",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70.14", "§72.8", "§76"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": governance_reject_map_path,
            "sources": [crm_path, support_path],
        })),
    );
}

fn cove_map_execution_file() -> Vec<u8> {
    cove_map_file_with_sections([0x51; 16], cove_map_execution_sections())
}

fn cove_map_file_with_sections(file_id: [u8; 16], sections: Vec<CovemapSection>) -> Vec<u8> {
    let mut header = CovemapHeaderV1::new(file_id, 1_700_000_000_000_000);
    header.required_features = FEATURE_SEMANTIC_MAP;
    CovemapFile {
        header,
        mapping_version: "2026.05".into(),
        sections,
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

fn cove_map_execution_sections() -> Vec<CovemapSection> {
    vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "2026.05",
                "sources": [{
                    "source_id": "people",
                    "row_identity_rules": ["person_by_id", "team_by_id"]
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "2026.05",
                "functions": [{
                    "function_id": "identity",
                    "version": "1.0.0",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "2026.05",
                "identity_rules": [
                    {
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "person_id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    },
                    {
                        "rule_id": "team_by_id",
                        "object_type": "Team",
                        "semantic_role": "group",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "team_id",
                            "source_column": "team_id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }
                ],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "2026.05",
                "rules": [
                    {
                        "rule_id": "upsert_person",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "association", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": ["person_name_assertion", "member_of_assertion"],
                        "association_endpoints": ["team_by_id"],
                        "property_bindings": [{
                            "assertion_id": "person_name_assertion",
                            "property_id": "person_name",
                            "property_name": "name",
                            "source_column": "person_name",
                            "logical_type": "utf8",
                            "nullable": false,
                            "missing_policy": "reject"
                        }],
                        "association_bindings": [{
                            "assertion_id": "member_of_assertion",
                            "association_type": "member_of",
                            "target_identity_rule_id": "team_by_id",
                            "source_endpoint_expression": "source.goid",
                            "target_endpoint_expression": "identity(team_by_id)",
                            "source_role": "member",
                            "target_role": "team",
                            "valid_from_expression": "source.valid_from",
                            "valid_to_expression": "source.valid_to",
                            "cardinality_policy": "many_to_one",
                            "missing_policy": "reject"
                        }]
                    },
                    {
                        "rule_id": "upsert_team",
                        "source_id": "people",
                        "identity_rule_id": "team_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": ["team_name_assertion"],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "team_name_assertion",
                            "property_id": "team_name",
                            "property_name": "team_name",
                            "source_column": "team_name",
                            "logical_type": "utf8",
                            "nullable": false,
                            "missing_policy": "reject"
                        }]
                    }
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "2026.05",
                "projections": [
                    {
                        "projection_id": "person_projection",
                        "output_table": "people_projection",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "aggregate",
                        "columns": [
                            {"name": "person_goid", "value": "object.goid", "logical_type": "uuid"},
                            {"name": "name", "value": "name", "logical_type": "utf8"},
                            {"name": "membership_count", "value": "count(association(member_of))", "logical_type": "uint64"}
                        ],
                        "output_modes": ["json", "arrow", "cove-t", "cove-o"]
                    },
                    {
                        "projection_id": "membership_projection",
                        "output_table": "membership_projection",
                        "row_grain": "one_row_per_association",
                        "anchor": {"association_type": "member_of"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "explode",
                        "columns": [
                            {"name": "association_goid", "value": "association.goid", "logical_type": "uuid"},
                            {"name": "source_goid", "value": "association.source_goid", "logical_type": "uuid"},
                            {"name": "target_goid", "value": "association.target_goid", "logical_type": "uuid"},
                            {"name": "source_role", "value": "association.source_role", "logical_type": "utf8"},
                            {"name": "target_role", "value": "association.target_role", "logical_type": "utf8"},
                            {"name": "valid_from", "value": "association.valid_from", "logical_type": "json"},
                            {"name": "valid_to", "value": "association.valid_to", "logical_type": "json"},
                            {"name": "cardinality_policy", "value": "association.cardinality_policy", "logical_type": "utf8"}
                        ],
                        "output_modes": ["json", "cove-o"]
                    }
                ]
            }),
        ),
    ]
}

fn cove_map_resolution_catalog_file(bad_digest: bool) -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections.push(covemap_section(
        SectionKind::MapResolutionCatalog,
        resolution_catalog_value(bad_digest),
    ));
    cove_map_file_with_sections([0x6d; 16], sections)
}

fn cove_map_resolution_alias_file() -> Vec<u8> {
    cove_map_resolution_alias_policy_file(
        0x6e,
        &["company_aliases"],
        company_resolution_catalog_value(),
    )
}

fn cove_map_resolution_alias_policy_file(
    file_id_byte: u8,
    source_ids: &[&str],
    resolution_catalog: Value,
) -> Vec<u8> {
    let sources = source_ids
        .iter()
        .map(|source_id| {
            json!({
                "source_id": source_id,
                "row_identity_rules": ["company_by_resolved_name"]
            })
        })
        .collect::<Vec<_>>();
    let row_rules = source_ids
        .iter()
        .enumerate()
        .map(|(index, source_id)| {
            let rule_id = if source_ids.len() == 1 {
                "company_alias_row".to_string()
            } else {
                format!("company_alias_row_{}", index + 1)
            };
            json!({
                "rule_id": rule_id,
                "source_id": source_id,
                "identity_rule_id": "company_by_resolved_name",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": ["identity"],
                "output_assertion_ids": [],
                "association_endpoints": []
            })
        })
        .collect::<Vec<_>>();
    let sections = vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "sources": sources
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "functions": [
                    {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "identity_rules": [{
                    "rule_id": "company_by_resolved_name",
                    "object_type": "Company",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "auto_merge": true,
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "company",
                        "source_column": "company_name",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared",
                        "resolution": {
                            "resolver_id": "uk_company_name_resolver"
                        }
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "rules": row_rules
            }),
        ),
        covemap_section(SectionKind::MapResolutionCatalog, resolution_catalog),
    ];
    cove_map_file_with_sections([file_id_byte; 16], sections)
}

fn cove_map_resolution_projection_file(
    file_id_byte: u8,
    source_id: &str,
    resolution_catalog: Value,
    include_parent_join_key: bool,
    projection_columns: Value,
) -> Vec<u8> {
    let mut join_keys = vec![json!({
        "role_id": "company",
        "source_column": "company_name",
        "logical_type": "utf8",
        "canonicalization": "identity",
        "null_policy": "reject",
        "ordering": "declared",
        "resolution": {
            "resolver_id": "uk_company_name_resolver"
        }
    })];
    if include_parent_join_key {
        join_keys.push(json!({
            "role_id": "parent_company",
            "source_column": "parent_company_name",
            "logical_type": "utf8",
            "canonicalization": "identity",
            "null_policy": "reject",
            "ordering": "declared",
            "resolution": {
                "resolver_id": "uk_company_name_resolver"
            }
        }));
    }
    let sections = vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "sources": [{
                    "source_id": source_id,
                    "row_identity_rules": ["company_by_resolved_name"]
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "functions": [
                    {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "identity_rules": [{
                    "rule_id": "company_by_resolved_name",
                    "object_type": "Company",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "auto_merge": true,
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": join_keys
                }],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "rules": [{
                    "rule_id": "company_resolution_project_row",
                    "source_id": source_id,
                    "identity_rule_id": "company_by_resolved_name",
                    "row_semantics_kind": "Object",
                    "assertion_kinds": ["object", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": [],
                    "association_endpoints": []
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "2026.06",
                "projections": [{
                    "projection_id": "company_resolution_projection",
                    "output_table": "company_resolution",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Company"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": projection_columns,
                    "output_modes": ["json", "cove-o"]
                }]
            }),
        ),
        covemap_section(SectionKind::MapResolutionCatalog, resolution_catalog),
    ];
    cove_map_file_with_sections([file_id_byte; 16], sections)
}

fn cove_map_candidate_rules_file(file_id_byte: u8, max_pairs_per_block: u64) -> Vec<u8> {
    cove_map_candidate_rules_file_for_source(
        file_id_byte,
        max_pairs_per_block,
        "company_candidates",
    )
}

fn cove_map_candidate_rules_file_for_source(
    file_id_byte: u8,
    max_pairs_per_block: u64,
    source_id: &str,
) -> Vec<u8> {
    let sections = vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "company-candidate-map",
                "mapping_version": "2026.06",
                "sources": [{
                    "source_id": source_id,
                    "row_identity_rules": ["company_by_raw_name"]
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "company-candidate-map",
                "mapping_version": "2026.06",
                "functions": [
                    {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "company-candidate-map",
                "mapping_version": "2026.06",
                "identity_rules": [{
                    "rule_id": "company_by_raw_name",
                    "object_type": "Company",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "company",
                        "source_column": "company_name",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "company-candidate-map",
                "mapping_version": "2026.06",
                "rules": [{
                    "rule_id": "company_candidate_row",
                    "source_id": source_id,
                    "identity_rule_id": "company_by_raw_name",
                    "row_semantics_kind": "Object",
                    "assertion_kinds": ["object", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": [],
                    "association_endpoints": []
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapResolutionCatalog,
            company_candidate_resolution_catalog_value(source_id, max_pairs_per_block),
        ),
    ];
    cove_map_file_with_sections([file_id_byte; 16], sections)
}

fn cove_map_reviewed_equivalence_file(
    file_id_byte: u8,
    allow_reviewed_equivalence: bool,
    include_do_not_merge_conflict: bool,
) -> Vec<u8> {
    let mut reviewed_decisions = vec![json!({
        "decision_id": "review:crm-support",
        "decision": "same_object",
        "confidence_class": "reviewed_authoritative",
        "reviewed_by": "mapping-author",
        "reviewed_at": "2026-06-25T00:00:00Z",
        "left": {
            "kind": "identity_alias",
            "object_type": "Person",
            "identity_alias": "reviewed_crm:0"
        },
        "right": {
            "kind": "identity_alias",
            "object_type": "Person",
            "identity_alias": "reviewed_support:0"
        }
    })];
    if include_do_not_merge_conflict {
        reviewed_decisions.push(json!({
            "decision_id": "review:crm-support-do-not-merge",
            "decision": "do_not_merge",
            "confidence_class": "reviewed_authoritative",
            "reviewed_by": "mapping-author",
            "reviewed_at": "2026-06-25T00:00:00Z",
            "left": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_crm:0"
            },
            "right": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_support:0"
            }
        }));
    }

    cove_map_file_with_sections(
        [file_id_byte; 16],
        reviewed_people_sections(
            &["reviewed_crm", "reviewed_support"],
            allow_reviewed_equivalence,
            reviewed_decisions,
        ),
    )
}

fn cove_map_reviewed_transitive_equivalence_file(
    file_id_byte: u8,
    include_do_not_merge_conflict: bool,
) -> Vec<u8> {
    let mut reviewed_decisions = vec![
        json!({
            "decision_id": "review:crm-support",
            "decision": "same_object",
            "confidence_class": "reviewed_authoritative",
            "reviewed_by": "mapping-author",
            "reviewed_at": "2026-06-25T00:00:00Z",
            "left": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_crm:0"
            },
            "right": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_support:0"
            }
        }),
        json!({
            "decision_id": "review:support-ops",
            "decision": "same_object",
            "confidence_class": "reviewed_authoritative",
            "reviewed_by": "mapping-author",
            "reviewed_at": "2026-06-25T00:00:00Z",
            "left": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_support:0"
            },
            "right": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_ops:0"
            }
        }),
    ];
    if include_do_not_merge_conflict {
        reviewed_decisions.push(json!({
            "decision_id": "review:crm-ops-do-not-merge",
            "decision": "do_not_merge",
            "confidence_class": "reviewed_authoritative",
            "reviewed_by": "mapping-author",
            "reviewed_at": "2026-06-25T00:00:00Z",
            "left": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_crm:0"
            },
            "right": {
                "kind": "identity_alias",
                "object_type": "Person",
                "identity_alias": "reviewed_ops:0"
            }
        }));
    }

    cove_map_file_with_sections(
        [file_id_byte; 16],
        reviewed_people_sections(
            &["reviewed_crm", "reviewed_support", "reviewed_ops"],
            true,
            reviewed_decisions,
        ),
    )
}

fn reviewed_cross_rule_source_bytes() -> &'static [u8] {
    b"id,email\n1,ada@example.test\n"
}

fn cove_map_reviewed_cross_rule_anchor_file(file_id_byte: u8, source_bytes: &[u8]) -> Vec<u8> {
    let source_id = "reviewed_cross_rule";
    let source_snapshot_digest = sha256_digest_string(source_bytes);
    let source_schema_fingerprint = reviewed_cross_rule_source_schema_fingerprint();
    let row_schema_fingerprint = reviewed_cross_rule_row_schema_fingerprint();
    let reviewed_decisions = vec![json!({
        "decision_id": "review:person-id-email",
        "decision": "same_object",
        "confidence_class": "reviewed_authoritative",
        "reviewed_by": "mapping-author",
        "reviewed_at": "2026-06-25T00:00:00Z",
        "left": {
            "kind": "source_row",
            "object_type": "Person",
            "identity_rule_id": "person_by_id",
            "source_id": source_id,
            "source_row_identity": "reviewed_cross_rule:0",
            "source_snapshot_digest": source_snapshot_digest,
            "schema_fingerprint": row_schema_fingerprint
        },
        "right": {
            "kind": "source_row",
            "object_type": "Person",
            "identity_rule_id": "person_by_email",
            "source_id": source_id,
            "source_row_identity": "reviewed_cross_rule:0",
            "source_snapshot_digest": source_snapshot_digest,
            "schema_fingerprint": row_schema_fingerprint
        },
        "canonical_anchor": {
            "kind": "resolved_join_key",
            "object_type": "Person",
            "identity_rule_id": "person_by_id",
            "components": [{
                "role_id": "person_id",
                "logical_type": "utf8",
                "resolved_value": "1"
            }]
        }
    })];

    let sections = vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "reviewed-cross-rule-map",
                "mapping_version": "2026.06",
                "sources": [{
                    "source_id": source_id,
                    "schema_fingerprint": source_schema_fingerprint,
                    "snapshot_digest": source_snapshot_digest,
                    "row_identity_rules": ["person_by_id", "person_by_email"],
                    "replay_claimed": true
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "reviewed-cross-rule-map",
                "mapping_version": "2026.06",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "reviewed-cross-rule-map",
                "mapping_version": "2026.06",
                "identity_rules": [
                    {
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "allow_reviewed_equivalence": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    },
                    {
                        "rule_id": "person_by_email",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "allow_reviewed_equivalence": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "email",
                            "source_column": "email",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }
                ],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "reviewed-cross-rule-map",
                "mapping_version": "2026.06",
                "rules": [
                    {
                        "rule_id": "person_id_row",
                        "source_id": source_id,
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    },
                    {
                        "rule_id": "person_email_row",
                        "source_id": source_id,
                        "identity_rule_id": "person_by_email",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapResolutionCatalog,
            json!({
                "mapping_id": "reviewed-cross-rule-map",
                "mapping_version": "2026.06",
                "normalization_pipelines": [],
                "resolvers": [],
                "match_rules": [],
                "reviewed_decisions": reviewed_decisions
            }),
        ),
    ];
    cove_map_file_with_sections([file_id_byte; 16], sections)
}

fn reviewed_cross_rule_source_schema_fingerprint() -> String {
    format!(
        "cove-map-schema-v1:{}",
        sha256_hex_string(b"csv\nemail:string|id:string")
    )
}

fn reviewed_cross_rule_row_schema_fingerprint() -> String {
    sha256_hex_string(b"email:utf8|id:utf8")
}

fn reviewed_people_sections(
    source_ids: &[&str],
    allow_reviewed_equivalence: bool,
    reviewed_decisions: Vec<Value>,
) -> Vec<CovemapSection> {
    let sources = source_ids
        .iter()
        .map(|source_id| {
            json!({
                "source_id": source_id,
                "row_identity_rules": ["person_by_id"]
            })
        })
        .collect::<Vec<_>>();
    let rules = source_ids
        .iter()
        .map(|source_id| {
            json!({
                "rule_id": format!("{source_id}_person"),
                "source_id": source_id,
                "identity_rule_id": "person_by_id",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": ["identity"],
                "output_assertion_ids": [],
                "association_endpoints": []
            })
        })
        .collect::<Vec<_>>();
    vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "reviewed-people-map",
                "mapping_version": "2026.06",
                "sources": sources
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "reviewed-people-map",
                "mapping_version": "2026.06",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "reviewed-people-map",
                "mapping_version": "2026.06",
                "identity_rules": [{
                    "rule_id": "person_by_id",
                    "object_type": "Person",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "allow_reviewed_equivalence": allow_reviewed_equivalence,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "person_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "reviewed-people-map",
                "mapping_version": "2026.06",
                "rules": rules
            }),
        ),
        covemap_section(
            SectionKind::MapResolutionCatalog,
            json!({
                "mapping_id": "reviewed-people-map",
                "mapping_version": "2026.06",
                "normalization_pipelines": [],
                "resolvers": [],
                "match_rules": [],
                "reviewed_decisions": reviewed_decisions
            }),
        ),
    ]
}

fn cove_map_reviewed_source_row_missing_field_file(
    file_id_byte: u8,
    missing_field: &str,
) -> Vec<u8> {
    let mut left = json!({
        "kind": "source_row",
        "object_type": "Person",
        "identity_rule_id": "person_by_id",
        "source_id": "reviewed_crm_bound",
        "source_row_identity": "reviewed_crm_bound:0",
        "source_snapshot_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "schema_fingerprint": "schema:reviewed-crm:v1"
    });
    left.as_object_mut().unwrap().remove(missing_field);
    let right = json!({
        "kind": "source_row",
        "object_type": "Person",
        "identity_rule_id": "person_by_id",
        "source_id": "reviewed_support_bound",
        "source_row_identity": "reviewed_support_bound:0",
        "source_snapshot_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        "schema_fingerprint": "schema:reviewed-support:v1"
    });
    let reviewed_decisions = vec![json!({
        "decision_id": "review:source-row-bound",
        "decision": "same_object",
        "confidence_class": "reviewed_authoritative",
        "reviewed_by": "mapping-author",
        "reviewed_at": "2026-06-25T00:00:00Z",
        "left": left,
        "right": right
    })];
    cove_map_file_with_sections(
        [file_id_byte; 16],
        reviewed_source_row_sections(reviewed_decisions),
    )
}

fn reviewed_source_row_sections(reviewed_decisions: Vec<Value>) -> Vec<CovemapSection> {
    vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "reviewed-source-row-map",
                "mapping_version": "2026.06",
                "sources": [
                    {
                        "source_id": "reviewed_crm_bound",
                        "schema_fingerprint": "schema:reviewed-crm:v1",
                        "snapshot_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                        "row_identity_rules": ["person_by_id"]
                    },
                    {
                        "source_id": "reviewed_support_bound",
                        "schema_fingerprint": "schema:reviewed-support:v1",
                        "snapshot_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                        "row_identity_rules": ["person_by_id"]
                    }
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "reviewed-source-row-map",
                "mapping_version": "2026.06",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "reviewed-source-row-map",
                "mapping_version": "2026.06",
                "identity_rules": [{
                    "rule_id": "person_by_id",
                    "object_type": "Person",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "allow_reviewed_equivalence": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "person_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "reviewed-source-row-map",
                "mapping_version": "2026.06",
                "rules": [
                    {
                        "rule_id": "reviewed_crm_bound_person",
                        "source_id": "reviewed_crm_bound",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    },
                    {
                        "rule_id": "reviewed_support_bound_person",
                        "source_id": "reviewed_support_bound",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapResolutionCatalog,
            json!({
                "mapping_id": "reviewed-source-row-map",
                "mapping_version": "2026.06",
                "normalization_pipelines": [],
                "resolvers": [],
                "match_rules": [],
                "reviewed_decisions": reviewed_decisions
            }),
        ),
    ]
}

fn resolution_catalog_value(bad_digest: bool) -> Value {
    let pipeline_input = json!({
        "pipeline_id": "person_name.v1",
        "functions": [{
            "function_id": "identity",
            "version": "1.0.0"
        }],
        "tables": []
    });
    let pipeline_digest = sha256_digest_string(&canonical_json(&pipeline_input));

    let alias_catalog_input = json!({
        "alias_catalog_id": "person_aliases",
        "entries": [{
            "alias_entry_id": "person:alice",
            "canonical_key": "person:alice",
            "canonical_label": "Alice",
            "aliases": ["Alice", "Alice A."]
        }]
    });
    let mut catalog_digest = sha256_digest_string(&canonical_json(&alias_catalog_input));
    if bad_digest {
        catalog_digest =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string();
    }

    let resolver_digest_input = json!({
        "resolver_id": "person_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Person",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "person_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": "candidate_only",
        "miss_confidence_class": null,
        "ambiguous_policy": "reject_auto_merge",
        "catalog_digest": catalog_digest,
        "evidence_policy": "retain_raw",
    });
    let resolver_digest = sha256_digest_string(&canonical_json(&resolver_digest_input));

    json!({
        "mapping_id": "people-map",
        "mapping_version": "2026.05",
        "normalization_pipelines": [{
            "pipeline_id": "person_name.v1",
            "functions": [{
                "function_id": "identity",
                "version": "1.0.0"
            }],
            "tables": []
        }],
        "resolvers": [{
            "resolver_id": "person_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Person",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "person_name.v1",
            "on_hit": "canonical_key",
            "on_miss": "candidate_only",
            "ambiguous_policy": "reject_auto_merge",
            "catalog_digest": catalog_digest,
            "pipeline_digest": pipeline_digest,
            "resolver_digest": resolver_digest,
            "alias_catalog": {
                "alias_catalog_id": "person_aliases",
                "entries": [{
                    "alias_entry_id": "person:alice",
                    "canonical_key": "person:alice",
                    "canonical_label": "Alice",
                    "aliases": ["Alice A.", "Alice"]
                }]
            }
        }],
        "match_rules": [],
        "reviewed_decisions": []
    })
}

fn company_candidate_resolution_catalog_value(source_id: &str, max_pairs_per_block: u64) -> Value {
    json!({
        "mapping_id": "company-candidate-map",
        "mapping_version": "2026.06",
        "normalization_pipelines": [{
            "pipeline_id": "company_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        }],
        "resolvers": [],
        "match_rules": [{
            "match_rule_id": "company_name_similarity",
            "object_type": "Company",
            "inputs": [{
                "source_id": source_id,
                "column": "company_name"
            }],
            "blocking": {
                "kind": "normalized_prefix",
                "length": 4
            },
            "normalization_pipeline_id": "company_name.v1",
            "scoring": {
                "kind": "token_jaccard",
                "candidate_threshold": 0.3,
                "merge_behavior": "never",
                "score_scale": 1000000,
                "rounding": "floor"
            },
            "limits": {
                "max_pairs_per_block": max_pairs_per_block,
                "max_pairs_total": 100,
                "on_limit": "fail_closed"
            },
            "output": {
                "assertion_kinds": ["candidate_match", "evidence"]
            }
        }],
        "reviewed_decisions": []
    })
}

fn company_resolution_catalog_value() -> Value {
    company_resolution_catalog_value_with_policy("candidate_only", None, "reject_auto_merge", false)
}

fn company_resolution_catalog_value_with_reordered_alias_entries() -> Value {
    let mut catalog =
        company_resolution_catalog_value_with_normalized_alias_collision("candidate_only", true);
    catalog["resolvers"][0]["alias_catalog"]["entries"]
        .as_array_mut()
        .unwrap()
        .reverse();
    catalog
}

fn company_alpha_resolution_catalog_value() -> Value {
    let pipeline_input = json!({
        "pipeline_id": "company_name.v1",
        "functions": [
            {"function_id": "unicode_nfkc", "version": "1"},
            {"function_id": "unicode_casefold", "version": "1"},
            {"function_id": "trim", "version": "1"},
            {"function_id": "collapse_whitespace", "version": "1"}
        ],
        "tables": []
    });
    let pipeline_digest = sha256_digest_string(&canonical_json(&pipeline_input));

    let alias_entry = json!({
        "alias_entry_id": "company:alpha",
        "canonical_key": "uk-company:alpha",
        "canonical_label": "Alpha",
        "aliases": ["Alpha Limited", "Alpha Ltd"]
    });
    let alias_catalog_input = json!({
        "alias_catalog_id": "company_aliases",
        "entries": [alias_entry]
    });
    let catalog_digest = sha256_digest_string(&canonical_json(&alias_catalog_input));
    let resolver_digest_input = json!({
        "resolver_id": "uk_company_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Company",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "company_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": "candidate_only",
        "miss_confidence_class": null,
        "ambiguous_policy": "reject_auto_merge",
        "catalog_digest": catalog_digest,
        "evidence_policy": "retain_raw",
    });
    let resolver_digest = sha256_digest_string(&canonical_json(&resolver_digest_input));

    json!({
        "mapping_id": "company-alias-conflict-map",
        "mapping_version": "2026.06",
        "normalization_pipelines": [{
            "pipeline_id": "company_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        }],
        "resolvers": [{
            "resolver_id": "uk_company_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Company",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "company_name.v1",
            "on_hit": "canonical_key",
            "on_miss": "candidate_only",
            "ambiguous_policy": "reject_auto_merge",
            "catalog_digest": catalog_digest,
            "pipeline_digest": pipeline_digest,
            "resolver_digest": resolver_digest,
            "evidence_policy": "retain_raw",
            "alias_catalog": {
                "alias_catalog_id": "company_aliases",
                "entries": [{
                    "alias_entry_id": "company:alpha",
                    "canonical_key": "uk-company:alpha",
                    "canonical_label": "Alpha",
                    "aliases": ["Alpha Limited", "Alpha Ltd"]
                }]
            }
        }],
        "match_rules": [],
        "reviewed_decisions": []
    })
}

fn company_resolution_catalog_value_with_stale_pipeline_version() -> Value {
    let mut catalog = company_resolution_catalog_value();
    catalog["normalization_pipelines"][0]["functions"][0]["version"] = json!("2");
    catalog
}

fn company_resolution_catalog_value_with_stale_suffix_table_digest() -> Value {
    let mut catalog = company_resolution_catalog_value();
    catalog["normalization_pipelines"][0]["functions"][0]["suffix_table_digest"] =
        json!("sha256:1111111111111111111111111111111111111111111111111111111111111111");
    catalog
}

fn company_resolution_catalog_value_with_stale_resolver_digest() -> Value {
    let mut catalog = company_resolution_catalog_value();
    catalog["resolvers"][0]["resolver_digest"] =
        json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
    catalog
}

fn company_resolution_catalog_value_with_normalized_alias_collision(
    ambiguous_policy: &str,
    mark_entries_ambiguous: bool,
) -> Value {
    let pipeline_input = json!({
        "pipeline_id": "company_name.v1",
        "functions": [
            {"function_id": "unicode_nfkc", "version": "1"},
            {"function_id": "unicode_casefold", "version": "1"},
            {"function_id": "trim", "version": "1"},
            {"function_id": "collapse_whitespace", "version": "1"}
        ],
        "tables": []
    });
    let pipeline_digest = sha256_digest_string(&canonical_json(&pipeline_input));

    let mut tesco_entry = json!({
        "alias_entry_id": "company:tesco",
        "canonical_key": "uk-company:tesco",
        "canonical_label": "Tesco",
        "aliases": ["Tesco PLC"]
    });
    let mut tesco_retail_entry = json!({
        "alias_entry_id": "company:tesco-retail",
        "canonical_key": "uk-company:tesco-retail",
        "canonical_label": "Tesco Retail",
        "aliases": ["tesco plc"]
    });
    if mark_entries_ambiguous {
        tesco_entry["ambiguous"] = json!(true);
        tesco_retail_entry["ambiguous"] = json!(true);
    }
    let alias_catalog_input = json!({
        "alias_catalog_id": "company_aliases",
        "entries": [tesco_entry, tesco_retail_entry]
    });
    let catalog_digest = sha256_digest_string(&canonical_json(&alias_catalog_input));
    let resolver_digest_input = json!({
        "resolver_id": "uk_company_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Company",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "company_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": "candidate_only",
        "miss_confidence_class": null,
        "ambiguous_policy": ambiguous_policy,
        "catalog_digest": catalog_digest,
        "evidence_policy": "retain_raw",
    });
    let resolver_digest = sha256_digest_string(&canonical_json(&resolver_digest_input));

    json!({
        "mapping_id": "company-map",
        "mapping_version": "2026.06",
        "normalization_pipelines": [{
            "pipeline_id": "company_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        }],
        "resolvers": [{
            "resolver_id": "uk_company_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Company",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "company_name.v1",
            "on_hit": "canonical_key",
            "on_miss": "candidate_only",
            "ambiguous_policy": ambiguous_policy,
            "catalog_digest": catalog_digest,
            "pipeline_digest": pipeline_digest,
            "resolver_digest": resolver_digest,
            "evidence_policy": "retain_raw",
            "alias_catalog": {
                "alias_catalog_id": "company_aliases",
                "entries": [{
                    "alias_entry_id": "company:tesco",
                    "canonical_key": "uk-company:tesco",
                    "canonical_label": "Tesco",
                    "aliases": ["Tesco PLC"],
                    "ambiguous": mark_entries_ambiguous
                }, {
                    "alias_entry_id": "company:tesco-retail",
                    "canonical_key": "uk-company:tesco-retail",
                    "canonical_label": "Tesco Retail",
                    "aliases": ["tesco plc"],
                    "ambiguous": mark_entries_ambiguous
                }]
            }
        }],
        "match_rules": [],
        "reviewed_decisions": []
    })
}

fn team_resolution_catalog_value() -> Value {
    let pipeline_input = json!({
        "pipeline_id": "team_name.v1",
        "functions": [
            {"function_id": "unicode_nfkc", "version": "1"},
            {"function_id": "unicode_casefold", "version": "1"},
            {"function_id": "trim", "version": "1"},
            {"function_id": "collapse_whitespace", "version": "1"}
        ],
        "tables": []
    });
    let pipeline_digest = sha256_digest_string(&canonical_json(&pipeline_input));
    let alias_catalog_input = json!({
        "alias_catalog_id": "team_aliases",
        "entries": [{
            "alias_entry_id": "team:alpha",
            "canonical_key": "team:alpha",
            "canonical_label": "Alpha Team",
            "aliases": ["Alpha Team Ltd", "Team Alpha", "alpha team"]
        }]
    });
    let catalog_digest = sha256_digest_string(&canonical_json(&alias_catalog_input));
    let resolver_digest_input = json!({
        "resolver_id": "team_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Team",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "team_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": "reject",
        "miss_confidence_class": null,
        "ambiguous_policy": "reject_auto_merge",
        "catalog_digest": catalog_digest,
        "evidence_policy": "retain_raw",
    });
    let resolver_digest = sha256_digest_string(&canonical_json(&resolver_digest_input));

    json!({
        "mapping_id": "people-map",
        "mapping_version": "2026.06",
        "normalization_pipelines": [{
            "pipeline_id": "team_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        }],
        "resolvers": [{
            "resolver_id": "team_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Team",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "team_name.v1",
            "on_hit": "canonical_key",
            "on_miss": "reject",
            "ambiguous_policy": "reject_auto_merge",
            "catalog_digest": catalog_digest,
            "pipeline_digest": pipeline_digest,
            "resolver_digest": resolver_digest,
            "evidence_policy": "retain_raw",
            "alias_catalog": {
                "alias_catalog_id": "team_aliases",
                "entries": [{
                    "alias_entry_id": "team:alpha",
                    "canonical_key": "team:alpha",
                    "canonical_label": "Alpha Team",
                    "aliases": ["Team Alpha", "Alpha Team Ltd", "alpha team"]
                }]
            }
        }],
        "match_rules": [],
        "reviewed_decisions": []
    })
}

fn company_resolution_catalog_value_with_policy(
    on_miss: &str,
    miss_confidence_class: Option<&str>,
    ambiguous_policy: &str,
    ambiguous_alias: bool,
) -> Value {
    company_resolution_catalog_value_with_policy_and_evidence(
        on_miss,
        miss_confidence_class,
        ambiguous_policy,
        ambiguous_alias,
        "retain_raw",
    )
}

fn company_resolution_catalog_value_with_policy_and_evidence(
    on_miss: &str,
    miss_confidence_class: Option<&str>,
    ambiguous_policy: &str,
    ambiguous_alias: bool,
    evidence_policy: &str,
) -> Value {
    let pipeline_input = json!({
        "pipeline_id": "company_name.v1",
        "functions": [
            {"function_id": "unicode_nfkc", "version": "1"},
            {"function_id": "unicode_casefold", "version": "1"},
            {"function_id": "trim", "version": "1"},
            {"function_id": "collapse_whitespace", "version": "1"}
        ],
        "tables": []
    });
    let pipeline_digest = sha256_digest_string(&canonical_json(&pipeline_input));

    let mut alias_entry = json!({
        "alias_entry_id": "company:tesco",
        "canonical_key": "uk-company:tesco",
        "canonical_label": "Tesco",
        "aliases": ["Tesco", "Tesco PLC", "tesco supermarket"]
    });
    if ambiguous_alias {
        alias_entry["ambiguous"] = json!(true);
    }
    let alias_catalog_input = json!({
        "alias_catalog_id": "company_aliases",
        "entries": [alias_entry]
    });
    let catalog_digest = sha256_digest_string(&canonical_json(&alias_catalog_input));
    let resolver_digest_input = json!({
        "resolver_id": "uk_company_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Company",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "company_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": on_miss,
        "miss_confidence_class": miss_confidence_class,
        "ambiguous_policy": ambiguous_policy,
        "catalog_digest": catalog_digest,
        "evidence_policy": evidence_policy,
    });
    let resolver_digest = sha256_digest_string(&canonical_json(&resolver_digest_input));

    let mut catalog = json!({
        "mapping_id": "company-map",
        "mapping_version": "2026.06",
        "normalization_pipelines": [{
            "pipeline_id": "company_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        }],
        "resolvers": [{
            "resolver_id": "uk_company_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Company",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "company_name.v1",
            "on_hit": "canonical_key",
            "on_miss": on_miss,
            "ambiguous_policy": ambiguous_policy,
            "catalog_digest": catalog_digest,
            "pipeline_digest": pipeline_digest,
            "resolver_digest": resolver_digest,
            "evidence_policy": evidence_policy,
            "alias_catalog": {
                "alias_catalog_id": "company_aliases",
                "entries": [{
                    "alias_entry_id": "company:tesco",
                    "canonical_key": "uk-company:tesco",
                    "canonical_label": "Tesco",
                    "aliases": ["tesco supermarket", "Tesco PLC", "Tesco"]
                }]
            }
        }],
        "match_rules": [],
        "reviewed_decisions": []
    });
    if let Some(confidence_class) = miss_confidence_class {
        catalog["resolvers"][0]["miss_confidence_class"] = json!(confidence_class);
    }
    if ambiguous_alias {
        catalog["resolvers"][0]["alias_catalog"]["entries"][0]["ambiguous"] = json!(true);
    }
    catalog
}

fn cove_map_candidate_identity_file() -> Vec<u8> {
    cove_map_file_with_sections(
        [0x53; 16],
        vec![
            covemap_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "candidate-map",
                    "mapping_version": "2026.05",
                    "sources": [{
                        "source_id": "people",
                        "row_identity_rules": ["person_name_candidate"]
                    }]
                }),
            ),
            covemap_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "candidate-map",
                    "mapping_version": "2026.05",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1.0.0",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            covemap_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "candidate-map",
                    "mapping_version": "2026.05",
                    "identity_rules": [{
                        "rule_id": "person_name_candidate",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "candidate",
                        "candidate_only": true,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_name",
                            "source_column": "person_name",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            covemap_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "candidate-map",
                    "mapping_version": "2026.05",
                    "rules": [{
                        "rule_id": "candidate_person",
                        "source_id": "people",
                        "identity_rule_id": "person_name_candidate",
                        "row_semantics_kind": "EvidenceOnly",
                        "assertion_kinds": ["candidate_match", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }]
                }),
            ),
        ],
    )
}

fn cove_map_association_only_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections[3] = covemap_section(
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "rules": [
                {
                    "rule_id": "person_membership_only",
                    "source_id": "people",
                    "identity_rule_id": "person_by_id",
                    "row_semantics_kind": "AssociationOnly",
                    "assertion_kinds": ["association", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": ["member_of_assertion"],
                    "association_endpoints": ["team_by_id"],
                    "association_bindings": [{
                        "assertion_id": "member_of_assertion",
                        "association_type": "member_of",
                        "target_identity_rule_id": "team_by_id",
                        "source_endpoint_expression": "source.goid",
                        "target_endpoint_expression": "identity(team_by_id)",
                        "source_role": "member",
                        "target_role": "team",
                        "valid_from_expression": "source.valid_from",
                        "valid_to_expression": "source.valid_to",
                        "cardinality_policy": "many_to_one",
                        "missing_policy": "reject"
                    }]
                },
                {
                    "rule_id": "upsert_team",
                    "source_id": "people",
                    "identity_rule_id": "team_by_id",
                    "row_semantics_kind": "Object",
                    "assertion_kinds": ["object", "property", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": ["team_name_assertion"],
                    "association_endpoints": [],
                    "property_bindings": [{
                        "assertion_id": "team_name_assertion",
                        "property_id": "team_name",
                        "property_name": "team_name",
                        "source_column": "team_name",
                        "logical_type": "utf8",
                        "nullable": false,
                        "missing_policy": "reject"
                    }]
                }
            ]
        }),
    );
    cove_map_file_with_sections([0x54; 16], sections)
}

fn cove_map_alias_backed_association_file() -> Vec<u8> {
    cove_map_file_with_sections(
        [0x82; 16],
        vec![
            covemap_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "2026.06",
                    "sources": [
                        {
                            "source_id": "cove_map_alias_memberships",
                            "row_identity_rules": ["person_by_id"]
                        },
                        {
                            "source_id": "cove_map_alias_teams",
                            "row_identity_rules": ["team_by_name"]
                        }
                    ]
                }),
            ),
            covemap_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "2026.06",
                    "functions": [
                        {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                    ]
                }),
            ),
            covemap_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "2026.06",
                    "identity_rules": [
                        {
                            "rule_id": "person_by_id",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "person_id",
                                "source_column": "person_id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        },
                        {
                            "rule_id": "team_by_name",
                            "object_type": "Team",
                            "semantic_role": "organization",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "team",
                                "source_column": "team_name",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared",
                                "resolution": {
                                    "resolver_id": "team_name_resolver"
                                }
                            }]
                        }
                    ],
                    "do_not_merge": []
                }),
            ),
            covemap_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "2026.06",
                    "rules": [
                        {
                            "rule_id": "membership_row",
                            "source_id": "cove_map_alias_memberships",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "association", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["member_of_assertion"],
                            "association_endpoints": [],
                            "association_bindings": [{
                                "assertion_id": "member_of_assertion",
                                "association_type": "member_of",
                                "source_identity_rule_id": "person_by_id",
                                "source_endpoint_expression": "source.goid",
                                "target_identity_rule_id": "team_by_name",
                                "target_endpoint_expression": "identity(team_by_name)",
                                "source_role": "member",
                                "target_role": "team",
                                "valid_from_expression": "source.valid_from",
                                "valid_to_expression": "source.valid_to",
                                "cardinality_policy": "many_to_one",
                                "missing_policy": "reject"
                            }]
                        },
                        {
                            "rule_id": "team_row",
                            "source_id": "cove_map_alias_teams",
                            "identity_rule_id": "team_by_name",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        }
                    ]
                }),
            ),
            covemap_section(
                SectionKind::MapResolutionCatalog,
                team_resolution_catalog_value(),
            ),
        ],
    )
}

fn cove_map_composite_row_semantics_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections[3] = covemap_section(
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "rules": [
                {
                    "rule_id": "upsert_person",
                    "source_id": "people",
                    "identity_rule_id": "person_by_id",
                    "row_semantics_kind": "Composite",
                    "assertion_kinds": ["object", "property", "association", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": ["person_name_assertion", "member_of_assertion"],
                    "association_endpoints": ["team_by_id"],
                    "property_bindings": [{
                        "assertion_id": "person_name_assertion",
                        "property_id": "person_name",
                        "property_name": "name",
                        "source_column": "person_name",
                        "logical_type": "utf8",
                        "nullable": false,
                        "missing_policy": "reject"
                    }],
                    "association_bindings": [{
                        "assertion_id": "member_of_assertion",
                        "association_type": "member_of",
                        "target_identity_rule_id": "team_by_id",
                        "source_endpoint_expression": "source.goid",
                        "target_endpoint_expression": "identity(team_by_id)",
                        "source_role": "member",
                        "target_role": "team",
                        "valid_from_expression": "source.valid_from",
                        "valid_to_expression": "source.valid_to",
                        "cardinality_policy": "many_to_one",
                        "missing_policy": "reject"
                    }]
                },
                {
                    "rule_id": "upsert_team",
                    "source_id": "people",
                    "identity_rule_id": "team_by_id",
                    "row_semantics_kind": "Object",
                    "assertion_kinds": ["object", "property", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": ["team_name_assertion"],
                    "association_endpoints": [],
                    "property_bindings": [{
                        "assertion_id": "team_name_assertion",
                        "property_id": "team_name",
                        "property_name": "team_name",
                        "source_column": "team_name",
                        "logical_type": "utf8",
                        "nullable": false,
                        "missing_policy": "reject"
                    }]
                }
            ]
        }),
    );
    cove_map_file_with_sections([0x55; 16], sections)
}

fn cove_map_tombstone_row_semantics_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections.truncate(4);
    sections[3] = covemap_section(
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "delete_person",
                "source_id": "people",
                "identity_rule_id": "person_by_id",
                "row_semantics_kind": "Tombstone",
                "assertion_kinds": ["object", "tombstone", "evidence"],
                "tombstone_target": "object",
                "function_ids": ["identity"],
                "output_assertion_ids": [],
                "association_endpoints": []
            }]
        }),
    );
    cove_map_file_with_sections([0x56; 16], sections)
}

fn cove_map_invalid_row_semantics_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections[3] = covemap_section(
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "bad_person",
                "source_id": "people",
                "identity_rule_id": "person_by_id",
                "row_semantics_kind": "ProjectionOnly",
                "assertion_kinds": ["object"]
            }]
        }),
    );
    cove_map_file_with_sections([0x57; 16], sections)
}

fn cove_map_association_bad_endpoint_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections[3] = covemap_section(
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "bad_association",
                "source_id": "people",
                "identity_rule_id": "person_by_id",
                "row_semantics_kind": "AssociationOnly",
                "assertion_kinds": ["association", "evidence"],
                "function_ids": ["identity"],
                "output_assertion_ids": ["member_of_assertion"],
                "association_endpoints": ["missing_team_by_id"],
                "association_bindings": [{
                    "assertion_id": "member_of_assertion",
                    "association_type": "member_of",
                    "target_identity_rule_id": "missing_team_by_id",
                    "source_endpoint_expression": "source.goid",
                    "target_endpoint_expression": "identity(missing_team_by_id)",
                    "source_role": "member",
                    "target_role": "team",
                    "cardinality_policy": "many_to_one",
                    "missing_policy": "reject"
                }]
            }]
        }),
    );
    cove_map_file_with_sections([0x58; 16], sections)
}

fn cove_map_projection_missing_policy_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections[4] = covemap_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "projections": [{
                "projection_id": "person_projection",
                "output_table": "people_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "columns": [
                    {"name": "person_goid", "value": "object.goid", "logical_type": "uuid"},
                    {"name": "name", "value": "name", "logical_type": "utf8"}
                ],
                "output_modes": ["json"]
            }]
        }),
    );
    cove_map_file_with_sections([0x59; 16], sections)
}

fn cove_map_projection_undeclared_function_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections[4] = covemap_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "projections": [{
                "projection_id": "person_projection",
                "output_table": "people_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "normalized_type", "value": "lower(object.type)", "logical_type": "utf8"}
                ],
                "output_modes": ["json"]
            }]
        }),
    );
    cove_map_file_with_sections([0x5a; 16], sections)
}

fn cove_map_projection_aggregate_policy_file() -> Vec<u8> {
    let mut sections = cove_map_execution_sections();
    sections[4] = covemap_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "2026.05",
            "projections": [{
                "projection_id": "person_projection",
                "output_table": "people_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "membership_count", "value": "count(association(member_of))", "logical_type": "uint64"}
                ],
                "output_modes": ["json"]
            }]
        }),
    );
    cove_map_file_with_sections([0x5b; 16], sections)
}

fn covemap_section(section_kind: SectionKind, value: Value) -> CovemapSection {
    let payload = map_payload_bytes(covemap_payload_value(section_kind, value));
    CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: section_kind as u32,
            offset: 0,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: 0,
            payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
            required: true,
            reserved: 0,
            checksum: 0,
        },
        payload,
    }
}

fn sha256_digest_string(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex_string(bytes))
}

fn sha256_hex_string(bytes: &[u8]) -> String {
    let digest = compute_digest(DigestAlgorithm::Sha256, bytes).unwrap();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    out
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + nibble - 10) as char,
        _ => unreachable!(),
    }
}

fn canonical_json(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_canonical_json(value, &mut out);
    out
}

fn write_canonical_json(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(value) => {
            out.extend_from_slice(serde_json::to_string(value).unwrap().as_bytes());
        }
        Value::Array(values) => {
            out.push(b'[');
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                write_canonical_json(value, out);
            }
            out.push(b']');
        }
        Value::Object(object) => {
            out.push(b'{');
            let mut keys = object
                .keys()
                .filter(|key| key.as_str() != "non_semantic_metadata")
                .collect::<Vec<_>>();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (idx, key) in keys.iter().enumerate() {
                if idx > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(serde_json::to_string(key).unwrap().as_bytes());
                out.push(b':');
                write_canonical_json(object.get(*key).unwrap(), out);
            }
            out.push(b'}');
        }
    }
}

fn write_cove_map_case_pair(
    root: &Path,
    entries: &mut Vec<Value>,
    map_path: &str,
    source_paths: &[&str],
    convert_case_path: &str,
    project_case_path: &str,
) {
    let map = root.join(map_path);
    let sources = source_paths
        .iter()
        .map(|path| root.join(path))
        .collect::<Vec<_>>();
    let summary = cove_map::conversion_summary_from_paths(&map, &sources).unwrap();
    let report = summary.get("report").cloned().unwrap_or(Value::Null);
    let projected = cove_map::projected_rows_from_paths(&map, &sources).unwrap();

    write_fixture(
        root,
        entries,
        fixture(
            convert_case_path,
            "cove_map_convert_case",
            "accept",
            None,
            &[
                "§61", "§70.2", "§70.3", "§70.5", "§70.6", "§70.9", "§70.10", "§70.12", "§70.13",
                "§72.8", "§73.6",
            ],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": map_path,
            "sources": source_paths,
            "expected_conversion": {
                "mapping_id": report["mapping_id"],
                "mapping_version": report["mapping_version"],
                "source_count": report["source_count"],
                "row_count": report["row_count"],
                "object_count": report["object_count"],
                "association_count": report["association_count"],
                "property_value_count": report["property_value_count"],
            },
            "expected_conversion_summary": {
                "materialized_row_count": summary["materialized_row_count"],
                "evidence_entry_count": summary["evidence_entry_count"],
                "assertion_count": summary["assertion_count"],
            },
            "expect_cove_o_valid": true,
            "expect_semantic_map_optional": true,
            "expect_association_readback_flags": true,
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            project_case_path,
            "cove_map_project_case",
            "accept",
            None,
            &["§70.9", "§70.10", "§72.8", "§73.6"],
        ),
        suite_contract_fixture_bytes(json!({
            "mapping": map_path,
            "sources": source_paths,
            "expected_projection": {
                "format": projected["format"],
                "mapping_id": projected["mapping_id"],
                "mapping_version": projected["mapping_version"],
            },
            "expected_projection_outputs": [
                {"format": "arrow", "projection_id": "person_projection"},
                {"format": "cove-t", "projection_id": "person_projection"}
            ],
            "expect_persisted_projection_rows": true,
            "expected_projected_rows": projected["rows"],
        })),
    );
}

fn cove_map_execution_source_bytes() -> &'static [u8] {
    b"person_id,person_name,team_id,team_name,valid_from,valid_to\np1,Ada,t1,Core,2026-01-01,2026-12-31\np2,Linus,t2,Systems,2026-02-01,2026-12-31\n"
}

fn cove_map_execution_source_batch() -> RecordBatch {
    RecordBatch::try_from_iter(vec![
        (
            "person_id",
            Arc::new(StringArray::from(vec!["p1", "p2"])) as ArrayRef,
        ),
        (
            "person_name",
            Arc::new(StringArray::from(vec!["Ada", "Linus"])) as ArrayRef,
        ),
        (
            "team_id",
            Arc::new(StringArray::from(vec!["t1", "t2"])) as ArrayRef,
        ),
        (
            "team_name",
            Arc::new(StringArray::from(vec!["Core", "Systems"])) as ArrayRef,
        ),
        (
            "valid_from",
            Arc::new(StringArray::from(vec!["2026-01-01", "2026-02-01"])) as ArrayRef,
        ),
        (
            "valid_to",
            Arc::new(StringArray::from(vec!["2026-12-31", "2026-12-31"])) as ArrayRef,
        ),
    ])
    .unwrap()
}

fn cove_map_crm_source_batch() -> RecordBatch {
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(vec!["p1"])) as ArrayRef),
        (
            "name",
            Arc::new(StringArray::from(vec!["CRM Name"])) as ArrayRef,
        ),
    ])
    .unwrap()
}

fn cove_map_support_source_batch() -> RecordBatch {
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(vec!["p1"])) as ArrayRef),
        (
            "name",
            Arc::new(StringArray::from(vec!["Support Name"])) as ArrayRef,
        ),
    ])
    .unwrap()
}

fn write_arrow_ipc(batch: &RecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = IpcFileWriter::try_new(&mut bytes, &batch.schema()).unwrap();
        writer.write(batch).unwrap();
        writer.finish().unwrap();
    }
    bytes
}

fn write_parquet(batch: &RecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut bytes, batch.schema(), None).unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
    }
    bytes
}

fn write_orc(batch: &RecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = OrcWriterBuilder::new(&mut bytes, batch.schema())
            .try_build()
            .unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
    }
    bytes
}

fn cove_map_execution_parquet_source_bytes() -> Vec<u8> {
    write_parquet(&cove_map_execution_source_batch())
}

fn cove_map_execution_orc_source_bytes() -> Vec<u8> {
    write_orc(&cove_map_execution_source_batch())
}

fn cove_map_execution_arrow_source_bytes() -> Vec<u8> {
    write_arrow_ipc(&cove_map_execution_source_batch())
}

fn cove_map_crm_source_bytes() -> &'static [u8] {
    b"id,name\np1,CRM Name\n"
}

fn cove_map_crm_parquet_source_bytes() -> Vec<u8> {
    write_parquet(&cove_map_crm_source_batch())
}

fn cove_map_support_source_bytes() -> &'static [u8] {
    b"id,name\np1,Support Name\n"
}

fn cove_map_support_orc_source_bytes() -> Vec<u8> {
    write_orc(&cove_map_support_source_batch())
}

fn cove_map_conflict_file(conflict_policy: &str, governance_policy: &str) -> Vec<u8> {
    let mut header = CovemapHeaderV1::new([0x52; 16], 1_700_000_000_000_001);
    header.required_features = FEATURE_SEMANTIC_MAP;
    CovemapFile {
        header,
        mapping_version: "2026.05".into(),
        sections: cove_map_conflict_sections(conflict_policy, governance_policy),
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

fn cove_map_projectable_conflict_file(conflict_policy: &str, governance_policy: &str) -> Vec<u8> {
    cove_map_file_with_sections(
        [0x53; 16],
        cove_map_projectable_conflict_sections(conflict_policy, governance_policy),
    )
}

fn cove_map_alias_property_conflict_file(conflict_policy: &str) -> Vec<u8> {
    cove_map_file_with_sections(
        [if conflict_policy == "reject_conflict" {
            0x96
        } else {
            0x97
        }; 16],
        cove_map_alias_property_conflict_sections(conflict_policy),
    )
}

fn cove_map_alias_property_conflict_sections(conflict_policy: &str) -> Vec<CovemapSection> {
    vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "company-alias-conflict-map",
                "mapping_version": "2026.06",
                "governance_reconciliation_policy": "emit_effective_policy",
                "sources": [
                    {
                        "source_id": "company_alias_conflict_crm",
                        "row_identity_rules": ["company_by_resolved_name"],
                        "source_priority": 10,
                        "sensitivity_label": "public",
                        "sensitivity_rank": 1,
                        "access_policy_ids": ["internal"]
                    },
                    {
                        "source_id": "company_alias_conflict_support",
                        "row_identity_rules": ["company_by_resolved_name"],
                        "source_priority": 1,
                        "sensitivity_label": "public",
                        "sensitivity_rank": 1,
                        "access_policy_ids": ["internal"]
                    }
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "company-alias-conflict-map",
                "mapping_version": "2026.06",
                "functions": [
                    {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "company-alias-conflict-map",
                "mapping_version": "2026.06",
                "identity_rules": [{
                    "rule_id": "company_by_resolved_name",
                    "object_type": "Company",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "company",
                        "source_column": "company_name",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared",
                        "resolution": {
                            "resolver_id": "uk_company_name_resolver"
                        }
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "company-alias-conflict-map",
                "mapping_version": "2026.06",
                "rules": [
                    {
                        "rule_id": "company_alias_conflict_crm_row",
                        "source_id": "company_alias_conflict_crm",
                        "identity_rule_id": "company_by_resolved_name",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence", "conflict"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": ["crm_display_name_assertion"],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "crm_display_name_assertion",
                            "property_id": "display_name",
                            "property_name": "display_name",
                            "source_column": "display_name",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": conflict_policy,
                            "missing_policy": "null"
                        }]
                    },
                    {
                        "rule_id": "company_alias_conflict_support_row",
                        "source_id": "company_alias_conflict_support",
                        "identity_rule_id": "company_by_resolved_name",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence", "conflict"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": ["support_display_name_assertion"],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "support_display_name_assertion",
                            "property_id": "display_name",
                            "property_name": "display_name",
                            "source_column": "display_name",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": conflict_policy,
                            "missing_policy": "null"
                        }]
                    }
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapResolutionCatalog,
            company_alpha_resolution_catalog_value(),
        ),
    ]
}

fn cove_map_conflict_sections(
    conflict_policy: &str,
    governance_policy: &str,
) -> Vec<CovemapSection> {
    vec![
        covemap_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-priority-map",
                "mapping_version": "2026.05",
                "governance_reconciliation_policy": governance_policy,
                "sources": [
                    {
                        "source_id": "cove_map_crm",
                        "row_identity_rules": ["person_by_id"],
                        "source_priority": 10,
                        "sensitivity_label": "public",
                        "sensitivity_rank": 1,
                        "access_policy_ids": ["internal"]
                    },
                    {
                        "source_id": "cove_map_support",
                        "row_identity_rules": ["person_by_id"],
                        "source_priority": 1,
                        "sensitivity_label": "restricted",
                        "sensitivity_rank": 5,
                        "access_policy_ids": ["hipaa"]
                    }
                ]
            }),
        ),
        covemap_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-priority-map",
                "mapping_version": "2026.05",
                "functions": [{
                    "function_id": "identity",
                    "version": "1.0.0",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        covemap_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-priority-map",
                "mapping_version": "2026.05",
                "identity_rules": [{
                    "rule_id": "person_by_id",
                    "object_type": "Person",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "person_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        covemap_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-priority-map",
                "mapping_version": "2026.05",
                "rules": [
                    {
                        "rule_id": "crm_person",
                        "source_id": "cove_map_crm",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence", "conflict"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": ["crm_name_assertion"],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "crm_name_assertion",
                            "property_id": "name",
                            "property_name": "name",
                            "source_column": "name",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": conflict_policy,
                            "missing_policy": "null"
                        }]
                    },
                    {
                        "rule_id": "support_person",
                        "source_id": "cove_map_support",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence", "conflict"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": ["support_name_assertion"],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "support_name_assertion",
                            "property_id": "name",
                            "property_name": "name",
                            "source_column": "name",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": conflict_policy,
                            "missing_policy": "null"
                        }]
                    }
                ]
            }),
        ),
    ]
}

fn cove_map_projectable_conflict_sections(
    conflict_policy: &str,
    governance_policy: &str,
) -> Vec<CovemapSection> {
    let mut sections = cove_map_conflict_sections(conflict_policy, governance_policy);
    sections.push(covemap_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-priority-map",
            "mapping_version": "2026.05",
            "projections": [{
                "projection_id": "person_projection",
                "output_table": "people_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "person_goid", "value": "object.goid", "logical_type": "uuid"},
                    {"name": "name", "value": "name", "logical_type": "utf8"}
                ],
                "output_modes": ["json", "arrow", "cove-t", "cove-o"]
            }]
        }),
    ));
    sections
}

pub(crate) fn cove_map_valid_file() -> Vec<u8> {
    semantic_profile_cove_file(
        PrimaryProfile::SemanticMapping,
        FEATURE_SEMANTIC_MAP,
        0,
        valid_map_sections(),
    )
}

pub(crate) fn cove_map_invalid_file() -> Vec<u8> {
    semantic_profile_cove_file(
        PrimaryProfile::SemanticMapping,
        FEATURE_SEMANTIC_MAP,
        0,
        vec![map_section(
            SectionKind::MapSourceCatalog,
            1,
            json!({
                "mapping_version": "2026.05",
                "sources": [{
                    "source_id": "crm.customers",
                    "schema_fingerprint": "schema-v1",
                    "snapshot_digest": "digest-v1",
                    "row_identity_rules": ["customer_id"],
                    "replay_claimed": true
                }]
            }),
        )],
    )
}

pub(crate) fn cove_map_function_undeclared_file() -> Vec<u8> {
    let mut sections = valid_map_sections();
    sections[1] = map_section(
        SectionKind::MapFunctionRegistry,
        0,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "functions": []
        }),
    );
    semantic_profile_cove_file(
        PrimaryProfile::SemanticMapping,
        FEATURE_SEMANTIC_MAP,
        0,
        sections,
    )
}

pub(crate) fn cove_map_identity_conflict_file() -> Vec<u8> {
    let mut sections = valid_map_sections();
    sections[2] = map_section(
        SectionKind::MapIdentityRuleCatalog,
        1,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "customer_identity",
                "object_type": "Customer",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": ["trim_lower"],
                "join_keys": [{
                    "role_id": "customer_id",
                    "source_column": "customer_id",
                    "logical_type": "utf8",
                    "canonicalization": "trim_lower",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": [{
                "left_identity": "customer:1",
                "right_identity": "customer:2"
            }]
        }),
    );
    sections.push(map_section(
        SectionKind::MapIdentityEquivalenceIndex,
        1,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "equivalences": [{
                "left_identity": "customer:1",
                "right_identity": "customer:2"
            }]
        }),
    ));
    semantic_profile_cove_file(
        PrimaryProfile::SemanticMapping,
        FEATURE_SEMANTIC_MAP,
        0,
        sections,
    )
}

pub(crate) fn cove_map_source_stale_file() -> Vec<u8> {
    let mut sections = valid_map_sections();
    sections[6] = map_section(
        SectionKind::MapConversionReport,
        1,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.customers",
                "schema_fingerprint": "schema-v2",
                "snapshot_digest": "digest-v1"
            }]
        }),
    );
    semantic_profile_cove_file(
        PrimaryProfile::SemanticMapping,
        FEATURE_SEMANTIC_MAP,
        0,
        sections,
    )
}

pub(crate) fn cove_map_evidence_invalid_file() -> Vec<u8> {
    let mut sections = valid_map_sections();
    sections[5] = map_section(
        SectionKind::MapEvidenceIndex,
        1,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "entries": [{
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_customer",
                "assertion_id": "assert_missing",
                "output_object_id": "goid:customer:1",
                "observed_schema_fingerprint": "schema-v1",
                "observed_snapshot_digest": "digest-v1"
            }]
        }),
    );
    semantic_profile_cove_file(
        PrimaryProfile::SemanticMapping,
        FEATURE_SEMANTIC_MAP,
        0,
        sections,
    )
}

fn valid_map_sections() -> Vec<SectionPayload> {
    vec![
        map_section(
            SectionKind::MapSourceCatalog,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "sources": [{
                    "source_id": "crm.customers",
                    "schema_fingerprint": "schema-v1",
                    "snapshot_digest": "digest-v1",
                    "row_identity_rules": ["customer_id"],
                    "replay_claimed": true
                }]
            }),
        ),
        map_section(
            SectionKind::MapFunctionRegistry,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "functions": [{
                    "function_id": "trim_lower",
                    "version": "1.0.0",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        map_section(
            SectionKind::MapIdentityRuleCatalog,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "identity_rules": [{
                    "rule_id": "customer_identity",
                    "object_type": "Customer",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["trim_lower"],
                    "join_keys": [{
                        "role_id": "customer_id",
                        "source_column": "customer_id",
                        "logical_type": "utf8",
                        "canonicalization": "trim_lower",
                        "null_policy": "reject",
                        "ordering": "asc"
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        map_section(
            SectionKind::MapRowSemanticsCatalog,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "rules": [{
                    "rule_id": "upsert_customer",
                    "source_id": "crm.customers",
                    "identity_rule_id": "customer_identity",
                    "row_semantics_kind": "Object",
                    "assertion_kinds": ["object", "property", "evidence"],
                    "function_ids": ["trim_lower"],
                    "output_assertion_ids": ["assert_customer_name"],
                    "association_endpoints": []
                }]
            }),
        ),
        map_section(
            SectionKind::MapAssertionLog,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "assertions": [{
                    "assertion_id": "assert_customer_name",
                    "output_object_id": "goid:customer:1"
                }]
            }),
        ),
        map_section(
            SectionKind::MapEvidenceIndex,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "entries": [{
                    "source_id": "crm.customers",
                    "source_row_identity": "customer_id=1",
                    "rule_id": "upsert_customer",
                    "assertion_id": "assert_customer_name",
                    "output_object_id": "goid:customer:1",
                    "observed_schema_fingerprint": "schema-v1",
                    "observed_snapshot_digest": "digest-v1"
                }]
            }),
        ),
        map_section(
            SectionKind::MapConversionReport,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "sources": [{
                    "source_id": "crm.customers",
                    "schema_fingerprint": "schema-v1",
                    "snapshot_digest": "digest-v1"
                }]
            }),
        ),
        map_section(
            SectionKind::MapProjectionCatalog,
            1,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "projections": [{
                    "projection_id": "customer_projection",
                    "assertion_ids": ["assert_customer_name"]
                }]
            }),
        ),
    ]
}

fn map_section(section_kind: SectionKind, item_count: u64, value: Value) -> SectionPayload {
    SectionPayload {
        section_kind: section_kind as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: map_payload_bytes(covemap_payload_value(section_kind, value)),
    }
}
