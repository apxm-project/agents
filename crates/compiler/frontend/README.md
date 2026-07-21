# APXM Compiler Frontends

## Overview

APXM supports three authoring frontends: Rust `AirModuleBuilder`/
`FrontendGraph`, Python `GraphRecorder`/`ApxmGraph`, and TypeScript
`GraphBuilder`/`ApxmGraph`. Each records the compiler-owned `FrontendGraph`
shape, and only the Rust compiler validates and prints canonical AIR through
the explicit native bridge exposed by `apxm canonical-air`.

Python lives in [`python/`](python/); the public TypeScript package lives in
[`typescript/`](typescript/) with its own [README](typescript/README.md).
Neither language frontend contains an MLIR string printer or an independent
operation vocabulary.

## Package Structure

| Module | Description |
|--------|-------------|
| `decorators` | `@compile()` decorator that captures a function into an `.air` file |
| `proxy` | `GraphRecorder` and `NodeRef` for recording operation calls |
| `ir` | `ApxmGraph`, `GraphNode`, `GraphEdge`, `Parameter` data classes |
| `module` | `FlowModule` for multi-flow agent definitions |
| `sugar` | `AgentHandle` and `Team` convenience wrappers |
| `config` | `AgentConfig`, `NodePolicy`, `ExecutionOptions`, hook and middleware config dataclasses |
| `execution` | `CompiledFlow`, `ExecutionMode`, `WorkflowCheckpoint`, `validate_graph` |
| `providers` | `ProviderSpec`, `list_providers`, `resolve_provider` |
| `constants` | Generated graph-attribute constants from the shared `apxm-core` contract |
| `utils` | Shared utilities |
| `_generated/` | Auto-generated contract bindings from Rust definitions |

## Generated Code (`_generated/`)

| File | Description |
|------|-------------|
| `operations.py` | All AIS operation types and metadata from the shared contract |
| `constants.py` | Graph attribute constants from the shared contract |
| `agents.py` | Generated `AgentRef` type and release-owned profile catalogue |
| `providers.py` | Built-in provider specs and protocols from `apxm-backends` |
| `models.py` | Built-in model metadata from `apxm-backends` |

## Key Exports

- `compile` -- decorator that converts a Python function to `.air`
- `GraphRecorder` -- proxy object for recording `g.ask()`, `g.spawn_agent()`, etc.
- `NodeRef` -- handle to a recorded operation node
- `ApxmGraph` -- in-memory graph representation
- `NodePolicy` -- typed graph/node defaults lowered to stable node attrs
- `ExecutionOptions` -- execution-time hooks, built-in middleware, session-root, and request-time controls
- `FlowModule` -- multi-flow module with entry flow and sub-flows
- `AgentHandle` / `Team` -- sugar for multi-agent graphs

## Usage

```python
from apxm import compile, GraphRecorder

@compile()
def my_workflow(g: GraphRecorder):
    answer = g.ask(name="step", prompt="What is 2+2?")
    g.done(answer)
```

## Policy Defaults And Local Runtime Controls

```python
from pathlib import Path
from apxm import (
    ExecutionOptions,
    GraphRecorder,
    HookConfig,
    HookEvent,
    NodePolicy,
    TimeoutMiddlewareConfig,
    compile,
)

@compile(default_policy=NodePolicy(capability_groups=["web"], token_budget=256))
def workflow(g: GraphRecorder, topic: str):
    draft = g.ask(name="draft", prompt=f"Research {{topic}}")
    g.done(draft)

options = ExecutionOptions(
    session_root=Path(".apxm/sessions"),
    hooks=[HookConfig(event=HookEvent.NODE_COMPLETE, command="echo {{node_id}}")],
    middlewares=[TimeoutMiddlewareConfig(default_timeout_ms=5000)],
)

result = workflow.run_sync("middleware design", execution=options)
```

## Cross-Workflow Invocation

Use `g.call(...)` for same-process registered subflows and `g.workflow_spawn(...)`
when you want a separate child execution with its own session tree.

```python
from pathlib import Path
from apxm import GraphRecorder, NodePolicy, WorkflowTargetKind, compile

@compile(default_policy=NodePolicy(timeout_ms=5_000))
def parent(g: GraphRecorder):
    child = g.workflow_spawn(
        target_kind=WorkflowTargetKind.AIR_PATH,
        target=Path("workflows/review.air"),
        session_root=Path(".apxm/child-sessions"),
        node_policy=NodePolicy(timeout_ms=2_000),
    )
    g.done(child)
```

## Template Variable Resolution

Templates support named `{var_name}` references only:
```python
plan = g.ask(name="plan", prompt="Create a plan for {topic}")  # auto-wires from 'topic' variable
code = g.ask(name="code", prompt="Implement: {plan}")          # auto-wires from 'plan' variable
```

## Execution Flow

1. `@compile()` analyzes function signature → derives graph parameters
2. Calls function with `GraphRecorder` + placeholder arguments
3. User code creates nodes via `g.ask()`, `g.think()`, etc.
4. `GraphRecorder` returns `ApxmGraph`
5. `CompiledFlow` serializes graph → HTTP `/v1/execute` → results
