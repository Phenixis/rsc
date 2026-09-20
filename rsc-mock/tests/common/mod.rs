//! Shared helpers of the `rsc-mock` integration tests (black box over HTTP).
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::redirect::Policy;
use reqwest::{Response, StatusCode};
use rsc_mock::{MockConfig, MockServer, RegisteredClient, spawn};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

pub const CLIENT_ID: &str = "mock-client-id";
pub const CLIENT_SECRET: &str = "mock-client-secret";
pub const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
pub const GUIDE_LINK: &str = "https://developers.soundcloud.com/docs/api/guide#authentication";
pub const OPEN_API_LINK: &str = "https://developers.soundcloud.com/docs/api/explorer/open-api";
pub const JSON_UTF8: &str = "application/json; charset=utf-8";

/// `base64url_nopad(sha256(verifier))`, computed independently of the crate under test.
pub fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A valid 43-character verifier, different for every `n`.
pub fn verifier(n: u32) -> String {
    format!("verifier-{n:0>34}")
}

/// A valid verifier of exactly `len` characters (43..=128), different for every `seed`.
pub fn verifier_of_len(len: usize, seed: u32) -> String {
    let prefix = format!("v{seed}-");
    let mut v = prefix;
    while v.len() < len {
        v.push_str("aZ09-._~");
    }
    v.truncate(len);
    v
}

/// Opaque credential shape promised by the spec: 32..=128 characters of `[A-Za-z0-9_-]`.
pub fn is_opaque(s: &str) -> bool {
    (32..=128).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A second registered client, with its own redirect URIs.
pub fn other_client() -> RegisteredClient {
    RegisteredClient {
        client_id: "other-client-id".to_owned(),
        client_secret: "other-client-secret".to_owned(),
        redirect_uris: vec!["http://127.0.0.1:9999/other".to_owned()],
    }
}

/// Default config plus a second client.
pub fn two_client_config() -> MockConfig {
    let mut config = MockConfig::default();
    config.clients.push(other_client());
    config
}

/// What came back on a raw TCP connection.
#[derive(Debug)]
pub struct RawResponse {
    pub status: u16,
    /// Status line and headers, without the final blank line.
    pub head: String,
    pub body: Vec<u8>,
}

impl RawResponse {
    /// First header with this (case-insensitive) name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then_some(value.trim())
        })
    }

    /// Asserts a SoundCloud error of the OAuth kind (link, keys and code included).
    pub fn assert_sc_error(&self, status: u16, message: &str) {
        assert_eq!(self.status, status, "status; response was {self:?}");
        let body: Value = serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {self:?}"));
        let object = body.as_object().expect("a JSON object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["code", "link", "message"], "{self:?}");
        assert_eq!(object["code"].as_u64(), Some(u64::from(status)), "{self:?}");
        assert_eq!(object["message"].as_str(), Some(message), "{self:?}");
        assert_eq!(object["link"].as_str(), Some(GUIDE_LINK), "{self:?}");
    }
}

fn parse_raw_response(data: &[u8]) -> Option<RawResponse> {
    let end = data.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&data[..end]).into_owned();
    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(RawResponse {
        status,
        head,
        body: data[end + 4..].to_vec(),
    })
}

/// Sends `request` (raw bytes, `Connection: close` expected) on a fresh TCP connection and
/// returns what the server answered, or `None` when it answered nothing (closed or reset the
/// connection). A reset while we are still writing is not an error: the server may answer and
/// close before it has read everything, and the answer is kept when it arrived.
pub async fn raw_http(addr: SocketAddr, request: Vec<u8>) -> Option<RawResponse> {
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(addr).ok()?;
        let timeout = Some(Duration::from_secs(10));
        stream.set_read_timeout(timeout).ok()?;
        stream.set_write_timeout(timeout).ok()?;
        let _ = stream.write_all(&request);
        let mut data = Vec::new();
        let _ = stream.read_to_end(&mut data);
        parse_raw_response(&data)
    })
    .await
    .expect("the blocking task finishes")
}

