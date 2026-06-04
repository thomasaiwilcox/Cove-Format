use cove_core::profile::cove_o::{
    CoveObjectAssociationEndpointCandidate, CoveObjectPropertyPredicateCandidate,
    CoveObjectPropertyPredicateLiteral, CoveObjectPropertyPredicateOp,
    CoveObjectReadPushdownOptions, CoveObjectReadPushdownReport, CoveObjectTemporalCut,
};
use cove_core::{constants::CovePhysicalKind, types::logical_type_from_name};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    predicate::{
        FilterClassification, LogicalPredicateForm, LogicalPredicateKind, PredicatePlacement,
        RepresentationClass,
    },
    AssociationEndpointRole, AstAssociationDirection, AstCompareOp, BranchSelector,
    FunctionExecutionClass, PlannedQuery, ResolvedAssociationRoot, ResolvedExpr,
    ResolvedFunctionContract, ResolvedLiteral, ResolvedLiteralValue, ResolvedPath,
    ResolvedPredicate, ResolvedRoot, ResolvedSystemField, TemporalMode, TemporalRole,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushdownOptions {
    pub enabled: bool,
    pub optional_metadata_fail_open: bool,
    pub verify_residual_equivalence: bool,
}

impl Default for PushdownOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            optional_metadata_fail_open: true,
            verify_residual_equivalence: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushdownReport {
    pub enabled: bool,
    pub outcome: PushdownOutcome,
    pub decisions: Vec<PushdownDecision>,
    pub counters: PushdownCounters,
    pub residual_predicates: Vec<String>,
    pub decode_boundaries: Vec<String>,
}

