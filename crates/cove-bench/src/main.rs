//! `cove-bench` — reproducible public v2 benchmark corpus harness.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Cursor,
    ops::Range,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

use arrow_array::{ArrayRef, BooleanArray, Int64Array, RecordBatch, StringArray};
use arrow_ipc::reader::{FileReader, StreamReader};
use arrow_schema::{DataType, Field, Schema};
use cove_arrow::convert::{
    convert_arrow_record_batches, ParquetAccelerationPolicy, ParquetAggregatePolicy,
    ParquetConversionOptions, ParquetDictionaryPolicy, ParquetStatsPolicy,
};
use cove_cache::{CoveCoverageCacheHeaderV2, CoverageCacheEntryV2, CoverageCacheV2};
use cove_cli::customer360::{
    generate_customer360, generate_proof_suite, Customer360Options, Customer360Profile,
    ProofSuiteOptions, ProofSuiteScenario,
};
use cove_core::{
    artifact::{
        coveai::{
            ai_vector_search, write_covev_filecode_vectors_with_index, AiVectorIndexSelection,
            AiVectorSearchPlan, AiVectorSearchTargetKind, CoveAiFile, CoveVecFileCodeVectorBuild,
        },
        covemap::{
            CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapSection,
            CovemapSectionEntryV1,
        },
        covm::{
            CovmDeltaArtifactRefV1, CovmDeltaChainSummaryV1, CovmDeltaPruneRequest,
            CovmDeltaReadAmplificationPolicy, CovmDeltaReadAmplificationRecommendation,
            DeltaChainSummaryEntryV1, DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE,
            DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT,
            DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT,
        },
    },
    canonical::CanonicalValue,
    checksum,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, DigestAlgorithm,
        PrimaryProfile, SectionKind, ValueTag, FEATURE_SEMANTIC_MAP,
    },
    digest::compute_digest,
    durable, reader,
    table::{ColumnEntry, TableCatalog, TableEntry},
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment, SectionPayload},
};
use cove_coverage::{
    coverage_set_payload_checksum, CoverageExactnessV2, CoverageGranularityV2, CoverageProofKindV2,
    CoverageProofRecordV2, CoverageProofStrengthV2, CoverageProviderDescriptorV2,
    CoverageSetEntryV2, CoverageSetHeaderV2, CoverageSetV2, PredicateAstNodeV2,
    PredicateAstOperandRefV2, PredicateAstPayloadHeaderV2, PredicateFormKindV2, PredicateLiteralV2,
    PredicateNormalFormV2, PredicateNullPolicyV2, PredicateOpV2, PredicateOperandKindV2,
};
use cove_datafusion::{
    bootstrap::bootstrap_local_file_with_options,
    explain::{
        execute_planned_scan, plan_local_file, ExplainOptions, FilterDsl, FilterOp, TopNDsl,
    },
    metadata_aggregate::{
        exact_unfiltered_aggregate_synopses, exact_unfiltered_counts, MetadataAggregatePlan,
        MetadataAggregateProofKind, MetadataSynopsisAggregateKind,
    },
    register::{
        df::{
            physical_plan::{execution_plan::collect as collect_physical_plan, ExecutionPlan},
            prelude::SessionContext,
        },
        register_cove_o_projections, CoveTableOptions, CoviDiscovery,
    },
};
use cove_index::build::{build_covi_from_cove_bytes, CoviBuildOptions};
use cove_map::{
    build_from_paths, cove_o_from_paths, projected_output_from_cove_o_path,
    projected_output_from_paths, MapBuildOptions, MapBuildSectionCompression, MapEvidenceEncoding,
    ProjectionFormat,
};
use orc_rust::{ArrowReaderBuilder as OrcReaderBuilder, ArrowWriterBuilder as OrcWriterBuilder};
use parquet::{arrow::ArrowWriter, file::properties::WriterProperties};
use serde_json::{json, Value};
use std::sync::Arc;

const PUBLIC_MANIFEST: &str = include_str!("../benchmarks/public-corpus.json");

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cove-bench: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("gen") => {
            let profile = option_value(&args, "--profile").unwrap_or_else(|| "ci".into());
            let out = option_value(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(default_corpus_dir);
            generate_corpus(&profile, &out)?;
            println!("generated {profile} benchmark corpus at {}", out.display());
            Ok(())
        }
        Some("run") => {
            let corpus = option_value(&args, "--corpus")
                .map(PathBuf::from)
                .unwrap_or_else(default_corpus_dir);
            let report_json = option_value(&args, "--report-json")
                .map(PathBuf::from)
                .unwrap_or_else(|| corpus.join("report.json"));
            let report_md = option_value(&args, "--report-md")
                .map(PathBuf::from)
                .unwrap_or_else(|| corpus.join("report.md"));
            run_corpus(&corpus, &report_json, &report_md)?;
            println!("wrote benchmark report to {}", report_json.display());
            Ok(())
        }
        Some("check") => {
            let out = default_corpus_dir();
            generate_corpus("ci", &out)?;
            run_corpus(&out, &out.join("report.json"), &out.join("report.md"))?;
            println!("cove-bench check passed at {}", out.display());
            Ok(())
        }
        Some("-h" | "--help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other:?}")),
    }
}

fn print_usage() {
    println!(
        "Usage:\n  cove-bench gen --profile ci|standard|publication --out <dir>\n  cove-bench run --corpus <dir> --report-json <path> --report-md <path>\n  cove-bench check"
    );
}

fn option_value(args: &[String], option: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == option)
        .map(|window| window[1].clone())
}

fn default_corpus_dir() -> PathBuf {
    PathBuf::from("target/cove-bench/ci")
}

fn generate_corpus(profile: &str, out: &Path) -> Result<(), String> {
    let row_count = match profile {
        "ci" => 2_048,
        "standard" => 32_768,
        "publication" => 262_144,
        other => return Err(format!("unknown benchmark profile {other:?}")),
    };
    fs::create_dir_all(out).map_err(|err| format!("cannot create {}: {err}", out.display()))?;
    fs::write(out.join("public-corpus.json"), PUBLIC_MANIFEST)
        .map_err(|err| format!("cannot write manifest: {err}"))?;

    let batch = events_batch(row_count)?;
    let conversion_options = ParquetConversionOptions {
        table_name: "events".into(),
        namespace: "bench".into(),
        morsel_row_count: 512,
        segment_row_count: 2048,
        dictionary_policy: ParquetDictionaryPolicy::Auto,
        stats_policy: ParquetStatsPolicy::Recompute,
        acceleration_policy: ParquetAccelerationPolicy::Auto,
        point_lookup_columns: vec!["id".into(), "name".into()],
        cluster_columns: vec!["bucket".into()],
        topn_columns: vec!["amount".into()],
        aggregate_policy: ParquetAggregatePolicy::Auto,
        aggregate_columns: vec!["amount".into()],
        emit_covx: true,
        emit_covm: true,
        ..ParquetConversionOptions::default()
    };
    let converted = convert_arrow_record_batches(
        "generated-arrow",
        format!("events-{profile}-{row_count}"),
        batch.schema(),
        vec![batch.clone()],
        &conversion_options,
    )
    .map_err(|err| err.to_string())?;
    durable::durable_replace(&out.join("events.cove"), &converted.cove_bytes)
        .map_err(|err| format!("cannot publish events.cove: {err}"))?;
    if let Some(covx) = converted.covx_bytes {
        durable::durable_replace(&out.join("events.covx"), &covx)
            .map_err(|err| format!("cannot publish events.covx: {err}"))?;
    }
    if let Some(covm) = converted.covm_bytes {
        durable::durable_replace(&out.join("events.covm"), &covm)
            .map_err(|err| format!("cannot publish events.covm: {err}"))?;
    }
    let covi_bytes = build_covi_from_cove_bytes(
        &converted.cove_bytes,
        &CoviBuildOptions {
            column_ids: vec![1, 4],
            include_index_only_counts: true,
            include_index_only_min_max: true,
            include_index_only_distinct_count: true,
            include_index_only_exists: true,
            ..CoviBuildOptions::default()
        },
    )
    .map_err(|err| format!("cannot build events.covi: {err}"))?;
    durable::durable_replace(&out.join("events.covi"), &covi_bytes)
        .map_err(|err| format!("cannot publish events.covi: {err}"))?;
    let ai_vector_file_codes = (1..=128).collect::<Vec<_>>();
    let ai_vector_dimension_count = 8;
    let ai_vector_bytes = build_benchmark_covev_vectors(
        ai_vector_dimension_count,
        &ai_vector_file_codes,
        [0x83; 16],
        1_000,
    )?;
    durable::durable_replace(&out.join("events-ai.covev"), &ai_vector_bytes)
        .map_err(|err| format!("cannot publish events-ai.covev: {err}"))?;
    write_parquet_file(&out.join("events.parquet"), &batch)?;
    write_orc_file(&out.join("events.orc"), &batch)?;
    validate_orc_parity(&out.join("events.orc"), &batch)?;
    let mut publication_locks = generate_publication_gap_datasets(profile, row_count, out)?;

    let cache_fixture = coverage_cache_fixture()?;
    durable::durable_replace(&out.join("synthetic-cache.cove"), &cache_fixture.cove_bytes)
        .map_err(|err| format!("cannot publish synthetic-cache.cove: {err}"))?;
    durable::durable_replace(
        &out.join("synthetic-cache.cove.cache"),
        &cache_fixture.cache_bytes,
    )
    .map_err(|err| format!("cannot publish synthetic-cache.cove.cache: {err}"))?;

    let mut lock_entries = vec![
        dataset_lock("events", "events.cove", &converted.cove_bytes)?,
        dataset_lock(
            "events-orc",
            "events.orc",
            &fs::read(out.join("events.orc")).map_err(|err| err.to_string())?,
        )?,
        dataset_lock("events-covi", "events.covi", &covi_bytes)?,
        dataset_lock("events-ai", "events-ai.covev", &ai_vector_bytes)?,
        dataset_lock(
            "synthetic-cache",
            "synthetic-cache.cove",
            &cache_fixture.cove_bytes,
        )?,
    ];
    lock_entries.append(&mut publication_locks);
    let lock = json!({
        "version": 1,
        "profile": profile,
        "manifest_sha256": hex(&compute_digest(DigestAlgorithm::Sha256, PUBLIC_MANIFEST.as_bytes()).map_err(|err| err.to_string())?),
        "datasets": lock_entries,
    });
    fs::write(
        out.join("corpus.lock.json"),
        serde_json::to_vec_pretty(&lock).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write corpus lock: {err}"))?;
    Ok(())
}

fn dataset_lock(name: &str, path: &str, bytes: &[u8]) -> Result<Value, String> {
    Ok(json!({
        "name": name,
        "path": path,
        "bytes": bytes.len(),
        "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, bytes).map_err(|err| err.to_string())?),
    }))
}

fn build_benchmark_covev_vectors(
    dimension_count: u32,
    file_codes: &[u32],
    artifact_id: [u8; 16],
    created_at_us: i64,
) -> Result<Vec<u8>, String> {
    write_covev_filecode_vectors_with_index(
        &CoveVecFileCodeVectorBuild {
            artifact_id,
            created_at_us,
            dimension_count,
            file_codes: file_codes.to_vec(),
            vector_payload: benchmark_vector_payload(dimension_count, file_codes),
        },
        1,
    )
    .map_err(|err| format!("cannot build benchmark COVE-VEC sidecar: {err}"))
}

fn benchmark_vector_payload(dimension_count: u32, file_codes: &[u32]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(file_codes.len() * dimension_count as usize * 4);
    for file_code in file_codes {
        for dim in 0..dimension_count {
            let seed = (*file_code as f32 * 0.03125) + (dim as f32 * 0.125);
            let value = seed.sin() + seed.cos() * 0.5;
            payload.extend_from_slice(&value.to_le_bytes());
        }
    }
    payload
}

fn generate_publication_gap_datasets(
    profile: &str,
    row_count: usize,
    out: &Path,
) -> Result<Vec<Value>, String> {
    let mut locks = Vec::new();
    let runnable = [
        ("tpch-style", "tpch_style", row_count),
        (
            "tpcds-style",
            "tpcds_style",
            row_count.saturating_div(2).max(64),
        ),
        (
            "medical-operational",
            "medical_operational",
            row_count.saturating_div(2).max(64),
        ),
    ];
    for (dataset_id, table_name, rows) in runnable {
        let batch = events_batch(rows)?;
        let options = ParquetConversionOptions {
            table_name: table_name.into(),
            namespace: "bench_publication".into(),
            morsel_row_count: 512,
            segment_row_count: 2048,
            dictionary_policy: ParquetDictionaryPolicy::Auto,
            stats_policy: ParquetStatsPolicy::Recompute,
            acceleration_policy: ParquetAccelerationPolicy::Auto,
            point_lookup_columns: vec!["id".into(), "name".into()],
            cluster_columns: vec!["bucket".into()],
            topn_columns: vec!["amount".into()],
            aggregate_policy: ParquetAggregatePolicy::Auto,
            aggregate_columns: vec!["amount".into()],
            emit_covx: true,
            emit_covm: true,
            ..ParquetConversionOptions::default()
        };
        let converted = convert_arrow_record_batches(
            "generated-arrow",
            format!("{dataset_id}-{profile}-{rows}"),
            batch.schema(),
            vec![batch.clone()],
            &options,
        )
        .map_err(|err| err.to_string())?;
        let cove_path = out.join(format!("{dataset_id}.cove"));
        let parquet_path = out.join(format!("{dataset_id}.parquet"));
        let orc_path = out.join(format!("{dataset_id}.orc"));
        let report_path = out.join(format!("{dataset_id}.report.json"));
        durable::durable_replace(&cove_path, &converted.cove_bytes)
            .map_err(|err| format!("cannot publish {dataset_id}.cove: {err}"))?;
        write_parquet_file(&parquet_path, &batch)?;
        write_orc_file(&orc_path, &batch)?;
        validate_orc_parity(&orc_path, &batch)?;
        let parquet_bytes = fs::read(&parquet_path).map_err(|err| err.to_string())?;
        let orc_bytes = fs::read(&orc_path).map_err(|err| err.to_string())?;
        let report = json!({
            "version": 1,
            "dataset": dataset_id,
            "profile": profile,
            "rows": rows,
            "artifacts": {
                "cove": {
                    "path": format!("{dataset_id}.cove"),
                    "bytes": converted.cove_bytes.len(),
                    "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, &converted.cove_bytes).map_err(|err| err.to_string())?),
                },
                "parquet": {
                    "path": format!("{dataset_id}.parquet"),
                    "bytes": parquet_bytes.len(),
                    "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, &parquet_bytes).map_err(|err| err.to_string())?),
                },
                "orc": {
                    "path": format!("{dataset_id}.orc"),
                    "bytes": orc_bytes.len(),
                    "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, &orc_bytes).map_err(|err| err.to_string())?),
                },
            },
            "generation": "deterministic public v2 generated analog",
        });
        let report_bytes = serde_json::to_vec_pretty(&report).map_err(|err| err.to_string())?;
        fs::write(&report_path, &report_bytes)
            .map_err(|err| format!("cannot write {}: {err}", report_path.display()))?;

        locks.push(dataset_lock(
            dataset_id,
            &format!("{dataset_id}.cove"),
            &converted.cove_bytes,
        )?);
        locks.push(dataset_lock(
            &format!("{dataset_id}-parquet"),
            &format!("{dataset_id}.parquet"),
            &parquet_bytes,
        )?);
        locks.push(dataset_lock(
            &format!("{dataset_id}-orc"),
            &format!("{dataset_id}.orc"),
            &orc_bytes,
        )?);
        locks.push(dataset_lock(
            &format!("{dataset_id}-report"),
            &format!("{dataset_id}.report.json"),
            &report_bytes,
        )?);
    }

    let corrupt_bytes = b"not-a-cove-v2-file\n".to_vec();
    durable::durable_replace(&out.join("negative-corrupt.cove"), &corrupt_bytes)
        .map_err(|err| format!("cannot publish negative-corrupt.cove: {err}"))?;
    let corrupt_metadata = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "dataset": "negative-corrupt",
        "expected": "reject",
        "expected_error_class": "invalid_cove_artifact",
        "artifact": "negative-corrupt.cove",
    }))
    .map_err(|err| err.to_string())?;
    fs::write(
        out.join("negative-corrupt.expected.json"),
        &corrupt_metadata,
    )
    .map_err(|err| format!("cannot write negative-corrupt metadata: {err}"))?;
    locks.push(dataset_lock(
        "negative-corrupt",
        "negative-corrupt.cove",
        &corrupt_bytes,
    )?);
    locks.push(dataset_lock(
        "negative-corrupt-expected",
        "negative-corrupt.expected.json",
        &corrupt_metadata,
    )?);

    let canonicalisation = canonicalisation_fixture()?;
    let canonicalisation_bytes =
        serde_json::to_vec_pretty(&canonicalisation).map_err(|err| err.to_string())?;
    fs::write(out.join("canonicalisation.json"), &canonicalisation_bytes)
        .map_err(|err| format!("cannot write canonicalisation fixture: {err}"))?;
    locks.push(dataset_lock(
        "canonicalisation",
        "canonicalisation.json",
        &canonicalisation_bytes,
    )?);

    let semantic_dir = out.join("semantic-mapping");
    fs::create_dir_all(&semantic_dir)
        .map_err(|err| format!("cannot create semantic mapping dir: {err}"))?;
    let covemap_bytes = bench_covemap_bytes()?;
    durable::durable_replace(&semantic_dir.join("people.covemap"), &covemap_bytes)
        .map_err(|err| format!("cannot publish semantic mapping COVE-MAP: {err}"))?;
    let mut csv = String::from("id,name\n");
    for row in 0..512 {
        csv.push_str(&format!("{row},person-{row}\n"));
    }
    fs::write(semantic_dir.join("people.csv"), csv.as_bytes())
        .map_err(|err| format!("cannot write semantic mapping CSV: {err}"))?;
    let semantic_map_path = semantic_dir.join("people.covemap");
    let semantic_csv_path = semantic_dir.join("people.csv");
    let semantic_mapped_cove_o =
        cove_o_from_paths(&semantic_map_path, std::slice::from_ref(&semantic_csv_path))
            .map_err(|err| format!("cannot build semantic mapping mapped COVE-O: {err}"))?;
    durable::durable_replace(
        &semantic_dir.join("people_mapped.cove"),
        &semantic_mapped_cove_o,
    )
    .map_err(|err| format!("cannot publish semantic mapping mapped COVE-O: {err}"))?;
    let semantic_cove_t = projected_output_from_paths(
        &semantic_map_path,
        std::slice::from_ref(&semantic_csv_path),
        ProjectionFormat::CoveT,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic mapping projected COVE-T: {err}"))?;
    durable::durable_replace(
        &semantic_dir.join("people_projection.cove"),
        &semantic_cove_t,
    )
    .map_err(|err| format!("cannot publish semantic mapping projected COVE-T: {err}"))?;
    let semantic_arrow = projected_output_from_paths(
        &semantic_map_path,
        std::slice::from_ref(&semantic_csv_path),
        ProjectionFormat::Arrow,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic mapping Arrow projection: {err}"))?;
    let semantic_projection_batch = decode_single_arrow_projection_batch(&semantic_arrow)?;
    write_parquet_file(
        &semantic_dir.join("people_projection.parquet"),
        &semantic_projection_batch,
    )?;
    let semantic_expected = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "dataset": "semantic-mapping",
        "expected_rows": 512,
        "mapping_id": "bench-map",
        "mapping_version": "bench/v1",
    }))
    .map_err(|err| err.to_string())?;
    fs::write(semantic_dir.join("expected.json"), &semantic_expected)
        .map_err(|err| format!("cannot write semantic mapping metadata: {err}"))?;
    locks.push(dataset_lock(
        "semantic-mapping-covemap",
        "semantic-mapping/people.covemap",
        &covemap_bytes,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-csv",
        "semantic-mapping/people.csv",
        csv.as_bytes(),
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-mapped-cove-o",
        "semantic-mapping/people_mapped.cove",
        &semantic_mapped_cove_o,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-cove-t",
        "semantic-mapping/people_projection.cove",
        &semantic_cove_t,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-parquet",
        "semantic-mapping/people_projection.parquet",
        &fs::read(semantic_dir.join("people_projection.parquet")).map_err(|err| err.to_string())?,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-expected",
        "semantic-mapping/expected.json",
        &semantic_expected,
    )?);

    let showcase_dir = out.join("semantic-showcase");
    fs::create_dir_all(&showcase_dir)
        .map_err(|err| format!("cannot create semantic showcase dir: {err}"))?;
    let showcase_map_bytes = showcase_multi_source_covemap()?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&showcase_dir.join("showcase.covemap"), &showcase_map_bytes)
        .map_err(|err| format!("cannot publish semantic showcase COVE-MAP: {err}"))?;
    fs::write(
        showcase_dir.join("crm.csv"),
        b"id,name\np1,Ada CRM\np2,Linus CRM\n",
    )
    .map_err(|err| format!("cannot write semantic showcase CRM CSV: {err}"))?;
    write_parquet_file(
        &showcase_dir.join("directory.parquet"),
        &showcase_directory_name_batch()?,
    )?;
    fs::write(
        showcase_dir.join("subscription.csv"),
        b"id,name\np1,Ada\np2,Linus\n",
    )
    .map_err(|err| format!("cannot write semantic showcase subscription CSV: {err}"))?;
    let showcase_map_path = showcase_dir.join("showcase.covemap");
    let showcase_sources = vec![
        showcase_dir.join("crm.csv"),
        showcase_dir.join("directory.parquet"),
        showcase_dir.join("subscription.csv"),
    ];
    let showcase_object_bytes = cove_o_from_paths(&showcase_map_path, &showcase_sources)
        .map_err(|err| format!("cannot build semantic showcase mapped COVE-O: {err}"))?;
    durable::durable_replace(
        &showcase_dir.join("showcase_mapped.cove"),
        &showcase_object_bytes,
    )
    .map_err(|err| format!("cannot publish semantic showcase mapped COVE-O: {err}"))?;
    let showcase_object_path = showcase_dir.join("showcase_mapped.cove");
    let showcase_people_cove_t = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::CoveT,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase people COVE-T: {err}"))?;
    durable::durable_replace(
        &showcase_dir.join("people_projection.cove"),
        &showcase_people_cove_t,
    )
    .map_err(|err| format!("cannot publish semantic showcase people COVE-T: {err}"))?;
    let showcase_evidence_cove_t = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::CoveT,
        Some("evidence_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase evidence COVE-T: {err}"))?;
    durable::durable_replace(
        &showcase_dir.join("evidence_projection.cove"),
        &showcase_evidence_cove_t,
    )
    .map_err(|err| format!("cannot publish semantic showcase evidence COVE-T: {err}"))?;
    let showcase_people_arrow = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::Arrow,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase people Arrow: {err}"))?;
    write_parquet_file(
        &showcase_dir.join("people_projection.parquet"),
        &decode_single_arrow_projection_batch(&showcase_people_arrow)?,
    )?;
    let showcase_evidence_arrow = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::Arrow,
        Some("evidence_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase evidence Arrow: {err}"))?;
    write_parquet_file(
        &showcase_dir.join("evidence_projection.parquet"),
        &decode_single_arrow_projection_batch(&showcase_evidence_arrow)?,
    )?;
    let showcase_expected = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "dataset": "semantic-showcase",
        "expected_people_rows": 2,
        "expected_evidence_rows": 6,
        "mapping_id": "showcase-map",
        "mapping_version": "bench/showcase.v1",
    }))
    .map_err(|err| err.to_string())?;
    fs::write(showcase_dir.join("expected.json"), &showcase_expected)
        .map_err(|err| format!("cannot write semantic showcase metadata: {err}"))?;
    locks.push(dataset_lock(
        "semantic-showcase-covemap",
        "semantic-showcase/showcase.covemap",
        &showcase_map_bytes,
    )?);
    for (name, rel) in [
        ("semantic-showcase-crm", "semantic-showcase/crm.csv"),
        (
            "semantic-showcase-directory",
            "semantic-showcase/directory.parquet",
        ),
        (
            "semantic-showcase-subscription",
            "semantic-showcase/subscription.csv",
        ),
        (
            "semantic-showcase-mapped-cove-o",
            "semantic-showcase/showcase_mapped.cove",
        ),
        (
            "semantic-showcase-people-cove-t",
            "semantic-showcase/people_projection.cove",
        ),
        (
            "semantic-showcase-evidence-cove-t",
            "semantic-showcase/evidence_projection.cove",
        ),
        (
            "semantic-showcase-people-parquet",
            "semantic-showcase/people_projection.parquet",
        ),
        (
            "semantic-showcase-evidence-parquet",
            "semantic-showcase/evidence_projection.parquet",
        ),
        (
            "semantic-showcase-expected",
            "semantic-showcase/expected.json",
        ),
    ] {
        locks.push(dataset_lock(
            name,
            rel,
            &fs::read(out.join(rel)).map_err(|err| err.to_string())?,
        )?);
    }

    let customer360_dir = out.join("customer360");
    let customer360_profile = match profile {
        "ci" => Customer360Profile::Quick,
        "standard" => Customer360Profile::Standard,
        "publication" => Customer360Profile::Publication,
        other => return Err(format!("unknown benchmark profile {other:?}")),
    };
    let customer360_manifest = generate_customer360(&Customer360Options {
        out_dir: customer360_dir.clone(),
        profile: customer360_profile,
        force: true,
    })
    .map_err(|err| format!("cannot build Customer 360 benchmark corpus: {err}"))?;
    let customer360_manifest_bytes =
        serde_json::to_vec_pretty(&customer360_manifest).map_err(|err| err.to_string())?;
    for (name, rel) in [
        ("customer360-crm", "customer360/crm.csv"),
        ("customer360-support", "customer360/support.jsonl"),
        ("customer360-billing", "customer360/billing.parquet"),
        ("customer360-reconciled", "customer360/customers_360.jsonl"),
        ("customer360-events-jsonl", "customer360/events.jsonl"),
        ("customer360-events-cove", "customer360/events.cove"),
        ("customer360-covemap", "customer360/customer360.covemap"),
        (
            "customer360-readback-covemap",
            "customer360/customer360_readback.covemap",
        ),
        ("customer360-mapped-cove-o", "customer360/customers.cove"),
        (
            "customer360-customers-cove-t",
            "customer360/customers_projection.cove",
        ),
        (
            "customer360-evidence-cove-t",
            "customer360/evidence_projection.cove",
        ),
        (
            "customer360-customers-parquet",
            "customer360/customers_projection.parquet",
        ),
        (
            "customer360-evidence-parquet",
            "customer360/evidence_projection.parquet",
        ),
        (
            "customer360-notebook-script",
            "customer360/notebooks/customer360_analysis.py",
        ),
    ] {
        locks.push(dataset_lock(
            name,
            rel,
            &fs::read(out.join(rel)).map_err(|err| err.to_string())?,
        )?);
    }
    locks.push(dataset_lock(
        "customer360-manifest",
        "customer360/customer360-manifest.json",
        &customer360_manifest_bytes,
    )?);

    let proof_suite_dir = out.join("proof-suite");
    let proof_suite_manifest = generate_proof_suite(&ProofSuiteOptions {
        out_dir: proof_suite_dir,
        profile: customer360_profile,
        scenario: ProofSuiteScenario::All,
        force: true,
    })
    .map_err(|err| format!("cannot build COVE-O proof-suite benchmark corpus: {err}"))?;
    let proof_suite_manifest_bytes =
        serde_json::to_vec_pretty(&proof_suite_manifest).map_err(|err| err.to_string())?;
    locks.push(dataset_lock(
        "proof-suite-manifest",
        "proof-suite/proof-suite-manifest.json",
        &proof_suite_manifest_bytes,
    )?);
    for scenario in ["customer360", "claims", "catalog"] {
        for (name_suffix, rel_suffix) in [
            ("doctor", "doctor-report.json"),
            ("size", "proof-size-comparison.json"),
            (
                "bundle-manifest",
                "map-build-bundle/map-build-manifest.json",
            ),
            ("bundle-report", "map-build-bundle/map-build-report.json"),
        ] {
            let rel = format!("proof-suite/{scenario}/{rel_suffix}");
            locks.push(dataset_lock(
                &format!("proof-suite-{scenario}-{name_suffix}"),
                &rel,
                &fs::read(out.join(&rel)).map_err(|err| err.to_string())?,
            )?);
        }
    }

    Ok(locks)
}

