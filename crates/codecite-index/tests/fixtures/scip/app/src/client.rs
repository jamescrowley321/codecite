/// A client.
pub struct Client {
    pub issuer: String,
}

impl Client {
    /// A client with no issuer.
    pub fn new() -> Self {
        Client {
            issuer: String::new(),
        }
    }

    /// Whether the token is usable.
    pub fn validate(&self, token: &str) -> bool {
        check(token) && !self.issuer.is_empty()
    }
}

fn check(token: &str) -> bool {
    !token.is_empty()
}
