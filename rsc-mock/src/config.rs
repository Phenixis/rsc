//! Configuration of a mock server, and its validation.

use std::collections::HashSet;
use std::time::Duration;

use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_secret: String,
    /// Exact-match list. An empty list means the client can never complete /authorize.
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserDecision {
    /// The user approves every authorization request automatically.
    Approve,
    /// The user refuses every authorization request (`error=access_denied`).
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockConfig {
    pub clients: Vec<RegisteredClient>,
    /// Lifetime of access tokens; also the `expires_in` of token responses.
    /// A non-zero whole number of seconds.
    pub access_token_lifetime: Duration,
    /// Lifetime of authorization codes. A non-zero whole number of seconds.
    pub authorization_code_lifetime: Duration,
    pub user_decision: UserDecision,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            clients: vec![RegisteredClient {
                client_id: "mock-client-id".to_owned(),
                client_secret: "mock-client-secret".to_owned(),
                redirect_uris: vec!["http://127.0.0.1:8888/callback".to_owned()],
            }],
            // Same value as the `expires_in` in SoundCloud's documented token response.
            access_token_lifetime: Duration::from_secs(3599),
            authorization_code_lifetime: Duration::from_secs(600),
            user_decision: UserDecision::Approve,
        }
    }
}

impl MockConfig {
    /// Checks the invariants the server relies on; the error says which one is broken.
    pub(crate) fn validate(&self) -> Result<(), String> {
        check_lifetime("access_token_lifetime", self.access_token_lifetime)?;
        check_lifetime(
            "authorization_code_lifetime",
            self.authorization_code_lifetime,
        )?;

        let mut seen = HashSet::new();
        for client in &self.clients {
            if !seen.insert(client.client_id.as_str()) {
                return Err(format!("duplicate client_id {:?}", client.client_id));
            }
            for uri in &client.redirect_uris {
                check_redirect_uri(uri)?;
            }
        }
        Ok(())
    }
}

/// `expires_in` is an integer number of seconds, and a zero lifetime would issue tokens that
/// are already expired.
fn check_lifetime(name: &str, lifetime: Duration) -> Result<(), String> {
    if lifetime.is_zero() || lifetime.subsec_nanos() != 0 {
        return Err(format!(
            "{name} must be a non-zero whole number of seconds, got {lifetime:?}"
        ));
    }
    Ok(())
}

/// Redirect URIs are used verbatim as the base of a `Location` header, then extended with a
/// query (RFC 6749 section 3.1.2: absolute URI, no fragment).
fn check_redirect_uri(uri: &str) -> Result<(), String> {
    if Url::parse(uri).is_err() {
        return Err(format!("redirect URI {uri:?} is not an absolute URL"));
    }
    if uri.contains('#') {
        return Err(format!("redirect URI {uri:?} contains a fragment"));
    }
    // A control character cannot be carried in a header value.
    if uri.chars().any(char::is_control) {
        return Err(format!("redirect URI {uri:?} contains a control character"));
    }
    Ok(())
}
