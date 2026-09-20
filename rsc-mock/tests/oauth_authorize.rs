//! `GET /authorize`: the browser-facing half of the authorization code flow.

mod common;

use std::collections::HashSet;

use common::*;
use reqwest::{Response, StatusCode};
use rsc_mock::{MockConfig, UserDecision};

/// Default config, but the user refuses every request.
fn denying() -> MockConfig {
    MockConfig {
        user_decision: UserDecision::Deny,
        ..MockConfig::default()
    }
}

fn raw_query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// Authorize parameters for an arbitrary client / redirect URI.
fn params_for<'a>(
    client_id: &'a str,
    redirect_uri: &'a str,
    challenge: &'a str,
    state: &'a str,
) -> Vec<(&'a str, &'a str)> {
    vec![
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ]
}

/// The mock must not redirect: 400 with the SoundCloud error body.
async fn assert_no_redirect(resp: Response, message: &str) {
    assert!(
        resp.headers().get("location").is_none(),
        "must not redirect to an unverified redirect_uri"
    );
    assert_sc_error(resp, 400, message).await;
}

/// 302 to the default redirect URI carrying `error` and `state` (and nothing else).
fn assert_error_redirect(resp: &Response, error: &str, state: &str) {
    assert_eq!(resp.status(), StatusCode::FOUND, "an error redirect");
    let loc = location(resp);
    assert_eq!(loc.host_str(), Some("127.0.0.1"));
    assert_eq!(loc.port(), Some(8888));
    assert_eq!(loc.path(), "/callback");
    assert_eq!(param(&loc, "error").as_deref(), Some(error));
    assert_eq!(param(&loc, "state").as_deref(), Some(state));
    assert_eq!(param_names(&loc), ["error", "state"], "no code on error");
    let raw = header(resp, "location").unwrap();
    assert!(
        raw.starts_with(&format!("{REDIRECT_URI}?error={error}&state=")),
        "error comes first, then state: {raw}"
    );
}

// spec: B8
#[tokio::test]
async fn authorize_redirects_to_the_redirect_uri_with_code_and_state() {
    let h = start().await;
    for (n, state) in [(1, "abc"), (2, "Zm9vYmFy_-x"), (3, "0")] {
        let challenge = s256(&verifier(n));
        let resp = h.authorize(&authorize_params(&challenge, state)).await;
        assert_eq!(resp.status(), StatusCode::FOUND);
        let loc = location(&resp);
        assert_eq!(loc.scheme(), "http");
        assert_eq!(loc.host_str(), Some("127.0.0.1"));
        assert_eq!(loc.port(), Some(8888));
        assert_eq!(loc.path(), "/callback");
        assert_eq!(loc.fragment(), None);
        assert_eq!(param_names(&loc), ["code", "state"]);
        assert_eq!(param(&loc, "state").as_deref(), Some(state));
        let code = param(&loc, "code").unwrap();
        assert!(
            is_opaque(&code),
            "code {code:?} must be opaque and URL-safe"
        );
        // Documented order: code, then state.
        let raw = header(&resp, "location").unwrap();
        assert!(
            raw.starts_with(&format!("{REDIRECT_URI}?code={code}&state=")),
            "{raw}"
        );
    }
}

// spec: B8
#[tokio::test]
async fn authorize_ignores_unknown_parameters() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let mut params = authorize_params(&challenge, "s1");
    params.push(("nonce", "n-0S6_WzA2Mj"));
    params.push(("display", "popup"));
    params.push(("response_mode", "fragment"));
    let resp = h.authorize(&params).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert_eq!(param_names(&loc), ["code", "state"]);
}

// spec: B9
#[tokio::test]
async fn every_authorization_gets_a_fresh_code() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let mut codes = HashSet::new();
    for _ in 0..50 {
        // Same request every time: still a different code.
        let resp = h.authorize(&authorize_params(&challenge, "same")).await;
        let code = param(&location(&resp), "code").expect("code");
        assert!(codes.insert(code), "codes must be unique");
    }
}

