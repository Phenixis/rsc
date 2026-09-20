//! The HTTP server: routing, the shared state behind one lock, and the `MockServer` handle.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::routing::any;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

use crate::clock::Clock;
use crate::config::{MockConfig, RegisteredClient, UserDecision};
use crate::error::ApiError;
use crate::oauth::store::Store;
use crate::oauth::{authorize, token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Issued for the authenticated user (authorization_code grant, then refresh_token grant).
    User,
    /// Issued to the app alone (client_credentials grant): no user behind it.
    App,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenInfo {
    pub kind: TokenKind,
    pub client_id: String,
    /// Time left before the token expires, at the current mock time. Never zero.
    pub remaining: Duration,
}

/// What every request handler shares.
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) config: MockConfig,
    /// One lock around everything that changes, so that checking a code or a refresh token and
    /// consuming it is atomic, and the clock cannot move in the middle of a request.
    state: Mutex<State>,
}

/// The mutable part of the server.
#[derive(Debug)]
pub(crate) struct State {
    pub(crate) clock: Clock,
    pub(crate) user_decision: UserDecision,
    pub(crate) store: Store,
}

impl Shared {
    fn new(config: MockConfig) -> Self {
        let state = State {
            clock: Clock::default(),
            user_decision: config.user_decision,
            store: Store::new(
                config.access_token_lifetime,
                config.authorization_code_lifetime,
            ),
        };
        Self {
            config,
            state: Mutex::new(state),
        }
    }

    /// Locks the state. Every critical section leaves it consistent, so a panic in another
    /// request handler (which poisons the lock) is no reason to stop serving.
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The registered client with this exact id.
    pub(crate) fn client(&self, client_id: &str) -> Option<&RegisteredClient> {
        self.config
            .clients
            .iter()
            .find(|client| client.client_id == client_id)
    }
}

/// Binds 127.0.0.1 on an ephemeral port and serves on a background tokio task.
/// Must be called inside a tokio runtime.
///
/// Fails with `ErrorKind::InvalidInput` when the configuration is invalid, without starting
/// anything.
pub async fn spawn(config: MockConfig) -> io::Result<MockServer> {
    config
        .validate()
        .map_err(|reason| io::Error::new(io::ErrorKind::InvalidInput, reason))?;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;

    let shared = Arc::new(Shared::new(config));
    let task = tokio::spawn(accept_loop(listener, router(Arc::clone(&shared))));
    Ok(MockServer { addr, shared, task })
}

/// A running mock server. Dropping it stops the server.
#[derive(Debug)]
pub struct MockServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl MockServer {
    /// `http://127.0.0.1:<port>` (no trailing slash).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stops accepting connections, drops the open ones, and returns once the listening
    /// socket is closed. Dropping a `MockServer` without calling this also stops the server.
    pub async fn shutdown(mut self) {
        self.task.abort();
        // The task owns the listener; it is closed once the task has been cancelled. The
        // result is only ever "cancelled".
        let _ = (&mut self.task).await;
    }

    /// Mock time elapsed since `spawn` (starts at zero).
    pub fn elapsed(&self) -> Duration {
        self.shared.lock().clock.now()
    }

    /// Moves the mock clock forward. The mock clock never moves on its own in the library.
    pub fn advance_clock(&self, by: Duration) {
        self.shared.lock().clock.advance(by);
    }

    /// Changes the user's answer to future /authorize requests.
    pub fn set_user_decision(&self, decision: UserDecision) {
        self.shared.lock().user_decision = decision;
    }

    /// `Some` iff `access_token` was issued by this server, has not been revoked and has not
    /// expired at the current mock time. Used by the API slices to authenticate requests.
    pub fn token_info(&self, access_token: &str) -> Option<TokenInfo> {
        let state = self.shared.lock();
        state
            .store
            .access_token_info(access_token, state.clock.now())
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Every route of the mock. Paths match exactly and case-sensitively; the handlers of the
/// two OAuth endpoints answer wrong methods themselves, with SoundCloud's error body.
fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/authorize", any(authorize::handler))
        .route("/oauth/token", any(token::handler))
        .fallback(not_found)
        .with_state(shared)
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}

/// Accepts connections until the task is cancelled. The connection tasks live in a `JoinSet`
/// owned by this one, so cancelling it closes the listener *and* every open connection.
async fn accept_loop(listener: TcpListener, app: Router) {
    let mut connections = JoinSet::new();
    loop {
        // Reap the connections that are over.
        while connections.try_join_next().is_some() {}

        let stream = match listener.accept().await {
            Ok((stream, _peer)) => stream,
            Err(_) => {
                // Typically out of file descriptors: back off instead of spinning.
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };

        let service = TowerToHyperService::new(app.clone());
        connections.spawn(async move {
            // A connection ending in error (client reset, malformed or oversized request) only
            // concerns that client.
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}
