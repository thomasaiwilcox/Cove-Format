//! MCP server facade for COVE query discovery and bounded CoveQL execution.

pub mod config;
pub mod results;
pub mod server;
pub mod transport;

pub use config::{
    AllowedRoot, AuthConfig, CoveMcpConfig, CoveMcpError, HttpConfig, SourceRef, TransportMode,
};
pub use results::{PagedResult, ResultHandle, ResultStore};
pub use server::CoveMcpServer;
pub use transport::{serve_http, serve_stdio};
