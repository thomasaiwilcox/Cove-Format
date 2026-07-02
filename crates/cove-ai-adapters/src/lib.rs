//! Stable facade for COVE-AI archive import, verification, reporting, and export.
//!
//! The public API returns typed `AiAdapterError` values; their `Display`
//! implementation is kept compatible with the CLI-facing diagnostics.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    error::Error,
    fmt, fs,
    io::Cursor,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use cove_core::{
    artifact::{
        coveai::{
            write_coveai_descriptor_bundle, AiDescriptorTablesV1, AiDigestEntryV1,
            AiPayloadAccessState, AiPayloadEncodingV1, AiPayloadReader, AiPayloadRefEntryV1,
            AiPolicyRefEntryV1, AiPrivacySummaryEntryV1, AiRequirednessScopeV1, AiStorageKindV1,
            AiStringEntryV1, CoveAiAccessContext, CoveAiArtifactKind, CoveAiDescriptorBundleBuild,
            CoveAiFile, CoveAiWritableSection, DatasetSplitV1, DedupGroupV1,
            GenerationDecodingProfileV1, GeneratorProvenanceV1, HumanReviewEntryV1,
            ModelActorDescriptorV1, PreferencePairEntryV1, TrainingEpochPlanV1,
            TrainingLabelEntryV1, TrainingProfileV1, TrainingSampleEntryV1,
            AI_POLICY_KIND_DISCLOSURE, AI_POLICY_KIND_LICENSE,
        },
        covm::{
            CovmAiSidecarExtensionV1, CovmAiSidecarRefV1, CovmFile, CovmFileEntryV1, CovmHeaderV1,
        },
    },
    checksum,
    constants::{DigestAlgorithm, PrimaryProfile, SectionKind},
    digest::compute_digest,
    durable::durable_replace,
    CoveError,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const PAYLOAD_SECTION_ID: u32 = 10;
const DEFAULT_TRAIN_PPM: u64 = 980_000;
const DEFAULT_VALIDATION_PPM: u64 = 10_000;

#[derive(Debug)]
#[non_exhaustive]
pub enum AiAdapterError {
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidSidecar {
        path: PathBuf,
        source: CoveError,
    },
    UnsupportedImportSchema {
        schema: String,
    },
    UnsupportedSplitPolicy {
        policy: String,
    },
    UnsupportedExportFormat {
        format: String,
    },
    Export {
        message: String,
    },
    InvalidInput {
        message: String,
    },
}

impl fmt::Display for AiAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AiAdapterError::Io {
                action,
                path,
                source,
            } => write!(f, "cannot {action} {}: {source}", path.display()),
            AiAdapterError::InvalidSidecar { path, source } => {
                write!(
                    f,
                    "{} is not a valid COVE-AI sidecar: {source}",
                    path.display()
                )
            }
            AiAdapterError::UnsupportedImportSchema { schema } => write!(
                f,
                "unsupported AI import schema '{schema}'; expected instruction, chat, pretrain, preference, or rag"
            ),
            AiAdapterError::UnsupportedSplitPolicy { policy } => {
                write!(
                    f,
                    "unsupported split policy '{policy}'; only deterministic is implemented"
                )
            }
            AiAdapterError::UnsupportedExportFormat { format } => {
                write!(
                    f,
                    "unsupported AI export format '{format}'; expected json, jsonl, hf-jsonl, arrow, parquet, or webdataset"
                )
            }
            AiAdapterError::Export { message } | AiAdapterError::InvalidInput { message } => {
                f.write_str(message)
            }
        }
    }
}

impl Error for AiAdapterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            AiAdapterError::Io { source, .. } => Some(source),
            AiAdapterError::InvalidSidecar { source, .. } => Some(source),
            AiAdapterError::UnsupportedImportSchema { .. }
            | AiAdapterError::UnsupportedSplitPolicy { .. }
            | AiAdapterError::UnsupportedExportFormat { .. }
            | AiAdapterError::Export { .. }
            | AiAdapterError::InvalidInput { .. } => None,
        }
    }
}

impl From<String> for AiAdapterError {
    fn from(message: String) -> Self {
        AiAdapterError::InvalidInput { message }
    }
}

impl From<&str> for AiAdapterError {
    fn from(message: &str) -> Self {
        AiAdapterError::InvalidInput {
            message: message.to_string(),
        }
    }
}

#[derive(Debug)]
enum AiImportBuildError {
    InvalidInput(String),
    Descriptor(String),
    Digest(String),
    Manifest(String),
    OutputWrite { path: PathBuf, message: String },
}

impl fmt::Display for AiImportBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message)
            | Self::Descriptor(message)
            | Self::Digest(message)
            | Self::Manifest(message) => f.write_str(message),
            Self::OutputWrite { path, message } => {
                write!(f, "cannot write {}: {message}", path.display())
            }
        }
    }
}

impl Error for AiImportBuildError {}

impl From<AiImportBuildError> for AiAdapterError {
    fn from(error: AiImportBuildError) -> Self {
        AiAdapterError::InvalidInput {
            message: error.to_string(),
        }
    }
}

#[must_use]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiArchiveOpenOptions {
    pub cove_ai: Option<PathBuf>,
    pub dataset_dir: Option<PathBuf>,
}

#[must_use]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiVerifyOptions {
    pub policy_report: bool,
    pub strict_training: bool,
}

impl Default for AiVerifyOptions {
    fn default() -> Self {
        Self {
            policy_report: true,
            strict_training: false,
        }
    }
}

#[must_use]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiSampleIteratorOptions {
    pub split: Option<String>,
    pub include_payloads: bool,
}

/// COVE-AI export format names accepted at CLI, Python, report, and file-output boundaries.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiExportFormat {
    /// Pretty JSON report export.
    #[serde(rename = "json")]
    Json,
    /// JSON Lines sample export.
    #[serde(rename = "jsonl")]
    Jsonl,
    /// Hugging Face-compatible JSON Lines sample export.
    #[serde(rename = "hf-jsonl")]
    HfJsonl,
    /// Apache Arrow IPC file export.
    #[serde(rename = "arrow")]
    Arrow,
    /// Apache Parquet file export.
    #[serde(rename = "parquet")]
    Parquet,
    /// WebDataset tar export.
    #[serde(rename = "webdataset")]
    WebDataset,
}

impl AiExportFormat {
    /// Parse a COVE-AI export format from its spec-facing string value.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError::UnsupportedExportFormat`] when `value` is not
    /// one of `json`, `jsonl`, `hf-jsonl`, `arrow`, `parquet`, or `webdataset`.
    pub fn parse(value: &str) -> Result<Self, AiAdapterError> {
        match value {
            "json" => Ok(Self::Json),
            "jsonl" => Ok(Self::Jsonl),
            "hf-jsonl" => Ok(Self::HfJsonl),
            "arrow" => Ok(Self::Arrow),
            "parquet" => Ok(Self::Parquet),
            "webdataset" => Ok(Self::WebDataset),
            other => Err(AiAdapterError::UnsupportedExportFormat {
                format: other.to_string(),
            }),
        }
    }

    /// Return the spec-facing string value for this export format.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Jsonl => "jsonl",
            Self::HfJsonl => "hf-jsonl",
            Self::Arrow => "arrow",
            Self::Parquet => "parquet",
            Self::WebDataset => "webdataset",
        }
    }
}

impl FromStr for AiExportFormat {
    type Err = AiAdapterError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[must_use]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiExportOptions {
    pub format: AiExportFormat,
    pub out: Option<PathBuf>,
    pub split: Option<String>,
    pub include_payloads: bool,
    pub policy_report: bool,
}

impl Default for AiExportOptions {
    fn default() -> Self {
        Self {
            format: AiExportFormat::Jsonl,
            out: None,
            split: None,
            include_payloads: false,
            policy_report: true,
        }
    }
}

#[must_use]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiImportOptions {
    pub schema: AiImportSchema,
    pub split_policy: AiSplitPolicy,
    pub split_column: Option<String>,
    pub mapping: Option<AiImportMapping>,
    pub dry_run: bool,
    pub publish_covm: bool,
    pub artifact_id: Option<[u8; 16]>,
    pub created_at_us: Option<i64>,
}

impl Default for AiImportOptions {
    fn default() -> Self {
        Self {
            schema: AiImportSchema::Instruction,
            split_policy: AiSplitPolicy::Deterministic,
            split_column: None,
            mapping: None,
            dry_run: false,
            publish_covm: false,
            artifact_id: None,
            created_at_us: None,
        }
    }
}

#[must_use]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiImportMapping {
    pub split_field: Option<String>,
    pub quality_score_ppm_field: Option<String>,
    pub sample_weight_ppm_field: Option<String>,
    pub dedup_key_field: Option<String>,
    pub policy_field: Option<String>,
    pub license_field: Option<String>,
    pub source_ref_field: Option<String>,
    pub evidence_field: Option<String>,
    pub labels_field: Option<String>,
    pub generator_field: Option<String>,
    pub human_review_field: Option<String>,
    pub epoch_plan: Option<AiImportEpochPlanMapping>,
}

#[must_use]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiImportEpochPlanMapping {
    pub enabled: bool,
    pub seed: Option<u64>,
    pub split: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiImportSchema {
    Instruction,
    Chat,
    Pretrain,
    Preference,
    Rag,
}

