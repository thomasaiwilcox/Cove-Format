use super::*;

#[derive(Debug, Clone, Copy)]
pub(crate) struct JoinKeyComponent<'a> {
    pub(crate) role_id: &'a str,
    pub(crate) logical_type_id: &'a str,
    pub(crate) value: Option<&'a [u8]>,
}

pub(crate) fn join_key_tuple(
    object_type_id: u32,
    identity_rule_id: &str,
    components: &[JoinKeyComponent<'_>],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"COVE-MAP-JOIN-KEY-V1");
    out.extend_from_slice(&object_type_id.to_le_bytes());
    append_len_bytes(&mut out, identity_rule_id.as_bytes());
    out.extend_from_slice(&(components.len() as u32).to_le_bytes());
    for component in components {
        append_len_bytes(&mut out, component.role_id.as_bytes());
        append_len_bytes(&mut out, component.logical_type_id.as_bytes());
        match component.value {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                append_len_bytes(&mut out, value);
            }
        }
    }
    out
}

pub(crate) fn join_key_tuple_from_rule_with_context(
    rule: &MapIdentityRule,
    row: &SourceRow,
    object_type_id: u32,
    context: Option<&MappingContext>,
) -> Result<JoinKeyEvaluation, String> {
    let mut encoded_values = Vec::<Option<Vec<u8>>>::with_capacity(rule.join_keys.len());
    let mut resolution_metadata = Vec::new();
    let mut materializes_identity = true;
    let mut effective_confidence_class = None::<String>;
    for component in &rule.join_keys {
        let raw_value = row.values.get(&component.source_column);
        if raw_value.is_none() || matches!(raw_value, Some(Value::Null)) {
            if matches!(
                component.null_policy.as_str(),
                "reject" | "reject-null" | "all_components_required"
            ) {
                return Err(format!(
                    "identity rule '{}' rejected null/missing source column '{}'",
                    rule.rule_id, component.source_column
                ));
            }
            encoded_values.push(None);
            continue;
        }
        let value = if component.resolution.is_some() {
            let Some(context) = context else {
                return Err(format!(
                    "identity rule '{}' uses resolver-backed join key '{}' without resolution context",
                    rule.rule_id, component.role_id
                ));
            };
            let resolved = resolve_join_key_component(
                component,
                raw_value.unwrap(),
                &rule.object_type,
                context,
            )?;
            materializes_identity &= resolved.materializes_identity;
            if let Some(class) = &resolved.effective_confidence_class {
                effective_confidence_class = Some(match effective_confidence_class.take() {
                    Some(existing) => most_restrictive_confidence(&existing, class).to_string(),
                    None => class.clone(),
                });
            }
            resolution_metadata.push(resolved.metadata);
            Value::String(resolved.identity_value)
        } else {
            apply_canonicalization(
                raw_value.unwrap(),
                &component.canonicalization,
                &rule.function_ids,
            )?
        };
        encoded_values.push(Some(canonical_component_bytes(
            &component.logical_type,
            &value,
        )?));
    }
    let components = rule
        .join_keys
        .iter()
        .zip(encoded_values.iter())
        .map(|(component, bytes)| JoinKeyComponent {
            role_id: component.role_id.as_str(),
            logical_type_id: component.logical_type.as_str(),
            value: bytes.as_deref(),
        })
        .collect::<Vec<_>>();
    Ok(JoinKeyEvaluation {
        tuple: join_key_tuple(object_type_id, &rule.rule_id, &components),
        materializes_identity,
        effective_confidence_class,
        resolution_metadata,
    })
}

struct ResolvedJoinKeyComponent {
    identity_value: String,
    materializes_identity: bool,
    effective_confidence_class: Option<String>,
    metadata: ResolutionMetadata,
}

