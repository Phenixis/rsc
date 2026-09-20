---
feature: mock-oauth
summary: rsc-mock server foundation and the OAuth 2.1 authorization server (authorize, token, PKCE)
crates: [rsc-mock]
tests:
  - rsc-mock/src/pkce/tests.rs
  - rsc-mock/tests/common/mod.rs
  - rsc-mock/tests/server.rs
  - rsc-mock/tests/oauth_authorize.rs
  - rsc-mock/tests/oauth_token.rs
  - rsc-mock/tests/oauth_refresh.rs
  - rsc-mock/tests/oauth_flow.rs
  - rsc-mock/tests/raw_http.rs
---

# rsc-mock, slice 1: server foundation and OAuth authorization server

## Purpose

First slice of `rsc-mock` (overview: `specs/rsc-mock.md`, read it first: purpose, crate layout,
conventions, dataset). It creates the `rsc-mock` library crate and gives it:

- the foundation every other slice builds on: `MockConfig`, `spawn`, `MockServer`, a manual
  clock;
- a faithful fake of SoundCloud's authorization server: `GET /authorize` and
  `POST /oauth/token` (authorization code + mandatory PKCE, single-use rotating refresh
  tokens, client credentials);
- `MockServer::token_info`, the query the API slices use to authenticate requests.

`rsc` (and the tests of other crates) point both `RSC_AUTH_BASE` and `RSC_API_BASE` at
`MockServer::base_url()`.

The real behavior is only partly documented (see `specs/soundcloud-api-notes.md`); every place
where this spec fills a gap is marked **Assumption** and listed under "Assumptions".

## Public API

The foundation API (`MockConfig`, `RegisteredClient`, `UserDecision`, `TokenKind`,
`TokenInfo`, `spawn`, `MockServer` and all its methods) is defined **verbatim** in
`specs/rsc-mock.md`, section "Foundation public API". DEV implements it exactly as written
there. Every item is importable from the crate root (`use rsc_mock::{MockConfig, ...}`).

Additional public module of this slice, `rsc_mock::pkce` (file `rsc-mock/src/pkce.rs`):

```rust
// rsc_mock::pkce

/// `base64url_nopad(sha256(verifier))` (RFC 7636 section 4.2). Does not validate the input.
pub fn s256_challenge(verifier: &str) -> String;

/// RFC 7636 section 4.1: 43 to 128 characters, all from ALPHA / DIGIT / "-" / "." / "_" / "~".
/// Length is counted in characters; any non-ASCII character makes the verifier invalid.
pub fn is_valid_verifier(verifier: &str) -> bool;

/// An S256 challenge is the unpadded base64url of a 32-byte digest: exactly 43 characters
/// from ALPHA / DIGIT / "-" / "_". Padding ("=") and the standard alphabet ("+", "/") are invalid.
pub fn is_valid_challenge(challenge: &str) -> bool;

/// `is_valid_verifier(verifier) && s256_challenge(verifier) == challenge`.
/// The comparison of the two digests should be constant-time.
pub fn verify_s256(verifier: &str, challenge: &str) -> bool;
```

Suggested (not imposed) module layout: `config.rs`, `server.rs`, `clock.rs`, `oauth/` (state:
codes, tokens, families), `pkce.rs`, `error.rs`. Tests only use the crate root and
`rsc_mock::pkce`.

## HTTP contract

Vocabulary: "SoundCloud error" = the body `{"code": <status>, "message": <m>, "link": <l>}`
with `Content-Type: application/json; charset=utf-8` (exactly these three keys, `code` an
integer equal to the HTTP status). The **OAuth link** is
`https://developers.soundcloud.com/docs/api/guide#authentication`; the **API link** is
`https://developers.soundcloud.com/docs/api/explorer/open-api`. "Opaque credential" = a string
of 32 to 128 characters from `[A-Za-z0-9_-]`, generated from a CSPRNG with at least 128 bits
of entropy, never derived from a counter or the clock.

Query and form parameters are decoded as `application/x-www-form-urlencoded` (`+` is a space,
`%XX` is percent-decoding; invalid UTF-8 is replaced lossily). An **empty** parameter counts as
**missing** everywhere below, except `state`, which is echoed even when empty.

### Processing order of `GET /authorize`

1. Method (B7). 2. `client_id` (B12), `redirect_uri` (B13, B14), duplicated `client_id` /
`redirect_uri` (B20): failures here are **never** redirected. 3. Only when the redirect URI is
trusted: `response_type` (B15), `code_challenge` (B16), `code_challenge_method` (B17), other
duplicates (B20), each failure being a redirect with `error=`. 4. The user's decision (B19).
5. Success (B8).

### Processing order of `POST /oauth/token`

1. Method (B7). 2. Content type (B24). 3. Body size (B25). 4. Duplicated parameters (B26).
5. Client authentication (B27 to B31). 6. `grant_type` (B32). 7. Grant-specific parameters
and checks (B36 to B44, B45 to B52, B53 to B55).

