#!/usr/bin/env bash
# run-showcase.sh - Compile and run the APXM showcase demo

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SHOWCASE_DIR="$PROJECT_ROOT/examples/python/demo"
OUTPUT_DIR="$PROJECT_ROOT/output/showcase"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

print_header() {
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}$1${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo
}

print_step() {
    echo -e "${GREEN}▶${NC} $1"
}

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

# Create output directory
mkdir -p "$OUTPUT_DIR"

cd "$PROJECT_ROOT"

print_header "APXM SHOWCASE DEMO - Optimization Comparison"

# Step 1: Generate MLIR
print_step "Step 1: Generate MLIR (.air format)"
print_info "Running: python3 -m examples.python.demo.showcase > showcase_mlir.air"
echo

if python3 -m examples.python.demo.showcase > "$OUTPUT_DIR/showcase_mlir.air" 2>&1; then
    print_success "MLIR generated: $OUTPUT_DIR/showcase_mlir.air"
    MLIR_LINES=$(wc -l < "$OUTPUT_DIR/showcase_mlir.air")
    print_info "MLIR output: $MLIR_LINES lines"
else
    print_error "Failed to generate MLIR"
    exit 1
fi
echo

# Step 2: Compile at O0
print_step "Step 2: Compile at O0 (no optimizations)"
print_info "Running: dekk apxm compile showcase.air -O0"
echo

if dekk apxm compile "$SHOWCASE_DIR/showcase.air" -O0 -o "$OUTPUT_DIR/showcase_O0.apxmobj" 2>&1 | tee "$OUTPUT_DIR/compile_O0.log"; then
    print_success "Compiled O0: $OUTPUT_DIR/showcase_O0.apxmobj"
    O0_SIZE=$(stat -c%s "$OUTPUT_DIR/showcase_O0.apxmobj" 2>/dev/null || stat -f%z "$OUTPUT_DIR/showcase_O0.apxmobj")
    print_info "Artifact size: $O0_SIZE bytes"
else
    print_warning "O0 compilation failed (check if .air was generated)"
fi
echo

# Step 3: Compile at O2
print_step "Step 3: Compile at O2 (all optimizations)"
print_info "Running: dekk apxm compile showcase.air -O2 --emit-diagnostics"
echo

if dekk apxm compile "$SHOWCASE_DIR/showcase.air" -O2 -o "$OUTPUT_DIR/showcase_O2.apxmobj" --emit-diagnostics "$OUTPUT_DIR/diagnostics_O2.json" 2>&1 | tee "$OUTPUT_DIR/compile_O2.log"; then
    print_success "Compiled O2: $OUTPUT_DIR/showcase_O2.apxmobj"
    O2_SIZE=$(stat -c%s "$OUTPUT_DIR/showcase_O2.apxmobj" 2>/dev/null || stat -f%z "$OUTPUT_DIR/showcase_O2.apxmobj")
    print_info "Artifact size: $O2_SIZE bytes"

    if [ -f "$OUTPUT_DIR/diagnostics_O2.json" ]; then
        print_info "Diagnostics: $OUTPUT_DIR/diagnostics_O2.json"
    fi
else
    print_warning "O2 compilation failed (check if .air was generated)"
fi
echo

# Step 4: Analyze parallelism
print_step "Step 4: Analyze parallelism and critical path"
print_info "Running: dekk apxm analyze showcase.air"
echo

if dekk apxm analyze "$SHOWCASE_DIR/showcase.air" --json > "$OUTPUT_DIR/analysis.json" 2>&1; then
    print_success "Analysis complete: $OUTPUT_DIR/analysis.json"

    # Try to extract key metrics if jq is available
    if command -v jq &> /dev/null; then
        PARALLELISM=$(jq -r '.parallelism // "N/A"' "$OUTPUT_DIR/analysis.json" 2>/dev/null || echo "N/A")
        CRITICAL_PATH=$(jq -r '.critical_path_length // "N/A"' "$OUTPUT_DIR/analysis.json" 2>/dev/null || echo "N/A")
        SPEEDUP=$(jq -r '.estimated_speedup // "N/A"' "$OUTPUT_DIR/analysis.json" 2>/dev/null || echo "N/A")

        echo -e "${MAGENTA}  Parallelism:${NC} $PARALLELISM"
        echo -e "${MAGENTA}  Critical Path:${NC} $CRITICAL_PATH"
        echo -e "${MAGENTA}  Estimated Speedup:${NC} $SPEEDUP"
    fi
else
    print_warning "Analysis failed (check if .air was generated)"
fi
echo

# Step 5: Decompile to compare
print_step "Step 5: Decompile artifacts to compare transformations"

if [ -f "$OUTPUT_DIR/showcase_O0.apxmobj" ]; then
    print_info "Decompiling O0 artifact..."
    if dekk apxm decompile "$OUTPUT_DIR/showcase_O0.apxmobj" > "$OUTPUT_DIR/decompiled_O0.air" 2>&1; then
        print_success "Decompiled O0: $OUTPUT_DIR/decompiled_O0.air"
    fi
fi

if [ -f "$OUTPUT_DIR/showcase_O2.apxmobj" ]; then
    print_info "Decompiling O2 artifact..."
    if dekk apxm decompile "$OUTPUT_DIR/showcase_O2.apxmobj" > "$OUTPUT_DIR/decompiled_O2.air" 2>&1; then
        print_success "Decompiled O2: $OUTPUT_DIR/decompiled_O2.air"
    fi
fi
echo

# Step 6: Explain workflow
print_step "Step 6: Generate human-readable workflow explanation"
if dekk apxm explain "$SHOWCASE_DIR/showcase.air" > "$OUTPUT_DIR/explanation.txt" 2>&1; then
    print_success "Explanation: $OUTPUT_DIR/explanation.txt"
fi
echo

# Summary
print_header "SUMMARY"

echo -e "${MAGENTA}Generated files:${NC}"
echo "  $OUTPUT_DIR/"
ls -lh "$OUTPUT_DIR" | tail -n +2 | awk '{printf "    %s  %s\n", $9, $5}'
echo

echo -e "${MAGENTA}Next steps:${NC}"
echo "  1. View MLIR:           cat $OUTPUT_DIR/showcase_mlir.air"
echo "  2. Compare artifacts:   diff $OUTPUT_DIR/decompiled_O0.air $OUTPUT_DIR/decompiled_O2.air"
echo "  3. View diagnostics:    cat $OUTPUT_DIR/diagnostics_O2.json | jq"
echo "  4. View analysis:       cat $OUTPUT_DIR/analysis.json | jq"
echo "  5. Read explanation:    cat $OUTPUT_DIR/explanation.txt"
echo

echo -e "${MAGENTA}To execute the workflow:${NC}"
echo "  # With session tracing:"
echo "  dekk apxm execute $SHOWCASE_DIR/showcase.air --emit-session --emit-metrics $OUTPUT_DIR/metrics.json"
echo
echo "  # Run pre-compiled artifact:"
echo "  dekk apxm run $OUTPUT_DIR/showcase_O2.apxmobj --emit-metrics $OUTPUT_DIR/metrics_run.json"
echo

print_success "Showcase demo processing complete!"
