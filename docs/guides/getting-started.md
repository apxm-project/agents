# Getting Started with APXM

APXM (Agent Programming eXecution Model) is a compiler and runtime for AI agent workflows — like LLVM for agent programs. It provides:

- **Python frontend** — The primary authoring experience for workflows
- **ApxmGraph IR** — Canonical intermediate representation (dataflow DAGs)
- **Compiler** — Lowers to MLIR, optimizes, generates `.apxmobj` artifacts
- **Runtime** — Parallel dataflow scheduler with memory tiers and LLM backends

Write agent workflows once, compile them, run with any LLM backend.

---

## Installation

### Prerequisites

- **Conda or Mamba** ([Miniforge](https://github.com/conda-forge/miniforge) recommended)
- **Git**
- **dekk** (`pip install dekk`)

Rust, CMake, and MLIR/LLVM are installed automatically by the installer.

### Install

```bash
git clone https://github.com/randreshg/apxm
cd apxm
dekk apxm install
```

This sets up the conda environment (MLIR/LLVM 21, build tools), builds APXM, and installs default components. Use `dekk apxm <command>` to interact with the project — it resolves the nearest `.dekk.toml` and activates the environment automatically.

#### Interactive mode (default)

Running `dekk apxm install` interactively prompts you to select components:

```
APXM Install
────────────────────────────────────

? Select components to install:
 » ● Compiler + Runtime — APXM compiler and dataflow runtime
   ● MCP Server — HTTP API + MCP protocol server for IDE integration
   ○ Graph Visualizer — Interactive web-based DAG visualizer (requires Node.js)

[1/4] ▶ Setting up environment
  ✓ Setting up environment
[2/4] ▶ Installing Compiler + Runtime
  ✓ Installing Compiler + Runtime (138s)
[3/4] ▶ Installing MCP Server
  ✓ Installing MCP Server (42s)

  ✓ Installation complete!
```

`»` = cursor, `●` (blue) = selected, `○` (dim) = not selected. Arrow keys to move, Space to toggle, Enter to confirm. Escape cancels selection; Ctrl-C cancels installation.

#### Non-interactive mode (agents / CI)

```bash
dekk apxm install --no-interactive   # default components, no prompt
dekk apxm install --all              # all components, no prompt
dekk apxm install --components mcp-server,graph-viewer  # explicit list
```

#### Optional global wrapper

By default, you use `dekk apxm <command>` which is worktree-safe. If you prefer a bare `apxm` command (single-worktree setup), add `--wrap`:

```bash
dekk apxm install --wrap
# Now: apxm doctor, apxm compile, etc.
```

### Verify

```bash
dekk apxm test
```

### Troubleshooting

- **Build failures** — Check `.dekk/install.log` for details, then re-run `dekk apxm install`
- **Verbose error output** — `dekk apxm install --verbose` shows the last 15 lines of build output on failure
- **Start fresh** — `dekk apxm setup --force && dekk apxm install`

---

## Register an Inference Backend

Before running programs, register at least one LLM provider:

```bash
# OpenAI
apxm backend add my-openai --type cloud --protocol openai --api-key sk-...

# Anthropic
apxm backend add my-anthropic --type cloud --protocol anthropic --api-key sk-ant-...

# Ollama (local, no API key)
apxm backend add local --type cloud --protocol ollama
```

Verify: `apxm backend test`

See [Backend Setup](backends.md) for backend configuration, enterprise gateways, and security details.

---

## Your First Program

### Write it

Create `hello.py`:

```python
from apxm.graph import GraphRecorder, compile


@compile
def hello_world(g: GraphRecorder) -> dict:
    greeting = g.ask(
        "greeting",
        "Generate a friendly greeting for someone learning about AI agents",
    )
    g.return_("output", source=greeting)
    return g.to_graph().to_dict()
```

Emit the canonical `.air` IR:

```bash
PYTHONPATH=crates/apxm-frontend/python python3 hello.py > hello.air
```

### Run it

```bash
dekk apxm execute hello.air
```

You can also execute the Python workflow directly:

```bash
dekk apxm execute hello.py
```

What happens under the hood:
1. Python authoring code builds an `ApxmGraph`
2. The graph is serialized as `.air` (the canonical frontend exchange format)
3. The compiler lowers the graph to MLIR and optimizes it
4. An executable artifact is generated and run by the dataflow scheduler
5. The `ask` operation calls your configured LLM backend

### Compile and run separately

For production, separate compilation from execution:

```bash
# Compile to artifact
dekk apxm compile hello.air -o hello.apxmobj

# Run the artifact (skips compilation)
dekk apxm run hello.apxmobj
```

Artifacts are portable, contain no source code, and can be distributed independently.

### With parameters

```python
from apxm.graph import GraphRecorder, compile


@compile
def researcher(g: GraphRecorder, topic: str) -> dict:
    findings = g.ask("findings", "Research this topic: {0}")
    g.return_("output", source=findings)
    return g.to_graph().to_dict()
```

```bash
PYTHONPATH=crates/apxm-frontend/python python3 researcher.py > researcher.json
dekk apxm execute researcher.json -- "quantum computing"
```

---

## Common Patterns

### Multi-step reasoning

```python
@compile
def analyst(g: GraphRecorder, topic: str) -> dict:
    concepts = g.ask("concepts", "Key concepts in {0}")
    analysis = g.think("analysis", "Analyze in depth: {0}")
    conclusion = g.reason("conclusion", "Synthesize: {0}")
    concepts >> analysis >> conclusion
    g.return_("output", source=conclusion)
    return g.to_graph().to_dict()
```

### Parallel expert council

```python
@compile
def council(g: GraphRecorder, question: str) -> dict:
    e1 = g.ask("expert_1", "Expert 1: {0}")
    e2 = g.ask("expert_2", "Expert 2: {0}")
    e3 = g.ask("expert_3", "Expert 3: {0}")
    result = g.ask("result", "Synthesize: {0} {1} {2}")
    e1 | result
    e2 | result
    e3 | result
    g.return_("output", source=result)
    return g.to_graph().to_dict()
```

### Multi-agent collaboration

```python
@compile
def coordinator(g: GraphRecorder, topic: str) -> dict:
    researcher = g.spawn_agent("researcher", profile="claude")
    findings = g.communicate("findings", recipient=researcher, protocol="acp", message="{0}")
    summary = g.ask("summary", "Summarize: {0}")
    findings >> summary
    g.return_("output", source=summary)
    return g.to_graph().to_dict()
```

### Tool use

```python
@compile
def tool_agent(g: GraphRecorder) -> dict:
    register = g.register_capability(
        "register_search",
        capability_name="search",
        description="Search capability for web queries",
        parameters_schema={"type": "object", "properties": {"query": {"type": "string"}}},
    )
    topic = g.ask("topic", "What should we research?")
    register >> topic
    results = g.invoke("results", capability="search", params={"query": "{0}"})
    topic | results
    summary = g.ask("summary", "Summarize: {0}")
    results | summary
    g.return_("output", source=summary)
    return g.to_graph().to_dict()
```

---

## Input Formats

APXM supports three relevant formats:

| Format | Extension | Best for |
|--------|-----------|----------|
| **Python API** | `.py` | Primary workflow authoring experience |
| **Agent IR** | `.air` | Canonical text IR for CLI handoff, diffs, and generated graphs |
| **ApxmGraph JSON** | `.json` | Utility/debug export and compatibility tooling |
| **Artifact** | `.apxmobj` | Distribution and repeated execution |

Python frontends emit `.air` before MLIR lowering. JSON remains available as a utility export.

---

## Key Concepts

- **Dataflow execution** — Operations run when inputs are ready, not in textual order. Independent operations automatically parallelize.
- **Memory tiers** — STM (working memory), LTM (persistent facts), Episodic (event history).
- **Artifacts** — Compiled `.apxmobj` files are portable binaries. Ship without source code, run with any LLM backend.

For deeper understanding, see [PXM Foundations](../pxm/foundations.md).

---

## Next Steps

1. **Explore examples** — `examples/python/basics/hello.py`, `examples/python/basics/tool_use.py`, `examples/python/patterns/plan-fan-out/plan_fan_out.py`
2. **CLI reference** — Run `apxm --help` for all commands and options
3. **Backends** — [Backend Setup](backends.md) for backend configuration
4. **Architecture** — [Architecture](../implementation/architecture.md) for system design
