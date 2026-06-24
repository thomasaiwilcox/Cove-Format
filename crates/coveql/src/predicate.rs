use crate::{
    AstCompareOp, CodeDomainId, FunctionExecutionClass, ResolvedExpr, ResolvedFunctionContract,
    ResolvedLiteral, ResolvedLiteralValue, ResolvedPath, ResolvedPathRootKind, ResolvedPredicate,
    ResolvedRoot, ResolvedSystemField,
};
use cove_core::collation::CollationKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicatePlacement {
    PreReconstruction,
    PostReconstruction,
    Visibility,
    Redaction,
    Association,
    Evidence,
    Residual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterClassification {
    System,
    ObjectType,
    Temporal,
    Branch,
    Tombstone,
    PropertyCodedCandidate,
    PropertyResidual,
    AssociationSemiJoin,
    EvidenceResidual,
    Aggregate,
    ResidualMaterialized,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepresentationClass {
    CodePure,
    TypedNumeric,
    DictionaryLiftedCandidate,
    OrdinalMapCandidate,
    DecodeBoundary,
    CrossSourceBridgeCandidate,
    ResidualMaterialized,
    NonBeneficial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateProofState {
    ProvenExact,
    CandidateNeedsResidual,
    DecodeRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepresentationContract {
    pub contract_version: String,
    pub representation: RepresentationClass,
    pub logical_type: Option<String>,
    pub physical_kind: Option<String>,
    pub collation_id: Option<u16>,
    pub null_policy: Option<String>,
    pub code_domain_id: Option<CodeDomainId>,
    pub security_scope: String,
    pub exact: bool,
    pub proof_state: PredicateProofState,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum LogicalPredicateKind {
    Compare {
        op: AstCompareOp,
        left: String,
        right: String,
    },
    InList {
        expr: String,
        literal_count: usize,
    },
    NullCheck {
        expr: String,
        negated: bool,
    },
    Exists {
        expr: String,
    },
    BoolExpr {
        expr: String,
    },
    Not(Box<LogicalPredicateForm>),
    And(Vec<LogicalPredicateForm>),
    Or(Vec<LogicalPredicateForm>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalPredicateForm {
    pub kind: LogicalPredicateKind,
    pub placement: PredicatePlacement,
    pub classification: FilterClassification,
    pub representation: RepresentationContract,
    pub residual_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredicatePlanning {
    pub all_forms: Vec<LogicalPredicateForm>,
    pub pre_reconstruction: Vec<LogicalPredicateForm>,
    pub post_reconstruction: Vec<LogicalPredicateForm>,
    pub association: Vec<LogicalPredicateForm>,
    pub evidence: Vec<LogicalPredicateForm>,
    pub residual: Vec<LogicalPredicateForm>,
    pub decode_boundaries: Vec<String>,
}

impl PredicatePlanning {
    pub(crate) fn from_predicate_with_dataset(
        predicate: Option<&ResolvedPredicate>,
        root: &ResolvedRoot,
        security_scope: String,
        dataset: &crate::DatasetScopeContext,
    ) -> Self {
        let Some(predicate) = predicate else {
            return Self::default();
        };
        let mut planning = Self::default();
        let form = classify_predicate_for_dataset(predicate, root, &security_scope, dataset);
        planning.record(form);
        planning
    }

    fn record(&mut self, form: LogicalPredicateForm) {
        match &form.kind {
            LogicalPredicateKind::And(parts) => {
                for part in parts {
                    self.record_leaf(part.clone());
                }
            }
            LogicalPredicateKind::Or(_) | LogicalPredicateKind::Not(_) => {
                self.record_leaf(form.clone());
            }
            _ => self.record_leaf(form.clone()),
        }
        self.all_forms.push(form);
    }

    fn record_leaf(&mut self, form: LogicalPredicateForm) {
        if form.representation.representation == RepresentationClass::DecodeBoundary {
            self.decode_boundaries.push(
                form.residual_reason
                    .clone()
                    .unwrap_or_else(|| form.representation.reason.clone()),
            );
        }
        match form.placement {
            PredicatePlacement::PreReconstruction => self.pre_reconstruction.push(form),
            PredicatePlacement::PostReconstruction => self.post_reconstruction.push(form),
            PredicatePlacement::Association => self.association.push(form),
            PredicatePlacement::Evidence => self.evidence.push(form),
            PredicatePlacement::Visibility
            | PredicatePlacement::Redaction
            | PredicatePlacement::Residual => self.residual.push(form),
        }
    }
}

pub fn classify_predicate(
    predicate: &ResolvedPredicate,
    root: &ResolvedRoot,
    security_scope: &str,
) -> LogicalPredicateForm {
    classify_predicate_scoped(predicate, root, security_scope, None)
}

pub(crate) fn classify_predicate_for_dataset(
    predicate: &ResolvedPredicate,
    root: &ResolvedRoot,
    security_scope: &str,
    dataset: &crate::DatasetScopeContext,
) -> LogicalPredicateForm {
    classify_predicate_scoped(predicate, root, security_scope, Some(dataset))
}

fn classify_predicate_scoped(
    predicate: &ResolvedPredicate,
    root: &ResolvedRoot,
    security_scope: &str,
    dataset: Option<&crate::DatasetScopeContext>,
) -> LogicalPredicateForm {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            let (placement, classification, representation, residual_reason) =
                classify_expr_pair(left, right, *op, root, security_scope, dataset);
            LogicalPredicateForm {
                kind: LogicalPredicateKind::Compare {
                    op: *op,
                    left: expr_label(left),
                    right: expr_label(right),
                },
                placement,
                classification,
                representation,
                residual_reason,
            }
        }
        ResolvedPredicate::InList { expr, values } => {
            let (placement, classification, representation, residual_reason) =
                classify_single_expr(expr, root, security_scope, dataset);
            LogicalPredicateForm {
                kind: LogicalPredicateKind::InList {
                    expr: expr_label(expr),
                    literal_count: values.len(),
                },
                placement,
                classification,
                representation,
                residual_reason,
            }
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let (placement, classification, representation, residual_reason) =
                classify_null_check_expr(expr, root, security_scope, dataset);
            LogicalPredicateForm {
                kind: LogicalPredicateKind::NullCheck {
                    expr: expr_label(expr),
                    negated: *negated,
                },
                placement,
                classification,
                representation,
                residual_reason,
            }
        }
        ResolvedPredicate::Exists(expr) => {
            let (placement, classification, representation, residual_reason) =
                if matches!(expr, ResolvedExpr::Association(_)) {
                    (
                        PredicatePlacement::Association,
                        FilterClassification::AssociationSemiJoin,
                        generic_contract(
                            RepresentationClass::ResidualMaterialized,
                            false,
                            "association existence is a logical semi-join boundary",
                            security_scope,
                        ),
                        None,
                    )
                } else if matches!(expr, ResolvedExpr::Evidence(_)) {
                    (
                        PredicatePlacement::Evidence,
                        FilterClassification::EvidenceResidual,
                        generic_contract(
                            RepresentationClass::ResidualMaterialized,
                            false,
                            "evidence existence is planned after visibility/redaction",
                            security_scope,
                        ),
                        Some("evidence existence can disclose protected lineage".into()),
                    )
                } else {
                    (
                        PredicatePlacement::Residual,
                        FilterClassification::ResidualMaterialized,
                        generic_contract(
                            RepresentationClass::ResidualMaterialized,
                            false,
                            "exists over non-association expression is residual",
                            security_scope,
                        ),
                        Some("unsupported exists expression".into()),
                    )
                };
            LogicalPredicateForm {
                kind: LogicalPredicateKind::Exists {
                    expr: expr_label(expr),
                },
                placement,
                classification,
                representation,
                residual_reason,
            }
        }
        ResolvedPredicate::BoolExpr(expr) => {
            let (placement, classification, representation, residual_reason) =
                classify_single_expr(expr, root, security_scope, dataset);
            LogicalPredicateForm {
                kind: LogicalPredicateKind::BoolExpr {
                    expr: expr_label(expr),
                },
                placement,
                classification,
                representation,
                residual_reason,
            }
        }
        ResolvedPredicate::Not(inner) => {
            let child = classify_predicate_scoped(inner, root, security_scope, dataset);
            if child.placement == PredicatePlacement::PreReconstruction
                && child.representation.exact
                && child.residual_reason.is_none()
            {
                let classification = child.classification;
                let mut representation = child.representation.clone();
                representation.reason =
                    "NOT complement over an exact pre-reconstruction predicate preserves CoveQL three-valued truth semantics".into();
                return LogicalPredicateForm {
                    kind: LogicalPredicateKind::Not(Box::new(child)),
                    placement: PredicatePlacement::PreReconstruction,
                    classification,
                    representation,
                    residual_reason: None,
                };
            }
            LogicalPredicateForm {
                kind: LogicalPredicateKind::Not(Box::new(child)),
                placement: PredicatePlacement::Residual,
                classification: FilterClassification::ResidualMaterialized,
                representation: generic_contract(
                    RepresentationClass::ResidualMaterialized,
                    false,
                    "NOT is conservative residual unless complement semantics are proven",
                    security_scope,
                ),
                residual_reason: Some(
                    "NOT complement requires an exact child predicate proof".into(),
                ),
            }
        }
        ResolvedPredicate::And(parts) => {
            let children = parts
                .iter()
                .map(|part| classify_predicate_scoped(part, root, security_scope, dataset))
                .collect::<Vec<_>>();
            let placement = if children
                .iter()
                .all(|child| child.placement == PredicatePlacement::PreReconstruction)
            {
                PredicatePlacement::PreReconstruction
            } else {
                PredicatePlacement::Residual
            };
            LogicalPredicateForm {
                kind: LogicalPredicateKind::And(children),
                placement,
                classification: FilterClassification::None,
                representation: generic_contract(
                    RepresentationClass::NonBeneficial,
                    false,
                    "AND is decomposed into child predicate placements",
                    security_scope,
                ),
                residual_reason: None,
            }
        }
        ResolvedPredicate::Or(parts) => {
            let children = parts
                .iter()
                .map(|part| classify_predicate_scoped(part, root, security_scope, dataset))
                .collect::<Vec<_>>();
            if let Some(path) = goid_or_candidate_path(parts) {
                return LogicalPredicateForm {
                    kind: LogicalPredicateKind::Or(children),
                    placement: PredicatePlacement::PreReconstruction,
                    classification: FilterClassification::System,
                    representation: path_contract(
                        path,
                        RepresentationClass::CodePure,
                        false,
                        "GOID OR can form a no-false-negative candidate union before materialized verification",
                        security_scope,
                    ),
                    residual_reason: Some(
                        "GOID OR candidate union still requires materialized CoveQL truth verification"
                            .into(),
                    ),
                };
            }
            if let Some((classification, representation)) =
                exact_same_path_or_contract(parts, root, security_scope, dataset)
            {
                return LogicalPredicateForm {
                    kind: LogicalPredicateKind::Or(children),
                    placement: PredicatePlacement::PreReconstruction,
                    classification,
                    representation,
                    residual_reason: None,
                };
            }
            if let Some((classification, representation)) =
                exact_pre_reconstruction_or_contract(&children, security_scope)
            {
                return LogicalPredicateForm {
                    kind: LogicalPredicateKind::Or(children),
                    placement: PredicatePlacement::PreReconstruction,
                    classification,
                    representation,
                    residual_reason: None,
                };
            }
            LogicalPredicateForm {
                kind: LogicalPredicateKind::Or(children),
                placement: PredicatePlacement::Residual,
                classification: FilterClassification::ResidualMaterialized,
                representation: generic_contract(
                    RepresentationClass::ResidualMaterialized,
                    false,
                    "OR requires compatible coverage proof before pruning",
                    security_scope,
                ),
                residual_reason: Some(
                    "OR requires compatible exact child predicates or a coverage-safe union proof"
                        .into(),
                ),
            }
        }
    }
}

fn exact_same_path_or_contract(
    parts: &[ResolvedPredicate],
    root: &ResolvedRoot,
    security_scope: &str,
    dataset: Option<&crate::DatasetScopeContext>,
) -> Option<(FilterClassification, RepresentationContract)> {
    let mut candidate_path = None::<&ResolvedPath>;
    for part in parts {
        let path = positive_same_path_equality_path(part)?;
        if let Some(existing) = candidate_path {
            if existing != path {
                return None;
            }
        } else {
            candidate_path = Some(path);
        }
    }
    let path = candidate_path?;
    let (placement, classification, representation, residual_reason) =
        classify_path_predicate(path, AstCompareOp::Eq, root, security_scope, dataset);
    (placement == PredicatePlacement::PreReconstruction
        && representation.exact
        && residual_reason.is_none())
    .then_some((
        classification,
        RepresentationContract {
            reason: "same-path equality/IN disjunction is equivalent to one CoveQL IN predicate and preserves three-valued null semantics".into(),
            ..representation
        },
    ))
}

fn exact_pre_reconstruction_or_contract(
    children: &[LogicalPredicateForm],
    security_scope: &str,
) -> Option<(FilterClassification, RepresentationContract)> {
    if children.is_empty()
        || !children.iter().all(|child| {
            child.placement == PredicatePlacement::PreReconstruction
                && child.representation.exact
                && child.representation.proof_state == PredicateProofState::ProvenExact
                && child.residual_reason.is_none()
        })
    {
        return None;
    }
    let first_classification = children.first()?.classification;
    let classification = if children
        .iter()
        .all(|child| child.classification == first_classification)
    {
        first_classification
    } else {
        FilterClassification::None
    };
    let representation = if children
        .iter()
        .all(|child| child.representation.representation == RepresentationClass::TypedNumeric)
    {
        RepresentationClass::TypedNumeric
    } else {
        RepresentationClass::CodePure
    };
    Some((
        classification,
        generic_contract(
            representation,
            true,
            "OR of exact pre-reconstruction predicates is an exact encoded disjunction; child contracts carry field, type, null, and code-domain proof details",
            security_scope,
        ),
    ))
}

fn positive_same_path_equality_path(predicate: &ResolvedPredicate) -> Option<&ResolvedPath> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } if *op == AstCompareOp::Eq => {
            path_non_null_literal_compare_path(left, right)
        }
        ResolvedPredicate::InList { expr, values } => {
            let ResolvedExpr::Path(path) = expr else {
                return None;
            };
            (!values.is_empty() && values.iter().all(is_non_null_literal)).then_some(path)
        }
        _ => None,
    }
}

fn path_non_null_literal_compare_path<'a>(
    left: &'a ResolvedExpr,
    right: &'a ResolvedExpr,
) -> Option<&'a ResolvedPath> {
    match (left, right) {
        (ResolvedExpr::Path(path), ResolvedExpr::Literal(literal))
            if is_non_null_literal(literal) =>
        {
            Some(path)
        }
        (ResolvedExpr::Literal(literal), ResolvedExpr::Path(path))
            if is_non_null_literal(literal) =>
        {
            Some(path)
        }
        _ => None,
    }
}

fn is_non_null_literal(literal: &ResolvedLiteral) -> bool {
    !matches!(literal.typed_value, ResolvedLiteralValue::Null)
}

fn goid_or_candidate_path(parts: &[ResolvedPredicate]) -> Option<&ResolvedPath> {
    let mut candidate_path = None;
    for part in parts {
        let path = positive_goid_candidate_path(part)?;
        candidate_path.get_or_insert(path);
    }
    candidate_path
}

fn positive_goid_candidate_path(predicate: &ResolvedPredicate) -> Option<&ResolvedPath> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } if *op == AstCompareOp::Eq => {
            goid_literal_compare_path(left, right)
        }
        ResolvedPredicate::InList { expr, values } => {
            let ResolvedExpr::Path(path) = expr else {
                return None;
            };
            if is_goid_path(path) && !values.is_empty() && values.iter().all(is_uuid_literal) {
                Some(path)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn goid_literal_compare_path<'a>(
    left: &'a ResolvedExpr,
    right: &'a ResolvedExpr,
) -> Option<&'a ResolvedPath> {
    match (left, right) {
        (ResolvedExpr::Path(path), ResolvedExpr::Literal(literal))
            if is_goid_path(path) && is_uuid_literal(literal) =>
        {
            Some(path)
        }
        (ResolvedExpr::Literal(literal), ResolvedExpr::Path(path))
            if is_goid_path(path) && is_uuid_literal(literal) =>
        {
            Some(path)
        }
        _ => None,
    }
}

