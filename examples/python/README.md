# APXM Python Frontend Examples

## API Overview

```python
from apxm import compile, GraphRecorder

@compile()
def my_workflow(g: GraphRecorder, param: str) -> dict:
    """Docstring becomes workflow description."""
    node1 = g.ask(name="node1", prompt="Question: {param}")
    node2 = g.think(name="node2", prompt="Analysis: {node1}")
    result = g.merge("result", [node1, node2])
    g.done(result)

print(my_workflow.to_air())
```

### Core Operations

| Method | AIS Op | Purpose |
|--------|--------|---------|
| `g.ask()` | ASK | Query an LLM |
| `g.think()` | THINK | Reason over context |
| `g.reason()` | REASON | Extended reasoning |
| `g.print()` | PRINT | Output to user |
| `g.merge()` | MERGE | Combine multiple inputs |
| `g.workflow_spawn()` | WORKFLOW_SPAWN | Run a child workflow or artifact as a separate execution |
| `g.done()` | Terminal | Mark workflow output |

### Edges

```python
node1 | node2    # Data edge: node2 receives node1's output
node1 >> node2   # Control edge: node2 runs after node1
```

### Agent Operations

```python
from apxm._generated.agents import claude

agent = g.spawn("name", profile=claude, cwd=cwd)    # Returns AgentHandle
result = agent.ask("message")                       # COMMUNICATE via ACP

team = g.team("name")                               # Create team
member = team.add("name", profile=claude)            # Spawn into team
team.wait_all("sync")                                # Barrier
team.merge("results")                                # Merge outputs
```

## Structure

```
examples/python/
    getting-started/     First contact (hello world, tool use)
    parallelism/         Implicit DAG-based parallel execution
    optimization/        Compiler passes, workflow hints, and metrics
    multi-agent/         Native multi-agent coordination
    multi-provider/      Per-node model routing
    memory/              Three-tier memory and RAG
    patterns/            Reusable workflow patterns
    real-world/          Complete production workflows
    native-tools/        Native Python agent/tool handoff
    self-hosted/         APXM workflows for the APXM repo
    demos/               Skill-library proof-point demos
```

## Running Examples

```bash
# Execute directly through the CLI
dekk apxm execute examples/python/getting-started/hello.py

# Or compile and run separately
dekk apxm compile examples/python/getting-started/hello.py -O2 -o hello.apxmobj
dekk apxm run hello.apxmobj

# Compare optimization levels
dekk apxm execute examples/python/optimization/dead_context.py -O0
dekk apxm execute examples/python/optimization/dead_context.py -O2
```

## Runtime Requirements

Use `dekk apxm ...` for normal runs. Direct `python3` execution is useful for
small mock-backed demos and scripts that explicitly document direct execution,
but Dekk is the supported path for environment setup.

Example categories:

| Directory | Extra requirement |
|-----------|-------------------|
| `getting-started/`, `parallelism/`, `optimization/`, `patterns/` | APXM install; real LLM execution needs a registered backend |
| `multi-provider/` | Registered backend/model routes in APXM config |
| `native-tools/` | Mock backend for local runs or a registered real backend |
| `multi-agent/`, `real-world/`, `self-hosted/` | APXM ACP profile imports and authenticated agent CLIs |
| `self-hosted/vllm_graph_smoke.py` | APXM vLLM fork plus a registered served model alias |

APXM ACP profiles are available as typed imports. If an example imports
`apxm._generated.agents.claude`, the profile must exist in generated frontend
code and the host must be able to run the configured command. In this checkout:

- `claude` runs `npx -y @agentclientprotocol/claude-agent-acp@^0.24.2` and
  needs Node/npm plus a working Claude Code setup.
- `codex` runs `npx -y @zed-industries/codex-acp@^0.16.0` and needs Node/npm plus
  a working Codex/OpenAI setup.

Check what is available before executing agent examples:

```bash
dekk apxm agent list
dekk apxm agent test claude
dekk apxm agent test codex
```

Public examples are model-agnostic. Configure models through:

```bash
dekk apxm backend add <name> --type <cloud|onprem|local> --protocol <protocol>
dekk apxm backend add-model <name> <SERVED_MODEL_ID> --alias <role>
```

vLLM examples use role aliases such as `smoke`, `showcase`, or `benchmark`.
Those aliases are examples, not default models.

## API Reference

- **GraphRecorder**: `crates/compiler/apxm-frontend/python/apxm/proxy.py`
- **AgentHandle / Team**: `crates/compiler/apxm-frontend/python/apxm/sugar.py`
- **Agent profiles**: `crates/compiler/apxm-frontend/python/apxm/_generated/agents.py`
- **Model IDs**: `crates/compiler/apxm-frontend/python/apxm/_generated/models.py`
