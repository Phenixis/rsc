//! The two OAuth endpoints: the authorize URL (opened in the browser) and the token
//! endpoint (code exchange and refresh).

use std::fmt;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::Value;
use url::Url;

use super::tokens::TokenResponse;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// `invalid_grant`: the code or refresh token is unknown, expired or already used.
    #[error("the authorization code or refresh token was rejected (invalid_grant)")]
    InvalidGrant,
    /// `invalid_client`: wrong `client_id` / `client_secret`.
    #[error(
        "the app credentials were rejected (invalid_client): check client_id and client_secret"
    )]
    InvalidClient,
    #[error("the token endpoint answered HTTP {0}")]
    Status(u16),
    #[error("unexpected response from the token endpoint: {0}")]
    BadResponse(String),
    #[error("cannot reach the token endpoint: {0}")]
    Transport(#[from] reqwest::Error),
}

pub struct TokenClient {
    http: reqwest::Client,
    auth_base: Url,
    client_id: String,
    client_secret: String,
}

impl TokenClient {
    /// `auth_base` is `https://secure.soundcloud.com` (or a mock server).
    pub fn new(auth_base: &str, client_id: &str, client_secret: &str) -> Result<Self> {
        let mut auth_base = Url::parse(auth_base)?;
        if auth_base.cannot_be_a_base() {
            bail!("{auth_base} is not a usable base URL");
        }
        // `Url::join` replaces the last path segment unless the path ends with '/'.
        if !auth_base.path().ends_with('/') {
            let with_slash = format!("{}/", auth_base.path());
            auth_base.set_path(&with_slash);
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()?,
            auth_base,
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        self.auth_base
            .join(path)
            .expect("a relative path always joins onto a base URL")
    }

    /// URL to open in the browser. Never contains the client secret.
    pub fn authorize_url(&self, redirect_uri: &str, code_challenge: &str, state: &str) -> Url {
        let mut url = self.endpoint("authorize");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
        url
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, TokenError> {
        self.token_request(&[
            ("grant_type", "authorization_code"),
            ("client_id", &self.client_id),
            // Required even with PKCE: every SoundCloud client is treated as confidential.
            ("client_secret", &self.client_secret),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
            ("code", code),
        ])
        .await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, TokenError> {
        self.token_request(&[
            ("grant_type", "refresh_token"),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<TokenResponse, TokenError> {
        let response = self
            .http
            .post(self.endpoint("oauth/token"))
            .form(form)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            return serde_json::from_str(&body).map_err(|e| TokenError::BadResponse(e.to_string()));
        }
        Err(classify_failure(status.as_u16(), &body))
    }
}

/// OAuth errors travel as `{"error": "<code>", ...}` (RFC 6749 §5.2).
fn classify_failure(status: u16, body: &str) -> TokenError {
    let code = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned));
    match code.as_deref() {
        Some("invalid_grant") => TokenError::InvalidGrant,
        Some("invalid_client") => TokenError::InvalidClient,
        _ => TokenError::Status(status),
    }
}

impl fmt::Debug for TokenClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenClient")
            .field("auth_base", &self.auth_base.as_str())
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests;
