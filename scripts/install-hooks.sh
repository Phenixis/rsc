#!/usr/bin/env bash
# Enables the versioned git hooks in .githooks/ for this clone (undo: `git config --unset core.hooksPath`).
set -euo pipefail
cd "$(dirname "$0")/.."
git config core.hooksPath .githooks
echo "Git hooks enabled: .githooks/pre-push will run scripts/check.sh before each push."
