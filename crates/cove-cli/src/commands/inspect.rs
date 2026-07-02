fn run_doctor(file: &Path, json: bool) -> Result<(), String> {
    let bytes =
        fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    let discovery = discover_query_surfaces(
        &bytes,
        QuerySurfaceDiscoveryOptions {
            source_name: Some(file.display().to_string()),
        },
    );
    let bundle = discover_acceleration_bundle(
        &bytes,
        file,
        AccelerationBundleOptions {
            auto_discover: true,
            strict_source_digest: true,
        },
    );
    let suggestions = suggest_queries(&discovery);
    let mut findings = Vec::new();
    if discovery.queryable {
        findings.push("artifact exposes queryable rows".to_string());
    } else {
        findings.push(discovery.guidance.clone());
    }
    if bundle.has_usable_sidecars() {
        findings.push("validated acceleration sidecars are available".to_string());
    } else if discovery.queryable {
        findings.push(format!(
            "no validated acceleration bundle found; run `cove optimize {}`",
            file.display()
        ));
    }
    for diagnostic in &discovery.diagnostics {
        findings.push(format!("{}: {}", diagnostic.code, diagnostic.message));
    }
    for diagnostic in &bundle.diagnostics {
        findings.push(format!("{}: {}", diagnostic.code, diagnostic.message));
    }

    if json {
        let value = serde_json::json!({
            "file": file.display().to_string(),
            "artifact": discovery.artifact_label,
            "queryable": discovery.queryable,
            "guidance": discovery.guidance,
            "findings": findings,
            "suggested_queries": suggestions,
            "performance": acceleration_report_json(&bundle),
        });
        print_json_pretty(&value)?;
        return Ok(());
    }

    println!("Doctor: {}", file.display());
    println!("Artifact: {}", discovery.artifact_label);
    println!(
        "Queryable: {}",
        if discovery.queryable { "yes" } else { "no" }
    );
    println!("Guidance: {}", discovery.guidance);
    println!();
    println!("Findings:");
    for finding in &findings {
        println!("  - {finding}");
    }
    if !suggestions.is_empty() {
        println!();
        println!("Try next:");
        for suggestion in suggestions.iter().take(3) {
            println!("  - {}", suggestion.query);
        }
    }
    println!();
    println!("Useful commands:");
    println!("  cove inspect --queries --performance {}", file.display());
    if discovery.queryable && !bundle.has_usable_sidecars() {
        println!("  cove optimize {}", file.display());
    }
    println!("  cove query --help");
    Ok(())
}

fn run_inspect(
    file: &Path,
    queries: bool,
    json: bool,
    performance: bool,
    ai: bool,
) -> Result<(), String> {
    let bytes =
        fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    if ai {
        return run_ai_inspect(file, &bytes, json);
    }
    if delta::is_covedelta_bytes(&bytes) {
        if queries || performance {
            return Err(
                "COVEDELTA inspect does not support --queries or --performance; use `cove delta inspect`"
                    .into(),
            );
        }
        return delta::inspect_covedelta_for_beginner(file, json);
    }
    let discovery = discover_query_surfaces(
        &bytes,
        QuerySurfaceDiscoveryOptions {
            source_name: Some(file.display().to_string()),
        },
    );
    if json {
        let mut value = serde_json::to_value(&discovery)
            .map_err(|error| format!("cannot serialize discovery: {error}"))?;
        if queries {
            value["suggested_queries"] = serde_json::to_value(suggest_queries(&discovery))
                .map_err(|error| format!("cannot serialize suggested queries: {error}"))?;
        }
        if performance {
            let bundle = discover_acceleration_bundle(
                &bytes,
                file,
                AccelerationBundleOptions {
                    auto_discover: true,
                    strict_source_digest: true,
                },
            );
            value["performance"] = acceleration_report_json(&bundle);
        }
        print_json_pretty(&value)?;
        return Ok(());
    }
    print_discovery(&discovery, queries);
    if performance {
        let bundle = discover_acceleration_bundle(
            &bytes,
            file,
            AccelerationBundleOptions {
                auto_discover: true,
                strict_source_digest: true,
            },
        );
        print_performance_discovery(&bundle);
    }
    Ok(())
}

