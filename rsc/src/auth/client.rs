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
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

    use super::*;
    use crate::test_support::token_response_json;

    const REDIRECT: &str = "http://127.0.0.1:8888/callback";

    fn client(server: &MockServer) -> TokenClient {
        TokenClient::new(&server.uri(), "client-id", "client-secret").unwrap()
    }

    /// Matches a form-encoded body containing exactly these fields, nothing more.
    struct Form(BTreeMap<String, String>);

    fn form(pairs: &[(&str, &str)]) -> Form {
        Form(
            pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        )
    }

    impl Match for Form {
        fn matches(&self, request: &Request) -> bool {
            let got: BTreeMap<String, String> = url::form_urlencoded::parse(&request.body)
                .into_owned()
                .collect();
            got == self.0
        }
    }

    async fn token_endpoint_answers(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    // --- authorize URL ------------------------------------------------------------------------

    #[test]
    fn authorize_url_carries_exactly_the_code_flow_and_pkce_parameters() {
        let client = TokenClient::new(
            "https://secure.soundcloud.com",
            "client-id",
            "client-secret",
        )
        .unwrap();
        let url = client.authorize_url(REDIRECT, "CHALLENGE", "STATE");

        assert_eq!(
            (url.scheme(), url.host_str(), url.path()),
            ("https", Some("secure.soundcloud.com"), "/authorize")
        );
        let query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        let expected: BTreeMap<String, String> = [
            ("client_id", "client-id"),
            ("redirect_uri", REDIRECT), // byte for byte what will be sent on the token request
            ("response_type", "code"),
            ("code_challenge", "CHALLENGE"),
            ("code_challenge_method", "S256"),
            ("state", "STATE"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into();
        assert_eq!(query, expected);
    }

    #[test]
    fn authorize_url_never_contains_the_client_secret() {
        let client = TokenClient::new(
            "https://secure.soundcloud.com",
            "client-id",
            "client-secret",
        )
        .unwrap();
        assert!(
            !client
                .authorize_url(REDIRECT, "C", "S")
                .as_str()
                .contains("client-secret")
        );
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_matter() {
        for base in ["http://127.0.0.1:9000", "http://127.0.0.1:9000/"] {
            let client = TokenClient::new(base, "id", "secret").unwrap();
            assert_eq!(
                client.authorize_url(REDIRECT, "C", "S").path(),
                "/authorize",
                "{base}"
            );
        }
    }

    // --- token requests -----------------------------------------------------------------------

    #[tokio::test]
    async fn exchange_code_posts_the_form_the_provider_expects() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(form(&[
                ("grant_type", "authorization_code"),
                ("client_id", "client-id"),
                ("client_secret", "client-secret"), // required even with PKCE
                ("redirect_uri", REDIRECT),
                ("code_verifier", "VERIFIER"),
                ("code", "CODE"),
            ]))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(token_response_json(
                    "new-access",
                    "new-refresh",
                    3600,
                )),
            )
            .expect(1)
            .mount(&server)
            .await;

        let tokens = client(&server)
            .exchange_code("CODE", "VERIFIER", REDIRECT)
            .await
            .unwrap();
        assert_eq!(tokens.access_token, "new-access");
        assert_eq!(tokens.refresh_token, "new-refresh");
        assert_eq!(tokens.expires_in, 3600);
    }

    #[tokio::test]
    async fn refresh_posts_the_refresh_token_grant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(form(&[
                ("grant_type", "refresh_token"),
                ("client_id", "client-id"),
                ("client_secret", "client-secret"),
                ("refresh_token", "OLD-REFRESH"),
            ]))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(token_response_json("a2", "r2", 3599)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let tokens = client(&server).refresh("OLD-REFRESH").await.unwrap();
        assert_eq!(
            (tokens.access_token.as_str(), tokens.refresh_token.as_str()),
            ("a2", "r2")
        );
    }

    // --- failures -----------------------------------------------------------------------------

    #[tokio::test]
    async fn invalid_grant_is_recognised_for_both_grants() {
        let server = MockServer::start().await;
        let body =
            json!({"error": "invalid_grant", "error_description": "refresh token already used"});
        token_endpoint_answers(&server, ResponseTemplate::new(400).set_body_json(body)).await;
        let client = client(&server);

        let refresh = client.refresh("r").await.unwrap_err();
        let exchange = client.exchange_code("c", "v", REDIRECT).await.unwrap_err();
        assert!(matches!(refresh, TokenError::InvalidGrant), "{refresh:?}");
        assert!(matches!(exchange, TokenError::InvalidGrant), "{exchange:?}");
    }

    #[tokio::test]
    async fn invalid_client_is_recognised_so_the_user_can_fix_their_credentials() {
        let server = MockServer::start().await;
        token_endpoint_answers(
            &server,
            ResponseTemplate::new(401).set_body_json(json!({"error": "invalid_client"})),
        )
        .await;

        let err = client(&server).refresh("r").await.unwrap_err();
        assert!(matches!(err, TokenError::InvalidClient), "{err:?}");
        assert!(err.to_string().contains("client_secret"), "{err}");
    }

    #[tokio::test]
    async fn other_http_errors_carry_their_status_and_are_not_mistaken_for_a_logout() {
        let server = MockServer::start().await;
        token_endpoint_answers(
            &server,
            ResponseTemplate::new(503).set_body_string("try later"),
        )
        .await;

        let err = client(&server).refresh("r").await.unwrap_err();
        assert!(matches!(err, TokenError::Status(503)), "{err:?}");
    }

    #[tokio::test]
    async fn a_success_status_with_a_garbage_body_is_an_error() {
        let server = MockServer::start().await;
        token_endpoint_answers(
            &server,
            ResponseTemplate::new(200).set_body_string("<html>captive portal</html>"),
        )
        .await;

        let err = client(&server).refresh("r").await.unwrap_err();
        assert!(matches!(err, TokenError::BadResponse(_)), "{err:?}");
    }

    #[test]
    fn the_client_secret_never_shows_up_in_debug_output() {
        let client = TokenClient::new("http://127.0.0.1:1", "client-id", "client-secret").unwrap();
        assert!(!format!("{client:?}").contains("client-secret"));
    }
}
