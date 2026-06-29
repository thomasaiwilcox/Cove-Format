use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalSidecarInputs {
    #[serde(skip)]
    pub coverage_plan_candidate_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub coverage_proof_record_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub coverage_set_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub covi_artifact_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub covx_artifact_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub layout_plan_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub scan_split_index_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub page_cluster_directory_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub zero_copy_buffer_map_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub coverage_cache_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub cove_e_artifact_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    pub cove_ai_artifact_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalSidecarStatus {
    Disabled,
    Missing,
    TrustedCandidate,
    Ignored,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalSidecarValidation {
    pub report_version: String,
    pub name: String,
    pub status: PhysicalSidecarStatus,
    pub candidate_count: usize,
    pub safe_details: serde_json::Value,
    pub fallback_reason: Option<String>,
    pub redacted: bool,
}

impl PhysicalSidecarValidation {
    pub(crate) fn disabled(name: impl Into<String>) -> Self {
        Self {
            report_version: crate::PHYSICAL_SIDECAR_VALIDATION_VERSION.into(),
            name: name.into(),
            status: PhysicalSidecarStatus::Disabled,
            candidate_count: 0,
            safe_details: serde_json::json!({}),
            fallback_reason: Some("candidate planning disabled by physical plan options".into()),
            redacted: false,
        }
    }

    pub(crate) fn missing(name: impl Into<String>) -> Self {
        Self {
            report_version: crate::PHYSICAL_SIDECAR_VALIDATION_VERSION.into(),
            name: name.into(),
            status: PhysicalSidecarStatus::Missing,
            candidate_count: 0,
            safe_details: serde_json::json!({}),
            fallback_reason: Some("optional sidecar metadata was not supplied".into()),
            redacted: false,
        }
    }

    pub(crate) fn trusted(
        name: impl Into<String>,
        candidate_count: usize,
        safe_details: serde_json::Value,
    ) -> Self {
        Self {
            report_version: crate::PHYSICAL_SIDECAR_VALIDATION_VERSION.into(),
            name: name.into(),
            status: PhysicalSidecarStatus::TrustedCandidate,
            candidate_count,
            safe_details,
            fallback_reason: None,
            redacted: true,
        }
    }

    pub(crate) fn ignored(
        name: impl Into<String>,
        reason: impl Into<String>,
        safe_details: serde_json::Value,
    ) -> Self {
        Self {
            report_version: crate::PHYSICAL_SIDECAR_VALIDATION_VERSION.into(),
            name: name.into(),
            status: PhysicalSidecarStatus::Ignored,
            candidate_count: 0,
            safe_details,
            fallback_reason: Some(reason.into()),
            redacted: true,
        }
    }
}
