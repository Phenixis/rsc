//! Foundation: config defaults, spawn, shutdown, manual clock, routing errors.

mod common;

use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::time::Duration;

use common::*;
use reqwest::Method;
use rsc_mock::{MockConfig, RegisteredClient, UserDecision, spawn};

// spec: B1
#[test]
fn default_config_has_the_documented_client_and_lifetimes() {
    let config = MockConfig::default();
    assert_eq!(
        config.clients,
        vec![RegisteredClient {
            client_id: "mock-client-id".to_owned(),
            client_secret: "mock-client-secret".to_owned(),
            redirect_uris: vec!["http://127.0.0.1:8888/callback".to_owned()],
        }]
    );
    assert_eq!(config.access_token_lifetime, Duration::from_secs(3599));
    assert_eq!(config.authorization_code_lifetime, Duration::from_secs(600));
    assert_eq!(config.user_decision, UserDecision::Approve);
}

// spec: B1
#[tokio::test]
async fn the_default_server_uses_the_default_config() {
    let h = start().await;
    let tokens = h.session(1).await;
    assert_eq!(tokens.expires_in, 3599);
}

// spec: B2
#[tokio::test]
async fn spawn_binds_loopback_on_an_ephemeral_port() {
    let h = start().await;
    let addr = h.server.addr();
    assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(addr.port(), 0);
    assert_eq!(
        h.server.base_url(),
        format!("http://127.0.0.1:{}", addr.port())
    );
    assert!(!h.server.base_url().ends_with('/'));
}

// spec: B2
#[tokio::test]
async fn two_servers_get_different_ports() {
    let a = start().await;
    let b = start().await;
    assert_ne!(a.server.addr().port(), b.server.addr().port());
}

// spec: B2
#[tokio::test]
async fn the_server_answers_http_on_base_url() {
    let h = start().await;
    let resp = h
        .http
        .get(h.url("/authorize"))
        .send()
        .await
        .expect("answer");
    // No parameters: a client error, but it is the mock that answers.
    assert_eq!(resp.status().as_u16(), 400);
}

// spec: B2
#[tokio::test]
async fn servers_do_not_share_state() {
    let a = start().await;
    let b = start().await;

    let login = a.login(1).await;
    // A code of server A means nothing to server B.
    assert_sc_error(b.exchange(&login).await, 400, "invalid_grant").await;

    let tokens = a.exchange_ok(&login).await;
    assert!(a.server.token_info(&tokens.access_token).is_some());
    assert!(b.server.token_info(&tokens.access_token).is_none());
    assert_sc_error(b.refresh(tokens.refresh()).await, 400, "invalid_grant").await;

    // The clocks are independent as well.
    a.server.advance_clock(Duration::from_secs(42));
    assert_eq!(a.server.elapsed(), Duration::from_secs(42));
    assert_eq!(b.server.elapsed(), Duration::ZERO);
}

// spec: B3
#[tokio::test]
async fn shutdown_closes_the_listening_socket() {
    let h = start().await;
    let addr = h.server.addr();
    let Harness { server, http } = h;

    // Alive before.
    let resp = http
        .get(format!("http://{addr}/authorize"))
        .send()
        .await
        .expect("alive before shutdown");
    assert_eq!(resp.status().as_u16(), 400);

    server.shutdown().await;

    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "nothing listens on {addr} after shutdown"
    );
}