// spec: B10
#[tokio::test]
async fn state_survives_the_round_trip_whatever_it_contains() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let states = [
        "a b",
        "a&b=c",
        "x#y",
        "\u{e9}\u{2713}\u{1f600}",
        "100%",
        "+plus+",
        "a/b?c",
        "=",
        "%zz",
        "%41",
        "line\nbreak",
        "tab\there",
        "\"quoted\" <tag>",
        ";",
        "&code=injected",
        "a=b&error=access_denied",
    ];
    for state in states {
        let resp = h.authorize(&authorize_params(&challenge, state)).await;
        assert_eq!(resp.status(), StatusCode::FOUND, "state {state:?}");
        let raw = header(&resp, "location").expect("Location is a valid ASCII header");
        let (_, encoded_state) = raw.split_once("&state=").expect("state at the end");
        for forbidden in ['&', '#', ' ', '\n', '"'] {
            assert!(
                !encoded_state.contains(forbidden),
                "{forbidden:?} must be encoded in {encoded_state}"
            );
        }
        let loc = location(&resp);
        assert_eq!(
            loc.fragment(),
            None,
            "state {state:?} must not open a fragment"
        );
        assert_eq!(param_names(&loc), ["code", "state"], "state {state:?}");
        assert_eq!(param(&loc, "state").as_deref(), Some(state));
    }
}

// spec: B10
#[tokio::test]
async fn state_is_encoded_in_error_redirects_too() {
    let h = start_with(denying()).await;
    let challenge = s256(&verifier(1));
    for state in ["a b&c", "x#y", "\u{e9}\u{1f600}", "%zz"] {
        let resp = h.authorize(&authorize_params(&challenge, state)).await;
        assert_eq!(resp.status(), StatusCode::FOUND);
        let loc = location(&resp);
        assert_eq!(loc.fragment(), None);
        assert_eq!(param_names(&loc), ["error", "state"]);
        assert_eq!(param(&loc, "state").as_deref(), Some(state));
    }
}

// spec: B10
#[tokio::test]
async fn an_empty_state_is_echoed_and_an_absent_state_is_omitted() {
    let h = start().await;
    let challenge = s256(&verifier(1));

    let params = authorize_params_with(&challenge, "", "state", Some(""));
    let resp = h.authorize(&params).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert_eq!(param_names(&loc), ["code", "state"]);
    assert_eq!(param(&loc, "state").as_deref(), Some(""));

    let params = authorize_params_with(&challenge, "", "state", None);
    let resp = h.authorize(&params).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(param_names(&location(&resp)), ["code"]);
}

// spec: B10
#[tokio::test]
async fn a_long_state_is_echoed_in_full() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for len in [256, 4000] {
        let state: String = "s0-".repeat(len / 3);
        let resp = h.authorize(&authorize_params(&challenge, &state)).await;
        assert_eq!(
            resp.status(),
            StatusCode::FOUND,
            "state of {len} characters"
        );
        assert_eq!(
            param(&location(&resp), "state").as_deref(),
            Some(state.as_str())
        );
    }
}

// spec: B11
#[tokio::test]
async fn an_existing_query_on_the_redirect_uri_is_preserved() {
    let with_query = "http://127.0.0.1:8888/callback?foo=bar&x=%20y";
    let mut config = MockConfig::default();
    config.clients[0].redirect_uris = vec![with_query.to_owned()];
    let h = start_with(config).await;
    let challenge = s256(&verifier(1));

    let resp = h
        .authorize(&params_for(CLIENT_ID, with_query, &challenge, "st"))
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let raw = header(&resp, "location").unwrap();
    assert!(
        raw.starts_with("http://127.0.0.1:8888/callback?foo=bar&x=%20y&code="),
        "registered URI kept verbatim, then code: {raw}"
    );
    let loc = location(&resp);
    assert_eq!(param_names(&loc), ["code", "foo", "state", "x"]);
    assert_eq!(param(&loc, "foo").as_deref(), Some("bar"));
    assert_eq!(param(&loc, "x").as_deref(), Some(" y"));
    assert_eq!(param(&loc, "state").as_deref(), Some("st"));
}

