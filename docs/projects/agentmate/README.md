# AgentMate — Frontend SDK for A-PXM

AgentMate is the developer-facing framework for building agents and workflows on top of A-PXM. It provides ergonomic Rust and Python APIs for authoring agent graphs that compile to optimized A-PXM artifacts, but it is a frontend layer rather than the architectural core.

**A-PXM is the engine. AgentMate is one steering wheel.**

## Relationship

```
Developer → AgentMate (authoring) → A-PXM (compilation + execution) → LLM/Tool backends
```

| Layer | Role | Analogy |
|-------|------|---------|
| **AgentMate** | Define agents, flows, tools, guardrails | Clang (frontend) |
| **A-PXM** | Compile, optimize, schedule, execute | LLVM (backend) |
| **LLM backends** | Inference | CPU/GPU hardware |

## Strategic Role

AgentMate is useful when it accelerates adoption of the substrate. It should not duplicate the core runtime architecture or become the center of the platform thesis. Codex-on-APXM is the proof target; AgentMate is a supporting frontend whose strongest pieces may be reused selectively.

## What AgentMate Provides (That A-PXM Doesn't)

| Feature | AgentMate Crate | Description |
|---------|----------------|-------------|
| Agent builder | am-agents | `AgentBuilder` with fluent API, `DeepAgent`, `ReasoningAgent` |
| Tools | am-tools | Built-in tools (bash, read, write, search) + custom tool adapters |
| Sandbox | am-sandbox | OS-level isolation (Seatbelt on macOS, Landlock on Linux) |
| TUI | am-tui | Terminal UI with streaming, spinners, design system |
| RAG | am-rag | Retrieval-augmented generation pipeline |
| Documents | am-documents | Document parsing and text extraction |
| MCP | am-mcp | Model Context Protocol client + server |
| Skills | am-skills | Reusable agent skill packages |
| Config | am-config | Layered configuration + prompt templates |
| CLI | am-cli | Extensible CLI framework (chat, exec, models) |
| Macros | am-macros | `apxm_flow!` declarative workflow DSL |
| Python | am-py | PyO3 bridge — same runtime from Python |

## Quick Start

### Rust

```rust
use agentmate::AgentBuilder;

let agent = AgentBuilder::new()
    .name("coder")
    .model("deepseek-r1:14b")
    .with_standard_tools()
    .build()?;

let result = agent.run("Fix the bug").await?;
```

### Python

```python
import agentmate.graph as ag

@ag.compile(opt_level=2)
def research(g: ag.GraphRecorder, topic: str):
    r = g.ask("research", "Research: {0}")
    c = g.ask("critique", "Critique: {0}")
    s = g.wait_all("sync", r, c)
    report = g.ask("report", "Synthesize: {0}")
    s >> report
    return report

result = await research("quantum computing")
```

## Documents

| Document | Description |
|----------|-------------|
| [architecture.md](architecture.md) | How AgentMate layers on A-PXM (pipeline, tool flow, execution paths) |
| [integration.md](integration.md) | What APXM crates AgentMate depends on and how |
| [status.md](status.md) | Current implementation status and remaining gaps |

## Source

AgentMate lives at `$HOME/projects/agents/agentmate`. It depends on A-PXM crates via local path patches during development:

```toml
# From agentmate/Cargo.toml
[patch."https://github.com/randreshg/apxm.git"]
apxm-artifact = { path = "../apxm/crates/apxm-artifact" }
apxm-backends = { path = "../apxm/crates/apxm-backends" }
apxm-compiler = { path = "../apxm/crates/apxm-compiler" }
apxm-core     = { path = "../apxm/crates/apxm-core" }
apxm-graph    = { path = "../apxm/crates/apxm-graph" }
apxm-runtime  = { path = "../apxm/crates/apxm-runtime" }
apxm-tools    = { path = "../apxm/crates/apxm-tools" }
```