fn is_goid_path(path: &ResolvedPath) -> bool {
    path.system_field.as_ref() == Some(&ResolvedSystemField::Goid)
}

fn is_uuid_literal(literal: &ResolvedLiteral) -> bool {
    matches!(literal.typed_value, ResolvedLiteralValue::Uuid { .. })
}

fn classify_expr_pair(
    left: &ResolvedExpr,
    right: &ResolvedExpr,
    op: AstCompareOp,
    root: &ResolvedRoot,
    security_scope: &str,
    dataset: Option<&crate::DatasetScopeContext>,
) -> (
    PredicatePlacement,
    FilterClassification,
    RepresentationContract,
    Option<String>,
) {
    if let Some(classification) = classify_coded_function_compare(left, right, op, security_scope) {
        return classification;
    }
    if let Some(path) = path_operand(left).or_else(|| path_operand(right)) {
        classify_path_predicate(path, op, root, security_scope, dataset)
    } else {
        (
            PredicatePlacement::Residual,
            FilterClassification::ResidualMaterialized,
            generic_contract(
                RepresentationClass::ResidualMaterialized,
                false,
                "comparison has no directly plannable path operand",
                security_scope,
            ),
            Some("comparison requires materialized expression evaluation".into()),
        )
    }
}

fn classify_single_expr(
    expr: &ResolvedExpr,
    root: &ResolvedRoot,
    security_scope: &str,
    dataset: Option<&crate::DatasetScopeContext>,
) -> (
    PredicatePlacement,
    FilterClassification,
    RepresentationContract,
    Option<String>,
) {
    if let Some(classification) = classify_coded_bool_function_expr(expr, security_scope) {
        return classification;
    }
    if let Some(path) = path_operand(expr) {
        classify_path_predicate(path, AstCompareOp::Eq, root, security_scope, dataset)
    } else {
        (
            PredicatePlacement::Residual,
            FilterClassification::ResidualMaterialized,
            generic_contract(
                RepresentationClass::ResidualMaterialized,
                false,
                "expression is not a directly plannable path",
                security_scope,
            ),
            Some("expression requires materialized evaluation".into()),
        )
    }
}

