#!/usr/bin/env bash
# Orchestrator helper for the SPEC / DEV / TEST pipeline (see specs/README.md).
#
#   scripts/workflow.sh start <feature>   begin a feature: sets .workflow/current, status 'spec'
#   scripts/workflow.sh status            show the active feature, its status and files
#   scripts/workflow.sh stop              deactivate (no active feature)
set -euo pipefail
cd "$(dirname "$0")/.."

current() { tr -d '[:space:]' 2>/dev/null <.workflow/current || true; }

case "${1:-status}" in
  start)
    feature="${2:?usage: workflow.sh start <feature>}"
    [[ "$feature" =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "invalid feature slug '$feature' (lowercase letters, digits, hyphens)" >&2; exit 1; }
    mkdir -p ".workflow/$feature"
    echo "$feature" >.workflow/current
    [[ -f ".workflow/$feature/status" ]] || echo spec >".workflow/$feature/status"
    echo "Active feature: $feature (status: $(cat ".workflow/$feature/status"))"
    echo "Next: SPEC writes specs/$feature.md and the tests, then TEST confirms they fail."
    ;;
  status)
    f="$(current)"
    if [[ -z "$f" ]]; then echo "No active feature."; exit 0; fi
    echo "Active feature: $f"
    echo "Status:         $(cat ".workflow/$f/status" 2>/dev/null || echo none)"
    [[ -f "specs/$f.md" ]] && echo "Spec:           specs/$f.md" || echo "Spec:           (missing)"
    for n in dev-notes.md spec-notes.md questions.md answers.md; do
      if [[ -s ".workflow/$f/$n" ]]; then echo "                .workflow/$f/$n"; fi
    done
    ;;
  stop)
    rm -f .workflow/current
    echo "No active feature."
    ;;
  *)
    echo "usage: workflow.sh start <feature> | status | stop" >&2
    exit 1
    ;;
esac