impl PushdownReport {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            outcome: PushdownOutcome::Disabled,
            decisions: vec![PushdownDecision::new(
                PushdownDecisionKind::Fallback,
                PushdownOutcome::Disabled,
                "pushdown disabled by execution options",
                json!({}),
                false,
            )],
            counters: PushdownCounters::default(),
            residual_predicates: Vec::new(),
            decode_boundaries: Vec::new(),
        }
    }

    pub fn not_executed(options: &PushdownOptions) -> Self {
        Self {
            enabled: options.enabled,
            outcome: PushdownOutcome::NotExecuted,
            decisions: Vec::new(),
            counters: PushdownCounters::default(),
            residual_predicates: Vec::new(),
            decode_boundaries: Vec::new(),
        }
    }

    pub fn not_applicable(options: &PushdownOptions, reason: impl Into<String>) -> Self {
        Self {
            enabled: options.enabled,
            outcome: PushdownOutcome::NotApplicable,
            decisions: vec![PushdownDecision::new(
                PushdownDecisionKind::Fallback,
                PushdownOutcome::NotApplicable,
                reason,
                json!({}),
                false,
            )],
            counters: PushdownCounters::default(),
            residual_predicates: Vec::new(),
            decode_boundaries: Vec::new(),
        }
    }

    fn recompute_outcome(&mut self) {
        if !self.enabled {
            self.outcome = PushdownOutcome::Disabled;
            return;
        }
        if self
            .decisions
            .iter()
            .any(|decision| decision.outcome == PushdownOutcome::Applied)
        {
            self.outcome = PushdownOutcome::Applied;
        } else if self.decisions.is_empty() {
            self.outcome = PushdownOutcome::NoCandidates;
        } else {
            self.outcome = PushdownOutcome::NoCandidates;
        }
    }

    pub(crate) fn merge_core_report(&mut self, core: CoveObjectReadPushdownReport) {
        self.counters.segments_seen += core.segments_seen;
        self.counters.segments_skipped += core.segments_skipped;
        self.counters.rows_seen += core.rows_seen;
        self.counters.rows_candidates += core.rows_candidates;
        self.counters.rows_after_candidate_retain += core
            .rows_seen
            .saturating_sub(core.rows_skipped_by_property_candidates);
        self.counters.rows_skipped_by_property_candidates +=
            core.rows_skipped_by_property_candidates;
        self.counters.property_columns_requested += core.property_columns_requested;
        for decision in core.decisions {
            self.decisions.push(PushdownDecision::new(
                core_decision_kind(&decision.kind),
                core_outcome(&decision.outcome),
                decision.reason,
                json!({ "source": "cove_o_readback" }),
                decision.redacted,
            ));
        }
        self.recompute_outcome();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushdownOutcome {
    Disabled,
    Applied,
    NoCandidates,
    Residual,
    Fallback,
    NotApplicable,
    NotExecuted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushdownDecision {
    pub kind: PushdownDecisionKind,
    pub outcome: PushdownOutcome,
    pub reason: String,
    pub safe_details: Value,
    pub redacted: bool,
}

impl PushdownDecision {
    pub fn new(
        kind: PushdownDecisionKind,
        outcome: PushdownOutcome,
        reason: impl Into<String>,
        safe_details: Value,
        redacted: bool,
    ) -> Self {
        Self {
            kind,
            outcome,
            reason: reason.into(),
            safe_details,
            redacted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushdownDecisionKind {
    TemporalSegmentPrune,
    BranchRowCandidate,
    GoidRowCandidate,
    TombstoneCandidate,
    PropertyColumnPrune,
    PropertyCandidateSeed,
    ProjectionColumnPrune,
    ProjectionFilterCandidate,
    ValidityNullCheckCandidate,
    NumericDateTimeCandidate,
    AssociationEndpointCandidate,
    TemporalBloomIgnored,
    ResidualMaterialized,
    Fallback,
}

pub(crate) fn projection_readback_report(
    planned: &PlannedQuery,
    options: &PushdownOptions,
    pushed_columns: Option<&[String]>,
    pushed_filter_count: usize,
    input_rows: usize,
    output_rows: usize,
) -> PushdownReport {
    if !options.enabled {
        return PushdownReport::disabled();
    }

    let contracts = &planned.dependencies.projection_contracts;
    let mut decisions = Vec::new();
    let pushed_columns = pushed_columns.unwrap_or(&[]);
    if !pushed_columns.is_empty() {
        decisions.push(PushdownDecision::new(
            PushdownDecisionKind::ProjectionColumnPrune,
            PushdownOutcome::Applied,
            "selected projection columns were pushed into COVE-MAP projection readback",
            json!({
                "projection_ids": contracts
                    .iter()
                    .map(|contract| contract.projection_id.clone())
                    .collect::<Vec<_>>(),
                "columns": pushed_columns,
                "column_count": pushed_columns.len(),
            }),
            true,
        ));
    }

    for contract in contracts {
        for predicate in &contract.pushed_predicates {
            decisions.push(PushdownDecision::new(
                PushdownDecisionKind::ProjectionFilterCandidate,
                PushdownOutcome::Applied,
                "primitive projection predicate was pushed into COVE-MAP projection readback",
                json!({
                    "projection_id": contract.projection_id,
                    "predicate": predicate,
                }),
                true,
            ));
        }
        for predicate in &contract.residual_predicates {
            decisions.push(PushdownDecision::new(
                PushdownDecisionKind::ResidualMaterialized,
                PushdownOutcome::Residual,
                "projection predicate could not be proven safe for COVE-MAP readback and remains an OQL residual",
                json!({
                    "projection_id": contract.projection_id,
                    "predicate": predicate,
                }),
                true,
            ));
        }
        for reason in &contract.residual_reasons {
            decisions.push(PushdownDecision::new(
                PushdownDecisionKind::ResidualMaterialized,
                PushdownOutcome::Residual,
                reason,
                json!({
                    "projection_id": contract.projection_id,
                }),
                true,
            ));
        }
    }

    if pushed_filter_count
        > contracts
            .iter()
            .map(|contract| contract.pushed_predicates.len())
            .sum::<usize>()
    {
        decisions.push(PushdownDecision::new(
            PushdownDecisionKind::ProjectionFilterCandidate,
            PushdownOutcome::Applied,
            "COVE-MAP projection readback accepted additional primitive filter fragments produced by predicate lowering",
            json!({ "filter_count": pushed_filter_count }),
            true,
        ));
    }

    let mut report = PushdownReport {
        enabled: true,
        outcome: PushdownOutcome::NoCandidates,
        decisions,
        counters: PushdownCounters {
            rows_seen: input_rows,
            rows_candidates: input_rows,
            rows_after_candidate_retain: output_rows,
            property_columns_requested: pushed_columns.len(),
            property_predicate_candidates: pushed_filter_count,
            residual_predicates: contracts
                .iter()
                .map(|contract| {
                    contract.residual_predicates.len() + contract.residual_reasons.len()
                })
                .sum(),
            ..PushdownCounters::default()
        },
        residual_predicates: contracts
            .iter()
            .flat_map(|contract| {
                contract
                    .residual_predicates
                    .iter()
                    .chain(contract.residual_reasons.iter())
                    .cloned()
            })
            .collect(),
        decode_boundaries: planned.logical_plan.decode_boundaries.clone(),
    };
    report.recompute_outcome();
    report
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushdownCounters {
    pub segments_seen: usize,
    pub segments_skipped: usize,
    pub rows_seen: usize,
    pub rows_candidates: usize,
    pub rows_after_candidate_retain: usize,
    pub rows_skipped_by_property_candidates: usize,
    pub property_columns_requested: usize,
    pub property_predicate_candidates: usize,
    pub residual_predicates: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct PushdownReadPlan {
    pub read_options: CoveObjectReadPushdownOptions,
    pub report: PushdownReport,
}

pub(crate) fn pushdown_read_plan(
    planned: &PlannedQuery,
    options: &PushdownOptions,
) -> PushdownReadPlan {
    if !options.enabled {
        return PushdownReadPlan {
            read_options: CoveObjectReadPushdownOptions::default(),
            report: PushdownReport::disabled(),
        };
    }

    let mut report = PushdownReport {
        enabled: true,
        outcome: PushdownOutcome::NoCandidates,
        decisions: Vec::new(),
        counters: PushdownCounters::default(),
        residual_predicates: planned.logical_plan.decode_boundaries.clone(),
        decode_boundaries: planned.logical_plan.decode_boundaries.clone(),
    };

    let temporal_cut = temporal_cut(
        &planned.resolved.temporal.mode,
        planned.resolved.temporal.role,
    );
    if let Some(cut) = temporal_cut {
        if cut != CoveObjectTemporalCut::LatestCommitted {
            report.decisions.push(PushdownDecision::new(
                PushdownDecisionKind::TemporalSegmentPrune,
                PushdownOutcome::Applied,
                "asOf temporal cut can exclude future segments and rows without changing logical truth",
                json!({ "temporal_mode": format!("{:?}", planned.resolved.temporal.mode) }),
                true,
            ));
        }
    }

    let branch_key = match planned.resolved.branch.selector {
        BranchSelector::BranchKey(branch_key) => {
            report.decisions.push(PushdownDecision::new(
                PushdownDecisionKind::BranchRowCandidate,
                PushdownOutcome::Applied,
                "concrete branch key can narrow temporal rows before reconstruction",
                json!({ "branch_key": branch_key }),
                true,
            ));
            Some(branch_key)
        }
        BranchSelector::Default | BranchSelector::RejectAmbiguous => None,
    };

    if !planned.resolved.tombstone.include_tombstones {
        report.decisions.push(PushdownDecision::new(
            PushdownDecisionKind::TombstoneCandidate,
            PushdownOutcome::Residual,
            "tombstone rows are retained until reconstruction so deletions cannot resurrect earlier states",
            json!({}),
            false,
        ));
    }

    let mut candidate_goids = Vec::new();
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_predicate_candidates(predicate, &mut candidate_goids, &mut report);
    }
    let association_endpoint_candidates = association_endpoint_candidates(
        planned,
        branch_key,
        temporal_cut,
        &candidate_goids,
        &mut report,
    );
    let property_candidates = property_predicate_candidates(planned, &mut report);

    classify_logical_predicates(&planned.logical_plan.predicate_forms, &mut report);
    report.counters.residual_predicates = report
        .decisions
        .iter()
        .filter(|decision| decision.outcome == PushdownOutcome::Residual)
        .count();
    report.recompute_outcome();

    PushdownReadPlan {
        read_options: CoveObjectReadPushdownOptions {
            enabled: true,
            temporal_cut,
            branch_key,
            candidate_goids,
            include_tombstones: Some(planned.resolved.tombstone.include_tombstones),
            association_endpoint_candidates,
            property_candidates,
        },
        report,
    }
}

fn property_predicate_candidates(
    planned: &PlannedQuery,
    report: &mut PushdownReport,
) -> Vec<CoveObjectPropertyPredicateCandidate> {
    let mut out = Vec::new();
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_property_predicate_candidates(predicate, &mut out);
    }
    if !out.is_empty() {
        report.decisions.push(PushdownDecision::new(
            PushdownDecisionKind::PropertyCandidateSeed,
            PushdownOutcome::Applied,
            "typed property predicate candidates were passed to COVE-O readback; residual materialized checks remain authoritative",
            json!({ "candidate_count": out.len() }),
            true,
        ));
    }
    out
}

fn collect_property_predicate_candidates(
    predicate: &ResolvedPredicate,
    out: &mut Vec<CoveObjectPropertyPredicateCandidate>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            if let Some(candidate) = property_compare_candidate(left, *op, right) {
                out.push(candidate);
            } else if let Some(candidate) =
                property_compare_candidate(right, flip_compare_op(*op), left)
            {
                out.push(candidate);
            }
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            if let Some(path) = resolved_property_path(expr) {
                if let Some(candidate) = property_candidate(
                    path,
                    if *negated {
                        CoveObjectPropertyPredicateOp::IsNotNull
                    } else {
                        CoveObjectPropertyPredicateOp::IsNull
                    },
                    None,
                ) {
                    out.push(candidate);
                }
            }
        }
        ResolvedPredicate::BoolExpr(expr) => {
            if let Some(path) = resolved_property_path(expr) {
                if let Some(candidate) = property_candidate(
                    path,
                    CoveObjectPropertyPredicateOp::Eq,
                    Some(CoveObjectPropertyPredicateLiteral::Bool(true)),
                ) {
                    out.push(candidate);
                }
            }
        }
        ResolvedPredicate::And(parts) => {
            for part in parts {
                collect_property_predicate_candidates(part, out);
            }
        }
        ResolvedPredicate::Not(_)
        | ResolvedPredicate::Or(_)
        | ResolvedPredicate::InList { .. }
        | ResolvedPredicate::Exists(_) => {}
    }
}

fn property_compare_candidate(
    left: &ResolvedExpr,
    op: AstCompareOp,
    right: &ResolvedExpr,
) -> Option<CoveObjectPropertyPredicateCandidate> {
    let path = resolved_property_path(left)?;
    let literal = resolved_literal(right)?;
    property_candidate(path, property_compare_op(op)?, Some(literal))
}

fn resolved_property_path(expr: &ResolvedExpr) -> Option<&crate::ResolvedPath> {
    match expr {
        ResolvedExpr::Path(path) if path.object_type_id.is_some() && path.property_id.is_some() => {
            Some(path)
        }
        _ => None,
    }
}

fn resolved_literal(expr: &ResolvedExpr) -> Option<CoveObjectPropertyPredicateLiteral> {
    match expr {
        ResolvedExpr::Literal(literal) => literal_candidate(literal),
        _ => None,
    }
}

fn literal_candidate(literal: &ResolvedLiteral) -> Option<CoveObjectPropertyPredicateLiteral> {
    match &literal.typed_value {
        ResolvedLiteralValue::Null => Some(CoveObjectPropertyPredicateLiteral::Null),
        ResolvedLiteralValue::Boolean(value) => {
            Some(CoveObjectPropertyPredicateLiteral::Bool(*value))
        }
        ResolvedLiteralValue::String(value) => {
            Some(CoveObjectPropertyPredicateLiteral::String(value.clone()))
        }
        ResolvedLiteralValue::SignedInteger(value) => {
            Some(CoveObjectPropertyPredicateLiteral::I64(*value))
        }
        ResolvedLiteralValue::UnsignedInteger(value) => {
            Some(CoveObjectPropertyPredicateLiteral::U64(*value))
        }
        ResolvedLiteralValue::TimestampMicros { micros, .. } => {
            Some(CoveObjectPropertyPredicateLiteral::I64(*micros))
        }
        ResolvedLiteralValue::BigInteger(_)
        | ResolvedLiteralValue::Decimal { .. }
        | ResolvedLiteralValue::Uuid { .. }
        | ResolvedLiteralValue::Binary { .. } => None,
    }
}

fn property_candidate(
    path: &crate::ResolvedPath,
    op: CoveObjectPropertyPredicateOp,
    literal: Option<CoveObjectPropertyPredicateLiteral>,
) -> Option<CoveObjectPropertyPredicateCandidate> {
    let logical_type = logical_type_from_name(&path.logical_type).ok()?;
    let physical_kind = physical_kind_from_name(&path.physical_kind)?;
    let proven_exact = property_candidate_is_proven_exact(physical_kind, op);
    Some(CoveObjectPropertyPredicateCandidate {
        object_type_id: path.object_type_id?,
        property_id: path.property_id?,
        logical_type,
        physical_kind,
        collation_id: path.collation_id,
        null_policy: Some(path.null_policy.clone()),
        op,
        literal,
        proof_state: if proven_exact {
            "proven_exact".into()
        } else {
            "candidate_needs_residual".into()
        },
    })
}

fn property_candidate_is_proven_exact(
    physical_kind: CovePhysicalKind,
    op: CoveObjectPropertyPredicateOp,
) -> bool {
    match physical_kind {
        CovePhysicalKind::NumCode => true,
        CovePhysicalKind::Boolean | CovePhysicalKind::FixedBytes => matches!(
            op,
            CoveObjectPropertyPredicateOp::Eq
                | CoveObjectPropertyPredicateOp::Ne
                | CoveObjectPropertyPredicateOp::IsNull
                | CoveObjectPropertyPredicateOp::IsNotNull
        ),
        CovePhysicalKind::FileCode
        | CovePhysicalKind::VarBytes
        | CovePhysicalKind::List
        | CovePhysicalKind::Struct
        | CovePhysicalKind::Map => false,
        _ => false,
    }
}

fn property_compare_op(op: AstCompareOp) -> Option<CoveObjectPropertyPredicateOp> {
    Some(match op {
        AstCompareOp::Eq => CoveObjectPropertyPredicateOp::Eq,
        AstCompareOp::Ne => CoveObjectPropertyPredicateOp::Ne,
        AstCompareOp::Lt => CoveObjectPropertyPredicateOp::Lt,
        AstCompareOp::Le => CoveObjectPropertyPredicateOp::LtEq,
        AstCompareOp::Gt => CoveObjectPropertyPredicateOp::Gt,
        AstCompareOp::Ge => CoveObjectPropertyPredicateOp::GtEq,
    })
}

fn flip_compare_op(op: AstCompareOp) -> AstCompareOp {
    match op {
        AstCompareOp::Eq => AstCompareOp::Eq,
        AstCompareOp::Ne => AstCompareOp::Ne,
        AstCompareOp::Lt => AstCompareOp::Gt,
        AstCompareOp::Le => AstCompareOp::Ge,
        AstCompareOp::Gt => AstCompareOp::Lt,
        AstCompareOp::Ge => AstCompareOp::Le,
    }
}

fn physical_kind_from_name(name: &str) -> Option<CovePhysicalKind> {
    match name {
        "file_code" => Some(CovePhysicalKind::FileCode),
        "num_code" => Some(CovePhysicalKind::NumCode),
        "boolean" => Some(CovePhysicalKind::Boolean),
        "fixed_bytes" => Some(CovePhysicalKind::FixedBytes),
        "var_bytes" => Some(CovePhysicalKind::VarBytes),
        "list" => Some(CovePhysicalKind::List),
        "struct" => Some(CovePhysicalKind::Struct),
        "map" => Some(CovePhysicalKind::Map),
        _ => None,
    }
}

fn association_endpoint_candidates(
    planned: &PlannedQuery,
    branch_key: Option<u64>,
    temporal_cut: Option<CoveObjectTemporalCut>,
    candidate_goids: &[[u8; 16]],
    report: &mut PushdownReport,
) -> Vec<CoveObjectAssociationEndpointCandidate> {
    let mut associations = Vec::new();
    if let ResolvedRoot::Association(root) = &planned.resolved.root {
        associations.push(root);
    }
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_association_predicate_candidates(predicate, &mut associations);
    }
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            collect_association_expr_candidates(&item.expr, &mut associations);
        }
    }
    let mut out = Vec::new();
    for association in associations {
        if association.endpoint_role == AssociationEndpointRole::Unknown {
            report.decisions.push(PushdownDecision::new(
                PushdownDecisionKind::Fallback,
                PushdownOutcome::Fallback,
                "association endpoint flags are missing or ambiguous; endpoint pruning is disabled",
                json!({ "association_endpoint_candidate": false }),
                true,
            ));
            continue;
        }
        let candidate_goids = if candidate_goids.is_empty() {
            vec![None]
        } else {
            candidate_goids.iter().copied().map(Some).collect()
        };
        for candidate_goid in candidate_goids {
            report.decisions.push(PushdownDecision::new(
                PushdownDecisionKind::AssociationEndpointCandidate,
                if candidate_goid.is_some() {
                    PushdownOutcome::Applied
                } else {
                    PushdownOutcome::Residual
                },
                "association endpoint candidate can narrow readback candidates before materialized verification when a concrete GOID is available",
                json!({
                    "association_type_id": association.object_type_id,
                    "endpoint_role": endpoint_role_name(association.endpoint_role),
                    "candidate_goid_present": candidate_goid.is_some(),
                }),
                true,
            ));
            out.push(CoveObjectAssociationEndpointCandidate {
                association_type_id: association.object_type_id,
                direction: association
                    .direction
                    .map(direction_name)
                    .map(str::to_string),
                endpoint_role: endpoint_role_name(association.endpoint_role).into(),
                branch_key,
                temporal_cut,
                candidate_goid,
                include_tombstones: Some(planned.resolved.tombstone.include_tombstones),
            });
        }
    }
    out
}

