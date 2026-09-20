//! Keeps a valid access token available: loads it, refreshes it when it expires (refresh
//! tokens are single-use, so refreshes are serialised and saved *before* anything else),
//! and completes a login.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex;

use super::client::{TokenClient, TokenError};
use super::store::TokenStore;
use super::tokens::Tokens;

/// Unix seconds. A trait so tests can move time without sleeping.
pub trait Clock: Send + Sync {
    fn now(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("not logged in: run `rsc login`")]
    NotLoggedIn,
    #[error("your session has expired or was revoked: run `rsc login` again")]
    SessionExpired,
    /// Reading or writing the token file failed. If it happens right after a refresh, the
    /// old refresh token is already spent: the message must say where the problem is.
    #[error("token storage problem: {0:#}")]
    Storage(anyhow::Error),
    #[error(transparent)]
    Token(#[from] TokenError),
}

pub struct AuthManager {
    store: TokenStore,
    client: TokenClient,
    clock: Arc<dyn Clock>,
    /// Held for the whole refresh: refresh tokens are single-use, so two concurrent
    /// refreshes with the same token would burn it and log the user out.
    refresh_lock: Mutex<()>,
}

impl AuthManager {
    pub fn new(store: TokenStore, client: TokenClient, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            client,
            clock,
            refresh_lock: Mutex::new(()),
        }
    }

    /// An access token that is valid now, refreshing it first if needed.
    pub async fn get_valid_token(&self) -> Result<String, AuthError> {
        let tokens = self.load().await?;
        if !tokens.is_expired(self.clock.now()) {
            return Ok(tokens.access_token);
        }
        self.refresh_if(|stored, now| stored.is_expired(now)).await
    }

    /// The API rejected `rejected` (HTTP 401). Returns a token to retry with: the stored one
    /// if somebody else already refreshed, a freshly refreshed one otherwise.
    pub async fn refresh_after_unauthorized(&self, rejected: &str) -> Result<String, AuthError> {
        self.refresh_if(|stored, _| stored.access_token == rejected)
            .await
    }

