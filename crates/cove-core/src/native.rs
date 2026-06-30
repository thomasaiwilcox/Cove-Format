//! Shared native execution primitives for Cove readers.
//!
//! This module is the scalar, safe-Rust foundation for Cove-native execution.
//! It keeps hot-path state as compact lanes, validity views, row sets, and
//! selection buffers so higher-level engines can avoid row-wise materialization.

use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
    hash::Hash,
};

use crate::{
    array::logical_type_fixed_width,
    constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind},
    encoding::local_codebook::{LocalCodebookPayload, LocalCodebookValues},
    page::{PAGE_FLAG_ALL_NON_NULL, PAGE_FLAG_ALL_NULL},
    page_payload::{
        ColumnPagePayloadV1, CoveEncodingNodeV1, PageBufferKind, RetainedColumnPagePayloadV1,
    },
    profile::cove_o::{RetainedTemporalSegmentData, TemporalRowEntryV1, TemporalSegmentData},
    segment::{TableColumnDirectoryEntryV1, TableSegmentPayloadV1},
    wire, CoveError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeLaneId(pub u32);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NativeKernelDispatch {
    #[default]
    Scalar,
    Auto,
    Avx2,
    Neon,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KernelStats {
    pub rows_seen: usize,
    pub rows_valid: usize,
    pub rows_matched: usize,
    pub bitmap_words_touched: usize,
    pub bytes_touched_estimate: usize,
    pub dispatch: NativeKernelDispatch,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeScanPlan {
    pub row_count: usize,
    pub projected_lanes: Vec<NativeLaneId>,
    pub predicate_count: usize,
    pub dispatch: NativeKernelDispatch,
    pub scan_program: NativeScanProgram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePredicateExactness {
    PruningOnly,
    FullRowPredicateExact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDecodeKernel {
    NullBitmap,
    DirectFileCode,
    PreparedFileCode,
    PreparedNumCode,
    PreparedFixedBytes,
    PreparedVarBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NativePredicateCost {
    NullBitmap,
    NumericCode,
    FileCode,
    VarBytes,
    ResidualOrUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeScanOp {
    Null {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
    },
    Numeric {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
    },
    NumericIn {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_count: usize,
    },
    NumericNotIn {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_count: usize,
    },
    FileCodeIn {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_count: usize,
    },
    FileCodeNotIn {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_count: usize,
    },
    FixedBytesEq {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_len: usize,
    },
    FixedBytesIn {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_count: usize,
        literal_len: usize,
    },
    VarBytesEq {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_len: usize,
    },
    VarBytesIn {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_count: usize,
    },
    VarBytesPrefix {
        column_index: usize,
        column_id: u32,
        exactness: NativePredicateExactness,
        kernel: NativeDecodeKernel,
        literal_len: usize,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeScanProgram {
    pub ops: Vec<NativeScanOp>,
    pub exact_filters: usize,
    pub inexact_filters: usize,
    pub lookup_rowref_eligible: bool,
    pub predicate_ordered: bool,
}

impl NativeScanProgram {
    pub fn display_summary(&self) -> String {
        format!(
            "ops={}, exact_filters={}, inexact_filters={}, lookup_rowref_eligible={}, predicate_ordered={}",
            self.ops.len(),
            self.exact_filters,
            self.inexact_filters,
            self.lookup_rowref_eligible,
            self.predicate_ordered
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeCodeDomain {
    pub file_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub table_id: Option<u32>,
    pub object_type_id: Option<u32>,
    pub property_id: Option<u32>,
    pub column_id: Option<u32>,
    pub dictionary_id: Option<String>,
    pub semantic_domain_id: Option<String>,
    pub dictionary_epoch: Option<u64>,
    pub collation_id: Option<u16>,
    pub null_policy: Option<String>,
    pub security_scope: Option<String>,
}

impl NativeCodeDomain {
    pub fn code_equality_compatible(&self, other: &Self) -> bool {
        let has_domain = self.semantic_domain_id.is_some()
            || self.dictionary_id.is_some()
            || self.table_id.is_some()
            || self.object_type_id.is_some()
            || self.property_id.is_some()
            || self.column_id.is_some();
        has_domain
            && self.file_id == other.file_id
            && self.snapshot_id == other.snapshot_id
            && self.table_id == other.table_id
            && self.object_type_id == other.object_type_id
            && self.property_id == other.property_id
            && self.column_id == other.column_id
            && self.dictionary_id == other.dictionary_id
            && self.semantic_domain_id == other.semantic_domain_id
            && self.dictionary_epoch == other.dictionary_epoch
            && self.collation_id == other.collation_id
            && self.null_policy == other.null_policy
            && self.security_scope == other.security_scope
    }
}

mod filters;
mod lanes;
mod pages;
mod relational;

pub use filters::*;
pub use lanes::*;
pub use pages::*;
pub use relational::*;

#[cfg(test)]
mod tests;
