use std::path::Path;

use cove_core::{
    artifact::{coveai::CoveAiFile, covedelta::CoveDeltaFile, covemap::CovemapFile},
    constants::{MAGIC_COVE, MAGIC_COVEAI, MAGIC_COVEDELTA, MAGIC_COVEMAP, MAGIC_COVEV},
    feature_scope::FeatureUseRequestV2,
    reader::{self, ValidationOptions},
};
use cove_profile_validation::EmbeddedOptionalProfileValidator;

use crate::{args::CliArgs, format::profile_name};

pub(crate) fn validate_paths(args: &CliArgs) -> bool {
    let mut all_ok = true;
    if args.json_out {
        print!("[");
    }
    let mut first = true;
    for path in &args.file_paths {
        let ok = if args.json_out {
            if !first {
                print!(",");
            }
            first = false;
            validate_file_json(
                Path::new(path),
                args.validation.clone(),
                args.feature_use.clone(),
                args.object_delta,
                args.explain,
            )
        } else {
            validate_file_text(
                Path::new(path),
                args.validation.clone(),
                args.feature_use.clone(),
                args.object_delta,
            )
        };
        if !ok {
            all_ok = false;
        }
    }
    if args.json_out {
        println!("]");
    }
    all_ok
}

/// Machine-readable validation output (Spec §73, §76): emits one JSON object
/// per file with `path`, `ok`, optional `error_code`, and (when --explain)
/// the validated header/footer summary.
fn validate_file_json(
    path: &Path,
    opts: ValidationOptions,
    feature_use: Option<FeatureUseRequestV2>,
    object_delta: bool,
    explain: bool,
) -> bool {
    let path_str = path.display().to_string();
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            print!(
                "{{\"path\":{},\"ok\":false,\"error\":{}}}",
                json_str(&path_str),
                json_str(&format!("io: {e}"))
            );
            return false;
        }
    };

    if data.len() >= 4 && data[data.len() - 4..] == MAGIC_COVEMAP {
        return validate_covemap_json(
            &path_str,
            &data,
            opts.semantic,
            opts.verify_digests,
            explain,
        );
    }

    if data.len() >= 4 && data[data.len() - 4..] == MAGIC_COVEDELTA {
        return validate_covedelta_json(&path_str, &data, object_delta, explain);
    }

    if data.len() >= 4
        && (data[data.len() - 4..] == MAGIC_COVEAI || data[data.len() - 4..] == MAGIC_COVEV)
    {
        return validate_coveai_json(&path_str, &data, explain);
    }

    match validate_cove_bytes(&data, opts, feature_use) {
        Ok(report) => {
            let info = &report.validated;
            print!(
                "{{\"path\":{},\"ok\":true,\"semantic\":{},\"version_major\":{},\"version_minor\":{},\"primary_profile\":{},\"section_count\":{}",
                json_str(&path_str),
                report.semantic_checked,
                info.header.version_major,
                info.header.version_minor,
                info.header.primary_profile,
                info.footer.sections.len()
            );
            if !report.ignored_optional_sections.is_empty() {
                print!(",\"ignored_optional_sections\":[");
                for (index, section) in report.ignored_optional_sections.iter().enumerate() {
                    if index > 0 {
                        print!(",");
                    }
                    print!(
                        "{{\"section_id\":{},\"section_kind\":{},\"reason\":{}}}",
                        section.section_id,
                        section.section_kind,
                        json_str(&section.reason)
                    );
                }
                print!("]");
            }
            if explain {
                print!(",\"sections\":[");
                for (index, section) in info.footer.sections.iter().enumerate() {
                    if index > 0 {
                        print!(",");
                    }
                    print!(
                        "{{\"kind\":{},\"offset\":{},\"length\":{}}}",
                        section.section_kind, section.offset, section.length
                    );
                }
                print!("]");
            }
            print!("}}");
            true
        }
        Err(error) => {
            print!("{{\"path\":{},\"ok\":false", json_str(&path_str));
            if let Some(code) = error.spec_code() {
                print!(",\"error_code\":{}", json_str(code));
            }
            print!(",\"error\":{}}}", json_str(&error.to_string()));
            false
        }
    }
}