    /// Second half of a login: trades the authorization code for tokens and saves them.
    /// Existing tokens are only replaced on success.
    pub async fn complete_login(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<(), AuthError> {
        let response = self
            .client
            .exchange_code(code, code_verifier, redirect_uri)
            .await?;
        let tokens = Tokens::from_response(response, self.clock.now());
        self.store.save(&tokens).await.map_err(AuthError::Storage)
    }

    /// Refreshes under the lock, unless `needs_refresh` says the stored tokens are fine.
    ///
    /// The check is made *again* once the lock is ours: while we waited, another caller may
    /// have refreshed, and spending the refresh token a second time would be fatal. The new
    /// tokens are written to disk before anyone gets to use them.
    async fn refresh_if(
        &self,
        needs_refresh: impl Fn(&Tokens, u64) -> bool,
    ) -> Result<String, AuthError> {
        let _guard = self.refresh_lock.lock().await;

        let stored = self.load().await?;
        if !needs_refresh(&stored, self.clock.now()) {
            return Ok(stored.access_token);
        }

        let response =
            self.client
                .refresh(&stored.refresh_token)
                .await
                .map_err(|err| match err {
                    TokenError::InvalidGrant => AuthError::SessionExpired,
                    other => AuthError::Token(other),
                })?;
        let fresh = Tokens::from_response(response, self.clock.now());
        self.store.save(&fresh).await.map_err(AuthError::Storage)?;
        Ok(fresh.access_token)
    }

    async fn load(&self) -> Result<Tokens, AuthError> {
        match self.store.load().await {
            Ok(Some(tokens)) => Ok(tokens),
            Ok(None) => Err(AuthError::NotLoggedIn),
            Err(err) => Err(AuthError::Storage(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;
    use tokio::task::JoinSet;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    use super::*;
    use crate::test_support::{Fixture, NOW, token_response_json, tokens};

    const REDIRECT: &str = "http://127.0.0.1:8888/callback";

    // --- the happy paths ----------------------------------------------------------------------

    #[tokio::test]
    async fn a_valid_token_is_returned_without_any_network_call() {
        let f = Fixture::with_tokens(tokens("access-1", "refresh-1", NOW + 1000)).await;

        assert_eq!(f.auth.get_valid_token().await.unwrap(), "access-1");
        assert_eq!(f.total_requests().await, 0);
    }

    #[tokio::test]
    async fn an_expired_token_is_refreshed_with_the_stored_refresh_token() {
        let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
        f.mount_refresh_ok("new-access", "new-refresh").await;

        assert_eq!(f.auth.get_valid_token().await.unwrap(), "new-access");
        assert_eq!(f.refresh_tokens_sent().await, ["old-refresh"]);
    }

    #[tokio::test]
    async fn refreshed_tokens_are_saved_with_an_expiry_computed_from_the_clock() {
        let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
        f.mount_refresh_ok("new-access", "new-refresh").await;

        f.auth.get_valid_token().await.unwrap();
        // expires_in 3600, minus the 60 s safety margin
        assert_eq!(
            f.stored().await,
            Some(tokens("new-access", "new-refresh", NOW + 3600 - 60))
        );
    }

    #[tokio::test]
    async fn a_token_that_expires_later_is_refreshed_once_the_clock_passes_it() {
        let f = Fixture::with_tokens(tokens("a1", "r1", NOW + 100)).await;
        f.mount_refresh_ok("a2", "r2").await;

        assert_eq!(f.auth.get_valid_token().await.unwrap(), "a1");
        f.clock.set(NOW + 100);
        assert_eq!(f.auth.get_valid_token().await.unwrap(), "a2");
        assert_eq!(f.auth.get_valid_token().await.unwrap(), "a2"); // and now it is valid again
        assert_eq!(f.total_requests().await, 1);
    }

    // --- refresh tokens are single-use: the classic ways to lose a login -----------------------

    /// The server burns a refresh token on first use, so a second refresh with the same
    /// token is an `invalid_grant`. Eight callers hit an expired token at once: exactly one
    /// may go to the network, the others must wait and reuse its result.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_callers_cause_exactly_one_refresh() {
        let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
        f.mount_single_use_refresh().await;

        let mut callers = JoinSet::new();
        for _ in 0..8 {
            let auth = f.auth.clone();
            callers.spawn(async move { auth.get_valid_token().await });
        }
        while let Some(result) = callers.join_next().await {
            assert_eq!(result.unwrap().unwrap(), "new-access");
        }
        assert_eq!(f.refresh_tokens_sent().await, ["old-refresh"]);
    }

    #[tokio::test]
    async fn a_refused_refresh_token_means_the_user_must_log_in_again() {
        let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
        f.mount_token_endpoint(
            ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})),
        )
        .await;

        let err = f.auth.get_valid_token().await.unwrap_err();
        assert!(matches!(err, AuthError::SessionExpired), "{err:?}");
        assert_eq!(
            f.stored().await,
            Some(tokens("old-access", "old-refresh", NOW - 1)),
            "file must be untouched"
        );
    }

    #[tokio::test]
    async fn a_server_error_is_not_a_logout_and_the_next_attempt_can_succeed() {
        let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&f.server)
            .await;
        f.mount_refresh_ok("new-access", "new-refresh").await;

        let err = f.auth.get_valid_token().await.unwrap_err();
        assert!(
            matches!(err, AuthError::Token(TokenError::Status(503))),
            "{err:?}"
        );
        assert_eq!(
            f.stored().await,
            Some(tokens("old-access", "old-refresh", NOW - 1)),
            "refresh token kept"
        );

        assert_eq!(f.auth.get_valid_token().await.unwrap(), "new-access");
    }

