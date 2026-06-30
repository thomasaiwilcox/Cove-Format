use crate::constants::SectionKind;

use super::CoveAiSection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPayloadAccessState {
    StructurallyAllowed,
    PolicyBlockedMissingPrivacySummary,
}

pub(super) fn payload_access_state(
    sections: &[CoveAiSection],
    has_privacy_summary: bool,
) -> AiPayloadAccessState {
    if !has_privacy_summary
        && sections
            .iter()
            .any(|section| is_payload_bearing_section(section.entry.section_kind))
    {
        AiPayloadAccessState::PolicyBlockedMissingPrivacySummary
    } else {
        AiPayloadAccessState::StructurallyAllowed
    }
}

fn is_payload_bearing_section(section_kind: u32) -> bool {
    matches!(
        SectionKind::from_u16(section_kind as u16),
        Some(
            SectionKind::AiPayloadBytes
                | SectionKind::AiTokenBlock
                | SectionKind::AiVectorPayloadBlock
                | SectionKind::AiVectorDirectory
                | SectionKind::AiTokenSequencePack
                | SectionKind::AiTrainingSampleIndex
                | SectionKind::AiMultimodalSequence
                | SectionKind::AiAssetManifest
        )
    )
}
