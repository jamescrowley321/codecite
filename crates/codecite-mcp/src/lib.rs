//! MCP server for codecite: exposes the query engine to AI agents over stdio.

mod error;
pub use error::{Error, Result};

mod server;
pub use server::{Server, serve_stdio};