fn collect_association_predicate_candidates<'a>(
    predicate: &'a ResolvedPredicate,
    out: &mut Vec<&'a ResolvedAssociationRoot>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_association_expr_candidates(left, out);
            collect_association_expr_candidates(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_association_expr_candidates(expr, out),
        ResolvedPredicate::Not(inner) => collect_association_predicate_candidates(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_association_predicate_candidates(part, out);
            }
        }
    }
}

fn collect_association_expr_candidates<'a>(
    expr: &'a ResolvedExpr,
    out: &mut Vec<&'a ResolvedAssociationRoot>,
) {
    match expr {
        ResolvedExpr::Association(association) => out.push(association),
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_association_expr_candidates(arg, out);
            }
        }
        ResolvedExpr::AggregateCall { arg: Some(arg), .. } => {
            collect_association_expr_candidates(arg, out);
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_association_predicate_candidates(predicate, out);
            collect_association_expr_candidates(then_expr, out);
            collect_association_expr_candidates(else_expr, out);
        }
        _ => {}
    }
}

fn direction_name(direction: AstAssociationDirection) -> &'static str {
    match direction {
        AstAssociationDirection::Out => "out",
        AstAssociationDirection::In => "in",
        AstAssociationDirection::Either => "either",
    }
}

fn endpoint_role_name(role: AssociationEndpointRole) -> &'static str {
    match role {
        AssociationEndpointRole::Source => "source",
        AssociationEndpointRole::Target => "target",
        AssociationEndpointRole::Either => "either",
        AssociationEndpointRole::Unknown => "unknown",
    }
}

