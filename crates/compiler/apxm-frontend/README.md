# apxm-frontend

Python frontend for authoring APXM graphs.

## Overview

`apxm-frontend` is a pure-Python package that lets users define APXM graphs using a `@compile()` decorator and `GraphRecorder` proxy. The recorder captures operation calls, builds an in-memory graph, and emits `.air` (MLIR text in the AIS dialect) for the compiler pipeline.

## Package Structure

| Module | Description |
|--------|-------------|
| `decorators` | `@compile()` decorator that captures a function into an `.air` file |
| `proxy` | `GraphRecorder` and `NodeRef` for recording operation calls |
| `ir` | `ApxmGraph`, `GraphNode`, `GraphEdge`, `Parameter` data classes |
| `module` | `FlowModule` for multi-flow agent definitions |
| `sugar` | `AgentHandle` and `Team` convenience wrappers |
| `config` | `AgentConfig`, `ToolsConfig`, `BashConfig`, `ReadConfig`, `WriteConfig` |
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
| `agents.py` | Built-in agent profiles from `apxm-acp` |
| `emission.py` | MLIR emission helpers |
| `providers.py` | Built-in provider specs and protocols from `apxm-core` |

## Key Exports

- `compile` -- decorator that converts a Python function to `.air`
- `GraphRecorder` -- proxy object for recording `g.ask()`, `g.spawn_agent()`, etc.
- `NodeRef` -- handle to a recorded operation node
- `ApxmGraph` -- in-memory graph representation
- `FlowModule` -- multi-flow module with entry flow and sub-flows
- `AgentHandle` / `Team` -- sugar for multi-agent graphs

## Usage

```python
from apxm import compile, GraphRecorder

@compile()
def my_workflow(g: GraphRecorder):
    answer = g.ask("step", "What is 2+2?")
    g.done(answer)
```

## Template Variable Resolution

Templates support `{var_name}` for auto-wired references and `{N}` for positional:
```python
plan = g.ask("plan", "Create a plan for {topic}")  # auto-wires from 'topic' variable
code = g.ask("code", "Implement: {0}", plan)        # positional reference to 'plan'
```

## Execution Flow

1. `@compile()` analyzes function signature → derives graph parameters
2. Calls function with `GraphRecorder` + placeholder arguments
3. User code creates nodes via `g.ask()`, `g.think()`, etc.
4. `GraphRecorder` returns `ApxmGraph`
5. `CompiledFlow` serializes graph → subprocess `apxm execute` → results