// spec: B11
#[tokio::test]
async fn an_existing_query_is_preserved_on_error_redirects_too() {
    let with_query = "http://127.0.0.1:8888/callback?foo=bar";
    let mut config = denying();
    config.clients[0].redirect_uris = vec![with_query.to_owned()];
    let h = start_with(config).await;
    let challenge = s256(&verifier(1));

    let resp = h
        .authorize(&params_for(CLIENT_ID, with_query, &challenge, "st"))
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let raw = header(&resp, "location").unwrap();
    assert!(
        raw.starts_with("http://127.0.0.1:8888/callback?foo=bar&error=access_denied&state="),
        "{raw}"
    );
}

// spec: B11
#[tokio::test]
async fn a_redirect_uri_without_query_gets_a_question_mark() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let resp = h.authorize(&authorize_params(&challenge, "st")).await;
    let raw = header(&resp, "location").unwrap();
    assert_eq!(raw.matches('?').count(), 1, "{raw}");
}

// spec: B12
#[tokio::test]
async fn an_unknown_client_is_rejected_without_redirect() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for client_id in [
        "nope",
        "MOCK-CLIENT-ID",
        "mock-client-id ",
        " mock-client-id",
        "mock-client-i",
        "mock-client-id2",
        "other-client-id",
    ] {
        let resp = h
            .authorize(&params_for(client_id, REDIRECT_URI, &challenge, "s"))
            .await;
        assert_no_redirect(resp, "invalid_client").await;
    }
}

// spec: B12
#[tokio::test]
async fn a_missing_or_empty_client_id_is_rejected_without_redirect() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let resp = h
        .authorize(&authorize_params_with(&challenge, "s", "client_id", None))
        .await;
    assert_no_redirect(resp, "invalid_client").await;
    let resp = h
        .authorize(&authorize_params_with(
            &challenge,
            "s",
            "client_id",
            Some(""),
        ))
        .await;
    assert_no_redirect(resp, "invalid_client").await;
}

// spec: B12
#[tokio::test]
async fn an_unknown_client_is_reported_before_a_bad_redirect_uri() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let resp = h
        .authorize(&params_for(
            "nope",
            "http://evil.example/cb",
            &challenge,
            "s",
        ))
        .await;
    assert_no_redirect(resp, "invalid_client").await;
}

// spec: B13
#[tokio::test]
async fn the_redirect_uri_must_match_exactly() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for uri in [
        "http://localhost:8888/callback",
        "http://127.0.0.1:8888/callback/",
        "http://127.0.0.1:9999/callback",
        "http://127.0.0.1:8889/callback",
        "http://127.0.0.1/callback",
        "https://127.0.0.1:8888/callback",
        "http://127.0.0.1:8888/Callback",
        "http://127.0.0.1:8888/callback?x=1",
        "http://127.0.0.1:8888/callback#frag",
        "HTTP://127.0.0.1:8888/callback",
        " http://127.0.0.1:8888/callback",
        "http://127.0.0.1:8888/callback ",
        "http://127.0.0.1:8888",
        "http://127.0.0.1:8888/",
        "http://127.0.0.1:8888//callback",
        "http://127.0.0.1:8888/callback/../callback",
        "http://127.0.0.1:08888/callback",
        "http://user@127.0.0.1:8888/callback",
        "http://[::1]:8888/callback",
        "http://127.0.0.1:8888/call",
        "http://127.0.0.1:8888/callbackx",
        "http://evil.example/callback",
        "127.0.0.1:8888/callback",
        "/callback",
        "",
    ] {
        let resp = h
            .authorize(&params_for(CLIENT_ID, uri, &challenge, "s"))
            .await;
        assert!(
            resp.headers().get("location").is_none(),
            "redirect_uri {uri:?} must not be redirected to"
        );
        assert_sc_error(resp, 400, "invalid_request").await;
    }
}