/// `POST <path>` with the given extra header lines (raw bytes, without CRLF) and body.
pub fn raw_post(path: &str, headers: &[&[u8]], body: &[u8]) -> Vec<u8> {
    let mut request = format!("POST {path} HTTP/1.1\r\n").into_bytes();
    request.extend_from_slice(b"Host: 127.0.0.1\r\nConnection: close\r\n");
    request.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    for header in headers {
        request.extend_from_slice(header);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(body);
    request
}

/// `GET <target>` where the target may contain arbitrary bytes.
pub fn raw_get(target: &[u8], headers: &[&[u8]]) -> Vec<u8> {
    let mut request = b"GET ".to_vec();
    request.extend_from_slice(target);
    request.extend_from_slice(b" HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for header in headers {
        request.extend_from_slice(header);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    request
}

pub struct Harness {
    pub server: MockServer,
    pub http: reqwest::Client,
}

pub async fn start() -> Harness {
    start_with(MockConfig::default()).await
}

pub async fn start_with(config: MockConfig) -> Harness {
    let server = spawn(config).await.expect("the mock server starts");
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client");
    Harness { server, http }
}

pub struct Login {
    pub code: String,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub raw: Value,
}

impl Tokens {
    pub fn refresh(&self) -> &str {
        self.refresh_token.as_deref().expect("a refresh_token")
    }
}

/// Valid `/authorize` parameters for the default client.
pub fn authorize_params<'a>(challenge: &'a str, state: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT_URI),
        ("response_type", "code"),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ]
}

/// Same, with one parameter replaced (or removed when `value` is `None`).
pub fn authorize_params_with<'a>(
    challenge: &'a str,
    state: &'a str,
    name: &str,
    value: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut params: Vec<(&str, &str)> = authorize_params(challenge, state)
        .into_iter()
        .filter(|(k, _)| *k != name)
        .collect();
    if let Some(value) = value {
        // `name` is one of the fixed parameter names above.
        let name: &'static str = match name {
            "client_id" => "client_id",
            "redirect_uri" => "redirect_uri",
            "response_type" => "response_type",
            "code_challenge" => "code_challenge",
            "code_challenge_method" => "code_challenge_method",
            "state" => "state",
            other => panic!("unknown authorize parameter {other}"),
        };
        params.push((name, value));
    }
    params
}

pub fn header(resp: &Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .map(|v| v.to_str().expect("ASCII header").to_owned())
}

/// The `Location` header, parsed. Panics if it is missing or not an absolute URL.
pub fn location(resp: &Response) -> Url {
    let raw = header(resp, "location").expect("a Location header");
    Url::parse(&raw).unwrap_or_else(|e| panic!("Location {raw:?} is not a URL: {e}"))
}