impl AiImportSchema {
    /// Parse an AI import schema from its spec-facing string value.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError::UnsupportedImportSchema`] when `value` is not
    /// one of `instruction`, `chat`, `pretrain`, `preference`, or `rag`.
    pub fn parse(value: &str) -> Result<Self, AiAdapterError> {
        match value {
            "instruction" => Ok(Self::Instruction),
            "chat" => Ok(Self::Chat),
            "pretrain" => Ok(Self::Pretrain),
            "preference" => Ok(Self::Preference),
            "rag" => Ok(Self::Rag),
            other => Err(AiAdapterError::UnsupportedImportSchema {
                schema: other.to_string(),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instruction => "instruction",
            Self::Chat => "chat",
            Self::Pretrain => "pretrain",
            Self::Preference => "preference",
            Self::Rag => "rag",
        }
    }
}

impl FromStr for AiImportSchema {
    type Err = AiAdapterError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiSplitPolicy {
    Deterministic,
}

impl AiSplitPolicy {
    /// Parse an AI split policy from its spec-facing string value.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError::UnsupportedSplitPolicy`] when `value` is not
    /// `deterministic`.
    pub fn parse(value: &str) -> Result<Self, AiAdapterError> {
        match value {
            "deterministic" => Ok(Self::Deterministic),
            other => Err(AiAdapterError::UnsupportedSplitPolicy {
                policy: other.to_string(),
            }),
        }
    }
}

impl FromStr for AiSplitPolicy {
    type Err = AiAdapterError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[must_use]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiArchiveReport {
    pub path: PathBuf,
    pub artifact_id: String,
    pub artifact_kind: String,
    pub payload_access: String,
    pub training_sample_count: usize,
    pub training_label_count: usize,
    pub preference_pair_count: usize,
    pub dedup_group_count: usize,
    pub generator_provenance_count: usize,
    pub epoch_plan_count: usize,
    pub split_counts: BTreeMap<String, usize>,
    pub replayability: String,
    pub contamination_risk_count: usize,
    pub withheld_count: usize,
    pub diagnostics: Vec<AiWithheldDiagnostic>,
}

#[must_use]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiWithheldDiagnostic {
    pub code: String,
    pub sample_id: Option<String>,
    pub message: String,
}

#[must_use]
#[derive(Debug, Clone)]
pub struct AiTrainingArchive {
    path: PathBuf,
    bytes: Vec<u8>,
    sidecar: CoveAiFile,
}

#[must_use]
#[derive(Debug, Clone)]
pub struct AiExportData {
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
    pub report: Value,
}

#[derive(Debug, Clone)]
struct ImportedSample {
    sample_id_text: String,
    sample_id: u64,
    schema: AiImportSchema,
    split: SplitName,
    quality_score_ppm: u32,
    sample_weight_ppm: u32,
    dedup_key: Option<String>,
    policy: Option<Value>,
    license: Option<Value>,
    source_ref: Option<Value>,
    evidence: Option<Value>,
    labels: Vec<Value>,
    generator: Option<Value>,
    human_review: Option<Value>,
    input: Vec<u8>,
    target: Vec<u8>,
    metadata: Vec<u8>,
    diagnostics: Vec<AiWithheldDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SplitName {
    Train,
    Validation,
    Test,
}

impl SplitName {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "train" => Some(Self::Train),
            "validation" | "valid" | "val" => Some(Self::Validation),
            "test" => Some(Self::Test),
            _ => None,
        }
    }

    fn id(self) -> u32 {
        match self {
            Self::Train => 1,
            Self::Validation => 2,
            Self::Test => 3,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Validation => "validation",
            Self::Test => "test",
        }
    }
}

impl AiTrainingArchive {
    /// Open a COVE-AI training archive from a sidecar or manifest path.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError`] if the sidecar path cannot be resolved, the
    /// sidecar cannot be read, or the bytes do not parse as a valid COVE-AI
    /// archive.
    pub fn open(
        path: impl AsRef<Path>,
        options: AiArchiveOpenOptions,
    ) -> Result<Self, AiAdapterError> {
        let requested = path.as_ref();
        let sidecar_path = resolve_sidecar_path(requested, &options)?;
        let bytes = fs::read(&sidecar_path).map_err(|source| AiAdapterError::Io {
            action: "read",
            path: sidecar_path.clone(),
            source,
        })?;
        let sidecar =
            CoveAiFile::parse(&bytes).map_err(|source| AiAdapterError::InvalidSidecar {
                path: sidecar_path.clone(),
                source,
            })?;
        Ok(Self {
            path: sidecar_path,
            bytes,
            sidecar,
        })
    }

    /// Verify the archive and return a JSON verification report.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError`] if archive reporting fails or the typed report
    /// cannot be serialized to JSON.
    pub fn verify(&self, options: AiVerifyOptions) -> Result<Value, AiAdapterError> {
        let report = self.report(options)?;
        Ok(serde_json::to_value(report).map_err(|error| error.to_string())?)
    }

    /// Build a typed verification report for the archive.
    ///
    /// # Errors
    ///
    /// This report construction does not currently fail; payload disclosure
    /// failures are captured as diagnostics in the returned report. The `Result`
    /// shape is kept aligned with the JSON verification API.
    pub fn report(&self, options: AiVerifyOptions) -> Result<AiArchiveReport, AiAdapterError> {
        let mut split_counts = BTreeMap::new();
        for sample in &self.sidecar.descriptor_tables.training_samples {
            let split = split_name_for_ref(sample.split_ref);
            *split_counts.entry(split.to_string()).or_insert(0usize) += 1;
        }
        let mut diagnostics = Vec::new();
        if self.sidecar.payload_access != AiPayloadAccessState::StructurallyAllowed {
            diagnostics.push(AiWithheldDiagnostic {
                code: "COVE_AI_PAYLOAD_POLICY_BLOCKED".to_string(),
                sample_id: None,
                message: "payload access is blocked because privacy summaries are missing"
                    .to_string(),
            });
        }
        if options.policy_report {
            for sample in &self.sidecar.descriptor_tables.training_samples {
                for (label, payload_ref) in [
                    ("input", sample.input_ref),
                    ("target", sample.target_ref),
                    ("metadata", sample.metadata_ref),
                ] {
                    if payload_ref != 0 {
                        let reader = AiPayloadReader::new(
                            &self.bytes,
                            &self.sidecar,
                            CoveAiAccessContext::for_operation("ai_verify"),
                        );
                        if let Err(error) = reader.disclosure_for_payload_ref(payload_ref) {
                            diagnostics.push(AiWithheldDiagnostic {
                                code: "COVE_AI_PAYLOAD_DISCLOSURE_CHECK_FAILED".to_string(),
                                sample_id: Some(sample.sample_id.to_string()),
                                message: format!("{label} payload_ref {payload_ref}: {error}"),
                            });
                        }
                    }
                }
            }
        }
        let quality = training_quality_diagnostics(&self.sidecar);
        diagnostics.extend(quality.diagnostics);
        if options.strict_training
            && diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with("COVE_AI_TRAINING_STRICT_"))
        {
            return Err(AiAdapterError::InvalidInput {
                message: "strict COVE-AI training verification failed; rerun without --strict-training or inspect the JSON diagnostics".to_string(),
            });
        }
        Ok(AiArchiveReport {
            path: self.path.clone(),
            artifact_id: hex_bytes(&self.sidecar.header.artifact_id),
            artifact_kind: match self.sidecar.artifact_kind {
                CoveAiArtifactKind::CoveAiBundle => "coveai",
                CoveAiArtifactKind::CoveVec => "covev",
            }
            .to_string(),
            payload_access: format!("{:?}", self.sidecar.payload_access),
            training_sample_count: self.sidecar.descriptor_tables.training_samples.len(),
            training_label_count: self.sidecar.descriptor_tables.training_labels.len(),
            preference_pair_count: self.sidecar.descriptor_tables.preference_pairs.len(),
            dedup_group_count: self.sidecar.descriptor_tables.dedup_groups.len(),
            generator_provenance_count: self.sidecar.descriptor_tables.generator_provenance.len(),
            epoch_plan_count: self.sidecar.descriptor_tables.training_epoch_plans.len(),
            split_counts,
            replayability: quality.replayability,
            contamination_risk_count: quality.contamination_risk_count,
            withheld_count: diagnostics.len(),
            diagnostics,
        })
    }

    /// Return training sample descriptor rows, optionally materializing payloads.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError`] if the requested split name is unsupported.
    pub fn training_samples(
        &self,
        options: AiSampleIteratorOptions,
    ) -> Result<Vec<Value>, AiAdapterError> {
        let split_filter = options
            .split
            .as_deref()
            .map(parse_split_filter)
            .transpose()?;
        let reader = AiPayloadReader::new(
            &self.bytes,
            &self.sidecar,
            if options.include_payloads {
                CoveAiAccessContext::for_operation("ai_training_samples")
            } else {
                CoveAiAccessContext::descriptor_only("ai_training_samples")
            },
        );
        let mut rows = Vec::new();
        for sample in &self.sidecar.descriptor_tables.training_samples {
            if let Some(split) = split_filter {
                if sample.split_ref != split.id() {
                    continue;
                }
            }
            rows.push(training_sample_value(
                sample,
                options.include_payloads,
                &reader,
            ));
        }
        Ok(rows)
    }

    /// Return the number of training samples matching an optional split filter.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError`] if the requested split name is unsupported.
    pub fn training_sample_count(&self, split: Option<&str>) -> Result<usize, AiAdapterError> {
        let split_filter = split.map(parse_split_filter).transpose()?;
        Ok(self
            .sidecar
            .descriptor_tables
            .training_samples
            .iter()
            .filter(|sample| split_filter.is_none_or(|split| sample.split_ref == split.id()))
            .count())
    }

    /// Return one training sample by filtered ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError`] if the requested split name is unsupported.
    pub fn training_sample_at(
        &self,
        index: usize,
        options: AiSampleIteratorOptions,
    ) -> Result<Option<Value>, AiAdapterError> {
        let split_filter = options
            .split
            .as_deref()
            .map(parse_split_filter)
            .transpose()?;
        let reader = AiPayloadReader::new(
            &self.bytes,
            &self.sidecar,
            if options.include_payloads {
                CoveAiAccessContext::for_operation("ai_training_sample_at")
            } else {
                CoveAiAccessContext::descriptor_only("ai_training_sample_at")
            },
        );
        let mut ordinal = 0usize;
        for sample in &self.sidecar.descriptor_tables.training_samples {
            if let Some(split) = split_filter {
                if sample.split_ref != split.id() {
                    continue;
                }
            }
            if ordinal == index {
                return Ok(Some(training_sample_value(
                    sample,
                    options.include_payloads,
                    &reader,
                )));
            }
            ordinal += 1;
        }
        Ok(None)
    }

    /// Return COVE-AI text chunk descriptor rows.
    ///
    /// # Errors
    ///
    /// This descriptor traversal does not currently fail, but the `Result`
    /// shape is kept aligned with the other adapter read APIs.
    pub fn chunks(&self, include_text: bool) -> Result<Vec<Value>, AiAdapterError> {
        let reader = AiPayloadReader::new(
            &self.bytes,
            &self.sidecar,
            if include_text {
                CoveAiAccessContext::for_operation("ai_chunks")
            } else {
                CoveAiAccessContext::descriptor_only("ai_chunks")
            },
        );
        Ok(self
            .sidecar
            .descriptor_tables
            .text_chunks
            .iter()
            .map(|chunk| {
                json!({
                    "record_kind": "text_chunk",
                    "chunk_id": chunk.chunk_id,
                    "source_ref": chunk.source_ref,
                    "byte_start": chunk.byte_start,
                    "byte_length": chunk.byte_length,
                    "chunk_text": payload_ref_json(chunk.source_ref, include_text, &reader),
                    "text_reconstruction": if include_text { "policy_gated_payload_lease" } else { "not_requested" },
                })
            })
            .collect())
    }

    /// Return COVE-AI token block descriptor rows.
    ///
    /// # Errors
    ///
    /// This descriptor traversal does not currently fail, but the `Result`
    /// shape is kept aligned with the other adapter read APIs.
    pub fn tokens(&self, include_payloads: bool) -> Result<Vec<Value>, AiAdapterError> {
        let reader = AiPayloadReader::new(
            &self.bytes,
            &self.sidecar,
            if include_payloads {
                CoveAiAccessContext::for_operation("ai_tokens")
            } else {
                CoveAiAccessContext::descriptor_only("ai_tokens")
            },
        );
        Ok(self
            .sidecar
            .descriptor_tables
            .token_blocks
            .iter()
            .map(|block| {
                json!({
                    "record_kind": "token_block",
                    "token_block_id": block.token_block_id,
                    "tokenizer_profile_id": block.tokenizer_profile_id,
                    "token_count": block.token_count,
                    "token_id_width": block.token_id_width,
                    "payload": payload_ref_json(block.payload_ref, include_payloads, &reader),
                })
            })
            .collect())
    }

    /// Return COVE-AI multimodal sequence descriptor rows.
    ///
    /// # Errors
    ///
    /// This descriptor traversal does not currently fail, but the `Result`
    /// shape is kept aligned with the other adapter read APIs.
    pub fn multimodal(&self, include_payloads: bool) -> Result<Vec<Value>, AiAdapterError> {
        let reader = AiPayloadReader::new(
            &self.bytes,
            &self.sidecar,
            if include_payloads {
                CoveAiAccessContext::for_operation("ai_multimodal")
            } else {
                CoveAiAccessContext::descriptor_only("ai_multimodal")
            },
        );
        Ok(self
            .sidecar
            .descriptor_tables
            .multimodal_sequence_elements
            .iter()
            .map(|element| {
                json!({
                    "record_kind": "multimodal_sequence_element",
                    "element_id": element.element_id,
                    "sequence_pack_id": element.sequence_pack_id,
                    "ordinal": element.ordinal,
                    "modality": element.modality,
                    "role": element.role,
                    "asset_ref": element.asset_ref,
                    "tensor_ref": element.tensor_ref,
                    "vector_ref": element.vector_ref,
                    "position_stream": payload_ref_json(element.position_stream_ref, include_payloads, &reader),
                    "evidence": payload_ref_json(element.evidence_ref, include_payloads, &reader),
                })
            })
            .collect())
    }

    /// Export training samples and report metadata in the requested AI format.
    ///
    /// # Errors
    ///
    /// Returns [`AiAdapterError`] if sample collection, JSON serialization,
    /// Arrow/Parquet writing, or WebDataset tar construction fails.
    pub fn export(&self, options: AiExportOptions) -> Result<AiExportData, AiAdapterError> {
        let samples = self.training_samples(AiSampleIteratorOptions {
            split: options.split.clone(),
            include_payloads: options.include_payloads,
        })?;
        let report = json!({
            "path": self.path.display().to_string(),
            "format": options.format.as_str(),
            "split": options.split,
            "include_payloads": options.include_payloads,
            "policy_report": options.policy_report,
            "artifact_id": hex_bytes(&self.sidecar.header.artifact_id),
            "payload_access": format!("{:?}", self.sidecar.payload_access),
            "sample_count": samples.len(),
            "samples": samples,
            "diagnostics": self.report(AiVerifyOptions { policy_report: options.policy_report, strict_training: false })?.diagnostics,
        });
        export_value(&report, options.format)
    }
}

/// Open a COVE-AI archive from a sidecar or manifest path.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if path resolution, file reading, or sidecar
/// parsing fails.
pub fn open(
    path: impl AsRef<Path>,
    options: AiArchiveOpenOptions,
) -> Result<AiTrainingArchive, AiAdapterError> {
    AiTrainingArchive::open(path, options)
}

/// Verify a COVE-AI archive and return a JSON report.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if the archive cannot be opened or verification
/// report construction fails.
pub fn verify_archive(
    path: impl AsRef<Path>,
    options: AiVerifyOptions,
) -> Result<Value, AiAdapterError> {
    AiTrainingArchive::open(path, AiArchiveOpenOptions::default())?.verify(options)
}

/// Import instruction, chat, pretrain, preference, or RAG samples from JSONL.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if the input cannot be read, a row is invalid
/// JSON, sample conversion fails, or the output sidecar cannot be published.
pub fn import_jsonl(
    input: impl AsRef<Path>,
    out: Option<impl AsRef<Path>>,
    options: AiImportOptions,
) -> Result<Value, AiAdapterError> {
    let input = input.as_ref();
    let text = fs::read_to_string(input).map_err(|source| AiAdapterError::Io {
        action: "read",
        path: input.to_path_buf(),
        source,
    })?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            format!(
                "{}:{}: invalid JSONL row: {error}",
                input.display(),
                index + 1
            )
        })?;
        rows.push(value);
    }
    Ok(import_values(
        input,
        &rows,
        out.as_ref().map(AsRef::as_ref),
        &options,
    )?)
}

/// Import Hugging Face-style JSONL records from a directory.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if the directory cannot be read, any JSONL file
/// cannot be read or parsed, no records are present, sample conversion fails, or
/// the output sidecar cannot be published.
pub fn import_hf_dir(
    input_dir: impl AsRef<Path>,
    out: Option<impl AsRef<Path>>,
    options: AiImportOptions,
) -> Result<Value, AiAdapterError> {
    let input_dir = input_dir.as_ref();
    let mut rows = Vec::new();
    for entry in fs::read_dir(input_dir).map_err(|source| AiAdapterError::Io {
        action: "read",
        path: input_dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| AiAdapterError::Io {
            action: "read",
            path: input_dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|source| AiAdapterError::Io {
            action: "read",
            path: path.clone(),
            source,
        })?;
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line).map_err(|error| {
                format!(
                    "{}:{}: invalid HF JSONL row: {error}",
                    path.display(),
                    index + 1
                )
            })?;
            rows.push(value);
        }
    }
    if rows.is_empty() {
        return Err(AiAdapterError::InvalidInput {
            message: format!(
                "{} did not contain any .jsonl records to import",
                input_dir.display()
            ),
        });
    }
    Ok(import_values(
        input_dir,
        &rows,
        out.as_ref().map(AsRef::as_ref),
        &options,
    )?)
}

