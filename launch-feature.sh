#!/bin/bash
# launch-feature.sh — Substitute feature text into the workflow and execute via dekk
# Usage: ./launch-feature.sh "your feature description"
# Example: ./launch-feature.sh "Add dynamic model router to APXM runtime (Gap A1)"

set -euo pipefail

FEATURE="${1:-}"
if [[ -z "$FEATURE" ]]; then
    echo "Usage: $0 \"feature description\""
    echo ""
    echo "Example:"
    echo "  $0 \"Add ModelRouter with health monitoring and circuit breaker (Gap A1)\""
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKFLOW_TEMPLATE="$SCRIPT_DIR/examples/14-add-feature/add-apxm-feature.json"
TMP_WORKFLOW="/tmp/apxm-feature-$(date +%s).json"

echo "=== APXM Feature Workflow ==="
echo "Feature: $FEATURE"
echo ""

# Escape the feature text for JSON embedding (handles quotes, backslashes)
ESCAPED_FEATURE=$(python3 -c "import json,sys; print(json.dumps(sys.argv[1])[1:-1])" "$FEATURE")
sed "s|{{FEATURE_REQUEST}}|${ESCAPED_FEATURE}|g" "$WORKFLOW_TEMPLATE" > "$TMP_WORKFLOW"

echo "Workflow prepared: $TMP_WORKFLOW"
echo ""

cd "$SCRIPT_DIR"
echo "Launching workflow via dekk..."
dekk apxm execute "$TMP_WORKFLOW"

rm -f "$TMP_WORKFLOW"
