#!/usr/bin/env bash
# apxm-autofix-agent.sh - Spawn Claude Code agent for autofix task
#
# DEPRECATED: This script is deprecated. Use the APXM runtime instead:
#   python3 scripts/apxm-autofix.py --auto-fix
#   OR
#   dekk apxm execute .agents/skills/autofix/autofix_workflow.air --emit-session -- "SCOPE"
#
# Usage (legacy):
#   bash scripts/apxm-autofix-agent.sh /tmp/autofix-tasks/cluster-attr_mismatch.txt

set -euo pipefail

echo "=========================================="
echo "⚠️  WARNING: This script is deprecated"
echo "=========================================="
echo ""
echo "Recommended approach:"
echo "  python3 scripts/apxm-autofix.py --auto-fix"
echo ""
echo "Or directly via APXM runtime:"
echo "  dekk apxm execute .agents/skills/autofix/autofix_workflow.air --emit-session -- \"examples/python\""
echo ""
echo "Continuing with legacy direct Claude invocation..."
echo "=========================================="
echo ""

if [ $# -ne 1 ]; then
    echo "Usage: $0 <task-file>" >&2
    exit 1
fi

TASK_FILE="$1"

if [ ! -f "$TASK_FILE" ]; then
    echo "Error: Task file not found: $TASK_FILE" >&2
    exit 1
fi

TASK_NAME=$(basename "$TASK_FILE" .txt)
LOG_FILE="/tmp/autofix-${TASK_NAME}-$(date +%Y%m%d-%H%M%S).log"

echo "=========================================="
echo "APXM Autofix Agent"
echo "=========================================="
echo "Task: $TASK_NAME"
echo "Task file: $TASK_FILE"
echo "Log file: $LOG_FILE"
echo ""
echo "Starting Claude Code agent..."
echo ""

# Read the task file and pass to Claude Code
TASK_PROMPT=$(cat "$TASK_FILE")

# Spawn Claude Code with the task
# --permission-mode bypassPermissions: Allow agent to make file changes
# --print: Output results to stdout
#
# Note: This assumes 'claude' is available in PATH
# Adjust the command based on actual Claude Code CLI interface

if command -v claude &> /dev/null; then
    claude \
        --permission-mode bypassPermissions \
        --print \
        "$TASK_PROMPT" 2>&1 | tee "$LOG_FILE"

    EXIT_CODE=${PIPESTATUS[0]}
else
    echo "Error: 'claude' command not found in PATH" >&2
    echo "" >&2
    echo "Please install Claude Code or manually run:" >&2
    echo "  cat $TASK_FILE" >&2
    echo "" >&2
    exit 127
fi

echo ""
echo "=========================================="
if [ $EXIT_CODE -eq 0 ]; then
    echo "✓ Agent completed successfully"
else
    echo "✗ Agent failed with exit code: $EXIT_CODE"
fi
echo "Log saved to: $LOG_FILE"
echo "=========================================="

exit $EXIT_CODE
