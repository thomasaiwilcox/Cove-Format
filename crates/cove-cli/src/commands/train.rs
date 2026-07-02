fn run_train(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!(
            "missing train subcommand\n\n{}",
            usage(HelpTopic::Train)
        ));
    }
    let subcommand = args.remove(0);
    match subcommand.as_str() {
        "export" => run_train_export(args),
        "-h" | "--help" => {
            print_usage(HelpTopic::Train);
            Ok(())
        }
        other => Err(format!(
            "unknown train subcommand '{other}'\n\n{}",
            usage(HelpTopic::Train)
        )),
    }
}

fn run_train_export(args: Vec<String>) -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut format = "json".to_string();
    let mut profile_filter: Option<u32> = None;
    let mut split_filter: Option<u32> = None;
    let mut epoch_plan_filter: Option<u64> = None;
    let mut include_payloads = false;
    let mut policy_report = false;
    let mut strict_training = false;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--include-payloads" => include_payloads = true,
            "--policy-report" => policy_report = true,
            "--strict-training" => strict_training = true,
            "--format" => {
                format = iter
                    .next()
                    .ok_or_else(|| "--format requires a value".to_string())?;
                if !matches!(
                    format.as_str(),
                    "json" | "jsonl" | "hf-jsonl" | "arrow" | "parquet" | "webdataset"
                ) {
                    return Err(
                        "--format must be json, jsonl, hf-jsonl, arrow, parquet, or webdataset"
                            .into(),
                    );
                }
            }
            "--out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out requires a path".to_string())?;
                out = Some(PathBuf::from(value));
            }
            "--profile" => {
                profile_filter = Some(parse_u32_arg(iter.next().as_deref(), "--profile")?);
            }
            "--split" => {
                split_filter = Some(parse_u32_arg(iter.next().as_deref(), "--split")?);
            }
            "--epoch-plan" => {
                epoch_plan_filter = Some(parse_u64(iter.next().as_deref(), "--epoch-plan")?);
            }
            "-h" | "--help" => {
                print_usage(HelpTopic::Train);
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown train export argument '{value}'"));
            }
            value => {
                if input.replace(PathBuf::from(value)).is_some() {
                    return Err("train export accepts exactly one input sidecar".into());
                }
            }
        }
    }

    let input = input
        .ok_or_else(|| "train export requires <training.coveai|training.covev>".to_string())?;
    let bytes =
        fs::read(&input).map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    if bytes.len() < 4
        || (bytes[bytes.len() - 4..] != MAGIC_COVEAI && bytes[bytes.len() - 4..] != MAGIC_COVEV)
    {
        return Err(format!(
            "{} is not a COVE-AI companion artifact (.coveai/.covev)",
            input.display()
        ));
    }
    let sidecar = CoveAiFile::parse(&bytes)
        .map_err(|error| format!("{}: invalid COVE-AI sidecar: {error}", input.display()))?;
    if strict_training {
        open_ai_archive(&input, AiArchiveOpenOptions::default())
            .map_err(|err| err.to_string())?
            .verify(AiVerifyOptions {
                policy_report: true,
                strict_training: true,
            })
            .map_err(|err| err.to_string())?;
    }
    let payload_reader = AiPayloadReader::new(
        &bytes,
        &sidecar,
        if include_payloads {
            CoveAiAccessContext::for_operation("train_export")
        } else {
            CoveAiAccessContext::descriptor_only("train_export")
        },
    );

    if matches!(format.as_str(), "jsonl" | "hf-jsonl") {
        let text = training_export_jsonl(
            &sidecar,
            profile_filter,
            split_filter,
            include_payloads,
            &payload_reader,
        )?;
        if let Some(out) = out {
            fs::write(&out, text)
                .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
        } else {
            print!("{text}");
        }
    } else {
        let value = training_export_json(
            &input,
            &sidecar,
            profile_filter,
            split_filter,
            epoch_plan_filter,
            include_payloads,
            policy_report,
            &payload_reader,
            &format,
        );
        write_ai_export_output(&value, &format, out)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn training_export_json(
    input: &Path,
    sidecar: &CoveAiFile,
    profile_filter: Option<u32>,
    split_filter: Option<u32>,
    epoch_plan_filter: Option<u64>,
    include_payloads: bool,
    policy_report: bool,
    payload_reader: &AiPayloadReader<'_>,
    format: &str,
) -> serde_json::Value {
    let samples = filtered_training_samples(sidecar, profile_filter, split_filter)
        .into_iter()
        .map(|sample| training_sample_json_with_payloads(sample, include_payloads, payload_reader))
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    if !matches!(
        sidecar.payload_access,
        cove_core::artifact::coveai::AiPayloadAccessState::StructurallyAllowed
    ) {
        diagnostics.push(serde_json::json!({
            "code": "COVE_AI_PAYLOAD_POLICY_BLOCKED",
            "message": "direct AI payload access is blocked until privacy summaries and policy scopes validate",
            "payload_access": format!("{:?}", sidecar.payload_access),
        }));
    }
    serde_json::json!({
        "path": input.display().to_string(),
        "artifact": match sidecar.artifact_kind {
            CoveAiArtifactKind::CoveAiBundle => "coveai",
            CoveAiArtifactKind::CoveVec => "covev",
        },
        "artifact_id": hex_bytes(&sidecar.header.artifact_id),
        "format": format,
        "include_payloads": include_payloads,
        "policy_report": policy_report,
        "payload_access": format!("{:?}", sidecar.payload_access),
        "filters": {
            "training_profile_id": profile_filter,
            "split_id": split_filter,
            "epoch_plan_id": epoch_plan_filter,
        },
        "counts": {
            "training_profiles": sidecar.descriptor_tables.training_profiles.len(),
            "dataset_splits": sidecar.descriptor_tables.dataset_splits.len(),
            "dedup_groups": sidecar.descriptor_tables.dedup_groups.len(),
            "training_epoch_plans": sidecar.descriptor_tables.training_epoch_plans.len(),
            "training_labels": sidecar.descriptor_tables.training_labels.len(),
            "samples_total": sidecar.descriptor_tables.training_samples.len(),
            "samples_exported": samples.len(),
        },
        "training_profiles": sidecar.descriptor_tables.training_profiles.iter().map(|profile| serde_json::json!({
            "training_profile_id": profile.training_profile_id,
            "profile_name_ref": profile.profile_name_ref,
            "task_family": profile.task_family,
            "modality_mask": profile.modality_mask,
            "source_snapshot_ref": profile.source_snapshot_ref,
            "map_profile_ref": profile.map_profile_ref,
            "chunk_profile_ref": profile.chunk_profile_ref,
            "tokenizer_profile_ref": profile.tokenizer_profile_ref,
            "vector_space_ref": profile.vector_space_ref,
            "multimodal_sequence_profile_ref": profile.multimodal_sequence_profile_ref,
            "split_policy_ref": profile.split_policy_ref,
            "sampling_policy_ref": profile.sampling_policy_ref,
            "dedup_policy_ref": profile.dedup_policy_ref,
            "quality_policy_ref": profile.quality_policy_ref,
            "license_policy_ref": profile.license_policy_ref,
            "redaction_policy_ref": profile.redaction_policy_ref,
            "default_generator_provenance_ref": profile.default_generator_provenance_ref,
            "reproducibility_class": profile.reproducibility_class,
            "flags": profile.flags,
        })).collect::<Vec<_>>(),
        "dataset_splits": sidecar.descriptor_tables.dataset_splits.iter().map(|split| serde_json::json!({
            "split_id": split.split_id,
            "split_name_ref": split.split_name_ref,
            "split_method": split.split_method,
            "source_snapshot_ref": split.source_snapshot_ref,
            "filter_policy_ref": split.filter_policy_ref,
            "seed": split.seed,
            "hash_function_ref": split.hash_function_ref,
            "stratification_path_ref": split.stratification_path_ref,
            "grouping_ref": split.grouping_ref,
            "ordering_policy_ref": split.ordering_policy_ref,
            "dedup_policy_ref": split.dedup_policy_ref,
            "sample_count": split.sample_count,
            "first_sample_ref": split.first_sample_ref,
            "flags": split.flags,
        })).collect::<Vec<_>>(),
        "dedup_groups": sidecar.descriptor_tables.dedup_groups.iter().map(|group| serde_json::json!({
            "dedup_group_id": group.dedup_group_id,
            "dedup_policy_ref": group.dedup_policy_ref,
            "canonical_member_sample_id": group.canonical_member_sample_id,
            "similarity_kind": group.similarity_kind,
            "dedup_authority": group.dedup_authority,
            "confidence_ppm": group.confidence_ppm,
            "first_member_ref": group.first_member_ref,
            "member_count": group.member_count,
            "flags": group.flags,
        })).collect::<Vec<_>>(),
        "training_epoch_plans": sidecar.descriptor_tables.training_epoch_plans.iter().filter(|plan| {
            epoch_plan_filter.is_none_or(|epoch_plan_id| plan.epoch_plan_id == epoch_plan_id)
        }).map(|plan| serde_json::json!({
            "epoch_plan_id": plan.epoch_plan_id,
            "training_profile_id": plan.training_profile_id,
            "split_ref": plan.split_ref,
            "seed": plan.seed,
            "permutation_kind": plan.permutation_kind,
            "rng_algorithm_ref": plan.rng_algorithm_ref,
            "permutation_function_ref": plan.permutation_function_ref,
            "shard_count": plan.shard_count,
            "first_shard_ref": plan.first_shard_ref,
            "shard_ref_count": plan.shard_ref_count,
            "flags": plan.flags,
        })).collect::<Vec<_>>(),
        "training_labels": sidecar.descriptor_tables.training_labels.iter().map(|label| serde_json::json!({
            "label_id": label.label_id,
            "label_kind": label.label_kind,
            "label_authority": label.label_authority,
            "label_payload_ref": label.label_payload_ref,
            "generator_provenance_ref": label.generator_provenance_ref,
            "human_review_ref": label.human_review_ref,
            "confidence_ppm": label.confidence_ppm,
            "evidence_ref": label.evidence_ref,
            "policy_ref": label.policy_ref,
            "flags": label.flags,
        })).collect::<Vec<_>>(),
        "samples": samples,
        "policy_withheld_diagnostics": diagnostics,
    })
}

fn training_export_jsonl(
    sidecar: &CoveAiFile,
    profile_filter: Option<u32>,
    split_filter: Option<u32>,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> Result<String, String> {
    let mut out = String::new();
    for sample in filtered_training_samples(sidecar, profile_filter, split_filter) {
        out.push_str(
            &serde_json::to_string(&training_sample_json_with_payloads(
                sample,
                include_payloads,
                payload_reader,
            ))
            .map_err(|error| format!("cannot serialize training sample JSON: {error}"))?,
        );
        out.push('\n');
    }
    Ok(out)
}

fn filtered_training_samples(
    sidecar: &CoveAiFile,
    profile_filter: Option<u32>,
    split_filter: Option<u32>,
) -> Vec<&cove_core::artifact::coveai::TrainingSampleEntryV1> {
    sidecar
        .descriptor_tables
        .training_samples
        .iter()
        .filter(|sample| {
            profile_filter.is_none_or(|profile| sample.training_profile_id == profile)
                && split_filter.is_none_or(|split| sample.split_ref == split)
        })
        .collect()
}

fn training_sample_json(
    sample: &cove_core::artifact::coveai::TrainingSampleEntryV1,
) -> serde_json::Value {
    serde_json::json!({
        "sample_id": sample.sample_id,
        "training_profile_id": sample.training_profile_id,
        "example_kind": sample.example_kind,
        "split_ref": sample.split_ref,
        "source_ref": sample.source_ref,
        "evidence_ref": sample.evidence_ref,
        "input_ref": sample.input_ref,
        "target_ref": sample.target_ref,
        "label_ref": sample.label_ref,
        "metadata_ref": sample.metadata_ref,
        "token_sequence_pack_ref": sample.token_sequence_pack_ref,
        "multimodal_sequence_pack_ref": sample.multimodal_sequence_pack_ref,
        "vector_ref": sample.vector_ref,
        "quality_score_ppm": sample.quality_score_ppm,
        "sample_weight_ppm": sample.sample_weight_ppm,
        "dedup_group_ref": sample.dedup_group_ref,
        "license_ref": sample.license_ref,
        "policy_ref": sample.policy_ref,
        "teacher_model_ref": sample.teacher_model_ref,
        "generator_provenance_ref": sample.generator_provenance_ref,
        "judge_generator_provenance_ref": sample.judge_generator_provenance_ref,
        "label_generator_provenance_ref": sample.label_generator_provenance_ref,
        "flags": sample.flags,
    })
}

fn training_sample_json_with_payloads(
    sample: &cove_core::artifact::coveai::TrainingSampleEntryV1,
    include_payloads: bool,
    payload_reader: &AiPayloadReader<'_>,
) -> serde_json::Value {
    let mut value = training_sample_json(sample);
    value["input"] = cli_payload_ref_json(sample.input_ref, include_payloads, payload_reader);
    value["target"] = cli_payload_ref_json(sample.target_ref, include_payloads, payload_reader);
    value["metadata"] = cli_payload_ref_json(sample.metadata_ref, include_payloads, payload_reader);
    value["evidence"] = cli_payload_ref_json(sample.evidence_ref, include_payloads, payload_reader);
    value
}

fn deterministic_vector_payload(
    file_codes: &[u32],
    dimension_count: u32,
    deterministic_seed: u64,
) -> Result<Vec<u8>, String> {
    let value_count = file_codes
        .len()
        .checked_mul(
            usize::try_from(dimension_count)
                .map_err(|_| "--dimension is too large for this platform".to_string())?,
        )
        .ok_or_else(|| "deterministic vector payload size overflows usize".to_string())?;
    let mut payload = Vec::with_capacity(
        value_count
            .checked_mul(4)
            .ok_or_else(|| "deterministic vector payload size overflows usize".to_string())?,
    );
    for file_code in file_codes {
        for dimension in 0..dimension_count {
            let seed = u64::from(*file_code)
                .wrapping_mul(1_000_003)
                .wrapping_add(u64::from(dimension).wrapping_mul(97))
                .wrapping_add(deterministic_seed);
            let value = (seed % 10_000) as f32 / 10_000.0;
            payload.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(payload)
}

fn vec_build_integrity_report(
    out: &Path,
    bytes: &[u8],
    parsed: &CoveAiFile,
    index_kind: &str,
    metric: &str,
    quantization: &str,
    index_parameters: &[(String, String)],
) -> Result<Vec<u8>, String> {
    let payload_bytes = parsed
        .descriptor_tables
        .payload_refs
        .iter()
        .map(|payload| payload.payload_length)
        .sum::<u64>();
    let report = serde_json::json!({
        "artifact": out.display().to_string(),
        "artifact_bytes": bytes.len(),
        "artifact_crc32c": checksum::crc32c(bytes),
        "payload_ref_count": parsed.descriptor_tables.payload_refs.len(),
        "payload_integrity_count": parsed.descriptor_tables.payload_integrity.len(),
        "payload_bytes": payload_bytes,
        "vector_spaces": parsed.descriptor_tables.vector_spaces.iter().map(|space| serde_json::json!({
            "vector_space_id": space.vector_space_id,
            "dimension_count": space.dimension_count,
            "element_type": space.element_type,
            "metric": space.metric,
            "quantization_policy": space.quantization_policy,
        })).collect::<Vec<_>>(),
        "vector_indexes": parsed.descriptor_tables.vector_indexes.iter().map(|index| serde_json::json!({
            "vector_index_id": index.vector_index_id,
            "index_kind": index.index_kind,
            "exactness_kind": index.exactness_kind,
            "metric": index.metric,
        })).collect::<Vec<_>>(),
        "build": {
            "index": index_kind,
            "metric": metric,
            "quantization": quantization,
            "index_parameters": index_parameters.iter().map(|(name, value)| serde_json::json!({
                "name": name,
                "value": value,
            })).collect::<Vec<_>>(),
        }
    });
    serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())
}

fn cove_vec_index_kind(value: &str) -> Result<u8, String> {
    match value {
        "exact" | "exact-flat" | "exact_flat" => Ok(0),
        "hnsw" => Ok(1),
        "ivf-flat" | "ivf_flat" => Ok(2),
        "ivf-pq" | "ivf_pq" => Ok(3),
        "diskann" => Ok(4),
        "vamana" => Ok(5),
        other => Err(format!(
            "unsupported COVE-VEC index kind '{other}'; expected exact|hnsw|ivf-flat|ivf-pq|diskann|vamana"
        )),
    }
}

fn cove_vec_metric(value: &str) -> Result<u8, String> {
    match value {
        "cosine" => Ok(0),
        "dot" => Ok(1),
        "l2" => Ok(2),
        "l1" => Ok(3),
        other => Err(format!(
            "unsupported COVE-VEC metric '{other}'; expected cosine|dot|l2|l1"
        )),
    }
}

fn cove_vec_quantization_kind(value: &str) -> Result<u8, String> {
    match value {
        "none" => Ok(0),
        "int8" => Ok(1),
        "uint8" => Ok(2),
        "pq" => Ok(3),
        other => Err(format!(
            "unsupported COVE-VEC quantization '{other}'; expected none|int8|uint8|pq"
        )),
    }
}