fn temporal_cut(mode: &TemporalMode, role: TemporalRole) -> Option<CoveObjectTemporalCut> {
    if role == TemporalRole::AssociationValidTime {
        return None;
    }
    match mode {
        TemporalMode::Latest => Some(CoveObjectTemporalCut::LatestCommitted),
        TemporalMode::AsOfCsn(csn) => Some(CoveObjectTemporalCut::Csn(*csn)),
        TemporalMode::AsOfTimestampMicros(timestamp) if role == TemporalRole::CommitTime => {
            Some(CoveObjectTemporalCut::TimestampUs(*timestamp))
        }
        TemporalMode::AsOfTimestampMicros(_) => None,
        TemporalMode::HistoryRecords
        | TemporalMode::HistoryStates
        | TemporalMode::HistoryRecordsAndStates
        | TemporalMode::ChangesRecords
        | TemporalMode::ChangesStateTransitions
        | TemporalMode::ChangesPropertyDiffs
        | TemporalMode::ChangesFinalObjects => None,
    }
}

fn collect_predicate_candidates(
    predicate: &ResolvedPredicate,
    candidate_goids: &mut Vec<[u8; 16]>,
    report: &mut PushdownReport,
) {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            if *op == AstCompareOp::Eq {
                if let Some(goid) = goid_equality_candidate(left, right) {
                    if !candidate_goids.contains(&goid) {
                        candidate_goids.push(goid);
                    }
                    report.decisions.push(PushdownDecision::new(
                        PushdownDecisionKind::GoidRowCandidate,
                        PushdownOutcome::Applied,
                        "GOID equality can narrow row candidates while retaining full matching object chains",
                        json!({ "candidate_goids": 1 }),
                        true,
                    ));
                    return;
                }
            }
            if (contains_function(left) || contains_function(right))
                && !coded_function_compare_is_pushdown_safe(left, right, *op)
            {
                report_residual(
                    report,
                    "function comparison requires materialized evaluation",
                );
            }
        }
        ResolvedPredicate::InList { expr, values } => {
            if path_is_system(expr, ResolvedSystemField::Goid) {
                let goids = goid_in_list_candidates(values);
                if goids.is_empty() {
                    report_residual(
                        report,
                        "GOID in-list contains no parseable GOID literals and remains materialized",
                    );
                } else {
                    let mut added = 0usize;
                    for goid in goids {
                        if !candidate_goids.contains(&goid) {
                            candidate_goids.push(goid);
                            added += 1;
                        }
                    }
                    report.decisions.push(PushdownDecision::new(
                        PushdownDecisionKind::GoidRowCandidate,
                        PushdownOutcome::Applied,
                        "GOID in-list can narrow row candidates while retaining materialized membership verification",
                        json!({ "candidate_goids": added }),
                        true,
                    ));
                }
            }
        }
        ResolvedPredicate::NullCheck { .. } => {}
        ResolvedPredicate::Exists(expr) => {
            if matches!(
                expr,
                ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_)
            ) {
                report_residual(
                    report,
                    "association/evidence existence remains a materialized boundary",
                );
            }
        }
        ResolvedPredicate::BoolExpr(expr) => {
            if contains_function(expr) && !coded_bool_function_is_pushdown_safe(expr) {
                report_residual(
                    report,
                    "function boolean expression requires materialized evaluation",
                );
            }
        }
        ResolvedPredicate::Not(_) => {
            report_residual(
                report,
                "NOT is residual without no-false-negative complement proof",
            );
        }
        ResolvedPredicate::And(parts) => {
            for part in parts {
                collect_predicate_candidates(part, candidate_goids, report);
            }
        }
        ResolvedPredicate::Or(parts) => {
            if let Some(goids) = goid_or_candidates(parts) {
                let mut added = 0usize;
                for goid in goids {
                    if !candidate_goids.contains(&goid) {
                        candidate_goids.push(goid);
                        added += 1;
                    }
                }
                report.decisions.push(PushdownDecision::new(
                    PushdownDecisionKind::GoidRowCandidate,
                    PushdownOutcome::Applied,
                    "GOID OR predicate can narrow to the union of candidate object IDs while retaining materialized verification",
                    json!({ "candidate_goids": added, "or_terms": parts.len() }),
                    true,
                ));
            } else {
                report_residual(
                    report,
                    "OR is residual without compatible coverage proof composition",
                );
            }
        }
    }
}

