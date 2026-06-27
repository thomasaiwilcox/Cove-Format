//! COVE-native scan program compilation.
//!
//! This module is intentionally DataFusion-agnostic. It records the predicate
//! shape Cove has promised to evaluate and the conservative exactness contract
//! used by DataFusion pushdown classification.

use cove_core::{
    constants::{CoveLogicalType, CovePhysicalKind},
    index::lookup::LookupKeyKind,
    native::{
        NativeDecodeKernel, NativePredicateCost, NativePredicateExactness, NativeScanOp,
        NativeScanProgram,
    },
};

use crate::{
    dataset_state::DatasetState,
    planner::{CoveFilterUse, CovePredicate, FilterPlan, NumericPredicateOp},
};

pub type PredicateExactness = NativePredicateExactness;
pub type DecodeKernel = NativeDecodeKernel;
pub type PredicateCost = NativePredicateCost;
pub type ScanOp = NativeScanOp;
pub type CoveScanProgram = NativeScanProgram;

/// Promote a lowered filter only when Cove can evaluate the full row predicate
/// itself. Unsupported or advisory-only predicates stay pruning-only so
/// DataFusion keeps the residual filter.
pub fn promote_filter_exactness(state: &DatasetState, filter: &mut FilterPlan) {
    if filter.use_kind == CoveFilterUse::Unsupported {
        return;
    }
    filter.use_kind = match filter_exactness(state, filter) {
        PredicateExactness::FullRowPredicateExact => CoveFilterUse::FullRowPredicateExact,
        PredicateExactness::PruningOnly => CoveFilterUse::PruningOnly,
    };
}

pub fn compile_scan_program(state: &DatasetState, filters: &[FilterPlan]) -> CoveScanProgram {
    let mut program = CoveScanProgram::default();
    for filter in filters {
        let exactness = match filter.use_kind {
            CoveFilterUse::FullRowPredicateExact => {
                program.exact_filters += 1;
                PredicateExactness::FullRowPredicateExact
            }
            CoveFilterUse::PruningOnly => {
                program.inexact_filters += 1;
                PredicateExactness::PruningOnly
            }
            CoveFilterUse::Unsupported => continue,
        };
        let Some(predicate) = &filter.predicate else {
            continue;
        };
        let Some(column_index) = predicate_column_index(predicate) else {
            continue;
        };
        let Some(column) = state.table().columns.get(column_index) else {
            continue;
        };
        let kernel = predicate_kernel(predicate);
        let op = match predicate {
            CovePredicate::Null { .. } => ScanOp::Null {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
            },
            CovePredicate::Numeric { .. } => ScanOp::Numeric {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
            },
            CovePredicate::NumericIn { literals, .. } => ScanOp::NumericIn {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_count: literals.len(),
            },
            CovePredicate::NumericNotIn { literals, .. } => ScanOp::NumericNotIn {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_count: literals.len(),
            },
            CovePredicate::FileCodeIn {
                file_codes,
                canonical_values,
                canonical_keys,
                ..
            } => ScanOp::FileCodeIn {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_count: file_codes
                    .len()
                    .max(canonical_values.len())
                    .max(canonical_keys.len()),
            },
            CovePredicate::FileCodeNotIn {
                file_codes,
                canonical_values,
                canonical_keys,
                ..
            } => ScanOp::FileCodeNotIn {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_count: file_codes
                    .len()
                    .max(canonical_values.len())
                    .max(canonical_keys.len()),
            },
            CovePredicate::FixedBytesEq { literal, .. } => ScanOp::FixedBytesEq {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_len: literal.len(),
            },
            CovePredicate::FixedBytesIn { literals, .. } => ScanOp::FixedBytesIn {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_count: literals.len(),
                literal_len: literals.first().map_or(0, Vec::len),
            },
            CovePredicate::VarBytesEq { literal, .. } => ScanOp::VarBytesEq {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_len: literal.len(),
            },
            CovePredicate::VarBytesIn { literals, .. } => ScanOp::VarBytesIn {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_count: literals.len(),
            },
            CovePredicate::VarBytesPrefix { prefix, .. } => ScanOp::VarBytesPrefix {
                column_index,
                column_id: column.column_id,
                exactness,
                kernel,
                literal_len: prefix.len(),
            },
        };
        program.ops.push(op);
    }
    program.lookup_rowref_eligible = lookup_rowref_eligible(state, filters);
    program
}

