#!/usr/bin/env bash
# Orchestrator helper: one feature = one branch = one scope contract = one pull request.
# main only changes through pull requests (see CONTRIBUTING.md, specs/README.md).
#
#   workflow.sh init <slug> <type> --user-confirmed
#                                     the ONLY way to begin work. Needs the scope summary written by
#                                     /grill-scope in .workflow/drafts/<slug>.md (validated), and the
#                                     user's explicit agreement. Then it creates the branch
#                                     <type>/<slug> from origin/main and the workspace, and confirms
#                                     the scope. Types: feat fix docs test refactor chore ci build perf
#   workflow.sh scope-check           compare the branch's changes with the scope: paths and size
#   workflow.sh pr [--allow-drift "reason"]
#                                     scope check, push the branch, open the pull request from the scope
#   workflow.sh status                where we are
#   workflow.sh stop                  deactivate the current feature
set -euo pipefail
cd "$(dirname "$0")/.."

TYPES='feat|fix|docs|test|refactor|chore|ci|build|perf'

die() { echo "workflow: $*" >&2; exit 1; }
current() { tr -d '[:space:]' 2>/dev/null <.workflow/current || true; }
need_feature() { f="$(current)"; [[ -n "$f" ]] || die "no active feature: scripts/workflow.sh start <slug> [type]"; scope=".workflow/$f/scope.md"; }
base_ref() { git rev-parse --verify -q origin/main >/dev/null && echo origin/main || echo main; }

# Front matter field of the scope, quotes removed.
fm() { awk -v k="$1" '/^---[[:space:]]*$/ { n++; next } n == 1 && index($0, k ":") == 1 { sub("^" k ":[[:space:]]*", ""); gsub(/^"|"$/, ""); print; exit }' "$scope"; }
# Items of the `paths:` list in the front matter.
fm_paths() { awk '/^---[[:space:]]*$/ { n++; next } n == 1 && /^paths:/ { on = 1; next } n == 1 && on && /^[[:space:]]*-[[:space:]]+/ { sub(/^[[:space:]]*-[[:space:]]+/, ""); print; next } n == 1 && on { on = 0 }' "$scope"; }
# Body of a `## <name>` section.
section() { awk -v h="## $1" '$0 == h { on = 1; next } /^## / { on = 0 } on' "$scope"; }

