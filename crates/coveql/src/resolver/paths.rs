use super::*;

pub(super) fn association_disclosure_outcome(
    endpoint_role: AssociationEndpointRole,
    security: &SecurityContext,
) -> AssociationDisclosureOutcome {
    if endpoint_role == AssociationEndpointRole::Unknown {
        return AssociationDisclosureOutcome::ProtectedEndpoint;
    }
    if security.metadata_disclosure_policy == MetadataDisclosurePolicy::AllowProtected {
        AssociationDisclosureOutcome::Public
    } else {
        AssociationDisclosureOutcome::ProtectedEndpoint
    }
}

pub(super) fn find_property<'a>(
    object_type: &'a ObjectTypeEntryV1,
    name: &str,
) -> Option<&'a PropertyEntryV1> {
    object_type
        .properties
        .iter()
        .find(|property| property.property_name == name)
}

pub(super) fn projection_column_source_property_id(
    value: &str,
    object_type: &ObjectTypeEntryV1,
) -> Option<u32> {
    let property_name = value.strip_prefix("property.")?;
    (!property_name.is_empty() && !property_name.contains('.'))
        .then_some(property_name)
        .and_then(|property_name| find_property(object_type, property_name))
        .map(|property| property.property_id)
}

pub(super) fn property_id_by_flag(properties: &[PropertyEntryV1], flag: u32) -> Option<u32> {
    properties
        .iter()
        .find(|property| property.flags & flag != 0)
        .map(|property| property.property_id)
}

pub(super) fn object_system_field(field: &str) -> Option<ResolvedSystemField> {
    match field {
        "goid" => Some(ResolvedSystemField::Goid),
        "object_type" => Some(ResolvedSystemField::ObjectType),
        "branch_key" => Some(ResolvedSystemField::BranchKey),
        "timestamp_us" => Some(ResolvedSystemField::TimestampUs),
        "csn" => Some(ResolvedSystemField::Csn),
        "record_kind" => Some(ResolvedSystemField::RecordKind),
        _ => None,
    }
}

pub(super) fn association_system_field(field: &str) -> Option<ResolvedSystemField> {
    match field {
        "source_goid" => Some(ResolvedSystemField::SourceGoid),
        "target_goid" => Some(ResolvedSystemField::TargetGoid),
        "association_type" => Some(ResolvedSystemField::AssociationType),
        "valid_from" => Some(ResolvedSystemField::ValidFrom),
        "valid_to" => Some(ResolvedSystemField::ValidTo),
        other => object_system_field(other),
    }
}

pub(super) fn evidence_system_field(field: &str) -> Option<ResolvedSystemField> {
    match field {
        "source_id" => Some(ResolvedSystemField::SourceId),
        "source_row_identity" => Some(ResolvedSystemField::SourceRowIdentity),
        "rule_id" => Some(ResolvedSystemField::RuleId),
        "assertion_id" => Some(ResolvedSystemField::AssertionId),
        "output_object_id" => Some(ResolvedSystemField::OutputObjectId),
        "observed_schema_fingerprint" => Some(ResolvedSystemField::ObservedSchemaFingerprint),
        "observed_snapshot_digest" => Some(ResolvedSystemField::ObservedSnapshotDigest),
        _ => None,
    }
}

