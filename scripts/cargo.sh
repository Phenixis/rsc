#!/usr/bin/env bash
# `cargo` for shells whose PATH does not include rustup's bin directory (Claude Code
# subagents, git hooks). The agent guard treats `scripts/cargo.sh <args>` exactly like
# `cargo <args>`, so the same allow and deny rules apply.
export PATH="$HOME/.cargo/bin:$PATH"
exec cargo "$@"
