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
    let output = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("examples/coveql"));
    fs::create_dir_all(&output).unwrap();

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

Try from `v2/`:

```bash
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
