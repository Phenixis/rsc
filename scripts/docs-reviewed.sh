#!/usr/bin/env bash
# Records that you re-read documents and found them accurate, so that scripts/check-docs.sh lets
# the push through without a change to them.
#
#   scripts/docs-reviewed.sh README.md docs/architecture.md
#
# Paths are relative to the root of the repository. Each record is a line "<commit> <document>" in
# a file under .git/, valid for the current commit only: the next commit asks again. This is a
# prompt to stop and read, not proof that you did.
set -uo pipefail

die() {
  echo "docs-reviewed: $*" >&2
  exit 1
}

(($# > 0)) || die "usage: scripts/docs-reviewed.sh DOC..."
head_sha="$(git rev-parse --verify -q HEAD)" || die "no commit yet"
top="$(git rev-parse --show-toplevel)"
for doc in "$@"; do
  [[ -f "$top/$doc" ]] || die "$doc is not a file (paths are relative to $top)"
done

ack="$(git rev-parse --path-format=absolute --git-path rsc-docs-reviewed)"
{
  [[ -f "$ack" ]] && grep "^$head_sha " "$ack"
  for doc in "$@"; do echo "$head_sha $doc"; done
} | sort -u >"$ack.new"
mv "$ack.new" "$ack"
echo "docs-reviewed: recorded for ${head_sha:0:7}: $*"
