# Multi-Provider

Per-node model routing for cost, quality, and privacy optimization.

## Why This Matters

Different tasks need different models. Triage needs speed, deep analysis needs
capability, and sensitive data needs to stay local. APXM lets you assign models
per-node so each task gets the right trade-off.

## Examples

- **model_routing.py** -- Route a support ticket through fast, local, and powerful models. `dekk agents execute examples/python/multi-provider/model_routing.py`

## Requirements

Register backend/model routes with role aliases before executing the example.
The example expects `fast`, `local-sensitive`, `powerful`, and `formatter` to
resolve through the APXM backend registry:

```bash
dekk agents backend list
dekk agents backend add-model <backend> <SERVED_MODEL_ID> --alias fast
dekk agents backend add-model <backend> <SERVED_MODEL_ID> --alias local-sensitive
dekk agents backend add-model <backend> <SERVED_MODEL_ID> --alias powerful
dekk agents backend add-model <backend> <SERVED_MODEL_ID> --alias formatter
```

## Key API

```python
from apxm.backends import select_backend

fast_route = select_backend(alias="fast")
powerful_route = select_backend(alias="powerful")

triage = g.ask(name="triage", prompt="Classify this ticket: {ticket}", route=fast_route)
solution = g.reason(
    name="solution",
    prompt="Root cause analysis: {triage}\n{data}",
    route=powerful_route,
)
```

## Learn More

- [Model IDs](../../../crates/compiler/frontend/python/apxm/_generated/models.py)
- [Backend configuration](../../../crates/runtime/backends/README.md) (or run `dekk agents ops list`)
