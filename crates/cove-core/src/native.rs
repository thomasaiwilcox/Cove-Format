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
    CoveError,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidityRef<'a> {
    AllValid { row_count: usize },
    AllNull { row_count: usize },
    CoveNullBitmap { bytes: &'a [u8], row_count: usize },
}

impl<'a> ValidityRef<'a> {
    pub fn row_count(self) -> usize {
        match self {
            Self::AllValid { row_count }
            | Self::AllNull { row_count }
            | Self::CoveNullBitmap { row_count, .. } => row_count,
        }
    }

    pub fn is_valid(self, row: usize) -> bool {
        match self {
            Self::AllValid { row_count } => row < row_count,
            Self::AllNull { .. } => false,
            Self::CoveNullBitmap { bytes, row_count } => {
                if row >= row_count {
                    return false;
                }
                let Some(byte) = bytes.get(row / 8).copied() else {
                    return false;
                };
                ((byte >> (row % 8)) & 1) == 0
            }
        }
    }

    pub fn valid_count(self) -> usize {
        let row_count = self.row_count();
        match self {
            Self::AllValid { .. } => row_count,
            Self::AllNull { .. } => 0,
            Self::CoveNullBitmap { .. } => {
                let mut count = 0usize;
                for row in 0..row_count {
                    count += self.is_valid(row) as usize;
                }
                count
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LaneRef<'a> {
    Bool {
        lane_id: NativeLaneId,
        values: &'a [u8],
        row_count: usize,
        validity: ValidityRef<'a>,
        domain: NativeCodeDomain,
    },
    NumCodeU64 {
        lane_id: NativeLaneId,
        values: &'a [u64],
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    NumCodeU64LeBytes {
        lane_id: NativeLaneId,
        bytes: &'a [u8],
        row_count: usize,
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    NumCodeI64 {
        lane_id: NativeLaneId,
        values: &'a [i64],
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    NumCodeI64LeBytes {
        lane_id: NativeLaneId,
        bytes: &'a [u8],
        row_count: usize,
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    FileCodeU32 {
        lane_id: NativeLaneId,
        values: &'a [u32],
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    FileCodeU32LeBytes {
        lane_id: NativeLaneId,
        bytes: &'a [u8],
        row_count: usize,
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    LocalCodeU8 {
        lane_id: NativeLaneId,
        values: Cow<'a, [u8]>,
        validity: ValidityRef<'a>,
        local_to_global: Cow<'a, [u64]>,
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        domain: NativeCodeDomain,
    },
    LocalCodeU16 {
        lane_id: NativeLaneId,
        values: Cow<'a, [u16]>,
        validity: ValidityRef<'a>,
        local_to_global: Cow<'a, [u64]>,
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        domain: NativeCodeDomain,
    },
    LocalCodeU32 {
        lane_id: NativeLaneId,
        values: Cow<'a, [u32]>,
        validity: ValidityRef<'a>,
        local_to_global: Cow<'a, [u64]>,
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        domain: NativeCodeDomain,
    },
    FixedBytes {
        lane_id: NativeLaneId,
        values: &'a [u8],
        width: usize,
        row_count: usize,
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    VarBytes {
        lane_id: NativeLaneId,
        row_offsets: Cow<'a, [u32]>,
        values: &'a [u8],
        validity: ValidityRef<'a>,
        logical_type: CoveLogicalType,
        domain: NativeCodeDomain,
    },
    DecodeBoundary {
        lane_id: NativeLaneId,
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        row_count: usize,
        validity: Option<ValidityRef<'a>>,
        reason: &'a str,
    },
}

impl<'a> LaneRef<'a> {
    pub fn lane_id(&self) -> NativeLaneId {
        match self {
            Self::Bool { lane_id, .. }
            | Self::NumCodeU64 { lane_id, .. }
            | Self::NumCodeU64LeBytes { lane_id, .. }
            | Self::NumCodeI64 { lane_id, .. }
            | Self::NumCodeI64LeBytes { lane_id, .. }
            | Self::FileCodeU32 { lane_id, .. }
            | Self::FileCodeU32LeBytes { lane_id, .. }
            | Self::LocalCodeU8 { lane_id, .. }
            | Self::LocalCodeU16 { lane_id, .. }
            | Self::LocalCodeU32 { lane_id, .. }
            | Self::FixedBytes { lane_id, .. }
            | Self::VarBytes { lane_id, .. }
            | Self::DecodeBoundary { lane_id, .. } => *lane_id,
        }
    }

    pub fn row_count(&self) -> usize {
        match self {
            Self::Bool { row_count, .. } => *row_count,
            Self::NumCodeU64 { values, .. } => values.len(),
            Self::NumCodeU64LeBytes { row_count, .. } => *row_count,
            Self::NumCodeI64 { values, .. } => values.len(),
            Self::NumCodeI64LeBytes { row_count, .. } => *row_count,
            Self::FileCodeU32 { values, .. } => values.len(),
            Self::FileCodeU32LeBytes { row_count, .. } => *row_count,
            Self::LocalCodeU8 { values, .. } => values.len(),
            Self::LocalCodeU16 { values, .. } => values.len(),
            Self::LocalCodeU32 { values, .. } => values.len(),
            Self::FixedBytes { row_count, .. } => *row_count,
            Self::VarBytes { row_offsets, .. } => row_offsets.len(),
            Self::DecodeBoundary { row_count, .. } => *row_count,
        }
    }

    pub fn validity(&self) -> Option<ValidityRef<'a>> {
        match self {
            Self::Bool { validity, .. }
            | Self::NumCodeU64 { validity, .. }
            | Self::NumCodeU64LeBytes { validity, .. }
            | Self::NumCodeI64 { validity, .. }
            | Self::NumCodeI64LeBytes { validity, .. }
            | Self::FileCodeU32 { validity, .. }
            | Self::FileCodeU32LeBytes { validity, .. }
            | Self::LocalCodeU8 { validity, .. }
            | Self::LocalCodeU16 { validity, .. }
            | Self::LocalCodeU32 { validity, .. }
            | Self::FixedBytes { validity, .. }
            | Self::VarBytes { validity, .. } => Some(*validity),
            Self::DecodeBoundary { validity, .. } => *validity,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeObjectTemporalBatch<'a> {
    pub segment_id: u32,
    pub object_type_id: u32,
    pub row_count: usize,
    pub rows: &'a [TemporalRowEntryV1],
    pub property_pages: Vec<NativeObjectPropertyPage<'a>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeObjectPropertyPage<'a> {
    pub lane_id: NativeLaneId,
    pub property_id: u32,
    pub morsel_id: u32,
    pub row_start: usize,
    pub row_count: usize,
    pub lane: LaneRef<'a>,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeTablePageRef<'a, P: NativeColumnPagePayload + ?Sized> {
    pub column: &'a TableColumnDirectoryEntryV1,
    pub page: &'a crate::page::ColumnPageIndexEntryV1,
    pub payload: Option<&'a P>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeTableBatch<'a> {
    pub table_id: u32,
    pub segment_id: u32,
    pub row_start: u64,
    pub row_count: usize,
    pub column_pages: Vec<NativeTableColumnPage<'a>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeTableColumnPage<'a> {
    pub lane_id: NativeLaneId,
    pub column_id: u32,
    pub morsel_id: u32,
    pub row_start_in_segment: usize,
    pub row_start: u64,
    pub row_count: usize,
    pub lane: LaneRef<'a>,
}

pub fn native_table_batch_from_page_refs<'a, P: NativeColumnPagePayload + ?Sized>(
    segment: &'a TableSegmentPayloadV1,
    page_refs: &[NativeTablePageRef<'a, P>],
    base_domain: NativeCodeDomain,
) -> Result<NativeTableBatch<'a>, CoveError> {
    let row_count =
        usize::try_from(segment.header.row_count).map_err(|_| CoveError::OffsetRange)?;
    let mut column_pages = Vec::with_capacity(page_refs.len());

    for page_ref in page_refs {
        let column = page_ref.column;
        if page_ref.page.column_id != column.column_id {
            return Err(CoveError::PageCorrupt);
        }
        if !segment.columns.iter().any(|candidate| {
            candidate.column_id == column.column_id
                && candidate.logical_type == column.logical_type
                && candidate.physical_kind == column.physical_kind
        }) {
            return Err(CoveError::SegmentCorrupt);
        }

        let morsel = segment.morsels.morsel_by_id(page_ref.page.morsel_id)?;
        if morsel.row_count != page_ref.page.row_count {
            return Err(CoveError::PageCorrupt);
        }
        let page_row_count =
            usize::try_from(page_ref.page.row_count).map_err(|_| CoveError::OffsetRange)?;
        let row_start_in_segment =
            usize::try_from(morsel.first_row_in_segment).map_err(|_| CoveError::OffsetRange)?;
        let row_end = row_start_in_segment
            .checked_add(page_row_count)
            .ok_or(CoveError::ArithOverflow)?;
        if row_end > row_count {
            return Err(CoveError::PageCorrupt);
        }
        let row_start = segment
            .header
            .row_start
            .checked_add(u64::from(morsel.first_row_in_segment))
            .ok_or(CoveError::ArithOverflow)?;

        let lane_id = NativeLaneId(column.column_id);
        let mut domain = base_domain.clone();
        domain.table_id.get_or_insert(segment.header.table_id);
        domain.column_id.get_or_insert(column.column_id);
        if column.domain_ref != 0 {
            domain
                .semantic_domain_id
                .get_or_insert_with(|| format!("table-domain:{}", column.domain_ref));
        }

        let lane = match page_ref.payload {
            Some(payload) => native_lane_from_object_page_payload(
                lane_id,
                column.logical_type,
                column.physical_kind,
                page_ref.page,
                payload,
                domain,
            )?,
            None => LaneRef::DecodeBoundary {
                lane_id,
                logical_type: column.logical_type,
                physical_kind: column.physical_kind,
                row_count: page_row_count,
                validity: elided_page_validity(page_ref.page),
                reason: decode_boundary_reason_for_elided_page(page_ref.page.flags),
            },
        };

        column_pages.push(NativeTableColumnPage {
            lane_id,
            column_id: column.column_id,
            morsel_id: page_ref.page.morsel_id,
            row_start_in_segment,
            row_start,
            row_count: page_row_count,
            lane,
        });
    }

    Ok(NativeTableBatch {
        table_id: segment.header.table_id,
        segment_id: segment.header.segment_id,
        row_start: segment.header.row_start,
        row_count,
        column_pages,
    })
}

pub fn native_object_temporal_batch_from_segment<'a>(
    segment: &'a TemporalSegmentData,
    base_domain: NativeCodeDomain,
) -> Result<NativeObjectTemporalBatch<'a>, CoveError> {
    let row_count =
        usize::try_from(segment.header.row_count).map_err(|_| CoveError::OffsetRange)?;
    if row_count != segment.rows.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut property_pages = Vec::new();
    for column in &segment.property_columns {
        for page in &column.pages {
            let page_row_count =
                usize::try_from(page.index_entry.row_count).map_err(|_| CoveError::OffsetRange)?;
            let row_start = usize::try_from(page.index_entry.morsel_id)
                .map_err(|_| CoveError::OffsetRange)?
                .checked_mul(
                    usize::try_from(segment.header.morsel_row_count)
                        .map_err(|_| CoveError::OffsetRange)?,
                )
                .ok_or(CoveError::ArithOverflow)?;
            let row_end = row_start
                .checked_add(page_row_count)
                .ok_or(CoveError::ArithOverflow)?;
            if row_end > row_count {
                return Err(CoveError::PageCorrupt);
            }

            let lane_id = NativeLaneId(column.directory.column_id);
            let mut domain = base_domain.clone();
            domain
                .object_type_id
                .get_or_insert(segment.header.object_type_id);
            domain.property_id.get_or_insert(column.directory.column_id);
            domain.column_id.get_or_insert(column.directory.column_id);

            let lane = match &page.payload {
                Some(payload) => native_lane_from_object_page_payload(
                    lane_id,
                    column.directory.logical_type,
                    column.directory.physical_kind,
                    &page.index_entry,
                    payload,
                    domain,
                )?,
                None => LaneRef::DecodeBoundary {
                    lane_id,
                    logical_type: column.directory.logical_type,
                    physical_kind: column.directory.physical_kind,
                    row_count: page_row_count,
                    validity: elided_page_validity(&page.index_entry),
                    reason: decode_boundary_reason_for_elided_page(page.index_entry.flags),
                },
            };

            property_pages.push(NativeObjectPropertyPage {
                lane_id,
                property_id: column.directory.column_id,
                morsel_id: page.index_entry.morsel_id,
                row_start,
                row_count: page_row_count,
                lane,
            });
        }
    }

    Ok(NativeObjectTemporalBatch {
        segment_id: segment.header.segment_id,
        object_type_id: segment.header.object_type_id,
        row_count,
        rows: &segment.rows,
        property_pages,
    })
}

pub fn native_object_temporal_batch_from_retained_segment<'a>(
    segment: &'a RetainedTemporalSegmentData,
    base_domain: NativeCodeDomain,
) -> Result<NativeObjectTemporalBatch<'a>, CoveError> {
    let row_count =
        usize::try_from(segment.header.row_count).map_err(|_| CoveError::OffsetRange)?;
    if row_count != segment.rows.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut property_pages = Vec::new();
    for column in &segment.property_columns {
        for page in &column.pages {
            let page_row_count =
                usize::try_from(page.index_entry.row_count).map_err(|_| CoveError::OffsetRange)?;
            let row_start = usize::try_from(page.index_entry.morsel_id)
                .map_err(|_| CoveError::OffsetRange)?
                .checked_mul(
                    usize::try_from(segment.header.morsel_row_count)
                        .map_err(|_| CoveError::OffsetRange)?,
                )
                .ok_or(CoveError::ArithOverflow)?;
            let row_end = row_start
                .checked_add(page_row_count)
                .ok_or(CoveError::ArithOverflow)?;
            if row_end > row_count {
                return Err(CoveError::PageCorrupt);
            }

            let lane_id = NativeLaneId(column.directory.column_id);
            let mut domain = base_domain.clone();
            domain
                .object_type_id
                .get_or_insert(segment.header.object_type_id);
            domain.property_id.get_or_insert(column.directory.column_id);
            domain.column_id.get_or_insert(column.directory.column_id);

            let lane = match &page.payload {
                Some(payload) => native_lane_from_object_page_payload(
                    lane_id,
                    column.directory.logical_type,
                    column.directory.physical_kind,
                    &page.index_entry,
                    payload,
                    domain,
                )?,
                None => LaneRef::DecodeBoundary {
                    lane_id,
                    logical_type: column.directory.logical_type,
                    physical_kind: column.directory.physical_kind,
                    row_count: page_row_count,
                    validity: elided_page_validity(&page.index_entry),
                    reason: decode_boundary_reason_for_elided_page(page.index_entry.flags),
                },
            };

            property_pages.push(NativeObjectPropertyPage {
                lane_id,
                property_id: column.directory.column_id,
                morsel_id: page.index_entry.morsel_id,
                row_start,
                row_count: page_row_count,
                lane,
            });
        }
    }

    Ok(NativeObjectTemporalBatch {
        segment_id: segment.header.segment_id,
        object_type_id: segment.header.object_type_id,
        row_count,
        rows: &segment.rows,
        property_pages,
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NativeBatch<'a> {
    pub row_count: usize,
    pub row_ids: Option<&'a [u32]>,
    pub lanes: Vec<LaneRef<'a>>,
}

impl<'a> NativeBatch<'a> {
    pub fn lane(&self, lane_id: NativeLaneId) -> Option<&LaneRef<'a>> {
        self.lanes.iter().find(|lane| lane.lane_id() == lane_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSet {
    All(usize),
    Bitmap(SelectionBitmap),
    Vector(SelectionVector),
}

impl RowSet {
    pub fn len(&self) -> usize {
        match self {
            Self::All(len) => *len,
            Self::Bitmap(bitmap) => bitmap.count_ones(),
            Self::Vector(vector) => vector.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn to_selection_vector(&self) -> SelectionVector {
        match self {
            Self::All(len) => SelectionVector {
                rows: (0..*len).map(|row| row as u32).collect(),
            },
            Self::Bitmap(bitmap) => bitmap.to_selection_vector(),
            Self::Vector(vector) => vector.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionBitmap {
    words: Vec<u64>,
    len: usize,
}

impl Default for SelectionBitmap {
    fn default() -> Self {
        Self::none(0)
    }
}

impl SelectionBitmap {
    pub fn all(len: usize) -> Self {
        let mut words = vec![u64::MAX; len.div_ceil(64)];
        mask_last_word(&mut words, len);
        Self { words, len }
    }

    pub fn none(len: usize) -> Self {
        Self {
            words: vec![0; len.div_ceil(64)],
            len,
        }
    }

    pub fn from_words(mut words: Vec<u64>, len: usize) -> Self {
        words.truncate(len.div_ceil(64));
        if words.len() < len.div_ceil(64) {
            words.resize(len.div_ceil(64), 0);
        }
        mask_last_word(&mut words, len);
        Self { words, len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn words(&self) -> &[u64] {
        &self.words
    }

    pub fn words_mut(&mut self) -> &mut [u64] {
        &mut self.words
    }

    pub fn set(&mut self, row: usize) {
        if row < self.len {
            self.words[row / 64] |= 1u64 << (row % 64);
        }
    }

    pub fn clear(&mut self, row: usize) {
        if row < self.len {
            self.words[row / 64] &= !(1u64 << (row % 64));
        }
    }

    pub fn clear_bit(&mut self, row: usize) {
        self.clear(row);
    }

    pub fn contains(&self, row: usize) -> bool {
        row < self.len && (self.words[row / 64] & (1u64 << (row % 64))) != 0
    }

    pub fn count_ones(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    pub fn clear_all(&mut self) {
        self.words.fill(0);
    }

    pub fn fill_all(&mut self, len: usize) {
        self.len = len;
        self.words.clear();
        self.words.resize(len.div_ceil(64), u64::MAX);
        mask_last_word(&mut self.words, self.len);
    }

    pub fn fill_none(&mut self, len: usize) {
        self.len = len;
        self.words.clear();
        self.words.resize(len.div_ceil(64), 0);
    }

    pub fn clone_from_mask(&mut self, other: &SelectionBitmap) {
        self.len = other.len;
        self.words.clone_from(&other.words);
    }

    pub fn intersect_with(&mut self, other: &SelectionBitmap) {
        debug_assert_eq!(self.len, other.len);
        intersect_words_scalar(&mut self.words, &other.words);
    }

    pub fn intersect_with_dispatch(
        &mut self,
        other: &SelectionBitmap,
        policy: NativeKernelDispatch,
    ) -> NativeKernelDispatch {
        debug_assert_eq!(self.len, other.len);
        if self.words.len() != other.words.len() {
            intersect_words_scalar(&mut self.words, &other.words);
            mask_last_word(&mut self.words, self.len);
            return NativeKernelDispatch::Scalar;
        }
        let dispatch = match policy {
            NativeKernelDispatch::Auto => intersect_words_auto(&mut self.words, &other.words),
            NativeKernelDispatch::Scalar
            | NativeKernelDispatch::Avx2
            | NativeKernelDispatch::Neon => {
                intersect_words_scalar(&mut self.words, &other.words);
                NativeKernelDispatch::Scalar
            }
        };
        mask_last_word(&mut self.words, self.len);
        dispatch
    }

    pub fn and_inplace(&mut self, other: &SelectionBitmap) {
        self.intersect_with(other);
    }

    pub fn and_inplace_dispatch(
        &mut self,
        other: &SelectionBitmap,
        policy: NativeKernelDispatch,
    ) -> NativeKernelDispatch {
        self.intersect_with_dispatch(other, policy)
    }

    pub fn all_zero(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    pub fn all_set(&self) -> bool {
        let full_words = self.len / 64;
        if self
            .words
            .get(..full_words)
            .is_some_and(|words| words.iter().any(|word| *word != u64::MAX))
        {
            return false;
        }
        let tail_bits = self.len % 64;
        if tail_bits == 0 {
            return self.words.len() == full_words;
        }
        let expected_tail = (1u64 << tail_bits) - 1;
        self.words.get(full_words).copied() == Some(expected_tail)
            && self.words.len() == full_words + 1
    }

    pub fn union_with(&mut self, other: &SelectionBitmap) {
        debug_assert_eq!(self.len, other.len);
        for (left, right) in self.words.iter_mut().zip(&other.words) {
            *left |= *right;
        }
        mask_last_word(&mut self.words, self.len);
    }

    pub fn retain_set_bits<F>(&mut self, mut keep: F)
    where
        F: FnMut(usize) -> bool,
    {
        for (word_index, word) in self.words.iter_mut().enumerate() {
            let mut live = *word;
            while live != 0 {
                let bit = live.trailing_zeros() as usize;
                let Some(row) = word_index
                    .checked_mul(64)
                    .and_then(|base| base.checked_add(bit))
                else {
                    break;
                };
                if row >= self.len {
                    break;
                }
                if !keep(row) {
                    *word &= !(1u64 << bit);
                }
                live &= live - 1;
            }
        }
    }

    pub fn to_selection_vector(&self) -> SelectionVector {
        let mut rows = Vec::with_capacity(self.count_ones());
        for (word_index, word) in self.words.iter().copied().enumerate() {
            let mut live = word;
            while live != 0 {
                let bit = live.trailing_zeros() as usize;
                let row = word_index * 64 + bit;
                if row < self.len {
                    rows.push(row as u32);
                }
                live &= live - 1;
            }
        }
        SelectionVector { rows }
    }

    pub fn write_selected_rows(&self, rows: &mut Vec<u32>) -> Result<(), CoveError> {
        let _ = compact_selection_bitmap_into(self, rows, NativeKernelDispatch::Scalar)?;
        Ok(())
    }
}

pub fn compact_selection_bitmap(
    bitmap: &SelectionBitmap,
    policy: NativeKernelDispatch,
) -> Result<(SelectionVector, KernelStats), CoveError> {
    let mut rows = Vec::with_capacity(bitmap.count_ones());
    let stats = compact_selection_bitmap_into(bitmap, &mut rows, policy)?;
    Ok((SelectionVector::from_rows(rows), stats))
}

pub fn compact_selection_bitmap_into(
    bitmap: &SelectionBitmap,
    rows: &mut Vec<u32>,
    policy: NativeKernelDispatch,
) -> Result<KernelStats, CoveError> {
    rows.clear();
    rows.reserve(bitmap.count_ones());
    let mut stats = KernelStats {
        rows_seen: bitmap.len(),
        bitmap_words_touched: bitmap.word_count(),
        bytes_touched_estimate: bitmap.word_count() * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (word_index, word) in bitmap.words.iter().copied().enumerate() {
        if word == 0 {
            continue;
        }
        let base = word_index.checked_mul(64).ok_or(CoveError::ArithOverflow)?;
        if word == u64::MAX {
            let end = base.checked_add(64).ok_or(CoveError::ArithOverflow)?;
            if end <= bitmap.len {
                let dispatch = append_dense_u32_rows_dispatch(rows, base, policy)?;
                if dispatch != NativeKernelDispatch::Scalar {
                    stats.dispatch = dispatch;
                }
                continue;
            }
        }
        let mut live = word;
        while live != 0 {
            let bit = live.trailing_zeros() as usize;
            let row = base.checked_add(bit).ok_or(CoveError::ArithOverflow)?;
            if row < bitmap.len {
                rows.push(u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?);
            }
            live &= live - 1;
        }
    }
    stats.rows_matched = rows.len();
    stats.rows_valid = rows.len();
    Ok(stats)
}

fn append_dense_u32_rows_dispatch(
    rows: &mut Vec<u32>,
    base: usize,
    policy: NativeKernelDispatch,
) -> Result<NativeKernelDispatch, CoveError> {
    let base = u32::try_from(base).map_err(|_| CoveError::ArithOverflow)?;
    base.checked_add(63).ok_or(CoveError::ArithOverflow)?;

    match policy {
        NativeKernelDispatch::Auto => {
            if let Some(dispatch) = append_dense_u32_rows_neon_dispatch(rows, base) {
                return Ok(dispatch);
            }
            if let Some(dispatch) = append_dense_u32_rows_avx2_dispatch(rows, base) {
                return Ok(dispatch);
            }
        }
        NativeKernelDispatch::Avx2 => {
            if let Some(dispatch) = append_dense_u32_rows_avx2_dispatch(rows, base) {
                return Ok(dispatch);
            }
        }
        NativeKernelDispatch::Neon => {
            if let Some(dispatch) = append_dense_u32_rows_neon_dispatch(rows, base) {
                return Ok(dispatch);
            }
        }
        NativeKernelDispatch::Scalar => {}
    }

    append_dense_u32_rows_scalar(rows, base);
    Ok(NativeKernelDispatch::Scalar)
}

fn append_dense_u32_rows_scalar(rows: &mut Vec<u32>, base: u32) {
    rows.reserve(64);
    for offset in 0..64 {
        rows.push(base + offset);
    }
}

fn append_dense_u32_rows_avx2_dispatch(
    rows: &mut Vec<u32>,
    base: u32,
) -> Option<NativeKernelDispatch> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        // SAFETY: Runtime feature detection proves AVX2 support. `base + 63`
        // was checked by the caller, and the callee reserves room before
        // writing exactly 64 initialized `u32` row ids.
        unsafe {
            append_dense_u32_rows_avx2(rows, base);
        }
        Some(NativeKernelDispatch::Avx2)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (rows, base);
        None
    }
}

fn append_dense_u32_rows_neon_dispatch(
    rows: &mut Vec<u32>,
    base: u32,
) -> Option<NativeKernelDispatch> {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: AArch64 provides NEON as a baseline feature. `base + 63`
        // was checked by the caller, and the callee reserves room before
        // writing exactly 64 initialized `u32` row ids.
        unsafe {
            append_dense_u32_rows_neon(rows, base);
        }
        Some(NativeKernelDispatch::Neon)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (rows, base);
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionVector {
    rows: Vec<u32>,
}

impl SelectionVector {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            rows: Vec::with_capacity(capacity),
        }
    }

    pub fn from_rows(rows: Vec<u32>) -> Self {
        Self { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rows(&self) -> &[u32] {
        &self.rows
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    pub fn push(&mut self, row: u32) {
        self.rows.push(row);
    }
}

pub fn filter_validity(
    row_count: usize,
    validity: ValidityRef<'_>,
    want_valid: bool,
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    let mut stats = KernelStats {
        rows_seen: row_count,
        rows_valid: validity.valid_count(),
        rows_matched: 0,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: match validity {
            ValidityRef::CoveNullBitmap { bytes, .. } => bytes.len(),
            ValidityRef::AllValid { .. } | ValidityRef::AllNull { .. } => 0,
        },
        dispatch: NativeKernelDispatch::Scalar,
    };

    let selected = match (validity, want_valid, base) {
        (ValidityRef::AllValid { .. }, true, Some(base)) => base.clone(),
        (ValidityRef::AllValid { .. }, true, None) => SelectionBitmap::all(row_count),
        (ValidityRef::AllValid { .. }, false, _) => SelectionBitmap::none(row_count),
        (ValidityRef::AllNull { .. }, true, _) => SelectionBitmap::none(row_count),
        (ValidityRef::AllNull { .. }, false, Some(base)) => base.clone(),
        (ValidityRef::AllNull { .. }, false, None) => SelectionBitmap::all(row_count),
        _ => {
            let mut selected = SelectionBitmap::none(row_count);
            for row in 0..row_count {
                if base.is_some_and(|bitmap| !bitmap.contains(row)) {
                    continue;
                }
                if validity.is_valid(row) == want_valid {
                    selected.set(row);
                }
            }
            selected
        }
    };
    stats.rows_matched = selected.count_ones();
    (selected, stats)
}

/// Return every valid row that is not present in `excluded`.
///
/// Cove null bitmap pages are fail-closed: bytes missing from a short bitmap are
/// treated as null rows, matching [`ValidityRef::is_valid`].
pub fn valid_rows_except(
    row_count: usize,
    validity: ValidityRef<'_>,
    excluded: &SelectionBitmap,
) -> SelectionBitmap {
    debug_assert_eq!(excluded.len(), row_count);
    let mut words = vec![0u64; row_count.div_ceil(64)];
    match validity {
        ValidityRef::AllValid { .. } => {
            for (word_index, word) in words.iter_mut().enumerate() {
                *word = !excluded.words().get(word_index).copied().unwrap_or(0);
            }
        }
        ValidityRef::AllNull { .. } => {}
        ValidityRef::CoveNullBitmap { bytes, .. } => {
            for (word_index, word) in words.iter_mut().enumerate() {
                let valid_word = cove_validity_word(bytes, word_index);
                let excluded_word = excluded.words().get(word_index).copied().unwrap_or(0);
                *word = valid_word & !excluded_word;
            }
        }
    }
    mask_last_word(&mut words, row_count);
    SelectionBitmap::from_words(words, row_count)
}

fn cove_validity_word(null_bitmap: &[u8], word_index: usize) -> u64 {
    let byte_start = word_index.saturating_mul(8);
    let mut null_bytes = [0xFFu8; 8];
    if let Some(available) = null_bitmap.get(byte_start..) {
        let copy_len = available.len().min(null_bytes.len());
        null_bytes[..copy_len].copy_from_slice(&available[..copy_len]);
    }
    !u64::from_le_bytes(null_bytes)
}

pub fn filter_u64_eq(
    values: &[u64],
    validity: ValidityRef<'_>,
    needle: u64,
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_u32_in_sorted(
    values: &[u32],
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((out, dispatch)) = try_filter_u32_values_in_simd(values, validity, needles, base) {
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: std::mem::size_of_val(values),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if needles.binary_search(&value).is_ok() {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_u32_not_in_sorted(
    values: &[u32],
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((matched, dispatch)) =
        try_filter_u32_values_in_simd(values, validity, needles, base)
    {
        let out = valid_rows_except(values.len(), validity, &matched);
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: std::mem::size_of_val(values),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if needles.binary_search(&value).is_err() {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_u64_le_eq(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: u64,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    filter_u64_le_eq_dispatch(
        bytes,
        row_count,
        validity,
        needle,
        base,
        NativeKernelDispatch::Auto,
    )
}

pub fn filter_u64_le_eq_dispatch(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: u64,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    if let Some((out, dispatch)) =
        try_filter_u64_le_eq_simd(bytes, row_count, validity, needle, base, policy)
    {
        let rows_matched = out.count_ones();
        return Ok((
            out,
            KernelStats {
                rows_seen: row_count,
                rows_valid: row_count,
                rows_matched,
                bitmap_words_touched: row_count.div_ceil(64),
                bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
                dispatch,
            },
        ));
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u64>();
        let value = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_u32_le_eq_dispatch(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: u32,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u32>(), validity)?;
    if let Some((out, dispatch)) =
        try_filter_u32_le_eq_simd(bytes, row_count, validity, needle, base, policy)
    {
        let rows_matched = out.count_ones();
        return Ok((
            out,
            KernelStats {
                rows_seen: row_count,
                rows_valid: row_count,
                rows_matched,
                bitmap_words_touched: row_count.div_ceil(64),
                bytes_touched_estimate: row_count * std::mem::size_of::<u32>(),
                dispatch,
            },
        ));
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u32>();
        let value = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeNumericPredicateOp {
    Eq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeNumericLiteral {
    Int64(i64),
    UInt64(u64),
    Float64(f64),
}

pub fn filter_numcode_le_typed(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    logical_type: CoveLogicalType,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    if matches!(logical_type, CoveLogicalType::Int64) {
        if let NativeNumericLiteral::Int64(needle) = literal {
            if let Some((out, dispatch)) = try_filter_i64_le_cmp_simd(
                bytes,
                row_count,
                validity,
                op,
                needle,
                base,
                NativeKernelDispatch::Auto,
            ) {
                let rows_matched = out.count_ones();
                return Ok((
                    out,
                    KernelStats {
                        rows_seen: row_count,
                        rows_valid: row_count,
                        rows_matched,
                        bitmap_words_touched: row_count.div_ceil(64),
                        bytes_touched_estimate: row_count * std::mem::size_of::<i64>(),
                        dispatch,
                    },
                ));
            }
        }
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u64>();
        let value = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if compare_native_numcode_value(logical_type, value, op, literal)? {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_numcode_le_in_typed(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    logical_type: CoveLogicalType,
    literals: &[NativeNumericLiteral],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    if let [literal] = literals {
        return filter_numcode_le_typed(
            bytes,
            row_count,
            validity,
            logical_type,
            NativeNumericPredicateOp::Eq,
            *literal,
            base,
        );
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if literals.is_empty() {
        return Ok((out, stats));
    }
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u64>();
        let value = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let mut matched = false;
        for literal in literals {
            if compare_native_numcode_value(
                logical_type,
                value,
                NativeNumericPredicateOp::Eq,
                *literal,
            )? {
                matched = true;
                break;
            }
        }
        if matched {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_numcode_le_not_in_typed(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    logical_type: CoveLogicalType,
    literals: &[NativeNumericLiteral],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    if base.is_none() {
        if let [literal] = literals {
            let (matched, mut stats) = filter_numcode_le_typed(
                bytes,
                row_count,
                validity,
                logical_type,
                NativeNumericPredicateOp::Eq,
                *literal,
                None,
            )?;
            let selected = valid_rows_except(row_count, validity, &matched);
            stats.rows_matched = selected.count_ones();
            return Ok((selected, stats));
        }
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u64>();
        let value = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let mut matched = false;
        for literal in literals {
            if compare_native_numcode_value(
                logical_type,
                value,
                NativeNumericPredicateOp::Eq,
                *literal,
            )? {
                matched = true;
                break;
            }
        }
        if !matched {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_i64_le_range(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<i64>(), validity)?;
    if let Some((out, dispatch)) =
        try_filter_i64_le_range_simd(bytes, row_count, validity, lower, upper, base)
    {
        let rows_matched = out.count_ones();
        return Ok((
            out,
            KernelStats {
                rows_seen: row_count,
                rows_valid: row_count,
                rows_matched,
                bitmap_words_touched: row_count.div_ceil(64),
                bytes_touched_estimate: row_count * std::mem::size_of::<i64>(),
                dispatch,
            },
        ));
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<i64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<i64>();
        let value = i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if !i64_value_in_range(value, lower, upper) {
            continue;
        }
        out.set(row);
        stats.rows_matched += 1;
    }
    Ok((out, stats))
}

pub fn filter_u32_le_in_sorted(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u32>(), validity)?;
    if let [needle] = needles {
        return filter_u32_le_eq_dispatch(
            bytes,
            row_count,
            validity,
            *needle,
            base,
            NativeKernelDispatch::Auto,
        );
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u32>();
        let value = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if needles.binary_search(&value).is_ok() {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_u32_le_not_in_sorted(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u32>(), validity)?;
    if base.is_none() {
        if let [needle] = needles {
            let (matched, mut stats) = filter_u32_le_eq_dispatch(
                bytes,
                row_count,
                validity,
                *needle,
                None,
                NativeKernelDispatch::Auto,
            )?;
            let selected = valid_rows_except(row_count, validity, &matched);
            stats.rows_matched = selected.count_ones();
            return Ok((selected, stats));
        }
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u32>();
        let value = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if needles.binary_search(&value).is_err() {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_i64_range(
    values: &[i64],
    validity: ValidityRef<'_>,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if lower
            .is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, false))
        {
            continue;
        }
        if upper
            .is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, true))
        {
            continue;
        }
        out.set(row);
        stats.rows_matched += 1;
    }
    (out, stats)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundInclusive {
    Inclusive,
    Exclusive,
}

pub fn local_membership_u8(local_to_global: &[u64], global_needles: &[u64]) -> Vec<bool> {
    let mut membership = vec![false; local_to_global.len()];
    for (local, global) in local_to_global.iter().copied().enumerate() {
        if global_needles.binary_search(&global).is_ok() {
            membership[local] = true;
        }
    }
    membership
}

pub fn filter_local_u8_membership(
    values: &[u8],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((out, dispatch)) =
        try_filter_local_u8_membership_simd(values, validity, local_membership, base)
    {
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: values.len(),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: values.len(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, local) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if local_membership
            .get(local as usize)
            .copied()
            .unwrap_or(false)
        {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_local_u16_membership(
    values: &[u16],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((out, dispatch)) =
        try_filter_local_u16_membership_simd(values, validity, local_membership, base)
    {
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: std::mem::size_of_val(values),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, local) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if local_membership
            .get(local as usize)
            .copied()
            .unwrap_or(false)
        {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_local_u32_membership(
    values: &[u32],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((out, dispatch)) =
        try_filter_local_u32_membership_simd(values, validity, local_membership, base)
    {
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: std::mem::size_of_val(values),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, local) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if local_membership
            .get(usize::try_from(local).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(false)
        {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_bool_eq(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: bool,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(values, row_count, 1, validity)?;
    let needle = u8::from(needle);
    if base.is_none()
        && matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        validate_bool_bytes(values, row_count, validity)?;
        if let Some((out, dispatch)) = try_filter_bool_eq_simd(values, row_count, needle) {
            let rows_matched = out.count_ones();
            return Ok((
                out,
                KernelStats {
                    rows_seen: row_count,
                    rows_valid: row_count,
                    rows_matched,
                    bitmap_words_touched: row_count.div_ceil(64),
                    bytes_touched_estimate: row_count,
                    dispatch,
                },
            ));
        }
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let value = values.get(row).copied().ok_or(CoveError::PageCorrupt)?;
        if !matches!(value, 0 | 1) {
            return Err(CoveError::PageCorrupt);
        }
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_fixed_bytes_eq(
    values: &[u8],
    row_count: usize,
    width: usize,
    validity: ValidityRef<'_>,
    needle: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if width == 0 || needle.len() != width {
        return Err(CoveError::BadSchema(
            "fixed-byte equality requires a non-zero width matching the literal".into(),
        ));
    }
    validate_fixed_le_width_for_validity(values, row_count, width, validity)?;
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count
            .checked_mul(width)
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row.checked_mul(width).ok_or(CoveError::ArithOverflow)?;
        let value = values
            .get(offset..offset + width)
            .ok_or(CoveError::PageCorrupt)?;
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_fixed_bytes_in(
    values: &[u8],
    row_count: usize,
    width: usize,
    validity: ValidityRef<'_>,
    needles: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if width == 0 || needles.is_empty() || !needles.len().is_multiple_of(width) {
        return Err(CoveError::BadSchema(
            "fixed-byte IN requires non-empty literals with a width multiple".into(),
        ));
    }
    validate_fixed_le_width_for_validity(values, row_count, width, validity)?;
    let literal_count = needles.len() / width;
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count
            .checked_mul(width)
            .and_then(|value| value.checked_add(needles.len()))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row.checked_mul(width).ok_or(CoveError::ArithOverflow)?;
        let value = values
            .get(offset..offset + width)
            .ok_or(CoveError::PageCorrupt)?;
        for literal_index in 0..literal_count {
            let literal_offset = literal_index
                .checked_mul(width)
                .ok_or(CoveError::ArithOverflow)?;
            let needle = needles
                .get(literal_offset..literal_offset + width)
                .ok_or(CoveError::PageCorrupt)?;
            if value == needle {
                out.set(row);
                stats.rows_matched += 1;
                break;
            }
        }
    }
    Ok((out, stats))
}

pub fn filter_varbytes_eq(
    row_offsets: &[u32],
    values: &[u8],
    validity: ValidityRef<'_>,
    needle: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    let row_count = row_offsets.len();
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_add(std::mem::size_of_val(row_offsets))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if varbytes_payload_at(row_offsets, values, row)? == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_varbytes_in(
    row_offsets: &[u32],
    values: &[u8],
    validity: ValidityRef<'_>,
    needles: &[&[u8]],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if needles.is_empty() {
        return Err(CoveError::BadSchema(
            "VarBytes IN requires at least one literal".into(),
        ));
    }
    let row_count = row_offsets.len();
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_add(std::mem::size_of_val(row_offsets))
            .and_then(|value| {
                needles
                    .iter()
                    .try_fold(value, |acc, needle| acc.checked_add(needle.len()))
            })
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let value = varbytes_payload_at(row_offsets, values, row)?;
        if needles.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_length_prefixed_varbytes_eq(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values.len(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok((out, stats));
    }

    let all_selected = base.is_none_or(|bitmap| bitmap.count_ones() == row_count);
    let mut offset = 0usize;
    for row in 0..row_count {
        let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        let Some(len_bytes) = values.get(offset..len_end) else {
            return Err(CoveError::BufferTooShort);
        };
        let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
        let value_start = len_end;
        let value_end = value_start
            .checked_add(len)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(value) = values.get(value_start..value_end) else {
            return Err(CoveError::BufferTooShort);
        };

        if (all_selected || base.is_some_and(|bitmap| bitmap.contains(row)))
            && validity.is_valid(row)
        {
            stats.rows_valid += 1;
            if value == needle {
                out.set(row);
                stats.rows_matched += 1;
            }
        }
        offset = value_end;
    }
    if offset != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok((out, stats))
}

pub fn filter_length_prefixed_varbytes_in(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needles: &[&[u8]],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if needles.is_empty() {
        return Err(CoveError::BadSchema(
            "VarBytes IN requires at least one literal".into(),
        ));
    }
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: needles
            .iter()
            .try_fold(values.len(), |acc, needle| acc.checked_add(needle.len()))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok((out, stats));
    }

    let all_selected = base.is_none_or(|bitmap| bitmap.count_ones() == row_count);
    let mut offset = 0usize;
    for row in 0..row_count {
        let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        let Some(len_bytes) = values.get(offset..len_end) else {
            return Err(CoveError::BufferTooShort);
        };
        let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
        let value_start = len_end;
        let value_end = value_start
            .checked_add(len)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(value) = values.get(value_start..value_end) else {
            return Err(CoveError::BufferTooShort);
        };

        if (all_selected || base.is_some_and(|bitmap| bitmap.contains(row)))
            && validity.is_valid(row)
        {
            stats.rows_valid += 1;
            if needles.contains(&value) {
                out.set(row);
                stats.rows_matched += 1;
            }
        }
        offset = value_end;
    }
    if offset != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok((out, stats))
}

pub fn filter_length_prefixed_varbytes_prefix(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    prefix: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values.len(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok((out, stats));
    }

    let all_selected = base.is_none_or(|bitmap| bitmap.count_ones() == row_count);
    let mut offset = 0usize;
    for row in 0..row_count {
        let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        let Some(len_bytes) = values.get(offset..len_end) else {
            return Err(CoveError::BufferTooShort);
        };
        let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
        let value_start = len_end;
        let value_end = value_start
            .checked_add(len)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(value) = values.get(value_start..value_end) else {
            return Err(CoveError::BufferTooShort);
        };

        if (all_selected || base.is_some_and(|bitmap| bitmap.contains(row)))
            && validity.is_valid(row)
        {
            stats.rows_valid += 1;
            if value.starts_with(prefix) {
                out.set(row);
                stats.rows_matched += 1;
            }
        }
        offset = value_end;
    }
    if offset != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok((out, stats))
}

pub fn filter_varbytes_prefix(
    row_offsets: &[u32],
    values: &[u8],
    validity: ValidityRef<'_>,
    prefix: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    let row_count = row_offsets.len();
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_add(std::mem::size_of_val(row_offsets))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if varbytes_payload_at(row_offsets, values, row)?.starts_with(prefix) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeNullOrder {
    First,
    Last,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeDenseGroupCounts {
    pub counts: Vec<u64>,
    pub null_count: u64,
    pub rows_grouped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeHashGroupCounts {
    pub counts: HashMap<u32, u64>,
    pub null_count: u64,
    pub rows_grouped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeI64HashGroupCounts {
    pub counts: HashMap<i64, u64>,
    pub null_count: u64,
    pub rows_grouped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeU32I64HashGroupAggregates {
    pub aggregates: HashMap<u32, NativeI64Aggregates>,
    pub row_counts: HashMap<u32, u64>,
    pub null_aggregate: NativeI64Aggregates,
    pub null_row_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeI64I64HashGroupAggregates {
    pub aggregates: HashMap<i64, NativeI64Aggregates>,
    pub row_counts: HashMap<i64, u64>,
    pub null_aggregate: NativeI64Aggregates,
    pub null_row_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeInnerJoinPairs {
    pub left_rows: Vec<u32>,
    pub right_rows: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeI64Aggregates {
    pub count: u64,
    pub null_count: u64,
    pub sum: i128,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeDenseI64GroupAggregates {
    pub aggregates: Vec<NativeI64Aggregates>,
    pub row_counts: Vec<u64>,
    pub null_aggregate: NativeI64Aggregates,
    pub null_row_count: u64,
}

pub fn group_count_u32_dense(
    values: &[u32],
    validity: ValidityRef<'_>,
    group_count: usize,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; group_count],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        let group = usize::try_from(value).map_err(|_| CoveError::ArithOverflow)?;
        let Some(count) = groups.counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {value} is outside declared group count {group_count}"
            )));
        };
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped += 1;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_u8_dense(
    values: &[u8],
    validity: ValidityRef<'_>,
    group_count: usize,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; group_count],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        let group = usize::from(value);
        let Some(count) = groups.counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {value} is outside declared group count {group_count}"
            )));
        };
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped += 1;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_u16_dense(
    values: &[u16],
    validity: ValidityRef<'_>,
    group_count: usize,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; group_count],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        let group = usize::from(value);
        let Some(count) = groups.counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {value} is outside declared group count {group_count}"
            )));
        };
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped += 1;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_bool(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(values, row_count, 1, validity)?;
    validate_bool_bytes(values, row_count, validity)?;
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; 2],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        stats.rows_valid += 1;
        let value = values.get(row).copied().ok_or(CoveError::PageCorrupt)?;
        let count = groups
            .counts
            .get_mut(usize::from(value))
            .ok_or(CoveError::PageCorrupt)?;
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_u32_hash(
    values: &[u32],
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeHashGroupCounts, KernelStats), CoveError> {
    let capacity = base.map_or(values.len(), SelectionBitmap::count_ones);
    let mut groups = NativeHashGroupCounts {
        counts: HashMap::with_capacity(capacity.min(values.len())),
        ..NativeHashGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        let count = groups.counts.entry(value).or_default();
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok((groups, stats))
}

pub fn group_count_u32_le_bytes(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeHashGroupCounts, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u32>(), validity)?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeHashGroupCounts {
        counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeHashGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        let offset = row * std::mem::size_of::<u32>();
        let value = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        let count = groups.counts.entry(value).or_default();
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok((groups, stats))
}

pub fn group_count_i64_le_bytes(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64HashGroupCounts, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeI64HashGroupCounts {
        counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeI64HashGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        let offset = row * std::mem::size_of::<u64>();
        let value = i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        let count = groups.counts.entry(value).or_default();
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok((groups, stats))
}

pub fn distinct_u32(
    values: &[u32],
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(Vec<u32>, KernelStats), CoveError> {
    let mut seen = HashSet::with_capacity(base.map_or(values.len(), SelectionBitmap::count_ones));
    let mut distinct = Vec::new();
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if seen.insert(value) {
            distinct.push(value);
            stats.rows_matched += 1;
        }
    }
    Ok((distinct, stats))
}

pub fn inner_join_u32_eq(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code inner join requires a proven shared code equality domain".into(),
        ));
    }
    inner_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
    )
}

pub fn inner_join_u64_eq(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code inner join requires a proven shared code equality domain".into(),
        ));
    }
    inner_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
    )
}

pub fn inner_join_i64_eq(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 inner join requires a proven shared equality domain".into(),
        ));
    }
    inner_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
    )
}

#[derive(Debug, Clone, Copy)]
struct NativeJoinBucket {
    start: usize,
    len: usize,
    filled: usize,
}

fn inner_join_eq<K>(
    left_values: &[K],
    left_validity: ValidityRef<'_>,
    right_values: &[K],
    right_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError>
where
    K: Copy + Eq + Hash,
{
    let mut counts = HashMap::<K, usize>::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            *counts.entry(value).or_default() += 1;
        }
    }

    let mut buckets = HashMap::<K, NativeJoinBucket>::with_capacity(counts.len());
    let mut right_row_ids = Vec::<u32>::new();
    for (value, len) in counts {
        let start = right_row_ids.len();
        right_row_ids.resize(start + len, 0);
        buckets.insert(
            value,
            NativeJoinBucket {
                start,
                len,
                filled: 0,
            },
        );
    }

    for (row, value) in right_values.iter().copied().enumerate() {
        if !right_validity.is_valid(row) {
            continue;
        }
        let Some(bucket) = buckets.get_mut(&value) else {
            return Err(CoveError::PageCorrupt);
        };
        let slot = bucket
            .start
            .checked_add(bucket.filled)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(target) = right_row_ids.get_mut(slot) else {
            return Err(CoveError::PageCorrupt);
        };
        *target = u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        bucket.filled += 1;
    }

    let mut pairs = NativeInnerJoinPairs::default();
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len()) * std::mem::size_of::<K>()
            + right_row_ids.len() * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (left_row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(left_row)) {
            continue;
        }
        if !left_validity.is_valid(left_row) {
            continue;
        }
        stats.rows_valid += 1;
        let Some(bucket) = buckets.get(&value) else {
            continue;
        };
        debug_assert_eq!(bucket.filled, bucket.len);
        let left_row = u32::try_from(left_row).map_err(|_| CoveError::ArithOverflow)?;
        let end = bucket
            .start
            .checked_add(bucket.len)
            .ok_or(CoveError::ArithOverflow)?;
        let right_matches = right_row_ids
            .get(bucket.start..end)
            .ok_or(CoveError::PageCorrupt)?;
        pairs.left_rows.reserve(right_matches.len());
        pairs.right_rows.reserve(right_matches.len());
        for right_row in right_matches {
            pairs.left_rows.push(left_row);
            pairs.right_rows.push(*right_row);
        }
        stats.rows_matched = stats
            .rows_matched
            .checked_add(right_matches.len())
            .ok_or(CoveError::ArithOverflow)?;
    }
    stats.bytes_touched_estimate += pairs.left_rows.len() * 2 * std::mem::size_of::<u32>();
    Ok((pairs, stats))
}

pub fn aggregate_i64(
    values: &[i64],
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> (NativeI64Aggregates, KernelStats) {
    let mut aggregates = NativeI64Aggregates::default();
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            aggregates.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        aggregates.count += 1;
        aggregates.sum += i128::from(value);
        aggregates.min = Some(aggregates.min.map_or(value, |min| min.min(value)));
        aggregates.max = Some(aggregates.max.map_or(value, |max| max.max(value)));
    }
    (aggregates, stats)
}

pub fn aggregate_i64_le_bytes(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64Aggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    let mut aggregates = NativeI64Aggregates::default();
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            aggregates.null_count += 1;
            continue;
        }
        let offset = row * std::mem::size_of::<u64>();
        let value = i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        aggregates.count = aggregates
            .count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        aggregates.sum = aggregates
            .sum
            .checked_add(i128::from(value))
            .ok_or(CoveError::ArithOverflow)?;
        aggregates.min = Some(aggregates.min.map_or(value, |min| min.min(value)));
        aggregates.max = Some(aggregates.max.map_or(value, |max| max.max(value)));
    }
    Ok((aggregates, stats))
}

pub fn aggregate_i64_by_bool(
    key_values: &[u8],
    value_values: &[i64],
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    let row_count = value_values.len();
    validate_fixed_le_width_for_validity(key_values, row_count, 1, key_validity)?;
    validate_bool_bytes(key_values, row_count, key_validity)?;
    if value_validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); 2],
        row_counts: vec![0; 2],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (1 + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in value_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_bool_group_for_row(key_values, key_validity, &mut groups, row)?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_bool(
    key_values: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(key_values, row_count, 1, key_validity)?;
    validate_bool_bytes(key_values, row_count, key_validity)?;
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); 2],
        row_counts: vec![0; 2],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (1 + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_bool_group_for_row(key_values, key_validity, &mut groups, row)?;
        if value_validity.is_valid(row) {
            let offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = i64::from_le_bytes(
                value_bytes[offset..offset + std::mem::size_of::<u64>()]
                    .try_into()
                    .unwrap(),
            );
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_by_u8_dense(
    key_values: &[u8],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_by_dense_key(
        key_values,
        value_values,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_by_u16_dense(
    key_values: &[u16],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_by_dense_key(
        key_values,
        value_values,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_by_u32_dense(
    key_values: &[u32],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_by_dense_key(
        key_values,
        value_values,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

fn aggregate_i64_by_dense_key<K>(
    key_values: &[K],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError>
where
    K: Copy + TryInto<usize>,
{
    let row_count = value_values.len();
    if key_values.len() != row_count
        || key_validity.row_count() != row_count
        || value_validity.row_count() != row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); group_count],
        row_counts: vec![0; group_count],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (std::mem::size_of::<K>() + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, (key, value)) in key_values
        .iter()
        .copied()
        .zip(value_values.iter().copied())
        .enumerate()
    {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_key_group_for_row(
            &mut groups,
            key_validity,
            row,
            key.try_into().map_err(|_| CoveError::ArithOverflow)?,
        )?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_u8_dense(
    key_values: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_le_bytes_by_dense_key(
        key_values,
        value_bytes,
        row_count,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_le_bytes_by_u16_dense(
    key_values: &[u16],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_le_bytes_by_dense_key(
        key_values,
        value_bytes,
        row_count,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_le_bytes_by_u32_dense(
    key_values: &[u32],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_le_bytes_by_dense_key(
        key_values,
        value_bytes,
        row_count,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

fn aggregate_i64_le_bytes_by_dense_key<K>(
    key_values: &[K],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError>
where
    K: Copy + TryInto<usize>,
{
    if key_values.len() != row_count || key_validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); group_count],
        row_counts: vec![0; group_count],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (std::mem::size_of::<K>() + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, key) in key_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_key_group_for_row(
            &mut groups,
            key_validity,
            row,
            key.try_into().map_err(|_| CoveError::ArithOverflow)?,
        )?;
        if value_validity.is_valid(row) {
            let value_offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = i64::from_le_bytes(
                value_bytes[value_offset..value_offset + std::mem::size_of::<u64>()]
                    .try_into()
                    .unwrap(),
            );
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

fn dense_bool_group_for_row<'a>(
    key_values: &[u8],
    key_validity: ValidityRef<'_>,
    groups: &'a mut NativeDenseI64GroupAggregates,
    row: usize,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let key = key_values.get(row).copied().ok_or(CoveError::PageCorrupt)?;
        let group = usize::from(key);
        let row_count = groups
            .row_counts
            .get_mut(group)
            .ok_or(CoveError::PageCorrupt)?;
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups
            .aggregates
            .get_mut(group)
            .ok_or(CoveError::PageCorrupt)
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

fn dense_key_group_for_row<'a>(
    groups: &'a mut NativeDenseI64GroupAggregates,
    key_validity: ValidityRef<'_>,
    row: usize,
    group: usize,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let declared_group_count = groups.row_counts.len();
        let Some(row_count) = groups.row_counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {group} is outside declared group count {declared_group_count}"
            )));
        };
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups
            .aggregates
            .get_mut(group)
            .ok_or(CoveError::PageCorrupt)
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

fn accumulate_i64(aggregate: &mut NativeI64Aggregates, value: i64) -> Result<(), CoveError> {
    aggregate.count = aggregate
        .count
        .checked_add(1)
        .ok_or(CoveError::ArithOverflow)?;
    aggregate.sum = aggregate
        .sum
        .checked_add(i128::from(value))
        .ok_or(CoveError::ArithOverflow)?;
    aggregate.min = Some(aggregate.min.map_or(value, |min| min.min(value)));
    aggregate.max = Some(aggregate.max.map_or(value, |max| max.max(value)));
    Ok(())
}

pub fn aggregate_i64_by_i64(
    key_values: &[i64],
    value_values: &[i64],
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64I64HashGroupAggregates, KernelStats), CoveError> {
    let row_count = value_values.len();
    if key_values.len() != row_count
        || key_validity.row_count() != row_count
        || value_validity.row_count() != row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeI64I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeI64I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<i64>() + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, (key, value)) in key_values
        .iter()
        .copied()
        .zip(value_values.iter().copied())
        .enumerate()
    {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = i64_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_i64_le_bytes(
    key_bytes: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64I64HashGroupAggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(
        key_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        key_validity,
    )?;
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeI64I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeI64I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<u64>() + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let key_offset = row
            .checked_mul(std::mem::size_of::<u64>())
            .ok_or(CoveError::ArithOverflow)?;
        let key = i64::from_le_bytes(
            key_bytes[key_offset..key_offset + std::mem::size_of::<u64>()]
                .try_into()
                .unwrap(),
        );
        let aggregate = i64_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            let value_offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = i64::from_le_bytes(
                value_bytes[value_offset..value_offset + std::mem::size_of::<u64>()]
                    .try_into()
                    .unwrap(),
            );
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

fn i64_group_for_row<'a>(
    groups: &'a mut NativeI64I64HashGroupAggregates,
    key_validity: ValidityRef<'_>,
    row: usize,
    key: i64,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let row_count = groups.row_counts.entry(key).or_default();
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        Ok(groups.aggregates.entry(key).or_default())
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

pub fn aggregate_i64_by_u32(
    key_values: &[u32],
    value_values: &[i64],
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeU32I64HashGroupAggregates, KernelStats), CoveError> {
    let row_count = value_values.len();
    if key_values.len() != row_count
        || key_validity.row_count() != row_count
        || value_validity.row_count() != row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeU32I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeU32I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<u32>() + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, (key, value)) in key_values
        .iter()
        .copied()
        .zip(value_values.iter().copied())
        .enumerate()
    {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = u32_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_u32_le_bytes(
    key_bytes: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeU32I64HashGroupAggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(
        key_bytes,
        row_count,
        std::mem::size_of::<u32>(),
        key_validity,
    )?;
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeU32I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeU32I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<u32>() + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let key_offset = row
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or(CoveError::ArithOverflow)?;
        let key = u32::from_le_bytes(
            key_bytes[key_offset..key_offset + std::mem::size_of::<u32>()]
                .try_into()
                .unwrap(),
        );
        let aggregate = u32_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            let value_offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = i64::from_le_bytes(
                value_bytes[value_offset..value_offset + std::mem::size_of::<u64>()]
                    .try_into()
                    .unwrap(),
            );
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

fn u32_group_for_row<'a>(
    groups: &'a mut NativeU32I64HashGroupAggregates,
    key_validity: ValidityRef<'_>,
    row: usize,
    key: u32,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let row_count = groups.row_counts.entry(key).or_default();
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        Ok(groups.aggregates.entry(key).or_default())
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

pub fn sort_rows_i64(
    values: &[i64],
    validity: ValidityRef<'_>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
    base: Option<&SelectionBitmap>,
) -> Result<SelectionVector, CoveError> {
    sort_rows_i64_with_stats(values, validity, direction, null_order, base)
        .map(|(selection, _stats)| selection)
}

pub fn sort_rows_i64_with_stats(
    values: &[i64],
    validity: ValidityRef<'_>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionVector, KernelStats), CoveError> {
    if validity.row_count() != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut rows = Vec::with_capacity(base.map_or(values.len(), SelectionBitmap::count_ones));
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_mul(std::mem::size_of::<i64>())
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..values.len() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if validity.is_valid(row) {
            stats.rows_valid += 1;
        }
        stats.rows_matched += 1;
        rows.push(u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?);
    }
    stats.bytes_touched_estimate = stats
        .bytes_touched_estimate
        .checked_add(
            rows.len()
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or(CoveError::ArithOverflow)?,
        )
        .ok_or(CoveError::ArithOverflow)?;
    rows.sort_by(|left, right| {
        compare_i64_rows(
            values,
            validity,
            *left as usize,
            *right as usize,
            direction,
            null_order,
        )
    });
    Ok((SelectionVector::from_rows(rows), stats))
}

pub fn top_n_rows_i64_with_stats(
    values: &[i64],
    validity: ValidityRef<'_>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
    base: Option<&SelectionBitmap>,
    limit: usize,
) -> Result<(SelectionVector, KernelStats), CoveError> {
    if validity.row_count() != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_mul(std::mem::size_of::<i64>())
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if limit == 0 {
        return Ok((SelectionVector::new(), stats));
    }

    let mut heap = BinaryHeap::<I64TopRow<'_>>::with_capacity(limit);
    for row in 0..values.len() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if validity.is_valid(row) {
            stats.rows_valid += 1;
        }
        stats.rows_matched += 1;

        let row = u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        let item = I64TopRow {
            row,
            values,
            validity,
            direction,
            null_order,
        };
        if heap.len() < limit {
            heap.push(item);
            continue;
        }
        let candidate_beats_worst = heap
            .peek()
            .is_some_and(|worst| item.desired_cmp(worst) == Ordering::Less);
        if candidate_beats_worst {
            heap.pop();
            heap.push(item);
        }
    }

    let mut rows = heap.into_iter().map(|row| row.row).collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        compare_i64_rows(
            values,
            validity,
            *left as usize,
            *right as usize,
            direction,
            null_order,
        )
    });
    stats.bytes_touched_estimate = stats
        .bytes_touched_estimate
        .checked_add(
            rows.len()
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or(CoveError::ArithOverflow)?,
        )
        .ok_or(CoveError::ArithOverflow)?;
    Ok((SelectionVector::from_rows(rows), stats))
}

#[derive(Clone, Copy)]
struct I64TopRow<'a> {
    row: u32,
    values: &'a [i64],
    validity: ValidityRef<'a>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
}

impl I64TopRow<'_> {
    fn desired_cmp(&self, other: &Self) -> Ordering {
        compare_i64_rows(
            self.values,
            self.validity,
            self.row as usize,
            other.row as usize,
            self.direction,
            self.null_order,
        )
    }
}

impl PartialEq for I64TopRow<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
    }
}

impl Eq for I64TopRow<'_> {}

impl PartialOrd for I64TopRow<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for I64TopRow<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.desired_cmp(other)
    }
}

pub fn semi_join_u32_eq(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code semi-join requires a proven shared code equality domain".into(),
        ));
    }
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len())
            * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn anti_join_u32_eq(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        false,
    )
}

pub fn anti_join_u32_eq_left_nulls_unmatched(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        true,
    )
}

pub fn semi_join_u64_eq(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code semi-join requires a proven shared code equality domain".into(),
        ));
    }
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len())
            * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn semi_join_i64_eq(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 semi-join requires a proven shared equality domain".into(),
        ));
    }
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len())
            * std::mem::size_of::<i64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn anti_join_u64_eq(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        false,
    )
}

pub fn anti_join_i64_eq(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 anti-join requires a proven shared equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        false,
    )
}

pub fn anti_join_i64_eq_left_nulls_unmatched(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 anti-join requires a proven shared equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        true,
    )
}

pub fn anti_join_u64_eq_left_nulls_unmatched(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        true,
    )
}

fn anti_join_eq<K>(
    left_values: &[K],
    left_validity: ValidityRef<'_>,
    right_values: &[K],
    right_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
    left_nulls_unmatched: bool,
) -> Result<(SelectionBitmap, KernelStats), CoveError>
where
    K: Copy + Eq + Hash,
{
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len()) * std::mem::size_of::<K>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            if left_nulls_unmatched {
                out.set(row);
                stats.rows_matched += 1;
            }
            continue;
        }
        stats.rows_valid += 1;
        if !build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

fn compare_bound(ordering: Ordering, inclusive: BoundInclusive, upper: bool) -> bool {
    !matches!(
        (upper, inclusive, ordering),
        (false, BoundInclusive::Inclusive, Ordering::Less)
            | (
                false,
                BoundInclusive::Exclusive,
                Ordering::Less | Ordering::Equal
            )
            | (true, BoundInclusive::Inclusive, Ordering::Greater)
            | (
                true,
                BoundInclusive::Exclusive,
                Ordering::Greater | Ordering::Equal
            )
    )
}

#[derive(Debug, Clone, Copy)]
enum NativeTypedNumericValue {
    Signed(i128),
    Unsigned(u128),
    Float(f64),
}

const I64_MAX_PLUS_ONE_AS_F64: f64 = 9_223_372_036_854_775_808.0;
const U64_MAX_PLUS_ONE_AS_F64: f64 = 18_446_744_073_709_551_616.0;

fn compare_native_numcode_value(
    logical_type: CoveLogicalType,
    value: u64,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<bool, CoveError> {
    let Some(value) = typed_native_numcode_value(logical_type, value) else {
        return Err(CoveError::UnsupportedEncoding(format!(
            "numeric predicate for {logical_type:?} NumCode"
        )));
    };
    Ok(compare_native_typed_numeric_value(value, op, literal))
}

pub fn native_numcode_matches(
    logical_type: CoveLogicalType,
    value: u64,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<bool, CoveError> {
    compare_native_numcode_value(logical_type, value, op, literal)
}

fn typed_native_numcode_value(
    logical_type: CoveLogicalType,
    value: u64,
) -> Option<NativeTypedNumericValue> {
    use crate::types;

    let value = match logical_type {
        CoveLogicalType::Bool => match value {
            0 => NativeTypedNumericValue::Unsigned(0),
            1 => NativeTypedNumericValue::Unsigned(1),
            _ => return None,
        },
        CoveLogicalType::Int8 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i8(value) as i128)
        }
        CoveLogicalType::Int16 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i16(value) as i128)
        }
        CoveLogicalType::Int32 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i32(value) as i128)
        }
        CoveLogicalType::Int64 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i64(value) as i128)
        }
        CoveLogicalType::UInt8 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u8(value) as u128)
        }
        CoveLogicalType::UInt16 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u16(value) as u128)
        }
        CoveLogicalType::UInt32 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u32(value) as u128)
        }
        CoveLogicalType::UInt64 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u64(value) as u128)
        }
        CoveLogicalType::Float32 => {
            let value = types::numcode_as_f32(value);
            if value.is_nan() {
                return Some(NativeTypedNumericValue::Float(f64::NAN));
            }
            NativeTypedNumericValue::Float(f64::from(value))
        }
        CoveLogicalType::Float64 => {
            let value = types::numcode_as_f64(value);
            if value.is_nan() {
                return Some(NativeTypedNumericValue::Float(f64::NAN));
            }
            NativeTypedNumericValue::Float(value)
        }
        CoveLogicalType::Decimal64 => {
            NativeTypedNumericValue::Signed(types::numcode_as_decimal64(value) as i128)
        }
        CoveLogicalType::DateDays => {
            NativeTypedNumericValue::Signed(types::numcode_as_date_days(value) as i128)
        }
        CoveLogicalType::TimestampMicros => {
            NativeTypedNumericValue::Signed(types::numcode_as_timestamp_micros(value) as i128)
        }
        CoveLogicalType::TimestampNanos => {
            NativeTypedNumericValue::Signed(types::numcode_as_timestamp_nanos(value) as i128)
        }
        _ => return None,
    };
    Some(value)
}

fn compare_native_typed_numeric_value(
    value: NativeTypedNumericValue,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> bool {
    match value {
        NativeTypedNumericValue::Signed(value) => {
            compare_native_signed_numeric_value(value, op, literal)
        }
        NativeTypedNumericValue::Unsigned(value) => {
            compare_native_unsigned_numeric_value(value, op, literal)
        }
        NativeTypedNumericValue::Float(value) => {
            let Some(literal) = native_literal_as_f64(literal) else {
                return false;
            };
            if value.is_nan() || literal.is_nan() {
                false
            } else {
                compare_native_ordered(value, op, literal)
            }
        }
    }
}

fn compare_native_signed_numeric_value(
    value: i128,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> bool {
    match literal {
        NativeNumericLiteral::Int64(literal) => compare_native_ordered(value, op, literal as i128),
        NativeNumericLiteral::UInt64(literal) => {
            compare_native_signed_unsigned(value, op, literal as u128)
        }
        NativeNumericLiteral::Float64(literal) => {
            compare_native_signed_float_literal(value, op, literal)
        }
    }
}

fn compare_native_unsigned_numeric_value(
    value: u128,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> bool {
    match literal {
        NativeNumericLiteral::Int64(literal) if literal < 0 => match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Lt
            | NativeNumericPredicateOp::LtEq => false,
            NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => true,
        },
        NativeNumericLiteral::Int64(literal) => compare_native_ordered(value, op, literal as u128),
        NativeNumericLiteral::UInt64(literal) => compare_native_ordered(value, op, literal as u128),
        NativeNumericLiteral::Float64(literal) => {
            compare_native_unsigned_float_literal(value, op, literal)
        }
    }
}

fn compare_native_signed_float_literal(
    value: i128,
    op: NativeNumericPredicateOp,
    literal: f64,
) -> bool {
    if !literal.is_finite() {
        return false;
    }
    if let Some(rhs) = f64_to_i64_exact(literal) {
        return compare_native_ordered(value, op, rhs as i128);
    }
    if literal < i64::MIN as f64 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Lt
            | NativeNumericPredicateOp::LtEq => false,
            NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => true,
        };
    }
    if literal >= I64_MAX_PLUS_ONE_AS_F64 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Gt
            | NativeNumericPredicateOp::GtEq => false,
            NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => true,
        };
    }
    let floor = literal.floor() as i128;
    let ceil = literal.ceil() as i128;
    compare_native_integer_to_fractional_bounds(value, op, floor, ceil)
}

fn compare_native_unsigned_float_literal(
    value: u128,
    op: NativeNumericPredicateOp,
    literal: f64,
) -> bool {
    if !literal.is_finite() {
        return false;
    }
    if let Some(rhs) = f64_to_u64_exact(literal) {
        return compare_native_ordered(value, op, rhs as u128);
    }
    if literal < 0.0 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Lt
            | NativeNumericPredicateOp::LtEq => false,
            NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => true,
        };
    }
    if literal >= U64_MAX_PLUS_ONE_AS_F64 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Gt
            | NativeNumericPredicateOp::GtEq => false,
            NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => true,
        };
    }
    let floor = literal.floor() as u128;
    let ceil = literal.ceil() as u128;
    compare_native_integer_to_fractional_bounds(value, op, floor, ceil)
}

fn compare_native_integer_to_fractional_bounds<T>(
    value: T,
    op: NativeNumericPredicateOp,
    floor: T,
    ceil: T,
) -> bool
where
    T: PartialOrd,
{
    match op {
        NativeNumericPredicateOp::Eq => false,
        NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => value <= floor,
        NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => value >= ceil,
    }
}

fn compare_native_signed_unsigned(
    value: i128,
    op: NativeNumericPredicateOp,
    literal: u128,
) -> bool {
    if value < 0 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Gt
            | NativeNumericPredicateOp::GtEq => false,
            NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => true,
        };
    }
    compare_native_ordered(value as u128, op, literal)
}

fn compare_native_ordered<T: PartialOrd + PartialEq>(
    left: T,
    op: NativeNumericPredicateOp,
    right: T,
) -> bool {
    match op {
        NativeNumericPredicateOp::Eq => left == right,
        NativeNumericPredicateOp::Lt => left < right,
        NativeNumericPredicateOp::LtEq => left <= right,
        NativeNumericPredicateOp::Gt => left > right,
        NativeNumericPredicateOp::GtEq => left >= right,
    }
}

fn native_literal_as_f64(literal: NativeNumericLiteral) -> Option<f64> {
    match literal {
        NativeNumericLiteral::Int64(value) => Some(value as f64),
        NativeNumericLiteral::UInt64(value) => Some(value as f64),
        NativeNumericLiteral::Float64(value) if !value.is_nan() => Some(value),
        NativeNumericLiteral::Float64(_) => None,
    }
}

#[inline]
fn f64_to_i64_exact(value: f64) -> Option<i64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value >= I64_MAX_PLUS_ONE_AS_F64
    {
        return None;
    }
    let candidate = value as i64;
    ((candidate as f64) == value).then_some(candidate)
}

#[inline]
fn f64_to_u64_exact(value: f64) -> Option<u64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || !(0.0..U64_MAX_PLUS_ONE_AS_F64).contains(&value)
    {
        return None;
    }
    let candidate = value as u64;
    ((candidate as f64) == value).then_some(candidate)
}