fn classify_coded_function_compare(
    left: &ResolvedExpr,
    right: &ResolvedExpr,
    op: AstCompareOp,
    security_scope: &str,
) -> Option<(
    PredicatePlacement,
    FilterClassification,
    RepresentationContract,
    Option<String>,
)> {
    let (function, literal) = match (left, right) {
        (function @ ResolvedExpr::FunctionCall { .. }, ResolvedExpr::Literal(literal))
        | (ResolvedExpr::Literal(literal), function @ ResolvedExpr::FunctionCall { .. }) => {
            (function, literal)
        }
        _ => return None,
    };
    let (path, representation, reason) = coded_function_compare_contract(function, op, literal)?;
    let contract = match path {
        Some(path) => path_contract(path, representation, true, reason, security_scope),
        None => generic_contract(representation, true, reason, security_scope),
    };
    Some((
        PredicatePlacement::PreReconstruction,
        FilterClassification::PropertyCodedCandidate,
        contract,
        None,
    ))
}

fn classify_coded_bool_function_expr(
    expr: &ResolvedExpr,
    security_scope: &str,
) -> Option<(
    PredicatePlacement,
    FilterClassification,
    RepresentationContract,
    Option<String>,
)> {
    let (path, representation, reason) = coded_bool_function_contract(expr)?;
    let contract = match path {
        Some(path) => path_contract(path, representation, true, reason, security_scope),
        None => generic_contract(representation, true, reason, security_scope),
    };
    Some((
        PredicatePlacement::PreReconstruction,
        FilterClassification::PropertyCodedCandidate,
        contract,
        None,
    ))
}

