# rsc

A minimal SoundCloud client for the terminal, written in Rust.

> **Status: early development.** Playback of local folders works today; logging in
> to a SoundCloud account is not implemented yet. See [Roadmap](#roadmap).
>
> rsc is an independent project and is **not affiliated with, endorsed by or
> sponsored by SoundCloud**.

## What it will do

- Log in to your SoundCloud account (OAuth 2.1, Authorization Code + PKCE)
- List your playlists, pick one, play its tracks in order
- Pause, next/previous track, quit; a "now playing" line with proper attribution

Out of scope for the MVP: search, likes, feed, shuffle, seek bar, MPRIS, caching, gapless.

## Requirements

- Linux
- [`mpv`](https://mpv.io) in your `PATH` (rsc drives it over its IPC socket)
- Rust (stable), installed with [rustup](https://rustup.rs)
- `ffmpeg`, only to generate test audio (optional)

## Try it

Today rsc plays audio from a local folder: every sub-folder of the library is a
playlist and every audio file in it (sorted by name) is a track.

```bash
# Generate a test playlist of 5 sine tones (~/Music/rsc-dev/test-playlist)
scripts/gen-dev-music.sh

cargo run -p rsc -- playlists
cargo run -p rsc -- play test-playlist
```

| Key | Action |
|---|---|
| `space` | pause / resume (the state is printed) |
| `n` or `→` | next track |
| `p` or `←` | previous track (restarts the first one) |
| `q`, `Esc`, `Ctrl-C` | quit |

Options (also available as environment variables):

| Flag | Variable | Meaning |
|---|---|---|
| `--library <DIR>` | `RSC_LOCAL_DIR` | library folder (default `~/Music/rsc-dev`) |
| `--backend local` | `RSC_BACKEND` | where playlists come from |
| `--mpv-arg <ARG>` | | extra argument for mpv, e.g. `--mpv-arg=--ao=null` for no sound |

## SoundCloud and its terms

rsc will use the official SoundCloud API, so it follows the
[API Terms of Use](https://developers.soundcloud.com/docs/api/terms-of-use):

- **Bring your own app.** There is no shared `client_secret`: each user registers
  their own SoundCloud app and provides their credentials in the config.
- **No downloads, no offline cache.** Audio is streamed only.
- **Attribution.** The uploader, SoundCloud as the source and a link to the track
  are always shown.
- Only tracks SoundCloud makes streamable outside its platform can be played;
  previews and blocked tracks are marked or skipped.

## Development

```bash
cargo test                # everything below
scripts/check.sh          # what CI runs: fmt + clippy (warnings are errors) + all tests
scripts/install-hooks.sh  # optional: run scripts/check.sh before every `git push`
```

The test suite has three layers:

- **Unit and component tests** (`src/`, `tests/session.rs`): pure logic, fake backend
  and fake player. No mpv needed.
- **Real mpv** (`tests/mpv.rs`): the IPC layer against a real mpv, without sound
  (`--ao=null`). Covers `end-file` reasons, pause, shutdown and orphan processes.
- **End to end** (`tests/audio_e2e.rs`, `tests/interactive.rs`, `tests/signals.rs`): the
  real `rsc` binary.
  - *Audio*: mpv writes what it plays to a file instead of a sound card, and the test
    listens for the tones in it (Goertzel analysis), so it checks that the right tracks
    play, once each, in order. It cannot tell what your sound card does with the samples.
  - *Terminal*: `rsc` runs in a pseudo-terminal; the test types keys, reads the screen
    and checks the terminal mode is restored on exit.
  - *Signals*: SIGINT, SIGTERM and SIGHUP leave no mpv process and no socket behind.

These tests need `mpv`. Without it they are skipped locally, but set `RSC_REQUIRE_E2E=1`
to make a missing mpv a failure instead (`scripts/check.sh` and CI do).

CI (`.github/workflows/ci.yml`) runs `scripts/check.sh` on every push to `main` and every
pull request, on Ubuntu with the distribution's mpv and no sound hardware. It can also be
called from a release workflow (`workflow_call`) so a release requires green checks.

A Docker environment (Rust, mpv, ffmpeg) is provided as an option if you would
rather not install a toolchain: `./dev` opens a shell, `./dev cargo test` runs
a command, `./dev check` runs `scripts/check.sh`. Real audio output and the
browser login are easier on the host.

## Roadmap

The detailed plan (in French) is in [`rsc-mvp-plan.md`](rsc-mvp-plan.md).

- [x] Local backend, mpv player, queue, keyboard controls
- [x] Login building blocks: PKCE, loopback callback, token storage, code exchange,
      single-use refresh tokens, authenticated API client that retries once on 401
- [ ] `rsc login` command: opens the browser, reads your app credentials from the config
- [ ] `rsc-mock`: a fake SoundCloud server to develop and test without API access
- [ ] Playlists from the SoundCloud API
- [ ] Full-screen terminal UI (ratatui)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
