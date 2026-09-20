#!/usr/bin/env bash
# PreToolUse hook (Bash): before a `git push`, make sure README.md has been kept up to date.
#
# Allows the push when any of these holds:
#   - the command is not a push (or is a branch deletion);
#   - there is nothing to push;
#   - README.md is modified by the commits about to be pushed;
#   - README.md was explicitly reviewed at the current HEAD (see the ack file below).
# Otherwise it exits 2: the message on stderr is fed back to Claude, the push does not run.
set -uo pipefail

command_line="$(jq -r '.tool_input.command // ""')"
case "$command_line" in
  *"git push"*) ;;
  *) exit 0 ;;
esac
case "$command_line" in
  *" --delete"* | *" -d "*) exit 0 ;;
esac

repo="${CLAUDE_PROJECT_DIR:-.}"
git() { command git -C "$repo" "$@"; }

head_sha="$(git rev-parse --verify -q HEAD)" || exit 0

# What the remote already has: the upstream, else origin/<branch>, else origin/main.
branch="$(git rev-parse --abbrev-ref HEAD)"
base=""
for ref in "@{upstream}" "origin/$branch" "origin/main"; do
  if base="$(git rev-parse --verify -q "$ref" 2>/dev/null)"; then break; fi
  base=""
done

if [ -n "$base" ]; then
  [ "$base" = "$head_sha" ] && exit 0
  changed="$(git diff --name-only "$base" HEAD)"
else
  changed="$(git ls-tree -r --name-only HEAD)" # nothing on the remote yet: everything is new
fi

if printf '%s\n' "$changed" | grep -qx 'README.md'; then
  exit 0
fi

ack_file="$(git rev-parse --git-path rsc-readme-reviewed)"
case "$ack_file" in /*) ;; *) ack_file="$repo/$ack_file" ;; esac
if [ -f "$ack_file" ] && [ "$(cat "$ack_file")" = "$head_sha" ]; then
  exit 0
fi

cat >&2 <<EOF
Push blocked: README.md is not part of the commits being pushed and has not been
reviewed at the current HEAD (${head_sha:0:7}).

Before pushing:
  1. Re-read README.md against the current code (features, commands, flags, requirements,
     dev workflow, roadmap checkboxes) and against the changes about to be pushed.
  2. If something is outdated or missing, update README.md and commit it.
  3. If it is already accurate, record that with:
       git rev-parse HEAD > "$ack_file"
  4. Run the push again (on its own, not chained after a commit in the same command).
EOF
exit 2