fn compare_i64_rows(
    values: &[i64],
    validity: ValidityRef<'_>,
    left: usize,
    right: usize,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
) -> Ordering {
    let left_valid = validity.is_valid(left);
    let right_valid = validity.is_valid(right);
    match (left_valid, right_valid) {
        (false, false) => left.cmp(&right),
        (false, true) => match null_order {
            NativeNullOrder::First => Ordering::Less,
            NativeNullOrder::Last => Ordering::Greater,
        },
        (true, false) => match null_order {
            NativeNullOrder::First => Ordering::Greater,
            NativeNullOrder::Last => Ordering::Less,
        },
        (true, true) => {
            let ordering = values[left]
                .cmp(&values[right])
                .then_with(|| left.cmp(&right));
            match direction {
                NativeSortDirection::Ascending => ordering,
                NativeSortDirection::Descending => ordering.reverse(),
            }
        }
    }
}

pub trait NativeColumnPagePayload {
    fn header_row_count(&self) -> u32;
    fn root_node_ref(&self) -> Result<&CoveEncodingNodeV1, CoveError>;
    fn buffer_bytes_ref(&self, kind: PageBufferKind) -> Result<Option<&[u8]>, CoveError>;
}

impl NativeColumnPagePayload for ColumnPagePayloadV1 {
    fn header_row_count(&self) -> u32 {
        self.header.row_count
    }

