use super::*;

pub(super) fn write_error_surface_fixtures(writer: &mut CorpusWriter<'_>) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    for (path, code) in [
        (
            "reject/error_surface_bad_version.json",
            "COVE_E_BAD_VERSION",
        ),
        (
            "reject/error_surface_arith_overflow.json",
            "COVE_E_ARITH_OVERFLOW",
        ),
        ("reject/error_surface_dict_miss.json", "COVE_E_DICT_MISS"),
        (
            "reject/error_surface_bad_filecode.json",
            "COVE_E_BAD_FILECODE",
        ),
        (
            "reject/error_surface_bad_numcode.json",
            "COVE_E_BAD_NUMCODE",
        ),
        (
            "reject/error_surface_bad_extension.json",
            "COVE_E_BAD_EXTENSION",
        ),
        (
            "reject/error_surface_execution_code_map.json",
            "COVE_E_EXECUTION_CODE_MAP",
        ),
        (
            "reject/error_surface_harbor_mount_lease.json",
            "COVE_E_HARBOR_MOUNT_LEASE",
        ),
        (
            "reject/error_surface_not_self_contained.json",
            "COVE_E_NOT_SELF_CONTAINED",
        ),
        (
            "reject/error_surface_redaction_policy.json",
            "COVE_E_REDACTION_POLICY",
        ),
        (
            "reject/error_surface_sidecar_stale.json",
            "COVE_E_SIDECAR_STALE",
        ),
        (
            "reject/error_surface_map_invalid.json",
            "COVE_E_MAP_INVALID",
        ),
        (
            "reject/error_surface_map_function_undeclared.json",
            "COVE_E_MAP_FUNCTION_UNDECLARED",
        ),
        (
            "reject/error_surface_map_identity_conflict.json",
            "COVE_E_MAP_IDENTITY_CONFLICT",
        ),
        (
            "reject/error_surface_map_source_stale.json",
            "COVE_E_MAP_SOURCE_STALE",
        ),
        (
            "reject/error_surface_map_evidence_invalid.json",
            "COVE_E_MAP_EVIDENCE_INVALID",
        ),
        (
            "reject/error_surface_bad_codec_extension.json",
            "COVE_E_BAD_CODEC_EXTENSION",
        ),
        (
            "reject/error_surface_codec_unsupported.json",
            "COVE_E_CODEC_UNSUPPORTED",
        ),
        (
            "reject/error_surface_bad_layout_plan.json",
            "COVE_E_BAD_LAYOUT_PLAN",
        ),
        (
            "reject/error_surface_runtime_hint_unsupported.json",
            "COVE_E_RUNTIME_HINT_UNSUPPORTED",
        ),
        (
            "reject/error_surface_bad_coverage.json",
            "COVE_E_BAD_COVERAGE",
        ),
        (
            "reject/error_surface_coverage_stale.json",
            "COVE_E_COVERAGE_STALE",
        ),
        ("reject/error_surface_bad_covi.json", "COVE_E_BAD_COVI"),
        (
            "reject/error_surface_index_only_unsafe.json",
            "COVE_E_INDEX_ONLY_UNSAFE",
        ),
        (
            "reject/error_surface_cache_stale.json",
            "COVE_E_CACHE_STALE",
        ),
        (
            "reject/error_surface_query_discovery_invalid.json",
            "COVE_E_QUERY_DISCOVERY_INVALID",
        ),
    ] {
        write_fixture(
            root,
            entries,
            fixture(path, "error_surface_case", "reject", Some(code), &["§76"]),
            error_surface_fixture_bytes(json!({ "code": code })),
        );
    }
}
