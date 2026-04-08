#!/bin/bash
# DSPy-Enabled APXM Compilation
#
# Wrapper script for compiling APXM graphs with DSPy prompt optimization.
# This is a prototype — full integration will add --dspy flags to `apxm compile`.
#
# Usage:
#   ./scripts/dspy-compile.sh graph.apxm [training-data.json] [optimizer] [target]
#
# Examples:
#   # Basic optimization with training data
#   ./scripts/dspy-compile.sh workflow.apxm examples.json
#
#   # Auto-detect from session profiles (future)
#   ./scripts/dspy-compile.sh workflow.apxm auto
#
#   # Use specific optimizer and target
#   ./scripts/dspy-compile.sh workflow.apxm examples.json bootstrap_fewshot latency

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Parse arguments
GRAPH_FILE="$1"
TRAINING_DATA="${2:-auto}"
OPTIMIZER="${3:-labeled_fewshot}"
TARGET="${4:-balanced}"

if [ -z "$GRAPH_FILE" ]; then
    cat <<EOF
Usage: $0 <graph.apxm> [training-data.json] [optimizer] [target]

Arguments:
  graph.apxm         Input APXM graph file
  training-data      Training data JSON (default: auto)
  optimizer          DSPy optimizer: labeled_fewshot, bootstrap_fewshot (default: labeled_fewshot)
  target             Optimization target: quality, latency, tokens, balanced (default: balanced)

Examples:
  # Basic optimization
  $0 workflow.apxm examples.json

  # Optimize for latency
  $0 workflow.apxm examples.json labeled_fewshot latency

  # Use BootstrapFewShot (requires API key)
  $0 workflow.apxm examples.json bootstrap_fewshot balanced
EOF
    exit 1
fi

if [ ! -f "$GRAPH_FILE" ]; then
    echo "ERROR: Graph file not found: $GRAPH_FILE"
    exit 1
fi

echo "==============================================="
echo " DSPy-Enabled APXM Compilation (Prototype)"
echo "==============================================="
echo "Graph:          $GRAPH_FILE"
echo "Training data:  $TRAINING_DATA"
echo "Optimizer:      $OPTIMIZER"
echo "Target:         $TARGET"
echo ""

# Auto-detect training data from session profiles
if [ "$TRAINING_DATA" = "auto" ]; then
    echo "⚙ Auto-detecting training data from ~/.apxm/sessions/..."

    # TODO: Implement profile scanner that:
    # 1. Scans ~/.apxm/sessions/ for recent executions
    # 2. Filters successful runs (manifest.json status: "Completed")
    # 3. Extracts node inputs/outputs from results.json
    # 4. Converts to DSPy training data format
    # 5. Saves to /tmp/dspy_auto_training.json

    echo "WARNING: Auto-detection not yet implemented"
    echo "         Using stub training data for prototype"

    TRAINING_DATA="/tmp/dspy_stub_training.json"
    cat > "$TRAINING_DATA" <<'EOF'
[
  {
    "inputs": {"question": "What is microservices?"},
    "output": "A software architecture pattern where applications are composed of independent services."
  },
  {
    "inputs": {"question": "How does caching work?"},
    "output": "Caching stores frequently accessed data in fast memory to reduce expensive operations."
  }
]
EOF
    echo "         Stub data: 2 examples"
fi

if [ ! -f "$TRAINING_DATA" ]; then
    echo "ERROR: Training data file not found: $TRAINING_DATA"
    exit 1
fi

# Extract LLM nodes from graph (simplified — full implementation would parse JSON)
echo ""
echo "⚙ Analyzing graph for LLM operations..."
LLM_NODE_COUNT=$(grep -o '"op":\s*"ASK"\|"op":\s*"THINK"\|"op":\s*"REASON"' "$GRAPH_FILE" | wc -l)
echo "   Found $LLM_NODE_COUNT LLM operations to optimize"

if [ "$LLM_NODE_COUNT" -eq 0 ]; then
    echo ""
    echo "⚠ No LLM operations found in graph. Skipping DSPy optimization."
    echo "   Compiling without optimization..."
    dekk apxm compile "$GRAPH_FILE" -O2
    exit 0
fi

# Run DSPy optimization on each template
# NOTE: This is a simplified prototype. Full implementation would:
# 1. Parse graph JSON to extract all ASK/THINK/REASON nodes
# 2. For each node, extract template_str
# 3. Run DSPy optimization via Python subprocess
# 4. Update graph with optimized templates
# 5. Save optimized graph to .apxm file
# 6. Compile optimized graph

echo ""
echo "⚙ Running DSPy optimization..."
echo "   Optimizer: $OPTIMIZER"
echo "   Target metric: $TARGET"
echo ""

# For prototype, just show what would be optimized
echo "   [Prototype mode: Demonstrating DSPy pass invocation]"
echo ""

# Example: Optimize a single template
EXAMPLE_TEMPLATE="{{question}}"

echo "   Example template: $EXAMPLE_TEMPLATE"
echo "   Calling DSPy pass..."

PYTHONPATH="$PROJECT_ROOT/crates/apxm-frontend/python:$PYTHONPATH" \
    python3 -m apxm.dspy_pass \
        --template "$EXAMPLE_TEMPLATE" \
        --training-data "$TRAINING_DATA" \
        --optimizer "$OPTIMIZER" \
        --target "$TARGET" \
        --output /tmp/dspy_optimized_template.json 2>&1 | sed 's/^/     /'

if [ $? -eq 0 ]; then
    echo ""
    echo "✓ DSPy optimization completed"
    echo ""
    echo "   Optimized template preview:"
    jq -r '.template' /tmp/dspy_optimized_template.json | head -n 10 | sed 's/^/     /'
    echo "     ..."
    echo ""
    echo "   Quality score: $(jq -r '.score' /tmp/dspy_optimized_template.json)"
    echo "   Template length: $(jq -r '.template | length' /tmp/dspy_optimized_template.json) chars"
else
    echo ""
    echo "✗ DSPy optimization failed. See errors above."
    exit 1
fi

# In full implementation, this would update the graph and compile
echo ""
echo "==============================================="
echo " Next Steps (Full Implementation)"
echo "==============================================="
echo ""
echo "1. Parse $GRAPH_FILE to extract all LLM nodes"
echo "2. Optimize each node's template_str with DSPy"
echo "3. Update graph with optimized templates"
echo "4. Save to ${GRAPH_FILE%.apxm}_optimized.apxm"
echo "5. Compile: dekk apxm compile ${GRAPH_FILE%.apxm}_optimized.apxm -O3"
echo ""
echo "For now, compile manually:"
echo "  dekk apxm compile $GRAPH_FILE -O2"
echo ""
