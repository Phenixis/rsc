#!/usr/bin/env bash
# Self-test of the access rules that separate the SPEC, DEV and TEST agents
# (.claude/hooks/agent-guard.sh and spec-sync-check.sh). Every rule is exercised with
# a call that must be allowed and a call that must be blocked, including the obvious
# ways around it. Needs jq.
set -uo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
guard="$here/.claude/hooks/agent-guard.sh"
sync="$here/.claude/hooks/spec-sync-check.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
root="$tmp/repo"
mkdir -p "$root"/{rsc/src/foo,rsc/tests,specs,.workflow/x,target/debug,.git,.claude/hooks}
touch "$root"/rsc/src/foo.rs "$root"/rsc/src/foo/tests.rs "$root"/rsc/src/test_support.rs "$root"/rsc/tests/api.rs \
  "$root"/specs/x.md "$root"/.workflow/x/dev-notes.md "$root"/.workflow/x/spec-notes.md "$root"/target/debug/bin "$root"/.git/config
ln -s "$root/rsc/src/foo/tests.rs" "$root/rsc/src/innocent.rs"
ln -s "$root/rsc/src/foo.rs" "$root/specs/link.md"

set_state() { # feature, status  (empty feature = no active workflow)
  if [[ -n "$1" ]]; then echo "$1" >"$root/.workflow/current"; echo "$2" >"$root/.workflow/$1/status"; else rm -f "$root/.workflow/current"; fi
}

total=0
failed=0
expect() { # want(0|2)  role  label  json
  local want="$1" role="$2" label="$3" json="$4" got
  CLAUDE_PROJECT_DIR="$root" "$guard" "$role" <<<"$json" >/dev/null 2>&1
  got=$?
  total=$((total + 1))
  if [[ "$got" != "$want" ]]; then
    failed=$((failed + 1))
    printf 'FAIL  %-5s %s (wanted exit %s, got %s)\n' "$role" "$label" "$want" "$got"
  fi
}
allow() { expect 0 "$@"; }
block() { expect 2 "$@"; }

file_call() { jq -nc --arg t "$1" --arg p "$2" '{tool_name:$t,tool_input:{file_path:$p}}'; }
write_call() { jq -nc --arg p "$1" --arg c "${2:-x}" '{tool_name:"Write",tool_input:{file_path:$p,content:$c}}'; }
bash_call() { jq -nc --arg c "$1" '{tool_name:"Bash",tool_input:{command:$c}}'; }
tool_call() { jq -nc --arg t "$1" '{tool_name:$t,tool_input:{pattern:"x"}}'; }

# ---------------------------------------------------------------- SPEC
set_state x spec
allow spec "writes a spec"                    "$(write_call specs/x.md)"
allow spec "writes a unit test file"          "$(write_call rsc/src/foo/tests.rs)"
allow spec "writes an integration test"       "$(write_call rsc/tests/api.rs)"
allow spec "writes shared test helpers"       "$(write_call rsc/src/test_support.rs)"
allow spec "writes a fixture"                 "$(write_call rsc-mock/fixtures/data.json)"
allow spec "writes its answers file"          "$(write_call .workflow/x/answers.md)"
block spec "writes application code"          "$(write_call rsc/src/foo.rs)"
block spec "writes another crate's code"      "$(write_call rsc-mock/src/lib.rs)"
block spec "writes Cargo.toml"                "$(write_call Cargo.toml)"
block spec "writes outside the repository"    "$(write_call /etc/passwd)"
block spec "goes through .. into app code"    "$(write_call specs/../rsc/src/foo.rs)"
block spec "writes app code through a symlink" "$(write_call specs/link.md)"
allow spec "writes a test through a symlink"   "$(write_call rsc/src/innocent.rs)"
block spec "writes the workflow status"       "$(write_call .workflow/x/status red)"
block spec "writes the hooks"                 "$(write_call .claude/hooks/agent-guard.sh)"
block spec "edits app code"                   "$(file_call Edit rsc/src/foo.rs)"
block spec "runs a shell command"             "$(bash_call 'cargo test')"
allow spec "reads app code"                   "$(file_call Read rsc/src/foo.rs)"
allow spec "reads tests"                      "$(file_call Read rsc/src/foo/tests.rs)"
allow spec "greps"                            "$(tool_call Grep)"

