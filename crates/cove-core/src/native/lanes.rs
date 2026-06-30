use super::*;

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
    pub(super) words: Vec<u64>,
    pub(super) len: usize,
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
        let value = wire::read_u64_le_checked(bytes, offset)?;
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
        let value = wire::read_u32_le_checked(bytes, offset)?;
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}
