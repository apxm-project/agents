# Getting Started

First contact with the APXM Python frontend.

## Why This Matters

These examples show the minimal workflow: define nodes, wire them with edges,
and record FrontendGraph for the compiler bridge. If you're new to APXM, start
here.

## Examples

- **tool_use.py** -- Native Python tool registration and invocation. Compile packaged source with `dekk agents compile-service-canonical <source-package>`.
- **hello.py** -- Hello world: single ASK node. Requires a registered LLM backend for execution. `dekk agents execute examples/python/getting-started/hello.py`
- **capability_groups_and_policy.py** -- Typed node-policy defaults for tool groups and token budgets. `dekk agents execute examples/python/getting-started/capability_groups_and_policy.py`

## Key API

```python
from apxm import GraphRecorder, NodePolicy, compile

@compile(default_policy=NodePolicy(capability_groups=["web"], token_budget=256))
def hello(g: GraphRecorder):
    result = g.ask(name="greet", prompt="Say hello in one sentence.")
    g.done(result)
```

## Learn More

- [docs/README.md](../../../docs/README.md) -- conceptual overview and learning path
- [API reference](../../../crates/compiler/frontend/python/apxm/proxy.py)