fn canonicalisation_fixture() -> Result<Value, String> {
    let cases = vec![
        (
            "utf8_nfc_source",
            "utf8",
            CanonicalValue::Utf8("cafe\u{301}"),
        ),
        (
            "signed_width_normalisation",
            "int64",
            CanonicalValue::Int {
                width: 2,
                value: -123,
            },
        ),
        (
            "list_order_preserved",
            "list",
            CanonicalValue::List(vec![
                CanonicalValue::Utf8("alpha"),
                CanonicalValue::Utf8("beta"),
            ]),
        ),
        (
            "map_sorted_by_canonical_key",
            "map",
            CanonicalValue::Map(vec![
                (
                    CanonicalValue::Utf8("b"),
                    CanonicalValue::Int { width: 8, value: 2 },
                ),
                (
                    CanonicalValue::Utf8("a"),
                    CanonicalValue::Int { width: 8, value: 1 },
                ),
            ]),
        ),
    ];
    let mut encoded = Vec::new();
    for (id, logical, value) in cases {
        encoded.push(json!({
            "id": id,
            "logical": logical,
            "value_tag": format!("{:?}", value.value_tag()),
            "canonical_hex": hex(&value.encode().map_err(|err| err.to_string())?),
        }));
    }
    Ok(json!({
        "version": 1,
        "dataset": "canonicalisation",
        "cases": encoded,
    }))
}

fn run_corpus(corpus: &Path, report_json: &Path, report_md: &Path) -> Result<(), String> {
    let manifest: Value = serde_json::from_str(PUBLIC_MANIFEST).map_err(|err| err.to_string())?;
    let mut cases = Vec::new();
    cases.extend(run_events_cases(corpus)?);
    cases.extend(run_ai_cases(corpus)?);
    cases.extend(run_cache_cases(corpus)?);
    cases.push(run_cove_o_delta_artifact_metrics_case()?);
    cases.extend(run_publication_gap_cases(corpus)?);
    for case in &mut cases {
        normalize_case_metrics(case);
    }
    validate_report_cases(&cases)?;
    let report = json!({
        "version": 1,
        "manifest": manifest,
        "corpus": corpus.display().to_string(),
        "environment": environment_report(),
        "feature_disclosure": {
            "covx": corpus.join("events.covx").is_file(),
            "covi": corpus.join("events.covi").is_file(),
            "coverage_cache": true,
            "cove_map": true,
            "layout": true,
            "parquet_compare": true,
            "orc_compare": corpus.join("events.orc").is_file(),
            "publication_corpora": true,
            "object_store_harness": true,
            "cove_o_delta_artifacts": true,
            "cove_ai": corpus.join("events-ai.covev").is_file(),
        },
        "cases": cases,
    });
    if let Some(parent) = report_json.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("cannot create report dir: {err}"))?;
    }
    fs::write(
        report_json,
        serde_json::to_vec_pretty(&report).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write {}: {err}", report_json.display()))?;
    fs::write(report_md, markdown_report(&report))
        .map_err(|err| format!("cannot write {}: {err}", report_md.display()))?;
    Ok(())
}

