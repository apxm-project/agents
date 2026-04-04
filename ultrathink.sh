#!/bin/bash
# ultrathink.sh — Launch the ultrathink-coder workflow
# Usage: ./ultrathink.sh "Add X feature to APXM"

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GRAPH="$SCRIPT_DIR/examples/workflows/ultrathink-coder.apxm"

if [ -z "$1" ]; then
  echo "Usage: $0 \"<task description>\""
  echo ""
  echo "Example: $0 \"Add rate limiting to the LLM backend router\""
  exit 1
fi

TASK="$1"

echo "🧠 ULTRATHINK: $TASK"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Phase 1: Cache check (LTM)"
echo "Phase 2: Parallel planners — Architect + Adversary + Impl-Expert"
echo "Phase 3: Synthesis (ultrathink)"
echo "Phase 4: Dual implementation — Conservative (codex) + Bold (claude)"
echo "Phase 5: Verify + Reflect + Pick winner"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

cd "$SCRIPT_DIR"
dekk apxm execute "$GRAPH" \
  --param "TASK=$TASK" \
  --emit-session \
  --emit-metrics /tmp/ultrathink-metrics.json

echo ""
echo "📊 Metrics saved to /tmp/ultrathink-metrics.json"
