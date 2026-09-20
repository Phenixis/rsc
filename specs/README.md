# Specifications and the SPEC / DEV / TEST pipeline

Features are built by three Claude Code subagents with **separated powers**, coordinated by
the main session (the *orchestrator*). The goal is to keep the tests honest: the agent that
writes the code cannot see or edit the tests, so it has to build what the specification says.

| Agent | Writes | Reads | Runs |
|---|---|---|---|
| `rsc-spec` | `specs/*.md`, test files | everything | nothing |
| `rsc-dev` | application code, `questions.md` | everything except tests | `scripts/cargo.sh build/check/clippy/add/...` (a `cargo` with a working PATH) |
| `rsc-test` | its report and the status | everything | `scripts/cargo.sh test/clippy/fmt --check/mutants`, read-only git |

Test files are: `tests.rs`, anything under `tests/`, `test_support.rs`, `fixtures/`,
`testdata/`. Unit tests live in `<module>/tests.rs` (declared with `#[cfg(test)] mod tests;`),
never inline in application files, so that the boundary is a path.

The rules are enforced by hooks declared in each agent (`.claude/hooks/agent-guard.sh`) and
tested by `scripts/test-agent-guard.sh`. They are guardrails against an agent drifting, not a
sandbox against an adversary.

## The loop

1. **Orchestrator**: `scripts/workflow.sh start <feature>` (slug: lowercase, digits, hyphens).
2. **SPEC** writes `specs/<feature>.md` and the tests. *A human reviews both.*
3. **TEST** checks that the tests exist and fail, and sets the status to `red`.
   DEV cannot write anything before that.
4. **DEV** implements from the spec. Questions go to `.workflow/<feature>/questions.md`.
5. **TEST** runs everything and writes `.workflow/<feature>/dev-notes.md` for DEV
   and `spec-notes.md` for SPEC. Status `red` or `green`.
6. `red`: back to 4 (at most 5 rounds, then ask a human). Notes for SPEC lead to test and spec
   changes, which restart from 3.
7. `green`: the orchestrator runs `cargo fmt`, `scripts/check.sh`, and commits.

`.workflow/` (current feature, status, reports) is local state and is not committed.
`specs/` is committed: it is the living documentation of each feature.

## Spec conventions

- Behaviors are numbered `- **B1.** ...`. Every test cites the behaviors it checks with a
  comment `// spec: B1` (or `B1, B2`). A stop hook on SPEC refuses to finish if a behavior has
  no test, a test cites an unknown behavior, or a listed test file is newer than the spec.
- Whenever SPEC changes a test it updates the spec (behaviors, change log) in the same run.
- Front matter lists the test files (`tests:`); see `_template.md`.
- `soundcloud-api-notes.md` records what we know about the real SoundCloud API, for the mock.