## Behaviors

### Foundation

- **B1.** `MockConfig::default()` has exactly one client (`client_id` `mock-client-id`,
  `client_secret` `mock-client-secret`, `redirect_uris` `["http://127.0.0.1:8888/callback"]`),
  `access_token_lifetime` 3599 s, `authorization_code_lifetime` 600 s and `user_decision`
  `Approve`. A server spawned with it issues tokens with `expires_in` 3599.
- **B2.** `spawn` binds `127.0.0.1` on an ephemeral port and serves HTTP. `addr()` is that
  socket address (IP `127.0.0.1`, port non-zero) and `base_url()` is `http://` followed by it,
  without trailing slash. Two servers get different ports and share no state: codes, tokens,
  refresh tokens and the clock of one mean nothing to the other. The server is bound to the
  loopback address `127.0.0.1` only, not to every interface: a connection to the same port on
  another address, such as `127.0.0.2`, is refused.
- **B3.** After `shutdown().await` returns, nothing listens on the old address: a TCP connect
  to it fails.
- **B4.** `spawn` returns `Err` with `ErrorKind::InvalidInput` (and starts nothing) when the
  config is invalid: `access_token_lifetime` or `authorization_code_lifetime` is zero or not a
  whole number of seconds; two clients share a `client_id`; a redirect URI is not an absolute
  URL (empty, or relative like `/callback`) or contains a fragment. A client with no redirect
  URI, and a config with no client at all, are valid (such a client can never finish
  `/authorize`; with no client every authentication fails).
- **B5.** The mock clock starts at zero (`elapsed() == 0`), never moves by itself (not while
  requests are served, not as real time passes), and `advance_clock(d)` adds exactly `d`
  (sub-second amounts and repeated calls accumulate; advancing by zero is a no-op). A real
  pause (the tests sleep 150 ms) changes neither `elapsed()` nor the `remaining` time of a token
  nor the age of a code, to the nanosecond.
- **B6.** A request to a path that no endpoint serves, with any method (`GET`, `POST`, `PUT`,
  `DELETE`, `PATCH`, `OPTIONS`, `HEAD`, extension methods such as `PROPFIND`), is answered `404`
  with the SoundCloud error, `message` `not_found` and the **API link** (for `HEAD`: the same
  status and headers, no body). Paths match exactly and case-sensitively: `/authorize/`,
  `/oauth/token/`, `/OAUTH/TOKEN`, `/oauth2/token`, `/Authorize`, `//authorize`,
  `/oauth//token` are unknown. (Later slices add routes; this behavior is about paths nobody
  serves.)
- **B7.** `/authorize` accepts only `GET` and `HEAD`, and `/oauth/token` only `POST`. Any
  other method (including `OPTIONS` and, for the token endpoint, `HEAD` and `GET`) gets `405`,
  the SoundCloud error with `message` `method_not_allowed` and the **OAuth link**, and an
  `Allow` header naming the accepted method: `GET` for `/authorize` (and no method that is not
  accepted), `POST` for `/oauth/token` (and no other). The method is checked first: a wrong
  method is a `405` whatever the content type, parameters or credentials of the request.
  `HEAD /authorize` behaves as `GET /authorize`: same status and headers (`302` with
  `Location`, `400` for a bad client, ...), no body. `HEAD /oauth/token` is a `405`.

### `GET /authorize`

- **B8.** For a valid request from a known client with a registered redirect URI, and
  `user_decision` `Approve`, the answer is `302` with `Location` = the redirect URI with the
  query parameters `code` then `state` appended (`state` only if the request had one). The
  `code` is an opaque credential. The response has no other effect on the URL: same scheme,
  host, port, path, no fragment. Parameters the mock does not know (`nonce`, `display`, ...) are
  ignored.
- **B9.** Every successful authorization issues a new code: 50 identical requests give 50
  different codes.
- **B10.** `state` is echoed unchanged and correctly encoded, whatever it contains: spaces,
  `&`, `=`, `#`, `%`, `+`, `/`, `?`, newlines, quotes, non-ASCII and emoji, a value that looks
  like `&code=injected`. Encoding is application/x-www-form-urlencoded; the `Location` header is
  pure ASCII, contains no `&`, `#`, space, newline or `"` inside the state value, and parsing
  its query yields exactly the original state. The same holds on error redirects. An empty
  `state=` is echoed as `state=`; an absent `state` gives a `Location` without `state`; a state
  of 4000 characters is echoed in full.
- **B11.** The registered redirect URI is used verbatim as the base of `Location`. If it
  already has a query (`http://127.0.0.1:8888/callback?foo=bar&x=%20y`) that query is kept
  untouched and the new parameters are appended with `&` after it; if it has none, they are
  appended with `?`. Also true for error redirects.
