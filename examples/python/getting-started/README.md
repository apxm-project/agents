# Getting Started

First contact with the APXM Python frontend.

## Why This Matters

These examples show the minimal workflow: define nodes, wire them with edges,
and emit `.air` for the compiler. If you're new to APXM, start here.

## Examples

- **hello.py** -- Hello world: single ASK node. `dekk apxm execute examples/python/getting-started/hello.py`
- **tool_use.py** -- Native Python tool registration and invocation. `dekk apxm execute examples/python/getting-started/tool_use.py`
- **tool_groups_and_policy.py** -- Typed node-policy defaults for tool groups and token budgets. `dekk apxm execute examples/python/getting-started/tool_groups_and_policy.py`

## Key API

```python
from apxm import GraphRecorder, NodePolicy, compile

@compile(default_policy=NodePolicy(tool_groups=["web"], token_budget=256))
def hello(g: GraphRecorder):
    result = g.ask(name="greet", prompt="Say hello in one sentence.")
    g.done(result)
```

## Learn More

- [docs/README.md](../../../docs/README.md) -- conceptual overview and learning path
- [API reference](../../../crates/compiler/apxm-frontend/python/apxm/proxy.py)
