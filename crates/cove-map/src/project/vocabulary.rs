use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectionRowGrain {
    Object,
    EventObject,
    ObjectAsOfTime,
    Association,
    LinkObject,
    PropertyVersion,
    EvidenceAssertion,
}

impl ProjectionRowGrain {
    pub(super) fn parse(value: &str) -> Option<Self> {
        value.parse().ok()
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Object => "one_row_per_object",
            Self::EventObject => "one_row_per_event_object",
            Self::ObjectAsOfTime => "one_row_per_object_as_of_time",
            Self::Association => "one_row_per_association",
            Self::LinkObject => "one_row_per_link_object",
            Self::PropertyVersion => "one_row_per_property_version",
            Self::EvidenceAssertion => "one_row_per_evidence_assertion",
        }
    }

    pub(super) fn supports_object_property_lineage(self) -> bool {
        matches!(
            self,
            Self::Object | Self::EventObject | Self::ObjectAsOfTime
        )
    }
}

impl fmt::Display for ProjectionRowGrain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProjectionRowGrain {
    type Err = ParseProjectionVocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "one_row_per_object" => Ok(Self::Object),
            "one_row_per_event_object" => Ok(Self::EventObject),
            "one_row_per_object_as_of_time" => Ok(Self::ObjectAsOfTime),
            "one_row_per_association" => Ok(Self::Association),
            "one_row_per_link_object" => Ok(Self::LinkObject),
            "one_row_per_property_version" => Ok(Self::PropertyVersion),
            "one_row_per_evidence_assertion" => Ok(Self::EvidenceAssertion),
            _ => Err(ParseProjectionVocabularyError),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectionTemporalMode {
    LatestCommitted,
    FullHistory,
    CommitOrder,
    ValidTime,
    ObservedTime,
    AsOfTimestamp(i64),
    AsOfCsn(u64),
}

impl ProjectionTemporalMode {
    pub(super) fn parse(value: &str) -> Option<Self> {
        value.parse().ok()
    }

    pub(super) fn reads_reconstructed_rows_for_access(self) -> bool {
        matches!(self, Self::LatestCommitted | Self::ValidTime)
    }
}

impl FromStr for ProjectionTemporalMode {
    type Err = ParseProjectionVocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "latest_committed" => Ok(Self::LatestCommitted),
            "full_history" => Ok(Self::FullHistory),
            "commit_order" => Ok(Self::CommitOrder),
            "valid_time" => Ok(Self::ValidTime),
            "observed_time" => Ok(Self::ObservedTime),
            _ => parse_temporal_cut_value(value).ok_or(ParseProjectionVocabularyError),
        }
    }
}

fn parse_temporal_cut_value(value: &str) -> Option<ProjectionTemporalMode> {
    for prefix in [
        "as_of_timestamp_us:",
        "as_of_timestamp_us=",
        "timestamp_us:",
        "timestamp_us=",
        "as_of_time:",
        "as_of_time=",
    ] {
        if let Some(raw) = value.strip_prefix(prefix) {
            return raw.parse().ok().map(ProjectionTemporalMode::AsOfTimestamp);
        }
    }
    for prefix in ["as_of_csn:", "as_of_csn=", "csn:", "csn="] {
        if let Some(raw) = value.strip_prefix(prefix) {
            return raw.parse().ok().map(ProjectionTemporalMode::AsOfCsn);
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionFilterPushdown {
    ProjectionCoviPrefilter,
}

impl ProjectionFilterPushdown {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ProjectionCoviPrefilter => "projection_covi_prefilter",
        }
    }
}

impl fmt::Display for ProjectionFilterPushdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionCoviLineageStatus {
    ColumnNotFound,
    Missing,
    Ineligible,
    Present,
}

impl ProjectionCoviLineageStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ColumnNotFound => "column_not_found",
            Self::Missing => "missing",
            Self::Ineligible => "ineligible",
            Self::Present => "present",
        }
    }
}

impl fmt::Display for ProjectionCoviLineageStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionCoviFilterReason {
    ColumnNotFound,
    MissingLineage,
    LineageNotCoviEligible,
    NotEqual,
    NullLiteral,
    EmptyInList,
    IsNull,
    Eligible,
}

impl ProjectionCoviFilterReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ColumnNotFound => "column_not_found",
            Self::MissingLineage => "missing_lineage",
            Self::LineageNotCoviEligible => "lineage_not_covi_eligible",
            Self::NotEqual => "not_equal",
            Self::NullLiteral => "null_literal",
            Self::EmptyInList => "empty_in_list",
            Self::IsNull => "is_null",
            Self::Eligible => "eligible",
        }
    }
}

impl fmt::Display for ProjectionCoviFilterReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParseProjectionVocabularyError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_grain_strings_are_stable() {
        for (raw, expected) in [
            ("one_row_per_object", ProjectionRowGrain::Object),
            ("one_row_per_event_object", ProjectionRowGrain::EventObject),
            (
                "one_row_per_object_as_of_time",
                ProjectionRowGrain::ObjectAsOfTime,
            ),
            ("one_row_per_association", ProjectionRowGrain::Association),
            ("one_row_per_link_object", ProjectionRowGrain::LinkObject),
            (
                "one_row_per_property_version",
                ProjectionRowGrain::PropertyVersion,
            ),
            (
                "one_row_per_evidence_assertion",
                ProjectionRowGrain::EvidenceAssertion,
            ),
        ] {
            assert_eq!(ProjectionRowGrain::parse(raw), Some(expected));
            assert_eq!(expected.as_str(), raw);
            assert_eq!(expected.to_string(), raw);
        }
        assert_eq!(ProjectionRowGrain::parse("not_a_row_grain"), None);
    }

    #[test]
    fn temporal_mode_cut_strings_parse_without_normalizing_output() {
        assert_eq!(
            ProjectionTemporalMode::parse("latest_committed"),
            Some(ProjectionTemporalMode::LatestCommitted)
        );
        assert_eq!(
            ProjectionTemporalMode::parse("as_of_timestamp_us:42"),
            Some(ProjectionTemporalMode::AsOfTimestamp(42))
        );
        assert_eq!(
            ProjectionTemporalMode::parse("timestamp_us=42"),
            Some(ProjectionTemporalMode::AsOfTimestamp(42))
        );
        assert_eq!(
            ProjectionTemporalMode::parse("as_of_csn:7"),
            Some(ProjectionTemporalMode::AsOfCsn(7))
        );
        assert_eq!(ProjectionTemporalMode::parse("as_of_csn:nope"), None);
    }

    #[test]
    fn covi_diagnostic_strings_are_stable() {
        assert_eq!(
            ProjectionFilterPushdown::ProjectionCoviPrefilter.as_str(),
            "projection_covi_prefilter"
        );
        assert_eq!(
            ProjectionCoviLineageStatus::ColumnNotFound.as_str(),
            "column_not_found"
        );
        assert_eq!(
            ProjectionCoviFilterReason::LineageNotCoviEligible.as_str(),
            "lineage_not_covi_eligible"
        );
        assert_eq!(
            ProjectionCoviFilterReason::EmptyInList.to_string(),
            "empty_in_list"
        );
    }
}
