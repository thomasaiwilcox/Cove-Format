use std::{fs, path::PathBuf};

use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1,
        CovemapSection, CovemapSectionEntryV1,
    },
    constants::{
        CoveEncodingKind, CoveLogicalType, CovePhysicalKind, SectionKind, FEATURE_SEMANTIC_MAP,
    },
    table::{ColumnEntry, TableCatalog, TableEntry},
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment},
};
use serde_json::{json, Value};

fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let showcase = args.first().is_some_and(|arg| arg == "--showcase");
    if showcase {
        args.remove(0);
    }
    let output = args.first().map(PathBuf::from).unwrap_or_else(|| {
        if showcase {
            PathBuf::from("examples/showcase")
        } else {
            PathBuf::from("examples/coveql")
        }
    });
    fs::create_dir_all(&output).unwrap();
    if showcase {
        write_showcase_samples(&output);
    } else {
        write_beginner_samples(&output);
    }
}

fn write_beginner_samples(output: &std::path::Path) {
    let source_path = output.join("people.jsonl");
    fs::write(
        &source_path,
        [
            r#"{"id":"p1","active":true,"score":10,"rating":15,"status":"open","nickname":"ada"}"#,
            r#"{"id":"p2","active":false,"score":20,"rating":25,"status":"closed","nickname":null}"#,
            r#"{"id":"p3","active":true,"score":30,"rating":35,"status":"open","nickname":"grace"}"#,
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let map_path = output.join("people.covemap");
    fs::write(&map_path, beginner_people_covemap().serialize().unwrap()).unwrap();
    let object_bytes = cove_map::cove_o_from_paths(&map_path, std::slice::from_ref(&source_path))
        .expect("COVE-O sample conversion");
    fs::write(output.join("people.cove"), object_bytes).unwrap();

    fs::write(output.join("events.cove"), beginner_events_cove_t()).unwrap();
    fs::write(output.join("README.md"), sample_readme()).unwrap();
    println!("wrote beginner CoveQL samples to {}", output.display());
}

fn write_showcase_samples(output: &std::path::Path) {
    let crm_source = output.join("crm_people.jsonl");
    fs::write(
        &crm_source,
        [
            r#"{"id":"p1","full_name":"Ada Lovelace","region":"uk","tier":"platinum"}"#,
            r#"{"id":"p2","full_name":"Grace Hopper","region":"us","tier":"gold"}"#,
            r#"{"id":"p4","full_name":"Katherine Johnson","region":"us","tier":"silver"}"#,
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let support_source = output.join("support_people.jsonl");
    fs::write(
        &support_source,
        [
            r#"{"id":"p1","active":true,"score":95,"status":"active"}"#,
            r#"{"id":"p2","active":true,"score":88,"status":"active"}"#,
            r#"{"id":"p3","active":false,"score":42,"status":"dormant"}"#,
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let map_path = output.join("customer_identity.covemap");
    fs::write(&map_path, showcase_covemap().serialize().unwrap()).unwrap();

    let readback_source = output.join("customers_360.jsonl");
    fs::write(
        &readback_source,
        [
            r#"{"id":"p1","full_name":"Ada Lovelace","region":"uk","tier":"platinum","active":true,"score":95,"status":"active"}"#,
            r#"{"id":"p2","full_name":"Grace Hopper","region":"us","tier":"gold","active":true,"score":88,"status":"active"}"#,
            r#"{"id":"p3","full_name":null,"region":null,"tier":null,"active":false,"score":42,"status":"dormant"}"#,
            r#"{"id":"p4","full_name":"Katherine Johnson","region":"us","tier":"silver","active":null,"score":null,"status":null}"#,
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    let readback_map_path = output.join("customer_readback.covemap");
    fs::write(
        &readback_map_path,
        showcase_readback_covemap().serialize().unwrap(),
    )
    .unwrap();
    let object_bytes = cove_map::cove_o_from_paths(&readback_map_path, &[readback_source])
        .expect("showcase COVE-O readback conversion");
    fs::write(output.join("customers.cove"), object_bytes).unwrap();
    fs::write(output.join("events.cove"), showcase_events_cove_t()).unwrap();
    fs::write(output.join("README.md"), showcase_readme()).unwrap();
    println!("wrote COVE showcase samples to {}", output.display());
}

fn beginner_events_cove_t() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                ColumnEntry {
                    column_id: 1,
                    name: "id".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
                ColumnEntry {
                    column_id: 2,
                    name: "score".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
            ],
        }],
    };
    let mut ids = Vec::new();
    let mut scores = Vec::new();
    for (id, score) in [(1u64, 10u64), (2, 20), (3, 30)] {
        ids.extend_from_slice(&id.to_le_bytes());
        scores.extend_from_slice(&score.to_le_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, 3, 2);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(3, ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(3, scores).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn beginner_people_covemap() -> CovemapFile {
    CovemapFile {
        header: CovemapHeaderV1::new([0x51; 16], 0),
        mapping_version: "beginner/v1".into(),
        sections: vec![
            map_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "beginner-people-map",
                    "mapping_version": "beginner/v1",
                    "sources": [{
                        "source_id": "people",
                        "row_identity_rules": ["person_by_id"]
                    }]
                }),
            ),
            map_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "beginner-people-map",
                    "mapping_version": "beginner/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            map_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "beginner-people-map",
                    "mapping_version": "beginner/v1",
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
            map_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "beginner-people-map",
                    "mapping_version": "beginner/v1",
                    "rules": [{
                        "rule_id": "person_row",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [
                            property_binding("active", "bool"),
                            property_binding("score", "int64"),
                            property_binding("rating", "int64"),
                            property_binding("status", "utf8"),
                            property_binding("nickname", "utf8")
                        ]
                    }]
                }),
            ),
            map_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "beginner-people-map",
                    "mapping_version": "beginner/v1",
                    "projections": [{
                        "projection_id": "people_primitives.v1",
                        "output_table": "people",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [
                            {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                            {"name": "active", "value": "property.active", "logical_type": "bool"},
                            {"name": "score", "value": "score", "logical_type": "int64"},
                            {"name": "rating", "value": "rating", "logical_type": "int64"},
                            {"name": "status", "value": "status", "logical_type": "utf8"},
                            {"name": "nickname", "value": "nickname", "logical_type": "utf8"}
                        ],
                        "output_modes": ["json", "arrow"]
                    }]
                }),
            ),
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
}

fn showcase_covemap() -> CovemapFile {
    CovemapFile {
        header: CovemapHeaderV1::new([0x72; 16], 0),
        mapping_version: "showcase/v1".into(),
        sections: vec![
            map_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "showcase-customer-identity",
                    "mapping_version": "showcase/v1",
                    "sources": [
                        {"source_id": "crm_people", "row_identity_rules": ["person_by_id"]},
                        {"source_id": "support_people", "row_identity_rules": ["person_by_id"]}
                    ]
                }),
            ),
            map_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "showcase-customer-identity",
                    "mapping_version": "showcase/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            map_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "showcase-customer-identity",
                    "mapping_version": "showcase/v1",
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
            map_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "showcase-customer-identity",
                    "mapping_version": "showcase/v1",
                    "rules": [
                        {
                            "rule_id": "crm_person_row",
                            "source_id": "crm_people",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "property_bindings": [
                                property_binding("full_name", "utf8"),
                                property_binding("region", "utf8"),
                                property_binding("tier", "utf8")
                            ]
                        },
                        {
                            "rule_id": "support_person_row",
                            "source_id": "support_people",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "property_bindings": [
                                property_binding("active", "bool"),
                                property_binding("score", "int64"),
                                property_binding("status", "utf8")
                            ]
                        }
                    ]
                }),
            ),
            map_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "showcase-customer-identity",
                    "mapping_version": "showcase/v1",
                    "projections": [{
                        "projection_id": "customer_360.v1",
                        "output_table": "customers",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [
                            {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                            {"name": "full_name", "value": "full_name", "logical_type": "utf8"},
                            {"name": "region", "value": "region", "logical_type": "utf8"},
                            {"name": "tier", "value": "tier", "logical_type": "utf8"},
                            {"name": "active", "value": "active", "logical_type": "bool"},
                            {"name": "score", "value": "score", "logical_type": "int64"},
                            {"name": "status", "value": "status", "logical_type": "utf8"}
                        ],
                        "output_modes": ["json", "arrow", "cove-t"]
                    }]
                }),
            ),
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
}