// spec: B13
#[tokio::test]
async fn a_missing_redirect_uri_is_rejected_without_redirect() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let resp = h
        .authorize(&authorize_params_with(
            &challenge,
            "s",
            "redirect_uri",
            None,
        ))
        .await;
    assert_no_redirect(resp, "invalid_request").await;
}

// spec: B13
#[tokio::test]
async fn a_bad_redirect_uri_wins_over_every_other_error() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let bad = "http://127.0.0.1:8888/callback/";
    // Wrong response_type, no challenge, plain method: still no redirect.
    let resp = h
        .authorize(&[
            ("client_id", CLIENT_ID),
            ("redirect_uri", bad),
            ("response_type", "token"),
            ("code_challenge_method", "plain"),
            ("state", "s"),
        ])
        .await;
    assert_no_redirect(resp, "invalid_request").await;

    let h2 = start_with(denying()).await;
    let resp = h2
        .authorize(&params_for(CLIENT_ID, bad, &challenge, "s"))
        .await;
    assert_no_redirect(resp, "invalid_request").await;
}

// spec: B14
#[tokio::test]
async fn a_client_can_register_several_redirect_uris() {
    let second = "http://127.0.0.1:8899/other/cb";
    let mut config = two_client_config();
    config.clients[0].redirect_uris.push(second.to_owned());
    let h = start_with(config).await;
    let challenge = s256(&verifier(1));

    for uri in [REDIRECT_URI, second] {
        let resp = h
            .authorize(&params_for(CLIENT_ID, uri, &challenge, "s"))
            .await;
        assert_eq!(resp.status(), StatusCode::FOUND, "{uri}");
        let raw = header(&resp, "location").unwrap();
        assert!(raw.starts_with(&format!("{uri}?code=")), "{raw}");
    }
    // The other client has its own list.
    let resp = h
        .authorize(&params_for(
            "other-client-id",
            "http://127.0.0.1:9999/other",
            &challenge,
            "s",
        ))
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert!(
        header(&resp, "location")
            .unwrap()
            .starts_with("http://127.0.0.1:9999/other?code=")
    );
}

// spec: B14
#[tokio::test]
async fn a_redirect_uri_of_another_client_is_not_accepted() {
    let h = start_with(two_client_config()).await;
    let challenge = s256(&verifier(1));
    let resp = h
        .authorize(&params_for(
            CLIENT_ID,
            "http://127.0.0.1:9999/other",
            &challenge,
            "s",
        ))
        .await;
    assert_no_redirect(resp, "invalid_request").await;
    let resp = h
        .authorize(&params_for(
            "other-client-id",
            REDIRECT_URI,
            &challenge,
            "s",
        ))
        .await;
    assert_no_redirect(resp, "invalid_request").await;
}

// spec: B15
#[tokio::test]
async fn response_type_other_than_code_redirects_with_unsupported_response_type() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for response_type in ["token", "id_token", "CODE", "code token", "code ", "codes"] {
        let params =
            authorize_params_with(&challenge, "st4te-1", "response_type", Some(response_type));
        let resp = h.authorize(&params).await;
        assert_error_redirect(&resp, "unsupported_response_type", "st4te-1");
    }
}

// spec: B15
#[tokio::test]
async fn a_missing_or_empty_response_type_redirects_with_invalid_request() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for value in [None, Some("")] {
        let params = authorize_params_with(&challenge, "st4te-2", "response_type", value);
        let resp = h.authorize(&params).await;
        assert_error_redirect(&resp, "invalid_request", "st4te-2");
    }
}

