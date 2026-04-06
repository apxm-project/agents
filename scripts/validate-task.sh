#!/usr/bin/env bash
set -euo pipefail

TASK_INPUT="${1:-}"

if [[ -z "${TASK_INPUT}" ]]; then
  echo "ERROR: missing task description"
  echo "usage: $0 '<task description>'"
  exit 2
fi

NORMALIZED="$(echo "${TASK_INPUT}" | tr '[:lower:]' '[:upper:]' | xargs)"

if [[ "${NORMALIZED}" == "INVALID TASK" ]]; then
  echo "ERROR: task description is explicitly invalid"
  echo "Refusing to proceed without a real requirement."
  exit 3
fi

echo "Task description accepted."
exit 0