/// Import AI training records from a Parquet file.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if the file cannot be opened, Parquet metadata or
/// batches cannot be read, sample conversion fails, or the output sidecar cannot
/// be published.
pub fn import_parquet(
    input: impl AsRef<Path>,
    out: Option<impl AsRef<Path>>,
    options: AiImportOptions,
) -> Result<Value, AiAdapterError> {
    let input = input.as_ref();
    let file = fs::File::open(input).map_err(|source| AiAdapterError::Io {
        action: "open",
        path: input.to_path_buf(),
        source,
    })?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|error| {
        format!(
            "cannot read Parquet metadata from {}: {error}",
            input.display()
        )
    })?;
    let reader = builder.build().map_err(|error| {
        format!(
            "cannot build Parquet row reader for {}: {error}",
            input.display()
        )
    })?;
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|error| format!("Parquet batch read failed: {error}"))?;
        rows.extend(record_batch_to_json_rows(&batch)?);
    }
    Ok(import_values(
        input,
        &rows,
        out.as_ref().map(AsRef::as_ref),
        &options,
    )?)
}

/// Open and export a COVE-AI archive in one step.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if the archive cannot be opened or export
/// serialization fails.
pub fn stream_archive(
    input: impl AsRef<Path>,
    options: AiExportOptions,
) -> Result<AiExportData, AiAdapterError> {
    let archive = AiTrainingArchive::open(input, AiArchiveOpenOptions::default())?;
    archive.export(options)
}

/// Diff two COVE-AI archives by a key field.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if either archive cannot be opened or sample
/// collection for the requested key field fails.
pub fn diff_archives(
    old_path: impl AsRef<Path>,
    new_path: impl AsRef<Path>,
    key_field: &str,
) -> Result<Value, AiAdapterError> {
    let old = AiTrainingArchive::open(old_path, AiArchiveOpenOptions::default())?;
    let new = AiTrainingArchive::open(new_path, AiArchiveOpenOptions::default())?;
    let old_samples = keyed_samples(&old, key_field)?;
    let new_samples = keyed_samples(&new, key_field)?;
    let old_keys = old_samples.keys().cloned().collect::<BTreeSet<_>>();
    let new_keys = new_samples.keys().cloned().collect::<BTreeSet<_>>();
    let added = new_keys
        .difference(&old_keys)
        .cloned()
        .collect::<Vec<String>>();
    let removed = old_keys
        .difference(&new_keys)
        .cloned()
        .collect::<Vec<String>>();
    let changed = old_keys
        .intersection(&new_keys)
        .filter(|key| old_samples.get(*key) != new_samples.get(*key))
        .cloned()
        .collect::<Vec<String>>();
    Ok(json!({
        "old": old.path.display().to_string(),
        "new": new.path.display().to_string(),
        "key_field": key_field,
        "old_count": old_samples.len(),
        "new_count": new_samples.len(),
        "added": added,
        "removed": removed,
        "changed": changed,
    }))
}

/// Build a deterministic COVE-AI training archive showcase.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if the output directory is invalid, fixture
/// generation fails, archive import or verification fails, or showcase export
/// files cannot be written.
pub fn build_ai_training_showcase(
    out_dir: impl AsRef<Path>,
    profile: &str,
    force: bool,
) -> Result<Value, AiAdapterError> {
    let out_dir = out_dir.as_ref();
    prepare_output_dir(out_dir, force)?;
    let sample_count = match profile {
        "quick" => 8usize,
        "standard" => 128usize,
        "publication" => 512usize,
        other => {
            return Err(AiAdapterError::InvalidInput {
                message: format!("unknown ai-training showcase profile '{other}'"),
            })
        }
    };
    let source_path = out_dir.join("training-source.jsonl");
    let mut source = String::new();
    for index in 0..sample_count {
        let topic = ["retention", "chunking", "vector search", "redaction"][index % 4];
        let withheld = index % 7 == 0;
        let sample = serde_json::to_string(&json!({
            "sample_id": format!("demo-{index:04}"),
            "instruction": format!("Explain governed COVE-AI {topic} behavior."),
            "input": format!("source_row={index}; policy_scope={}", if withheld { "withheld" } else { "public" }),
            "output": format!("COVE-AI records {topic} evidence, policy, and payload authority for sample {index}."),
            "generator": {
                "provider": "cove-deterministic-showcase",
                "model": "demo-generator-v1",
                "reproducibility_class": "deterministic-fixture"
            },
            "policy": {
                "payload_permission": !withheld,
                "diagnostic": if withheld { "showcase_policy_withheld" } else { "allowed" }
            }
        }))
        .map_err(|error| AiAdapterError::Export {
            message: format!("cannot serialize showcase sample: {error}"),
        })?;
        source.push_str(&sample);
        source.push('\n');
    }
    fs::write(&source_path, source)
        .map_err(|error| format!("cannot write {}: {error}", source_path.display()))?;
    let sidecar_path = out_dir.join("training.coveai");
    let import_report = import_jsonl(
        &source_path,
        Some(&sidecar_path),
        AiImportOptions {
            schema: AiImportSchema::Instruction,
            publish_covm: true,
            ..AiImportOptions::default()
        },
    )?;
    let archive = AiTrainingArchive::open(&sidecar_path, AiArchiveOpenOptions::default())?;
    let verify_report = archive.verify(AiVerifyOptions {
        policy_report: true,
        strict_training: false,
    })?;
    fs::write(
        out_dir.join("verification-report.json"),
        serde_json::to_vec_pretty(&verify_report).map_err(|error| AiAdapterError::Export {
            message: format!("cannot serialize verification report: {error}"),
        })?,
    )
    .map_err(|error| format!("cannot write verification report: {error}"))?;
    write_export_file(
        archive.export(AiExportOptions {
            format: AiExportFormat::HfJsonl,
            out: Some(out_dir.join("training.hf.jsonl")),
            include_payloads: true,
            ..AiExportOptions::default()
        })?,
        Some(out_dir.join("training.hf.jsonl")),
    )?;
    write_export_file(
        archive.export(AiExportOptions {
            format: AiExportFormat::Parquet,
            out: Some(out_dir.join("training.parquet")),
            include_payloads: true,
            ..AiExportOptions::default()
        })?,
        Some(out_dir.join("training.parquet")),
    )?;
    write_export_file(
        archive.export(AiExportOptions {
            format: AiExportFormat::WebDataset,
            out: Some(out_dir.join("training.tar")),
            include_payloads: true,
            ..AiExportOptions::default()
        })?,
        Some(out_dir.join("training.tar")),
    )?;
    let readme = format!(
        "# COVE-AI Training Archive Showcase\n\nThis deterministic {profile} profile demonstrates COVE-AI as a governed training archive of record.\n\nFiles:\n- `training-source.jsonl`: interop source rows.\n- `training.coveai`: authoritative COVE-AI training archive.\n- `training.covm`: digest-bound manifest with AI sidecar reference.\n- `verification-report.json`: policy/freshness report.\n- `training.hf.jsonl`, `training.parquet`, `training.tar`: export targets for existing AI stacks.\n\nPython:\n\n```bash\npython load_archive.py training.coveai\n```\n"
    );
    fs::write(out_dir.join("README.md"), readme)
        .map_err(|error| format!("cannot write showcase README: {error}"))?;
    fs::write(
        out_dir.join("load_archive.py"),
        "import sys\nimport cove_ai\narchive = cove_ai.open(sys.argv[1])\nprint(archive.verify())\nfor row in archive.training_samples(split='train', include_payloads=True):\n    print(row)\n    break\n",
    )
    .map_err(|error| format!("cannot write Python loader: {error}"))?;
    Ok(json!({
        "showcase": "ai-training",
        "profile": profile,
        "out_dir": out_dir.display().to_string(),
        "sample_count": sample_count,
        "import": import_report,
        "verify": verify_report,
        "exports": {
            "hf_jsonl": out_dir.join("training.hf.jsonl").display().to_string(),
            "parquet": out_dir.join("training.parquet").display().to_string(),
            "webdataset": out_dir.join("training.tar").display().to_string(),
        }
    }))
}

fn import_values(
    input_path: &Path,
    rows: &[Value],
    out: Option<&Path>,
    options: &AiImportOptions,
) -> Result<Value, AiImportBuildError> {
    let mut samples = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let sample = imported_sample_from_value(index, row, options)?;
        if !seen.insert(sample.sample_id_text.clone()) {
            return Err(AiImportBuildError::InvalidInput(format!(
                "duplicate AI training sample id '{}'",
                sample.sample_id_text
            )));
        }
        diagnostics.extend(sample.diagnostics.iter().cloned());
        samples.push(sample);
    }
    let mut split_counts = BTreeMap::new();
    for sample in &samples {
        *split_counts
            .entry(sample.split.as_str().to_string())
            .or_insert(0usize) += 1;
    }
    let report = json!({
        "input": input_path.display().to_string(),
        "schema": options.schema.as_str(),
        "split_policy": "deterministic",
        "sample_count": samples.len(),
        "split_counts": split_counts,
        "dry_run": options.dry_run,
        "diagnostics": diagnostics,
    });
    if options.dry_run {
        return Ok(report);
    }
    let out = out
        .ok_or_else(|| {
            AiImportBuildError::InvalidInput(
                "AI import requires --out unless --dry-run is used".to_string(),
            )
        })?
        .to_path_buf();
    let bytes = build_training_sidecar(&samples, options)?;
    durable_replace(&out, &bytes).map_err(|error| AiImportBuildError::OutputWrite {
        path: out.clone(),
        message: error.to_string(),
    })?;
    let sidecar = CoveAiFile::parse(&bytes).map_err(|error| {
        AiImportBuildError::Descriptor(format!(
            "internal error: imported sidecar did not validate: {error}"
        ))
    })?;
    let mut full_report = report;
    full_report["out"] = json!(out.display().to_string());
    full_report["artifact_id"] = json!(hex_bytes(&sidecar.header.artifact_id));
    full_report["payload_access"] = json!(format!("{:?}", sidecar.payload_access));
    if options.publish_covm {
        let covm_path = publish_import_covm(input_path, &out, &bytes, options.created_at_us)
            .map_err(AiImportBuildError::Manifest)?;
        full_report["covm"] = json!(covm_path.display().to_string());
    }
    Ok(full_report)
}

fn imported_sample_from_value(
    index: usize,
    row: &Value,
    options: &AiImportOptions,
) -> Result<ImportedSample, AiImportBuildError> {
    let mapping = options.mapping.as_ref();
    let sample_id_text = row
        .get("sample_id")
        .or_else(|| row.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("sample-{index:012}"));
    let split_field = mapping
        .and_then(|mapping| mapping.split_field.as_deref())
        .or(options.split_column.as_deref());
    let split = match split_field
        .and_then(|column| row.get(column))
        .and_then(Value::as_str)
        .and_then(SplitName::parse)
    {
        Some(split) => split,
        None => deterministic_split(&sample_id_text, row)?,
    };
    let quality_score_ppm = mapped_u32_ppm(
        row,
        mapping.and_then(|mapping| mapping.quality_score_ppm_field.as_deref()),
        &["quality_score_ppm", "quality_ppm", "quality_score"],
        1_000_000,
    )?;
    let sample_weight_ppm = mapped_u32_ppm(
        row,
        mapping.and_then(|mapping| mapping.sample_weight_ppm_field.as_deref()),
        &["sample_weight_ppm", "weight_ppm", "sample_weight"],
        1_000_000,
    )?;
    let dedup_key = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.dedup_key_field.as_deref()),
        &["dedup_key", "dedup_group", "source_hash"],
    )
    .map(value_to_key);
    let policy = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.policy_field.as_deref()),
        &["policy"],
    )
    .cloned();
    let license = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.license_field.as_deref()),
        &["license"],
    )
    .cloned();
    let source_ref = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.source_ref_field.as_deref()),
        &["source_ref", "source_refs", "source"],
    )
    .cloned();
    let evidence = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.evidence_field.as_deref()),
        &["evidence"],
    )
    .cloned();
    let labels = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.labels_field.as_deref()),
        &["labels", "label"],
    )
    .map(labels_from_value)
    .unwrap_or_default();
    let generator = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.generator_field.as_deref()),
        &["generator"],
    )
    .cloned();
    let human_review = mapped_value(
        row,
        mapping.and_then(|mapping| mapping.human_review_field.as_deref()),
        &["human_review"],
    )
    .cloned();
    let mut diagnostics = Vec::new();
    let ImportedPayloadParts {
        input,
        target,
        mut metadata,
    } = match options.schema {
        AiImportSchema::Instruction => instruction_payloads(&sample_id_text, row, &mut diagnostics),
        AiImportSchema::Chat => chat_payloads(&sample_id_text, row, &mut diagnostics),
        AiImportSchema::Pretrain => pretrain_payloads(&sample_id_text, row, &mut diagnostics),
        AiImportSchema::Preference => preference_payloads(&sample_id_text, row, &mut diagnostics),
        AiImportSchema::Rag => rag_payloads(&sample_id_text, row, &mut diagnostics),
    };
    metadata["sample_id"] = json!(sample_id_text);
    metadata["schema"] = json!(options.schema.as_str());
    metadata["split"] = json!(split.as_str());
    if row
        .pointer("/policy/payload_permission")
        .and_then(Value::as_bool)
        == Some(false)
    {
        diagnostics.push(AiWithheldDiagnostic {
            code: "COVE_AI_IMPORT_POLICY_WITHHELD".to_string(),
            sample_id: Some(sample_id_text.clone()),
            message: "source row declares payload_permission=false; payload is archived but marked in diagnostics".to_string(),
        });
    }
    let sample_id = sample_id_u64(&sample_id_text, row)?;
    Ok(ImportedSample {
        sample_id_text,
        sample_id,
        schema: options.schema,
        split,
        quality_score_ppm,
        sample_weight_ppm,
        dedup_key,
        policy,
        license,
        source_ref,
        evidence,
        labels,
        generator,
        human_review,
        input: serde_json::to_vec(&input).map_err(|error| {
            AiImportBuildError::InvalidInput(format!("cannot serialize AI sample input: {error}"))
        })?,
        target: serde_json::to_vec(&target).map_err(|error| {
            AiImportBuildError::InvalidInput(format!("cannot serialize AI sample target: {error}"))
        })?,
        metadata: serde_json::to_vec(&metadata).map_err(|error| {
            AiImportBuildError::InvalidInput(format!(
                "cannot serialize AI sample metadata: {error}"
            ))
        })?,
        diagnostics,
    })
}