// spec: B16
#[tokio::test]
async fn a_missing_or_malformed_code_challenge_redirects_with_invalid_request() {
    let h = start().await;
    let a42 = "A".repeat(42);
    let a44 = "A".repeat(44);
    let padded = format!("{}=", s256(&verifier(1)));
    let dotted = format!("{}.", "A".repeat(42));
    let tilde = format!("{}~", "A".repeat(42));
    let plus = format!("{}+", "A".repeat(42));
    let slash = format!("{}/", "A".repeat(42));
    let spaced = format!("{} ", "A".repeat(42));
    let accents = "\u{e9}".repeat(43);
    let mut cases: Vec<Option<&str>> = vec![None];
    for bad in [
        "",
        a42.as_str(),
        a44.as_str(),
        padded.as_str(),
        dotted.as_str(),
        tilde.as_str(),
        plus.as_str(),
        slash.as_str(),
        spaced.as_str(),
        accents.as_str(),
    ] {
        cases.push(Some(bad));
    }
    for value in cases {
        let params = authorize_params_with("", "st4te-3", "code_challenge", value);
        let resp = h.authorize(&params).await;
        assert_error_redirect(&resp, "invalid_request", "st4te-3");
    }
}

// spec: B16
#[tokio::test]
async fn a_well_formed_challenge_is_accepted_whatever_its_value() {
    let h = start().await;
    for challenge in [
        "A".repeat(43),
        "-".repeat(43),
        "_".repeat(43),
        "9".repeat(43),
        s256("anything at all"),
    ] {
        let resp = h.authorize(&authorize_params(&challenge, "s")).await;
        assert_eq!(resp.status(), StatusCode::FOUND, "challenge {challenge}");
        assert!(param(&location(&resp), "code").is_some());
    }
}

// spec: B17
#[tokio::test]
async fn only_the_s256_method_is_accepted() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for method in ["plain", "s256", "S512", "", "S256 ", "S256,plain", "sha256"] {
        let params =
            authorize_params_with(&challenge, "st4te-4", "code_challenge_method", Some(method));
        let resp = h.authorize(&params).await;
        assert_error_redirect(&resp, "invalid_request", "st4te-4");
    }
}

// spec: B17
#[tokio::test]
async fn a_missing_method_is_not_plain_by_default() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let params = authorize_params_with(&challenge, "st4te-5", "code_challenge_method", None);
    let resp = h.authorize(&params).await;
    assert_error_redirect(&resp, "invalid_request", "st4te-5");
}

// spec: B17
#[tokio::test]
async fn plain_pkce_is_refused_even_with_a_plausible_verifier_as_challenge() {
    let h = start().await;
    // A plain-PKCE client would send the verifier itself; 43 unreserved characters.
    let plain_challenge = "A".repeat(43);
    let params = authorize_params_with(
        &plain_challenge,
        "st4te-6",
        "code_challenge_method",
        Some("plain"),
    );
    let resp = h.authorize(&params).await;
    assert_error_redirect(&resp, "invalid_request", "st4te-6");
}

// spec: B18
#[tokio::test]
async fn scope_is_ignored() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for scope in ["", "*", "non-expiring", "bogus scope !", "a b c", "\u{e9}"] {
        let mut params = authorize_params(&challenge, "s");
        params.push(("scope", scope));
        let resp = h.authorize(&params).await;
        assert_eq!(resp.status(), StatusCode::FOUND, "scope {scope:?}");
        assert_eq!(param_names(&location(&resp)), ["code", "state"]);
    }
}

// spec: B19
#[tokio::test]
async fn a_user_who_denies_gets_access_denied_with_state() {
    let h = start_with(denying()).await;
    let challenge = s256(&verifier(1));
    for state in ["st-1", "another one", "\u{e9}"] {
        let resp = h.authorize(&authorize_params(&challenge, state)).await;
        assert_eq!(resp.status(), StatusCode::FOUND);
        let loc = location(&resp);
        assert_eq!(loc.path(), "/callback");
        assert_eq!(loc.port(), Some(8888));
        assert_eq!(param(&loc, "error").as_deref(), Some("access_denied"));
        assert_eq!(param(&loc, "state").as_deref(), Some(state));
        assert_eq!(param_names(&loc), ["error", "state"]);
    }
}

