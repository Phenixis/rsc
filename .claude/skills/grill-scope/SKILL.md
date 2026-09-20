---
name: grill-scope
description: Interview the user relentlessly until the perimeter and scope of ONE pull request are settled, then record it as a scope contract. Use at the start of every feature, fix, refactor, docs or chore change, before any branch, spec, test or code exists.
argument-hint: [what you want to change]
---

Adapted from the `grilling` skill of [mattpocock/skills](https://github.com/mattpocock/skills)
(MIT, see `LICENSE-grilling.md`), specialised to define the scope of a pull request.

**Why this is a skill and not a sub-agent**: a sub-agent cannot converse with the user (it has no
`AskUserQuestion` and returns a single message). The interview must run here, in the main
conversation. Sub-agents are used for what they are good at: looking facts up.

Nothing exists yet when this skill starts: no branch, no workspace, no spec. It ends by handing a
confirmed scope contract to `scripts/workflow.sh init`, the only way to begin work.

## 1. Look facts up first (never ask the user for what you can find)

Do this before the first question, in parallel where you can:

- Current state: `git status`, `git branch --show-current`, `scripts/workflow.sh status`, and
  `gh pr list --state open --json number,title,headRefName,files`: which pull requests are open and
  which files they touch (overlaps are a scope problem).
- Dispatch the `rsc-scout` sub-agent for what needs reading: the modules, specs
  (`specs/*.md`), roadmap items (`README.md`, `rsc-mvp-plan.md`) and tests the request touches, and how big
  the change would probably be. Do not wait for it to ask the questions that do not depend on it.

## 2. Interview in rounds

Interview the user until you reach a shared understanding. Map the request as a **design tree**:
every decision branches into the decisions that hang off it. Work it in **rounds**. The **frontier**
is every decision whose prerequisites are already settled: ask the whole frontier in one round,
numbered, each with your recommended answer, then wait for the user's answers.

```
❓ **Q1** - **<question title>**: <question body, may be several paragraphs, may offer choices>

➡️ <your recommended answer>

---

❓ **Q2** - ...
```

Answers reshape the tree: recompute the frontier and ask the next round. A question that depends
on another open question in the same round belongs to a later round. If the user asks for one question
at a time, do that.

**Facts are your job, decisions are the user's.** Put every decision to the user and wait. Never
answer your own questions and never settle a decision "for them".

Decisions to reach for, roughly in this order (skip what does not apply, add what you find):

1. **Goal**: what changes and for whom, and why now. What would make the user say "done"?
2. **Boundaries**: what is explicitly in; what is explicitly out even though it is nearby (each of
   those becomes a follow-up); what must not change (public API, formats, behavior).
3. **Size**: is this one pull request? A pull request should be reviewable in one sitting and do one
   thing. If the work has several independent concerns, or would exceed roughly 600 changed lines
   (tests included), **propose splitting it into an ordered series of pull requests** and grill
   the first one now. Each later one gets its own scope, later.
4. **Perimeter**: which files and folders may change (these become `paths:` globs in the contract);
   what is expected to be new, modified, deleted.
5. **Kind**: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `build` or `perf`, which
   sets the branch name and the title. Does it change behavior (then it goes through the
   SPEC/DEV/TEST pipeline of `specs/README.md`) or not (then it is done directly on the branch)?
6. **Dependencies**: does it need another open or planned pull request first? Does it conflict with
   an open one?
7. **Acceptance**: the observable conditions, commands and tests that prove it.
8. **Risks and unknowns**: what could make the scope grow; what is unverified (for `rsc-mock`, anything not
   confirmed against the real SoundCloud API).
9. **Impact**: README, specs, CI, docs to update in the same pull request.

Some questions cannot be settled by talking (how something should look or feel): say so, and propose
a small throwaway experiment instead of guessing.

## 3. Record the scope and get the user's agreement

The interview is over when the frontier is empty: every branch visited, nothing silently assumed.

1. Write the summary to `.workflow/drafts/<slug>.md` (create the folder) from
   `.claude/skills/grill-scope/scope-template.md`: fill every front matter field and every section
   (write `None.` where the answer is none). `title` is the pull request title
   (`type: imperative summary`, at most 72 characters). `paths` are globs. `Decisions` records each
   question and its answer in one line. Leave `confirmed: false`.
2. Show the user the summary and ask them to confirm (the `AskUserQuestion` tool is a good fit).
   Change it until they do. **Do not go on without an explicit yes.**
3. Only then run `scripts/workflow.sh init <slug> <type> --user-confirmed`. That flag states that
   the user explicitly agreed to this scope: never add it on your own judgment. The script validates
   the summary, creates the branch `<type>/<slug>` from `origin/main`, and confirms the scope.
4. Follow `specs/README.md` from the SPEC step (behavior changes) or work directly on the branch
   (docs, chores). Either way, stay inside the scope: `scripts/workflow.sh scope-check` compares the
   branch with it, and `scripts/workflow.sh pr` opens the pull request from it.

Do not act on anything the user agreed to until they confirm that the understanding is shared.
