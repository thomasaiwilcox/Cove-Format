use std::{path::Path, sync::Arc};

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use arrow_ipc::writer::FileWriter as IpcFileWriter;
use orc_rust::ArrowWriterBuilder as OrcWriterBuilder;
use parquet::arrow::ArrowWriter;

use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1,
        CovemapSection, CovemapSectionEntryV1,
    },
    constants::{PrimaryProfile, SectionKind, FEATURE_SEMANTIC_MAP},
    writer::SectionPayload,
};
use serde_json::{json, Value};

use super::{
    covemap_payload_value, fixture, map_payload_bytes, semantic_profile_cove_file,
    suite_contract_fixture_bytes, write_auxiliary_file, write_fixture,
};

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
