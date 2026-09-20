//! Raw TCP tests: requests that an HTTP client library would not let us build (raw invalid
//! UTF-8 and NUL bytes, chunked bodies, a `Content-Length` that lies about the body).
//!
//! They use `std::net::TcpStream` on a blocking task and never sleep: every request says
//! `Connection: close`, so the server ends the exchange itself.

mod common;

use std::time::{Duration, Instant};

use common::*;
use rsc_mock::{MockConfig, RegisteredClient};

const FORM_HEADER: &[u8] = b"Content-Type: application/x-www-form-urlencoded";
const CREDS: &str = "client_id=mock-client-id&client_secret=mock-client-secret";
const OVER_LIMIT: usize = 65_537;

/// `POST /oauth/token` with these headers and body, raw.
async fn post(h: &Harness, headers: &[&[u8]], body: &[u8]) -> Option<RawResponse> {
    raw_http(h.server.addr(), raw_post("/oauth/token", headers, body)).await
}

/// The default client's credentials followed by `tail`.
fn with_creds(tail: &[u8]) -> Vec<u8> {
    let mut body = CREDS.as_bytes().to_vec();
    body.push(b'&');
    body.extend_from_slice(tail);
    body
}

/// A refresh request whose (unknown) refresh token pads the body to exactly `len` bytes.
fn padded_body(len: usize) -> Vec<u8> {
    let mut body = with_creds(b"grant_type=refresh_token&refresh_token=");
    assert!(body.len() <= len);
    body.resize(len, b't');
    body
}