// spec: B19
#[tokio::test]
async fn request_validation_comes_before_the_users_decision() {
    let h = start_with(denying()).await;
    let challenge = s256(&verifier(1));

    let params = authorize_params_with(&challenge, "s9", "response_type", Some("token"));
    assert_error_redirect(
        &h.authorize(&params).await,
        "unsupported_response_type",
        "s9",
    );
    let params = authorize_params_with(&challenge, "s9", "code_challenge_method", Some("plain"));
    assert_error_redirect(&h.authorize(&params).await, "invalid_request", "s9");
    let params = authorize_params_with(&challenge, "s9", "client_id", Some("nope"));
    assert_no_redirect(h.authorize(&params).await, "invalid_client").await;
}

// spec: B19
#[tokio::test]
async fn the_decision_can_change_while_the_server_runs() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let params = authorize_params(&challenge, "s7");

    let resp = h.authorize(&params).await;
    assert!(param(&location(&resp), "code").is_some());

    h.server.set_user_decision(UserDecision::Deny);
    assert_error_redirect(&h.authorize(&params).await, "access_denied", "s7");

    h.server.set_user_decision(UserDecision::Approve);
    let resp = h.authorize(&params).await;
    assert!(param(&location(&resp), "code").is_some());
}

// spec: B19
#[tokio::test]
async fn a_denied_request_issues_no_code() {
    let h = start_with(denying()).await;
    let login_verifier = verifier(1);
    let challenge = s256(&login_verifier);
    let resp = h.authorize(&authorize_params(&challenge, "s")).await;
    let loc = location(&resp);
    assert_eq!(param(&loc, "code"), None);
    // Nothing to exchange: any guess is unknown.
    let resp = h
        .token(&[
            ("grant_type", "authorization_code"),
            ("code", "denied"),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", login_verifier.as_str()),
        ])
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;
}

// spec: B20
#[tokio::test]
async fn a_duplicated_client_id_or_redirect_uri_is_rejected_without_redirect() {
    let h = start_with(two_client_config()).await;
    let challenge = s256(&verifier(1));

    let mut params = authorize_params(&challenge, "s");
    params.push(("client_id", CLIENT_ID));
    assert_no_redirect(
        h.authorize_raw(&raw_query(&params)).await,
        "invalid_request",
    )
    .await;

    let mut params = authorize_params(&challenge, "s");
    params.push(("client_id", "other-client-id"));
    assert_no_redirect(
        h.authorize_raw(&raw_query(&params)).await,
        "invalid_request",
    )
    .await;

    let mut params = authorize_params(&challenge, "s");
    params.push(("redirect_uri", REDIRECT_URI));
    assert_no_redirect(
        h.authorize_raw(&raw_query(&params)).await,
        "invalid_request",
    )
    .await;

    let mut params = authorize_params(&challenge, "s");
    params.push(("redirect_uri", "http://evil.example/cb"));
    assert_no_redirect(
        h.authorize_raw(&raw_query(&params)).await,
        "invalid_request",
    )
    .await;
}

// spec: B20
#[tokio::test]
async fn other_duplicated_parameters_redirect_with_invalid_request() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let other_challenge = s256(&verifier(2));
    for (name, second) in [
        ("response_type", "code"),
        ("code_challenge", other_challenge.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ] {
        let mut params = authorize_params(&challenge, "st4te-7");
        params.push((name, second));
        let resp = h.authorize_raw(&raw_query(&params)).await;
        assert_error_redirect(&resp, "invalid_request", "st4te-7");
    }
}

// spec: B20
#[tokio::test]
async fn a_duplicated_state_is_an_error_and_the_first_one_is_echoed() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let mut params = authorize_params(&challenge, "first");
    params.push(("state", "second"));
    let resp = h.authorize_raw(&raw_query(&params)).await;
    assert_error_redirect(&resp, "invalid_request", "first");
}

