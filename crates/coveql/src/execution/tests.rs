use std::collections::{BTreeMap, BTreeSet};

use cove_core::{
    constants::{CoveLogicalType, CovePhysicalKind},
    profile::cove_o::{CoveObjectTombstoneStatus, RecordKind},
};

use crate::AstLiteral;

use super::*;

fn test_projection_root() -> ResolvedRoot {
    ResolvedRoot::Projection(crate::ResolvedProjectionRoot {
        projection_id: "people_projection".into(),
        mapping_id: "people-map".into(),
        mapping_version: "2026.05".into(),
        output_table: Some("people_projection".into()),
        row_grain: Some("one_row_per_object".into()),
        anchor: Some(crate::ResolvedProjectionAnchor {
            object_type: Some("Person".into()),
            association_type: None,
        }),
        temporal_mode: Some("latest_committed".into()),
        columns: vec![
            test_projection_column("active", "bool"),
            test_projection_column("name", "utf8"),
            test_projection_column("score", "float64"),
        ],
        assertion_ids: Vec::new(),
        multi_value_policy: Some("reject".into()),
        missing_policy: "null".into(),
        ordering: Vec::new(),
        evidence_policy: "none".into(),
        output_modes: vec!["json".into(), "arrow".into()],
        column_count: 3,
    })
}

fn test_projection_column(name: &str, logical_type: &str) -> crate::ResolvedProjectionColumn {
    crate::ResolvedProjectionColumn {
        name: name.into(),
        value: format!("property.{name}"),
        logical_type: Some(logical_type.into()),
        nested_shape: None,
        conflict_policy: "latest".into(),
        missing_policy: "null".into(),
        source_property_id: None,
    }
}

fn test_projection_path(column: &str, logical_type: &str) -> ResolvedExpr {
    ResolvedExpr::Path(ResolvedPath {
        display_name: column.into(),
        root_kind: crate::ResolvedPathRootKind::Projection,
        object_type_id: None,
        property_id: None,
        association_type_id: None,
        evidence_field_id: None,
        projection_id: Some("people_projection".into()),
        projection_column: Some(column.into()),
        system_field: None,
        logical_type: logical_type.into(),
        physical_kind: logical_type.into(),
        collation_id: None,
        nullable: true,
        null_policy: "cove_null_semantics_preserved".into(),
        temporal_role: None,
        code_domain_id: crate::CodeDomainId::Placeholder {
            root: "projection".into(),
            object_type_id: None,
            property_id: None,
            projection_id: Some("people_projection".into()),
            field: Some(column.into()),
        },
    })
}

fn test_bool_literal(value: bool) -> ResolvedExpr {
    ResolvedExpr::Literal(ResolvedLiteral {
        literal: AstLiteral::Boolean(value),
        logical_type: "bool".into(),
        canonical: value.to_string(),
        typed_value: ResolvedLiteralValue::Boolean(value),
        precision: None,
        scale: None,
    })
}

fn test_null_literal() -> ResolvedExpr {
    ResolvedExpr::Literal(ResolvedLiteral {
        literal: AstLiteral::Null,
        logical_type: "null".into(),
        canonical: "null".into(),
        typed_value: ResolvedLiteralValue::Null,
        precision: None,
        scale: None,
    })
}

fn test_eq_predicate(left: ResolvedExpr, right: ResolvedExpr) -> ResolvedPredicate {
    ResolvedPredicate::Compare {
        left,
        op: AstCompareOp::Eq,
        right,
    }
}

#[test]
fn evidence_default_order_aliases_match_materialized_evidence_fields() {
    let row = MaterializedEvidenceRow {
        fields: BTreeMap::from([
            ("output_object_id".into(), json!("object-1")),
            ("source_id".into(), json!("crm.customers")),
            ("source_row_identity".into(), json!("customer_id=1")),
            ("assertion_id".into(), json!("assertion-1")),
        ]),
    };

    assert_eq!(
        evidence_default_sort_field_value(&row, "target_id"),
        json!("object-1")
    );
    assert_eq!(
        evidence_default_sort_field_value(&row, "source_system"),
        json!("crm.customers")
    );
    assert_eq!(
        evidence_default_sort_field_value(&row, "source_row_identity"),
        json!("customer_id=1")
    );
    assert_eq!(
        evidence_default_sort_field_value(&row, "evidence_id"),
        json!("assertion-1")
    );
}

fn test_function_expr(function_id: &str, arg: ResolvedExpr, logical_type: &str) -> ResolvedExpr {
    ResolvedExpr::FunctionCall {
        function_id: function_id.into(),
        deterministic: true,
        logical_type: logical_type.into(),
        physical_kind: logical_type.into(),
        contract: crate::ResolvedFunctionContract {
            function_id: function_id.into(),
            version: "1".into(),
            deterministic: true,
            dependency: "materialized".into(),
            execution_class: crate::FunctionExecutionClass::MaterializedOnly,
            unicode_or_collation_contract: None,
        },
        args: vec![arg],
    }
}

