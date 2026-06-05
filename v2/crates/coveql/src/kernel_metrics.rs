use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{AssociationOptimizationReport, EvidenceOptimizationReport, LineageReuseReport};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelExecutionOptions {
    pub mode: KernelExecutionMode,
    pub batch_size: Option<usize>,
    pub scratch_budget_bytes: usize,
    pub redact_exact_counters: bool,
}

impl Default for KernelExecutionOptions {
    fn default() -> Self {
        Self {
            mode: KernelExecutionMode::Auto,
            batch_size: None,
            scratch_budget_bytes: 16 * 1024 * 1024,
            redact_exact_counters: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelExecutionMode {
    Auto,
    ForceMaterialized,
    ForceKernel,
    CompareWithMaterialized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelExecutionReport {
    pub enabled: bool,
    pub mode: KernelExecutionMode,
    pub decision: KernelDecision,
    pub decisions: Vec<KernelDecision>,
    pub optimization_authority: OptimizationAuthorityReport,
    pub counters: KernelCounters,
    pub metrics: KernelMetricSnapshot,
    pub fallback_reason: Option<KernelFallbackReason>,
    pub compared_with_materialized: bool,
    pub materialized_fingerprint: Option<String>,
    pub kernel_fingerprint: Option<String>,
    pub association: AssociationOptimizationReport,
    pub evidence: EvidenceOptimizationReport,
    pub lineage: LineageReuseReport,
}

impl KernelExecutionReport {
    pub fn disabled(mode: KernelExecutionMode, reason: impl Into<String>) -> Self {
        let decision = KernelDecision::new(KernelDecisionKind::Disabled, reason, json!({}), false);
        Self {
            enabled: false,
            mode,
            decision: decision.clone(),
            decisions: vec![decision],
            optimization_authority: OptimizationAuthorityReport::materialized_fallback(
                "kernel execution did not run",
            ),
            counters: KernelCounters::default(),
            metrics: KernelMetricSnapshot::default(),
            fallback_reason: Some(KernelFallbackReason::DisabledByOptions),
            compared_with_materialized: false,
            materialized_fingerprint: None,
            kernel_fingerprint: None,
            association: AssociationOptimizationReport::default(),
            evidence: EvidenceOptimizationReport::default(),
            lineage: LineageReuseReport::default(),
        }
    }

    pub fn fallback(
        mode: KernelExecutionMode,
        reason: KernelFallbackReason,
        message: impl Into<String>,
    ) -> Self {
        let decision = KernelDecision::new(
            KernelDecisionKind::Fallback,
            message,
            json!({ "reason": format!("{reason:?}") }),
            false,
        );
        Self {
            enabled: true,
            mode,
            decision: decision.clone(),
            decisions: vec![decision],
            optimization_authority: OptimizationAuthorityReport::materialized_fallback(
                "materialized baseline executed after kernel fallback",
            ),
            counters: KernelCounters::default(),
            metrics: KernelMetricSnapshot::default(),
            fallback_reason: Some(reason),
            compared_with_materialized: false,
            materialized_fingerprint: None,
            kernel_fingerprint: None,
            association: AssociationOptimizationReport::default(),
            evidence: EvidenceOptimizationReport::default(),
            lineage: LineageReuseReport::default(),
        }
    }

    pub fn applied(mode: KernelExecutionMode, counters: KernelCounters) -> Self {
        let decision = KernelDecision::new(
            KernelDecisionKind::Applied,
            "mechanical-sympathy kernel path executed with guarded residual verification",
            json!({}),
            false,
        );
        Self {
            enabled: true,
            mode,
            decision: decision.clone(),
            decisions: vec![decision],
            optimization_authority: OptimizationAuthorityReport::residual_required(
                "kernel pruning is exact/no-false-negative and residual checks remain inside the optimized execution path",
            ),
            metrics: KernelMetricSnapshot {
                rows_scanned: counters.rows_scanned,
                rows_pruned_by_bitmap: counters
                    .rows_scanned
                    .saturating_sub(counters.rows_after_bitmap),
                rows_pruned_by_selection_vector: counters
                    .rows_after_bitmap
                    .saturating_sub(counters.rows_after_selection_vector),
                residual_rows_checked: counters.residual_rows_checked,
                coded_predicate_rows: counters.coded_predicate_rows,
                typed_predicate_rows: counters.typed_predicate_rows,
                scratch_high_water_bytes: counters.scratch_high_water_bytes,
                bytes_touched_estimate: counters.bytes_touched_estimate,
                dictionary_lookups_at_materialization: counters
                    .dictionary_lookups_at_materialization,
                final_materialization_rows: counters.output_rows,
            },
            counters,
            fallback_reason: None,
            compared_with_materialized: false,
            materialized_fingerprint: None,
            kernel_fingerprint: None,
            association: AssociationOptimizationReport::default(),
            evidence: EvidenceOptimizationReport::default(),
            lineage: LineageReuseReport::default(),
        }
    }

    pub fn to_json(&self, allow_protected: bool) -> Value {
        let counters = if allow_protected {
            serde_json::to_value(&self.counters).unwrap_or(Value::Null)
        } else {
            json!({
                "redacted": true,
                "rows_scanned_present": self.counters.rows_scanned > 0,
                "selection_used": self.counters.bitmap_words > 0 || self.counters.selection_vector_len > 0,
                "output_rows_present": self.counters.output_rows > 0,
            })
        };
        json!({
            "enabled": self.enabled,
            "mode": self.mode,
            "decision": self.decision,
            "decisions": self.decisions,
            "optimization_authority": self.optimization_authority,
            "counters": counters,
            "metrics": if allow_protected { serde_json::to_value(&self.metrics).unwrap_or(Value::Null) } else { json!({ "redacted": true }) },
            "fallback_reason": self.fallback_reason,
            "compared_with_materialized": self.compared_with_materialized,
            "materialized_fingerprint": if allow_protected { self.materialized_fingerprint.clone().map(Value::String).unwrap_or(Value::Null) } else { Value::Null },
            "kernel_fingerprint": if allow_protected { self.kernel_fingerprint.clone().map(Value::String).unwrap_or(Value::Null) } else { Value::Null },
            "association": self.association.to_json(allow_protected),
            "evidence": self.evidence.to_json(allow_protected),
            "lineage": self.lineage.to_json(allow_protected),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizationAuthorityReport {
    pub state: OptimizationAuthorityState,
    pub authoritative: bool,
    pub candidate_only: bool,
    pub residual_required: bool,
    pub materialized_fallback: bool,
    pub reason: String,
}

impl OptimizationAuthorityReport {
    pub fn authoritative(reason: impl Into<String>) -> Self {
        Self {
            state: OptimizationAuthorityState::Authoritative,
            authoritative: true,
            candidate_only: false,
            residual_required: false,
            materialized_fallback: false,
            reason: reason.into(),
        }
    }

    pub fn residual_required(reason: impl Into<String>) -> Self {
        Self {
            state: OptimizationAuthorityState::ResidualRequired,
            authoritative: true,
            candidate_only: false,
            residual_required: true,
            materialized_fallback: false,
            reason: reason.into(),
        }
    }

    pub fn candidate_only(reason: impl Into<String>) -> Self {
        Self {
            state: OptimizationAuthorityState::CandidateOnly,
            authoritative: false,
            candidate_only: true,
            residual_required: true,
            materialized_fallback: false,
            reason: reason.into(),
        }
    }

    pub fn materialized_fallback(reason: impl Into<String>) -> Self {
        Self {
            state: OptimizationAuthorityState::MaterializedFallback,
            authoritative: true,
            candidate_only: false,
            residual_required: false,
            materialized_fallback: true,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationAuthorityState {
    Authoritative,
    CandidateOnly,
    ResidualRequired,
    MaterializedFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelDecision {
    pub kind: KernelDecisionKind,
    pub reason: String,
    pub safe_details: Value,
    pub redacted: bool,
}

impl KernelDecision {
    pub fn new(
        kind: KernelDecisionKind,
        reason: impl Into<String>,
        safe_details: Value,
        redacted: bool,
    ) -> Self {
        Self {
            kind,
            reason: reason.into(),
            safe_details,
            redacted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelDecisionKind {
    Applied,
    Compared,
    Disabled,
    Fallback,
    Rejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelCounters {
    pub rows_scanned: usize,
    pub rows_after_bitmap: usize,
    pub rows_after_selection_vector: usize,
    pub candidate_object_keys: usize,
    pub retained_record_chain_rows: usize,
    pub reconstructed_states: usize,
    pub residual_rows_checked: usize,
    pub output_rows: usize,
    pub bitmap_words: usize,
    pub selection_vector_len: usize,
    pub coded_predicate_rows: usize,
    pub typed_predicate_rows: usize,
    pub dictionary_lookups_at_materialization: usize,
    pub bytes_touched_estimate: usize,
    pub scratch_high_water_bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelMetricSnapshot {
    pub rows_scanned: usize,
    pub rows_pruned_by_bitmap: usize,
    pub rows_pruned_by_selection_vector: usize,
    pub residual_rows_checked: usize,
    pub coded_predicate_rows: usize,
    pub typed_predicate_rows: usize,
    pub scratch_high_water_bytes: usize,
    pub bytes_touched_estimate: usize,
    pub dictionary_lookups_at_materialization: usize,
    pub final_materialization_rows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelFallbackReason {
    DisabledByOptions,
    ExplainOnly,
    PhysicalPlanRequired,
    NonObjectRoot,
    UnsupportedOutputMode,
    UnsupportedTemporalMode,
    UnsupportedMethod,
    UnsupportedPredicate,
    UnsupportedProjection,
    UnsafeCodedPredicate,
    AmbiguousAssociationRole,
    MissingAssociationEndpointFlags,
    UnsupportedEvidenceGrain,
    ProtectedEvidenceExistence,
    ProtectedEndpointDisclosure,
    LineageDependencyFallback,
    MaterializedComparisonMismatch,
    ReadbackFallback,
}
