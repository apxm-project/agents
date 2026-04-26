# Optimization

Compiler passes that remove duplicate work, unused context, and redundant
runtime interpolation.

## Why This Matters

APXM's compiler applies production-safe optimization passes to your agent
workflow. Default `-O2` keeps config-gated DSPy prompt optimization, dead-context
elimination, template specialization, scheduling metadata, and shared-prefix
analysis active. Semantic ASK-chain mutation, generic CSE, and prompt-layout
canonicalization are explicit experiments until their typed contracts are
enforced.

## Examples

- **dead_context.py** -- Dead context elimination: 5 inputs at O0, 2 at O2. `dekk apxm execute examples/python/optimization/dead_context.py`
- **shared_prefix.py** -- Explicit shared-prefix experiment. `dekk apxm execute examples/python/optimization/shared_prefix.py`
- **optimization_showcase.py** -- All optimizations in one workflow. `dekk apxm execute examples/python/optimization/optimization_showcase.py`

## Key API

```python
# Dead context: unused inputs are pruned
ctx1 | analysis  # Used (referenced as {ctx1})
ctx2 | analysis  # Dead -- template doesn't reference {ctx2}

# Compare optimization levels
# dekk apxm execute workflow.py -O0   # No optimizations
# dekk apxm execute workflow.py -O2   # Standard production-safe optimizations
```

## Learn More

- [_benchmarks/](../_benchmarks/) -- Stress tests for each optimization pass
