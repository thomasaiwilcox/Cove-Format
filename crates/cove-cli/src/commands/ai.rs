fn run_ai(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!("missing ai subcommand\n\n{}", usage(HelpTopic::Ai)));
    }
    let subcommand = args.remove(0);
    match subcommand.as_str() {
        "export" => run_ai_export(args),
        "import" => run_ai_import(args),
        "verify" => run_ai_verify(args),
        "stream" => run_ai_stream(args),
        "diff" => run_ai_diff(args),
        "-h" | "--help" => {
            print_usage(HelpTopic::Ai);
            Ok(())
        }
        other => Err(format!(
            "unknown ai subcommand '{other}'\n\n{}",
            usage(HelpTopic::Ai)
        )),
    }
}

fn run_ai_import(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!(
            "missing ai import source kind\n\n{}",
            usage(HelpTopic::Ai)
        ));
    }
    let source_kind = args.remove(0);
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut schema = AiImportSchema::Instruction;
    let mut split_policy = AiSplitPolicy::Deterministic;
    let mut split_column = None;
    let mut dry_run = false;
    let mut publish_covm = false;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                out = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--out requires a path".to_string())?,
                ));
            }
            "--schema" => {
                schema = AiImportSchema::parse(
                    &iter
                        .next()
                        .ok_or_else(|| "--schema requires a value".to_string())?,
                )
                .map_err(|err| err.to_string())?;
            }
            "--split-policy" => {
                split_policy = AiSplitPolicy::parse(
                    &iter
                        .next()
                        .ok_or_else(|| "--split-policy requires a value".to_string())?,
                )
                .map_err(|err| err.to_string())?;
            }
            "--split-column" => {
                split_column = Some(
                    iter.next()
                        .ok_or_else(|| "--split-column requires a column name".to_string())?,
                );
            }
            "--dry-run" => dry_run = true,
            "--publish-covm" => publish_covm = true,
            "-h" | "--help" => {
                print_usage(HelpTopic::Ai);
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown ai import argument '{value}'"));
            }
            value => {
                if input.replace(PathBuf::from(value)).is_some() {
                    return Err("ai import accepts exactly one input path".into());
                }
            }
        }
    }
    let input = input.ok_or_else(|| "ai import requires an input path".to_string())?;
    let options = AiImportOptions {
        schema,
        split_policy,
        split_column,
        dry_run,
        publish_covm,
        ..AiImportOptions::default()
    };
    let report = match source_kind.as_str() {
        "jsonl" => import_jsonl(&input, out.as_deref(), options),
        "parquet" => import_parquet(&input, out.as_deref(), options),
        "hf" => import_hf_dir(&input, out.as_deref(), options),
        other => {
            return Err(format!(
                "unknown ai import kind '{other}'; expected jsonl, parquet, or hf"
            ))
        }
    }
    .map_err(|err| err.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("cannot serialize AI import report: {error}"))?
    );
    Ok(())
}

