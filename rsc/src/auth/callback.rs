//! Loopback listener that receives the OAuth redirect
//! (`http://127.0.0.1:<port>/callback?code=…&state=…`).

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

pub const CALLBACK_PATH: &str = "/callback";

/// Bound on what we read from a connection: this is a one-shot local listener.
const MAX_REQUEST_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackError {
    /// Not an HTTP request line at all.
    #[error("malformed request")]
    Malformed,
    /// A well-formed request for something other than `GET /callback` (favicon, probes…).
    #[error("not the OAuth callback")]
    NotCallback,
    #[error("the `state` parameter does not match: possible forged callback")]
    StateMismatch,
    #[error("authorization refused: {0}")]
    Denied(String),
    #[error("the callback carries no authorization code")]
    MissingCode,
}

/// Extracts the authorization code from the request line of the redirect
/// (e.g. `GET /callback?code=abc&state=xyz HTTP/1.1`), checking `state` first.
pub fn parse_request_line(line: &str, expected_state: &str) -> Result<String, CallbackError> {
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(CallbackError::Malformed);
    };
    if method.is_empty() || !target.starts_with('/') || !version.starts_with("HTTP/") {
        return Err(CallbackError::Malformed);
    }

    // Compared as plain strings: `//host/callback` is a different path, not a redirect target.
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if method != "GET" || path != CALLBACK_PATH {
        return Err(CallbackError::NotCallback);
    }

    let params: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
    let param = |name: &str| {
        params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_ref())
    };

    if param("state") != Some(expected_state) {
        return Err(CallbackError::StateMismatch);
    }
    if let Some(reason) = param("error") {
        return Err(CallbackError::Denied(reason.to_owned()));
    }
    match param("code") {
        Some(code) if !code.is_empty() => Ok(code.to_owned()),
        _ => Err(CallbackError::MissingCode),
    }
}

/// Binds `127.0.0.1:<port>` (loopback only). Do this *before* opening the browser.
pub async fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::AddrInUse => {
                anyhow!("port {port} is already in use (is another rsc login still running?)")
            }
            _ => anyhow!(err).context(format!("cannot listen on 127.0.0.1:{port}")),
        })
}

/// Waits for the redirect and returns the authorization code.
///
/// Unrelated requests get a 404 and are ignored; a forged `state` gets a 400 and fails the
/// login. `overall` bounds the whole wait.
pub async fn wait_for_code(
    listener: TcpListener,
    expected_state: &str,
    overall: Duration,
) -> Result<String> {
    wait_for_code_with(listener, expected_state, overall, Duration::from_secs(2)).await
}

/// Like [`wait_for_code`], with a bound on how long a connection may stay silent
/// (browsers open speculative connections that never send anything).
pub async fn wait_for_code_with(
    listener: TcpListener,
    expected_state: &str,
    overall: Duration,
    per_connection: Duration,
) -> Result<String> {
    let accept_until_callback = async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let Some(line) = read_request_line(&mut stream, per_connection).await else {
                continue;
            };
            match parse_request_line(&line, expected_state) {
                Ok(code) => {
                    respond(&mut stream, "200 OK", SUCCESS_PAGE).await;
                    return Ok(code);
                }
                Err(CallbackError::NotCallback | CallbackError::Malformed) => {
                    respond(&mut stream, "404 Not Found", NOT_FOUND_PAGE).await;
                }
                Err(err) => {
                    respond(&mut stream, "400 Bad Request", FAILURE_PAGE).await;
                    return Err(err.into());
                }
            }
        }
    };
    timeout(overall, accept_until_callback)
        .await
        .map_err(|_| anyhow!("timed out waiting for the login to complete in the browser"))?
}

/// First line of the request. The rest of the headers is drained too: closing a socket
/// with unread data makes the kernel reset the connection and the browser can lose our page.
async fn read_request_line(stream: &mut TcpStream, per_connection: Duration) -> Option<String> {
    let read = async {
        let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES));
        let mut first = String::new();
        if reader.read_line(&mut first).await.ok()? == 0 {
            return None;
        }
        loop {
            let mut header = String::new();
            let n = reader.read_line(&mut header).await.ok()?;
            if n == 0 || header.trim_end().is_empty() {
                return Some(first.trim_end().to_owned());
            }
        }
    };
    timeout(per_connection, read).await.ok().flatten()
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The browser may already be gone; there is nothing useful to do about it.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

const SUCCESS_PAGE: &str = "<!doctype html><meta charset=utf-8><title>rsc</title>\
    <p>Login successful. You can close this tab and go back to rsc.</p>";
const FAILURE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>rsc</title>\
    <p>Login failed. See the terminal for details.</p>";
const NOT_FOUND_PAGE: &str = "<!doctype html><meta charset=utf-8><title>rsc</title>\
    <p>Not found.</p>";

#[cfg(test)]
mod tests {
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
        let request =
            format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
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
}