    fn root_node_ref(&self) -> Result<&CoveEncodingNodeV1, CoveError> {
        self.root_node()
    }

    fn buffer_bytes_ref(&self, kind: PageBufferKind) -> Result<Option<&[u8]>, CoveError> {
        self.buffer_bytes(kind)
    }
}

impl NativeColumnPagePayload for RetainedColumnPagePayloadV1 {
    fn header_row_count(&self) -> u32 {
        self.header.row_count
    }

    fn root_node_ref(&self) -> Result<&CoveEncodingNodeV1, CoveError> {
        self.root_node()
    }

    fn buffer_bytes_ref(&self, kind: PageBufferKind) -> Result<Option<&[u8]>, CoveError> {
        self.buffer_bytes(kind)
    }
}

pub fn native_lane_from_column_page_payload<'a, P: NativeColumnPagePayload + ?Sized>(
    column: &TableColumnDirectoryEntryV1,
    page: &crate::page::ColumnPageIndexEntryV1,
    payload: &'a P,
    mut domain: NativeCodeDomain,
) -> Result<LaneRef<'a>, CoveError> {
    if page.column_id != column.column_id {
        return Err(CoveError::PageCorrupt);
    }
    domain.column_id.get_or_insert(column.column_id);
    if column.domain_ref != 0 {
        domain
            .semantic_domain_id
            .get_or_insert_with(|| format!("table-domain:{}", column.domain_ref));
    }
    native_lane_from_object_page_payload(
        NativeLaneId(column.column_id),
        column.logical_type,
        column.physical_kind,
        page,
        payload,
        domain,
    )
}

