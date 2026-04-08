#!/bin/bash
# Profile-Guided Optimization loop for APXM workflows
#
# Usage: scripts/pgo-compile.sh <graph.apxm> [output.apxmobj]
#
# Process:
# 1. Cold compile (no profile, baseline optimization)
# 2. Profile run (collect runtime statistics)
# 3. PGO compile (use profile to guide optimization decisions)
# 4. Comparison (show improvements)

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $0 <graph.apxm> [output.apxmobj]"
    echo ""
    echo "Profile-Guided Optimization loop:"
    echo "  1. Compile without profile (cold start)"
    echo "  2. Execute and collect profile"
    echo "  3. Recompile WITH profile (warm optimization)"
    echo "  4. Execute again — should be faster"
    exit 1
fi

GRAPH=$1
OUTPUT=${2:-${GRAPH%.apxm}_pgo.apxmobj}
COLD_OBJ=/tmp/apxm_cold.apxmobj
PROFILE=/tmp/apxm_profile.json
PGO_OBJ=/tmp/apxm_pgo.apxmobj
COLD_SESSION=/tmp/apxm_cold_session
PGO_SESSION=/tmp/apxm_pgo_session

echo "=== PGO Workflow for $GRAPH ==="
echo ""

# Step 1: Cold compile (no profile)
echo "[1/4] Cold compile (baseline, no profile)"
dekk apxm compile "$GRAPH" -o "$COLD_OBJ" -O2
echo "    → $COLD_OBJ"
echo ""

# Step 2: Profile run
echo "[2/4] Profile run (collect runtime statistics)"
dekk apxm run "$COLD_OBJ" --emit-session "$COLD_SESSION" --emit-profile "$PROFILE"
echo "    → Profile: $PROFILE"
echo ""

# Step 3: PGO compile (with profile)
echo "[3/4] PGO compile (profile-guided optimization)"
dekk apxm compile "$GRAPH" -o "$PGO_OBJ" -O2 --profile "$PROFILE"
echo "    → $PGO_OBJ"
echo ""

# Step 4: PGO run (verify improvement)
echo "[4/4] PGO run (should be faster)"
dekk apxm run "$PGO_OBJ" --emit-session "$PGO_SESSION"
echo ""

# Compare timings
echo "=== Comparison ==="
COLD_TIME=$(jq -r '.duration_ms' "$COLD_SESSION"/*/manifest.json)
PGO_TIME=$(jq -r '.duration_ms' "$PGO_SESSION"/*/manifest.json)

echo "Cold run:  ${COLD_TIME}ms"
echo "PGO run:   ${PGO_TIME}ms"

if [ "$PGO_TIME" -lt "$COLD_TIME" ]; then
    SPEEDUP=$(awk "BEGIN {printf \"%.2f\", ($COLD_TIME - $PGO_TIME) / $COLD_TIME * 100}")
    echo "Speedup:   ${SPEEDUP}% faster"
else
    SLOWDOWN=$(awk "BEGIN {printf \"%.2f\", ($PGO_TIME - $COLD_TIME) / $COLD_TIME * 100}")
    echo "Change:    ${SLOWDOWN}% slower (optimization may vary)"
fi

echo ""
echo "=== Profile Summary ==="
jq -r '.node_stats | to_entries[] | "\(.key): \(.value.avg_latency_ms)ms (calls: \(.value.call_count))"' "$PROFILE"

echo ""
echo "=== Artifacts ==="
echo "Cold artifact:   $COLD_OBJ"
echo "PGO artifact:    $PGO_OBJ"
echo "Profile:         $PROFILE"
echo "Cold session:    $COLD_SESSION"
echo "PGO session:     $PGO_SESSION"

# Copy PGO artifact to final output if specified
if [ "$OUTPUT" != "$PGO_OBJ" ]; then
    cp "$PGO_OBJ" "$OUTPUT"
    echo "Output artifact: $OUTPUT"
fi
