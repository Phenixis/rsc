# rsc-mock: a faithful fake of the SoundCloud API (overview)

This file is the **overview** shared by all `rsc-mock` slices. It is not a feature spec: it has
no behavior ids and no tests. Each slice has its own spec (`specs/mock-*.md`) with numbered
behaviors and tests. Where a slice spec and this overview disagree, the slice spec wins for
that slice's behaviors, and the disagreement must be fixed here.

What we know (and do not know) about the real API is in `specs/soundcloud-api-notes.md`; the
official OpenAPI description is `.cache/soundcloud-openapi/api.yaml` (fetched by
`scripts/fetch-openapi.sh`, not committed).

## Purpose

An official SoundCloud app needs an Artist Pro account (100 EUR/year). Contributors without
credentials must still be able to develop and test `rsc` end to end: login (PKCE), refresh,
playlists, pagination, blocked/preview tracks, HLS streaming, failures. `rsc-mock` is a
serious fake that copies the **real** behavior of SoundCloud (as far as we know it), including
its inconvenient parts, so that bugs in `rsc` show up against the mock rather than in front of
a real user. It is not a toy that says yes to everything.

Rules for every slice:

- Copy the real API, not RFC idealism: SoundCloud's own error body shape, `OAuth <token>`
  authorization header, URNs, `next_href` pagination, single-use refresh tokens.
- When the real behavior is unverified, the slice spec says so (`Assumption:`), picks the most
  defensive reading, and documents it, so that it is corrected once real credentials exist.
- Strictness over leniency: an exact `redirect_uri` match, mandatory PKCE, single-use codes.
- Deterministic: time is a manual clock in the library (tests never sleep for an hour), tokens
  are random but the dataset is fixed.

## Crate layout

- New workspace member **`rsc-mock`** (directory `rsc-mock/`, package name `rsc-mock`, library
  name `rsc_mock`, edition 2024 like `rsc`). The workspace `Cargo.toml` lists `members =
  ["rsc", "rsc-mock"]`.
- A **library** (`src/lib.rs`) that contains the whole mock. It is used in-process by tests of
  other crates (`rsc`'s integration tests call `rsc_mock::spawn(...)`) and by the binary.
- A **binary** `rsc-mock` (`src/main.rs`): added by the `mock-faults` slice (CLI flags, real
  clock, control endpoints). Until then the crate is a library only. The binary must stay a
  thin shell around the library.
- Suggested stack (not imposed): `axum` on `tokio`, `serde`/`serde_json`, `url`, `sha2`,
  `base64`, `rand`. No dependency on the `rsc` crate.
- The mock is one HTTP server on **one host**: the same origin answers the authorization
  endpoints (`/authorize`, `/oauth/token`, real host `secure.soundcloud.com`) and the API
  endpoints (real host `api.soundcloud.com`). A client is pointed at `base_url()` for both
  bases (`RSC_AUTH_BASE` and `RSC_API_BASE` in the CLI).

## Foundation public API

Used by every slice. Defined here; the `mock-oauth` slice implements and tests it. All items
are re-exported at the crate root unless a module path is given.

```rust
// rsc_mock (crate root)
pub use config::{MockConfig, RegisteredClient, UserDecision};
pub use server::{MockServer, TokenInfo, TokenKind, spawn};
pub mod pkce; // see specs/mock-oauth.md

// ---- config ----
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_secret: String,
    /// Exact-match list. An empty list means the client can never complete /authorize.
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserDecision {
    /// The user approves every authorization request automatically.
    Approve,
    /// The user refuses every authorization request (`error=access_denied`).
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockConfig {
    pub clients: Vec<RegisteredClient>,
    /// Lifetime of access tokens; also the `expires_in` of token responses.
    /// A non-zero whole number of seconds.
    pub access_token_lifetime: std::time::Duration,
    /// Lifetime of authorization codes. A non-zero whole number of seconds.
    pub authorization_code_lifetime: std::time::Duration,
    pub user_decision: UserDecision,
}

impl Default for MockConfig {
    // one client: client_id "mock-client-id", client_secret "mock-client-secret",
    //   redirect_uris ["http://127.0.0.1:8888/callback"];
    // access_token_lifetime 3599 s; authorization_code_lifetime 600 s;
    // user_decision Approve.
}

// ---- server ----
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
    pub remaining: std::time::Duration,
}

/// Binds 127.0.0.1 on an ephemeral port and serves on a background tokio task.
/// Must be called inside a tokio runtime.
pub async fn spawn(config: MockConfig) -> std::io::Result<MockServer>;

pub struct MockServer { /* private; Send + Sync + 'static */ }

impl MockServer {
    /// `http://127.0.0.1:<port>` (no trailing slash).
    pub fn base_url(&self) -> String;
    pub fn addr(&self) -> std::net::SocketAddr;
    /// Stops accepting connections and returns once the listening socket is closed.
    /// Dropping a `MockServer` without calling this also stops the server.
    pub async fn shutdown(self);

    /// Mock time elapsed since `spawn` (starts at zero).
    pub fn elapsed(&self) -> std::time::Duration;
    /// Moves the mock clock forward. The mock clock never moves on its own in the library.
    pub fn advance_clock(&self, by: std::time::Duration);

    /// Changes the user's answer to future /authorize requests.
    pub fn set_user_decision(&self, decision: UserDecision);

    /// `Some` iff `access_token` was issued by this server, has not been revoked and has not
    /// expired at the current mock time. Used by the API slices to authenticate requests.
    pub fn token_info(&self, access_token: &str) -> Option<TokenInfo>;
}
```

The mock clock: all lifetimes (access token, authorization code) are measured on it, never on
the wall clock. The `mock-faults` slice adds a real-time mode for the binary; the library
default stays manual so that tests are deterministic.

## Conventions

- JSON responses use `Content-Type: application/json; charset=utf-8`.
- Errors use SoundCloud's own body, **not** RFC 6749's `{"error": ...}`:
  `{"code": <HTTP status, integer>, "message": "<text>", "link": "<url>"}`, and nothing else.
  For OAuth failures `message` is the OAuth error code (`invalid_client`, `invalid_grant`,
  `unsupported_grant`, ...) and `link` is
  `https://developers.soundcloud.com/docs/api/guide#authentication`. For API failures `message`
  is a human-readable detail and `link` is
  `https://developers.soundcloud.com/docs/api/explorer/open-api`.
