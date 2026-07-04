//! COVE-QD query-discovery manifest model and strict JSON validation.
//!
//! COVE-QD is advisory metadata. This module validates the manifest envelope and
//! schema-level invariants; CoveQL resolution, source freshness, policy, and
//! sidecar checks remain authoritative at use time.

mod build;
mod embed;
mod helpers;
mod ident;
mod model;
mod relationships;
mod render;
mod source;
mod surfaces;
mod templates;
#[cfg(test)]
mod tests;
mod validate;

pub use build::{build_query_discovery_manifest, build_query_discovery_manifest_value};
pub use embed::{
    embedded_query_discovery_manifests, query_discovery_section_payload,
    query_discovery_validation_context_for_embedded_source,
    query_discovery_validation_context_for_source,
};
pub use ident::coveql_identifier;
pub use model::*;
pub use render::render_query_discovery_template;
pub use validate::validate_query_discovery_manifest;

pub(crate) fn query_discovery_error(message: impl Into<String>) -> crate::CoveError {
    crate::CoveError::QueryDiscoveryInvalid(message.into())
}