fn coded_function_compare_contract<'a>(
    expr: &'a ResolvedExpr,
    op: AstCompareOp,
    literal: &ResolvedLiteral,
) -> Option<(Option<&'a ResolvedPath>, RepresentationClass, &'static str)> {
    let ResolvedExpr::FunctionCall {
        function_id,
        deterministic,
        contract,
        args,
        ..
    } = expr
    else {
        return None;
    };
    if !(*deterministic && function_contract_is_coded_safe(contract)) {
        return None;
    }
    match function_id.as_str() {
        "startsWith" if bool_literal(literal).is_some() => {
            let path = starts_with_path_arg(args)?;
            Some((
                Some(path),
                RepresentationClass::DictionaryLiftedCandidate,
                "startsWith comparison uses a deterministic COVE-MAP coded function contract over a direct string path",
            ))
        }
        "length" if integer_literal(literal).is_some() && comparison_has_order_or_equality(op) => {
            let path = single_string_path_arg(args)?;
            Some((
                Some(path),
                RepresentationClass::DictionaryLiftedCandidate,
                "length comparison uses a deterministic COVE-MAP coded function contract over a direct string path",
            ))
        }
        "lower" | "lowercase" | "upper" | "uppercase" | "trim"
            if string_literal(literal).is_some() =>
        {
            let path = single_string_path_arg(args)?;
            Some((
                Some(path),
                RepresentationClass::DictionaryLiftedCandidate,
                "string scalar comparison uses a deterministic COVE-MAP coded function contract with declared Unicode/collation semantics",
            ))
        }
        "isNull" | "isNotNull" if bool_literal(literal).is_some() => {
            let path = single_non_execution_path_arg(args)?;
            Some((
                Some(path),
                RepresentationClass::CodePure,
                "null-check function comparison uses validity/null lanes with CoveQL two-valued function semantics",
            ))
        }
        "identity" if bool_literal(literal).is_some() => {
            let path = single_bool_path_arg(args)?;
            Some((
                Some(path),
                RepresentationClass::CodePure,
                "identity boolean comparison preserves the direct boolean lane and null policy",
            ))
        }
        "cast" if bool_literal(literal).is_some() => {
            let path = identity_cast_bool_path_arg(args)?;
            Some((
                Some(path),
                RepresentationClass::CodePure,
                "identity safe-cast boolean comparison preserves the direct boolean lane and null policy",
            ))
        }
        "coalesce" if bool_literal(literal).is_some() && coalesce_bool_args_are_safe(args) => Some((
            first_path_arg(args),
            RepresentationClass::CodePure,
            "coalesce boolean comparison uses boolean/null lanes and literal defaults with CoveQL null semantics",
        )),
        _ => None,
    }
}

