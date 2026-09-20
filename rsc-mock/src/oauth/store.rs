//! Everything the authorization server remembers: codes, access tokens, refresh tokens and
//! the token families that let a replayed code revoke what it produced.
//!
//! The store is a plain data structure without locking or clock of its own. The server keeps
//! it behind one mutex, so each method below is a single critical section: checking a code or a
//! refresh token and consuming it can never interleave with another request.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::pkce;
use crate::server::{TokenInfo, TokenKind};

/// Random bytes behind each credential: 256 bits, which encode to 43 URL-safe characters.
const CREDENTIAL_BYTES: usize = 32;

/// The set of tokens descended from one authorization code (the code itself, then every
/// refresh). Replaying the code revokes the whole family (RFC 6749 section 4.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FamilyId(u64);

#[derive(Debug)]
struct AuthCode {
    client_id: String,
    /// Exactly as sent to `/authorize`; the token request must repeat it (RFC 6749 4.1.3).
    redirect_uri: String,
    code_challenge: String,
    issued_at: Duration,
    family: FamilyId,
    /// Redeemed codes are kept so that a replay, even after expiry, is recognized as one.
    redeemed: bool,
}

#[derive(Debug)]
struct AccessToken {
    kind: TokenKind,
    client_id: String,
    expires_at: Duration,
    /// App tokens have no authorization behind them, hence no family.
    family: Option<FamilyId>,
}

#[derive(Debug)]
struct RefreshToken {
    client_id: String,
    family: FamilyId,
    /// Refresh tokens are single-use; spent ones are kept to tell a replay from garbage.
    spent: bool,
}

/// Tokens handed out by a successful grant.
#[derive(Debug)]
pub(crate) struct Issued {
    pub(crate) access_token: String,
    /// `None` for `client_credentials`: an app can always authenticate again.
    pub(crate) refresh_token: Option<String>,
}

/// A credential the server cannot honour. The reason is deliberately not reported: telling
/// "unknown" from "expired" from "wrong client" would help an attacker, and the wire error is
/// the same for all of them.
#[derive(Debug)]
pub(crate) struct InvalidGrant;

/// The parameters of an `authorization_code` token request, once client-authenticated.
#[derive(Debug)]
pub(crate) struct CodeRedemption<'a> {
    pub(crate) client_id: &'a str,
    pub(crate) code: &'a str,
    pub(crate) redirect_uri: &'a str,
    pub(crate) code_verifier: &'a str,
}

#[derive(Debug)]
pub(crate) struct Store {
    access_token_lifetime: Duration,
    authorization_code_lifetime: Duration,
    codes: HashMap<String, AuthCode>,
    access_tokens: HashMap<String, AccessToken>,
    refresh_tokens: HashMap<String, RefreshToken>,
    revoked_families: HashSet<FamilyId>,
    next_family: u64,
}

impl Store {
    pub(crate) fn new(
        access_token_lifetime: Duration,
        authorization_code_lifetime: Duration,
    ) -> Self {
        Self {
            access_token_lifetime,
            authorization_code_lifetime,
            codes: HashMap::new(),
            access_tokens: HashMap::new(),
            refresh_tokens: HashMap::new(),
            revoked_families: HashSet::new(),
            next_family: 0,
        }
    }

    /// Records an approved authorization request and returns its new code.
    pub(crate) fn issue_code(
        &mut self,
        client_id: &str,
        redirect_uri: &str,
        code_challenge: &str,
        now: Duration,
    ) -> String {
        let family = FamilyId(self.next_family);
        self.next_family += 1;

        let code = self.fresh_credential();
        self.codes.insert(
            code.clone(),
            AuthCode {
                client_id: client_id.to_owned(),
                redirect_uri: redirect_uri.to_owned(),
                code_challenge: code_challenge.to_owned(),
                issued_at: now,
                family,
                redeemed: false,
            },
        );
        code
    }

    /// Exchanges a code for user tokens.
    ///
    /// A request that fails on the redirect URI or the verifier leaves the code usable, so a
    /// stray request cannot burn a legitimate login. Only a success consumes it.
    pub(crate) fn redeem_code(
        &mut self,
        request: &CodeRedemption<'_>,
        now: Duration,
    ) -> Result<Issued, InvalidGrant> {
        let code = self.codes.get_mut(request.code).ok_or(InvalidGrant)?;

        // A code belongs to the client it was issued to. Another client presenting it (RFC 6749
        // section 4.1.3) is refused without touching anything: it is not evidence that the
        // code leaked from its owner.
        if code.client_id != request.client_id {
            return Err(InvalidGrant);
        }

        // RFC 6749 section 4.1.2: when a code is used more than once, the server SHOULD revoke
        // every token previously issued from it. Checked before expiry so that a late replay
        // still counts.
        if code.redeemed {
            self.revoked_families.insert(code.family);
            return Err(InvalidGrant);
        }

        if now
            >= code
                .issued_at
                .saturating_add(self.authorization_code_lifetime)
        {
            return Err(InvalidGrant);
        }
        if code.redirect_uri != request.redirect_uri
            || !pkce::verify_s256(request.code_verifier, &code.code_challenge)
        {
            return Err(InvalidGrant);
        }

        code.redeemed = true;
        let family = code.family;
        Ok(self.issue_user_tokens(request.client_id, family, now))
    }