fn resolve_join_key_component(
    component: &MapJoinKeyComponent,
    raw_value: &Value,
    object_type: &str,
    context: &MappingContext,
) -> Result<ResolvedJoinKeyComponent, String> {
    let binding = component
        .resolution
        .as_ref()
        .ok_or_else(|| "missing resolution binding".to_string())?;
    let catalog = context
        .resolution_catalog
        .as_ref()
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG_MISSING: resolver-backed identity rule has no resolution catalog".to_string())?;
    let resolver = catalog
        .resolvers
        .iter()
        .find(|resolver| resolver.resolver_id == binding.resolver_id)
        .ok_or_else(|| {
            format!(
                "MAP_RESOLUTION_CATALOG_MISSING: identity rule references missing resolver '{}'",
                binding.resolver_id
            )
        })?;
    if resolver.kind != "alias_catalog" {
        return Err(format!(
            "MAP_RESOLVER_UNSUPPORTED: unsupported resolver kind '{}'",
            resolver.kind
        ));
    }
    if resolver.object_type != object_type {
        return Err(format!(
            "MAP_RESOLUTION_CATALOG_MISMATCH: resolver '{}' targets object type '{}' but identity rule targets '{}'",
            resolver.resolver_id, resolver.object_type, object_type
        ));
    }
    let pipeline = catalog
        .normalization_pipelines
        .iter()
        .find(|pipeline| pipeline.pipeline_id == resolver.normalization_pipeline_id)
        .ok_or_else(|| {
            format!(
                "MAP_PIPELINE_DIGEST_MISMATCH: resolver '{}' references missing pipeline '{}'",
                resolver.resolver_id, resolver.normalization_pipeline_id
            )
        })?;
    let raw_observed_value = string_arg(raw_value, "resolver alias lookup")?.to_string();
    let normalized_value = apply_resolution_pipeline(&raw_observed_value, pipeline)?;
    let alias_catalog = resolver.alias_catalog.as_ref().ok_or_else(|| {
        format!(
            "MAP_RESOLVER_UNSUPPORTED: resolver '{}' has no alias catalog",
            resolver.resolver_id
        )
    })?;
    let mut hits = Vec::<&MapAliasEntry>::new();
    for entry in &alias_catalog.entries {
        for alias in &entry.aliases {
            if apply_resolution_pipeline(alias, pipeline)? == normalized_value {
                hits.push(entry);
                break;
            }
        }
    }
    hits.sort_by_key(|entry| entry.alias_entry_id.clone());
    hits.dedup_by_key(|entry| entry.alias_entry_id.clone());

    if hits.is_empty() {
        return resolve_alias_miss(component, resolver, &raw_observed_value, &normalized_value);
    }

    let canonical_keys = hits
        .iter()
        .map(|entry| entry.canonical_key.as_str())
        .collect::<BTreeSet<_>>();
    let ambiguous = canonical_keys.len() > 1 || hits.iter().any(|entry| entry.ambiguous);
    if ambiguous {
        if resolver.ambiguous_policy == "candidate_only" {
            return Ok(ResolvedJoinKeyComponent {
                identity_value: normalized_value.clone(),
                materializes_identity: false,
                effective_confidence_class: Some("candidate_only".to_string()),
                metadata: resolution_metadata_base(
                    component,
                    resolver,
                    &raw_observed_value,
                    &normalized_value,
                    Some(normalized_value.clone()),
                )
                .with_alias_ambiguous(alias_catalog.alias_catalog_id.clone()),
            });
        }
        let display_value = resolver_error_observed_value(resolver, &normalized_value);
        return Err(format!(
            "MAP_ALIAS_AMBIGUOUS: normalized alias '{}' matched multiple canonical keys for resolver '{}'",
            display_value, resolver.resolver_id
        ));
    }

    let entry = hits[0];
    Ok(ResolvedJoinKeyComponent {
        identity_value: entry.canonical_key.clone(),
        materializes_identity: true,
        effective_confidence_class: Some(resolver.confidence_class.clone()),
        metadata: resolution_metadata_base(
            component,
            resolver,
            &raw_observed_value,
            &normalized_value,
            Some(entry.canonical_key.clone()),
        )
        .with_alias_hit(
            entry.canonical_key.clone(),
            entry.canonical_label.clone(),
            alias_catalog.alias_catalog_id.clone(),
            entry.alias_entry_id.clone(),
        ),
    })
}