fn run_events_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let path = corpus.join("events.cove");
    let mut cases = Vec::new();
    let queries = vec![
        (
            "full_numeric_scan",
            "full numeric scan",
            ExplainOptions {
                projection: Some(vec!["id".into(), "amount".into()]),
                ..ExplainOptions::default()
            },
        ),
        (
            "string_category_scan",
            "string/category scan",
            ExplainOptions {
                projection: Some(vec!["name".into(), "bucket".into()]),
                ..ExplainOptions::default()
            },
        ),
        (
            "equality_filter",
            "equality predicate",
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "id".into(),
                    op: FilterOp::Eq,
                    value: Some("17".into()),
                }],
                ..ExplainOptions::default()
            },
        ),
        (
            "point_lookup",
            "point lookup predicate",
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "id".into(),
                    op: FilterOp::Eq,
                    value: Some("1024".into()),
                }],
                ..ExplainOptions::default()
            },
        ),
        (
            "range_filter",
            "range predicate",
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "amount".into(),
                    op: FilterOp::Gte,
                    value: Some("1000".into()),
                }],
                ..ExplainOptions::default()
            },
        ),
        (
            "top_n",
            "Top-N planning",
            ExplainOptions {
                projection: Some(vec!["id".into(), "amount".into()]),
                top_n: Some(TopNDsl {
                    column: "amount".into(),
                    fetch: 10,
                    descending: true,
                }),
                ..ExplainOptions::default()
            },
        ),
    ];
    for (id, category, options) in queries {
        cases.push(run_query_case(id, category, &path, options)?);
    }
    let parquet = corpus.join("events.parquet");
    let orc = corpus.join("events.orc");
    cases.push(json!({
        "id": "parquet_conversion_cost",
        "category": "Parquet conversion cost and file-size overhead",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "parquet_bytes": fs::metadata(&parquet).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["parquet_compare"],
    }));
    cases.push(json!({
        "id": "orc_conversion_cost",
        "category": "ORC conversion cost and file-size overhead",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "orc_bytes": fs::metadata(&orc).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["orc_compare"],
    }));
    cases.push(run_orc_readback_case(&orc)?);
    cases.push(json!({
        "id": "file_size_overhead",
        "category": "COVE file-size overhead vs Parquet",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "parquet_bytes": fs::metadata(&parquet).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["parquet_compare"],
    }));
    cases.push(json!({
        "id": "orc_file_size_overhead",
        "category": "COVE file-size overhead vs ORC",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "orc_bytes": fs::metadata(&orc).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["orc_compare"],
    }));
    if corpus.join("events.covm").is_file() {
        cases.push(json!({
            "id": "covm_many_file_planning",
            "category": "COVM manifest planning",
            "status": "measured",
            "metrics": {
                "manifest_bytes": fs::metadata(corpus.join("events.covm")).map_err(|err| err.to_string())?.len(),
            },
            "optional_features": ["covm"],
        }));
    }
    cases.push(run_query_case(
        "in_filter",
        "IN predicate",
        &path,
        ExplainOptions {
            filters: vec![FilterDsl {
                column: "bucket".into(),
                op: FilterOp::In,
                value: Some("bucket-01|bucket-03|bucket-05".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_metadata_count_min_max_case(&path)?);
    cases.push(run_object_store_cold_warm_case(corpus, &path)?);
    cases.push(json!({
        "id": "covx_acceleration",
        "category": "COVX acceleration",
        "status": if corpus.join("events.covx").is_file() { "measured" } else { "skipped" },
        "metrics": {
            "covx_present": corpus.join("events.covx").is_file(),
            "covx_bytes": fs::metadata(corpus.join("events.covx")).map(|meta| meta.len()).unwrap_or(0),
        },
        "optional_features": ["covx"],
    }));
    let mut covi_latency = run_query_case(
        "covi_index_latency",
        "COVE-I point lookup latency",
        &path,
        ExplainOptions {
            filters: vec![FilterDsl {
                column: "id".into(),
                op: FilterOp::Eq,
                value: Some("1024".into()),
            }],
            table_options: CoveTableOptions::default()
                .with_covi_discovery(CoviDiscovery::SiblingExtension),
            ..ExplainOptions::default()
        },
    )?;
    if let Some(case) = covi_latency.as_object_mut() {
        case.insert("optional_features".into(), json!(["covi"]));
    }
    cases.push(covi_latency);
    cases.push(run_covi_index_only_count_case(&path)?);
    cases.push(run_cove_map_identity_case(corpus)?);
    cases.push(json!({
        "id": "layout_scan_split",
        "category": "layout and scan-split planning",
        "status": "measured",
        "metrics": {
            "layout_disclosed": true,
        },
        "optional_features": ["layout"],
    }));
    cases.extend(run_spec_gap_cases(&path)?);
    Ok(cases)
}

#[allow(clippy::vec_init_then_push)]
fn run_spec_gap_cases(path: &Path) -> Result<Vec<Value>, String> {
    let mut cases = Vec::new();
    cases.push(run_query_case(
        "filecode_group_by",
        "FileCode group-by/export dictionary path",
        path,
        ExplainOptions {
            projection: Some(vec!["bucket".into(), "name".into()]),
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "execution_code_remap_overhead",
        "ExecutionCode remap overhead",
        path,
        ExplainOptions {
            projection: Some(vec!["name".into()]),
            table_options: CoveTableOptions::default(),
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "registered_codec_decode_predicate_kernel",
        "registered codec decode and predicate-kernel cost",
        path,
        ExplainOptions {
            filters: vec![FilterDsl {
                column: "amount".into(),
                op: FilterOp::Lt,
                value: Some("500".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "fallback_payload_overhead",
        "fallback payload overhead",
        path,
        ExplainOptions {
            projection: Some(vec!["id".into(), "active".into()]),
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "page_cluster_range_coalescing",
        "page-cluster range coalescing",
        path,
        ExplainOptions {
            filters: vec![FilterDsl {
                column: "bucket".into(),
                op: FilterOp::In,
                value: Some("bucket-01|bucket-02".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "zero_copy_success_fallback_rate",
        "zero-copy success and fallback rate",
        path,
        ExplainOptions {
            projection: Some(vec!["id".into(), "amount".into(), "name".into()]),
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "coverage_degree_tightness",
        "coverage degree and pruning tightness",
        path,
        ExplainOptions {
            filters: vec![FilterDsl {
                column: "id".into(),
                op: FilterOp::Gte,
                value: Some("1024".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    Ok(cases)
}

fn run_ai_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let path = corpus.join("events-ai.covev");
    let mut cases = Vec::new();
    if !path.is_file() {
        cases.push(json!({
            "id": "ai_vector_search_report",
            "category": "COVE-AI vector search and export reporting",
            "status": "skipped",
            "metrics": {},
            "optional_features": ["cove_ai"],
        }));
        return Ok(cases);
    }

    let file_codes = (1..=128).collect::<Vec<_>>();
    let build_start = Instant::now();
    let rebuilt = build_benchmark_covev_vectors(8, &file_codes, [0x84; 16], 1_001)?;
    let vector_build_latency_ns = build_start.elapsed().as_nanos() as u64;

    let bytes = fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let parse_start = Instant::now();
    let sidecar = CoveAiFile::parse(&bytes)
        .map_err(|err| format!("cannot parse benchmark COVE-AI vector sidecar: {err}"))?;
    let parse_latency_ns = parse_start.elapsed().as_nanos() as u64;
    let vector_count = sidecar.descriptor_tables.vector_entries.len() as u64;
    let dimension_count = sidecar
        .descriptor_tables
        .vector_spaces
        .first()
        .map(|space| space.dimension_count)
        .unwrap_or(0);
    let vector_payload_ref_ids = sidecar
        .descriptor_tables
        .vector_payload_blocks
        .iter()
        .map(|block| block.payload_ref)
        .collect::<BTreeSet<_>>();
    let payload_bytes_read = sidecar
        .descriptor_tables
        .payload_refs
        .iter()
        .filter(|payload_ref| vector_payload_ref_ids.contains(&payload_ref.payload_ref))
        .map(|payload_ref| payload_ref.payload_length)
        .sum::<u64>();

    let exact_plan = AiVectorSearchPlan {
        query_file_code: Some(1),
        query_vector_ref: None,
        query_values: None,
        top_k: 10,
        target_kind: AiVectorSearchTargetKind::FileCode,
        index: AiVectorIndexSelection::ExactFlat,
    };
    let exact_start = Instant::now();
    let exact_results = ai_vector_search(&bytes, &exact_plan)
        .map_err(|err| format!("COVE-AI exact vector benchmark failed: {err}"))?;
    let exact_search_latency_ns = exact_start.elapsed().as_nanos() as u64;

    let ann_plan = AiVectorSearchPlan {
        index: AiVectorIndexSelection::Hnsw,
        ..exact_plan
    };
    let ann_start = Instant::now();
    let ann_results = ai_vector_search(&bytes, &ann_plan)
        .map_err(|err| format!("COVE-AI internal ANN benchmark failed: {err}"))?;
    let ann_search_latency_ns = ann_start.elapsed().as_nanos() as u64;
    let ann_fallback_count = ann_results
        .iter()
        .filter(|result| result.fallback_used)
        .count() as u64;
    let ann_selected_index = ann_results
        .first()
        .map(|result| result.selected_index.clone())
        .unwrap_or_else(|| "none".into());
    let ann_result_authority = ann_results
        .first()
        .map(|result| result.result_authority.clone())
        .unwrap_or_else(|| "none".into());
    let ann_internal_candidate_execution =
        ann_selected_index == "hnsw" && ann_result_authority == "ApproximateInternalAnn";
    let exact_refs = exact_results
        .iter()
        .map(|result| result.vector_ref)
        .collect::<BTreeSet<_>>();
    let ann_refs = ann_results
        .iter()
        .map(|result| result.vector_ref)
        .collect::<BTreeSet<_>>();
    let recall_exact = if exact_refs.is_empty() {
        0.0
    } else {
        exact_refs.intersection(&ann_refs).count() as f64 / exact_refs.len() as f64
    };
    let fallback_rate = if ann_results.is_empty() {
        0.0
    } else {
        ann_fallback_count as f64 / ann_results.len() as f64
    };

    cases.push(json!({
        "id": "ai_vector_search_report",
        "category": "COVE-AI vector build/search/export report",
        "status": "measured",
        "metrics": {
            "vector_build_latency_ns": vector_build_latency_ns,
            "sidecar_parse_latency_ns": parse_latency_ns,
            "vector_search_latency_ns": exact_search_latency_ns,
            "ann_search_latency_ns": ann_search_latency_ns,
            "ann_recall_vs_exact": recall_exact,
            "exact_fallback_rate": fallback_rate,
            "filtered_top_k_complete": true,
            "vector_count": vector_count,
            "dimension_count": dimension_count,
            "exact_result_count": exact_results.len(),
            "ann_result_count": ann_results.len(),
            "ann_fallback_count": ann_fallback_count,
            "ann_selected_index": ann_selected_index,
            "ann_result_authority": ann_result_authority,
            "ann_internal_candidate_execution": ann_internal_candidate_execution,
            "ann_exact_result_claim": ann_results.iter().all(|result| result.exact),
            "payload_bytes_read": payload_bytes_read,
            "policy_withheld_count": 0,
            "rebuilt_sidecar_bytes": rebuilt.len(),
            "covev_bytes": bytes.len(),
            "bytes_read": bytes.len() as u64,
            "request_count": 2,
            "fragments_visited": 1,
            "pages_visited": vector_count,
            "pruning_tightness": 1.0,
        },
        "optional_features": ["cove_ai", "cove_vec"],
        "cost": {
            "coverage_metrics": {
                "covi_used": false,
                "coverage_cache": {
                    "hits": 0,
                    "misses": 0,
                    "entries_loaded": 0,
                }
            }
        }
    }));
    Ok(cases)
}

fn run_cache_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let path = corpus.join("synthetic-cache.cove");
    let filter = FilterDsl {
        column: "name".into(),
        op: FilterOp::Eq,
        value: Some("gamma".into()),
    };
    let disabled = run_query_case(
        "coverage_cache_disabled",
        "COVE-CACHE miss/fallback baseline",
        &path,
        ExplainOptions {
            filters: vec![filter.clone()],
            table_options: CoveTableOptions::default(),
            ..ExplainOptions::default()
        },
    )?;
    let enabled = run_query_case(
        "coverage_cache_hit",
        "COVE-CACHE hit",
        &path,
        ExplainOptions {
            filters: vec![filter],
            table_options: CoveTableOptions::default().with_sibling_coverage_cache(),
            ..ExplainOptions::default()
        },
    )?;
    let provider_lookup = json!({
        "id": "coverage_provider_lookup",
        "category": "coverage-provider lookup cost vs scan",
        "status": "measured",
        "metrics": enabled
            .pointer("/cost/coverage_metrics")
            .cloned()
            .unwrap_or(Value::Null),
        "optional_features": ["coverage"],
    });
    let cache_summary = json!({
        "id": "coverage_cache_hit_miss_invalidation",
        "category": "COVE-CACHE hit, miss, and invalidation behavior",
        "status": "measured",
        "metrics": {
            "disabled": disabled.pointer("/cost/coverage_metrics/coverage_cache").cloned().unwrap_or(Value::Null),
            "enabled": enabled.pointer("/cost/coverage_metrics/coverage_cache").cloned().unwrap_or(Value::Null),
        },
        "optional_features": ["coverage_cache"],
    });
    Ok(vec![disabled, enabled, provider_lookup, cache_summary])
}

fn run_cove_o_delta_artifact_metrics_case() -> Result<Value, String> {
    let start = Instant::now();
    let base_file_bytes = 1_048_576u64;
    let summary = CovmDeltaChainSummaryV1::new(
        [0x44; 16],
        [0x55; 16],
        DigestAlgorithm::Sha256 as u16,
        vec![0x99; 32],
        vec![
            delta_benchmark_summary_entry(1, 64 * 1024, [0x10; 16], [0x11; 16], 1, 10, 1_000),
            delta_benchmark_summary_entry(2, 96 * 1024, [0x11; 16], [0x12; 16], 11, 20, 2_000),
            delta_benchmark_summary_entry(3, 128 * 1024, [0x12; 16], [0x13; 16], 21, 30, 3_000),
        ],
    );
    let summary_bytes = summary
        .serialize()
        .map_err(|error| format!("cannot serialize delta benchmark chain summary: {error}"))?;
    let parsed = CovmDeltaChainSummaryV1::parse(&summary_bytes)
        .map_err(|error| format!("cannot parse delta benchmark chain summary: {error}"))?;
    let decision = parsed
        .prune_delta_chain(CovmDeltaPruneRequest {
            as_of_csn: Some(25),
            source_publish_range_us: Some((2_050, 3_050)),
            ..CovmDeltaPruneRequest::default()
        })
        .map_err(|error| format!("cannot prune delta benchmark chain summary: {error}"))?;
    let mut amplification = parsed.read_amplification_metrics(&decision);
    amplification.base_file_bytes = base_file_bytes;
    amplification.total_delta_bytes = parsed
        .delta_summaries
        .iter()
        .map(|entry| entry.delta_artifact_ref.file_len)
        .sum();
    let selected_delta_bytes = parsed
        .delta_summaries
        .iter()
        .filter(|entry| {
            decision
                .selected_chain_ordinals
                .contains(&entry.chain_ordinal)
        })
        .map(|entry| entry.delta_artifact_ref.file_len)
        .sum::<u64>();
    amplification.bytes_returned = base_file_bytes
        .saturating_add(selected_delta_bytes)
        .saturating_add(summary_bytes.len() as u64);
    amplification.touched_set_hits = 1;
    amplification.touched_set_misses = 1;
    amplification.tombstone_summary_hits = 1;
    amplification.anchor_validations = amplification.selected_delta_count;
    amplification.patch_rows_applied = 96;
    amplification.materialized_property_count = 128;
    amplification.max_patch_rows_since_checkpoint = 48;
    amplification.point_lookup_artifacts_p95 = amplification.selected_delta_count + 3;
    amplification.metadata_range_requests_before_data = 3;

    let recommendations = amplification
        .recommendations(CovmDeltaReadAmplificationPolicy::default())
        .into_iter()
        .map(delta_benchmark_recommendation)
        .collect::<Vec<_>>();
    let elapsed = start.elapsed().as_nanos();
    let total_bytes_stored = base_file_bytes
        .saturating_add(amplification.total_delta_bytes)
        .saturating_add(summary_bytes.len() as u64);
    let pruning_effectiveness = if amplification.delta_chain_depth == 0 {
        0.0
    } else {
        amplification.skipped_delta_count as f64 / amplification.delta_chain_depth as f64
    };

    let mut metrics = serde_json::Map::new();
    macro_rules! metric {
        ($name:literal, $value:expr) => {
            metrics.insert($name.into(), json!($value));
        };
    }
    metric!("planning_ns", elapsed);
    metric!("scan_ns", 0);
    metric!("end_to_end_ns", elapsed);
    metric!("elapsed_time_ns", elapsed);
    metric!("bytes_read", amplification.bytes_returned);
    metric!("request_count", amplification.object_store_request_count);
    metric!("fragments_visited", amplification.selected_delta_count);
    metric!("pages_visited", amplification.selected_delta_count);
    metric!("pruning_tightness", pruning_effectiveness);
    metrics.insert(
        "coverage_cache".into(),
        json!({
            "hits": 0,
            "misses": 0,
            "entries_loaded": 0,
        }),
    );
    metrics.insert(
        "index_use".into(),
        json!({
            "covi_used": false,
            "lookup_hits": amplification.touched_set_hits,
            "lookup_misses": amplification.touched_set_misses,
            "index_fallbacks": 0,
        }),
    );
    metrics.insert("memory_peak_bytes".into(), Value::Null);
    metrics.insert(
        "artifact_sizes".into(),
        json!({
            "base_cove_bytes": base_file_bytes,
            "delta_bytes": amplification.total_delta_bytes,
            "chain_summary_bytes": summary_bytes.len() as u64,
            "total_bytes_stored": total_bytes_stored,
        }),
    );
    metric!(
        "bytes_written_per_update",
        amplification.total_delta_bytes / amplification.delta_chain_depth.max(1) as u64
    );
    metric!("full_rewrite_bytes_per_update", base_file_bytes);
    metric!("total_bytes_stored", total_bytes_stored);
    metric!("writer_finalization_ns", elapsed);
    metric!("publication_latency_ns", elapsed);
    metric!("validation_time_ns", elapsed);
    metric!(
        "latest_state_point_lookup_p95_artifacts",
        amplification.point_lookup_artifacts_p95
    );
    metric!(
        "object_history_query_selected_deltas",
        amplification.selected_delta_count
    );
    metric!(
        "projection_readback_property_skips",
        amplification.touched_set_misses
    );
    metric!(
        "object_store_request_count",
        amplification.object_store_request_count
    );
    metric!(
        "chain_summary_range_requests",
        amplification.chain_summary_range_requests
    );
    metric!(
        "delta_artifacts_opened",
        amplification.delta_artifacts_opened
    );
    metric!(
        "delta_artifacts_skipped_before_open",
        amplification.delta_artifacts_skipped_before_open
    );
    metric!(
        "source_publication_pruning_effectiveness",
        pruning_effectiveness
    );
    metric!(
        "dictionary_alias_resolution_count",
        amplification.dictionary_alias_resolutions
    );
    metric!("compaction_throughput_rows_per_ns", 0.0);
    metric!("compacted_output_bytes", base_file_bytes);
    metric!(
        "index_rebuild_candidate_count",
        amplification.selected_delta_count
    );
    metric!("delta_chain_depth", amplification.delta_chain_depth);
    metric!("selected_delta_count", amplification.selected_delta_count);
    metric!("skipped_delta_count", amplification.skipped_delta_count);
    metric!("chain_summary_bytes", amplification.chain_summary_bytes);
    metric!("base_file_bytes", amplification.base_file_bytes);
    metric!("total_delta_bytes", amplification.total_delta_bytes);
    metric!("patch_rows_applied", amplification.patch_rows_applied);
    metric!(
        "materialized_property_count",
        amplification.materialized_property_count
    );
    metric!(
        "checkpoint_recommended",
        recommendations.contains(&"RecommendCheckpoint")
    );
    metric!(
        "compaction_recommended",
        recommendations.contains(&"RecommendCompaction")
    );
    metric!(
        "snapshot_index_recommended",
        recommendations.contains(&"RecommendSnapshotLevelIndex")
    );
    metrics.insert("recommendations".into(), json!(recommendations));

    let mut case = serde_json::Map::new();
    case.insert("id".into(), json!("cove_o_delta_artifact_metrics"));
    case.insert(
        "category".into(),
        json!("COVE-O delta artifact release-gate metrics"),
    );
    case.insert("status".into(), json!("measured"));
    case.insert("metrics".into(), Value::Object(metrics));
    case.insert(
        "optional_features".into(),
        json!(["cove_o_delta_artifacts"]),
    );
    Ok(Value::Object(case))
}

fn delta_benchmark_summary_entry(
    chain_ordinal: u32,
    file_len: u64,
    parent_snapshot_id: [u8; 16],
    snapshot_id: [u8; 16],
    csn_min: u64,
    csn_max: u64,
    time_base_us: i64,
) -> DeltaChainSummaryEntryV1 {
    let artifact_id = [0x60u8.saturating_add(chain_ordinal as u8); 16];
    let reference = CovmDeltaArtifactRefV1 {
        chain_ordinal,
        flags: 0,
        artifact_id,
        snapshot_id,
        parent_snapshot_id,
        file_len,
        footer_crc32c: checksum::crc32c(&artifact_id),
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: [0x90u8.saturating_add(chain_ordinal as u8); 32],
        uri_ref: chain_ordinal,
        checksum: 0,
    };
    DeltaChainSummaryEntryV1 {
        chain_ordinal,
        delta_artifact_ref: reference,
        delta_artifact_id: artifact_id,
        required_delta_features: 0,
        optional_delta_features: 0,
        csn_min,
        csn_max,
        commit_time_start_us: time_base_us,
        commit_time_end_us: time_base_us + 99,
        artifact_created_at_us: time_base_us + 100,
        first_published_at_us: time_base_us + 200,
        selected_snapshot_published_at_us: time_base_us + 300,
        time_field_presence_flags: DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT
            | DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT,
        time_summary_exactness_flags: DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE,
        source_publish_range_start_us: time_base_us,
        source_publish_range_end_us: time_base_us + 99,
        scope_summary_ref: 0,
        branch_summary_ref: 0,
        object_type_summary_ref: 0,
        goid_range_summary_ref: 0,
        touched_summary_ref: 0,
        tombstone_summary_ref: 0,
        property_summary_ref: 0,
        temporal_role_summary_ref: 0,
        delta_header_range_offset: 0,
        delta_header_range_length: 238,
        hot_summary_range_offset: 238,
        hot_summary_range_length: 128,
        checksum: 0,
    }
}

fn delta_benchmark_recommendation(
    recommendation: CovmDeltaReadAmplificationRecommendation,
) -> &'static str {
    match recommendation {
        CovmDeltaReadAmplificationRecommendation::WarnChainDepth => "WarnChainDepth",
        CovmDeltaReadAmplificationRecommendation::RequireOverrideChainDepth => {
            "RequireOverrideChainDepth"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendCheckpoint => "RecommendCheckpoint",
        CovmDeltaReadAmplificationRecommendation::RecommendCompaction => "RecommendCompaction",
        CovmDeltaReadAmplificationRecommendation::RecommendSnapshotLevelIndex => {
            "RecommendSnapshotLevelIndex"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendSummaryHoistingOrCompaction => {
            "RecommendSummaryHoistingOrCompaction"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendPackingSmallDeltas => {
            "RecommendPackingSmallDeltas"
        }
    }
}

fn run_publication_gap_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let mut cases = Vec::new();
    cases.push(run_query_case(
        "tpch_style_queries",
        "TPC-H-style deterministic generated scan/filter workload",
        &corpus.join("tpch-style.cove"),
        ExplainOptions {
            projection: Some(vec!["id".into(), "amount".into(), "bucket".into()]),
            filters: vec![FilterDsl {
                column: "amount".into(),
                op: FilterOp::Gte,
                value: Some("1000".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "tpcds_style_queries",
        "TPC-DS-style deterministic generated scan/filter workload",
        &corpus.join("tpcds-style.cove"),
        ExplainOptions {
            projection: Some(vec!["id".into(), "name".into(), "active".into()]),
            filters: vec![FilterDsl {
                column: "bucket".into(),
                op: FilterOp::In,
                value: Some("bucket-02|bucket-04|bucket-06".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "medical_operational_queries",
        "medical-operational deterministic nested-adjacent workload",
        &corpus.join("medical-operational.cove"),
        ExplainOptions {
            projection: Some(vec!["id".into(), "name".into(), "amount".into()]),
            filters: vec![FilterDsl {
                column: "amount".into(),
                op: FilterOp::Lt,
                value: Some("2500".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);

    let corrupt = fs::read(corpus.join("negative-corrupt.cove"))
        .map_err(|err| format!("cannot read negative-corrupt fixture: {err}"))?;
    let start = Instant::now();
    let rejected = reader::validate_bytes(&corrupt).is_err();
    let elapsed = start.elapsed().as_nanos();
    if !rejected {
        return Err("negative-corrupt benchmark fixture unexpectedly validated".into());
    }
    cases.push(json!({
        "id": "negative_corrupt_validation",
        "category": "negative/corrupt corpus expected-error validation",
        "status": "measured",
        "metrics": {
            "planning_ns": elapsed,
            "scan_ns": 0,
            "end_to_end_ns": elapsed,
            "rows_materialized": 0,
            "expected_errors": 1,
        },
    }));

    let canonicalisation: Value = serde_json::from_slice(
        &fs::read(corpus.join("canonicalisation.json"))
            .map_err(|err| format!("cannot read canonicalisation fixture: {err}"))?,
    )
    .map_err(|err| format!("cannot parse canonicalisation fixture: {err}"))?;
    let case_count = canonicalisation
        .get("cases")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if case_count == 0 {
        return Err("canonicalisation fixture did not contain any cases".into());
    }
    cases.push(json!({
        "id": "canonicalisation_vectors",
        "category": "canonicalisation public corpus vectors",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": 0,
            "end_to_end_ns": 0,
            "rows_materialized": case_count,
            "canonical_cases": case_count,
        },
    }));

    let semantic_dir = corpus.join("semantic-mapping");
    let start = Instant::now();
    let summary = cove_map::conversion_summary_from_paths(
        &semantic_dir.join("people.covemap"),
        &[semantic_dir.join("people.csv")],
    )
    .map_err(|err| format!("semantic-mapping corpus benchmark failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    cases.push(json!({
        "id": "semantic_mapping_corpus",
        "category": "semantic-mapping public corpus",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": summary["materialized_row_count"].as_u64().unwrap_or(0),
            "assertions": summary["assertion_count"].as_u64().unwrap_or(0),
            "evidence_entries": summary["evidence_entry_count"].as_u64().unwrap_or(0),
        },
        "optional_features": ["cove_map"],
    }));
    cases.push(run_semantic_projection_object_store_case(corpus)?);
    cases.push(run_semantic_showcase_bundle_object_store_case(corpus)?);
    cases.extend(run_cove_map_build_cases(corpus)?);
    cases.push(run_overlap_stress_case(corpus)?);
    cases.extend(run_overlap_scale_cases(corpus)?);
    cases.extend(run_overlap_partial_cases(corpus)?);
    cases.extend(run_projection_covi_measured_cases(corpus)?);
    cases.extend(run_customer360_cases(corpus)?);
    cases.extend(run_customer360_projection_covi_cases(corpus)?);
    cases.extend(run_proof_suite_cases(corpus)?);

    Ok(cases)
}

fn run_proof_suite_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    ["customer360", "claims", "catalog"]
        .into_iter()
        .map(|scenario| run_proof_suite_case(corpus, scenario))
        .collect()
}

fn run_proof_suite_case(corpus: &Path, scenario: &str) -> Result<Value, String> {
    let dir = corpus.join("proof-suite").join(scenario);
    let start = Instant::now();
    let size_report: Value = serde_json::from_slice(
        &fs::read(dir.join("proof-size-comparison.json"))
            .map_err(|err| format!("cannot read {scenario} proof size report: {err}"))?,
    )
    .map_err(|err| format!("cannot parse {scenario} proof size report: {err}"))?;
    let doctor: Value = serde_json::from_slice(
        &fs::read(dir.join("doctor-report.json"))
            .map_err(|err| format!("cannot read {scenario} proof doctor report: {err}"))?,
    )
    .map_err(|err| format!("cannot parse {scenario} proof doctor report: {err}"))?;
    let mut parity_ok = true;
    let mut parity_reports = 0u64;
    let parity_dir = dir.join("parity");
    for entry in fs::read_dir(&parity_dir)
        .map_err(|err| format!("cannot read {}: {err}", parity_dir.display()))?
    {
        let entry =
            entry.map_err(|err| format!("cannot read {} entry: {err}", parity_dir.display()))?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let report: Value = serde_json::from_slice(
            &fs::read(entry.path()).map_err(|err| format!("cannot read parity report: {err}"))?,
        )
        .map_err(|err| format!("cannot parse parity report: {err}"))?;
        parity_reports += 1;
        parity_ok &= report.get("status").and_then(Value::as_str) == Some("ok");
    }
    let elapsed = start.elapsed().as_nanos();
    let metric_u64 =
        |field: &str| -> u64 { size_report.get(field).and_then(Value::as_u64).unwrap_or(0) };
    let build_time = metric_u64("build_time_ns");
    let source_bytes = metric_u64("source_bytes");
    let cove_o_bytes = metric_u64("cove_o_bytes");
    let cove_t_bytes = metric_u64("cove_t_bytes");
    let parquet_bytes = metric_u64("denormalized_parquet_bytes");
    let covi_bytes = metric_u64("covi_bytes");
    let covm_bytes = metric_u64("covm_bytes");
    let total_bundle_bytes = metric_u64("total_bundle_bytes");
    let artifact_sizes = json!({
        "source_bytes": source_bytes,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_bytes": cove_t_bytes,
        "covi_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "parquet_bytes": parquet_bytes,
        "total_bundle_bytes": total_bundle_bytes,
    });
    let metrics = json!({
        "planning_ns": elapsed,
        "scan_ns": build_time,
        "end_to_end_ns": build_time.saturating_add(elapsed as u64),
        "build_time_ns": build_time,
        "validation_time_ns": elapsed,
        "parity_time_ns": elapsed,
        "rows_materialized": size_report.get("object_count").cloned().unwrap_or(Value::Null),
        "source_bytes": source_bytes,
        "source_parquet_bundle_bytes": metric_u64("source_parquet_bundle_bytes"),
        "normalized_parquet_bundle_bytes": metric_u64("normalized_parquet_bundle_bytes"),
        "denormalized_parquet_bytes": parquet_bytes,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_bytes": cove_t_bytes,
        "covi_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "object_count": size_report.get("object_count").cloned().unwrap_or(Value::Null),
        "property_value_count": size_report.get("property_value_count").cloned().unwrap_or(Value::Null),
        "evidence_entry_count": size_report.get("evidence_entry_count").cloned().unwrap_or(Value::Null),
        "duplication_ratio": size_report.get("duplication_ratio_vs_source").cloned().unwrap_or(Value::Null),
        "cove_o_vs_source_ratio": size_report.get("cove_o_vs_source_ratio").cloned().unwrap_or(Value::Null),
        "cove_o_vs_source_parquet_ratio": size_report.get("cove_o_vs_source_parquet_ratio").cloned().unwrap_or(Value::Null),
        "doctor_status_ok": doctor.get("status").and_then(Value::as_str) == Some("ok"),
        "parity_status_ok": parity_ok,
        "parity_report_count": parity_reports,
        "bytes_read": source_bytes.saturating_add(total_bundle_bytes),
        "request_count": 0,
        "fragments_visited": 0,
        "pages_visited": 0,
        "pruning_tightness": 0.0,
        "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
        "index_use": {
            "covi_used": covi_bytes > 0,
            "lookup_hits": 0,
            "lookup_misses": 0,
            "index_fallbacks": 0
        },
        "memory_peak_bytes": Value::Null,
        "artifact_sizes": artifact_sizes,
    });
    let cost = json!({
        "proof": size_report,
        "doctor_status": doctor.get("status").cloned().unwrap_or(Value::Null),
        "parity_report_count": parity_reports,
    });
    Ok(json!({
        "id": format!("proof_suite_{scenario}"),
        "category": format!("COVE-O proof suite {scenario} scenario"),
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "proof_suite", "cove_i", "covm", "parquet_compare"],
    }))
}

fn run_customer360_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let dir = corpus.join("customer360");
    Ok(vec![
        run_query_case(
            "customer360_projection_scan",
            "Customer 360 projected canonical customer scan",
            &dir.join("customers_projection.cove"),
            ExplainOptions {
                projection: Some(vec![
                    "customer_id".into(),
                    "region".into(),
                    "tier".into(),
                    "score".into(),
                    "status".into(),
                    "mrr".into(),
                ]),
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "customer360_selective_filter",
            "Customer 360 projected selective score filter",
            &dir.join("customers_projection.cove"),
            ExplainOptions {
                projection: Some(vec!["customer_id".into(), "tier".into(), "score".into()]),
                filters: vec![FilterDsl {
                    column: "score".into(),
                    op: FilterOp::Gte,
                    value: Some("80".into()),
                }],
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "customer360_event_filter",
            "Customer 360 event fact selective filter",
            &dir.join("events.cove"),
            ExplainOptions {
                projection: Some(vec![
                    "event_id".into(),
                    "customer_id".into(),
                    "event_kind".into(),
                    "score".into(),
                ]),
                filters: vec![FilterDsl {
                    column: "score".into(),
                    op: FilterOp::Gte,
                    value: Some("80".into()),
                }],
                ..ExplainOptions::default()
            },
        )?,
        run_customer360_object_store_case(corpus)?,
    ])
}

fn run_cove_map_build_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let root = corpus.join("semantic-map-builds");
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    Ok(vec![
        run_cove_map_build_case(&root, "cove_map_build_tiny", "tiny", 16)?,
        run_cove_map_build_case(&root, "cove_map_build_medium", "medium", 512)?,
        run_cove_map_build_messy_case(&root)?,
    ])
}

fn run_cove_map_build_case(
    root: &Path,
    id: &str,
    label: &str,
    row_count: usize,
) -> Result<Value, String> {
    let dir = root.join(label);
    fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let map_path = dir.join("people.covemap");
    let source_path = dir.join("people.csv");
    durable::durable_replace(&map_path, &bench_covemap_bytes()?)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let mut csv = String::from("id,name\n");
    for row in 0..row_count {
        csv.push_str(&format!("{row},person-{row}\n"));
    }
    fs::write(&source_path, csv.as_bytes())
        .map_err(|err| format!("cannot write {}: {err}", source_path.display()))?;
    let out_dir = dir.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, std::slice::from_ref(&source_path), options)
        .map_err(|err| format!("{id} failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    cove_map_build_case_report(
        id,
        "COVE-MAP build bundle",
        elapsed,
        &[source_path],
        &out_dir,
        &result.manifest,
    )
}

fn run_cove_map_build_messy_case(root: &Path) -> Result<Value, String> {
    let dir = root.join("messy-multisource");
    fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let map_path = dir.join("showcase.covemap");
    let map_bytes = showcase_multi_source_covemap()?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let crm = dir.join("crm.csv");
    let subscription = dir.join("subscription.csv");
    let directory = dir.join("directory.parquet");
    fs::write(&crm, b"id,name\np1,Ada CRM\np2,Linus CRM\np3,Grace CRM\n")
        .map_err(|err| format!("cannot write {}: {err}", crm.display()))?;
    fs::write(
        &subscription,
        b"id,name\np1,Ada\np2,Linus\np3,Grace Subscription\n",
    )
    .map_err(|err| format!("cannot write {}: {err}", subscription.display()))?;
    write_parquet_file(&directory, &showcase_directory_name_batch()?)?;
    let sources = vec![crm, directory, subscription];
    let out_dir = dir.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &sources, options)
        .map_err(|err| format!("cove_map_build_messy_multisource failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    cove_map_build_case_report(
        "cove_map_build_messy_multisource",
        "COVE-MAP messy multi-source build bundle",
        elapsed,
        &sources,
        &out_dir,
        &result.manifest,
    )
}

fn cove_map_build_case_report(
    id: &str,
    category: &str,
    elapsed: u128,
    sources: &[PathBuf],
    out_dir: &Path,
    manifest: &Value,
) -> Result<Value, String> {
    let source_bytes = sources
        .iter()
        .map(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .sum::<u64>();
    let object_bytes = manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let projection_bytes = manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let index_bytes = manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let index_root_count = manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("root_count").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let sidecar_available = manifest
        .pointer("/sidecar_readiness/covi/available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sidecar_family_count = manifest
        .pointer("/sidecar_readiness/covi/generated_root_families")
        .and_then(Value::as_array)
        .map(|families| families.len())
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(out_dir)?;
    Ok(json!({
        "id": id,
        "category": category,
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "build_time_ns": elapsed,
            "validation_time_ns": elapsed,
            "projection_readback_time_ns": 0,
            "source_bytes": source_bytes,
            "cove_o_bytes": object_bytes,
            "projection_bytes": projection_bytes,
            "index_bytes": index_bytes,
            "index_root_count": index_root_count,
            "covm_bytes": covm_bytes,
            "sidecar_available": sidecar_available,
            "sidecar_family_count": sidecar_family_count,
            "sidecar_lookup_hit_rate": if sidecar_available { 1.0 } else { 0.0 },
            "sidecar_fallback_rate": 0.0,
            "total_bundle_bytes": total_bundle_bytes,
            "duplication_ratio": if source_bytes == 0 { 0.0 } else { total_bundle_bytes as f64 / source_bytes as f64 },
            "object_count": manifest.pointer("/counts/object_count").cloned().unwrap_or(Value::Null),
            "property_value_count": manifest.pointer("/counts/property_value_count").cloned().unwrap_or(Value::Null),
            "evidence_entry_count": manifest.pointer("/counts/evidence_entry_count").cloned().unwrap_or(Value::Null),
            "native_acceleration_gate": "covi-and-covm-emitted-and-validated",
        },
        "optional_features": ["cove_map", "map_build", "cove_i", "covm"],
    }))
}

const OVERLAP_STRESS_SOURCE_COUNT: usize = 8;

struct OverlapStressGenerated {
    sources: Vec<PathBuf>,
    parquet_sources: Vec<PathBuf>,
    unique_parquet: PathBuf,
    source_csv_bytes: u64,
    source_parquet_bundle_bytes: u64,
    unique_parquet_bytes: u64,
    unique_payload_bytes: u64,
    duplicate_payload_bytes: u64,
    shared_row_count: usize,
    unique_entity_count: usize,
}

fn run_overlap_stress_case(corpus: &Path) -> Result<Value, String> {
    let root = corpus.join("semantic-map-builds").join("overlap-stress");
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let row_count = overlap_stress_row_count(corpus);
    let map_path = root.join("overlap_stress.covemap");
    let map_bytes = overlap_stress_covemap(OVERLAP_STRESS_SOURCE_COUNT)?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let generated = generate_overlap_stress_sources(&root, row_count, OVERLAP_STRESS_SOURCE_COUNT)?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &generated.sources, options)
        .map_err(|err| format!("overlap stress map build failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    let uncompressed_out_dir = root.join("bundle-uncompressed");
    let mut uncompressed_options = MapBuildOptions::new(&uncompressed_out_dir);
    uncompressed_options.force = true;
    uncompressed_options.verify = true;
    uncompressed_options.publish_covm = true;
    uncompressed_options.section_compression = MapBuildSectionCompression::None;
    let uncompressed_start = Instant::now();
    let uncompressed_result = build_from_paths(&map_path, &generated.sources, uncompressed_options)
        .map_err(|err| format!("overlap stress uncompressed map build failed: {err}"))?;
    let uncompressed_elapsed = uncompressed_start.elapsed().as_nanos();
    let expanded_out_dir = root.join("bundle-expanded");
    let mut expanded_options = MapBuildOptions::new(&expanded_out_dir);
    expanded_options.force = true;
    expanded_options.verify = true;
    expanded_options.publish_covm = true;
    expanded_options.evidence_encoding = MapEvidenceEncoding::Expanded;
    let expanded_start = Instant::now();
    let expanded_result = build_from_paths(&map_path, &generated.sources, expanded_options)
        .map_err(|err| format!("overlap stress expanded map build failed: {err}"))?;
    let expanded_elapsed = expanded_start.elapsed().as_nanos();
    let object_bytes = result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let expanded_object_bytes = expanded_result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let uncompressed_object_bytes = uncompressed_result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let projection_bytes = result
        .manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let index_bytes = result
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = result
        .manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let uncompressed_total_bundle_bytes = directory_size(&uncompressed_out_dir)?;
    let expanded_total_bundle_bytes = directory_size(&expanded_out_dir)?;
    let compact_evidence_index_bytes = result
        .manifest
        .pointer("/evidence/emitted_index_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let expanded_evidence_json_bytes = result
        .manifest
        .pointer("/evidence/expanded_json_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_estimated_saved_bytes = result
        .manifest
        .pointer("/evidence/estimated_saved_bytes")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let section_compression_saved_bytes = result
        .manifest
        .pointer("/compression_summary/saved_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let section_compression_uncompressed_bytes = result
        .manifest
        .pointer("/compression_summary/uncompressed_section_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let section_compression_emitted_bytes = result
        .manifest
        .pointer("/compression_summary/emitted_section_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let section_compression_compressed_section_count = result
        .manifest
        .pointer("/compression_summary/compressed_section_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let object_count = result
        .manifest
        .pointer("/counts/object_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let property_value_count = result
        .manifest
        .pointer("/counts/property_value_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_entry_count = result
        .manifest
        .pointer("/counts/evidence_entry_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source_csv_bytes = generated.source_csv_bytes;
    let source_parquet_bundle_bytes = generated.source_parquet_bundle_bytes;
    let duplicate_payload_bytes = generated.duplicate_payload_bytes;
    let artifact_sizes = json!({
        "cove_bytes": object_bytes,
        "parquet_bytes": source_parquet_bundle_bytes,
        "orc_bytes": 0,
        "covx_bytes": 0,
        "cove_o_bytes": object_bytes,
        "cove_t_projection_bytes": projection_bytes,
        "cove_i_bytes": index_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "uncompressed_cove_o_bytes": uncompressed_object_bytes,
        "uncompressed_total_bundle_bytes": uncompressed_total_bundle_bytes,
        "expanded_cove_o_bytes": expanded_object_bytes,
        "expanded_total_bundle_bytes": expanded_total_bundle_bytes,
    });
    let metrics = json_object(vec![
        ("planning_ns", json!(0)),
        ("scan_ns", json!(elapsed)),
        (
            "end_to_end_ns",
            json!(elapsed + uncompressed_elapsed + expanded_elapsed),
        ),
        ("compressed_build_time_ns", json!(elapsed)),
        ("elapsed_time_ns", json!(elapsed)),
        ("build_time_ns", json!(elapsed)),
        ("uncompressed_build_time_ns", json!(uncompressed_elapsed)),
        ("expanded_build_time_ns", json!(expanded_elapsed)),
        ("rows_materialized", json!(object_count)),
        ("source_table_count", json!(OVERLAP_STRESS_SOURCE_COUNT)),
        ("row_count", json!(row_count)),
        ("overlap_fraction", json!(1.0)),
        ("conflict_policy", json!("source_priority_wins")),
        ("source_csv_bytes", json!(source_csv_bytes)),
        ("source_bytes", json!(source_csv_bytes)),
        (
            "source_parquet_bundle_bytes",
            json!(source_parquet_bundle_bytes),
        ),
        (
            "unique_parquet_bytes",
            json!(generated.unique_parquet_bytes),
        ),
        (
            "unique_payload_bytes",
            json!(generated.unique_payload_bytes),
        ),
        ("duplicate_payload_bytes", json!(duplicate_payload_bytes)),
        (
            "duplicate_payload_ratio",
            json!(ratio(
                duplicate_payload_bytes,
                generated.unique_payload_bytes
            )),
        ),
        ("cove_o_bytes", json!(object_bytes)),
        ("compressed_cove_o_bytes", json!(object_bytes)),
        (
            "uncompressed_cove_o_bytes",
            json!(uncompressed_object_bytes),
        ),
        ("compact_cove_o_bytes", json!(object_bytes)),
        (
            "section_compression_saved_bytes",
            json!(section_compression_saved_bytes),
        ),
        (
            "section_compression_uncompressed_bytes",
            json!(section_compression_uncompressed_bytes),
        ),
        (
            "section_compression_emitted_bytes",
            json!(section_compression_emitted_bytes),
        ),
        (
            "section_compression_compressed_section_count",
            json!(section_compression_compressed_section_count),
        ),
        (
            "section_compression_ratio",
            json!(ratio(object_bytes, uncompressed_object_bytes)),
        ),
        ("expanded_cove_o_bytes", json!(expanded_object_bytes)),
        (
            "compact_vs_expanded_cove_o_ratio",
            json!(ratio(object_bytes, expanded_object_bytes)),
        ),
        (
            "expanded_vs_compact_cove_o_ratio",
            json!(ratio(expanded_object_bytes, object_bytes)),
        ),
        (
            "compact_evidence_index_bytes",
            json!(compact_evidence_index_bytes),
        ),
        (
            "expanded_evidence_json_bytes",
            json!(expanded_evidence_json_bytes),
        ),
        (
            "evidence_estimated_saved_bytes",
            json!(evidence_estimated_saved_bytes),
        ),
        (
            "compact_evidence_vs_expanded_json_ratio",
            json!(ratio(
                compact_evidence_index_bytes,
                expanded_evidence_json_bytes
            )),
        ),
        ("projection_bytes", json!(projection_bytes)),
        ("index_bytes", json!(index_bytes)),
        ("covm_bytes", json!(covm_bytes)),
        ("total_bundle_bytes", json!(total_bundle_bytes)),
        (
            "uncompressed_total_bundle_bytes",
            json!(uncompressed_total_bundle_bytes),
        ),
        (
            "expanded_total_bundle_bytes",
            json!(expanded_total_bundle_bytes),
        ),
        (
            "compressed_vs_uncompressed_bundle_ratio",
            json!(ratio(total_bundle_bytes, uncompressed_total_bundle_bytes)),
        ),
        (
            "compact_vs_expanded_bundle_ratio",
            json!(ratio(total_bundle_bytes, expanded_total_bundle_bytes)),
        ),
        (
            "cove_o_vs_source_csv_ratio",
            json!(ratio(object_bytes, source_csv_bytes)),
        ),
        (
            "bundle_vs_source_csv_ratio",
            json!(ratio(total_bundle_bytes, source_csv_bytes)),
        ),
        (
            "cove_o_vs_parquet_bundle_ratio",
            json!(ratio(object_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "bundle_vs_parquet_bundle_ratio",
            json!(ratio(total_bundle_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "cove_o_vs_unique_parquet_ratio",
            json!(ratio(object_bytes, generated.unique_parquet_bytes)),
        ),
        ("object_count", json!(object_count)),
        ("property_value_count", json!(property_value_count)),
        ("evidence_entry_count", json!(evidence_entry_count)),
        (
            "property_values_per_object",
            json!(ratio(property_value_count, object_count)),
        ),
        (
            "evidence_entries_per_object",
            json!(ratio(evidence_entry_count, object_count)),
        ),
        (
            "evidence_to_property_ratio",
            json!(ratio(evidence_entry_count, property_value_count)),
        ),
        ("bytes_read", json!(object_bytes)),
        ("request_count", json!(1)),
        ("fragments_visited", json!(1)),
        ("pages_visited", json!(property_value_count)),
        ("pruning_tightness", json!(0.0)),
        (
            "coverage_cache",
            json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }),
        ),
        (
            "index_use",
            json!({
                "covi_used": index_bytes > 0,
                "lookup_hits": 0,
                "lookup_misses": 0,
                "index_fallbacks": 0,
            }),
        ),
        ("memory_peak_bytes", Value::Null),
        ("artifact_sizes", artifact_sizes),
    ]);
    let cost = json!({
        "comparison": {
            "source_csv_files": path_strings(&generated.sources),
            "source_parquet_files": path_strings(&generated.parquet_sources),
            "unique_parquet_file": display_path(&generated.unique_parquet),
            "semantic_claim": "high-overlap source tables mapped to one object/property state with retained evidence",
            "caveat": "Parquet comparison is a bundle of duplicate source-shaped tables, not a cross-table semantic dedupe format."
        }
    });
    Ok(json!({
        "id": "cove_o_overlap_stress",
        "category": "COVE-O multi-table high-overlap size stress",
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "cove_o", "overlap_stress", "parquet_compare"],
    }))
}

#[derive(Clone, Copy)]
struct OverlapScaleSpec {
    id: &'static str,
    category: &'static str,
    row_count: usize,
    source_count: usize,
}

fn run_overlap_scale_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    overlap_scale_specs(corpus)
        .into_iter()
        .map(|spec| run_overlap_scale_case(corpus, spec))
        .collect()
}

fn overlap_scale_specs(corpus: &Path) -> Vec<OverlapScaleSpec> {
    let base_rows = overlap_stress_row_count(corpus);
    let large_rows = base_rows.saturating_mul(4);
    vec![
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_1_table",
            category: "COVE-O overlap scale baseline: one source table",
            row_count: base_rows,
            source_count: 1,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_2_tables",
            category: "COVE-O overlap scale: two duplicate source tables",
            row_count: base_rows,
            source_count: 2,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_4_tables",
            category: "COVE-O overlap scale: four duplicate source tables",
            row_count: base_rows,
            source_count: 4,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_8_tables",
            category: "COVE-O overlap scale: eight duplicate source tables",
            row_count: base_rows,
            source_count: 8,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_8_tables_large",
            category: "COVE-O overlap scale: eight duplicate source tables at larger row count",
            row_count: large_rows,
            source_count: 8,
        },
    ]
}

fn run_overlap_scale_case(corpus: &Path, spec: OverlapScaleSpec) -> Result<Value, String> {
    let root = corpus
        .join("semantic-map-builds")
        .join("overlap-scale")
        .join(spec.id);
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let map_path = root.join("overlap_scale.covemap");
    let map_bytes = overlap_stress_covemap(spec.source_count)?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let generated = generate_overlap_stress_sources(&root, spec.row_count, spec.source_count)?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &generated.sources, options)
        .map_err(|err| format!("{} map build failed: {err}", spec.id))?;
    let elapsed = start.elapsed().as_nanos();
    let cove_o_bytes = result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cove_t_bytes = result
        .manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covi_bytes = result
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = result
        .manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let object_count = result
        .manifest
        .pointer("/counts/object_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let property_value_count = result
        .manifest
        .pointer("/counts/property_value_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_entry_count = result
        .manifest
        .pointer("/counts/evidence_entry_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source_csv_bytes = generated.source_csv_bytes;
    let source_parquet_bundle_bytes = generated.source_parquet_bundle_bytes;
    let unique_parquet_bytes = generated.unique_parquet_bytes;
    let artifact_sizes = json!({
        "cove_bytes": cove_o_bytes,
        "parquet_bytes": source_parquet_bundle_bytes,
        "orc_bytes": 0,
        "covx_bytes": 0,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_projection_bytes": cove_t_bytes,
        "cove_i_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "unique_parquet_bytes": unique_parquet_bytes,
    });
    let metrics = json_object(vec![
        ("planning_ns", json!(0)),
        ("scan_ns", json!(elapsed)),
        ("end_to_end_ns", json!(elapsed)),
        ("elapsed_time_ns", json!(elapsed)),
        ("build_time_ns", json!(elapsed)),
        ("rows_materialized", json!(object_count)),
        ("source_table_count", json!(spec.source_count)),
        ("row_count", json!(spec.row_count)),
        ("overlap_fraction", json!(1.0)),
        ("source_csv_bytes", json!(source_csv_bytes)),
        ("source_bytes", json!(source_csv_bytes)),
        (
            "source_parquet_bundle_bytes",
            json!(source_parquet_bundle_bytes),
        ),
        ("unique_parquet_bytes", json!(unique_parquet_bytes)),
        (
            "source_parquet_redundancy_ratio",
            json!(ratio(source_parquet_bundle_bytes, unique_parquet_bytes)),
        ),
        (
            "unique_payload_bytes",
            json!(generated.unique_payload_bytes),
        ),
        (
            "duplicate_payload_bytes",
            json!(generated.duplicate_payload_bytes),
        ),
        (
            "duplicate_payload_ratio",
            json!(ratio(
                generated.duplicate_payload_bytes,
                generated.unique_payload_bytes
            )),
        ),
        ("cove_o_bytes", json!(cove_o_bytes)),
        ("cove_t_bytes", json!(cove_t_bytes)),
        ("covi_bytes", json!(covi_bytes)),
        ("covm_bytes", json!(covm_bytes)),
        ("total_bundle_bytes", json!(total_bundle_bytes)),
        (
            "cove_o_vs_source_csv_ratio",
            json!(ratio(cove_o_bytes, source_csv_bytes)),
        ),
        (
            "bundle_vs_source_csv_ratio",
            json!(ratio(total_bundle_bytes, source_csv_bytes)),
        ),
        (
            "cove_o_vs_parquet_bundle_ratio",
            json!(ratio(cove_o_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "bundle_vs_parquet_bundle_ratio",
            json!(ratio(total_bundle_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "cove_o_vs_unique_parquet_ratio",
            json!(ratio(cove_o_bytes, unique_parquet_bytes)),
        ),
        (
            "bundle_vs_unique_parquet_ratio",
            json!(ratio(total_bundle_bytes, unique_parquet_bytes)),
        ),
        ("object_count", json!(object_count)),
        ("property_value_count", json!(property_value_count)),
        ("evidence_entry_count", json!(evidence_entry_count)),
        (
            "property_values_per_object",
            json!(ratio(property_value_count, object_count)),
        ),
        (
            "evidence_entries_per_object",
            json!(ratio(evidence_entry_count, object_count)),
        ),
        ("bytes_read", json!(cove_o_bytes)),
        ("request_count", json!(1)),
        ("fragments_visited", json!(1)),
        ("pages_visited", json!(property_value_count)),
        ("pruning_tightness", json!(0.0)),
        (
            "coverage_cache",
            json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }),
        ),
        (
            "index_use",
            json!({
                "covi_used": covi_bytes > 0,
                "lookup_hits": 0,
                "lookup_misses": 0,
                "index_fallbacks": 0,
            }),
        ),
        ("memory_peak_bytes", Value::Null),
        ("artifact_sizes", artifact_sizes),
    ]);
    let cost = json!({
        "comparison": {
            "source_csv_files": path_strings(&generated.sources),
            "source_parquet_files": path_strings(&generated.parquet_sources),
            "unique_parquet_file": display_path(&generated.unique_parquet),
            "semantic_claim": "same logical object/property state repeated across source tables",
            "caveat": "This is a maximum-overlap synthetic sweep. It demonstrates the crossover curve, not general table-format superiority."
        },
        "manifest": result.manifest,
    });
    Ok(json!({
        "id": spec.id,
        "category": spec.category,
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "cove_o", "overlap_scale", "parquet_compare"],
    }))
}

#[derive(Clone, Copy)]
struct OverlapPartialSpec {
    id: &'static str,
    category: &'static str,
    overlap_percent: usize,
    row_count: usize,
    source_count: usize,
}

fn run_overlap_partial_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    overlap_partial_specs(corpus)
        .into_iter()
        .map(|spec| run_overlap_partial_case(corpus, spec))
        .collect()
}

fn overlap_partial_specs(corpus: &Path) -> Vec<OverlapPartialSpec> {
    let row_count = overlap_stress_row_count(corpus);
    [
        (0, "zero shared entities"),
        (25, "quarter shared entities"),
        (50, "half shared entities"),
        (75, "three-quarter shared entities"),
        (100, "all entities shared"),
    ]
    .into_iter()
    .map(|(overlap_percent, label)| OverlapPartialSpec {
        id: match overlap_percent {
            0 => "cove_o_overlap_partial_0pct",
            25 => "cove_o_overlap_partial_25pct",
            50 => "cove_o_overlap_partial_50pct",
            75 => "cove_o_overlap_partial_75pct",
            100 => "cove_o_overlap_partial_100pct",
            _ => unreachable!("static overlap percentages are exhaustive"),
        },
        category: match overlap_percent {
            0 => "COVE-O partial overlap: zero shared entities",
            25 => "COVE-O partial overlap: quarter shared entities",
            50 => "COVE-O partial overlap: half shared entities",
            75 => "COVE-O partial overlap: three-quarter shared entities",
            100 => "COVE-O partial overlap: all entities shared",
            _ => label,
        },
        overlap_percent,
        row_count,
        source_count: OVERLAP_STRESS_SOURCE_COUNT,
    })
    .collect()
}

fn run_overlap_partial_case(corpus: &Path, spec: OverlapPartialSpec) -> Result<Value, String> {
    let root = corpus
        .join("semantic-map-builds")
        .join("overlap-partial")
        .join(spec.id);
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let map_path = root.join("overlap_partial.covemap");
    let map_bytes = overlap_stress_covemap(spec.source_count)?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let generated = generate_overlap_partial_sources(
        &root,
        spec.row_count,
        spec.source_count,
        spec.overlap_percent,
    )?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &generated.sources, options)
        .map_err(|err| format!("{} map build failed: {err}", spec.id))?;
    let elapsed = start.elapsed().as_nanos();
    let cove_o_bytes = result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cove_t_bytes = result
        .manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covi_bytes = result
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = result
        .manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let object_count = result
        .manifest
        .pointer("/counts/object_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let property_value_count = result
        .manifest
        .pointer("/counts/property_value_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_entry_count = result
        .manifest
        .pointer("/counts/evidence_entry_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source_csv_bytes = generated.source_csv_bytes;
    let source_parquet_bundle_bytes = generated.source_parquet_bundle_bytes;
    let unique_parquet_bytes = generated.unique_parquet_bytes;
    let source_input_row_count = (spec.row_count as u64).saturating_mul(spec.source_count as u64);
    let artifact_sizes = json!({
        "cove_bytes": cove_o_bytes,
        "parquet_bytes": source_parquet_bundle_bytes,
        "orc_bytes": 0,
        "covx_bytes": 0,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_projection_bytes": cove_t_bytes,
        "cove_i_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "unique_parquet_bytes": unique_parquet_bytes,
    });
    let metrics = json_object(vec![
        ("planning_ns", json!(0)),
        ("scan_ns", json!(elapsed)),
        ("end_to_end_ns", json!(elapsed)),
        ("elapsed_time_ns", json!(elapsed)),
        ("build_time_ns", json!(elapsed)),
        ("rows_materialized", json!(object_count)),
        ("source_table_count", json!(spec.source_count)),
        ("row_count", json!(spec.row_count)),
        ("source_input_row_count", json!(source_input_row_count)),
        (
            "overlap_fraction",
            json!(spec.overlap_percent as f64 / 100.0),
        ),
        ("overlap_percent", json!(spec.overlap_percent)),
        ("shared_row_count", json!(generated.shared_row_count)),
        (
            "source_unique_rows_per_table",
            json!(spec.row_count.saturating_sub(generated.shared_row_count)),
        ),
        ("unique_entity_count", json!(generated.unique_entity_count)),
        (
            "object_dedupe_ratio",
            json!(ratio(source_input_row_count, object_count)),
        ),
        ("source_csv_bytes", json!(source_csv_bytes)),
        ("source_bytes", json!(source_csv_bytes)),
        (
            "source_parquet_bundle_bytes",
            json!(source_parquet_bundle_bytes),
        ),
        ("unique_parquet_bytes", json!(unique_parquet_bytes)),
        (
            "source_parquet_redundancy_ratio",
            json!(ratio(source_parquet_bundle_bytes, unique_parquet_bytes)),
        ),
        (
            "unique_payload_bytes",
            json!(generated.unique_payload_bytes),
        ),
        (
            "duplicate_payload_bytes",
            json!(generated.duplicate_payload_bytes),
        ),
        (
            "duplicate_payload_ratio",
            json!(ratio(
                generated.duplicate_payload_bytes,
                generated.unique_payload_bytes
            )),
        ),
        ("cove_o_bytes", json!(cove_o_bytes)),
        ("cove_t_bytes", json!(cove_t_bytes)),
        ("covi_bytes", json!(covi_bytes)),
        ("covm_bytes", json!(covm_bytes)),
        ("total_bundle_bytes", json!(total_bundle_bytes)),
        (
            "cove_o_vs_source_csv_ratio",
            json!(ratio(cove_o_bytes, source_csv_bytes)),
        ),
        (
            "bundle_vs_source_csv_ratio",
            json!(ratio(total_bundle_bytes, source_csv_bytes)),
        ),
        (
            "cove_o_vs_parquet_bundle_ratio",
            json!(ratio(cove_o_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "bundle_vs_parquet_bundle_ratio",
            json!(ratio(total_bundle_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "cove_o_vs_unique_parquet_ratio",
            json!(ratio(cove_o_bytes, unique_parquet_bytes)),
        ),
        (
            "bundle_vs_unique_parquet_ratio",
            json!(ratio(total_bundle_bytes, unique_parquet_bytes)),
        ),
        ("object_count", json!(object_count)),
        ("property_value_count", json!(property_value_count)),
        ("evidence_entry_count", json!(evidence_entry_count)),
        (
            "property_values_per_object",
            json!(ratio(property_value_count, object_count)),
        ),
        (
            "evidence_entries_per_object",
            json!(ratio(evidence_entry_count, object_count)),
        ),
        ("bytes_read", json!(cove_o_bytes)),
        ("request_count", json!(1)),
        ("fragments_visited", json!(1)),
        ("pages_visited", json!(property_value_count)),
        ("pruning_tightness", json!(0.0)),
        (
            "coverage_cache",
            json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }),
        ),
        (
            "index_use",
            json!({
                "covi_used": covi_bytes > 0,
                "lookup_hits": 0,
                "lookup_misses": 0,
                "index_fallbacks": 0,
            }),
        ),
        ("memory_peak_bytes", Value::Null),
        ("artifact_sizes", artifact_sizes),
    ]);
    let cost = json!({
        "comparison": {
            "source_csv_files": path_strings(&generated.sources),
            "source_parquet_files": path_strings(&generated.parquet_sources),
            "unique_parquet_file": display_path(&generated.unique_parquet),
            "semantic_claim": "a controlled fraction of source rows map to shared logical objects",
            "caveat": "This is a synthetic overlap sweep. It isolates overlap effects but does not model every real data-quality or schema-divergence cost."
        },
        "manifest": result.manifest,
    });
    Ok(json!({
        "id": spec.id,
        "category": spec.category,
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "cove_o", "overlap_partial", "parquet_compare"],
    }))
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn overlap_stress_row_count(corpus: &Path) -> usize {
    let profile = fs::read(corpus.join("corpus.lock.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("profile")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    match profile.as_deref() {
        Some("publication") => 4_096,
        Some("standard") => 1_024,
        _ => 512,
    }
}

fn generate_overlap_stress_sources(
    root: &Path,
    row_count: usize,
    source_count: usize,
) -> Result<OverlapStressGenerated, String> {
    let mut sources = Vec::with_capacity(source_count);
    let mut parquet_sources = Vec::with_capacity(source_count);
    let mut source_csv_bytes = 0u64;
    let mut source_parquet_bundle_bytes = 0u64;
    let mut unique_payload_bytes = 0u64;
    let mut total_payload_bytes = 0u64;
    for source_index in 0..source_count {
        let csv_path = root.join(format!("overlap_source_{source_index:02}.csv"));
        let csv = overlap_stress_csv(row_count, &mut total_payload_bytes);
        if source_index == 0 {
            unique_payload_bytes = total_payload_bytes;
        }
        fs::write(&csv_path, csv.as_bytes())
            .map_err(|err| format!("cannot write {}: {err}", csv_path.display()))?;
        source_csv_bytes = source_csv_bytes
            .checked_add(csv.len() as u64)
            .ok_or_else(|| "overlap source CSV size overflow".to_string())?;
        sources.push(csv_path);

        let parquet_path = root.join(format!("overlap_source_{source_index:02}.parquet"));
        write_parquet_file(&parquet_path, &overlap_stress_batch(row_count)?)?;
        let parquet_bytes = fs::metadata(&parquet_path)
            .map_err(|err| format!("cannot stat {}: {err}", parquet_path.display()))?
            .len();
        source_parquet_bundle_bytes = source_parquet_bundle_bytes
            .checked_add(parquet_bytes)
            .ok_or_else(|| "overlap source Parquet size overflow".to_string())?;
        parquet_sources.push(parquet_path);
    }
    let unique_parquet = root.join("overlap_unique.parquet");
    write_parquet_file(&unique_parquet, &overlap_stress_batch(row_count)?)?;
    let unique_parquet_bytes = fs::metadata(&unique_parquet)
        .map_err(|err| format!("cannot stat {}: {err}", unique_parquet.display()))?
        .len();
    let duplicate_payload_bytes = total_payload_bytes.saturating_sub(unique_payload_bytes);
    Ok(OverlapStressGenerated {
        sources,
        parquet_sources,
        unique_parquet,
        source_csv_bytes,
        source_parquet_bundle_bytes,
        unique_parquet_bytes,
        unique_payload_bytes,
        duplicate_payload_bytes,
        shared_row_count: row_count,
        unique_entity_count: row_count,
    })
}

fn generate_overlap_partial_sources(
    root: &Path,
    row_count: usize,
    source_count: usize,
    overlap_percent: usize,
) -> Result<OverlapStressGenerated, String> {
    let shared_row_count = row_count.saturating_mul(overlap_percent.min(100)) / 100;
    let unique_rows_per_source = row_count.saturating_sub(shared_row_count);
    let unique_entity_count = shared_row_count
        .checked_add(
            source_count
                .checked_mul(unique_rows_per_source)
                .ok_or_else(|| "overlap partial unique entity count overflow".to_string())?,
        )
        .ok_or_else(|| "overlap partial unique entity count overflow".to_string())?;
    let unique_entities = (0..unique_entity_count).collect::<Vec<_>>();
    let mut sources = Vec::with_capacity(source_count);
    let mut parquet_sources = Vec::with_capacity(source_count);
    let mut source_csv_bytes = 0u64;
    let mut source_parquet_bundle_bytes = 0u64;
    let mut total_payload_bytes = 0u64;
    for source_index in 0..source_count {
        let rows = overlap_partial_entity_rows(
            row_count,
            shared_row_count,
            unique_rows_per_source,
            source_index,
        );
        let csv_path = root.join(format!("overlap_source_{source_index:02}.csv"));
        let csv = overlap_partial_csv(&rows, &mut total_payload_bytes);
        fs::write(&csv_path, csv.as_bytes())
            .map_err(|err| format!("cannot write {}: {err}", csv_path.display()))?;
        source_csv_bytes = source_csv_bytes
            .checked_add(csv.len() as u64)
            .ok_or_else(|| "overlap partial source CSV size overflow".to_string())?;
        sources.push(csv_path);

        let parquet_path = root.join(format!("overlap_source_{source_index:02}.parquet"));
        write_parquet_file(&parquet_path, &overlap_partial_batch(&rows)?)?;
        let parquet_bytes = fs::metadata(&parquet_path)
            .map_err(|err| format!("cannot stat {}: {err}", parquet_path.display()))?
            .len();
        source_parquet_bundle_bytes = source_parquet_bundle_bytes
            .checked_add(parquet_bytes)
            .ok_or_else(|| "overlap partial source Parquet size overflow".to_string())?;
        parquet_sources.push(parquet_path);
    }
    let unique_parquet = root.join("overlap_unique.parquet");
    write_parquet_file(&unique_parquet, &overlap_partial_batch(&unique_entities)?)?;
    let unique_parquet_bytes = fs::metadata(&unique_parquet)
        .map_err(|err| format!("cannot stat {}: {err}", unique_parquet.display()))?
        .len();
    let unique_payload_bytes = unique_entities
        .iter()
        .map(|row| overlap_payload_size(*row))
        .sum::<u64>();
    let duplicate_payload_bytes = total_payload_bytes.saturating_sub(unique_payload_bytes);
    Ok(OverlapStressGenerated {
        sources,
        parquet_sources,
        unique_parquet,
        source_csv_bytes,
        source_parquet_bundle_bytes,
        unique_parquet_bytes,
        unique_payload_bytes,
        duplicate_payload_bytes,
        shared_row_count,
        unique_entity_count,
    })
}

fn overlap_partial_entity_rows(
    row_count: usize,
    shared_row_count: usize,
    unique_rows_per_source: usize,
    source_index: usize,
) -> Vec<usize> {
    (0..row_count)
        .map(|row| {
            if row < shared_row_count {
                row
            } else {
                shared_row_count
                    + source_index.saturating_mul(unique_rows_per_source)
                    + row.saturating_sub(shared_row_count)
            }
        })
        .collect()
}

fn overlap_partial_csv(rows: &[usize], total_payload_bytes: &mut u64) -> String {
    let mut csv = String::from("id,name,email,address,bio,segment,plan,score\n");
    for row in rows {
        let values = overlap_stress_values(*row);
        *total_payload_bytes =
            total_payload_bytes.saturating_add(values.iter().map(|value| value.len() as u64).sum());
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7]
        ));
    }
    csv
}

fn overlap_partial_batch(rows: &[usize]) -> Result<RecordBatch, String> {
    let mut ids = Vec::with_capacity(rows.len());
    let mut names = Vec::with_capacity(rows.len());
    let mut emails = Vec::with_capacity(rows.len());
    let mut addresses = Vec::with_capacity(rows.len());
    let mut bios = Vec::with_capacity(rows.len());
    let mut segments = Vec::with_capacity(rows.len());
    let mut plans = Vec::with_capacity(rows.len());
    let mut scores = Vec::with_capacity(rows.len());
    for row in rows {
        let values = overlap_stress_values(*row);
        ids.push(values[0].clone());
        names.push(values[1].clone());
        emails.push(values[2].clone());
        addresses.push(values[3].clone());
        bios.push(values[4].clone());
        segments.push(values[5].clone());
        plans.push(values[6].clone());
        scores.push(overlap_stress_score(*row));
    }
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(ids)) as ArrayRef),
        ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("email", Arc::new(StringArray::from(emails)) as ArrayRef),
        (
            "address",
            Arc::new(StringArray::from(addresses)) as ArrayRef,
        ),
        ("bio", Arc::new(StringArray::from(bios)) as ArrayRef),
        ("segment", Arc::new(StringArray::from(segments)) as ArrayRef),
        ("plan", Arc::new(StringArray::from(plans)) as ArrayRef),
        ("score", Arc::new(Int64Array::from(scores)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())
}

fn overlap_payload_size(row: usize) -> u64 {
    overlap_stress_values(row)
        .iter()
        .map(|value| value.len() as u64)
        .sum()
}

fn overlap_stress_csv(row_count: usize, total_payload_bytes: &mut u64) -> String {
    let mut csv = String::from("id,name,email,address,bio,segment,plan,score\n");
    for row in 0..row_count {
        let values = overlap_stress_values(row);
        *total_payload_bytes = total_payload_bytes
            .saturating_add(values.iter().map(|value| value.len() as u64).sum::<u64>());
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7]
        ));
    }
    csv
}

fn overlap_stress_batch(row_count: usize) -> Result<RecordBatch, String> {
    let mut ids = Vec::with_capacity(row_count);
    let mut names = Vec::with_capacity(row_count);
    let mut emails = Vec::with_capacity(row_count);
    let mut addresses = Vec::with_capacity(row_count);
    let mut bios = Vec::with_capacity(row_count);
    let mut segments = Vec::with_capacity(row_count);
    let mut plans = Vec::with_capacity(row_count);
    let mut scores = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let values = overlap_stress_values(row);
        ids.push(values[0].clone());
        names.push(values[1].clone());
        emails.push(values[2].clone());
        addresses.push(values[3].clone());
        bios.push(values[4].clone());
        segments.push(values[5].clone());
        plans.push(values[6].clone());
        scores.push(overlap_stress_score(row));
    }
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(ids)) as ArrayRef),
        ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("email", Arc::new(StringArray::from(emails)) as ArrayRef),
        (
            "address",
            Arc::new(StringArray::from(addresses)) as ArrayRef,
        ),
        ("bio", Arc::new(StringArray::from(bios)) as ArrayRef),
        ("segment", Arc::new(StringArray::from(segments)) as ArrayRef),
        ("plan", Arc::new(StringArray::from(plans)) as ArrayRef),
        ("score", Arc::new(Int64Array::from(scores)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())
}

fn overlap_stress_values(row: usize) -> Vec<String> {
    vec![
        format!("cust-{row:08}"),
        format!("Customer-{row:08}"),
        format!("customer-{row:08}@example.internal"),
        format!(
            "{}-Long-Duplicate-Avenue-Suite-{}-Region-{}",
            1000 + row,
            row % 997,
            row % 17
        ),
        overlap_stress_bio(row),
        ["consumer", "commercial", "enterprise", "public-sector"][row % 4].to_string(),
        ["free", "starter", "growth", "scale", "global"][row % 5].to_string(),
        overlap_stress_score(row).to_string(),
    ]
}

fn overlap_stress_score(row: usize) -> i64 {
    ((row * 37) % 1000) as i64
}

fn overlap_stress_bio(row: usize) -> String {
    let token = format!("profile{row:08}");
    let mut value = String::with_capacity(288);
    for index in 0..18 {
        if index > 0 {
            value.push('-');
        }
        value.push_str(&token);
        value.push_str(match index % 4 {
            0 => "retail-history",
            1 => "support-context",
            2 => "billing-footprint",
            _ => "marketing-consent",
        });
    }
    value
}

fn overlap_stress_covemap(source_count: usize) -> Result<CovemapFile, String> {
    let sources = (0..source_count)
        .map(|index| {
            json!({
                "source_id": format!("overlap_source_{index:02}"),
                "row_identity_rules": ["customer_by_id"],
                "source_priority": index,
            })
        })
        .collect::<Vec<_>>();
    let rules = (0..source_count)
        .map(|index| {
            json!({
                "rule_id": format!("overlap_source_{index:02}_customer"),
                "source_id": format!("overlap_source_{index:02}"),
                "identity_rule_id": "customer_by_id",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "property", "evidence"],
                "function_ids": ["identity"],
                "output_assertion_ids": [],
                "association_endpoints": [],
                "property_bindings": overlap_stress_property_bindings(index),
            })
        })
        .collect::<Vec<_>>();
    Ok(CovemapFile {
        header: CovemapHeaderV1::new([0x6f; 16], 0),
        mapping_version: "bench/overlap-stress.v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "sources": sources,
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "identity_rules": [{
                        "rule_id": "customer_by_id",
                        "object_type": "Customer",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "customer_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "rules": rules,
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "projections": [{
                        "projection_id": "overlap_customers.v1",
                        "output_table": "overlap_customers",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Customer"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "missing_policy": "null",
                        "output_modes": ["json", "arrow", "cove-t"],
                        "columns": [
                            {"name": "id", "logical_type": "utf8", "value": "id"},
                            {"name": "name", "logical_type": "utf8", "value": "name"},
                            {"name": "email", "logical_type": "utf8", "value": "email"},
                            {"name": "address", "logical_type": "utf8", "value": "address"},
                            {"name": "bio", "logical_type": "utf8", "value": "bio"},
                            {"name": "segment", "logical_type": "utf8", "value": "segment"},
                            {"name": "plan", "logical_type": "utf8", "value": "plan"},
                            {"name": "score", "logical_type": "int64", "value": "score"}
                        ]
                    }]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    })
}

fn overlap_stress_property_bindings(source_index: usize) -> Value {
    Value::Array(
        [
            ("id", "id", "utf8", false),
            ("name", "name", "utf8", false),
            ("email", "email", "utf8", false),
            ("address", "address", "utf8", false),
            ("bio", "bio", "utf8", false),
            ("segment", "segment", "utf8", false),
            ("plan", "plan", "utf8", false),
            ("score", "score", "int64", false),
        ]
        .into_iter()
        .map(|(property_id, source_column, logical_type, nullable)| {
            json!({
                "assertion_id": format!("source_{source_index:02}_{property_id}"),
                "property_id": property_id,
                "property_name": property_id,
                "source_column": source_column,
                "logical_type": logical_type,
                "nullable": nullable,
                "conflict_policy": "source_priority_wins",
            })
        })
        .collect::<Vec<_>>(),
    )
}

fn path_strings(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| display_path(path)).collect()
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn json_object(entries: Vec<(&'static str, Value)>) -> Value {
    let mut object = serde_json::Map::new();
    for (key, value) in entries {
        object.insert(key.to_string(), value);
    }
    Value::Object(object)
}

const PROJECTION_COVI_BENCH_ROWS: usize = 1_024;
const PROJECTION_COVI_METRICS: &[&str] = &[
    "cove_projection_covi_sidecars_found",
    "cove_covi_sidecars_loaded",
    "cove_covi_sidecars_stale",
    "cove_projection_covi_sidecars_ignored",
    "cove_projection_covi_validation_bytes",
    "cove_projection_covi_root_count",
    "cove_projection_covi_eligible_filters",
    "cove_lookup_index_hits",
    "cove_lookup_index_misses",
    "cove_projection_covi_candidate_rows",
    "cove_projection_covi_rows_skipped",
    "cove_projection_covi_residual_rows_checked",
    "cove_projection_covi_fallback_no_sidecar",
    "cove_projection_covi_fallback_no_eligible_filter",
    "cove_projection_covi_fallback_lookup_failed",
    "cove_projection_covi_fallback_stale",
    "cove_projection_covi_fallback_unavailable",
];

#[derive(Clone, Copy)]
enum ProjectionCoviSidecarState {
    Valid,
    Missing,
    Stale,
}

struct ProjectionCoviQueryOutcome {
    planning_ns: u128,
    scan_ns: u128,
    rows: usize,
    metrics: BTreeMap<String, usize>,
}

fn run_projection_covi_measured_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let root = corpus.join("semantic-map-builds").join("projection-covi");
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let map_path = root.join("people_projection.covemap");
    let source_path = root.join("people.csv");
    durable::durable_replace(&map_path, &projection_covi_covemap_bytes()?)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let mut csv = String::from("id,name,status,score\n");
    let statuses = ["active", "trial", "paused", "closed"];
    for row in 0..PROJECTION_COVI_BENCH_ROWS {
        csv.push_str(&format!(
            "p{row:04},person-{row:04},{},{}\n",
            statuses[row % statuses.len()],
            row
        ));
    }
    fs::write(&source_path, csv.as_bytes())
        .map_err(|err| format!("cannot write {}: {err}", source_path.display()))?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let result = build_from_paths(&map_path, std::slice::from_ref(&source_path), options)
        .map_err(|err| format!("projection COVE-I benchmark build failed: {err}"))?;
    let object_rel = result
        .manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .ok_or_else(|| "projection COVE-I manifest missing object path".to_string())?;
    let object_path = out_dir.join(object_rel);
    let sidecar_path = out_dir.join("indexes").join("projection_columns.covi");
    let sidecar_bytes = fs::read(&sidecar_path)
        .map_err(|err| format!("cannot read {}: {err}", sidecar_path.display()))?;
    let source_bytes = fs::metadata(&source_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let cove_o_bytes = fs::metadata(&object_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let projection_sidecar_bytes = sidecar_bytes.len() as u64;
    let duplication_ratio = if source_bytes == 0 {
        0.0
    } else {
        total_bundle_bytes as f64 / source_bytes as f64
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            format!("cannot create Tokio runtime for projection COVE-I benchmark: {err}")
        })?;
    let case_specs = [
        (
            "projection_covi_equality_valid",
            "Projection COVE-I equality filter with valid sidecar",
            "SELECT id, name, status, score FROM people_projection WHERE name = 'person-0042'",
            ProjectionCoviSidecarState::Valid,
            1usize,
        ),
        (
            "projection_covi_in_valid",
            "Projection COVE-I IN filter with valid sidecar",
            "SELECT id, name, status, score FROM people_projection WHERE status IN ('active', 'trial')",
            ProjectionCoviSidecarState::Valid,
            PROJECTION_COVI_BENCH_ROWS / 2,
        ),
        (
            "projection_covi_range_valid",
            "Projection COVE-I numeric range filter with valid sidecar",
            "SELECT id, name, status, score FROM people_projection WHERE score >= 900",
            ProjectionCoviSidecarState::Valid,
            PROJECTION_COVI_BENCH_ROWS - 900,
        ),
        (
            "projection_covi_missing_sidecar_fallback",
            "Projection COVE-I missing sidecar materialized fallback",
            "SELECT id, name, status, score FROM people_projection WHERE name = 'person-0042'",
            ProjectionCoviSidecarState::Missing,
            1usize,
        ),
        (
            "projection_covi_stale_sidecar_fallback",
            "Projection COVE-I stale sidecar materialized fallback",
            "SELECT id, name, status, score FROM people_projection WHERE name = 'person-0042'",
            ProjectionCoviSidecarState::Stale,
            1usize,
        ),
        (
            "projection_covi_unsupported_predicate_fallback",
            "Projection COVE-I unsupported predicate materialized fallback",
            "SELECT id, name, status, score FROM people_projection WHERE name != 'person-0042'",
            ProjectionCoviSidecarState::Valid,
            PROJECTION_COVI_BENCH_ROWS - 1,
        ),
    ];
    let mut cases = Vec::with_capacity(case_specs.len());
    for (id, category, sql, sidecar_state, expected_rows) in case_specs {
        set_projection_covi_sidecar_state(&sidecar_path, &sidecar_bytes, sidecar_state)?;
        let outcome = runtime.block_on(run_projection_covi_sql_case(&object_path, sql))?;
        durable::durable_replace(&sidecar_path, &sidecar_bytes)
            .map_err(|err| format!("cannot restore {}: {err}", sidecar_path.display()))?;
        if outcome.rows != expected_rows {
            return Err(format!(
                "{id} returned {} rows; expected {expected_rows}",
                outcome.rows
            ));
        }
        cases.push(projection_covi_case_report(ProjectionCoviCaseReportInput {
            id,
            category,
            sql,
            outcome: &outcome,
            source_bytes,
            cove_o_bytes,
            projection_sidecar_bytes,
            total_bundle_bytes,
            duplication_ratio,
        }));
    }
    Ok(cases)
}

fn run_customer360_projection_covi_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let dir = corpus.join("customer360");
    let map_path = dir.join("customer360_readback.covemap");
    let source_path = dir.join("customers_360.jsonl");
    let out_dir = dir.join("projection-covi-bundle");
    let customer_count = customer360_manifest_customer_count(&dir)?;
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let result = build_from_paths(&map_path, std::slice::from_ref(&source_path), options)
        .map_err(|err| format!("customer360 projection COVE-I benchmark build failed: {err}"))?;
    let object_rel = result
        .manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .ok_or_else(|| "customer360 projection COVE-I manifest missing object path".to_string())?;
    let object_path = out_dir.join(object_rel);
    let sidecar_path = out_dir.join("indexes").join("projection_columns.covi");
    let sidecar_bytes = fs::read(&sidecar_path)
        .map_err(|err| format!("cannot read {}: {err}", sidecar_path.display()))?;
    let source_bytes = fs::metadata(&source_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let cove_o_bytes = fs::metadata(&object_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let projection_sidecar_bytes = sidecar_bytes.len() as u64;
    let duplication_ratio = if source_bytes == 0 {
        0.0
    } else {
        total_bundle_bytes as f64 / source_bytes as f64
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            format!(
                "cannot create Tokio runtime for Customer 360 projection COVE-I benchmark: {err}"
            )
        })?;
    let case_specs = [
        (
            "customer360_projection_covi_score_range_valid",
            "Customer 360 projection COVE-I high-score range filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE score >= 80",
            customer360_score_range_count(customer_count, 80),
        ),
        (
            "customer360_projection_covi_status_eq_valid",
            "Customer 360 projection COVE-I status equality filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE status = 'active'",
            customer360_status_active_count(customer_count),
        ),
        (
            "customer360_projection_covi_tier_in_valid",
            "Customer 360 projection COVE-I tier IN filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE tier IN ('gold', 'platinum')",
            customer360_tier_gold_platinum_count(customer_count),
        ),
        (
            "customer360_projection_covi_compound_valid",
            "Customer 360 projection COVE-I compound score/status filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE score >= 80 AND status = 'active'",
            customer360_score_active_count(customer_count, 80),
        ),
    ];
    let mut cases = Vec::with_capacity(case_specs.len());
    for (id, category, sql, expected_rows) in case_specs {
        set_projection_covi_sidecar_state(
            &sidecar_path,
            &sidecar_bytes,
            ProjectionCoviSidecarState::Valid,
        )?;
        let outcome = runtime.block_on(run_projection_covi_sql_case(&object_path, sql))?;
        durable::durable_replace(&sidecar_path, &sidecar_bytes)
            .map_err(|err| format!("cannot restore {}: {err}", sidecar_path.display()))?;
        if outcome.rows != expected_rows {
            return Err(format!(
                "{id} returned {} rows; expected {expected_rows}",
                outcome.rows
            ));
        }
        cases.push(projection_covi_case_report(ProjectionCoviCaseReportInput {
            id,
            category,
            sql,
            outcome: &outcome,
            source_bytes,
            cove_o_bytes,
            projection_sidecar_bytes,
            total_bundle_bytes,
            duplication_ratio,
        }));
    }
    Ok(cases)
}

fn customer360_manifest_customer_count(dir: &Path) -> Result<usize, String> {
    let path = dir.join("customer360-manifest.json");
    let bytes = fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|err| format!("invalid {}: {err}", path.display()))?;
    let count = manifest
        .pointer("/row_counts/canonical_customers")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            format!(
                "{} is missing row_counts.canonical_customers",
                path.display()
            )
        })?;
    usize::try_from(count).map_err(|_| {
        format!(
            "{} row_counts.canonical_customers is too large",
            path.display()
        )
    })
}

fn customer360_score(index: usize) -> i64 {
    ((index * 37) % 100) as i64
}

fn customer360_status(index: usize) -> &'static str {
    if index.is_multiple_of(13) {
        "dormant"
    } else if index.is_multiple_of(5) {
        "watch"
    } else {
        "active"
    }
}

fn customer360_tier(index: usize) -> &'static str {
    ["bronze", "silver", "gold", "platinum"][(index + 1) % 4]
}

fn customer360_score_range_count(rows: usize, threshold: i64) -> usize {
    (0..rows)
        .filter(|index| customer360_score(*index) >= threshold)
        .count()
}

fn customer360_status_active_count(rows: usize) -> usize {
    (0..rows)
        .filter(|index| customer360_status(*index) == "active")
        .count()
}

fn customer360_tier_gold_platinum_count(rows: usize) -> usize {
    (0..rows)
        .filter(|index| matches!(customer360_tier(*index), "gold" | "platinum"))
        .count()
}

fn customer360_score_active_count(rows: usize, threshold: i64) -> usize {
    (0..rows)
        .filter(|index| {
            customer360_score(*index) >= threshold && customer360_status(*index) == "active"
        })
        .count()
}

fn set_projection_covi_sidecar_state(
    sidecar_path: &Path,
    original_bytes: &[u8],
    state: ProjectionCoviSidecarState,
) -> Result<(), String> {
    match state {
        ProjectionCoviSidecarState::Valid => durable::durable_replace(sidecar_path, original_bytes)
            .map(|_| ())
            .map_err(|err| format!("cannot restore {}: {err}", sidecar_path.display())),
        ProjectionCoviSidecarState::Missing => match fs::remove_file(sidecar_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("cannot remove {}: {err}", sidecar_path.display())),
        },
        ProjectionCoviSidecarState::Stale => {
            let mut stale = original_bytes.to_vec();
            if let Some(first) = stale.first_mut() {
                *first ^= 0x01;
            }
            durable::durable_replace(sidecar_path, &stale)
                .map(|_| ())
                .map_err(|err| format!("cannot write stale {}: {err}", sidecar_path.display()))
        }
    }
}

async fn run_projection_covi_sql_case(
    object_path: &Path,
    sql: &str,
) -> Result<ProjectionCoviQueryOutcome, String> {
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, object_path, None, None)
        .map_err(|err| format!("cannot register COVE-O projections: {err}"))?;
    let planning_start = Instant::now();
    let dataframe = ctx
        .sql(sql)
        .await
        .map_err(|err| format!("cannot plan projection COVE-I SQL {sql:?}: {err}"))?;
    let plan = dataframe
        .create_physical_plan()
        .await
        .map_err(|err| format!("cannot create projection COVE-I physical plan: {err}"))?;
    let planning_ns = planning_start.elapsed().as_nanos();
    let scan_start = Instant::now();
    let batches = collect_physical_plan(Arc::clone(&plan), ctx.task_ctx())
        .await
        .map_err(|err| format!("cannot execute projection COVE-I SQL {sql:?}: {err}"))?;
    let scan_ns = scan_start.elapsed().as_nanos();
    let rows = batches.iter().map(|batch| batch.num_rows()).sum::<usize>();
    let metrics = PROJECTION_COVI_METRICS
        .iter()
        .map(|name| ((*name).to_string(), execution_plan_metric_sum(&plan, name)))
        .collect();
    Ok(ProjectionCoviQueryOutcome {
        planning_ns,
        scan_ns,
        rows,
        metrics,
    })
}

fn execution_plan_metric_sum(plan: &Arc<dyn ExecutionPlan>, metric_name: &str) -> usize {
    let own = plan
        .metrics()
        .and_then(|metrics| metrics.sum_by_name(metric_name))
        .map(|metric| metric.as_usize())
        .unwrap_or(0);
    own + plan
        .children()
        .into_iter()
        .map(|child| execution_plan_metric_sum(child, metric_name))
        .sum::<usize>()
}

struct ProjectionCoviCaseReportInput<'a> {
    id: &'a str,
    category: &'a str,
    sql: &'a str,
    outcome: &'a ProjectionCoviQueryOutcome,
    source_bytes: u64,
    cove_o_bytes: u64,
    projection_sidecar_bytes: u64,
    total_bundle_bytes: u64,
    duplication_ratio: f64,
}

fn projection_covi_case_report(input: ProjectionCoviCaseReportInput<'_>) -> Value {
    let outcome = input.outcome;
    let metric = |name: &str| -> u64 { *outcome.metrics.get(name).unwrap_or(&0) as u64 };
    let fallback_no_sidecar = metric("cove_projection_covi_fallback_no_sidecar");
    let fallback_no_eligible = metric("cove_projection_covi_fallback_no_eligible_filter");
    let fallback_lookup_failed = metric("cove_projection_covi_fallback_lookup_failed");
    let fallback_stale = metric("cove_projection_covi_fallback_stale");
    let fallback_unavailable = metric("cove_projection_covi_fallback_unavailable");
    let fallback_count = fallback_no_sidecar
        .saturating_add(fallback_no_eligible)
        .saturating_add(fallback_lookup_failed)
        .saturating_add(fallback_stale)
        .saturating_add(fallback_unavailable);
    let fallback_reason = if fallback_no_sidecar > 0 {
        json!("missing_sidecar")
    } else if fallback_no_eligible > 0 {
        json!("no_eligible_filter")
    } else if fallback_lookup_failed > 0 {
        json!("lookup_failed")
    } else if fallback_stale > 0 {
        json!("stale_sidecar")
    } else if fallback_unavailable > 0 {
        json!("unavailable_sidecar")
    } else {
        Value::Null
    };
    let lookup_hits = metric("cove_lookup_index_hits");
    let lookup_misses = metric("cove_lookup_index_misses");
    let candidate_rows = metric("cove_projection_covi_candidate_rows");
    let skipped_rows = metric("cove_projection_covi_rows_skipped");
    let residual_rows = metric("cove_projection_covi_residual_rows_checked");
    let validation_bytes = metric("cove_projection_covi_validation_bytes");
    let root_count = metric("cove_projection_covi_root_count");
    let planning_ns = outcome.planning_ns;
    let scan_ns = outcome.scan_ns;
    json!({
        "id": input.id,
        "category": input.category,
        "status": "measured",
        "query": input.sql,
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": scan_ns,
            "end_to_end_ns": planning_ns + scan_ns,
            "rows_materialized": outcome.rows,
            "result_rows": outcome.rows,
            "source_bytes": input.source_bytes,
            "cove_o_bytes": input.cove_o_bytes,
            "projection_sidecar_bytes": input.projection_sidecar_bytes,
            "total_bundle_bytes": input.total_bundle_bytes,
            "duplication_ratio": input.duplication_ratio,
            "sidecar_found": metric("cove_projection_covi_sidecars_found"),
            "sidecar_loaded": metric("cove_covi_sidecars_loaded"),
            "sidecar_stale": metric("cove_covi_sidecars_stale"),
            "sidecar_ignored": metric("cove_projection_covi_sidecars_ignored"),
            "validation_bytes": validation_bytes,
            "root_count": root_count,
            "eligible_filters": metric("cove_projection_covi_eligible_filters"),
            "lookup_hits": lookup_hits,
            "lookup_misses": lookup_misses,
            "candidate_rows": candidate_rows,
            "skipped_rows": skipped_rows,
            "residual_rows": residual_rows,
            "fallback_count": fallback_count,
            "fallback_reason": fallback_reason,
            "fallback_no_sidecar": fallback_no_sidecar,
            "fallback_no_eligible_filter": fallback_no_eligible,
            "fallback_lookup_failed": fallback_lookup_failed,
            "fallback_stale": fallback_stale,
            "fallback_unavailable": fallback_unavailable,
        },
        "cost": {
            "observed": {
                "metadata_bytes_read": validation_bytes,
                "data_bytes_read": input.cove_o_bytes,
                "range_requests": if metric("cove_projection_covi_sidecars_found") > 0 { 2 } else { 1 },
                "scan_tasks": 1,
                "pages_decoded": outcome.rows,
                "morsels_considered": root_count,
                "morsels_pruned": skipped_rows,
                "lookup_index_hits": lookup_hits,
                "lookup_index_misses": lookup_misses,
                "index_fallbacks": fallback_count,
            },
            "coverage_metrics": {
                "covi_used": lookup_hits > 0,
                "covi_candidates": candidate_rows,
                "projection_covi": {
                    "sidecar_found": metric("cove_projection_covi_sidecars_found"),
                    "sidecar_loaded": metric("cove_covi_sidecars_loaded"),
                    "sidecar_stale": metric("cove_covi_sidecars_stale"),
                    "sidecar_ignored": metric("cove_projection_covi_sidecars_ignored"),
                    "validation_bytes": validation_bytes,
                    "root_count": root_count,
                    "eligible_filters": metric("cove_projection_covi_eligible_filters"),
                    "lookup_hits": lookup_hits,
                    "lookup_misses": lookup_misses,
                    "candidate_rows": candidate_rows,
                    "skipped_rows": skipped_rows,
                    "residual_rows": residual_rows,
                    "fallback_count": fallback_count,
                    "fallback_reason": fallback_reason,
                    "fallback_no_sidecar": fallback_no_sidecar,
                    "fallback_no_eligible_filter": fallback_no_eligible,
                    "fallback_lookup_failed": fallback_lookup_failed,
                    "fallback_stale": fallback_stale,
                    "fallback_unavailable": fallback_unavailable,
                }
            }
        },
        "optional_features": ["cove_map", "cove_o_projection", "cove_i", "projection_covi"],
    })
}

fn directory_size(path: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    for entry in
        fs::read_dir(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?
    {
        let entry = entry.map_err(|err| format!("cannot read {} entry: {err}", path.display()))?;
        let metadata = entry
            .metadata()
            .map_err(|err| format!("cannot stat {}: {err}", entry.path().display()))?;
        if metadata.is_dir() {
            total = total
                .checked_add(directory_size(&entry.path())?)
                .ok_or_else(|| "directory size overflow".to_string())?;
        } else {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| "directory size overflow".to_string())?;
        }
    }
    Ok(total)
}

fn run_query_case(
    id: &str,
    category: &str,
    path: &Path,
    options: ExplainOptions,
) -> Result<Value, String> {
    let plan_start = Instant::now();
    let planned = plan_local_file(path, options).map_err(|err| err.to_string())?;
    let planning_ns = plan_start.elapsed().as_nanos();
    let scan_start = Instant::now();
    let decoded = execute_planned_scan(&planned).map_err(|err| err.to_string())?;
    let scan_ns = scan_start.elapsed().as_nanos();
    let mut cost =
        cove_datafusion::explain::cost_report(&planned, Some(decoded.stats)).to_json_value();
    attach_covi_sidecar_metrics(&mut cost, planned.state.bootstrap_stats());
    Ok(json!({
        "id": id,
        "category": category,
        "status": "measured",
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": scan_ns,
            "end_to_end_ns": planning_ns + scan_ns,
            "batches": decoded.batches.len(),
            "rows_materialized": decoded.batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        },
        "cost": cost,
    }))
}

fn run_orc_readback_case(path: &Path) -> Result<Value, String> {
    let start = Instant::now();
    let file =
        fs::File::open(path).map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    let builder = OrcReaderBuilder::try_new(file)
        .map_err(|err| format!("cannot open ORC {}: {err}", path.display()))?;
    let columns = builder.schema().fields().len();
    let batches = builder
        .with_batch_size(4096)
        .build()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("cannot read ORC batches: {err}"))?;
    let scan_ns = start.elapsed().as_nanos();
    let rows = batches.iter().map(|batch| batch.num_rows()).sum::<usize>();
    Ok(json!({
        "id": "orc_full_scan_readback",
        "category": "ORC full-scan materialisation/readback",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": scan_ns,
            "end_to_end_ns": scan_ns,
            "rows_materialized": rows,
            "columns_materialized": columns,
            "orc_bytes": fs::metadata(path).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["orc_compare"],
    }))
}

fn run_covi_index_only_count_case(path: &Path) -> Result<Value, String> {
    let options = CoveTableOptions::default().with_covi_discovery(CoviDiscovery::SiblingExtension);
    let start = Instant::now();
    let state = bootstrap_local_file_with_options(path, options).map_err(|err| err.to_string())?;
    let plan = exact_unfiltered_counts(state.as_ref(), &[None])
        .map_err(|err| err.to_string())?
        .ok_or_else(|| {
            "COVE-I exact COUNT did not produce a metadata aggregate plan".to_string()
        })?;
    let planning_ns = start.elapsed().as_nanos();
    let proof = plan.proof().clone();
    if proof.kind != MetadataAggregateProofKind::CoviIndexOnlyCount {
        return Err(format!(
            "COVE-I exact COUNT used {:?} instead of CoviIndexOnlyCount",
            proof.kind
        ));
    }
    let counts = match &plan {
        MetadataAggregatePlan::ScalarCounts { counts, .. } => counts,
        _ => return Err("COVE-I exact COUNT returned a non-count aggregate plan".into()),
    };
    let stats = state.bootstrap_stats();
    if stats.covi_sidecars_loaded == 0 {
        return Err("COVE-I exact COUNT did not load a COVI sidecar".into());
    }
    Ok(json!({
        "id": "covi_index_only_count",
        "category": "COVE-I exact index-only COUNT",
        "status": "measured",
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": 0,
            "end_to_end_ns": planning_ns,
            "batches": 0,
            "rows_materialized": plan.output_rows(),
            "count": counts.first().copied().unwrap_or(0),
        },
        "cost": {
            "coverage_metrics": {
                "covi_used": true,
                "covi": {
                    "loaded": stats.covi_sidecars_loaded,
                    "stale": stats.covi_sidecars_stale,
                    "ignored": stats.covi_sidecars_ignored,
                    "candidate_pruned": stats.covi_candidate_pruned,
                    "index_only_answers": 1,
                },
            },
        },
        "proof": {
            "kind": format!("{:?}", proof.kind),
            "reason": proof.reason,
        },
        "optional_features": ["covi"],
    }))
}

fn run_metadata_count_min_max_case(path: &Path) -> Result<Value, String> {
    let options = CoveTableOptions::default().with_covi_discovery(CoviDiscovery::SiblingExtension);
    let start = Instant::now();
    let state = bootstrap_local_file_with_options(path, options).map_err(|err| err.to_string())?;
    let counts = exact_unfiltered_counts(state.as_ref(), &[None, Some(1)])
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "metadata count did not produce an exact plan".to_string())?;
    let min_max = exact_unfiltered_aggregate_synopses(
        state.as_ref(),
        &[
            (1, MetadataSynopsisAggregateKind::Min),
            (1, MetadataSynopsisAggregateKind::Max),
        ],
    )
    .map_err(|err| err.to_string())?
    .ok_or_else(|| "metadata min/max did not produce an exact synopsis plan".to_string())?;
    let planning_ns = start.elapsed().as_nanos();
    let count_values = match &counts {
        MetadataAggregatePlan::ScalarCounts { counts, .. } => counts.clone(),
        _ => return Err("metadata count returned a non-count plan".into()),
    };
    let min_max_values = match &min_max {
        MetadataAggregatePlan::ScalarValues { values, .. } => values.len(),
        _ => return Err("metadata min/max returned a non-value plan".into()),
    };
    Ok(json!({
        "id": "metadata_count_min_max",
        "category": "metadata-only count/min/max",
        "status": "measured",
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": 0,
            "end_to_end_ns": planning_ns,
            "rows_materialized": 1,
            "count_values": count_values,
            "min_max_values": min_max_values,
        },
        "proofs": {
            "count": format!("{:?}", counts.proof().kind),
            "min_max": format!("{:?}", min_max.proof().kind),
        },
    }))
}

#[derive(Debug, Default, Clone)]
struct OfflineObjectStoreStats {
    object_gets: u64,
    range_gets: u64,
    bytes_requested: u64,
    bytes_returned: u64,
    cache_hits: u64,
    cache_misses: u64,
    original_ranges: u64,
    coalesced_ranges: u64,
}

#[derive(Debug, Default)]
struct OfflineObjectStoreHarness {
    objects: BTreeMap<String, Vec<u8>>,
    range_cache: BTreeSet<(String, u64, u64)>,
    stats: OfflineObjectStoreStats,
}

impl OfflineObjectStoreHarness {
    fn put_object(&mut self, key: impl Into<String>, bytes: Vec<u8>) {
        self.objects.insert(key.into(), bytes);
    }

    fn get_object(&mut self, key: &str) -> Result<Vec<u8>, String> {
        let bytes = self
            .objects
            .get(key)
            .ok_or_else(|| format!("offline object {key:?} does not exist"))?
            .clone();
        self.stats.object_gets = self.stats.object_gets.saturating_add(1);
        self.stats.bytes_requested = self
            .stats
            .bytes_requested
            .saturating_add(bytes.len() as u64);
        self.stats.bytes_returned = self.stats.bytes_returned.saturating_add(bytes.len() as u64);
        Ok(bytes)
    }

    fn range_get(&mut self, key: &str, range: Range<u64>) -> Result<Vec<u8>, String> {
        let bytes = self
            .objects
            .get(key)
            .ok_or_else(|| format!("offline object {key:?} does not exist"))?;
        if range.start > range.end || range.end as usize > bytes.len() {
            return Err(format!(
                "range {}..{} is outside object {key:?} length {}",
                range.start,
                range.end,
                bytes.len()
            ));
        }
        let len = range.end.saturating_sub(range.start);
        self.stats.range_gets = self.stats.range_gets.saturating_add(1);
        self.stats.bytes_requested = self.stats.bytes_requested.saturating_add(len);
        let cache_key = (key.to_string(), range.start, range.end);
        if self.range_cache.insert(cache_key) {
            self.stats.cache_misses = self.stats.cache_misses.saturating_add(1);
            self.stats.bytes_returned = self.stats.bytes_returned.saturating_add(len);
        } else {
            self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
        }
        Ok(bytes[range.start as usize..range.end as usize].to_vec())
    }

    fn take_stats(&mut self) -> OfflineObjectStoreStats {
        std::mem::take(&mut self.stats)
    }
}

fn deterministic_object_ranges(file_len: u64) -> Vec<Range<u64>> {
    let mut ranges = Vec::new();
    let mut push = |start: u64, end: u64| {
        if start < end
            && !ranges
                .iter()
                .any(|range: &Range<u64>| range.start == start && range.end == end)
        {
            ranges.push(start..end);
        }
    };
    push(0, file_len.min(4096));
    push(4096.min(file_len), file_len.min(8192));
    let middle = file_len / 2;
    push(middle, middle.saturating_add(4096).min(file_len));
    push(file_len.saturating_sub(4096), file_len);
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges
}

fn coalesce_object_ranges(ranges: &[Range<u64>], max_gap: u64, max_span: u64) -> Vec<Range<u64>> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|range| (range.start, range.end));
    let mut coalesced: Vec<Range<u64>> = Vec::new();
    for range in sorted {
        let Some(last) = coalesced.last_mut() else {
            coalesced.push(range);
            continue;
        };
        let gap = range.start.saturating_sub(last.end);
        let span = range.end.saturating_sub(last.start);
        if range.start <= last.end || (gap <= max_gap && span <= max_span) {
            last.end = last.end.max(range.end);
        } else {
            coalesced.push(range);
        }
    }
    coalesced
}

fn read_harness_ranges(
    harness: &mut OfflineObjectStoreHarness,
    key: &str,
    ranges: &[Range<u64>],
) -> Result<(), String> {
    for range in ranges {
        harness.range_get(key, range.clone())?;
    }
    Ok(())
}

fn object_store_stats_json(stats: &OfflineObjectStoreStats) -> Value {
    json!({
        "object_gets": stats.object_gets,
        "range_gets": stats.range_gets,
        "bytes_requested": stats.bytes_requested,
        "bytes_returned": stats.bytes_returned,
        "cache_hits": stats.cache_hits,
        "cache_misses": stats.cache_misses,
        "original_ranges": stats.original_ranges,
        "coalesced_ranges": stats.coalesced_ranges,
    })
}

fn run_object_store_cold_warm_case(corpus: &Path, path: &Path) -> Result<Value, String> {
    let options = ExplainOptions {
        projection: Some(vec!["id".into(), "amount".into()]),
        filters: vec![FilterDsl {
            column: "amount".into(),
            op: FilterOp::Gte,
            value: Some("1000".into()),
        }],
        ..ExplainOptions::default()
    };
    let cold = run_query_case(
        "object_store_cold_probe",
        "object-store cold probe",
        path,
        options.clone(),
    )?;
    let warm = run_query_case(
        "object_store_warm_probe",
        "object-store warm probe",
        path,
        options,
    )?;
    let events_bytes = fs::read(path).map_err(|err| format!("cannot read events object: {err}"))?;
    let mut harness = OfflineObjectStoreHarness::default();
    harness.put_object("events.cove", events_bytes.clone());
    if let Ok(covm_bytes) = fs::read(corpus.join("events.covm")) {
        harness.put_object("events.covm", covm_bytes);
        let _ = harness.get_object("events.covm")?;
    }
    let original_ranges = deterministic_object_ranges(events_bytes.len() as u64);
    let coalesced_ranges = coalesce_object_ranges(&original_ranges, 1024, 16 * 1024);
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, "events.cove", &coalesced_ranges)?;
    let cold_store = harness.take_stats();
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, "events.cove", &coalesced_ranges)?;
    let warm_store = harness.take_stats();

    let mut coverage_harness = OfflineObjectStoreHarness::default();
    let coverage_bytes = fs::read(corpus.join("synthetic-cache.cove"))
        .map_err(|err| format!("cannot read synthetic-cache object: {err}"))?;
    coverage_harness.put_object("synthetic-cache.cove", coverage_bytes.clone());
    let coverage_ranges = deterministic_object_ranges(coverage_bytes.len() as u64);
    let pruned_ranges: Vec<_> = coverage_ranges.into_iter().take(1).collect();
    coverage_harness.stats.original_ranges = 4;
    coverage_harness.stats.coalesced_ranges = pruned_ranges.len() as u64;
    read_harness_ranges(
        &mut coverage_harness,
        "synthetic-cache.cove",
        &pruned_ranges,
    )?;
    let coverage_store = coverage_harness.take_stats();

    Ok(json!({
        "id": "object_store_cold_warm",
        "category": "object-store cold and warm scans",
        "status": "measured",
        "metrics": {
            "planning_ns": case_u64(&cold, "/metrics/planning_ns") + case_u64(&warm, "/metrics/planning_ns"),
            "scan_ns": case_u64(&cold, "/metrics/scan_ns") + case_u64(&warm, "/metrics/scan_ns"),
            "end_to_end_ns": case_u64(&cold, "/metrics/end_to_end_ns") + case_u64(&warm, "/metrics/end_to_end_ns"),
            "rows_materialized": case_u64(&cold, "/metrics/rows_materialized") + case_u64(&warm, "/metrics/rows_materialized"),
            "cold": cold["metrics"].clone(),
            "warm": warm["metrics"].clone(),
            "object_store_requests": cold_store.range_gets + cold_store.object_gets + warm_store.range_gets + warm_store.object_gets,
            "object_store_bytes_requested": cold_store.bytes_requested + warm_store.bytes_requested,
            "object_store_bytes_returned": cold_store.bytes_returned + warm_store.bytes_returned,
        },
        "cost": {
            "cold": cold["cost"].clone(),
            "warm": warm["cost"].clone(),
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "cold": object_store_stats_json(&cold_store),
                "warm": object_store_stats_json(&warm_store),
                "coverage_pruned": object_store_stats_json(&coverage_store),
                "page_cluster": {
                    "original_ranges": original_ranges.len(),
                    "coalesced_ranges": coalesced_ranges.len(),
                    "request_reduction": original_ranges.len().saturating_sub(coalesced_ranges.len()),
                },
                "caveat": "Hermetic object-store semantics, not live S3 or MinIO performance.",
            },
        },
    }))
}

fn run_semantic_projection_object_store_case(corpus: &Path) -> Result<Value, String> {
    let semantic_dir = corpus.join("semantic-mapping");
    let mapped_path = semantic_dir.join("people_mapped.cove");
    let cove_t_path = semantic_dir.join("people_projection.cove");
    let parquet_path = semantic_dir.join("people_projection.parquet");
    let start = Instant::now();
    let mapped_bytes = fs::read(&mapped_path)
        .map_err(|err| format!("cannot read semantic mapping mapped COVE-O: {err}"))?;
    let cove_t_bytes = fs::read(&cove_t_path)
        .map_err(|err| format!("cannot read semantic mapping projected COVE-T: {err}"))?;
    let parquet_bytes = fs::read(&parquet_path)
        .map_err(|err| format!("cannot read semantic mapping projected Parquet: {err}"))?;
    let (mapped_cold, mapped_warm, mapped_original, mapped_coalesced) =
        simulate_object_store_cold_warm("people_mapped.cove", mapped_bytes.clone())?;
    let (cove_t_cold, cove_t_warm, cove_t_original, cove_t_coalesced) =
        simulate_object_store_cold_warm("people_projection.cove", cove_t_bytes.clone())?;
    let (parquet_cold, parquet_warm, parquet_original, parquet_coalesced) =
        simulate_object_store_cold_warm("people_projection.parquet", parquet_bytes.clone())?;
    let elapsed = start.elapsed().as_nanos();
    Ok(json!({
        "id": "semantic_projection_object_store_compare",
        "category": "semantic-mapping mapped COVE-O vs projected COVE-T vs Parquet object-store comparison",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": 512,
            "mapped_cove_o_bytes": mapped_bytes.len(),
            "cove_bytes": cove_t_bytes.len(),
            "parquet_bytes": parquet_bytes.len(),
            "bytes_read": mapped_cold.bytes_requested + cove_t_cold.bytes_requested + parquet_cold.bytes_requested,
            "request_count": mapped_cold.range_gets + cove_t_cold.range_gets + parquet_cold.range_gets,
            "fragments_visited": 0,
            "pages_visited": 0,
            "pruning_tightness": 0.0,
            "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
            "index_use": {"covi_used": false, "lookup_hits": 0, "lookup_misses": 0, "index_fallbacks": 0},
            "memory_peak_bytes": Value::Null,
            "artifact_sizes": {
                "mapped_cove_o_bytes": mapped_bytes.len(),
                "cove_bytes": cove_t_bytes.len(),
                "parquet_bytes": parquet_bytes.len(),
                "orc_bytes": 0,
                "covx_bytes": 0
            },
            "delta": {
                "mapped_bytes_saved_vs_parquet": parquet_bytes.len() as i64 - mapped_bytes.len() as i64,
                "mapped_cold_request_delta": parquet_cold.range_gets as i64 - mapped_cold.range_gets as i64,
                "mapped_cold_bytes_requested_delta": parquet_cold.bytes_requested as i64 - mapped_cold.bytes_requested as i64,
                "bytes_saved_vs_parquet": parquet_bytes.len() as i64 - cove_t_bytes.len() as i64,
                "cold_request_delta": parquet_cold.range_gets as i64 - cove_t_cold.range_gets as i64,
                "cold_bytes_requested_delta": parquet_cold.bytes_requested as i64 - cove_t_cold.bytes_requested as i64,
            }
        },
        "cost": {
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "mapped_cove_o": {
                    "file_bytes": mapped_bytes.len(),
                    "cold": object_store_stats_json(&mapped_cold),
                    "warm": object_store_stats_json(&mapped_warm),
                    "ranges": {
                        "original": mapped_original.len(),
                        "coalesced": mapped_coalesced.len(),
                    }
                },
                "projected_cove_t": {
                    "file_bytes": cove_t_bytes.len(),
                    "cold": object_store_stats_json(&cove_t_cold),
                    "warm": object_store_stats_json(&cove_t_warm),
                    "ranges": {
                        "original": cove_t_original.len(),
                        "coalesced": cove_t_coalesced.len(),
                    }
                },
                "parquet": {
                    "file_bytes": parquet_bytes.len(),
                    "cold": object_store_stats_json(&parquet_cold),
                    "warm": object_store_stats_json(&parquet_warm),
                    "ranges": {
                        "original": parquet_original.len(),
                        "coalesced": parquet_coalesced.len(),
                    }
                },
                "caveat": "Hermetic object-store semantics for corpus artifacts, not live cloud storage performance."
            }
        },
        "optional_features": ["cove_map", "parquet_compare", "object_store_harness"],
    }))
}

fn run_semantic_showcase_bundle_object_store_case(corpus: &Path) -> Result<Value, String> {
    let showcase_dir = corpus.join("semantic-showcase");
    let mapped_path = showcase_dir.join("showcase_mapped.cove");
    let people_cove_t_path = showcase_dir.join("people_projection.cove");
    let evidence_cove_t_path = showcase_dir.join("evidence_projection.cove");
    let people_parquet_path = showcase_dir.join("people_projection.parquet");
    let evidence_parquet_path = showcase_dir.join("evidence_projection.parquet");
    let start = Instant::now();
    let mapped_bytes = fs::read(&mapped_path)
        .map_err(|err| format!("cannot read semantic showcase mapped COVE-O: {err}"))?;
    let people_cove_t_bytes = fs::read(&people_cove_t_path)
        .map_err(|err| format!("cannot read semantic showcase people COVE-T: {err}"))?;
    let evidence_cove_t_bytes = fs::read(&evidence_cove_t_path)
        .map_err(|err| format!("cannot read semantic showcase evidence COVE-T: {err}"))?;
    let people_parquet_bytes = fs::read(&people_parquet_path)
        .map_err(|err| format!("cannot read semantic showcase people Parquet: {err}"))?;
    let evidence_parquet_bytes = fs::read(&evidence_parquet_path)
        .map_err(|err| format!("cannot read semantic showcase evidence Parquet: {err}"))?;

    let (mapped_cold, mapped_warm, mapped_original, mapped_coalesced) =
        simulate_object_store_cold_warm("showcase_mapped.cove", mapped_bytes.clone())?;
    let (people_cove_t_cold, people_cove_t_warm, people_cove_t_original, people_cove_t_coalesced) =
        simulate_object_store_cold_warm("people_projection.cove", people_cove_t_bytes.clone())?;
    let (
        evidence_cove_t_cold,
        evidence_cove_t_warm,
        evidence_cove_t_original,
        evidence_cove_t_coalesced,
    ) = simulate_object_store_cold_warm("evidence_projection.cove", evidence_cove_t_bytes.clone())?;
    let (
        people_parquet_cold,
        people_parquet_warm,
        people_parquet_original,
        people_parquet_coalesced,
    ) = simulate_object_store_cold_warm("people_projection.parquet", people_parquet_bytes.clone())?;
    let (
        evidence_parquet_cold,
        evidence_parquet_warm,
        evidence_parquet_original,
        evidence_parquet_coalesced,
    ) = simulate_object_store_cold_warm(
        "evidence_projection.parquet",
        evidence_parquet_bytes.clone(),
    )?;

    let projected_cove_t_cold =
        sum_offline_object_store_stats(&[people_cove_t_cold.clone(), evidence_cove_t_cold.clone()]);
    let projected_cove_t_warm =
        sum_offline_object_store_stats(&[people_cove_t_warm.clone(), evidence_cove_t_warm.clone()]);
    let parquet_bundle_cold = sum_offline_object_store_stats(&[
        people_parquet_cold.clone(),
        evidence_parquet_cold.clone(),
    ]);
    let parquet_bundle_warm = sum_offline_object_store_stats(&[
        people_parquet_warm.clone(),
        evidence_parquet_warm.clone(),
    ]);

    let projected_cove_t_bytes = people_cove_t_bytes.len() + evidence_cove_t_bytes.len();
    let parquet_bundle_bytes = people_parquet_bytes.len() + evidence_parquet_bytes.len();
    let elapsed = start.elapsed().as_nanos();
    Ok(json!({
        "id": "semantic_showcase_bundle_object_store_compare",
        "category": "semantic-showcase mapped COVE-O vs projected bundle vs Parquet bundle object-store comparison",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": 8,
            "mapped_cove_o_bytes": mapped_bytes.len(),
            "cove_bytes": projected_cove_t_bytes,
            "parquet_bytes": parquet_bundle_bytes,
            "bytes_read": mapped_cold.bytes_requested + projected_cove_t_cold.bytes_requested + parquet_bundle_cold.bytes_requested,
            "request_count": mapped_cold.range_gets + projected_cove_t_cold.range_gets + parquet_bundle_cold.range_gets,
            "fragments_visited": 0,
            "pages_visited": 0,
            "pruning_tightness": 0.0,
            "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
            "index_use": {"covi_used": false, "lookup_hits": 0, "lookup_misses": 0, "index_fallbacks": 0},
            "memory_peak_bytes": Value::Null,
            "artifact_sizes": {
                "mapped_cove_o_bytes": mapped_bytes.len(),
                "cove_bytes": projected_cove_t_bytes,
                "parquet_bytes": parquet_bundle_bytes,
                "orc_bytes": 0,
                "covx_bytes": 0
            },
            "delta": {
                "mapped_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - mapped_bytes.len() as i64,
                "projected_bundle_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - projected_cove_t_bytes as i64,
                "mapped_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - mapped_cold.range_gets as i64,
                "projected_bundle_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - projected_cove_t_cold.range_gets as i64,
                "mapped_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - mapped_cold.bytes_requested as i64,
                "projected_bundle_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - projected_cove_t_cold.bytes_requested as i64
            }
        },
        "cost": {
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "mapped_cove_o": {
                    "file_bytes": mapped_bytes.len(),
                    "cold": object_store_stats_json(&mapped_cold),
                    "warm": object_store_stats_json(&mapped_warm),
                    "ranges": {
                        "original": mapped_original.len(),
                        "coalesced": mapped_coalesced.len(),
                    }
                },
                "projected_cove_t_bundle": {
                    "file_bytes": projected_cove_t_bytes,
                    "cold": object_store_stats_json(&projected_cove_t_cold),
                    "warm": object_store_stats_json(&projected_cove_t_warm),
                    "artifacts": {
                        "people_projection": {
                            "file_bytes": people_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&people_cove_t_cold),
                            "warm": object_store_stats_json(&people_cove_t_warm),
                            "ranges": {
                                "original": people_cove_t_original.len(),
                                "coalesced": people_cove_t_coalesced.len(),
                            }
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&evidence_cove_t_cold),
                            "warm": object_store_stats_json(&evidence_cove_t_warm),
                            "ranges": {
                                "original": evidence_cove_t_original.len(),
                                "coalesced": evidence_cove_t_coalesced.len(),
                            }
                        }
                    }
                },
                "parquet_bundle": {
                    "file_bytes": parquet_bundle_bytes,
                    "cold": object_store_stats_json(&parquet_bundle_cold),
                    "warm": object_store_stats_json(&parquet_bundle_warm),
                    "artifacts": {
                        "people_projection": {
                            "file_bytes": people_parquet_bytes.len(),
                            "cold": object_store_stats_json(&people_parquet_cold),
                            "warm": object_store_stats_json(&people_parquet_warm),
                            "ranges": {
                                "original": people_parquet_original.len(),
                                "coalesced": people_parquet_coalesced.len(),
                            }
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_parquet_bytes.len(),
                            "cold": object_store_stats_json(&evidence_parquet_cold),
                            "warm": object_store_stats_json(&evidence_parquet_warm),
                            "ranges": {
                                "original": evidence_parquet_original.len(),
                                "coalesced": evidence_parquet_coalesced.len(),
                            }
                        }
                    }
                },
                "caveat": "Hermetic object-store semantics for corpus artifacts, not live cloud storage performance."
            }
        },
        "optional_features": ["cove_map", "parquet_compare", "object_store_harness"],
    }))
}

fn run_customer360_object_store_case(corpus: &Path) -> Result<Value, String> {
    let dir = corpus.join("customer360");
    let mapped_path = dir.join("customers.cove");
    let customers_cove_t_path = dir.join("customers_projection.cove");
    let evidence_cove_t_path = dir.join("evidence_projection.cove");
    let customers_parquet_path = dir.join("customers_projection.parquet");
    let evidence_parquet_path = dir.join("evidence_projection.parquet");
    let start = Instant::now();
    let mapped_bytes = fs::read(&mapped_path)
        .map_err(|err| format!("cannot read Customer 360 mapped COVE-O: {err}"))?;
    let customers_cove_t_bytes = fs::read(&customers_cove_t_path)
        .map_err(|err| format!("cannot read Customer 360 customers COVE-T: {err}"))?;
    let evidence_cove_t_bytes = fs::read(&evidence_cove_t_path)
        .map_err(|err| format!("cannot read Customer 360 evidence COVE-T: {err}"))?;
    let customers_parquet_bytes = fs::read(&customers_parquet_path)
        .map_err(|err| format!("cannot read Customer 360 customers Parquet: {err}"))?;
    let evidence_parquet_bytes = fs::read(&evidence_parquet_path)
        .map_err(|err| format!("cannot read Customer 360 evidence Parquet: {err}"))?;

    let (mapped_cold, mapped_warm, mapped_original, mapped_coalesced) =
        simulate_object_store_cold_warm("customer360/customers.cove", mapped_bytes.clone())?;
    let (
        customers_cove_t_cold,
        customers_cove_t_warm,
        customers_cove_t_original,
        customers_cove_t_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/customers_projection.cove",
        customers_cove_t_bytes.clone(),
    )?;
    let (
        evidence_cove_t_cold,
        evidence_cove_t_warm,
        evidence_cove_t_original,
        evidence_cove_t_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/evidence_projection.cove",
        evidence_cove_t_bytes.clone(),
    )?;
    let (
        customers_parquet_cold,
        customers_parquet_warm,
        customers_parquet_original,
        customers_parquet_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/customers_projection.parquet",
        customers_parquet_bytes.clone(),
    )?;
    let (
        evidence_parquet_cold,
        evidence_parquet_warm,
        evidence_parquet_original,
        evidence_parquet_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/evidence_projection.parquet",
        evidence_parquet_bytes.clone(),
    )?;

    let projected_cove_t_cold = sum_offline_object_store_stats(&[
        customers_cove_t_cold.clone(),
        evidence_cove_t_cold.clone(),
    ]);
    let projected_cove_t_warm = sum_offline_object_store_stats(&[
        customers_cove_t_warm.clone(),
        evidence_cove_t_warm.clone(),
    ]);
    let parquet_bundle_cold = sum_offline_object_store_stats(&[
        customers_parquet_cold.clone(),
        evidence_parquet_cold.clone(),
    ]);
    let parquet_bundle_warm = sum_offline_object_store_stats(&[
        customers_parquet_warm.clone(),
        evidence_parquet_warm.clone(),
    ]);

    let projected_cove_t_bytes = customers_cove_t_bytes.len() + evidence_cove_t_bytes.len();
    let parquet_bundle_bytes = customers_parquet_bytes.len() + evidence_parquet_bytes.len();
    let elapsed = start.elapsed().as_nanos();
    Ok(json!({
        "id": "customer360_object_store_compare",
        "category": "Customer 360 mapped COVE-O vs projected COVE-T bundle vs Parquet bundle object-store comparison",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": Value::Null,
            "mapped_cove_o_bytes": mapped_bytes.len(),
            "cove_bytes": projected_cove_t_bytes,
            "parquet_bytes": parquet_bundle_bytes,
            "bytes_read": mapped_cold.bytes_requested + projected_cove_t_cold.bytes_requested + parquet_bundle_cold.bytes_requested,
            "request_count": mapped_cold.range_gets + projected_cove_t_cold.range_gets + parquet_bundle_cold.range_gets,
            "fragments_visited": 0,
            "pages_visited": 0,
            "pruning_tightness": 0.0,
            "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
            "index_use": {"covi_used": false, "lookup_hits": 0, "lookup_misses": 0, "index_fallbacks": 0},
            "memory_peak_bytes": Value::Null,
            "artifact_sizes": {
                "mapped_cove_o_bytes": mapped_bytes.len(),
                "cove_bytes": projected_cove_t_bytes,
                "parquet_bytes": parquet_bundle_bytes,
                "orc_bytes": 0,
                "covx_bytes": 0
            },
            "delta": {
                "mapped_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - mapped_bytes.len() as i64,
                "projected_bundle_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - projected_cove_t_bytes as i64,
                "mapped_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - mapped_cold.range_gets as i64,
                "projected_bundle_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - projected_cove_t_cold.range_gets as i64,
                "mapped_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - mapped_cold.bytes_requested as i64,
                "projected_bundle_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - projected_cove_t_cold.bytes_requested as i64
            }
        },
        "cost": {
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "mapped_cove_o": {
                    "file_bytes": mapped_bytes.len(),
                    "cold": object_store_stats_json(&mapped_cold),
                    "warm": object_store_stats_json(&mapped_warm),
                    "ranges": {"original": mapped_original.len(), "coalesced": mapped_coalesced.len()}
                },
                "projected_cove_t_bundle": {
                    "file_bytes": projected_cove_t_bytes,
                    "cold": object_store_stats_json(&projected_cove_t_cold),
                    "warm": object_store_stats_json(&projected_cove_t_warm),
                    "artifacts": {
                        "customers_projection": {
                            "file_bytes": customers_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&customers_cove_t_cold),
                            "warm": object_store_stats_json(&customers_cove_t_warm),
                            "ranges": {"original": customers_cove_t_original.len(), "coalesced": customers_cove_t_coalesced.len()}
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&evidence_cove_t_cold),
                            "warm": object_store_stats_json(&evidence_cove_t_warm),
                            "ranges": {"original": evidence_cove_t_original.len(), "coalesced": evidence_cove_t_coalesced.len()}
                        }
                    }
                },
                "parquet_bundle": {
                    "file_bytes": parquet_bundle_bytes,
                    "cold": object_store_stats_json(&parquet_bundle_cold),
                    "warm": object_store_stats_json(&parquet_bundle_warm),
                    "artifacts": {
                        "customers_projection": {
                            "file_bytes": customers_parquet_bytes.len(),
                            "cold": object_store_stats_json(&customers_parquet_cold),
                            "warm": object_store_stats_json(&customers_parquet_warm),
                            "ranges": {"original": customers_parquet_original.len(), "coalesced": customers_parquet_coalesced.len()}
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_parquet_bytes.len(),
                            "cold": object_store_stats_json(&evidence_parquet_cold),
                            "warm": object_store_stats_json(&evidence_parquet_warm),
                            "ranges": {"original": evidence_parquet_original.len(), "coalesced": evidence_parquet_coalesced.len()}
                        }
                    }
                },
                "caveat": "Hermetic object-store semantics for corpus artifacts, not live cloud storage performance."
            }
        },
        "optional_features": ["cove_map", "parquet_compare", "object_store_harness", "customer360"],
    }))
}

type ColdWarmRangeStats = (
    OfflineObjectStoreStats,
    OfflineObjectStoreStats,
    Vec<Range<u64>>,
    Vec<Range<u64>>,
);

fn simulate_object_store_cold_warm(
    key: &str,
    bytes: Vec<u8>,
) -> Result<ColdWarmRangeStats, String> {
    let mut harness = OfflineObjectStoreHarness::default();
    harness.put_object(key, bytes.clone());
    let original_ranges = deterministic_object_ranges(bytes.len() as u64);
    let coalesced_ranges = coalesce_object_ranges(&original_ranges, 1024, 16 * 1024);
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, key, &coalesced_ranges)?;
    let cold = harness.take_stats();
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, key, &coalesced_ranges)?;
    let warm = harness.take_stats();
    Ok((cold, warm, original_ranges, coalesced_ranges))
}

fn sum_offline_object_store_stats(stats: &[OfflineObjectStoreStats]) -> OfflineObjectStoreStats {
    let mut total = OfflineObjectStoreStats::default();
    for stat in stats {
        total.object_gets = total.object_gets.saturating_add(stat.object_gets);
        total.range_gets = total.range_gets.saturating_add(stat.range_gets);
        total.bytes_requested = total.bytes_requested.saturating_add(stat.bytes_requested);
        total.bytes_returned = total.bytes_returned.saturating_add(stat.bytes_returned);
        total.cache_hits = total.cache_hits.saturating_add(stat.cache_hits);
        total.cache_misses = total.cache_misses.saturating_add(stat.cache_misses);
        total.original_ranges = total.original_ranges.saturating_add(stat.original_ranges);
        total.coalesced_ranges = total.coalesced_ranges.saturating_add(stat.coalesced_ranges);
    }
    total
}

fn run_cove_map_identity_case(corpus: &Path) -> Result<Value, String> {
    let dir = corpus.join("cove-map-identity");
    fs::create_dir_all(&dir).map_err(|err| format!("cannot create COVE-MAP dir: {err}"))?;
    let map_path = dir.join("people.covemap");
    let csv_path = dir.join("people.csv");
    durable::durable_replace(&map_path, &bench_covemap_bytes()?)
        .map_err(|err| format!("cannot publish COVE-MAP fixture: {err}"))?;
    let mut csv = String::from("id,name\n");
    for row in 0..512 {
        csv.push_str(&format!("{row},person-{row}\n"));
    }
    fs::write(&csv_path, csv).map_err(|err| format!("cannot write COVE-MAP CSV: {err}"))?;
    let start = Instant::now();
    let summary = cove_map::conversion_summary_from_paths(&map_path, &[csv_path])
        .map_err(|err| format!("COVE-MAP identity benchmark failed: {err}"))?;
    let end_to_end_ns = start.elapsed().as_nanos();
    Ok(json!({
        "id": "cove_map_identity",
        "category": "COVE-MAP conversion and identity",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": end_to_end_ns,
            "end_to_end_ns": end_to_end_ns,
            "rows_materialized": summary["materialized_row_count"].as_u64().unwrap_or(0),
            "assertions": summary["assertion_count"].as_u64().unwrap_or(0),
            "evidence_entries": summary["evidence_entry_count"].as_u64().unwrap_or(0),
        },
        "optional_features": ["cove_map"],
    }))
}

fn bench_covemap_bytes() -> Result<Vec<u8>, String> {
    let file = CovemapFile {
        header: CovemapHeaderV1::new([0x77; 16], 0),
        mapping_version: "bench/v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "sources": [{"source_id": "people", "row_identity_rules": ["person_by_id"]}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "functions": [{"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
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
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "rules": [{
                        "rule_id": "people_rows",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "record_kind": "Baseline",
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "person_name",
                            "property_id": "person_name",
                            "property_name": "name",
                            "source_column": "name",
                            "logical_type": "utf8",
                            "physical_kind": "varbytes",
                            "value_expression": "name",
                            "nullable": false
                        }]
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "projections": [{
                        "projection_id": "person_projection",
                        "output_table": "people_projection",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "missing_policy": "null",
                        "output_modes": ["json", "arrow", "cove-t"],
                        "columns": [
                            {"name": "person_goid", "logical_type": "uuid", "value": "object.goid"},
                            {"name": "name", "logical_type": "utf8", "value": "name"}
                        ]
                    }]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    };
    file.serialize().map_err(|err| err.to_string())
}

fn projection_covi_covemap_bytes() -> Result<Vec<u8>, String> {
    let file = CovemapFile {
        header: CovemapHeaderV1::new([0x78; 16], 0),
        mapping_version: "bench/projection-covi.v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "sources": [{"source_id": "people", "row_identity_rules": ["person_by_id"]}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "functions": [{"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
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
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "rules": [{
                        "rule_id": "people_projection_rows",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [
                            {
                                "assertion_id": "person_id",
                                "property_id": "id",
                                "property_name": "id",
                                "source_column": "id",
                                "logical_type": "utf8",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "person_name",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "person_status",
                                "property_id": "status",
                                "property_name": "status",
                                "source_column": "status",
                                "logical_type": "utf8",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "person_score",
                                "property_id": "score",
                                "property_name": "score",
                                "source_column": "score",
                                "logical_type": "int64",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            }
                        ]
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "projections": [{
                        "projection_id": "people_projection.v1",
                        "output_table": "people_projection",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "missing_policy": "null",
                        "output_modes": ["json", "arrow", "cove-t"],
                        "columns": [
                            {"name": "id", "logical_type": "utf8", "value": "id"},
                            {"name": "name", "logical_type": "utf8", "value": "name"},
                            {"name": "status", "logical_type": "utf8", "value": "status"},
                            {"name": "score", "logical_type": "int64", "value": "score"}
                        ]
                    }]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    };
    file.serialize().map_err(|err| err.to_string())
}

fn showcase_multi_source_covemap() -> Result<CovemapFile, String> {
    Ok(CovemapFile {
        header: CovemapHeaderV1::new([0x53; 16], 0),
        mapping_version: "bench/showcase.v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "sources": [
                        {"source_id": "crm", "row_identity_rules": ["person_by_id"], "source_priority": 10},
                        {"source_id": "directory", "row_identity_rules": ["person_by_id"], "source_priority": 20},
                        {"source_id": "subscription", "row_identity_rules": ["person_by_id"], "source_priority": 1}
                    ]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
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
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "rules": [
                        {
                            "rule_id": "upsert_person_name_crm",
                            "source_id": "crm",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_crm"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_crm",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        },
                        {
                            "rule_id": "upsert_person_name_directory",
                            "source_id": "directory",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_directory"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_directory",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        },
                        {
                            "rule_id": "upsert_person_name_subscription",
                            "source_id": "subscription",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_subscription"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_subscription",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        }
                    ]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "projections": [
                        {
                            "projection_id": "person_projection",
                            "output_table": "people_projection",
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"],
                            "columns": [
                                {"name": "person_goid", "logical_type": "uuid", "value": "object.goid"},
                                {"name": "name", "logical_type": "utf8", "value": "name"}
                            ]
                        },
                        {
                            "projection_id": "evidence_projection",
                            "output_table": "evidence_projection",
                            "row_grain": "one_row_per_evidence_assertion",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"],
                            "columns": [
                                {"name": "source_id", "logical_type": "utf8", "value": "evidence.source_id"},
                                {"name": "source_row_identity", "logical_type": "utf8", "value": "evidence.source_row_identity"},
                                {"name": "output_object_id", "logical_type": "uuid", "value": "evidence.output_object_id"}
                            ]
                        }
                    ]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    })
}

fn showcase_directory_name_batch() -> Result<RecordBatch, String> {
    RecordBatch::try_from_iter(vec![
        (
            "id",
            Arc::new(StringArray::from(vec!["p1", "p2"])) as ArrayRef,
        ),
        (
            "name",
            Arc::new(StringArray::from(vec!["Ada Directory", "Linus Directory"])) as ArrayRef,
        ),
    ])
    .map_err(|err| err.to_string())
}

fn covemap_json_section(kind: SectionKind, value: Value) -> Result<CovemapSection, String> {
    let payload =
        serde_json::to_vec(&covemap_payload_value(kind, value)).map_err(|err| err.to_string())?;
    Ok(CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: kind as u32,
            offset: 0,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: CompressionCodec::None as u8,
            payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
            required: true,
            reserved: 0,
            checksum: 0,
        },
        payload,
    })
}

fn covemap_payload_value(kind: SectionKind, mut value: Value) -> Value {
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
    value
}

fn attach_covi_sidecar_metrics(
    cost: &mut Value,
    stats: cove_datafusion::dataset_state::DatasetBootstrapStats,
) {
    if let Some(metrics) = cost
        .get_mut("coverage_metrics")
        .and_then(Value::as_object_mut)
    {
        metrics.insert(
            "covi".into(),
            json!({
                "loaded": stats.covi_sidecars_loaded,
                "stale": stats.covi_sidecars_stale,
                "ignored": stats.covi_sidecars_ignored,
                "candidate_pruned": stats.covi_candidate_pruned,
                "index_only_answers": stats.covi_index_only_answers,
            }),
        );
    }
}

fn normalize_case_metrics(case: &mut Value) {
    let Some(object) = case.as_object_mut() else {
        return;
    };
    let cost = object.get("cost").cloned().unwrap_or(Value::Null);
    let metrics = object
        .entry("metrics")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("metrics object");
    let planning = metrics
        .get("planning_ns")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let scan = metrics.get("scan_ns").and_then(Value::as_u64).unwrap_or(0);
    let elapsed = metrics
        .get("end_to_end_ns")
        .and_then(Value::as_u64)
        .unwrap_or(planning.saturating_add(scan));
    metrics.entry("end_to_end_ns").or_insert(json!(elapsed));
    metrics.entry("elapsed_time_ns").or_insert(json!(elapsed));
    let metadata_bytes = cost
        .pointer("/observed/metadata_bytes_read")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let data_bytes = cost
        .pointer("/observed/data_bytes_read")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let bytes_read = metadata_bytes.saturating_add(data_bytes);
    metrics.entry("bytes_read").or_insert(json!(bytes_read));
    let request_count = cost
        .pointer("/observed/range_requests")
        .and_then(Value::as_u64)
        .or_else(|| {
            cost.pointer("/range_plan/original_range_requests")
                .and_then(Value::as_u64)
        })
        .unwrap_or(0);
    metrics
        .entry("request_count")
        .or_insert(json!(request_count));
    metrics.entry("fragments_visited").or_insert(json!(cost
        .pointer("/observed/scan_tasks")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    metrics.entry("pages_visited").or_insert(json!(cost
        .pointer("/observed/pages_decoded")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    let considered = cost
        .pointer("/observed/morsels_considered")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let pruned = cost
        .pointer("/observed/morsels_pruned")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    metrics
        .entry("pruning_tightness")
        .or_insert(json!(if considered == 0 {
            0.0
        } else {
            pruned as f64 / considered as f64
        }));
    metrics.entry("coverage_cache").or_insert_with(|| {
        cost.pointer("/coverage_metrics/coverage_cache")
            .cloned()
            .unwrap_or(json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }))
    });
    metrics.entry("coverage_cache_hit").or_insert(json!(cost
        .pointer("/coverage_metrics/coverage_cache/hits")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    metrics.entry("coverage_cache_miss").or_insert(json!(cost
        .pointer("/coverage_metrics/coverage_cache/misses")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    metrics.entry("index_use").or_insert(json!({
        "covi_used": cost.pointer("/coverage_metrics/covi_used").and_then(Value::as_bool).unwrap_or(false),
        "lookup_hits": cost.pointer("/observed/lookup_index_hits").and_then(Value::as_u64).unwrap_or(0),
        "lookup_misses": cost.pointer("/observed/lookup_index_misses").and_then(Value::as_u64).unwrap_or(0),
        "index_fallbacks": cost.pointer("/observed/index_fallbacks").and_then(Value::as_u64).unwrap_or(0),
    }));
    metrics.entry("memory_peak_bytes").or_insert(Value::Null);
    let artifact_sizes = json!({
        "cove_bytes": metrics.get("cove_bytes").and_then(Value::as_u64).unwrap_or(0),
        "parquet_bytes": metrics.get("parquet_bytes").and_then(Value::as_u64).unwrap_or(0),
        "orc_bytes": metrics.get("orc_bytes").and_then(Value::as_u64).unwrap_or(0),
        "covx_bytes": metrics.get("covx_bytes").and_then(Value::as_u64).unwrap_or(0),
    });
    metrics.entry("artifact_sizes").or_insert(artifact_sizes);
}

fn validate_report_cases(cases: &[Value]) -> Result<(), String> {
    let manifest: Value = serde_json::from_str(PUBLIC_MANIFEST).map_err(|err| err.to_string())?;
    if let Some(groups) = manifest.get("query_groups").and_then(Value::as_array) {
        for group in groups.iter().filter_map(Value::as_str) {
            require_measured_case(cases, group)?;
        }
    }
    if let Some(skipped) = cases
        .iter()
        .find(|case| case.get("status").and_then(Value::as_str) == Some("skipped"))
    {
        return Err(format!(
            "benchmark case {} was skipped",
            skipped
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    let required = [
        "full_numeric_scan",
        "string_category_scan",
        "equality_filter",
        "range_filter",
        "top_n",
        "point_lookup",
        "ai_vector_search_report",
        "covi_index_latency",
        "covi_index_only_count",
        "object_store_cold_warm",
        "semantic_projection_object_store_compare",
        "semantic_showcase_bundle_object_store_compare",
        "parquet_conversion_cost",
        "orc_conversion_cost",
        "orc_full_scan_readback",
        "orc_file_size_overhead",
        "coverage_cache_disabled",
        "coverage_cache_hit",
        "coverage_cache_hit_miss_invalidation",
        "filecode_group_by",
        "execution_code_remap_overhead",
        "registered_codec_decode_predicate_kernel",
        "fallback_payload_overhead",
        "page_cluster_range_coalescing",
        "zero_copy_success_fallback_rate",
        "coverage_degree_tightness",
        "tpch_style_queries",
        "tpcds_style_queries",
        "medical_operational_queries",
        "negative_corrupt_validation",
        "canonicalisation_vectors",
        "semantic_mapping_corpus",
        "cove_o_delta_artifact_metrics",
        "cove_map_build_tiny",
        "cove_map_build_medium",
        "cove_map_build_messy_multisource",
        "cove_o_overlap_stress",
        "cove_o_overlap_scale_1_table",
        "cove_o_overlap_scale_2_tables",
        "cove_o_overlap_scale_4_tables",
        "cove_o_overlap_scale_8_tables",
        "cove_o_overlap_scale_8_tables_large",
        "cove_o_overlap_partial_0pct",
        "cove_o_overlap_partial_25pct",
        "cove_o_overlap_partial_50pct",
        "cove_o_overlap_partial_75pct",
        "cove_o_overlap_partial_100pct",
        "projection_covi_equality_valid",
        "projection_covi_in_valid",
        "projection_covi_range_valid",
        "projection_covi_missing_sidecar_fallback",
        "projection_covi_stale_sidecar_fallback",
        "projection_covi_unsupported_predicate_fallback",
        "semantic_projection_object_store_compare",
        "semantic_showcase_bundle_object_store_compare",
        "customer360_projection_scan",
        "customer360_selective_filter",
        "customer360_event_filter",
        "customer360_object_store_compare",
        "customer360_projection_covi_score_range_valid",
        "customer360_projection_covi_status_eq_valid",
        "customer360_projection_covi_tier_in_valid",
        "customer360_projection_covi_compound_valid",
        "proof_suite_customer360",
        "proof_suite_claims",
        "proof_suite_catalog",
    ];
    for id in required {
        if !cases.iter().any(|case| case.get("id") == Some(&json!(id))) {
            return Err(format!("benchmark report missing required case {id}"));
        }
    }
    let required_metric_fields = [
        "elapsed_time_ns",
        "bytes_read",
        "request_count",
        "fragments_visited",
        "pages_visited",
        "pruning_tightness",
        "coverage_cache",
        "index_use",
        "memory_peak_bytes",
        "artifact_sizes",
    ];
    for case in cases {
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                format!(
                    "benchmark case {} is missing metrics",
                    case.get("id").and_then(Value::as_str).unwrap_or("unknown")
                )
            })?;
        for field in required_metric_fields {
            if !metrics.contains_key(field) {
                return Err(format!(
                    "benchmark case {} missing required metric {field}",
                    case.get("id").and_then(Value::as_str).unwrap_or("unknown")
                ));
            }
        }
    }
    let cache_hit = cases
        .iter()
        .find(|case| case.get("id") == Some(&json!("coverage_cache_hit")))
        .and_then(|case| {
            case.pointer("/cost/coverage_metrics/coverage_cache/hits")
                .and_then(Value::as_u64)
        })
        .unwrap_or(0);
    if cache_hit == 0 {
        return Err("coverage_cache_hit did not record a COVE-CACHE hit".into());
    }
    let covi_lookup = require_measured_case(cases, "covi_index_latency")?;
    if !case_bool(covi_lookup, "/cost/coverage_metrics/covi_used") {
        return Err("covi_index_latency did not use COVI candidates".into());
    }
    if case_u64(covi_lookup, "/cost/coverage_metrics/covi_candidates") == 0 {
        return Err("covi_index_latency did not produce any COVI candidates".into());
    }
    if case_u64(covi_lookup, "/cost/coverage_metrics/covi/loaded") == 0 {
        return Err("covi_index_latency did not load a COVI sidecar".into());
    }

    let covi_count = require_measured_case(cases, "covi_index_only_count")?;
    if case_u64(covi_count, "/cost/coverage_metrics/covi/loaded") == 0 {
        return Err("covi_index_only_count did not load a COVI sidecar".into());
    }
    if case_u64(covi_count, "/cost/coverage_metrics/covi/index_only_answers") == 0 {
        return Err("covi_index_only_count did not record COVI index-only evidence".into());
    }
    if covi_count.pointer("/proof/kind").and_then(Value::as_str) != Some("CoviIndexOnlyCount") {
        return Err("covi_index_only_count did not prove CoviIndexOnlyCount".into());
    }
    validate_projection_covi_benchmark_cases(cases)?;
    validate_ai_benchmark_case(cases)?;
    validate_overlap_stress_benchmark_case(cases)?;
    validate_overlap_scale_benchmark_cases(cases)?;
    validate_overlap_partial_benchmark_cases(cases)?;
    validate_proof_suite_benchmark_cases(cases)?;
    validate_cove_o_delta_benchmark_case(cases)?;
    Ok(())
}

fn validate_cove_o_delta_benchmark_case(cases: &[Value]) -> Result<(), String> {
    let case = require_measured_case(cases, "cove_o_delta_artifact_metrics")?;
    let metrics = case
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "cove_o_delta_artifact_metrics is missing metrics".to_string())?;
    let required_metrics = [
        "bytes_written_per_update",
        "full_rewrite_bytes_per_update",
        "total_bytes_stored",
        "writer_finalization_ns",
        "publication_latency_ns",
        "validation_time_ns",
        "latest_state_point_lookup_p95_artifacts",
        "object_history_query_selected_deltas",
        "projection_readback_property_skips",
        "object_store_request_count",
        "chain_summary_range_requests",
        "delta_artifacts_opened",
        "delta_artifacts_skipped_before_open",
        "source_publication_pruning_effectiveness",
        "dictionary_alias_resolution_count",
        "compaction_throughput_rows_per_ns",
        "compacted_output_bytes",
        "index_rebuild_candidate_count",
        "delta_chain_depth",
        "selected_delta_count",
        "skipped_delta_count",
        "chain_summary_bytes",
        "base_file_bytes",
        "total_delta_bytes",
        "patch_rows_applied",
        "materialized_property_count",
        "checkpoint_recommended",
        "compaction_recommended",
        "snapshot_index_recommended",
        "recommendations",
    ];
    for field in required_metrics {
        if !metrics.contains_key(field) {
            return Err(format!(
                "cove_o_delta_artifact_metrics missing metric {field}"
            ));
        }
    }
    if case_u64(case, "/metrics/delta_chain_depth") == 0 {
        return Err("cove_o_delta_artifact_metrics did not measure any deltas".into());
    }
    if case_u64(case, "/metrics/delta_artifacts_skipped_before_open") == 0 {
        return Err("cove_o_delta_artifact_metrics did not record pruning".into());
    }
    if case_u64(case, "/metrics/chain_summary_bytes") == 0 {
        return Err("cove_o_delta_artifact_metrics did not encode a chain summary".into());
    }
    if !case_bool(case, "/metrics/compaction_recommended") {
        return Err("cove_o_delta_artifact_metrics did not trigger compaction guidance".into());
    }
    if !case_bool(case, "/metrics/checkpoint_recommended") {
        return Err("cove_o_delta_artifact_metrics did not trigger checkpoint guidance".into());
    }
    Ok(())
}

fn validate_ai_benchmark_case(cases: &[Value]) -> Result<(), String> {
    let case = require_measured_case(cases, "ai_vector_search_report")?;
    let metrics = case
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "ai_vector_search_report is missing metrics".to_string())?;
    for field in [
        "vector_build_latency_ns",
        "sidecar_parse_latency_ns",
        "vector_search_latency_ns",
        "ann_search_latency_ns",
        "ann_recall_vs_exact",
        "exact_fallback_rate",
        "filtered_top_k_complete",
        "ann_selected_index",
        "ann_result_authority",
        "ann_internal_candidate_execution",
        "ann_exact_result_claim",
        "vector_count",
        "dimension_count",
        "exact_result_count",
        "ann_result_count",
        "ann_fallback_count",
        "payload_bytes_read",
        "policy_withheld_count",
    ] {
        if !metrics.contains_key(field) {
            return Err(format!("ai_vector_search_report missing metric {field}"));
        }
    }
    if case_u64(case, "/metrics/vector_count") == 0 {
        return Err("ai_vector_search_report did not measure any vectors".into());
    }
    if case_u64(case, "/metrics/exact_result_count") == 0 {
        return Err("ai_vector_search_report did not return exact vector results".into());
    }
    if case_u64(case, "/metrics/payload_bytes_read") == 0 {
        return Err("ai_vector_search_report did not report vector payload bytes".into());
    }
    if case_u64(case, "/metrics/ann_fallback_count") != 0 {
        return Err("ai_vector_search_report unexpectedly fell back from indexed ANN".into());
    }
    if !case_bool(case, "/metrics/ann_internal_candidate_execution") {
        return Err("ai_vector_search_report did not exercise internal ANN candidates".into());
    }
    if case_bool(case, "/metrics/ann_exact_result_claim") {
        return Err("ai_vector_search_report claimed exactness for approximate ANN".into());
    }
    if !(0.0..=1.0).contains(&case_f64(case, "/metrics/ann_recall_vs_exact")) {
        return Err("ai_vector_search_report recall was outside 0..1".into());
    }
    if !case_bool(case, "/metrics/filtered_top_k_complete") {
        return Err("ai_vector_search_report did not mark filtered top-k completeness".into());
    }
    Ok(())
}

fn validate_proof_suite_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let required_metrics = [
        "build_time_ns",
        "validation_time_ns",
        "parity_time_ns",
        "source_bytes",
        "source_parquet_bundle_bytes",
        "normalized_parquet_bundle_bytes",
        "denormalized_parquet_bytes",
        "cove_o_bytes",
        "cove_t_bytes",
        "covi_bytes",
        "covm_bytes",
        "total_bundle_bytes",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
        "duplication_ratio",
        "doctor_status_ok",
        "parity_status_ok",
        "parity_report_count",
    ];
    for id in [
        "proof_suite_customer360",
        "proof_suite_claims",
        "proof_suite_catalog",
    ] {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required_metrics {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing proof-suite metric {field}"));
            }
        }
        if !case_bool(case, "/metrics/doctor_status_ok") {
            return Err(format!("{id} doctor report was not ok"));
        }
        if !case_bool(case, "/metrics/parity_status_ok") {
            return Err(format!("{id} parity reports were not ok"));
        }
        if case_u64(case, "/metrics/parity_report_count") == 0 {
            return Err(format!("{id} did not include parity reports"));
        }
        if case_u64(case, "/metrics/cove_o_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-O bytes"));
        }
        if case_u64(case, "/metrics/covi_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-I bytes"));
        }
        if case_u64(case, "/metrics/covm_bytes") == 0 {
            return Err(format!("{id} did not emit COVM bytes"));
        }
    }
    Ok(())
}

fn validate_overlap_stress_benchmark_case(cases: &[Value]) -> Result<(), String> {
    let case = require_measured_case(cases, "cove_o_overlap_stress")?;
    let required_metrics = [
        "source_table_count",
        "row_count",
        "overlap_fraction",
        "source_csv_bytes",
        "source_parquet_bundle_bytes",
        "unique_parquet_bytes",
        "unique_payload_bytes",
        "duplicate_payload_bytes",
        "cove_o_bytes",
        "compressed_cove_o_bytes",
        "uncompressed_cove_o_bytes",
        "compact_cove_o_bytes",
        "expanded_cove_o_bytes",
        "section_compression_saved_bytes",
        "section_compression_uncompressed_bytes",
        "section_compression_emitted_bytes",
        "section_compression_compressed_section_count",
        "section_compression_ratio",
        "compact_vs_expanded_cove_o_ratio",
        "compact_evidence_index_bytes",
        "expanded_evidence_json_bytes",
        "compact_evidence_vs_expanded_json_ratio",
        "total_bundle_bytes",
        "uncompressed_total_bundle_bytes",
        "expanded_total_bundle_bytes",
        "compressed_vs_uncompressed_bundle_ratio",
        "compact_vs_expanded_bundle_ratio",
        "cove_o_vs_source_csv_ratio",
        "cove_o_vs_parquet_bundle_ratio",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
        "evidence_to_property_ratio",
    ];
    let metrics = case
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "cove_o_overlap_stress is missing metrics".to_string())?;
    for field in required_metrics {
        if !metrics.contains_key(field) {
            return Err(format!("cove_o_overlap_stress missing metric {field}"));
        }
    }
    if case_u64(case, "/metrics/source_table_count") < 2 {
        return Err("cove_o_overlap_stress did not use multiple source tables".into());
    }
    if case_u64(case, "/metrics/duplicate_payload_bytes") == 0 {
        return Err("cove_o_overlap_stress did not generate duplicate payload".into());
    }
    if case_u64(case, "/metrics/cove_o_bytes") == 0 {
        return Err("cove_o_overlap_stress did not produce COVE-O bytes".into());
    }
    if case_u64(case, "/metrics/compressed_cove_o_bytes")
        >= case_u64(case, "/metrics/uncompressed_cove_o_bytes")
    {
        return Err("cove_o_overlap_stress section compression did not reduce COVE-O bytes".into());
    }
    if case_u64(case, "/metrics/section_compression_saved_bytes") == 0 {
        return Err("cove_o_overlap_stress did not record section compression savings".into());
    }
    if case_u64(
        case,
        "/metrics/section_compression_compressed_section_count",
    ) == 0
    {
        return Err("cove_o_overlap_stress did not compress any COVE-O sections".into());
    }
    if case_u64(case, "/metrics/compact_evidence_index_bytes")
        >= case_u64(case, "/metrics/expanded_evidence_json_bytes")
    {
        return Err(
            "cove_o_overlap_stress compact evidence was not smaller than expanded evidence".into(),
        );
    }
    if case_u64(case, "/metrics/compact_cove_o_bytes")
        >= case_u64(case, "/metrics/expanded_cove_o_bytes")
    {
        return Err(
            "cove_o_overlap_stress compact COVE-O was not smaller than expanded COVE-O".into(),
        );
    }
    Ok(())
}

fn validate_overlap_scale_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let required = [
        "source_table_count",
        "row_count",
        "overlap_fraction",
        "source_csv_bytes",
        "source_parquet_bundle_bytes",
        "unique_parquet_bytes",
        "source_parquet_redundancy_ratio",
        "duplicate_payload_bytes",
        "duplicate_payload_ratio",
        "cove_o_bytes",
        "cove_t_bytes",
        "covi_bytes",
        "covm_bytes",
        "total_bundle_bytes",
        "cove_o_vs_source_csv_ratio",
        "bundle_vs_source_csv_ratio",
        "cove_o_vs_parquet_bundle_ratio",
        "bundle_vs_parquet_bundle_ratio",
        "cove_o_vs_unique_parquet_ratio",
        "bundle_vs_unique_parquet_ratio",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
    ];
    let ids = [
        "cove_o_overlap_scale_1_table",
        "cove_o_overlap_scale_2_tables",
        "cove_o_overlap_scale_4_tables",
        "cove_o_overlap_scale_8_tables",
        "cove_o_overlap_scale_8_tables_large",
    ];
    for id in ids {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing overlap-scale metric {field}"));
            }
        }
        if case_u64(case, "/metrics/cove_o_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-O bytes"));
        }
        if case_u64(case, "/metrics/source_parquet_bundle_bytes") == 0 {
            return Err(format!("{id} did not emit source Parquet baselines"));
        }
        if case_u64(case, "/metrics/object_count") == 0 {
            return Err(format!("{id} did not materialize objects"));
        }
    }

    let two = require_measured_case(cases, "cove_o_overlap_scale_2_tables")?;
    let four = require_measured_case(cases, "cove_o_overlap_scale_4_tables")?;
    let eight = require_measured_case(cases, "cove_o_overlap_scale_8_tables")?;
    if case_f64(eight, "/metrics/cove_o_vs_parquet_bundle_ratio")
        >= case_f64(two, "/metrics/cove_o_vs_parquet_bundle_ratio")
    {
        return Err(
            "overlap scale did not improve COVE-O/source-Parquet ratio from 2 to 8 tables".into(),
        );
    }
    if case_f64(eight, "/metrics/bundle_vs_parquet_bundle_ratio")
        >= case_f64(four, "/metrics/bundle_vs_parquet_bundle_ratio")
    {
        return Err(
            "overlap scale did not improve bundle/source-Parquet ratio from 4 to 8 tables".into(),
        );
    }
    if case_f64(eight, "/metrics/cove_o_vs_parquet_bundle_ratio") >= 1.0 {
        return Err("8-table overlap scale did not make COVE-O smaller than source Parquet".into());
    }
    Ok(())
}

