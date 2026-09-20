---
feature: <slug>
type: <feat|fix|docs|test|refactor|chore|ci|build|perf>
title: "<type>: <imperative summary, at most 72 characters>"
confirmed: false
max_changed_lines: 600
paths:
  - <glob of files or folders this pull request may touch, e.g. rsc-mock/**>
---

# Scope: <title>

The contract for ONE pull request. `confirmed` is set by `scripts/workflow.sh confirm-scope`
after the user agrees to it: agents cannot change it. Anything outside `paths` or above
`max_changed_lines` is scope drift: split it into another pull request (or amend this contract
with the user's agreement).

## Goal
Why this pull request exists and what changes for the user or the code. A few lines.

## In scope
- What this pull request does.

## Out of scope
- What it deliberately does not do, even though it is nearby. Each item is a candidate follow-up.

## Acceptance
- Observable conditions that mean "done" (behaviors, commands, tests).

## Follow-ups
- Work deferred to later pull requests, in the order it should happen.

## Decisions
- The questions settled in the interview and the answers, one line each.