// spec: B21
#[tokio::test]
async fn huge_parameters_never_crash_the_server() {
    let h = start().await;
    let challenge = s256(&verifier(1));

    let huge_state = "x".repeat(1024 * 1024);
    let result = h
        .http
        .get(h.url("/authorize"))
        .query(&authorize_params(&challenge, &huge_state))
        .send()
        .await;
    // A closed connection is acceptable (the spec allows it); an answer must be a 4xx.
    if let Ok(resp) = result {
        assert!(
            resp.status().is_client_error(),
            "a 1 MiB state is refused with a 4xx, got {}",
            resp.status()
        );
    }

    let huge_uri = format!("http://127.0.0.1:8888/{}", "a".repeat(200 * 1024));
    let result = h
        .http
        .get(h.url("/authorize"))
        .query(&params_for(CLIENT_ID, &huge_uri, &challenge, "s"))
        .send()
        .await;
    if let Ok(resp) = result {
        assert!(resp.status().is_client_error(), "got {}", resp.status());
        assert!(resp.headers().get("location").is_none());
    }

    // The server is still healthy.
    let login = h.login(1).await;
    assert!(is_opaque(&login.code));
}

// spec: B21
#[tokio::test]
async fn urls_of_60_kib_are_processed_normally() {
    let h = start().await;
    let challenge = s256(&verifier(1));

    // A 60 KiB state is echoed in full, and it is not a reason to fail.
    let state = "s".repeat(60 * 1024);
    let resp = h.authorize(&authorize_params(&challenge, &state)).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert_eq!(param(&loc, "state").as_deref(), Some(state.as_str()));
    assert!(param(&loc, "code").is_some());

    // A 60 KiB redirect_uri is simply not a registered one: 400, and never redirected to.
    let big_uri = format!("http://127.0.0.1:8888/{}", "a".repeat(60 * 1024));
    let resp = h
        .authorize(&params_for(CLIENT_ID, &big_uri, &challenge, "s"))
        .await;
    assert_no_redirect(resp, "invalid_request").await;

    // Same for a 60 KiB client_id and a 60 KiB code_challenge.
    let big_id = "c".repeat(60 * 1024);
    let resp = h
        .authorize(&params_for(&big_id, REDIRECT_URI, &challenge, "s"))
        .await;
    assert_no_redirect(resp, "invalid_client").await;
    let big_challenge = "A".repeat(60 * 1024);
    let resp = h
        .authorize(&authorize_params(&big_challenge, "st4te-8"))
        .await;
    assert_error_redirect(&resp, "invalid_request", "st4te-8");
}

// spec: B61
#[tokio::test]
async fn invalid_utf8_in_the_query_is_replaced_by_the_replacement_character() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    let base = raw_query(&authorize_params_with(&challenge, "", "state", None));
    for (encoded, expected) in [
        ("a%FFb", "a\u{fffd}b"),
        ("%ff", "\u{fffd}"),
        ("%C3%28", "\u{fffd}("),
        ("x%FE%FFy", "x\u{fffd}\u{fffd}y"),
        // Valid UTF-8 written in percent-encoding stays what it is.
        ("%C3%A9", "\u{e9}"),
    ] {
        let resp = h.authorize_raw(&format!("{base}&state={encoded}")).await;
        assert_eq!(resp.status(), StatusCode::FOUND, "state {encoded}");
        let raw = header(&resp, "location").expect("a Location");
        assert!(raw.is_ascii(), "{raw}");
        let loc = location(&resp);
        assert_eq!(
            param(&loc, "state").as_deref(),
            Some(expected),
            "state {encoded}"
        );
    }
}

