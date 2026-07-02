use super::*;

pub(super) fn temporal_for_as_of(
    bound: &AstTimeBound,
    resolve_options: &ResolveOptions,
) -> Result<TemporalContext, BuildResolvedQueryError> {
    match bound {
        AstTimeBound::Csn(csn) => Ok(TemporalContext {
            mode: TemporalMode::AsOfCsn(*csn),
            role: TemporalRole::CommitTime,
            role_binding: None,
        }),
        AstTimeBound::Timestamp { role, timestamp } => {
            let (role, role_binding) = temporal_role(*role, resolve_options);
            Ok(TemporalContext {
                mode: TemporalMode::AsOfTimestampMicros(
                    timestamp_micros(timestamp, &resolve_options.security)?.0,
                ),
                role,
                role_binding,
            })
        }
    }
}

pub(super) fn temporal_role(
    role: AstTimeRole,
    resolve_options: &ResolveOptions,
) -> (TemporalRole, Option<String>) {
    let resolved = match role {
        AstTimeRole::Time | AstTimeRole::CommitTime => {
            return (TemporalRole::CommitTime, None);
        }
        AstTimeRole::AssociationValidTime => {
            return (TemporalRole::AssociationValidTime, None);
        }
        AstTimeRole::ValidTime => TemporalRole::ValidTime,
        AstTimeRole::ObservedTime => TemporalRole::ObservedTime,
        AstTimeRole::SourceEventTime => TemporalRole::SourceEventTime,
    };
    let binding = resolve_options
        .temporal_role_bindings
        .contains_key(&resolved)
        .then(|| resolve_options.temporal_role_bindings[&resolved].clone());
    (resolved, binding)
}

pub(super) fn branch_for_selector(
    selector: &AstBranchSelector,
    resolve_options: &ResolveOptions,
) -> Result<BranchContext, BuildResolvedQueryError> {
    let ambiguous_alias = |name: &str| {
        resolve_options
            .ambiguous_branch_aliases
            .get(name)
            .is_some_and(|aliases| aliases.len() > 1)
    };
    let ambiguous_error = || {
        BuildResolvedQueryError::single(diagnostic(
            "E_AMBIGUOUS_BRANCH",
            "branch alias is ambiguous for selected query scope",
            "resolve",
            &resolve_options.security,
        ))
    };
    let branch = match selector {
        AstBranchSelector::UInt(value) => BranchSelector::BranchKey(*value),
        AstBranchSelector::Identifier(identifier) if identifier.name == "default" => {
            BranchSelector::Default
        }
        AstBranchSelector::Identifier(identifier) if identifier.name == "reject_ambiguous" => {
            BranchSelector::RejectAmbiguous
        }
        AstBranchSelector::String(value) if value == "default" => BranchSelector::Default,
        AstBranchSelector::String(value) if value == "reject_ambiguous" => {
            BranchSelector::RejectAmbiguous
        }
        AstBranchSelector::Identifier(identifier) => resolve_options
            .branch_aliases
            .get(&identifier.name)
            .copied()
            .map(BranchSelector::BranchKey)
            .ok_or_else(|| {
                if ambiguous_alias(&identifier.name) {
                    ambiguous_error()
                } else {
                    BuildResolvedQueryError::single(diagnostic(
                        "E_UNKNOWN_BRANCH",
                        "unknown branch alias for selected query scope",
                        "resolve",
                        &resolve_options.security,
                    ))
                }
            })?,
        AstBranchSelector::String(value) => resolve_options
            .branch_aliases
            .get(value)
            .copied()
            .map(BranchSelector::BranchKey)
            .ok_or_else(|| {
                if ambiguous_alias(value) {
                    ambiguous_error()
                } else {
                    BuildResolvedQueryError::single(diagnostic(
                        "E_UNKNOWN_BRANCH",
                        "unknown branch alias for selected query scope",
                        "resolve",
                        &resolve_options.security,
                    ))
                }
            })?,
    };
    Ok(BranchContext { selector: branch })
}

pub(super) fn temporal_role_inference_names(role: TemporalRole) -> &'static [&'static str] {
    match role {
        TemporalRole::ValidTime => &["valid_time"],
        TemporalRole::ObservedTime => &["observed_time"],
        TemporalRole::SourceEventTime => &["source_event_time", "event_time"],
        TemporalRole::CommitTime | TemporalRole::AssociationValidTime => &[],
    }
}

impl Resolver {
    pub(super) fn resolve_temporal_context(
        &self,
        mut temporal: TemporalContext,
        root: &ResolvedRoot,
    ) -> Result<TemporalContext, BuildResolvedQueryError> {
        if temporal.role_binding.is_some()
            || matches!(
                temporal.role,
                TemporalRole::CommitTime | TemporalRole::AssociationValidTime
            )
        {
            return Ok(temporal);
        }
        if let Some(binding) = self.infer_temporal_role_binding(temporal.role, root)? {
            temporal.role_binding = Some(binding);
            return Ok(temporal);
        }
        Err(BuildResolvedQueryError::single(diagnostic(
            "E_UNSUPPORTED_TEMPORAL_ROLE",
            format!(
                "{:?} requires an exact temporal role binding in ResolveOptions or an unambiguous timestamp property on the root object type",
                temporal.role
            ),
            "resolve",
            &self.options.security,
        )))
    }

    pub(super) fn infer_temporal_role_binding(
        &self,
        role: TemporalRole,
        root: &ResolvedRoot,
    ) -> Result<Option<String>, BuildResolvedQueryError> {
        let ResolvedRoot::Object(root) = root else {
            return Ok(None);
        };
        let Some(object_type) = self
            .surface
            .object_types
            .iter()
            .find(|object_type| object_type.object_type_id == root.object_type_id)
        else {
            return Ok(None);
        };
        let names = temporal_role_inference_names(role);
        let matches = object_type
            .properties
            .iter()
            .filter(|property| {
                names.contains(&property.property_name.as_str())
                    && logical_type_name(property.logical_type) == "timestamp_micros"
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [property] => Ok(Some(property.property_name.clone())),
            _ => Err(BuildResolvedQueryError::single(diagnostic(
                "E_AMBIGUOUS_TEMPORAL_ROLE",
                format!(
                    "{role:?} matches multiple timestamp properties; pass an explicit temporal role binding"
                ),
                "resolve",
                &self.options.security,
            ))),
        }
    }

    pub(super) fn resolve_change_bound(
        &self,
        bound: &AstChangeBound,
    ) -> Result<ResolvedTimeBound, BuildResolvedQueryError> {
        match bound {
            AstChangeBound::Csn(value) => Ok(ResolvedTimeBound::Csn(*value)),
            AstChangeBound::Timestamp { role, timestamp } => {
                let (role, binding) = temporal_role(*role, &self.options);
                let (timestamp_micros, canonical_rfc3339) =
                    timestamp_micros(timestamp, &self.options.security)?;
                Ok(ResolvedTimeBound::TimestampMicros {
                    role,
                    binding,
                    timestamp_micros,
                    canonical_rfc3339,
                })
            }
        }
    }
}
