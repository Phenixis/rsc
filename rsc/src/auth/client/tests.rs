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
    let body = json!({"error": "invalid_grant", "error_description": "refresh token already used"});
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