fn run_ai_verify(args: Vec<String>) -> Result<(), String> {
    let mut input = None;
    let mut policy_report = false;
    let mut json_output = false;
    let mut dataset_dir: Option<PathBuf> = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--policy-report" => policy_report = true,
            "--json" => json_output = true,
            "--dataset" => {
                dataset_dir =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory".to_string()
                    })?));
            }
            "-h" | "--help" => {
                print_usage(HelpTopic::Ai);
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown ai verify argument '{value}'"));
            }
            value => {
                if input.replace(PathBuf::from(value)).is_some() {
                    return Err("ai verify accepts exactly one sidecar or manifest".into());
                }
            }
        }
    }
    let input = input.ok_or_else(|| "ai verify requires a sidecar or manifest".to_string())?;
    let archive = open_ai_archive(
        &input,
        AiArchiveOpenOptions {
            cove_ai: None,
            dataset_dir,
        },
    )
    .map_err(|err| err.to_string())?;
    let report = archive
        .verify(AiVerifyOptions { policy_report })
        .map_err(|err| err.to_string())?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("cannot serialize AI verify report: {error}"))?
        );
    } else {
        println!("COVE-AI archive: {}", input.display());
        println!(
            "  samples: {}",
            report
                .get("training_sample_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        );
        println!(
            "  payload_access: {}",
            report
                .get("payload_access")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
        );
        println!(
            "  withheld diagnostics: {}",
            report
                .get("withheld_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        );
    }
    Ok(())
}

fn run_ai_stream(args: Vec<String>) -> Result<(), String> {
    let mut input = None;
    let mut out = None;
    let mut format = AiExportFormat::Jsonl;
    let mut split = None;
    let mut include_payloads = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--format" => {
                format = AiExportFormat::parse(
                    &iter
                        .next()
                        .ok_or_else(|| "--format requires a value".to_string())?,
                )
                .map_err(|err| err.to_string())?;
            }
            "--split" => {
                split =
                    Some(iter.next().ok_or_else(|| {
                        "--split requires train, validation, or test".to_string()
                    })?);
            }
            "--include-payloads" => include_payloads = true,
            "--out" => {
                out = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--out requires a path".to_string())?,
                ));
            }
            "-h" | "--help" => {
                print_usage(HelpTopic::Ai);
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown ai stream argument '{value}'"));
            }
            value => {
                if input.replace(PathBuf::from(value)).is_some() {
                    return Err("ai stream accepts exactly one sidecar or manifest".into());
                }
            }
        }
    }
    let input = input.ok_or_else(|| "ai stream requires a sidecar or manifest".to_string())?;
    if matches!(
        format,
        AiExportFormat::Arrow | AiExportFormat::Parquet | AiExportFormat::WebDataset
    ) && out.is_none()
    {
        return Err(format!(
            "ai stream --format {} requires --out",
            format.as_str()
        ));
    }
    let data = stream_archive(
        &input,
        AiExportOptions {
            format,
            out: out.clone(),
            split,
            include_payloads,
            policy_report: true,
        },
    )
    .map_err(|err| err.to_string())?;
    write_export_file(data, out).map_err(|err| err.to_string())
}

fn run_ai_diff(args: Vec<String>) -> Result<(), String> {
    let mut old = None;
    let mut new = None;
    let mut key_field = "sample_id".to_string();
    let mut report_path = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--keys" => {
                key_field = iter
                    .next()
                    .ok_or_else(|| "--keys requires a key field".to_string())?;
            }
            "--report" => {
                report_path = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--report requires a path".to_string())?,
                ));
            }
            "-h" | "--help" => {
                print_usage(HelpTopic::Ai);
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown ai diff argument '{value}'"));
            }
            value if old.is_none() => old = Some(PathBuf::from(value)),
            value if new.is_none() => new = Some(PathBuf::from(value)),
            _ => return Err("ai diff accepts exactly two sidecars".into()),
        }
    }
    let old = old.ok_or_else(|| "ai diff requires <old.coveai>".to_string())?;
    let new = new.ok_or_else(|| "ai diff requires <new.coveai>".to_string())?;
    let report = diff_archives(&old, &new, &key_field).map_err(|err| err.to_string())?;
    let text = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize AI diff report: {error}"))?;
    if let Some(report_path) = report_path {
        fs::write(&report_path, text)
            .map_err(|error| format!("cannot write {}: {error}", report_path.display()))?;
    } else {
        println!("{text}");
    }
    Ok(())
}

