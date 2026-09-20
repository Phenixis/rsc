//! Authenticated calls to the SoundCloud API.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::de::DeserializeOwned;
use url::Url;

use crate::auth::{AuthError, AuthManager};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("the API answered HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("unexpected API response: {0}")]
    Decode(String),
    #[error("invalid API path {0:?}")]
    InvalidPath(String),
    #[error("cannot reach the API: {0}")]
    Transport(#[from] reqwest::Error),
}

pub struct ApiClient {
    http: reqwest::Client,
    base: Url,
    auth: Arc<AuthManager>,
}

impl ApiClient {
    /// `api_base` is `https://api.soundcloud.com` (or a mock server).
    pub fn new(api_base: &str, auth: Arc<AuthManager>) -> Result<Self> {
        let base = Url::parse(api_base)?;
        if base.cannot_be_a_base() {
            bail!("{base} is not a usable base URL");
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()?,
            base,
            auth,
        })
    }

    /// `GET <base><path_and_query>` with `Authorization: OAuth <token>`. A 401 triggers one
    /// refresh and one retry; a second 401 means the user must log in again.
    pub async fn get_json<T: DeserializeOwned>(&self, path_and_query: &str) -> Result<T, ApiError> {
        let url = self
            .base
            .join(path_and_query)
            .map_err(|_| ApiError::InvalidPath(path_and_query.to_owned()))?;

        let token = self.auth.get_valid_token().await?;
        let mut response = self.send(&url, &token).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            let token = self.auth.refresh_after_unauthorized(&token).await?;
            response = self.send(&url, &token).await?;
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(AuthError::SessionExpired.into());
            }
        }

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(ApiError::Status {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    async fn send(&self, url: &Url, token: &str) -> Result<reqwest::Response, reqwest::Error> {
        self.http
            .get(url.clone())
            // SoundCloud wants the `OAuth` scheme, not `Bearer`.
            .header(AUTHORIZATION, format!("OAuth {token}"))
            .header(ACCEPT, "application/json; charset=utf-8")
            .send()
            .await
    }
}

#[cfg(test)]
mod tests;
