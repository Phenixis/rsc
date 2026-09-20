#!/usr/bin/env bash
# PreToolUse hook (Bash) for EVERY Claude Code session and agent in this repository:
# main only changes through pull requests.
#
#   - no commit while on main (start a branch first)
#   - no push that targets main, no push without an explicit target while on main,
#     no --all / --mirror
#   - never skip the git hooks (--no-verify, commit -n)
#   - never merge a pull request with --admin (that bypasses the rules)
#
# Exit 2 blocks the call; the message on stderr goes back to the agent.
# Tested by scripts/test-pr-guard.sh.
set -uo pipefail
set -f

cmd="$(jq -r '.tool_input.command // ""')"
root="$(cd "${CLAUDE_PROJECT_DIR:-.}" && pwd -P)"
branch="$(git -C "$root" symbolic-ref --short -q HEAD 2>/dev/null || true)"
deny() { echo "pr-guard: $*" >&2; exit 2; }
on_protected() { [[ "$branch" == main || "$branch" == master ]]; }
targets_protected() { [[ "$1" =~ ^\+?(refs/heads/)?(main|master)$ || "$1" =~ ^[^:]*:(refs/heads/)?(main|master)$ ]]; }

# `git [-C dir] <sub> args...`: prints the args of the first match, if any
git_args() {
  [[ "$cmd" =~ (^|[^[:alnum:]_/.-])git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?$1([[:space:]][^\;\&\|]*)?($|[\;\&\|]) ]] || return 1
  printf '%s' "${BASH_REMATCH[3]}"
}

if args="$(git_args commit)"; then
  on_protected && deny "you are on '$branch': commit on a branch instead (scripts/workflow.sh start <slug> [type]). main only changes through pull requests."
  for a in $args; do
    [[ "$a" == --no-verify || "$a" == -n ]] && deny "do not skip the git hooks (--no-verify / -n): fix what they report."
  done
fi

if args="$(git_args push)"; then
  positional=0
  for a in $args; do
    case "$a" in
      --no-verify) deny "do not skip the git hooks (--no-verify): fix what they report." ;;
      --all | --mirror) deny "'$a' would push main too: push your branch by name." ;;
      -*) ;;
      *)
        positional=$((positional + 1))
        targets_protected "$a" && deny "'$a' targets the protected branch: push your branch and open a pull request."
        [[ "$a" == HEAD ]] && on_protected && deny "HEAD is '$branch' here: switch to a branch first."
        ;;
    esac
  done
  # `git push` / `git push origin` push the current branch: fine unless it is main.
  if ((positional < 2)) && on_protected; then
    deny "you are on '$branch': a push here would update it. Work on a branch and open a pull request."
  fi
fi

if [[ "$cmd" =~ (^|[^[:alnum:]_/.-])gh[[:space:]]+pr[[:space:]]+merge([[:space:]]|$) ]]; then
  for a in $cmd; do
    [[ "$a" == --admin ]] && deny "do not merge with --admin: it bypasses the rules that keep main clean."
  done
fi
exit 0