#[test]
fn projection_output_columns_collect_residual_expression_inputs() {
    let root = test_projection_root();
    let method_chain = ResolvedMethodChain {
        select: Some(vec![crate::ResolvedSelectItem {
            alias: Some("display_name".into()),
            expr: test_function_expr("lower", test_projection_path("name", "utf8"), "utf8"),
        }]),
        where_predicate: Some(ResolvedPredicate::Not(Box::new(
            ResolvedPredicate::BoolExpr(test_projection_path("active", "bool")),
        ))),
        order_by: Some(crate::ResolvedOrderClause {
            expr: test_function_expr("lower", test_projection_path("name", "utf8"), "utf8"),
            direction: AstOrderDirection::Asc,
            nulls: AstNullOrdering::NullsLast,
            uses_default_ordering: false,
        }),
        group_by: Some(vec![test_projection_path("score", "float64")]),
        ..ResolvedMethodChain::default()
    };

    let output_columns = projection_output_columns_for_parts(&root, &method_chain).unwrap();

    assert_eq!(output_columns, vec!["active", "name", "score"]);
}

#[test]
fn projection_same_column_or_lowers_to_single_in_list_filter() {
    let predicate = ResolvedPredicate::Or(vec![
        test_eq_predicate(
            test_projection_path("active", "bool"),
            test_bool_literal(true),
        ),
        ResolvedPredicate::InList {
            expr: test_projection_path("active", "bool"),
            values: vec![ResolvedLiteral {
                literal: AstLiteral::Boolean(false),
                logical_type: "bool".into(),
                canonical: "false".into(),
                typed_value: ResolvedLiteralValue::Boolean(false),
                precision: None,
                scale: None,
            }],
        },
    ]);

    let filters = projection_filters_for_predicate(&predicate).unwrap();

    assert_eq!(
        filters,
        vec![ProjectionFilter::InList {
            column: "active".into(),
            literals: vec![
                ProjectionFilterLiteral::Boolean(true),
                ProjectionFilterLiteral::Boolean(false)
            ],
        }]
    );
}

#[test]
fn projection_or_with_null_literal_stays_residual() {
    let predicate = ResolvedPredicate::Or(vec![
        test_eq_predicate(
            test_projection_path("active", "bool"),
            test_bool_literal(true),
        ),
        test_eq_predicate(test_projection_path("active", "bool"), test_null_literal()),
    ]);

    assert!(projection_filters_for_predicate(&predicate).is_none());
}

#[test]
fn projection_or_across_columns_stays_residual() {
    let predicate = ResolvedPredicate::Or(vec![
        test_eq_predicate(
            test_projection_path("active", "bool"),
            test_bool_literal(true),
        ),
        test_eq_predicate(
            test_projection_path("name", "utf8"),
            test_bool_literal(true),
        ),
    ]);

    assert!(projection_filters_for_predicate(&predicate).is_none());
}

#[test]
fn evidence_object_rows_reuse_materialized_evidence_shape() {
    let states = vec![
        CoveObjectState {
            object_type_id: 9,
            object_type_name: "Evidence".into(),
            object_type_flags: OBJECT_TYPE_FLAG_EVIDENCE_OBJECT,
            branch_key: 3,
            goid: [1; 16],
            latest_record_id: [2; 16],
            latest_segment_id: 4,
            latest_row_index: 5,
            timestamp_us: 1_767_225_600_000_000,
            csn: 6,
            record_kind: RecordKind::Baseline,
            tombstone_status: CoveObjectTombstoneStatus::Live,
            association: None,
            properties: vec![
                CoveObjectPropertyValue {
                    property_id: 1,
                    property_name: "source_id".into(),
                    logical_type: CoveLogicalType::Utf8,
                    physical_kind: CovePhysicalKind::VarBytes,
                    flags: 0,
                    value: json!("row-1"),
                    redacted: false,
                },
                CoveObjectPropertyValue {
                    property_id: 2,
                    property_name: "raw_evidence".into(),
                    logical_type: CoveLogicalType::Utf8,
                    physical_kind: CovePhysicalKind::VarBytes,
                    flags: PROPERTY_FLAG_EVIDENCE_REF,
                    value: json!("ev-source"),
                    redacted: false,
                },
                CoveObjectPropertyValue {
                    property_id: 3,
                    property_name: "mapping_rule".into(),
                    logical_type: CoveLogicalType::Utf8,
                    physical_kind: CovePhysicalKind::VarBytes,
                    flags: PROPERTY_FLAG_MAPPING_RULE_REF,
                    value: json!("rule-7"),
                    redacted: false,
                },
            ],
        },
        CoveObjectState {
            object_type_id: 1,
            object_type_name: "Thing".into(),
            object_type_flags: 0,
            branch_key: 3,
            goid: [3; 16],
            latest_record_id: [4; 16],
            latest_segment_id: 4,
            latest_row_index: 6,
            timestamp_us: 1_767_225_600_000_001,
            csn: 7,
            record_kind: RecordKind::Baseline,
            tombstone_status: CoveObjectTombstoneStatus::Live,
            association: None,
            properties: Vec::new(),
        },
    ];

    let rows = evidence_object_rows_from_states(&states);
    assert_eq!(rows.len(), 1);
    let ExecutionRow::Evidence(row) = &rows[0] else {
        panic!("expected evidence row");
    };
    assert_eq!(row.fields["object_type_name"], json!("Evidence"));
    assert_eq!(row.fields["branch_key"], json!(3));
    assert_eq!(row.fields["source_id"], json!("row-1"));
    assert_eq!(row.fields["raw_evidence"], json!("ev-source"));
    assert_eq!(row.fields["source_evidence_id"], json!("ev-source"));
    assert_eq!(row.fields["mapping_rule"], json!("rule-7"));
    assert_eq!(row.fields["rule_id"], json!("rule-7"));
    assert_eq!(row.fields["grain"], json!("object"));
}