fn coded_bool_function_contract(
    expr: &ResolvedExpr,
) -> Option<(Option<&ResolvedPath>, RepresentationClass, &'static str)> {
    let ResolvedExpr::FunctionCall {
        function_id,
        deterministic,
        contract,
        args,
        ..
    } = expr
    else {
        return None;
    };
    if !(*deterministic && function_contract_is_coded_safe(contract)) {
        return None;
    }
    match function_id.as_str() {
        "startsWith" => Some((
            Some(starts_with_path_arg(args)?),
            RepresentationClass::DictionaryLiftedCandidate,
            "startsWith predicate uses a deterministic COVE-MAP coded function contract over a direct string path",
        )),
        "isNull" | "isNotNull" => Some((
            Some(single_non_execution_path_arg(args)?),
            RepresentationClass::CodePure,
            "null-check predicate uses validity/null lanes with CoveQL two-valued function semantics",
        )),
        "identity" => Some((
            Some(single_bool_path_arg(args)?),
            RepresentationClass::CodePure,
            "identity boolean predicate preserves the direct boolean lane and null policy",
        )),
        "cast" => Some((
            Some(identity_cast_bool_path_arg(args)?),
            RepresentationClass::CodePure,
            "identity safe-cast boolean predicate preserves the direct boolean lane and null policy",
        )),
        "coalesce" if coalesce_bool_args_are_safe(args) => Some((
            first_path_arg(args),
            RepresentationClass::CodePure,
            "coalesce boolean predicate uses boolean/null lanes and literal defaults with CoveQL null semantics",
        )),
        _ => None,
    }
}

fn classify_null_check_expr(
    expr: &ResolvedExpr,
    root: &ResolvedRoot,
    security_scope: &str,
    dataset: Option<&crate::DatasetScopeContext>,
) -> (
    PredicatePlacement,
    FilterClassification,
    RepresentationContract,
    Option<String>,
) {
    let Some(path) = path_operand(expr) else {
        return classify_single_expr(expr, root, security_scope, dataset);
    };
    if path.property_id.is_some() {
        return (
            PredicatePlacement::PreReconstruction,
            FilterClassification::PropertyCodedCandidate,
            path_contract(
                path,
                RepresentationClass::CodePure,
                true,
                "property null checks use validity bitmap or missing-value presence metadata",
                security_scope,
            ),
            None,
        );
    }
    if path.system_field.is_some() {
        return (
            PredicatePlacement::PreReconstruction,
            FilterClassification::System,
            path_contract(
                path,
                RepresentationClass::CodePure,
                true,
                "system-field null checks use canonical system-column presence",
                security_scope,
            ),
            None,
        );
    }
    classify_path_predicate(path, AstCompareOp::Eq, root, security_scope, dataset)
}

