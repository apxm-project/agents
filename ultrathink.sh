#!/bin/bash
# ultrathink.sh — Launch the ultrathink-coder workflow
# The workflow will interactively ask for the task via ASK node.
# Usage: ./ultrathink.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GRAPH="$SCRIPT_DIR/examples/workflows/ultrathink-coder.ais"

echo "🧠 ULTRATHINK CODER"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Phase 1: Ask for task"
echo "Phase 2: Parallel planners — Architect + Adversary + Impl-Expert"
echo "Phase 3: Synthesis (ultrathink)"
echo "Phase 4: Implementation via Claude Code agent"
echo "Phase 5: Reflect + LTM update"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

cd "$SCRIPT_DIR"
dekk apxm execute "$GRAPH" \
  --emit-session \
  --emit-metrics /tmp/ultrathink-metrics.json

echo ""
echo "📊 Metrics saved to /tmp/ultrathink-metrics.json"
