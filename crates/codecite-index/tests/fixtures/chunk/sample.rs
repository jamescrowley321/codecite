//! A sample module.

use std::fmt;

/// A client.
pub struct Client {
    issuer: String,
}

impl Client {
    pub fn new(issuer: &str) -> Self {
        Self { issuer: issuer.into() }
    }

    pub fn validate(&self, token: &str) -> bool {
        !token.is_empty()
    }
}

impl fmt::Display for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.issuer)
    }
}

pub enum Grant {
    Code,
    Implicit,
}

pub trait Verifier {
    fn verify(&self) -> bool;
}

fn helper() -> u32 {
    fn inner() -> u32 {
        1
    }
    inner()
}
