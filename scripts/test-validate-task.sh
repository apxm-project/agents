#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT_DIR}/scripts/validate-task.sh"

run_expect_success() {
  local input="$1"
  if "${SCRIPT}" "${input}" >/dev/null 2>&1; then
    echo "PASS: success as expected for '${input}'"
  else
    echo "FAIL: expected success for '${input}'"
    exit 1
  fi
}

run_expect_failure() {
  local input="$1"
  if "${SCRIPT}" "${input}" >/dev/null 2>&1; then
    echo "FAIL: expected failure for '${input}'"
    exit 1
  else
    echo "PASS: failure as expected for '${input}'"
  fi
}

run_expect_failure ""
run_expect_failure "INVALID TASK"
run_expect_failure " invalid task "
run_expect_success "Add pluggable eviction policies to MemoCache"
run_expect_success "Route long-context requests through ModelRouter based on ContextStack depth"

echo "All tests passed."