fn run_ai_inspect(file: &Path, bytes: &[u8], json: bool) -> Result<(), String> {
    if bytes.len() >= 4
        && (bytes[bytes.len() - 4..] == MAGIC_COVEAI || bytes[bytes.len() - 4..] == MAGIC_COVEV)
    {
        let sidecar = CoveAiFile::parse(bytes)
            .map_err(|error| format!("{}: invalid COVE-AI sidecar: {error}", file.display()))?;
        let explain = ai_explain_report(&sidecar);
        if json {
            let version = serde_json::json!({
                "major": sidecar.header.version_major,
                "minor": sidecar.header.version_minor,
            });
            let runtime = serde_json::json!({
                "payload_exposure_eligible": matches!(
                    explain.payload_access,
                    cove_core::artifact::coveai::AiPayloadAccessState::StructurallyAllowed
                ),
                "vector_space_count": explain.vector_space_count,
                "vector_index_count": explain.vector_index_count,
                "payload_ref_count": explain.payload_ref_count,
                "privacy_summary_count": explain.privacy_summary_count,
                "supported_indexes": explain.supported_indexes,
                "stale_or_withheld": explain.stale_or_withheld,
            });
            let records = serde_json::json!({
                "source_bindings": sidecar.descriptor_tables.source_bindings.len(),
                "privacy_summaries": sidecar.descriptor_tables.privacy_summaries.len(),
                "payload_refs": sidecar.descriptor_tables.payload_refs.len(),
                "payload_integrity": sidecar.descriptor_tables.payload_integrity.len(),
                "chunk_profiles": sidecar.descriptor_tables.chunk_profiles.len(),
                "text_chunks": sidecar.descriptor_tables.text_chunks.len(),
                "tokenizer_profiles": sidecar.descriptor_tables.tokenizer_profiles.len(),
                "vector_spaces": sidecar.descriptor_tables.vector_spaces.len(),
                "vector_payload_blocks": sidecar.descriptor_tables.vector_payload_blocks.len(),
                "vector_entries": sidecar.descriptor_tables.vector_entries.len(),
                "filecode_vector_bindings": sidecar.descriptor_tables.filecode_vector_bindings.len(),
                "chunk_vector_bindings": sidecar.descriptor_tables.chunk_vector_bindings.len(),
                "object_state_vector_bindings": sidecar.descriptor_tables.object_state_vector_bindings.len(),
                "training_sample_vector_bindings": sidecar.descriptor_tables.training_sample_vector_bindings.len(),
                "association_state_vector_bindings": sidecar.descriptor_tables.association_state_vector_bindings.len(),
                "asset_vector_bindings": sidecar.descriptor_tables.asset_vector_bindings.len(),
                "multimodal_sequence_vector_bindings": sidecar.descriptor_tables.multimodal_sequence_vector_bindings.len(),
                "vector_indexes": sidecar.descriptor_tables.vector_indexes.len(),
                "token_blocks": sidecar.descriptor_tables.token_blocks.len(),
                "tokenized_spans": sidecar.descriptor_tables.tokenized_spans.len(),
                "token_sequence_packs": sidecar.descriptor_tables.token_sequence_packs.len(),
                "training_profiles": sidecar.descriptor_tables.training_profiles.len(),
                "training_samples": sidecar.descriptor_tables.training_samples.len(),
                "dataset_splits": sidecar.descriptor_tables.dataset_splits.len(),
                "dedup_groups": sidecar.descriptor_tables.dedup_groups.len(),
                "training_epoch_plans": sidecar.descriptor_tables.training_epoch_plans.len(),
                "training_labels": sidecar.descriptor_tables.training_labels.len(),
                "preference_pairs": sidecar.descriptor_tables.preference_pairs.len(),
                "generator_provenance": sidecar.descriptor_tables.generator_provenance.len(),
                "model_actors": sidecar.descriptor_tables.model_actors.len(),
                "generation_decoding_profiles": sidecar.descriptor_tables.generation_decoding_profiles.len(),
                "human_reviews": sidecar.descriptor_tables.human_reviews.len(),
                "tensor_layouts": sidecar.descriptor_tables.tensor_layouts.len(),
                "device_transfer_hints": sidecar.descriptor_tables.device_transfer_hints.len(),
                "assets": sidecar.descriptor_tables.assets.len(),
                "multimodal_sequence_packs": sidecar.descriptor_tables.multimodal_sequence_packs.len(),
                "multimodal_sequence_elements": sidecar.descriptor_tables.multimodal_sequence_elements.len(),
            });
            let sections = sidecar
                .sections
                .iter()
                .map(|section| {
                    serde_json::json!({
                        "section_id": section.entry.section_id,
                        "section_kind": section.entry.section_kind,
                        "offset": section.entry.offset,
                        "length": section.entry.length,
                        "profile": section.entry.profile_kind,
                        "payload_encoding": section.entry.payload_encoding,
                        "records": section.record_headers.len(),
                    })
                })
                .collect::<Vec<_>>();
            let value = serde_json::json!({
                "path": file.display().to_string(),
                "artifact": match sidecar.artifact_kind {
                    CoveAiArtifactKind::CoveAiBundle => "coveai",
                    CoveAiArtifactKind::CoveVec => "covev",
                },
                "version": version,
                "artifact_id": hex_bytes(&sidecar.header.artifact_id),
                "section_count": sidecar.sections.len(),
                "payload_access": format!("{:?}", sidecar.payload_access),
                "runtime": runtime,
                "records": records,
                "sections": sections,
            });
            print_json_pretty(&value)?;
            return Ok(());
        }
        println!("AI Inspect: {}", file.display());
        println!(
            "Artifact: {}",
            match sidecar.artifact_kind {
                CoveAiArtifactKind::CoveAiBundle => ".coveai (CVA2)",
                CoveAiArtifactKind::CoveVec => ".covev (CVV2)",
            }
        );
        println!("Artifact ID: {}", hex_bytes(&sidecar.header.artifact_id));
        println!("Sections: {}", sidecar.sections.len());
        println!("Payload access: {:?}", sidecar.payload_access);
        println!(
            "Runtime: payload_exposure_eligible={} vector_spaces={} vector_indexes={} payload_refs={} privacy_summaries={}",
            matches!(
                explain.payload_access,
                cove_core::artifact::coveai::AiPayloadAccessState::StructurallyAllowed
            ),
            explain.vector_space_count,
            explain.vector_index_count,
            explain.payload_ref_count,
            explain.privacy_summary_count
        );
        if !explain.supported_indexes.is_empty() {
            println!(
                "Supported index descriptors: {}",
                explain.supported_indexes.join(", ")
            );
        }
        if !explain.stale_or_withheld.is_empty() {
            println!("Withheld/stale: {}", explain.stale_or_withheld.join(", "));
        }
        println!("Records:");
        println!(
            "  source_bindings={} privacy_summaries={} payload_refs={} payload_integrity={}",
            sidecar.descriptor_tables.source_bindings.len(),
            sidecar.descriptor_tables.privacy_summaries.len(),
            sidecar.descriptor_tables.payload_refs.len(),
            sidecar.descriptor_tables.payload_integrity.len()
        );
        println!(
            "  chunk_profiles={} text_chunks={} tokenizer_profiles={} token_blocks={} tokenized_spans={} token_sequence_packs={} training_profiles={} training_samples={} dataset_splits={} dedup_groups={} training_epoch_plans={} training_labels={} preference_pairs={} generator_provenance={} model_actors={} generation_decoding_profiles={} human_reviews={} tensor_layouts={} device_transfer_hints={} assets={} multimodal_sequence_packs={} multimodal_sequence_elements={} vector_spaces={} vector_blocks={} vector_entries={} filecode_bindings={} chunk_bindings={} object_bindings={} training_vector_bindings={} association_bindings={} asset_bindings={} multimodal_vector_bindings={} vector_indexes={}",
            sidecar.descriptor_tables.chunk_profiles.len(),
            sidecar.descriptor_tables.text_chunks.len(),
            sidecar.descriptor_tables.tokenizer_profiles.len(),
            sidecar.descriptor_tables.token_blocks.len(),
            sidecar.descriptor_tables.tokenized_spans.len(),
            sidecar.descriptor_tables.token_sequence_packs.len(),
            sidecar.descriptor_tables.training_profiles.len(),
            sidecar.descriptor_tables.training_samples.len(),
            sidecar.descriptor_tables.dataset_splits.len(),
            sidecar.descriptor_tables.dedup_groups.len(),
            sidecar.descriptor_tables.training_epoch_plans.len(),
            sidecar.descriptor_tables.training_labels.len(),
            sidecar.descriptor_tables.preference_pairs.len(),
            sidecar.descriptor_tables.generator_provenance.len(),
            sidecar.descriptor_tables.model_actors.len(),
            sidecar.descriptor_tables.generation_decoding_profiles.len(),
            sidecar.descriptor_tables.human_reviews.len(),
            sidecar.descriptor_tables.tensor_layouts.len(),
            sidecar.descriptor_tables.device_transfer_hints.len(),
            sidecar.descriptor_tables.assets.len(),
            sidecar.descriptor_tables.multimodal_sequence_packs.len(),
            sidecar.descriptor_tables.multimodal_sequence_elements.len(),
            sidecar.descriptor_tables.vector_spaces.len(),
            sidecar.descriptor_tables.vector_payload_blocks.len(),
            sidecar.descriptor_tables.vector_entries.len(),
            sidecar.descriptor_tables.filecode_vector_bindings.len(),
            sidecar.descriptor_tables.chunk_vector_bindings.len(),
            sidecar.descriptor_tables.object_state_vector_bindings.len(),
            sidecar.descriptor_tables.training_sample_vector_bindings.len(),
            sidecar.descriptor_tables.association_state_vector_bindings.len(),
            sidecar.descriptor_tables.asset_vector_bindings.len(),
            sidecar.descriptor_tables.multimodal_sequence_vector_bindings.len(),
            sidecar.descriptor_tables.vector_indexes.len()
        );
        if !sidecar.sections.is_empty() {
            println!("Sections:");
            for section in &sidecar.sections {
                println!(
                    "  - id={} kind={} offset={} len={} records={}",
                    section.entry.section_id,
                    section.entry.section_kind,
                    section.entry.offset,
                    section.entry.length,
                    section.record_headers.len()
                );
            }
        }
        return Ok(());
    }

    if bytes.len() >= 4 && bytes[bytes.len() - 4..] == MAGIC_COVEMAP {
        let map = CovemapFile::parse_validated(bytes)
            .map_err(|error| format!("{}: invalid COVE-MAP artifact: {error}", file.display()))?;
        let embedded = parse_covemap_embedded_sections(&map)?;
        let summary = map_ai_summary(&embedded);
        if json {
            let value = map_ai_summary_json(file, "covemap", &summary);
            print_json_pretty(&value)?;
            return Ok(());
        }
        print_map_ai_summary(file, "covemap", &summary);
        return Ok(());
    }

    let parsed = validate_bytes_with_options(bytes, ValidationOptions::default())
        .map_err(|error| format!("{}: invalid COVE file: {error}", file.display()))?;
    let ai_sections = parsed
        .validated
        .footer
        .sections
        .iter()
        .filter(|section| {
            SectionKind::from_u16(section.section_kind)
                .map(is_ai_section_kind_for_inspect)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let embedded_map_ai_sections = parse_cove_embedded_map_ai_sections(bytes, &parsed.validated)?;
    let summary = map_ai_summary(&embedded_map_ai_sections);
    if json {
        let value = serde_json::json!({
            "path": file.display().to_string(),
            "artifact": "cove",
            "embedded_ai_sections": ai_sections.iter().map(|section| serde_json::json!({
                "section_id": section.section_id,
                "section_kind": section.section_kind,
                "offset": section.offset,
                "length": section.length,
                "required_features": section.required_features,
                "optional_features": section.optional_features,
            })).collect::<Vec<_>>(),
            "map_ai": map_ai_summary_value(&summary),
        });
        print_json_pretty(&value)?;
        return Ok(());
    }
    println!("AI Inspect: {}", file.display());
    println!("Artifact: .cove");
    println!("Embedded AI sections: {}", ai_sections.len());
    print_map_ai_summary_details(&summary);
    for section in ai_sections {
        let section_kind = SectionKind::from_u16(section.section_kind)
            .map(|kind| format!("{kind:?}"))
            .unwrap_or_else(|| format!("unknown({})", section.section_kind));
        println!(
            "  - id={} kind={} offset={} len={}",
            section.section_id, section_kind, section.offset, section.length
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct MapAiInspectSummary {
    active_profiles: Vec<String>,
    inactive_profiles: Vec<String>,
    slot_policies: Vec<MapAiSlotInspect>,
    forbidden_slots: Vec<MapAiSlotInspect>,
    template_ids: Vec<String>,
    training_policy_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct MapAiSlotInspect {
    slot_policy_id: String,
    path: String,
    role: String,
    decision: String,
    granularity: String,
    sensitivity: String,
    source_id: Option<String>,
    source_column: Option<String>,
    object_type: Option<String>,
    property_id: Option<String>,
    association_type: Option<String>,
    template_id: Option<String>,
    chunk_profile_id: Option<String>,
    tokenizer_profile_id: Option<String>,
    training_policy_id: Option<String>,
}

fn parse_covemap_embedded_sections(map: &CovemapFile) -> Result<Vec<EmbeddedMapSection>, String> {
    let mut out = Vec::new();
    for section in &map.sections {
        let kind = u16::try_from(section.entry.section_id)
            .ok()
            .and_then(SectionKind::from_u16)
            .ok_or_else(|| format!("unknown COVE-MAP section {}", section.entry.section_id))?;
        if !is_map_ai_section_kind(kind) {
            continue;
        }
        out.push(
            parse_embedded_section(kind, &section.payload)
                .map_err(|error| format!("invalid {kind:?} payload: {error}"))?,
        );
    }
    Ok(out)
}

fn parse_cove_embedded_map_ai_sections(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<Vec<EmbeddedMapSection>, String> {
    let mut out = Vec::new();
    for entry in &validated.footer.sections {
        let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
            continue;
        };
        if !is_map_ai_section_kind(kind) {
            continue;
        }
        let payload = compression::section_payload(bytes, entry)
            .map_err(|error| format!("cannot decode embedded {kind:?}: {error}"))?;
        out.push(
            parse_embedded_section(kind, &payload)
                .map_err(|error| format!("invalid embedded {kind:?}: {error}"))?,
        );
    }
    Ok(out)
}

fn map_ai_summary(sections: &[EmbeddedMapSection]) -> MapAiInspectSummary {
    let mut summary = MapAiInspectSummary::default();
    for section in sections {
        match section {
            EmbeddedMapSection::AiProfileCatalog(catalog) => {
                for profile in &catalog.profiles {
                    if profile.active {
                        summary.active_profiles.push(profile.profile_id.clone());
                    } else {
                        summary.inactive_profiles.push(profile.profile_id.clone());
                    }
                }
                for slot in &catalog.slot_policies {
                    let inspect = MapAiSlotInspect {
                        slot_policy_id: slot.slot_policy_id.clone(),
                        path: slot.path.clone(),
                        role: slot.role.clone(),
                        decision: slot.decision.clone(),
                        granularity: slot.granularity.clone(),
                        sensitivity: slot.sensitivity.clone(),
                        source_id: slot.source_id.clone(),
                        source_column: slot.source_column.clone(),
                        object_type: slot.object_type.clone(),
                        property_id: slot.property_id.clone(),
                        association_type: slot.association_type.clone(),
                        template_id: slot.template_id.clone(),
                        chunk_profile_id: slot.chunk_profile_id.clone(),
                        tokenizer_profile_id: slot.tokenizer_profile_id.clone(),
                        training_policy_id: slot.training_policy_id.clone(),
                    };
                    if inspect.decision == "Forbidden" || inspect.sensitivity == "Forbidden" {
                        summary.forbidden_slots.push(inspect.clone());
                    }
                    summary.slot_policies.push(inspect);
                }
            }
            EmbeddedMapSection::AiTemplateCatalog(catalog) => {
                summary.template_ids.extend(
                    catalog
                        .templates
                        .iter()
                        .map(|template| template.template_id.clone()),
                );
            }
            EmbeddedMapSection::AiTrainingPolicyCatalog(catalog) => {
                summary.training_policy_ids.extend(
                    catalog
                        .training_policies
                        .iter()
                        .map(|policy| policy.training_policy_id.clone()),
                );
            }
            _ => {}
        }
    }
    summary.active_profiles.sort();
    summary.inactive_profiles.sort();
    summary.template_ids.sort();
    summary.training_policy_ids.sort();
    summary
}

fn map_ai_summary_json(
    file: &Path,
    artifact: &str,
    summary: &MapAiInspectSummary,
) -> serde_json::Value {
    serde_json::json!({
        "path": file.display().to_string(),
        "artifact": artifact,
        "map_ai": map_ai_summary_value(summary),
    })
}

fn map_ai_summary_value(summary: &MapAiInspectSummary) -> serde_json::Value {
    serde_json::json!({
        "active_profiles": summary.active_profiles,
        "inactive_profiles": summary.inactive_profiles,
        "slot_policy_count": summary.slot_policies.len(),
        "template_count": summary.template_ids.len(),
        "training_policy_count": summary.training_policy_ids.len(),
        "forbidden_slot_count": summary.forbidden_slots.len(),
        "templates": summary.template_ids,
        "training_policies": summary.training_policy_ids,
        "slot_policies": summary.slot_policies.iter().map(map_ai_slot_json).collect::<Vec<_>>(),
        "forbidden_slots": summary.forbidden_slots.iter().map(map_ai_slot_json).collect::<Vec<_>>(),
    })
}

fn map_ai_slot_json(slot: &MapAiSlotInspect) -> serde_json::Value {
    serde_json::json!({
        "slot_policy_id": slot.slot_policy_id,
        "path": slot.path,
        "role": slot.role,
        "decision": slot.decision,
        "granularity": slot.granularity,
        "sensitivity": slot.sensitivity,
        "source_id": slot.source_id,
        "source_column": slot.source_column,
        "object_type": slot.object_type,
        "property_id": slot.property_id,
        "association_type": slot.association_type,
        "template_id": slot.template_id,
        "chunk_profile_id": slot.chunk_profile_id,
        "tokenizer_profile_id": slot.tokenizer_profile_id,
        "training_policy_id": slot.training_policy_id,
    })
}

fn print_map_ai_summary(file: &Path, artifact: &str, summary: &MapAiInspectSummary) {
    println!("AI Inspect: {}", file.display());
    println!("Artifact: .{artifact}");
    print_map_ai_summary_details(summary);
}

fn print_map_ai_summary_details(summary: &MapAiInspectSummary) {
    println!("COVE-MAP-AI:");
    println!("  active_profiles: {}", summary.active_profiles.len());
    for profile in &summary.active_profiles {
        println!("    - {profile}");
    }
    println!("  slot_policies: {}", summary.slot_policies.len());
    println!("  templates: {}", summary.template_ids.len());
    println!("  training_policies: {}", summary.training_policy_ids.len());
    println!("  forbidden_slots: {}", summary.forbidden_slots.len());
    for slot in &summary.forbidden_slots {
        println!(
            "    - {} decision={} sensitivity={}",
            slot.path, slot.decision, slot.sensitivity
        );
    }
    for slot in &summary.slot_policies {
        println!(
            "    slot {} path={} role={} decision={} sensitivity={}",
            slot.slot_policy_id, slot.path, slot.role, slot.decision, slot.sensitivity
        );
    }
}

fn is_map_ai_section_kind(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::MapAiProfileCatalog
            | SectionKind::MapAiTemplateCatalog
            | SectionKind::MapAiTrainingPolicyCatalog
    )
}

fn is_ai_section_kind_for_inspect(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::MapAiProfileCatalog
            | SectionKind::MapAiTemplateCatalog
            | SectionKind::MapAiTrainingPolicyCatalog
            | SectionKind::AiCompanionArtifactRef
            | SectionKind::AiSourceBinding
            | SectionKind::AiChunkProfile
            | SectionKind::AiTextChunkIndex
            | SectionKind::AiTokenizerProfile
            | SectionKind::AiTokenBlock
            | SectionKind::AiTokenizedSpan
            | SectionKind::AiTokenSequencePack
            | SectionKind::AiVectorSpace
            | SectionKind::AiVectorBinding
            | SectionKind::AiVectorPayloadBlock
            | SectionKind::AiVectorComposition
            | SectionKind::AiVectorIndex
            | SectionKind::AiTensorLayout
            | SectionKind::AiAssetManifest
            | SectionKind::AiMultimodalSequence
            | SectionKind::AiTrainingProfile
            | SectionKind::AiTrainingSampleIndex
            | SectionKind::AiTrainingSplitDedupEpoch
            | SectionKind::AiLabelPreference
            | SectionKind::AiGeneratorProvenance
            | SectionKind::AiReferenceTables
            | SectionKind::AiPayloadIntegrity
            | SectionKind::AiPrivacySummary
            | SectionKind::AiSectionFeatureBinding
            | SectionKind::AiVectorDirectory
            | SectionKind::AiPayloadBytes
    )
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