- **B12.** A missing, empty or unknown `client_id` (comparison is exact and case-sensitive) is
  answered `400`, SoundCloud error, `message` `invalid_client`, **OAuth link**, and **no
  `Location` header**. This check comes before the redirect URI check.
- **B13.** With a known client, a `redirect_uri` that is missing, empty or not **byte-for-byte
  equal** to one of that client's registered URIs is answered `400`, SoundCloud error,
  `message` `invalid_request`, **OAuth link**, and no `Location` header. No leniency of any
  kind: `localhost` vs `127.0.0.1`, a trailing slash, another port, scheme or path case,
  an added query or fragment, surrounding spaces, `..` segments, userinfo, an IPv6 host, a
  URL-encoded variant. This check comes before every other request check (including the user's
  decision): a bad redirect URI is never redirected to.
- **B14.** A client may register several redirect URIs; each one works. Redirect URIs are
  per client: another client's URI is a mismatch (`400 invalid_request`), for either client.
- **B15.** With a trusted redirect URI, a `response_type` that is missing or empty gives a
  `302` to the redirect URI with `error=invalid_request`; any other value than exactly `code`
  (`token`, `id_token`, `CODE`, `code token`, `code `) gives `error=unsupported_response_type`.
  Error redirects carry `error` then `state` (if any) and never a `code`.
- **B16.** A `code_challenge` that is missing, empty or not a valid S256 challenge
  (`pkce::is_valid_challenge`: 43 characters of `[A-Za-z0-9_-]`) gives a `302` with
  `error=invalid_request`. PKCE is mandatory. Any well-formed challenge is accepted whatever its
  value.
- **B17.** Only `code_challenge_method=S256` (exact, case-sensitive) is accepted. Missing,
  empty, `plain`, `s256`, `S512`, `sha256`, `S256 `, `S256,plain`: `302` with
  `error=invalid_request`. A missing method does **not** default to `plain`. Checked in this
  order: `response_type`, `code_challenge`, `code_challenge_method`.
- **B18.** `scope` is accepted with any value and ignored; it has no effect on the redirect or
  on issued tokens.
- **B19.** When the user's decision is `Deny`, a request that passed the checks above gets a
  `302` with `error=access_denied` and `state` (no `code`, and no code is stored). Request
  validation comes first: with `Deny`, a bad `response_type` still gives
  `unsupported_response_type`, a bad method `invalid_request`, an unknown client `400
  invalid_client`, a bad redirect URI `400 invalid_request`. `MockServer::set_user_decision`
  changes the answer for subsequent requests on a running server, in both directions.
- **B20.** A parameter present more than once is invalid (RFC 6749 section 3.1). If it is
  `client_id` or `redirect_uri`: `400 invalid_request` without redirect (even when the copies
  are identical, and even when one copy would be valid). For `response_type`, `code_challenge`,
  `code_challenge_method` or `state`: `302` to the (valid) redirect URI with
  `error=invalid_request`, and `state` set to the first occurrence of `state`.
- **B21.** Oversized or empty input never crashes or hangs the server, and never yields a `5xx`.
  A request line or URL larger than the server can read (tested with a 1 MiB `state` and a
  200 KiB `redirect_uri`) is answered with a `4xx` or the connection is closed; a huge
  `redirect_uri` is never redirected to; the server keeps serving afterwards. An empty query
  string gives `400 invalid_client`; empty values are treated as missing (B12, B13, B15 to B17),
  with an empty `state` echoed as `state=`. Below that size, URLs are processed normally: a
  request line with a 60 KiB `state` gets its `302` with the state echoed in full, and a
  60 KiB `redirect_uri`, `client_id` or `code_challenge` is treated like any other wrong value
  (`400 invalid_request` without redirect, `400 invalid_client`, and a `302` with
  `error=invalid_request` respectively).
- **B22.** Every response of `/authorize` (success redirect, error redirect, `400`, `405`, and
  the answer to `HEAD`) carries `Cache-Control: no-store`.
- **B23.** When the request has no `state` parameter, error redirects (B15 to B17, B19) omit
  `state` too: the redirect is `...?error=<code>` and nothing else.

### `POST /oauth/token`: request format and client authentication

- **B24.** The request `Content-Type` must be `application/x-www-form-urlencoded`
  (media type compared case-insensitively; parameters such as `; charset=UTF-8` are allowed).
  Anything else (`application/json`, `text/plain`, multipart, a longer type like
  `application/x-www-form-urlencodedx`, no content type) gets `415`, SoundCloud error,
  `message` `unsupported_media_type`, **OAuth link**, even if the credentials are right, and
  also when the credentials are wrong or missing, the parameters are repeated, the grant is
  bogus, the body is empty or oversized, or the announced `Content-Length` is over the limit
  (B25): the content type is checked first, before anything about the body.