pub fn order_filters_by_cost(filters: &mut [FilterPlan]) -> bool {
    let before = filters.iter().map(filter_order_key).collect::<Vec<_>>();
    let mut indexed = filters.iter().cloned().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(index, filter)| (filter_order_key(filter), *index));
    for (slot, (_, filter)) in filters.iter_mut().zip(indexed) {
        *slot = filter;
    }
    filters.iter().map(filter_order_key).ne(before)
}

pub fn predicate_cost(filter: &FilterPlan) -> PredicateCost {
    if filter.use_kind != CoveFilterUse::FullRowPredicateExact {
        return PredicateCost::ResidualOrUnsupported;
    }
    match filter.predicate {
        Some(CovePredicate::Null { .. }) => PredicateCost::NullBitmap,
        Some(
            CovePredicate::Numeric { .. }
            | CovePredicate::NumericIn { .. }
            | CovePredicate::NumericNotIn { .. },
        ) => PredicateCost::NumericCode,
        Some(CovePredicate::FileCodeIn { .. } | CovePredicate::FileCodeNotIn { .. }) => {
            PredicateCost::FileCode
        }
        Some(CovePredicate::FixedBytesEq { .. } | CovePredicate::FixedBytesIn { .. }) => {
            PredicateCost::VarBytes
        }
        Some(
            CovePredicate::VarBytesEq { .. }
            | CovePredicate::VarBytesIn { .. }
            | CovePredicate::VarBytesPrefix { .. },
        ) => PredicateCost::VarBytes,
        None => PredicateCost::ResidualOrUnsupported,
    }
}

fn filter_order_key(filter: &FilterPlan) -> (u8, PredicateCost) {
    match filter.use_kind {
        CoveFilterUse::FullRowPredicateExact => (0, predicate_cost(filter)),
        CoveFilterUse::PruningOnly => (1, PredicateCost::ResidualOrUnsupported),
        CoveFilterUse::Unsupported => (2, PredicateCost::ResidualOrUnsupported),
    }
}

fn filter_exactness(state: &DatasetState, filter: &FilterPlan) -> PredicateExactness {
    let Some(predicate) = &filter.predicate else {
        return PredicateExactness::PruningOnly;
    };
    match predicate {
        CovePredicate::Null { column_index, .. } => {
            if state.table().columns.get(*column_index).is_some() {
                PredicateExactness::FullRowPredicateExact
            } else {
                PredicateExactness::PruningOnly
            }
        }
        CovePredicate::Numeric { column_index, .. } => {
            let Some(column) = state.table().columns.get(*column_index) else {
                return PredicateExactness::PruningOnly;
            };
            if column.physical == CovePhysicalKind::NumCode {
                PredicateExactness::FullRowPredicateExact
            } else {
                PredicateExactness::PruningOnly
            }
        }
        CovePredicate::NumericIn { column_index, .. } => {
            let Some(column) = state.table().columns.get(*column_index) else {
                return PredicateExactness::PruningOnly;
            };
            if column.physical == CovePhysicalKind::NumCode {
                PredicateExactness::FullRowPredicateExact
            } else {
                PredicateExactness::PruningOnly
            }
        }
        CovePredicate::NumericNotIn { column_index, .. } => {
            let Some(column) = state.table().columns.get(*column_index) else {
                return PredicateExactness::PruningOnly;
            };
            if column.physical == CovePhysicalKind::NumCode
                && non_float_numeric_complement_supported(column.logical)
            {
                PredicateExactness::FullRowPredicateExact
            } else {
                PredicateExactness::PruningOnly
            }
        }
        CovePredicate::FileCodeIn {
            column_index,
            canonical_values,
            ..
        }
        | CovePredicate::FileCodeNotIn {
            column_index,
            canonical_values,
            ..
        } => {
            let Some(column) = state.table().columns.get(*column_index) else {
                return PredicateExactness::PruningOnly;
            };
            if column.physical != CovePhysicalKind::FileCode {
                return PredicateExactness::PruningOnly;
            }
            if state.files().iter().any(|file| file.has_redaction()) {
                return PredicateExactness::PruningOnly;
            }
            if !canonical_values.is_empty()
                && state
                    .files()
                    .iter()
                    .any(|file| file.mounted().reverse_lookup.is_none())
            {
                return PredicateExactness::PruningOnly;
            }
            PredicateExactness::FullRowPredicateExact
        }
        CovePredicate::FixedBytesEq { column_index, .. }
        | CovePredicate::FixedBytesIn { column_index, .. } => {
            let Some(column) = state.table().columns.get(*column_index) else {
                return PredicateExactness::PruningOnly;
            };
            if column.physical == CovePhysicalKind::FixedBytes {
                PredicateExactness::FullRowPredicateExact
            } else {
                PredicateExactness::PruningOnly
            }
        }
        CovePredicate::VarBytesEq { column_index, .. }
        | CovePredicate::VarBytesIn { column_index, .. } => {
            let Some(column) = state.table().columns.get(*column_index) else {
                return PredicateExactness::PruningOnly;
            };
            if column.physical == CovePhysicalKind::VarBytes
                && column.logical == CoveLogicalType::Utf8
            {
                PredicateExactness::FullRowPredicateExact
            } else {
                PredicateExactness::PruningOnly
            }
        }
        CovePredicate::VarBytesPrefix { column_index, .. } => {
            let Some(column) = state.table().columns.get(*column_index) else {
                return PredicateExactness::PruningOnly;
            };
            if column.physical == CovePhysicalKind::VarBytes
                && column.logical == CoveLogicalType::Utf8
            {
                PredicateExactness::FullRowPredicateExact
            } else {
                PredicateExactness::PruningOnly
            }
        }
    }
}

