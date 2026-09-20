#!/usr/bin/env bash
# Separation of powers between the rsc-spec, rsc-dev and rsc-test subagents.
#
# Wired as a PreToolUse hook in each agent's frontmatter: `agent-guard.sh spec|dev|test`.
# Exit 2 blocks the tool call; the message on stderr goes back to the agent.
#
#   spec  writes specs and tests only            (never application code)
#   dev   writes application code only           (never tests or specs, never reads tests,
#                                                 cannot start before the tests are red)
#   test  writes only its report and the status  (runs the tests, reads everything)
#
# This is a guardrail, not a sandbox: it stops an agent that follows its instructions from
# drifting, and it makes deliberate shortcuts fail loudly. Tested by scripts/test-agent-guard.sh.
set -uo pipefail
set -f # no globbing while we look at untrusted strings

role="${1:-}"
case "$role" in spec | dev | test) ;; *) echo "usage: agent-guard.sh spec|dev|test" >&2; exit 2 ;; esac

payload="$(cat)"
root="$(cd "${CLAUDE_PROJECT_DIR:-.}" && pwd -P)"
tool="$(jq -r '.tool_name // ""' <<<"$payload")"
field() { jq -r "$1 // \"\"" <<<"$payload"; }
deny() { echo "rsc-$role: $*" >&2; exit 2; }

# --- paths -------------------------------------------------------------------------------------