- **B25.** The body is limited to 64 KiB (65 536 bytes). A larger body gets `413`,
  SoundCloud error, `message` `payload_too_large`, **OAuth link**, whatever the credentials
  or parameters it carries (wrong credentials, repeated parameters, a bogus grant: still
  `413`), and it consumes nothing (a valid code or refresh token inside a refused request stays
  usable). A refused body that the client is still sending may make the server close the
  connection; a client may observe that instead of the response, but an answer is never a
  2xx and is always the `413` (`415` if the content type is wrong, B24). A `Content-Length`
  from 65 537 up to and including 2^40 (1 099 511 627 776, about 1 TiB) is refused with `413`
  (SoundCloud error, `Cache-Control: no-store`) right away, **without waiting for the body**
  (a request that announces `Content-Length: 10737418240` and sends nothing, or only a short
  prefix, still gets its `413`, within a few seconds at most). Above 2^40 the HTTP layer
  (hyper) may refuse the request itself before the mock sees it (for
  `Content-Length: 18446744073709551615` hyper answers a bare `431`, and the mock cannot
  change that without a shim in front of hyper, which is not worth it: the value is nil for
  `rsc`): for such a length, and for a `Content-Length` that is not a decimal number that fits
  in a u64 (`-1`, `abc`, `99999999999999999999999`), any `4xx` status or a closed connection is
  accepted, never a 2xx, never a 5xx, and never a wait for the body. The limit applies to
  chunked bodies (`Transfer-Encoding: chunked`) as well, and a chunked body within the limit is
  read like any other. A chunked body over the limit of up to 100 000 bytes (sent in one chunk
  or several, the request fully written by the client) always gets its `413` (the server may
  not close silently), and consumes nothing; a larger chunked body gets the `413` or a closed
  connection, never a 2xx. Bodies up to and including exactly 65 536 bytes, including a single
  30 KiB parameter, are read in full; 65 537 bytes is over.
- **B26.** A parameter that appears more than once in the body (`grant_type`, `refresh_token`,
  `code`, `code_verifier`, `client_id`, `client_secret`, ...) gets `400 invalid_request`. This
  is checked before client authentication and before the grant, and consumes nothing: a code
  or refresh token sent in such a request stays usable. So it is a `400 invalid_request`, not a
  `401`, even when the credentials are wrong, unknown or missing, or the `Authorization` header
  is wrong or malformed, and not `unsupported_grant` for a repeated bogus `grant_type`.
- **B27.** A client is authenticated by either (a) `client_id` and `client_secret` in the form
  body, or (b) an `Authorization: Basic base64(client_id:client_secret)` header (scheme name
  case-insensitive; the decoded value is split at the **first** colon, so secrets may contain
  colons; values are used raw, not percent-decoded). (b) may be accompanied by a body
  `client_id` equal to the header's. Secrets containing symbols work in both forms.
- **B28.** Wrong credentials give `401`, SoundCloud error, `message` `invalid_client`,
  **OAuth link**, and a `WWW-Authenticate` header starting with `Basic`. Wrong means: unknown
  `client_id`, wrong or empty secret (exact comparison; near misses such as a truncated, padded,
  upper-cased secret fail), another client's secret, swapped id and secret. An unknown client
  and a wrong secret produce byte-identical bodies (no client enumeration).
- **B29.** Missing or incomplete credentials give the same `401 invalid_client`: nothing at all
  (even with an empty body), `client_id` without `client_secret` (the secret is required even
  with PKCE), `client_secret` without `client_id`, empty values. If an `Authorization` header is
  present it is the authentication attempt: when it is not valid Basic (not base64, no colon,
  empty secret, a `Bearer`/`OAuth`/`Token` scheme, no scheme, raw non-ASCII bytes in the value)
  the request gets `401 invalid_client` even if the body carries valid credentials. (For raw
  bytes above 0x7F or a NUL byte in a header, the HTTP layer may refuse the request first with
  a `400` that is not the answer to the valid body, or close the connection; what is never
  allowed is to ignore the header and honor the body credentials.)
- **B30.** A well-formed Basic header together with a body `client_secret` (whatever its value)
  gets `400 invalid_request` (RFC 6749 section 2.3: one authentication method per request). A
  body `client_id` that differs from the Basic header's `client_id` gets `401 invalid_client`.
- **B31.** Only the request body counts: credentials or grant parameters in the URL query
  string are ignored (credentials only in the query: `401 invalid_client`; grant parameters
  only in the query: missing, `400 invalid_request`).
- **B32.** Client authentication comes before anything about the grant: bad credentials with a
  bogus, missing or unsupported `grant_type` still give `401 invalid_client`. With valid
  credentials, a missing or empty `grant_type` gives `400 invalid_request`, and a value other
  than exactly `authorization_code`, `refresh_token` or `client_credentials` (`password`,
  `implicit`, `AUTHORIZATION_CODE`, a JWT-bearer URN, `client_credentials `, ...) gives `400`,
  SoundCloud error, `message` `unsupported_grant`, **OAuth link**.