    /// The new refresh token exists nowhere else: if it cannot be written, pretending
    /// everything is fine would silently log the user out on the next run.
    #[tokio::test]
    async fn if_the_new_tokens_cannot_be_saved_the_call_fails_and_says_where() {
        let f = Fixture::with_tokens(tokens("old-access", "old-refresh", NOW - 1)).await;
        f.mount_refresh_ok("new-access", "new-refresh").await;
        let dir = f.store_path.parent().unwrap().to_owned();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap(); // reading works, writing does not
        let restore =
            || std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        if File::create(dir.join("probe")).is_ok() {
            restore();
            eprintln!("directory permissions are not enforced here (running as root?), skipping");
            return;
        }

        let err = f.auth.get_valid_token().await.unwrap_err();
        restore();
        assert!(matches!(err, AuthError::Storage(_)), "{err:?}");
        assert!(err.to_string().contains("tokens.json"), "{err}");
    }

    // --- not logged in / broken files ---------------------------------------------------------

    #[tokio::test]
    async fn without_stored_tokens_the_user_must_log_in() {
        let f = Fixture::new().await;

        let err = f.auth.get_valid_token().await.unwrap_err();
        assert!(matches!(err, AuthError::NotLoggedIn), "{err:?}");
        assert_eq!(f.total_requests().await, 0);
    }

    #[tokio::test]
    async fn a_corrupted_token_file_is_a_storage_problem_not_a_silent_logout() {
        let f = Fixture::new().await;
        std::fs::write(&f.store_path, b"{ not json").unwrap();

        let err = f.auth.get_valid_token().await.unwrap_err();
        assert!(matches!(err, AuthError::Storage(_)), "{err:?}");
        assert!(err.to_string().contains("tokens.json"), "{err}");
    }

    // --- after a 401 from the API -------------------------------------------------------------

    #[tokio::test]
    async fn after_a_401_the_rejected_token_is_refreshed_even_if_it_looks_valid() {
        let f = Fixture::with_tokens(tokens("access-1", "refresh-1", NOW + 1000)).await;
        f.mount_refresh_ok("access-2", "refresh-2").await;

        assert_eq!(
            f.auth.refresh_after_unauthorized("access-1").await.unwrap(),
            "access-2"
        );
        assert_eq!(f.refresh_tokens_sent().await, ["refresh-1"]);
    }

    #[tokio::test]
    async fn after_a_401_a_token_somebody_else_already_refreshed_is_reused() {
        // The caller was rejected with access-1, but by now access-2 is stored.
        let f = Fixture::with_tokens(tokens("access-2", "refresh-2", NOW + 1000)).await;

        assert_eq!(
            f.auth.refresh_after_unauthorized("access-1").await.unwrap(),
            "access-2"
        );
        assert_eq!(f.total_requests().await, 0);
    }

    // --- login ---------------------------------------------------------------------------------

    #[tokio::test]
    async fn completing_a_login_exchanges_the_code_and_saves_the_tokens() {
        let f = Fixture::new().await;
        f.mount_token_endpoint(
            ResponseTemplate::new(200).set_body_json(token_response_json("a1", "r1", 3600)),
        )
        .await;

        f.auth
            .complete_login("CODE", "VERIFIER", REDIRECT)
            .await
            .unwrap();

        assert_eq!(f.stored().await, Some(tokens("a1", "r1", NOW + 3600 - 60)));
        let form = f.last_token_request_form().await;
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(
            (form["code"].as_str(), form["code_verifier"].as_str()),
            ("CODE", "VERIFIER")
        );
        assert_eq!(form["redirect_uri"], REDIRECT);
    }

    #[tokio::test]
    async fn a_failed_login_leaves_the_existing_tokens_untouched() {
        let f = Fixture::with_tokens(tokens("keep-access", "keep-refresh", NOW + 1000)).await;
        f.mount_token_endpoint(
            ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})),
        )
        .await;

        let err = f
            .auth
            .complete_login("BAD", "V", REDIRECT)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AuthError::Token(TokenError::InvalidGrant)),
            "{err:?}"
        );
        assert_eq!(
            f.stored().await,
            Some(tokens("keep-access", "keep-refresh", NOW + 1000))
        );
    }
}
