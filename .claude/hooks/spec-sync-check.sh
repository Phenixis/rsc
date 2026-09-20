#!/usr/bin/env bash
# Stop hook of the rsc-spec agent: it may not finish while the spec and the tests disagree.
#   - specs/<feature>.md exists and lists its test files (front matter, `tests:`)
#   - none of those test files is newer than the spec (a test change means a spec update)
#   - every behavior B<n> defined in the spec is cited by at least one test (`spec: B<n>`)
#   - no test cites a behavior the spec does not define
set -uo pipefail

payload="$(cat)"
# Already blocked once in this run: let it finish rather than loop forever.
jq -e '.stop_hook_active == true' >/dev/null 2>&1 <<<"$payload" && exit 0

root="$(cd "${CLAUDE_PROJECT_DIR:-.}" && pwd -P)"
feature="$(tr -d '[:space:]' 2>/dev/null <"$root/.workflow/current" || true)"
[[ "$feature" =~ ^[a-z0-9][a-z0-9-]*$ ]] || exit 0 # no active workflow: nothing to check

fail() { echo "rsc-spec cannot finish yet: $*" >&2; exit 2; }

spec="$root/specs/$feature.md"
[[ -f "$spec" ]] || fail "specs/$feature.md does not exist."

# Test files listed in the front matter:  tests:\n  - path\n  - path
mapfile -t tests < <(awk '
  /^---[[:space:]]*$/ { fm++; next }
  fm == 1 && /^tests:/ { in_list = 1; next }
  fm == 1 && in_list && /^[[:space:]]*-[[:space:]]+/ { sub(/^[[:space:]]*-[[:space:]]+/, ""); print; next }
  fm == 1 && in_list { in_list = 0 }
' "$spec")
((${#tests[@]} > 0)) || fail "specs/$feature.md lists no test files in its front matter ('tests:')."

stale=()
for t in "${tests[@]}"; do
  [[ -f "$root/$t" ]] || fail "the spec lists '$t' but that file does not exist."
  [[ "$root/$t" -nt "$spec" ]] && stale+=("$t")
done
((${#stale[@]} == 0)) || fail "these test files are newer than specs/$feature.md: ${stale[*]}. Update the spec (behaviors, change log) so it describes them, then finish."

# Behaviors defined in the spec:  - **B1.** ...
mapfile -t defined < <(grep -oE '^[[:space:]]*[-*][[:space:]]+\*\*B[0-9]+' "$spec" | grep -oE 'B[0-9]+' | sort -u)
((${#defined[@]} > 0)) || fail "specs/$feature.md defines no behavior (expected list items like '- **B1.** ...')."
mapfile -t cited < <(cd "$root" && cat -- "${tests[@]}" | grep -oE 'spec:[[:space:]]*B[0-9]+([,[:space:]]+B[0-9]+)*' | grep -oE 'B[0-9]+' | sort -u)

missing=$(comm -23 <(printf '%s\n' "${defined[@]}") <(printf '%s\n' "${cited[@]}") | tr '\n' ' ')
unknown=$(comm -13 <(printf '%s\n' "${defined[@]}") <(printf '%s\n' "${cited[@]}") | tr '\n' ' ')
[[ -z "${missing// /}" ]] || fail "behaviors with no test: $missing (add tests citing them with '// spec: B<n>', or remove them from the spec)."
[[ -z "${unknown// /}" ]] || fail "tests cite behaviors the spec does not define: $unknown"
exit 0