fn resolve_alias_miss(
    component: &MapJoinKeyComponent,
    resolver: &MapResolver,
    raw_observed_value: &str,
    normalized_value: &str,
) -> Result<ResolvedJoinKeyComponent, String> {
    match resolver.on_miss.as_str() {
        "reject" => {
            let display_value = resolver_error_observed_value(resolver, raw_observed_value);
            Err(format!(
                "MAP_ALIAS_MISS: resolver '{}' did not match '{}'",
                resolver.resolver_id, display_value
            ))
        }
        "candidate_only" => Ok(ResolvedJoinKeyComponent {
            identity_value: normalized_value.to_string(),
            materializes_identity: false,
            effective_confidence_class: Some("candidate_only".to_string()),
            metadata: resolution_metadata_base(
                component,
                resolver,
                raw_observed_value,
                normalized_value,
                Some(normalized_value.to_string()),
            )
            .with_alias_miss(),
        }),
        "source_scoped" => Ok(ResolvedJoinKeyComponent {
            identity_value: normalized_value.to_string(),
            materializes_identity: true,
            effective_confidence_class: Some("source_scoped".to_string()),
            metadata: resolution_metadata_base(
                component,
                resolver,
                raw_observed_value,
                normalized_value,
                Some(normalized_value.to_string()),
            )
            .with_alias_miss(),
        }),
        "normalized_value" => {
            let class = resolver.miss_confidence_class.clone().ok_or_else(|| {
                format!(
                    "MAP_RESOLUTION_NOT_REPLAYABLE: resolver '{}' missing miss_confidence_class",
                    resolver.resolver_id
                )
            })?;
            Ok(ResolvedJoinKeyComponent {
                identity_value: normalized_value.to_string(),
                materializes_identity: true,
                effective_confidence_class: Some(class),
                metadata: resolution_metadata_base(
                    component,
                    resolver,
                    raw_observed_value,
                    normalized_value,
                    Some(normalized_value.to_string()),
                )
                .with_alias_miss(),
            })
        }
        other => Err(format!(
            "MAP_RESOLVER_UNSUPPORTED: unsupported resolver miss policy '{other}'"
        )),
    }
}

fn resolver_error_observed_value(resolver: &MapResolver, observed_value: &str) -> String {
    if resolver.evidence_policy == "redact_raw" {
        "<redacted>".to_string()
    } else {
        observed_value.to_string()
    }
}

fn resolution_metadata_base(
    component: &MapJoinKeyComponent,
    resolver: &MapResolver,
    raw_observed_value: &str,
    normalized_value: &str,
    resolved_identity_value: Option<String>,
) -> ResolutionMetadata {
    ResolutionMetadata {
        role_id: component.role_id.clone(),
        resolution_kind: resolver.kind.clone(),
        resolver_id: resolver.resolver_id.clone(),
        resolver_digest: resolver.resolver_digest.clone(),
        catalog_digest: resolver.catalog_digest.clone(),
        pipeline_digest: resolver.pipeline_digest.clone(),
        normalization_pipeline_id: resolver.normalization_pipeline_id.clone(),
        evidence_policy: resolver.evidence_policy.clone(),
        redacted_resolution_evidence: resolver.evidence_policy == "redact_raw",
        raw_observed_value: raw_observed_value.to_string(),
        normalized_value: normalized_value.to_string(),
        resolved_identity_value,
        canonical_key: None,
        canonical_label: None,
        alias_catalog_id: None,
        alias_entry_id: None,
        alias_hit: false,
        alias_miss: false,
        alias_ambiguous: false,
        miss_policy: Some(resolver.on_miss.clone()),
    }
}

impl ResolutionMetadata {
    fn with_alias_hit(
        mut self,
        canonical_key: String,
        canonical_label: String,
        alias_catalog_id: String,
        alias_entry_id: String,
    ) -> Self {
        self.canonical_key = Some(canonical_key);
        self.canonical_label = Some(canonical_label);
        self.alias_catalog_id = Some(alias_catalog_id);
        self.alias_entry_id = Some(alias_entry_id);
        self.alias_hit = true;
        self.miss_policy = None;
        self
    }

    fn with_alias_miss(mut self) -> Self {
        self.alias_miss = true;
        self
    }

    fn with_alias_ambiguous(mut self, alias_catalog_id: String) -> Self {
        self.alias_catalog_id = Some(alias_catalog_id);
        self.alias_ambiguous = true;
        self
    }
}

pub(crate) fn apply_resolution_pipeline(
    raw: &str,
    pipeline: &MapNormalizationPipeline,
) -> Result<String, String> {
    let mut value = raw.to_string();
    for function in &pipeline.functions {
        value = match function.function_id.as_str() {
            "identity" => value,
            "trim" => value.trim().to_string(),
            "unicode_nfkc" => {
                let normalizer = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc();
                normalizer.normalize(&value).into_owned()
            }
            "unicode_casefold" => {
                let case_mapper = icu_casemap::CaseMapper::new();
                case_mapper.fold_string(&value).into_owned()
            }
            "strip_punctuation" => value
                .chars()
                .filter(|ch| !ch.is_ascii_punctuation())
                .collect::<String>(),
            "collapse_whitespace" => value.split_whitespace().collect::<Vec<_>>().join(" "),
            "strip_legal_suffix" => strip_legal_suffix(&value, pipeline, function.table_id.as_deref())?,
            "sort_tokens" => {
                let mut tokens = value.split_whitespace().collect::<Vec<_>>();
                tokens.sort_unstable();
                tokens.join(" ")
            }
            other => {
                return Err(format!(
                    "MAP_RESOLVER_UNSUPPORTED: normalization function '{other}' is not implemented by resolver execution"
                ))
            }
        };
    }
    Ok(value)
}