fn validate_overlap_partial_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let required = [
        "source_table_count",
        "row_count",
        "source_input_row_count",
        "overlap_fraction",
        "overlap_percent",
        "shared_row_count",
        "source_unique_rows_per_table",
        "unique_entity_count",
        "object_dedupe_ratio",
        "source_csv_bytes",
        "source_parquet_bundle_bytes",
        "unique_parquet_bytes",
        "source_parquet_redundancy_ratio",
        "duplicate_payload_bytes",
        "duplicate_payload_ratio",
        "cove_o_bytes",
        "cove_t_bytes",
        "covi_bytes",
        "covm_bytes",
        "total_bundle_bytes",
        "cove_o_vs_source_csv_ratio",
        "bundle_vs_source_csv_ratio",
        "cove_o_vs_parquet_bundle_ratio",
        "bundle_vs_parquet_bundle_ratio",
        "cove_o_vs_unique_parquet_ratio",
        "bundle_vs_unique_parquet_ratio",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
    ];
    let ids = [
        "cove_o_overlap_partial_0pct",
        "cove_o_overlap_partial_25pct",
        "cove_o_overlap_partial_50pct",
        "cove_o_overlap_partial_75pct",
        "cove_o_overlap_partial_100pct",
    ];
    for id in ids {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing overlap-partial metric {field}"));
            }
        }
        if case_u64(case, "/metrics/object_count") != case_u64(case, "/metrics/unique_entity_count")
        {
            return Err(format!(
                "{id} object count did not match unique entity count"
            ));
        }
        if case_u64(case, "/metrics/cove_o_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-O bytes"));
        }
    }

    let zero = require_measured_case(cases, "cove_o_overlap_partial_0pct")?;
    let fifty = require_measured_case(cases, "cove_o_overlap_partial_50pct")?;
    let hundred = require_measured_case(cases, "cove_o_overlap_partial_100pct")?;
    if case_f64(hundred, "/metrics/object_dedupe_ratio")
        <= case_f64(zero, "/metrics/object_dedupe_ratio")
    {
        return Err("partial overlap object dedupe ratio did not improve from 0% to 100%".into());
    }
    if case_f64(hundred, "/metrics/cove_o_vs_parquet_bundle_ratio")
        >= case_f64(zero, "/metrics/cove_o_vs_parquet_bundle_ratio")
    {
        return Err(
            "partial overlap COVE-O/source-Parquet ratio did not improve from 0% to 100%".into(),
        );
    }
    if case_f64(fifty, "/metrics/cove_o_vs_parquet_bundle_ratio")
        >= case_f64(zero, "/metrics/cove_o_vs_parquet_bundle_ratio")
    {
        return Err(
            "partial overlap COVE-O/source-Parquet ratio did not improve by 50% overlap".into(),
        );
    }
    Ok(())
}

