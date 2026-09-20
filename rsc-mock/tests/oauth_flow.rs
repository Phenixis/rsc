//! End-to-end flows and the Rust-side token validity query (`MockServer::token_info`).

mod common;

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::*;
use reqwest::StatusCode;
use rsc_mock::{MockConfig, TokenInfo, TokenKind, UserDecision};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

// spec: B57
#[tokio::test]
async fn a_full_login_the_way_rsc_performs_it() {
    let h = start().await;
    let base = h.server.base_url();

    // PKCE material shaped like rsc's: 32 random bytes, base64url without padding.
    let code_verifier = URL_SAFE_NO_PAD.encode((0u8..32).collect::<Vec<u8>>());
    assert_eq!(code_verifier.len(), 43);
    let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
    let state = URL_SAFE_NO_PAD.encode((100u8..132).collect::<Vec<u8>>());

    // 1. The URL rsc opens in the browser (TokenClient::authorize_url).
    let mut authorize = Url::parse(&base).unwrap().join("authorize").unwrap();
    authorize
        .query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("response_type", "code")
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);
    assert!(!authorize.as_str().contains(CLIENT_SECRET));

    // 2. The browser follows it and is redirected to rsc's loopback listener.
    let resp = h.http.get(authorize).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);
    let redirect = location(&resp);
    assert_eq!(
        format!(
            "{}://{}:{}{}",
            redirect.scheme(),
            redirect.host_str().unwrap(),
            redirect.port().unwrap(),
            redirect.path()
        ),
        REDIRECT_URI
    );

    // 3. rsc's callback listener parses the request line (form-urlencoded query).
    let query = redirect.query().expect("a query string");
    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let get = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get("state").as_deref(), Some(state.as_str()));
    assert_eq!(get("error"), None);
    let code = get("code").filter(|c| !c.is_empty()).expect("a code");

    // 4. TokenClient::exchange_code.
    let resp = h
        .http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", code_verifier.as_str()),
            ("code", code.as_str()),
        ])
        .send()
        .await
        .unwrap();
    let tokens = read_tokens(resp).await;
    assert_eq!(tokens.expires_in, 3599);
    assert_eq!(tokens.raw["token_type"], "bearer");
    assert_eq!(
        h.server.token_info(&tokens.access_token),
        Some(TokenInfo {
            kind: TokenKind::User,
            client_id: CLIENT_ID.to_owned(),
            remaining: Duration::from_secs(3599),
        })
    );

    // 5. An hour later the access token is dead; TokenClient::refresh gets a new one.
    h.server.advance_clock(Duration::from_secs(3599));
    assert!(h.server.token_info(&tokens.access_token).is_none());
    let resp = h
        .http
        .post(format!("{base}/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("refresh_token", tokens.refresh()),
        ])
        .send()
        .await
        .unwrap();
    let renewed = read_tokens(resp).await;
    assert!(h.server.token_info(&renewed.access_token).is_some());

    // 6. rsc stores `renewed.refresh_token` (the old one is now useless).
    assert_sc_error(h.refresh(tokens.refresh()).await, 400, "invalid_grant").await;
    h.refresh_ok(renewed.refresh()).await;
}

// spec: B57
#[tokio::test]
async fn a_login_the_user_refuses_ends_with_access_denied_and_no_tokens() {
    let h = start().await;
    h.server.set_user_decision(UserDecision::Deny);
    let state = "denied-state";
    let challenge = s256(&verifier(1));
    let resp = h.authorize(&authorize_params(&challenge, state)).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let redirect = location(&resp);
    let query = redirect.query().unwrap();
    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    assert!(params.contains(&("state".to_owned(), state.to_owned())));
    assert!(params.contains(&("error".to_owned(), "access_denied".to_owned())));
    assert!(!params.iter().any(|(k, _)| k == "code"));
}

// spec: B57
#[tokio::test]
async fn a_second_login_after_the_first_is_independent() {
    let h = start().await;
    let first = h.session(1).await;
    h.server.advance_clock(Duration::from_secs(120));
    let second = h.session(2).await;
    assert_ne!(first.access_token, second.access_token);
    assert!(h.server.token_info(&first.access_token).is_some());
    assert!(h.server.token_info(&second.access_token).is_some());
    // Logging in again does not invalidate the earlier session.
    h.refresh_ok(first.refresh()).await;
    h.refresh_ok(second.refresh()).await;
}

