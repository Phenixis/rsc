#!/usr/bin/env bash
# Enables the versioned git hooks in .githooks/ for this clone (undo: `git config --unset core.hooksPath`).
set -euo pipefail
cd "$(dirname "$0")/.."
git config core.hooksPath .githooks
echo "Git hooks enabled: no commit on main, no push to main, and scripts/check.sh before each push."