fn run_ai_export(args: Vec<String>) -> Result<(), String> {
    let mut kind: Option<String> = None;
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut format = AiExportFormat::Json;
    let mut include_payloads = false;
    let mut policy_report = false;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--include-payloads" => include_payloads = true,
            "--policy-report" => policy_report = true,
            "--format" => {
                format = AiExportFormat::parse(
                    &iter
                        .next()
                        .ok_or_else(|| "--format requires a value".to_string())?,
                )
                .map_err(|err| err.to_string())?;
            }
            "--out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out requires a path".to_string())?;
                out = Some(PathBuf::from(value));
            }
            "-h" | "--help" => {
                println!(
                    "Usage:\n  cove ai export <chunks|tokens|vectors|training|multimodal|assets|tensors> <sidecar> [--include-payloads] [--format json|jsonl|hf-jsonl|arrow|parquet|webdataset] [--out <path>] [--policy-report]"
                );
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown ai export argument '{value}'"));
            }
            value if kind.is_none() => kind = Some(value.to_string()),
            value if input.is_none() => input = Some(PathBuf::from(value)),
            _ => return Err("ai export accepts exactly one kind and one input sidecar".into()),
        }
    }

    let kind = kind.ok_or_else(|| "ai export requires <kind>".to_string())?;
    let input = input.ok_or_else(|| "ai export requires <sidecar>".to_string())?;
    let bytes =
        fs::read(&input).map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    let sidecar = CoveAiFile::parse_for_operation(&bytes, OperationKindV2::AiTrainingSampleExport)
        .map_err(|error| format!("{}: invalid COVE-AI sidecar: {error}", input.display()))?;
    let reader = AiPayloadReader::new(
        &bytes,
        &sidecar,
        if include_payloads {
            CoveAiAccessContext::for_operation(format!("ai_export_{kind}"))
        } else {
            CoveAiAccessContext::descriptor_only(format!("ai_export_{kind}"))
        },
    );
    let value = ai_export_json_value(
        &input,
        &kind,
        format.as_str(),
        include_payloads,
        policy_report,
        &sidecar,
        &reader,
    )?;
    write_ai_export_output(&value, format.as_str(), out)
}

fn ai_export_jsonl(value: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(records) = value.get("records").and_then(|records| records.as_array()) {
        for record in records {
            out.push_str(&record.to_string());
            out.push('\n');
        }
    }
    out
}

fn write_ai_export_output(
    value: &serde_json::Value,
    format: &str,
    out: Option<PathBuf>,
) -> Result<(), String> {
    match format {
        "json" => {
            let text = json_pretty_string(value)?;
            if let Some(out) = out {
                fs::write(&out, text)
                    .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
            } else {
                println!("{text}");
            }
            Ok(())
        }
        "jsonl" | "hf-jsonl" => {
            let text = ai_export_jsonl(value);
            if let Some(out) = out {
                fs::write(&out, text)
                    .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
            } else {
                print!("{text}");
            }
            Ok(())
        }
        "arrow" | "parquet" | "webdataset" => {
            let out = out.ok_or_else(|| format!("ai export --format {format} requires --out"))?;
            let bytes = match format {
                "arrow" => {
                    let batch = ai_export_record_batch(value)?;
                    arrow_export::write_ipc(&batch.schema(), &[batch])?
                }
                "parquet" => {
                    let batch = ai_export_record_batch(value)?;
                    write_ai_export_parquet(&batch)?
                }
                "webdataset" => write_ai_export_webdataset(value)?,
                _ => unreachable!(),
            };
            cove_core::durable::durable_replace(&out, &bytes).map_err(|error| {
                format!(
                    "cannot durably publish {} AI export: {error}",
                    out.display()
                )
            })?;
            Ok(())
        }
        other => Err(format!("unsupported ai export format '{other}'")),
    }
}

fn ai_export_record_batch(value: &serde_json::Value) -> Result<RecordBatch, String> {
    let records = ai_export_records(value)?;
    let ordinals = UInt64Array::from_iter_values(0..records.len() as u64);
    let record_kinds = StringArray::from(
        records
            .iter()
            .map(|record| {
                record
                    .get("record_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or("record")
                    .to_string()
            })
            .collect::<Vec<_>>(),
    );
    let payload_access = StringArray::from(
        records
            .iter()
            .map(ai_record_payload_access_summary)
            .collect::<Vec<_>>(),
    );
    let record_json = StringArray::from(
        records
            .iter()
            .map(|record| record.to_string())
            .collect::<Vec<_>>(),
    );
    let mut metadata = HashMap::new();
    for key in ["path", "kind", "format", "artifact_id", "payload_access"] {
        if let Some(text) = value.get(key).and_then(|value| value.as_str()) {
            metadata.insert(format!("cove.ai.{key}"), text.to_string());
        }
    }
    for key in ["include_payloads", "policy_report"] {
        if let Some(flag) = value.get(key).and_then(|value| value.as_bool()) {
            metadata.insert(format!("cove.ai.{key}"), flag.to_string());
        }
    }
    if let Some(diagnostics) = value.get("diagnostics") {
        metadata.insert(
            "cove.ai.diagnostics_json".to_string(),
            diagnostics.to_string(),
        );
    }
    let schema = Schema::new(vec![
        Field::new("record_ordinal", DataType::UInt64, false),
        Field::new("record_kind", DataType::Utf8, false),
        Field::new("payload_access", DataType::Utf8, false),
        Field::new("record_json", DataType::Utf8, false),
    ])
    .with_metadata(metadata);
    RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(ordinals) as ArrayRef,
            Arc::new(record_kinds) as ArrayRef,
            Arc::new(payload_access) as ArrayRef,
            Arc::new(record_json) as ArrayRef,
        ],
    )
    .map_err(|error| format!("cannot build AI export Arrow batch: {error}"))
}