fn validate_covedelta_json(path_str: &str, data: &[u8], object_delta: bool, explain: bool) -> bool {
    match CoveDeltaFile::parse(data).and_then(|file| {
        if object_delta {
            file.validate_object_delta().map(|_| file)
        } else {
            Ok(file)
        }
    }) {
        Ok(file) => {
            print!(
                "{{\"path\":{},\"ok\":true,\"artifact\":\"covedelta\",\"object_delta\":{},\"version_major\":{},\"version_minor\":{},\"section_count\":{},\"parent_ref_count\":{}",
                json_str(path_str),
                object_delta,
                file.header.version_major,
                file.header.version_minor,
                file.sections.len(),
                file.parent_refs.len()
            );
            if explain {
                print!(",\"sections\":[");
                for (index, section) in file.sections.iter().enumerate() {
                    if index > 0 {
                        print!(",");
                    }
                    print!(
                        "{{\"id\":{},\"kind\":{},\"offset\":{},\"length\":{}}}",
                        section.entry.section_id,
                        section.entry.section_kind,
                        section.entry.offset,
                        section.entry.length
                    );
                }
                print!("]");
            }
            print!("}}");
            true
        }
        Err(error) => {
            print!("{{\"path\":{},\"ok\":false", json_str(path_str));
            if let Some(code) = error.spec_code() {
                print!(",\"error_code\":{}", json_str(code));
            }
            print!(",\"error\":{}}}", json_str(&error.to_string()));
            false
        }
    }
}

fn validate_covemap_json(
    path_str: &str,
    data: &[u8],
    semantic: bool,
    verify_digests: bool,
    explain: bool,
) -> bool {
    match validate_covemap_bytes(data, semantic) {
        Ok(file) => {
            print!(
                "{{\"path\":{},\"ok\":true,\"artifact\":\"covemap\",\"semantic\":{},\"version_major\":{},\"version_minor\":{},\"mapping_version\":{},\"section_count\":{}",
                json_str(path_str),
                semantic,
                file.header.version_major,
                file.header.version_minor,
                json_str(&file.mapping_version),
                file.sections.len()
            );
            if verify_digests {
                print!(",\"verify_digests_skipped\":true");
            }
            if explain {
                print!(",\"sections\":[");
                for (index, section) in file.sections.iter().enumerate() {
                    if index > 0 {
                        print!(",");
                    }
                    print!(
                        "{{\"kind\":{},\"offset\":{},\"length\":{},\"uncompressed_length\":{},\"required\":{}}}",
                        section.entry.section_id,
                        section.entry.offset,
                        section.entry.length,
                        section.entry.uncompressed_length,
                        section.entry.required
                    );
                }
                print!("]");
            }
            print!("}}");
            true
        }
        Err(error) => {
            print!("{{\"path\":{},\"ok\":false", json_str(path_str));
            if let Some(code) = error.spec_code() {
                print!(",\"error_code\":{}", json_str(code));
            }
            print!(",\"error\":{}}}", json_str(&error.to_string()));
            false
        }
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Validate a single COVE file. Returns `true` if valid.
fn validate_file_text(
    path: &Path,
    opts: ValidationOptions,
    feature_use: Option<FeatureUseRequestV2>,
    object_delta: bool,
) -> bool {
    let display = path.display();
    print!("Validating {display} ... ");

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            println!("ERROR");
            eprintln!("  [I/O] {e}");
            return false;
        }
    };

    if data.len() >= 4 && data[data.len() - 4..] == MAGIC_COVEMAP {
        return validate_covemap_file(&data, opts.semantic, opts.verify_digests);
    }

    if data.len() >= 4 && data[data.len() - 4..] == MAGIC_COVEDELTA {
        return validate_covedelta_file(&data, object_delta);
    }

    if data.len() >= 4
        && (data[data.len() - 4..] == MAGIC_COVEAI || data[data.len() - 4..] == MAGIC_COVEV)
    {
        return validate_coveai_file(&data);
    }

    if data.len() < 4 || data[data.len() - 4..] != MAGIC_COVE {
        println!("INVALID");
        eprintln!("  [ERR] COVE_E_BAD_MAGIC: unrecognized trailing magic");
        return false;
    }

    match validate_cove_bytes(&data, opts, feature_use) {
        Ok(report) => {
            let mode = if report.semantic_checked {
                "semantic"
            } else {
                "structural"
            };
            println!("OK [{mode}]");
            let info = &report.validated;
            println!(
                "  COVE v{}.{}",
                info.header.version_major, info.header.version_minor
            );
            println!("  file_len        : {} bytes", data.len());
            println!(
                "  primary_profile : {}",
                profile_name(info.header.primary_profile)
            );
            println!("  section_count   : {}", info.footer.sections.len());
            if let Some(n) = report.dict_entry_count {
                println!("  dict_entries    : {n}");
            }
            if !report.ignored_optional_sections.is_empty() {
                println!(
                    "  ignored optional: {} section(s)",
                    report.ignored_optional_sections.len()
                );
            }
            let metadata_json_preview = String::from_utf8_lossy(&info.footer.metadata_json)
                .chars()
                .take(120)
                .collect::<String>()
                .replace('\n', " ");
            if !metadata_json_preview.is_empty() {
                println!("  metadata (first 120 chars): {}", &metadata_json_preview);
            }
            true
        }
        Err(error) => {
            println!("INVALID");
            eprintln!("  [ERR] {error}");
            false
        }
    }
}

