use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

use cove_cache::CoveCoverageCacheHeaderV2;
use cove_core::{
    artifact::{
        covemap::{CovemapFile, CovemapHeaderV1, CovemapPostscriptV1},
        covm::{CovmFile, CovmFileEntryV1, CovmHeaderV1, CovmPostscriptV1},
    },
    constants::{
        CoveEncodingKind, CoveLogicalType, CovePhysicalKind, DigestAlgorithm, PrimaryProfile,
        FEATURE_SEMANTIC_MAP,
    },
    digest::compute_digest,
    reader::validate_bytes,
    table::{ColumnEntry, TableCatalog, TableEntry},
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment},
};
use cove_coverage::{
    CoverageExactnessV2, CoverageGranularityV2, CoverageProofStrengthV2,
    CoverageProviderDescriptorV2,
};
use cove_layout::{LayoutPlanHeaderV2, LayoutPlanNodeV2, LayoutPlanV2};
use cove_runtime::{RuntimeCompatibilityHintV2, RuntimeHintKindV2};

fn temp_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cove-cli-smoke-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn cove_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cove"))
}

fn sample_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/coveql")
        .join(name)
}

fn showcase_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/showcase")
        .join(name)
}

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_cove(args: &[&str]) -> Output {
    Command::new(cove_bin()).args(args).output().unwrap()
}

