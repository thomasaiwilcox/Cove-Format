use super::*;

pub(crate) type SelectionMask = cove_core::native::SelectionBitmap;

#[derive(Debug, Default)]
pub(crate) struct DecodeScratch {
    pub(crate) selected_mask: SelectionMask,
    pub(crate) filter_mask: SelectionMask,
    pub(crate) selected_rows: Vec<u32>,
    pub(crate) selection: Selection,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum Selection {
    #[default]
    None,
    AllRows {
        len: usize,
    },
    Bitset(SelectionMask),
    RowIndices(Vec<u32>),
}

impl Selection {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::None => 0,
            Self::AllRows { len } => *len,
            Self::Bitset(mask) => mask.count_ones(),
            Self::RowIndices(rows) => rows.len(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::None => true,
            Self::AllRows { len } => *len == 0,
            Self::Bitset(mask) => mask.all_zero(),
            Self::RowIndices(rows) => rows.is_empty(),
        }
    }

    pub(crate) fn from_mask(mask: &SelectionMask, rows: &mut Vec<u32>) -> Result<Self, CoveError> {
        let selected = mask.count_ones();
        if selected == 0 {
            return Ok(Self::None);
        }
        if selected == mask.len() {
            return Ok(Self::AllRows { len: mask.len() });
        }
        if selected * 5 <= mask.len() {
            let _ = cove_core::native::compact_selection_bitmap_into(
                mask,
                rows,
                cove_core::native::NativeKernelDispatch::Auto,
            )?;
            return Ok(Self::RowIndices(rows.clone()));
        }
        Ok(Self::Bitset(mask.clone()))
    }

    pub(crate) fn from_rows(rows: &[u32], row_count: usize) -> Self {
        if rows.is_empty() {
            return Self::None;
        }
        if rows.len() == row_count
            && rows
                .iter()
                .enumerate()
                .all(|(index, row)| u32::try_from(index).ok() == Some(*row))
        {
            return Self::AllRows { len: row_count };
        }
        if rows.len() * 5 <= row_count {
            Self::RowIndices(rows.to_vec())
        } else {
            let mut mask = SelectionMask::default();
            mask.fill_none(row_count);
            for row in rows {
                let index = *row as usize;
                if index < row_count {
                    mask.set(index);
                }
            }
            Self::Bitset(mask)
        }
    }

    pub(crate) fn write_rows(&self, rows: &mut Vec<u32>) -> Result<(), CoveError> {
        rows.clear();
        match self {
            Self::None => Ok(()),
            Self::AllRows { len } => {
                rows.reserve(*len);
                for row in 0..*len {
                    rows.push(u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?);
                }
                Ok(())
            }
            Self::Bitset(mask) => mask.write_selected_rows(rows),
            Self::RowIndices(values) => {
                rows.extend_from_slice(values);
                Ok(())
            }
        }
    }

    pub(crate) fn record(&self, stats: &mut DecodeStats) {
        match self {
            Self::None => stats.selection_none += 1,
            Self::AllRows { .. } => stats.selection_all_rows += 1,
            Self::Bitset(_) => stats.selection_bitsets += 1,
            Self::RowIndices(_) => stats.selection_row_indices += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitset_selection_empty_checks_words_without_changing_len_semantics() {
        let mut mask = SelectionMask::none(130);
        let selection = Selection::Bitset(mask.clone());
        assert!(selection.is_empty());
        assert_eq!(selection.len(), 0);

        mask.set(129);
        let selection = Selection::Bitset(mask);
        assert!(!selection.is_empty());
        assert_eq!(selection.len(), 1);
    }

    #[test]
    fn sparse_mask_selection_compacts_with_shared_native_kernel() {
        let mut mask = SelectionMask::none(128);
        mask.set(2);
        mask.set(65);
        let mut rows = vec![99];

        let selection = Selection::from_mask(&mask, &mut rows).unwrap();

        assert!(matches!(selection, Selection::RowIndices(_)));
        assert_eq!(rows, vec![2, 65]);
        let mut written = Vec::new();
        selection.write_rows(&mut written).unwrap();
        assert_eq!(written, vec![2, 65]);
    }
}