fn showcase_readback_covemap() -> CovemapFile {
    CovemapFile {
        header: CovemapHeaderV1::new([0x73; 16], 0),
        mapping_version: "showcase-readback/v1".into(),
        sections: vec![
            map_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "showcase-customer-readback",
                    "mapping_version": "showcase-readback/v1",
                    "sources": [{
                        "source_id": "customers_360",
                        "row_identity_rules": ["person_by_id"]
                    }]
                }),
            ),
            map_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "showcase-customer-readback",
                    "mapping_version": "showcase-readback/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            map_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "showcase-customer-readback",
                    "mapping_version": "showcase-readback/v1",
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
            map_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "showcase-customer-readback",
                    "mapping_version": "showcase-readback/v1",
                    "rules": [{
                        "rule_id": "customer_360_row",
                        "source_id": "customers_360",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [
                            property_binding("full_name", "utf8"),
                            property_binding("region", "utf8"),
                            property_binding("tier", "utf8"),
                            property_binding("active", "bool"),
                            property_binding("score", "int64"),
                            property_binding("status", "utf8")
                        ]
                    }]
                }),
            ),
            map_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "showcase-customer-readback",
                    "mapping_version": "showcase-readback/v1",
                    "projections": [{
                        "projection_id": "customer_360.v1",
                        "output_table": "customers",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [
                            {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                            {"name": "full_name", "value": "full_name", "logical_type": "utf8"},
                            {"name": "region", "value": "region", "logical_type": "utf8"},
                            {"name": "tier", "value": "tier", "logical_type": "utf8"},
                            {"name": "active", "value": "active", "logical_type": "bool"},
                            {"name": "score", "value": "score", "logical_type": "int64"},
                            {"name": "status", "value": "status", "logical_type": "utf8"}
                        ],
                        "output_modes": ["json", "arrow", "cove-t"]
                    }]
                }),
            ),
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
}

