# Getting Started with APXM

APXM (Agent Programming eXecution Model) is a compiler and runtime for AI agent workflows — like LLVM for agent programs. It provides:

- **AIS DSL** — A high-level language for writing agent programs
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

Create `hello.ais`:

```ais
agent HelloWorld {
    @entry flow main() -> str {
        ask("Generate a friendly greeting for someone learning about AI agents") -> greeting
        return greeting
    }
}
```

### Run it

```bash
apxm execute hello.ais
```

What happens under the hood:
1. AIS DSL is parsed into an AST
2. AST is lowered to ApxmGraph JSON (canonical IR)
3. Graph is compiled to MLIR and optimized
4. Executable artifact is generated and run by the dataflow scheduler
5. The `ask` operation calls your configured LLM backend

### Compile and run separately

For production, separate compilation from execution:

```bash
# Compile to artifact
apxm compile hello.ais -o hello.apxmobj

# Run the artifact (skips compilation)
apxm run hello.apxmobj
```

Artifacts are portable, contain no source code, and can be distributed independently.

### With parameters

```ais
agent Researcher {
    @entry flow main(topic: str) -> str {
        ask("Research this topic: " + topic) -> findings
        return findings
    }
}
```

```bash
apxm execute researcher.ais "quantum computing"
```

---

## Common Patterns

### Multi-step reasoning

```ais
agent Analyst {
    @entry flow main(topic: str) -> str {
        ask("Key concepts in " + topic) -> concepts
        think("Analyze in depth: " + concepts) -> analysis
        reason("Synthesize: " + analysis) -> conclusion
        return conclusion
    }
}
```

### Parallel expert council

```ais
agent Council {
    @entry flow main(question: str) -> str {
        // These three run in parallel automatically (no data dependencies)
        ask("Expert 1: " + question) -> e1
        ask("Expert 2: " + question) -> e2
        ask("Expert 3: " + question) -> e3

        // Synthesis waits for all three
        ask("Synthesize: " + e1 + e2 + e3) -> result
        return result
    }
}
```

### Multi-agent collaboration

```ais
agent Researcher {
    flow research(topic: str) -> str {
        think("Research: " + topic) -> findings
        return findings
    }
}

agent Coordinator {
    @entry flow main() -> str {
        ask("What topic?") -> topic
        Researcher.research(topic) -> findings
        ask("Summarize: " + findings) -> summary
        return summary
    }
}
```

### Tool use

```ais
agent ToolAgent {
    capability search(query: str) -> str;
    tools: [search]

    @entry flow main() -> str {
        ask("What should we research?") -> topic
        search(topic) -> results
        ask("Summarize: " + results) -> summary
        return summary
    }
}
```

---

## Input Formats

APXM supports three input formats:

| Format | Extension | Best for |
|--------|-----------|----------|
| **AIS DSL** | `.ais` | Writing programs by hand (recommended) |
| **ApxmGraph JSON** | `.json` | Programmatic generation, low-level control |
| **Python API** | `.py` | Dynamic graph construction, integration |

All formats compile to the same ApxmGraph IR before MLIR lowering.

---

## Key Concepts

- **Dataflow execution** — Operations run when inputs are ready, not in textual order. Independent operations automatically parallelize.
- **Memory tiers** — STM (working memory), LTM (persistent facts), Episodic (event history).
- **Artifacts** — Compiled `.apxmobj` files are portable binaries. Ship without source code, run with any LLM backend.

For deeper understanding, see [PXM Foundations](../pxm/foundations.md).

---

## Next Steps

1. **Explore examples** — `examples/basics/hello.ais`, `examples/multi-agent/apxm_council.ais`, `examples/multi-agent/multi_flow.ais`
2. **CLI reference** — Run `apxm --help` for all commands and options
3. **Backends** — [Backend Setup](backends.md) for backend configuration
4. **Architecture** — [Architecture](../implementation/architecture.md) for system design
