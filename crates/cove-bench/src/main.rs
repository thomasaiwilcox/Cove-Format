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
use cove_ai_adapters::{
    import_jsonl, open as open_ai_archive, AiArchiveOpenOptions, AiExportFormat, AiExportOptions,
    AiImportOptions, AiImportSchema, AiSampleIteratorOptions, AiVerifyOptions,
};
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

mod ai;
mod cli;
mod corpus;
mod customer360;
mod delta;
mod fixtures;
mod object_store;
mod overlap;
mod projection_covi;
mod reports;
mod validation;

use ai::*;
use cli::*;
use corpus::*;
use customer360::*;
use delta::*;
use fixtures::*;
use object_store::*;
use overlap::*;
use projection_covi::*;
use reports::*;
use validation::*;

const PUBLIC_MANIFEST: &str = include_str!("../benchmarks/public-corpus.json");

#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchError(String);

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BenchError {}

impl From<String> for BenchError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for BenchError {
    fn from(message: &str) -> Self {
        Self(message.into())
    }
}

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cove-bench: {message}");
            ExitCode::FAILURE
        }
    }
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
