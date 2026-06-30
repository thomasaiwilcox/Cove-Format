use super::*;

pub(super) fn resolved_object_root(object_type: &ObjectTypeEntryV1) -> ResolvedObjectRoot {
    ResolvedObjectRoot {
        object_type_id: object_type.object_type_id,
        type_name: object_type.type_name.clone(),
        flags: object_type.flags,
    }
}

pub(super) fn contextual_evidence_target(root: &ResolvedRoot) -> ResolvedEvidenceTarget {
    match root {
        ResolvedRoot::Object(object) => ResolvedEvidenceTarget::ObjectType {
            object_type_id: object.object_type_id,
            type_name: object.type_name.clone(),
        },
        ResolvedRoot::Association(association) => ResolvedEvidenceTarget::AssociationType {
            object_type_id: association.object_type_id,
            type_name: association.type_name.clone(),
        },
        ResolvedRoot::Node(node) => ResolvedEvidenceTarget::GraphNode {
            object_type_id: node.object.object_type_id,
            type_name: node.object.type_name.clone(),
            label: node
                .binding_name
                .clone()
                .unwrap_or_else(|| node.label.clone()),
        },
        ResolvedRoot::Edge(edge) => ResolvedEvidenceTarget::GraphEdge {
            object_type_id: edge.association.object_type_id,
            type_name: edge.association.type_name.clone(),
            label: edge
                .binding_name
                .clone()
                .unwrap_or_else(|| edge.label.clone()),
        },
        ResolvedRoot::Table(table) => ResolvedEvidenceTarget::TableRow {
            table_id: table.table_id.clone(),
            table_name: table.table_name.clone(),
            projection_id: table.projection.projection_id.clone(),
        },
        ResolvedRoot::Projection(projection) => ResolvedEvidenceTarget::Projection {
            projection_id: projection.projection_id.clone(),
        },
        ResolvedRoot::Evidence(_) => ResolvedEvidenceTarget::CurrentRoot,
    }
}

pub(super) fn object_type_is_association_like(object_type: &ObjectTypeEntryV1) -> bool {
    object_type.flags & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT) != 0
}

impl Resolver {
    pub(super) fn resolve_root(
        &self,
        root: &AstRoot,
    ) -> Result<ResolvedRoot, BuildResolvedQueryError> {
        match root {
            AstRoot::Object(identifier) => self
                .resolve_object_type(&identifier.name)
                .map(|object| ResolvedRoot::Object(resolved_object_root(object))),
            AstRoot::Association(association) => self
                .resolve_association_root(association, None, AssociationResolveUsage::RootScan)
                .map(ResolvedRoot::Association),
            AstRoot::Node(identifier) => self
                .resolve_graph_node_root(identifier)
                .map(ResolvedRoot::Node),
            AstRoot::Edge(edge) => self.resolve_graph_edge_root(edge).map(ResolvedRoot::Edge),
            AstRoot::Projection(identifier) => self
                .resolve_projection_root(&identifier.name)
                .map(ResolvedRoot::Projection),
            AstRoot::Table(identifier) => self
                .resolve_table_root(&identifier.name)
                .map(ResolvedRoot::Table),
            AstRoot::Evidence(evidence) => self
                .resolve_evidence_root(evidence, None)
                .map(ResolvedRoot::Evidence),
            AstRoot::Path(path) => self.resolve_path_root_start(path),
        }
    }

