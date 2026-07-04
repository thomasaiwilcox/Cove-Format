//! Helpers for resolving local paths declared by manifest-like artifacts.

use std::path::{Component, Path, PathBuf};

use crate::CoveError;

/// Resolve a manifest URI against a local dataset root.
///
/// Dataset-rooted manifests use portable relative paths. This rejects absolute
/// paths, URI schemes, Windows-style prefixes, and parent-directory traversal so
/// manifest contents cannot escape the caller-provided root.
pub fn resolve_manifest_relative_path(base: &Path, uri: &str) -> Result<PathBuf, CoveError> {
    validate_manifest_path_uri(uri, ManifestPathMode::DatasetRelative)?;
    Ok(base.join(Path::new(uri)))
}

/// Resolve a local path URI for APIs whose caller intentionally supplies the
/// complete local path in the manifest-like structure.
///
/// This still rejects URI schemes and parent-directory traversal, but it allows
/// absolute local paths because there is no dataset root to enforce.
pub fn resolve_manifest_local_path(uri: &str) -> Result<PathBuf, CoveError> {
    validate_manifest_path_uri(uri, ManifestPathMode::LocalAbsoluteOrRelative)?;
    Ok(PathBuf::from(uri))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestPathMode {
    DatasetRelative,
    LocalAbsoluteOrRelative,
}

fn validate_manifest_path_uri(uri: &str, mode: ManifestPathMode) -> Result<(), CoveError> {
    if uri.is_empty() {
        return Err(invalid_manifest_path("manifest URI path must not be empty"));
    }
    if uri.as_bytes().contains(&0) {
        return Err(invalid_manifest_path("manifest URI path contains NUL"));
    }
    if uri.contains('\\') {
        return Err(invalid_manifest_path(
            "manifest URI path must use forward slashes",
        ));
    }
    let has_windows_drive_prefix = has_windows_drive_prefix(uri);
    if has_uri_scheme(uri) {
        return Err(invalid_manifest_path(
            "manifest URI path must not include a URI scheme",
        ));
    }
    if mode == ManifestPathMode::DatasetRelative && has_windows_drive_prefix {
        return Err(invalid_manifest_path(
            "manifest URI path must not include a drive prefix",
        ));
    }

    let path = Path::new(uri);
    if mode == ManifestPathMode::DatasetRelative && path.is_absolute() {
        return Err(invalid_manifest_path(
            "manifest URI path must be relative to the dataset root",
        ));
    }

    let mut has_normal_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(invalid_manifest_path(
                    "manifest URI path must not contain parent-directory components",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                if mode == ManifestPathMode::DatasetRelative {
                    return Err(invalid_manifest_path(
                        "manifest URI path must be relative to the dataset root",
                    ));
                }
            }
        }
    }
    if !has_normal_component {
        return Err(invalid_manifest_path(
            "manifest URI path must name a file under the dataset root",
        ));
    }
    Ok(())
}

fn has_uri_scheme(uri: &str) -> bool {
    let first_separator = uri.find('/').unwrap_or(uri.len());
    let Some(colon) = uri[..first_separator].find(':') else {
        return false;
    };
    !(colon == 1 && has_windows_drive_prefix(uri))
}

fn has_windows_drive_prefix(uri: &str) -> bool {
    let bytes = uri.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn invalid_manifest_path(message: &'static str) -> CoveError {
    CoveError::BadSection(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_relative_path_accepts_nested_forward_slash_paths() {
        assert_eq!(
            resolve_manifest_relative_path(Path::new("dataset"), "nested/part.cove").unwrap(),
            PathBuf::from("dataset").join("nested/part.cove")
        );
    }

    #[test]
    fn manifest_relative_path_rejects_escape_forms() {
        for uri in [
            "",
            ".",
            "../part.cove",
            "nested/../../part.cove",
            "/tmp/part.cove",
            "file:///tmp/part.cove",
            "s3://bucket/part.cove",
            "C:/dataset/part.cove",
            "nested\\part.cove",
        ] {
            assert!(
                resolve_manifest_relative_path(Path::new("/dataset"), uri).is_err(),
                "expected {uri:?} to be rejected"
            );
        }
    }

    #[test]
    fn manifest_local_path_allows_absolute_but_rejects_uri_and_parent_paths() {
        assert_eq!(
            resolve_manifest_local_path("/tmp/part.cove").unwrap(),
            PathBuf::from("/tmp/part.cove")
        );
        assert_eq!(
            resolve_manifest_local_path("C:/tmp/part.cove").unwrap(),
            PathBuf::from("C:/tmp/part.cove")
        );
        for uri in [
            "file:///tmp/part.cove",
            "s3://bucket/part.cove",
            "../part.cove",
            "nested/../part.cove",
        ] {
            assert!(
                resolve_manifest_local_path(uri).is_err(),
                "expected {uri:?} to be rejected"
            );
        }
    }
}
