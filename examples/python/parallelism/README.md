# Parallelism

Implicit DAG-based parallel execution.

## Why This Matters

In traditional frameworks you manually manage threads or async tasks. APXM's
compiler analyzes the data dependency graph and schedules independent nodes
concurrently -- you just declare the edges.

## Examples

- **fan_out_synthesize.py** -- Plan, fan-out 3 parallel writers, synthesize. `dekk apxm execute examples/python/parallelism/fan_out_synthesize.py`
- **expert_council.py** -- 5 parallel expert analyses merged into consensus. `dekk apxm execute examples/python/parallelism/expert_council.py`
- **worker_pool.py** -- Parallel worker pool pattern. `dekk apxm execute examples/python/parallelism/worker_pool.py`

## Key API

```python
# Nodes with no edges between them run in parallel automatically
section_1 = g.ask(name="s1", prompt="Write section 1 from plan: {plan}")
section_2 = g.ask(name="s2", prompt="Write section 2 from plan: {plan}")
section_3 = g.ask(name="s3", prompt="Write section 3 from plan: {plan}")

# Assembly waits for all -- DAG enforces the barrier
result = g.think(name="assemble", prompt="{s1}\n{s2}\n{s3}")
```

## Learn More

- [docs/README.md](../../../docs/README.md) -- Architecture and scheduling