fn classify_path_predicate(
    path: &ResolvedPath,
    op: AstCompareOp,
    root: &ResolvedRoot,
    security_scope: &str,
    dataset: Option<&crate::DatasetScopeContext>,
) -> (
    PredicatePlacement,
    FilterClassification,
    RepresentationContract,
    Option<String>,
) {
    if path.evidence_field_id.is_some() {
        if matches!(root, ResolvedRoot::Evidence(_))
            && path.root_kind == ResolvedPathRootKind::Evidence
        {
            return (
                PredicatePlacement::PostReconstruction,
                if path.system_field.is_some() {
                    FilterClassification::System
                } else {
                    FilterClassification::EvidenceResidual
                },
                path_contract(
                    path,
                    RepresentationClass::CodePure,
                    true,
                    "evidence-root fields are direct visible row values after COVE-MAP evidence validation and disclosure policy checks",
                    security_scope,
                ),
                None,
            );
        }
        return (
            PredicatePlacement::Evidence,
            FilterClassification::EvidenceResidual,
            path_contract(
                path,
                RepresentationClass::ResidualMaterialized,
                false,
                "evidence predicates are planned after visibility/redaction",
                security_scope,
            ),
            Some("evidence fields can disclose protected lineage".into()),
        );
    }

    if let Some(system_field) = &path.system_field {
        let classification = match system_field {
            ResolvedSystemField::ObjectType => FilterClassification::ObjectType,
            ResolvedSystemField::BranchKey => FilterClassification::Branch,
            ResolvedSystemField::TimestampUs
            | ResolvedSystemField::Csn
            | ResolvedSystemField::ValidFrom
            | ResolvedSystemField::ValidTo => FilterClassification::Temporal,
            _ => FilterClassification::System,
        };
        return (
            PredicatePlacement::PreReconstruction,
            classification,
            path_contract(
                path,
                if path.physical_kind == "num_code" {
                    RepresentationClass::TypedNumeric
                } else {
                    RepresentationClass::CodePure
                },
                true,
                "system fields are canonical logical planning inputs",
                security_scope,
            ),
            None,
        );
    }

    if matches!(root, ResolvedRoot::Projection(_))
        && path.root_kind == ResolvedPathRootKind::Projection
        && path.projection_column.is_some()
    {
        let (representation, exact, reason, residual_reason) =
            projection_path_predicate_contract(path, op);
        return (
            if exact {
                PredicatePlacement::PostReconstruction
            } else {
                PredicatePlacement::Residual
            },
            if exact {
                FilterClassification::PropertyCodedCandidate
            } else {
                FilterClassification::ResidualMaterialized
            },
            path_contract(path, representation, exact, reason, security_scope),
            residual_reason.map(str::to_string),
        );
    }

    if path.property_id.is_some() {
        if path.physical_kind == "file_code" {
            if matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) {
                if let Some(dataset) = dataset {
                    if dataset.files.len() <= 1 {
                        return (
                            PredicatePlacement::PreReconstruction,
                            FilterClassification::PropertyCodedCandidate,
                            path_contract(
                                path,
                                RepresentationClass::CodePure,
                                true,
                                "single-file FileCode equality uses one validated file dictionary/code domain and preserves CoveQL null semantics",
                                security_scope,
                            ),
                            None,
                        );
                    }
                    if dataset_has_exact_code_domain_bridge(dataset) {
                        return (
                            PredicatePlacement::PreReconstruction,
                            FilterClassification::PropertyCodedCandidate,
                            path_contract(
                                path,
                                RepresentationClass::CrossSourceBridgeCandidate,
                                true,
                                "multi-file FileCode equality uses a validated exact manifest code-domain bridge",
                                security_scope,
                            ),
                            None,
                        );
                    }
                }
            }
            if matches!(
                op,
                AstCompareOp::Lt | AstCompareOp::Le | AstCompareOp::Gt | AstCompareOp::Ge
            ) && path_has_utf8_ordering_collation_contract(path)
            {
                return (
                    PredicatePlacement::PreReconstruction,
                    FilterClassification::PropertyCodedCandidate,
                    path_contract(
                        path,
                        RepresentationClass::DecodeBoundary,
                        true,
                        "FileCode ordered comparison decodes values under the effective UTF-8 bytewise collation; raw dictionary code order is not trusted",
                        security_scope,
                    ),
                    None,
                );
            }
            return (
                PredicatePlacement::Residual,
                FilterClassification::PropertyResidual,
                path_contract(
                    path,
                    RepresentationClass::DecodeBoundary,
                    false,
                    "FileCode predicates need dictionary-domain proof before coded execution",
                    security_scope,
                ),
                Some(
                    "FileCode dictionary equality/order proof requires a validated COVE-E or manifest code-domain bridge"
                        .into(),
                ),
            );
        }

        let representation = match path.physical_kind.as_str() {
            "num_code" => RepresentationClass::TypedNumeric,
            "boolean" | "fixed_bytes" if matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) => {
                RepresentationClass::CodePure
            }
            _ if matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) => {
                RepresentationClass::DictionaryLiftedCandidate
            }
            _ => RepresentationClass::DecodeBoundary,
        };
        let exact = matches!(
            representation,
            RepresentationClass::TypedNumeric | RepresentationClass::CodePure
        );
        return (
            if exact {
                PredicatePlacement::PreReconstruction
            } else {
                PredicatePlacement::Residual
            },
            if exact {
                FilterClassification::PropertyCodedCandidate
            } else {
                FilterClassification::PropertyResidual
            },
            path_contract(
                path,
                representation,
                exact,
                if matches!(
                    representation,
                    RepresentationClass::DictionaryLiftedCandidate
                ) {
                    "dictionary-lifted property predicate needs same-domain dictionary proof before coded execution"
                } else {
                    "property predicate carries logical type, physical kind, collation, null policy, and code domain"
                },
                security_scope,
            ),
            if exact {
                None
            } else {
                Some("property predicate requires materialized comparison".into())
            },
        );
    }

    (
        PredicatePlacement::Residual,
        FilterClassification::ResidualMaterialized,
        path_contract(
            path,
            RepresentationClass::ResidualMaterialized,
            false,
            "path has no coded planning contract",
            security_scope,
        ),
        Some("path is residual without a coded planning contract".into()),
    )
}

