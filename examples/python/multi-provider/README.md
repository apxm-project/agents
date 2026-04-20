# Multi-Provider

Per-node model routing for cost, quality, and privacy optimization.

## Why This Matters

Different tasks need different models. Triage needs speed, deep analysis needs
capability, and sensitive data needs to stay local. APXM lets you assign models
per-node so each task gets the right trade-off.

## Examples

- **model_routing.py** -- Route a support ticket through fast, local, and powerful models. `dekk apxm execute examples/python/multi-provider/model_routing.py`

## Key API

```python
# Fast model for triage
triage = g.ask("triage", "Classify this ticket: {ticket}")

# Powerful model for analysis
solution = g.reason("solution", "Root cause analysis: {triage}\n{data}")

# In production, specify model per-node:
# g.ask("triage", "...", model="claude-haiku-4-5")
# g.reason("solution", "...", model="claude-opus-4")
```

## Learn More

- [Model IDs](../../../crates/compiler/apxm-frontend/python/apxm/_generated/models.py)
- [Backend configuration](../../../crates/runtime/apxm-backends/README.md) (or run `dekk apxm ops list`)
