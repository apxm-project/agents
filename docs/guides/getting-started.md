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
from apxm import compile, GraphRecorder


@compile()
def hello_world(g: GraphRecorder):
    """Simple greeting workflow."""
    greeting = g.ask("Generate a friendly greeting for someone learning about AI agents")
    g.print("Greeting: {greeting}")
    g.done(greeting)


if __name__ == "__main__":
    print(hello_world._graph.to_air())
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
from apxm import compile, GraphRecorder


@compile()
def researcher(g: GraphRecorder, topic: str):
    """Research a topic."""
    g.param("topic", "str")

    findings = g.ask("Research this topic: {topic}")
    summary = g.think("Summarize key insights: {findings}")

    g.print("=== Research on {topic} ===\n{summary}")
    g.done(summary)


if __name__ == "__main__":
    print(researcher._graph.to_air())
```

```bash
PYTHONPATH=crates/apxm-frontend/python python3 researcher.py > researcher.air
dekk apxm execute researcher.air -- "quantum computing"
```

---

## Common Patterns

### Multi-step reasoning

```python
from apxm import compile, GraphRecorder


@compile()
def analyst(g: GraphRecorder, topic: str):
    """Deep analysis workflow."""
    g.param("topic", "str")

    concepts = g.ask("Key concepts in {topic}")
    analysis = g.think("Analyze in depth: {concepts}")
    conclusion = g.reason("Synthesize findings: {analysis}")

    concepts >> analysis >> conclusion

    g.print("=== Analysis of {topic} ===\n{conclusion}")
    g.done(conclusion)
```

### Parallel expert council

```python
from apxm import compile, GraphRecorder


@compile()
def council(g: GraphRecorder, question: str):
    """Multi-expert consensus."""
    g.param("question", "str")

    e1 = g.ask("Expert 1 perspective: {question}")
    e2 = g.ask("Expert 2 perspective: {question}")
    e3 = g.ask("Expert 3 perspective: {question}")

    # All experts feed into synthesis
    synthesis = g.think("Synthesize expert opinions: {e1}\n{e2}\n{e3}")
    e1 | synthesis
    e2 | synthesis
    e3 | synthesis

    g.print("=== Council Decision ===\n{synthesis}")
    g.done(synthesis)
```

### Multi-agent collaboration

```python
from apxm import compile, GraphRecorder
from apxm._generated.agents import claude


@compile()
def coordinator(g: GraphRecorder, task: str):
    """Spawn and coordinate with external agent."""
    g.param("task", "str")

    # Spawn Claude agent
    researcher = g.spawn("researcher", profile=claude)

    # Delegate task to agent
    researcher.ask("Research this topic: {task}")
    findings = researcher.get_last_node()

    # Process agent output
    summary = g.think("Summarize these findings: {findings}")

    g.print("=== Research Summary ===\n{summary}")
    g.done(summary)
```

### Tool use

```python
from apxm import compile, GraphRecorder


@compile()
def tool_agent(g: GraphRecorder):
    """Tool invocation workflow."""
    # Register capability
    search = g.register_capability(
        capability_name="search",
        description="Search capability for web queries",
        parameters_schema={
            "type": "object",
            "properties": {"query": {"type": "string"}}
        },
    )

    # Decide what to search
    topic = g.ask("What topic should we research?")
    search >> topic

    # Invoke tool
    results = g.invoke(capability="search", params={"query": "{topic}"})
    topic | results

    # Summarize results
    summary = g.think("Summarize these search results: {results}")
    results | summary

    g.print("=== Search Results ===\n{summary}")
    g.done(summary)
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

---

## Real-World Performance

APXM's compiler optimizations and runtime scheduler deliver measurable speedups in production scenarios:

**vLLM on vendor GPU** (April 2026):
- **1.51x speedup** with O2 optimization vs O0 baseline
- **70% prefix cache hit rate** on shared context patterns
- **4,368 tokens saved** through PromptCanonicalization pass

See [docs/benchmarks/VLLM-LIVE-RESULTS.md](../benchmarks/VLLM-LIVE-RESULTS.md) for full report and methodology.

The combination of APXM's compiler passes and vLLM's prefix caching creates multiplicative performance gains for graph workflows with shared context patterns — exactly the scenario that agent systems encounter in real deployments.