#[derive(Debug, Clone)]
struct ImportedPayloadParts {
    input: Value,
    target: Value,
    metadata: Value,
}

fn instruction_payloads(
    sample_id: &str,
    row: &Value,
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
) -> ImportedPayloadParts {
    let instruction =
        required_string_or_diagnostic_placeholder(row, "instruction", sample_id, diagnostics);
    let input = row.get("input").cloned().unwrap_or(Value::Null);
    let output = required_value_or_diagnostic_placeholder(row, "output", sample_id, diagnostics);
    ImportedPayloadParts {
        input: json!({ "instruction": instruction, "input": input }),
        target: json!({ "output": output }),
        metadata: metadata_from_row(row),
    }
}

fn chat_payloads(
    sample_id: &str,
    row: &Value,
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
) -> ImportedPayloadParts {
    let messages = row
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if messages.is_empty() {
        push_import_diagnostic(
            diagnostics,
            "COVE_AI_IMPORT_MALFORMED_CHAT",
            sample_id,
            "chat schema requires messages[]",
        );
    }
    for message in &messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        if !matches!(role, "system" | "user" | "assistant" | "tool") {
            push_import_diagnostic(
                diagnostics,
                "COVE_AI_IMPORT_MALFORMED_CHAT_ROLE",
                sample_id,
                format!("unsupported chat role '{role}'"),
            );
        }
    }
    let target = messages
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .cloned()
        .unwrap_or_else(|| {
            push_import_diagnostic(
                diagnostics,
                "COVE_AI_IMPORT_MISSING_CHAT_TARGET",
                sample_id,
                "chat row has no assistant response",
            );
            Value::Null
        });
    ImportedPayloadParts {
        input: json!({ "messages": messages }),
        target: json!({ "assistant": target }),
        metadata: metadata_from_row(row),
    }
}

fn pretrain_payloads(
    sample_id: &str,
    row: &Value,
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
) -> ImportedPayloadParts {
    let text = required_string_or_diagnostic_placeholder(row, "text", sample_id, diagnostics);
    ImportedPayloadParts {
        input: json!({ "text": text }),
        target: Value::Null,
        metadata: metadata_from_row(row),
    }
}

fn preference_payloads(
    sample_id: &str,
    row: &Value,
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
) -> ImportedPayloadParts {
    let prompt = required_value_or_diagnostic_placeholder(row, "prompt", sample_id, diagnostics);
    let chosen = required_value_or_diagnostic_placeholder(row, "chosen", sample_id, diagnostics);
    let rejected =
        required_value_or_diagnostic_placeholder(row, "rejected", sample_id, diagnostics);
    ImportedPayloadParts {
        input: json!({ "prompt": prompt }),
        target: json!({ "chosen": chosen, "rejected": rejected }),
        metadata: metadata_from_row(row),
    }
}

fn rag_payloads(
    sample_id: &str,
    row: &Value,
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
) -> ImportedPayloadParts {
    let query = required_value_or_diagnostic_placeholder(row, "query", sample_id, diagnostics);
    let context = row.get("context").cloned().unwrap_or_else(|| {
        push_import_diagnostic(
            diagnostics,
            "COVE_AI_IMPORT_MISSING_RAG_CONTEXT",
            sample_id,
            "rag schema requires context[]",
        );
        Value::Array(Vec::new())
    });
    let answer = required_value_or_diagnostic_placeholder(row, "answer", sample_id, diagnostics);
    ImportedPayloadParts {
        input: json!({ "query": query, "context": context }),
        target: json!({ "answer": answer }),
        metadata: metadata_from_row(row),
    }
}

fn required_string_or_diagnostic_placeholder(
    row: &Value,
    field: &str,
    sample_id: &str,
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
) -> String {
    row.get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            push_import_diagnostic(
                diagnostics,
                "COVE_AI_IMPORT_MISSING_PAYLOAD_FIELD",
                sample_id,
                format!("missing required string field '{field}'"),
            );
            String::new()
        })
}

fn required_value_or_diagnostic_placeholder(
    row: &Value,
    field: &str,
    sample_id: &str,
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
) -> Value {
    row.get(field).cloned().unwrap_or_else(|| {
        push_import_diagnostic(
            diagnostics,
            "COVE_AI_IMPORT_MISSING_PAYLOAD_FIELD",
            sample_id,
            format!("missing required field '{field}'"),
        );
        Value::Null
    })
}

fn push_import_diagnostic(
    diagnostics: &mut Vec<AiWithheldDiagnostic>,
    code: &'static str,
    sample_id: &str,
    message: impl Into<String>,
) {
    diagnostics.push(AiWithheldDiagnostic {
        code: code.to_string(),
        sample_id: Some(sample_id.to_string()),
        message: message.into(),
    });
}

fn metadata_from_row(row: &Value) -> Value {
    let mut metadata = Map::new();
    for key in [
        "generator",
        "policy",
        "labels",
        "source_refs",
        "license",
        "evidence",
        "dedup_key",
        "quality_score_ppm",
        "sample_weight_ppm",
        "human_review",
    ] {
        if let Some(value) = row.get(key) {
            metadata.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(metadata)
}

fn mapped_value<'a>(
    row: &'a Value,
    mapped_field: Option<&str>,
    conventional_fields: &[&str],
) -> Option<&'a Value> {
    mapped_field
        .and_then(|field| row.get(field))
        .or_else(|| conventional_fields.iter().find_map(|field| row.get(*field)))
}

fn mapped_u32_ppm(
    row: &Value,
    mapped_field: Option<&str>,
    conventional_fields: &[&str],
    default: u32,
) -> Result<u32, AiImportBuildError> {
    let Some(value) = mapped_value(row, mapped_field, conventional_fields) else {
        return Ok(default);
    };
    let ppm = match value {
        Value::Number(number) => {
            if let Some(raw) = number.as_u64() {
                raw
            } else if let Some(raw) = number.as_f64() {
                if raw <= 1.0 {
                    (raw.max(0.0) * 1_000_000.0).round() as u64
                } else {
                    raw.round() as u64
                }
            } else {
                return Err(AiImportBuildError::InvalidInput(
                    "AI ppm field must be a non-negative number".to_string(),
                ));
            }
        }
        _ => {
            return Err(AiImportBuildError::InvalidInput(
                "AI ppm field must be numeric".to_string(),
            ));
        }
    };
    if ppm > 1_000_000 {
        return Err(AiImportBuildError::InvalidInput(format!(
            "AI ppm field exceeds 1_000_000: {ppm}"
        )));
    }
    Ok(ppm as u32)
}

fn labels_from_value(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(values) => values.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    }
}

fn build_training_sidecar(
    samples: &[ImportedSample],
    options: &AiImportOptions,
) -> Result<Vec<u8>, AiImportBuildError> {
    let mut payload = Vec::new();
    let mut tables = AiDescriptorTablesV1::default();
    let mut next_payload_ref = 1u32;
    let mut next_string_ref = 1u32;
    let mut next_digest_ref = 1u32;
    let mut next_policy_ref = 1u32;
    let mut next_label_id = 1u64;
    let mut next_model_actor_id = 1u32;
    let mut next_decoding_profile_id = 1u32;
    let mut next_human_review_id = 1u32;
    let mut model_actors = BTreeMap::<String, u32>::new();
    let mut decoding_profiles = BTreeMap::<String, u32>::new();
    let mut dedup_groups = BTreeMap::<String, u32>::new();
    let mut sample_records = Vec::with_capacity(samples.len());
    for sample in samples {
        if let Some(key) = &sample.dedup_key {
            if !dedup_groups.contains_key(key) {
                let next_group_id = u32::try_from(dedup_groups.len() + 1).map_err(|_| {
                    AiImportBuildError::Descriptor("too many AI dedup groups".to_string())
                })?;
                dedup_groups.insert(key.clone(), next_group_id);
            }
        }
    }
    let epoch_plan_enabled = options
        .mapping
        .as_ref()
        .and_then(|mapping| mapping.epoch_plan.as_ref())
        .is_some_and(|epoch| epoch.enabled);
    let mut split_source_snapshot_ref = 0;
    let mut split_hash_function_ref = 0;
    let mut split_grouping_ref = 0;
    let mut split_filter_policy_ref = 0;
    let mut split_ordering_policy_ref = 0;
    let mut split_dedup_policy_ref = 0;
    let mut epoch_rng_algorithm_ref = 0;
    let mut epoch_permutation_function_ref = 0;
    if epoch_plan_enabled {
        split_source_snapshot_ref = push_json_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &json!({
                "kind": "cove-ai-import-sample-set",
                "artifact_material": samples.iter().map(|sample| sample.sample_id_text.as_str()).collect::<Vec<_>>()
            }),
        )?;
        split_hash_function_ref = push_json_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &json!({"hash": "sha256", "input": "sample_id_or_canonical_json"}),
        )?;
        split_grouping_ref = push_json_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &json!({"grouping": "dedup_key_when_present"}),
        )?;
        split_filter_policy_ref = push_policy_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &mut next_digest_ref,
            &mut next_policy_ref,
            AI_POLICY_KIND_DISCLOSURE,
            &json!({"filter": "all-imported-samples"}),
        )?;
        split_ordering_policy_ref = push_policy_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &mut next_digest_ref,
            &mut next_policy_ref,
            AI_POLICY_KIND_DISCLOSURE,
            &json!({"ordering": "stable-sample-id"}),
        )?;
        split_dedup_policy_ref = push_policy_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &mut next_digest_ref,
            &mut next_policy_ref,
            AI_POLICY_KIND_DISCLOSURE,
            &json!({"dedup": "dedup_key-groups-when-present"}),
        )?;
        epoch_rng_algorithm_ref = push_json_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &json!({"rng": "deterministic-stable-sort", "seeded": true}),
        )?;
        epoch_permutation_function_ref = push_json_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &json!({"permutation": "split-then-sample-id"}),
        )?;
    }
    for sample in samples {
        let input_ref = push_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &sample.input,
        )?;
        let target_ref = push_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &sample.target,
        )?;
        let metadata_ref = push_payload_ref(
            &mut payload,
            &mut tables,
            &mut next_payload_ref,
            &sample.metadata,
        )?;
        let source_ref = sample
            .source_ref
            .as_ref()
            .map(|value| {
                push_json_payload_ref(&mut payload, &mut tables, &mut next_payload_ref, value)
            })
            .transpose()?
            .unwrap_or(0);
        let evidence_ref = sample
            .evidence
            .as_ref()
            .map(|value| {
                push_json_payload_ref(&mut payload, &mut tables, &mut next_payload_ref, value)
            })
            .transpose()?
            .unwrap_or(0);
        let policy_ref = sample
            .policy
            .as_ref()
            .map(|value| {
                push_policy_ref(
                    &mut payload,
                    &mut tables,
                    &mut next_payload_ref,
                    &mut next_digest_ref,
                    &mut next_policy_ref,
                    AI_POLICY_KIND_DISCLOSURE,
                    value,
                )
            })
            .transpose()?
            .unwrap_or(0);
        let license_ref = sample
            .license
            .as_ref()
            .map(|value| {
                push_policy_ref(
                    &mut payload,
                    &mut tables,
                    &mut next_payload_ref,
                    &mut next_digest_ref,
                    &mut next_policy_ref,
                    AI_POLICY_KIND_LICENSE,
                    value,
                )
            })
            .transpose()?
            .unwrap_or(0);
        let human_review_ref = sample
            .human_review
            .as_ref()
            .map(|value| {
                push_human_review(
                    &mut payload,
                    &mut tables,
                    &mut next_payload_ref,
                    &mut next_string_ref,
                    &mut next_human_review_id,
                    value,
                    policy_ref,
                )
            })
            .transpose()?
            .unwrap_or(0);
        let generator_provenance_ref = sample
            .generator
            .as_ref()
            .map(|value| {
                push_generator_provenance(
                    &mut payload,
                    &mut tables,
                    &mut next_payload_ref,
                    &mut next_string_ref,
                    &mut next_model_actor_id,
                    &mut next_decoding_profile_id,
                    &mut model_actors,
                    &mut decoding_profiles,
                    value,
                    sample.sample_id,
                    human_review_ref,
                    policy_ref,
                )
            })
            .transpose()?
            .unwrap_or(0);
        let mut first_label_ref = 0u32;
        for label in &sample.labels {
            let label_id = next_label_id;
            next_label_id = next_label_id.checked_add(1).ok_or_else(|| {
                AiImportBuildError::Descriptor("too many AI training labels".to_string())
            })?;
            if first_label_ref == 0 {
                first_label_ref = u32::try_from(label_id).unwrap_or(0);
            }
            let label_payload_ref =
                push_json_payload_ref(&mut payload, &mut tables, &mut next_payload_ref, label)?;
            tables.training_labels.push(TrainingLabelEntryV1 {
                label_id,
                label_kind: mapped_u8(label, "label_kind").unwrap_or(0),
                label_authority: mapped_u8(label, "label_authority").unwrap_or(0),
                label_payload_ref,
                generator_provenance_ref,
                human_review_ref,
                confidence_ppm: mapped_ppm_value(
                    label
                        .get("confidence_ppm")
                        .or_else(|| label.get("confidence")),
                )?,
                evidence_ref,
                policy_ref,
                flags: 0,
                checksum: 0,
            });
        }
        if sample.schema == AiImportSchema::Preference {
            push_preference_pair_from_sample(
                &mut payload,
                &mut tables,
                &mut next_payload_ref,
                sample,
                generator_provenance_ref,
                human_review_ref,
                evidence_ref,
                policy_ref,
            )?;
        }
        sample_records.push(TrainingSampleEntryV1 {
            sample_id: sample.sample_id,
            training_profile_id: 1,
            example_kind: example_kind(sample.schema),
            split_ref: sample.split.id(),
            source_ref,
            evidence_ref,
            input_ref,
            target_ref,
            label_ref: first_label_ref,
            metadata_ref,
            token_sequence_pack_ref: 0,
            multimodal_sequence_pack_ref: 0,
            vector_ref: 0,
            quality_score_ppm: sample.quality_score_ppm,
            sample_weight_ppm: sample.sample_weight_ppm,
            dedup_group_ref: sample
                .dedup_key
                .as_ref()
                .and_then(|key| dedup_groups.get(key).copied())
                .unwrap_or(0),
            license_ref,
            policy_ref,
            teacher_model_ref: 0,
            generator_provenance_ref,
            judge_generator_provenance_ref: generator_provenance_ref,
            label_generator_provenance_ref: generator_provenance_ref,
            flags: 0,
            checksum: 0,
        });
    }
    for (key, group_id) in &dedup_groups {
        let canonical = samples
            .iter()
            .filter(|sample| sample.dedup_key.as_ref() == Some(key))
            .map(|sample| sample.sample_id)
            .min()
            .unwrap_or(0);
        tables.dedup_groups.push(DedupGroupV1 {
            dedup_group_id: u64::from(*group_id),
            dedup_policy_ref: split_dedup_policy_ref,
            canonical_member_sample_id: canonical,
            similarity_kind: 0,
            dedup_authority: 0,
            confidence_ppm: 1_000_000,
            first_member_ref: 0,
            member_count: 0,
            flags: 0,
            checksum: 0,
        });
    }
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 0,
        sensitivity_bits_ref: 0,
        policy_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        retention_state: 1,
        disclosure_state: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.training_profiles.push(TrainingProfileV1 {
        training_profile_id: 1,
        profile_name_ref: 0,
        task_family: example_kind(options.schema),
        modality_mask: 1,
        source_snapshot_ref: 0,
        map_profile_ref: 0,
        chunk_profile_ref: 0,
        tokenizer_profile_ref: 0,
        vector_space_ref: 0,
        multimodal_sequence_profile_ref: 0,
        split_policy_ref: split_filter_policy_ref,
        sampling_policy_ref: 0,
        dedup_policy_ref: split_dedup_policy_ref,
        quality_policy_ref: 0,
        license_policy_ref: tables
            .policies
            .iter()
            .find(|policy| policy.policy_kind == AI_POLICY_KIND_LICENSE)
            .map(|policy| policy.policy_ref)
            .unwrap_or(0),
        redaction_policy_ref: 0,
        default_generator_provenance_ref: tables
            .generator_provenance
            .first()
            .map(|record| record.generator_provenance_id)
            .unwrap_or(0),
        reproducibility_class: 1,
        flags: 0,
        checksum: 0,
    });
    for split in [SplitName::Train, SplitName::Validation, SplitName::Test] {
        let count = samples
            .iter()
            .filter(|sample| sample.split == split)
            .count();
        tables.dataset_splits.push(DatasetSplitV1 {
            split_id: split.id(),
            split_name_ref: 0,
            split_method: 1,
            source_snapshot_ref: split_source_snapshot_ref,
            filter_policy_ref: split_filter_policy_ref,
            seed: options
                .mapping
                .as_ref()
                .and_then(|mapping| mapping.epoch_plan.as_ref())
                .and_then(|epoch| epoch.seed)
                .unwrap_or(0),
            hash_function_ref: split_hash_function_ref,
            stratification_path_ref: 0,
            grouping_ref: split_grouping_ref,
            ordering_policy_ref: split_ordering_policy_ref,
            dedup_policy_ref: split_dedup_policy_ref,
            sample_count: count as u64,
            first_sample_ref: 0,
            flags: 0,
            checksum: 0,
        });
    }
    if epoch_plan_enabled {
        let selected_split = options
            .mapping
            .as_ref()
            .and_then(|mapping| mapping.epoch_plan.as_ref())
            .and_then(|epoch| epoch.split.as_deref())
            .and_then(SplitName::parse);
        for split in [SplitName::Train, SplitName::Validation, SplitName::Test] {
            if selected_split.is_some_and(|selected| selected != split) {
                continue;
            }
            tables.training_epoch_plans.push(TrainingEpochPlanV1 {
                epoch_plan_id: u64::from(split.id()),
                training_profile_id: 1,
                split_ref: split.id(),
                seed: options
                    .mapping
                    .as_ref()
                    .and_then(|mapping| mapping.epoch_plan.as_ref())
                    .and_then(|epoch| epoch.seed)
                    .unwrap_or(0),
                permutation_kind: 1,
                rng_algorithm_ref: epoch_rng_algorithm_ref,
                permutation_function_ref: epoch_permutation_function_ref,
                shard_count: 0,
                first_shard_ref: 0,
                shard_ref_count: 0,
                flags: 0,
                checksum: 0,
            });
        }
    }
    tables.training_samples = sample_records;
    let payload_section = CoveAiWritableSection {
        section_id: PAYLOAD_SECTION_ID,
        section_kind: SectionKind::AiPayloadBytes as u32,
        profile_kind: PrimaryProfile::CoveAiShared as u8,
        payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
        requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
        source_binding_ref: 0,
        required_ai_features: 0,
        optional_ai_features: 0,
        feature_binding_ref: 0,
        payload,
    };
    write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: match options.artifact_id {
            Some(artifact_id) => artifact_id,
            None => artifact_id_from_samples(samples)?,
        },
        created_at_us: options.created_at_us.unwrap_or_else(now_us),
        payload_sections: vec![payload_section],
        descriptor_tables: tables,
    })
    .map_err(|error| {
        AiImportBuildError::Descriptor(format!("cannot write COVE-AI descriptor bundle: {error}"))
    })
}

