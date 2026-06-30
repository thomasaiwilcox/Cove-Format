use std::fmt;

use crate::{AstChangeMode, AstHistoryMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PhysicalTemporalRowGrain {
    HistoryRecord,
    HistoryState,
    HistoryRecordsAndStates,
    ChangeRecord,
    ChangeStateTransition,
    ChangePropertyDiff,
    ChangeFinalRow,
}

impl PhysicalTemporalRowGrain {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::HistoryRecord => "history_record",
            Self::HistoryState => "history_state",
            Self::HistoryRecordsAndStates => "history_records_and_states",
            Self::ChangeRecord => "change_record",
            Self::ChangeStateTransition => "change_state_transition",
            Self::ChangePropertyDiff => "change_property_diff",
            Self::ChangeFinalRow => "change_final_row",
        }
    }
}

impl fmt::Display for PhysicalTemporalRowGrain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PhysicalTemporalGrain {
    mode: &'static str,
    row_grain: PhysicalTemporalRowGrain,
}

impl PhysicalTemporalGrain {
    pub(super) fn history(mode: AstHistoryMode) -> Self {
        match mode {
            AstHistoryMode::Records => Self {
                mode: "history_records",
                row_grain: PhysicalTemporalRowGrain::HistoryRecord,
            },
            AstHistoryMode::States => Self {
                mode: "history_states",
                row_grain: PhysicalTemporalRowGrain::HistoryState,
            },
            AstHistoryMode::RecordsAndStates => Self {
                mode: "history_records_and_states",
                row_grain: PhysicalTemporalRowGrain::HistoryRecordsAndStates,
            },
        }
    }

    pub(super) fn changes(mode: AstChangeMode) -> Self {
        match mode {
            AstChangeMode::Records => Self {
                mode: "changes_records",
                row_grain: PhysicalTemporalRowGrain::ChangeRecord,
            },
            AstChangeMode::StateTransitions => Self {
                mode: "changes_state_transitions",
                row_grain: PhysicalTemporalRowGrain::ChangeStateTransition,
            },
            AstChangeMode::PropertyDiffs => Self {
                mode: "changes_property_diffs",
                row_grain: PhysicalTemporalRowGrain::ChangePropertyDiff,
            },
            AstChangeMode::FinalRows => Self {
                mode: "changes_final_rows",
                row_grain: PhysicalTemporalRowGrain::ChangeFinalRow,
            },
        }
    }

    pub(super) fn mode(self) -> &'static str {
        self.mode
    }

    pub(super) fn row_grain(self) -> PhysicalTemporalRowGrain {
        self.row_grain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_grain_strings_are_stable() {
        let history = PhysicalTemporalGrain::history(AstHistoryMode::RecordsAndStates);
        assert_eq!(history.mode(), "history_records_and_states");
        assert_eq!(history.row_grain().as_str(), "history_records_and_states");

        let changes = PhysicalTemporalGrain::changes(AstChangeMode::StateTransitions);
        assert_eq!(changes.mode(), "changes_state_transitions");
        assert_eq!(changes.row_grain().as_str(), "change_state_transition");
    }
}