fn showcase_events_cove_t() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 5,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                ColumnEntry {
                    column_id: 1,
                    name: "event_id".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
                ColumnEntry {
                    column_id: 2,
                    name: "person_id".into(),
                    logical: CoveLogicalType::Utf8,
                    physical: CovePhysicalKind::VarBytes,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
                ColumnEntry {
                    column_id: 3,
                    name: "score".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
            ],
        }],
    };
    let mut event_ids = Vec::new();
    let mut person_ids = Vec::new();
    let mut scores = Vec::new();
    for (event_id, person_id, score) in [
        (1001u64, "p1", 30u64),
        (1002, "p1", 15),
        (1003, "p2", 25),
        (1004, "p3", 5),
        (1005, "p4", 40),
    ] {
        event_ids.extend_from_slice(&event_id.to_le_bytes());
        person_ids.extend_from_slice(&(person_id.len() as u32).to_le_bytes());
        person_ids.extend_from_slice(person_id.as_bytes());
        scores.extend_from_slice(&score.to_le_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, 5, 3);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(5, event_ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(5, person_ids).with_encoding_root(CoveEncodingKind::VarBytes as u32)],
    );
    segment.set_column_pages(
        3,
        vec![ScanPageSpec::new(5, scores).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn property_binding(name: &str, logical_type: &str) -> Value {
    json!({
        "assertion_id": name,
        "property_id": name,
        "property_name": name,
        "source_column": name,
        "logical_type": logical_type,
        "nullable": true,
        "conflict_policy": "reject_conflict"
    })
}

fn map_section(kind: SectionKind, mut value: Value) -> CovemapSection {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".to_string(),
            Value::String("org.coveformat.covemap.v2".to_string()),
        );
        object.insert(
            "section_id".to_string(),
            Value::Number((kind as u16).into()),
        );
    }
    let payload = serde_json::to_vec_pretty(&value).unwrap();
    CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: kind as u32,
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

fn sample_readme() -> &'static str {
    r#"# Beginner CoveQL Samples

Files:

- people.cove: COVE-O object sample generated from people.jsonl and people.covemap.
- events.cove: COVE-T table sample with an events table.
- people.covemap: reusable COVE-MAP mapping used to build people.cove.
- people.jsonl: source rows for the object sample.

Try from the repository root:

```bash
cargo run -p cove-cli -- examples
cargo run -p cove-cli -- doctor examples/coveql/people.cove
cargo run -p cove-cli -- inspect examples/coveql/people.cove --queries
cargo run -p cove-cli -- query examples/coveql/people.cove 'table(people).select(score, status, nickname).take(5)'
cargo run -p cove-cli -- query examples/coveql/events.cove 'table(events).where(score >= 20).select(id, score)'
cargo run -p cove-cli -- query examples/coveql/events.cove --engine physical 'table(events).where(score >= 20).select(id, score)'
cargo run -p cove-cli -- query examples/coveql/people.cove 'node(Person) as p.degree(kind: total).select(id: p.goid, degree).take(3)'
printf 'id,score\n1,10\n2,20\n' > /tmp/coveql-people.csv
cargo run -p cove-cli -- query --external-table people=/tmp/coveql-people.csv 'table(people).where(score >= 20).select(id, score)'
```

Regenerate these files with:

```bash
cargo run -p cove-cli --example generate_beginner_samples -- examples/coveql
```
"#
}

