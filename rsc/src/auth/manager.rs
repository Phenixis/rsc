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
mod tests;