fn push_payload_ref(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    bytes: &[u8],
) -> Result<u32, AiImportBuildError> {
    if bytes.is_empty() {
        return Ok(0);
    }
    let payload_ref = *next_payload_ref;
    *next_payload_ref = next_payload_ref
        .checked_add(1)
        .ok_or_else(|| AiImportBuildError::Descriptor("too many AI payload refs".to_string()))?;
    let offset = payload.len() as u64;
    payload.extend_from_slice(bytes);
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: PAYLOAD_SECTION_ID,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: offset,
        payload_length: bytes.len() as u64,
        decoded_length: bytes.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: checksum::crc32c(bytes),
    });
    Ok(payload_ref)
}

fn push_json_payload_ref(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    value: &Value,
) -> Result<u32, AiImportBuildError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        AiImportBuildError::InvalidInput(format!("cannot serialize AI mapped payload: {error}"))
    })?;
    push_payload_ref(payload, tables, next_payload_ref, &bytes)
}

fn push_digest_ref(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    next_digest_ref: &mut u32,
    bytes: &[u8],
) -> Result<u32, AiImportBuildError> {
    let digest = compute_digest(DigestAlgorithm::Sha256, bytes).map_err(|error| {
        AiImportBuildError::Digest(format!("cannot digest AI mapped payload: {error}"))
    })?;
    let digest_payload_ref = push_payload_ref(payload, tables, next_payload_ref, &digest)?;
    let digest_ref = *next_digest_ref;
    *next_digest_ref = next_digest_ref
        .checked_add(1)
        .ok_or_else(|| AiImportBuildError::Descriptor("too many AI digest refs".to_string()))?;
    tables.digests.push(AiDigestEntryV1 {
        digest_ref,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: digest.len() as u16,
        digest_payload_ref,
        domain_hint: 0,
        flags: 0,
        crc32c: 0,
    });
    Ok(digest_ref)
}

fn push_policy_ref(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    next_digest_ref: &mut u32,
    next_policy_ref: &mut u32,
    policy_kind: u8,
    value: &Value,
) -> Result<u32, AiImportBuildError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        AiImportBuildError::InvalidInput(format!("cannot serialize AI policy payload: {error}"))
    })?;
    let payload_ref = push_payload_ref(payload, tables, next_payload_ref, &bytes)?;
    let digest_ref = push_digest_ref(payload, tables, next_payload_ref, next_digest_ref, &bytes)?;
    let policy_ref = *next_policy_ref;
    *next_policy_ref = next_policy_ref
        .checked_add(1)
        .ok_or_else(|| AiImportBuildError::Descriptor("too many AI policy refs".to_string()))?;
    tables.policies.push(AiPolicyRefEntryV1 {
        policy_ref,
        policy_kind,
        authority_ref: 0,
        payload_ref,
        digest_ref,
        flags: 0,
        crc32c: 0,
    });
    Ok(policy_ref)
}

fn push_string_ref(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    next_string_ref: &mut u32,
    value: &str,
) -> Result<u32, AiImportBuildError> {
    if value.is_empty() {
        return Ok(0);
    }
    let string_ref = *next_string_ref;
    *next_string_ref = next_string_ref
        .checked_add(1)
        .ok_or_else(|| AiImportBuildError::Descriptor("too many AI string refs".to_string()))?;
    let payload_ref = push_payload_ref(payload, tables, next_payload_ref, value.as_bytes())?;
    tables.strings.push(AiStringEntryV1 {
        string_ref,
        utf8_byte_length: value.len() as u32,
        payload_ref,
        flags: 0,
        crc32c: 0,
    });
    Ok(string_ref)
}

#[allow(clippy::too_many_arguments)]
fn push_generator_provenance(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    next_string_ref: &mut u32,
    next_model_actor_id: &mut u32,
    next_decoding_profile_id: &mut u32,
    model_actors: &mut BTreeMap<String, u32>,
    decoding_profiles: &mut BTreeMap<String, u32>,
    value: &Value,
    source_sample_ref: u64,
    human_review_ref: u32,
    policy_ref: u32,
) -> Result<u64, AiImportBuildError> {
    let model_actor_ref = push_model_actor(
        payload,
        tables,
        next_payload_ref,
        next_string_ref,
        next_model_actor_id,
        model_actors,
        value,
        policy_ref,
    )?;
    let decoding_profile_ref = push_decoding_profile(
        tables,
        next_decoding_profile_id,
        decoding_profiles,
        value.get("decoding").unwrap_or(value),
    )?;
    let prompt_template_ref = value
        .get("prompt_template")
        .or_else(|| value.get("prompt_template_id"))
        .map(|payload_value| {
            push_json_payload_ref(payload, tables, next_payload_ref, payload_value)
        })
        .transpose()?
        .unwrap_or(0);
    let generator_id = source_sample_ref.max(1);
    tables.generator_provenance.push(GeneratorProvenanceV1 {
        generator_provenance_id: generator_id,
        generator_kind: 1,
        model_actor_ref,
        prompt_template_ref,
        decoding_profile_ref,
        toolchain_ref: 0,
        source_input_ref: 0,
        source_context_ref: 0,
        source_sample_ref,
        parent_generator_provenance_ref: 0,
        generation_time_us: value
            .get("generation_time_us")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        confidence_ppm: mapped_ppm_value(
            value
                .get("confidence_ppm")
                .or_else(|| value.get("confidence")),
        )?,
        human_review_ref,
        policy_ref,
        reproducibility_class: reproducibility_class_from_value(value.get("reproducibility_class")),
        flags: 0,
        checksum: 0,
    });
    Ok(generator_id)
}

#[allow(clippy::too_many_arguments)]
fn push_model_actor(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    next_string_ref: &mut u32,
    next_model_actor_id: &mut u32,
    model_actors: &mut BTreeMap<String, u32>,
    value: &Value,
    policy_ref: u32,
) -> Result<u32, AiImportBuildError> {
    let namespace = string_field(value, &["namespace", "model_namespace"]).unwrap_or("default");
    let name = string_field(value, &["model", "model_name", "name"]).unwrap_or("unknown-model");
    let version = string_field(value, &["version", "model_version"]).unwrap_or("");
    let provider = string_field(value, &["provider"]).unwrap_or("");
    let endpoint = string_field(value, &["endpoint"]).unwrap_or("");
    let key = format!("{namespace}\0{name}\0{version}\0{provider}\0{endpoint}");
    if let Some(id) = model_actors.get(&key) {
        return Ok(*id);
    }
    let model_actor_id = *next_model_actor_id;
    *next_model_actor_id = next_model_actor_id
        .checked_add(1)
        .ok_or_else(|| AiImportBuildError::Descriptor("too many AI model actors".to_string()))?;
    let model_namespace_ref = push_string_ref(
        payload,
        tables,
        next_payload_ref,
        next_string_ref,
        namespace,
    )?;
    let model_name_ref = push_string_ref(payload, tables, next_payload_ref, next_string_ref, name)?;
    let model_version_ref =
        push_string_ref(payload, tables, next_payload_ref, next_string_ref, version)?;
    let provider_ref =
        push_string_ref(payload, tables, next_payload_ref, next_string_ref, provider)?;
    let endpoint_ref =
        push_string_ref(payload, tables, next_payload_ref, next_string_ref, endpoint)?;
    let model_family_ref = push_string_ref(
        payload,
        tables,
        next_payload_ref,
        next_string_ref,
        string_field(value, &["family", "model_family"]).unwrap_or(""),
    )?;
    tables.model_actors.push(ModelActorDescriptorV1 {
        model_actor_id,
        model_namespace_ref,
        model_name_ref,
        model_version_ref,
        model_checkpoint_digest_ref: 0,
        provider_ref,
        endpoint_ref,
        endpoint_version_ref: 0,
        model_family_ref,
        modality_mask: 1,
        license_ref: 0,
        policy_ref,
        flags: 0,
        checksum: 0,
    });
    model_actors.insert(key, model_actor_id);
    Ok(model_actor_id)
}