fn validate_covedelta_file(data: &[u8], object_delta: bool) -> bool {
    match CoveDeltaFile::parse(data).and_then(|file| {
        if object_delta {
            file.validate_object_delta().map(|_| file)
        } else {
            Ok(file)
        }
    }) {
        Ok(_) => {
            let mode = if object_delta {
                "object-delta"
            } else {
                "structural"
            };
            println!("OK [covedelta {mode}]");
            true
        }
        Err(error) => {
            println!("INVALID");
            eprintln!("  [ERR] {error}");
            false
        }
    }
}

fn validate_coveai_json(path_str: &str, data: &[u8], explain: bool) -> bool {
    match CoveAiFile::parse(data) {
        Ok(file) => {
            print!(
                "{{\"path\":{},\"ok\":true,\"artifact\":\"{}\",\"version_major\":{},\"version_minor\":{},\"section_count\":{},\"payload_access\":{}",
                json_str(path_str),
                match file.artifact_kind {
                    cove_core::artifact::coveai::CoveAiArtifactKind::CoveAiBundle => "coveai",
                    cove_core::artifact::coveai::CoveAiArtifactKind::CoveVec => "covev",
                },
                file.header.version_major,
                file.header.version_minor,
                file.sections.len(),
                json_str(&format!("{:?}", file.payload_access))
            );
            if explain {
                print!(
                    ",\"records\":{{\"source_bindings\":{},\"payload_refs\":{},\"payload_integrity\":{},\"privacy_summaries\":{},\"vector_spaces\":{},\"vector_payload_blocks\":{},\"vector_entries\":{},\"filecode_vector_bindings\":{}}}",
                    file.descriptor_tables.source_bindings.len(),
                    file.descriptor_tables.payload_refs.len(),
                    file.descriptor_tables.payload_integrity.len(),
                    file.descriptor_tables.privacy_summaries.len(),
                    file.descriptor_tables.vector_spaces.len(),
                    file.descriptor_tables.vector_payload_blocks.len(),
                    file.descriptor_tables.vector_entries.len(),
                    file.descriptor_tables.filecode_vector_bindings.len()
                );
                print!(",\"sections\":[");
                for (index, section) in file.sections.iter().enumerate() {
                    if index > 0 {
                        print!(",");
                    }
                    print!(
                        "{{\"id\":{},\"kind\":{},\"offset\":{},\"length\":{},\"encoding\":{},\"profile\":{},\"records\":{}}}",
                        section.entry.section_id,
                        section.entry.section_kind,
                        section.entry.offset,
                        section.entry.length,
                        section.entry.payload_encoding,
                        section.entry.profile_kind,
                        section.record_headers.len()
                    );
                }
                print!("]");
            }
            print!("}}");
            true
        }
        Err(error) => {
            print!("{{\"path\":{},\"ok\":false", json_str(path_str));
            if let Some(code) = error.spec_code() {
                print!(",\"error_code\":{}", json_str(code));
            }
            print!(",\"error\":{}}}", json_str(&error.to_string()));
            false
        }
    }
}