# ---------------------------------------------------------------- DEV (feature x, status red)
set_state x red
allow dev "writes app code"                   "$(write_call rsc/src/foo.rs)"
allow dev "edits app code"                    "$(file_call Edit rsc/src/foo.rs)"
allow dev "writes Cargo.toml"                 "$(write_call rsc/Cargo.toml)"
allow dev "writes a new crate"                "$(write_call rsc-mock/src/lib.rs)"
allow dev "writes its questions file"         "$(write_call .workflow/x/questions.md)"
block dev "writes a unit test file"           "$(write_call rsc/src/foo/tests.rs)"
block dev "writes an integration test"        "$(write_call rsc/tests/api.rs)"
block dev "writes test helpers"               "$(write_call rsc/src/test_support.rs)"
block dev "writes a fixture"                  "$(write_call rsc-mock/fixtures/data.json)"
block dev "writes a new tests.rs"             "$(write_call rsc/src/bar/tests.rs)"
block dev "goes through .. into a test"       "$(write_call rsc/src/../src/foo/tests.rs)"
block dev "writes through a symlink"          "$(write_call rsc/src/innocent.rs)"
block dev "writes a spec"                     "$(write_call specs/x.md)"
block dev "writes the workflow status"        "$(write_call .workflow/x/status green)"
block dev "writes the report"                 "$(write_call .workflow/x/dev-notes.md)"
block dev "writes the hooks"                  "$(write_call .claude/hooks/agent-guard.sh)"
block dev "writes the CI"                     "$(write_call .github/workflows/ci.yml)"
block dev "writes outside the repository"     "$(write_call /tmp/evil.rs)"
allow dev "reads app code"                    "$(file_call Read rsc/src/foo.rs)"
allow dev "reads the spec"                    "$(file_call Read specs/x.md)"
allow dev "reads the report"                  "$(file_call Read .workflow/x/dev-notes.md)"
allow dev "reads a crate outside the repo"    "$(file_call Read /usr/include/stdio.h)"
block dev "reads a unit test file"            "$(file_call Read rsc/src/foo/tests.rs)"
block dev "reads an integration test"         "$(file_call Read rsc/tests/api.rs)"
block dev "reads test helpers"                "$(file_call Read rsc/src/test_support.rs)"
block dev "reads a test through .."           "$(file_call Read rsc/src/bar/../foo/tests.rs)"
block dev "reads a test through a symlink"    "$(file_call Read rsc/src/innocent.rs)"
block dev "reads build output"                "$(file_call Read target/debug/bin)"
block dev "reads git history"                 "$(file_call Read .git/config)"
block dev "reads the notes meant for SPEC"    "$(file_call Read .workflow/x/spec-notes.md)"
block dev "greps"                             "$(tool_call Grep)"
allow dev "globs (names only)"                "$(tool_call Glob)"
allow dev "cargo build"                       "$(bash_call 'cargo build')"
allow dev "cargo check for one package"       "$(bash_call 'cargo check -p rsc-mock')"
allow dev "cargo clippy"                      "$(bash_call 'cargo clippy')"
allow dev "cargo add a dependency"            "$(bash_call 'cargo add axum --features macros')"
allow dev "scripts/cargo.sh build"            "$(bash_call 'scripts/cargo.sh build')"
allow dev "scripts/cargo.sh check -p rsc-mock" "$(bash_call 'scripts/cargo.sh check -p rsc-mock')"
block dev "scripts/cargo.sh test"             "$(bash_call 'scripts/cargo.sh test')"
block dev "scripts/cargo.sh clippy --all-targets" "$(bash_call 'scripts/cargo.sh clippy --all-targets')"
block dev "scripts/cargo.sh fmt"              "$(bash_call 'scripts/cargo.sh fmt')"
block dev "another script"                    "$(bash_call 'scripts/evil.sh build')"
block dev "cargo test"                        "$(bash_call 'cargo test')"
block dev "cargo test --no-run"               "$(bash_call 'cargo test --no-run')"
block dev "cargo clippy --all-targets"        "$(bash_call 'cargo clippy --all-targets')"
block dev "cargo check --tests"               "$(bash_call 'cargo check --tests')"
block dev "cargo build --test x"              "$(bash_call 'cargo build --test api')"
block dev "cargo fmt (would rewrite tests)"   "$(bash_call 'cargo fmt')"
block dev "cat a test file"                   "$(bash_call 'cat rsc/src/foo/tests.rs')"
block dev "grep the sources"                  "$(bash_call 'grep -r spec rsc')"
block dev "git show old tests"                "$(bash_call 'git show HEAD:rsc/src/foo/tests.rs')"
block dev "chained with &&"                   "$(bash_call 'cargo build && cat rsc/src/foo/tests.rs')"
block dev "chained with ;"                    "$(bash_call 'cargo build; cat rsc/src/foo/tests.rs')"
block dev "piped"                             "$(bash_call 'cargo build | cat')"
block dev "command substitution"              "$(bash_call 'cargo build $(cat x)')"
block dev "backticks"                         "$(bash_call 'cargo build `cat x`')"
block dev "redirection"                       "$(bash_call 'cargo build > out.txt')"
block dev "multiline"                         "$(bash_call $'cargo build\ncat x')"