fn push_decoding_profile(
    tables: &mut AiDescriptorTablesV1,
    next_decoding_profile_id: &mut u32,
    decoding_profiles: &mut BTreeMap<String, u32>,
    value: &Value,
) -> Result<u32, AiImportBuildError> {
    if !value.is_object() {
        return Ok(0);
    }
    let key = canonical_json(value);
    if let Some(id) = decoding_profiles.get(&key) {
        return Ok(*id);
    }
    let decoding_profile_id = *next_decoding_profile_id;
    *next_decoding_profile_id = next_decoding_profile_id.checked_add(1).ok_or_else(|| {
        AiImportBuildError::Descriptor("too many AI decoding profiles".to_string())
    })?;
    tables
        .generation_decoding_profiles
        .push(GenerationDecodingProfileV1 {
            decoding_profile_id,
            temperature_micros: float_micros(value.get("temperature")).unwrap_or(0),
            top_p_micros: float_micros(value.get("top_p")).unwrap_or(0),
            top_k: value.get("top_k").and_then(Value::as_u64).unwrap_or(0) as u32,
            seed: value.get("seed").and_then(Value::as_u64).unwrap_or(0),
            max_output_tokens: value
                .get("max_output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
            stop_sequence_ref: 0,
            safety_policy_ref: 0,
            deterministic_claim: value
                .get("deterministic")
                .and_then(Value::as_bool)
                .map(u8::from)
                .unwrap_or(0),
            flags: 0,
            checksum: 0,
        });
    decoding_profiles.insert(key, decoding_profile_id);
    Ok(decoding_profile_id)
}

fn push_human_review(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    next_string_ref: &mut u32,
    next_human_review_id: &mut u32,
    value: &Value,
    policy_ref: u32,
) -> Result<u32, AiImportBuildError> {
    let human_review_id = *next_human_review_id;
    *next_human_review_id = next_human_review_id
        .checked_add(1)
        .ok_or_else(|| AiImportBuildError::Descriptor("too many AI human reviews".to_string()))?;
    let notes_ref = value
        .get("notes")
        .map(|notes| push_json_payload_ref(payload, tables, next_payload_ref, notes))
        .transpose()?
        .unwrap_or(0);
    let reviewer_role_ref = push_string_ref(
        payload,
        tables,
        next_payload_ref,
        next_string_ref,
        string_field(value, &["reviewer_role", "role"]).unwrap_or(""),
    )?;
    tables.human_reviews.push(HumanReviewEntryV1 {
        human_review_id,
        review_kind: mapped_u8(value, "review_kind").unwrap_or(0),
        reviewer_role_ref,
        review_time_us: value
            .get("review_time_us")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        rating_ppm: mapped_ppm_value(value.get("rating_ppm").or_else(|| value.get("rating")))?,
        notes_ref,
        policy_ref,
        flags: 0,
        checksum: 0,
    });
    Ok(human_review_id)
}

#[allow(clippy::too_many_arguments)]
fn push_preference_pair_from_sample(
    payload: &mut Vec<u8>,
    tables: &mut AiDescriptorTablesV1,
    next_payload_ref: &mut u32,
    sample: &ImportedSample,
    judge_generator_provenance_ref: u64,
    human_review_ref: u32,
    evidence_ref: u32,
    policy_ref: u32,
) -> Result<(), AiImportBuildError> {
    let input: Value = serde_json::from_slice(&sample.input).map_err(|error| {
        AiImportBuildError::InvalidInput(format!("cannot parse preference input payload: {error}"))
    })?;
    let target: Value = serde_json::from_slice(&sample.target).map_err(|error| {
        AiImportBuildError::InvalidInput(format!("cannot parse preference target payload: {error}"))
    })?;
    let prompt_ref = push_json_payload_ref(
        payload,
        tables,
        next_payload_ref,
        input.get("prompt").unwrap_or(&Value::Null),
    )?;
    let chosen_ref = push_json_payload_ref(
        payload,
        tables,
        next_payload_ref,
        target.get("chosen").unwrap_or(&Value::Null),
    )?;
    let rejected_ref = push_json_payload_ref(
        payload,
        tables,
        next_payload_ref,
        target.get("rejected").unwrap_or(&Value::Null),
    )?;
    tables.preference_pairs.push(PreferencePairEntryV1 {
        preference_pair_id: sample.sample_id,
        prompt_ref,
        chosen_ref,
        rejected_ref,
        judge_generator_provenance_ref,
        human_review_ref,
        preference_strength_ppm: 1_000_000,
        confidence_ppm: 1_000_000,
        evidence_ref,
        policy_ref,
        flags: 0,
        checksum: 0,
    });
    Ok(())
}

fn mapped_ppm_value(value: Option<&Value>) -> Result<u32, AiImportBuildError> {
    let Some(value) = value else {
        return Ok(1_000_000);
    };
    mapped_u32_ppm(&json!({"value": value}), Some("value"), &[], 1_000_000)
}

fn mapped_u8(value: &Value, field: &str) -> Option<u8> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|raw| u8::try_from(raw).ok())
}

fn string_field<'a>(value: &'a Value, fields: &[&str]) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_str))
}

fn float_micros(value: Option<&Value>) -> Option<u32> {
    let raw = value?.as_f64()?;
    if !(0.0..=1_000_000.0).contains(&raw) {
        return None;
    }
    Some((raw * 1_000_000.0).round().min(u32::MAX as f64) as u32)
}

fn reproducibility_class_from_value(value: Option<&Value>) -> u8 {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|raw| u8::try_from(raw).ok())
            .unwrap_or(1)
            .min(5),
        Some(Value::String(value)) => match value.as_str() {
            "deterministic" | "deterministic-fixture" => 2,
            "externalAuditOnly" | "external-audit-only" => 3,
            "runtimeAdvisory" | "runtime-advisory" => 1,
            _ => 1,
        },
        _ => 1,
    }
}

fn export_value(report: &Value, format: AiExportFormat) -> Result<AiExportData, AiAdapterError> {
    let (media_type, bytes) = match format {
        AiExportFormat::Json => (
            "application/json",
            serde_json::to_vec_pretty(report).map_err(|error| {
                ai_export_error(format!("cannot serialize AI JSON export: {error}"))
            })?,
        ),
        AiExportFormat::Jsonl | AiExportFormat::HfJsonl => {
            let mut out = Vec::new();
            for sample in report
                .get("samples")
                .and_then(Value::as_array)
                .ok_or_else(|| ai_export_error("AI export report is missing samples"))?
            {
                let line = serde_json::to_string(sample).map_err(|error| {
                    ai_export_error(format!("cannot serialize AI JSONL sample: {error}"))
                })?;
                out.extend_from_slice(line.as_bytes());
                out.push(b'\n');
            }
            ("application/x-ndjson", out)
        }
        AiExportFormat::Arrow => (
            "application/vnd.apache.arrow.file",
            write_arrow_ipc(report).map_err(ai_export_error)?,
        ),
        AiExportFormat::Parquet => (
            "application/vnd.apache.parquet",
            write_parquet(report).map_err(ai_export_error)?,
        ),
        AiExportFormat::WebDataset => (
            "application/x-tar",
            write_webdataset(report).map_err(ai_export_error)?,
        ),
    };
    Ok(AiExportData {
        media_type,
        bytes,
        report: report.clone(),
    })
}

fn ai_export_error(message: impl Into<String>) -> AiAdapterError {
    AiAdapterError::Export {
        message: message.into(),
    }
}

/// Write exported AI data to a file or stdout.
///
/// # Errors
///
/// Returns [`AiAdapterError`] if durable file publication fails.
pub fn write_export_file(
    data: AiExportData,
    out: Option<impl AsRef<Path>>,
) -> Result<(), AiAdapterError> {
    if let Some(out) = out {
        let out = out.as_ref();
        durable_replace(out, &data.bytes)
            .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    } else {
        print!("{}", String::from_utf8_lossy(&data.bytes));
    }
    Ok(())
}

fn write_arrow_ipc(value: &Value) -> Result<Vec<u8>, String> {
    let batch = export_record_batch(value)?;
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = arrow_ipc::writer::FileWriter::try_new(&mut cursor, &batch.schema())
            .map_err(|error| format!("Arrow IPC writer: {error}"))?;
        writer
            .write(&batch)
            .map_err(|error| format!("Arrow IPC write: {error}"))?;
        writer
            .finish()
            .map_err(|error| format!("Arrow IPC finish: {error}"))?;
    }
    Ok(cursor.into_inner())
}

fn write_parquet(value: &Value) -> Result<Vec<u8>, String> {
    let batch = export_record_batch(value)?;
    let mut bytes = Vec::new();
    {
        let mut writer = parquet::arrow::ArrowWriter::try_new(&mut bytes, batch.schema(), None)
            .map_err(|error| format!("Parquet writer: {error}"))?;
        writer
            .write(&batch)
            .map_err(|error| format!("Parquet write: {error}"))?;
        writer
            .close()
            .map_err(|error| format!("Parquet close: {error}"))?;
    }
    Ok(bytes)
}

