use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use cove_core::{
    artifact::{
        covedelta::{
            CoveDeltaFile, CoveDeltaSectionKind, DELTA_FEATURE_MAP_EVIDENCE_PATCH,
            DELTA_FEATURE_PROJECTION_PATCH,
        },
        covm::{
            validate_selected_delta_chain_with_base, CovmDeltaArtifactRefV1,
            CovmDeltaChainExtensionV1, CovmDeltaChainSummaryV1, CovmDeltaPruneDecision,
            CovmDeltaPruneReason, CovmDeltaPruneRequest, CovmDeltaReadAmplificationMetrics,
            CovmDeltaReadAmplificationRecommendation, CovmFile, COVM_DELTA_ARTIFACT_REF_LEN,
            COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN, COVM_DELTA_CHAIN_SUMMARY_KIND_NONE,
            COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED, COVM_POSTSCRIPT_LEN,
            COVM_POSTSCRIPT_TAIL_SIZE,
        },
    },
    profile::cove_o::{
        read_object_surface_from_base_and_delta_files_with_parent_identity,
        read_object_surface_from_bytes_with_options, reconstruct_object_states,
        CoveObjectDeltaParentIdentity, CoveObjectReadOptions, CoveObjectReconstructionOptions,
        CoveObjectRedactionReadPolicy, CoveObjectState, CoveObjectSurface, CoveObjectTemporalCut,
        ObjectTypeEntryV1,
    },
    utility::hex_encode,
    wire,
};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct ManifestDeltaContext {
    pub manifest: CovmFile,
    pub extension: Option<CovmDeltaChainExtensionV1>,
    pub inline_summary_bytes: Option<Vec<u8>>,
    pub extension_source: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArtifactBytes {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ValidatedDeltaSnapshot {
    pub extension: CovmDeltaChainExtensionV1,
    pub base: ArtifactBytes,
    pub deltas: Vec<ArtifactBytes>,
    pub parsed_deltas: Vec<CoveDeltaFile>,
    pub summary_validated: bool,
    pub plan: DeltaSnapshotPlan,
}

#[derive(Debug, Clone)]
pub struct DeltaSnapshotPlan {
    pub decision: CovmDeltaPruneDecision,
    pub metrics: CovmDeltaReadAmplificationMetrics,
    pub recommendations: Vec<CovmDeltaReadAmplificationRecommendation>,
    pub temporal_cut: CoveObjectTemporalCut,
    pub summary_validated: bool,
}

#[derive(Debug, Clone)]
pub struct MaterializedSnapshot {
    pub bytes: Vec<u8>,
    pub state_count: Option<usize>,
    pub passthrough_base: bool,
}

#[derive(Debug, Clone)]
pub struct ReconstructedObjectSnapshot {
    pub object_types: Vec<ObjectTypeEntryV1>,
    pub states: Vec<CoveObjectState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectDeltaObjectSurfaceSupport {
    Supported,
    RequiresMaterializedPlannerMetadata { reason: String },
}

fn cove_object_delta_parent_identity(
    reference: &CovmDeltaArtifactRefV1,
) -> CoveObjectDeltaParentIdentity {
    CoveObjectDeltaParentIdentity {
        artifact_id: reference.artifact_id,
        snapshot_id: reference.snapshot_id,
        file_len: reference.file_len,
        footer_crc32c: reference.footer_crc32c,
    }
}

pub fn delta_chain_required(bytes: &[u8]) -> Result<bool, String> {
    let manifest = CovmFile::parse_delta_aware(bytes)
        .map_err(|error| format!("not a valid COVM manifest: {error}"))?;
    Ok(manifest.postscript.flags & COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED != 0)
}

pub fn load_manifest_delta_context(
    manifest_path: &Path,
    extension_override: Option<&Path>,
) -> Result<ManifestDeltaContext, String> {
    let manifest_bytes = read_file(manifest_path)?;
    let manifest = CovmFile::parse_delta_aware(&manifest_bytes).map_err(|error| {
        format!(
            "{} is not a valid COVM manifest: {error}",
            manifest_path.display()
        )
    })?;
    let (extension, inline_summary_bytes, extension_source) = if let Some(path) = extension_override
    {
        let bytes = read_file(path)?;
        let extension = CovmDeltaChainExtensionV1::parse(&bytes).map_err(|error| {
            format!(
                "{} is not a valid delta-chain extension: {error}",
                path.display()
            )
        })?;
        (Some(extension), None, Some(path.display().to_string()))
    } else {
        let region = manifest_extension_region(&manifest_bytes, &manifest)?;
        if region.is_empty() {
            (None, None, None)
        } else {
            let extension_len = covm_delta_extension_encoded_len(region)?;
            if extension_len > region.len() {
                return Err(
                    "COVM delta-chain extension extends past manifest extension region".into(),
                );
            }
            let extension = CovmDeltaChainExtensionV1::parse(&region[..extension_len])
                .map_err(|error| format!("manifest delta-chain extension is invalid: {error}"))?;
            let summary = if extension_len < region.len() {
                Some(region[extension_len..].to_vec())
            } else {
                None
            };
            (Some(extension), summary, Some("manifest".into()))
        }
    };
    Ok(ManifestDeltaContext {
        manifest,
        extension,
        inline_summary_bytes,
        extension_source,
    })
}

pub fn load_validated_delta_snapshot(
    manifest: &Path,
    dataset: &Path,
    request: CovmDeltaPruneRequest,
) -> Result<ValidatedDeltaSnapshot, String> {
    let context = load_manifest_delta_context(manifest, None)?;
    load_validated_delta_snapshot_from_context(&context, dataset, request)
}

pub fn load_validated_delta_snapshot_from_context(
    context: &ManifestDeltaContext,
    dataset: &Path,
    request: CovmDeltaPruneRequest,
) -> Result<ValidatedDeltaSnapshot, String> {
    let extension = context
        .extension
        .as_ref()
        .ok_or_else(|| "manifest does not contain a delta-chain extension".to_string())?
        .clone();
    let (base, deltas) = load_selected_artifacts(context, &extension, dataset)?;
    let summary = context
        .inline_summary_bytes
        .as_deref()
        .map(CovmDeltaChainSummaryV1::parse)
        .transpose()
        .map_err(|error| format!("invalid delta chain summary: {error}"))?;
    if extension.chain_summary_kind != COVM_DELTA_CHAIN_SUMMARY_KIND_NONE && summary.is_none() {
        return Err("delta chain requires an inline summary to plan snapshot reads".into());
    }
    if let Some(summary) = &summary {
        summary
            .validate_against_delta_chain_extension(&extension)
            .map_err(|error| format!("delta chain summary is stale: {error}"))?;
    }
    let delta_slices = deltas
        .iter()
        .map(|artifact| artifact.bytes.as_slice())
        .collect::<Vec<_>>();
    let parsed_deltas = validate_selected_delta_chain_with_base(
        &extension,
        summary.as_ref(),
        Some(base.bytes.as_slice()),
        &delta_slices,
    )
    .map_err(|error| format!("selected delta chain is invalid: {error}"))?;
    let plan = plan_delta_snapshot(&extension, summary.as_ref(), request)?;
    Ok(ValidatedDeltaSnapshot {
        extension,
        base,
        deltas,
        parsed_deltas,
        summary_validated: summary.is_some(),
        plan,
    })
}

pub fn plan_delta_snapshot(
    extension: &CovmDeltaChainExtensionV1,
    summary: Option<&CovmDeltaChainSummaryV1>,
    request: CovmDeltaPruneRequest,
) -> Result<DeltaSnapshotPlan, String> {
    let decision = match summary {
        Some(summary) => summary
            .prune_delta_chain(request)
            .map_err(|error| format!("cannot plan delta chain: {error}"))?,
        None if request == CovmDeltaPruneRequest::default() => CovmDeltaPruneDecision {
            selected_chain_ordinals: extension
                .ordered_delta_artifact_refs
                .iter()
                .map(|reference| reference.chain_ordinal)
                .collect(),
            skipped: Vec::new(),
        },
        None => {
            return Err("delta snapshot pruning requires an inline delta chain summary".to_string())
        }
    };
    let mut metrics = summary
        .map(|summary| summary.read_amplification_metrics(&decision))
        .unwrap_or_else(|| {
            let mut metrics = CovmDeltaReadAmplificationMetrics::from_prune_decision(&decision);
            metrics.base_ranges_requested = 1;
            metrics.delta_ranges_requested = metrics.selected_delta_count;
            metrics.object_store_request_count =
                1usize.saturating_add(metrics.selected_delta_count);
            metrics
        });
    metrics.base_file_bytes = extension.base_artifact_ref.file_len;
    metrics.total_delta_bytes = extension
        .ordered_delta_artifact_refs
        .iter()
        .map(|reference| reference.file_len)
        .sum();
    metrics.bytes_returned = metrics.base_file_bytes.saturating_add(selected_delta_bytes(
        extension,
        &decision.selected_chain_ordinals,
    ));
    let temporal_cut = temporal_cut_for_request(request)?;
    let recommendations = metrics.recommendations(Default::default());
    Ok(DeltaSnapshotPlan {
        decision,
        metrics,
        recommendations,
        temporal_cut,
        summary_validated: summary.is_some(),
    })
}

pub fn materialize_validated_delta_snapshot(
    snapshot: &ValidatedDeltaSnapshot,
) -> Result<MaterializedSnapshot, String> {
    let selected_prefix_len = selected_prefix_len(
        &snapshot.plan.decision.selected_chain_ordinals,
        snapshot.parsed_deltas.len(),
    )?;
    let selected_deltas = &snapshot.parsed_deltas[..selected_prefix_len];
    if snapshot.plan.temporal_cut == CoveObjectTemporalCut::LatestCommitted
        && selected_deltas
            .iter()
            .all(|delta| delta.sections.is_empty())
    {
        return Ok(MaterializedSnapshot {
            bytes: snapshot.base.bytes.clone(),
            state_count: None,
            passthrough_base: true,
        });
    }

    let ReconstructedObjectSnapshot {
        object_types,
        states,
    } = reconstruct_validated_object_snapshot(snapshot)?;
    let state_count = states.len();
    let bytes = cove_map::compact_cove_o_from_object_states(object_types, &states)
        .map_err(|error| format!("cannot write reconstructed COVE-O snapshot: {error}"))?;
    Ok(MaterializedSnapshot {
        bytes,
        state_count: Some(state_count),
        passthrough_base: false,
    })
}

pub fn direct_object_surface_support(
    snapshot: &ValidatedDeltaSnapshot,
) -> DirectDeltaObjectSurfaceSupport {
    let selected_prefix_len = match selected_prefix_len(
        &snapshot.plan.decision.selected_chain_ordinals,
        snapshot.parsed_deltas.len(),
    ) {
        Ok(len) => len,
        Err(error) => {
            return DirectDeltaObjectSurfaceSupport::RequiresMaterializedPlannerMetadata {
                reason: error,
            }
        }
    };
    for delta in &snapshot.parsed_deltas[..selected_prefix_len] {
        let feature_bits =
            delta.header.required_delta_features | delta.header.optional_delta_features;
        if feature_bits & (DELTA_FEATURE_MAP_EVIDENCE_PATCH | DELTA_FEATURE_PROJECTION_PATCH) != 0 {
            return DirectDeltaObjectSurfaceSupport::RequiresMaterializedPlannerMetadata {
                reason:
                    "selected deltas carry evidence/projection metadata patches; planner metadata must come from a materialized snapshot"
                        .into(),
            };
        }
        if delta.sections.iter().any(|section| {
            matches!(
                CoveDeltaSectionKind::from_u16(section.entry.section_kind),
                Some(
                    CoveDeltaSectionKind::CatalogPatch
                        | CoveDeltaSectionKind::EvidencePatch
                        | CoveDeltaSectionKind::ProjectionPatch
                )
            )
        }) {
            return DirectDeltaObjectSurfaceSupport::RequiresMaterializedPlannerMetadata {
                reason:
                    "selected deltas carry catalog/evidence/projection patches; planner metadata must come from a materialized snapshot"
                        .into(),
            };
        }
    }
    DirectDeltaObjectSurfaceSupport::Supported
}

pub fn read_validated_delta_object_surface(
    snapshot: &ValidatedDeltaSnapshot,
) -> Result<CoveObjectSurface, String> {
    let selected_prefix_len = selected_prefix_len(
        &snapshot.plan.decision.selected_chain_ordinals,
        snapshot.parsed_deltas.len(),
    )?;
    let selected_deltas = &snapshot.parsed_deltas[..selected_prefix_len];
    let read_options = CoveObjectReadOptions {
        include_records: true,
        include_association_object_types: false,
        include_projection_catalog: true,
        include_function_registry: true,
        include_evidence_index: true,
        redaction_read_policy: CoveObjectRedactionReadPolicy::PreserveMarker,
        ..CoveObjectReadOptions::default()
    };
    if selected_deltas
        .iter()
        .all(|delta| delta.sections.is_empty())
    {
        return read_object_surface_from_bytes_with_options(&snapshot.base.bytes, &read_options)
            .map_err(|error| format!("cannot read selected COVE-O base object surface: {error}"));
    }
    read_object_surface_from_base_and_delta_files_with_parent_identity(
        &snapshot.base.bytes,
        Some(&cove_object_delta_parent_identity(
            &snapshot.extension.base_artifact_ref,
        )),
        selected_deltas,
        &read_options,
    )
    .map_err(|error| format!("cannot read selected COVE-O delta object surface: {error}"))
}

pub fn reconstruct_validated_object_snapshot(
    snapshot: &ValidatedDeltaSnapshot,
) -> Result<ReconstructedObjectSnapshot, String> {
    let selected_prefix_len = selected_prefix_len(
        &snapshot.plan.decision.selected_chain_ordinals,
        snapshot.parsed_deltas.len(),
    )?;
    let selected_deltas = &snapshot.parsed_deltas[..selected_prefix_len];
    let read_options = CoveObjectReadOptions::default();
    let surface = if selected_deltas
        .iter()
        .all(|delta| delta.sections.is_empty())
    {
        read_object_surface_from_bytes_with_options(&snapshot.base.bytes, &read_options)
            .map_err(|error| format!("cannot read selected COVE-O base snapshot: {error}"))?
    } else {
        read_object_surface_from_base_and_delta_files_with_parent_identity(
            &snapshot.base.bytes,
            Some(&cove_object_delta_parent_identity(
                &snapshot.extension.base_artifact_ref,
            )),
            selected_deltas,
            &read_options,
        )
        .map_err(|error| format!("cannot read selected COVE-O delta snapshot: {error}"))?
    };
    let states = reconstruct_object_states(
        &surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: snapshot.plan.temporal_cut,
            ..CoveObjectReconstructionOptions::default()
        },
    )
    .map_err(|error| format!("cannot reconstruct selected COVE-O object states: {error}"))?;
    Ok(ReconstructedObjectSnapshot {
        object_types: surface.object_types,
        states,
    })
}

pub fn materialize_delta_snapshot(
    manifest: &Path,
    dataset: &Path,
    request: CovmDeltaPruneRequest,
) -> Result<(ValidatedDeltaSnapshot, MaterializedSnapshot), String> {
    let snapshot = load_validated_delta_snapshot(manifest, dataset, request)?;
    let materialized = materialize_validated_delta_snapshot(&snapshot)?;
    Ok((snapshot, materialized))
}

pub fn delta_snapshot_plan_json(
    manifest: Option<&Path>,
    plan: &DeltaSnapshotPlan,
    extension: &CovmDeltaChainExtensionV1,
) -> Value {
    json!({
        "manifest": manifest.map(|path| path.display().to_string()),
        "selected_chain_ordinals": plan.decision.selected_chain_ordinals,
        "skipped": plan.decision.skipped.iter().map(|skip| json!({
            "chain_ordinal": skip.chain_ordinal,
            "reason": prune_reason_name(skip.reason),
        })).collect::<Vec<_>>(),
        "metrics": read_amplification_json(plan.metrics),
        "recommendations": plan.recommendations.iter().map(|item| recommendation_name(*item)).collect::<Vec<_>>(),
        "summary_validated": plan.summary_validated,
        "base_snapshot_id": hex16(&extension.base_snapshot_id),
        "result_snapshot_id": hex16(&extension.result_snapshot_id),
    })
}

pub fn print_delta_snapshot_plan_text(manifest: Option<&Path>, plan: &DeltaSnapshotPlan) {
    if let Some(manifest) = manifest {
        println!("Delta snapshot plan: {}", manifest.display());
    } else {
        println!("Delta snapshot plan:");
    }
    println!("  selected: {:?}", plan.decision.selected_chain_ordinals);
    if plan.decision.skipped.is_empty() {
        println!("  skipped: none");
    } else {
        println!("  skipped:");
        for skip in &plan.decision.skipped {
            println!(
                "    - {} ({})",
                skip.chain_ordinal,
                prune_reason_name(skip.reason)
            );
        }
    }
    println!("  chain_depth: {}", plan.metrics.delta_chain_depth);
    println!(
        "  selected_delta_count: {}",
        plan.metrics.selected_delta_count
    );
    println!(
        "  skipped_delta_count: {}",
        plan.metrics.skipped_delta_count
    );
    println!(
        "  object_store_request_count: {}",
        plan.metrics.object_store_request_count
    );
    println!("  bytes_returned: {}", plan.metrics.bytes_returned);
    if plan.recommendations.is_empty() {
        println!("  recommendations: none");
    } else {
        println!("  recommendations:");
        for item in &plan.recommendations {
            println!("    - {}", recommendation_name(*item));
        }
    }
}

pub fn read_amplification_json(metrics: CovmDeltaReadAmplificationMetrics) -> Value {
    json!({
        "delta_chain_depth": metrics.delta_chain_depth,
        "chain_summary_bytes": metrics.chain_summary_bytes,
        "chain_summary_range_requests": metrics.chain_summary_range_requests,
        "selected_delta_count": metrics.selected_delta_count,
        "skipped_delta_count": metrics.skipped_delta_count,
        "delta_artifacts_opened": metrics.delta_artifacts_opened,
        "delta_artifacts_skipped_before_open": metrics.delta_artifacts_skipped_before_open,
        "base_ranges_requested": metrics.base_ranges_requested,
        "delta_ranges_requested": metrics.delta_ranges_requested,
        "object_store_request_count": metrics.object_store_request_count,
        "bytes_returned": metrics.bytes_returned,
        "base_file_bytes": metrics.base_file_bytes,
        "total_delta_bytes": metrics.total_delta_bytes,
        "source_publish_range_prunes": metrics.source_publish_range_prunes,
        "commit_time_range_prunes": metrics.commit_time_range_prunes,
    })
}

pub fn prune_reason_name(reason: CovmDeltaPruneReason) -> &'static str {
    match reason {
        CovmDeltaPruneReason::AsOfCsnBeforeDelta => "as_of_csn_before_delta",
        CovmDeltaPruneReason::AsOfCommitBeforeDelta => "as_of_commit_before_delta",
        CovmDeltaPruneReason::SourcePublishRangeOutsideDelta => {
            "source_publish_range_outside_delta"
        }
    }
}

pub fn recommendation_name(
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

fn temporal_cut_for_request(
    request: CovmDeltaPruneRequest,
) -> Result<CoveObjectTemporalCut, String> {
    if request.as_of_valid_time_us.is_some() {
        return Err(
            "delta snapshot materialization does not yet support --as-of-valid-time-us".into(),
        );
    }
    match (request.as_of_csn, request.as_of_commit_timestamp_us) {
        (Some(_), Some(_)) => Err(
            "delta snapshot materialization accepts only one of --as-of-csn or --as-of-commit-us"
                .into(),
        ),
        (Some(csn), None) => Ok(CoveObjectTemporalCut::Csn(csn)),
        (None, Some(timestamp_us)) => Ok(CoveObjectTemporalCut::TimestampUs(timestamp_us)),
        (None, None) => Ok(CoveObjectTemporalCut::LatestCommitted),
    }
}

fn selected_prefix_len(selected: &[u32], total: usize) -> Result<usize, String> {
    for (index, ordinal) in selected.iter().enumerate() {
        let expected =
            u32::try_from(index + 1).map_err(|_| "selected delta ordinal overflows".to_string())?;
        if *ordinal != expected {
            return Err(
                "delta snapshot materialization requires selected deltas to form a dense prefix"
                    .into(),
            );
        }
    }
    if selected.len() > total {
        return Err("delta snapshot plan selected more deltas than were validated".into());
    }
    Ok(selected.len())
}

fn load_selected_artifacts(
    context: &ManifestDeltaContext,
    extension: &CovmDeltaChainExtensionV1,
    dataset: &Path,
) -> Result<(ArtifactBytes, Vec<ArtifactBytes>), String> {
    let paths = artifact_uri_map(&context.manifest);
    let base = read_artifact_for_ref(dataset, &paths, &extension.base_artifact_ref, "base")?;
    let deltas = extension
        .ordered_delta_artifact_refs
        .iter()
        .map(|reference| read_artifact_for_ref(dataset, &paths, reference, "delta"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((base, deltas))
}

fn artifact_uri_map(manifest: &CovmFile) -> BTreeMap<[u8; 16], String> {
    manifest
        .files
        .iter()
        .map(|entry| (entry.file_id, entry.uri.clone()))
        .collect()
}

fn read_artifact_for_ref(
    dataset: &Path,
    paths: &BTreeMap<[u8; 16], String>,
    reference: &CovmDeltaArtifactRefV1,
    label: &str,
) -> Result<ArtifactBytes, String> {
    let uri = paths.get(&reference.artifact_id).ok_or_else(|| {
        format!(
            "manifest does not contain {label} artifact URI for {}",
            hex16(&reference.artifact_id)
        )
    })?;
    let path = dataset.join(uri);
    let bytes = read_file(&path)?;
    Ok(ArtifactBytes { path, bytes })
}

fn manifest_extension_region<'a>(
    manifest_bytes: &'a [u8],
    manifest: &CovmFile,
) -> Result<&'a [u8], String> {
    let entries_start = usize::try_from(manifest.postscript.entries_offset)
        .map_err(|_| "COVM entries offset out of range".to_string())?;
    let entries_len = usize::try_from(manifest.postscript.entries_len)
        .map_err(|_| "COVM entries length out of range".to_string())?;
    let entries_end = entries_start
        .checked_add(entries_len)
        .ok_or_else(|| "COVM entries range overflows".to_string())?;
    let postscript_total = COVM_POSTSCRIPT_LEN as usize + COVM_POSTSCRIPT_TAIL_SIZE;
    if manifest_bytes.len() < postscript_total {
        return Err("COVM file too short for postscript".into());
    }
    let postscript_start = manifest_bytes.len() - postscript_total;
    if entries_end > postscript_start {
        return Err("COVM entries overlap postscript".into());
    }
    Ok(&manifest_bytes[entries_end..postscript_start])
}

fn covm_delta_extension_encoded_len(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN {
        return Err("COVM delta-chain extension header is truncated".into());
    }
    let ordered_delta_count = read_delta_chain_u32(bytes, 184)? as usize;
    let chain_digest_len = read_delta_chain_u16(bytes, 206)? as usize;
    let chain_summary_digest_len = read_delta_chain_u16(bytes, 240)? as usize;
    COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN
        .checked_add(
            ordered_delta_count
                .checked_mul(COVM_DELTA_ARTIFACT_REF_LEN)
                .ok_or_else(|| "COVM delta artifact ref length overflows".to_string())?,
        )
        .and_then(|len| len.checked_add(chain_digest_len))
        .and_then(|len| len.checked_add(chain_summary_digest_len))
        .ok_or_else(|| "COVM delta-chain extension length overflows".to_string())
}

fn read_delta_chain_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    wire::read_u16_le_checked(bytes, offset)
        .map_err(|_| "COVM delta-chain extension header is truncated".to_string())
}

fn read_delta_chain_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    wire::read_u32_le_checked(bytes, offset)
        .map_err(|_| "COVM delta-chain extension header is truncated".to_string())
}

fn selected_delta_bytes(extension: &CovmDeltaChainExtensionV1, selected: &[u32]) -> u64 {
    let selected = selected
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    extension
        .ordered_delta_artifact_refs
        .iter()
        .filter(|reference| selected.contains(&reference.chain_ordinal))
        .map(|reference| reference.file_len)
        .sum()
}

fn hex16(bytes: &[u8; 16]) -> String {
    hex_encode(bytes)
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}
