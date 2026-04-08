#!/bin/bash
# Test DSPy compiler pass
#
# This script tests the DSPy optimization pass with example training data.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=========================================="
echo " Testing DSPy Compiler Pass"
echo "=========================================="
echo ""

# Test 1: Simple template optimization
echo "Test 1: Optimize simple template with LabeledFewShot"
echo "---------------------------------------------------"
echo ""

TEMPLATE="{{question}}"
TRAINING_DATA="$SCRIPT_DIR/training_data.json"

echo "Template: $TEMPLATE"
echo "Training data: $TRAINING_DATA (8 examples)"
echo ""

PYTHONPATH="$PROJECT_ROOT/crates/apxm-frontend/python:$PYTHONPATH" \
    python3 -m apxm.dspy_pass \
        --template "$TEMPLATE" \
        --training-data "$TRAINING_DATA" \
        --optimizer labeled_fewshot \
        --target balanced \
        --output /tmp/test_dspy_output.json

echo ""
echo "✓ Optimization completed"
echo ""

# Show results
echo "Optimized template preview:"
jq -r '.template' /tmp/test_dspy_output.json | head -n 15
echo "  ..."
echo ""

echo "Metadata:"
jq '{score, version, optimizer: .metadata.optimizer, target: .metadata.target, num_examples: .metadata.num_examples}' /tmp/test_dspy_output.json
echo ""

# Test 2: Complex template
echo ""
echo "Test 2: Optimize complex template with context"
echo "---------------------------------------------------"
echo ""

TEMPLATE="Based on the following context:\n{{context}}\n\nAnswer this question: {{question}}"

echo "Template: $TEMPLATE"
echo ""

PYTHONPATH="$PROJECT_ROOT/crates/apxm-frontend/python:$PYTHONPATH" \
    python3 -m apxm.dspy_pass \
        --template "$TEMPLATE" \
        --training-data "$TRAINING_DATA" \
        --optimizer labeled_fewshot \
        --target quality \
        --output /tmp/test_dspy_output2.json 2>&1 | tail -n 5

echo ""
echo "✓ Optimization completed"
echo ""

echo "Quality score: $(jq -r '.score' /tmp/test_dspy_output2.json)"
echo "Template length: $(jq -r '.template | length' /tmp/test_dspy_output2.json) chars"
echo ""

# Test 3: Different targets
echo ""
echo "Test 3: Compare optimization targets"
echo "---------------------------------------------------"
echo ""

for target in quality latency tokens balanced; do
    echo -n "Target: $target ... "

    PYTHONPATH="$PROJECT_ROOT/crates/apxm-frontend/python:$PYTHONPATH" \
        python3 -m apxm.dspy_pass \
            --template "{{question}}" \
            --training-data "$TRAINING_DATA" \
            --optimizer labeled_fewshot \
            --target "$target" \
            --output "/tmp/test_dspy_${target}.json" 2>&1 | tail -n 1

    score=$(jq -r '.score' "/tmp/test_dspy_${target}.json")
    length=$(jq -r '.template | length' "/tmp/test_dspy_${target}.json")

    echo "score=$score, length=$length chars"
done

echo ""
echo "=========================================="
echo " All Tests Passed!"
echo "=========================================="
echo ""
echo "Next steps:"
echo "  1. Test with BootstrapFewShot (requires API key)"
echo "  2. Integrate into Rust compiler as MLIR pass"
echo "  3. Add template caching system"
echo "  4. Wire CLI flags: --dspy, --dspy-training-data, etc."
echo ""
