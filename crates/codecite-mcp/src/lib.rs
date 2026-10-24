//! MCP server for codecite: exposes the query engine to AI agents over stdio.

mod error;
pub use error::{Error, Result};

mod server;
pub use server::{ErrorInfo, ReadParams, SearchParams, Server, ToolError, serve_stdio};
