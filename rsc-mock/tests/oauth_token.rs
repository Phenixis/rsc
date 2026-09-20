//! `POST /oauth/token`: request format, client authentication, the authorization_code and
//! client_credentials grants, and the shape of every response.

mod common;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use common::*;
use reqwest::Response;
use rsc_mock::{MockConfig, RegisteredClient, TokenKind};

/// A request that is well-formed and asks for something that does not exist: it answers
/// `400 invalid_grant` when the client was authenticated and `401 invalid_client` when not.
const PROBE: [(&str, &str); 2] = [
    ("grant_type", "refresh_token"),
    ("refresh_token", "unknown-refresh-token"),
];

/// Form parameters, in order, repeats allowed.
type Pairs<'a> = &'a [(&'a str, &'a str)];

/// An `Authorization` header value (or none) and the form parameters of one request.
type AuthAndPairs<'a> = (Option<&'a str>, Pairs<'a>);

fn basic(id: &str, secret: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{id}:{secret}")))
}

fn raw_form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// POST with an explicit `Authorization` header (or none) and a form body.
async fn post_form_with_auth(h: &Harness, auth: Option<&str>, form: &[(&str, &str)]) -> Response {
    let mut request = h.http.post(h.url("/oauth/token")).form(form);
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    request.send().await.expect("request sent")
}

const FORM: &str = "application/x-www-form-urlencoded";

/// POST with an explicit content type (or none) and a raw body. The result is not unwrapped:
/// when the server refuses a big body early, the client may see a closed connection.
async fn post_body_result(
    h: &Harness,
    content_type: Option<&str>,
    body: String,
) -> Result<Response, reqwest::Error> {
    let mut request = h.http.post(h.url("/oauth/token")).body(body);
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    request.send().await
}

/// POST with an explicit content type (or none) and a raw body.
async fn post_body(h: &Harness, content_type: Option<&str>, body: String) -> Response {
    post_body_result(h, content_type, body)
        .await
        .expect("request sent")
}

/// A body of exactly `len` bytes: `prefix` followed by padding (the value of its last parameter).
fn form_of_len(prefix: &str, len: usize) -> String {
    assert!(len >= prefix.len(), "the prefix is longer than {len} bytes");
    format!("{prefix}{}", "t".repeat(len - prefix.len()))
}

/// Shannon entropy in bits, summed over the character positions, estimated on `samples`.
/// A random generator gives about 6 bits per position for 64 symbols; a counter gives none
/// except in its last few positions.
fn positional_entropy_bits(samples: &[String]) -> f64 {
    let total = samples.len() as f64;
    let shortest = samples.iter().map(String::len).min().expect("samples");
    (0..shortest)
        .map(|position| {
            let mut counts: HashMap<u8, usize> = HashMap::new();
            for sample in samples {
                *counts.entry(sample.as_bytes()[position]).or_default() += 1;
            }
            counts
                .values()
                .map(|&count| {
                    let p = count as f64 / total;
                    -p * p.log2()
                })
                .sum::<f64>()
        })
        .sum()
}

/// Fraction of consecutive pairs that are in ascending order (about one half when random).
fn ascending_fraction(samples: &[String]) -> f64 {
    let ascending = samples.windows(2).filter(|pair| pair[0] < pair[1]).count();
    ascending as f64 / (samples.len() - 1) as f64
}

fn code_fields(login: &Login) -> Vec<(&str, &str)> {
    vec![
        ("grant_type", "authorization_code"),
        ("code", login.code.as_str()),
        ("redirect_uri", login.redirect_uri.as_str()),
        ("code_verifier", login.verifier.as_str()),
    ]
}

/// The same fields with `name` removed (`None`) or replaced by `value`.
fn code_fields_with<'a>(
    login: &'a Login,
    name: &str,
    value: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut fields = Vec::new();
    for (k, v) in code_fields(login) {
        if k != name {
            fields.push((k, v));
        } else if let Some(value) = value {
            fields.push((k, value));
        }
    }
    fields
}

fn change_last_char(s: &str) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    let last = chars.pop().expect("non-empty");
    chars.push(if last == 'a' { 'b' } else { 'a' });
    chars.into_iter().collect()
}

// ---------------------------------------------------------------- request format

// spec: B24
#[tokio::test]
async fn the_body_must_be_form_urlencoded() {
    let h = start().await;
    let form = raw_form(&[
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("grant_type", "refresh_token"),
        ("refresh_token", "x"),
    ]);
    let json = serde_json::json!({
        "client_id": CLIENT_ID,
        "client_secret": CLIENT_SECRET,
        "grant_type": "refresh_token",
        "refresh_token": "x",
    })
    .to_string();

    for (content_type, body) in [
        (Some("application/json"), json.clone()),
        (Some("application/json; charset=utf-8"), json),
        (Some("text/plain"), form.clone()),
        (Some("multipart/form-data; boundary=x"), form.clone()),
        (Some("application/x-www-form-urlencodedx"), form.clone()),
        (
            Some("application/x-www-form-urlencoded-extra"),
            form.clone(),
        ),
        (None, form.clone()),
    ] {
        let resp = post_body(&h, content_type, body).await;
        assert_sc_error(resp, 415, "unsupported_media_type").await;
    }
}

// spec: B24
#[tokio::test]
async fn form_content_type_parameters_and_case_are_accepted() {
    let h = start().await;
    let form = raw_form(&[
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("grant_type", "refresh_token"),
        ("refresh_token", "x"),
    ]);
    for content_type in [
        "application/x-www-form-urlencoded",
        "application/x-www-form-urlencoded; charset=UTF-8",
        "application/x-www-form-urlencoded;charset=utf-8",
        "APPLICATION/X-WWW-FORM-URLENCODED",
    ] {
        let resp = post_body(&h, Some(content_type), form.clone()).await;
        // Parsed and authenticated: only the (unknown) refresh token is wrong.
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
}

// spec: B25
#[tokio::test]
async fn bodies_over_64_kib_are_refused_and_smaller_ones_are_read() {
    let h = start().await;

    // 30 KiB token: within the limit, read in full, and simply unknown.
    let big_but_ok = "t".repeat(30 * 1024);
    let resp = h
        .token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", big_but_ok.as_str()),
        ])
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;

    // 200 KiB: refused. The server may close the connection while we are still sending.
    let too_big = "t".repeat(200 * 1024);
    let result = h
        .http
        .post(h.url("/oauth/token"))
        .form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("grant_type", "refresh_token"),
            ("refresh_token", too_big.as_str()),
        ])
        .send()
        .await;
    if let Ok(resp) = result {
        assert_sc_error(resp, 413, "payload_too_large").await;
    }

    // Still healthy afterwards.
    assert_sc_error(h.token(&PROBE).await, 400, "invalid_grant").await;
}

// spec: B26
#[tokio::test]
async fn a_repeated_parameter_is_an_invalid_request() {
    let h = start().await;
    let base = [
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("grant_type", "refresh_token"),
        ("refresh_token", "x"),
    ];
    for (name, second) in [
        ("grant_type", "refresh_token"),
        ("grant_type", "authorization_code"),
        ("refresh_token", "x"),
        ("refresh_token", "y"),
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("client_secret", "wrong"),
    ] {
        let mut pairs = base.to_vec();
        pairs.push((name, second));
        let resp = post_body(
            &h,
            Some("application/x-www-form-urlencoded"),
            raw_form(&pairs),
        )
        .await;
        assert_sc_error(resp, 400, "invalid_request").await;
    }
}