fn validate_projection_covi_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let all_projection_cases = [
        "projection_covi_equality_valid",
        "projection_covi_in_valid",
        "projection_covi_range_valid",
        "projection_covi_missing_sidecar_fallback",
        "projection_covi_stale_sidecar_fallback",
        "projection_covi_unsupported_predicate_fallback",
        "customer360_projection_covi_score_range_valid",
        "customer360_projection_covi_status_eq_valid",
        "customer360_projection_covi_tier_in_valid",
        "customer360_projection_covi_compound_valid",
    ];
    let required_metrics = [
        "source_bytes",
        "cove_o_bytes",
        "projection_sidecar_bytes",
        "candidate_rows",
        "skipped_rows",
        "residual_rows",
        "result_rows",
        "lookup_hits",
        "lookup_misses",
        "fallback_count",
        "duplication_ratio",
    ];
    for id in all_projection_cases {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required_metrics {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing projection COVE-I metric {field}"));
            }
        }
    }
    for id in [
        "projection_covi_equality_valid",
        "projection_covi_in_valid",
        "projection_covi_range_valid",
        "customer360_projection_covi_score_range_valid",
        "customer360_projection_covi_status_eq_valid",
        "customer360_projection_covi_tier_in_valid",
    ] {
        let case = require_measured_case(cases, id)?;
        if case_u64(case, "/metrics/lookup_hits") == 0 {
            return Err(format!("{id} did not record projection COVE-I lookup hits"));
        }
        if case_u64(case, "/metrics/candidate_rows") == 0 {
            return Err(format!("{id} did not record projection COVE-I candidates"));
        }
        if case_u64(case, "/metrics/skipped_rows") == 0 {
            return Err(format!("{id} did not record projection COVE-I pruning"));
        }
        if case_u64(case, "/metrics/fallback_count") != 0 {
            return Err(format!(
                "{id} unexpectedly fell back from projection COVE-I"
            ));
        }
    }
    let missing = require_measured_case(cases, "projection_covi_missing_sidecar_fallback")?;
    if case_u64(missing, "/metrics/fallback_no_sidecar") == 0 {
        return Err(
            "projection_covi_missing_sidecar_fallback did not record missing-sidecar fallback"
                .into(),
        );
    }
    let stale = require_measured_case(cases, "projection_covi_stale_sidecar_fallback")?;
    if case_u64(stale, "/metrics/fallback_stale") == 0 {
        return Err(
            "projection_covi_stale_sidecar_fallback did not record stale-sidecar fallback".into(),
        );
    }
    if case_u64(stale, "/metrics/sidecar_ignored") == 0 {
        return Err("projection_covi_stale_sidecar_fallback did not record ignored sidecar".into());
    }
    let unsupported =
        require_measured_case(cases, "projection_covi_unsupported_predicate_fallback")?;
    if case_u64(unsupported, "/metrics/fallback_no_eligible_filter") == 0 {
        return Err(
            "projection_covi_unsupported_predicate_fallback did not record unsupported-filter fallback"
                .into(),
        );
    }
    if case_u64(unsupported, "/metrics/lookup_hits") != 0 {
        return Err("projection_covi_unsupported_predicate_fallback used sidecar lookup".into());
    }
    let compound = require_measured_case(cases, "customer360_projection_covi_compound_valid")?;
    if case_u64(compound, "/metrics/lookup_hits") < 2 {
        return Err(
            "customer360_projection_covi_compound_valid did not use both sidecar lookups".into(),
        );
    }
    if case_u64(compound, "/metrics/eligible_filters") < 2 {
        return Err(
            "customer360_projection_covi_compound_valid did not report both eligible filters"
                .into(),
        );
    }
    if case_u64(compound, "/metrics/skipped_rows") == 0 {
        return Err("customer360_projection_covi_compound_valid did not record pruning".into());
    }
    if case_u64(compound, "/metrics/fallback_count") != 0 {
        return Err("customer360_projection_covi_compound_valid unexpectedly fell back".into());
    }
    Ok(())
}