fn export_record_batch(value: &Value) -> Result<RecordBatch, String> {
    let samples = value
        .get("samples")
        .and_then(Value::as_array)
        .ok_or_else(|| "AI export value missing samples array".to_string())?;
    let ordinals = UInt64Array::from_iter_values(0..samples.len() as u64);
    let splits = StringArray::from(
        samples
            .iter()
            .map(|sample| {
                sample
                    .get("split")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect::<Vec<_>>(),
    );
    let payload_access = StringArray::from(
        samples
            .iter()
            .map(ai_record_payload_access_summary)
            .collect::<Vec<_>>(),
    );
    let record_json = StringArray::from(
        samples
            .iter()
            .map(|sample| {
                serde_json::to_string(sample)
                    .map_err(|error| format!("cannot serialize AI record JSON: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    let mut metadata = HashMap::new();
    if let Some(path) = value.get("path").and_then(Value::as_str) {
        metadata.insert("cove.ai.path".to_string(), path.to_string());
    }
    if let Some(artifact_id) = value.get("artifact_id").and_then(Value::as_str) {
        metadata.insert("cove.ai.artifact_id".to_string(), artifact_id.to_string());
    }
    let schema = Schema::new(vec![
        Field::new("record_ordinal", DataType::UInt64, false),
        Field::new("split", DataType::Utf8, false),
        Field::new("payload_access", DataType::Utf8, false),
        Field::new("record_json", DataType::Utf8, false),
    ])
    .with_metadata(metadata);
    RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(ordinals) as ArrayRef,
            Arc::new(splits) as ArrayRef,
            Arc::new(payload_access) as ArrayRef,
            Arc::new(record_json) as ArrayRef,
        ],
    )
    .map_err(|error| format!("cannot build AI archive Arrow batch: {error}"))
}

fn write_webdataset(value: &Value) -> Result<Vec<u8>, String> {
    let samples = value
        .get("samples")
        .and_then(Value::as_array)
        .ok_or_else(|| "AI export value missing samples array".to_string())?;
    let mut out = Vec::new();
    let mut metadata = value.clone();
    if let Some(object) = metadata.as_object_mut() {
        object.remove("samples");
    }
    write_tar_entry(
        &mut out,
        "metadata.json",
        &serde_json::to_vec_pretty(&metadata)
            .map_err(|error| format!("cannot serialize WebDataset metadata: {error}"))?,
    )?;
    for (index, sample) in samples.iter().enumerate() {
        let sample_json = serde_json::to_string(sample)
            .map_err(|error| format!("cannot serialize WebDataset sample: {error}"))?;
        write_tar_entry(
            &mut out,
            &format!("{index:06}.json"),
            sample_json.as_bytes(),
        )?;
    }
    out.extend_from_slice(&[0u8; 1024]);
    Ok(out)
}

fn training_sample_value(
    sample: &TrainingSampleEntryV1,
    include_payloads: bool,
    reader: &AiPayloadReader<'_>,
) -> Value {
    json!({
        "record_kind": "training_sample",
        "sample_id": sample.sample_id,
        "split": split_name_for_ref(sample.split_ref),
        "training_profile_id": sample.training_profile_id,
        "example_kind": sample.example_kind,
        "quality_score_ppm": sample.quality_score_ppm,
        "sample_weight_ppm": sample.sample_weight_ppm,
        "input": payload_ref_json(sample.input_ref, include_payloads, reader),
        "target": payload_ref_json(sample.target_ref, include_payloads, reader),
        "metadata": payload_ref_json(sample.metadata_ref, include_payloads, reader),
        "diagnostics": [],
    })
}

fn payload_ref_json(
    payload_ref: u32,
    include_payloads: bool,
    reader: &AiPayloadReader<'_>,
) -> Value {
    if payload_ref == 0 {
        return json!({
            "payload_ref": 0,
            "payload_access": "not_declared",
        });
    }
    if !include_payloads {
        return json!({
            "payload_ref": payload_ref,
            "payload_access": "not_requested",
        });
    }
    match reader.lease_payload_ref(payload_ref) {
        Ok(lease) => match std::str::from_utf8(lease.bytes) {
            Ok(text) => {
                let parsed = serde_json::from_str::<Value>(text).ok();
                json!({
                    "payload_ref": payload_ref,
                    "payload_access": lease.disclosure.as_str(),
                    "decoded_length": lease.decoded_length,
                    "json": parsed,
                    "text": text,
                })
            }
            Err(_) => json!({
                "payload_ref": payload_ref,
                "payload_access": lease.disclosure.as_str(),
                "decoded_length": lease.decoded_length,
                "bytes_hex": hex_bytes(lease.bytes),
            }),
        },
        Err(error) => json!({
            "payload_ref": payload_ref,
            "payload_access": "withheld",
            "withholding_reason": error.to_string(),
        }),
    }
}

fn keyed_samples(
    archive: &AiTrainingArchive,
    key_field: &str,
) -> Result<BTreeMap<String, Value>, String> {
    let mut keyed = BTreeMap::new();
    for sample in archive
        .training_samples(AiSampleIteratorOptions {
            split: None,
            include_payloads: true,
        })
        .map_err(|error| error.to_string())?
    {
        let key = sample_key(&sample, key_field).unwrap_or_else(|| {
            sample
                .get("sample_id")
                .map(Value::to_string)
                .unwrap_or_default()
        });
        keyed.insert(key, sample);
    }
    Ok(keyed)
}

fn sample_key(sample: &Value, key_field: &str) -> Option<String> {
    if key_field == "sample_id" {
        return sample.get("sample_id").map(Value::to_string);
    }
    for payload_name in ["metadata", "input", "target"] {
        let json = sample
            .get(payload_name)
            .and_then(|value| value.get("json"))?;
        if let Some(value) = json.get(key_field) {
            return Some(value_to_key(value));
        }
    }
    None
}

fn record_batch_to_json_rows(batch: &RecordBatch) -> Result<Vec<Value>, String> {
    let schema = batch.schema();
    let mut rows = Vec::with_capacity(batch.num_rows());
    for row_index in 0..batch.num_rows() {
        let mut object = Map::new();
        for (column_index, field) in schema.fields().iter().enumerate() {
            object.insert(
                field.name().clone(),
                arrow_scalar_to_json(batch.column(column_index).as_ref(), row_index)?,
            );
        }
        rows.push(Value::Object(object));
    }
    Ok(rows)
}

fn arrow_scalar_to_json(array: &dyn Array, row_index: usize) -> Result<Value, String> {
    if array.is_null(row_index) {
        return Ok(Value::Null);
    }
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(Value::String(values.value(row_index).to_string()));
    }
    if let Some(values) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok(json!(values.value(row_index)));
    }
    if let Some(values) = array.as_any().downcast_ref::<UInt64Array>() {
        return Ok(json!(values.value(row_index)));
    }
    if let Some(values) = array.as_any().downcast_ref::<Float64Array>() {
        return Ok(json!(values.value(row_index)));
    }
    if let Some(values) = array.as_any().downcast_ref::<BooleanArray>() {
        return Ok(json!(values.value(row_index)));
    }
    Err("Parquet import currently supports scalar Utf8, Int64, UInt64, Float64, and Boolean columns".to_string())
}

struct TrainingQualityDiagnostics {
    replayability: String,
    contamination_risk_count: usize,
    diagnostics: Vec<AiWithheldDiagnostic>,
}

fn training_quality_diagnostics(sidecar: &CoveAiFile) -> TrainingQualityDiagnostics {
    let tables = &sidecar.descriptor_tables;
    let mut diagnostics = Vec::new();
    let replayable_splits = tables.dataset_splits.iter().all(|split| {
        split.source_snapshot_ref != 0
            && split.hash_function_ref != 0
            && split.filter_policy_ref != 0
            && split.ordering_policy_ref != 0
            && split.dedup_policy_ref != 0
    });
    let replayable_epochs = !tables.training_epoch_plans.is_empty()
        && tables
            .training_epoch_plans
            .iter()
            .all(|plan| plan.rng_algorithm_ref != 0 && plan.permutation_function_ref != 0);
    if !replayable_splits {
        diagnostics.push(AiWithheldDiagnostic {
            code: "COVE_AI_TRAINING_STRICT_SPLIT_NOT_REPLAYABLE".to_string(),
            sample_id: None,
            message: "training splits are advisory because source snapshot, hash, filter, ordering, or dedup policy metadata is incomplete".to_string(),
        });
    }
    if !replayable_epochs {
        diagnostics.push(AiWithheldDiagnostic {
            code: "COVE_AI_TRAINING_STRICT_EPOCH_NOT_REPLAYABLE".to_string(),
            sample_id: None,
            message:
                "no replayable epoch plan declares both RNG algorithm and permutation function"
                    .to_string(),
        });
    }
    let synthetic_or_labeled = tables
        .training_samples
        .iter()
        .any(|sample| sample.generator_provenance_ref != 0 || sample.label_ref != 0)
        || !tables.training_labels.is_empty()
        || !tables.preference_pairs.is_empty();
    if synthetic_or_labeled && tables.generator_provenance.is_empty() {
        diagnostics.push(AiWithheldDiagnostic {
            code: "COVE_AI_TRAINING_STRICT_PROVENANCE_MISSING".to_string(),
            sample_id: None,
            message: "labels, preferences, or generated samples are present without generator provenance records".to_string(),
        });
    }
    let sample_split_by_id = tables
        .training_samples
        .iter()
        .map(|sample| (sample.sample_id, sample.split_ref))
        .collect::<BTreeMap<_, _>>();
    let mut groups = BTreeMap::<u32, BTreeSet<u32>>::new();
    for sample in &tables.training_samples {
        if sample.dedup_group_ref != 0 {
            groups
                .entry(sample.dedup_group_ref)
                .or_default()
                .insert(sample.split_ref);
        }
    }
    for group in &tables.dedup_groups {
        if let Some(split) = sample_split_by_id.get(&group.canonical_member_sample_id) {
            groups
                .entry(group.dedup_group_id as u32)
                .or_default()
                .insert(*split);
        }
    }
    let contamination_risk_count = groups
        .values()
        .filter(|splits| splits.contains(&1) && (splits.contains(&2) || splits.contains(&3)))
        .count();
    if contamination_risk_count != 0 {
        diagnostics.push(AiWithheldDiagnostic {
            code: "COVE_AI_TRAINING_STRICT_DEDUP_CONTAMINATION".to_string(),
            sample_id: None,
            message: format!(
                "{contamination_risk_count} dedup group(s) cross train and evaluation/test splits"
            ),
        });
    }
    let replayability = if replayable_splits && replayable_epochs {
        "replayable"
    } else {
        "advisory"
    }
    .to_string();
    TrainingQualityDiagnostics {
        replayability,
        contamination_risk_count,
        diagnostics,
    }
}

fn resolve_sidecar_path(path: &Path, options: &AiArchiveOpenOptions) -> Result<PathBuf, String> {
    if let Some(sidecar) = &options.cove_ai {
        return Ok(sidecar.clone());
    }
    if path.extension().and_then(|value| value.to_str()) == Some("covm") {
        let bytes =
            fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let manifest = CovmFile::parse(&bytes)
            .map_err(|error| format!("cannot parse COVM manifest {}: {error}", path.display()))?;
        let extension = CovmAiSidecarExtensionV1::find_in_covm_bytes(&bytes)
            .map_err(|error| format!("cannot inspect COVM AI sidecar refs: {error}"))?
            .ok_or_else(|| format!("{} has no COVM AI sidecar extension", path.display()))?;
        let reference = extension
            .refs
            .first()
            .ok_or_else(|| format!("{} has no AI sidecar references", path.display()))?;
        reference
            .validate_source_member(&manifest)
            .map_err(|error| {
                format!(
                    "{} has a stale COVM AI source binding: {error}",
                    path.display()
                )
            })?;
        let base = options.dataset_dir.clone().unwrap_or_else(|| {
            path.parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        });
        validate_covm_source_member_bytes(&base, &manifest, reference)?;
        let sidecar_path = base.join(&reference.uri);
        let sidecar_bytes = fs::read(&sidecar_path)
            .map_err(|error| format!("cannot read {}: {error}", sidecar_path.display()))?;
        reference
            .validate_sidecar_bytes(&sidecar_bytes)
            .map_err(|error| {
                format!(
                    "{} is stale or invalid for {}: {error}",
                    sidecar_path.display(),
                    path.display()
                )
            })?;
        return Ok(sidecar_path);
    }
    Ok(path.to_path_buf())
}

fn validate_covm_source_member_bytes(
    base: &Path,
    manifest: &CovmFile,
    reference: &CovmAiSidecarRefV1,
) -> Result<(), String> {
    let entry = manifest
        .files
        .iter()
        .find(|entry| entry.file_id == reference.source_file_id)
        .ok_or_else(|| "COVM AI sidecar source member is missing".to_string())?;
    let source_path = base.join(&entry.uri);
    let bytes = fs::read(&source_path).map_err(|error| {
        format!(
            "cannot read COVM AI source member {}: {error}",
            source_path.display()
        )
    })?;
    if bytes.len() as u64 != entry.file_len {
        return Err(format!(
            "{} is stale for COVM AI source binding: length mismatch",
            source_path.display()
        ));
    }
    let algorithm = DigestAlgorithm::from_u16(entry.digest_algorithm).ok_or_else(|| {
        format!(
            "{} uses unsupported COVM source digest algorithm {}",
            source_path.display(),
            entry.digest_algorithm
        )
    })?;
    let digest = compute_digest(algorithm, &bytes)
        .map_err(|error| format!("cannot digest {}: {error}", source_path.display()))?;
    if digest != entry.digest {
        return Err(format!(
            "{} is stale for COVM AI source binding: digest mismatch",
            source_path.display()
        ));
    }
    Ok(())
}

fn publish_import_covm(
    input_path: &Path,
    sidecar_path: &Path,
    sidecar_bytes: &[u8],
    created_at_us: Option<i64>,
) -> Result<PathBuf, String> {
    let input_bytes = fs::read(input_path).map_err(|error| {
        format!(
            "cannot read source {} for COVM publication: {error}",
            input_path.display()
        )
    })?;
    let digest = compute_digest(DigestAlgorithm::Sha256, &input_bytes)
        .map_err(|error| format!("cannot digest source {}: {error}", input_path.display()))?;
    let mut file_id = [0u8; 16];
    file_id.copy_from_slice(&digest[..16]);
    let mut dataset_id = [0u8; 16];
    let sidecar_digest = compute_digest(DigestAlgorithm::Sha256, sidecar_bytes)
        .map_err(|error| format!("cannot digest sidecar: {error}"))?;
    dataset_id.copy_from_slice(&sidecar_digest[..16]);
    let source_uri = covm_source_uri(input_path, sidecar_path);
    let sidecar_uri = sidecar_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("{} has no file name", sidecar_path.display()))?
        .to_string();
    let covm = CovmFile {
        header: CovmHeaderV1::new(dataset_id, 1, 1, created_at_us.unwrap_or_else(now_us)),
        files: vec![CovmFileEntryV1 {
            file_id,
            uri: source_uri,
            file_len: input_bytes.len() as u64,
            footer_crc32c: 0,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest,
            row_count: 0,
            segment_count: 1,
            file_stats_ref: 0,
            file_exact_set_ref: 0,
            flags: 0,
        }],
        postscript: cove_core::artifact::covm::CovmPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    };
    let extension = CovmAiSidecarExtensionV1 {
        flags: 0,
        refs: vec![CovmAiSidecarRefV1::new(
            file_id,
            CoveAiArtifactKind::CoveAiBundle,
            sidecar_uri,
            sidecar_bytes,
        )
        .map_err(|error| format!("cannot create COVM AI sidecar ref: {error}"))?],
    };
    let covm_bytes = covm
        .serialize_with_extension_region(
            &extension
                .serialize()
                .map_err(|error| format!("cannot serialize COVM AI sidecar extension: {error}"))?,
        )
        .map_err(|error| format!("cannot serialize COVM manifest: {error}"))?;
    let covm_path = sidecar_path.with_extension("covm");
    durable_replace(&covm_path, &covm_bytes)
        .map_err(|error| format!("cannot write {}: {error}", covm_path.display()))?;
    Ok(covm_path)
}

fn covm_source_uri(input_path: &Path, sidecar_path: &Path) -> String {
    let source_abs = input_path
        .canonicalize()
        .unwrap_or_else(|_| input_path.to_path_buf());
    let sidecar_parent_abs = sidecar_path
        .parent()
        .and_then(|parent| parent.canonicalize().ok());
    if source_abs.parent() == sidecar_parent_abs.as_deref() {
        return source_abs
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("source")
            .to_string();
    }
    source_abs.to_string_lossy().to_string()
}

fn deterministic_split(sample_id: &str, row: &Value) -> Result<SplitName, AiImportBuildError> {
    let key = if sample_id.starts_with("sample-") {
        canonical_json(row)
    } else {
        sample_id.to_string()
    };
    let digest = compute_digest(DigestAlgorithm::Sha256, key.as_bytes()).map_err(|error| {
        AiImportBuildError::Digest(format!("cannot digest AI split key: {error}"))
    })?;
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    let bucket = u64::from_le_bytes(first) % 1_000_000;
    Ok(if bucket < DEFAULT_TRAIN_PPM {
        SplitName::Train
    } else if bucket < DEFAULT_TRAIN_PPM + DEFAULT_VALIDATION_PPM {
        SplitName::Validation
    } else {
        SplitName::Test
    })
}

fn sample_id_u64(sample_id: &str, row: &Value) -> Result<u64, AiImportBuildError> {
    if let Some(value) = row.get("sample_id").and_then(Value::as_u64) {
        return Ok(value.max(1));
    }
    let digest = compute_digest(DigestAlgorithm::Sha256, sample_id.as_bytes())
        .map_err(|error| AiImportBuildError::Digest(format!("cannot digest sample id: {error}")))?;
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    Ok(u64::from_le_bytes(first).max(1))
}

fn artifact_id_from_samples(samples: &[ImportedSample]) -> Result<[u8; 16], AiImportBuildError> {
    let mut material = String::new();
    for sample in samples {
        material.push_str(&sample.sample_id_text);
        material.push('\n');
    }
    let digest = compute_digest(DigestAlgorithm::Sha256, material.as_bytes()).map_err(|error| {
        AiImportBuildError::Digest(format!("cannot digest AI artifact id: {error}"))
    })?;
    let mut artifact_id = [0u8; 16];
    artifact_id.copy_from_slice(&digest[..16]);
    Ok(artifact_id)
}

fn parse_split_filter(value: &str) -> Result<SplitName, String> {
    SplitName::parse(value)
        .ok_or_else(|| format!("unknown split '{value}'; expected train, validation, or test"))
}

fn split_name_for_ref(split_ref: u32) -> &'static str {
    match split_ref {
        1 => "train",
        2 => "validation",
        3 => "test",
        _ => "unspecified",
    }
}

fn example_kind(schema: AiImportSchema) -> u8 {
    match schema {
        AiImportSchema::Instruction => 1,
        AiImportSchema::Chat => 2,
        AiImportSchema::Pretrain => 3,
        AiImportSchema::Preference => 4,
        AiImportSchema::Rag => 5,
    }
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in object {
                sorted.insert(key.clone(), canonical_json(value));
            }
            let mut out = String::from("{");
            for (index, (key, value)) in sorted.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                out.push_str(value);
            }
            out.push('}');
            out
        }
        Value::Array(array) => {
            let mut out = String::from("[");
            for (index, value) in array.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json(value));
            }
            out.push(']');
            out
        }
        _ => value.to_string(),
    }
}

