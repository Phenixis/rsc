#!/usr/bin/env bash
# Applies the repository settings that keep main clean, from files in this repository:
#   - .github/rulesets/main.json: main only changes through pull requests (no direct push, no
#     force push, no deletion, linear history, squash merge, CI and PR checks must pass, review
#     conversations resolved, nobody bypasses it, administrators included)
#   - merge settings: squash only, the pull request title and description become the commit on
#     main, and the branch is deleted after the merge
# Needs the GitHub CLI logged in as a repository administrator. Safe to run again.
set -euo pipefail
cd "$(dirname "$0")/.."

repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
name="$(jq -r .name .github/rulesets/main.json)"

echo "Merge settings for $repo: squash only, branch deleted after merge"
gh api -X PATCH "repos/$repo" \
  -F allow_squash_merge=true \
  -F allow_merge_commit=false \
  -F allow_rebase_merge=false \
  -F delete_branch_on_merge=true \
  -f squash_merge_commit_title=PR_TITLE \
  -f squash_merge_commit_message=PR_BODY >/dev/null

id="$(gh api "repos/$repo/rulesets" --jq ".[] | select(.name == \"$name\") | .id")"
if [[ -n "$id" ]]; then
  echo "Updating ruleset '$name' (#$id)"
  gh api -X PUT "repos/$repo/rulesets/$id" --input .github/rulesets/main.json >/dev/null
else
  echo "Creating ruleset '$name'"
  gh api -X POST "repos/$repo/rulesets" --input .github/rulesets/main.json >/dev/null
fi

echo "Active rules on main:"
gh api "repos/$repo/rules/branches/main" --jq '.[] | "  " + .type'
