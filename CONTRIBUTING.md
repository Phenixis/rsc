# Contributing

Everyone taking part in the project follows the [code of conduct](CODE_OF_CONDUCT.md). To report
a vulnerability, follow the [security policy](SECURITY.md) and do not open a public issue.

`main` only changes through pull requests. Nobody pushes to it directly, administrators
included: a repository ruleset enforces it, and local hooks stop you earlier. Each pull request
is squash-merged, so `main` reads as one commit per pull request, in the order they were merged,
each with an explicit title.

## The flow

1. **Settle the scope first.** One pull request does one thing and stays reviewable in one sitting
   (a few hundred changed lines, tests included). Write down the goal, what is in, what is
   explicitly out, and how you will know it is done. With Claude Code, run `/grill-scope`: it
   interviews you until the perimeter is clear and writes the scope contract. Without it, fill in
   the same sections by hand in the pull request description.
2. **Branch from an up-to-date `main`**, named `<type>/<slug>` (lowercase, digits, hyphens):
   `feat/mock-api`, `fix/token-error-format`, `docs/contributing`. Types: `feat`, `fix`,
   `refactor`, `docs`, `test`, `chore`, `ci`, `build`, `perf`.
   `scripts/workflow.sh init <slug> <type> --user-confirmed` does it from a confirmed scope.
3. **Work in the scope.** Anything you discover on the way that does not belong to it becomes a
   follow-up, not a bigger pull request. `scripts/workflow.sh scope-check` compares your changes
   with the scope contract (files and size).
4. **Open the pull request** (`scripts/workflow.sh pr` builds it from the scope contract). Fill in
   the template: goal, in scope, out of scope, follow-ups, test plan.
   The title is the future commit message: `type: imperative summary`, at most 72 characters
   (`feat: add the mock API endpoints`).
5. **Get it green and merged.** Required: the `check` job (formatting, lints, all tests) and the
   `pull-request` job (title, branch and description). Review conversations must be resolved.
   The merge is a squash merge and deletes the branch.

Keep your branch up to date by rebasing on `main`, not by merging `main` into it. Use
`git push --force-with-lease` on your own branch when you do.

## Finding your way around

[`docs/architecture.md`](docs/architecture.md) explains the crates, the modules and the login flow.

## Setting up your clone

```bash
scripts/install-hooks.sh   # blocks commits on main and pushes to it, and runs scripts/check.sh before a push
```

`scripts/check.sh` is what CI runs; run it before you push. `git commit --no-verify` and
`git push --no-verify` exist, but the point of the hooks is that you do not need them.

## Working with Claude Code

The repository ships agents and hooks (see [`specs/README.md`](specs/README.md)):

- `/grill-scope` interviews you about the scope of the change and records it.
- Behavior changes go through the SPEC / DEV / TEST agents: one writes the specification and the
  tests, one implements without seeing the tests, one runs everything and reports.
- A hook (`.claude/hooks/pr-guard.sh`) stops any session, agent or not, from committing on `main`,
  pushing to it, skipping git hooks or merging with `--admin`.

## For maintainers

The rules on `main` are described in [`.github/rulesets/main.json`](.github/rulesets/main.json) and
applied with `scripts/apply-repo-settings.sh` (needs the GitHub CLI, logged in as an administrator).
Changing them is itself a pull request. While the project has a single maintainer, a pull request
needs no approval; raise `required_approving_review_count` in the ruleset when reviewers join.