validate_scope() {
  [[ -f "$scope" ]] || die "$scope does not exist"
  [[ "$(fm feature)" == "$f" ]] || die "scope 'feature' must be '$f'"
  [[ "$(fm type)" =~ ^($TYPES)$ ]] || die "scope 'type' must be one of: ${TYPES//|/ }"
  local title; title="$(fm title)"
  [[ "$title" =~ ^($TYPES)(\([a-z0-9-]+\))?!?:\ .{6,}$ && ${#title} -le 72 ]] || die "scope 'title' must look like 'type: summary' and stay within 72 characters (got: $title)"
  [[ "$(fm max_changed_lines)" =~ ^[0-9]+$ ]] || die "scope 'max_changed_lines' must be a number"
  local paths; paths="$(fm_paths)"
  [[ -n "$paths" && "$paths" != *"<"* ]] || die "scope 'paths' must list the files this pull request may touch (no placeholders)"
  local s
  for s in "Goal" "In scope" "Out of scope" "Acceptance" "Follow-ups"; do
    [[ -n "$(section "$s" | tr -d '[:space:]')" ]] || die "scope section '## $s' is missing or empty (write 'None.' if that is the answer)"
  done
  ! grep -q '<[a-z].*>' <<<"$(section Goal)$(section 'In scope')" || die "scope still contains template placeholders"
}
scope_confirmed() { grep -qE '^confirmed:[[:space:]]*true[[:space:]]*$' "$scope"; }

# Prints the drift report; returns 1 when files fall outside the scope or the change is too big.
scope_check() {
  local base files=() outside=() file glob ok lines=0 added deleted budget
  base="$(git merge-base "$(base_ref)" HEAD)"
  mapfile -t files < <({ git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u | grep -v '^\.workflow/' || true)
  mapfile -t globs < <({ fm_paths; echo "specs/$f.md"; echo "Cargo.lock"; })
  set +f
  for file in "${files[@]}"; do
    ok=0
    for glob in "${globs[@]}"; do
      # shellcheck disable=SC2053
      [[ "$file" == $glob ]] && { ok=1; break; }
    done
    ((ok)) || outside+=("$file")
  done
  while read -r added deleted _; do
    [[ "$added" =~ ^[0-9]+$ ]] && lines=$((lines + added + deleted))
  done < <(git diff --numstat "$base")
  for file in $(git ls-files --others --exclude-standard | grep -v '^\.workflow/' || true); do lines=$((lines + $(wc -l <"$file"))); done
  budget="$(fm max_changed_lines)"
  echo "Files changed: ${#files[@]}   Lines changed: $lines (budget $budget)"
  local rc=0
  if ((${#outside[@]} > 0)); then
    echo "Outside the declared paths:"; printf '  %s\n' "${outside[@]}"; rc=1
  fi
  if ((lines > budget)); then echo "Over the size budget by $((lines - budget)) lines."; rc=1; fi
  ((rc == 0)) && echo "Within scope."
  return $rc
}

case "${1:-status}" in
  init)
    slug="${2:?usage: workflow.sh init <slug> <type> --user-confirmed}"; type="${3:?usage: workflow.sh init <slug> <type> --user-confirmed}"
    [[ "$slug" =~ ^[a-z0-9][a-z0-9-]*$ ]] || die "invalid slug '$slug' (lowercase letters, digits, hyphens)"
    [[ "$type" =~ ^($TYPES)$ ]] || die "invalid type '$type' (one of: ${TYPES//|/ })"
    [[ "${4:-}" == --user-confirmed ]] || die "the user must agree to the scope first (run /grill-scope), then add --user-confirmed"
    f="$slug"; draft=".workflow/drafts/$slug.md"; scope="$draft"
    [[ -f "$draft" ]] || die "no scope summary at $draft: run /grill-scope first, it writes the summary there"
    [[ "$(fm type)" == "$type" ]] || die "the summary says type '$(fm type)', not '$type'"
    validate_scope
    branch="$type/$slug"
    git fetch -q origin main 2>/dev/null || echo "workflow: could not fetch origin/main; using the local one" >&2
    if [[ "$(git branch --show-current)" != "$branch" ]]; then
      [[ -z "$(git status --porcelain)" ]] || die "the working tree has uncommitted changes: commit or stash them first"
      ! git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1 || die "branch $branch already exists on origin: pick another slug, or resume it with git switch"
      if git rev-parse --verify -q "refs/heads/$branch" >/dev/null; then git switch -q "$branch"; else git switch -q --no-track -c "$branch" "$(base_ref)"; fi
    fi
    # Overlap with other open pull requests: a warning, since the interview should have covered it.
    if command -v gh >/dev/null && others="$(gh pr list --state open --json number,title,files --jq '.[] | "#\(.number) \(.title)\t\([.files[].path] | join(" "))"' 2>/dev/null)" && [[ -n "$others" ]]; then
      set +f; mapfile -t globs < <(fm_paths)
      while IFS=$'\t' read -r label filelist; do
        for file in $filelist; do for glob in "${globs[@]}"; do
          # shellcheck disable=SC2053
          if [[ "$file" == $glob ]]; then echo "workflow: WARNING open pull request $label also touches $file" >&2; break 2; fi
        done; done
      done <<<"$others"
    fi
    mkdir -p ".workflow/$slug"
    mv "$draft" ".workflow/$slug/scope.md"
    scope=".workflow/$slug/scope.md"
    sed -i 's/^confirmed:.*/confirmed: true/' "$scope"
    echo "$slug" >.workflow/current
    echo spec >".workflow/$slug/status"
    echo "Branch $branch ready. Scope confirmed: $(fm title)"
    echo "Next: the SPEC agent writes specs/$slug.md and the tests (see specs/README.md)."
    ;;
  scope-check)
    need_feature
    validate_scope
    scope_check
    ;;
  pr)
    need_feature
    allow_drift=""
    if [[ "${2:-}" == --allow-drift ]]; then allow_drift="${3:?usage: pr --allow-drift \"reason\"}"; fi
    branch="$(git branch --show-current)"
    [[ "$branch" != main && "$branch" != master ]] || die "you are on '$branch': pull requests come from a branch"
    validate_scope; scope_confirmed || die "the scope is not confirmed: run /grill-scope, then scripts/workflow.sh init"
    [[ -z "$(git status --porcelain)" ]] || die "uncommitted changes: commit them first (the pull request is made of commits)"
    report="$(mktemp)"; trap 'rm -f "$report"' EXIT
    if ! scope_check >"$report"; then
      [[ -n "$allow_drift" ]] || { cat "$report" >&2; die "scope drift: split it into another pull request, or amend the scope with the user's agreement (or --allow-drift \"reason\")"; }
      printf '\nScope drift accepted: %s\n' "$allow_drift" >>"$report"
    fi
    body=".workflow/$f/pr-body.md"
    {
      for s in "Goal" "In scope" "Out of scope" "Follow-ups"; do printf '## %s\n%s\n\n' "$s" "$(section "$s")"; done
      printf '## Test plan\n%s\n\n' "$(section Acceptance)"
      printf '## Scope check\n```\n%s\n```\n' "$(cat "$report")"
    } >"$body"
    git push -u origin HEAD
    if url="$(gh pr view "$branch" --json url --jq .url 2>/dev/null)"; then
      echo "A pull request already exists: $url"
    else
      gh pr create --base main --head "$branch" --title "$(fm title)" --body-file "$body"
    fi
    ;;
  status)
    f="$(current)"
    echo "Branch:         $(git branch --show-current)"
    if [[ -z "$f" ]]; then echo "No active feature."; exit 0; fi
    scope=".workflow/$f/scope.md"
    echo "Active feature: $f"
    echo "Status:         $(cat ".workflow/$f/status" 2>/dev/null || echo none)"
    if [[ -f "$scope" ]] && scope_confirmed; then echo "Scope:          confirmed ($(fm title))"; else echo "Scope:          NOT confirmed"; fi
    if [[ -f "specs/$f.md" ]]; then echo "Spec:           specs/$f.md"; else echo "Spec:           (missing)"; fi
    for n in dev-notes.md spec-notes.md questions.md answers.md; do
      if [[ -s ".workflow/$f/$n" ]]; then echo "                .workflow/$f/$n"; fi
    done
    if url="$(gh pr view "$(git branch --show-current)" --json url,state --jq '"\(.state) \(.url)"' 2>/dev/null)"; then echo "Pull request:   $url"; fi
    ;;
  stop)
    rm -f .workflow/current
    echo "No active feature."
    ;;
  *)
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 1
    ;;
esac