fn lookup_rowref_eligible(state: &DatasetState, filters: &[FilterPlan]) -> bool {
    let exact_row_predicates = filters
        .iter()
        .filter(|filter| filter.use_kind == CoveFilterUse::FullRowPredicateExact)
        .filter(|filter| {
            matches!(
                filter.predicate,
                Some(
                    CovePredicate::Numeric {
                        op: NumericPredicateOp::Eq,
                        ..
                    } | CovePredicate::NumericIn { .. }
                        | CovePredicate::FileCodeIn { .. }
                )
            )
        })
        .collect::<Vec<_>>();
    if exact_row_predicates.len() != 1 || filters.len() != 1 {
        return false;
    }
    let Some(predicate) = &exact_row_predicates[0].predicate else {
        return false;
    };
    let (column_index, key_kind) = match predicate {
        CovePredicate::FileCodeIn { column_index, .. } => (*column_index, LookupKeyKind::FileCode),
        CovePredicate::Numeric {
            column_index,
            op: NumericPredicateOp::Eq,
            literal,
        } if state
            .table()
            .columns
            .get(*column_index)
            .is_some_and(|column| {
                !crate::decode::numeric_lookup_keys(column.logical, *literal).is_empty()
            }) =>
        {
            (*column_index, LookupKeyKind::NumCode)
        }
        CovePredicate::NumericIn {
            column_index,
            literals,
        } if state
            .table()
            .columns
            .get(*column_index)
            .is_some_and(|column| !numeric_in_lookup_keys(column.logical, literals).is_empty()) =>
        {
            (*column_index, LookupKeyKind::NumCode)
        }
        _ => return false,
    };
    let Some(column) = state.table().columns.get(column_index) else {
        return false;
    };
    state.files().iter().all(|file| {
        file.pruning().lookups.iter().any(|index| {
            index.header.column_id == column.column_id && index.header.key_kind == key_kind
        })
    })
}

fn predicate_kernel(predicate: &CovePredicate) -> DecodeKernel {
    match predicate {
        CovePredicate::Null { .. } => DecodeKernel::NullBitmap,
        CovePredicate::Numeric { .. }
        | CovePredicate::NumericIn { .. }
        | CovePredicate::NumericNotIn { .. } => DecodeKernel::PreparedNumCode,
        CovePredicate::FileCodeIn { .. } | CovePredicate::FileCodeNotIn { .. } => {
            DecodeKernel::PreparedFileCode
        }
        CovePredicate::FixedBytesEq { .. } | CovePredicate::FixedBytesIn { .. } => {
            DecodeKernel::PreparedFixedBytes
        }
        CovePredicate::VarBytesEq { .. }
        | CovePredicate::VarBytesIn { .. }
        | CovePredicate::VarBytesPrefix { .. } => DecodeKernel::PreparedVarBytes,
    }
}

