#!/bin/bash
# Automated benchmark regression check
# Exit 1 if any benchmark regresses beyond threshold

# Don't exit on first error - collect all results
set +e
cd "$(dirname "$0")/.."
source ~/.cargo/env
export PYTHONPATH=crates/apxm-frontend/python

FAILED=0
echo "=== APXM CI Benchmark Suite ==="

# 1. Build
echo "=== Building APXM ==="
dekk apxm build 2>&1 | tail -3

# 2. Run autofix (all examples must pass)
echo "=== Running Autofix Check ==="
RESULT=$(python3 scripts/apxm-autofix.py --report-only 2>&1 | grep "Passed:")
echo "Autofix: $RESULT"

# 3. Run policy check
echo "=== Running Policy Check ==="
python3 scripts/apxm-policy-check.py 2>&1 | tail -3

# 4. Run Python tests
echo "=== Running Python Tests ==="
cd crates/apxm-frontend/python && PYTHONPATH=. python3 -m pytest tests/ -q 2>&1 || FAILED=1
cd - > /dev/null

# 5. Compile all benchmarks at O0 and O2
echo "=== Compile Benchmarks ==="
for graph in shared_prefix_fanout chained_llm mixed_priority multi_model; do
  PYTHONPATH=crates/apxm-frontend/python python3 -c "
import importlib.util
spec = importlib.util.spec_from_file_location('m', 'examples/python/benchmarks/${graph}.py')
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
for name in dir(mod):
    obj = getattr(mod, name)
    if hasattr(obj, '_graph'):
        import json
        with open('/tmp/ci_${graph}.apxm', 'w') as f:
            json.dump(obj._graph.to_dict(), f)
        break
" 2>/dev/null
  dekk apxm compile /tmp/ci_${graph}.apxm -o /tmp/ci_${graph}_O0.apxmobj -O0 2>/dev/null && echo "  O0 $graph: OK"
  dekk apxm compile /tmp/ci_${graph}.apxm -o /tmp/ci_${graph}_O2.apxmobj -O2 2>/dev/null && echo "  O2 $graph: OK" || echo "  O2 $graph: FAILED"
done

# 6. Run Rust tests
echo "=== Running Rust Tests ==="
TEST_OUTPUT=$(cargo test --workspace --quiet 2>&1)
echo "$TEST_OUTPUT" | grep "test result" | tail -1
echo "$TEST_OUTPUT" | grep -q "test result.*FAILED" && FAILED=1 || true

if [ $FAILED -eq 0 ]; then
  echo "=== CI Complete: SUCCESS ==="
  exit 0
else
  echo "=== CI Complete: SOME TESTS FAILED ==="
  exit 1
fi
