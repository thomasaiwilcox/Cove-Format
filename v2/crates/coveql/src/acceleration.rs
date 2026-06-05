use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use cove_core::{
    compression,
    constants::{PrimaryProfile, SectionKind, FEATURE_ENGINE_PROFILE},
    digest::compute_digest,
    mount::{mount_cove_file, MountOptions},
    profile::cove_e::{
        CodeSpaceDescriptorV1, EngineMountPolicyV1, EngineProfileEntryV1, EngineProfileRegistry,
        ExecutionCodeCanonicality, ExecutionCodeComparisonScope, ExecutionCodeDescriptorV1,
        ExecutionCodeKind, ExecutionCodeLifetime, FileCodeMappingKind, MissingValuePolicy,
        NullCodePolicy, ReverseLookupPolicy, StaleMappingPolicy,
    },
    reader::{validate_bytes, ValidatedCoveFile},
    segment::TableSegmentIndex,
    utility::{build_covx_artifact, hex_encode},
    writer::{MinimalCoveWriter, SectionPayload},
    CoveError,
};
use cove_index::build::{build_covi_from_cove_bytes, CoviBuildOptions};
use cove_layout::{build_default_layout_plan, build_default_scan_split_index};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    ArtifactExecutionEngine, ExecuteArtifactOptions, KernelExecutionMode, KernelExecutionOptions,
    PhysicalPlanOptions, PhysicalSidecarInputs,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccelerationBundleOptions {
    pub auto_discover: bool,
    pub strict_source_digest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveAccelerationBundle {
    pub source_path: PathBuf,
    pub manifest_path: Option<PathBuf>,
    pub source_digest: String,
    pub sidecars: BTreeMap<String, CoveAccelerationSidecar>,
    pub diagnostics: Vec<CoveAccelerationDiagnostic>,
}

impl CoveAccelerationBundle {
    pub fn has_usable_sidecars(&self) -> bool {
        self.sidecars
            .values()
            .any(|sidecar| matches!(sidecar.status, CoveAccelerationSidecarStatus::Present))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveAccelerationSidecar {
    pub kind: String,
    pub path: PathBuf,
    pub status: CoveAccelerationSidecarStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveAccelerationSidecarStatus {
    Present,
    Missing,
    Stale,
    NotApplicable,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveAccelerationDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveOptimizationOptions {
    pub source_path: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub full: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveOptimizationPlan {
    pub source_path: Option<PathBuf>,
    pub out_dir: PathBuf,
    pub source_digest: String,
    pub source_file_id: Option<String>,
    pub steps: Vec<CoveOptimizationStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveOptimizationStep {
    pub kind: String,
    pub path: PathBuf,
    pub action: CoveOptimizationAction,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveOptimizationAction {
    Generate,
    SkipNotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveOptimizeReport {
    pub source_path: Option<PathBuf>,
    pub manifest_path: PathBuf,
    pub source_digest: String,
    pub source_file_id: Option<String>,
    pub generated: Vec<CoveGeneratedSidecar>,
    pub skipped: Vec<CoveSkippedSidecar>,
    pub diagnostics: Vec<CoveAccelerationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveGeneratedSidecar {
    pub kind: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub validation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveSkippedSidecar {
    pub kind: String,
    pub path: PathBuf,
    pub reason: String,
}

pub fn discover_acceleration_bundle(
    bytes: &[u8],
    path: &Path,
    options: AccelerationBundleOptions,
) -> CoveAccelerationBundle {
    let source_digest = digest_hex(bytes);
    let manifest_path = perf_manifest_path(path);
    let mut diagnostics = Vec::new();
    let mut sidecars = BTreeMap::new();

    if options.auto_discover && manifest_path.exists() {
        match fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|text| serde_json::from_str::<CoveOptimizeReport>(&text).ok())
        {
            Some(report) => {
                if options.strict_source_digest && report.source_digest != source_digest {
                    diagnostics.push(CoveAccelerationDiagnostic {
                        code: "W_ACCELERATION_MANIFEST_STALE".into(),
                        message: format!(
                            "{} was built for a different source digest",
                            manifest_path.display()
                        ),
                    });
                } else {
                    for generated in report.generated {
                        record_path_sidecar(
                            &mut sidecars,
                            &generated.kind,
                            generated.path,
                            "discovered through covperf manifest",
                        );
                    }
                    for skipped in report.skipped {
                        sidecars.insert(
                            skipped.kind.clone(),
                            CoveAccelerationSidecar {
                                kind: skipped.kind,
                                path: skipped.path,
                                status: CoveAccelerationSidecarStatus::NotApplicable,
                                message: skipped.reason,
                            },
                        );
                    }
                }
            }
            None => diagnostics.push(CoveAccelerationDiagnostic {
                code: "W_ACCELERATION_MANIFEST_UNREADABLE".into(),
                message: format!("could not read {}", manifest_path.display()),
            }),
        }
    }

    for (kind, candidate) in conventional_sidecar_paths(path) {
        sidecars.entry(kind.clone()).or_insert_with(|| {
            let status = if candidate.exists() {
                CoveAccelerationSidecarStatus::Present
            } else {
                CoveAccelerationSidecarStatus::Missing
            };
            CoveAccelerationSidecar {
                kind,
                path: candidate,
                status,
                message: "conventional sibling sidecar path".into(),
            }
        });
    }

    CoveAccelerationBundle {
        source_path: path.to_path_buf(),
        manifest_path: manifest_path.exists().then_some(manifest_path),
        source_digest,
        sidecars,
        diagnostics,
    }
}

pub fn plan_acceleration(bytes: &[u8], options: CoveOptimizationOptions) -> CoveOptimizationPlan {
    let source_digest = digest_hex(bytes);
    let out_dir = options
        .out_dir
        .clone()
        .or_else(|| {
            options
                .source_path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| PathBuf::from("."));
    let source_file_id = validate_bytes(bytes)
        .ok()
        .map(|validated| hex_encode(&validated.header.file_id));
    let source_path = options.source_path.clone();
    let mut steps = Vec::new();
    let table_shape = table_shape(bytes).ok();
    let base = source_path
        .as_deref()
        .map(base_name)
        .unwrap_or_else(|| "cove".into());
    let paths = generated_sidecar_paths(&out_dir, &base);

    let has_table = table_shape
        .as_ref()
        .is_some_and(|shape| !shape.tables.is_empty());
    let has_file_code = table_shape
        .as_ref()
        .is_some_and(|shape| shape.has_file_code)
        || has_file_dictionary(bytes);
    let has_segments = table_shape
        .as_ref()
        .is_some_and(|shape| !shape.segments.entries.is_empty());

    steps.push(step(
        "COVE-I",
        &paths.covi,
        has_table,
        "table columns can be indexed for lookup and index-only aggregate candidates",
        "no COVE-T table catalog was found",
    ));
    steps.push(step(
        "COVX",
        &paths.covx,
        source_path.is_some(),
        "source identity can be summarized for archive/index discovery",
        "source path is required to build a COVX referenced-file artifact",
    ));
    steps.push(step(
        "COVE-E",
        &paths.cove_e,
        has_file_code,
        "FileCode columns can advertise execution-code remap contracts",
        "no FileCode columns were found",
    ));
    steps.push(step(
        "COVE-L scan splits",
        &paths.scan_split_index,
        has_table && has_segments,
        "table segment metadata can provide deterministic scan splits",
        "no table segments were found",
    ));
    steps.push(step(
        "COVE-L layout plan",
        &paths.layout_plan,
        has_table && has_segments,
        "table segment metadata can provide a default layout plan",
        "no table segments were found",
    ));
    for (kind, path, reason) in [
        (
            "COVE-COVERAGE plan",
            &paths.coverage_plan,
            "coverage proof generation needs query-normal-form templates for this source",
        ),
        (
            "COVE-COVERAGE proof",
            &paths.coverage_proof,
            "coverage proof generation needs exact source/provider authority",
        ),
        (
            "COVE-COVERAGE set",
            &paths.coverage_set,
            "coverage set generation needs exact source/provider authority",
        ),
        (
            "COVE-CACHE",
            &paths.coverage_cache,
            "coverage cache entries are runtime observations and are generated after query execution",
        ),
        (
            "COVE-L page clusters",
            &paths.page_cluster_directory,
            "page cluster generation needs page-cluster authority not present in this source",
        ),
        (
            "COVE-L zero-copy map",
            &paths.zero_copy_buffer_map,
            "zero-copy maps need retained-buffer/page-lifetime authority",
        ),
        (
            "CoveQL graph index",
            &paths.graph_index,
            "graph acceleration needs declared graph traversal/algorithm contracts",
        ),
    ] {
        steps.push(CoveOptimizationStep {
            kind: kind.into(),
            path: path.clone(),
            action: CoveOptimizationAction::SkipNotApplicable,
            reason: reason.into(),
        });
    }

    CoveOptimizationPlan {
        source_path,
        out_dir,
        source_digest,
        source_file_id,
        steps,
    }
}

pub fn generate_acceleration_sidecars(
    bytes: &[u8],
    plan: CoveOptimizationPlan,
    out_dir: &Path,
) -> Result<CoveOptimizeReport, CoveError> {
    fs::create_dir_all(out_dir)?;
    let mut generated = Vec::new();
    let mut skipped = Vec::new();
    let mut diagnostics = Vec::new();
    let table_shape = table_shape(bytes).ok();

    for step in &plan.steps {
        match step.action {
            CoveOptimizationAction::SkipNotApplicable => skipped.push(CoveSkippedSidecar {
                kind: step.kind.clone(),
                path: step.path.clone(),
                reason: step.reason.clone(),
            }),
            CoveOptimizationAction::Generate => {
                let result = match step.kind.as_str() {
                    "COVE-I" => generate_covi(bytes, &step.path),
                    "COVX" => generate_covx(&plan, &step.path),
                    "COVE-E" => generate_cove_e(bytes, &step.path),
                    "COVE-L scan splits" => generate_scan_splits(&table_shape, &step.path),
                    "COVE-L layout plan" => generate_layout_plan(&table_shape, &step.path),
                    _ => Ok(None),
                };
                match result {
                    Ok(Some(bytes_written)) => generated.push(CoveGeneratedSidecar {
                        kind: step.kind.clone(),
                        path: step.path.clone(),
                        bytes: bytes_written,
                        validation: "generated and parsed".into(),
                    }),
                    Ok(None) => skipped.push(CoveSkippedSidecar {
                        kind: step.kind.clone(),
                        path: step.path.clone(),
                        reason: step.reason.clone(),
                    }),
                    Err(error) => {
                        diagnostics.push(CoveAccelerationDiagnostic {
                            code: "W_ACCELERATION_GENERATION_SKIPPED".into(),
                            message: format!("{}: {error}", step.kind),
                        });
                        skipped.push(CoveSkippedSidecar {
                            kind: step.kind.clone(),
                            path: step.path.clone(),
                            reason: error.to_string(),
                        });
                    }
                }
            }
        }
    }

    let manifest_path = out_dir.join(format!(
        "{}.covperf.json",
        plan.source_path
            .as_deref()
            .map(base_name)
            .unwrap_or_else(|| "cove".into())
    ));
    let report = CoveOptimizeReport {
        source_path: plan.source_path,
        manifest_path: manifest_path.clone(),
        source_digest: plan.source_digest,
        source_file_id: plan.source_file_id,
        generated,
        skipped,
        diagnostics,
    };
    let manifest = serde_json::to_vec_pretty(&report)
        .map_err(|error| CoveError::BadSection(error.to_string()))?;
    fs::write(&manifest_path, manifest)?;
    Ok(report)
}

pub fn apply_acceleration_bundle(
    bundle: &CoveAccelerationBundle,
    mut options: ExecuteArtifactOptions,
) -> ExecuteArtifactOptions {
    let mut physical_options = match &options.execution_engine {
        ArtifactExecutionEngine::Physical {
            physical_options, ..
        } => physical_options.clone(),
        ArtifactExecutionEngine::Materialized => PhysicalPlanOptions::default(),
    };
    apply_paths_to_inputs(&mut physical_options.sidecars, &bundle.sidecars);
    let kernel_options = match options.execution_engine {
        ArtifactExecutionEngine::Physical { kernel_options, .. } => kernel_options,
        ArtifactExecutionEngine::Materialized => KernelExecutionOptions {
            mode: KernelExecutionMode::Auto,
            ..KernelExecutionOptions::default()
        },
    };
    options.execution_engine = ArtifactExecutionEngine::Physical {
        physical_options,
        kernel_options,
    };
    options
}

pub fn acceleration_report_json(bundle: &CoveAccelerationBundle) -> Value {
    json!({
        "source": bundle.source_path,
        "source_digest": bundle.source_digest,
        "manifest": bundle.manifest_path,
        "sidecars": bundle.sidecars,
        "diagnostics": bundle.diagnostics,
    })
}

fn record_path_sidecar(
    sidecars: &mut BTreeMap<String, CoveAccelerationSidecar>,
    kind: &str,
    path: PathBuf,
    message: &str,
) {
    let status = if path.exists() {
        CoveAccelerationSidecarStatus::Present
    } else {
        CoveAccelerationSidecarStatus::Missing
    };
    sidecars.insert(
        kind.into(),
        CoveAccelerationSidecar {
            kind: kind.into(),
            path,
            status,
            message: message.into(),
        },
    );
}

fn apply_paths_to_inputs(
    inputs: &mut PhysicalSidecarInputs,
    sidecars: &BTreeMap<String, CoveAccelerationSidecar>,
) {
    for sidecar in sidecars.values() {
        if sidecar.status != CoveAccelerationSidecarStatus::Present {
            continue;
        }
        let bytes = fs::read(&sidecar.path).ok();
        match sidecar.kind.as_str() {
            "COVE-I" => inputs.covi_artifact_bytes = bytes,
            "COVX" => inputs.covx_artifact_bytes = bytes,
            "COVE-E" => inputs.cove_e_artifact_bytes = bytes,
            "COVE-COVERAGE plan" => inputs.coverage_plan_candidate_bytes = bytes,
            "COVE-COVERAGE proof" => inputs.coverage_proof_record_bytes = bytes,
            "COVE-COVERAGE set" => inputs.coverage_set_bytes = bytes,
            "COVE-L layout plan" => inputs.layout_plan_bytes = bytes,
            "COVE-L scan splits" => inputs.scan_split_index_bytes = bytes,
            "COVE-L page clusters" => inputs.page_cluster_directory_bytes = bytes,
            "COVE-L zero-copy map" => inputs.zero_copy_buffer_map_bytes = bytes,
            "COVE-CACHE" => inputs.coverage_cache_bytes = bytes,
            _ => {}
        }
    }
}

fn generate_covi(bytes: &[u8], path: &Path) -> Result<Option<u64>, CoveError> {
    let options = CoviBuildOptions {
        all_columns: true,
        include_index_only_counts: true,
        include_index_only_exists: true,
        include_index_only_min_max: true,
        include_index_only_distinct_count: true,
        include_index_only_sum_avg: true,
        ..CoviBuildOptions::default()
    };
    let out = build_covi_from_cove_bytes(bytes, &options)?;
    fs::write(path, &out)?;
    Ok(Some(out.len() as u64))
}

fn generate_covx(plan: &CoveOptimizationPlan, path: &Path) -> Result<Option<u64>, CoveError> {
    let Some(source_path) = plan.source_path.as_ref() else {
        return Ok(None);
    };
    let (out, _) = build_covx_artifact(path, std::slice::from_ref(source_path))?;
    fs::write(path, &out)?;
    Ok(Some(out.len() as u64))
}

fn generate_cove_e(bytes: &[u8], path: &Path) -> Result<Option<u64>, CoveError> {
    let source_digest = compute_digest(cove_core::constants::DigestAlgorithm::Sha256, bytes)?;
    let registry = EngineProfileRegistry {
        flags: 0,
        profiles: vec![EngineProfileEntryV1 {
            profile_id: 1,
            namespace: "org.coveformat.coveql".into(),
            profile_name: "coveql-dictionary-execution-code".into(),
            version_major: 1,
            version_minor: 0,
            required_features: 0,
            optional_features: 0,
            execution_descriptor_ref: 1,
            mount_policy_ref: 1,
            private_payload_ref: 0,
            checksum: 0,
        }],
    };
    let code_space = CodeSpaceDescriptorV1 {
        code_space_id: 1,
        namespace: "org.coveformat.coveql.execution".into(),
        stable_id: source_digest,
        epoch: 1,
        flags: 0,
        private_payload_ref: 0,
    };
    let descriptor = ExecutionCodeDescriptorV1 {
        descriptor_id: 1,
        code_kind: ExecutionCodeKind::DictionaryKey,
        code_width_bits: 64,
        byte_order: 0,
        lifetime: ExecutionCodeLifetime::Scan,
        comparison_scope: ExecutionCodeComparisonScope::File,
        canonicality: ExecutionCodeCanonicality::Transient,
        null_code_policy: NullCodePolicy::NullBitmapOnly,
        flags: 0,
        scope_ref: 0,
        code_space_ref: 1,
        checksum: 0,
    };
    let policy = EngineMountPolicyV1 {
        policy_id: 1,
        filecode_mapping_kind: FileCodeMappingKind::MapToExecutionCode,
        missing_value_policy: MissingValuePolicy::DecodeValueOnly,
        stale_mapping_policy: StaleMappingPolicy::IgnoreIfOptional,
        reverse_lookup_policy: ReverseLookupPolicy::BuildFromDictionary,
        flags: 0,
        dictionary_digest_ref: 0,
        code_space_ref: 1,
        cache_key_ref: 0,
        private_payload_ref: 0,
        checksum: 0,
    };
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::EngineExecution as u8;
    writer.required_features = FEATURE_ENGINE_PROFILE;
    writer.sections = vec![
        engine_section(SectionKind::EngineProfileRegistry, 1, registry.serialize()?),
        engine_section(
            SectionKind::ExecutionCodeDescriptor,
            1,
            descriptor.serialize().to_vec(),
        ),
        engine_section(SectionKind::CodeSpaceDescriptor, 1, code_space.serialize()?),
        engine_section(
            SectionKind::EngineMountPolicy,
            1,
            policy.serialize().to_vec(),
        ),
    ];
    let out = writer.write()?;
    let _ = validate_bytes(&out)?;
    fs::write(path, &out)?;
    Ok(Some(out.len() as u64))
}

fn engine_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data,
    }
}

fn generate_scan_splits(shape: &Option<TableShape>, path: &Path) -> Result<Option<u64>, CoveError> {
    let Some(shape) = shape else {
        return Ok(None);
    };
    let Some(table) = shape.tables.first() else {
        return Ok(None);
    };
    let splits = build_default_scan_split_index(table, &shape.segments.entries)?;
    let out = splits.serialize()?;
    fs::write(path, &out)?;
    Ok(Some(out.len() as u64))
}

fn generate_layout_plan(shape: &Option<TableShape>, path: &Path) -> Result<Option<u64>, CoveError> {
    let Some(shape) = shape else {
        return Ok(None);
    };
    let Some(table) = shape.tables.first() else {
        return Ok(None);
    };
    let splits = build_default_scan_split_index(table, &shape.segments.entries)?;
    let plan = build_default_layout_plan(table, &shape.segments.entries, Some(&splits))?;
    let out = plan.serialize()?;
    fs::write(path, &out)?;
    Ok(Some(out.len() as u64))
}

fn step(
    kind: &str,
    path: &Path,
    can_generate: bool,
    generate_reason: &str,
    skip_reason: &str,
) -> CoveOptimizationStep {
    CoveOptimizationStep {
        kind: kind.into(),
        path: path.to_path_buf(),
        action: if can_generate {
            CoveOptimizationAction::Generate
        } else {
            CoveOptimizationAction::SkipNotApplicable
        },
        reason: if can_generate {
            generate_reason.into()
        } else {
            skip_reason.into()
        },
    }
}

#[derive(Debug, Clone)]
struct TableShape {
    tables: Vec<cove_core::table::TableEntry>,
    segments: TableSegmentIndex,
    has_file_code: bool,
}

fn table_shape(bytes: &[u8]) -> Result<TableShape, CoveError> {
    let validated = validate_bytes(bytes)?;
    let mounted = mount_cove_file(bytes, MountOptions::default(), None)?;
    let tables = mounted
        .table_catalog
        .as_ref()
        .map(|catalog| catalog.tables.clone())
        .unwrap_or_default();
    let segments = parse_table_segment_index(bytes, &validated)?;
    let has_file_code = tables.iter().any(|table| {
        table
            .columns
            .iter()
            .any(|column| column.physical == cove_core::constants::CovePhysicalKind::FileCode)
    });
    Ok(TableShape {
        tables,
        segments,
        has_file_code,
    })
}

fn has_file_dictionary(bytes: &[u8]) -> bool {
    mount_cove_file(bytes, MountOptions::default(), None)
        .ok()
        .and_then(|mounted| mounted.dictionary)
        .is_some()
}

fn parse_table_segment_index(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<TableSegmentIndex, CoveError> {
    let Some(entry) = validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::TableSegmentIndex as u16)
    else {
        return Ok(TableSegmentIndex {
            flags: 0,
            entries: Vec::new(),
        });
    };
    let payload = compression::section_payload(bytes, entry)?;
    TableSegmentIndex::parse(payload.as_ref())
}

#[derive(Debug, Clone)]
struct GeneratedPaths {
    covi: PathBuf,
    covx: PathBuf,
    cove_e: PathBuf,
    coverage_plan: PathBuf,
    coverage_proof: PathBuf,
    coverage_set: PathBuf,
    layout_plan: PathBuf,
    scan_split_index: PathBuf,
    page_cluster_directory: PathBuf,
    zero_copy_buffer_map: PathBuf,
    coverage_cache: PathBuf,
    graph_index: PathBuf,
}

fn generated_sidecar_paths(out_dir: &Path, base: &str) -> GeneratedPaths {
    GeneratedPaths {
        covi: out_dir.join(format!("{base}.covi")),
        covx: out_dir.join(format!("{base}.covx")),
        cove_e: out_dir.join(format!("{base}.covee")),
        coverage_plan: out_dir.join(format!("{base}.coverage-plan.bin")),
        coverage_proof: out_dir.join(format!("{base}.coverage-proof.bin")),
        coverage_set: out_dir.join(format!("{base}.coverage-set.bin")),
        layout_plan: out_dir.join(format!("{base}.layout.bin")),
        scan_split_index: out_dir.join(format!("{base}.splits.bin")),
        page_cluster_directory: out_dir.join(format!("{base}.clusters.bin")),
        zero_copy_buffer_map: out_dir.join(format!("{base}.zerocopy.bin")),
        coverage_cache: out_dir.join(format!("{base}.covcache")),
        graph_index: out_dir.join(format!("{base}.graph-index.json")),
    }
}

fn conventional_sidecar_paths(path: &Path) -> Vec<(String, PathBuf)> {
    let out_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let paths = generated_sidecar_paths(out_dir, &base_name(path));
    vec![
        ("COVE-I".into(), paths.covi),
        ("COVX".into(), paths.covx),
        ("COVE-E".into(), paths.cove_e),
        ("COVE-COVERAGE plan".into(), paths.coverage_plan),
        ("COVE-COVERAGE proof".into(), paths.coverage_proof),
        ("COVE-COVERAGE set".into(), paths.coverage_set),
        ("COVE-L layout plan".into(), paths.layout_plan),
        ("COVE-L scan splits".into(), paths.scan_split_index),
        ("COVE-L page clusters".into(), paths.page_cluster_directory),
        ("COVE-L zero-copy map".into(), paths.zero_copy_buffer_map),
        ("COVE-CACHE".into(), paths.coverage_cache),
        ("CoveQL graph index".into(), paths.graph_index),
    ]
}

fn perf_manifest_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.covperf.json", base_name(path)))
}

fn base_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("cove")
        .to_string()
}

fn digest_hex(bytes: &[u8]) -> String {
    hex_encode(
        &compute_digest(cove_core::constants::DigestAlgorithm::Sha256, bytes).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cove_e_sidecar_generation_writes_valid_engine_metadata() {
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("test")
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = std::env::temp_dir().join(format!(
            "coveql-cove-e-{}-{}.covee",
            std::process::id(),
            thread_name
        ));
        let bytes = b"source bytes for execution-code sidecar";
        let written = generate_cove_e(bytes, &path).unwrap().unwrap();
        assert!(written > 0);
        let sidecar = fs::read(&path).unwrap();
        let mounted = mount_cove_file(&sidecar, MountOptions::default(), None).unwrap();
        assert!(!mounted.execution_descriptors.is_empty());
        assert!(!mounted.code_spaces.is_empty());
        assert!(!mounted.engine_mount_policies.is_empty());
        let _ = fs::remove_file(path);
    }
}
