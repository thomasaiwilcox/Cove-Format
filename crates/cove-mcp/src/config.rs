use std::{
    collections::BTreeMap,
    fmt,
    net::{IpAddr, SocketAddr},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum CoveMcpError {
    Config(String),
    Io(std::io::Error),
    Source(String),
    Query(String),
}

impl fmt::Display for CoveMcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) | Self::Source(message) | Self::Query(message) => {
                f.write_str(message)
            }
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CoveMcpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Config(_) | Self::Source(_) | Self::Query(_) => None,
        }
    }
}

impl From<std::io::Error> for CoveMcpError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    Stdio,
    Http,
}

#[derive(Debug, Clone)]
pub struct CoveMcpConfig {
    roots: BTreeMap<String, AllowedRoot>,
    pub default_take: usize,
    pub max_take: usize,
    pub page_size: usize,
    pub max_response_bytes: usize,
    pub result_ttl: Duration,
    pub max_result_handles: usize,
    pub developer_mode: bool,
}

impl CoveMcpConfig {
    pub fn new(roots: Vec<AllowedRoot>) -> Result<Self, CoveMcpError> {
        if roots.is_empty() {
            return Err(CoveMcpError::Config(
                "at least one --root <id=path> must be configured".into(),
            ));
        }
        let mut map = BTreeMap::new();
        for root in roots {
            if root.id.trim().is_empty() {
                return Err(CoveMcpError::Config("root id must not be empty".into()));
            }
            if map.insert(root.id.clone(), root).is_some() {
                return Err(CoveMcpError::Config("duplicate root id".into()));
            }
        }
        Ok(Self {
            roots: map,
            default_take: 50,
            max_take: 500,
            page_size: 100,
            max_response_bytes: 1_048_576,
            result_ttl: Duration::from_secs(600),
            max_result_handles: 128,
            developer_mode: false,
        })
    }

    pub fn roots(&self) -> impl Iterator<Item = &AllowedRoot> {
        self.roots.values()
    }

    pub fn allowed_root(&self, id: &str) -> Option<&AllowedRoot> {
        self.roots.get(id)
    }

    pub fn resolve_source(&self, source: &SourceRef) -> Result<PathBuf, CoveMcpError> {
        let root = self.allowed_root(&source.root).ok_or_else(|| {
            CoveMcpError::Source(format!("unknown configured root `{}`", source.root))
        })?;
        root.resolve(&source.path)
    }
}

#[derive(Debug, Clone)]
pub struct AllowedRoot {
    pub id: String,
    pub path: PathBuf,
    pub display_name: String,
}

impl AllowedRoot {
    pub fn new(id: impl Into<String>, path: impl AsRef<Path>) -> Result<Self, CoveMcpError> {
        let id = id.into();
        let path = path.as_ref().canonicalize().map_err(|error| {
            CoveMcpError::Config(format!(
                "cannot canonicalize root `{id}` at {}: {error}",
                path.as_ref().display()
            ))
        })?;
        Ok(Self {
            display_name: path.display().to_string(),
            id,
            path,
        })
    }

    pub fn resolve(&self, relative: &str) -> Result<PathBuf, CoveMcpError> {
        reject_uri_like_path(relative)?;
        let rel = Path::new(relative);
        if rel.is_absolute() {
            return Err(CoveMcpError::Source(
                "source path must be relative to a configured root".into(),
            ));
        }
        for component in rel.components() {
            if matches!(component, Component::ParentDir | Component::Prefix(_)) {
                return Err(CoveMcpError::Source(
                    "source path must not escape the configured root".into(),
                ));
            }
        }
        let candidate = self.path.join(rel).canonicalize().map_err(|error| {
            CoveMcpError::Source(format!(
                "cannot resolve source `{relative}` under root `{}`: {error}",
                self.id
            ))
        })?;
        if !candidate.starts_with(&self.path) {
            return Err(CoveMcpError::Source(
                "source path resolved outside the configured root".into(),
            ));
        }
        Ok(candidate)
    }
}

fn reject_uri_like_path(path: &str) -> Result<(), CoveMcpError> {
    let Some(colon) = path.find(':') else {
        return Ok(());
    };
    let slash = path.find('/').unwrap_or(usize::MAX);
    let backslash = path.find('\\').unwrap_or(usize::MAX);
    if colon < slash && colon < backslash {
        return Err(CoveMcpError::Source(
            "source path must be a configured-root relative path, not a URI".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceRef {
    pub root: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub bind: SocketAddr,
    pub auth: AuthConfig,
    pub allowed_origins: Vec<String>,
}

impl HttpConfig {
    pub fn validate(&self) -> Result<(), CoveMcpError> {
        let loopback = match self.bind.ip() {
            IpAddr::V4(addr) => addr.is_loopback(),
            IpAddr::V6(addr) => addr.is_loopback(),
        };
        if !loopback && self.auth.bearer_token.is_none() {
            return Err(CoveMcpError::Config(
                "non-loopback HTTP bind requires bearer-token auth".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub bearer_token: Option<String>,
    pub allow_no_auth_local: bool,
}

impl AuthConfig {
    pub fn required_token(&self) -> Option<&str> {
        self.bearer_token.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_uri_source_paths() {
        let err = reject_uri_like_path("file:///tmp/a.cove").unwrap_err();
        assert!(err.to_string().contains("not a URI"));
    }
}
