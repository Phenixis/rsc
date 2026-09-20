//! Fixtures shared by the auth and API unit tests: a fake SoundCloud (wiremock), a fake
//! clock, and an `AuthManager` wired to both.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::auth::store::TokenStore;
use crate::auth::tokens::Tokens;
use crate::auth::{AuthManager, Clock, TokenClient};

pub const NOW: u64 = 1_000_000;
pub const CLIENT_ID: &str = "client-id";
pub const CLIENT_SECRET: &str = "client-secret";

pub struct FakeClock(AtomicU64);

impl FakeClock {
    pub fn at(now: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(now)))
    }

    pub fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub fn tokens(access: &str, refresh: &str, expires_at: u64) -> Tokens {
    Tokens {
        access_token: access.into(),
        refresh_token: refresh.into(),
        expires_at,
    }
}

/// Body of a successful `POST /oauth/token`.
pub fn token_response_json(access: &str, refresh: &str, expires_in: u64) -> serde_json::Value {
    json!({
        "access_token": access,
        "refresh_token": refresh,
        "expires_in": expires_in,
        "scope": "",
        "token_type": "bearer",
    })
}

fn form_of(request: &Request) -> BTreeMap<String, String> {
    url::form_urlencoded::parse(&request.body)
        .into_owned()
        .collect()
}

/// Like the real server: a refresh token works once. Replaying it is an `invalid_grant`.
/// Answers slowly, to give concurrent callers time to pile up.
struct SingleUseRefresh {
    spent: Mutex<HashSet<String>>,
}

impl Respond for SingleUseRefresh {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let token = form_of(request).remove("refresh_token").unwrap_or_default();
        if self.spent.lock().unwrap().insert(token) {
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(150))
                .set_body_json(token_response_json("new-access", "new-refresh", 3600))
        } else {
            ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" }))
        }
    }
}

pub struct Fixture {
    pub server: MockServer,
    /// Owns the temporary folder: it is deleted when the fixture is dropped.
    pub _dir: tempfile::TempDir,
    pub store_path: PathBuf,
    pub clock: Arc<FakeClock>,
    pub auth: Arc<AuthManager>,
}

impl Fixture {
    /// Nobody is logged in yet; the clock reads [`NOW`].
    pub async fn new() -> Self {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("tokens.json");
        let clock = FakeClock::at(NOW);
        let client = TokenClient::new(&server.uri(), CLIENT_ID, CLIENT_SECRET).unwrap();
        let auth = Arc::new(AuthManager::new(
            TokenStore::new(&store_path),
            client,
            clock.clone(),
        ));
        Self {
            server,
            _dir: dir,
            store_path,
            clock,
            auth,
        }
    }

    pub async fn with_tokens(tokens: Tokens) -> Self {
        let fixture = Self::new().await;
        TokenStore::new(&fixture.store_path)
            .save(&tokens)
            .await
            .unwrap();
        fixture
    }

    /// What is on disk right now.
    pub async fn stored(&self) -> Option<Tokens> {
        TokenStore::new(&self.store_path).load().await.unwrap()
    }

    pub async fn mount_token_endpoint(&self, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(response)
            .mount(&self.server)
            .await;
    }

    pub async fn mount_refresh_ok(&self, access: &str, refresh: &str) {
        self.mount_token_endpoint(
            ResponseTemplate::new(200).set_body_json(token_response_json(access, refresh, 3600)),
        )
        .await;
    }

    pub async fn mount_single_use_refresh(&self) {
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(SingleUseRefresh {
                spent: Mutex::new(HashSet::new()),
            })
            .mount(&self.server)
            .await;
    }

    pub async fn total_requests(&self) -> usize {
        self.server.received_requests().await.unwrap().len()
    }

    pub async fn requests_to(&self, request_path: &str) -> usize {
        let all = self.server.received_requests().await.unwrap();
        all.iter().filter(|r| r.url.path() == request_path).count()
    }

    /// The `refresh_token` field of every refresh request received, in order.
    pub async fn refresh_tokens_sent(&self) -> Vec<String> {
        let all = self.server.received_requests().await.unwrap();
        all.iter()
            .filter(|r| r.url.path() == "/oauth/token")
            .filter_map(|r| form_of(r).remove("refresh_token"))
            .collect()
    }

    pub async fn last_token_request_form(&self) -> BTreeMap<String, String> {
        let all = self.server.received_requests().await.unwrap();
        form_of(
            all.iter()
                .rfind(|r| r.url.path() == "/oauth/token")
                .expect("no token request"),
        )
    }
}