# DEV cannot start without failing tests
set_state x none
block dev "starts with status 'none'"         "$(write_call rsc/src/foo.rs)"
set_state x spec
block dev "starts with status 'spec'"         "$(write_call rsc/src/foo.rs)"
set_state x no-tests
block dev "starts with status 'no-tests'"     "$(write_call rsc/src/foo.rs)"
set_state x green
block dev "writes once everything is green"   "$(write_call rsc/src/foo.rs)"
allow dev "can still ask a question then"     "$(write_call .workflow/x/questions.md)"
set_state "" ""
block dev "starts with no active feature"     "$(write_call rsc/src/foo.rs)"

# ---------------------------------------------------------------- TEST
set_state x red
allow test "writes its report"                "$(write_call .workflow/x/dev-notes.md)"
allow test "writes notes for SPEC"            "$(write_call .workflow/x/spec-notes.md)"
allow test "sets the status to red"           "$(write_call .workflow/x/status red)"
allow test "sets the status to green"         "$(write_call .workflow/x/status green)"
allow test "sets the status to no-tests"      "$(write_call .workflow/x/status no-tests)"
block test "sets a made-up status"            "$(write_call .workflow/x/status banana)"
block test "writes app code"                  "$(write_call rsc/src/foo.rs)"
block test "writes a test"                    "$(write_call rsc/src/foo/tests.rs)"
block test "writes a spec"                    "$(write_call specs/x.md)"
block test "writes another feature's report"  "$(write_call .workflow/y/dev-notes.md)"
block test "edits anything"                   "$(file_call Edit .workflow/x/dev-notes.md)"
allow test "reads everything"                 "$(file_call Read rsc/src/foo/tests.rs)"
allow test "greps"                            "$(tool_call Grep)"
allow test "cargo test"                       "$(bash_call 'cargo test')"
allow test "cargo test with e2e required"     "$(bash_call 'RSC_REQUIRE_E2E=1 cargo test --no-fail-fast')"
allow test "scripts/cargo.sh test -p rsc-mock" "$(bash_call 'scripts/cargo.sh test -p rsc-mock')"
allow test "e2e-required scripts/cargo.sh"    "$(bash_call 'RSC_REQUIRE_E2E=1 scripts/cargo.sh test --no-fail-fast 2>&1 | tail -80')"
allow test "scripts/cargo.sh mutants"         "$(bash_call 'scripts/cargo.sh mutants --file rsc-mock/src/pkce.rs')"
block test "scripts/cargo.sh fmt"             "$(bash_call 'scripts/cargo.sh fmt')"
block test "scripts/cargo.sh mutants --in-place" "$(bash_call 'scripts/cargo.sh mutants --in-place')"
block test "another script"                   "$(bash_call 'scripts/evil.sh test')"
allow test "cargo test piped to tail"         "$(bash_call 'cargo test 2>&1 | tail -50')"
allow test "cargo test piped to grep"         "$(bash_call 'cargo test | grep FAILED')"
allow test "cargo clippy --all-targets"       "$(bash_call 'cargo clippy --all-targets -- -D warnings')"
allow test "cargo fmt --check"                "$(bash_call 'cargo fmt --check')"
allow test "cargo mutants"                    "$(bash_call 'cargo mutants --package rsc-mock')"
allow test "scripts/check.sh"                 "$(bash_call 'scripts/check.sh')"
allow test "git diff"                         "$(bash_call 'git diff --stat')"
allow test "git status"                       "$(bash_call 'git status --short')"
block test "cargo fmt (rewrites files)"       "$(bash_call 'cargo fmt')"
block test "cargo mutants --in-place"         "$(bash_call 'cargo mutants --in-place')"
block test "redirects output to a file"       "$(bash_call 'cargo test > results.txt')"
block test "pipes into a shell"               "$(bash_call 'cargo test | sh')"
block test "pipes into tee"                   "$(bash_call 'cargo test | tee out.txt')"
block test "rm"                               "$(bash_call 'rm -rf rsc')"
block test "git commit"                       "$(bash_call 'git commit -m x')"
block test "git checkout"                     "$(bash_call 'git checkout -- .')"
block test "chained"                          "$(bash_call 'cargo test && rm x')"
block test "sed -i"                           "$(bash_call 'sed -i s/a/b/ rsc/src/foo.rs')"
set_state "" ""
block test "writes a report with no active feature" "$(write_call .workflow/x/dev-notes.md)"