pub(super) fn system_resolved_path(
    field: &str,
    root_kind: ResolvedPathRootKind,
    object_type_id: Option<u32>,
    system: ResolvedSystemField,
) -> ResolvedPath {
    let (logical_type, physical_kind, nullable, temporal_role) = match system {
        ResolvedSystemField::Goid
        | ResolvedSystemField::SourceGoid
        | ResolvedSystemField::TargetGoid => ("uuid", "fixed_bytes", false, None),
        ResolvedSystemField::BranchKey | ResolvedSystemField::Csn => {
            ("uint64", "num_code", false, None)
        }
        ResolvedSystemField::TimestampUs
        | ResolvedSystemField::ValidFrom
        | ResolvedSystemField::ValidTo => (
            "timestamp_micros",
            "num_code",
            false,
            Some(match system {
                ResolvedSystemField::ValidFrom | ResolvedSystemField::ValidTo => {
                    TemporalRole::AssociationValidTime
                }
                _ => TemporalRole::CommitTime,
            }),
        ),
        ResolvedSystemField::ObservedSchemaFingerprint
        | ResolvedSystemField::ObservedSnapshotDigest => ("utf8", "var_bytes", true, None),
        _ => ("utf8", "var_bytes", false, None),
    };
    ResolvedPath {
        display_name: field.into(),
        root_kind,
        object_type_id,
        property_id: None,
        association_type_id: if root_kind == ResolvedPathRootKind::Association {
            object_type_id
        } else {
            None
        },
        evidence_field_id: if root_kind == ResolvedPathRootKind::Evidence {
            Some(field.into())
        } else {
            None
        },
        projection_id: None,
        projection_column: None,
        system_field: Some(system),
        logical_type: logical_type.into(),
        physical_kind: physical_kind.into(),
        collation_id: None,
        nullable,
        null_policy: if nullable { "nullable" } else { "not_null" }.into(),
        temporal_role,
        code_domain_id: CodeDomainId::Placeholder {
            root: format!("{root_kind:?}").to_ascii_lowercase(),
            object_type_id,
            property_id: None,
            projection_id: None,
            field: Some(field.into()),
        },
    }
}

pub(super) fn property_resolved_path(
    field: &str,
    root_kind: ResolvedPathRootKind,
    object_type_id: Option<u32>,
    association_type_id: Option<u32>,
    property: &PropertyEntryV1,
) -> ResolvedPath {
    ResolvedPath {
        display_name: field.into(),
        root_kind,
        object_type_id,
        property_id: Some(property.property_id),
        association_type_id,
        evidence_field_id: None,
        projection_id: None,
        projection_column: None,
        system_field: None,
        logical_type: logical_type_name(property.logical_type).into(),
        physical_kind: physical_kind_name(property.physical_kind).into(),
        collation_id: Some(property.collation_id),
        nullable: property.nullable,
        null_policy: if property.nullable {
            "nullable"
        } else {
            "not_null"
        }
        .into(),
        temporal_role: None,
        code_domain_id: CodeDomainId::Placeholder {
            root: format!("{root_kind:?}").to_ascii_lowercase(),
            object_type_id,
            property_id: Some(property.property_id),
            projection_id: None,
            field: Some(field.into()),
        },
    }
}

pub(super) fn path_names(path: &AstPath) -> Vec<String> {
    path.parts.iter().map(|part| part.name.clone()).collect()
}

pub(super) fn path_without_graph_binding(
    path: &AstPath,
    label: &str,
    alias: Option<&str>,
) -> Result<AstPath, BuildResolvedQueryError> {
    let parts = path_names(path);
    if parts.len() <= 1 {
        return Ok(path.clone());
    }
    let binding = &parts[0];
    if binding == label || alias.is_some_and(|alias| binding == alias) {
        return Ok(AstPath {
            parts: path.parts[1..].to_vec(),
        });
    }
    Err(BuildResolvedQueryError::single(diagnostic(
        "E_BINDING_OUT_OF_SCOPE",
        "qualified graph path references a binding that is not in scope",
        "resolve",
        &SecurityContext::default(),
    )))
}

pub(super) fn evidence_key_exists(index: &MapEvidenceIndex, key: &str) -> bool {
    index
        .entries
        .iter()
        .any(|entry| entry.operation_metadata.contains_key(key))
}