- Authorization header for the API: `Authorization: OAuth <access_token>`.
- Identifiers are URNs: `soundcloud:tracks:1001`, `soundcloud:users:1000`,
  `soundcloud:playlists:2001`. Objects carry `urn`, `kind`, `uri`, `permalink_url`.
- Pagination: `linked_partitioning=true` gives `{"collection": [...], "next_href": "..."}`.
- Tokens, codes: opaque, unpredictable (CSPRNG), URL-safe.

## Built-in dataset (used from `mock-api` on)

Fixed and deterministic. Later slices may add more but must not change these.

- **One authenticated persona**: URN `soundcloud:users:1000`, username `mock-listener`.
  Every user token belongs to this persona.
- **Tracks** have URNs `soundcloud:tracks:<n>`. `access` is `playable` unless stated.
- **Playlist `soundcloud:playlists:2001`** ("Eight tracks"): 8 tracks
  `soundcloud:tracks:1001` .. `soundcloud:tracks:1008`, in that order; `1003` is `preview`,
  `1005` is `blocked`, the other six are `playable`.
- **Playlist `soundcloud:playlists:2002`** ("Empty"): no tracks.
- **Playlist `soundcloud:playlists:2003`** ("One hundred twenty tracks"): 120 tracks
  `soundcloud:tracks:1101` .. `soundcloud:tracks:1220`, all `playable`, for pagination.
- All three belong to the persona and appear in `GET /me/playlists`.

## Slices

1. **mock-oauth**: server foundation (config, spawn, manual clock) and the OAuth authorization
   server: `GET /authorize`, `POST /oauth/token` (authorization_code + PKCE, single-use refresh
   tokens, client_credentials), token validity query for the other slices.
2. **mock-api**: the API endpoints over the dataset: `GET /me`, `/me/playlists`,
   `/playlists/{urn}` (+ `/tracks`), `/tracks/{urn}`, `/tracks?urns=`; `OAuth` header
   authentication (user vs app tokens), `access` filtering, `next_href` pagination, 401/404
   bodies.
3. **mock-media**: HLS streams: `GET /tracks/{urn}/streams` and the media playlists and audio
   segments they point to, `preview` handling, blocked tracks, served from generated fixtures.
4. **mock-faults**: fault injection (`fail-next` 401/429/503, truncated playlists, slow token
   endpoint, rejected refresh), control endpoints, the `rsc-mock` binary with its CLI flags, and
   a real-time clock mode.
5. **mock-contract**: every mock response validated against the official OpenAPI schemas
   (`.cache/soundcloud-openapi/api.yaml`), so that the mock cannot drift from the documented
   contract.

## Consequences for `rsc`

- `rsc::auth::client::TokenClient` reads RFC-style `{"error": ...}` bodies; the real API (and
  this mock) answer `{"code", "message", "link"}`. Against the mock, `invalid_grant` and
  `invalid_client` will surface as `TokenError::Status` until `TokenClient` is fixed. That is a
  separate follow-up feature, not part of `rsc-mock`.
- `rsc` needs `RSC_AUTH_BASE` / `RSC_API_BASE` (or equivalent) to point at `base_url()`.

## Change log
- 2026-09-20: initial overview (purpose, layout, foundation API, dataset, slices).