fn run_cove_with_stdin(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(cove_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn cove_t_events_bytes() -> Vec<u8> {
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

fn cove_t_labels_bytes() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "labels".into(),
            row_count: 2,
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
                    name: "label".into(),
                    logical: CoveLogicalType::Utf8,
                    physical: CovePhysicalKind::VarBytes,
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
    for id in [1u64, 2] {
        ids.extend_from_slice(&id.to_le_bytes());
    }
    let mut labels = Vec::new();
    for label in ["supercalifragilistic", "short"] {
        labels.extend_from_slice(&(label.len() as u32).to_le_bytes());
        labels.extend_from_slice(label.as_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, 2, 2);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(2, ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(2, labels).with_encoding_root(CoveEncodingKind::VarBytes as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn cove_t_relational_bytes() -> Vec<u8> {
    fn column(column_id: u32, name: &str) -> ColumnEntry {
        ColumnEntry {
            column_id,
            name: name.into(),
            logical: CoveLogicalType::Int64,
            physical: CovePhysicalKind::NumCode,
            nullable: false,
            sort_order: 0,
            collation_id: 0,
            precision: 0,
            scale: 0,
            flags: 0,
        }
    }

    let catalog = TableCatalog {
        flags: 0,
        tables: vec![
            TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "people".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(1, "id"), column(2, "score")],
            },
            TableEntry {
                table_id: 2,
                namespace: "public".into(),
                name: "bonus".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(1, "id"), column(2, "bonus")],
            },
            TableEntry {
                table_id: 3,
                namespace: "public".into(),
                name: "people_more".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(1, "id"), column(2, "score")],
            },
        ],
    };

    fn numcode_page(values: &[u64]) -> ScanPageSpec {
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        ScanPageSpec::new(values.len() as u32, bytes)
            .with_encoding_root(CoveEncodingKind::NumCode as u32)
    }

    fn segment(table_id: u32, rows: &[(u64, u64)]) -> ScanSegment {
        let mut segment = ScanSegment::new(table_id, 0, 0, rows.len() as u32, 2);
        let left = rows.iter().map(|(left, _)| *left).collect::<Vec<_>>();
        let right = rows.iter().map(|(_, right)| *right).collect::<Vec<_>>();
        segment.set_column_pages(1, vec![numcode_page(&left)]);
        segment.set_column_pages(2, vec![numcode_page(&right)]);
        segment
    }

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment(1, &[(1, 10), (2, 20)]));
    writer.push_segment(segment(2, &[(2, 5), (3, 7)]));
    writer.push_segment(segment(3, &[(2, 20), (3, 30)]));
    writer.write().unwrap()
}

fn cove_t_medium_bytes() -> Vec<u8> {
    fn column(column_id: u32, name: &str) -> ColumnEntry {
        ColumnEntry {
            column_id,
            name: name.into(),
            logical: CoveLogicalType::Int64,
            physical: CovePhysicalKind::NumCode,
            nullable: false,
            sort_order: 0,
            collation_id: 0,
            precision: 0,
            scale: 0,
            flags: 0,
        }
    }

    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "metrics".into(),
            row_count: 25,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(1, "id"), column(2, "bucket"), column(3, "score")],
        }],
    };
    let mut ids = Vec::new();
    let mut buckets = Vec::new();
    let mut scores = Vec::new();
    for id in 1u64..=25 {
        ids.extend_from_slice(&id.to_le_bytes());
        buckets.extend_from_slice(&(id % 5).to_le_bytes());
        scores.extend_from_slice(&(id * 3).to_le_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, 25, 3);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(25, ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(25, buckets).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        3,
        vec![ScanPageSpec::new(25, scores).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn covemap_bytes() -> Vec<u8> {
    CovemapFile {
        header: CovemapHeaderV1::new([0x11; 16], 0),
        mapping_version: "test/v1".into(),
        sections: Vec::new(),
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

fn coverage_provider_bytes() -> Vec<u8> {
    CoverageProviderDescriptorV2 {
        provider_id: 1,
        provider_kind: 0,
        profile: PrimaryProfile::TableScan as u8,
        granularity: CoverageGranularityV2::File,
        proof_strength: CoverageProofStrengthV2::ExactTight,
        exactness: CoverageExactnessV2::Exact,
        flags: 0,
        referenced_table_id: 1,
        referenced_column_id: u32::MAX,
        referenced_path_ref: u32::MAX,
        logical_type: 0,
        collation_id: 0,
        null_semantics: 0,
        snapshot_validity_ref: u32::MAX,
        predicate_form_ref: u32::MAX,
        producer_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .to_vec()
}

fn layout_plan_bytes() -> Vec<u8> {
    LayoutPlanV2 {
        header: LayoutPlanHeaderV2 {
            layout_id: 1,
            node_count: 1,
            root_node_id: 1,
            flags: 0,
            checksum: 0,
        },
        nodes: vec![LayoutPlanNodeV2 {
            node_id: 1,
            parent_node_id: u32::MAX,
            node_kind: 0,
            flags: 0,
            table_id: u32::MAX,
            column_id: u32::MAX,
            segment_id: u32::MAX,
            first_morsel_id: 0,
            morsel_count: 0,
            row_start: 0,
            row_count: 0,
            section_id: u32::MAX,
            cluster_id: u32::MAX,
            first_child_index: 0,
            child_count: 0,
            stats_ref: u32::MAX,
            split_ref: u32::MAX,
            checksum: 0,
        }],
    }
    .serialize()
    .unwrap()
}

fn cache_bytes() -> Vec<u8> {
    CoveCoverageCacheHeaderV2 {
        cache_format_namespace_ref: 0,
        cache_format_version_major: 1,
        cache_format_version_minor: 0,
        flags: 0,
        cache_id: [1; 16],
        dataset_id: [2; 16],
        snapshot_id: [3; 16],
        entry_count: 0,
        created_at_us: 0,
        producer_engine_ref: u32::MAX,
        reserved: [0; 32],
        checksum: 0,
    }
    .serialize()
    .to_vec()
}

fn runtime_hint_bytes() -> Vec<u8> {
    RuntimeCompatibilityHintV2 {
        hint_id: 1,
        hint_kind: RuntimeHintKindV2::EngineAdapter,
        required: true,
        flags: 0,
        namespace: "org.cove".into(),
        name: "adapter".into(),
        version_major: 1,
        version_minor: 0,
        payload_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .unwrap()
}

fn covm_manifest_for_members(members: &[(&str, &[u8])]) -> Vec<u8> {
    let files = members
        .iter()
        .map(|(uri, bytes)| {
            let validated = validate_bytes(bytes).unwrap();
            CovmFileEntryV1 {
                file_id: validated.header.file_id,
                uri: (*uri).into(),
                file_len: validated.postscript.file_len,
                footer_crc32c: validated.postscript.footer.crc32c,
                digest_algorithm: DigestAlgorithm::Sha256 as u16,
                digest: compute_digest(DigestAlgorithm::Sha256, bytes).unwrap(),
                row_count: 0,
                segment_count: 0,
                file_stats_ref: 0,
                file_exact_set_ref: 0,
                flags: 0,
            }
        })
        .collect::<Vec<_>>();
    CovmFile {
        header: CovmHeaderV1::new([0xC1; 16], 1, files.len() as u32, 1_700_000_000_000_000),
        files,
        postscript: CovmPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

#[test]
fn inspect_and_query_checked_in_cove_o_sample() {
    let people = sample_path("people.cove");
    let people = people.to_str().unwrap();

    let inspect = run_cove(&["inspect", people, "--queries"]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspect_stdout.contains("COVE-O"), "stdout={inspect_stdout}");
    assert!(
        inspect_stdout.contains("object(Person)"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout.contains("projection(`people_primitives.v1`)"),
        "stdout={inspect_stdout}"
    );

    let projection = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "projection(`people_primitives.v1`).where(score >= 20).select(score, status, nickname).take(2)",
    ]);
    assert!(
        projection.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&projection.stderr)
    );
    let projection_stdout = String::from_utf8_lossy(&projection.stdout);
    assert!(projection_stdout.contains(r#""score":30"#));
    assert!(projection_stdout.contains(r#""status":"closed""#));

    let evidence = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "evidence(Person, grain: object).select(source_id, source_row_identity).take(1)",
    ]);
    assert!(
        evidence.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&evidence.stderr)
    );
    let evidence_stdout = String::from_utf8_lossy(&evidence.stdout);
    assert!(evidence_stdout.contains(r#""source_id":"people""#));
    assert!(evidence_stdout.contains(r#""source_row_identity":"people:"#));
}

#[test]
fn cove_o_mapped_tables_expose_sql_like_query_suite() {
    let people = sample_path("people.cove");
    let people = people.to_str().unwrap();

    let inspect = run_cove(&["inspect", people, "--queries"]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        inspect_stdout.contains("Tables:"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout.contains("table(people)"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout.contains(
            "table(people).select(rows: count(*), total: sum(score), average: avg(score))"
        ),
        "stdout={inspect_stdout}"
    );

    let aggregate = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "table(people).select(rows: count(*), total: sum(score), average: avg(score))",
    ]);
    assert!(
        aggregate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&aggregate.stderr)
    );
    let aggregate_stdout = String::from_utf8_lossy(&aggregate.stdout);
    assert!(aggregate_stdout.contains(r#""rows":3"#));
    assert!(aggregate_stdout.contains(r#""total":60"#));

    let ordered = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "table(people).orderBy(score, desc).select(score, status).take(2)",
    ]);
    assert!(
        ordered.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&ordered.stderr)
    );
    let ordered_lines = String::from_utf8_lossy(&ordered.stdout)
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(ordered_lines[0].contains(r#""score":30"#));
    assert!(ordered_lines[1].contains(r#""score":20"#));

    let window = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "table(people).window(orderBy: score).select(score, rn: row_number()).take(2)",
    ]);
    assert!(
        window.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&window.stderr)
    );
    let window_stdout = String::from_utf8_lossy(&window.stdout);
    assert!(window_stdout.contains(r#""rn":1"#));
    assert!(window_stdout.contains(r#""rn":2"#));
}

#[test]
fn cove_t_tables_expose_relational_query_suite() {
    let file = temp_file("relational.cove");
    fs::write(&file, cove_t_relational_bytes()).unwrap();

    let join = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people) as p.join(table(bonus) as b, on: p.id == b.id, kind: inner).select(id: p.id, score: p.score, bonus: b.bonus)",
    ]);
    assert!(
        join.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&join.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&join.stdout).trim(),
        r#"{"id":2,"score":20,"bonus":5}"#
    );

    let union = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people).union(table(people_more), all: false).select(id).orderBy(id)",
    ]);
    assert!(
        union.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&union.stderr)
    );
    let union_stdout = String::from_utf8_lossy(&union.stdout);
    assert_eq!(
        union_stdout.lines().collect::<Vec<_>>(),
        vec![r#"{"id":1}"#, r#"{"id":2}"#, r#"{"id":3}"#]
    );

    let window = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people).window(orderBy: score).select(id, rn: row_number())",
    ]);
    assert!(
        window.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&window.stderr)
    );
    let window_stdout = String::from_utf8_lossy(&window.stdout);
    assert!(window_stdout.contains(r#""rn":1"#));
    assert!(window_stdout.contains(r#""rn":2"#));
}

#[test]
fn cove_t_tables_expose_full_relational_methods() {
    let file = temp_file("relational-full.cove");
    fs::write(&file, cove_t_relational_bytes()).unwrap();
    let file = file.to_str().unwrap();

    let lookup = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people) as p.lookup(table(bonus) as b, on: p.id == b.id, cardinality: many).select(id: p.id, bonus: b.bonus).orderBy(id)",
    ]);
    assert!(
        lookup.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&lookup.stderr)
    );
    let lookup_stdout = String::from_utf8_lossy(&lookup.stdout);
    assert!(lookup_stdout.contains(r#""id":1,"bonus":null"#));
    assert!(lookup_stdout.contains(r#""id":2,"bonus":5"#));

    let semi = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people) as p.semiJoin(table(bonus) as b, on: p.id == b.id).select(id: p.id)",
    ]);
    assert!(
        semi.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&semi.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&semi.stdout).trim(), r#"{"id":2}"#);

    let anti = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people) as p.antiJoin(table(bonus) as b, on: p.id == b.id).select(id: p.id)",
    ]);
    assert!(
        anti.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&anti.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&anti.stdout).trim(), r#"{"id":1}"#);

    let intersect = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people).intersect(table(people_more), all: false).select(id).orderBy(id)",
    ]);
    assert!(
        intersect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&intersect.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&intersect.stdout).trim(),
        r#"{"id":2}"#
    );

    let except = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people).except(table(people_more), all: false).select(id).orderBy(id)",
    ]);
    assert!(
        except.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&except.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&except.stdout).trim(),
        r#"{"id":1}"#
    );

    let recursive = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people).withRecursive(name: reach, seed: table(people), step: table(people_more), key: id, maxIterations: 4).join(table(reach) as r, on: people.id == r.id, kind: right, cardinality: many).select(id: r.id).orderBy(id)",
    ]);
    assert!(
        recursive.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&recursive.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&recursive.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![r#"{"id":1}"#, r#"{"id":2}"#, r#"{"id":3}"#]
    );
}