fn validate_coveai_file(data: &[u8]) -> bool {
    match CoveAiFile::parse(data) {
        Ok(file) => {
            println!(
                "OK [{} structural]",
                match file.artifact_kind {
                    cove_core::artifact::coveai::CoveAiArtifactKind::CoveAiBundle => "coveai",
                    cove_core::artifact::coveai::CoveAiArtifactKind::CoveVec => "covev",
                }
            );
            println!("  section_count   : {}", file.sections.len());
            println!("  payload_access  : {:?}", file.payload_access);
            println!(
                "  records         : source_bindings={} payload_refs={} vectors={}",
                file.descriptor_tables.source_bindings.len(),
                file.descriptor_tables.payload_refs.len(),
                file.descriptor_tables.vector_entries.len()
            );
            true
        }
        Err(error) => {
            println!("INVALID");
            eprintln!("  [ERR] {error}");
            false
        }
    }
}

fn validate_cove_bytes(
    data: &[u8],
    opts: ValidationOptions,
    feature_use: Option<FeatureUseRequestV2>,
) -> Result<reader::ValidationReport, cove_core::CoveError> {
    let optional_pushdown_policy = opts.optional_pushdown_policy;
    let embedded_feature_use = feature_use.clone();
    let validator = EmbeddedOptionalProfileValidator::default_builtins();
    let report = match feature_use {
        Some(request) => reader::validate_bytes_for_feature_use_with_optional_profile_validator(
            data, opts, request, &validator,
        ),
        None => reader::validate_bytes_with_options(data, opts),
    }?;
    if report.semantic_checked || embedded_feature_use.is_some() {
        validator.validate_embedded_optional_profile_sections(
            data,
            &report,
            optional_pushdown_policy,
            embedded_feature_use.as_ref(),
            !report.semantic_checked,
        )?;
    }
    Ok(report)
}

fn validate_covemap_bytes(
    data: &[u8],
    semantic: bool,
) -> Result<CovemapFile, cove_core::CoveError> {
    let file = CovemapFile::parse(data)?;
    if semantic {
        file.validate_map_sections()?;
    }
    Ok(file)
}

fn validate_covemap_file(data: &[u8], semantic: bool, verify_digests: bool) -> bool {
    match validate_covemap_bytes(data, semantic) {
        Ok(file) => {
            let mode = if semantic { "semantic" } else { "structural" };
            println!("OK [{mode}]");
            println!(
                "  COVEMAP v{}.{}",
                file.header.version_major, file.header.version_minor
            );
            println!("  file_len        : {} bytes", data.len());
            println!("  mapping_version : {}", file.mapping_version);
            println!("  section_count   : {}", file.sections.len());
            for warning in file.compatibility_warnings() {
                eprintln!("  [WARN] {warning}");
            }
            if verify_digests {
                eprintln!(
                    "  [NOTE] --verify-digests is not applicable to COVEMAP artifacts (skipped)"
                );
            }
            true
        }
        Err(error) => {
            println!("INVALID");
            eprintln!("  [ERR] {error}");
            false
        }
    }
}