#[test]
fn external_overlay_visibility_filters_helper_association_rows_by_association_identity() {
    let visible = MaterializedAssociationRow {
        dataset_file_ordinal: None,
        dataset_file_source: None,
        dataset_file_id: None,
        output_grain: OutputGrain::AssociationState,
        change: None,
        object_type_id: 7,
        association_type: Some("CustomerPlacedOrder".into()),
        branch_key: 0,
        goid: "assoc-visible".into(),
        record_id: "assoc-record-visible".into(),
        source_goid: Some("person".into()),
        target_goid: Some("order".into()),
        timestamp_us: 0,
        csn: 1,
        record_kind: "baseline".into(),
        tombstone_status: "live".into(),
        properties: BTreeMap::new(),
        property_ids: BTreeMap::new(),
        redacted_properties: BTreeSet::new(),
    };
    let hidden = MaterializedAssociationRow {
        goid: "assoc-hidden".into(),
        record_id: "assoc-record-hidden".into(),
        ..visible.clone()
    };
    let overlay = VisibilityOverlay {
        overlay_id: "tenant-a".into(),
        visible_goids: BTreeSet::from(["assoc-visible".into()]),
        visible_record_ids: BTreeSet::new(),
    };

    assert!(association_row_visible_in_overlay(&visible, &overlay));
    assert!(!association_row_visible_in_overlay(&hidden, &overlay));
}

#[test]
fn external_overlay_visibility_filters_helper_evidence_rows_by_evidence_identity() {
    let mut visible_fields = BTreeMap::new();
    visible_fields.insert("evidence_id".into(), json!("evidence-visible"));
    visible_fields.insert("output_object_id".into(), json!("object-visible"));
    let mut hidden_fields = BTreeMap::new();
    hidden_fields.insert("output_object_id".into(), json!("object-visible"));
    hidden_fields.insert("assertion_id".into(), json!("assertion-hidden"));
    let visible = MaterializedEvidenceRow {
        fields: visible_fields,
    };
    let hidden = MaterializedEvidenceRow {
        fields: hidden_fields,
    };
    let overlay = VisibilityOverlay {
        overlay_id: "tenant-a".into(),
        visible_goids: BTreeSet::from(["evidence-visible".into(), "object-visible".into()]),
        visible_record_ids: BTreeSet::new(),
    };

    assert!(evidence_row_visible_in_overlay(&visible, &overlay));
    assert!(!evidence_row_visible_in_overlay(&hidden, &overlay));
}

#[test]
fn half_open_change_bounds_use_temporal_role_binding_value() {
    let record = CoveObjectRecord {
        object_type_id: 1,
        object_type_name: "Thing".into(),
        object_type_flags: 0,
        segment_id: 0,
        row_index: 0,
        timestamp_us: 5,
        csn: 10,
        branch_key: 0,
        goid: [1; 16],
        record_id: [2; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
        properties: vec![CoveObjectPropertyValue {
            property_id: 9,
            property_name: "source_event_time".into(),
            logical_type: CoveLogicalType::Int64,
            physical_kind: CovePhysicalKind::NumCode,
            flags: 0,
            value: json!(1_000),
            redacted: false,
        }],
        association: None,
    };
    let from = ResolvedTimeBound::TimestampMicros {
        role: TemporalRole::SourceEventTime,
        binding: Some("source_event_time".into()),
        timestamp_micros: 900,
        canonical_rfc3339: "n/a".into(),
    };
    let to = ResolvedTimeBound::TimestampMicros {
        role: TemporalRole::SourceEventTime,
        binding: Some("source_event_time".into()),
        timestamp_micros: 1_100,
        canonical_rfc3339: "n/a".into(),
    };

    assert!(record_in_half_open_bound(&record, &from, &to).unwrap());

    let late_from = ResolvedTimeBound::TimestampMicros {
        role: TemporalRole::SourceEventTime,
        binding: Some("source_event_time".into()),
        timestamp_micros: 1_100,
        canonical_rfc3339: "n/a".into(),
    };
    let late_to = ResolvedTimeBound::TimestampMicros {
        role: TemporalRole::SourceEventTime,
        binding: Some("source_event_time".into()),
        timestamp_micros: 1_200,
        canonical_rfc3339: "n/a".into(),
    };
    assert!(!record_in_half_open_bound(&record, &late_from, &late_to).unwrap());
}
