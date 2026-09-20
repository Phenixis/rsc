#!/usr/bin/env bash
# Self-test of scripts/workflow.sh in a throwaway repository with a real (local) remote:
# `init` needs a valid, user-confirmed scope summary, creates the branch from origin/main,
# and `scope-check` / `pr` refuse changes that leave the declared scope.
set -uo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# A repository with one commit, and a bare clone of it playing the part of origin.
git init -q -b main "$tmp/seed"
git -C "$tmp/seed" config user.email t@t
git -C "$tmp/seed" config user.name t
mkdir "$tmp/seed/scripts" && cp "$here/scripts/workflow.sh" "$tmp/seed/scripts/"
echo base >"$tmp/seed/README.md"
printf '.workflow/\n' >"$tmp/seed/.gitignore"
git -C "$tmp/seed" add -A
git -C "$tmp/seed" commit -q -m init
git clone -q --bare "$tmp/seed" "$tmp/origin.git"
git clone -q "$tmp/origin.git" "$tmp/repo" 2>/dev/null
cd "$tmp/repo"
git config user.email t@t
git config user.name t

total=0
failed=0
LAST=""
check() { # want(0|1) label command...
  local want="$1" label="$2" got out
  shift 2
  out="$("$@" 2>&1)"
  got=$?
  total=$((total + 1))
  if [[ "$got" != "$want" ]]; then
    failed=$((failed + 1))
    printf 'FAIL  %s (wanted exit %s, got %s)\n%s\n' "$label" "$want" "$got" "$out"
  fi
  LAST="$out"
}
expect_text() { # label text
  total=$((total + 1))
  if ! grep -q -- "$2" <<<"$LAST"; then
    failed=$((failed + 1))
    printf 'FAIL  %s: output lacks "%s"\n%s\n' "$1" "$2" "$LAST"
  fi
}
fail_if() { # label condition-result
  total=$((total + 1))
  if [[ "$2" != ok ]]; then failed=$((failed + 1)); echo "FAIL  $1"; fi
}

draft() { # slug type [paths] [max] [title]
  local slug="$1" type="$2" paths="${3:-src/**}" max="${4:-50}" title="${5:-$2: add the $1 thing}" p
  mkdir -p .workflow/drafts
  {
    printf -- '---\nfeature: %s\ntype: %s\ntitle: "%s"\nconfirmed: false\nmax_changed_lines: %s\npaths:\n' "$slug" "$type" "$title" "$max"
    for p in $paths; do printf '  - %s\n' "$p"; done
    printf -- '---\n# Scope\n## Goal\nDo the thing.\n## In scope\n- the thing\n## Out of scope\n- other things\n## Acceptance\n- it works\n## Follow-ups\nNone.\n'
  } >".workflow/drafts/$slug.md"
}

# ---------------------------------------------------------------- init refuses
check 1 "init without any summary" scripts/workflow.sh init thing feat --user-confirmed
expect_text "names the missing summary" "no scope summary"
draft thing feat
check 1 "init without the user's agreement" scripts/workflow.sh init thing feat
expect_text "asks for agreement" "user must agree"
check 1 "init with a wrong type" scripts/workflow.sh init thing fix --user-confirmed
expect_text "type mismatch" "says type"
check 1 "init with a bad slug" scripts/workflow.sh init Bad_Slug feat --user-confirmed
draft thing feat 'src/**' 50 'add the thing'
check 1 "init with a title that is not 'type: summary'" scripts/workflow.sh init thing feat --user-confirmed
expect_text "title rule" "title"
draft thing feat '<glob>'
check 1 "init with placeholder paths" scripts/workflow.sh init thing feat --user-confirmed
expect_text "paths rule" "paths"
draft thing feat
sed -i '/^- other things/d' .workflow/drafts/thing.md
check 1 "init with an empty section" scripts/workflow.sh init thing feat --user-confirmed
expect_text "section rule" "Out of scope"
[[ "$(git branch --show-current)" == main && ! -e .workflow/current ]] && r=ok || r=bad
fail_if "a refused init must leave no branch and no active feature behind" "$r"

# ---------------------------------------------------------------- init works
draft thing feat
check 0 "init with a valid summary" scripts/workflow.sh init thing feat --user-confirmed
[[ "$(git branch --show-current)" == feat/thing ]] && r=ok || r=bad
fail_if "init must switch to feat/thing" "$r"
[[ "$(cat .workflow/current)" == thing && "$(cat .workflow/thing/status)" == spec ]] && r=ok || r=bad
fail_if "init must activate the feature with status 'spec'" "$r"
{ grep -q '^confirmed: true' .workflow/thing/scope.md && [[ ! -e .workflow/drafts/thing.md ]]; } && r=ok || r=bad
fail_if "init must confirm the scope and consume the draft" "$r"
[[ -z "$(git config "branch.feat/thing.merge" || true)" ]] && r=ok || r=bad
fail_if "the branch must not track origin/main" "$r"
check 0 "status" scripts/workflow.sh status
expect_text "status shows the scope" "confirmed"

# ---------------------------------------------------------------- scope-check
mkdir -p src other
seq 1 10 >src/a.txt
check 0 "a change inside the paths" scripts/workflow.sh scope-check
expect_text "within scope" "Within scope"
echo x >other/b.txt
check 1 "a file outside the paths" scripts/workflow.sh scope-check
expect_text "names the file" "other/b.txt"
rm other/b.txt
seq 1 100 >src/big.txt
check 1 "a change over the size budget" scripts/workflow.sh scope-check
expect_text "budget" "Over the size budget"
rm src/big.txt
echo lock >Cargo.lock
mkdir -p specs
echo spec >specs/thing.md
check 0 "Cargo.lock and the feature's own spec are always allowed" scripts/workflow.sh scope-check
echo x >.workflow/thing/notes.txt
check 0 ".workflow is never counted" scripts/workflow.sh scope-check

# ---------------------------------------------------------------- pr refusals (before anything is pushed)
check 1 "pr with uncommitted changes" scripts/workflow.sh pr
expect_text "asks to commit" "uncommitted"
git add -A
git commit -q -m "feat: the thing"
echo x >other/c.txt
git add -A
git commit -q -m "feat: stray"
check 1 "pr with scope drift" scripts/workflow.sh pr
expect_text "drift" "scope drift"
[[ -z "$(git ls-remote --heads origin feat/thing)" ]] && r=ok || r=bad
fail_if "a refused pr must not push the branch" "$r"
git switch -q main
check 1 "pr from main" scripts/workflow.sh pr
expect_text "from a branch" "branch"

if ((failed > 0)); then
  echo "workflow: $failed of $total checks FAILED"
  exit 1
fi
echo "workflow: all $total checks passed"
