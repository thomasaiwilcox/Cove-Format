use std::{collections::BTreeSet, fmt};

use cove_core::{
    error::CoveError,
    profile::cove_o::{
        reconstruct_object_states, CoveObjectReconstructionOptions, CoveObjectState,
        CoveObjectSurface,
    },
};

use crate::kernel_predicate::SelectionVector;

#[derive(Debug)]
pub(crate) enum KernelReconstructError {
    ObjectReconstruction(CoveError),
}

impl fmt::Display for KernelReconstructError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ObjectReconstruction(err) => {
                write!(f, "kernel object reconstruction failed: {err}")
            }
        }
    }
}

impl std::error::Error for KernelReconstructError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ObjectReconstruction(err) => Some(err),
        }
    }
}

impl From<CoveError> for KernelReconstructError {
    fn from(err: CoveError) -> Self {
        Self::ObjectReconstruction(err)
    }
}

pub(crate) fn reconstruct_selected_object_states(
    surface: &CoveObjectSurface,
    selection: &SelectionVector,
    options: &CoveObjectReconstructionOptions,
    retained_object_type_ids: &BTreeSet<u32>,
) -> Result<(Vec<CoveObjectState>, usize, usize), KernelReconstructError> {
    let mut selected_keys = selection
        .rows()
        .iter()
        .filter_map(|row| surface.records.get(*row as usize))
        .map(|record| (record.object_type_id, record.branch_key, record.goid))
        .collect::<BTreeSet<_>>();
    selected_keys.extend(
        surface
            .records
            .iter()
            .filter(|record| retained_object_type_ids.contains(&record.object_type_id))
            .map(|record| (record.object_type_id, record.branch_key, record.goid)),
    );

    if selected_keys.is_empty() {
        return Ok((Vec::new(), 0, 0));
    }

    let records = surface
        .records
        .iter()
        .filter(|record| {
            selected_keys.contains(&(record.object_type_id, record.branch_key, record.goid))
        })
        .cloned()
        .collect::<Vec<_>>();
    let retained_record_chain_rows = records.len();
    let candidate_object_keys = selected_keys.len();
    let selected_surface = CoveObjectSurface {
        object_types: surface.object_types.clone(),
        records,
        projection_catalog: surface.projection_catalog.clone(),
        evidence_index: surface.evidence_index.clone(),
        embedded_function_ids: surface.embedded_function_ids.clone(),
        embedded_map_sections: surface.embedded_map_sections.clone(),
    };
    let states = reconstruct_object_states(&selected_surface, options)?;
    Ok((states, candidate_object_keys, retained_record_chain_rows))
}
