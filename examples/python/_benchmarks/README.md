# Benchmarks

Internal performance tests for APXM compiler optimization passes.

These benchmarks measure the effectiveness of specific compiler passes by
comparing O0 (no optimization) vs O2 (all passes enabled). They are not
intended as user-facing examples.

## Files

- **fusion_stress.py** -- 10 ASK/THINK pairs for FuseAskOps pass
- **dead_context_stress.py** -- 5 contexts, 1 used, for DeadContextElimination
- **shared_prefix_fanout.py** -- Shared prefix across 4 parallel nodes
- **demo_code_critique.py** -- Composite code-critique benchmark graph for APXM x vLLM runs; checked in, but not yet a finalized measured talk asset
- **prefix_fanout_large.py** -- Large-scale prefix sharing (stress test)
- **cse_stress.py** -- Common subexpression elimination
- **memo_cache_stress.py** -- Memoization cache effectiveness
- **chained_llm.py** -- Long sequential LLM chains
- **mixed_priority.py** -- Mixed priority scheduling
- **priority_scheduling.py** -- Priority-based node scheduling
- **multi_model.py** -- Multi-model routing baseline
- **benchmark_e2e.py** -- Repeated `dekk apxm execute` driver with CSV output
- **comparison_report.py** -- Markdown summary for `benchmark_e2e.py` CSV output

## Usage

```bash
dekk apxm execute examples/python/_benchmarks/fusion_stress.py -O0
dekk apxm execute examples/python/_benchmarks/fusion_stress.py -O2

# Composite benchmark graph
dekk apxm execute examples/python/_benchmarks/demo_code_critique.py -O0
dekk apxm execute examples/python/_benchmarks/demo_code_critique.py -O2

# Backend-free validation (compile only)
python3 examples/python/_benchmarks/benchmark_e2e.py --compile-only

# Full execution benchmark (requires a configured APXM backend)
python3 examples/python/_benchmarks/benchmark_e2e.py \
  --graph examples/python/_benchmarks/demo_code_critique.py \
  --iterations 5 \
  --output examples/python/_benchmarks/results/demo_code_critique.csv

# Markdown report from a benchmark CSV
python3 examples/python/_benchmarks/comparison_report.py \
  examples/python/_benchmarks/results/demo_code_critique.csv
```

## Notes

- `benchmark_e2e.py` drives the public `dekk apxm execute` or `dekk apxm compile`
  command, so it matches the repo's existing benchmark harness style.
- `--compile-only` is the safe default when no local backend is configured.
- `demo_code_critique.py` bootstraps the in-repo Python frontend path so
  `python3 examples/python/_benchmarks/demo_code_critique.py` can emit AIR
  directly from a fresh checkout.
- Treat `demo_code_critique.py` as a benchmark source until fresh metrics are
  captured; do not use it for speedup or KV hit-rate claims without measured
  results.

## Config

- Register a benchmark route with `dekk apxm backend add ...` and
  `dekk apxm backend add-model ... --alias benchmark`; benchmark graphs resolve
  that route with `select_backend(...)`.
- **dspy_stress.training.json** -- DSPy training data for optimization
