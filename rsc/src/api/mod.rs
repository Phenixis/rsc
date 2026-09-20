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
mod tests {
    use serde::Deserialize;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, ResponseTemplate};

    use super::*;
    use crate::test_support::{Fixture, NOW, tokens};

    #[derive(Debug, Deserialize, PartialEq)]
    struct Me {
        username: String,
    }

    fn api(f: &Fixture) -> ApiClient {
        ApiClient::new(&f.server.uri(), f.auth.clone()).unwrap()
    }

    fn me_ok(username: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({ "username": username }))
    }

    #[tokio::test]
    async fn requests_use_the_oauth_scheme_not_bearer() {
        let f = Fixture::with_tokens(tokens("access-1", "refresh-1", NOW + 1000)).await;
        // Only answers when the header is exactly `OAuth access-1`; anything else is a 404.
        Mock::given(method("GET"))
            .and(path("/me"))
            .and(header("authorization", "OAuth access-1"))
            .respond_with(me_ok("maxime"))
            .expect(1)
            .mount(&f.server)
            .await;

        let me: Me = api(&f).get_json("/me").await.unwrap();
        assert_eq!(
            me,
            Me {
                username: "maxime".into()
            }
        );
    }

    #[tokio::test]
    async fn an_expired_token_is_refreshed_before_the_request_not_after_a_401() {
        let f = Fixture::with_tokens(tokens("old", "r1", NOW - 1)).await;
        f.mount_refresh_ok("new", "r2").await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .and(header("authorization", "OAuth new"))
            .respond_with(me_ok("maxime"))
            .mount(&f.server)
            .await;

        let me: Me = api(&f).get_json("/me").await.unwrap();
        assert_eq!(me.username, "maxime");
        assert_eq!(
            (
                f.requests_to("/oauth/token").await,
                f.requests_to("/me").await
            ),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn a_401_triggers_one_refresh_and_one_retry_with_the_new_token() {
        let f = Fixture::with_tokens(tokens("old", "r1", NOW + 1000)).await; // looks valid, but the API disagrees
        f.mount_refresh_ok("new", "r2").await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .and(header("authorization", "OAuth old"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .and(header("authorization", "OAuth new"))
            .respond_with(me_ok("maxime"))
            .mount(&f.server)
            .await;

        let me: Me = api(&f).get_json("/me").await.unwrap();
        assert_eq!(me.username, "maxime");
        assert_eq!(
            (
                f.requests_to("/oauth/token").await,
                f.requests_to("/me").await
            ),
            (1, 2)
        );
        assert_eq!(f.stored().await, Some(tokens("new", "r2", NOW + 3600 - 60)));
    }

    #[tokio::test]
    async fn a_second_401_means_log_in_again_and_never_loops() {
        let f = Fixture::with_tokens(tokens("old", "r1", NOW + 1000)).await;
        f.mount_refresh_ok("new", "r2").await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&f.server)
            .await;

        let err = api(&f).get_json::<Me>("/me").await.unwrap_err();
        assert!(
            matches!(err, ApiError::Auth(AuthError::SessionExpired)),
            "{err:?}"
        );
        assert_eq!(
            (
                f.requests_to("/oauth/token").await,
                f.requests_to("/me").await
            ),
            (1, 2)
        );
    }

    #[tokio::test]
    async fn other_error_statuses_are_reported_with_their_body_and_never_retried() {
        for (status, body) in [(403, "forbidden"), (404, "no such thing"), (500, "boom")] {
            let f = Fixture::with_tokens(tokens("a", "r", NOW + 1000)).await;
            Mock::given(method("GET"))
                .and(path("/me"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .mount(&f.server)
                .await;

            let err = api(&f).get_json::<Me>("/me").await.unwrap_err();
            assert!(
                matches!(&err, ApiError::Status { status: s, body: b } if *s == status && b.contains(body)),
                "{status}: {err:?}"
            );
            assert_eq!(
                (
                    f.requests_to("/me").await,
                    f.requests_to("/oauth/token").await
                ),
                (1, 0),
                "{status}"
            );
        }
    }

    #[tokio::test]
    async fn a_body_of_the_wrong_shape_is_a_decode_error() {
        let f = Fixture::with_tokens(tokens("a", "r", NOW + 1000)).await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([1, 2])))
            .mount(&f.server)
            .await;

        let err = api(&f).get_json::<Me>("/me").await.unwrap_err();
        assert!(matches!(err, ApiError::Decode(_)), "{err:?}");
    }
}