### `POST /oauth/token`: responses

- **B33.** A successful `authorization_code` or `refresh_token` response is `200`,
  `Content-Type: application/json; charset=utf-8`, `Cache-Control: no-store`,
  `Pragma: no-cache`, and a JSON object with exactly the keys `access_token` (string),
  `refresh_token` (string), `expires_in` (JSON integer), `scope` (the empty string `""`) and
  `token_type` (`"bearer"`). Errors of the token endpoint (`400`, `401`, `415`, `413`) are
  SoundCloud errors with the same content type and also carry `Cache-Control: no-store`.
- **B34.** Codes, access tokens and refresh tokens are opaque credentials, unique across
  everything the server ever issued (30 sessions, their refreshes and 10 app tokens: all
  distinct, and no access token equals any refresh token or code). They look random, not
  generated from a counter, the clock or a pattern: over 150 samples of one kind (codes, access
  tokens, refresh tokens, app tokens, taken in issue order) the samples are all distinct and
  well-formed, consecutive samples are in ascending string order between 30 % and 70 % of the
  time (no monotonic sequence), the samples use at least 16 different characters (hexadecimal is
  the poorest alphabet accepted), and their positional entropy is at least 100 bits: the sum
  over character positions (up to the shortest sample) of the empirical Shannon entropy of the
  characters found at that position. A 128-bit random value encoded in base64url or hexadecimal
  scores 120 to 250 in this estimate, a counter padded to 32 characters about 10.
- **B35.** `expires_in` is `access_token_lifetime` in whole seconds (for example 1, 20, 3599,
  7200), for the authorization_code, refresh_token and client_credentials grants alike, and it
  is the **full** lifetime at issuance whatever the mock clock reads (a token issued after
  advancing the clock by 50 000 s still reports 3599 and has 3599 s remaining).

### `authorization_code` grant

- **B36.** `code`, `redirect_uri` and `code_verifier` are all required, with either
  authentication form; a missing or empty one gives `400 invalid_request`. Such a request
  consumes nothing.
- **B37.** An unknown code (garbage, a code with one changed character, different case,
  trailing space, 30 KiB long, or a value that is an access or refresh token) gives `400`,
  SoundCloud error, `message` `invalid_grant`, **OAuth link**. It affects no real code.
- **B38.** `redirect_uri` must be **exactly** the string that was sent to `/authorize`
  (same rules as B13, and another URI registered for the same client is still a mismatch);
  otherwise `400 invalid_grant`. A failed attempt does not consume the code.
- **B39.** `code_verifier` is checked with `pkce::verify_s256` against the challenge stored at
  `/authorize`: a verifier that differs, is malformed (42 or 129 or 5000 characters, characters
  outside the unreserved set such as space, `+`, `/`, `=`, non-ASCII), is the challenge itself,
  or belongs to another authorization gives `400 invalid_grant` (a malformed verifier is
  `invalid_grant`, not `invalid_request`). A failed attempt does not consume the code (see
  Assumptions). Verifiers of exactly 43, 44, 64, 100, 127 and 128 characters work. The RFC 7636
  appendix B pair works end to end.