/// A request that announces `announced` as its `Content-Length` and sends only `prefix`.
fn post_announcing(headers: &[&[u8]], announced: &str, prefix: &[u8]) -> Vec<u8> {
    let mut request = b"POST /oauth/token HTTP/1.1\r\nHost: 127.0.0.1\r\n".to_vec();
    request.extend_from_slice(b"Connection: close\r\n");
    request.extend_from_slice(format!("Content-Length: {announced}\r\n").as_bytes());
    for header in headers {
        request.extend_from_slice(header);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(prefix);
    request
}

/// A `Transfer-Encoding: chunked` request, one chunk per slice.
fn post_chunked(headers: &[&[u8]], chunks: &[&[u8]]) -> Vec<u8> {
    let mut request = b"POST /oauth/token HTTP/1.1\r\nHost: 127.0.0.1\r\n".to_vec();
    request.extend_from_slice(b"Connection: close\r\nTransfer-Encoding: chunked\r\n");
    for header in headers {
        request.extend_from_slice(header);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    for chunk in chunks {
        request.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        request.extend_from_slice(chunk);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"0\r\n\r\n");
    request
}

/// The server is still up and working normally.
async fn assert_healthy(h: &Harness) {
    let login = h.login(1).await;
    h.exchange_ok(&login).await;
}

// spec: B59
#[tokio::test]
async fn raw_invalid_utf8_and_nul_bytes_in_the_values_are_handled() {
    let h = start().await;
    // Valid credentials: the odd bytes only reach values that are unknown or ignored.
    let cases: [(&[u8], &str); 6] = [
        (
            b"grant_type=refresh_token&refresh_token=\xff\xfe",
            "invalid_grant",
        ),
        (
            b"grant_type=refresh_token&refresh_token=\x00",
            "invalid_grant",
        ),
        (
            b"grant_type=refresh_token&refresh_token=x&junk=\x00\x80\xc3\x28",
            "invalid_grant",
        ),
        (
            b"grant_type=refresh_token\x00&refresh_token=x",
            "unsupported_grant",
        ),
        (b"grant_type=\xff", "unsupported_grant"),
        (
            b"\x00&grant_type=refresh_token&\xff=\xfe&refresh_token=x",
            "invalid_grant",
        ),
    ];
    for (tail, message) in cases {
        let resp = post(&h, &[FORM_HEADER], &with_creds(tail))
            .await
            .expect("a well-formed HTTP request is answered");
        resp.assert_sc_error(400, message);
    }
    assert_healthy(&h).await;
}

// spec: B59
#[tokio::test]
async fn raw_invalid_utf8_and_nul_bytes_in_the_credentials_are_a_401() {
    let h = start().await;
    let cases: [&[u8]; 6] = [
        b"client_id=mock-client-id&client_secret=mock\x00-client-secret&grant_type=refresh_token&refresh_token=x",
        b"client_id=mock-client-id\x00&client_secret=mock-client-secret&grant_type=refresh_token",
        b"\x00\x00\x00",
        b"\xff\xfe\xfd\xfc",
        b"client_id=\xff&client_secret=\xff&grant_type=refresh_token",
        b"%00\x00=\xff&&\x00",
    ];
    for body in cases {
        let resp = post(&h, &[FORM_HEADER], body)
            .await
            .expect("a well-formed HTTP request is answered");
        resp.assert_sc_error(401, "invalid_client");
    }
    assert_healthy(&h).await;
}

// spec: B61
#[tokio::test]
async fn raw_invalid_utf8_is_replaced_by_the_replacement_character() {
    let config = MockConfig {
        clients: vec![RegisteredClient {
            client_id: "odd".to_owned(),
            client_secret: "s\u{fffd}x".to_owned(),
            redirect_uris: vec![REDIRECT_URI.to_owned()],
        }],
        ..MockConfig::default()
    };
    let h = start_with(config).await;
    let tail = b"&grant_type=refresh_token&refresh_token=zzz";
    // The raw byte 0xFF is the secret's U+FFFD, as it would be if percent-encoded.
    let same: [&[u8]; 4] = [b"s\xffx", b"s\xfex", b"s\xc3x", b"s\xef\xbf\xbdx"];
    for secret in same {
        let mut body = b"client_id=odd&client_secret=".to_vec();
        body.extend_from_slice(secret);
        body.extend_from_slice(tail);
        let resp = post(&h, &[FORM_HEADER], &body).await.expect("answered");
        resp.assert_sc_error(400, "invalid_grant");
    }
    let different: [&[u8]; 4] = [b"s\xc3(x", b"s\xff\xffx", b"sx", b"s\xff"];
    for secret in different {
        let mut body = b"client_id=odd&client_secret=".to_vec();
        body.extend_from_slice(secret);
        body.extend_from_slice(tail);
        let resp = post(&h, &[FORM_HEADER], &body).await.expect("answered");
        resp.assert_sc_error(401, "invalid_client");
    }
}

// spec: B59
#[tokio::test]
async fn raw_bytes_in_the_request_head_never_cause_a_5xx() {
    let h = start().await;
    let requests = [
        raw_get(b"/authorize?client_id=mock-client-id\x00&x=1", &[]),
        raw_get(b"/authorize?state=\xff\xfe", &[]),
        raw_get(b"/authorize?state=\xc3\x28&client_id=mock-client-id", &[]),
        raw_get(b"/oauth/token?\x00", &[]),
        raw_get(b"/\x00", &[]),
        raw_get(b"/authorize", &[b"X-Bad: a\x00b"]),
        raw_get(b"/authorize", &[b"Bad Header Name: x"]),
        raw_get(b"/authorize", &[b"X-Odd: \xff\xfe"]),
        raw_post(
            "/oauth/token",
            &[FORM_HEADER, b"X-Bad: a\x00b"],
            CREDS.as_bytes(),
        ),
    ];
    for request in requests {
        // Refused by the HTTP layer with a 4xx, or by closing the connection; a header with
        // high bytes may even be accepted and then it is a normal answer. Never a 5xx.
        if let Some(resp) = raw_http(h.server.addr(), request).await {
            assert!(resp.status < 500, "a 5xx for a malformed request: {resp:?}");
        }
    }
    assert_healthy(&h).await;
}

// spec: B29
#[tokio::test]
async fn an_authorization_header_with_raw_high_bytes_is_not_valid_basic() {
    let h = start().await;
    let body = with_creds(b"grant_type=refresh_token&refresh_token=x");
    let headers: [&[u8]; 4] = [
        b"Authorization: Basic \xff\xfe",
        b"Authorization: \xff",
        b"Authorization: Basic \x80\x81\x82",
        b"Authorization: Bearer \xe9",
    ];
    for auth in headers {
        // Either the HTTP layer refuses the header (a 4xx that is not the answer to a valid
        // request), or the mock treats it as a failed authentication attempt: the valid
        // credentials of the body must not rescue it.
        if let Some(resp) = post(&h, &[FORM_HEADER, auth], &body).await {
            let text = String::from_utf8_lossy(&resp.body).into_owned();
            assert!(
                resp.status == 401 || (resp.status == 400 && !text.contains("invalid_grant")),
                "the body credentials must not rescue a bad Authorization header: {resp:?}"
            );
            if resp.status == 401 {
                resp.assert_sc_error(401, "invalid_client");
            }
        }
    }
    assert_healthy(&h).await;
}

/// The answer to `request` and how long it took to arrive. The harness would wait 10 seconds
/// for a server that waits for a body that never comes; that is not acceptable here.
async fn timed(h: &Harness, request: Vec<u8>) -> (Option<RawResponse>, Duration) {
    let start = Instant::now();
    let resp = raw_http(h.server.addr(), request).await;
    (resp, start.elapsed())
}

/// "Right away": far below the harness timeout, generous for a loaded CI machine.
const PROMPTLY: Duration = Duration::from_secs(4);

// spec: B25
#[tokio::test]
async fn an_announced_length_over_the_limit_is_refused_without_waiting_for_the_body() {
    let h = start().await;
    let some_body = with_creds(b"grant_type=refresh_token&refresh_token=x");
    let prefixes: [&[u8]; 2] = [&[], some_body.as_slice()];
    // From 65 537 up to and including 2^40 the answer is the strict SoundCloud 413.
    for announced in [
        "65537",
        "70000",
        "1048576",
        "10737418240",
        "1099511627775",
        "1099511627776",
    ] {
        for prefix in prefixes {
            let request = post_announcing(&[FORM_HEADER], announced, prefix);
            let (resp, took) = timed(&h, request).await;
            let resp = resp.unwrap_or_else(|| panic!("no answer for {announced}"));
            assert!(
                took < PROMPTLY,
                "the 413 for {announced} took {took:?}: the server waited for the body"
            );
            resp.assert_sc_error(413, "payload_too_large");
            assert_eq!(resp.header("cache-control"), Some("no-store"));
        }
    }
    assert_healthy(&h).await;
}

// spec: B25
#[tokio::test]
async fn an_absurd_announced_length_is_never_accepted_and_never_waited_for() {
    let h = start().await;
    // Beyond 2^40 the HTTP layer (hyper) may refuse the request itself, before the mock sees it
    // (u64::MAX is answered with a bare 431): any 4xx or a closed connection is fine; never a
    // 2xx, never a 5xx, never a wait. The same goes for lengths that are not a number.
    for announced in [
        "1099511627777",
        "9223372036854775808",
        "18446744073709551614",
        "18446744073709551615",
        "18446744073709551616",
        "99999999999999999999999",
        "-1",
        "abc",
    ] {
        for prefix in [&b""[..], b"client_id=mock-client-id"] {
            let request = post_announcing(&[FORM_HEADER], announced, prefix);
            let (resp, took) = timed(&h, request).await;
            assert!(
                took < PROMPTLY,
                "the answer for {announced} took {took:?}: the server waited for the body"
            );
            if let Some(resp) = resp {
                assert!((400..500).contains(&resp.status), "{announced}: {resp:?}");
            }
        }
    }
    assert_healthy(&h).await;
}

// spec: B24
#[tokio::test]
async fn the_content_type_is_checked_before_the_announced_length() {
    let h = start().await;
    let content_types: [&[u8]; 3] = [
        b"Content-Type: text/plain",
        b"Content-Type: application/json",
        b"X-No-Content-Type: 1",
    ];
    for content_type in content_types {
        let request = post_announcing(&[content_type], "10737418240", b"");
        let resp = raw_http(h.server.addr(), request)
            .await
            .expect("answered without waiting for the body");
        resp.assert_sc_error(415, "unsupported_media_type");
    }
}

// spec: B25
#[tokio::test]
async fn chunked_bodies_within_the_limit_are_read() {
    let h = start().await;
    let chunks: [&[u8]; 3] = [
        b"client_id=mock-client-id&",
        b"client_secret=mock-client-secret&",
        b"grant_type=refresh_token&refresh_token=x",
    ];
    let request = post_chunked(&[FORM_HEADER], &chunks);
    let resp = raw_http(h.server.addr(), request).await.expect("answered");
    resp.assert_sc_error(400, "invalid_grant");

    // Exactly at the limit, in chunks of 10 000 bytes.
    let body = padded_body(65_536);
    let chunks: Vec<&[u8]> = body.chunks(10_000).collect();
    let request = post_chunked(&[FORM_HEADER], &chunks);
    let resp = raw_http(h.server.addr(), request).await.expect("answered");
    resp.assert_sc_error(400, "invalid_grant");

    // A whole valid login through a chunked body works too.
    let login = h.login(3).await;
    let form = format!(
        "{CREDS}&grant_type=authorization_code&code={}&redirect_uri=http%3A%2F%2F127.0.0.1%3A8888%2Fcallback&code_verifier={}",
        login.code, login.verifier
    );
    let halves = form.as_bytes().split_at(form.len() / 2);
    let request = post_chunked(&[FORM_HEADER], &[halves.0, halves.1]);
    let resp = raw_http(h.server.addr(), request).await.expect("answered");
    assert_eq!(resp.status, 200, "{resp:?}");
}

// spec: B25
#[tokio::test]
async fn chunked_bodies_slightly_over_the_limit_always_get_their_413() {
    let h = start().await;
    // Up to 100 000 bytes the whole request fits in the socket buffers: the server must answer,
    // not close silently.
    for len in [OVER_LIMIT, 70_000, 100_000] {
        let body = padded_body(len);
        let one_chunk: [&[u8]; 1] = [&body];
        let many_chunks: Vec<&[u8]> = body.chunks(10_000).collect();
        for chunks in [&one_chunk[..], &many_chunks[..]] {
            let request = post_chunked(&[FORM_HEADER], chunks);
            let (resp, took) = timed(&h, request).await;
            let resp = resp.unwrap_or_else(|| {
                panic!("a chunked body of {len} bytes got no answer (expected 413)")
            });
            assert!(took < PROMPTLY, "the 413 took {took:?}");
            resp.assert_sc_error(413, "payload_too_large");
        }
    }
    assert_healthy(&h).await;
}

// spec: B25
#[tokio::test]
async fn chunked_bodies_far_over_the_limit_are_never_accepted() {
    let h = start().await;
    for len in [200_000, 400_000] {
        let body = padded_body(len);
        let chunks: Vec<&[u8]> = body.chunks(10_000).collect();
        let request = post_chunked(&[FORM_HEADER], &chunks);
        // The server may close the connection while we are still sending, so no answer is
        // acceptable here; but an answer is the 413, never a 2xx (nor anything else).
        let (resp, took) = timed(&h, request).await;
        assert!(took < PROMPTLY, "the connection stayed open {took:?}");
        if let Some(resp) = resp {
            assert!(
                !(200..300).contains(&resp.status),
                "an over-limit chunked body was accepted: {resp:?}"
            );
            resp.assert_sc_error(413, "payload_too_large");
        }
    }
    assert_healthy(&h).await;
}

// spec: B25
#[tokio::test]
async fn a_refused_chunked_body_consumes_nothing() {
    let h = start().await;
    let login = h.login(9).await;
    let tokens = h.exchange_ok(&login).await;
    for len in [OVER_LIMIT, 90_000] {
        let tail = format!(
            "grant_type=refresh_token&refresh_token={}&pad=",
            tokens.refresh()
        );
        let mut body = with_creds(tail.as_bytes());
        body.resize(len, b'p');
        let chunks: Vec<&[u8]> = body.chunks(7_000).collect();
        let request = post_chunked(&[FORM_HEADER], &chunks);
        let resp = raw_http(h.server.addr(), request)
            .await
            .expect("a chunked body slightly over the limit is answered");
        resp.assert_sc_error(413, "payload_too_large");
    }
    // The refresh token in the refused bodies was neither redeemed nor revoked.
    let renewed = h.refresh_ok(tokens.refresh()).await;
    assert_ne!(renewed.access_token, tokens.access_token);
}