#[test]
fn query_supports_physical_compare_and_graph_algorithm_modes() {
    let file = temp_file("physical-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let physical = run_cove(&[
        "query",
        "--engine",
        "physical",
        "--batch-size",
        "2",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        physical.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&physical.stderr)
    );
    assert!(String::from_utf8_lossy(&physical.stdout).contains(r#""score":20"#));

    let people = sample_path("people.cove");
    let compare = run_cove(&[
        "query",
        "--compare",
        people.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people).where(score >= 20).select(score).take(2)",
    ]);
    assert!(
        compare.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&compare.stderr)
    );
    assert!(String::from_utf8_lossy(&compare.stdout).contains(r#""score":30"#));

    let graph = run_cove(&[
        "query",
        people.to_str().unwrap(),
        "--format",
        "jsonl",
        "node(Person) as p.connectedComponents().degree(kind: total).select(id: p.goid, component_id, degree).take(2)",
    ]);
    assert!(
        graph.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&graph.stderr)
    );
    let graph_stdout = String::from_utf8_lossy(&graph.stdout);
    assert!(graph_stdout.contains(r#""component_id":0"#));
    assert!(graph_stdout.contains(r#""degree":0"#));
}

#[test]
fn query_covm_manifest_registers_cove_t_member_tables() {
    let member = cove_t_events_bytes();
    let manifest = covm_manifest_for_members(&[("events.cove", &member)]);
    let manifest_path = temp_file("dataset.covm");
    let member_path = temp_file("events-member.cove");
    fs::write(&manifest_path, manifest).unwrap();
    fs::write(&member_path, member).unwrap();

    let output = run_cove(&[
        "query",
        manifest_path.to_str().unwrap(),
        "--member",
        &format!("events.cove={}", member_path.display()),
        "--format",
        "jsonl",
        "table(events).where(score >= 20).select(id, score)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(r#""id":2,"score":20"#));
    assert!(stdout.contains(r#""id":3,"score":30"#));
}

#[test]
fn query_external_file_backed_tables_without_cove_artifact() {
    let csv = temp_file("external-people.csv");
    fs::write(&csv, "id,score,active\n1,10,true\n2,20,false\n3,30,true\n").unwrap();
    let external_arg = format!("people={}", csv.display());
    let output = run_cove(&[
        "query",
        "--external-table",
        &external_arg,
        "--format",
        "jsonl",
        "table(people).where(score >= 20).select(id, score).orderBy(id)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![r#"{"id":2,"score":20}"#, r#"{"id":3,"score":30}"#]
    );

    let jsonl = temp_file("external-people.jsonl");
    fs::write(&jsonl, "{\"id\":1,\"score\":10}\n{\"id\":2,\"score\":20}\n").unwrap();
    let external_arg = format!("people={}", jsonl.display());
    let aggregate = run_cove(&[
        "query",
        "--external-table",
        &external_arg,
        "--format",
        "jsonl",
        "table(people).select(rows: count(*), total: sum(score))",
    ]);

    assert!(
        aggregate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&aggregate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&aggregate.stdout).trim(),
        r#"{"rows":2,"total":30}"#
    );
}

#[test]
fn query_external_tables_join_with_cove_t_tables() {
    let file = temp_file("events-with-external.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let weights = temp_file("weights.json");
    fs::write(&weights, r#"[{"id":2,"weight":5},{"id":3,"weight":7}]"#).unwrap();
    let external_arg = format!("weights={}", weights.display());

    let output = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--external-table",
        &external_arg,
        "--format",
        "jsonl",
        "table(events) as e.join(table(weights) as w, on: e.id == w.id, kind: inner).select(id: e.id, score: e.score, weight: w.weight).orderBy(id)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![
            r#"{"id":2,"score":20,"weight":5}"#,
            r#"{"id":3,"score":30,"weight":7}"#
        ]
    );
}

#[test]
fn query_file_and_stdin_queries_execute() {
    let file = temp_file("events-query-file.cove");
    let query_file = temp_file("events-query.coveql");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    fs::write(&query_file, "table(events).where(score >= 20).select(id)").unwrap();

    let from_file = run_cove(&[
        "query",
        "--query-file",
        query_file.to_str().unwrap(),
        "--format",
        "jsonl",
        file.to_str().unwrap(),
    ]);
    assert!(
        from_file.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&from_file.stderr)
    );
    assert!(String::from_utf8_lossy(&from_file.stdout).contains(r#""id":2"#));

    let from_stdin = run_cove_with_stdin(
        &[
            "query",
            "--query-file",
            "-",
            "--format",
            "jsonl",
            file.to_str().unwrap(),
        ],
        "table(events).where(score >= 30).select(score)",
    );
    assert!(
        from_stdin.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&from_stdin.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&from_stdin.stdout).trim(),
        r#"{"score":30}"#
    );
}

#[test]
fn table_output_respects_max_cell_width() {
    let file = temp_file("labels.cove");
    fs::write(&file, cove_t_labels_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        "--max-cell-width",
        "12",
        file.to_str().unwrap(),
        "table(labels).select(label)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("supercali..."), "stdout={stdout}");
    assert!(stdout.contains("long cells truncated"), "stdout={stdout}");
}

#[test]
fn inspect_cove_t_prints_query_suggestions() {
    let file = temp_file("events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&["inspect", file.to_str().unwrap(), "--queries"]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("COVE-T"), "stdout={stdout}");
    assert!(stdout.contains("table(events)"));
    assert!(stdout.contains("table(events).select(id, score).take(10)"));
}

#[test]
fn query_cove_t_outputs_jsonl_and_honors_take() {
    let file = temp_file("events-query.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "--take",
        "2",
        "table(events).select(id, score)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(r#""id":1"#));
    assert!(lines[1].contains(r#""score":20"#));
}

#[test]
fn query_cove_t_outputs_table_json_and_csv() {
    let file = temp_file("events-formats.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let table = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        table.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&table.stderr)
    );
    let table_stdout = String::from_utf8_lossy(&table.stdout);
    assert!(table_stdout.contains("| id | score |"));
    assert!(table_stdout.contains("2 rows"));

    let json = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "json",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        json.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&json.stderr)
    );
    let json_value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(json_value[0]["score"], serde_json::json!(20));

    let csv = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "csv",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        csv.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&csv.stderr)
    );
    let csv_stdout = String::from_utf8_lossy(&csv.stdout);
    assert!(csv_stdout.starts_with("id,score\n"));
    assert!(csv_stdout.contains("2,20\n"));
}

#[test]
fn query_cove_t_explain_prints_coveql_explain() {
    let file = temp_file("events-explain.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--explain",
        "coded",
        "table(events).select(id)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("schema_version: 0.1"), "stdout={stdout}");
    assert!(stdout.contains("mode: coded"), "stdout={stdout}");
    assert!(
        stdout.contains("operation: explain_table"),
        "stdout={stdout}"
    );
}

#[test]
fn optimize_generates_acceleration_manifest_and_auto_query_uses_it() {
    let file = temp_file("events-optimized.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let optimize = run_cove(&["optimize", file.to_str().unwrap()]);
    assert!(
        optimize.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&optimize.stderr)
    );
    let stdout = String::from_utf8_lossy(&optimize.stdout);
    assert!(stdout.contains("Generated sidecars"), "stdout={stdout}");

    let base = file.with_file_name("events-optimized");
    assert!(base.with_extension("covperf.json").exists());
    assert!(base.with_extension("covi").exists());
    assert!(base.with_extension("covx").exists());
    assert!(file.with_file_name("events-optimized.splits.bin").exists());
    assert!(file.with_file_name("events-optimized.layout.bin").exists());

    let inspect = run_cove(&["inspect", "--performance", file.to_str().unwrap()]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        inspect_stdout.contains("Performance:"),
        "stdout={inspect_stdout}"
    );
    assert!(inspect_stdout.contains("COVE-I"), "stdout={inspect_stdout}");

    let query = run_cove(&[
        "query",
        "--perf-report",
        "--strict-performance",
        "--format",
        "jsonl",
        file.to_str().unwrap(),
        "table(events).where(score >= 20).select(id, score).orderBy(id)",
    ]);
    assert!(
        query.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query_stdout = String::from_utf8_lossy(&query.stdout);
    assert!(query_stdout.contains("\"id\":2"), "stdout={query_stdout}");
    let query_stderr = String::from_utf8_lossy(&query.stderr);
    assert!(
        query_stderr.contains("Performance report"),
        "stderr={query_stderr}"
    );
    assert!(
        query_stderr.contains("usable sidecars"),
        "stderr={query_stderr}"
    );
}

#[test]
fn strict_performance_rejects_missing_acceleration_sidecars() {
    let file = temp_file("events-strict-missing.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        "--strict-performance",
        file.to_str().unwrap(),
        "table(events).select(id).take(1)",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("strict performance requested"),
        "stderr={stderr}"
    );
}

#[test]
fn query_covemap_sidecar_reports_guidance() {
    let file = temp_file("mapping.covemap");
    fs::write(&file, covemap_bytes()).unwrap();

    let output = run_cove(&["query", file.to_str().unwrap(), "table(events).take(1)"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("COVE-MAP mapping artifact"));
    assert!(stderr.contains("Use it with"));
}

#[test]
fn examples_and_doctor_commands_are_beginner_friendly() {
    let examples = run_cove(&["examples"]);
    assert!(
        examples.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&examples.stderr)
    );
    let examples_stdout = String::from_utf8_lossy(&examples.stdout);
    assert!(examples_stdout.contains("CoveQL examples"));
    assert!(examples_stdout.contains("cove inspect --queries --performance"));
    assert!(examples_stdout.contains("cove query examples/coveql/events.cove"));

    let examples_json = run_cove(&["examples", "--json"]);
    assert!(
        examples_json.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&examples_json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&examples_json.stdout).unwrap();
    assert_eq!(value["sample_dir"], serde_json::json!("examples/coveql"));
    assert!(value["examples"].as_array().unwrap().len() >= 4);

    let file = temp_file("doctor events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let doctor = run_cove(&["doctor", file.to_str().unwrap()]);
    assert!(
        doctor.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor_stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(doctor_stdout.contains("Doctor:"));
    assert!(doctor_stdout.contains("Queryable: yes"));
    assert!(doctor_stdout.contains("cove optimize"));
    assert!(doctor_stdout.contains("table(events).select(id, score).take(10)"));

    let doctor_json = run_cove(&["doctor", "--json", file.to_str().unwrap()]);
    assert!(
        doctor_json.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor_json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&doctor_json.stdout).unwrap();
    assert_eq!(value["queryable"], serde_json::json!(true));
    assert!(value["findings"].as_array().unwrap().iter().any(|finding| {
        finding
            .as_str()
            .unwrap_or_default()
            .contains("artifact exposes queryable rows")
    }));
}

#[test]
fn cli_outputs_have_stable_golden_shapes() {
    let file = temp_file("golden-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let file = file.to_str().unwrap();

    let table = run_cove(&[
        "query",
        file,
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        table.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&table.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&table.stdout),
        "+----+-------+\n| id | score |\n+----+-------+\n| 2  | 20    |\n| 3  | 30    |\n+----+-------+\n2 rows\n"
    );

    let csv = run_cove(&[
        "query",
        file,
        "--format",
        "csv",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        csv.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&csv.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&csv.stdout),
        "id,score\n2,20\n3,30\n"
    );

    let jsonl = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        jsonl.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&jsonl.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&jsonl.stdout),
        "{\"id\":2,\"score\":20}\n{\"id\":3,\"score\":30}\n"
    );
}

#[test]
fn cli_negative_paths_report_actionable_errors() {
    let missing = run_cove(&["inspect", "/tmp/cove-cli-definitely-missing-file.cove"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot read"));

    let mixed_inspect = run_cove(&["inspect", "--queries", "--sections", "stats", "file.cove"]);
    assert!(!mixed_inspect.status.success());
    let stderr = String::from_utf8_lossy(&mixed_inspect.stderr);
    assert!(
        stderr.contains("cannot be combined") && stderr.contains("--sections"),
        "stderr={stderr}"
    );

    let bad_flag = run_cove(&["query", "--wat", "table(events).take(1)"]);
    assert!(!bad_flag.status.success());
    assert!(String::from_utf8_lossy(&bad_flag.stderr).contains("unknown query option"));

    let bad_format = run_cove(&[
        "query",
        "--external-table",
        "people=/tmp/nope.jsonl",
        "--format",
        "xml",
        "table(people).take(1)",
    ]);
    assert!(!bad_format.status.success());
    assert!(String::from_utf8_lossy(&bad_format.stderr).contains("unsupported --format"));

    let file = temp_file("negative-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let malformed = run_cove(&["query", file.to_str().unwrap(), "table(events).where("]);
    assert!(!malformed.status.success());
    let stderr = String::from_utf8_lossy(&malformed.stderr);
    assert!(stderr.contains("E_PARSE"), "stderr={stderr}");
    assert!(
        stderr.contains("Check the CoveQL syntax"),
        "stderr={stderr}"
    );

    let people = sample_path("people.cove");
    let unknown_table = run_cove(&[
        "query",
        people.to_str().unwrap(),
        "table(missing).select(id).take(1)",
    ]);
    assert!(!unknown_table.status.success());
    let stderr = String::from_utf8_lossy(&unknown_table.stderr);
    assert!(
        stderr.contains("E_UNKNOWN_TABLE_SURFACE"),
        "stderr={stderr}"
    );
    assert!(stderr.contains("cove inspect --queries"), "stderr={stderr}");

    let bad_jsonl = temp_file("bad-external.jsonl");
    fs::write(&bad_jsonl, "{\"id\":1}\nnot-json\n").unwrap();
    let external_arg = format!("bad={}", bad_jsonl.display());
    let bad_external = run_cove(&[
        "query",
        "--external-table",
        &external_arg,
        "table(bad).select(id)",
    ]);
    assert!(!bad_external.status.success());
    assert!(String::from_utf8_lossy(&bad_external.stderr).contains("cannot parse JSONL"));
}

#[test]
fn parent_command_help_exits_successfully() {
    for args in [
        &["convert", "--help"][..],
        &["convert"][..],
        &["export", "--help"][..],
        &["export"][..],
        &["perf", "--help"][..],
        &["perf"][..],
        &["digest", "--help"][..],
        &["digest"][..],
    ] {
        let output = run_cove(args);
        assert!(
            output.status.success(),
            "args={args:?} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage:"), "args={args:?} stdout={stdout}");
    }
}

#[test]
fn command_specific_help_surfaces_reference_workflows() {
    let cases = [
        (
            &["query", "--help"][..],
            "Authority model:",
            "sidecar-backed execution",
        ),
        (
            &["inspect", "--help"][..],
            "Beginner inspect",
            "--performance",
        ),
        (
            &["optimize", "--help"][..],
            "Writes a sibling",
            "materialized readback",
        ),
        (
            &["sidecar", "--help"][..],
            "cove sidecar build covi",
            "--all-columns",
        ),
        (
            &["convert", "--help"][..],
            "cove convert parquet",
            "cove-to-source",
        ),
        (
            &["map", "--help"][..],
            "cove map project",
            "cove map convert",
        ),
    ];
    for (args, expected_a, expected_b) in cases {
        let output = run_cove(args);
        assert!(
            output.status.success(),
            "args={args:?} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage:"), "args={args:?} stdout={stdout}");
        assert!(
            stdout.contains(expected_a) && stdout.contains(expected_b),
            "args={args:?} stdout={stdout}"
        );
    }
}

#[test]
fn release_gate_help_does_not_run_gate() {
    let output = Command::new("sh")
        .arg("scripts/release-gates.sh")
        .arg("--help")
        .current_dir(workspace_dir())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--quick"));
    assert!(!stdout.contains("cargo test"));
}

#[test]
fn paths_with_spaces_and_query_files_work() {
    let file = temp_file("events final (v1).cove");
    let query_file = temp_file("query with spaces.coveql");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    fs::write(
        &query_file,
        "table(events).where(score >= 30).select(id, score)",
    )
    .unwrap();

    let from_query_file = run_cove(&[
        "query",
        "--query-file",
        query_file.to_str().unwrap(),
        "--format",
        "jsonl",
        file.to_str().unwrap(),
    ]);
    assert!(
        from_query_file.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&from_query_file.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&from_query_file.stdout).trim(),
        r#"{"id":3,"score":30}"#
    );

    let inspect = run_cove(&["inspect", "--queries", file.to_str().unwrap()]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(stdout.contains("Try next:"));
    assert!(stdout.contains("events final (v1).cove"));
}

#[test]
fn medium_cove_t_fixture_covers_group_sort_and_windows() {
    let file = temp_file("metrics-medium.cove");
    fs::write(&file, cove_t_medium_bytes()).unwrap();
    let file = file.to_str().unwrap();

    let grouped = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(metrics).groupBy(bucket).select(bucket, rows: count(*), total: sum(score)).orderBy(bucket)",
    ]);
    assert!(
        grouped.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&grouped.stderr)
    );
    let grouped_stdout = String::from_utf8_lossy(&grouped.stdout);
    assert_eq!(grouped_stdout.lines().count(), 5);
    assert!(grouped_stdout.contains(r#""bucket":0,"rows":5"#));
    assert!(grouped_stdout.contains(r#""bucket":4,"rows":5"#));

    let window = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(metrics).where(bucket == 2).window(partitionBy: bucket, orderBy: score).select(bucket, score, rn: row_number()).take(3)",
    ]);
    assert!(
        window.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&window.stderr)
    );
    let window_stdout = String::from_utf8_lossy(&window.stdout);
    assert!(window_stdout.contains(r#""rn":1"#));
    assert!(window_stdout.contains(r#""rn":2"#));
    assert!(window_stdout.contains(r#""rn":3"#));
}

#[test]
fn doctor_reports_sidecar_guidance_for_nonqueryable_artifacts() {
    let file = temp_file("mapping-doctor.covemap");
    fs::write(&file, covemap_bytes()).unwrap();

    let output = run_cove(&["doctor", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Queryable: no"));
    assert!(stdout.contains("COVE-MAP mapping artifact"));

    let output = run_cove(&["doctor", "--json", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["queryable"], serde_json::json!(false));
}

#[test]
fn unified_file_utility_commands_cover_validate_inspect_dump_export_and_perf() {
    let file = temp_file("unified-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let path = file.to_str().unwrap();

    let validate = run_cove(&["validate", "--json", path]);
    assert!(
        validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&validate.stderr)
    );
    assert!(String::from_utf8_lossy(&validate.stdout).contains("\"ok\":true"));

    let inspect = run_cove(&["inspect", "--json", "--sections", "stats", path]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_json: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspect_json["artifact"], serde_json::json!("COVE"));

    let dump = run_cove(&["dump", path, "--metadata"]);
    assert!(
        dump.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&dump.stderr)
    );
    assert!(String::from_utf8_lossy(&dump.stdout).contains("metadata"));

    let exported = temp_file("events-export.json");
    let export = run_cove(&[
        "export",
        "arrow",
        "--format",
        "json",
        path,
        exported.to_str().unwrap(),
        "--report",
        "-",
    ]);
    assert!(
        export.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(exported.metadata().unwrap().len() > 0);
    assert!(String::from_utf8_lossy(&export.stdout).contains("\"rows\""));

    let pruning = run_cove(&["perf", "explain-pruning", path]);
    assert!(
        pruning.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&pruning.stderr)
    );
    assert!(String::from_utf8_lossy(&pruning.stdout).contains("\"version\""));

    let cost = run_cove(&["perf", "plan-cost", "--execute", path]);
    assert!(
        cost.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&cost.stderr)
    );
    assert!(String::from_utf8_lossy(&cost.stdout).contains("\"version\""));
}

#[test]
fn unified_convert_and_map_commands_delegate_existing_tools() {
    let csv = temp_file("unified-source.csv");
    let cove = temp_file("unified-source.cove");
    fs::write(&csv, "id,name\n1,Ada\n2,Linus\n").unwrap();

    let convert = run_cove(&[
        "convert",
        "csv",
        csv.to_str().unwrap(),
        cove.to_str().unwrap(),
        "--report",
        "-",
    ]);
    assert!(
        convert.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&convert.stderr)
    );
    assert!(cove.metadata().unwrap().len() > 0);
    assert!(String::from_utf8_lossy(&convert.stdout).contains("\"source_identifier\""));

    let report = run_cove(&[
        "convert",
        "report",
        "--source-format",
        "csv",
        csv.to_str().unwrap(),
    ]);
    assert!(
        report.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&report.stderr)
    );
    assert!(String::from_utf8_lossy(&report.stdout).contains("\"source_identifier\""));

    let map = temp_file("unified-map.covemap");
    fs::write(&map, covemap_bytes()).unwrap();
    let map_validate = run_cove(&["map", "validate", map.to_str().unwrap()]);
    assert!(
        map_validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&map_validate.stderr)
    );
    assert!(String::from_utf8_lossy(&map_validate.stdout).contains("\"ok\":true"));

    let map_preview = run_cove(&["map", "preview", map.to_str().unwrap()]);
    assert!(
        map_preview.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&map_preview.stderr)
    );
    assert!(String::from_utf8_lossy(&map_preview.stdout).contains("test/v1"));
}

#[test]
fn unified_sidecar_profile_digest_and_canonicalise_commands_work() {
    let file = temp_file("sidecar-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let covi = temp_file("sidecar-events.covi");
    let covi_build = run_cove(&[
        "sidecar",
        "build",
        "covi",
        file.to_str().unwrap(),
        covi.to_str().unwrap(),
        "--all-columns",
    ]);
    assert!(
        covi_build.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covi_build.stderr)
    );
    let covi_inspect = run_cove(&["sidecar", "inspect", "index", covi.to_str().unwrap()]);
    assert!(
        covi_inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covi_inspect.stderr)
    );
    assert!(String::from_utf8_lossy(&covi_inspect.stdout).contains("valid COVE-I"));

    let covm = temp_file("sidecar-events.covm");
    let covm_build = run_cove(&[
        "sidecar",
        "build",
        "covm",
        covm.to_str().unwrap(),
        file.to_str().unwrap(),
    ]);
    assert!(
        covm_build.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covm_build.stderr)
    );
    assert!(covm.metadata().unwrap().len() > 0);

    let covx = temp_file("sidecar-events.covx");
    let covx_build = run_cove(&[
        "sidecar",
        "build",
        "covx",
        covx.to_str().unwrap(),
        file.to_str().unwrap(),
    ]);
    assert!(
        covx_build.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covx_build.stderr)
    );
    assert!(covx.metadata().unwrap().len() > 0);

    for (kind, bytes, expected) in [
        ("coverage", coverage_provider_bytes(), "valid COVE-COVERAGE"),
        ("layout", layout_plan_bytes(), "valid COVE-L"),
        ("cache", cache_bytes(), "valid COVE-CACHE"),
        ("runtime", runtime_hint_bytes(), "valid COVE-R"),
    ] {
        let path = temp_file(&format!("{kind}.bin"));
        fs::write(&path, bytes).unwrap();
        let inspect = run_cove(&["sidecar", "inspect", kind, path.to_str().unwrap()]);
        assert!(
            inspect.status.success(),
            "kind={kind}\nstderr={}",
            String::from_utf8_lossy(&inspect.stderr)
        );
        assert!(
            String::from_utf8_lossy(&inspect.stdout).contains(expected),
            "stdout={}",
            String::from_utf8_lossy(&inspect.stdout)
        );
    }

    let digest = run_cove(&["digest", "verify", file.to_str().unwrap()]);
    assert!(
        digest.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&digest.stderr)
    );
    assert!(String::from_utf8_lossy(&digest.stdout).contains("missing_manifest"));

    let profile_section = temp_file("execution-code.bin");
    let profile = run_cove(&[
        "profile",
        "generate",
        "--kind",
        "execution-code",
        "--out",
        profile_section.to_str().unwrap(),
    ]);
    assert!(
        profile.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&profile.stderr)
    );
    let profile_validate = run_cove(&[
        "profile",
        "validate-section",
        profile_section.to_str().unwrap(),
        "--kind",
        "execution-code",
    ]);
    assert!(
        profile_validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&profile_validate.stderr)
    );

    let canonicalise = run_cove(&[
        "canonicalise",
        "validate-payload",
        "--tag",
        "int64",
        "--hex",
        "2a00000000000000",
    ]);
    assert!(
        canonicalise.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&canonicalise.stderr)
    );
    assert!(String::from_utf8_lossy(&canonicalise.stdout).contains("\"valid\": true"));
}

#[test]
fn checked_in_showcase_exercises_reference_workflow() {
    let customers = showcase_path("customers.cove");
    let events = showcase_path("events.cove");
    let map = showcase_path("customer_identity.covemap");
    let crm = showcase_path("crm_people.jsonl");
    let support = showcase_path("support_people.jsonl");

    for path in [&customers, &events, &map, &crm, &support] {
        assert!(
            path.exists(),
            "missing showcase artifact {}",
            path.display()
        );
    }

    let doctor = run_cove(&["doctor", customers.to_str().unwrap()]);
    assert!(
        doctor.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("Queryable: yes"));

    let inspect = run_cove(&[
        "inspect",
        "--queries",
        "--performance",
        customers.to_str().unwrap(),
    ]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspect_stdout.contains("table=customers"));
    assert!(inspect_stdout.contains("Show evidence rows"));

    let projected = run_cove(&[
        "map",
        "project",
        "--format",
        "json",
        map.to_str().unwrap(),
        crm.to_str().unwrap(),
        support.to_str().unwrap(),
    ]);
    assert!(
        projected.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&projected.stderr)
    );
    let projected_stdout = String::from_utf8_lossy(&projected.stdout);
    assert!(projected_stdout.contains("\"mapping_id\": \"showcase-customer-identity\""));
    assert!(projected_stdout.contains("\"output_table\": \"customers\""));

    let customer_query = run_cove(&[
        "query",
        customers.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(customers).select(full_name, score, status)",
    ]);
    assert!(
        customer_query.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&customer_query.stderr)
    );
    let customer_stdout = String::from_utf8_lossy(&customer_query.stdout);
    assert!(customer_stdout.contains("\"full_name\":\"Ada Lovelace\""));
    assert!(customer_stdout.contains("\"status\":\"dormant\""));

    let evidence = run_cove(&[
        "query",
        customers.to_str().unwrap(),
        "evidence().select(source_id, source_row_identity, rule_id).take(10)",
    ]);
    assert!(
        evidence.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&evidence.stderr)
    );
    assert!(String::from_utf8_lossy(&evidence.stdout).contains("customers_360"));

    let optimized_events = temp_file("showcase-events-for-optimize.cove");
    fs::copy(&events, &optimized_events).unwrap();
    let optimized = run_cove(&["optimize", optimized_events.to_str().unwrap()]);
    assert!(
        optimized.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&optimized.stderr)
    );
    assert!(String::from_utf8_lossy(&optimized.stdout).contains("Generated sidecars"));

    let compare = run_cove(&[
        "query",
        "--engine",
        "compare",
        "--perf-report",
        optimized_events.to_str().unwrap(),
        "table(events).where(score >= 25).select(event_id, person_id, score)",
    ]);
    assert!(
        compare.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&compare.stderr)
    );
    assert!(String::from_utf8_lossy(&compare.stdout).contains("1005"));
    assert!(String::from_utf8_lossy(&compare.stderr).contains("Performance report"));
}

#[test]
fn showcase_generator_rebuilds_valid_artifacts() {
    let out_dir = temp_file("regenerated-showcase");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args([
            "run",
            "-q",
            "-p",
            "cove-cli",
            "--example",
            "generate_beginner_samples",
            "--",
            "--showcase",
            out_dir.to_str().unwrap(),
        ])
        .current_dir(workspace_dir())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let generated_customers = out_dir.join("customers.cove");
    let generated_events = out_dir.join("events.cove");
    let validate_customers = run_cove(&[
        "validate",
        "--semantic",
        generated_customers.to_str().unwrap(),
    ]);
    assert!(
        validate_customers.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&validate_customers.stderr)
    );
    let query_events = run_cove(&[
        "query",
        generated_events.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(events).where(score >= 25).select(event_id, person_id, score)",
    ]);
    assert!(
        query_events.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&query_events.stderr)
    );
    assert!(String::from_utf8_lossy(&query_events.stdout).contains("\"event_id\":1005"));
}