# ---------------------------------------------------------------- unknown role
CLAUDE_PROJECT_DIR="$root" "$guard" nonsense <<<"{}" >/dev/null 2>&1
got=$?
total=$((total + 1))
[[ $got == 2 ]] || { failed=$((failed + 1)); echo "FAIL  an unknown role must be refused"; }

# ---------------------------------------------------------------- SPEC stop check
sync_case() { # want label
  local want="$1" label="$2" got
  CLAUDE_PROJECT_DIR="$root" "$sync" <<<"${3:-{\}}" >/dev/null 2>&1
  got=$?
  total=$((total + 1))
  if [[ "$got" != "$want" ]]; then failed=$((failed + 1)); printf 'FAIL  spec-sync: %s (wanted exit %s, got %s)\n' "$label" "$want" "$got"; fi
}
write_spec() { # behaviors...
  {
    printf -- '---\nfeature: x\ntests:\n  - rsc/src/foo/tests.rs\n  - rsc/tests/api.rs\n---\n# x\n## Behaviors\n'
    for b in "$@"; do printf -- '- **%s.** something\n' "$b"; done
  } >"$root/specs/x.md"
}
write_tests() { # cited ids...
  : >"$root/rsc/src/foo/tests.rs"
  : >"$root/rsc/tests/api.rs"
  for b in "$@"; do printf '// spec: %s\n#[test] fn t_%s() {}\n' "$b" "$b" >>"$root/rsc/src/foo/tests.rs"; done
}
set_state x spec
rm -f "$root/rsc/src/innocent.rs" "$root/specs/link.md"
write_tests B1 B2; write_spec B1 B2;  touch -d '2 seconds ago' "$root/rsc/src/foo/tests.rs" "$root/rsc/tests/api.rs"
sync_case 0 "spec and tests agree"
sync_case 0 "a second stop after being blocked always passes" '{"stop_hook_active": true}'
write_spec B1 B2 B3;                  touch -d '1 second ago' "$root/specs/x.md"
sync_case 2 "a behavior has no test"
write_spec B1;                        touch -d '1 second ago' "$root/specs/x.md"
sync_case 2 "a test cites a behavior the spec lacks"
write_spec B1 B2;                     touch -d '10 seconds ago' "$root/specs/x.md"
sync_case 2 "a test file is newer than the spec"
touch "$root/specs/x.md"
sync_case 0 "the spec touched after the tests passes again"
rm "$root/specs/x.md"
sync_case 2 "there is no spec"
printf -- '---\nfeature: x\n---\n- **B1.** a\n' >"$root/specs/x.md"
sync_case 2 "the spec lists no test files"
write_spec B1 B2; rm "$root/rsc/tests/api.rs"
sync_case 2 "a listed test file is missing"
set_state "" ""
sync_case 0 "no active workflow: nothing to check"

if ((failed > 0)); then echo "agent guard: $failed of $total checks FAILED"; exit 1; fi
echo "agent guard: all $total checks passed"