pub(super) fn physical_kind_name(physical: CovePhysicalKind) -> &'static str {
    match physical {
        CovePhysicalKind::FileCode => "file_code",
        CovePhysicalKind::NumCode => "num_code",
        CovePhysicalKind::Boolean => "boolean",
        CovePhysicalKind::FixedBytes => "fixed_bytes",
        CovePhysicalKind::VarBytes => "var_bytes",
        CovePhysicalKind::List => "list",
        CovePhysicalKind::Struct => "struct",
        CovePhysicalKind::Map => "map",
        _ => "unknown",
    }
}

pub(super) fn physical_kind_for_logical_name(logical: &str) -> &'static str {
    match logical {
        "bool" | "boolean" => "boolean",
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => "num_code",
        "uuid" | "decimal128" => "fixed_bytes",
        "list" => "list",
        "struct" => "struct",
        "map" => "map",
        _ => "var_bytes",
    }
}

impl Resolver {
    pub(super) fn resolve_path(
        &self,
        path: &AstPath,
        root: &ResolvedRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        match root {
            ResolvedRoot::Object(object) => self.resolve_object_path(path, object),
            ResolvedRoot::Association(association) => {
                self.resolve_association_path(path, association)
            }
            ResolvedRoot::Node(node) => self.resolve_graph_path_with_scope(path, node),
            ResolvedRoot::Edge(edge) => self.resolve_graph_edge_path(path, edge),
            ResolvedRoot::Table(table) => self.resolve_table_path_with_scope(path, table),
            ResolvedRoot::Projection(projection) => self.resolve_projection_path(path, projection),
            ResolvedRoot::Evidence(evidence) => self.resolve_evidence_path(path, evidence),
        }
    }