fn classify_logical_predicates(forms: &[LogicalPredicateForm], report: &mut PushdownReport) {
    for form in forms {
        match &form.kind {
            LogicalPredicateKind::And(parts) | LogicalPredicateKind::Or(parts) => {
                classify_logical_predicates(parts, report);
            }
            LogicalPredicateKind::Not(inner) => classify_logical_leaf(inner, report),
            _ => classify_logical_leaf(form, report),
        }
    }
}

fn classify_logical_leaf(form: &LogicalPredicateForm, report: &mut PushdownReport) {
    match form.classification {
        FilterClassification::PropertyCodedCandidate if form.representation.exact => {
            let kind = match &form.kind {
                LogicalPredicateKind::NullCheck { negated: true, .. } => {
                    PushdownDecisionKind::ValidityNullCheckCandidate
                }
                _ if form
                    .representation
                    .logical_type
                    .as_deref()
                    .is_some_and(is_numeric_datetime)
                    || form.representation.representation == RepresentationClass::TypedNumeric =>
                {
                    PushdownDecisionKind::NumericDateTimeCandidate
                }
                _ => PushdownDecisionKind::PropertyCandidateSeed,
            };
            report.counters.property_predicate_candidates += 1;
            report.decisions.push(PushdownDecision::new(
                kind,
                PushdownOutcome::Residual,
                "property predicate is a safe candidate, but final materialized residual evaluation remains the semantic authority",
                json!({
                    "logical_type": form.representation.logical_type,
                    "physical_kind": form.representation.physical_kind,
                    "collation_id": form.representation.collation_id,
                    "null_policy": form.representation.null_policy,
                    "code_domain_id": form.representation.code_domain_id,
                }),
                true,
            ));
        }
        FilterClassification::PropertyCodedCandidate
        | FilterClassification::PropertyResidual
        | FilterClassification::AssociationSemiJoin
        | FilterClassification::EvidenceResidual
        | FilterClassification::Aggregate
        | FilterClassification::ResidualMaterialized => {
            report.decisions.push(PushdownDecision::new(
                PushdownDecisionKind::ResidualMaterialized,
                PushdownOutcome::Residual,
                form.residual_reason
                    .clone()
                    .unwrap_or_else(|| "predicate remains materialized residual".into()),
                json!({ "placement": form.placement, "classification": form.classification }),
                true,
            ));
        }
        FilterClassification::System
        | FilterClassification::ObjectType
        | FilterClassification::Temporal
        | FilterClassification::Branch
        | FilterClassification::Tombstone
        | FilterClassification::None => {
            if form.placement != PredicatePlacement::PreReconstruction {
                report_residual(report, "non-pre-reconstruction form remains residual");
            }
        }
    }
}