pub fn param(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Sorted names of the query parameters of `url`.
pub fn param_names(url: &Url) -> Vec<String> {
    let mut names: Vec<String> = url.query_pairs().map(|(k, _)| k.into_owned()).collect();
    names.sort();
    names
}

/// Checks that `resp` is the SoundCloud error shape with the given status and message.
pub async fn assert_sc_error_link(resp: Response, status: u16, message: &str, link: &str) {
    let got = resp.status().as_u16();
    let content_type = header(&resp, "content-type");
    let text = resp.text().await.expect("readable body");
    assert_eq!(got, status, "status; body was {text}");
    assert_eq!(
        content_type.as_deref(),
        Some(JSON_UTF8),
        "content type of {text}"
    );
    let body: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{text:?}: {e}"));
    let object = body.as_object().expect("a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["code", "link", "message"], "keys of {text}");
    assert_eq!(object["code"].as_u64(), Some(u64::from(status)), "{text}");
    assert_eq!(object["message"].as_str(), Some(message), "{text}");
    assert_eq!(object["link"].as_str(), Some(link), "{text}");
}

/// OAuth error: SoundCloud shape with the authentication-guide link.
pub async fn assert_sc_error(resp: Response, status: u16, message: &str) {
    assert_sc_error_link(resp, status, message, GUIDE_LINK).await;
}

/// Asserts a 200 token response and returns it parsed.
pub async fn read_tokens(resp: Response) -> Tokens {
    let status = resp.status();
    let text = resp.text().await.expect("readable body");
    assert_eq!(
        status,
        StatusCode::OK,
        "token endpoint answered {status}: {text}"
    );
    let raw: Value = serde_json::from_str(&text).expect("JSON body");
    Tokens {
        access_token: raw["access_token"]
            .as_str()
            .expect("access_token")
            .to_owned(),
        refresh_token: raw
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_owned),
        expires_in: raw["expires_in"].as_u64().expect("numeric expires_in"),
        raw,
    }
}

impl Harness {
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.server.base_url(), path)
    }

    /// `GET /authorize` with the given (percent-encoded by us) parameters.
    pub async fn authorize(&self, params: &[(&str, &str)]) -> Response {
        self.http
            .get(self.url("/authorize"))
            .query(params)
            .send()
            .await
            .expect("request sent")
    }

    /// `GET /authorize?<raw query>` exactly as given.
    pub async fn authorize_raw(&self, raw_query: &str) -> Response {
        self.http
            .get(format!("{}?{}", self.url("/authorize"), raw_query))
            .send()
            .await
            .expect("request sent")
    }

    /// A successful authorization of the default client, as the browser would do it.
    pub async fn login_with(&self, verifier: &str, state: &str) -> Login {
        let challenge = s256(verifier);
        let resp = self.authorize(&authorize_params(&challenge, state)).await;
        assert_eq!(resp.status(), StatusCode::FOUND, "authorize must redirect");
        let location = location(&resp);
        let code = param(&location, "code").expect("a code in the redirect");
        Login {
            code,
            verifier: verifier.to_owned(),
            state: state.to_owned(),
            redirect_uri: REDIRECT_URI.to_owned(),
        }
    }

    pub async fn login(&self, n: u32) -> Login {
        self.login_with(&verifier(n), &format!("state-{n}")).await
    }

    /// `POST /oauth/token` with `client_id` + `client_secret` of the default client in the body.
    pub async fn token(&self, fields: &[(&str, &str)]) -> Response {
        let mut form: Vec<(&str, &str)> =
            vec![("client_id", CLIENT_ID), ("client_secret", CLIENT_SECRET)];
        form.extend_from_slice(fields);
        self.token_raw(&form).await
    }

    /// `POST /oauth/token` with exactly this form and no client authentication added.
    pub async fn token_raw(&self, form: &[(&str, &str)]) -> Response {
        self.http
            .post(self.url("/oauth/token"))
            .form(form)
            .send()
            .await
            .expect("request sent")
    }

    /// `POST /oauth/token` with HTTP Basic client authentication and exactly this form.
    pub async fn token_basic(&self, id: &str, secret: &str, form: &[(&str, &str)]) -> Response {
        self.http
            .post(self.url("/oauth/token"))
            .basic_auth(id, Some(secret))
            .form(form)
            .send()
            .await
            .expect("request sent")
    }

    pub async fn exchange(&self, login: &Login) -> Response {
        self.token(&[
            ("grant_type", "authorization_code"),
            ("code", login.code.as_str()),
            ("redirect_uri", login.redirect_uri.as_str()),
            ("code_verifier", login.verifier.as_str()),
        ])
        .await
    }

    pub async fn exchange_ok(&self, login: &Login) -> Tokens {
        read_tokens(self.exchange(login).await).await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Response {
        self.token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    pub async fn refresh_ok(&self, refresh_token: &str) -> Tokens {
        read_tokens(self.refresh(refresh_token).await).await
    }

    /// Login + code exchange in one go.
    pub async fn session(&self, n: u32) -> Tokens {
        let login = self.login(n).await;
        self.exchange_ok(&login).await
    }

    /// Sends `n` identical token requests (default client credentials in the body) at the
    /// same moment. Returns `(status, body)` of each. Needs a multi-thread runtime to be
    /// really parallel.
    pub async fn race(&self, n: usize, fields: &[(&str, &str)]) -> Vec<(u16, String)> {
        let mut form: Vec<(String, String)> = vec![
            ("client_id".to_owned(), CLIENT_ID.to_owned()),
            ("client_secret".to_owned(), CLIENT_SECRET.to_owned()),
        ];
        form.extend(fields.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(n));
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..n {
            let http = self.http.clone();
            let url = self.url("/oauth/token");
            let form = form.clone();
            let barrier = barrier.clone();
            set.spawn(async move {
                barrier.wait().await;
                let resp = http
                    .post(url)
                    .form(&form)
                    .send()
                    .await
                    .expect("request sent");
                let status = resp.status().as_u16();
                (status, resp.text().await.expect("body"))
            });
        }
        let mut results = Vec::new();
        while let Some(joined) = set.join_next().await {
            results.push(joined.expect("task finished"));
        }
        results
    }

    pub async fn client_credentials(&self) -> Response {
        self.token_basic(
            CLIENT_ID,
            CLIENT_SECRET,
            &[("grant_type", "client_credentials")],
        )
        .await
    }
}