// spec: B26
#[tokio::test]
async fn a_repeated_code_or_verifier_is_an_invalid_request_and_burns_nothing() {
    let h = start().await;
    let login = h.login(1).await;
    let mut pairs = vec![("client_id", CLIENT_ID), ("client_secret", CLIENT_SECRET)];
    pairs.extend(code_fields(&login));
    pairs.push(("code_verifier", login.verifier.as_str()));
    let resp = post_body(
        &h,
        Some("application/x-www-form-urlencoded"),
        raw_form(&pairs),
    )
    .await;
    assert_sc_error(resp, 400, "invalid_request").await;
    // The malformed request did not consume the code.
    h.exchange_ok(&login).await;
}

// ---------------------------------------------------------------- client authentication

// spec: B27
#[tokio::test]
async fn credentials_in_the_body_authenticate_the_client() {
    let h = start().await;
    assert_sc_error(h.token(&PROBE).await, 400, "invalid_grant").await;
}

// spec: B27
#[tokio::test]
async fn http_basic_authenticates_the_client() {
    let h = start().await;
    let resp = h.token_basic(CLIENT_ID, CLIENT_SECRET, &PROBE).await;
    assert_sc_error(resp, 400, "invalid_grant").await;
}

// spec: B27
#[tokio::test]
async fn the_basic_scheme_name_is_case_insensitive() {
    let h = start().await;
    let value = STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"));
    for scheme in ["basic", "BASIC", "bAsIc"] {
        let resp = post_form_with_auth(&h, Some(&format!("{scheme} {value}")), &PROBE).await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
}

// spec: B27
#[tokio::test]
async fn basic_credentials_may_be_accompanied_by_the_same_client_id_in_the_body() {
    let h = start().await;
    let form = [
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", "unknown-refresh-token"),
    ];
    let resp = post_form_with_auth(&h, Some(&basic(CLIENT_ID, CLIENT_SECRET)), &form).await;
    assert_sc_error(resp, 400, "invalid_grant").await;
}

// spec: B27
#[tokio::test]
async fn a_secret_that_contains_colons_and_symbols_works_in_both_forms() {
    let secret = "s3:cr:et/with+odd=chars&more%20";
    let config = MockConfig {
        clients: vec![RegisteredClient {
            client_id: "odd".to_owned(),
            client_secret: secret.to_owned(),
            redirect_uris: vec![REDIRECT_URI.to_owned()],
        }],
        ..MockConfig::default()
    };
    let h = start_with(config).await;

    // Basic: split at the first colon only.
    let resp = h.token_basic("odd", secret, &PROBE).await;
    assert_sc_error(resp, 400, "invalid_grant").await;
    // Body: form-encoded by the client, decoded by the server.
    let mut form = vec![("client_id", "odd"), ("client_secret", secret)];
    form.extend_from_slice(&PROBE);
    assert_sc_error(h.token_raw(&form).await, 400, "invalid_grant").await;
    // A truncated secret is not accepted.
    let resp = h.token_basic("odd", "s3", &PROBE).await;
    assert_sc_error(resp, 401, "invalid_client").await;
}

// spec: B28
#[tokio::test]
async fn wrong_credentials_are_401_invalid_client() {
    let h = start_with(two_client_config()).await;
    let bad_secrets = [
        "wrong",
        "",
        "mock-client-secre",
        "mock-client-secret ",
        " mock-client-secret",
        "MOCK-CLIENT-SECRET",
        "mock-client-secret2",
        CLIENT_ID,
        "other-client-secret",
    ];
    for secret in bad_secrets {
        let mut form = vec![("client_id", CLIENT_ID), ("client_secret", secret)];
        form.extend_from_slice(&PROBE);
        assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
        let resp = h.token_basic(CLIENT_ID, secret, &PROBE).await;
        assert_sc_error(resp, 401, "invalid_client").await;
    }
    for id in ["nope", "", "MOCK-CLIENT-ID", "mock-client-id "] {
        let mut form = vec![("client_id", id), ("client_secret", CLIENT_SECRET)];
        form.extend_from_slice(&PROBE);
        assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
        let resp = h.token_basic(id, CLIENT_SECRET, &PROBE).await;
        assert_sc_error(resp, 401, "invalid_client").await;
    }
    // One client's secret with another client's id.
    let resp = h
        .token_basic("other-client-id", CLIENT_SECRET, &PROBE)
        .await;
    assert_sc_error(resp, 401, "invalid_client").await;
    let resp = h
        .token_basic(CLIENT_ID, "other-client-secret", &PROBE)
        .await;
    assert_sc_error(resp, 401, "invalid_client").await;
    // Swapped.
    let mut form = vec![("client_id", CLIENT_SECRET), ("client_secret", CLIENT_ID)];
    form.extend_from_slice(&PROBE);
    assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
}

// spec: B28
#[tokio::test]
async fn invalid_client_asks_for_basic_authentication() {
    let h = start().await;
    let mut form = vec![("client_id", CLIENT_ID), ("client_secret", "wrong")];
    form.extend_from_slice(&PROBE);
    let resp = h.token_raw(&form).await;
    let www = header(&resp, "www-authenticate").expect("a WWW-Authenticate header");
    assert!(www.starts_with("Basic"), "WWW-Authenticate: {www}");
    assert_sc_error(resp, 401, "invalid_client").await;

    let resp = h.token_basic("nope", "nope", &PROBE).await;
    let www = header(&resp, "www-authenticate").expect("a WWW-Authenticate header");
    assert!(www.starts_with("Basic"), "WWW-Authenticate: {www}");
}

// spec: B28
#[tokio::test]
async fn an_unknown_client_and_a_wrong_secret_are_indistinguishable() {
    let h = start().await;
    let unknown = h.token_basic("nobody", "whatever", &PROBE).await;
    let wrong_secret = h.token_basic(CLIENT_ID, "whatever", &PROBE).await;
    assert_eq!(unknown.status(), wrong_secret.status());
    assert_eq!(
        unknown.text().await.unwrap(),
        wrong_secret.text().await.unwrap()
    );
}

// spec: B29
#[tokio::test]
async fn missing_or_incomplete_credentials_are_401_invalid_client() {
    let h = start().await;
    // Nothing at all.
    assert_sc_error(h.token_raw(&PROBE).await, 401, "invalid_client").await;
    // client_id without its secret: the secret is required even with PKCE.
    let mut form = vec![("client_id", CLIENT_ID)];
    form.extend_from_slice(&PROBE);
    assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
    // client_secret without client_id.
    let mut form = vec![("client_secret", CLIENT_SECRET)];
    form.extend_from_slice(&PROBE);
    assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
    // Empty values.
    let mut form = vec![("client_id", ""), ("client_secret", "")];
    form.extend_from_slice(&PROBE);
    assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
    // Empty body.
    let resp = post_body(&h, Some("application/x-www-form-urlencoded"), String::new()).await;
    assert_sc_error(resp, 401, "invalid_client").await;
}

// spec: B29
#[tokio::test]
async fn malformed_authorization_headers_are_401_invalid_client() {
    let h = start().await;
    let no_colon = format!("Basic {}", STANDARD.encode("mock-client-id"));
    let bearer = format!(
        "Bearer {}",
        STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    let oauth = format!(
        "OAuth {}",
        STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    for auth in [
        "Basic !!!not-base64!!!",
        "Basic",
        "Basic ",
        no_colon.as_str(),
        bearer.as_str(),
        oauth.as_str(),
        "Token abc",
        "mock-client-id:mock-client-secret",
        // base64("mock-client-id:"): an empty secret.
        "Basic bW9jay1jbGllbnQtaWQ6",
    ] {
        let resp = post_form_with_auth(&h, Some(auth), &PROBE).await;
        assert_sc_error(resp, 401, "invalid_client").await;
    }
}

// spec: B29
#[tokio::test]
async fn a_bad_authorization_header_is_not_rescued_by_body_credentials() {
    let h = start().await;
    let mut form = vec![("client_id", CLIENT_ID), ("client_secret", CLIENT_SECRET)];
    form.extend_from_slice(&PROBE);
    // An Authorization header is the client's authentication attempt. If it is not valid
    // Basic, the request fails, even though the body carries valid credentials.
    for auth in ["Basic !!!", "Bearer abc", "OAuth abc"] {
        let resp = post_form_with_auth(&h, Some(auth), &form).await;
        assert_sc_error(resp, 401, "invalid_client").await;
    }
}

// spec: B30
#[tokio::test]
async fn basic_and_a_body_secret_together_are_an_invalid_request() {
    let h = start().await;
    for body_secret in [CLIENT_SECRET, "wrong", ""] {
        let form = [
            ("client_secret", body_secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", "x"),
        ];
        let resp = post_form_with_auth(&h, Some(&basic(CLIENT_ID, CLIENT_SECRET)), &form).await;
        assert_sc_error(resp, 400, "invalid_request").await;
    }
}

// spec: B30
#[tokio::test]
async fn a_body_client_id_that_differs_from_the_basic_one_is_invalid_client() {
    let h = start_with(two_client_config()).await;
    let form = [
        ("client_id", "other-client-id"),
        ("grant_type", "refresh_token"),
        ("refresh_token", "x"),
    ];
    let resp = post_form_with_auth(&h, Some(&basic(CLIENT_ID, CLIENT_SECRET)), &form).await;
    assert_sc_error(resp, 401, "invalid_client").await;
    let form = [
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", "x"),
    ];
    let resp = post_form_with_auth(
        &h,
        Some(&basic("other-client-id", "other-client-secret")),
        &form,
    )
    .await;
    assert_sc_error(resp, 401, "invalid_client").await;
}

// spec: B31
#[tokio::test]
async fn credentials_in_the_query_string_do_not_count() {
    let h = start().await;
    let resp = h
        .http
        .post(h.url("/oauth/token"))
        .query(&[("client_id", CLIENT_ID), ("client_secret", CLIENT_SECRET)])
        .form(&PROBE)
        .send()
        .await
        .unwrap();
    assert_sc_error(resp, 401, "invalid_client").await;

    // Grant parameters in the query string do not count either.
    let resp = h
        .http
        .post(h.url("/oauth/token"))
        .query(&PROBE)
        .form(&[("client_id", CLIENT_ID), ("client_secret", CLIENT_SECRET)])
        .send()
        .await
        .unwrap();
    assert_sc_error(resp, 400, "invalid_request").await;
}

// spec: B32
#[tokio::test]
async fn client_authentication_comes_before_grant_validation() {
    let h = start().await;
    for form in [
        vec![
            ("client_id", CLIENT_ID),
            ("client_secret", "wrong"),
            ("grant_type", "bogus"),
        ],
        vec![("client_id", CLIENT_ID), ("client_secret", "wrong")],
        vec![
            ("client_id", "nope"),
            ("client_secret", "nope"),
            ("grant_type", "authorization_code"),
        ],
    ] {
        assert_sc_error(h.token_raw(&form).await, 401, "invalid_client").await;
    }
}

// spec: B32
#[tokio::test]
async fn a_missing_or_empty_grant_type_is_an_invalid_request() {
    let h = start().await;
    assert_sc_error(h.token(&[]).await, 400, "invalid_request").await;
    assert_sc_error(h.token(&[("grant_type", "")]).await, 400, "invalid_request").await;
    let resp = h
        .token(&[
            ("refresh_token", "x"),
            ("code", "y"),
            ("code_verifier", "z"),
        ])
        .await;
    assert_sc_error(resp, 400, "invalid_request").await;
    let resp = h.token_basic(CLIENT_ID, CLIENT_SECRET, &[]).await;
    assert_sc_error(resp, 400, "invalid_request").await;
}

// spec: B32
#[tokio::test]
async fn unknown_grant_types_are_unsupported_grant() {
    let h = start().await;
    for grant in [
        "password",
        "implicit",
        "authorization-code",
        "AUTHORIZATION_CODE",
        "Refresh_Token",
        "client_credentials ",
        "urn:ietf:params:oauth:grant-type:jwt-bearer",
        "urn:ietf:params:oauth:grant-type:device_code",
        "\u{e9}",
        "x",
    ] {
        let resp = h.token(&[("grant_type", grant)]).await;
        assert_sc_error(resp, 400, "unsupported_grant").await;
        let resp = h
            .token_basic(CLIENT_ID, CLIENT_SECRET, &[("grant_type", grant)])
            .await;
        assert_sc_error(resp, 400, "unsupported_grant").await;
    }
}

// spec: B33
#[tokio::test]
async fn a_token_response_has_exactly_the_documented_shape() {
    let h = start().await;
    let login = h.login(1).await;
    let resp = h.exchange(&login).await;

    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(header(&resp, "content-type").as_deref(), Some(JSON_UTF8));
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    assert_eq!(header(&resp, "pragma").as_deref(), Some("no-cache"));

    let body: serde_json::Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    let object = body.as_object().expect("a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "access_token",
            "expires_in",
            "refresh_token",
            "scope",
            "token_type"
        ]
    );
    assert!(object["access_token"].is_string());
    assert!(object["refresh_token"].is_string());
    assert!(
        object["expires_in"].is_u64(),
        "expires_in is a JSON integer"
    );
    assert_eq!(object["expires_in"].as_u64(), Some(3599));
    assert_eq!(object["scope"], serde_json::json!(""));
    assert_eq!(object["token_type"], serde_json::json!("bearer"));
}

// spec: B33
#[tokio::test]
async fn errors_of_the_token_endpoint_are_never_cached_either() {
    let h = start().await;
    // 400
    let resp = h.token(&PROBE).await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    assert_eq!(header(&resp, "content-type").as_deref(), Some(JSON_UTF8));
    // 401
    let resp = h.token_raw(&PROBE).await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    // 415
    let resp = post_body(&h, Some("text/plain"), "x".to_owned()).await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    // client_credentials
    let resp = h.client_credentials().await;
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    assert_eq!(header(&resp, "pragma").as_deref(), Some("no-cache"));
}

// spec: B34
#[tokio::test]
async fn codes_and_tokens_are_opaque_and_unique() {
    let h = start().await;
    let mut seen = HashSet::new();
    for n in 0..30 {
        let login = h.login(n).await;
        assert!(is_opaque(&login.code), "code {:?}", login.code);
        assert!(seen.insert(login.code.clone()), "codes are unique");
        let tokens = h.exchange_ok(&login).await;
        for token in [tokens.access_token.clone(), tokens.refresh().to_owned()] {
            assert!(is_opaque(&token), "token {token:?}");
            assert!(seen.insert(token), "codes and tokens never collide");
        }
    }
    for _ in 0..10 {
        let app = read_tokens(h.client_credentials().await).await;
        assert!(is_opaque(&app.access_token));
        assert!(seen.insert(app.access_token), "app tokens are unique too");
    }
}

// spec: B35
#[tokio::test]
async fn expires_in_follows_the_configured_lifetime() {
    for lifetime in [1u64, 20, 3599, 7200] {
        let config = MockConfig {
            access_token_lifetime: Duration::from_secs(lifetime),
            ..MockConfig::default()
        };
        let h = start_with(config).await;
        let tokens = h.session(1).await;
        assert_eq!(tokens.expires_in, lifetime);
        let refreshed = h.refresh_ok(tokens.refresh()).await;
        assert_eq!(refreshed.expires_in, lifetime);
        let app = read_tokens(h.client_credentials().await).await;
        assert_eq!(app.expires_in, lifetime);
    }
}

// spec: B35
#[tokio::test]
async fn expires_in_is_the_full_lifetime_at_issuance_whatever_the_clock_says() {
    let h = start().await;
    h.server.advance_clock(Duration::from_secs(50_000));
    let tokens = h.session(1).await;
    assert_eq!(tokens.expires_in, 3599);
    let info = h.server.token_info(&tokens.access_token).expect("valid");
    assert_eq!(info.remaining, Duration::from_secs(3599));
}

// ---------------------------------------------------------------- authorization_code

// spec: B36
#[tokio::test]
async fn every_authorization_code_parameter_is_required() {
    let h = start().await;
    let login = h.login(1).await;
    for name in ["code", "redirect_uri", "code_verifier"] {
        let resp = h.token(&code_fields_with(&login, name, None)).await;
        assert_sc_error(resp, 400, "invalid_request").await;
        let resp = h.token(&code_fields_with(&login, name, Some(""))).await;
        assert_sc_error(resp, 400, "invalid_request").await;
    }
    // None of the failed attempts consumed the code.
    h.exchange_ok(&login).await;
}

// spec: B36
#[tokio::test]
async fn code_parameters_are_required_with_basic_authentication_too() {
    let h = start().await;
    let login = h.login(1).await;
    for name in ["code", "redirect_uri", "code_verifier"] {
        let fields = code_fields_with(&login, name, None);
        let resp = h.token_basic(CLIENT_ID, CLIENT_SECRET, &fields).await;
        assert_sc_error(resp, 400, "invalid_request").await;
    }
    let resp = h
        .token_basic(CLIENT_ID, CLIENT_SECRET, &code_fields(&login))
        .await;
    read_tokens(resp).await;
}

// spec: B37
#[tokio::test]
async fn unknown_codes_are_invalid_grant() {
    let h = start().await;
    let login = h.login(1).await;
    let other = h.session(2).await;
    let long = "c".repeat(30 * 1024);
    let neighbour = change_last_char(&login.code);
    let uppercase = login.code.to_uppercase();
    let padded = format!("{} ", login.code);
    let bad = vec![
        "nope".to_owned(),
        "0".to_owned(),
        "c".repeat(32),
        neighbour,
        uppercase,
        padded,
        long,
        // Credentials of other kinds are not codes.
        other.access_token.clone(),
        other.refresh().to_owned(),
    ];
    for code in &bad {
        if *code == login.code {
            continue;
        }
        let resp = h
            .token(&code_fields_with(&login, "code", Some(code.as_str())))
            .await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
    // The real one is untouched by all of that.
    h.exchange_ok(&login).await;
}

// spec: B38
#[tokio::test]
async fn the_redirect_uri_must_be_the_one_used_at_authorize() {
    let h = start().await;
    let login = h.login(1).await;
    for uri in [
        "http://localhost:8888/callback",
        "http://127.0.0.1:8888/callback/",
        "http://127.0.0.1:9999/callback",
        "http://127.0.0.1:8888/Callback",
        "https://127.0.0.1:8888/callback",
        "http://127.0.0.1:8888/callback?x=1",
        "http://evil.example/callback",
    ] {
        let resp = h
            .token(&code_fields_with(&login, "redirect_uri", Some(uri)))
            .await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
    // A failed attempt does not burn the code.
    h.exchange_ok(&login).await;
}

// spec: B38
#[tokio::test]
async fn another_registered_redirect_uri_is_still_a_mismatch() {
    let second = "http://127.0.0.1:8899/second";
    let mut config = MockConfig::default();
    config.clients[0].redirect_uris.push(second.to_owned());
    let h = start_with(config).await;
    let login = h.login(1).await; // authorized with REDIRECT_URI
    let resp = h
        .token(&code_fields_with(&login, "redirect_uri", Some(second)))
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;
    h.exchange_ok(&login).await;

    // And the other way round.
    let challenge = s256(&verifier(5));
    let resp = h
        .authorize(&[
            ("client_id", CLIENT_ID),
            ("redirect_uri", second),
            ("response_type", "code"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", "s"),
        ])
        .await;
    let code = param(&location(&resp), "code").expect("code");
    let login2 = Login {
        code,
        verifier: verifier(5),
        state: "s".to_owned(),
        redirect_uri: REDIRECT_URI.to_owned(),
    };
    assert_sc_error(h.exchange(&login2).await, 400, "invalid_grant").await;
    let login2 = Login {
        redirect_uri: second.to_owned(),
        ..login2
    };
    h.exchange_ok(&login2).await;
}

// spec: B39
#[tokio::test]
async fn a_wrong_or_malformed_verifier_is_invalid_grant_and_does_not_burn_the_code() {
    let h = start().await;
    let login = h.login(1).await;
    let almost = change_last_char(&login.verifier);
    let challenge_as_verifier = s256(&login.verifier);
    let uppercase = login.verifier.to_uppercase();
    let short = login.verifier[..42].to_owned();
    let long_129 = format!("{}{}", login.verifier, "x".repeat(86));
    assert_eq!(long_129.len(), 129);
    let huge = "a".repeat(5000);
    let with_space = format!("{} ", &login.verifier[..42]);
    let with_plus = format!("{}+", &login.verifier[..42]);
    let with_slash = format!("{}/", &login.verifier[..42]);
    let with_equals = format!("{}=", &login.verifier[..42]);
    let with_accent = format!("{}\u{e9}", &login.verifier[..42]);
    let other_login_verifier = verifier(2);
    let padded = format!("{} ", login.verifier);
    let bad = [
        other_login_verifier,
        almost,
        challenge_as_verifier,
        uppercase,
        short,
        long_129,
        huge,
        with_space,
        with_plus,
        with_slash,
        with_equals,
        with_accent,
        padded,
    ];
    for v in &bad {
        assert_ne!(*v, login.verifier);
        let resp = h
            .token(&code_fields_with(&login, "code_verifier", Some(v.as_str())))
            .await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
    h.exchange_ok(&login).await;
}

// spec: B39
#[tokio::test]
async fn verifiers_of_43_to_128_characters_work() {
    let h = start().await;
    for (seed, len) in [(1, 43), (2, 44), (3, 64), (4, 100), (5, 127), (6, 128)] {
        let v = verifier_of_len(len, seed);
        assert_eq!(v.len(), len);
        let login = h.login_with(&v, "s").await;
        let tokens = h.exchange_ok(&login).await;
        assert!(h.server.token_info(&tokens.access_token).is_some());
    }
}

// spec: B39
#[tokio::test]
async fn the_rfc_7636_appendix_b_pair_works_end_to_end() {
    let h = start().await;
    let rfc_verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let rfc_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert_eq!(s256(rfc_verifier), rfc_challenge);

    let resp = h.authorize(&authorize_params(rfc_challenge, "s")).await;
    let code = param(&location(&resp), "code").expect("code");
    let login = Login {
        code,
        verifier: rfc_verifier.to_owned(),
        state: "s".to_owned(),
        redirect_uri: REDIRECT_URI.to_owned(),
    };
    h.exchange_ok(&login).await;
}

// spec: B39
#[tokio::test]
async fn a_code_is_bound_to_the_challenge_of_its_own_authorization() {
    let h = start().await;
    let a = h.login(1).await;
    let b = h.login(2).await;
    // Each code needs its own verifier.
    let crossed = Login {
        code: a.code.clone(),
        verifier: b.verifier.clone(),
        state: String::new(),
        redirect_uri: REDIRECT_URI.to_owned(),
    };
    assert_sc_error(h.exchange(&crossed).await, 400, "invalid_grant").await;
    h.exchange_ok(&a).await;
    h.exchange_ok(&b).await;
}

// spec: B40
#[tokio::test]
async fn a_code_belongs_to_the_client_that_asked_for_it() {
    let h = start_with(two_client_config()).await;
    let login = h.login(1).await; // default client
    let other_redirect = "http://127.0.0.1:9999/other";

    // The other client, authenticated, tries to redeem it, with either redirect URI.
    for redirect in [REDIRECT_URI, other_redirect] {
        let mut fields = code_fields_with(&login, "redirect_uri", Some(redirect));
        fields.push(("client_id", "other-client-id"));
        fields.push(("client_secret", "other-client-secret"));
        assert_sc_error(h.token_raw(&fields).await, 400, "invalid_grant").await;
    }
    let resp = h
        .token_basic(
            "other-client-id",
            "other-client-secret",
            &code_fields(&login),
        )
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;

    // The owner is not affected.
    let tokens = h.exchange_ok(&login).await;
    // A replay by the other client is refused and does not revoke the owner's tokens.
    let resp = h
        .token_basic(
            "other-client-id",
            "other-client-secret",
            &code_fields(&login),
        )
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;
    assert!(h.server.token_info(&tokens.access_token).is_some());
    h.refresh_ok(tokens.refresh()).await;
}

// spec: B40
#[tokio::test]
async fn the_second_client_can_run_its_own_flow() {
    let h = start_with(two_client_config()).await;
    let redirect = "http://127.0.0.1:9999/other";
    let v = verifier(9);
    let challenge = s256(&v);
    let resp = h
        .authorize(&[
            ("client_id", "other-client-id"),
            ("redirect_uri", redirect),
            ("response_type", "code"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", "s"),
        ])
        .await;
    let code = param(&location(&resp), "code").expect("code");
    let resp = h
        .token_basic(
            "other-client-id",
            "other-client-secret",
            &[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", redirect),
                ("code_verifier", v.as_str()),
            ],
        )
        .await;
    let tokens = read_tokens(resp).await;
    let info = h.server.token_info(&tokens.access_token).expect("valid");
    assert_eq!(info.client_id, "other-client-id");
    assert_eq!(info.kind, TokenKind::User);
}

// spec: B41
#[tokio::test]
async fn a_code_can_be_redeemed_only_once() {
    let h = start().await;
    for n in 1..=3 {
        let login = h.login(n).await;
        h.exchange_ok(&login).await;
        for _ in 0..2 {
            assert_sc_error(h.exchange(&login).await, 400, "invalid_grant").await;
        }
    }
}

// spec: B42
#[tokio::test]
async fn replaying_a_code_revokes_the_tokens_issued_from_it() {
    let h = start().await;
    let login = h.login(1).await;
    let t1 = h.exchange_ok(&login).await;
    // A rotation in between: the whole family must go, not just the first pair.
    let t2 = h.refresh_ok(t1.refresh()).await;
    let t3 = h.refresh_ok(t2.refresh()).await;
    for t in [&t1, &t2, &t3] {
        assert!(h.server.token_info(&t.access_token).is_some());
    }

    assert_sc_error(h.exchange(&login).await, 400, "invalid_grant").await;

    for t in [&t1, &t2, &t3] {
        assert!(
            h.server.token_info(&t.access_token).is_none(),
            "access tokens of the family are revoked"
        );
    }
    // The latest refresh token was never used, so only the revocation can refuse it.
    assert_sc_error(h.refresh(t3.refresh()).await, 400, "invalid_grant").await;
}

// spec: B42
#[tokio::test]
async fn a_replay_with_a_wrong_verifier_or_redirect_uri_revokes_as_well() {
    let h = start().await;
    let login = h.login(1).await;
    let tokens = h.exchange_ok(&login).await;
    let resp = h
        .token(&code_fields_with(
            &login,
            "code_verifier",
            Some(verifier(7).as_str()),
        ))
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;
    assert!(h.server.token_info(&tokens.access_token).is_none());
    assert_sc_error(h.refresh(tokens.refresh()).await, 400, "invalid_grant").await;

    let login = h.login(2).await;
    let tokens = h.exchange_ok(&login).await;
    let resp = h
        .token(&code_fields_with(
            &login,
            "redirect_uri",
            Some("http://localhost:8888/callback"),
        ))
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;
    assert!(h.server.token_info(&tokens.access_token).is_none());
}

// spec: B42
#[tokio::test]
async fn replaying_one_code_leaves_other_sessions_alone() {
    let h = start().await;
    let login_a = h.login(1).await;
    let a = h.exchange_ok(&login_a).await;
    let b = h.session(2).await;
    let app = read_tokens(h.client_credentials().await).await;

    assert_sc_error(h.exchange(&login_a).await, 400, "invalid_grant").await;

    assert!(h.server.token_info(&a.access_token).is_none());
    assert!(h.server.token_info(&b.access_token).is_some());
    assert!(h.server.token_info(&app.access_token).is_some());
    h.refresh_ok(b.refresh()).await;
}

// spec: B43
#[tokio::test]
async fn a_code_is_valid_for_ten_minutes_of_mock_time() {
    let h = start().await;
    let login = h.login(1).await;
    h.server.advance_clock(Duration::from_secs(599));
    h.exchange_ok(&login).await;

    let h = start().await;
    let login = h.login(1).await;
    h.server.advance_clock(Duration::from_secs(600));
    assert_sc_error(h.exchange(&login).await, 400, "invalid_grant").await;
    // It stays expired.
    assert_sc_error(h.exchange(&login).await, 400, "invalid_grant").await;

    let h = start().await;
    let login = h.login(1).await;
    h.server.advance_clock(Duration::from_secs(3600));
    assert_sc_error(h.exchange(&login).await, 400, "invalid_grant").await;
}

// spec: B43
#[tokio::test]
async fn the_code_lifetime_is_configurable_and_counts_from_issuance() {
    let config = MockConfig {
        authorization_code_lifetime: Duration::from_secs(30),
        ..MockConfig::default()
    };
    let h = start_with(config.clone()).await;
    // Time passing before the authorization does not shorten the code's life.
    h.server.advance_clock(Duration::from_secs(1000));
    let login = h.login(1).await;
    h.server.advance_clock(Duration::from_secs(29));
    h.exchange_ok(&login).await;

    let h = start_with(config).await;
    h.server.advance_clock(Duration::from_secs(1000));
    let login = h.login(1).await;
    h.server.advance_clock(Duration::from_secs(30));
    assert_sc_error(h.exchange(&login).await, 400, "invalid_grant").await;
}

// spec: B43
#[tokio::test]
async fn an_expired_code_does_not_lose_the_others() {
    let h = start().await;
    let old = h.login(1).await;
    h.server.advance_clock(Duration::from_secs(400));
    let recent = h.login(2).await;
    h.server.advance_clock(Duration::from_secs(300)); // old: 700 s, recent: 300 s
    assert_sc_error(h.exchange(&old).await, 400, "invalid_grant").await;
    h.exchange_ok(&recent).await;
}

// spec: B44
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_redemptions_of_one_code_yield_exactly_one_success() {
    let h = start().await;
    for n in 1..=3 {
        let login = h.login(n).await;
        let results = h.race(24, &code_fields(&login)).await;
        let successes = results.iter().filter(|(status, _)| *status == 200).count();
        assert_eq!(successes, 1, "exactly one winner: {results:?}");
        for (status, body) in results.iter().filter(|(status, _)| *status != 200) {
            assert_eq!(*status, 400);
            assert!(body.contains("invalid_grant"), "{body}");
        }
    }
}

// ---------------------------------------------------------------- client_credentials

// spec: B53
#[tokio::test]
async fn client_credentials_needs_basic_authentication() {
    let h = start().await;
    let resp = h.client_credentials().await;
    assert_eq!(resp.status().as_u16(), 200);
    let resp = h
        .token_basic(CLIENT_ID, "wrong", &[("grant_type", "client_credentials")])
        .await;
    assert_sc_error(resp, 401, "invalid_client").await;
    let resp = h
        .token_basic(
            "nope",
            CLIENT_SECRET,
            &[("grant_type", "client_credentials")],
        )
        .await;
    assert_sc_error(resp, 401, "invalid_client").await;
}

// spec: B53
#[tokio::test]
async fn client_credentials_refuses_credentials_in_the_body() {
    let h = start().await;
    let resp = h.token(&[("grant_type", "client_credentials")]).await;
    let www = header(&resp, "www-authenticate").expect("a WWW-Authenticate header");
    assert!(www.starts_with("Basic"), "{www}");
    assert_sc_error(resp, 401, "invalid_client").await;

    let resp = h.token_raw(&[("grant_type", "client_credentials")]).await;
    assert_sc_error(resp, 401, "invalid_client").await;
}

// spec: B54
#[tokio::test]
async fn client_credentials_returns_an_app_token_without_refresh_token() {
    let h = start().await;
    let resp = h.client_credentials().await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(header(&resp, "content-type").as_deref(), Some(JSON_UTF8));

    let body: serde_json::Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    let object = body.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["access_token", "expires_in", "scope", "token_type"]);
    assert!(
        !object.contains_key("refresh_token"),
        "no refresh_token, not even null"
    );
    assert_eq!(object["expires_in"].as_u64(), Some(3599));
    assert_eq!(object["scope"], serde_json::json!(""));
    assert_eq!(object["token_type"], serde_json::json!("bearer"));

    let token = object["access_token"].as_str().unwrap();
    assert!(is_opaque(token));
    let info = h.server.token_info(token).expect("valid");
    assert_eq!(info.kind, TokenKind::App);
    assert_eq!(info.client_id, CLIENT_ID);
    assert_eq!(info.remaining, Duration::from_secs(3599));
}

// spec: B54
#[tokio::test]
async fn client_credentials_ignores_extra_parameters_and_expires_on_schedule() {
    let h = start().await;
    let resp = h
        .token_basic(
            CLIENT_ID,
            CLIENT_SECRET,
            &[
                ("grant_type", "client_credentials"),
                ("scope", "whatever"),
                ("code", "ignored"),
                ("redirect_uri", "http://elsewhere.example/"),
            ],
        )
        .await;
    let app = read_tokens(resp).await;
    assert!(app.refresh_token.is_none());

    h.server.advance_clock(Duration::from_secs(3598));
    assert!(h.server.token_info(&app.access_token).is_some());
    h.server.advance_clock(Duration::from_secs(1));
    assert!(h.server.token_info(&app.access_token).is_none());
}

// spec: B54
#[tokio::test]
async fn a_user_token_is_not_an_app_token() {
    let h = start().await;
    let user = h.session(1).await;
    let app = read_tokens(h.client_credentials().await).await;
    assert_eq!(
        h.server.token_info(&user.access_token).unwrap().kind,
        TokenKind::User
    );
    assert_eq!(
        h.server.token_info(&app.access_token).unwrap().kind,
        TokenKind::App
    );
    assert_ne!(user.access_token, app.access_token);
}

// spec: B55
#[tokio::test]
async fn credentials_of_the_wrong_kind_are_invalid_grant() {
    let h = start().await;
    let user = h.session(1).await;
    let app = read_tokens(h.client_credentials().await).await;
    let login = h.login(2).await;

    // Access tokens and codes are not refresh tokens.
    for wrong in [
        user.access_token.as_str(),
        app.access_token.as_str(),
        login.code.as_str(),
    ] {
        assert_sc_error(h.refresh(wrong).await, 400, "invalid_grant").await;
    }
    // Refresh tokens and access tokens are not codes.
    for wrong in [
        user.refresh(),
        user.access_token.as_str(),
        app.access_token.as_str(),
    ] {
        let resp = h
            .token(&code_fields_with(&login, "code", Some(wrong)))
            .await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
    // Nothing was consumed by those attempts.
    h.exchange_ok(&login).await;
    h.refresh_ok(user.refresh()).await;
}

// ---------------------------------------------------------------- empty and huge input

// spec: B59
#[tokio::test]
async fn empty_and_huge_values_are_handled() {
    let h = start().await;
    let login = h.login(1).await;

    // Every field empty, credentials fine.
    let resp = h
        .token(&[
            ("grant_type", "authorization_code"),
            ("code", ""),
            ("redirect_uri", ""),
            ("code_verifier", ""),
        ])
        .await;
    assert_sc_error(resp, 400, "invalid_request").await;
    let resp = h
        .token(&[("grant_type", "refresh_token"), ("refresh_token", "")])
        .await;
    assert_sc_error(resp, 400, "invalid_request").await;

    // Huge but within the body limit.
    let huge = "z".repeat(20 * 1024);
    for name in ["code", "redirect_uri", "code_verifier"] {
        let resp = h
            .token(&code_fields_with(&login, name, Some(huge.as_str())))
            .await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
    let resp = h
        .token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", huge.as_str()),
        ])
        .await;
    assert_sc_error(resp, 400, "invalid_grant").await;

    // Unknown extra fields are ignored.
    let mut fields = code_fields(&login);
    fields.push(("scope", "ignored"));
    fields.push(("unknown", "ignored"));
    read_tokens(h.token(&fields).await).await;
}

// spec: B59
#[tokio::test]
async fn non_utf8_and_malformed_percent_encoding_do_not_crash_the_server() {
    let h = start().await;
    let bodies = [
        "grant_type=%ff%fe&client_id=%zz&client_secret=%".to_owned(),
        "&&&&==&=".to_owned(),
        "grant_type".to_owned(),
        "%00=%00".to_owned(),
        String::new(),
    ];
    for body in bodies {
        let resp = post_body(&h, Some("application/x-www-form-urlencoded"), body).await;
        assert!(
            resp.status().is_client_error(),
            "malformed input is a client error, got {}",
            resp.status()
        );
    }
    assert_sc_error(h.token(&PROBE).await, 400, "invalid_grant").await;
}

// spec: B59
#[tokio::test]
async fn malformed_values_with_valid_credentials_are_handled_precisely() {
    let h = start().await;
    let creds = format!("client_id={CLIENT_ID}&client_secret={CLIENT_SECRET}");
    // Invalid UTF-8 and invalid percent-encoding are lossily decoded, never an error by
    // themselves: the value is simply not a known token.
    for tail in [
        "grant_type=refresh_token&refresh_token=%ff%fe",
        "grant_type=refresh_token&refresh_token=%zz%",
        "grant_type=refresh_token&refresh_token=%00",
        "grant_type=refresh_token&refresh_token=%",
        "grant_type=refresh_token&refresh_token=a%2",
        "grant_type=refresh_token&refresh_token=x&%ff=%fe&&=&stray",
    ] {
        let resp = post_body(&h, Some(FORM), format!("{creds}&{tail}")).await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
    for grant in ["%00", "%ff", "%zz", "a%20b"] {
        let resp = post_body(&h, Some(FORM), format!("{creds}&grant_type={grant}")).await;
        assert_sc_error(resp, 400, "unsupported_grant").await;
    }
    // Stray separators do not hide the valid parameters around them.
    let body = format!("&&{creds}&&=&grant_type=refresh_token&=&refresh_token=x&");
    let resp = post_body(&h, Some(FORM), body).await;
    assert_sc_error(resp, 400, "invalid_grant").await;
}

// spec: B61
#[tokio::test]
async fn invalid_utf8_in_a_form_body_is_replaced_by_the_replacement_character() {
    let secret = "s\u{fffd}x";
    let config = MockConfig {
        clients: vec![RegisteredClient {
            client_id: "odd".to_owned(),
            client_secret: secret.to_owned(),
            redirect_uris: vec![REDIRECT_URI.to_owned()],
        }],
        ..MockConfig::default()
    };
    let h = start_with(config).await;
    let tail = "grant_type=refresh_token&refresh_token=zzz";

    // Undecodable bytes become U+FFFD, exactly as if the secret had been sent as such.
    for encoded in ["s%FFx", "s%ffx", "s%FEx", "s%C3x"] {
        let body = format!("client_id=odd&client_secret={encoded}&{tail}");
        let resp = post_body(&h, Some(FORM), body).await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }
    let body = format!("client_id=odd&client_secret=s%EF%BF%BDx&{tail}");
    assert_sc_error(post_body(&h, Some(FORM), body).await, 400, "invalid_grant").await;

    // Anything else is not the secret.
    for encoded in ["s%C3%28x", "s%25FFx", "sx", "s%FF%FFx", "s%FF"] {
        let body = format!("client_id=odd&client_secret={encoded}&{tail}");
        let resp = post_body(&h, Some(FORM), body).await;
        assert_sc_error(resp, 401, "invalid_client").await;
    }
}

// ---------------------------------------------------------------- processing order

// spec: B24
#[tokio::test]
async fn the_content_type_is_checked_before_credentials_duplicates_and_grant() {
    let h = start().await;
    let bodies = [
        // Wrong credentials.
        raw_form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", "wrong"),
            ("grant_type", "refresh_token"),
            ("refresh_token", "x"),
        ]),
        // No credentials at all.
        raw_form(&PROBE),
        // Repeated parameters.
        raw_form(&[("grant_type", "a"), ("grant_type", "b"), ("client_id", "x")]),
        // Valid credentials and a bogus grant.
        raw_form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("grant_type", "bogus"),
        ]),
        String::new(),
    ];
    for body in bodies {
        for content_type in [
            Some("application/json"),
            Some("text/plain"),
            Some("application/x-www-form-urlencodedx"),
            None,
        ] {
            let resp = post_body(&h, content_type, body.clone()).await;
            assert_sc_error(resp, 415, "unsupported_media_type").await;
        }
    }
    // Same with a malformed or wrong Authorization header.
    let wrong_basic = basic("nope", "nope");
    for auth in ["Bearer abc", "Basic !!!", wrong_basic.as_str()] {
        let resp = h
            .http
            .post(h.url("/oauth/token"))
            .header("content-type", "application/json")
            .header("authorization", auth)
            .body("{}")
            .send()
            .await
            .expect("request sent");
        assert_sc_error(resp, 415, "unsupported_media_type").await;
    }
}

// spec: B25
#[tokio::test]
async fn a_body_of_exactly_64_kib_is_read_and_one_byte_more_is_refused() {
    let h = start().await;
    let prefix = format!(
        "client_id={CLIENT_ID}&client_secret={CLIENT_SECRET}&grant_type=refresh_token&refresh_token="
    );
    let exact = form_of_len(&prefix, 65_536);
    assert_eq!(exact.len(), 65_536);
    // Read in full: the (huge) refresh token is simply unknown.
    assert_sc_error(post_body(&h, Some(FORM), exact).await, 400, "invalid_grant").await;

    // Also with a bit less, and with a big value in the middle of the form.
    for len in [65_535, 60_000, 32_768] {
        let body = form_of_len(&prefix, len);
        let resp = post_body(&h, Some(FORM), body).await;
        assert_sc_error(resp, 400, "invalid_grant").await;
    }

    // One byte more: refused. The server may close the connection while we are still sending,
    // so a client-side error is tolerated; an answer must be the 413.
    for len in [65_537, 65_600, 100_000] {
        let body = form_of_len(&prefix, len);
        if let Ok(resp) = post_body_result(&h, Some(FORM), body).await {
            assert_sc_error(resp, 413, "payload_too_large").await;
        }
    }
    assert_sc_error(h.token(&PROBE).await, 400, "invalid_grant").await;
}

// spec: B25
#[tokio::test]
async fn an_oversized_body_is_refused_before_duplicates_and_credentials() {
    let h = start().await;
    let wrong_creds = "client_id=nope&client_secret=nope&grant_type=refresh_token&refresh_token=";
    let no_creds = "grant_type=refresh_token&refresh_token=";
    let duplicates = "grant_type=refresh_token&grant_type=refresh_token&refresh_token=";
    let bogus_grant = "client_id=nope&client_secret=nope&grant_type=bogus&x=";
    for prefix in [wrong_creds, no_creds, duplicates, bogus_grant] {
        for len in [65_537, 70_000] {
            let body = form_of_len(prefix, len);
            if let Ok(resp) = post_body_result(&h, Some(FORM), body).await {
                assert_sc_error(resp, 413, "payload_too_large").await;
            }
        }
    }
    // The content type still comes first: a wrong one is a 415, never a 413.
    for len in [65_537, 70_000] {
        let body = form_of_len(wrong_creds, len);
        if let Ok(resp) = post_body_result(&h, Some("text/plain"), body).await {
            assert_sc_error(resp, 415, "unsupported_media_type").await;
        }
    }
    assert_sc_error(h.token(&PROBE).await, 400, "invalid_grant").await;
}

// spec: B25
#[tokio::test]
async fn an_oversized_request_consumes_nothing() {
    let h = start().await;
    let login = h.login(1).await;
    let tokens = h.session(2).await;
    let redirect: String = url::form_urlencoded::byte_serialize(REDIRECT_URI.as_bytes()).collect();
    let prefix = format!(
        "client_id={CLIENT_ID}&client_secret={CLIENT_SECRET}&grant_type=authorization_code\
         &code={}&redirect_uri={redirect}&code_verifier={}&pad=",
        login.code, login.verifier
    );
    let body = form_of_len(&prefix, 70_000);
    if let Ok(resp) = post_body_result(&h, Some(FORM), body).await {
        assert_sc_error(resp, 413, "payload_too_large").await;
    }
    // The valid code inside the refused request was not redeemed.
    h.exchange_ok(&login).await;

    let prefix = format!(
        "client_id={CLIENT_ID}&client_secret={CLIENT_SECRET}&grant_type=refresh_token\
         &refresh_token={}&pad=",
        tokens.refresh()
    );
    let body = form_of_len(&prefix, 70_000);
    if let Ok(resp) = post_body_result(&h, Some(FORM), body).await {
        assert_sc_error(resp, 413, "payload_too_large").await;
    }
    h.refresh_ok(tokens.refresh()).await;
}

// spec: B26
#[tokio::test]
async fn repeated_parameters_are_refused_before_client_authentication() {
    let h = start().await;
    let wrong_basic = basic("nope", "nope");
    let cases: [AuthAndPairs<'_>; 8] = [
        // No credentials at all.
        (
            None,
            &[
                ("grant_type", "refresh_token"),
                ("grant_type", "refresh_token"),
                ("refresh_token", "x"),
            ],
        ),
        // Wrong secret in the body.
        (
            None,
            &[
                ("client_id", CLIENT_ID),
                ("client_secret", "wrong"),
                ("grant_type", "refresh_token"),
                ("refresh_token", "x"),
                ("refresh_token", "y"),
            ],
        ),
        // Unknown client, repeated id.
        (
            None,
            &[
                ("client_id", "nope"),
                ("client_id", "nope"),
                ("client_secret", "nope"),
                ("grant_type", "refresh_token"),
                ("refresh_token", "x"),
            ],
        ),
        // Repeated secrets, both wrong.
        (
            None,
            &[
                ("client_id", CLIENT_ID),
                ("client_secret", "wrong"),
                ("client_secret", "wronger"),
                ("grant_type", "refresh_token"),
                ("refresh_token", "x"),
            ],
        ),
        // A bogus, repeated grant: not `unsupported_grant`.
        (
            None,
            &[
                ("client_id", "nope"),
                ("client_secret", "nope"),
                ("grant_type", "bogus"),
                ("grant_type", "bogus"),
            ],
        ),
        // Wrong Basic credentials.
        (
            Some(wrong_basic.as_str()),
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", "x"),
                ("refresh_token", "x"),
            ],
        ),
        // A malformed Authorization header.
        (
            Some("Bearer abc"),
            &[
                ("grant_type", "refresh_token"),
                ("grant_type", "refresh_token"),
            ],
        ),
        // Repeated code and verifier with unknown credentials.
        (
            None,
            &[
                ("client_id", "nope"),
                ("client_secret", "nope"),
                ("grant_type", "authorization_code"),
                ("code", "a"),
                ("code", "b"),
                ("code_verifier", "v"),
                ("code_verifier", "v"),
            ],
        ),
    ];
    for (auth, pairs) in cases {
        let resp = post_form_with_auth(&h, auth, pairs).await;
        assert_sc_error(resp, 400, "invalid_request").await;
    }
}

// spec: B26
#[tokio::test]
async fn a_repeated_grant_type_is_refused_even_with_valid_credentials() {
    let h = start().await;
    let resp = h
        .token(&[
            ("grant_type", "client_credentials"),
            ("grant_type", "client_credentials"),
        ])
        .await;
    assert_sc_error(resp, 400, "invalid_request").await;
    let resp = h
        .token_basic(
            CLIENT_ID,
            CLIENT_SECRET,
            &[
                ("grant_type", "refresh_token"),
                ("grant_type", "authorization_code"),
                ("refresh_token", "x"),
            ],
        )
        .await;
    assert_sc_error(resp, 400, "invalid_request").await;
}

// ---------------------------------------------------------------- randomness

// spec: B34
#[tokio::test]
async fn credentials_do_not_look_like_a_counter_or_a_pattern() {
    let h = start().await;
    let mut codes = Vec::new();
    let mut access_tokens = Vec::new();
    let mut refresh_tokens = Vec::new();
    for n in 0..150 {
        let login = h.login(n).await;
        let tokens = h.exchange_ok(&login).await;
        refresh_tokens.push(tokens.refresh().to_owned());
        access_tokens.push(tokens.access_token);
        codes.push(login.code);
    }
    let mut app_tokens = Vec::new();
    for _ in 0..150 {
        app_tokens.push(read_tokens(h.client_credentials().await).await.access_token);
    }

    let mut everything = HashSet::new();
    for (kind, samples) in [
        ("codes", &codes),
        ("access tokens", &access_tokens),
        ("refresh tokens", &refresh_tokens),
        ("app tokens", &app_tokens),
    ] {
        for sample in samples {
            assert!(is_opaque(sample), "{kind}: {sample:?}");
            assert!(
                everything.insert(sample.clone()),
                "{kind}: {sample:?} twice"
            );
        }
        // At least 100 bits of entropy over the positions (a 128-bit random value gives about
        // 120 to 250 in this estimate; a counter padded to 32 characters gives about 10).
        let bits = positional_entropy_bits(samples);
        assert!(bits >= 100.0, "{kind}: only {bits:.1} bits of entropy");
        // Neither ascending nor descending: no sequence number, no timestamp.
        let ascending = ascending_fraction(samples);
        assert!(
            (0.3..=0.7).contains(&ascending),
            "{kind}: {ascending:.2} of the pairs are ascending"
        );
        // A rich alphabet (hexadecimal is the poorest one accepted, 16 symbols), not just digits.
        let symbols: HashSet<char> = samples.iter().flat_map(|s| s.chars()).collect();
        assert!(
            symbols.len() >= 16,
            "{kind}: only {} symbols",
            symbols.len()
        );
    }
}