fn is_numeric_datetime(logical_type: &str) -> bool {
    matches!(
        logical_type,
        "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
            | "decimal128"
            | "date_days"
            | "timestamp_micros"
            | "timestamp_nanos"
    )
}

fn report_residual(report: &mut PushdownReport, reason: impl Into<String>) {
    report.decisions.push(PushdownDecision::new(
        PushdownDecisionKind::ResidualMaterialized,
        PushdownOutcome::Residual,
        reason,
        json!({}),
        true,
    ));
}

fn goid_equality_candidate(left: &ResolvedExpr, right: &ResolvedExpr) -> Option<[u8; 16]> {
    match (left, right) {
        (ResolvedExpr::Path(path), ResolvedExpr::Literal(literal))
            if path.system_field.as_ref() == Some(&ResolvedSystemField::Goid) =>
        {
            parse_goid_literal(literal)
        }
        (ResolvedExpr::Literal(literal), ResolvedExpr::Path(path))
            if path.system_field.as_ref() == Some(&ResolvedSystemField::Goid) =>
        {
            parse_goid_literal(literal)
        }
        _ => None,
    }
}

fn parse_goid_literal(literal: &ResolvedLiteral) -> Option<[u8; 16]> {
    let text = literal
        .canonical
        .strip_prefix("0x")
        .unwrap_or(&literal.canonical);
    let normalized = text.replace('-', "");
    if normalized.len() != 32 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 16];
    for (index, slot) in out.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(&normalized[start..start + 2], 16).ok()?;
    }
    Some(out)
}