# Path relative to the repository root, with `..` and symlinks resolved; empty if outside it.
rel_path() {
  local p="$1"
  [[ "$p" == /* ]] || p="$root/$p"
  p="$(realpath -m -- "$p")"
  case "$p" in "$root"/*) printf '%s' "${p#"$root"/}" ;; *) printf '' ;; esac
}

# Files that belong to the tests: the SPEC agent owns them, the DEV agent never touches them.
is_test_path() {
  case "$1" in
    tests.rs | */tests.rs | tests/* | */tests/* | test_support.rs | */test_support.rs | \
      test_support/* | */test_support/* | fixtures/* | */fixtures/* | testdata/* | */testdata/*) return 0 ;;
  esac
  return 1
}

# --- workflow state ----------------------------------------------------------------------------

feature="$(tr -d '[:space:]' 2>/dev/null <"$root/.workflow/current" || true)"
has_feature() { [[ "$feature" =~ ^[a-z0-9][a-z0-9-]*$ ]]; }
feature_status() { tr -d '[:space:]' 2>/dev/null <"$root/.workflow/$feature/status" || echo none; }

# --- shell commands ----------------------------------------------------------------------------

# No chaining, redirection, substitution, expansion or escapes: one plain command.
plain_command() {
  local c="${1//2>&1/}"
  [[ "$c" != *$'\n'* && ! "$c" =~ [\;\&\`\<\>\(\)\{\}\\\$] ]]
}

ARGS='( [A-Za-z0-9_=:./@+,"'"'"' -]*)?'

# `scripts/cargo.sh` is `cargo` with a usable PATH: same rules, same commands.
as_cargo() { printf '%s' "${1//scripts\/cargo.sh/cargo}"; }

# --- the three roles ---------------------------------------------------------------------------

guard_spec() {
  case "$tool" in
    Read | Grep | Glob) exit 0 ;;
    Write | Edit | NotebookEdit)
      local r
      r="$(rel_path "$(field '.tool_input.file_path // .tool_input.notebook_path')")"
      [[ -n "$r" ]] || deny "refusing to write outside the repository"
      case "$r" in specs/*) exit 0 ;; esac
      is_test_path "$r" && exit 0
      has_feature && [[ "$r" == ".workflow/$feature/answers.md" ]] && exit 0
      deny "SPEC may only write specs/ and test files (tests.rs, tests/, test_support.rs, fixtures/), not '$r'. Application code is DEV's job."
      ;;
    Bash) deny "SPEC has no shell: it cannot compile or run anything. TEST does that." ;;
  esac
  exit 0
}

guard_dev() {
  case "$tool" in
    Glob) exit 0 ;; # lists names only
    Grep) deny "DEV has no Grep: it would return lines from the tests. Use Read on the files you need." ;;
    Read)
      local r
      r="$(rel_path "$(field '.tool_input.file_path')")"
      [[ -z "$r" ]] && exit 0 # outside the repo (crate sources under ~/.cargo, ...)
      is_test_path "$r" && deny "'$r' is a test file: DEV works from specs/$feature.md, not from the tests."
      case "$r" in
        .git | .git/* | target | target/*) deny "'$r' is off-limits for DEV (history and build output can contain the tests)." ;;
        .workflow/*/spec-notes.md) deny "spec-notes.md is addressed to SPEC, not to DEV." ;;
      esac
      exit 0
      ;;
    Write | Edit | NotebookEdit)
      local r
      r="$(rel_path "$(field '.tool_input.file_path // .tool_input.notebook_path')")"
      [[ -n "$r" ]] || deny "refusing to write outside the repository"
      is_test_path "$r" && deny "'$r' is a test file: DEV never modifies tests. If a test seems wrong, say so in .workflow/<feature>/questions.md."
      has_feature && [[ "$r" == ".workflow/$feature/questions.md" ]] && exit 0
      case "$r" in
        specs/* | .claude/* | .git/* | .github/* | .githooks/* | .workflow/* | .cache/* | target/* | scripts/*)
          deny "DEV writes application code only, not '$r'."
          ;;
      esac
      has_feature || deny "no feature is active (.workflow/current is empty): the orchestrator must start one before DEV can work."
      local st
      st="$(feature_status)"
      [[ "$st" == "red" ]] || deny "feature '$feature' has status '$st': DEV can only start once tests exist and fail (status 'red'). Ask the orchestrator to run SPEC, then TEST."
      exit 0
      ;;
    Bash)
      local c
      c="$(as_cargo "$(field '.tool_input.command')")"
      plain_command "$c" || deny "DEV runs one plain cargo command at a time (no ;, &&, |, redirections or substitutions)."
      [[ "$c" =~ ^cargo\ (build|check|clippy|add|remove|update|tree)$ARGS$ ]] ||
        deny "DEV may only run: cargo build | check | clippy | add | remove | update | tree. No cargo test, no git, no cat/grep."
      local w
      for w in $c; do
        case "$w" in
          --all-targets | --tests | --test | --test=* | --benches | --bench | --bench=* | --examples | --example | --example=*)
            deny "'$w' would compile the tests: DEV builds the application only."
            ;;
        esac
      done
      exit 0
      ;;
  esac
  exit 0
}

guard_test() {
  case "$tool" in
    Read | Grep | Glob) exit 0 ;;
    Edit | NotebookEdit) deny "TEST never edits files: it reports to DEV and SPEC." ;;
    Write)
      local r
      r="$(rel_path "$(field '.tool_input.file_path')")"
      has_feature || deny "no feature is active (.workflow/current is empty)."
      case "$r" in
        ".workflow/$feature/dev-notes.md" | ".workflow/$feature/spec-notes.md") exit 0 ;;
        ".workflow/$feature/status")
          case "$(field '.tool_input.content' | tr -d '[:space:]')" in
            red | green | no-tests) exit 0 ;;
          esac
          deny "the status must be exactly one of: red, green, no-tests."
          ;;
      esac
      deny "TEST may only write .workflow/$feature/dev-notes.md, spec-notes.md and status, not '${r:-outside the repository}'."
      ;;
    Bash)
      local c seg first=1
      c="$(as_cargo "$(field '.tool_input.command')")"
      # Pipes are allowed only into a few read-only filters.
      local IFS='|'
      for seg in $c; do
        seg="${seg#"${seg%%[![:space:]]*}"}"
        seg="${seg%"${seg##*[![:space:]]}"}"
        plain_command "$seg" || deny "TEST runs plain commands: no ;, &&, redirections (except 2>&1) or substitutions."
        seg="${seg//2>&1/}" # the one redirection we allow, once it has been checked
        seg="${seg%"${seg##*[![:space:]]}"}"
        if ((first)); then
          first=0
          [[ "$seg" =~ ^(RSC_REQUIRE_E2E=[01]\ )?(cargo\ (test|clippy|check|build|tree|mutants|fmt)|scripts/check\.sh|scripts/test-agent-guard\.sh|git\ (status|diff|log|show|ls-files|rev-parse|blame))$ARGS$ ]] ||
            deny "TEST may run cargo test|clippy|check|build|tree|mutants|fmt --check, scripts/check.sh, and read-only git."
          [[ "$seg" != *"cargo fmt"* || "$seg" == *"--check"* ]] || deny "TEST does not format files: use cargo fmt --check."
          [[ "$seg" != *"--in-place"* ]] || deny "cargo mutants --in-place would modify the sources."
        else
          [[ "$seg" =~ ^(head|tail|grep|wc|sort|uniq)$ARGS$ ]] || deny "TEST may pipe only into head, tail, grep, wc, sort or uniq."
        fi
      done
      exit 0
      ;;
  esac
  exit 0
}

"guard_$role"
