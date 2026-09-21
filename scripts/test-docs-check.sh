#!/usr/bin/env bash
# Self-test of the documentation check before a push: scripts/check-docs.sh (with the real rules in
# scripts/docs-map.txt), scripts/docs-reviewed.sh, the Claude Code hook that calls them, and the
# wiring of the git hook. Every scenario runs in a throw-away repository whose origin/main is the
# starting point of a branch, the way a pull request branch relates to main.
set -uo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
check="$here/scripts/check-docs.sh"
reviewed="$here/scripts/docs-reviewed.sh"
hook="$here/.claude/hooks/docs-before-push.sh"
real_map="$here/scripts/docs-map.txt"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
repo="$tmp/repo"
git init -q -b main "$repo"
g() { git -C "$repo" -c user.email=t@t -c user.name=t "$@"; }
edit() { mkdir -p "$(dirname "$repo/$1")" && echo "change $RANDOM" >>"$repo/$1"; }
commit() { g add -A && g commit -q -m "$1"; }
ack_file() { g rev-parse --path-format=absolute --git-path rsc-docs-reviewed; }

for f in README.md ROADMAP.md SECURITY.md CONTRIBUTING.md docs/architecture.md \
  rsc/src/lib.rs rsc/src/auth/store.rs rsc/src/player/mpv.rs rsc-mock/src/lib.rs \
  scripts/tool.sh .githooks/pre-push .github/workflows/ci.yml .claude/settings.json Cargo.toml; do
  edit "$f"
done
commit "base"
g branch base
g update-ref refs/remotes/origin/main base

total=0
failed=0
out=""
got=0
cur=""
map="$real_map"