fn showcase_readme() -> &'static str {
    r#"# COVE Reference Showcase

This directory is a compact end-to-end showcase for the unified `cove` CLI.

Files:

- `crm_people.jsonl`: CRM source rows with names, regions, and tiers.
- `support_people.jsonl`: support source rows with activity, scores, and statuses.
- `customer_identity.covemap`: COVE-MAP identity and projection metadata for both sources.
- `customers_360.jsonl`: reconciled readback rows derived from the two source shapes.
- `customer_readback.covemap`: COVE-MAP metadata used to generate the queryable COVE-O file.
- `customers.cove`: generated COVE-O object file with projection and evidence metadata.
- `events.cove`: companion COVE-T table with event scores by source person id.

Try from the repository root:

```bash
cargo run -p cove-cli -- doctor examples/showcase/customers.cove
cargo run -p cove-cli -- inspect --queries --performance examples/showcase/customers.cove
cargo run -p cove-cli -- map project --format json examples/showcase/customer_identity.covemap examples/showcase/crm_people.jsonl examples/showcase/support_people.jsonl
cargo run -p cove-cli -- query examples/showcase/customers.cove 'table(customers).select(full_name, region, tier, score, status).orderBy(score, desc)'
cargo run -p cove-cli -- query examples/showcase/customers.cove 'evidence().select(source_id, source_row_identity, rule_id).take(10)'
cargo run -p cove-cli -- query examples/showcase/events.cove 'table(events).where(score >= 25).select(event_id, person_id, score)'
cargo run -p cove-cli -- optimize examples/showcase/events.cove
cargo run -p cove-cli -- query --engine compare --perf-report examples/showcase/events.cove 'table(events).where(score >= 25).select(event_id, person_id, score)'
cargo run -p cove-cli -- query --format jsonl examples/showcase/customers.cove 'table(customers).select(full_name, score, status)'
cargo run -p cove-cli -- query --format csv examples/showcase/events.cove 'table(events).select(event_id, person_id, score)'
```

Regenerate these files with:

```bash
cargo run -p cove-cli --example generate_beginner_samples -- --showcase examples/showcase
```
"#
}