// spec: B4
#[tokio::test]
async fn spawn_rejects_a_zero_access_token_lifetime() {
    let config = MockConfig {
        access_token_lifetime: Duration::ZERO,
        ..MockConfig::default()
    };
    let err = spawn(config).await.expect_err("zero lifetime is invalid");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

// spec: B4
#[tokio::test]
async fn spawn_rejects_a_zero_code_lifetime() {
    let config = MockConfig {
        authorization_code_lifetime: Duration::ZERO,
        ..MockConfig::default()
    };
    let err = spawn(config).await.expect_err("zero lifetime is invalid");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

// spec: B4
#[tokio::test]
async fn spawn_rejects_lifetimes_that_are_not_whole_seconds() {
    for lifetime in [
        Duration::from_millis(1500),
        Duration::from_secs(20) + Duration::from_nanos(1),
    ] {
        let config = MockConfig {
            access_token_lifetime: lifetime,
            ..MockConfig::default()
        };
        let err = spawn(config).await.expect_err("fractional lifetime");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{lifetime:?}");
    }
}

// spec: B4
#[tokio::test]
async fn spawn_rejects_duplicate_client_ids() {
    let mut config = MockConfig::default();
    config.clients.push(RegisteredClient {
        client_id: CLIENT_ID.to_owned(),
        client_secret: "another-secret".to_owned(),
        redirect_uris: vec!["http://127.0.0.1:1/x".to_owned()],
    });
    let err = spawn(config).await.expect_err("duplicate client ids");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

// spec: B4
#[tokio::test]
async fn spawn_rejects_redirect_uris_that_are_not_absolute_urls_without_fragment() {
    for bad in [
        "/callback",
        "callback",
        "",
        "http://127.0.0.1:8888/callback#frag",
    ] {
        let mut config = MockConfig::default();
        config.clients[0].redirect_uris = vec![bad.to_owned()];
        let err = spawn(config).await.expect_err("bad redirect uri");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "redirect uri {bad:?}");
    }
}

// spec: B4
#[tokio::test]
async fn spawn_accepts_a_client_without_redirect_uris_and_a_config_without_clients() {
    let mut config = MockConfig::default();
    config.clients[0].redirect_uris.clear();
    let h = start_with(config).await;
    // Such a client can never complete /authorize.
    let challenge = s256(&verifier(1));
    let resp = h.authorize(&authorize_params(&challenge, "s")).await;
    assert_eq!(resp.status().as_u16(), 400);
    assert!(resp.headers().get("location").is_none());

    let empty = MockConfig {
        clients: Vec::new(),
        ..MockConfig::default()
    };
    let h = start_with(empty).await;
    assert_sc_error(h.client_credentials().await, 401, "invalid_client").await;
}

// spec: B5
#[tokio::test]
async fn the_clock_starts_at_zero_and_only_moves_when_advanced() {
    let h = start().await;
    assert_eq!(h.server.elapsed(), Duration::ZERO);

    // Serving requests (and real time passing) does not move it.
    let _ = h.session(1).await;
    let _ = h.session(2).await;
    assert_eq!(h.server.elapsed(), Duration::ZERO);

    h.server.advance_clock(Duration::from_secs(5));
    assert_eq!(h.server.elapsed(), Duration::from_secs(5));
    h.server.advance_clock(Duration::from_millis(1500));
    assert_eq!(h.server.elapsed(), Duration::from_millis(6500));
    h.server.advance_clock(Duration::ZERO);
    assert_eq!(h.server.elapsed(), Duration::from_millis(6500));
    h.server.advance_clock(Duration::from_secs(86_400 * 30));
    assert_eq!(
        h.server.elapsed(),
        Duration::from_millis(6500) + Duration::from_secs(86_400 * 30)
    );
}

// spec: B6
#[tokio::test]
async fn unknown_paths_answer_a_soundcloud_404() {
    let h = start().await;
    for path in [
        "/",
        "/nope",
        "/oauth",
        "/oauth/token/",
        "/authorize/",
        "/oauth2/token",
        "/OAUTH/TOKEN",
        "/authorize/extra",
        "//authorize",
        "/oauth//token",
        "/oauth/token/x",
        "/Authorize",
    ] {
        for method in [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
            Method::from_bytes(b"PROPFIND").expect("a valid method name"),
        ] {
            let resp = h
                .http
                .request(method.clone(), h.url(path))
                .send()
                .await
                .expect("answer");
            let status = resp.status().as_u16();
            assert_eq!(status, 404, "{method} {path}");
            assert_sc_error_link(resp, 404, "not_found", OPEN_API_LINK).await;
        }
    }
}

// spec: B6
#[tokio::test]
async fn head_on_an_unknown_path_is_a_404_without_body() {
    let h = start().await;
    for path in ["/", "/nope", "/authorize/", "/oauth/token/"] {
        let resp = h.http.head(h.url(path)).send().await.expect("answer");
        assert_eq!(resp.status().as_u16(), 404, "HEAD {path}");
        assert_eq!(header(&resp, "content-type").as_deref(), Some(JSON_UTF8));
        assert!(resp.bytes().await.expect("body").is_empty());
    }
}

// spec: B7
#[tokio::test]
async fn authorize_only_accepts_get() {
    let h = start().await;
    let challenge = s256(&verifier(1));
    for method in [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
    ] {
        let resp = h
            .http
            .request(method.clone(), h.url("/authorize"))
            .form(&authorize_params(&challenge, "s"))
            .send()
            .await
            .expect("answer");
        let allow = header(&resp, "allow").expect("an Allow header");
        assert!(allow.contains("GET"), "Allow: {allow} for {method}");
        for other in ["POST", "PUT", "DELETE", "PATCH"] {
            assert!(!allow.contains(other), "Allow: {allow} for {method}");
        }
        assert_sc_error(resp, 405, "method_not_allowed").await;
    }
}

// spec: B7
#[tokio::test]
async fn token_endpoint_only_accepts_post() {
    let h = start().await;
    for method in [
        Method::GET,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
    ] {
        let resp = h
            .http
            .request(method.clone(), h.url("/oauth/token"))
            .query(&[
                ("grant_type", "client_credentials"),
                ("client_id", CLIENT_ID),
                ("client_secret", CLIENT_SECRET),
            ])
            .send()
            .await
            .expect("answer");
        let allow = header(&resp, "allow").expect("an Allow header");
        assert!(allow.contains("POST"), "Allow: {allow} for {method}");
        for other in ["GET", "PUT", "DELETE", "PATCH", "HEAD"] {
            assert!(!allow.contains(other), "Allow: {allow} for {method}");
        }
        assert_sc_error(resp, 405, "method_not_allowed").await;
    }
}

// spec: B7
#[tokio::test]
async fn head_is_a_get_without_body_on_authorize_and_a_405_on_the_token_endpoint() {
    let h = start().await;
    let challenge = s256(&verifier(1));

    // HEAD /authorize behaves as GET /authorize: same status and headers, no body.
    let resp = h
        .http
        .head(h.url("/authorize"))
        .query(&authorize_params(&challenge, "st4te"))
        .send()
        .await
        .expect("answer");
    assert_eq!(resp.status().as_u16(), 302);
    let loc = location(&resp);
    assert_eq!(loc.path(), "/callback");
    assert_eq!(param(&loc, "state").as_deref(), Some("st4te"));
    assert!(is_opaque(&param(&loc, "code").expect("a code")));
    assert_eq!(header(&resp, "cache-control").as_deref(), Some("no-store"));
    assert!(resp.bytes().await.expect("body").is_empty());

    // HEAD on the error paths of /authorize.
    let resp = h
        .http
        .head(h.url("/authorize"))
        .send()
        .await
        .expect("answer");
    assert_eq!(resp.status().as_u16(), 400);
    assert!(resp.headers().get("location").is_none());

    // HEAD /oauth/token is not POST.
    let resp = h
        .http
        .head(h.url("/oauth/token"))
        .send()
        .await
        .expect("answer");
    assert_eq!(resp.status().as_u16(), 405);
    let allow = header(&resp, "allow").expect("an Allow header");
    assert!(allow.contains("POST"), "Allow: {allow}");
    assert!(!allow.contains("GET"), "Allow: {allow}");
}

// spec: B7
#[tokio::test]
async fn the_method_is_checked_before_anything_else_on_the_token_endpoint() {
    let h = start().await;
    // Wrong content type, repeated parameters, bad credentials, a bogus grant: still 405.
    let body = "client_id=nope&client_id=nope2&client_secret=x&grant_type=bogus";
    for method in [Method::GET, Method::PUT, Method::DELETE, Method::PATCH] {
        for content_type in ["text/plain", "application/x-www-form-urlencoded"] {
            let resp = h
                .http
                .request(method.clone(), h.url("/oauth/token"))
                .header("content-type", content_type)
                .body(body)
                .send()
                .await
                .expect("answer");
            let allow = header(&resp, "allow").expect("an Allow header");
            assert!(allow.contains("POST"), "Allow: {allow} for {method}");
            assert_sc_error(resp, 405, "method_not_allowed").await;
        }
    }
}

// spec: B2
#[tokio::test]
async fn the_server_listens_on_loopback_only() {
    let h = start().await;
    let port = h.server.addr().port();
    // Reachable on 127.0.0.1...
    tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .expect("connect to 127.0.0.1");
    // ...but not on another loopback address: on Linux all of 127.0.0.0/8 is local, so a server
    // bound to 0.0.0.0 would answer here and one bound to 127.0.0.1 only does not.
    let elsewhere = (Ipv4Addr::new(127, 0, 0, 2), port);
    let other = tokio::net::TcpStream::connect(elsewhere).await;
    assert!(
        other.is_err(),
        "the server must be bound to 127.0.0.1 only, not to every interface"
    );
}

// spec: B5
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_clock_does_not_move_as_real_time_passes() {
    let h = start().await;
    let tokens = h.session(1).await;
    let login = h.login(2).await;
    h.server.advance_clock(Duration::from_millis(2500));
    let before = h
        .server
        .token_info(&tokens.access_token)
        .expect("valid")
        .remaining;
    assert_eq!(before, Duration::from_millis(3599 * 1000 - 2500));

    // A real pause (the mock clock is manual: real time must mean nothing to it).
    std::thread::sleep(Duration::from_millis(150));

    assert_eq!(h.server.elapsed(), Duration::from_millis(2500));
    let after = h
        .server
        .token_info(&tokens.access_token)
        .expect("still valid")
        .remaining;
    assert_eq!(after, before);
    // The code of the login is exactly as old as the mock clock says: 599.9 s, so 100 ms
    // of real time drifting into the clock would make it expire.
    h.server.advance_clock(Duration::from_millis(597_400));
    h.exchange_ok(&login).await;
}

// spec: B5
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fresh_clock_is_still_at_zero_after_a_real_pause() {
    let h = start().await;
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(h.server.elapsed(), Duration::ZERO);
    let tokens = h.session(1).await;
    let info = h.server.token_info(&tokens.access_token).expect("valid");
    assert_eq!(info.remaining, Duration::from_secs(3599));
}
