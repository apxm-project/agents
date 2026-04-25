# APXM Python Frontend Examples

## API Overview

```python
from apxm import compile, GraphRecorder

@compile()
def my_workflow(g: GraphRecorder, param: str) -> dict:
    """Docstring becomes workflow description."""
    node1 = g.ask("node1", "Question: {param}")
    node2 = g.think("node2", "Analysis: {node1}")
    result = g.merge("result", node1, node2)
    g.done(result)

print(my_workflow._graph.to_air())
```

### Core Operations

| Method | AIS Op | Purpose |
|--------|--------|---------|
| `g.ask()` | ASK | Query an LLM |
| `g.think()` | THINK | Reason over context |
| `g.reason()` | REASON | Extended reasoning |
| `g.text()` | TEXT | Constant text node |
| `g.merge()` | MERGE | Combine multiple inputs |
| `g.workflow_spawn()` | WORKFLOW_SPAWN | Run a child graph, artifact, or workflow as a separate execution |
| `g.print()` | PRINT | Output to user |
| `g.done()` | Terminal | Mark graph output |

### Edges

```python
node1 | node2    # Data edge: node2 receives node1's output
node1 >> node2   # Control edge: node2 runs after node1
```

### Agent Operations

```python
from apxm._generated.agents import claude

agent = g.spawn("name", profile=claude, cwd=cwd)  # Returns AgentHandle
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
    optimization/        Compiler passes (fusion, DCE, shared prefix)
    multi-agent/         Native multi-agent coordination
    multi-provider/      Per-node model routing
    memory/              Three-tier memory and RAG
    patterns/            Reusable workflow patterns
    real-world/          Complete production workflows
    self-hosted/         APXM building APXM (crown jewels)
    _benchmarks/         Internal performance tests
```

## Running Examples

```bash
# Execute directly through the CLI
dekk apxm execute examples/python/getting-started/hello.py

# Or emit .air and compile separately
python3 examples/python/getting-started/hello.py > hello.air
dekk apxm compile hello.air -O2 -o hello.apxmobj
dekk apxm run hello.apxmobj

# Compare optimization levels
dekk apxm execute examples/python/optimization/fusion.py -O0
dekk apxm execute examples/python/optimization/fusion.py -O2
```

## API Reference

- **GraphRecorder**: `crates/compiler/apxm-frontend/python/apxm/proxy.py`
- **AgentHandle / Team**: `crates/compiler/apxm-frontend/python/apxm/sugar.py`
- **Agent profiles**: `crates/compiler/apxm-frontend/python/apxm/_generated/agents.py`
- **Model IDs**: `crates/compiler/apxm-frontend/python/apxm/_generated/models.py`