// spec: B21
#[tokio::test]
async fn empty_parameters_are_handled_like_missing_ones() {
    let h = start().await;
    // Empty query string: no client at all.
    assert_no_redirect(h.authorize_raw("").await, "invalid_client").await;
    // Everything present but empty.
    let resp = h
        .authorize(&[
            ("client_id", ""),
            ("redirect_uri", ""),
            ("response_type", ""),
            ("code_challenge", ""),
            ("code_challenge_method", ""),
            ("state", ""),
        ])
        .await;
    assert_no_redirect(resp, "invalid_client").await;
    // Valid client and redirect, everything else empty.
    let resp = h
        .authorize(&[
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("response_type", ""),
            ("code_challenge", ""),
            ("code_challenge_method", ""),
            ("state", ""),
        ])
        .await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert_eq!(param(&loc, "error").as_deref(), Some("invalid_request"));
    assert_eq!(param(&loc, "state").as_deref(), Some(""));
}

// spec: B23
#[tokio::test]
async fn error_redirects_omit_state_when_the_request_had_none() {
    let h = start().await;
    let challenge = s256(&verifier(1));

    let mut params = authorize_params_with(&challenge, "", "state", None);
    params.retain(|(k, _)| *k != "response_type");
    params.push(("response_type", "token"));
    let resp = h.authorize(&params).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert_eq!(
        param(&loc, "error").as_deref(),
        Some("unsupported_response_type")
    );
    assert_eq!(param_names(&loc), ["error"]);

    // Same for a refusal by the user.
    h.server.set_user_decision(UserDecision::Deny);
    let params = authorize_params_with(&challenge, "", "state", None);
    let resp = h.authorize(&params).await;
    let loc = location(&resp);
    assert_eq!(param(&loc, "error").as_deref(), Some("access_denied"));
    assert_eq!(param_names(&loc), ["error"]);
}

// spec: B22
#[tokio::test]
async fn authorize_responses_are_never_cached() {
    let h = start().await;
    let challenge = s256(&verifier(1));

    // Success redirect.
    let resp = h.authorize(&authorize_params(&challenge, "s")).await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    // Error redirect.
    let params = authorize_params_with(&challenge, "s", "response_type", Some("token"));
    let resp = h.authorize(&params).await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    // Error page (no redirect).
    let params = authorize_params_with(&challenge, "s", "client_id", Some("nope"));
    let resp = h.authorize(&params).await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    // Denied by the user.
    h.server.set_user_decision(UserDecision::Deny);
    let resp = h.authorize(&authorize_params(&challenge, "s")).await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
}

// spec: B22
#[tokio::test]
async fn method_errors_and_client_errors_of_authorize_are_never_cached_either() {
    let h = start().await;
    let challenge = s256(&verifier(1));

    // 405 for every method that is not GET / HEAD.
    for method in [
        reqwest::Method::POST,
        reqwest::Method::PUT,
        reqwest::Method::DELETE,
        reqwest::Method::PATCH,
        reqwest::Method::OPTIONS,
    ] {
        let resp = h
            .http
            .request(method.clone(), h.url("/authorize"))
            .query(&authorize_params(&challenge, "s"))
            .send()
            .await
            .expect("answer");
        assert_eq!(resp.status().as_u16(), 405, "{method}");
        assert_eq!(
            header(&resp, "cache-control").as_deref(),
            Some("no-store"),
            "{method}"
        );
    }

    // 400 for the other failures that do not redirect.
    for (name, value) in [
        ("client_id", Some("")),
        ("client_id", None),
        ("redirect_uri", Some("http://evil.example/cb")),
        ("redirect_uri", None),
    ] {
        let params = authorize_params_with(&challenge, "s", name, value);
        let resp = h.authorize(&params).await;
        assert_eq!(resp.status().as_u16(), 400, "{name} {value:?}");
        assert_eq!(
            header(&resp, "cache-control").as_deref(),
            Some("no-store"),
            "{name} {value:?}"
        );
    }

    // A duplicated client_id and a denied request.
    let mut params = authorize_params(&challenge, "s");
    params.push(("client_id", CLIENT_ID));
    let resp = h.authorize_raw(&raw_query(&params)).await;
    assert_eq!(resp.status().as_u16(), 400);
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));

    let h = start_with(denying()).await;
    let resp = h.authorize(&authorize_params(&challenge, "s")).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
}
