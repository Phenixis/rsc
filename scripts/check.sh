#!/usr/bin/env bash
# Everything CI checks, in one place: formatting, lints and the whole test suite.
# Used by the pre-push hook (.githooks/pre-push), the GitHub Action and `./dev check`,
# so the three cannot drift apart.
set -euo pipefail
cd "$(dirname "$0")/.."

# Git hooks run with a minimal PATH that may not include rustup's bin directory.
export PATH="$HOME/.cargo/bin:$PATH"

# Without this, tests that need mpv skip silently when it is missing: a green run that
# tested nothing. Here a missing mpv must fail.
export RSC_REQUIRE_E2E=1

command -v mpv >/dev/null || { echo "check: mpv is required to run the end-to-end tests" >&2; exit 1; }

# The mock's contract tests compare its answers with SoundCloud's official OpenAPI description.
# Offline, they fail with a clear message rather than pass silently (RSC_REQUIRE_E2E=1).
scripts/fetch-openapi.sh || echo "check: could not download the SoundCloud OpenAPI description" >&2

scripts/test-agent-guard.sh
scripts/test-pr-guard.sh
scripts/test-workflow.sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
