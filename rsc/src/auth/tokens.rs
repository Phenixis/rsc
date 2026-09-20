//! Token types. Their `Debug` output must never contain the secrets: these end up in logs.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Seconds shaved off `expires_in` so a token is refreshed slightly before it dies.
pub const EXPIRY_MARGIN_SECS: u64 = 60;

/// Body of a successful `POST /oauth/token`.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    #[serde(default)]
    pub scope: String,
}

/// What we persist. Times are unix seconds.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}

impl Tokens {
    pub fn from_response(response: TokenResponse, now: u64) -> Self {
        Self {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_at: now + response.expires_in.saturating_sub(EXPIRY_MARGIN_SECS),
        }
    }

    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .finish()
    }
}

#[cfg(test)]
mod tests;
