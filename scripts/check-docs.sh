#!/usr/bin/env bash
# Refuses a push when the documentation may be out of date. Used by the git pre-push hook
# (.githooks/pre-push) and by the Claude Code hook (.claude/hooks/docs-before-push.sh), so both
# apply the same rules.
#
# The rules are in scripts/docs-map.txt (override the file with DOCS_MAP). The whole branch is
# compared with origin/main (its merge base), not only the commits of this push: the question is
# "is the documentation up to date in this pull request?". A document listed by a triggered rule
# must be modified by the branch, or recorded as reviewed with scripts/docs-reviewed.sh.
#
# Exit 0: nothing to say. Exit 1: a document is missing, or the rules file is broken (the message on
# stderr says which). Run it from inside the repository.
set -uo pipefail

map="${DOCS_MAP:-$(dirname "${BASH_SOURCE[0]}")/docs-map.txt}"
die() {
  echo "check-docs: $*" >&2
  exit 1
}

# ------------------------------------------------------------------ the rules
[[ -f "$map" ]] || die "the rules file $map does not exist"

kinds=() globs=() docs=()
re_always='^[[:space:]]*always[[:space:]]*=>[[:space:]]*([^[:space:]]+)[[:space:]]*$'
re_kind='^[[:space:]]*(changed|structure):([^[:space:]]+)[[:space:]]*=>[[:space:]]*([^[:space:]]+)[[:space:]]*$'
lineno=0
while IFS= read -r original || [[ -n "$original" ]]; do
  lineno=$((lineno + 1))
  line="${original%%#*}"
  [[ -z "${line//[[:space:]]/}" ]] && continue
  if [[ "$line" =~ $re_always ]]; then
    kinds+=(always) globs+=("") docs+=("${BASH_REMATCH[1]}")
  elif [[ "$line" =~ $re_kind ]]; then
    kinds+=("${BASH_REMATCH[1]}") globs+=("${BASH_REMATCH[2]}") docs+=("${BASH_REMATCH[3]}")
  else
    die "$map: line $lineno is not a rule: $original"
  fi
done <"$map"

# ------------------------------------------------------------------ what the branch changes
head_sha="$(git rev-parse --verify -q HEAD)" || exit 0 # no commit yet: nothing to check
if ! base="$(git merge-base origin/main HEAD 2>/dev/null)"; then
  base="$(git hash-object -t tree /dev/null)" # no origin/main: everything is new
fi
[[ "$base" == "$head_sha" ]] && exit 0

change_status=() change_path=()
while IFS=$'\t' read -r status first second; do
  [[ -n "$status" ]] || continue
  case "${status:0:1}" in
    R) # a rename moves a file: both the old and the new path are structural changes
      change_status+=(R R) change_path+=("$first" "$second")
      ;;
    A | D) change_status+=("${status:0:1}") change_path+=("$first") ;;
    *) change_status+=(M) change_path+=("$first") ;;
  esac
done < <(git diff --name-status -M "$base" HEAD)
((${#change_path[@]} > 0)) || exit 0

matches() { # glob path (a leading ! negates the glob)
  local glob="$1" path="$2"
  if [[ "$glob" == '!'* ]]; then
    [[ "$path" != ${glob:1} ]]
  else
    [[ "$path" == $glob ]]
  fi
}

# ------------------------------------------------------------------ which documents are due
declare -A reason
due=()
for i in "${!kinds[@]}"; do
  doc="${docs[$i]}"
  why=""
  case "${kinds[$i]}" in
    always) why="every pull request" ;;
    changed | structure)
      for j in "${!change_path[@]}"; do
        [[ "${kinds[$i]}" == structure && "${change_status[$j]}" == M ]] && continue
        if matches "${globs[$i]}" "${change_path[$j]}"; then
          case "${change_status[$j]}" in
            A) verb=added ;; D) verb=deleted ;; R) verb=renamed ;; *) verb=changed ;;
          esac
          why="${change_path[$j]} $verb"
          break
        fi
      done
      ;;
  esac
  [[ -n "$why" && -z "${reason[$doc]:-}" ]] || continue
  reason["$doc"]="$why"
  due+=("$doc")
done

ack="$(git rev-parse --path-format=absolute --git-path rsc-docs-reviewed)"
missing=()
for doc in "${due[@]}"; do
  updated=0
  for path in "${change_path[@]}"; do
    [[ "$path" == "$doc" ]] && updated=1 && break
  done
  ((updated)) && continue
  [[ -f "$ack" ]] && grep -qxF "$head_sha $doc" "$ack" && continue
  missing+=("$doc")
done
((${#missing[@]} == 0)) && exit 0

{
  echo "Documentation check failed: this branch may leave documents out of date (${head_sha:0:7})."
  echo
  for doc in "${missing[@]}"; do
    printf '  - %s (%s)\n' "$doc" "${reason[$doc]}"
  done
  cat <<EOF

For each one: re-read it against the changes, and if something is outdated or missing, update it
and commit. If it is already accurate, record that you reviewed it:

  scripts/docs-reviewed.sh ${missing[*]}

The record is valid for the current commit only. Then push again (on its own, not chained after a
commit in the same command). The rules are in $map.
EOF
} >&2
exit 1
