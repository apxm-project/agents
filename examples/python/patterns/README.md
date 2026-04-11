# Patterns

Reusable workflow patterns for common agent tasks.

## Why This Matters

These patterns solve recurring problems: iterative refinement, cross-critique,
and resilient delegation. They can be composed into larger workflows.

## Examples

- **iterative_refine.py** -- Generate, critique, and refine in a loop-free chain. `dekk apxm execute examples/python/patterns/iterative_refine.py`
- **cross_critique.py** -- Two agents critique each other's work. `dekk apxm execute examples/python/patterns/cross_critique.py`
- **resilient_pipeline.py** -- Spawn worker, send task, handle result with retry semantics. `dekk apxm execute examples/python/patterns/resilient_pipeline.py`

## Key API

```python
# Iterative refinement: generate -> critique -> refine
draft = g.ask("draft", "Write a proposal")
critique = g.think("critique", "Find weaknesses in: {draft}")
final = g.ask("final", "Improve based on feedback: {draft}\n{critique}")

# Resilient delegation
worker = g.spawn("worker", profile=claude, cwd=cwd)
result = g.communicate("task", target_agent="worker", message="{formatted}")
```

## Learn More

- [multi-agent/](../multi-agent/) -- More coordination patterns
- [real-world/](../real-world/) -- Full production workflows using these patterns
