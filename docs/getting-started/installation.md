# Installing APXM

APXM (Agent Programming eXecution Model) compiles and runs AI agent workflows. Think LLVM for agent programs:

- **Python frontend** -- author workflows as decorated functions
- **ApxmGraph IR** -- canonical intermediate representation (dataflow DAGs)
- **Compiler** -- lowers to MLIR, optimizes, emits `.apxmobj` artifacts
- **Runtime** -- parallel dataflow scheduler with memory tiers and LLM backends

Write once, compile, run with any backend.

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| **Conda or Mamba** | [Miniforge](https://github.com/conda-forge/miniforge) recommended |
| **Git** | Any recent version |
| **dekk** | `pip install dekk` |

Rust, CMake, and MLIR/LLVM install automatically during setup.

---

## Install

```bash
git clone https://github.com/randreshg/apxm
cd apxm
dekk apxm install
```

This creates the conda environment (MLIR/LLVM 21, build tools), builds APXM, and registers default components. Use `dekk apxm <command>` for all interactions -- it finds the nearest `.dekk.toml` and activates the environment automatically.

### Interactive mode (default)

`dekk apxm install` prompts you to select components:

```
APXM Install
------------------------------------

? Select components to install:
 >> * Compiler + Runtime -- APXM compiler and dataflow runtime
    * MCP Server -- HTTP API + MCP protocol server for IDE integration
    o Graph Visualizer -- Interactive web-based DAG visualizer (requires Node.js)

[1/4] > Setting up environment
  OK Setting up environment
[2/4] > Installing Compiler + Runtime
  OK Installing Compiler + Runtime (138s)
[3/4] > Installing MCP Server
  OK Installing MCP Server (42s)

  OK Installation complete!
```

`>>` = cursor, `*` (blue) = selected, `o` (dim) = not selected. Arrow keys move, Space toggles, Enter confirms. Escape cancels selection; Ctrl-C cancels installation.

### Non-interactive mode (agents / CI)

```bash
dekk apxm install --no-interactive                      # default components, no prompt
dekk apxm install --all                                 # all components, no prompt
dekk apxm install --components mcp-server,graph-viewer  # explicit list
```

### Optional global wrapper

By default you use `dekk apxm <command>`, which is worktree-safe. For a bare `apxm` command (single-worktree setups), pass `--wrap`:

```bash
dekk apxm install --wrap
# Now: apxm doctor, apxm compile, etc.
```

---

## Verify

Run the test suite to confirm everything works:

```bash
dekk apxm test
```

Check environment health:

```bash
dekk apxm doctor
```

`doctor` validates MLIR/LLVM paths, conda environment, and backend connectivity. Pass `--json` for machine-readable output.

---

## Register an Inference Backend

APXM needs at least one LLM provider before it can run programs.

```bash
# OpenAI
apxm backend add my-openai --type cloud --protocol openai --api-key sk-...

# Anthropic
apxm backend add my-anthropic --type cloud --protocol anthropic --api-key sk-ant-...

# Ollama (local, no API key)
apxm backend add local --type cloud --protocol ollama
```

Verify connectivity:

```bash
apxm backend test
```

See [Backend Setup](../guides/backends.md) for enterprise gateways, model routing, and security details.

---

## First Run

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

Execute it directly:

```bash
dekk apxm execute hello.py
```

Or compile and run as separate steps:

```bash
# Emit IR
PYTHONPATH=crates/apxm-frontend/python python3 hello.py > hello.air

# Compile to artifact
dekk apxm compile hello.air -o hello.apxmobj

# Run the artifact (no recompilation)
dekk apxm run hello.apxmobj
```

What happens under the hood:

1. Python builds an `ApxmGraph` from the decorated function
2. The graph serializes to `.air` (canonical frontend exchange format)
3. The compiler lowers to MLIR and runs optimization passes
4. An executable artifact runs on the parallel dataflow scheduler
5. `ask` calls your configured LLM backend

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Build failures | Check `.dekk/install.log`, then re-run `dekk apxm install` |
| Verbose errors | `dekk apxm install --verbose` shows the last 15 lines of build output on failure |
| Start fresh | `dekk apxm setup --force && dekk apxm install` |
| Environment issues | `dekk apxm doctor --json` for detailed diagnostics |

---

## Next Steps

- **Build graphs** -- [Your First Graph](first-graph.md) walks through single-node, pipeline, fan-out, and parameterized patterns
- **Configure backends** -- [Backend Setup](../guides/backends.md) covers providers, model routing, and rate limiting
- **Optimize** -- [Optimization Overview](../optimization/overview.md) explains compiler passes and `-O2` gains
- **Integrate** -- [vLLM](../integrations/vllm.md) and [DSPy](../integrations/dspy.md) for advanced deployments
- **Reference** -- [Config Reference](../reference/config.md) for `~/.apxm/config.toml` options, [Graph Format](../reference/graph-format.md) for the `.apxm` JSON schema