    pub(super) fn resolve_object_path(
        &self,
        path: &AstPath,
        object: &ResolvedObjectRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 2 && parts[0] == object.type_name {
            parts[1].as_str()
        } else if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "path is ambiguous for selected object root",
                Some(&parts.join(".")),
            ));
        };
        if let Some(system) = object_system_field(field) {
            return Ok(system_resolved_path(
                field,
                ResolvedPathRootKind::Object,
                Some(object.object_type_id),
                system,
            ));
        }
        let object_type = self.resolve_object_type(&object.type_name)?;
        let property = find_property(object_type, field).ok_or_else(|| {
            self.unknown(
                "E_UNKNOWN_PROPERTY",
                "unknown object property for selected root",
                Some(field),
            )
        })?;
        Ok(property_resolved_path(
            field,
            ResolvedPathRootKind::Object,
            Some(object.object_type_id),
            None,
            property,
        ))
    }

    pub(super) fn resolve_association_path(
        &self,
        path: &AstPath,
        association: &ResolvedAssociationRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "association path is ambiguous",
                Some(&parts.join(".")),
            ));
        };
        if let Some(system) = association_system_field(field) {
            return Ok(system_resolved_path(
                field,
                ResolvedPathRootKind::Association,
                Some(association.object_type_id),
                system,
            ));
        }
        let object_type = self.resolve_object_type(&association.type_name)?;
        let property = find_property(object_type, field).ok_or_else(|| {
            self.unknown(
                "E_UNKNOWN_PROPERTY",
                "unknown association property for selected root",
                Some(field),
            )
        })?;
        Ok(property_resolved_path(
            field,
            ResolvedPathRootKind::Association,
            Some(association.object_type_id),
            Some(association.object_type_id),
            property,
        ))
    }

    pub(super) fn resolve_graph_node_path(
        &self,
        path: &AstPath,
        node: &ResolvedGraphNodeRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        if let [field] = parts.as_slice() {
            if let Some(logical_type) = graph_algorithm_output_logical_type(field) {
                return Ok(ResolvedPath {
                    display_name: field.to_string(),
                    root_kind: ResolvedPathRootKind::Node,
                    object_type_id: None,
                    property_id: None,
                    association_type_id: None,
                    evidence_field_id: None,
                    projection_id: None,
                    projection_column: Some(field.to_string()),
                    system_field: None,
                    logical_type: logical_type.into(),
                    physical_kind: physical_kind_for_logical_name(logical_type).into(),
                    collation_id: None,
                    nullable: true,
                    null_policy: "generated_algorithm_field".into(),
                    temporal_role: None,
                    code_domain_id: CodeDomainId::Placeholder {
                        root: "graph_algorithm".into(),
                        object_type_id: None,
                        property_id: None,
                        projection_id: None,
                        field: Some(field.to_string()),
                    },
                });
            }
        }
        let path = path_without_graph_binding(path, &node.label, node.binding_name.as_deref())?;
        let mut resolved = self.resolve_object_path(&path, &node.object)?;
        resolved.root_kind = ResolvedPathRootKind::Node;
        if let [binding, field] = parts.as_slice() {
            resolved.projection_column = Some(format!("{binding}.{field}"));
        }
        let CodeDomainId::Placeholder { root, .. } = &mut resolved.code_domain_id;
        *root = "node".into();
        Ok(resolved)
    }

    pub(super) fn resolve_graph_edge_path(
        &self,
        path: &AstPath,
        edge: &ResolvedGraphEdgeRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let path = path_without_graph_binding(path, &edge.label, edge.binding_name.as_deref())?;
        let mut resolved = self.resolve_association_path(&path, &edge.association)?;
        resolved.root_kind = ResolvedPathRootKind::Edge;
        if let [binding, field] = parts.as_slice() {
            resolved.projection_column = Some(format!("{binding}.{field}"));
        }
        let CodeDomainId::Placeholder { root, .. } = &mut resolved.code_domain_id;
        *root = "edge".into();
        Ok(resolved)
    }

    pub(super) fn resolve_graph_path_with_scope(
        &self,
        path: &AstPath,
        node: &ResolvedGraphNodeRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        if let [binding, ..] = parts.as_slice() {
            if binding == &node.label
                || node
                    .binding_name
                    .as_ref()
                    .is_some_and(|alias| binding == alias)
            {
                return self.resolve_graph_node_path(path, node);
            }
            if let Some(scoped_node) = self.graph_node_scope.iter().find(|scoped| {
                binding == &scoped.label
                    || scoped
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias)
            }) {
                return self.resolve_graph_node_path(path, scoped_node);
            }
            if let Some(scoped_edge) = self.graph_edge_scope.iter().find(|scoped| {
                binding == &scoped.label
                    || scoped
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias)
            }) {
                return self.resolve_graph_edge_path(path, scoped_edge);
            }
        }
        self.resolve_graph_node_path(path, node)
    }

    pub(super) fn resolve_projection_path(
        &self,
        path: &AstPath,
        projection: &ResolvedProjectionRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "projection path is ambiguous",
                Some(&parts.join(".")),
            ));
        };
        let catalog = self.projection_catalog()?;
        let entry = catalog
            .projections
            .iter()
            .find(|entry| entry.projection_id == projection.projection_id)
            .ok_or_else(|| {
                self.unknown(
                    "E_UNKNOWN_PROJECTION",
                    "unknown projection",
                    Some(&projection.projection_id),
                )
            })?;
        let column = entry
            .columns
            .iter()
            .find(|column| column.name == field)
            .ok_or_else(|| {
                self.unknown("E_UNKNOWN_PATH", "unknown projection column", Some(field))
            })?;
        let logical = column.logical_type.as_deref().unwrap_or("utf8");
        Ok(ResolvedPath {
            display_name: field.into(),
            root_kind: ResolvedPathRootKind::Projection,
            object_type_id: None,
            property_id: None,
            association_type_id: None,
            evidence_field_id: None,
            projection_id: Some(projection.projection_id.clone()),
            projection_column: Some(field.into()),
            system_field: None,
            logical_type: logical.into(),
            physical_kind: physical_kind_for_logical_name(logical).into(),
            collation_id: None,
            nullable: column.missing_policy != "reject",
            null_policy: column.missing_policy.clone(),
            temporal_role: None,
            code_domain_id: CodeDomainId::Placeholder {
                root: "projection".into(),
                object_type_id: None,
                property_id: None,
                projection_id: Some(projection.projection_id.clone()),
                field: Some(field.into()),
            },
        })
    }

    pub(super) fn resolve_table_path_with_scope(
        &self,
        path: &AstPath,
        table: &ResolvedTableRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        if let [binding, ..] = parts.as_slice() {
            if binding == &table.table_name
                || table
                    .binding_name
                    .as_ref()
                    .is_some_and(|alias| binding == alias)
            {
                return self.resolve_table_path_for_table(path, table);
            }
            if let Some(scoped) = self.lookup_scope.iter().find(|scoped| {
                binding == &scoped.table_name
                    || scoped
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias)
            }) {
                return self.resolve_table_path_for_table(path, scoped);
            }
        }
        self.resolve_table_path_for_table(path, table)
    }

    pub(super) fn resolve_table_path_for_table(
        &self,
        path: &AstPath,
        table: &ResolvedTableRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let (field, qualified_binding) = match parts.as_slice() {
            [field] => (field.as_str(), None),
            [binding, field]
                if binding == &table.table_name
                    || table
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias) =>
            {
                (field.as_str(), Some(binding.as_str()))
            }
            _ => {
                return Err(self.unknown(
                    "E_AMBIGUOUS_PATH",
                    "table path is ambiguous",
                    Some(&parts.join(".")),
                ));
            }
        };
        let column = table
            .projection
            .columns
            .iter()
            .find(|column| column.name == field)
            .ok_or_else(|| self.unknown("E_UNKNOWN_PATH", "unknown table column", Some(field)))?;
        let logical = column.logical_type.as_deref().unwrap_or("utf8");
        let value_key = qualified_binding
            .map(|binding| format!("{binding}.{field}"))
            .unwrap_or_else(|| field.to_string());
        Ok(ResolvedPath {
            display_name: field.into(),
            root_kind: ResolvedPathRootKind::Table,
            object_type_id: None,
            property_id: None,
            association_type_id: None,
            evidence_field_id: None,
            projection_id: Some(table.projection.projection_id.clone()),
            projection_column: Some(value_key.clone()),
            system_field: None,
            logical_type: logical.into(),
            physical_kind: physical_kind_for_logical_name(logical).into(),
            collation_id: None,
            nullable: column.missing_policy != "reject",
            null_policy: column.missing_policy.clone(),
            temporal_role: None,
            code_domain_id: CodeDomainId::Placeholder {
                root: "table".into(),
                object_type_id: None,
                property_id: None,
                projection_id: Some(table.projection.projection_id.clone()),
                field: Some(value_key),
            },
        })
    }

    pub(super) fn resolve_evidence_path(
        &self,
        path: &AstPath,
        _evidence: &ResolvedEvidenceRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "evidence path is ambiguous",
                Some(&parts.join(".")),
            ));
        };
        if let Some(system) = evidence_system_field(field) {
            return Ok(system_resolved_path(
                field,
                ResolvedPathRootKind::Evidence,
                None,
                system,
            ));
        }
        if self.evidence_metadata_key_exists(field) {
            return Ok(ResolvedPath {
                display_name: field.into(),
                root_kind: ResolvedPathRootKind::Evidence,
                object_type_id: None,
                property_id: None,
                association_type_id: None,
                evidence_field_id: Some(field.into()),
                projection_id: None,
                projection_column: None,
                system_field: None,
                logical_type: "json".into(),
                physical_kind: "var_bytes".into(),
                collation_id: None,
                nullable: true,
                null_policy: "missing_is_null".into(),
                temporal_role: None,
                code_domain_id: CodeDomainId::Placeholder {
                    root: "evidence".into(),
                    object_type_id: None,
                    property_id: None,
                    projection_id: None,
                    field: Some(field.into()),
                },
            });
        }
        Err(self.unknown("E_UNKNOWN_PATH", "unknown evidence field", Some(field)))
    }

    pub(super) fn resolve_literal(
        &self,
        literal: &AstLiteral,
    ) -> Result<ResolvedLiteral, BuildResolvedQueryError> {
        match literal {
            AstLiteral::Null => Ok(ResolvedLiteral {
                literal: literal.clone(),
                logical_type: "null".into(),
                canonical: "null".into(),
                typed_value: ResolvedLiteralValue::Null,
                precision: None,
                scale: None,
            }),
            AstLiteral::Boolean(value) => Ok(ResolvedLiteral {
                literal: literal.clone(),
                logical_type: "bool".into(),
                canonical: value.to_string(),
                typed_value: ResolvedLiteralValue::Boolean(*value),
                precision: None,
                scale: None,
            }),
            AstLiteral::String(value) => Ok(ResolvedLiteral {
                literal: literal.clone(),
                logical_type: "utf8".into(),
                canonical: value.clone(),
                typed_value: ResolvedLiteralValue::String(value.clone()),
                precision: None,
                scale: None,
            }),
            AstLiteral::Integer(value) => {
                let parsed = value.parse::<i128>().map_err(|_| {
                    BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "integer literal is out of range",
                        "resolve",
                        &self.options.security,
                    ))
                })?;
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: if parsed < 0 { "int64" } else { "uint64" }.into(),
                    canonical: parsed.to_string(),
                    typed_value: integer_literal_value(parsed),
                    precision: Some(value.trim_start_matches('-').len() as u32),
                    scale: Some(0),
                })
            }
            AstLiteral::Decimal(value) => {
                let unsigned = value.trim_start_matches('-');
                let mut split = unsigned.split('.');
                let whole = split.next().unwrap_or("");
                let fractional = split.next().unwrap_or("");
                if split.next().is_some() {
                    return Err(BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "malformed decimal literal",
                        "resolve",
                        &self.options.security,
                    )));
                }
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "decimal128".into(),
                    canonical: value.clone(),
                    typed_value: ResolvedLiteralValue::Decimal {
                        canonical: value.clone(),
                        precision: (whole.len() + fractional.len()) as u32,
                        scale: fractional.len() as u32,
                    },
                    precision: Some((whole.len() + fractional.len()) as u32),
                    scale: Some(fractional.len() as u32),
                })
            }
            AstLiteral::Timestamp(value) => {
                let (micros, canonical) = timestamp_micros(value, &self.options.security)?;
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "timestamp_micros".into(),
                    canonical: format!("{canonical}:{micros}"),
                    typed_value: ResolvedLiteralValue::TimestampMicros {
                        micros,
                        canonical_rfc3339: canonical,
                    },
                    precision: None,
                    scale: None,
                })
            }
            AstLiteral::Uuid(value) => {
                let Some((bytes, canonical_hex)) = parse_uuid_literal(value) else {
                    return Err(BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "malformed UUID literal",
                        "resolve",
                        &self.options.security,
                    )));
                };
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "uuid".into(),
                    canonical: canonical_hex.clone(),
                    typed_value: ResolvedLiteralValue::Uuid {
                        canonical_hex,
                        bytes,
                    },
                    precision: None,
                    scale: None,
                })
            }
            AstLiteral::Binary(value) => {
                let Some(bytes) = decode_hex_bytes(value) else {
                    return Err(BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "malformed binary literal",
                        "resolve",
                        &self.options.security,
                    )));
                };
                let canonical_hex = value.to_ascii_lowercase();
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "binary".into(),
                    canonical: canonical_hex.clone(),
                    typed_value: ResolvedLiteralValue::Binary {
                        canonical_hex,
                        bytes,
                    },
                    precision: None,
                    scale: None,
                })
            }
        }
    }
}
