# What we know about the SoundCloud API

Reference for `rsc-mock`, written in our own words from public sources. It is **not** the
source of truth: the mock is checked against the official OpenAPI description
(`scripts/fetch-openapi.sh` downloads it to `.cache/soundcloud-openapi/api.yaml`, pinned to a
commit and checksum). We do not copy that file into this repository: the `soundcloud/api`
repository has no license.

Sources: the OpenAPI description at `openapi/api.yaml`, `Agents.md` and
`.cursor/skills/*/SKILL.md` in <https://github.com/soundcloud/api>, and the developer guide.
Nothing below was observed against the live API (we have no credentials). Items marked
**(unverified)** are our best reading and must be confirmed once real credentials exist.

## Hosts and headers

- API: `https://api.soundcloud.com`. Authorization server: `https://secure.soundcloud.com`
  (`/authorize`, `/oauth/token`). The old `api.soundcloud.com/oauth2/token` is deprecated.
- API requests carry `Authorization: OAuth <access_token>` (not `Bearer`) and
  `Accept: application/json; charset=utf-8`.
- JSON responses use `application/json; charset=utf-8` (some error responses are documented
  as plain `application/json`).

## OAuth 2.1

- **Authorization code + PKCE** for user data (`/me`, ...): `GET /authorize` with
  `response_type=code`, `client_id`, `redirect_uri`, `code_challenge`,
  `code_challenge_method=S256`, `state`. The redirect URI must be registered and match exactly.
  The code is exchanged at `POST /oauth/token` (form-encoded) with `grant_type=authorization_code`,
  `client_id`, `client_secret`, `redirect_uri`, `code_verifier`, `code`. The client secret is
  required even with PKCE.
- **Refresh**: `grant_type=refresh_token` + `refresh_token`, `client_id`, `client_secret`.
  Refresh tokens are **single-use**; each response carries the next one.
- **Client credentials** (public resources only): `grant_type=client_credentials` with
  `Authorization: Basic base64(client_id:client_secret)`. Limits: 50 tokens per 12 h per app,
  30 per hour per IP. `/me` needs a user token, not this one.
- Token response: `{"access_token", "refresh_token", "expires_in": 3599, "scope": "",
  "token_type": "bearer"}`. Access tokens last about an hour.
- Error bodies use SoundCloud's own shape, **not** RFC 6749's `{"error": ...}`:
  `{"code": <http status>, "message": "<oauth error code>", "link": "..."}`. Documented examples:
  `invalid_client` (401), `invalid_grant` (400), `invalid_scope` (401), `unauthorized_client`
  (401), `unsupported_grant` (400). **(unverified)** that the live token endpoint uses these.

## Resources

- Path parameters are **URNs**: `soundcloud:tracks:308946187`, `soundcloud:users:948745750`,
  `soundcloud:playlists:12345`. Numeric ids are deprecated. Objects carry `urn`, `kind`
  (`track`, `playlist`, `user`), `uri` (API URL) and `permalink_url` (website URL).
- `GET /me` returns the authenticated user (`urn`, `username`, `full_name`, `permalink_url`,
  `avatar_url`, counts, `plan`, ...). 401 without a valid user token.
- `GET /me/playlists?show_tracks=&linked_partitioning=&limit=` lists the user's playlists.
  `limit` 1..200, default 50. With `linked_partitioning=true` the body is
  `{"collection": [...], "next_href": "<url>"}`; without it a bare array (deprecated).
  `show_tracks` defaults to true.
- `GET /playlists/{urn}` (`secret_token`, `access`, `show_tracks`) and
  `GET /playlists/{urn}/tracks` (`secret_token`, `access`, `linked_partitioning`).
- **`access` filter**: `playable`, `preview`, `blocked`; comma separated; **the default is
  `playable,preview`, so blocked tracks are left out unless asked for**.
- `Track.access` is `playable` (full stream), `preview` (excerpt only) or `blocked` (no
  stream). Other track fields: `urn`, `title`, `duration` (ms), `streamable`, `permalink_url`,
  `uri`, `user`, `artwork_url`, `genre`, `created_at`, counts; `stream_url` is deprecated.
- `GET /tracks/{urn}`, and `GET /tracks?urns=a,b,c` (batch; **`urns`, not `ids`**), plus search
  filters (`q`, `genres`, `tags`, `access`, `limit`, `offset`, `linked_partitioning`).
- **Streaming**: `GET /tracks/{urn}/streams` (needs auth) returns
  `{"hls_aac_160_url", "hls_mp3_128_url", "preview_mp3_128_url"}`: HLS playlist URLs, so
  players must handle HLS. `GET /tracks/{urn}/preview` answers 302. The legacy `/stream` is
  not to be used. **(unverified)**: whether a blocked track returns 403 or 404 on `/streams`,
  and whether stream URLs need the `Authorization` header.
- `Playlist` fields: `urn`, `title`, `track_count`, `tracks` (Track objects), `tracks_uri`,
  `duration`, `sharing` (`public`/`private`), `permalink_url`, `user`, `created_at`, ...
  **(unverified)**: whether `tracks` is complete or truncated on large playlists.

## Errors, limits

- Error bodies: `{"code": 401, "message": "...", "link": "https://developers.soundcloud.com/docs/api/explorer/open-api"}`.
  Statuses documented: 400, 401, 403, 404, 422, 429. The 429 body adds `spam_warning_urn` and links
  to the rate-limit docs. **(unverified)**: `Retry-After` and rate-limit headers.
- Play-stream quota: 15 000 stream requests per 24 h per `client_id`; clients should back off on 429.

## Where the plan (`rsc-mvp-plan.md`, in French) was wrong or unsure

- Batch fetch is `GET /tracks?urns=`, not `?ids=`.
- Identifiers in paths are URNs.
- Streaming is HLS through `/streams`; `stream_url` is deprecated.
- Playlists from `/playlists/{urn}` hide blocked tracks by default (`access`).
- OAuth error bodies are `{code, message, link}`. This makes `TokenClient` (which reads
  `error`) wrong for the real API: tracked as a follow-up feature.
