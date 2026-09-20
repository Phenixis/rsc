# Roadmap

The order below is the order of priority. Each item is meant to be **one pull request** with its
own scope (see [CONTRIBUTING.md](CONTRIBUTING.md): start with `/grill-scope`, or write the scope
by hand). Bigger items are split into several pull requests, listed in order.

Nobody needs SoundCloud credentials to work on most of this: `rsc-mock` stands in for the API.
An official app needs an Artist Pro account, so contributors should not have to pay for one.

## Done

- Local backend, mpv player, queue, keyboard controls, signal handling, end-to-end tests
- Login building blocks: PKCE, loopback callback, token storage, code exchange, single-use refresh
  tokens, authenticated API client that retries once on a 401
- `rsc-mock`, slice 1: the OAuth server (61 specified behaviors, 175 tests)
- Pull-request-only workflow, scope contracts (`/grill-scope`), SPEC / DEV / TEST agents, CI

## Next: make the project easy to join

Do this before adding features: it is the cheapest way to get other people involved.

1. **README up to date** (`docs`): correct the roadmap, say what `rsc-mock` is and why it exists,
   explain how to develop without credentials, and make it clear that contributing does not
   require Claude Code.
2. **Community files** (`docs`): `SECURITY.md`, a code of conduct, issue templates (bug report,
   feature request), so GitHub's community checklist is complete.
3. **Starter issues** (`chore`): turn the items of this roadmap into GitHub issues, and label a few
   `good first issue` with a clear scope and acceptance criteria.
4. **Architecture overview** (`docs`): one short page on the crates and modules, the login flow, and
   how `rsc` talks to `rsc-mock`.
5. **English everywhere** (`docs`): translate `rsc-mvp-plan.md`, and the French comments in
   `Dockerfile`, `compose.yaml` and the `dev` script.

## Next: correctness against the real API

6. **`fix/token-error-format`**: `TokenClient` reads RFC-style `{"error": ...}` bodies, but SoundCloud
   answers `{"code", "message", "link"}` with the OAuth error in `message`. Fix it, testing against
   `rsc-mock`.
7. **Verify the open assumptions** with real credentials (Gate 0 of the plan), and record the answers.
   The list is in [`specs/soundcloud-api-notes.md`](specs/soundcloud-api-notes.md) (items marked
   "unverified") and in the Assumptions of [`specs/mock-oauth.md`](specs/mock-oauth.md).

## Next: finish `rsc-mock`

Each slice goes through the SPEC / DEV / TEST pipeline. Overview:
[`specs/rsc-mock.md`](specs/rsc-mock.md).

8. **`mock-api`**: `/me`, `/me/playlists`, `/playlists/{urn}`, `/tracks`, `/tracks/{urn}`,
   `/tracks/{urn}/streams`; URNs, pagination (`linked_partitioning`, `next_href`), the `access` filter
   (blocked tracks are hidden by default), SoundCloud-shaped errors, and the built-in dataset.
9. **`mock-media`**: HLS playlists and audio segments, so `rsc` can really play from the mock.
10. **`mock-faults`**: fault injection (401, 429, 503, slow answers, short token lifetime, truncated
    playlists), control endpoints, and the `rsc-mock` binary with its command-line options.
11. **`mock-contract`**: check every mock response against SoundCloud's official OpenAPI schemas
    (`scripts/fetch-openapi.sh` already downloads them, pinned to a commit).

## Next: the MVP itself

12. **`rsc login`**: reads `client_id` and `client_secret` from the config, starts the loopback
    listener, opens the browser (`--manual` for headless use), and stores the tokens. Tested end to
    end against `rsc-mock`.
13. **Playlists from the API**: list your playlists, read tracks by URN, filter by `access`, follow
    `next_href`, play the HLS streams, show the attribution SoundCloud requires.
14. **Full-screen terminal UI** with ratatui, replacing the command-line output.

## Tooling and maintenance

- **Mutation testing**: install `cargo-mutants`, run it on `rsc-mock` and `rsc`, and fix the tests it
  finds too weak. Then consider a scheduled CI job.
- **Reviews**: raise `required_approving_review_count` in `.github/rulesets/main.json` once there
  are reviewers other than the maintainer.
- **Releases**: a release workflow that calls the CI workflow first (`workflow_call` is ready),
  then builds a package (`cargo-deb`).
- **Confirm the protection of `main`**: check that a direct push is rejected
  (the ruleset is applied; only a pull request merge has been observed so far).

## Later (after the MVP)

Search, likes, feed, shuffle, a seek bar, MPRIS, caching where the API terms allow it, gapless
playback, and keyring storage for tokens. See the exclusions in the plan.