fn native_lane_from_object_page_payload<'a, P: NativeColumnPagePayload + ?Sized>(
    lane_id: NativeLaneId,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    page: &crate::page::ColumnPageIndexEntryV1,
    payload: &'a P,
    domain: NativeCodeDomain,
) -> Result<LaneRef<'a>, CoveError> {
    if payload.header_row_count() != page.row_count {
        return Err(CoveError::PageCorrupt);
    }
    let root = payload.root_node_ref()?;
    if root.logical_type != logical_type
        || root.physical_kind != physical_kind
        || root.logical_len != page.row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let row_count = usize::try_from(page.row_count).map_err(|_| CoveError::OffsetRange)?;
    let validity = native_validity_from_page(payload, page)?;
    let values = payload
        .buffer_bytes_ref(PageBufferKind::Values)?
        .unwrap_or(&[]);

    match (root.encoding_kind, root.physical_kind) {
        (CoveEncodingKind::NumCode, CovePhysicalKind::NumCode) => {
            validate_fixed_le_width_for_validity(
                values,
                row_count,
                std::mem::size_of::<u64>(),
                validity,
            )?;
            Ok(LaneRef::NumCodeU64LeBytes {
                lane_id,
                bytes: values,
                row_count,
                validity,
                logical_type,
                domain,
            })
        }
        (CoveEncodingKind::FileCode, CovePhysicalKind::FileCode) => {
            validate_fixed_le_width_for_validity(
                values,
                row_count,
                std::mem::size_of::<u32>(),
                validity,
            )?;
            Ok(LaneRef::FileCodeU32LeBytes {
                lane_id,
                bytes: values,
                row_count,
                validity,
                logical_type,
                domain,
            })
        }
        (CoveEncodingKind::LocalCodebook, physical_kind) => local_codebook_lane_from_payload(
            lane_id,
            logical_type,
            physical_kind,
            values,
            row_count,
            validity,
            domain,
        ),
        (CoveEncodingKind::PlainFixed, CovePhysicalKind::Boolean) => {
            validate_fixed_le_width_for_validity(values, row_count, 1, validity)?;
            validate_bool_bytes(values, row_count, validity)?;
            Ok(LaneRef::Bool {
                lane_id,
                values,
                row_count,
                validity,
                domain,
            })
        }
        (CoveEncodingKind::PlainFixed, CovePhysicalKind::FixedBytes) => {
            let width = logical_type_fixed_width(logical_type).ok_or_else(|| {
                CoveError::UnsupportedEncoding(format!(
                    "plain-fixed native lane requires fixed-width logical type, got {logical_type:?}"
                ))
            })?;
            validate_fixed_le_width_for_validity(values, row_count, width, validity)?;
            Ok(LaneRef::FixedBytes {
                lane_id,
                values,
                width,
                row_count,
                validity,
                logical_type,
                domain,
            })
        }
        (CoveEncodingKind::VarBytes, CovePhysicalKind::VarBytes) => {
            let row_offsets = prepare_varbytes_row_offsets(values, row_count, validity)?;
            Ok(LaneRef::VarBytes {
                lane_id,
                row_offsets: Cow::Owned(row_offsets),
                values,
                validity,
                logical_type,
                domain,
            })
        }
        (encoding_kind, physical_kind) => Ok(LaneRef::DecodeBoundary {
            lane_id,
            logical_type,
            physical_kind,
            row_count,
            validity: Some(validity),
            reason: decode_boundary_reason_for_encoding(encoding_kind),
        }),
    }
}

fn local_codebook_lane_from_payload<'a>(
    lane_id: NativeLaneId,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'a>,
    domain: NativeCodeDomain,
) -> Result<LaneRef<'a>, CoveError> {
    let payload = LocalCodebookPayload::parse(values)?;
    let local_indexes = payload.decode_local_indexes()?;
    if local_indexes.len() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let local_to_global = match (physical_kind, &payload.values) {
        (CovePhysicalKind::FileCode, LocalCodebookValues::FileCode(values)) => {
            values.iter().copied().map(u64::from).collect::<Vec<_>>()
        }
        (CovePhysicalKind::NumCode, LocalCodebookValues::NumCode(values)) => values.clone(),
        (CovePhysicalKind::Boolean, LocalCodebookValues::Boolean(values)) => values
            .iter()
            .copied()
            .map(|value| u64::from(u8::from(value)))
            .collect::<Vec<_>>(),
        (CovePhysicalKind::VarBytes, LocalCodebookValues::VarBytes(_)) => {
            return Ok(LaneRef::DecodeBoundary {
                lane_id,
                logical_type,
                physical_kind,
                row_count,
                validity: Some(validity),
                reason: "local-codebook varbytes page needs local byte dictionary binding",
            });
        }
        _ => return Err(CoveError::PageCorrupt),
    };

    if local_to_global.len() <= (u8::MAX as usize) + 1 {
        let values = local_indexes
            .into_iter()
            .map(|value| u8::try_from(value).map_err(|_| CoveError::PageCorrupt))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(LaneRef::LocalCodeU8 {
            lane_id,
            values: Cow::Owned(values),
            validity,
            local_to_global: Cow::Owned(local_to_global),
            logical_type,
            physical_kind,
            domain,
        });
    }

    if local_to_global.len() <= (u16::MAX as usize) + 1 {
        let values = local_indexes
            .into_iter()
            .map(|value| u16::try_from(value).map_err(|_| CoveError::PageCorrupt))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(LaneRef::LocalCodeU16 {
            lane_id,
            values: Cow::Owned(values),
            validity,
            local_to_global: Cow::Owned(local_to_global),
            logical_type,
            physical_kind,
            domain,
        });
    }

    Ok(LaneRef::LocalCodeU32 {
        lane_id,
        values: Cow::Owned(local_indexes),
        validity,
        local_to_global: Cow::Owned(local_to_global),
        logical_type,
        physical_kind,
        domain,
    })
}

fn native_validity_from_page<'a, P: NativeColumnPagePayload + ?Sized>(
    payload: &'a P,
    page: &crate::page::ColumnPageIndexEntryV1,
) -> Result<ValidityRef<'a>, CoveError> {
    let row_count = usize::try_from(page.row_count).map_err(|_| CoveError::OffsetRange)?;
    if page.null_count == page.row_count || page.flags & PAGE_FLAG_ALL_NULL != 0 {
        return Ok(ValidityRef::AllNull { row_count });
    }
    if page.null_count == 0 || page.flags & PAGE_FLAG_ALL_NON_NULL != 0 {
        return Ok(ValidityRef::AllValid { row_count });
    }
    let Some(bytes) = payload.buffer_bytes_ref(PageBufferKind::NullBitmap)? else {
        return Err(CoveError::PageCorrupt);
    };
    validate_bitmap_width(bytes, row_count)?;
    Ok(ValidityRef::CoveNullBitmap { bytes, row_count })
}

fn elided_page_validity<'a>(page: &crate::page::ColumnPageIndexEntryV1) -> Option<ValidityRef<'a>> {
    let row_count = usize::try_from(page.row_count).ok()?;
    if page.null_count == page.row_count || page.flags & PAGE_FLAG_ALL_NULL != 0 {
        Some(ValidityRef::AllNull { row_count })
    } else if page.null_count == 0 || page.flags & PAGE_FLAG_ALL_NON_NULL != 0 {
        Some(ValidityRef::AllValid { row_count })
    } else {
        None
    }
}

fn decode_boundary_reason_for_encoding(encoding_kind: CoveEncodingKind) -> &'static str {
    match encoding_kind {
        CoveEncodingKind::LocalCodebook => "local-codebook page value kind is not code-native",
        CoveEncodingKind::VarBytes => "varbytes page needs offsets view",
        CoveEncodingKind::PlainFixed => "plain-fixed page needs width-specific lane binding",
        CoveEncodingKind::Validity => "validity page needs boolean lane binding",
        CoveEncodingKind::Constant => "constant page needs constant-lane binding",
        _ => "encoding is not lane-native in scalar native kernel yet",
    }
}

fn decode_boundary_reason_for_elided_page(flags: u32) -> &'static str {
    if flags & PAGE_FLAG_ALL_NULL != 0 {
        "all-null elided page"
    } else if flags & PAGE_FLAG_ALL_NON_NULL != 0 {
        "stats-only constant/elided non-null page requires stats materialization"
    } else {
        "property page has no retained payload"
    }
}

fn validate_fixed_le_width_for_validity(
    bytes: &[u8],
    row_count: usize,
    width: usize,
    validity: ValidityRef<'_>,
) -> Result<(), CoveError> {
    let expected = row_count
        .checked_mul(width)
        .ok_or(CoveError::ArithOverflow)?;
    if matches!(validity, ValidityRef::AllNull { .. }) && bytes.is_empty() {
        return Ok(());
    }
    if bytes.len() != expected {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

fn validate_bool_bytes(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
) -> Result<(), CoveError> {
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok(());
    }
    if values.len() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    if values.iter().any(|value| !matches!(value, 0 | 1)) {
        return Err(CoveError::PageCorrupt);
    }
    Ok(())
}

fn prepare_varbytes_row_offsets(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
) -> Result<Vec<u32>, CoveError> {
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok(vec![0; row_count]);
    }
    let mut row_offsets = Vec::with_capacity(row_count);
    let mut pos = 0usize;
    for _ in 0..row_count {
        row_offsets.push(u32::try_from(pos).map_err(|_| CoveError::ArithOverflow)?);
        let len_end = pos.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        let Some(len_bytes) = values.get(pos..len_end) else {
            return Err(CoveError::BufferTooShort);
        };
        let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
        pos = len_end.checked_add(len).ok_or(CoveError::ArithOverflow)?;
        if pos > values.len() {
            return Err(CoveError::BufferTooShort);
        }
    }
    if pos != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok(row_offsets)
}

fn varbytes_payload_at<'a>(
    row_offsets: &[u32],
    values: &'a [u8],
    row: usize,
) -> Result<&'a [u8], CoveError> {
    let offset = row_offsets
        .get(row)
        .copied()
        .ok_or(CoveError::OffsetRange)? as usize;
    let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
    let Some(len_bytes) = values.get(offset..len_end) else {
        return Err(CoveError::BufferTooShort);
    };
    let len = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
    let value_end = len_end.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    values
        .get(len_end..value_end)
        .ok_or(CoveError::BufferTooShort)
}

fn validate_bitmap_width(bytes: &[u8], row_count: usize) -> Result<(), CoveError> {
    let expected = row_count.checked_add(7).ok_or(CoveError::ArithOverflow)? / 8;
    if bytes.len() < expected {
        return Err(CoveError::BufferTooShort);
    }
    Ok(())
}

fn try_filter_u32_values_in_simd(
    values: &[u32],
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if needles.is_empty()
        || needles.len() > 8
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    if let Some(result) = filter_local_u32_membership_neon_dispatch(values, needles) {
        return Some(result);
    }
    filter_local_u32_membership_avx2_dispatch(values, needles)
}

fn try_filter_bool_eq_simd(
    values: &[u8],
    row_count: usize,
    needle: u8,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    debug_assert_eq!(values.len(), row_count);
    let needle = [needle];
    if let Some(result) = filter_local_u8_membership_neon_dispatch(values, &needle) {
        return Some(result);
    }
    filter_local_u8_membership_avx2_dispatch(values, &needle)
}

fn try_filter_local_u8_membership_simd(
    values: &[u8],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    let mut needles = [0u8; 8];
    let mut needle_count = 0usize;
    for (local, matches) in local_membership.iter().take(256).copied().enumerate() {
        if !matches {
            continue;
        }
        if needle_count == needles.len() {
            return None;
        }
        needles[needle_count] = local as u8;
        needle_count += 1;
    }
    if needle_count == 0 {
        return None;
    }
    if let Some(result) = filter_local_u8_membership_neon_dispatch(values, &needles[..needle_count])
    {
        return Some(result);
    }
    filter_local_u8_membership_avx2_dispatch(values, &needles[..needle_count])
}

fn try_filter_local_u16_membership_simd(
    values: &[u16],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    let (needles, needle_count) = collect_small_local_membership_u16(local_membership)?;
    if let Some(result) =
        filter_local_u16_membership_neon_dispatch(values, &needles[..needle_count])
    {
        return Some(result);
    }
    filter_local_u16_membership_avx2_dispatch(values, &needles[..needle_count])
}

fn try_filter_local_u32_membership_simd(
    values: &[u32],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count } if row_count == values.len())
    {
        return None;
    }
    let (needles, needle_count) = collect_small_local_membership_u32(local_membership)?;
    if let Some(result) =
        filter_local_u32_membership_neon_dispatch(values, &needles[..needle_count])
    {
        return Some(result);
    }
    filter_local_u32_membership_avx2_dispatch(values, &needles[..needle_count])
}

fn collect_small_local_membership_u16(local_membership: &[bool]) -> Option<([u16; 8], usize)> {
    let mut needles = [0u16; 8];
    let mut needle_count = 0usize;
    for (local, matches) in local_membership
        .iter()
        .take(usize::from(u16::MAX) + 1)
        .copied()
        .enumerate()
    {
        if !matches {
            continue;
        }
        if needle_count == needles.len() {
            return None;
        }
        needles[needle_count] = local as u16;
        needle_count += 1;
    }
    (needle_count != 0).then_some((needles, needle_count))
}

fn collect_small_local_membership_u32(local_membership: &[bool]) -> Option<([u32; 8], usize)> {
    const MAX_SIMD_MEMBERSHIP_SCAN: usize = 1 << 20;
    if local_membership.len() > MAX_SIMD_MEMBERSHIP_SCAN {
        return None;
    }
    let mut needles = [0u32; 8];
    let mut needle_count = 0usize;
    for (local, matches) in local_membership.iter().copied().enumerate() {
        if !matches {
            continue;
        }
        if needle_count == needles.len() {
            return None;
        }
        needles[needle_count] = u32::try_from(local).ok()?;
        needle_count += 1;
    }
    (needle_count != 0).then_some((needles, needle_count))
}

fn try_filter_u64_le_eq_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: u64,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    match policy {
        NativeKernelDispatch::Scalar => None,
        NativeKernelDispatch::Auto => filter_u64_le_eq_auto(bytes, row_count, needle),
        NativeKernelDispatch::Avx2 => filter_u64_le_eq_avx2_dispatch(bytes, row_count, needle),
        NativeKernelDispatch::Neon => filter_u64_le_eq_neon_dispatch(bytes, row_count, needle),
    }
}

fn try_filter_u32_le_eq_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: u32,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    match policy {
        NativeKernelDispatch::Scalar => None,
        NativeKernelDispatch::Auto => filter_u32_le_eq_auto(bytes, row_count, needle),
        NativeKernelDispatch::Avx2 => filter_u32_le_eq_avx2_dispatch(bytes, row_count, needle),
        NativeKernelDispatch::Neon => filter_u32_le_eq_neon_dispatch(bytes, row_count, needle),
    }
}

fn try_filter_i64_le_cmp_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    op: NativeNumericPredicateOp,
    needle: i64,
    base: Option<&SelectionBitmap>,
    policy: NativeKernelDispatch,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    match policy {
        NativeKernelDispatch::Scalar => None,
        NativeKernelDispatch::Auto => filter_i64_le_cmp_auto(bytes, row_count, op, needle),
        NativeKernelDispatch::Avx2 => filter_i64_le_cmp_avx2_dispatch(bytes, row_count, op, needle),
        NativeKernelDispatch::Neon => filter_i64_le_cmp_neon_dispatch(bytes, row_count, op, needle),
    }
}

fn try_filter_i64_le_range_simd(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    base: Option<&SelectionBitmap>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    if !cfg!(target_endian = "little")
        || base.is_some()
        || !matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        return None;
    }
    if let Some(result) = filter_i64_le_range_neon_dispatch(bytes, row_count, lower, upper) {
        return Some(result);
    }
    filter_i64_le_range_avx2_dispatch(bytes, row_count, lower, upper)
}

fn filter_u64_le_eq_auto(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        filter_u64_le_eq_neon_dispatch(bytes, row_count, needle)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                filter_u64_le_eq_avx2_dispatch(bytes, row_count, needle)
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (bytes, row_count, needle);
            None
        }
    }
}

fn filter_u32_le_eq_auto(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        filter_u32_le_eq_neon_dispatch(bytes, row_count, needle)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                filter_u32_le_eq_avx2_dispatch(bytes, row_count, needle)
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (bytes, row_count, needle);
            None
        }
    }
}

fn filter_i64_le_cmp_auto(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        filter_i64_le_cmp_neon_dispatch(bytes, row_count, op, needle)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") {
                filter_i64_le_cmp_avx2_dispatch(bytes, row_count, op, needle)
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (bytes, row_count, op, needle);
            None
        }
    }
}

fn filter_local_u8_membership_avx2_dispatch(
    values: &[u8],
    needles: &[u8],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // passes initialized `values`, at most eight initialized needle bytes,
        // and the SIMD routine performs only in-bounds unaligned loads plus a
        // scalar tail.
        unsafe {
            filter_local_u8_membership_avx2(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (values, needles);
        None
    }
}

fn filter_local_u8_membership_neon_dispatch(
    values: &[u8],
    needles: &[u8],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // passes initialized `values`, at most eight initialized needle bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u8_membership_neon(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (values, needles);
        None
    }
}

fn filter_local_u16_membership_avx2_dispatch(
    values: &[u16],
    needles: &[u16],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u16_membership_avx2(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (values, needles);
        None
    }
}

fn filter_local_u16_membership_neon_dispatch(
    values: &[u16],
    needles: &[u16],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u16_membership_neon(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (values, needles);
        None
    }
}

fn filter_local_u32_membership_avx2_dispatch(
    values: &[u32],
    needles: &[u32],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u32_membership_avx2(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (values, needles);
        None
    }
}

fn filter_local_u32_membership_neon_dispatch(
    values: &[u32],
    needles: &[u32],
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(values.len());
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // passes initialized `values`, at most eight initialized needle values,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_local_u32_membership_neon(values, needles, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (values, needles);
        None
    }
}

fn filter_u64_le_eq_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u64_le_eq_avx2(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

fn filter_u32_le_eq_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 4` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u32_le_eq_avx2(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

fn filter_i64_le_cmp_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_cmp_avx2(bytes, row_count, op, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, op, needle);
        None
    }
}

fn filter_i64_le_range_avx2_dispatch(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx2") {
            return None;
        }
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: Runtime feature detection proves AVX2 support. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_range_avx2(bytes, row_count, lower, upper, &mut out);
        }
        Some((out, NativeKernelDispatch::Avx2))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (bytes, row_count, lower, upper);
        None
    }
}

fn filter_u64_le_eq_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u64_le_eq_neon(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

fn filter_u32_le_eq_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 4` initialized bytes,
        // and the SIMD routine performs only in-bounds unaligned loads.
        unsafe {
            filter_u32_le_eq_neon(bytes, row_count, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, needle);
        None
    }
}

fn filter_i64_le_cmp_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_cmp_neon(bytes, row_count, op, needle, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, op, needle);
        None
    }
}

fn filter_i64_le_range_neon_dispatch(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
) -> Option<(SelectionBitmap, NativeKernelDispatch)> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut out = SelectionBitmap::none(row_count);
        // SAFETY: AArch64 provides NEON as a baseline feature. The caller
        // validated that `bytes` contains `row_count * 8` initialized bytes,
        // all rows are valid, and the SIMD routine performs only in-bounds
        // unaligned loads plus a scalar tail.
        unsafe {
            filter_i64_le_range_neon(bytes, row_count, lower, upper, &mut out);
        }
        Some((out, NativeKernelDispatch::Neon))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (bytes, row_count, lower, upper);
        None
    }
}

#[inline]
fn intersect_words_scalar(left: &mut [u64], right: &[u64]) {
    for (left, right) in left.iter_mut().zip(right) {
        *left &= *right;
    }
}

