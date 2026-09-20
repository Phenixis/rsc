use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::*;

const STATE: &str = "xyz";

fn parse(line: &str) -> Result<String, CallbackError> {
    parse_request_line(line, STATE)
}

#[test]
fn the_code_is_extracted_when_the_state_matches() {
    assert_eq!(
        parse("GET /callback?code=abc123&state=xyz HTTP/1.1"),
        Ok("abc123".into())
    );
}

#[test]
fn parameter_order_does_not_matter() {
    assert_eq!(
        parse("GET /callback?state=xyz&code=abc HTTP/1.1"),
        Ok("abc".into())
    );
}

#[test]
fn percent_encoded_values_are_decoded() {
    assert_eq!(
        parse("GET /callback?code=a%2Fb%3Dc&state=xyz HTTP/1.1"),
        Ok("a/b=c".into())
    );
}

#[test]
fn a_wrong_or_missing_state_is_rejected() {
    assert_eq!(
        parse("GET /callback?code=abc&state=evil HTTP/1.1"),
        Err(CallbackError::StateMismatch)
    );
    assert_eq!(
        parse("GET /callback?code=abc HTTP/1.1"),
        Err(CallbackError::StateMismatch)
    );
}

#[test]
fn a_provider_error_is_reported_with_its_reason() {
    assert_eq!(
        parse("GET /callback?error=access_denied&state=xyz HTTP/1.1"),
        Err(CallbackError::Denied("access_denied".into()))
    );
}

#[test]
fn the_state_is_checked_before_anything_else() {
    assert_eq!(
        parse("GET /callback?error=access_denied&state=evil HTTP/1.1"),
        Err(CallbackError::StateMismatch)
    );
}

#[test]
fn a_missing_or_empty_code_is_an_error() {
    assert_eq!(
        parse("GET /callback?state=xyz HTTP/1.1"),
        Err(CallbackError::MissingCode)
    );
    assert_eq!(
        parse("GET /callback?code=&state=xyz HTTP/1.1"),
        Err(CallbackError::MissingCode)
    );
}

#[test]
fn only_get_callback_is_the_callback() {
    for line in [
        "GET /favicon.ico HTTP/1.1",
        "GET / HTTP/1.1",
        "GET /callback/x?code=a&state=xyz HTTP/1.1",
        "GET //evil.example/callback?code=a&state=xyz HTTP/1.1",
        "POST /callback?code=a&state=xyz HTTP/1.1",
    ] {
        assert_eq!(parse(line), Err(CallbackError::NotCallback), "{line}");
    }
}

#[test]
fn garbage_is_malformed() {
    for line in ["", "hello", "GET /callback", "\u{0}\u{1}\u{2}"] {
        assert_eq!(parse(line), Err(CallbackError::Malformed), "{line:?}");
    }
}

// --- listener -------------------------------------------------------------------------

const LONG: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(100);

async fn get(addr: std::net::SocketAddr, target: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

async fn listening() -> (TcpListener, std::net::SocketAddr) {
    let listener = bind(0).await.unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

fn callback_error(err: &anyhow::Error) -> Option<&CallbackError> {
    err.downcast_ref::<CallbackError>()
}

#[tokio::test]
async fn the_listener_only_accepts_loopback_connections() {
    let (_listener, addr) = listening().await;
    assert!(addr.ip().is_loopback());
}

#[tokio::test]
async fn the_code_is_returned_and_the_browser_gets_a_success_page() {
    let (listener, addr) = listening().await;
    let waiting = tokio::spawn(wait_for_code_with(listener, "s", LONG, SHORT));

    let response = get(addr, "/callback?code=abc&state=s").await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("text/html"), "{response}");
    assert_eq!(waiting.await.unwrap().unwrap(), "abc");
}

#[tokio::test]
async fn a_forged_state_gets_a_400_and_fails_the_login() {
    let (listener, addr) = listening().await;
    let waiting = tokio::spawn(wait_for_code_with(listener, "s", LONG, SHORT));

    let response = get(addr, "/callback?code=abc&state=evil").await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    let err = waiting.await.unwrap().unwrap_err();
    assert_eq!(callback_error(&err), Some(&CallbackError::StateMismatch));
}

#[tokio::test]
async fn a_refusal_by_the_user_fails_the_login_with_the_reason() {
    let (listener, addr) = listening().await;
    let waiting = tokio::spawn(wait_for_code_with(listener, "s", LONG, SHORT));

    get(addr, "/callback?error=access_denied&state=s").await;
    let err = waiting.await.unwrap().unwrap_err();
    assert_eq!(
        callback_error(&err),
        Some(&CallbackError::Denied("access_denied".into()))
    );
}

#[tokio::test]
async fn unrelated_requests_get_a_404_and_do_not_end_the_wait() {
    let (listener, addr) = listening().await;
    let waiting = tokio::spawn(wait_for_code_with(listener, "s", LONG, SHORT));

    let response = get(addr, "/favicon.ico").await;
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");
    get(addr, "/callback?code=abc&state=s").await;
    assert_eq!(waiting.await.unwrap().unwrap(), "abc");
}

#[tokio::test]
async fn a_silent_connection_does_not_block_the_real_callback() {
    let (listener, addr) = listening().await;
    let waiting = tokio::spawn(wait_for_code_with(listener, "s", LONG, SHORT));

    let _preconnect = TcpStream::connect(addr).await.unwrap(); // never sends anything
    get(addr, "/callback?code=abc&state=s").await;
    assert_eq!(waiting.await.unwrap().unwrap(), "abc");
}

#[tokio::test]
async fn it_gives_up_when_nothing_arrives() {
    let (listener, _addr) = listening().await;
    let err = wait_for_code_with(listener, "s", SHORT, SHORT)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
}

#[tokio::test]
async fn binding_a_busy_port_explains_what_happened() {
    let (_first, addr) = listening().await;
    let err = bind(addr.port()).await.unwrap_err().to_string();
    assert!(err.contains(&addr.port().to_string()), "{err}");
    assert!(err.contains("in use"), "{err}");
}