fail() {
  failed=$((failed + 1))
  printf 'FAIL  %s\n' "$*"
}
# A fresh branch from a given commit (default: origin/main), with no review marker.
scenario() {
  cur="$1"
  g checkout -q -f -B "s$total" "${2:-base}" && g clean -qfd
  rm -f "$(ack_file)"
}
run_check() { out="$(cd "$repo" && DOCS_MAP="$map" "$check" 2>&1)"; got=$?; }
verdict() { # want
  total=$((total + 1))
  run_check
  if [[ "$got" != "$1" ]]; then
    fail "$cur (wanted exit $1, got $got)$(printf '\n        %s' "${out//$'\n'/$'\n        '}")"
  fi
}
allow() { verdict 0; }
block() { verdict 1; }
says() { # the output of the last check mentions...
  total=$((total + 1))
  grep -qF -- "$1" <<<"$out" || fail "$cur: the message should mention '$1'"
}
lacks() { # ...and does not mention
  total=$((total + 1))
  ! grep -qF -- "$1" <<<"$out" || fail "$cur: the message should not mention '$1'"
}
expect() { # label, condition command...
  local label="$1"
  shift
  total=$((total + 1))
  "$@" || fail "$label"
}

# ------------------------------------------------------------------ nothing to check
scenario "a branch with no commits of its own"
allow

# ------------------------------------------------------------------ README: any change
scenario "README.md alone"
edit README.md && commit x
allow

scenario "only docs/architecture.md: README.md is missing"
edit docs/architecture.md && commit x
block
says "README.md"
says "scripts/docs-reviewed.sh"

# ------------------------------------------------------------------ ROADMAP: non-Markdown files
scenario "a code change without ROADMAP.md"
edit rsc/src/lib.rs && edit README.md && commit x
block
says "ROADMAP.md"
lacks "docs/architecture.md" # a modification does not change the structure

scenario "a code change with README.md and ROADMAP.md"
edit rsc/src/lib.rs && edit README.md && edit ROADMAP.md && commit x
allow

scenario "Markdown only never asks for ROADMAP.md"
edit SECURITY.md && edit README.md && commit x
allow

# ------------------------------------------------------------------ architecture: structure
base_docs() { edit README.md && edit ROADMAP.md; }

scenario "a file added under rsc/src"
base_docs && edit rsc/src/new.rs && commit x
block
says "docs/architecture.md"

scenario "a file added under rsc/src, architecture updated"
base_docs && edit rsc/src/new.rs && edit docs/architecture.md && commit x
allow

scenario "a file deleted under rsc/src"
base_docs && g rm -q rsc/src/lib.rs && commit x
block
says "docs/architecture.md"

scenario "a file renamed under rsc/src"
base_docs && g mv rsc/src/lib.rs rsc/src/lib2.rs && commit x
block
says "docs/architecture.md"

scenario "a file added under rsc-mock/src"
base_docs && edit rsc-mock/src/new.rs && commit x
block
says "docs/architecture.md"

scenario "a change to the workspace Cargo.toml"
base_docs && edit Cargo.toml && commit x
block
says "docs/architecture.md"

scenario "a file modified under rsc-mock/src leaves the architecture alone"
base_docs && edit rsc-mock/src/lib.rs && commit x
allow

# ------------------------------------------------------------------ SECURITY
scenario "a change under rsc/src/auth"
base_docs && edit rsc/src/auth/store.rs && commit x
block
says "SECURITY.md"

scenario "a change to rsc/src/player/mpv.rs"
base_docs && edit rsc/src/player/mpv.rs && commit x
block
says "SECURITY.md"

scenario "a change under rsc/src/auth, SECURITY.md reviewed in the diff"
base_docs && edit rsc/src/auth/store.rs && edit SECURITY.md && commit x
allow

# ------------------------------------------------------------------ CONTRIBUTING
for f in scripts/tool.sh .githooks/pre-push .github/workflows/ci.yml .claude/settings.json; do
  scenario "a change to $f"
  base_docs && edit "$f" && commit x
  block
  says "CONTRIBUTING.md"
done

scenario "a change to scripts/, CONTRIBUTING.md updated"
base_docs && edit scripts/tool.sh && edit CONTRIBUTING.md && commit x
allow

# ------------------------------------------------------------------ the whole branch counts
scenario "documents updated in an earlier commit of the branch"
base_docs && commit docs
edit rsc/src/new.rs && edit docs/architecture.md && commit code
allow

scenario "a fix-up commit does not ask for the documents again"
base_docs && edit rsc/src/lib.rs && commit first
edit rsc/src/lib.rs && commit fixup
allow

scenario "a branch rebased on a main that moved on"
# main gets unrelated changes after the branch was created; the branch is rebased on them.
g checkout -q -f -B moved base
edit rsc/src/other.rs && edit rsc/src/auth/store.rs && commit "unrelated work on main"
g update-ref refs/remotes/origin/main moved
g checkout -q -f -B s-rebased moved
edit README.md && commit "the branch's own change"
allow
lacks "SECURITY.md"
g update-ref refs/remotes/origin/main base

# ------------------------------------------------------------------ review markers
scenario "a review marker for every triggered document"
edit rsc/src/lib.rs && commit x
block
(cd "$repo" && "$reviewed" README.md ROADMAP.md >/dev/null 2>&1)
allow

scenario "a review marker is per document"
edit rsc/src/new.rs && commit x
(cd "$repo" && "$reviewed" README.md ROADMAP.md >/dev/null 2>&1)
block
says "docs/architecture.md"
lacks "README.md"
(cd "$repo" && "$reviewed" docs/architecture.md >/dev/null 2>&1)
allow

scenario "a review marker goes stale with the next commit"
edit rsc/src/lib.rs && commit x
(cd "$repo" && "$reviewed" README.md ROADMAP.md >/dev/null 2>&1)
allow
old_sha="$(g rev-parse HEAD)"
edit rsc/src/lib.rs && commit y
block
says "README.md"
(cd "$repo" && "$reviewed" README.md >/dev/null 2>&1)
expect "the stale lines are dropped from the marker file" \
  bash -c '! grep -q "$1" "$2"' _ "$old_sha" "$(ack_file)"
expect "the marker file holds the sha and the document" \
  bash -c 'grep -qF "$1 README.md" "$2"' _ "$(g rev-parse HEAD)" "$(ack_file)"

# ------------------------------------------------------------------ scripts/docs-reviewed.sh
scenario "docs-reviewed.sh"
total=$((total + 1))
(cd "$repo" && "$reviewed" >/dev/null 2>&1) && fail "docs-reviewed.sh with no argument should fail"
total=$((total + 1))
(cd "$repo" && "$reviewed" nope.md >/dev/null 2>&1) && fail "docs-reviewed.sh with a file that does not exist should fail"
total=$((total + 1))
(cd "$repo" && "$reviewed" README.md >/dev/null 2>&1) || fail "docs-reviewed.sh README.md should succeed"

# ------------------------------------------------------------------ no origin/main
g update-ref -d refs/remotes/origin/main
scenario "no origin/main: everything is new, and the documents are part of it"
edit rsc/src/new.rs
allow
scenario "no origin/main: a missing document is still missing"
g rm -q SECURITY.md && commit x
block
says "SECURITY.md"
g update-ref refs/remotes/origin/main base

# ------------------------------------------------------------------ the rules file
scenario "the rules file"
edit README.md && commit x

map="$tmp/absent.txt"
cur="a missing rules file"
block
says "absent.txt"

map="$tmp/comments.txt"
printf '# a comment\n\nalways => README.md\n' >"$map"
cur="a rules file with comments and blank lines"
allow

map="$tmp/nonsense.txt"
printf 'always => README.md\nthis is not a rule\n' >"$map"
cur="a line that is not a rule"
block
says "line 2"
says "this is not a rule"

map="$tmp/unknown.txt"
printf 'weird:foo => README.md\n' >"$map"
cur="an unknown kind of rule"
block
says "line 1"

map="$tmp/nodoc.txt"
printf 'changed:rsc/src/* =>\n' >"$map"
cur="a rule without a document"
block
says "line 1"

map="$tmp/nomatch.txt"
printf 'changed:scripts/* => CONTRIBUTING.md\n' >"$map"
cur="a rule that does not trigger"
allow
map="$real_map"

# ------------------------------------------------------------------ the Claude Code hook
hook_run() { # command
  out="$(CLAUDE_PROJECT_DIR="$repo" "$hook" <<<"$(jq -nc --arg c "$1" '{tool_name:"Bash",tool_input:{command:$c}}')" 2>&1)"
  got=$?
}
hook_expect() { # want label command
  total=$((total + 1))
  hook_run "$3"
  [[ "$got" == "$1" ]] || fail "hook: $2: '$3' (wanted exit $1, got $got) $out"
}

scenario "hook on a branch with a missing document"
edit docs/architecture.md && commit x
hook_expect 0 "not a push" 'git status'
hook_expect 0 "not a push (commit)" 'git commit -m x'
hook_expect 0 "deleting a remote branch" 'git push origin --delete old'
hook_expect 2 "a push" 'git push origin HEAD'
hook_expect 2 "a bare push" 'git push'
says "README.md"
says "scripts/docs-reviewed.sh"
edit README.md && commit y
hook_expect 0 "a push with the documents up to date" 'git push origin HEAD'

# ------------------------------------------------------------------ wiring
expect "readme-before-push.sh is gone" test ! -e "$here/.claude/hooks/readme-before-push.sh"
expect "settings.json points at docs-before-push.sh" grep -q 'docs-before-push.sh' "$here/.claude/settings.json"
expect "settings.json no longer points at readme-before-push.sh" \
  bash -c '! grep -q readme-before-push.sh "$1"' _ "$here/.claude/settings.json"
expect ".githooks/pre-push runs the check" grep -q 'scripts/check-docs.sh' "$here/.githooks/pre-push"
expect "scripts/check.sh runs this self-test" grep -q 'scripts/test-docs-check.sh' "$here/scripts/check.sh"
for s in scripts/check-docs.sh scripts/docs-reviewed.sh .claude/hooks/docs-before-push.sh; do
  expect "$s is executable" test -x "$here/$s"
  expect "$s is valid bash" bash -n "$here/$s"
done

if ((failed > 0)); then
  echo "docs-check: $failed of $total checks FAILED"
  exit 1
fi
echo "docs-check: all $total checks passed"
