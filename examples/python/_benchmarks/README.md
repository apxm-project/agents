# Benchmarks

Internal performance tests for APXM compiler optimization passes.

These benchmarks measure the effectiveness of specific compiler passes by
comparing O0 (no optimization) vs O2 (all passes enabled). They are not
intended as user-facing examples.

## Files

- **fusion_stress.py** -- 10 ASK/THINK pairs for FuseAskOps pass
- **dead_context_stress.py** -- 5 contexts, 1 used, for DeadContextElimination
- **shared_prefix_fanout.py** -- Shared prefix across 4 parallel nodes
- **prefix_fanout_large.py** -- Large-scale prefix sharing (stress test)
- **cse_stress.py** -- Common subexpression elimination
- **memo_cache_stress.py** -- Memoization cache effectiveness
- **chained_llm.py** -- Long sequential LLM chains
- **mixed_priority.py** -- Mixed priority scheduling
- **priority_scheduling.py** -- Priority-based node scheduling
- **multi_model.py** -- Multi-model routing baseline

## Usage

```bash
dekk apxm execute examples/python/_benchmarks/fusion_stress.py -O0
dekk apxm execute examples/python/_benchmarks/fusion_stress.py -O2
```

## Config

- **vllm_bench_config.toml** -- vLLM backend configuration for benchmarks
- **dspy_stress.training.json** -- DSPy training data for optimization