fn ai_export_records(value: &serde_json::Value) -> Result<&[serde_json::Value], String> {
    value
        .get("records")
        .or_else(|| value.get("samples"))
        .and_then(|records| records.as_array())
        .map(Vec::as_slice)
        .ok_or_else(|| "AI export value missing records or samples array".to_string())
}

fn ai_record_payload_access_summary(record: &serde_json::Value) -> String {
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

fn collect_payload_access_values(value: &serde_json::Value, values: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(access) = object
                .get("payload_access")
                .and_then(|value| value.as_str())
            {
                values.push(access.to_string());
            }
            for child in object.values() {
                collect_payload_access_values(child, values);
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                collect_payload_access_values(child, values);
            }
        }
        _ => {}
    }
}

fn write_ai_export_parquet(batch: &RecordBatch) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut writer = parquet::arrow::ArrowWriter::try_new(&mut bytes, batch.schema(), None)
            .map_err(|error| format!("Parquet writer: {error}"))?;
        writer
            .write(batch)
            .map_err(|error| format!("Parquet write: {error}"))?;
        writer
            .close()
            .map_err(|error| format!("Parquet close: {error}"))?;
    }
    Ok(bytes)
}

fn write_ai_export_webdataset(value: &serde_json::Value) -> Result<Vec<u8>, String> {
    let records = ai_export_records(value)?;
    let mut out = Vec::new();
    let mut metadata = value.clone();
    if let Some(object) = metadata.as_object_mut() {
        object.remove("records");
        object.remove("samples");
    }
    write_tar_entry(&mut out, "metadata.json", &json_pretty_bytes(&metadata)?)?;
    for (index, record) in records.iter().enumerate() {
        write_tar_entry(
            &mut out,
            &format!("{index:06}.json"),
            record.to_string().as_bytes(),
        )?;
    }
    out.extend_from_slice(&[0u8; 1024]);
    Ok(out)
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

fn ai_export_json_value(
    input: &Path,
    kind: &str,
    format: &str,
    include_payloads: bool,
    policy_report: bool,
    sidecar: &CoveAiFile,
    reader: &AiPayloadReader<'_>,
) -> Result<serde_json::Value, String> {
    let records = match kind {
        "chunks" => sidecar
            .descriptor_tables
            .text_chunks
            .iter()
            .map(|chunk| serde_json::json!({
                "record_kind": "text_chunk",
                "chunk_id": chunk.chunk_id,
                "source_ref": chunk.source_ref,
                "byte_start": chunk.byte_start,
                "byte_length": chunk.byte_length,
                "source_value_hash_ref": chunk.source_value_hash_ref,
                "chunk_text_hash_ref": chunk.chunk_text_hash_ref,
                "text_reconstruction": "requires_source_cove_value",
            }))
            .collect::<Vec<_>>(),
        "tokens" => sidecar
            .descriptor_tables
            .token_blocks
            .iter()
            .map(|block| serde_json::json!({
                "record_kind": "token_block",
                "token_block_id": block.token_block_id,
                "tokenizer_profile_id": block.tokenizer_profile_id,
                "token_count": block.token_count,
                "token_id_width": block.token_id_width,
                "payload": cli_payload_ref_json(block.payload_ref, include_payloads, reader),
            }))
            .collect::<Vec<_>>(),
        "vectors" => {
            let mut records = sidecar
                .descriptor_tables
                .vector_entries
                .iter()
                .map(|entry| serde_json::json!({
                    "record_kind": "vector_entry",
                    "vector_ref": entry.vector_ref,
                    "block_id": entry.block_id,
                    "vector_ordinal": entry.vector_ordinal,
                    "payload_offset": entry.payload_offset,
                    "payload_length": entry.payload_length,
                }))
                .collect::<Vec<_>>();
            records.extend(sidecar.descriptor_tables.filecode_vector_bindings.iter().map(|binding| serde_json::json!({
                "record_kind": "filecode_vector_binding",
                "binding_id": binding.binding_id,
                "vector_space_id": binding.vector_space_id,
                "file_code": binding.file_code,
                "vector_ref": binding.vector_ref,
                "model_input_digest_ref": binding.model_input_digest_ref,
            })));
            records.extend(sidecar.descriptor_tables.chunk_vector_bindings.iter().map(|binding| serde_json::json!({
                "record_kind": "chunk_vector_binding",
                "binding_id": binding.binding_id,
                "vector_space_id": binding.vector_space_id,
                "chunk_id": binding.chunk_id,
                "vector_ref": binding.vector_ref,
                "model_input_digest_ref": binding.model_input_digest_ref,
            })));
            records.extend(sidecar.descriptor_tables.object_state_vector_bindings.iter().map(|binding| serde_json::json!({
                "record_kind": "object_state_vector_binding",
                "binding_id": binding.binding_id,
                "vector_space_id": binding.vector_space_id,
                "object_type_id": binding.object_type_id,
                "vector_ref": binding.vector_ref,
                "model_input_digest_ref": binding.model_input_digest_ref,
            })));
            records.extend(sidecar.descriptor_tables.training_sample_vector_bindings.iter().map(|binding| serde_json::json!({
                "record_kind": "training_sample_vector_binding",
                "binding_id": binding.binding_id,
                "vector_space_id": binding.vector_space_id,
                "sample_id": binding.sample_id,
                "vector_ref": binding.vector_ref,
                "model_input_digest_ref": binding.model_input_digest_ref,
            })));
            records.extend(sidecar.descriptor_tables.association_state_vector_bindings.iter().map(|binding| serde_json::json!({
                "record_kind": "association_state_vector_binding",
                "binding_id": binding.binding_id,
                "vector_space_id": binding.vector_space_id,
                "association_type_id": binding.association_type_id,
                "association_key_ref": binding.association_key_ref,
                "vector_ref": binding.vector_ref,
                "model_input_digest_ref": binding.model_input_digest_ref,
            })));
            records.extend(sidecar.descriptor_tables.asset_vector_bindings.iter().map(|binding| serde_json::json!({
                "record_kind": "asset_vector_binding",
                "binding_id": binding.binding_id,
                "vector_space_id": binding.vector_space_id,
                "asset_ref": binding.asset_ref,
                "transform_ref": binding.transform_ref,
                "vector_ref": binding.vector_ref,
                "model_input_digest_ref": binding.model_input_digest_ref,
            })));
            records.extend(sidecar.descriptor_tables.multimodal_sequence_vector_bindings.iter().map(|binding| serde_json::json!({
                "record_kind": "multimodal_sequence_vector_binding",
                "binding_id": binding.binding_id,
                "vector_space_id": binding.vector_space_id,
                "sequence_pack_id": binding.sequence_pack_id,
                "sequence_profile_ref": binding.sequence_profile_ref,
                "vector_ref": binding.vector_ref,
                "model_input_digest_ref": binding.model_input_digest_ref,
            })));
            records
        },
        "training" => filtered_training_samples(sidecar, None, None)
            .into_iter()
            .map(|sample| {
                let mut value = training_sample_json(sample);
                value["input"] = cli_payload_ref_json(sample.input_ref, include_payloads, reader);
                value["target"] = cli_payload_ref_json(sample.target_ref, include_payloads, reader);
                value["metadata"] = cli_payload_ref_json(sample.metadata_ref, include_payloads, reader);
                value
            })
            .collect::<Vec<_>>(),
        "multimodal" => sidecar
            .descriptor_tables
            .multimodal_sequence_elements
            .iter()
            .map(|element| serde_json::json!({
                "record_kind": "multimodal_sequence_element",
                "element_id": element.element_id,
                "sequence_pack_id": element.sequence_pack_id,
                "ordinal": element.ordinal,
                "modality": element.modality,
                "role": element.role,
                "asset_ref": element.asset_ref,
                "tensor_ref": element.tensor_ref,
                "vector_ref": element.vector_ref,
                "position_stream": cli_payload_ref_json(element.position_stream_ref, include_payloads, reader),
                "evidence": cli_payload_ref_json(element.evidence_ref, include_payloads, reader),
            }))
            .collect::<Vec<_>>(),
        "assets" => sidecar
            .descriptor_tables
            .assets
            .iter()
            .map(|asset| serde_json::json!({
                "record_kind": "asset",
                "asset_ref_id": asset.asset_ref_id,
                "asset_kind": asset.asset_kind,
                "uri_ref": asset.uri_ref,
                "embedded_section_ref": asset.embedded_section_ref,
                "media_type_ref": asset.media_type_ref,
                "byte_length": asset.byte_length,
                "digest_ref": asset.digest_ref,
                "policy_ref": asset.policy_ref,
            }))
            .collect::<Vec<_>>(),
        "tensors" => sidecar
            .descriptor_tables
            .tensor_layouts
            .iter()
            .map(|tensor| serde_json::json!({
                "record_kind": "tensor_layout",
                "tensor_layout_id": tensor.tensor_layout_id,
                "dtype": tensor.dtype,
                "rank": tensor.rank,
                "shape_ref": tensor.shape_ref,
                "stride_ref": tensor.stride_ref,
                "shape": cli_payload_ref_json(tensor.shape_ref, include_payloads, reader),
                "stride": cli_payload_ref_json(tensor.stride_ref, include_payloads, reader),
            }))
            .collect::<Vec<_>>(),
        other => {
            return Err(format!(
                "unknown ai export kind '{other}'; expected chunks, tokens, vectors, training, multimodal, assets, or tensors"
            ));
        }
    };

    let mut diagnostics = Vec::new();
    if include_payloads
        && !matches!(
            sidecar.payload_access,
            cove_core::artifact::coveai::AiPayloadAccessState::StructurallyAllowed
        )
    {
        diagnostics.push(serde_json::json!({
            "code": "COVE_AI_PAYLOAD_POLICY_BLOCKED",
            "message": "payload export requested but payload access is not structurally allowed",
            "payload_access": format!("{:?}", sidecar.payload_access),
        }));
    }
    Ok(serde_json::json!({
        "path": input.display().to_string(),
        "kind": kind,
        "format": format,
        "include_payloads": include_payloads,
        "policy_report": policy_report,
        "artifact_id": hex_bytes(&sidecar.header.artifact_id),
        "payload_access": format!("{:?}", sidecar.payload_access),
        "records": records,
        "diagnostics": diagnostics,
    }))
}

fn cli_payload_ref_json(
    payload_ref: u32,
    include_payloads: bool,
    reader: &AiPayloadReader<'_>,
) -> serde_json::Value {
    if payload_ref == 0 {
        return serde_json::json!({
            "payload_ref": 0,
            "payload_access": "not_declared",
        });
    }
    if !include_payloads {
        return serde_json::json!({
            "payload_ref": payload_ref,
            "payload_access": "not_requested",
        });
    }
    match reader.lease_payload_ref(payload_ref) {
        Ok(lease) => match std::str::from_utf8(lease.bytes) {
            Ok(text) => serde_json::json!({
                "payload_ref": payload_ref,
                "payload_access": lease.disclosure.as_str(),
                "decoded_length": lease.decoded_length,
                "text": text,
            }),
            Err(_) => serde_json::json!({
                "payload_ref": payload_ref,
                "payload_access": lease.disclosure.as_str(),
                "decoded_length": lease.decoded_length,
                "bytes_hex": hex_bytes(lease.bytes),
            }),
        },
        Err(error) => serde_json::json!({
            "payload_ref": payload_ref,
            "payload_access": "withheld",
            "withholding_reason": error.to_string(),
        }),
    }
}