// spec: B56
#[tokio::test]
async fn token_info_knows_nothing_about_strangers() {
    let h = start().await;
    let tokens = h.session(1).await;
    let login = h.login(2).await;

    let upper = tokens.access_token.to_uppercase();
    let padded = format!("{} ", tokens.access_token);
    let prefix = tokens.access_token[..tokens.access_token.len() - 1].to_owned();
    let mut unknown = vec![
        String::new(),
        "nope".to_owned(),
        "0".repeat(40),
        prefix,
        padded,
        // A refresh token, a code: not access tokens.
        tokens.refresh().to_owned(),
        login.code.clone(),
        "OAuth ".to_owned() + &tokens.access_token,
    ];
    if upper != tokens.access_token {
        unknown.push(upper);
    }
    for token in unknown {
        assert_eq!(h.server.token_info(&token), None, "token {token:?}");
    }
    assert!(h.server.token_info(&tokens.access_token).is_some());
}

// spec: B56
#[tokio::test]
async fn token_info_reports_the_remaining_lifetime_on_the_mock_clock() {
    let h = start().await;
    let tokens = h.session(1).await;
    let info = |h: &Harness| h.server.token_info(&tokens.access_token);

    assert_eq!(info(&h).unwrap().remaining, Duration::from_secs(3599));
    h.server.advance_clock(Duration::from_secs(1));
    assert_eq!(info(&h).unwrap().remaining, Duration::from_secs(3598));
    h.server.advance_clock(Duration::from_millis(500));
    assert_eq!(
        info(&h).unwrap().remaining,
        Duration::from_millis(3_597_500)
    );
    h.server.advance_clock(Duration::from_secs(1000));
    assert_eq!(
        info(&h).unwrap().remaining,
        Duration::from_millis(2_597_500)
    );
}

// spec: B56
#[tokio::test]
async fn a_token_is_valid_strictly_before_its_expiry_instant() {
    let h = start().await;
    let tokens = h.session(1).await;
    h.server.advance_clock(Duration::from_secs(3598));
    let info = h
        .server
        .token_info(&tokens.access_token)
        .expect("one second left");
    assert_eq!(info.remaining, Duration::from_secs(1));
    h.server.advance_clock(Duration::from_millis(999));
    assert!(
        h.server.token_info(&tokens.access_token).is_some(),
        "1 ms left"
    );
    h.server.advance_clock(Duration::from_millis(1));
    assert_eq!(
        h.server.token_info(&tokens.access_token),
        None,
        "expired exactly at issue time + lifetime"
    );
    // And it stays expired.
    h.server.advance_clock(Duration::from_secs(1_000_000));
    assert_eq!(h.server.token_info(&tokens.access_token), None);
}

// spec: B56
#[tokio::test]
async fn token_info_tells_user_tokens_from_app_tokens_and_names_the_client() {
    let h = start_with(two_client_config()).await;
    let user = h.session(1).await;
    let app = read_tokens(h.client_credentials().await).await;
    let other_app = read_tokens(
        h.token_basic(
            "other-client-id",
            "other-client-secret",
            &[("grant_type", "client_credentials")],
        )
        .await,
    )
    .await;

    let user_info = h.server.token_info(&user.access_token).unwrap();
    assert_eq!(user_info.kind, TokenKind::User);
    assert_eq!(user_info.client_id, CLIENT_ID);
    let app_info = h.server.token_info(&app.access_token).unwrap();
    assert_eq!(app_info.kind, TokenKind::App);
    assert_eq!(app_info.client_id, CLIENT_ID);
    let other_info = h.server.token_info(&other_app.access_token).unwrap();
    assert_eq!(other_info.kind, TokenKind::App);
    assert_eq!(other_info.client_id, "other-client-id");
}

// spec: B56
#[tokio::test]
async fn a_refreshed_user_token_is_still_a_user_token() {
    let h = start().await;
    let t1 = h.session(1).await;
    let t2 = h.refresh_ok(t1.refresh()).await;
    let info = h.server.token_info(&t2.access_token).unwrap();
    assert_eq!(info.kind, TokenKind::User);
    assert_eq!(info.client_id, CLIENT_ID);
}

// spec: B56
#[tokio::test]
async fn token_info_matches_what_the_response_promised() {
    for lifetime in [20u64, 3599, 100_000] {
        let config = MockConfig {
            access_token_lifetime: Duration::from_secs(lifetime),
            ..MockConfig::default()
        };
        let h = start_with(config).await;
        let tokens = h.session(1).await;
        let body: &Value = &tokens.raw;
        assert_eq!(body["expires_in"].as_u64(), Some(lifetime));
        let info = h.server.token_info(&tokens.access_token).unwrap();
        assert_eq!(info.remaining, Duration::from_secs(lifetime));
        h.server.advance_clock(Duration::from_secs(lifetime - 1));
        assert!(h.server.token_info(&tokens.access_token).is_some());
        h.server.advance_clock(Duration::from_secs(1));
        assert!(h.server.token_info(&tokens.access_token).is_none());
    }
}
