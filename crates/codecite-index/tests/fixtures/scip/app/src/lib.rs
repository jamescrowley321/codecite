//! A tiny crate for codecite's SCIP tests.

pub mod client;

use client::Client;

/// Builds a client and validates a token with it.
pub fn login(token: &str) -> bool {
    let client = Client::new();
    client.validate(token)
}

/// Logs in twice.
pub fn relogin(token: &str) -> bool {
    login(token) && login(token)
}
