use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use arrow_array::{ArrayRef, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use cove::{explain_policy_for_mode, ExplainMode, PreparedQueryTextOptions, QueryTextError};
use cove_ai_adapters::{
    build_ai_training_showcase, diff_archives, import_hf_dir, import_jsonl, import_parquet,
    open as open_ai_archive, stream_archive, write_export_file, AiArchiveOpenOptions,
    AiExportFormat, AiExportOptions, AiImportMapping, AiImportOptions, AiImportSchema,
    AiSplitPolicy, AiVerifyOptions,
};
use cove_core::{
    artifact::{
        coveai::{
            ai_explain_report, write_covev_filecode_vectors_with_options, AiPayloadReader,
            CoveAiAccessContext, CoveAiArtifactKind, CoveAiFile, CoveVecFileCodeVectorBuild,
            CoveVecFileCodeVectorBuildOptions,
        },
        covemap::CovemapFile,
        covm::{CovmAiSidecarExtensionV1, CovmDeltaPruneRequest, CovmFile},
    },
    checksum, compression,
    constants::{SectionKind, MAGIC_COVEAI, MAGIC_COVEMAP, MAGIC_COVEV},
    feature_binding::OperationKindV2,
    profile::{
        cove_map::{parse_embedded_section, EmbeddedMapSection},
        cove_o::CoveObjectSurface,
    },
    reader::{validate_bytes_with_options, ValidatedCoveFile, ValidationOptions},
    table::TableCatalog,
    writer::ScanProfileCoveWriter,
};
use coveql::{
    acceleration_report_json, apply_acceleration_bundle, discover_acceleration_bundle,
    discover_query_surfaces, execute_query_from_artifact, generate_acceleration_sidecars,
    parse_resolve_plan_and_execute_query_on_object_surface, plan_acceleration, suggest_queries,
    AccelerationBundleOptions, ArtifactExecutionEngine, CoveAccelerationBundle,
    CoveOptimizationOptions, CoveQlExecutionResult, CoveQlOutputMode, ExecuteArtifactOptions,
    ExecuteArtifactQueryError, ExecutedQuery, ExplainDisclosurePolicy, GraphTraversalContract,
    GraphTraversalDistinctPolicy, GraphTraversalMode, KernelExecutionMode, KernelExecutionOptions,
    PhysicalPlanOptions, PhysicalSidecarInputs, QueryArtifactMember, QuerySurfaceDiscovery,
    QuerySurfaceDiscoveryOptions, COVEQL_PROFILE_CONTRACT_VERSION,
};

use crate::{
    arrow_export,
    customer360::{
        generate_customer360, generate_proof_suite, Customer360Options, Customer360Profile,
        ProofSuiteOptions, ProofSuiteScenario,
    },
    delta,
    delta::run_delta,
    external_tables::{register_external_tables, ExternalTableSpec},
    help::{print_usage, usage, HelpTopic},
    output::{write_result, OutputFormat},
    perf,
    sidecar::run_sidecar,
    CliError,
};

include!("../args.rs");
include!("../command.rs");
include!("../util.rs");
include!("artifacts.rs");
include!("ai.rs");
include!("train.rs");
include!("map.rs");
include!("digest.rs");
include!("showcase.rs");
include!("inspect.rs");
include!("query.rs");
