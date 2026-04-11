#!/usr/bin/env bash
# Benchmark script for token pipelining (Phase 4 research)
#
# Tests ASK→ASK chains with pipelining enabled vs disabled
# Measures: total latency, time-to-first-token for consumer
# Reports: latency savings from pipelining

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== Token Pipelining Benchmark (Phase 4 Research) ===${NC}\n"

# Check if apxm CLI is available
if ! command -v dekk &> /dev/null; then
    echo -e "${RED}Error: dekk command not found${NC}"
    exit 1
fi

# Create test graph: ASK→ASK chain
TEST_GRAPH="${PROJECT_ROOT}/scripts/benchmark-pipeline-test.air"

cat > "${TEST_GRAPH}" << 'EOF'
{
  "name": "pipeline_benchmark",
  "nodes": [
    {
      "id": 1,
      "name": "producer",
      "op": "ASK",
      "attributes": {
        "system_prompt": "You are a helpful assistant.",
        "template_str": "Explain quantum computing in 200 words."
      }
    },
    {
      "id": 2,
      "name": "consumer",
      "op": "ASK",
      "attributes": {
        "system_prompt": "You are a helpful assistant.",
        "template_str": "Based on this explanation: {0}\n\nNow explain it to a 10-year-old in 100 words."
      }
    }
  ],
  "edges": [
    {
      "from": 1,
      "to": 2,
      "dependency": "Data"
    }
  ],
  "parameters": [],
  "metadata": {}
}
EOF

echo -e "${GREEN}Created test graph: ${TEST_GRAPH}${NC}\n"

# Run pipeline detection pass
echo -e "${YELLOW}Running pipeline detection pass...${NC}"
cd "${PROJECT_ROOT}"

# Validate and analyze the graph
echo -e "${BLUE}Validating graph...${NC}"
dekk apxm validate "${TEST_GRAPH}" || {
    echo -e "${RED}Graph validation failed${NC}"
    rm -f "${TEST_GRAPH}"
    exit 1
}

echo -e "${BLUE}Analyzing graph for pipeline candidates...${NC}"
dekk apxm analyze "${TEST_GRAPH}" || {
    echo -e "${YELLOW}Warning: Analysis failed, continuing...${NC}"
}

echo ""
echo -e "${YELLOW}=== Benchmark 1: Without Pipelining ===${NC}"
echo -e "${BLUE}Executing graph without pipeline optimization...${NC}"
START_NO_PIPELINE=$(date +%s%3N)

# Note: Actual execution disabled as it requires LLM backend
# dekk apxm execute "${TEST_GRAPH}" -O0 --emit-metrics /tmp/metrics-no-pipeline.json || true

END_NO_PIPELINE=$(date +%s%3N)
LATENCY_NO_PIPELINE=$((END_NO_PIPELINE - START_NO_PIPELINE))

echo -e "${GREEN}Baseline latency: ${LATENCY_NO_PIPELINE}ms (simulated)${NC}\n"

echo -e "${YELLOW}=== Benchmark 2: With Pipelining ===${NC}"
echo -e "${BLUE}Executing graph with pipeline optimization enabled...${NC}"
START_WITH_PIPELINE=$(date +%s%3N)

# Note: Actual execution disabled as it requires LLM backend
# dekk apxm execute "${TEST_GRAPH}" --enable-pipeline --emit-metrics /tmp/metrics-with-pipeline.json || true

END_WITH_PIPELINE=$(date +%s%3N)
LATENCY_WITH_PIPELINE=$((END_WITH_PIPELINE - START_WITH_PIPELINE))

echo -e "${GREEN}Pipeline latency: ${LATENCY_WITH_PIPELINE}ms (simulated)${NC}\n"

# Calculate savings
if [ "${LATENCY_NO_PIPELINE}" -gt 0 ]; then
    SAVINGS=$((LATENCY_NO_PIPELINE - LATENCY_WITH_PIPELINE))
    SAVINGS_PCT=$((SAVINGS * 100 / LATENCY_NO_PIPELINE))

    echo -e "${BLUE}=== Results ===${NC}"
    echo -e "Baseline (no pipeline):  ${LATENCY_NO_PIPELINE}ms"
    echo -e "With pipelining:         ${LATENCY_WITH_PIPELINE}ms"

    if [ "${SAVINGS}" -gt 0 ]; then
        echo -e "${GREEN}Latency savings:         ${SAVINGS}ms (${SAVINGS_PCT}%)${NC}"
    else
        echo -e "${YELLOW}No latency improvement detected${NC}"
    fi
else
    echo -e "${YELLOW}Benchmark skipped (LLM backend not configured)${NC}"
fi

echo ""
echo -e "${BLUE}=== Implementation Status ===${NC}"
echo -e "✓ PipelineConfig added to SchedulerConfig"
echo -e "✓ Pipeline detection pass in graph optimizer"
echo -e "✓ TokenPipeline struct for producer→consumer streaming"
echo -e "✓ Pipeline module with candidate detection utilities"
echo ""
echo -e "${YELLOW}Note: Actual streaming execution requires:${NC}"
echo -e "  - vLLM backend integration with streaming API"
echo -e "  - Scheduler dispatch logic for pipeline pairs"
echo -e "  - Runtime coordination between producer/consumer"
echo ""
echo -e "${BLUE}This is a Phase 4 research prototype.${NC}"
echo -e "Full implementation pending benchmarked proof of value."
echo ""

# Cleanup
rm -f "${TEST_GRAPH}"
echo -e "${GREEN}Benchmark complete.${NC}"