fn predicate_column_index(predicate: &CovePredicate) -> Option<usize> {
    match predicate {
        CovePredicate::Null { column_index, .. }
        | CovePredicate::Numeric { column_index, .. }
        | CovePredicate::NumericIn { column_index, .. }
        | CovePredicate::NumericNotIn { column_index, .. }
        | CovePredicate::FileCodeIn { column_index, .. }
        | CovePredicate::FileCodeNotIn { column_index, .. }
        | CovePredicate::FixedBytesEq { column_index, .. }
        | CovePredicate::FixedBytesIn { column_index, .. }
        | CovePredicate::VarBytesPrefix { column_index, .. }
        | CovePredicate::VarBytesIn { column_index, .. }
        | CovePredicate::VarBytesEq { column_index, .. } => Some(*column_index),
    }
}

fn non_float_numeric_complement_supported(logical: CoveLogicalType) -> bool {
    !matches!(logical, CoveLogicalType::Float32 | CoveLogicalType::Float64)
}

fn numeric_in_lookup_keys(
    logical: CoveLogicalType,
    literals: &[crate::planner::PredicateLiteral],
) -> Vec<u64> {
    let mut keys = Vec::new();
    for literal in literals {
        for key in crate::decode::numeric_lookup_keys(logical, *literal) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{FilterPlan, NumericPredicateOp, PredicateLiteral};

    #[test]
    fn order_filters_by_cost_puts_exact_numeric_before_filecode_and_residuals() {
        let mut filters = vec![
            FilterPlan::pruning_file_code_in(1, vec![7], "category IN"),
            FilterPlan::unsupported("unsupported"),
            FilterPlan::pruning_numeric(
                0,
                NumericPredicateOp::Eq,
                PredicateLiteral::UInt64(42),
                "id = 42",
            ),
        ];
        filters[0].use_kind = CoveFilterUse::FullRowPredicateExact;
        filters[2].use_kind = CoveFilterUse::FullRowPredicateExact;

        assert!(order_filters_by_cost(&mut filters));

        assert!(matches!(
            filters[0].predicate,
            Some(CovePredicate::Numeric { .. })
        ));
        assert!(matches!(
            filters[1].predicate,
            Some(CovePredicate::FileCodeIn { .. })
        ));
        assert_eq!(filters[2].use_kind, CoveFilterUse::Unsupported);
    }

    #[test]
    fn order_filters_by_cost_is_stable_for_equal_cost_filters() {
        let mut filters = vec![
            FilterPlan::pruning_numeric(
                1,
                NumericPredicateOp::Gt,
                PredicateLiteral::UInt64(10),
                "b > 10",
            ),
            FilterPlan::pruning_numeric(
                0,
                NumericPredicateOp::Eq,
                PredicateLiteral::UInt64(1),
                "a = 1",
            ),
        ];
        for filter in &mut filters {
            filter.use_kind = CoveFilterUse::FullRowPredicateExact;
        }

        assert!(!order_filters_by_cost(&mut filters));
        assert_eq!(filters[0].display, "b > 10");
        assert_eq!(filters[1].display, "a = 1");
    }

    #[test]
    fn scan_program_contract_is_shared_with_cove_core_native() {
        let op = ScanOp::Null {
            column_index: 1,
            column_id: 9,
            exactness: PredicateExactness::FullRowPredicateExact,
            kernel: DecodeKernel::NullBitmap,
        };
        let native_op: cove_core::native::NativeScanOp = op.clone();
        assert_eq!(native_op, op);

        let program = CoveScanProgram {
            ops: vec![op],
            exact_filters: 1,
            inexact_filters: 0,
            lookup_rowref_eligible: false,
            predicate_ordered: true,
        };
        let native_program: cove_core::native::NativeScanProgram = program.clone();
        assert_eq!(native_program, program);
    }
}
