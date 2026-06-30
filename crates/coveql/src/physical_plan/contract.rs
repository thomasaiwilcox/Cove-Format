use std::{borrow::Cow, fmt};

use super::PhysicalOperatorContract;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PhysicalContractName(Cow<'static, str>);

impl PhysicalContractName {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for PhysicalContractName {
    fn from(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl From<String> for PhysicalContractName {
    fn from(value: String) -> Self {
        Self(Cow::Owned(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PhysicalContractText(Cow<'static, str>);

impl PhysicalContractText {
    fn into_string(self) -> String {
        self.0.into_owned()
    }
}

impl From<&'static str> for PhysicalContractText {
    fn from(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl From<String> for PhysicalContractText {
    fn from(value: String) -> Self {
        Self(Cow::Owned(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicalCardinalityContract {
    ExactWhenProofsPassOtherwiseNoFalseNegatives,
}

impl PhysicalCardinalityContract {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExactWhenProofsPassOtherwiseNoFalseNegatives => {
                "exact-authoritative when proofs pass; otherwise no false negatives with residual checks"
            }
        }
    }
}

impl fmt::Display for PhysicalCardinalityContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicalOrderingContract {
    NoLogicalOrderingUnlessMaterializedSortFollows,
}

impl PhysicalOrderingContract {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoLogicalOrderingUnlessMaterializedSortFollows => {
                "does not establish logical ordering unless materialized sort follows"
            }
        }
    }
}

impl fmt::Display for PhysicalOrderingContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicalProtectedMetadataKind {
    Paths,
    Literals,
    SidecarIdentifiers,
}

impl PhysicalProtectedMetadataKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Paths => "paths",
            Self::Literals => "literals",
            Self::SidecarIdentifiers => "sidecar identifiers",
        }
    }
}

impl fmt::Display for PhysicalProtectedMetadataKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicalExplainField {
    Operator,
    CandidateCount,
    Fallback,
    RedactedMetadata,
}

impl PhysicalExplainField {
    fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::CandidateCount => "candidate_count",
            Self::Fallback => "fallback",
            Self::RedactedMetadata => "redacted_metadata",
        }
    }
}

impl fmt::Display for PhysicalExplainField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(super) fn contract<I, O, Input, Output>(
    name: impl Into<PhysicalContractName>,
    inputs: I,
    outputs: O,
    fallback: impl Into<PhysicalContractText>,
) -> PhysicalOperatorContract
where
    I: IntoIterator<Item = Input>,
    Input: Into<PhysicalContractText>,
    O: IntoIterator<Item = Output>,
    Output: Into<PhysicalContractText>,
{
    let name = name.into();
    PhysicalOperatorContract {
        contract_version: crate::PHYSICAL_OPERATOR_CONTRACT_VERSION.into(),
        inputs: inputs
            .into_iter()
            .map(Into::into)
            .map(PhysicalContractText::into_string)
            .collect(),
        outputs: outputs
            .into_iter()
            .map(Into::into)
            .map(PhysicalContractText::into_string)
            .collect(),
        preconditions: vec![format!(
            "{} preconditions must be validated before use",
            name.as_str()
        )],
        postconditions: vec![format!(
            "{} must preserve logical truth under its validated authority contract",
            name.as_str()
        )],
        cardinality: PhysicalCardinalityContract::ExactWhenProofsPassOtherwiseNoFalseNegatives
            .to_string(),
        ordering: PhysicalOrderingContract::NoLogicalOrderingUnlessMaterializedSortFollows
            .to_string(),
        protected_metadata: [
            PhysicalProtectedMetadataKind::Paths,
            PhysicalProtectedMetadataKind::Literals,
            PhysicalProtectedMetadataKind::SidecarIdentifiers,
        ]
        .into_iter()
        .map(|kind| kind.to_string())
        .collect(),
        pre_redaction_safe: false,
        index_only_eligible: false,
        fallback: fallback.into().into_string(),
        explain_fields: [
            PhysicalExplainField::Operator,
            PhysicalExplainField::CandidateCount,
            PhysicalExplainField::Fallback,
            PhysicalExplainField::RedactedMetadata,
        ]
        .into_iter()
        .map(|field| field.to_string())
        .collect(),
    }
}
