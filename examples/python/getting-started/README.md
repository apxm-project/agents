# Getting Started

First contact with the APXM Python frontend.

## Why This Matters

These examples show the minimal workflow: define nodes, wire them with edges,
and emit `.air` for the compiler. If you're new to APXM, start here.

## Examples

- **hello.py** -- Hello world: single ASK node. `dekk apxm execute examples/python/getting-started/hello.py`
- **tool_use.py** -- Tool-augmented LLM call. `dekk apxm execute examples/python/getting-started/tool_use.py`

## Key API

```python
from apxm import compile, GraphRecorder

@compile()
def hello(g: GraphRecorder):
    result = g.ask("greet", "Say hello in one sentence.")
    g.done(result)
```

## Learn More

- [docs/README.md](../../../docs/README.md) -- conceptual overview and learning path
- [API reference](../../../crates/compiler/apxm-frontend/python/apxm/proxy.py)
