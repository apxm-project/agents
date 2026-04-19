# Optimization

Compiler passes that reduce LLM calls, tokens, and latency.

## Why This Matters

APXM's compiler applies optimization passes (like a traditional compiler) to
your agent workflow. These passes are automatic at `-O2` and can dramatically
reduce cost and latency without changing your code.

## Examples

- **fusion.py** -- ASK/THINK fusion: 6 LLM calls at O0, 3 at O2. `dekk apxm execute examples/python/optimization/fusion.py`
- **dead_context.py** -- Dead context elimination: 5 inputs at O0, 2 at O2. `dekk apxm execute examples/python/optimization/dead_context.py`
- **shared_prefix.py** -- Shared prefix reuse for KV-cache hits. `dekk apxm execute examples/python/optimization/shared_prefix.py`
- **optimization_showcase.py** -- All optimizations in one workflow. `dekk apxm execute examples/python/optimization/optimization_showcase.py`

## Key API

```python
# Fusion: adjacent ASK->THINK pairs merge into single LLM calls
q = g.ask("q", "Question")
e = g.think("e", "Elaborate: {q}")  # Fused with q at O2

# Dead context: unused inputs are pruned
ctx1 | analysis  # Used (referenced as {ctx1})
ctx2 | analysis  # Dead -- template doesn't reference {ctx2}

# Compare optimization levels
# dekk apxm execute workflow.py -O0   # No optimizations
# dekk apxm execute workflow.py -O2   # All optimizations
```

## Learn More

- [_benchmarks/](../_benchmarks/) -- Stress tests for each optimization pass
