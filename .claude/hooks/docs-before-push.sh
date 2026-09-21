#!/usr/bin/env bash
# PreToolUse hook (Bash): before a `git push`, make sure the documentation is up to date.
#
# The rules and the logic are in scripts/check-docs.sh, shared with the git pre-push hook. This hook
# only decides whether the command is a push and turns a refusal into exit 2: the message on
# stderr is fed back to Claude and the push does not run.
set -uo pipefail

command_line="$(jq -r '.tool_input.command // ""')"
case "$command_line" in
  *"git push"*) ;;
  *) exit 0 ;;
esac
case "$command_line" in
  *" --delete"* | *" -d "*) exit 0 ;;
esac

script="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../scripts" && pwd)/check-docs.sh"
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

"$script" && exit 0
exit 2