fn goid_in_list_candidates(values: &[ResolvedLiteral]) -> Vec<[u8; 16]> {
    let mut out = Vec::new();
    for value in values {
        let Some(goid) = parse_goid_literal(value) else {
            continue;
        };
        if !out.contains(&goid) {
            out.push(goid);
        }
    }
    out
}

fn goid_positive_predicate_candidates(predicate: &ResolvedPredicate) -> Option<Vec<[u8; 16]>> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } if *op == AstCompareOp::Eq => {
            goid_equality_candidate(left, right).map(|goid| vec![goid])
        }
        ResolvedPredicate::InList { expr, values }
            if path_is_system(expr, ResolvedSystemField::Goid) =>
        {
            let goids = goid_in_list_candidates(values);
            (!goids.is_empty()).then_some(goids)
        }
        _ => None,
    }
}

fn goid_or_candidates(parts: &[ResolvedPredicate]) -> Option<Vec<[u8; 16]>> {
    let mut out = Vec::new();
    for part in parts {
        let goids = goid_positive_predicate_candidates(part)?;
        for goid in goids {
            if !out.contains(&goid) {
                out.push(goid);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

fn path_is_system(expr: &ResolvedExpr, field: ResolvedSystemField) -> bool {
    matches!(expr, ResolvedExpr::Path(path) if path.system_field.as_ref() == Some(&field))
}

fn coded_function_compare_is_pushdown_safe(
    left: &ResolvedExpr,
    right: &ResolvedExpr,
    op: AstCompareOp,
) -> bool {
    let (function, literal) = match (left, right) {
        (function @ ResolvedExpr::FunctionCall { .. }, ResolvedExpr::Literal(literal))
        | (ResolvedExpr::Literal(literal), function @ ResolvedExpr::FunctionCall { .. }) => {
            (function, literal)
        }
        _ => return false,
    };
    let ResolvedExpr::FunctionCall {
        function_id,
        deterministic,
        contract,
        args,
        ..
    } = function
    else {
        return false;
    };
    if !(*deterministic && function_contract_is_coded_safe(contract)) {
        return false;
    }
    match function_id.as_str() {
        "startsWith" => bool_literal(literal).is_some() && starts_with_path_arg(args).is_some(),
        "length" => {
            integer_literal(literal).is_some()
                && comparison_has_order_or_equality(op)
                && single_string_path_arg(args).is_some()
        }
        "lower" | "lowercase" | "upper" | "uppercase" | "trim" => {
            string_literal(literal).is_some() && single_string_path_arg(args).is_some()
        }
        "isNull" | "isNotNull" => {
            bool_literal(literal).is_some() && single_non_execution_path_arg(args).is_some()
        }
        "identity" => bool_literal(literal).is_some() && single_bool_path_arg(args).is_some(),
        "cast" => bool_literal(literal).is_some() && identity_cast_bool_path_arg(args).is_some(),
        "coalesce" => bool_literal(literal).is_some() && coalesce_bool_args_are_safe(args),
        _ => false,
    }
}

fn coded_bool_function_is_pushdown_safe(expr: &ResolvedExpr) -> bool {
    let ResolvedExpr::FunctionCall {
        function_id,
        deterministic,
        contract,
        args,
        ..
    } = expr
    else {
        return false;
    };
    if !(*deterministic && function_contract_is_coded_safe(contract)) {
        return false;
    }
    match function_id.as_str() {
        "startsWith" => starts_with_path_arg(args).is_some(),
        "isNull" | "isNotNull" => single_non_execution_path_arg(args).is_some(),
        "identity" => single_bool_path_arg(args).is_some(),
        "cast" => identity_cast_bool_path_arg(args).is_some(),
        "coalesce" => coalesce_bool_args_are_safe(args),
        _ => false,
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

fn contains_function(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::FunctionCall { .. } => true,
        ResolvedExpr::Conditional {
            then_expr,
            else_expr,
            predicate,
            ..
        } => {
            contains_function(then_expr)
                || contains_function(else_expr)
                || predicate_contains_function(predicate)
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            arg.as_ref().is_some_and(|arg| contains_function(arg))
        }
        _ => false,
    }
}

fn predicate_contains_function(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            contains_function(left) || contains_function(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => contains_function(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_function(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_function)
        }
    }
}

fn core_decision_kind(kind: &str) -> PushdownDecisionKind {
    match kind {
        "temporal_segment_prune" => PushdownDecisionKind::TemporalSegmentPrune,
        "temporal_bloom_prune" | "temporal_bloom_ignored" => {
            PushdownDecisionKind::TemporalBloomIgnored
        }
        "tombstone_candidate" => PushdownDecisionKind::TombstoneCandidate,
        "association_endpoint_candidate" => PushdownDecisionKind::AssociationEndpointCandidate,
        "property_predicate_candidate" => PushdownDecisionKind::PropertyCandidateSeed,
        _ => PushdownDecisionKind::Fallback,
    }
}

fn core_outcome(outcome: &str) -> PushdownOutcome {
    match outcome {
        "applied" => PushdownOutcome::Applied,
        "residual" => PushdownOutcome::Residual,
        "fallback" => PushdownOutcome::Fallback,
        "disabled" => PushdownOutcome::Disabled,
        _ => PushdownOutcome::Fallback,
    }
}
