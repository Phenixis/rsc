#!/usr/bin/env bash
# Self-test of .claude/hooks/pr-guard.sh: on main and on a branch, commands that must be
# allowed and commands that must be blocked, including the usual ways around the rule.
set -uo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
guard="$here/.claude/hooks/pr-guard.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
repo="$tmp/repo"
git init -q -b main "$repo"
git -C "$repo" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init

total=0
failed=0
run() { # want(0|2) label command
  local want="$1" label="$2" cmd="$3" got
  CLAUDE_PROJECT_DIR="$repo" "$guard" <<<"$(jq -nc --arg c "$cmd" '{tool_name:"Bash",tool_input:{command:$c}}')" >/dev/null 2>&1
  got=$?
  total=$((total + 1))
  if [[ "$got" != "$want" ]]; then
    failed=$((failed + 1))
    printf 'FAIL  [%s] %s: %s (wanted exit %s, got %s)\n' "$(git -C "$repo" branch --show-current)" "$label" "$cmd" "$want" "$got"
  fi
}
allow() { run 0 "$@"; }
block() { run 2 "$@"; }

# ------------------------------------------------------------------ on main
allow "reads are fine"                 'git status'
allow "log"                            'git log --oneline'
allow "unrelated commands"             'cargo build'
allow "creating a branch"              'git switch -c feat/x'
allow "pushing a named branch"         'git push -u origin feat/x'
allow "pulling"                        'git pull --ff-only'
block "commit on main"                 'git commit -m x'
block "amend on main"                  'git commit --amend --no-edit'
block "commit on main via -C"          'git -C . commit -m x'
block "commit chained"                 'git add -A && git commit -m x'
block "bare push"                      'git push'
block "push to a remote only"          'git push origin'
block "push HEAD from main"            'git push origin HEAD'
block "push main"                      'git push origin main'
block "push master"                    'git push origin master'
block "force push main"                'git push --force origin main'
block "plus-force push main"           'git push origin +main'
block "full ref"                       'git push origin refs/heads/main'
block "HEAD:main"                      'git push origin HEAD:main'
block "branch onto main"               'git push origin feat/x:main'
block "delete main"                    'git push origin :main'
block "push --all"                     'git push --all'
block "push --mirror"                  'git push --mirror origin'
block "push chained after a cd"        'cd /tmp && git push origin main'
block "skipping hooks on push"         'git push --no-verify origin feat/x'

# ------------------------------------------------------------------ on a branch
git -C "$repo" switch -q -c feat/x
allow "commit on a branch"             'git commit -m "feat: add x"'
allow "commit with a heredoc"          'git commit -F - <<EOF'
allow "amend on a branch"              'git commit --amend --no-edit'
allow "bare push of the branch"        'git push'
allow "push -u origin HEAD"            'git push -u origin HEAD'
allow "push by name"                   'git push origin feat/x'
allow "force with lease on the branch" 'git push --force-with-lease origin HEAD'
allow "merging a PR normally"          'gh pr merge 5 --squash --delete-branch'
allow "viewing a PR"                   'gh pr view 5'
block "push main from a branch"        'git push origin main'
block "branch onto main"               'git push origin feat/x:main'
block "HEAD onto main"                 'git push origin HEAD:main'
block "delete main"                    'git push origin :main'
block "skipping hooks on commit"       'git commit --no-verify -m x'
block "skipping hooks with -n"         'git commit -n -m x'
block "skipping hooks on push"         'git push --no-verify'
block "push everything"                'git push --all origin'
block "admin merge"                    'gh pr merge 5 --admin --squash'
block "chained push to main"           'git fetch && git push origin main'

if ((failed > 0)); then echo "pr-guard: $failed of $total checks FAILED"; exit 1; fi
echo "pr-guard: all $total checks passed"