#[inline]
fn intersect_words_auto(left: &mut [u64], right: &[u64]) -> NativeKernelDispatch {
    if left.len() < 16 {
        intersect_words_scalar(left, right);
        return NativeKernelDispatch::Scalar;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: Runtime feature detection proves AVX2 support. The
            // slices are valid for `left.len()` u64 words and the function
            // handles unaligned loads/stores plus a scalar tail.
            unsafe {
                intersect_words_avx2(left, right);
            }
            return NativeKernelDispatch::Avx2;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 has Advanced SIMD/NEON as part of the baseline ISA.
        // SAFETY: The slices are valid for `left.len()` u64 words and the
        // function handles unaligned loads/stores plus a scalar tail.
        unsafe {
            intersect_words_neon(left, right);
        }
        NativeKernelDispatch::Neon
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        intersect_words_scalar(left, right);
        NativeKernelDispatch::Scalar
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn intersect_words_avx2(left: &mut [u64], right: &[u64]) {
    use std::arch::x86_64::{__m256i, _mm256_and_si256, _mm256_loadu_si256, _mm256_storeu_si256};

    let chunks = left.len() / 4;
    for chunk in 0..chunks {
        let offset = chunk * 4;
        // SAFETY: `offset..offset+4` is inside both slices by construction.
        // AVX2 unaligned loads/stores accept arbitrary byte alignment.
        unsafe {
            let left_ptr = left.as_mut_ptr().add(offset).cast::<__m256i>();
            let right_ptr = right.as_ptr().add(offset).cast::<__m256i>();
            let lhs = _mm256_loadu_si256(left_ptr);
            let rhs = _mm256_loadu_si256(right_ptr);
            _mm256_storeu_si256(left_ptr, _mm256_and_si256(lhs, rhs));
        }
    }
    let tail = chunks * 4;
    intersect_words_scalar(&mut left[tail..], &right[tail..]);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn append_dense_u32_rows_avx2(rows: &mut Vec<u32>, base: u32) {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi32, _mm256_set1_epi32, _mm256_setr_epi32, _mm256_storeu_si256,
    };

    rows.reserve(64);
    let start = rows.len();
    // SAFETY: `rows.reserve(64)` guarantees space for 64 additional `u32`
    // values. The loop writes each new slot exactly once before `set_len`.
    unsafe {
        let ptr = rows.as_mut_ptr().add(start);
        let increments = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
        for offset in (0..64).step_by(8) {
            let chunk_base = base + offset as u32;
            let values = _mm256_add_epi32(_mm256_set1_epi32(chunk_base as i32), increments);
            _mm256_storeu_si256(ptr.add(offset).cast::<__m256i>(), values);
        }
        rows.set_len(start + 64);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn filter_local_u8_membership_avx2(
    values: &[u8],
    needles: &[u8],
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_or_si256,
        _mm256_set1_epi8, _mm256_setzero_si256,
    };

    let chunks = values.len() / 32;
    for chunk in 0..chunks {
        let row = chunk * 32;
        // SAFETY: `row..row+32` is inside `values` by construction. AVX2
        // unaligned loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row).cast::<__m256i>();
            let haystack = _mm256_loadu_si256(ptr);
            let mut matched = _mm256_setzero_si256();
            for needle in needles {
                let needle_vector = _mm256_set1_epi8(*needle as i8);
                matched = _mm256_or_si256(matched, _mm256_cmpeq_epi8(haystack, needle_vector));
            }
            _mm256_movemask_epi8(matched) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u8_membership_scalar_tail(values, chunks * 32, needles, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn filter_local_u16_membership_avx2(
    values: &[u16],
    needles: &[u16],
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi16, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_or_si256,
        _mm256_set1_epi16, _mm256_setzero_si256,
    };

    let chunks = values.len() / 16;
    for chunk in 0..chunks {
        let row = chunk * 16;
        // SAFETY: `row..row+16` is inside `values` by construction. AVX2
        // unaligned loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row).cast::<__m256i>();
            let haystack = _mm256_loadu_si256(ptr);
            let mut matched = _mm256_setzero_si256();
            for needle in needles {
                let needle_vector = _mm256_set1_epi16(*needle as i16);
                matched = _mm256_or_si256(matched, _mm256_cmpeq_epi16(haystack, needle_vector));
            }
            avx2_u16_lane_mask(_mm256_movemask_epi8(matched) as u32)
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u16_membership_scalar_tail(values, chunks * 16, needles, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn filter_local_u32_membership_avx2(
    values: &[u32],
    needles: &[u32],
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_ps, _mm256_cmpeq_epi32, _mm256_loadu_si256, _mm256_movemask_ps,
        _mm256_or_si256, _mm256_set1_epi32, _mm256_setzero_si256,
    };

    let chunks = values.len() / 8;
    for chunk in 0..chunks {
        let row = chunk * 8;
        // SAFETY: `row..row+8` is inside `values` by construction. AVX2
        // unaligned loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row).cast::<__m256i>();
            let haystack = _mm256_loadu_si256(ptr);
            let mut matched = _mm256_setzero_si256();
            for needle in needles {
                let needle_vector = _mm256_set1_epi32(*needle as i32);
                matched = _mm256_or_si256(matched, _mm256_cmpeq_epi32(haystack, needle_vector));
            }
            _mm256_movemask_ps(_mm256_castsi256_ps(matched)) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u32_membership_scalar_tail(values, chunks * 8, needles, out);
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn avx2_u16_lane_mask(byte_mask: u32) -> u64 {
    let mut lane_mask = 0u64;
    for lane in 0..16 {
        if byte_mask & (0b11u32 << (lane * 2)) != 0 {
            lane_mask |= 1u64 << lane;
        }
    }
    lane_mask
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn filter_u64_le_eq_avx2(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_pd, _mm256_cmpeq_epi64, _mm256_loadu_si256, _mm256_movemask_pd,
        _mm256_set1_epi64x,
    };

    let needle_vector = _mm256_set1_epi64x(needle as i64);
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let cmp = _mm256_cmpeq_epi64(values, needle_vector);
            _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u64_le_eq_scalar_tail(bytes, chunks * 4, row_count, needle, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn filter_u32_le_eq_avx2(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_ps, _mm256_cmpeq_epi32, _mm256_loadu_si256, _mm256_movemask_ps,
        _mm256_set1_epi32,
    };

    let needle_vector = _mm256_set1_epi32(needle as i32);
    let chunks = row_count / 8;
    for chunk in 0..chunks {
        let row = chunk * 8;
        // SAFETY: `row..row+8` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 4` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 4).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let cmp = _mm256_cmpeq_epi32(values, needle_vector);
            _mm256_movemask_ps(_mm256_castsi256_ps(cmp)) as u64
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u32_le_eq_scalar_tail(bytes, chunks * 8, row_count, needle, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn filter_i64_le_cmp_avx2(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_pd, _mm256_cmpeq_epi64, _mm256_cmpgt_epi64, _mm256_loadu_si256,
        _mm256_movemask_pd, _mm256_set1_epi64x,
    };

    let needle_vector = _mm256_set1_epi64x(needle);
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let raw_mask = match op {
                NativeNumericPredicateOp::Eq => {
                    let cmp = _mm256_cmpeq_epi64(values, needle_vector);
                    _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
                }
                NativeNumericPredicateOp::Gt => {
                    let cmp = _mm256_cmpgt_epi64(values, needle_vector);
                    _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
                }
                NativeNumericPredicateOp::GtEq => {
                    let cmp = _mm256_cmpgt_epi64(needle_vector, values);
                    !(_mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64)
                }
                NativeNumericPredicateOp::Lt => {
                    let cmp = _mm256_cmpgt_epi64(needle_vector, values);
                    _mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64
                }
                NativeNumericPredicateOp::LtEq => {
                    let cmp = _mm256_cmpgt_epi64(values, needle_vector);
                    !(_mm256_movemask_pd(_mm256_castsi256_pd(cmp)) as u64)
                }
            };
            raw_mask & 0b1111
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_cmp_scalar_tail(bytes, chunks * 4, row_count, op, needle, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn filter_i64_le_range_avx2(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    out: &mut SelectionBitmap,
) {
    use std::arch::x86_64::{
        __m256i, _mm256_castsi256_pd, _mm256_cmpgt_epi64, _mm256_loadu_si256, _mm256_movemask_pd,
        _mm256_set1_epi64x,
    };

    let lower_vector = lower.map(|(bound, inclusive)| (_mm256_set1_epi64x(bound), inclusive));
    let upper_vector = upper.map(|(bound, inclusive)| (_mm256_set1_epi64x(bound), inclusive));
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AVX2 unaligned
        // loads accept arbitrary byte alignment.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<__m256i>();
            let values = _mm256_loadu_si256(ptr);
            let mut mask = 0b1111u64;
            if let Some((bound_vector, inclusive)) = lower_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        let less_than_lower = _mm256_cmpgt_epi64(bound_vector, values);
                        !(_mm256_movemask_pd(_mm256_castsi256_pd(less_than_lower)) as u64)
                    }
                    BoundInclusive::Exclusive => {
                        let greater_than_lower = _mm256_cmpgt_epi64(values, bound_vector);
                        _mm256_movemask_pd(_mm256_castsi256_pd(greater_than_lower)) as u64
                    }
                };
                mask &= raw & 0b1111;
            }
            if let Some((bound_vector, inclusive)) = upper_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        let greater_than_upper = _mm256_cmpgt_epi64(values, bound_vector);
                        !(_mm256_movemask_pd(_mm256_castsi256_pd(greater_than_upper)) as u64)
                    }
                    BoundInclusive::Exclusive => {
                        let less_than_upper = _mm256_cmpgt_epi64(bound_vector, values);
                        _mm256_movemask_pd(_mm256_castsi256_pd(less_than_upper)) as u64
                    }
                };
                mask &= raw & 0b1111;
            }
            mask
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_range_scalar_tail(bytes, chunks * 4, row_count, lower, upper, out);
}

#[cfg(target_arch = "aarch64")]
unsafe fn append_dense_u32_rows_neon(rows: &mut Vec<u32>, base: u32) {
    use std::arch::aarch64::{uint32x4_t, vaddq_u32, vdupq_n_u32, vld1q_u32, vst1q_u32};

    const INCREMENTS: [u32; 4] = [0, 1, 2, 3];

    rows.reserve(64);
    let start = rows.len();
    // SAFETY: `INCREMENTS` is a 4-lane initialized table, and
    // `rows.reserve(64)` guarantees space for 64 additional `u32` values.
    // The loop writes each new slot exactly once before `set_len`.
    unsafe {
        let increments: uint32x4_t = vld1q_u32(INCREMENTS.as_ptr());
        let ptr = rows.as_mut_ptr().add(start);
        for offset in (0..64).step_by(4) {
            let chunk_base = base + offset as u32;
            let values = vaddq_u32(vdupq_n_u32(chunk_base), increments);
            vst1q_u32(ptr.add(offset), values);
        }
        rows.set_len(start + 64);
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn filter_local_u8_membership_neon(
    values: &[u8],
    needles: &[u8],
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        uint8x16_t, vaddv_u8, vandq_u8, vceqq_u8, vdupq_n_u8, vget_high_u8, vget_low_u8, vld1q_u8,
        vorrq_u8,
    };

    const BIT_WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];

    // SAFETY: `BIT_WEIGHTS` is a 16-byte initialized table.
    let weights = unsafe { vld1q_u8(BIT_WEIGHTS.as_ptr()) };
    let chunks = values.len() / 16;
    for chunk in 0..chunks {
        let row = chunk * 16;
        // SAFETY: `row..row+16` is inside `values` by construction. AArch64
        // vector loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row);
            let haystack: uint8x16_t = vld1q_u8(ptr);
            let mut matched = vdupq_n_u8(0);
            for needle in needles {
                let needle_vector = vdupq_n_u8(*needle);
                matched = vorrq_u8(matched, vceqq_u8(haystack, needle_vector));
            }
            let weighted = vandq_u8(matched, weights);
            let low = u64::from(vaddv_u8(vget_low_u8(weighted)));
            let high = u64::from(vaddv_u8(vget_high_u8(weighted))) << 8;
            low | high
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u8_membership_scalar_tail(values, chunks * 16, needles, out);
}

#[cfg(target_arch = "aarch64")]
unsafe fn filter_local_u16_membership_neon(
    values: &[u16],
    needles: &[u16],
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        uint16x8_t, vaddvq_u16, vandq_u16, vceqq_u16, vdupq_n_u16, vld1q_u16, vorrq_u16,
    };

    const BIT_WEIGHTS: [u16; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

    // SAFETY: `BIT_WEIGHTS` is an 8-lane initialized table.
    let weights = unsafe { vld1q_u16(BIT_WEIGHTS.as_ptr()) };
    let chunks = values.len() / 8;
    for chunk in 0..chunks {
        let row = chunk * 8;
        // SAFETY: `row..row+8` is inside `values` by construction. AArch64
        // vector loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row);
            let haystack: uint16x8_t = vld1q_u16(ptr);
            let mut matched = vdupq_n_u16(0);
            for needle in needles {
                let needle_vector = vdupq_n_u16(*needle);
                matched = vorrq_u16(matched, vceqq_u16(haystack, needle_vector));
            }
            u64::from(vaddvq_u16(vandq_u16(matched, weights)))
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u16_membership_scalar_tail(values, chunks * 8, needles, out);
}

#[cfg(target_arch = "aarch64")]
unsafe fn filter_local_u32_membership_neon(
    values: &[u32],
    needles: &[u32],
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        uint32x4_t, vaddvq_u32, vandq_u32, vceqq_u32, vdupq_n_u32, vld1q_u32, vorrq_u32,
    };

    const BIT_WEIGHTS: [u32; 4] = [1, 2, 4, 8];

    // SAFETY: `BIT_WEIGHTS` is a 4-lane initialized table.
    let weights = unsafe { vld1q_u32(BIT_WEIGHTS.as_ptr()) };
    let chunks = values.len() / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `values` by construction. AArch64
        // vector loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = values.as_ptr().add(row);
            let haystack: uint32x4_t = vld1q_u32(ptr);
            let mut matched = vdupq_n_u32(0);
            for needle in needles {
                let needle_vector = vdupq_n_u32(*needle);
                matched = vorrq_u32(matched, vceqq_u32(haystack, needle_vector));
            }
            u64::from(vaddvq_u32(vandq_u32(matched, weights)))
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_local_u32_membership_scalar_tail(values, chunks * 4, needles, out);
}

#[cfg(target_arch = "aarch64")]
unsafe fn intersect_words_neon(left: &mut [u64], right: &[u64]) {
    use std::arch::aarch64::{uint64x2_t, vandq_u64, vld1q_u64, vst1q_u64};

    let chunks = left.len() / 2;
    for chunk in 0..chunks {
        let offset = chunk * 2;
        // SAFETY: `offset..offset+2` is inside both slices by construction.
        // AArch64 vector loads/stores used here permit unaligned addresses.
        unsafe {
            let left_ptr = left.as_mut_ptr().add(offset);
            let right_ptr = right.as_ptr().add(offset);
            let lhs: uint64x2_t = vld1q_u64(left_ptr);
            let rhs: uint64x2_t = vld1q_u64(right_ptr);
            vst1q_u64(left_ptr, vandq_u64(lhs, rhs));
        }
    }
    let tail = chunks * 2;
    intersect_words_scalar(&mut left[tail..], &right[tail..]);
}

#[cfg(target_arch = "aarch64")]
unsafe fn filter_u64_le_eq_neon(
    bytes: &[u8],
    row_count: usize,
    needle: u64,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{uint64x2_t, vceqq_u64, vdupq_n_u64, vgetq_lane_u64, vld1q_u64};

    let needle_vector = vdupq_n_u64(needle);
    let chunks = row_count / 2;
    for chunk in 0..chunks {
        let row = chunk * 2;
        // SAFETY: `row..row+2` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let cmp = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<u64>();
            let values: uint64x2_t = vld1q_u64(ptr);
            vceqq_u64(values, needle_vector)
        };
        let mask = u64::from(vgetq_lane_u64::<0>(cmp) == u64::MAX)
            | (u64::from(vgetq_lane_u64::<1>(cmp) == u64::MAX) << 1);
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u64_le_eq_scalar_tail(bytes, chunks * 2, row_count, needle, out);
}

#[cfg(target_arch = "aarch64")]
unsafe fn filter_u32_le_eq_neon(
    bytes: &[u8],
    row_count: usize,
    needle: u32,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{uint32x4_t, vceqq_u32, vdupq_n_u32, vgetq_lane_u32, vld1q_u32};

    let needle_vector = vdupq_n_u32(needle);
    let chunks = row_count / 4;
    for chunk in 0..chunks {
        let row = chunk * 4;
        // SAFETY: `row..row+4` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 4` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let cmp = unsafe {
            let ptr = bytes.as_ptr().add(row * 4).cast::<u32>();
            let values: uint32x4_t = vld1q_u32(ptr);
            vceqq_u32(values, needle_vector)
        };
        let mask = u64::from(vgetq_lane_u32::<0>(cmp) == u32::MAX)
            | (u64::from(vgetq_lane_u32::<1>(cmp) == u32::MAX) << 1)
            | (u64::from(vgetq_lane_u32::<2>(cmp) == u32::MAX) << 2)
            | (u64::from(vgetq_lane_u32::<3>(cmp) == u32::MAX) << 3);
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_u32_le_eq_scalar_tail(bytes, chunks * 4, row_count, needle, out);
}

#[cfg(target_arch = "aarch64")]
unsafe fn filter_i64_le_cmp_neon(
    bytes: &[u8],
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        int64x2_t, uint64x2_t, vceqq_s64, vcgtq_s64, vdupq_n_s64, vgetq_lane_u64, vld1q_s64,
    };

    let needle_vector = vdupq_n_s64(needle);
    let chunks = row_count / 2;
    for chunk in 0..chunks {
        let row = chunk * 2;
        // SAFETY: `row..row+2` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<i64>();
            let values: int64x2_t = vld1q_s64(ptr);
            let raw_mask = match op {
                NativeNumericPredicateOp::Eq => neon_i64_cmp_mask(vceqq_s64(values, needle_vector)),
                NativeNumericPredicateOp::Gt => neon_i64_cmp_mask(vcgtq_s64(values, needle_vector)),
                NativeNumericPredicateOp::GtEq => {
                    !neon_i64_cmp_mask(vcgtq_s64(needle_vector, values))
                }
                NativeNumericPredicateOp::Lt => neon_i64_cmp_mask(vcgtq_s64(needle_vector, values)),
                NativeNumericPredicateOp::LtEq => {
                    !neon_i64_cmp_mask(vcgtq_s64(values, needle_vector))
                }
            };
            raw_mask & 0b11
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_cmp_scalar_tail(bytes, chunks * 2, row_count, op, needle, out);

    #[inline]
    fn neon_i64_cmp_mask(cmp: uint64x2_t) -> u64 {
        u64::from(unsafe { vgetq_lane_u64::<0>(cmp) } == u64::MAX)
            | (u64::from(unsafe { vgetq_lane_u64::<1>(cmp) } == u64::MAX) << 1)
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn filter_i64_le_range_neon(
    bytes: &[u8],
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    out: &mut SelectionBitmap,
) {
    use std::arch::aarch64::{
        int64x2_t, uint64x2_t, vcgtq_s64, vdupq_n_s64, vgetq_lane_u64, vld1q_s64,
    };

    let lower_vector = lower.map(|(bound, inclusive)| (vdupq_n_s64(bound), inclusive));
    let upper_vector = upper.map(|(bound, inclusive)| (vdupq_n_s64(bound), inclusive));
    let chunks = row_count / 2;
    for chunk in 0..chunks {
        let row = chunk * 2;
        // SAFETY: `row..row+2` is inside `row_count`, and callers validated
        // that `bytes` has `row_count * 8` initialized bytes. AArch64 vector
        // loads used here permit unaligned addresses.
        let mask = unsafe {
            let ptr = bytes.as_ptr().add(row * 8).cast::<i64>();
            let values: int64x2_t = vld1q_s64(ptr);
            let mut mask = 0b11u64;
            if let Some((bound_vector, inclusive)) = lower_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        !neon_i64_cmp_mask(vcgtq_s64(bound_vector, values))
                    }
                    BoundInclusive::Exclusive => neon_i64_cmp_mask(vcgtq_s64(values, bound_vector)),
                };
                mask &= raw & 0b11;
            }
            if let Some((bound_vector, inclusive)) = upper_vector {
                let raw = match inclusive {
                    BoundInclusive::Inclusive => {
                        !neon_i64_cmp_mask(vcgtq_s64(values, bound_vector))
                    }
                    BoundInclusive::Exclusive => neon_i64_cmp_mask(vcgtq_s64(bound_vector, values)),
                };
                mask &= raw & 0b11;
            }
            mask
        };
        out.words[row / 64] |= mask << (row % 64);
    }
    filter_i64_le_range_scalar_tail(bytes, chunks * 2, row_count, lower, upper, out);

    #[inline]
    fn neon_i64_cmp_mask(cmp: uint64x2_t) -> u64 {
        u64::from(unsafe { vgetq_lane_u64::<0>(cmp) } == u64::MAX)
            | (u64::from(unsafe { vgetq_lane_u64::<1>(cmp) } == u64::MAX) << 1)
    }
}

fn filter_u64_le_eq_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    needle: u64,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<u64>();
        let value = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if value == needle {
            out.set(row);
        }
    }
}

fn filter_u32_le_eq_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    needle: u32,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<u32>();
        let value = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if value == needle {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter_local_u8_membership_scalar_tail(
    values: &[u8],
    start: usize,
    needles: &[u8],
    out: &mut SelectionBitmap,
) {
    for (row, value) in values.iter().copied().enumerate().skip(start) {
        if needles.contains(&value) {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter_local_u16_membership_scalar_tail(
    values: &[u16],
    start: usize,
    needles: &[u16],
    out: &mut SelectionBitmap,
) {
    for (row, value) in values.iter().copied().enumerate().skip(start) {
        if needles.contains(&value) {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter_local_u32_membership_scalar_tail(
    values: &[u32],
    start: usize,
    needles: &[u32],
    out: &mut SelectionBitmap,
) {
    for (row, value) in values.iter().copied().enumerate().skip(start) {
        if needles.contains(&value) {
            out.set(row);
        }
    }
}

fn filter_i64_le_cmp_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    op: NativeNumericPredicateOp,
    needle: i64,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<i64>();
        let value = i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if compare_i64_predicate(value, op, needle) {
            out.set(row);
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter_i64_le_range_scalar_tail(
    bytes: &[u8],
    start: usize,
    row_count: usize,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    out: &mut SelectionBitmap,
) {
    for row in start..row_count {
        let offset = row * std::mem::size_of::<i64>();
        let value = i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if i64_value_in_range(value, lower, upper) {
            out.set(row);
        }
    }
}

#[inline]
fn i64_value_in_range(
    value: i64,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
) -> bool {
    if lower.is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, false)) {
        return false;
    }
    if upper.is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, true)) {
        return false;
    }
    true
}

#[inline]
fn compare_i64_predicate(value: i64, op: NativeNumericPredicateOp, needle: i64) -> bool {
    match op {
        NativeNumericPredicateOp::Eq => value == needle,
        NativeNumericPredicateOp::Lt => value < needle,
        NativeNumericPredicateOp::LtEq => value <= needle,
        NativeNumericPredicateOp::Gt => value > needle,
        NativeNumericPredicateOp::GtEq => value >= needle,
    }
}

fn mask_last_word(words: &mut [u64], len: usize) {
    let used = len % 64;
    if used == 0 {
        return;
    }
    if let Some(last) = words.last_mut() {
        *last &= (1u64 << used) - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        checksum,
        constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind},
        encoding::{
            bit_packed::BitPackedPayload,
            local_codebook::{LocalCodebookPayload, LocalCodebookValues, LocalIndexPayload},
        },
        page::ColumnPageIndexEntryV1,
        page_payload::{ColumnPagePayloadV1, RetainedColumnPagePayloadV1},
        profile::cove_o::{
            RecordKind, RetainedTemporalPropertyColumn, RetainedTemporalPropertyPage,
            RetainedTemporalSegmentData, TemporalPropertyColumn, TemporalPropertyPage,
            TemporalRowEntryV1, TemporalSegmentData, TemporalSegmentHeaderV1,
        },
        retained_bytes::RetainedBytes,
        segment::{
            RowMorselDirectory, RowMorselEntryV1, TableColumnDirectoryEntryV1,
            TableSegmentHeaderV1, TableSegmentPayloadV1,
        },
    };

    fn assert_fixed_width_auto_dispatch(stats: &KernelStats) {
        #[cfg(target_arch = "aarch64")]
        assert_eq!(stats.dispatch, NativeKernelDispatch::Neon);
        #[cfg(target_arch = "x86_64")]
        {
            let expected = if std::arch::is_x86_feature_detected!("avx2") {
                NativeKernelDispatch::Avx2
            } else {
                NativeKernelDispatch::Scalar
            };
            assert_eq!(stats.dispatch, expected);
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
    }

    fn assert_local_membership_auto_dispatch(stats: &KernelStats) {
        #[cfg(target_arch = "aarch64")]
        assert_eq!(stats.dispatch, NativeKernelDispatch::Neon);
        #[cfg(target_arch = "x86_64")]
        {
            let expected = if std::arch::is_x86_feature_detected!("avx2") {
                NativeKernelDispatch::Avx2
            } else {
                NativeKernelDispatch::Scalar
            };
            assert_eq!(stats.dispatch, expected);
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
    }

    #[test]
    fn bitmap_to_selection_vector_uses_set_bits_only() {
        let mut bitmap = SelectionBitmap::all(130);
        bitmap.clear(0);
        bitmap.clear(64);
        bitmap.clear(129);
        let vector = bitmap.to_selection_vector();
        assert_eq!(vector.len(), 127);
        assert_eq!(vector.rows()[0], 1);
        assert_eq!(vector.rows()[63], 65);
        assert_eq!(vector.rows()[126], 128);
    }

    #[test]
    fn compact_selection_bitmap_uses_dense_word_fast_path_and_tail_bits() {
        let mut bitmap = SelectionBitmap::all(130);
        bitmap.clear(64);
        bitmap.clear(129);
        let (vector, stats) =
            compact_selection_bitmap(&bitmap, NativeKernelDispatch::Auto).unwrap();

        assert_eq!(vector.len(), 128);
        assert_eq!(vector.rows()[0], 0);
        assert_eq!(vector.rows()[63], 63);
        assert_eq!(vector.rows()[64], 65);
        assert_eq!(vector.rows()[127], 128);
        assert_eq!(stats.rows_seen, 130);
        assert_eq!(stats.rows_matched, 128);
        assert_eq!(stats.rows_valid, 128);
        assert_fixed_width_auto_dispatch(&stats);
    }

    #[test]
    fn validity_ref_uses_cove_null_polarity() {
        let bytes = [0b0000_0101u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &bytes,
            row_count: 4,
        };
        assert!(!validity.is_valid(0));
        assert!(validity.is_valid(1));
        assert!(!validity.is_valid(2));
        assert!(validity.is_valid(3));
        assert_eq!(validity.valid_count(), 2);
    }

    #[test]
    fn validity_ref_all_null_never_accepts_rows() {
        let validity = ValidityRef::AllNull { row_count: 3 };
        assert_eq!(validity.row_count(), 3);
        assert_eq!(validity.valid_count(), 0);
        assert!(!validity.is_valid(0));
        assert!(!validity.is_valid(2));
    }

    #[test]
    fn validity_filter_respects_base_and_null_polarity() {
        let nulls = [0b0000_1010u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: 5,
        };
        let mut base = SelectionBitmap::all(5);
        base.clear(2);

        let (is_null, stats) = filter_validity(5, validity, false, Some(&base));

        assert_eq!(is_null.to_selection_vector().rows(), &[1, 3]);
        assert_eq!(stats.rows_seen, 5);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn validity_filter_fast_paths_all_valid_and_all_null() {
        let mut base = SelectionBitmap::none(4);
        base.set(1);
        base.set(3);

        let (all_valid, _) =
            filter_validity(4, ValidityRef::AllValid { row_count: 4 }, true, Some(&base));
        let (no_nulls, _) = filter_validity(
            4,
            ValidityRef::AllValid { row_count: 4 },
            false,
            Some(&base),
        );
        let (all_null, _) =
            filter_validity(4, ValidityRef::AllNull { row_count: 4 }, false, Some(&base));

        assert_eq!(all_valid, base);
        assert_eq!(no_nulls.count_ones(), 0);
        assert_eq!(all_null.to_selection_vector().rows(), &[1, 3]);
    }

    #[test]
    fn selection_bitmap_all_set_respects_tail_bits() {
        let mut selected = SelectionBitmap::all(130);
        assert!(selected.all_set());

        selected.clear(129);
        assert!(!selected.all_set());

        selected.set(129);
        assert!(selected.all_set());

        selected.words_mut()[2] |= !((1u64 << 2) - 1);
        assert!(!selected.all_set());

        let empty = SelectionBitmap::all(0);
        assert!(empty.all_set());
    }

    #[test]
    fn u64_eq_filter_respects_validity_and_base() {
        let values = [7, 8, 7, 7, 9];
        let nulls = [0b0000_1000u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };
        let mut base = SelectionBitmap::all(values.len());
        base.clear(0);
        let (selected, stats) = filter_u64_eq(&values, validity, 7, Some(&base));
        assert_eq!(selected.to_selection_vector().rows(), &[2]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn u64_le_eq_filter_reads_page_bytes_without_typed_casts() {
        let values = [7u64, 8, 7, 9];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0000_0100u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };

        let (selected, stats) = filter_u64_le_eq(&bytes, values.len(), validity, 7, None).unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn u64_le_eq_dispatch_auto_matches_scalar_on_all_valid_page() {
        let row_count = 257;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u64>());
        let mut expected_rows = Vec::new();
        for row in 0..row_count {
            let value = if row % 29 == 0 { 42 } else { (row % 17) as u64 };
            if value == 42 {
                expected_rows.push(row as u32);
            }
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::AllValid { row_count };

        let (scalar, scalar_stats) = filter_u64_le_eq_dispatch(
            &bytes,
            row_count,
            validity,
            42,
            None,
            NativeKernelDispatch::Scalar,
        )
        .unwrap();
        let (auto, auto_stats) = filter_u64_le_eq_dispatch(
            &bytes,
            row_count,
            validity,
            42,
            None,
            NativeKernelDispatch::Auto,
        )
        .unwrap();

        assert_eq!(auto, scalar);
        assert_eq!(auto.to_selection_vector().rows(), expected_rows.as_slice());
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_fixed_width_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }

    #[test]
    fn u64_le_eq_dispatch_auto_falls_back_with_validity_or_base() {
        let values = [42u64, 7, 42, 13, 42, 19];
        let mut bytes = Vec::with_capacity(values.len() * std::mem::size_of::<u64>());
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0001_0000u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };
        let (with_validity, validity_stats) = filter_u64_le_eq_dispatch(
            &bytes,
            values.len(),
            validity,
            42,
            None,
            NativeKernelDispatch::Auto,
        )
        .unwrap();
        assert_eq!(with_validity.to_selection_vector().rows(), &[0, 2]);
        assert_eq!(validity_stats.dispatch, NativeKernelDispatch::Scalar);

        let mut base = SelectionBitmap::all(values.len());
        base.clear(0);
        let (with_base, base_stats) = filter_u64_le_eq_dispatch(
            &bytes,
            values.len(),
            ValidityRef::AllValid {
                row_count: values.len(),
            },
            42,
            Some(&base),
            NativeKernelDispatch::Auto,
        )
        .unwrap();
        assert_eq!(with_base.to_selection_vector().rows(), &[2, 4]);
        assert_eq!(base_stats.dispatch, NativeKernelDispatch::Scalar);
    }

    #[test]
    fn u32_le_in_filter_respects_base_and_validity() {
        let values = [10u32, 20, 30, 40];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0000_0100u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };
        let mut base = SelectionBitmap::all(values.len());
        base.clear(1);

        let (selected, stats) =
            filter_u32_le_in_sorted(&bytes, values.len(), validity, &[20, 30, 40], Some(&base))
                .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[3]);
        assert_eq!(stats.rows_valid, 2);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn u32_not_in_filter_respects_base_and_validity() {
        let values = [10u32, 20, 30, 40, 50];
        let nulls = [0b0000_0100u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };
        let mut base = SelectionBitmap::all(values.len());
        base.clear(4);

        let (selected, stats) = filter_u32_not_in_sorted(&values, validity, &[20, 40], Some(&base));

        assert_eq!(selected.to_selection_vector().rows(), &[0]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn direct_u32_in_and_not_in_auto_match_scalar_for_small_needles() {
        let row_count = 257;
        let values = (0..row_count)
            .map(|row| (row % 64) as u32)
            .collect::<Vec<_>>();
        let needles = [3u32, 7, 13, 31, 61];
        let validity = ValidityRef::AllValid { row_count };
        let base = SelectionBitmap::all(row_count);

        let (scalar_in, scalar_in_stats) =
            filter_u32_in_sorted(&values, validity, &needles, Some(&base));
        let (auto_in, auto_in_stats) = filter_u32_in_sorted(&values, validity, &needles, None);
        assert_eq!(auto_in, scalar_in);
        assert_eq!(scalar_in_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_local_membership_auto_dispatch(&auto_in_stats);
        assert_eq!(auto_in_stats.rows_seen, row_count);
        assert_eq!(auto_in_stats.rows_valid, row_count);
        assert_eq!(auto_in_stats.rows_matched, scalar_in_stats.rows_matched);

        let (scalar_not_in, scalar_not_in_stats) =
            filter_u32_not_in_sorted(&values, validity, &needles, Some(&base));
        let (auto_not_in, auto_not_in_stats) =
            filter_u32_not_in_sorted(&values, validity, &needles, None);
        assert_eq!(auto_not_in, scalar_not_in);
        assert_eq!(scalar_not_in_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_local_membership_auto_dispatch(&auto_not_in_stats);
        assert_eq!(auto_not_in_stats.rows_seen, row_count);
        assert_eq!(auto_not_in_stats.rows_valid, row_count);
        assert_eq!(
            auto_not_in_stats.rows_matched,
            scalar_not_in_stats.rows_matched
        );
    }

    #[test]
    fn u32_le_not_in_filter_respects_base_and_validity() {
        let values = [10u32, 20, 30, 40, 50];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0000_0100u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };
        let mut base = SelectionBitmap::all(values.len());
        base.clear(4);

        let (selected, stats) =
            filter_u32_le_not_in_sorted(&bytes, values.len(), validity, &[20, 40], Some(&base))
                .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn u32_le_single_needle_not_in_filter_respects_nulls_without_base() {
        let values = [10u32, 20, 30, 20, 50];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0000_0100u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };

        let (selected, stats) =
            filter_u32_le_not_in_sorted(&bytes, values.len(), validity, &[20], None).unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0, 4]);
        assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn u32_le_single_needle_not_in_filter_masks_tail_bits() {
        let row_count = 130;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u32>());
        for row in 0..row_count {
            bytes.extend_from_slice(&((row as u32) + 1).to_le_bytes());
        }

        let (selected, stats) = filter_u32_le_not_in_sorted(
            &bytes,
            row_count,
            ValidityRef::AllValid { row_count },
            &[999],
            None,
        )
        .unwrap();

        assert!(selected.all_set());
        assert_eq!(selected.count_ones(), row_count);
        assert_fixed_width_auto_dispatch(&stats);
        assert_eq!(stats.rows_matched, row_count);
    }

    #[test]
    fn u32_le_single_needle_not_in_filter_treats_missing_null_bytes_as_null() {
        let values = [10u32, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &[0],
            row_count: values.len(),
        };

        let (selected, stats) =
            filter_u32_le_not_in_sorted(&bytes, values.len(), validity, &[20], None).unwrap();

        assert_eq!(
            selected.to_selection_vector().rows(),
            &[0, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
        assert_eq!(stats.rows_valid, 8);
        assert_eq!(stats.rows_matched, 7);
    }

    #[test]
    fn u32_le_single_needle_in_filter_matches_scalar_eq_dispatch() {
        let row_count = 259;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u32>());
        let mut expected_rows = Vec::new();
        for row in 0..row_count {
            let value = if row % 31 == 0 { 99 } else { (row % 23) as u32 };
            if value == 99 {
                expected_rows.push(row as u32);
            }
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::AllValid { row_count };

        let (scalar, scalar_stats) = filter_u32_le_eq_dispatch(
            &bytes,
            row_count,
            validity,
            99,
            None,
            NativeKernelDispatch::Scalar,
        )
        .unwrap();
        let (auto_in, auto_stats) =
            filter_u32_le_in_sorted(&bytes, row_count, validity, &[99], None).unwrap();

        assert_eq!(auto_in, scalar);
        assert_eq!(
            auto_in.to_selection_vector().rows(),
            expected_rows.as_slice()
        );
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_fixed_width_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }

    #[test]
    fn u32_le_single_needle_not_in_filter_matches_scalar_complement_dispatch() {
        let row_count = 259;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u32>());
        let mut expected_rows = Vec::new();
        for row in 0..row_count {
            let value = if row % 31 == 0 { 99 } else { (row % 23) as u32 };
            if value != 99 {
                expected_rows.push(row as u32);
            }
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::AllValid { row_count };
        let base = SelectionBitmap::all(row_count);

        let (scalar, scalar_stats) =
            filter_u32_le_not_in_sorted(&bytes, row_count, validity, &[99], Some(&base)).unwrap();
        let (auto_not_in, auto_stats) =
            filter_u32_le_not_in_sorted(&bytes, row_count, validity, &[99], None).unwrap();

        assert_eq!(auto_not_in, scalar);
        assert_eq!(
            auto_not_in.to_selection_vector().rows(),
            expected_rows.as_slice()
        );
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_fixed_width_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }

    #[test]
    fn numcode_le_not_in_typed_filters_valid_complement_rows() {
        let values = [1u64, 2, 3, 4, 5];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0000_0100u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };
        let mut base = SelectionBitmap::all(values.len());
        base.clear(4);

        let (selected, stats) = filter_numcode_le_not_in_typed(
            &bytes,
            values.len(),
            validity,
            CoveLogicalType::UInt64,
            &[
                NativeNumericLiteral::UInt64(2),
                NativeNumericLiteral::UInt64(4),
            ],
            Some(&base),
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn i64_range_filter_handles_inclusive_and_exclusive_bounds() {
        let values = [1, 2, 3, 4, 5];
        let (selected, stats) = filter_i64_range(
            &values,
            ValidityRef::AllValid {
                row_count: values.len(),
            },
            Some((2, BoundInclusive::Exclusive)),
            Some((5, BoundInclusive::Exclusive)),
            None,
        );
        assert_eq!(selected.to_selection_vector().rows(), &[2, 3]);
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn i64_le_range_filter_handles_signed_bounds() {
        let values = [-3i64, -1, 0, 4, 9];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }

        let (selected, stats) = filter_i64_le_range(
            &bytes,
            values.len(),
            ValidityRef::AllValid {
                row_count: values.len(),
            },
            Some((-1, BoundInclusive::Inclusive)),
            Some((9, BoundInclusive::Exclusive)),
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[1, 2, 3]);
        assert_eq!(stats.rows_matched, 3);
    }

    #[test]
    fn i64_le_range_auto_matches_scalar_for_bound_variants() {
        let row_count = 257;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
        for row in 0..row_count {
            let value = match row {
                0 => i64::MIN,
                1 => i64::MAX,
                _ => (row as i64 % 53) - 26,
            };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::AllValid { row_count };
        let base = SelectionBitmap::all(row_count);

        for (lower, upper) in [
            (
                Some((-3, BoundInclusive::Inclusive)),
                Some((11, BoundInclusive::Exclusive)),
            ),
            (
                Some((-3, BoundInclusive::Exclusive)),
                Some((11, BoundInclusive::Inclusive)),
            ),
            (None, Some((0, BoundInclusive::Exclusive))),
            (Some((0, BoundInclusive::Inclusive)), None),
            (None, None),
        ] {
            let (scalar, scalar_stats) =
                filter_i64_le_range(&bytes, row_count, validity, lower, upper, Some(&base))
                    .unwrap();
            let (auto, auto_stats) =
                filter_i64_le_range(&bytes, row_count, validity, lower, upper, None).unwrap();

            assert_eq!(auto, scalar);
            assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
            assert_fixed_width_auto_dispatch(&auto_stats);
            assert_eq!(auto_stats.rows_seen, row_count);
            assert_eq!(auto_stats.rows_valid, row_count);
            assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
        }
    }

    #[test]
    fn typed_numcode_filter_preserves_integer_float_edges() {
        let values = [0u64, 1u64 << 53, (1u64 << 53) + 1, u64::MAX];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }

        let (selected, stats) = filter_numcode_le_typed(
            &bytes,
            values.len(),
            ValidityRef::AllValid {
                row_count: values.len(),
            },
            CoveLogicalType::UInt64,
            NativeNumericPredicateOp::Gt,
            NativeNumericLiteral::Float64((1u64 << 53) as f64),
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[2, 3]);
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn typed_numcode_int64_cmp_auto_matches_scalar_for_all_ops() {
        let row_count = 257;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
        for row in 0..row_count {
            let value = match row {
                0 => i64::MIN,
                1 => i64::MAX,
                _ => (row as i64 % 41) - 20,
            };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::AllValid { row_count };
        let base = SelectionBitmap::all(row_count);

        for op in [
            NativeNumericPredicateOp::Eq,
            NativeNumericPredicateOp::Lt,
            NativeNumericPredicateOp::LtEq,
            NativeNumericPredicateOp::Gt,
            NativeNumericPredicateOp::GtEq,
        ] {
            let (scalar, scalar_stats) = filter_numcode_le_typed(
                &bytes,
                row_count,
                validity,
                CoveLogicalType::Int64,
                op,
                NativeNumericLiteral::Int64(-3),
                Some(&base),
            )
            .unwrap();
            let (auto, auto_stats) = filter_numcode_le_typed(
                &bytes,
                row_count,
                validity,
                CoveLogicalType::Int64,
                op,
                NativeNumericLiteral::Int64(-3),
                None,
            )
            .unwrap();

            assert_eq!(auto, scalar);
            assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
            assert_fixed_width_auto_dispatch(&auto_stats);
            assert_eq!(auto_stats.rows_seen, row_count);
            assert_eq!(auto_stats.rows_valid, row_count);
            assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
        }
    }

    #[test]
    fn typed_numcode_in_filter_scans_once_with_base_and_validity() {
        let values = [5u64, 12, 20, 12];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0000_0010u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };
        let mut base = SelectionBitmap::all(values.len());
        base.clear(0);

        let (selected, stats) = filter_numcode_le_in_typed(
            &bytes,
            values.len(),
            validity,
            CoveLogicalType::UInt64,
            &[
                NativeNumericLiteral::UInt64(5),
                NativeNumericLiteral::UInt64(12),
            ],
            Some(&base),
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[3]);
        assert_eq!(stats.rows_seen, 4);
        assert_eq!(stats.rows_valid, 2);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn typed_numcode_single_literal_in_reuses_equality_dispatch() {
        let row_count = 257;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
        for row in 0..row_count {
            let value = if row % 37 == 0 {
                -11
            } else {
                (row as i64 % 19) - 9
            };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::AllValid { row_count };
        let literal = [NativeNumericLiteral::Int64(-11)];
        let base = SelectionBitmap::all(row_count);

        let (scalar, scalar_stats) = filter_numcode_le_in_typed(
            &bytes,
            row_count,
            validity,
            CoveLogicalType::Int64,
            &literal,
            Some(&base),
        )
        .unwrap();
        let (auto, auto_stats) = filter_numcode_le_in_typed(
            &bytes,
            row_count,
            validity,
            CoveLogicalType::Int64,
            &literal,
            None,
        )
        .unwrap();

        assert_eq!(auto, scalar);
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_fixed_width_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }

    #[test]
    fn typed_numcode_single_literal_not_in_reuses_equality_dispatch() {
        let row_count = 257;
        let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
        for row in 0..row_count {
            let value = if row % 37 == 0 {
                -11
            } else {
                (row as i64 % 19) - 9
            };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let validity = ValidityRef::AllValid { row_count };
        let literal = [NativeNumericLiteral::Int64(-11)];
        let base = SelectionBitmap::all(row_count);

        let (scalar, scalar_stats) = filter_numcode_le_not_in_typed(
            &bytes,
            row_count,
            validity,
            CoveLogicalType::Int64,
            &literal,
            Some(&base),
        )
        .unwrap();
        let (auto, auto_stats) = filter_numcode_le_not_in_typed(
            &bytes,
            row_count,
            validity,
            CoveLogicalType::Int64,
            &literal,
            None,
        )
        .unwrap();

        assert_eq!(auto, scalar);
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_fixed_width_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }

    #[test]
    fn typed_numcode_single_literal_not_in_excludes_null_rows() {
        let values = [1u64, 2, 3, 2, 5];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let nulls = [0b0000_0100u8];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        };

        let (selected, stats) = filter_numcode_le_not_in_typed(
            &bytes,
            values.len(),
            validity,
            CoveLogicalType::UInt64,
            &[NativeNumericLiteral::UInt64(2)],
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0, 4]);
        assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn local_u8_membership_translates_global_needles_once() {
        let local_to_global = [10, 20, 30, 40];
        let membership = local_membership_u8(&local_to_global, &[20, 40]);
        let values = [0, 1, 3, 2, 1];
        let (selected, stats) = filter_local_u8_membership(
            &values,
            ValidityRef::AllValid {
                row_count: values.len(),
            },
            &membership,
            None,
        );
        assert_eq!(selected.to_selection_vector().rows(), &[1, 2, 4]);
        assert_eq!(stats.rows_matched, 3);
    }

    #[test]
    fn local_u8_membership_auto_matches_scalar_for_small_membership() {
        let row_count = 257;
        let values = (0..row_count)
            .map(|row| (row % 16) as u8)
            .collect::<Vec<_>>();
        let mut membership = vec![false; 16];
        for local in [1usize, 3, 5, 8, 13] {
            membership[local] = true;
        }
        let validity = ValidityRef::AllValid { row_count };
        let base = SelectionBitmap::all(row_count);

        let (scalar, scalar_stats) =
            filter_local_u8_membership(&values, validity, &membership, Some(&base));
        let (auto, auto_stats) = filter_local_u8_membership(&values, validity, &membership, None);

        assert_eq!(auto, scalar);
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_local_membership_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }

    #[test]
    fn fixed_bytes_in_filter_respects_base_and_validity() {
        let values = [1u8, 10, 2, 20, 3, 30, 4, 40];
        let nulls = [0b0000_0100];
        let validity = ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: 4,
        };
        let mut base = SelectionBitmap::all(4);
        base.clear(0);
        let needles = [1u8, 10, 3, 30, 4, 40];

        let (selected, stats) =
            filter_fixed_bytes_in(&values, 4, 2, validity, &needles, Some(&base)).unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[3]);
        assert_eq!(stats.rows_seen, 4);
        assert_eq!(stats.rows_valid, 2);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn local_u16_and_u32_membership_use_local_codes_without_remap_per_row() {
        let local_to_global = [1, 3, 5, 7];
        let membership = local_membership_u8(&local_to_global, &[3, 7]);

        let values16 = [0u16, 1, 3, 2, 3];
        let (selected16, stats16) = filter_local_u16_membership(
            &values16,
            ValidityRef::AllValid {
                row_count: values16.len(),
            },
            &membership,
            None,
        );
        assert_eq!(selected16.to_selection_vector().rows(), &[1, 2, 4]);
        assert_eq!(stats16.rows_matched, 3);

        let values32 = [3u32, 2, 1, 0];
        let (selected32, stats32) = filter_local_u32_membership(
            &values32,
            ValidityRef::AllValid {
                row_count: values32.len(),
            },
            &membership,
            None,
        );
        assert_eq!(selected32.to_selection_vector().rows(), &[0, 2]);
        assert_eq!(stats32.rows_matched, 2);
    }

    #[test]
    fn local_u16_and_u32_membership_auto_matches_scalar_for_small_membership() {
        let row_count = 259;
        let values16 = (0..row_count)
            .map(|row| (row % 64) as u16)
            .collect::<Vec<_>>();
        let values32 = (0..row_count)
            .map(|row| (row % 64) as u32)
            .collect::<Vec<_>>();
        let mut membership = vec![false; 64];
        for local in [2usize, 7, 13, 31, 61] {
            membership[local] = true;
        }
        let validity = ValidityRef::AllValid { row_count };
        let base = SelectionBitmap::all(row_count);

        let (scalar16, scalar16_stats) =
            filter_local_u16_membership(&values16, validity, &membership, Some(&base));
        let (auto16, auto16_stats) =
            filter_local_u16_membership(&values16, validity, &membership, None);
        assert_eq!(auto16, scalar16);
        assert_eq!(scalar16_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_local_membership_auto_dispatch(&auto16_stats);
        assert_eq!(auto16_stats.rows_seen, row_count);
        assert_eq!(auto16_stats.rows_valid, row_count);
        assert_eq!(auto16_stats.rows_matched, scalar16_stats.rows_matched);

        let (scalar32, scalar32_stats) =
            filter_local_u32_membership(&values32, validity, &membership, Some(&base));
        let (auto32, auto32_stats) =
            filter_local_u32_membership(&values32, validity, &membership, None);
        assert_eq!(auto32, scalar32);
        assert_eq!(scalar32_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_local_membership_auto_dispatch(&auto32_stats);
        assert_eq!(auto32_stats.rows_seen, row_count);
        assert_eq!(auto32_stats.rows_valid, row_count);
        assert_eq!(auto32_stats.rows_matched, scalar32_stats.rows_matched);
    }

    #[test]
    fn bool_filter_reads_plain_fixed_bytes() {
        let values = [1u8, 0, 1, 1];
        let nulls = [0b0000_0100u8];
        let (selected, stats) = filter_bool_eq(
            &values,
            values.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            true,
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0, 3]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn bool_eq_auto_matches_scalar_on_all_valid_page() {
        let row_count = 257;
        let values = (0..row_count)
            .map(|row| if row % 7 == 0 || row % 11 == 0 { 1 } else { 0 })
            .collect::<Vec<_>>();
        let validity = ValidityRef::AllValid { row_count };
        let base = SelectionBitmap::all(row_count);

        let (scalar, scalar_stats) =
            filter_bool_eq(&values, row_count, validity, true, Some(&base)).unwrap();
        let (auto, auto_stats) = filter_bool_eq(&values, row_count, validity, true, None).unwrap();

        assert_eq!(auto, scalar);
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_local_membership_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }

    #[test]
    fn bool_eq_auto_rejects_invalid_all_valid_byte() {
        let values = [1u8, 0, 2];

        assert_eq!(
            filter_bool_eq(
                &values,
                values.len(),
                ValidityRef::AllValid {
                    row_count: values.len(),
                },
                true,
                None,
            )
            .unwrap_err(),
            CoveError::PageCorrupt
        );
    }

    #[test]
    fn fixed_bytes_filter_compares_width_sized_payloads() {
        let mut values = Vec::new();
        values.extend_from_slice(&[1u8; 16]);
        values.extend_from_slice(&[2u8; 16]);
        values.extend_from_slice(&[1u8; 16]);
        let mut base = SelectionBitmap::all(3);
        base.clear(0);

        let (selected, stats) = filter_fixed_bytes_eq(
            &values,
            3,
            16,
            ValidityRef::AllValid { row_count: 3 },
            &[1u8; 16],
            Some(&base),
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[2]);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn varbytes_filters_use_prepared_length_prefix_offsets() {
        let values =
            length_prefixed_rows([b"hi".as_slice(), b"bye".as_slice(), b"hill".as_slice()]);
        let row_offsets =
            prepare_varbytes_row_offsets(&values, 3, ValidityRef::AllValid { row_count: 3 })
                .unwrap();

        let (equals, eq_stats) = filter_varbytes_eq(
            &row_offsets,
            &values,
            ValidityRef::AllValid { row_count: 3 },
            b"bye",
            None,
        )
        .unwrap();
        assert_eq!(equals.to_selection_vector().rows(), &[1]);
        assert_eq!(eq_stats.rows_matched, 1);

        let needles = [b"hi".as_slice(), b"hill".as_slice()];
        let (included, in_stats) = filter_varbytes_in(
            &row_offsets,
            &values,
            ValidityRef::AllValid { row_count: 3 },
            &needles,
            None,
        )
        .unwrap();
        assert_eq!(included.to_selection_vector().rows(), &[0, 2]);
        assert_eq!(in_stats.rows_matched, 2);

        let (prefixed, prefix_stats) = filter_varbytes_prefix(
            &row_offsets,
            &values,
            ValidityRef::AllValid { row_count: 3 },
            b"hi",
            None,
        )
        .unwrap();
        assert_eq!(prefixed.to_selection_vector().rows(), &[0, 2]);
        assert_eq!(prefix_stats.rows_matched, 2);
    }

    #[test]
    fn length_prefixed_varbytes_eq_scans_without_offset_allocation() {
        let values = length_prefixed_rows([b"aa".as_slice(), b"bbb".as_slice(), b"aa".as_slice()]);
        let nulls = [0b0000_0100u8];

        let (selected, stats) = filter_length_prefixed_varbytes_eq(
            &values,
            3,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: 3,
            },
            b"aa",
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0]);
        assert_eq!(stats.rows_valid, 2);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn length_prefixed_varbytes_in_scans_without_offset_allocation() {
        let values = length_prefixed_rows([
            b"aa".as_slice(),
            b"bbb".as_slice(),
            b"cc".as_slice(),
            b"aa".as_slice(),
        ]);
        let nulls = [0b0000_0100u8];
        let mut base = SelectionBitmap::all(4);
        base.clear(0);
        let needles = [b"aa".as_slice(), b"cc".as_slice()];

        let (selected, stats) = filter_length_prefixed_varbytes_in(
            &values,
            4,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: 4,
            },
            &needles,
            Some(&base),
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[3]);
        assert_eq!(stats.rows_valid, 2);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn length_prefixed_varbytes_prefix_scans_without_offset_allocation() {
        let values =
            length_prefixed_rows([b"alpha".as_slice(), b"beta".as_slice(), b"alps".as_slice()]);
        let mut base = SelectionBitmap::all(3);
        base.clear(0);

        let (selected, stats) = filter_length_prefixed_varbytes_prefix(
            &values,
            3,
            ValidityRef::AllValid { row_count: 3 },
            b"al",
            Some(&base),
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[2]);
        assert_eq!(stats.rows_valid, 2);
        assert_eq!(stats.rows_matched, 1);
    }

    #[test]
    fn dense_u32_group_count_uses_local_codes_without_hashing() {
        let values = [0u32, 2, 1, 2, 0, 3, 2];
        let nulls = [0b0010_0000u8];
        let (groups, stats) = group_count_u32_dense(
            &values,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            4,
            None,
        )
        .unwrap();

        assert_eq!(groups.counts, vec![2, 1, 3, 0]);
        assert_eq!(groups.null_count, 1);
        assert_eq!(groups.rows_grouped, 6);
        assert_eq!(stats.rows_matched, 6);
    }

    #[test]
    fn dense_u8_and_u16_group_counts_use_local_codes_without_widening() {
        let values_u8 = [0u8, 2, 1, 2, 0, 3, 2];
        let values_u16 = [0u16, 2, 1, 2, 0, 3, 2];
        let nulls = [0b0010_0000u8];

        let (groups_u8, stats_u8) = group_count_u8_dense(
            &values_u8,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values_u8.len(),
            },
            4,
            None,
        )
        .unwrap();
        let (groups_u16, stats_u16) = group_count_u16_dense(
            &values_u16,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values_u16.len(),
            },
            4,
            None,
        )
        .unwrap();

        assert_eq!(groups_u8, groups_u16);
        assert_eq!(groups_u8.counts, vec![2, 1, 3, 0]);
        assert_eq!(groups_u8.null_count, 1);
        assert_eq!(groups_u8.rows_grouped, 6);
        assert_eq!(stats_u8.rows_matched, 6);
        assert_eq!(stats_u16.rows_matched, 6);
    }

    #[test]
    fn bool_group_count_uses_two_dense_buckets() {
        let values = [1u8, 0, 1, 1, 0, 1];
        let nulls = [0b0000_1000u8];
        let mut base = SelectionBitmap::none(values.len());
        base.set(0);
        base.set(1);
        base.set(3);
        base.set(4);
        base.set(5);

        let (groups, stats) = group_count_bool(
            &values,
            values.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(groups.counts, vec![2, 2]);
        assert_eq!(groups.null_count, 1);
        assert_eq!(groups.rows_grouped, 4);
        assert_eq!(stats.rows_seen, values.len());
        assert_eq!(stats.rows_matched, 4);
        assert_eq!(stats.bytes_touched_estimate, values.len());
    }

    #[test]
    fn bool_key_i64_aggregate_uses_dense_group_accumulators() {
        let keys = [1u8, 0, 1, 0, 1];
        let values = [10i64, 20, 30, 40, 50];
        let value_bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let key_nulls = [0b0001_0000u8];
        let value_nulls = [0b0000_0100u8];
        let mut base = SelectionBitmap::all(keys.len());
        base.clear(3);

        let (groups, stats) = aggregate_i64_le_bytes_by_bool(
            &keys,
            &value_bytes,
            keys.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &key_nulls,
                row_count: keys.len(),
            },
            ValidityRef::CoveNullBitmap {
                bytes: &value_nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(groups.row_counts, vec![1, 2]);
        assert_eq!(groups.aggregates[0].count, 1);
        assert_eq!(groups.aggregates[0].sum, 20);
        assert_eq!(groups.aggregates[1].count, 1);
        assert_eq!(groups.aggregates[1].null_count, 1);
        assert_eq!(groups.aggregates[1].sum, 10);
        assert_eq!(groups.null_row_count, 1);
        assert_eq!(groups.null_aggregate.count, 1);
        assert_eq!(groups.null_aggregate.sum, 50);
        assert_eq!(stats.rows_seen, keys.len());
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 4);
    }

    #[test]
    fn dense_u8_key_i64_aggregate_uses_local_code_accumulators() {
        let keys = [2u8, 0, 2, 1, 0];
        let values = [10i64, 20, 30, 40, 50];
        let value_bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let key_nulls = [0b0000_1000u8];
        let value_nulls = [0b0000_0100u8];
        let mut base = SelectionBitmap::all(keys.len());
        base.clear(4);

        let (groups, stats) = aggregate_i64_le_bytes_by_u8_dense(
            &keys,
            &value_bytes,
            keys.len(),
            3,
            ValidityRef::CoveNullBitmap {
                bytes: &key_nulls,
                row_count: keys.len(),
            },
            ValidityRef::CoveNullBitmap {
                bytes: &value_nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(groups.row_counts, vec![1, 0, 2]);
        assert_eq!(groups.aggregates[0].sum, 20);
        assert_eq!(groups.aggregates[2].count, 1);
        assert_eq!(groups.aggregates[2].null_count, 1);
        assert_eq!(groups.aggregates[2].sum, 10);
        assert_eq!(groups.null_row_count, 1);
        assert_eq!(groups.null_aggregate.sum, 40);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 4);
    }

    #[test]
    fn u32_key_i64_aggregate_hashes_codes_without_materializing_keys() {
        let keys = [7u32, 9, 7, 11, 9];
        let values = [10i64, 20, 30, 40, 50];
        let key_bytes = keys
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let value_bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let key_nulls = [0b0000_1000u8];
        let value_nulls = [0b0000_0100u8];
        let mut base = SelectionBitmap::all(keys.len());
        base.clear(4);

        let (groups, stats) = aggregate_i64_le_bytes_by_u32_le_bytes(
            &key_bytes,
            &value_bytes,
            keys.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &key_nulls,
                row_count: keys.len(),
            },
            ValidityRef::CoveNullBitmap {
                bytes: &value_nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(groups.row_counts.get(&7), Some(&2));
        assert_eq!(groups.aggregates.get(&7).unwrap().count, 1);
        assert_eq!(groups.aggregates.get(&7).unwrap().null_count, 1);
        assert_eq!(groups.aggregates.get(&7).unwrap().sum, 10);
        assert_eq!(groups.row_counts.get(&9), Some(&1));
        assert_eq!(groups.aggregates.get(&9).unwrap().sum, 20);
        assert_eq!(groups.null_row_count, 1);
        assert_eq!(groups.null_aggregate.sum, 40);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 4);
    }

    #[test]
    fn i64_key_i64_aggregate_hashes_numeric_keys_without_payload_materialization() {
        let keys = [7i64, -2, 7, 11, -2];
        let values = [10i64, 20, 30, 40, 50];
        let key_bytes = keys
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let value_bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let key_nulls = [0b0000_1000u8];
        let value_nulls = [0b0000_0100u8];
        let mut base = SelectionBitmap::all(keys.len());
        base.clear(4);

        let (groups, stats) = aggregate_i64_le_bytes_by_i64_le_bytes(
            &key_bytes,
            &value_bytes,
            keys.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &key_nulls,
                row_count: keys.len(),
            },
            ValidityRef::CoveNullBitmap {
                bytes: &value_nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(groups.row_counts.get(&7), Some(&2));
        assert_eq!(groups.aggregates.get(&7).unwrap().count, 1);
        assert_eq!(groups.aggregates.get(&7).unwrap().null_count, 1);
        assert_eq!(groups.aggregates.get(&7).unwrap().sum, 10);
        assert_eq!(groups.row_counts.get(&-2), Some(&1));
        assert_eq!(groups.aggregates.get(&-2).unwrap().sum, 20);
        assert_eq!(groups.null_row_count, 1);
        assert_eq!(groups.null_aggregate.sum, 40);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 4);
    }

    #[test]
    fn hash_u32_group_count_handles_sparse_codes() {
        let values = [100u32, 7, 100, 42, 7, 9];
        let nulls = [0b0010_0000u8];
        let (groups, stats) = group_count_u32_hash(
            &values,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            None,
        )
        .unwrap();

        assert_eq!(groups.counts.get(&100), Some(&2));
        assert_eq!(groups.counts.get(&7), Some(&2));
        assert_eq!(groups.counts.get(&42), Some(&1));
        assert_eq!(groups.counts.get(&9), None);
        assert_eq!(groups.null_count, 1);
        assert_eq!(groups.rows_grouped, 5);
        assert_eq!(stats.rows_matched, 5);
    }

    #[test]
    fn u32_le_group_count_handles_file_codes_without_materialization() {
        let values = [100u32, 7, 100, 42, 7, 9];
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let nulls = [0b0010_0000u8];
        let mut base = SelectionBitmap::all(values.len());
        base.clear(3);

        let (groups, stats) = group_count_u32_le_bytes(
            &bytes,
            values.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(groups.counts.get(&100), Some(&2));
        assert_eq!(groups.counts.get(&7), Some(&2));
        assert_eq!(groups.counts.get(&42), None);
        assert_eq!(groups.counts.get(&9), None);
        assert_eq!(groups.null_count, 1);
        assert_eq!(groups.rows_grouped, 4);
        assert_eq!(stats.rows_matched, 4);
    }

    #[test]
    fn i64_le_group_count_handles_sparse_values_nulls_and_base_selection() {
        let values = [10i64, -2, 10, 7, -2, 99];
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let nulls = [0b0010_0000u8];
        let mut base = SelectionBitmap::all(values.len());
        base.clear(3);

        let (groups, stats) = group_count_i64_le_bytes(
            &bytes,
            values.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(groups.counts.get(&10), Some(&2));
        assert_eq!(groups.counts.get(&-2), Some(&2));
        assert_eq!(groups.counts.get(&7), None);
        assert_eq!(groups.counts.get(&99), None);
        assert_eq!(groups.null_count, 1);
        assert_eq!(groups.rows_grouped, 4);
        assert_eq!(stats.rows_matched, 4);
    }

    #[test]
    fn distinct_u32_preserves_first_seen_order_and_validity() {
        let values = [3u32, 1, 3, 2, 1, 4];
        let nulls = [0b0000_1000u8];
        let mut base = SelectionBitmap::all(values.len());
        base.clear(5);

        let (distinct, stats) = distinct_u32(
            &values,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(distinct, vec![3, 1]);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn i64_aggregates_preserve_wide_sum_and_nulls() {
        let values = [i64::MAX, -2, 5, 7];
        let nulls = [0b0000_0100u8];
        let (aggregates, stats) = aggregate_i64(
            &values,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            None,
        );

        assert_eq!(aggregates.count, 3);
        assert_eq!(aggregates.null_count, 1);
        assert_eq!(aggregates.sum, i128::from(i64::MAX) - 2 + 7);
        assert_eq!(aggregates.min, Some(-2));
        assert_eq!(aggregates.max, Some(i64::MAX));
        assert_eq!(stats.rows_valid, 3);
    }

    #[test]
    fn i64_le_byte_aggregates_match_typed_path_with_base_selection() {
        let values = [i64::MAX, -2, 5, 7];
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let nulls = [0b0000_0100u8];
        let mut base = SelectionBitmap::all(values.len());
        base.clear(3);

        let (aggregates, stats) = aggregate_i64_le_bytes(
            &bytes,
            values.len(),
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            Some(&base),
        )
        .unwrap();

        assert_eq!(aggregates.count, 2);
        assert_eq!(aggregates.null_count, 1);
        assert_eq!(aggregates.sum, i128::from(i64::MAX) - 2);
        assert_eq!(aggregates.min, Some(-2));
        assert_eq!(aggregates.max, Some(i64::MAX));
        assert_eq!(stats.rows_seen, values.len());
        assert_eq!(stats.rows_matched, 2);
    }

    #[test]
    fn i64_sort_orders_row_ids_not_payloads() {
        let values = [5, 1, 5, 0];
        let nulls = [0b0000_0100u8];
        let (rows, stats) = sort_rows_i64_with_stats(
            &values,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            NativeSortDirection::Ascending,
            NativeNullOrder::Last,
            None,
        )
        .unwrap();

        assert_eq!(rows.rows(), &[3, 1, 0, 2]);
        assert_eq!(stats.rows_seen, 4);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 4);
    }

    #[test]
    fn i64_top_n_matches_full_row_id_sort_prefix() {
        let values = [5, -1, 5, 0, 9, -1, 4];
        let nulls = [0b0100_0000u8];
        let (full, _) = sort_rows_i64_with_stats(
            &values,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            NativeSortDirection::Descending,
            NativeNullOrder::Last,
            None,
        )
        .unwrap();
        let (top, stats) = top_n_rows_i64_with_stats(
            &values,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: values.len(),
            },
            NativeSortDirection::Descending,
            NativeNullOrder::Last,
            None,
            3,
        )
        .unwrap();

        assert_eq!(top.rows(), &full.rows()[..3]);
        assert_eq!(top.rows(), &[4, 2, 0]);
        assert_eq!(stats.rows_seen, values.len());
        assert_eq!(stats.rows_valid, 6);
        assert_eq!(stats.rows_matched, values.len());
    }

    #[test]
    fn u32_semi_join_requires_compatible_code_domain() {
        let left = [1u32, 2, 3, 4];
        let right = [2u32, 4, 8];
        let mut left_domain = NativeCodeDomain {
            semantic_domain_id: Some("customer-status".into()),
            dictionary_epoch: Some(7),
            ..NativeCodeDomain::default()
        };
        let right_domain = left_domain.clone();
        let (selected, stats) = semi_join_u32_eq(
            &left,
            ValidityRef::AllValid {
                row_count: left.len(),
            },
            &left_domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &right_domain,
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[1, 3]);
        assert_eq!(stats.rows_matched, 2);

        left_domain.dictionary_epoch = Some(8);
        assert!(semi_join_u32_eq(
            &left,
            ValidityRef::AllValid {
                row_count: left.len(),
            },
            &left_domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &right_domain,
            None,
        )
        .is_err());
    }

    #[test]
    fn u32_anti_join_uses_same_domain_and_skips_nulls() {
        let left = [1u32, 2, 3, 4, 5];
        let right = [2u32, 4, 8];
        let nulls = [0b0000_0100u8];
        let domain = NativeCodeDomain {
            semantic_domain_id: Some("customer-status".into()),
            dictionary_epoch: Some(7),
            ..NativeCodeDomain::default()
        };
        let (selected, stats) = anti_join_u32_eq(
            &left,
            ValidityRef::CoveNullBitmap {
                bytes: &nulls,
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &domain,
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0, 4]);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 2);

        let other_domain = NativeCodeDomain {
            dictionary_epoch: Some(8),
            ..domain.clone()
        };
        assert!(anti_join_u32_eq(
            &left,
            ValidityRef::AllValid {
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &other_domain,
            None,
        )
        .is_err());
    }

    #[test]
    fn u32_inner_join_emits_row_pairs_and_rejects_domain_mismatch() {
        let left = [1u32, 2, 2, 3, 4];
        let right = [2u32, 2, 3, 8];
        let left_nulls = [0b0000_1000u8];
        let mut base = SelectionBitmap::all(left.len());
        base.clear(4);
        let domain = NativeCodeDomain {
            semantic_domain_id: Some("customer-status".into()),
            dictionary_epoch: Some(7),
            ..NativeCodeDomain::default()
        };

        let (pairs, stats) = inner_join_u32_eq(
            &left,
            ValidityRef::CoveNullBitmap {
                bytes: &left_nulls,
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &domain,
            Some(&base),
        )
        .unwrap();

        assert_eq!(pairs.left_rows, vec![1, 1, 2, 2]);
        assert_eq!(pairs.right_rows, vec![0, 1, 0, 1]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 4);

        let other_domain = NativeCodeDomain {
            dictionary_epoch: Some(8),
            ..domain.clone()
        };
        assert!(inner_join_u32_eq(
            &left,
            ValidityRef::AllValid {
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &other_domain,
            None,
        )
        .is_err());
    }

    #[test]
    fn i64_join_kernels_handle_negative_duplicates_and_left_null_policy() {
        let left = [-7i64, 2, -7, 9, 0];
        let right = [-7i64, -7, 4];
        let left_nulls = [0b0001_0000u8];
        let domain = NativeCodeDomain {
            semantic_domain_id: Some("cove.datafusion.native.i64".into()),
            null_policy: Some("validity-bitmap-nulls-never-match".into()),
            ..NativeCodeDomain::default()
        };

        let (pairs, inner_stats) = inner_join_i64_eq(
            &left,
            ValidityRef::CoveNullBitmap {
                bytes: &left_nulls,
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &domain,
            None,
        )
        .unwrap();

        assert_eq!(pairs.left_rows, vec![0, 0, 2, 2]);
        assert_eq!(pairs.right_rows, vec![0, 1, 0, 1]);
        assert_eq!(inner_stats.rows_valid, 4);
        assert_eq!(inner_stats.rows_matched, 4);

        let (semi, semi_stats) = semi_join_i64_eq(
            &left,
            ValidityRef::CoveNullBitmap {
                bytes: &left_nulls,
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &domain,
            None,
        )
        .unwrap();
        assert_eq!(semi.to_selection_vector().rows(), &[0, 2]);
        assert_eq!(semi_stats.rows_matched, 2);

        let (anti, anti_stats) = anti_join_i64_eq_left_nulls_unmatched(
            &left,
            ValidityRef::CoveNullBitmap {
                bytes: &left_nulls,
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &domain,
            None,
        )
        .unwrap();
        assert_eq!(anti.to_selection_vector().rows(), &[1, 3, 4]);
        assert_eq!(anti_stats.rows_matched, 3);
    }

    #[test]
    fn u64_semi_join_uses_shared_domain_and_base_selection() {
        let left = [10u64, 20, 30, 40, 50];
        let right = [20u64, 50, 90];
        let mut base = SelectionBitmap::all(left.len());
        base.clear(4);
        let domain = NativeCodeDomain {
            dictionary_id: Some("file-dict".into()),
            dictionary_epoch: Some(3),
            collation_id: Some(1),
            ..NativeCodeDomain::default()
        };

        let (selected, stats) = semi_join_u64_eq(
            &left,
            ValidityRef::AllValid {
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &domain,
            Some(&base),
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[1]);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 1);
        assert_eq!(
            stats.bytes_touched_estimate,
            (left.len() + right.len()) * std::mem::size_of::<u64>()
        );
    }

    #[test]
    fn u64_anti_join_skips_nulls_and_rejects_epoch_mismatch() {
        let left = [10u64, 20, 30, 40, 50];
        let right = [20u64, 50, 90];
        let left_nulls = [0b0000_0100u8];
        let right_nulls = [0b0000_0010u8];
        let domain = NativeCodeDomain {
            dictionary_id: Some("file-dict".into()),
            dictionary_epoch: Some(3),
            collation_id: Some(1),
            ..NativeCodeDomain::default()
        };

        let (selected, stats) = anti_join_u64_eq(
            &left,
            ValidityRef::CoveNullBitmap {
                bytes: &left_nulls,
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::CoveNullBitmap {
                bytes: &right_nulls,
                row_count: right.len(),
            },
            &domain,
            None,
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[0, 3, 4]);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 3);

        let other_domain = NativeCodeDomain {
            dictionary_epoch: Some(4),
            ..domain.clone()
        };
        assert!(anti_join_u64_eq(
            &left,
            ValidityRef::AllValid {
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::AllValid {
                row_count: right.len(),
            },
            &other_domain,
            None,
        )
        .is_err());
    }

    #[test]
    fn u64_anti_join_can_treat_left_nulls_as_unmatched() {
        let left = [10u64, 20, 30, 40, 50];
        let right = [20u64, 50, 90];
        let left_nulls = [0b0000_0100u8];
        let right_nulls = [0b0000_0010u8];
        let mut base = SelectionBitmap::all(left.len());
        base.clear(0);
        let domain = NativeCodeDomain {
            dictionary_id: Some("file-dict".into()),
            dictionary_epoch: Some(3),
            collation_id: Some(1),
            ..NativeCodeDomain::default()
        };

        let (selected, stats) = anti_join_u64_eq_left_nulls_unmatched(
            &left,
            ValidityRef::CoveNullBitmap {
                bytes: &left_nulls,
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::CoveNullBitmap {
                bytes: &right_nulls,
                row_count: right.len(),
            },
            &domain,
            Some(&base),
        )
        .unwrap();

        assert_eq!(selected.to_selection_vector().rows(), &[2, 3, 4]);
        assert_eq!(stats.rows_valid, 3);
        assert_eq!(stats.rows_matched, 3);
    }

    #[test]
    fn u64_inner_join_preserves_left_order_and_skips_right_nulls() {
        let left = [10u64, 20, 20, 30, 40];
        let right = [20u64, 20, 30, 40];
        let right_nulls = [0b0000_0100u8];
        let mut base = SelectionBitmap::all(left.len());
        base.clear(4);
        let domain = NativeCodeDomain {
            dictionary_id: Some("file-dict".into()),
            dictionary_epoch: Some(3),
            collation_id: Some(1),
            ..NativeCodeDomain::default()
        };

        let (pairs, stats) = inner_join_u64_eq(
            &left,
            ValidityRef::AllValid {
                row_count: left.len(),
            },
            &domain,
            &right,
            ValidityRef::CoveNullBitmap {
                bytes: &right_nulls,
                row_count: right.len(),
            },
            &domain,
            Some(&base),
        )
        .unwrap();

        assert_eq!(pairs.left_rows, vec![1, 1, 2, 2]);
        assert_eq!(pairs.right_rows, vec![0, 1, 0, 1]);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 4);
        assert!(stats.bytes_touched_estimate >= (left.len() + right.len()) * 8);
    }

    #[test]
    fn mutable_bitmap_operations_reuse_dense_words() {
        let mut selected = SelectionBitmap::default();
        selected.fill_all(70);
        selected.clear_bit(1);
        selected.clear_bit(64);

        let mut filter = SelectionBitmap::default();
        filter.fill_none(70);
        filter.set(0);
        filter.set(65);
        filter.set(69);

        selected.and_inplace(&filter);

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![0, 65, 69]);
        assert!(!selected.all_zero());
        assert_eq!(selected.words().len(), 2);
    }

    #[test]
    fn bitmap_intersection_dispatch_matches_scalar() {
        let mut scalar = SelectionBitmap::all(4097);
        let mut auto = scalar.clone();
        let mut filter = SelectionBitmap::none(4097);
        for row in (0..4097).step_by(3) {
            filter.set(row);
        }
        for row in (0..4097).step_by(11) {
            scalar.clear(row);
            auto.clear(row);
        }

        scalar.intersect_with(&filter);
        let dispatch = auto.intersect_with_dispatch(&filter, NativeKernelDispatch::Auto);

        assert_eq!(auto, scalar);
        #[cfg(target_arch = "aarch64")]
        assert_eq!(dispatch, NativeKernelDispatch::Neon);
        #[cfg(not(target_arch = "aarch64"))]
        assert!(matches!(
            dispatch,
            NativeKernelDispatch::Scalar | NativeKernelDispatch::Avx2
        ));
    }

    #[test]
    fn bitmap_intersection_scalar_policy_reports_scalar() {
        let mut left = SelectionBitmap::all(130);
        let mut right = SelectionBitmap::none(130);
        right.set(3);
        right.set(64);
        right.set(129);

        let dispatch = left.intersect_with_dispatch(&right, NativeKernelDispatch::Scalar);

        assert_eq!(dispatch, NativeKernelDispatch::Scalar);
        assert_eq!(left.to_selection_vector().rows(), &[3, 64, 129]);
    }

    #[test]
    fn selection_bitmap_compaction_reports_stats_and_preserves_order() {
        let mut bitmap = SelectionBitmap::none(130);
        bitmap.set(0);
        bitmap.set(63);
        bitmap.set(64);
        bitmap.set(129);

        let (vector, stats) =
            compact_selection_bitmap(&bitmap, NativeKernelDispatch::Auto).unwrap();

        assert_eq!(vector.rows(), &[0, 63, 64, 129]);
        assert_eq!(stats.rows_seen, 130);
        assert_eq!(stats.rows_valid, 4);
        assert_eq!(stats.rows_matched, 4);
        assert_eq!(stats.bitmap_words_touched, 3);
        assert_eq!(stats.bytes_touched_estimate, 3 * std::mem::size_of::<u64>());
        assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
    }

    #[test]
    fn retain_set_bits_visits_only_selected_rows() {
        let mut bitmap = SelectionBitmap::none(130);
        bitmap.set(3);
        bitmap.set(64);
        bitmap.set(129);
        let mut visited = Vec::new();

        bitmap.retain_set_bits(|row| {
            visited.push(row);
            row != 64
        });

        assert_eq!(visited, vec![3, 64, 129]);
        assert_eq!(bitmap.to_selection_vector().rows(), &[3, 129]);
    }

    #[test]
    fn scan_program_summary_reports_exactness_and_ordering() {
        let program = NativeScanProgram {
            ops: vec![NativeScanOp::Numeric {
                column_index: 0,
                column_id: 7,
                exactness: NativePredicateExactness::FullRowPredicateExact,
                kernel: NativeDecodeKernel::PreparedNumCode,
            }],
            exact_filters: 1,
            inexact_filters: 0,
            lookup_rowref_eligible: true,
            predicate_ordered: true,
        };

        assert_eq!(
            program.display_summary(),
            "ops=1, exact_filters=1, inexact_filters=0, lookup_rowref_eligible=true, predicate_ordered=true"
        );
    }

    #[test]
    fn retained_object_temporal_batch_exposes_filecode_page_lane() {
        let codes = [11u32, 22, 33];
        let mut values = Vec::new();
        for code in codes {
            values.extend_from_slice(&code.to_le_bytes());
        }
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::FileCode,
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            Some(vec![0b0000_0010]),
            values,
        )
        .unwrap();
        let payload =
            RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
        let segment = retained_segment_with_one_property_page(
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            payload,
            1,
        );

        let batch = native_object_temporal_batch_from_retained_segment(
            &segment,
            NativeCodeDomain {
                file_id: Some("file-a".into()),
                ..NativeCodeDomain::default()
            },
        )
        .unwrap();

        assert_eq!(batch.segment_id, 9);
        assert_eq!(batch.object_type_id, 7);
        assert_eq!(batch.row_count, 3);
        assert_eq!(batch.property_pages.len(), 1);
        let page = &batch.property_pages[0];
        assert_eq!(page.property_id, 42);
        assert_eq!(page.row_start, 0);
        match &page.lane {
            LaneRef::FileCodeU32LeBytes {
                bytes,
                row_count,
                validity,
                domain,
                ..
            } => {
                assert_eq!(*row_count, 3);
                assert_eq!(validity.valid_count(), 2);
                assert_eq!(domain.file_id.as_deref(), Some("file-a"));
                assert_eq!(domain.object_type_id, Some(7));
                assert_eq!(domain.property_id, Some(42));

                let (selected, stats) =
                    filter_u32_le_in_sorted(bytes, *row_count, *validity, &[22, 33], None).unwrap();
                assert_eq!(selected.to_selection_vector().rows(), &[2]);
                assert_eq!(stats.rows_matched, 1);
            }
            other => panic!("expected FileCodeU32LeBytes lane, got {other:?}"),
        }
    }

    #[test]
    fn owned_object_temporal_batch_exposes_numcode_page_lane() {
        let values = [10u64, 20, 30];
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::NumCode,
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            None,
            bytes,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload_bytes).unwrap();
        let segment = temporal_segment_with_one_property_page(
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            payload,
            0,
        );

        let batch = native_object_temporal_batch_from_segment(
            &segment,
            NativeCodeDomain {
                semantic_domain_id: Some("domain-a".into()),
                ..NativeCodeDomain::default()
            },
        )
        .unwrap();

        match &batch.property_pages[0].lane {
            LaneRef::NumCodeU64LeBytes {
                bytes,
                row_count,
                validity,
                domain,
                ..
            } => {
                assert_eq!(*row_count, 3);
                assert_eq!(domain.semantic_domain_id.as_deref(), Some("domain-a"));
                let (selected, stats) =
                    filter_u64_le_eq(bytes, *row_count, *validity, 20, None).unwrap();
                assert_eq!(selected.to_selection_vector().rows(), &[1]);
                assert_eq!(stats.rows_matched, 1);
            }
            other => panic!("expected NumCodeU64LeBytes lane, got {other:?}"),
        }
    }

    #[test]
    fn retained_object_temporal_batch_exposes_bool_page_lane() {
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            Some(vec![0b0000_0010]),
            vec![1, 0, 1],
        )
        .unwrap();
        let payload =
            RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
        let segment = retained_segment_with_one_property_page(
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            payload,
            1,
        );

        let batch = native_object_temporal_batch_from_retained_segment(
            &segment,
            NativeCodeDomain::default(),
        )
        .unwrap();

        match &batch.property_pages[0].lane {
            LaneRef::Bool {
                values,
                row_count,
                validity,
                ..
            } => {
                assert_eq!(*row_count, 3);
                let (selected, stats) =
                    filter_bool_eq(values, *row_count, *validity, true, None).unwrap();
                assert_eq!(selected.to_selection_vector().rows(), &[0, 2]);
                assert_eq!(stats.rows_valid, 2);
            }
            other => panic!("expected Bool lane, got {other:?}"),
        }
    }

    #[test]
    fn retained_object_temporal_batch_exposes_fixed_bytes_page_lane() {
        let mut values = Vec::new();
        values.extend_from_slice(&[1u8; 16]);
        values.extend_from_slice(&[2u8; 16]);
        values.extend_from_slice(&[1u8; 16]);
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Uuid,
            CovePhysicalKind::FixedBytes,
            None,
            values,
        )
        .unwrap();
        let payload =
            RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
        let segment = retained_segment_with_one_property_page(
            CoveLogicalType::Uuid,
            CovePhysicalKind::FixedBytes,
            payload,
            0,
        );

        let batch = native_object_temporal_batch_from_retained_segment(
            &segment,
            NativeCodeDomain::default(),
        )
        .unwrap();

        match &batch.property_pages[0].lane {
            LaneRef::FixedBytes {
                values,
                width,
                row_count,
                validity,
                ..
            } => {
                assert_eq!((*width, *row_count), (16, 3));
                let (selected, stats) =
                    filter_fixed_bytes_eq(values, *row_count, *width, *validity, &[1u8; 16], None)
                        .unwrap();
                assert_eq!(selected.to_selection_vector().rows(), &[0, 2]);
                assert_eq!(stats.rows_matched, 2);
            }
            other => panic!("expected FixedBytes lane, got {other:?}"),
        }
    }

    #[test]
    fn retained_object_temporal_batch_exposes_varbytes_page_lane() {
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::VarBytes,
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            None,
            length_prefixed_rows([b"red".as_slice(), b"blue".as_slice(), b"rose".as_slice()]),
        )
        .unwrap();
        let payload =
            RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
        let segment = retained_segment_with_one_property_page(
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            payload,
            0,
        );

        let batch = native_object_temporal_batch_from_retained_segment(
            &segment,
            NativeCodeDomain::default(),
        )
        .unwrap();

        match &batch.property_pages[0].lane {
            LaneRef::VarBytes {
                row_offsets,
                values,
                validity,
                ..
            } => {
                assert_eq!(row_offsets.as_ref(), &[0, 7, 15]);
                let (selected, stats) =
                    filter_varbytes_prefix(row_offsets, values, *validity, b"r", None).unwrap();
                assert_eq!(selected.to_selection_vector().rows(), &[0, 2]);
                assert_eq!(stats.rows_matched, 2);
            }
            other => panic!("expected VarBytes lane, got {other:?}"),
        }
    }

    #[test]
    fn retained_object_temporal_batch_exposes_local_codebook_filecode_lane() {
        let local_codebook = LocalCodebookPayload {
            values: LocalCodebookValues::FileCode(vec![10, 20, 30]),
            indexes: LocalIndexPayload::BitPacked(BitPackedPayload::pack(&[0, 1, 2], 2).unwrap()),
        };
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::LocalCodebook,
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            None,
            local_codebook.encode(),
        )
        .unwrap();
        let payload =
            RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
        let segment = retained_segment_with_one_property_page(
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            payload,
            0,
        );

        let batch = native_object_temporal_batch_from_retained_segment(
            &segment,
            NativeCodeDomain::default(),
        )
        .unwrap();

        match &batch.property_pages[0].lane {
            LaneRef::LocalCodeU8 {
                values,
                validity,
                local_to_global,
                ..
            } => {
                assert_eq!(values.as_ref(), &[0, 1, 2]);
                assert_eq!(local_to_global.as_ref(), &[10, 20, 30]);
                let membership = local_membership_u8(local_to_global, &[20]);
                let (selected, stats) =
                    filter_local_u8_membership(values, *validity, &membership, None);
                assert_eq!(selected.to_selection_vector().rows(), &[1]);
                assert_eq!(stats.rows_matched, 1);
            }
            other => panic!("expected LocalCodeU8 lane, got {other:?}"),
        }
    }

    #[test]
    fn table_batch_exposes_borrowed_numcode_page_lane() {
        let mut values = Vec::new();
        for value in [11u64, 22, 33] {
            values.extend_from_slice(&value.to_le_bytes());
        }
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::NumCode,
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload_bytes).unwrap();
        let mut column = table_column_directory(CoveLogicalType::UInt64, CovePhysicalKind::NumCode);
        column.domain_ref = 17;
        let segment = table_segment_with_one_column(column);
        let page = table_page_index_entry(42, 5, 3, 0);
        let page_ref = NativeTablePageRef {
            column: &segment.columns[0],
            page: &page,
            payload: Some(&payload),
        };

        let batch =
            native_table_batch_from_page_refs(&segment, &[page_ref], NativeCodeDomain::default())
                .unwrap();

        assert_eq!(batch.table_id, 77);
        assert_eq!(batch.segment_id, 88);
        assert_eq!(batch.row_start, 100);
        assert_eq!(batch.row_count, 3);
        assert_eq!(batch.column_pages[0].row_start, 100);
        assert_eq!(batch.column_pages[0].row_start_in_segment, 0);
        match &batch.column_pages[0].lane {
            LaneRef::NumCodeU64LeBytes {
                bytes,
                row_count,
                validity,
                domain,
                ..
            } => {
                assert_eq!(*row_count, 3);
                assert_eq!(bytes.len(), 24);
                assert_eq!(validity.row_count(), 3);
                assert_eq!(domain.table_id, Some(77));
                assert_eq!(domain.column_id, Some(42));
                assert_eq!(
                    domain.semantic_domain_id.as_deref(),
                    Some("table-domain:17")
                );
            }
            other => panic!("expected NumCodeU64LeBytes lane, got {other:?}"),
        }
    }

    #[test]
    fn direct_column_page_helper_exposes_borrowed_numcode_lane() {
        let mut values = Vec::new();
        for value in [11u64, 22, 33] {
            values.extend_from_slice(&value.to_le_bytes());
        }
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::NumCode,
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            None,
            values,
        )
        .unwrap();
        let payload = ColumnPagePayloadV1::parse(&payload_bytes).unwrap();
        let mut column = table_column_directory(CoveLogicalType::UInt64, CovePhysicalKind::NumCode);
        column.domain_ref = 17;
        let segment = table_segment_with_one_column(column);
        let page = table_page_index_entry(42, 5, 3, 0);
        let domain = NativeCodeDomain {
            table_id: Some(segment.header.table_id),
            ..NativeCodeDomain::default()
        };

        let lane =
            native_lane_from_column_page_payload(&segment.columns[0], &page, &payload, domain)
                .unwrap();

        match lane {
            LaneRef::NumCodeU64LeBytes {
                bytes,
                row_count,
                validity,
                domain,
                ..
            } => {
                assert_eq!(row_count, 3);
                assert_eq!(bytes.len(), 24);
                assert_eq!(validity.row_count(), 3);
                assert_eq!(domain.table_id, Some(77));
                assert_eq!(domain.column_id, Some(42));
                assert_eq!(
                    domain.semantic_domain_id.as_deref(),
                    Some("table-domain:17")
                );
            }
            other => panic!("expected NumCodeU64LeBytes lane, got {other:?}"),
        }
    }

    #[test]
    fn table_batch_exposes_retained_local_codebook_filecode_lane() {
        let local_codebook = LocalCodebookPayload {
            values: LocalCodebookValues::FileCode(vec![7, 9, 11]),
            indexes: LocalIndexPayload::BitPacked(BitPackedPayload::pack(&[2, 0, 1], 2).unwrap()),
        };
        let payload_bytes = ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::LocalCodebook,
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            None,
            local_codebook.encode(),
        )
        .unwrap();
        let payload =
            RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
        let column = table_column_directory(CoveLogicalType::Utf8, CovePhysicalKind::FileCode);
        let segment = table_segment_with_one_column(column);
        let page = table_page_index_entry(42, 5, 3, 0);
        let page_ref = NativeTablePageRef {
            column: &segment.columns[0],
            page: &page,
            payload: Some(&payload),
        };

        let batch =
            native_table_batch_from_page_refs(&segment, &[page_ref], NativeCodeDomain::default())
                .unwrap();

        match &batch.column_pages[0].lane {
            LaneRef::LocalCodeU8 {
                values,
                local_to_global,
                validity,
                ..
            } => {
                assert_eq!(values.as_ref(), &[2, 0, 1]);
                assert_eq!(local_to_global.as_ref(), &[7, 9, 11]);
                let membership = local_membership_u8(local_to_global, &[11]);
                let (selected, stats) =
                    filter_local_u8_membership(values, *validity, &membership, None);
                assert_eq!(selected.to_selection_vector().rows(), &[0]);
                assert_eq!(stats.rows_matched, 1);
            }
            other => panic!("expected LocalCodeU8 lane, got {other:?}"),
        }
    }

    fn temporal_segment_with_one_property_page(
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        payload: ColumnPagePayloadV1,
        null_count: u32,
    ) -> TemporalSegmentData {
        TemporalSegmentData {
            header: TemporalSegmentHeaderV1 {
                segment_id: 9,
                object_type_id: 7,
                time_range_start_us: 1,
                time_range_end_us: 3,
                csn_min: 1,
                csn_max: 3,
                row_count: 3,
                morsel_count: 1,
                morsel_row_count: 3,
                column_count: 1,
                row_directory_offset: 0,
                column_directory_offset: 0,
                page_index_offset: 0,
                data_offset: 0,
                flags: 0,
                checksum: 0,
            },
            rows: vec![temporal_row(1), temporal_row(2), temporal_row(3)],
            property_columns: vec![TemporalPropertyColumn {
                directory: table_column_directory(logical_type, physical_kind),
                page_index: crate::page::ColumnPageIndex {
                    entries: Vec::new(),
                },
                pages: vec![TemporalPropertyPage {
                    index_entry: ColumnPageIndexEntryV1 {
                        column_id: 42,
                        morsel_id: 0,
                        row_count: 3,
                        non_null_count: 3 - null_count,
                        null_count,
                        encoding_root: 0,
                        page_offset: 0,
                        page_length: 0,
                        uncompressed_length: 0,
                        stats_ref: 0,
                        flags: 0,
                        checksum: 0,
                    },
                    payload: Some(payload),
                }],
            }],
        }
    }

    fn retained_segment_with_one_property_page(
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
        payload: RetainedColumnPagePayloadV1,
        null_count: u32,
    ) -> RetainedTemporalSegmentData {
        RetainedTemporalSegmentData {
            header: TemporalSegmentHeaderV1 {
                segment_id: 9,
                object_type_id: 7,
                time_range_start_us: 1,
                time_range_end_us: 3,
                csn_min: 1,
                csn_max: 3,
                row_count: 3,
                morsel_count: 1,
                morsel_row_count: 3,
                column_count: 1,
                row_directory_offset: 0,
                column_directory_offset: 0,
                page_index_offset: 0,
                data_offset: 0,
                flags: 0,
                checksum: 0,
            },
            rows: vec![temporal_row(1), temporal_row(2), temporal_row(3)],
            property_columns: vec![RetainedTemporalPropertyColumn {
                directory: table_column_directory(logical_type, physical_kind),
                page_index: crate::page::ColumnPageIndex {
                    entries: Vec::new(),
                },
                pages: vec![RetainedTemporalPropertyPage {
                    index_entry: ColumnPageIndexEntryV1 {
                        column_id: 42,
                        morsel_id: 0,
                        row_count: 3,
                        non_null_count: 3 - null_count,
                        null_count,
                        encoding_root: 0,
                        page_offset: 0,
                        page_length: 0,
                        uncompressed_length: 0,
                        stats_ref: 0,
                        flags: 0,
                        checksum: 0,
                    },
                    payload: Some(payload),
                }],
            }],
        }
    }

    fn table_segment_with_one_column(column: TableColumnDirectoryEntryV1) -> TableSegmentPayloadV1 {
        TableSegmentPayloadV1 {
            header: TableSegmentHeaderV1 {
                table_id: 77,
                segment_id: 88,
                row_start: 100,
                row_count: 3,
                morsel_count: 1,
                morsel_row_count: 3,
                column_count: 1,
                morsel_directory_offset: 0,
                column_directory_offset: 0,
                page_index_offset: 0,
                data_offset: 0,
                flags: 0,
                checksum: 0,
            },
            morsels: RowMorselDirectory {
                entries: vec![RowMorselEntryV1 {
                    morsel_id: 5,
                    first_row_in_segment: 0,
                    row_count: 3,
                    flags: 0,
                    stats_ref: 0,
                    checksum: 0,
                }],
            },
            columns: vec![column],
        }
    }

    fn table_page_index_entry(
        column_id: u32,
        morsel_id: u32,
        row_count: u32,
        null_count: u32,
    ) -> ColumnPageIndexEntryV1 {
        ColumnPageIndexEntryV1 {
            column_id,
            morsel_id,
            row_count,
            non_null_count: row_count - null_count,
            null_count,
            encoding_root: 0,
            page_offset: 0,
            page_length: 0,
            uncompressed_length: 0,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        }
    }

    fn table_column_directory(
        logical_type: CoveLogicalType,
        physical_kind: CovePhysicalKind,
    ) -> TableColumnDirectoryEntryV1 {
        let mut bytes = [0u8; crate::segment::TABLE_COLUMN_DIRECTORY_ENTRY_LEN];
        bytes[0..4].copy_from_slice(&42u32.to_le_bytes());
        bytes[4..6].copy_from_slice(&(logical_type as u16).to_le_bytes());
        bytes[6] = physical_kind as u8;
        let checksum_value = checksum::crc32c(&bytes);
        bytes[48..52].copy_from_slice(&checksum_value.to_le_bytes());
        TableColumnDirectoryEntryV1::parse(&bytes).unwrap()
    }

    fn temporal_row(value: u8) -> TemporalRowEntryV1 {
        TemporalRowEntryV1 {
            timestamp_us: i64::from(value),
            csn: u64::from(value),
            branch_key: 0,
            goid: [value; 16],
            record_id: [value; 16],
            record_kind: RecordKind::Delta,
            prev_ref: None,
        }
    }

    fn length_prefixed_rows<const N: usize>(rows: [&[u8]; N]) -> Vec<u8> {
        let mut out = Vec::new();
        for row in rows {
            out.extend_from_slice(&(row.len() as u32).to_le_bytes());
            out.extend_from_slice(row);
        }
        out
    }
}
