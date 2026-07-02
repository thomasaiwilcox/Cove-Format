//! Writer facade for stable COVE v2 writer APIs.

use std::{path::Path, path::PathBuf};

pub use cove_core::{
    array, constants, dictionary, encoding, page, page_payload, segment, table, validity, writer,
    CoveError,
};

/// Serialize a scan-profile COVE table into an in-memory byte buffer.
///
/// # Errors
///
/// Returns a [`CoveError`] if the writer cannot produce a valid COVE table,
/// including invalid table metadata, section layout, or encoded page payloads.
pub fn write_table(writer: &writer::ScanProfileCoveWriter) -> Result<Vec<u8>, CoveError> {
    writer.write()
}

/// Durably publish a scan-profile COVE table to `path`.
///
/// # Errors
///
/// Returns a [`CoveError`] if serialization fails or the durable replace step
/// cannot write, sync, or atomically publish the target file.
pub fn publish_table(
    writer: &writer::ScanProfileCoveWriter,
    path: impl AsRef<Path>,
) -> Result<PathBuf, CoveError> {
    writer.publish_durable(path.as_ref())
}

/// Serialize a minimal COVE file into an in-memory byte buffer.
///
/// # Errors
///
/// Returns a [`CoveError`] if the minimal writer cannot produce valid COVE v2
/// header, footer, section, or digest metadata.
pub fn write_minimal(writer: &writer::MinimalCoveWriter) -> Result<Vec<u8>, CoveError> {
    writer.write()
}

/// Durably publish a minimal COVE file to `path`.
///
/// # Errors
///
/// Returns a [`CoveError`] if serialization fails or the durable replace step
/// cannot write, sync, or atomically publish the target file.
pub fn publish_minimal(
    writer: &writer::MinimalCoveWriter,
    path: impl AsRef<Path>,
) -> Result<PathBuf, CoveError> {
    writer.publish_durable(path.as_ref())
}
