//! LSP live provider for codecite: answers definition, references and call
//! hierarchy against the working tree, without storing results.
//!
//! [`Client`] is a blocking JSON-RPC 2.0 client over any reader and writer
//! pair; [`Server`] starts a language server on it, initializes it, waits for
//! it to be ready and shuts it down.

mod client;
mod deadline;
mod error;
pub mod framing;
pub mod message;
mod server;

pub use client::{Client, ClientOptions, NotificationHandler};
pub use error::{Error, Result};
pub use server::{Readiness, Server, ServerCommand, ServerOptions, file_uri};