fn strip_legal_suffix(
    value: &str,
    pipeline: &MapNormalizationPipeline,
    table_id: Option<&str>,
) -> Result<String, String> {
    let table_id = table_id.ok_or_else(|| {
        format!(
            "MAP_RESOLUTION_NOT_REPLAYABLE: strip_legal_suffix in pipeline '{}' has no table_id",
            pipeline.pipeline_id
        )
    })?;
    let table = pipeline
        .tables
        .iter()
        .find(|table| table.table_id == table_id)
        .ok_or_else(|| {
            format!(
                "MAP_RESOLUTION_NOT_REPLAYABLE: strip_legal_suffix references missing table '{table_id}'"
            )
        })?;
    let mut text = value.trim().to_string();
    loop {
        let mut changed = false;
        for suffix in &table.values {
            let suffix = suffix.trim();
            if suffix.is_empty() {
                continue;
            }
            if text == suffix {
                return Ok(text);
            }
            let Some(prefix) = text.strip_suffix(suffix) else {
                continue;
            };
            if prefix.is_empty() || !prefix.chars().next_back().is_some_and(char::is_whitespace) {
                continue;
            }
            text = prefix.trim_end().to_string();
            changed = true;
            break;
        }
        if !changed {
            return Ok(text);
        }
    }
}

fn most_restrictive_confidence(left: &str, right: &str) -> String {
    if identity_confidence_rank(left) >= identity_confidence_rank(right) {
        left.to_string()
    } else {
        right.to_string()
    }
}

fn identity_confidence_rank(class: &str) -> u8 {
    match class {
        "authoritative" | "reviewed_authoritative" => 0,
        "strong_deterministic" => 1,
        "source_scoped" => 2,
        "weak_deterministic" => 3,
        "candidate_only" | "candidate" => 4,
        _ => 5,
    }
}

pub(crate) fn apply_canonicalization(
    value: &Value,
    canonicalization: &str,
    declared_functions: &[String],
) -> Result<Value, String> {
    let function_id = if canonicalization == "none" {
        "identity"
    } else {
        canonicalization
    };
    if !declared_functions
        .iter()
        .any(|function| function == function_id || function == canonicalization)
    {
        return Err(format!(
            "canonicalization function '{canonicalization}' was not declared on the identity rule"
        ));
    }
    if !deterministic_builtin_function_ids().contains(&function_id) {
        return Err(format!(
            "canonicalization function '{canonicalization}' is not implemented by the deterministic reference runner"
        ));
    }
    match function_id {
        "identity" => Ok(value.clone()),
        "trim" => Ok(Value::String(string_arg(value, "trim")?.trim().to_string())),
        "ascii_lower" => Ok(Value::String(
            string_arg(value, "ascii_lower")?.to_ascii_lowercase(),
        )),
        "unicode_nfc" => {
            let text = string_arg(value, function_id)?;
            let normalizer = icu_normalizer::ComposingNormalizerBorrowed::new_nfc();
            Ok(Value::String(normalizer.normalize(text).into_owned()))
        }
        "unicode_nfkc" => {
            let text = string_arg(value, function_id)?;
            let normalizer = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc();
            Ok(Value::String(normalizer.normalize(text).into_owned()))
        }
        "unicode_casefold" => {
            let case_mapper = icu_casemap::CaseMapper::new();
            Ok(Value::String(
                case_mapper
                    .fold_string(string_arg(value, "unicode_casefold")?)
                    .into_owned(),
            ))
        }
        "trim_lower" => Ok(Value::String(
            string_arg(value, "trim_lower")?.trim().to_ascii_lowercase(),
        )),
        "concat_delimited" => {
            let items = value
                .as_array()
                .ok_or_else(|| "concat_delimited requires a JSON array".to_string())?;
            let mut out = Vec::new();
            for item in items {
                out.push(string_arg(item, "concat_delimited")?);
            }
            Ok(Value::String(out.join("|")))
        }
        "parse_int64" => {
            let text = string_arg(value, "parse_int64")?.trim();
            let parsed = text
                .parse::<i64>()
                .map_err(|_| "parse_int64 requires a base-10 int64 string".to_string())?;
            Ok(Value::Number(parsed.into()))
        }
        "parse_decimal" => {
            let text = string_arg(value, "parse_decimal")?.trim();
            validate_decimal_text(text)?;
            Ok(Value::String(text.to_string()))
        }
        "parse_timestamp_utc" => {
            let text = string_arg(value, "parse_timestamp_utc")?.trim();
            validate_utc_timestamp_text(text)?;
            Ok(Value::String(text.to_string()))
        }
        "sha256_hex" => Ok(Value::String(sha256_hex(
            string_arg(value, "sha256_hex")?.as_bytes(),
        ))),
        _ => unreachable!("registry membership checked above"),
    }
}