    pub(super) fn resolve_path_root_start(
        &self,
        path: &AstPathRootExpr,
    ) -> Result<ResolvedRoot, BuildResolvedQueryError> {
        let mut root = self.resolve_root(&path.start.root.node)?;
        apply_root_alias(&mut root, path.start.alias.as_ref());
        if !matches!(root, ResolvedRoot::Node(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "path roots require a node(...) start",
                &self.options,
            ));
        }
        Ok(root)
    }

    pub(super) fn resolve_object_type(
        &self,
        name: &str,
    ) -> Result<&ObjectTypeEntryV1, BuildResolvedQueryError> {
        let matches = self
            .surface
            .object_types
            .iter()
            .filter(|object_type| object_type.type_name == name)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [object_type] => Ok(*object_type),
            [] => Err(self.unknown("E_UNKNOWN_OBJECT_TYPE", "unknown object type", Some(name))),
            _ => Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "object type name is ambiguous",
                Some(name),
            )),
        }
    }

    pub(super) fn resolve_association_root(
        &self,
        association: &AstAssociationExpr,
        contextual_root: Option<&ResolvedRoot>,
        usage: AssociationResolveUsage,
    ) -> Result<ResolvedAssociationRoot, BuildResolvedQueryError> {
        let object_type = self.resolve_object_type(&association.type_name.name)?;
        if !object_type_is_association_like(object_type) {
            return Err(self.unknown(
                "E_UNKNOWN_ASSOCIATION_TYPE",
                "object type is not declared as an association/link type",
                Some(&association.type_name.name),
            ));
        }
        let source_property_id =
            property_id_by_flag(&object_type.properties, PROPERTY_FLAG_ASSOCIATION_FROM_GOID);
        let target_property_id =
            property_id_by_flag(&object_type.properties, PROPERTY_FLAG_ASSOCIATION_TO_GOID);
        let object_relative =
            usage == AssociationResolveUsage::ObjectRelative && contextual_root.is_some();
        let endpoint_role = self.resolve_association_endpoint_role(
            association,
            source_property_id,
            target_property_id,
            object_relative,
        )?;
        Ok(ResolvedAssociationRoot {
            object_type_id: object_type.object_type_id,
            type_name: object_type.type_name.clone(),
            flags: object_type.flags,
            source_property_id,
            target_property_id,
            association_type_property_id: property_id_by_flag(
                &object_type.properties,
                PROPERTY_FLAG_ASSOCIATION_TYPE,
            ),
            valid_from_property_id: property_id_by_flag(
                &object_type.properties,
                PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
            ),
            valid_to_property_id: property_id_by_flag(
                &object_type.properties,
                PROPERTY_FLAG_ASSOCIATION_VALID_TO,
            ),
            direction: association.direction,
            role: association.role_name.as_ref().map(|role| role.name.clone()),
            endpoint_role,
            disclosure_outcome: association_disclosure_outcome(
                endpoint_role,
                &self.options.security,
            ),
            object_relative,
            target_node_object_type_id: None,
            target_node_label: None,
        })
    }

    pub(super) fn resolve_relationship_association(
        &self,
        relationship: &AstRelationshipExpr,
        root: &ResolvedRoot,
    ) -> Result<ResolvedAssociationRoot, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Node(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "relationship expressions require a CoveQL/Graph node or path root grain",
                &self.options,
            ));
        }
        let association = AstAssociationExpr {
            type_name: relationship.edge.type_name.clone(),
            direction: Some(relationship.direction),
            role: relationship.edge.role,
            role_name: relationship.edge.role_name.clone(),
        };
        let mut resolved = self.resolve_association_root(
            &association,
            Some(root),
            AssociationResolveUsage::ObjectRelative,
        )?;
        if let Some(target) = &relationship.target {
            let mut target_root = self.resolve_root(&target.root.node)?;
            apply_root_alias(&mut target_root, target.alias.as_ref());
            let ResolvedRoot::Node(node) = target_root else {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "relationship targets require node(...) bindings",
                    &self.options,
                ));
            };
            resolved.target_node_object_type_id = Some(node.object.object_type_id);
            resolved.target_node_label = Some(node.label);
        }
        Ok(resolved)
    }

    pub(super) fn resolve_projection_root(
        &self,
        projection_id: &str,
    ) -> Result<ResolvedProjectionRoot, BuildResolvedQueryError> {
        let catalog = self.projection_catalog()?;
        let entry = catalog
            .projections
            .iter()
            .find(|entry| entry.projection_id == projection_id)
            .ok_or_else(|| {
                self.unknown(
                    "E_UNKNOWN_PROJECTION",
                    "unknown projection root",
                    Some(projection_id),
                )
            })?;
        let anchor_object = entry
            .anchor
            .as_ref()
            .and_then(|anchor| anchor.object_type.as_deref())
            .and_then(|object_type| self.resolve_object_type(object_type).ok());
        Ok(ResolvedProjectionRoot {
            projection_id: entry.projection_id.clone(),
            mapping_id: catalog.mapping_id.clone(),
            mapping_version: catalog.mapping_version.clone(),
            output_table: entry.output_table.clone(),
            row_grain: entry.row_grain.clone(),
            anchor: entry
                .anchor
                .as_ref()
                .map(|anchor| ResolvedProjectionAnchor {
                    object_type: anchor.object_type.clone(),
                    association_type: anchor.association_type.clone(),
                }),
            temporal_mode: entry.temporal_mode.clone(),
            columns: entry
                .columns
                .iter()
                .map(|column| ResolvedProjectionColumn {
                    name: column.name.clone(),
                    value: column.value.clone(),
                    logical_type: column.logical_type.clone(),
                    nested_shape: column.nested_shape.clone(),
                    conflict_policy: column.conflict_policy.clone(),
                    missing_policy: column.missing_policy.clone(),
                    source_property_id: anchor_object.and_then(|object_type| {
                        projection_column_source_property_id(column.value.as_str(), object_type)
                    }),
                })
                .collect(),
            assertion_ids: entry.assertion_ids.clone(),
            multi_value_policy: entry.multi_value_policy.clone(),
            missing_policy: entry.missing_policy.clone(),
            ordering: entry.ordering.clone(),
            evidence_policy: entry.evidence_policy.clone(),
            output_modes: entry.output_modes.clone(),
            column_count: entry.columns.len(),
        })
    }

    pub(super) fn resolve_evidence_root(
        &self,
        evidence: &AstEvidenceExpr,
        contextual_root: Option<&ResolvedRoot>,
    ) -> Result<ResolvedEvidenceRoot, BuildResolvedQueryError> {
        let target = match &evidence.target {
            Some(target) => Some(self.resolve_evidence_target(target, contextual_root)?),
            None => contextual_root.map(contextual_evidence_target),
        };
        let grain = evidence.grain.unwrap_or(match target {
            Some(ResolvedEvidenceTarget::AssociationType { .. }) => AstEvidenceGrain::Association,
            Some(ResolvedEvidenceTarget::Projection { .. }) => AstEvidenceGrain::Row,
            Some(ResolvedEvidenceTarget::TableRow { .. }) => AstEvidenceGrain::Row,
            Some(ResolvedEvidenceTarget::TableColumn { .. }) => AstEvidenceGrain::Column,
            Some(ResolvedEvidenceTarget::GraphNode { .. }) => AstEvidenceGrain::Node,
            Some(ResolvedEvidenceTarget::GraphEdge { .. }) => AstEvidenceGrain::Edge,
            Some(ResolvedEvidenceTarget::GraphPath { .. }) => AstEvidenceGrain::Path,
            Some(ResolvedEvidenceTarget::Property { .. }) => AstEvidenceGrain::Property,
            Some(ResolvedEvidenceTarget::CurrentRoot) => match contextual_root {
                Some(ResolvedRoot::Association(_)) => AstEvidenceGrain::Association,
                Some(ResolvedRoot::Node(_)) => AstEvidenceGrain::Node,
                Some(ResolvedRoot::Edge(_)) => AstEvidenceGrain::Edge,
                Some(ResolvedRoot::Table(_)) => AstEvidenceGrain::Row,
                Some(ResolvedRoot::Projection(_)) => AstEvidenceGrain::Row,
                _ => AstEvidenceGrain::Object,
            },
            _ => AstEvidenceGrain::Object,
        });
        if self.surface.evidence_index.is_none() {
            return Err(self.unknown(
                "E_MISSING_METADATA",
                "evidence roots and helpers require COVE-MAP evidence metadata",
                None,
            ));
        }
        let (mapping_id, mapping_version) = self
            .surface
            .evidence_index
            .as_ref()
            .map(|index| {
                (
                    Some(index.mapping_id.clone()),
                    Some(index.mapping_version.clone()),
                )
            })
            .unwrap_or((None, None));
        Ok(ResolvedEvidenceRoot {
            target,
            grain,
            mapping_id,
            mapping_version,
        })
    }

    pub(super) fn resolve_evidence_target(
        &self,
        target: &AstEvidenceTarget,
        contextual_root: Option<&ResolvedRoot>,
    ) -> Result<ResolvedEvidenceTarget, BuildResolvedQueryError> {
        match target {
            AstEvidenceTarget::SelfTarget => Ok(contextual_root
                .map(contextual_evidence_target)
                .unwrap_or(ResolvedEvidenceTarget::CurrentRoot)),
            AstEvidenceTarget::Path(path) => {
                if path.parts.len() == 1 {
                    let name = &path.parts[0].name;
                    if let Ok(object_type) = self.resolve_object_type(name) {
                        return Ok(ResolvedEvidenceTarget::ObjectType {
                            object_type_id: object_type.object_type_id,
                            type_name: object_type.type_name.clone(),
                        });
                    }
                }
                if path.parts.len() == 2 {
                    if let Ok(object_type) = self.resolve_object_type(&path.parts[0].name) {
                        let property_name = &path.parts[1].name;
                        let property =
                            find_property(object_type, property_name).ok_or_else(|| {
                                self.unknown(
                                    "E_UNKNOWN_PROPERTY",
                                    "unknown object property for evidence target",
                                    Some(property_name),
                                )
                            })?;
                        return Ok(ResolvedEvidenceTarget::Property {
                            object_type_id: Some(object_type.object_type_id),
                            property_id: property.property_id,
                            property_name: property.property_name.clone(),
                        });
                    }
                }
                let root = contextual_root.ok_or_else(|| {
                    self.unknown(
                        "E_UNSUPPORTED_CONSTRUCT",
                        "property evidence target requires a contextual root",
                        None,
                    )
                })?;
                let resolved = self.resolve_path(path, root)?;
                if let ResolvedRoot::Table(table) = root {
                    return Ok(ResolvedEvidenceTarget::TableColumn {
                        table_id: table.table_id.clone(),
                        table_name: table.table_name.clone(),
                        projection_id: table.projection.projection_id.clone(),
                        column_name: resolved.display_name,
                    });
                }
                Ok(ResolvedEvidenceTarget::Property {
                    object_type_id: resolved.object_type_id,
                    property_id: resolved.property_id.unwrap_or_default(),
                    property_name: resolved.display_name,
                })
            }
            AstEvidenceTarget::RootBinding(binding) => {
                if let AstRoot::Path(path) = &binding.root.node {
                    let mut root = self.resolve_path_root_start(path)?;
                    apply_root_alias(&mut root, binding.alias.as_ref());
                    if let ResolvedRoot::Node(node) = root {
                        return Ok(ResolvedEvidenceTarget::GraphPath {
                            start_object_type_id: node.object.object_type_id,
                            start_label: node.binding_name.unwrap_or(node.label),
                        });
                    }
                }
                let mut root = self.resolve_root(&binding.root.node)?;
                apply_root_alias(&mut root, binding.alias.as_ref());
                Ok(contextual_evidence_target(&root))
            }
            AstEvidenceTarget::Association(association) => {
                let resolved = self.resolve_association_root(
                    association,
                    contextual_root,
                    AssociationResolveUsage::EvidenceTarget,
                )?;
                Ok(ResolvedEvidenceTarget::AssociationType {
                    object_type_id: resolved.object_type_id,
                    type_name: resolved.type_name,
                })
            }
            AstEvidenceTarget::Projection(projection) => {
                let resolved = self.resolve_projection_root(&projection.name)?;
                Ok(ResolvedEvidenceTarget::Projection {
                    projection_id: resolved.projection_id,
                })
            }
        }
    }

    pub(super) fn projection_catalog(
        &self,
    ) -> Result<&MapProjectionCatalog, BuildResolvedQueryError> {
        self.surface.projection_catalog.as_ref().ok_or_else(|| {
            self.unknown(
                "E_MISSING_METADATA",
                "projection roots require COVE-MAP projection metadata",
                None,
            )
        })
    }

    pub(super) fn evidence_metadata_key_exists(&self, key: &str) -> bool {
        self.surface
            .evidence_index
            .as_ref()
            .map(|index| evidence_key_exists(index, key))
            .unwrap_or(false)
    }

    pub(super) fn unknown(
        &self,
        code: &'static str,
        public_message: &'static str,
        protected_name: Option<&str>,
    ) -> BuildResolvedQueryError {
        let mut diag = diagnostic(code, public_message, "resolve", &self.options.security);
        if self.options.security.metadata_disclosure_policy
            == MetadataDisclosurePolicy::AllowProtected
        {
            if let Some(name) = protected_name {
                diag.message = format!("{public_message}: {name}");
                diag.safe_details = json!({ "name": name });
                diag.redacted = false;
            }
        }
        BuildResolvedQueryError::single(diag)
    }
}