fn path_has_utf8_ordering_collation_contract(path: &ResolvedPath) -> bool {
    matches!(path.logical_type.as_str(), "utf8" | "string")
        && match path.collation_id {
            None => true,
            Some(id) if id == CollationKind::None.id() => true,
            Some(id) => {
                CollationKind::from_id(id).is_some_and(|kind| kind == CollationKind::Utf8Bytewise)
            }
        }
}

fn dataset_has_exact_code_domain_bridge(dataset: &crate::DatasetScopeContext) -> bool {
    dataset.files.len() <= 1
        || (!dataset.code_domain_bridges.is_empty()
            && dataset
                .code_domain_bridges
                .iter()
                .all(crate::code_domain_bridge_is_exact_coded_remap))
}

fn projection_path_predicate_contract(
    path: &ResolvedPath,
    op: AstCompareOp,
) -> (
    RepresentationClass,
    bool,
    &'static str,
    Option<&'static str>,
) {
    match path.physical_kind.as_str() {
        "boolean" if matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) => (
            RepresentationClass::CodePure,
            true,
            "direct projection boolean filters preserve COVE-MAP row truth and null policy",
            None,
        ),
        "num_code" => (
            RepresentationClass::TypedNumeric,
            true,
            "direct projection numeric/date/time filters use typed Arrow lanes with semantic ordering",
            None,
        ),
        "fixed_bytes" if matches!(op, AstCompareOp::Eq | AstCompareOp::Ne) => (
            RepresentationClass::CodePure,
            true,
            "direct projection fixed-width identity filters preserve exact equality semantics",
            None,
        ),
        _ => (
            RepresentationClass::DecodeBoundary,
            false,
            "projection path requires materialized comparison unless its type/order/collation contract is proven exact",
            Some("projection predicate requires materialized comparison"),
        ),
    }
}

fn function_contract_is_coded_safe(contract: &ResolvedFunctionContract) -> bool {
    matches!(contract.execution_class, FunctionExecutionClass::CodedSafe)
}

fn comparison_has_order_or_equality(op: AstCompareOp) -> bool {
    matches!(
        op,
        AstCompareOp::Eq
            | AstCompareOp::Ne
            | AstCompareOp::Lt
            | AstCompareOp::Le
            | AstCompareOp::Gt
            | AstCompareOp::Ge
    )
}

fn starts_with_path_arg(args: &[ResolvedExpr]) -> Option<&ResolvedPath> {
    let [ResolvedExpr::Path(path), ResolvedExpr::Literal(prefix)] = args else {
        return None;
    };
    (matches!(path.logical_type.as_str(), "utf8" | "string" | "json")
        && path.physical_kind != "execution_code"
        && string_literal(prefix).is_some())
    .then_some(path)
}

fn single_string_path_arg(args: &[ResolvedExpr]) -> Option<&ResolvedPath> {
    let [ResolvedExpr::Path(path)] = args else {
        return None;
    };
    (matches!(path.logical_type.as_str(), "utf8" | "string")
        && path.physical_kind != "execution_code")
        .then_some(path)
}

fn single_non_execution_path_arg(args: &[ResolvedExpr]) -> Option<&ResolvedPath> {
    let [ResolvedExpr::Path(path)] = args else {
        return None;
    };
    (path.physical_kind != "execution_code").then_some(path)
}

