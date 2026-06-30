use super::*;

pub(super) fn write_delta_and_sidecar_fixtures(writer: &mut CorpusWriter<'_>) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    let covedelta_bytes = valid_covedelta_file();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_valid.covedelta",
            "covedelta",
            "accept",
            None,
            &["§63.1"],
        ),
        covedelta_bytes.clone(),
    );
    let mut bad_covedelta_magic = covedelta_bytes;
    *bad_covedelta_magic.last_mut().unwrap() = b'X';
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_bad_tail_magic.covedelta",
            "covedelta",
            "reject",
            Some("COVE_E_BAD_MAGIC"),
            &["§63.1", "§76"],
        ),
        bad_covedelta_magic,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_missing_lineage_parent.covedelta",
            "covedelta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_missing_lineage_parent_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_duplicate_lineage_parent.covedelta",
            "covedelta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_duplicate_lineage_parent_file(),
    );

    let covedelta_object_delta_bytes = valid_covedelta_object_delta_file();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_bytes.clone(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_anchor_touched_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_anchor_touched_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_touched_property_bitmap_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_touched_property_bitmap_file(Some(
            DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP,
        )),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_branch_identity_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_branch_identity_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_sparse_patch_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_sparse_patch_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_dictionary_overlay_inline_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_dictionary_overlay_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_dictionary_overlay_parent_alias_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_parent_alias_dictionary_overlay_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_dictionary_overlay_hash_hint_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_hash_hint_dictionary_overlay_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_descriptor_tables_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_descriptor_tables_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_catalog_patch_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_catalog_patch_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_projection_patch_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69", "§70.10"],
        ),
        covedelta_object_delta_with_projection_patch_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_index_hints_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_index_hints_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_coverage_patch_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_coverage_patch_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_layout_hints_valid.covedelta",
            "covedelta_layout_hints",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_with_layout_hints_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_corrupt_optional_index_fallback.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_corrupt_optional_index_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_optional_index_unknown_required_fallback.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69", "§74"],
        ),
        covedelta_object_delta_with_optional_index_unknown_required_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_corrupt_optional_layout_fallback.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_corrupt_optional_section_file(
            CoveDeltaSectionKind::LayoutHints,
            b"corrupt optional delta-local layout",
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_corrupt_optional_coverage_fallback.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_corrupt_optional_section_file(
            CoveDeltaSectionKind::CoveragePatch,
            b"corrupt optional delta-local coverage",
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_sparse_patch_state_reconstruction_case.json",
            "covedelta_sparse_patch_state_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_sparse_patch_state_reconstruction_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_tombstone_set_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_with_tombstone_set_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_tombstone_reconstruction_case.json",
            "covedelta_reconstruction_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_tombstone_reconstruction_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_association_update_reconstruction_case.json",
            "covedelta_reconstruction_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_association_update_reconstruction_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_evidence_patch_inherited_map_fingerprint_case.json",
            "covedelta_reconstruction_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_evidence_patch_inherited_map_fingerprint_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_projection_patch_inherited_projection_fingerprint_case.json",
            "covedelta_reconstruction_case",
            "accept",
            None,
            &["§63.6", "§69", "§70.10"],
        ),
        covedelta_projection_patch_inherited_projection_fingerprint_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_membership_scope_case.json",
            "covedelta_object_membership_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_object_membership_scope_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_projection_property_skip_case.json",
            "covedelta_object_membership_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_object_projection_property_skip_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_covi_base_tombstone_overlay_case.json",
            "covedelta_covi_tombstone_overlay_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_covi_base_tombstone_overlay_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_base_plus_one_delta_reconstruction_case.json",
            "covedelta_reconstruction_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_base_plus_one_delta_reconstruction_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_compaction_equivalence_case.json",
            "covedelta_reconstruction_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_compaction_equivalence_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_catalog_patch_reconstruction_case.json",
            "covedelta_reconstruction_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_catalog_patch_reconstruction_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_catalog_patch_reinterprets_parent.json",
            "covedelta_reconstruction_case",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§63.6", "§69"],
        ),
        covedelta_catalog_patch_reinterprets_parent_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_object_delta_checkpoint_valid.covedelta",
            "covedelta_object_delta",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covedelta_object_delta_checkpoint_file(),
    );
    let rows = valid_temporal_rows();
    let bad_order_rows = vec![rows[1].clone(), rows[0].clone()];
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_bad_temporal_order.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§58", "§63.1", "§76"],
        ),
        covedelta_object_delta_file_with_rows(&bad_order_rows),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_duplicate_record_id.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_file_with_rows(&duplicate_record_id_temporal_rows()),
    );
    let mut unknown_required_covedelta_object_delta = covedelta_object_delta_bytes;
    rewrite_covedelta_required_features(&mut unknown_required_covedelta_object_delta, 1u64 << 63);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_unknown_required_feature.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_UNKNOWN_REQUIRED_FEATURE"),
            &["§63.1", "§69", "§76"],
        ),
        unknown_required_covedelta_object_delta,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_required_anchor_missing.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_file_with_rows_and_sections(
            &valid_temporal_rows(),
            DELTA_FEATURE_CONTINUATION_ANCHORS,
            Vec::new(),
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_checkpoint_missing_rows.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_checkpoint_missing_rows_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_sparse_patch_missing_ops.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_file_with_rows_and_sections(
            &valid_temporal_rows(),
            DELTA_FEATURE_SPARSE_PATCH_ROWS,
            Vec::new(),
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_sparse_patch_row_mismatch.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_object_delta_with_mismatched_sparse_patch_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_dictionary_overlay_parent_alias.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_parent_alias_dictionary_overlay_missing_feature_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_dictionary_overlay_unknown_parent_ref.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_parent_alias_dictionary_overlay_unknown_parent_ref_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_dictionary_overlay_hash_hint_zero_hash.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_zero_hash_hint_dictionary_overlay_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_dictionary_overlay_hash_hint_required.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_required_hash_hint_dictionary_overlay_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_dictionary_overlay_duplicate_local_code.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_duplicate_dictionary_overlay_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_descriptor_sparse_scope_ref.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_sparse_scope_descriptor_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_descriptor_sparse_summary_ref.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_sparse_summary_descriptor_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_evidence_patch_missing_map_fingerprint.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_object_delta_with_evidence_patch_missing_map_fingerprint_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_projection_patch_missing_section.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§70.10", "§76"],
        ),
        covedelta_object_delta_with_projection_patch_missing_section_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_projection_patch_missing_fingerprint.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§70.10", "§76"],
        ),
        covedelta_object_delta_with_projection_patch_missing_fingerprint_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_index_hints_missing_section.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_index_hints_missing_section_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_index_hints_lineage_parent.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_index_hints_lineage_parent_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_coverage_patch_wrong_kind.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_coverage_patch_wrong_kind_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_layout_hints_wrong_kind.covedelta",
            "covedelta_layout_hints",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_with_layout_hints_wrong_kind_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_anchor_underinclude.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_object_delta_with_underinclusive_anchor_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_anchor_state_hash_ref_missing.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_object_delta_with_unresolved_anchor_state_hash_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_single_scope_anchor_mismatch.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_single_scope_anchor_mismatch_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_touched_underinclude.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_object_delta_with_underinclusive_touched_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_touched_property_bitmap_missing_descriptor.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_touched_property_bitmap_file(None),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_touched_property_bitmap_wrong_descriptor.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§69", "§76"],
        ),
        covedelta_object_delta_with_touched_property_bitmap_file(Some(
            DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET,
        )),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_tombstone_underinclude.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_object_delta_with_underinclusive_tombstone_file(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_object_delta_tombstone_set_missing.covedelta",
            "covedelta_object_delta",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        covedelta_object_delta_file_with_rows_and_sections(
            &valid_tombstone_temporal_rows(),
            DELTA_FEATURE_EXACT_TOMBSTONE_SET,
            Vec::new(),
        ),
    );

    let covedelta_branch_identity_bytes = valid_delta_branch_identity().serialize().to_vec();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_branch_identity_valid.bin",
            "covedelta_branch_identity",
            "accept",
            None,
            &["§63.1"],
        ),
        covedelta_branch_identity_bytes,
    );
    let mut missing_branch_value = valid_delta_branch_identity();
    missing_branch_value.branch_value_ref = DELTA_REF_NONE;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_branch_identity_missing_value_ref.bin",
            "covedelta_branch_identity",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        missing_branch_value.serialize().to_vec(),
    );
    let mut reserved_branch_flags = valid_delta_branch_identity();
    reserved_branch_flags.flags = 1;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_branch_identity_reserved_flags.bin",
            "covedelta_branch_identity",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        reserved_branch_flags.serialize().to_vec(),
    );
    let mut raw_parent_file_code_branch_key = valid_delta_branch_identity();
    raw_parent_file_code_branch_key.branch_identity_kind = 2;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_branch_identity_raw_parent_filecode.bin",
            "covedelta_branch_identity",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        raw_parent_file_code_branch_key.serialize().to_vec(),
    );

    let covedelta_anchor_bytes = valid_delta_continuation_anchor().serialize().to_vec();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_continuation_anchor_valid.bin",
            "covedelta_continuation_anchor",
            "accept",
            None,
            &["§63.1"],
        ),
        covedelta_anchor_bytes.clone(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_existing_patch_anchor_valid.bin",
            "covedelta_existing_patch_anchor",
            "accept",
            None,
            &["§63.1"],
        ),
        covedelta_anchor_bytes,
    );
    let mut weak_anchor = valid_delta_continuation_anchor();
    weak_anchor.anchor_strength = DELTA_ANCHOR_STRENGTH_KEY_AND_RECORD_ID;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_existing_patch_anchor_weak.bin",
            "covedelta_existing_patch_anchor",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        weak_anchor.serialize().to_vec(),
    );

    let covedelta_state_hash_descriptor_bytes =
        valid_delta_state_hash_descriptor().serialize().to_vec();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_state_hash_descriptor_valid.bin",
            "covedelta_state_hash_descriptor",
            "accept",
            None,
            &["§63.1"],
        ),
        covedelta_state_hash_descriptor_bytes,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_state_hash_recompute_case.json",
            "covedelta_state_hash_case",
            "accept",
            None,
            &["§63.6", "§69"],
        ),
        covedelta_state_hash_recompute_case(),
    );
    let mut missing_state_hash_payload = valid_delta_state_hash_descriptor();
    missing_state_hash_payload.hash_payload_ref = DELTA_REF_NONE;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_state_hash_descriptor_missing_payload.bin",
            "covedelta_state_hash_descriptor",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        missing_state_hash_payload.serialize().to_vec(),
    );
    let mut reserved_state_hash_flags = valid_delta_state_hash_descriptor();
    reserved_state_hash_flags.flags = 1;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_state_hash_descriptor_reserved_flags.bin",
            "covedelta_state_hash_descriptor",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        reserved_state_hash_flags.serialize().to_vec(),
    );
    let mut bad_state_hash_len = valid_delta_state_hash_descriptor();
    bad_state_hash_len.hash_len = 31;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_state_hash_descriptor_bad_len.bin",
            "covedelta_state_hash_descriptor",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        bad_state_hash_len.serialize().to_vec(),
    );

    let covedelta_sparse_patch_record_bytes =
        valid_delta_sparse_patch_record().serialize().unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_sparse_patch_record_valid.bin",
            "covedelta_sparse_patch_record",
            "accept",
            None,
            &["§63.1"],
        ),
        covedelta_sparse_patch_record_bytes,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_sparse_patch_record_missing_value_ref.bin",
            "covedelta_sparse_patch_record",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        sparse_patch_record_missing_value_ref_bytes(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_sparse_patch_record_null_with_payload.bin",
            "covedelta_sparse_patch_record",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        sparse_patch_record_null_with_payload_bytes(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_sparse_patch_record_bad_tombstone_kind.bin",
            "covedelta_sparse_patch_record",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        sparse_patch_record_bad_tombstone_kind_bytes(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_sparse_patch_record_missing_redaction_ref.bin",
            "covedelta_sparse_patch_record",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        sparse_patch_record_missing_redaction_ref_bytes(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_sparse_patch_record_unsorted_properties.bin",
            "covedelta_sparse_patch_record",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        sparse_patch_record_unsorted_properties_bytes(),
    );

    let covedelta_touched_range_bytes = valid_delta_touched_object_range().serialize().to_vec();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covedelta_touched_object_range_valid.bin",
            "covedelta_touched_object_range",
            "accept",
            None,
            &["§63.1"],
        ),
        covedelta_touched_range_bytes,
    );
    let mut inverted_touched_range = valid_delta_touched_object_range();
    inverted_touched_range.min_goid = [0xC0; 16];
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covedelta_touched_object_range_inverted.bin",
            "covedelta_touched_object_range",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§63.1", "§76"],
        ),
        inverted_touched_range.serialize().to_vec(),
    );

    let covm_delta_chain_bytes = valid_covm_delta_chain_extension();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_chain_extension_valid.bin",
            "covm_delta_chain_extension",
            "accept",
            None,
            &["§63.1", "§69"],
        ),
        covm_delta_chain_bytes.clone(),
    );
    let mut bad_covm_delta_chain_digest = covm_delta_chain_bytes.clone();
    let chain_digest_offset = COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN + COVM_DELTA_ARTIFACT_REF_LEN;
    bad_covm_delta_chain_digest[chain_digest_offset] ^= 0xFF;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_extension_bad_digest.bin",
            "covm_delta_chain_extension",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            &["§69", "§76"],
        ),
        bad_covm_delta_chain_digest,
    );
    let mut unsupported_covm_delta_feature = covm_delta_chain_bytes.clone();
    rewrite_covm_delta_chain_required_features(&mut unsupported_covm_delta_feature, 1u64 << 63);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_extension_unknown_required_feature.bin",
            "covm_delta_chain_extension",
            "reject",
            Some("COVE_E_UNKNOWN_REQUIRED_FEATURE"),
            &["§69", "§76"],
        ),
        unsupported_covm_delta_feature,
    );
    let mut sparse_covm_delta_chain = covm_delta_chain_bytes;
    rewrite_covm_delta_ref_ordinal(&mut sparse_covm_delta_chain, 0, 2);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_extension_sparse_ordinal.bin",
            "covm_delta_chain_extension",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§76"],
        ),
        sparse_covm_delta_chain,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_extension_duplicate_artifact_id.bin",
            "covm_delta_chain_extension",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§76"],
        ),
        covm_delta_chain_extension_duplicate_artifact_id(),
    );

    let covm_delta_chain_summary = valid_covm_delta_chain_summary();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_chain_summary_valid.bin",
            "covm_delta_chain_summary",
            "accept",
            None,
            &["§69"],
        ),
        covm_delta_chain_summary.clone(),
    );
    let mut bad_covm_delta_chain_summary_magic = covm_delta_chain_summary.clone();
    bad_covm_delta_chain_summary_magic[0] = b'X';
    rewrite_covm_delta_chain_summary_header_crc(&mut bad_covm_delta_chain_summary_magic);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_summary_bad_magic.bin",
            "covm_delta_chain_summary",
            "reject",
            Some("COVE_E_BAD_MAGIC"),
            &["§69", "§76"],
        ),
        bad_covm_delta_chain_summary_magic,
    );
    let mut sparse_covm_delta_chain_summary = covm_delta_chain_summary.clone();
    rewrite_covm_delta_summary_entry_ordinal(&mut sparse_covm_delta_chain_summary, 0, 2);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_summary_sparse_ordinal.bin",
            "covm_delta_chain_summary",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§76"],
        ),
        sparse_covm_delta_chain_summary,
    );
    let mut unsupported_covm_delta_summary_feature = covm_delta_chain_summary;
    rewrite_covm_delta_summary_entry_required_features(
        &mut unsupported_covm_delta_summary_feature,
        0,
        1u64 << 63,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_summary_unknown_required_feature.bin",
            "covm_delta_chain_summary",
            "reject",
            Some("COVE_E_UNKNOWN_REQUIRED_FEATURE"),
            &["§69", "§76"],
        ),
        unsupported_covm_delta_summary_feature,
    );
    let mut non_append_csn_covm_delta_chain_summary = valid_covm_delta_pruning_summary(false);
    rewrite_covm_delta_summary_entry_csn_min(&mut non_append_csn_covm_delta_chain_summary, 1, 20);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_summary_non_append_csn.bin",
            "covm_delta_chain_summary",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§76"],
        ),
        non_append_csn_covm_delta_chain_summary,
    );
    let mut decreasing_commit_covm_delta_chain_summary = valid_covm_delta_pruning_summary(false);
    rewrite_covm_delta_summary_entry_commit_start(
        &mut decreasing_commit_covm_delta_chain_summary,
        1,
        900,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_summary_decreasing_commit.bin",
            "covm_delta_chain_summary",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§76"],
        ),
        decreasing_commit_covm_delta_chain_summary,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_chain_selection_valid.json",
            "covm_delta_chain_selection_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_chain_selection_case(false),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_missing_delta_bytes.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_delta_count_case(0),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_extra_delta_bytes.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_delta_count_case(2),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_bad_digest.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_case(true),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_reordered_deltas.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_SIDECAR_STALE"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_reordered_deltas_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_summary_underinclude_csn.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_SIDECAR_STALE"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_summary_underinclude_csn_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_summary_underinclude_commit.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_SIDECAR_STALE"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_summary_underinclude_commit_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_wrong_parent_snapshot.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_SIDECAR_STALE"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_wrong_parent_snapshot_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_chain_selection_with_base_valid.json",
            "covm_delta_chain_selection_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_chain_selection_with_base_case(false),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_stale_lineage_parent.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_SIDECAR_STALE"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_with_base_case(true),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_missing_summary.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_required_summary_case(false, false),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_corrupt_summary.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_required_summary_case(true, true),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_summary_wrong_chain_digest.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_SIDECAR_STALE"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_summary_wrong_chain_digest_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_summary_digest_mismatch.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_required_summary_digest_mismatch_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_chain_selection_unadvertised_required_delta_feature.json",
            "covm_delta_chain_selection_case",
            "reject",
            Some("COVE_E_SIDECAR_STALE"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_chain_selection_unadvertised_required_delta_feature_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_pruning_as_of_csn.json",
            "covm_delta_pruning_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_pruning_case(
            Some(25),
            None,
            &[1],
            &[(2, "as_of_csn_before_delta")],
            false,
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_pruning_as_of_csn_before.json",
            "covm_delta_pruning_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_pruning_case(
            Some(5),
            None,
            &[],
            &[(1, "as_of_csn_before_delta"), (2, "as_of_csn_before_delta")],
            false,
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_pruning_as_of_csn_inside.json",
            "covm_delta_pruning_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_pruning_case(
            Some(15),
            None,
            &[1],
            &[(2, "as_of_csn_before_delta")],
            false,
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_pruning_as_of_csn_after.json",
            "covm_delta_pruning_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_pruning_case(Some(45), None, &[1, 2], &[], false),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_pruning_commit_time.json",
            "covm_delta_pruning_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_pruning_case(
            None,
            Some(1_500),
            &[1],
            &[(2, "as_of_commit_before_delta")],
            false,
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_pruning_source_publish.json",
            "covm_delta_pruning_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_pruning_case_with_source_range(
            None,
            None,
            Some((1_050, 1_050)),
            &[1],
            &[(2, "source_publish_range_outside_delta")],
            false,
        ),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covm_delta_pruning_valid_time_no_summary.json",
            "covm_delta_pruning_case",
            "accept",
            None,
            &["§69", "§63.1"],
        ),
        covm_delta_pruning_valid_time_no_summary_case(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_pruning_missing_commit_fields.json",
            "covm_delta_pruning_case",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§69", "§63.1", "§76"],
        ),
        covm_delta_pruning_case(None, Some(1_500), &[], &[], true),
    );

    for (path, case) in [
        (
            "accept/sidecar_freshness_valid.json",
            SidecarFreshnessCase::Valid,
        ),
        (
            "accept/sidecar_freshness_file_id_stale.json",
            SidecarFreshnessCase::FileId,
        ),
        (
            "accept/sidecar_freshness_file_len_stale.json",
            SidecarFreshnessCase::FileLen,
        ),
        (
            "accept/sidecar_freshness_footer_crc_stale.json",
            SidecarFreshnessCase::FooterCrc,
        ),
        (
            "accept/sidecar_freshness_digest_stale.json",
            SidecarFreshnessCase::Digest,
        ),
        (
            "accept/sidecar_freshness_corrupt_ignored.json",
            SidecarFreshnessCase::Corrupt,
        ),
    ] {
        write_fixture(
            root,
            entries,
            fixture(
                path,
                "sidecar_freshness_case",
                "accept",
                None,
                &["§48", "§68", "§69"],
            ),
            sidecar_freshness_payload(case),
        );
    }
}
