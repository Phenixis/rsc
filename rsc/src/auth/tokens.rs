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
mod tests {
    use super::*;

    fn response(expires_in: u64) -> TokenResponse {
        TokenResponse {
            access_token: "access-secret".into(),
            refresh_token: "refresh-secret".into(),
            expires_in,
            scope: String::new(),
        }
    }

    #[test]
    fn expiry_is_now_plus_lifetime_minus_the_safety_margin() {
        let tokens = Tokens::from_response(response(3600), 1000);
        assert_eq!(tokens.expires_at, 1000 + 3600 - EXPIRY_MARGIN_SECS);
        assert_eq!(tokens.access_token, "access-secret");
        assert_eq!(tokens.refresh_token, "refresh-secret");
    }

    #[test]
    fn a_lifetime_shorter_than_the_margin_is_already_expired_not_an_underflow() {
        let tokens = Tokens::from_response(response(30), 1000);
        assert!(tokens.is_expired(1000));
    }

    #[test]
    fn is_expired_flips_exactly_at_expires_at() {
        let tokens = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 100,
        };
        assert!(!tokens.is_expired(99));
        assert!(tokens.is_expired(100));
        assert!(tokens.is_expired(101));
    }

    #[test]
    fn token_endpoint_json_is_parsed_and_scope_is_optional() {
        let full = r#"{"access_token":"a","refresh_token":"r","expires_in":3599,
                        "scope":"","token_type":"bearer"}"#;
        let parsed: TokenResponse = serde_json::from_str(full).unwrap();
        assert_eq!(
            (parsed.access_token.as_str(), parsed.expires_in),
            ("a", 3599)
        );

        let no_scope = r#"{"access_token":"a","refresh_token":"r","expires_in":1}"#;
        assert!(serde_json::from_str::<TokenResponse>(no_scope).is_ok());
        assert!(serde_json::from_str::<TokenResponse>(r#"{"access_token":"a"}"#).is_err());
    }

    #[test]
    fn debug_output_never_contains_the_secrets() {
        let tokens = Tokens::from_response(response(3600), 0);
        for shown in [format!("{tokens:?}"), format!("{:?}", response(1))] {
            assert!(!shown.contains("access-secret"), "{shown}");
            assert!(!shown.contains("refresh-secret"), "{shown}");
        }
    }
}