- **B40.** A code is bound to the `client_id` it was issued to: presenting it with the
  credentials of another client (with either client's redirect URI) gives `400 invalid_grant`
  and has no effect on the code or on tokens already issued from it. Another client can run its
  own flow with its own redirect URIs, and tokens issued to it carry its `client_id`.
- **B41.** A code can be redeemed once. The second and every later attempt with the right
  client, redirect URI and verifier gives `400 invalid_grant`.
- **B42.** Presenting an already redeemed code (after the required parameters were present,
  by the client it was issued to, authenticated, even if its verifier or redirect URI is wrong)
  gives `400 invalid_grant` **and revokes every token of that authorization**: the access
  tokens and refresh tokens issued from the code and from all refreshes descended from it
  (RFC 6749 section 4.1.2). After that `token_info` of those access tokens is `None` and their
  refresh tokens (even never-used ones) give `400 invalid_grant`. Other sessions and app tokens
  are unaffected. (Replay by another client is B40: no revocation.)
- **B43.** A code is valid while `mock time < issue time + authorization_code_lifetime`: with
  the default 600 s it works after advancing 599 s and gives `400 invalid_grant` after 600 s or
  more, and it stays expired. The lifetime is configurable and counts from the moment of
  issuance on the mock clock (time passed before the authorization does not count).
  Codes expire independently of one another.
- **B44.** Redemption is atomic: 24 simultaneous requests redeeming one code produce exactly
  one `200`; all others are `400 invalid_grant`.

### `refresh_token` grant

- **B45.** A refresh returns a new access token and a **new** refresh token (both different
  from the old ones, in the shape of B33, `expires_in` per B35). The old refresh token is
  spent: every later use, however often, gives `400 invalid_grant`; the new one works.
- **B46.** Over many rounds (12 in the tests) of refreshing the latest token, no access or
  refresh token is ever repeated and every refresh token spent earlier stays spent.
- **B47.** The old access token stays valid until its own expiry after a refresh (**Assumption**,
  unverified against the real API): `token_info` still returns it, with `remaining` decreasing
  on the mock clock as usual, independent of the new token.
- **B48.** A replayed (spent) refresh token gives `400 invalid_grant` but does **not** revoke
  the tokens issued since (**Assumption**): the newer access token stays valid and the newer
  refresh token still works. (Unlike B42: code reuse revokes, refresh reuse does not.)
- **B49.** Refresh tokens never expire: refreshing after the access token expired, or after 1,
  30 or 90 days of mock time, works, and the new access token has the full lifetime
  (`remaining` equals the lifetime at issuance).
- **B50.** `refresh_token` is required (missing or empty gives `400 invalid_request`, nothing
  consumed). An unknown value (garbage, one changed character, trailing space, 30 KiB long, an
  authorization code) gives `400 invalid_grant` and affects no real token. `redirect_uri`,
  `scope` and unknown parameters are optional and ignored (**Assumption**: the OpenAPI text
  says `redirect_uri` is required for refresh, but `rsc` and most clients do not send it).
- **B51.** A refresh token belongs to its client: another authenticated client presenting it
  gets `400 invalid_grant` and does not consume it. A request that fails client authentication
  (`401`) does not consume it either. Basic authentication works for refresh as for any grant.
  A refreshed token keeps its `client_id` and stays a user token.
- **B52.** Refresh is atomic: 32 simultaneous requests with the same refresh token produce
  exactly one `200`; all others are `400 invalid_grant` (repeated for 5 rounds on the successive
  tokens), and the winner's new tokens are valid and unaffected by the losers (B48). Refreshes
  of different tokens in parallel all succeed.

### `client_credentials` grant

- **B53.** `client_credentials` needs **HTTP Basic** authentication with valid credentials.
  Valid credentials sent in the body instead, or missing, wrong or unknown ones give `401
  invalid_client` with a `WWW-Authenticate` header starting with `Basic` (**Assumption**: the
  guide only documents Basic for this grant).
- **B54.** The response is `200` with headers as in B33 and exactly the keys `access_token`,
  `expires_in`, `scope` (`""`) and `token_type` (`"bearer"`): **no `refresh_token` key at
  all** (not `null`). Its token is an app token: `token_info` gives `TokenKind::App`, the
  client's `client_id` and `remaining` equal to the lifetime, and it expires on schedule on the
  mock clock. Extra parameters (`scope`, `code`, `redirect_uri`) are ignored, every call
  returns a new token.
- **B55.** Credentials are typed: an access token (user or app) or a code sent as
  `refresh_token`, and a refresh token or an access token sent as `code`, give `400
  invalid_grant`, and consume nothing.

### Token validity for the other slices

- **B56.** `MockServer::token_info(access_token)` returns `Some(TokenInfo)` iff the token is
  an access token issued by this server that is not revoked and not expired at the current mock
  time, else `None` (unknown strings, the empty string, near misses such as a truncated,
  padded, `OAuth `-prefixed token, refresh tokens, codes). `kind` is `User` for tokens from the
  authorization_code and refresh_token grants and `App` for client_credentials tokens;
  `client_id` is the client they were issued to; `remaining` is `expiry - now` exactly on the
  mock clock (sub-second precision). A token is valid while `now < issue time + lifetime`: with
  1 s left it is valid, at exactly the lifetime it is `None`, and it stays `None`.

### End to end, PKCE, independence

- **B57.** A full login the way `rsc` performs it works: the authorize URL built like
  `TokenClient::authorize_url` (query parameters `client_id`, `redirect_uri`, `response_type`,
  `code_challenge`, `code_challenge_method`, `state`; base64url state), a browser following it
  without following the redirect, the callback parsed with a form-urlencoded query
  (state equal, no `error`, non-empty `code`), the exchange with the form of
  `TokenClient::exchange_code`, then, an hour of mock time later, a refresh with the form of
  `TokenClient::refresh`. A refused login (`Deny`) ends with `error=access_denied` plus state and
  no tokens. Logging in again does not invalidate an earlier session.
- **B58.** `pkce::s256_challenge` reproduces the RFC 7636 appendix B vector
  (`dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` gives
  `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM`) and always returns 43 unpadded base64url
  characters. `is_valid_verifier` accepts 43 to 128 unreserved characters and rejects 42, 129,
  5000, any other character (space, `+`, `/`, `=`, `%`, `:`, newline, accented letters, emoji)
  and multi-byte strings even when their byte length would fit. `is_valid_challenge` accepts
  exactly 43 characters of `[A-Za-z0-9_-]`. `verify_s256` is true for a valid verifier with its
  own challenge, false when either differs, when the verifier is malformed even if the hash
  matches, and for empty inputs.
- **B59.** Empty and huge values at the token endpoint are handled: with valid credentials,
  empty `code`/`redirect_uri`/`code_verifier` or `refresh_token` give `400 invalid_request`;
  20 KiB values for `code`, `redirect_uri`, `code_verifier` or `refresh_token` give
  `400 invalid_grant` and consume nothing; unknown extra form fields are ignored. Bodies with
  invalid percent-encoding, invalid UTF-8, stray `&`/`=`, an empty body or NUL bytes are client
  errors (`4xx`), never a `5xx` or a crash, and the server keeps serving. This holds for the
  percent-encoded forms (`%ff`, `%00`, `%zz`, a lone `%`) and for the **raw** bytes (0xFF, 0xFE,
  0x80, NUL, a broken UTF-8 sequence such as `C3 28`) sent as they are. With valid credentials,
  such bytes in an unknown `refresh_token`, in a `junk` field or in an unknown `grant_type` give
  the usual `400 invalid_grant` / `400 invalid_request` / `400 unsupported_grant`; in the
  credentials they give `401 invalid_client`. NUL, high bytes or malformed syntax in the request
  line, the URL or a header line (of either endpoint) are answered by the HTTP layer with a
  `4xx` or by closing the connection, never a `5xx`.
- **B60.** Sessions are independent: refreshing, replaying or revoking one session (or one
  login) does not affect another. Two authorizations with the same verifier and state give
  different codes and, once redeemed, different tokens.
- **B61.** Invalid UTF-8 in a decoded query or form parameter is replaced lossily: every
  maximal invalid byte sequence becomes one U+FFFD (this is what `String::from_utf8_lossy` does).
  Observable on `/authorize` in the echoed `state` (`state=a%FFb` gives `a` U+FFFD `b`, `%C3%28`
  gives U+FFFD followed by `(`, `%FE%FF` gives two U+FFFD; `%C3%A9`, valid, stays `é`) and at the
  token endpoint in the credentials: a client whose secret is `s` U+FFFD `x` is authenticated by
  a body `client_secret` of `s%FFx` or of the raw bytes `s`, 0xFF, `x` (and by `s%FEx`, `s%C3x`,
  and the encoded U+FFFD), not by `s%C3%28x`, `s%FF%FFx`, `sx` or the literal `s%25FFx`.

## Assumptions

These fill gaps in what is publicly documented. Each must be checked against the live API once
credentials exist (`specs/soundcloud-api-notes.md`).

1. Error `message` values: `invalid_client`, `invalid_grant`, `unsupported_grant` come from the
   OpenAPI examples; `invalid_request`, `unsupported_response_type`, `access_denied`
   (redirect `error=`, RFC 6749) and the mock's own `not_found`, `method_not_allowed`,
   `payload_too_large`, `unsupported_media_type` are not documented.
2. The live `/authorize` shows a login page and reports some errors as HTML; the mock answers
   `400` JSON for untrusted client/redirect URIs (per the task) and RFC redirects otherwise.
3. Old access tokens stay valid after a refresh (B47). Refresh-token reuse does not revoke
   the newer tokens (B48). Refresh tokens never expire (B49).
4. A failed code redemption (wrong verifier or redirect URI) does not consume the code
   (B38, B39), unlike a successful one. Chosen so that a stray request cannot burn a legitimate
   login; OAuth 2.1 allows both.
5. `client_credentials` needs Basic authentication, and the other grants also accept
   credentials in the body (B27, B53). `redirect_uri` is optional on refresh (B50).
6. `expires_in` is 3599 and `scope` is `""` (from the OpenAPI example `ExpiringToken`).
7. Missing required token parameters give `400 invalid_request`.

## Implementation notes (not behaviors)

- Keep all state (clients, codes, tokens, families, clock) in one structure behind a single
  lock: check-and-consume of a code or refresh token must be one critical section (B44, B52).
- Compare secrets and digests in constant time.
- Never log secrets, codes or tokens.
- A token family is the set of tokens descended from one authorization code; revocation (B42)
  marks the family. Redeemed codes are remembered for the lifetime of the server so that a replay
  after expiry is still recognized.
- Expiry is checked lazily against the mock clock; no background task or real timers.

## Test wiring

What DEV must add so that the tests compile (SPEC cannot edit application files):

- Workspace `Cargo.toml`: `members = ["rsc", "rsc-mock"]`.
- New crate `rsc-mock/` (`Cargo.toml` with `name = "rsc-mock"`, edition 2024; `src/lib.rs`
  re-exporting the foundation API at the root and declaring `pub mod pkce;`). Do not add a
  binary in this slice.
- In `rsc-mock/src/pkce.rs`: `#[cfg(test)] mod tests;` (file `rsc-mock/src/pkce/tests.rs`).
- Integration tests are in `rsc-mock/tests/*.rs` with the shared helper `rsc-mock/tests/common/mod.rs`;
  they need no declaration in the application code.
- Dev-dependencies of `rsc-mock` (add as normal dependencies instead if the library already
  uses them):
  - `reqwest = { version = "0.12", default-features = false }`
  - `tokio = { version = "1", features = ["macros", "rt-multi-thread", "net", "sync"] }`
  - `serde_json = "1"`
  - `url = "2"`
  - `sha2 = "0.10"`
  - `base64 = "0.22"`

The tests use `reqwest` with redirects disabled and proxies disabled, `tokio::test` (current
thread and multi-thread), `tokio::sync::Barrier`, `tokio::task::JoinSet`, and
`tokio::task::spawn_blocking` with `std::net::TcpStream` for the raw-bytes tests
(`rsc-mock/tests/raw_http.rs`; no extra tokio feature needed). Mock time only moves through
`MockServer::advance_clock`; the only sleeps are two 150 ms `std::thread::sleep` in the B5 tests
that prove real time does not move the clock (multi-thread runtime, so a background ticker
would still run).

## Out of scope

- The API endpoints (`/me`, playlists, tracks), stream endpoints, fault injection, control
  endpoints, CLI flags, the binary, real-time clock mode, OpenAPI validation: later slices
  (`specs/rsc-mock.md`).
- A login page or any UI on `/authorize`; consent screens; sessions or cookies.
- Scopes (always `""`), `invalid_scope` and `unauthorized_client` errors, rate limits of the
  token endpoint (50 tokens per 12 h per app, 30 per hour per IP).
- Token revocation endpoints, introspection, OpenID Connect, dynamic client registration.
- Fixing `rsc::auth::client::TokenClient` (it reads RFC-style `{"error": ...}` bodies): a
  separate feature.

## Open questions

- Should the token endpoint also accept `application/json` bodies (some SoundCloud clients
  send them)? The mock refuses them for now (B24); revisit with real credentials.
- Non-ASCII characters in a *registered* redirect URI (the `Location` header would carry raw
  bytes) are neither validated by `spawn` nor tested; decide together with the config rules.

## Change log

- 2026-09-20: initial spec and tests (foundation, authorize, token, refresh, client credentials,
  PKCE, token_info).
- 2026-09-20 (round 2, after TEST): fixed the B58 test that built a 132-character verifier (over
  the 128 limit; now one copy of the 66-character alphabet, each character on its own, and a
  128-character cycle) and the clippy findings in the tests (`err_expect`, `single_match`).
  Strengthened tests and spec: B2 (loopback only: 127.0.0.2 refuses), B5 (real 150 ms pause),
  B6 (all methods incl. HEAD/OPTIONS/PROPFIND, more paths), B7 (HEAD, exact `Allow`, method
  checked first), B21 (60 KiB URLs processed normally), B22 (405, HEAD, more 400s), B24 (415
  before credentials, duplicates, grant, announced length), B25 (exactly 64 KiB read, 64 KiB + 1
  refused, announced `Content-Length` refused without waiting for the body, chunked bodies,
  nothing consumed, oversized before duplicates and credentials), B26 (duplicates with bad
  credentials or malformed `Authorization` give 400 not 401), B29 (raw high bytes in
  `Authorization`), B34 (entropy, monotonicity and alphabet estimates over 150 samples per
  kind), B59 (raw NUL and invalid UTF-8 over TCP, in bodies, URL and headers). New behavior B61
  (lossy decoding, observable). New test file `rsc-mock/tests/raw_http.rs`; new helpers in
  `rsc-mock/tests/common/mod.rs`. New expectations that the implementation may have to meet:
  B25 announced length and chunked limit, B21 60 KiB URLs, B7 `Allow` exactness and HEAD, B22
  405 no-store.
- 2026-09-20 (round 3, after TEST): B25 relaxed for absurd announced lengths: the strict 413
  covers 65 537 up to and including 2^40; above that (u64::MAX is answered by hyper itself with
  a bare 431) any 4xx or a closed connection, never a 2xx or 5xx, never a wait. B25 chunked
  clarified: over-limit chunked bodies up to 100 000 bytes must get their 413 (no silent close)
  and consume nothing; larger ones may be closed but never get a 2xx. Tests: the announced-length
  test now asserts the answer arrives within 4 seconds and is split in two (strict range, lenient
  range with more values); the chunked test is split in three (near the limit must answer 413,
  far over may close, a refused chunked body consumes nothing); clippy `type_complexity` fixed
  in `oauth_token.rs` with type aliases. No change needed in the implementation if it already
  passed the round-3 tests other than the u64::MAX case.