    /// Exchanges a refresh token for a new access token and a new refresh token; the old
    /// refresh token is spent.
    ///
    /// Refresh tokens never expire. Replaying a spent one is refused but does not revoke the
    /// tokens issued since (unlike a replayed code), so a client that lost a response to a
    /// network error is not locked out of the session it just refreshed.
    pub(crate) fn refresh(
        &mut self,
        client_id: &str,
        refresh_token: &str,
        now: Duration,
    ) -> Result<Issued, InvalidGrant> {
        let entry = self
            .refresh_tokens
            .get_mut(refresh_token)
            .ok_or(InvalidGrant)?;

        if entry.client_id != client_id
            || entry.spent
            || self.revoked_families.contains(&entry.family)
        {
            return Err(InvalidGrant);
        }

        entry.spent = true;
        let family = entry.family;
        Ok(self.issue_user_tokens(client_id, family, now))
    }

    /// A token for the app alone (`client_credentials`): no user, no refresh token.
    pub(crate) fn issue_app_token(&mut self, client_id: &str, now: Duration) -> Issued {
        let access_token = self.insert_access_token(TokenKind::App, client_id, None, now);
        Issued {
            access_token,
            refresh_token: None,
        }
    }

    /// The state of an access token at `now`, or `None` if it is unknown, revoked or expired.
    pub(crate) fn access_token_info(&self, token: &str, now: Duration) -> Option<TokenInfo> {
        let access_token = self.access_tokens.get(token)?;
        if access_token
            .family
            .is_some_and(|family| self.revoked_families.contains(&family))
        {
            return None;
        }
        // A token is valid while `now < expires_at`, so a valid one always has time left.
        let remaining = access_token
            .expires_at
            .checked_sub(now)
            .filter(|remaining| !remaining.is_zero())?;
        Some(TokenInfo {
            kind: access_token.kind,
            client_id: access_token.client_id.clone(),
            remaining,
        })
    }

    fn issue_user_tokens(&mut self, client_id: &str, family: FamilyId, now: Duration) -> Issued {
        let access_token = self.insert_access_token(TokenKind::User, client_id, Some(family), now);
        let refresh_token = self.fresh_credential();
        self.refresh_tokens.insert(
            refresh_token.clone(),
            RefreshToken {
                client_id: client_id.to_owned(),
                family,
                spent: false,
            },
        );
        Issued {
            access_token,
            refresh_token: Some(refresh_token),
        }
    }

    fn insert_access_token(
        &mut self,
        kind: TokenKind,
        client_id: &str,
        family: Option<FamilyId>,
        now: Duration,
    ) -> String {
        let token = self.fresh_credential();
        self.access_tokens.insert(
            token.clone(),
            AccessToken {
                kind,
                client_id: client_id.to_owned(),
                // Always the full lifetime from issuance, whatever the clock reads.
                expires_at: now.saturating_add(self.access_token_lifetime),
                family,
            },
        );
        token
    }

    /// A random credential that no code, access token or refresh token has ever used. With 256
    /// random bits the loop never repeats; it makes "unique across everything issued" a
    /// guarantee instead of a probability.
    fn fresh_credential(&self) -> String {
        loop {
            let candidate = random_credential();
            if !self.codes.contains_key(&candidate)
                && !self.access_tokens.contains_key(&candidate)
                && !self.refresh_tokens.contains_key(&candidate)
            {
                return candidate;
            }
        }
    }
}

/// An opaque credential: 256 bits from the operating system's CSPRNG, as unpadded base64url
/// (43 characters of `[A-Za-z0-9_-]`).
fn random_credential() -> String {
    let mut bytes = [0u8; CREDENTIAL_BYTES];
    // Without OS randomness the server cannot issue safe credentials at all, and there is no
    // sensible way to keep serving.
    getrandom::fill(&mut bytes).expect("the operating system's random number generator failed");
    URL_SAFE_NO_PAD.encode(bytes)
}