fn deterministic_builtin_function_ids() -> &'static [&'static str] {
    &[
        "identity",
        "trim",
        "ascii_lower",
        "unicode_nfc",
        "unicode_nfkc",
        "unicode_casefold",
        "trim_lower",
        "concat_delimited",
        "parse_int64",
        "parse_decimal",
        "parse_timestamp_utc",
        "sha256_hex",
    ]
}

fn string_arg<'a>(value: &'a Value, function_id: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{function_id} requires a string value"))
}

fn validate_decimal_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err("parse_decimal requires a non-empty decimal string".into());
    }
    let mut chars = text.chars();
    if matches!(chars.clone().next(), Some('+') | Some('-')) {
        chars.next();
    }
    let mut digits = 0usize;
    let mut dots = 0usize;
    for ch in chars {
        match ch {
            '0'..='9' => digits += 1,
            '.' => dots += 1,
            _ => return Err("parse_decimal only accepts base-10 decimal text".into()),
        }
    }
    if digits == 0 || dots > 1 {
        return Err("parse_decimal only accepts base-10 decimal text".into());
    }
    Ok(())
}

fn validate_utc_timestamp_text(text: &str) -> Result<(), String> {
    let has_utc_suffix = text.ends_with('Z') || text.ends_with("+00:00");
    if has_utc_suffix && text.contains('T') {
        Ok(())
    } else {
        Err("parse_timestamp_utc requires an ISO-8601 UTC timestamp".into())
    }
}

pub(crate) fn canonical_component_bytes(
    logical_type: &str,
    value: &Value,
) -> Result<Vec<u8>, String> {
    let canonical = match logical_type {
        "bool" | "boolean" => CanonicalValue::Bool(
            value
                .as_bool()
                .ok_or_else(|| "bool join key value must be JSON bool".to_string())?,
        ),
        "int64" | "int" => CanonicalValue::Int {
            width: 8,
            value: json_i64(value)? as i128,
        },
        "uint64" | "uint" => CanonicalValue::Uint {
            width: 8,
            value: json_u64(value)? as u128,
        },
        "float64" => CanonicalValue::Float64(json_f64(value)?),
        "utf8" | "string" => CanonicalValue::Utf8(
            value
                .as_str()
                .ok_or_else(|| "utf8 join key value must be JSON string".to_string())?,
        ),
        "binary" => CanonicalValue::Bytes(
            value
                .as_str()
                .ok_or_else(|| "binary join key value must be encoded as a string".to_string())?
                .as_bytes(),
        ),
        other => {
            return Err(format!(
                "logical type '{other}' is not supported in COVE-MAP join keys"
            ))
        }
    };
    canonical.encode().map_err(|err| err.to_string())
}

pub(crate) fn mapped_goid(
    mapping_id: &[u8],
    mapping_version: &[u8],
    object_type_id: u32,
    anchor_kind: &[u8],
    anchor_bytes: &[u8],
    source_scope: Option<&str>,
) -> [u8; 16] {
    let object_type_id = object_type_id.to_le_bytes();
    let source_scope = source_scope.unwrap_or("").as_bytes();
    goid16_parts(&[
        mapping_id,
        mapping_version,
        &object_type_id,
        anchor_kind,
        anchor_bytes,
        source_scope,
    ])
}

pub(crate) fn goid16_parts(parts: &[&[u8]]) -> [u8; 16] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}