fn value_to_key(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn ai_record_payload_access_summary(record: &Value) -> String {
    let mut values = Vec::new();
    collect_payload_access_values(record, &mut values);
    values.sort();
    values.dedup();
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(",")
    }
}

fn collect_payload_access_values(value: &Value, values: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            if let Some(access) = object.get("payload_access").and_then(Value::as_str) {
                values.push(access.to_string());
            }
            for child in object.values() {
                collect_payload_access_values(child, values);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_payload_access_values(child, values);
            }
        }
        _ => {}
    }
}

fn write_tar_entry(out: &mut Vec<u8>, name: &str, data: &[u8]) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    if name_bytes.is_empty() || name_bytes.len() > 100 {
        return Err(format!("WebDataset tar member name too long: {name}"));
    }
    let mut header = [0u8; 512];
    header[..name_bytes.len()].copy_from_slice(name_bytes);
    write_tar_octal(&mut header[100..108], 0o644);
    write_tar_octal(&mut header[108..116], 0);
    write_tar_octal(&mut header[116..124], 0);
    write_tar_octal(&mut header[124..136], data.len() as u64);
    write_tar_octal(&mut header[136..148], 0);
    for byte in &mut header[148..156] {
        *byte = b' ';
    }
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum = header
        .iter()
        .fold(0u32, |sum, byte| sum.saturating_add(u32::from(*byte)));
    let checksum_text = format!("{checksum:06o}\0 ");
    header[148..156].copy_from_slice(checksum_text.as_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(data);
    let padding = (512 - (data.len() % 512)) % 512;
    out.extend(std::iter::repeat_n(0u8, padding));
    Ok(())
}

fn write_tar_octal(field: &mut [u8], value: u64) {
    field.fill(0);
    let digits = field.len().saturating_sub(1);
    let text = format!("{value:0width$o}", width = digits);
    let bytes = text.as_bytes();
    let start = digits.saturating_sub(bytes.len());
    field[start..start + bytes.len()].copy_from_slice(bytes);
}

fn prepare_output_dir(out_dir: &Path, force: bool) -> Result<(), String> {
    if out_dir.exists() {
        if !force {
            let mut entries = fs::read_dir(out_dir)
                .map_err(|error| format!("cannot inspect {}: {error}", out_dir.display()))?;
            if entries.next().is_some() {
                return Err(format!(
                    "{} already exists and is not empty; pass --force to replace generated files",
                    out_dir.display()
                ));
            }
        }
    } else {
        fs::create_dir_all(out_dir)
            .map_err(|error| format!("cannot create {}: {error}", out_dir.display()))?;
    }
    Ok(())
}

fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_split_is_stable() {
        let row = json!({"sample_id": "stable-1", "instruction": "a", "output": "b"});
        assert_eq!(
            deterministic_split("stable-1", &row).unwrap().as_str(),
            deterministic_split("stable-1", &row).unwrap().as_str()
        );
    }

    #[test]
    fn import_rejects_duplicate_sample_ids() {
        let rows = vec![
            json!({"sample_id": "dup", "instruction": "a", "output": "b"}),
            json!({"sample_id": "dup", "instruction": "c", "output": "d"}),
        ];
        let error = import_values(
            Path::new("memory.jsonl"),
            &rows,
            None::<&Path>,
            &AiImportOptions {
                dry_run: true,
                ..AiImportOptions::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn import_schema_parse_reports_typed_error() {
        assert_eq!(
            "instruction".parse::<AiImportSchema>().unwrap(),
            AiImportSchema::Instruction
        );
        assert_eq!(
            "deterministic".parse::<AiSplitPolicy>().unwrap(),
            AiSplitPolicy::Deterministic
        );

        let error = AiImportSchema::parse("made-up-schema").unwrap_err();
        assert!(matches!(
            error,
            AiAdapterError::UnsupportedImportSchema { .. }
        ));
        assert!(error
            .to_string()
            .contains("unsupported AI import schema 'made-up-schema'"));
    }

    #[test]
    fn import_jsonl_reports_typed_read_error() {
        let error = import_jsonl(
            "__cove_ai_adapters_missing_import.jsonl__",
            None::<&Path>,
            AiImportOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(error, AiAdapterError::Io { action: "read", .. }));
        assert!(error
            .to_string()
            .contains("cannot read __cove_ai_adapters_missing_import.jsonl__"));
    }

    #[test]
    fn import_builds_valid_payload_lease_archive() {
        let rows = vec![json!({
            "sample_id": "s1",
            "instruction": "Summarize COVE-AI.",
            "input": "policy aware archive",
            "output": "COVE-AI records payload authority."
        })];
        let report = import_values(
            Path::new("memory.jsonl"),
            &rows,
            None::<&Path>,
            &AiImportOptions {
                dry_run: false,
                artifact_id: Some([7u8; 16]),
                created_at_us: Some(1),
                ..AiImportOptions::default()
            },
        );
        assert!(report.is_err(), "non-dry import requires an output path");

        let samples = vec![imported_sample_from_value(
            0,
            &json!({
                "sample_id": "s1",
                "instruction": "Summarize COVE-AI.",
                "input": "policy aware archive",
                "output": "COVE-AI records payload authority."
            }),
            &AiImportOptions {
                artifact_id: Some([7u8; 16]),
                created_at_us: Some(1),
                ..AiImportOptions::default()
            },
        )
        .unwrap()];
        let bytes = build_training_sidecar(
            &samples,
            &AiImportOptions {
                artifact_id: Some([7u8; 16]),
                created_at_us: Some(1),
                ..AiImportOptions::default()
            },
        )
        .unwrap();
        let sidecar = CoveAiFile::parse(&bytes).unwrap();
        let reader =
            AiPayloadReader::new(&bytes, &sidecar, CoveAiAccessContext::for_operation("test"));
        let lease = reader.lease_payload_ref(1).unwrap();
        assert!(std::str::from_utf8(lease.bytes)
            .unwrap()
            .contains("Summarize COVE-AI"));
    }

    #[test]
    fn mapped_import_populates_training_metadata_and_passes_strict_report() {
        let options = AiImportOptions {
            mapping: Some(AiImportMapping {
                split_field: Some("split".to_string()),
                quality_score_ppm_field: Some("quality".to_string()),
                sample_weight_ppm_field: Some("weight".to_string()),
                dedup_key_field: Some("dedup".to_string()),
                labels_field: Some("labels".to_string()),
                generator_field: Some("generator".to_string()),
                human_review_field: Some("review".to_string()),
                epoch_plan: Some(AiImportEpochPlanMapping {
                    enabled: true,
                    seed: Some(42),
                    split: None,
                }),
                ..AiImportMapping::default()
            }),
            artifact_id: Some([8u8; 16]),
            created_at_us: Some(1),
            ..AiImportOptions::default()
        };
        let rows = [
            json!({
                "sample_id": "s1",
                "split": "train",
                "instruction": "Explain COVE-AI.",
                "output": "COVE-AI archives training data.",
                "quality": 0.9,
                "weight": 0.5,
                "dedup": "source-a",
                "labels": [{"label": "accepted", "confidence": 0.8}],
                "generator": {
                    "provider": "local",
                    "model": "fixture-model",
                    "version": "1",
                    "reproducibility_class": "deterministic"
                },
                "review": {"role": "annotator", "rating": 1.0}
            }),
            json!({
                "sample_id": "s2",
                "split": "validation",
                "instruction": "Explain strict verification.",
                "output": "Strict verification requires replay metadata.",
                "dedup": "source-b",
                "generator": {
                    "provider": "local",
                    "model": "fixture-model",
                    "version": "1"
                }
            }),
        ];
        let samples = rows
            .iter()
            .enumerate()
            .map(|(index, row)| imported_sample_from_value(index, row, &options).unwrap())
            .collect::<Vec<_>>();
        let bytes = build_training_sidecar(&samples, &options).unwrap();
        let sidecar = CoveAiFile::parse(&bytes).unwrap();
        assert_eq!(sidecar.descriptor_tables.training_labels.len(), 1);
        assert_eq!(sidecar.descriptor_tables.generator_provenance.len(), 2);
        assert_eq!(sidecar.descriptor_tables.model_actors.len(), 1);
        assert_eq!(sidecar.descriptor_tables.dedup_groups.len(), 2);
        assert_eq!(sidecar.descriptor_tables.training_epoch_plans.len(), 3);
        assert_eq!(
            sidecar.descriptor_tables.training_samples[0].quality_score_ppm,
            900_000
        );
        assert_eq!(
            sidecar.descriptor_tables.training_samples[0].sample_weight_ppm,
            500_000
        );
        let archive = AiTrainingArchive {
            path: PathBuf::from("memory.coveai"),
            bytes,
            sidecar,
        };
        let report = archive
            .report(AiVerifyOptions {
                policy_report: true,
                strict_training: true,
            })
            .unwrap();
        assert_eq!(report.replayability, "replayable");
        assert_eq!(report.training_label_count, 1);
        assert_eq!(report.generator_provenance_count, 2);
        assert_eq!(report.contamination_risk_count, 0);
    }

    #[test]
    fn strict_training_report_rejects_advisory_imports() {
        let options = AiImportOptions {
            artifact_id: Some([9u8; 16]),
            created_at_us: Some(1),
            ..AiImportOptions::default()
        };
        let sample = imported_sample_from_value(
            0,
            &json!({
                "sample_id": "advisory",
                "instruction": "Explain advisory archives.",
                "output": "They are valid but not replayable."
            }),
            &options,
        )
        .unwrap();
        let bytes = build_training_sidecar(&[sample], &options).unwrap();
        let sidecar = CoveAiFile::parse(&bytes).unwrap();
        let archive = AiTrainingArchive {
            path: PathBuf::from("memory.coveai"),
            bytes,
            sidecar,
        };
        let report = archive
            .report(AiVerifyOptions {
                policy_report: true,
                strict_training: false,
            })
            .unwrap();
        assert_eq!(report.replayability, "advisory");
        let error = archive
            .report(AiVerifyOptions {
                policy_report: true,
                strict_training: true,
            })
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("strict COVE-AI training verification failed"));
    }

    #[test]
    fn import_missing_required_payload_fields_uses_placeholders_and_diagnostics() {
        let sample = imported_sample_from_value(
            0,
            &json!({ "sample_id": "missing-instruction-output" }),
            &AiImportOptions::default(),
        )
        .unwrap();

        assert_eq!(sample.sample_id_text, "missing-instruction-output");
        assert_eq!(
            sample
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "COVE_AI_IMPORT_MISSING_PAYLOAD_FIELD")
                .count(),
            2
        );

        let input: Value = serde_json::from_slice(&sample.input).unwrap();
        let target: Value = serde_json::from_slice(&sample.target).unwrap();
        let metadata: Value = serde_json::from_slice(&sample.metadata).unwrap();
        assert_eq!(input["instruction"], "");
        assert_eq!(input["input"], Value::Null);
        assert_eq!(target["output"], Value::Null);
        assert_eq!(metadata["sample_id"], "missing-instruction-output");
        assert_eq!(metadata["schema"], "instruction");
    }

    #[test]
    fn ai_export_format_parse_accepts_spec_strings() {
        for (value, expected) in [
            ("json", AiExportFormat::Json),
            ("jsonl", AiExportFormat::Jsonl),
            ("hf-jsonl", AiExportFormat::HfJsonl),
            ("arrow", AiExportFormat::Arrow),
            ("parquet", AiExportFormat::Parquet),
            ("webdataset", AiExportFormat::WebDataset),
        ] {
            let parsed = AiExportFormat::parse(value).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(value.parse::<AiExportFormat>().unwrap(), expected);
            assert_eq!(parsed.as_str(), value);
        }
    }

    #[test]
    fn ai_export_format_parse_reports_typed_format_error() {
        let error = AiExportFormat::parse("made-up-format").unwrap_err();
        assert!(matches!(
            error,
            AiAdapterError::UnsupportedExportFormat { .. }
        ));
        assert!(error
            .to_string()
            .contains("unsupported AI export format 'made-up-format'"));
    }

    #[test]
    fn webdataset_export_has_ustar_members() {
        let value = json!({
            "path": "training.coveai",
            "samples": [{"sample_id": 1, "split": "train"}],
        });
        let tar = write_webdataset(&value).unwrap();
        assert_eq!(&tar[257..263], b"ustar\0");
    }

    #[test]
    fn covm_source_uri_prefers_relative_only_for_sibling_source() {
        let root =
            std::env::temp_dir().join(format!("cove-ai-adapters-uri-{}", std::process::id()));
        let source_dir = root.join("source");
        let archive_dir = root.join("archive");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&archive_dir).unwrap();
        let sibling_source = archive_dir.join("samples.jsonl");
        let remote_source = source_dir.join("samples.jsonl");
        let sidecar = archive_dir.join("training.coveai");
        fs::write(&sibling_source, b"{}\n").unwrap();
        fs::write(&remote_source, b"{}\n").unwrap();
        assert_eq!(covm_source_uri(&sibling_source, &sidecar), "samples.jsonl");
        assert!(Path::new(&covm_source_uri(&remote_source, &sidecar)).is_absolute());
    }
}