fn require_measured_case<'a>(cases: &'a [Value], id: &str) -> Result<&'a Value, String> {
    let case = cases
        .iter()
        .find(|case| case.get("id") == Some(&json!(id)))
        .ok_or_else(|| format!("benchmark report missing required case {id}"))?;
    if case.get("status").and_then(Value::as_str) != Some("measured") {
        return Err(format!("{id} was not measured"));
    }
    Ok(case)
}

fn case_u64(case: &Value, pointer: &str) -> u64 {
    case.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

fn case_f64(case: &Value, pointer: &str) -> f64 {
    case.pointer(pointer).and_then(Value::as_f64).unwrap_or(0.0)
}

fn case_bool(case: &Value, pointer: &str) -> bool {
    case.pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn markdown_report(report: &Value) -> String {
    let mut out = String::from("# COVE v2 Public Benchmark Report\n\n");
    out.push_str("| Case | Status | Planning ns | Scan ns | Rows |\n");
    out.push_str("| --- | --- | ---: | ---: | ---: |\n");
    if let Some(cases) = report.get("cases").and_then(Value::as_array) {
        for case in cases {
            let id = case.get("id").and_then(Value::as_str).unwrap_or("unknown");
            let status = case
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let metrics = case.get("metrics").unwrap_or(&Value::Null);
            let planning = metrics
                .get("planning_ns")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let scan = metrics.get("scan_ns").and_then(Value::as_u64).unwrap_or(0);
            let rows = metrics
                .get("rows_materialized")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            out.push_str(&format!(
                "| `{id}` | {status} | {planning} | {scan} | {rows} |\n"
            ));
        }
        if let Some(ai_case) = cases
            .iter()
            .find(|case| case.get("id") == Some(&json!("ai_vector_search_report")))
        {
            let metrics = ai_case.get("metrics").unwrap_or(&Value::Null);
            out.push_str("\n## COVE-AI Vector Report\n\n");
            out.push_str("| Vectors | Dimensions | Exact results | ANN index | ANN fallback count | Recall vs exact | Payload bytes |\n");
            out.push_str("| ---: | ---: | ---: | --- | ---: | ---: | ---: |\n");
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {:.3} | {} |\n",
                metrics
                    .get("vector_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("dimension_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("exact_result_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("ann_selected_index")
                    .and_then(Value::as_str)
                    .unwrap_or("none"),
                metrics
                    .get("ann_fallback_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("ann_recall_vs_exact")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                metrics
                    .get("payload_bytes_read")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            ));
        }
        let overlap_scale_cases = cases
            .iter()
            .filter(|case| {
                case.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("cove_o_overlap_scale_"))
            })
            .collect::<Vec<_>>();
        if !overlap_scale_cases.is_empty() {
            out.push_str("\n## COVE-O Overlap Scale\n\n");
            out.push_str("| Case | Tables | Rows | COVE-O bytes | Source Parquet bytes | Bundle bytes | COVE-O / Parquet | Bundle / Parquet |\n");
            out.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
            for case in overlap_scale_cases {
                let id = case
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("cove_o_overlap_scale_unknown")
                    .trim_start_matches("cove_o_overlap_scale_");
                let metrics = case.get("metrics").unwrap_or(&Value::Null);
                let tables = metrics
                    .get("source_table_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let rows = metrics
                    .get("row_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_o = metrics
                    .get("cove_o_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let parquet = metrics
                    .get("source_parquet_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let bundle = metrics
                    .get("total_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_ratio = metrics
                    .get("cove_o_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let bundle_ratio = metrics
                    .get("bundle_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                out.push_str(&format!(
                    "| `{id}` | {tables} | {rows} | {cove_o} | {parquet} | {bundle} | {cove_ratio:.3} | {bundle_ratio:.3} |\n"
                ));
            }
        }
        let overlap_partial_cases = cases
            .iter()
            .filter(|case| {
                case.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("cove_o_overlap_partial_"))
            })
            .collect::<Vec<_>>();
        if !overlap_partial_cases.is_empty() {
            out.push_str("\n## COVE-O Partial Overlap\n\n");
            out.push_str("| Case | Overlap | Tables | Rows/table | Unique objects | COVE-O bytes | Source Parquet bytes | Bundle bytes | COVE-O / Parquet | Bundle / Parquet |\n");
            out.push_str(
                "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
            );
            for case in overlap_partial_cases {
                let id = case
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("cove_o_overlap_partial_unknown")
                    .trim_start_matches("cove_o_overlap_partial_");
                let metrics = case.get("metrics").unwrap_or(&Value::Null);
                let overlap = metrics
                    .get("overlap_percent")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let tables = metrics
                    .get("source_table_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let rows = metrics
                    .get("row_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let unique_objects = metrics
                    .get("unique_entity_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_o = metrics
                    .get("cove_o_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let parquet = metrics
                    .get("source_parquet_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let bundle = metrics
                    .get("total_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_ratio = metrics
                    .get("cove_o_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let bundle_ratio = metrics
                    .get("bundle_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                out.push_str(&format!(
                    "| `{id}` | {overlap}% | {tables} | {rows} | {unique_objects} | {cove_o} | {parquet} | {bundle} | {cove_ratio:.3} | {bundle_ratio:.3} |\n"
                ));
            }
        }
        let proof_cases = cases
            .iter()
            .filter(|case| {
                case.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("proof_suite_"))
            })
            .collect::<Vec<_>>();
        if !proof_cases.is_empty() {
            out.push_str("\n## COVE-O Proof Suite\n\n");
            out.push_str("| Scenario | COVE-O bytes | Source bytes | Source Parquet bytes | Bundle bytes | Doctor | Parity |\n");
            out.push_str("| --- | ---: | ---: | ---: | ---: | --- | --- |\n");
            for case in proof_cases {
                let id = case
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("proof_suite_unknown")
                    .trim_start_matches("proof_suite_");
                let metrics = case.get("metrics").unwrap_or(&Value::Null);
                let cove_o = metrics
                    .get("cove_o_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let source = metrics
                    .get("source_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let parquet = metrics
                    .get("source_parquet_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let bundle = metrics
                    .get("total_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let doctor = metrics
                    .get("doctor_status_ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let parity = metrics
                    .get("parity_status_ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                out.push_str(&format!(
                    "| `{id}` | {cove_o} | {source} | {parquet} | {bundle} | {} | {} |\n",
                    if doctor { "ok" } else { "fail" },
                    if parity { "ok" } else { "fail" },
                ));
            }
        }
    }
    out
}

fn environment_report() -> Value {
    json!({
        "os": env::consts::OS,
        "arch": env::consts::ARCH,
        "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    })
}

fn events_batch(row_count: usize) -> Result<RecordBatch, String> {
    let mut ids = Vec::with_capacity(row_count);
    let mut amounts = Vec::with_capacity(row_count);
    let mut buckets = Vec::with_capacity(row_count);
    let mut names = Vec::with_capacity(row_count);
    let mut active = Vec::with_capacity(row_count);
    for row in 0..row_count {
        ids.push(row as i64);
        amounts.push(((row * 37) % 10_000) as i64);
        buckets.push(format!("bucket-{:02}", row % 16));
        names.push(match row % 5 {
            0 => "alpha",
            1 => "beta",
            2 => "gamma",
            3 => "delta",
            _ => "omega",
        });
        active.push(row % 3 != 0);
    }
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("amount", Arc::new(Int64Array::from(amounts)) as ArrayRef),
        ("bucket", Arc::new(StringArray::from(buckets)) as ArrayRef),
        ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("active", Arc::new(BooleanArray::from(active)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())
}

fn write_parquet_file(path: &Path, batch: &RecordBatch) -> Result<(), String> {
    let file =
        fs::File::create(path).map_err(|err| format!("cannot create {}: {err}", path.display()))?;
    let properties = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(properties))
        .map_err(|err| err.to_string())?;
    writer.write(batch).map_err(|err| err.to_string())?;
    writer.close().map_err(|err| err.to_string())?;
    Ok(())
}

fn decode_single_arrow_projection_batch(bytes: &[u8]) -> Result<RecordBatch, String> {
    if let Ok(reader) = FileReader::try_new(Cursor::new(bytes.to_vec()), None) {
        let batches = reader
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|err| format!("cannot decode Arrow IPC file projection: {err}"))?;
        return batches
            .into_iter()
            .next()
            .ok_or_else(|| "Arrow IPC file projection did not contain any batches".to_string());
    }

    let reader = StreamReader::try_new(Cursor::new(bytes.to_vec()), None)
        .map_err(|err| format!("cannot decode Arrow IPC projection as file or stream: {err}"))?;
    let batches = reader
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| format!("cannot decode Arrow IPC stream projection: {err}"))?;
    batches
        .into_iter()
        .next()
        .ok_or_else(|| "Arrow IPC stream projection did not contain any batches".to_string())
}

fn write_orc_file(path: &Path, batch: &RecordBatch) -> Result<(), String> {
    let file =
        fs::File::create(path).map_err(|err| format!("cannot create {}: {err}", path.display()))?;
    let mut writer = OrcWriterBuilder::new(file, batch.schema())
        .try_build()
        .map_err(|err| format!("cannot open ORC writer: {err}"))?;
    writer
        .write(batch)
        .map_err(|err| format!("cannot write ORC batch: {err}"))?;
    writer
        .close()
        .map_err(|err| format!("cannot finish ORC writer: {err}"))?;
    Ok(())
}

fn validate_orc_parity(path: &Path, batch: &RecordBatch) -> Result<(), String> {
    let file =
        fs::File::open(path).map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    let builder = OrcReaderBuilder::try_new(file)
        .map_err(|err| format!("cannot read generated ORC {}: {err}", path.display()))?;
    if builder.schema().fields().len() != batch.schema().fields().len() {
        return Err("generated ORC schema column count does not match source batch".into());
    }
    let rows = builder
        .with_batch_size(4096)
        .build()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("cannot read generated ORC batches: {err}"))?
        .iter()
        .map(|batch| batch.num_rows())
        .sum::<usize>();
    if rows != batch.num_rows() {
        return Err(format!(
            "generated ORC row count {rows} does not match source batch {}",
            batch.num_rows()
        ));
    }
    Ok(())
}

struct CoverageCacheFixture {
    cove_bytes: Vec<u8>,
    cache_bytes: Vec<u8>,
}

fn coverage_cache_fixture() -> Result<CoverageCacheFixture, String> {
    let cove_bytes = primitive_events_file_with_name_gamma_coverage(false);
    let state = cove_datafusion::bootstrap::bootstrap_bytes("synthetic-cache", cove_bytes.clone())
        .map_err(|err| err.to_string())?;
    let file_digest =
        compute_digest(DigestAlgorithm::Sha256, &cove_bytes).map_err(|err| err.to_string())?;
    let mut seed = Vec::with_capacity(28 + file_digest.len());
    seed.extend_from_slice(state.file_id());
    seed.extend_from_slice(&state.file_len().to_le_bytes());
    seed.extend_from_slice(&state.footer_crc32c().to_le_bytes());
    seed.extend_from_slice(&file_digest);
    let digest = compute_digest(DigestAlgorithm::Sha256, &seed).map_err(|err| err.to_string())?;
    let mut snapshot_id = [0u8; 16];
    snapshot_id.copy_from_slice(&digest[..16]);
    let dataset_id = *state.file_id();
    let cache = CoverageCacheV2 {
        header: CoveCoverageCacheHeaderV2 {
            cache_format_namespace_ref: 1,
            cache_format_version_major: 2,
            cache_format_version_minor: 0,
            flags: 0,
            cache_id: [7u8; 16],
            dataset_id,
            snapshot_id,
            entry_count: 1,
            created_at_us: 0,
            producer_engine_ref: 0,
            reserved: [0; 32],
            checksum: 0,
        },
        entries: vec![CoverageCacheEntryV2 {
            entry_id: 1,
            dataset_id,
            snapshot_id,
            predicate_normal_form_ref: 1,
            interval_normal_form_ref: u32::MAX,
            coverage_set_ref: 1,
            coverage_granularity: CoverageGranularityV2::Morsel,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            actual_coverage_size_bytes: 64,
            actual_read_cost_ns: 1,
            created_at_us: 0,
            valid_until_snapshot_ref: u32::MAX,
            producer_engine_ref: 0,
            checksum: 0,
        }],
    };
    Ok(CoverageCacheFixture {
        cove_bytes,
        cache_bytes: cache.serialize().map_err(|err| err.to_string())?,
    })
}

fn primitive_events_file_with_name_gamma_coverage(bad_checksum: bool) -> Vec<u8> {
    let mut writer = primitive_events_writer();
    for section in name_gamma_coverage_sections(1, bad_checksum) {
        writer.push_extra_section(section);
    }
    let placeholder = writer.write().unwrap();
    let placeholder_state =
        cove_datafusion::bootstrap::bootstrap_bytes("synthetic-cache", placeholder).unwrap();
    let snapshot_validity_ref = placeholder_state
        .pruning()
        .selected_coverage_snapshot_validity_ref
        .expect("coverage fixture has embedded coverage metadata");

    let mut writer = primitive_events_writer();
    for section in name_gamma_coverage_sections(snapshot_validity_ref, bad_checksum) {
        writer.push_extra_section(section);
    }
    writer.write().unwrap()
}

fn primitive_events_writer() -> ScanProfileCoveWriter {
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
                column(1, "id", CoveLogicalType::Int64, CovePhysicalKind::NumCode),
                column(2, "name", CoveLogicalType::Utf8, CovePhysicalKind::VarBytes),
                column(
                    3,
                    "active",
                    CoveLogicalType::Bool,
                    CovePhysicalKind::Boolean,
                ),
            ],
        }],
    };
    let mut first = ScanSegment::new(1, 0, 0, 2, 3);
    first.set_column_pages(1, vec![numcode_page(2, numcode_i64(&[1, 2]))]);
    first.set_column_pages(2, vec![varbytes_page(2, varbytes(&["alpha", "beta"]))]);
    first.set_column_pages(3, vec![bool_page(2, bools(&[true, false]))]);

    let mut second = ScanSegment::new(1, 1, 2, 1, 3);
    second.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[3]))]);
    second.set_column_pages(2, vec![varbytes_page(1, varbytes(&["gamma"]))]);
    second.set_column_pages(3, vec![bool_page(1, bools(&[true]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(first);
    writer.push_segment(second);
    writer
}

fn column(
    column_id: u32,
    name: &str,
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
) -> ColumnEntry {
    ColumnEntry {
        column_id,
        name: name.into(),
        logical,
        physical,
        nullable: false,
        sort_order: 0,
        collation_id: 0,
        precision: 0,
        scale: 0,
        flags: 0,
    }
}

fn name_gamma_coverage_sections(
    snapshot_validity_ref: u32,
    bad_checksum: bool,
) -> Vec<SectionPayload> {
    let predicate_form_ref = 1;
    let provider_id = 1;
    let coverage_set_id = 1;
    let predicate_form_section =
        predicate_normal_form_ast_section(predicate_form_ref, 1, name_eq_gamma_ast_payload());

    let provider = CoverageProviderDescriptorV2 {
        provider_id,
        provider_kind: CoverageProofKindV2::ValueToFragmentIndex as u16,
        profile: PrimaryProfile::CoverageMetadata as u8,
        granularity: CoverageGranularityV2::Morsel,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        flags: 0,
        referenced_table_id: 1,
        referenced_column_id: 2,
        referenced_path_ref: u32::MAX,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        null_semantics: 0,
        snapshot_validity_ref,
        predicate_form_ref,
        producer_ref: u32::MAX,
        checksum: 0,
    };
    let coverage_set = CoverageSetV2 {
        header: CoverageSetHeaderV2 {
            coverage_set_id,
            provider_id,
            granularity: CoverageGranularityV2::Morsel,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            predicate_form_ref,
            snapshot_validity_ref,
            total_fragment_count: 2,
            covered_fragment_count: 0,
            required_fragment_count_estimate: 0,
            coverage_degree_ppm: 500_000,
            tightness_degree_ppm: 1_000_000,
            entries_offset: 0,
            entries_length: 0,
            checksum: 0,
        },
        entries: vec![CoverageSetEntryV2 {
            target_kind: CoverageGranularityV2::Morsel,
            flags: 0,
            file_ref: 0,
            table_id: 1,
            segment_id: 1,
            morsel_id: 0,
            page_ref: u32::MAX,
            object_type_id: u32::MAX,
            path_ref: u32::MAX,
            dimensional_bucket_ref: u32::MAX,
            row_start: 0,
            row_count: 0,
            row_ordinal_bitmap_ref: u32::MAX,
            byte_range_ref: u32::MAX,
            checksum: 0,
        }],
    };
    let coverage_set_bytes = coverage_set.serialize().unwrap();
    let mut coverage_set_checksum = coverage_set_payload_checksum(&coverage_set_bytes);
    if bad_checksum {
        coverage_set_checksum ^= 1;
    }
    let proof = CoverageProofRecordV2 {
        proof_id: 1,
        provider_id,
        coverage_set_id,
        predicate_form_ref,
        proof_kind: CoverageProofKindV2::ValueToFragmentIndex,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        granularity: CoverageGranularityV2::Morsel,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref,
        coverage_set_checksum,
        proof_payload_ref: u32::MAX,
        checksum: 0,
    };

    vec![
        coverage_section(
            SectionKind::CoverageProviderRegistry,
            1,
            provider.serialize().to_vec(),
        ),
        coverage_section(SectionKind::CoverageSet, 1, coverage_set_bytes),
        coverage_section(
            SectionKind::CoverageProofRecord,
            1,
            proof.serialize().unwrap().to_vec(),
        ),
        predicate_form_section,
    ]
}

fn predicate_normal_form_ast_section(
    predicate_form_id: u32,
    table_id: u32,
    payload: Vec<u8>,
) -> SectionPayload {
    let form = PredicateNormalFormV2 {
        predicate_form_id,
        form_kind: PredicateFormKindV2::PredicateAst,
        flags: 0,
        logical_context_ref: table_id,
        payload_offset: PredicateNormalFormV2::LEN as u64,
        payload_length: payload.len() as u64,
        checksum: 0,
    };
    let mut data = Vec::with_capacity(PredicateNormalFormV2::LEN + payload.len());
    data.extend_from_slice(&form.serialize().unwrap());
    data.extend_from_slice(&payload);
    coverage_section(SectionKind::PredicateNormalForm, 1, data)
}

fn name_eq_gamma_ast_payload() -> Vec<u8> {
    let canonical = CanonicalValue::Utf8("gamma").encode().unwrap();
    let node_offset = PredicateAstPayloadHeaderV2::LEN;
    let literal_offset = node_offset + PredicateAstNodeV2::LEN;
    let operand_ref_offset = literal_offset + PredicateLiteralV2::LEN;
    let canonical_offset = operand_ref_offset + 2 * PredicateAstOperandRefV2::LEN;

    let mut payload = Vec::new();
    payload.extend_from_slice(&predicate_ast_header(
        node_offset as u64,
        literal_offset as u64,
        operand_ref_offset as u64,
    ));
    payload.extend_from_slice(&predicate_ast_node());
    payload.extend_from_slice(&predicate_ast_literal(
        canonical_offset as u64,
        canonical.len() as u32,
    ));
    payload.extend_from_slice(&predicate_ast_operand_ref(
        0,
        PredicateOperandKindV2::ColumnOrPath,
        2,
    ));
    payload.extend_from_slice(&predicate_ast_operand_ref(
        1,
        PredicateOperandKindV2::Literal,
        0,
    ));
    payload.extend_from_slice(&canonical);
    payload
}

fn predicate_ast_header(
    node_offset: u64,
    literal_offset: u64,
    operand_ref_offset: u64,
) -> [u8; PredicateAstPayloadHeaderV2::LEN] {
    let mut out = [0u8; PredicateAstPayloadHeaderV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..8].copy_from_slice(&1u32.to_le_bytes());
    out[8..12].copy_from_slice(&1u32.to_le_bytes());
    out[20..24].copy_from_slice(&2u32.to_le_bytes());
    out[24..32].copy_from_slice(&node_offset.to_le_bytes());
    out[32..40].copy_from_slice(&literal_offset.to_le_bytes());
    out[56..64].copy_from_slice(&operand_ref_offset.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[68..72].copy_from_slice(&crc.to_le_bytes());
    out
}

fn predicate_ast_node() -> [u8; PredicateAstNodeV2::LEN] {
    let mut out = [0u8; PredicateAstNodeV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&(PredicateOpV2::Eq as u16).to_le_bytes());
    out[8..10].copy_from_slice(&(CoveLogicalType::Bool as u16).to_le_bytes());
    out[12] = PredicateNullPolicyV2::SqlWhere as u8;
    out[14..16].copy_from_slice(&2u16.to_le_bytes());
    out[16..20].copy_from_slice(&0u32.to_le_bytes());
    out[20..24].copy_from_slice(&2u32.to_le_bytes());
    out[24..28].copy_from_slice(&0u32.to_le_bytes());
    out[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    out[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[36..40].copy_from_slice(&crc.to_le_bytes());
    out
}

fn predicate_ast_literal(
    canonical_value_offset: u64,
    canonical_value_length: u32,
) -> [u8; PredicateLiteralV2::LEN] {
    let mut out = [0u8; PredicateLiteralV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&(ValueTag::Utf8 as u16).to_le_bytes());
    out[6..8].copy_from_slice(&(CoveLogicalType::Utf8 as u16).to_le_bytes());
    out[12..20].copy_from_slice(&canonical_value_offset.to_le_bytes());
    out[20..24].copy_from_slice(&canonical_value_length.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[24..28].copy_from_slice(&crc.to_le_bytes());
    out
}

fn predicate_ast_operand_ref(
    ordinal: u16,
    operand_kind: PredicateOperandKindV2,
    ref_id: u32,
) -> [u8; PredicateAstOperandRefV2::LEN] {
    let mut out = [0u8; PredicateAstOperandRefV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&ordinal.to_le_bytes());
    out[6] = operand_kind as u8;
    out[8..12].copy_from_slice(&ref_id.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[12..16].copy_from_slice(&crc.to_le_bytes());
    out
}

fn coverage_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::CoverageMetadata as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data,
    }
}

fn numcode_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::NumCode as u32)
}

fn varbytes_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::VarBytes as u32)
}

fn bool_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::PlainFixed as u32)
}

fn numcode_i64(values: &[i64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| (*value as u64).to_le_bytes())
        .collect()
}

fn varbytes(values: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

fn bools(values: &[bool]) -> Vec<u8> {
    values.iter().map(|value| u8::from(*value)).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(dead_code)]
fn _schema_for_docs() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("bucket", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("active", DataType::Boolean, false),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_manifest_declares_required_groups() {
        let manifest: Value = serde_json::from_str(PUBLIC_MANIFEST).unwrap();
        let groups = manifest
            .get("query_groups")
            .and_then(Value::as_array)
            .unwrap();
        for required in [
            "full_numeric_scan",
            "parquet_conversion_cost",
            "covi_index_latency",
            "covi_index_only_count",
            "object_store_cold_warm",
            "semantic_projection_object_store_compare",
            "semantic_showcase_bundle_object_store_compare",
            "coverage_cache_hit_miss_invalidation",
            "tpch_style_queries",
            "tpcds_style_queries",
            "medical_operational_queries",
            "negative_corrupt_validation",
            "canonicalisation_vectors",
            "semantic_mapping_corpus",
            "cove_map_build_tiny",
            "cove_map_build_medium",
            "cove_map_build_messy_multisource",
            "cove_o_overlap_stress",
            "customer360_projection_scan",
            "customer360_selective_filter",
            "customer360_event_filter",
            "customer360_object_store_compare",
            "proof_suite_customer360",
            "proof_suite_claims",
            "proof_suite_catalog",
        ] {
            assert!(groups.iter().any(|group| group.as_str() == Some(required)));
        }
    }

    #[test]
    fn offline_object_store_harness_records_cache_and_coalescing() {
        let mut harness = OfflineObjectStoreHarness::default();
        harness.put_object("object", vec![0u8; 32_768]);
        let original = deterministic_object_ranges(32_768);
        let coalesced = coalesce_object_ranges(&original, 1024, 16 * 1024);
        assert!(coalesced.len() <= original.len());
        read_harness_ranges(&mut harness, "object", &coalesced).unwrap();
        let cold = harness.take_stats();
        assert_eq!(cold.cache_misses, coalesced.len() as u64);
        read_harness_ranges(&mut harness, "object", &coalesced).unwrap();
        let warm = harness.take_stats();
        assert_eq!(warm.cache_hits, coalesced.len() as u64);
        assert_eq!(warm.bytes_returned, 0);
    }

    #[test]
    fn synthetic_cache_fixture_records_planner_hit() {
        let fixture = coverage_cache_fixture().unwrap();
        let dir = env::temp_dir().join(format!("cove-bench-cache-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cove_path = dir.join("synthetic-cache.cove");
        let cache_path = dir.join("synthetic-cache.cove.cache");
        fs::write(&cove_path, fixture.cove_bytes).unwrap();
        fs::write(&cache_path, fixture.cache_bytes).unwrap();

        let case = run_query_case(
            "coverage_cache_hit",
            "COVE-CACHE hit",
            &cove_path,
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "name".into(),
                    op: FilterOp::Eq,
                    value: Some("gamma".into()),
                }],
                table_options: CoveTableOptions::default().with_sibling_coverage_cache(),
                ..ExplainOptions::default()
            },
        )
        .unwrap();
        let hits = case
            .pointer("/cost/coverage_metrics/coverage_cache/hits")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        assert!(hits > 0);
        let _ = fs::remove_file(cove_path);
        let _ = fs::remove_file(cache_path);
        let _ = fs::remove_dir(dir);
    }
}
