# Specifications and the SPEC / DEV / TEST pipeline

Features are built by three Claude Code subagents with **separated powers**, coordinated by
the main session (the *orchestrator*). The goal is to keep the tests honest: the agent that
writes the code cannot see or edit the tests, so it has to build what the specification says.

| Agent | Writes | Reads | Runs |
|---|---|---|---|
| `rsc-scout` | nothing | everything (Read, Grep, Glob) | nothing; used by `/grill-scope` to look facts up |
| `rsc-spec` | `specs/*.md`, test files (once the scope is confirmed) | everything | nothing |
| `rsc-dev` | application code, `questions.md` | everything except tests | `scripts/cargo.sh build/check/clippy/add/...` (a `cargo` with a working PATH) |
| `rsc-test` | its notes and the status | everything | `scripts/cargo.sh test/clippy/fmt --check/mutants`, read-only git |

Test files are: `tests.rs`, anything under `tests/`, `test_support.rs`, `fixtures/`,
`testdata/`. Unit tests live in `<module>/tests.rs` (declared with `#[cfg(test)] mod tests;`),
never inline in application files, so that the boundary is a path.

The rules are enforced by hooks declared in each agent (`.claude/hooks/agent-guard.sh`) and
tested by `scripts/test-agent-guard.sh`. They are guardrails against an agent drifting, not a
sandbox against an adversary.

## The loop (one feature = one branch = one pull request)

`main` only changes through pull requests (see `CONTRIBUTING.md`); each pull request is squash-merged.

0. **Scope**: the orchestrator runs `/grill-scope` (a skill, because it must converse with the
   human): it interviews the user round by round, looks facts up through the read-only `rsc-scout`
   sub-agent, proposes to split work that is too big, and writes a scope summary to
   `.workflow/drafts/<slug>.md`. After the user's explicit yes, the orchestrator runs
   `scripts/workflow.sh init <slug> <type> --user-confirmed`: the only way to begin. It validates
   the summary, creates the branch `<type>/<slug>` from `origin/main`, and confirms the scope
   (`.workflow/<slug>/scope.md`: goal, in/out of scope, acceptance, follow-ups, allowed `paths`,
   size budget). Agents cannot write or change it, and SPEC cannot write anything before it exists.
1. **SPEC** writes `specs/<feature>.md` and the tests, inside the scope. *A human reviews both.*
2. **TEST** checks that the tests exist and fail, and sets the status to `red`.
   DEV cannot write anything before that.
3. **DEV** implements from the spec. Questions go to `.workflow/<feature>/questions.md`.
4. **TEST** runs everything, checks `scripts/workflow.sh scope-check` (drift), and writes
   `.workflow/<feature>/dev-notes.md` for DEV and `spec-notes.md` for SPEC. Status `red` or `green`.
5. `red`: back to 3 (at most 5 rounds, then ask a human). Notes for SPEC lead to test and spec
   changes, which restart from 2.
6. `green`: the orchestrator runs `cargo fmt` and `scripts/check.sh`, commits on the branch in
   logical commits, and runs `scripts/workflow.sh pr`, which checks the scope, pushes the branch and
   opens the pull request from the scope contract. Anything discovered on the way and out of scope
   becomes a follow-up, not more changes.

Changes that do not alter behavior (docs, CI, chores) skip steps 1 to 5 but not step 0 or the pull request.

`.workflow/` (drafts, current feature, scope, status, notes) is local state and is not committed.
`specs/` is committed: it is the living documentation of each feature. The scope of each pull
request lives on in its description.

## Spec conventions

- Behaviors are numbered `- **B1.** ...`. Every test cites the behaviors it checks with a
  comment `// spec: B1` (or `B1, B2`). A stop hook on SPEC refuses to finish if a behavior has
  no test, a test cites an unknown behavior, or a listed test file is newer than the spec.
- Whenever SPEC changes a test it updates the spec (behaviors, change log) in the same run.
- Front matter lists the test files (`tests:`); see `_template.md`.
- `soundcloud-api-notes.md` records what we know about the real SoundCloud API, for the mock.
