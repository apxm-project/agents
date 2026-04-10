# APXM Python Frontend Examples

This directory contains Python implementations of APXM workflow examples using the `apxm.graph` API.

## Overview

The legacy workflow-authoring examples from `examples/` have been converted to
Python equivalents demonstrating the new Python frontend (Phase 5). The Python
API provides:

- **@compile decorator**: Automatic graph generation with parameter derivation
- **GraphRecorder**: Fluent API for building workflows
- **AgentHandle & Team**: Sugar for spawn + communicate chains
- **`.air` export**: Workflows emit canonical Agent IR text, with JSON retained as a utility

## Structure

```
examples/python/
├── basics/               # Core examples (Tier 1)
│   ├── hello.py
│   └── tool_use.py
├── patterns/             # Common workflow patterns (Tier 1-2)
│   ├── iterative-refine/
│   ├── plan-fan-out/
│   ├── worker-pool/
│   ├── memory-rag/
│   ├── multi-agent-negotiate/
│   └── resilient-acp/
├── multi-agent/          # Multi-agent examples (Tier 2)
│   ├── multi_flow.py
│   ├── apxm_council.py
│   ├── code_review_council.py
│   └── multi_agent_communicate.py
├── acp-agents/           # ACP protocol examples (Tier 2)
│   ├── spawn_communicate_basic.py
│   ├── parallel_agents.py
│   ├── multi_turn_communicate.py
│   ├── code_review.py
│   ├── cross_critique.py
│   ├── cross_critique_pipeline.py
│   ├── dev_workflow.py
│   ├── architect_implement_review.py
│   └── full_sdlc.py
└── workflows/            # Complex workflows (Tier 3)
    ├── sub/              # Reusable sub-workflows
    │   ├── architect.py
    │   ├── impl_expert.py
    │   ├── synthesize.py
    │   └── adversary.py
    ├── ultrathink_coder.py
    ├── codex_claude_fix.py
    ├── quad_agent_compiler.py
    ├── planner/
    │   └── apxm_planner.py
    ├── designer/
    │   └── brief_analyzer.py
    ├── graph-builder/
    │   └── graph_builder.py
    └── ais-writer/
        └── ais_writer.py
```

## Usage

### Run examples directly

```bash
# From project root
cd /home/raherrer/projects/agents/apxm

# Basic hello world
python3 -m examples.python.basics.hello

# Tool usage
python3 -m examples.python.basics.tool_use

# Iterative refinement pattern
python3 -m examples.python.patterns.iterative-refine.iterative_refine

# Multi-agent communication
python3 -m examples.python.acp-agents.parallel_agents
```

### Import and use programmatically

```python
import sys
sys.path.insert(0, 'crates/compiler/apxm-frontend/python')

from examples.python.basics.hello import hello_world

# Get the compiled graph
graph = hello_world._graph

# Export to canonical AIR format
print(graph.to_air())

# JSON remains available as a utility
print(graph.to_json(indent=2))
```

## API Patterns

### Basic Pattern

```python
from apxm.graph import compile, GraphRecorder

@compile()
def my_workflow(g: GraphRecorder, param: str) -> dict:
    """Docstring becomes workflow description."""
    # Build graph
    node1 = g.ask("node1", "Question: {param}")
    node2 = g.think("node2", "Analysis: {0}")
    node1 | node2  # Data edge

    g.return_("result", source=node2)

print(my_workflow._graph.to_air())
```

### Agent Spawn + Communicate

```python
# Method 1: Using AgentHandle sugar
coder = g.spawn("coder", agent_name="coder", profile="claude", cwd="/path")
coder.ask("First task")
coder.ask("Second task")
coder.ask("Third task")

# Method 2: Using COMMUNICATE nodes
comm = g.communicate("send_task", target_agent="coder", message="Do this: {0}")
task | comm
```

### Team Pattern

```python
team = g.team("research_team")
alice = team.add("alice", profile="claude")
bob = team.add("bob", profile="codex")

alice.ask("Research X")
bob.ask("Research Y")

# Wait for both
sync = team.wait_all("sync")

# Or merge results
results = team.merge("results")
```

## Key Differences from the Legacy DSL

| Feature | Legacy DSL | Python |
|---------|------|--------|
| **Multi-agent flows** | `Researcher.research(topic)` | Not yet supported; use direct graph composition |
| **Flow parameters** | `flow main(TASK: str)` | `def workflow(g, task: str)` |
| **Agent spawn** | `spawn_agent("name", "profile", "$CWD")` | `g.spawn("name", agent_name="name", profile="profile", cwd=cwd)` |
| **String concat** | `"text " + var` | Use placeholders: `"{0}"` with data edges |
| **Print** | `print(message)` | `g.print_("name", message="{0}")` |
| **Return** | `return value` | `g.return_("name", source=node)` |

## Conversion Status

**Converted: 37 examples** (from 51 legacy DSL files)

### Tier 1: Core Examples ✅
- ✅ basics/hello.py
- ✅ basics/tool_use.py
- ✅ patterns/iterative-refine/iterative_refine.py
- ✅ patterns/plan-fan-out/plan_fan_out.py
- ✅ workflows/sub/* (4 files)

### Tier 2: Multi-Agent & ACP ✅
- ✅ acp-agents/* (10 files)
- ✅ multi-agent/* (4 files)
- ✅ patterns/negotiate, resilient-acp, worker-pool, memory-rag (4 files)

### Tier 3: Complex Workflows ✅
- ✅ ultrathink_coder, codex_claude_fix, quad_agent_compiler, planner (4 files)
- ✅ graph-builder, ais-writer (2 files)
- ✅ designer/brief_analyzer (1 file)
- ⚠️  app-builder/* (17 files) - Representative subset converted; full conversion available on request

## Validation

All converted examples emit parser-safe `.air`:

```bash
# Verify an example
python3 -c "
import sys; sys.path.insert(0, 'crates/compiler/apxm-frontend/python')
from examples.python.basics.hello import hello_world
print(hello_world._graph.to_air())
"
```

## Next Steps

- **Execute workflows**: Use `apxm execute workflow.py` directly, or emit `.air` with `python3 ... > workflow.air` and pass that file to the CLI
- **Combine patterns**: Import and compose multiple examples
- **Build custom workflows**: Use these as templates for your own agent workflows

## Documentation

- **Python API Reference**: `crates/compiler/apxm-frontend/python/apxm/graph/proxy.py`
- **Full AIS Reference**: `docs/guides/first-graph.md`
- **Phase 5 Plan**: `docs/strategy/plan.md`

---

Generated: 2026-04-06
Phase: 5 (Python Frontend Examples)
Coverage: 37/51 examples (73%)