fn single_bool_path_arg(args: &[ResolvedExpr]) -> Option<&ResolvedPath> {
    let [ResolvedExpr::Path(path)] = args else {
        return None;
    };
    (matches!(path.logical_type.as_str(), "bool" | "boolean")
        && path.physical_kind != "execution_code")
        .then_some(path)
}

fn identity_cast_bool_path_arg(args: &[ResolvedExpr]) -> Option<&ResolvedPath> {
    let [ResolvedExpr::Path(path), ResolvedExpr::Literal(target)] = args else {
        return None;
    };
    (matches!(path.logical_type.as_str(), "bool" | "boolean")
        && path.physical_kind != "execution_code"
        && string_literal(target).is_some_and(|target| {
            target.eq_ignore_ascii_case("bool") || target.eq_ignore_ascii_case("boolean")
        }))
    .then_some(path)
}

fn coalesce_bool_args_are_safe(args: &[ResolvedExpr]) -> bool {
    !args.is_empty()
        && args.iter().all(|arg| match arg {
            ResolvedExpr::Path(path) => {
                matches!(path.logical_type.as_str(), "bool" | "boolean")
                    && path.physical_kind != "execution_code"
            }
            ResolvedExpr::Literal(literal) => {
                bool_literal(literal).is_some()
                    || matches!(literal.typed_value, ResolvedLiteralValue::Null)
            }
            _ => false,
        })
}

fn first_path_arg(args: &[ResolvedExpr]) -> Option<&ResolvedPath> {
    args.iter().find_map(|arg| match arg {
        ResolvedExpr::Path(path) => Some(path),
        _ => None,
    })
}

fn bool_literal(literal: &ResolvedLiteral) -> Option<bool> {
    match &literal.typed_value {
        ResolvedLiteralValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn string_literal(literal: &ResolvedLiteral) -> Option<&str> {
    match &literal.typed_value {
        ResolvedLiteralValue::String(value) => Some(value.as_str()),
        _ => None,
    }
}

fn integer_literal(literal: &ResolvedLiteral) -> Option<i128> {
    match &literal.typed_value {
        ResolvedLiteralValue::SignedInteger(value) => Some((*value).into()),
        ResolvedLiteralValue::UnsignedInteger(value) => Some((*value).into()),
        _ => None,
    }
}

fn path_operand(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    match expr {
        ResolvedExpr::Path(path) => Some(path),
        _ => None,
    }
}

pub fn expr_label(expr: &ResolvedExpr) -> String {
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::Literal(literal) => format!("literal:{}", literal.logical_type),
        ResolvedExpr::FunctionCall { function_id, .. } => format!("function:{function_id}"),
        ResolvedExpr::AggregateCall { name, .. } => format!("aggregate:{name:?}"),
        ResolvedExpr::Association(association) => format!("association:{}", association.type_name),
        ResolvedExpr::Evidence(evidence) => format!("evidence:{:?}", evidence.grain),
        ResolvedExpr::TableExists(exists) => format!("exists:{}", exists.right.table_name),
        ResolvedExpr::Conditional { .. } => "if".into(),
    }
}

fn path_contract(
    path: &ResolvedPath,
    representation: RepresentationClass,
    exact: bool,
    reason: impl Into<String>,
    security_scope: &str,
) -> RepresentationContract {
    RepresentationContract {
        contract_version: crate::PREDICATE_REPRESENTATION_CONTRACT_VERSION.into(),
        representation,
        logical_type: Some(path.logical_type.clone()),
        physical_kind: Some(path.physical_kind.clone()),
        collation_id: path.collation_id,
        null_policy: Some(path.null_policy.clone()),
        code_domain_id: Some(path.code_domain_id.clone()),
        security_scope: security_scope.into(),
        exact,
        proof_state: proof_state_for_contract(representation, exact),
        reason: reason.into(),
    }
}

fn generic_contract(
    representation: RepresentationClass,
    exact: bool,
    reason: impl Into<String>,
    security_scope: &str,
) -> RepresentationContract {
    RepresentationContract {
        contract_version: crate::PREDICATE_REPRESENTATION_CONTRACT_VERSION.into(),
        representation,
        logical_type: None,
        physical_kind: None,
        collation_id: None,
        null_policy: None,
        code_domain_id: None,
        security_scope: security_scope.into(),
        exact,
        proof_state: proof_state_for_contract(representation, exact),
        reason: reason.into(),
    }
}

fn proof_state_for_contract(
    representation: RepresentationClass,
    exact: bool,
) -> PredicateProofState {
    if exact {
        return PredicateProofState::ProvenExact;
    }
    match representation {
        RepresentationClass::DictionaryLiftedCandidate
        | RepresentationClass::OrdinalMapCandidate
        | RepresentationClass::CrossSourceBridgeCandidate => {
            PredicateProofState::CandidateNeedsResidual
        }
        RepresentationClass::CodePure | RepresentationClass::TypedNumeric => {
            PredicateProofState::CandidateNeedsResidual
        }
        RepresentationClass::DecodeBoundary
        | RepresentationClass::ResidualMaterialized
        | RepresentationClass::NonBeneficial => PredicateProofState::DecodeRequired,
    }
}
