# AgentMate Architecture

AgentMate is a dual-language (Rust + Python) frontend SDK for A-PXM. All constructs lower to A-PXM IR, compile through MLIR, and execute via the A-PXM runtime.

## End-to-End Pipeline

```
┌──────────────────────────────────────────────────┐
│ User Frontends                                   │
├────────────────────────┬─────────────────────────┤
│ Rust                   │ Python                  │
│ AgentBuilder           │ FlowModule              │
│ WorkflowBuilder        │ GraphRecorder           │
│ apxm_flow! macro       │ @compile decorator      │
├────────────────────────┴─────────────────────────┤
│                ApxmGraph (JSON IR)               │
│        nodes, edges, parameters, metadata        │
├──────────────────────────────────────────────────┤
│ A-PXM Compiler                                   │
│ to_mlir → optimization passes → artifact         │
├──────────────────────────────────────────────────┤
│ A-PXM Runtime                                    │
│ dataflow scheduler → LLM backends + capabilities │
└──────────────────────────────────────────────────┘
```

## Workspace Crates

13 crates organized by function:

### Core
| Crate | Lines | Description |
|-------|-------|-------------|
| am-core | — | Core types, LLM provider definitions, shared constants |
| am-config | — | Layered configuration + prompt templates |
| am-macros | — | `apxm_flow!` declarative workflow DSL (proc macro) |

### Agent Layer
| Crate | Lines | Description |
|-------|-------|-------------|
| am-agents | — | Agent trait, AgentBuilder, DeepAgent, ReasoningAgent, Supervisor, Council |
| am-skills | — | Reusable skill packages (composable agent capabilities) |

### Tools & Security
| Crate | Lines | Description |
|-------|-------|-------------|
| am-tools | — | Tool definition, registration, built-ins (bash, read, write, search) |
| am-sandbox | — | OS-level isolation (Seatbelt macOS, Landlock Linux, bubblewrap) |

### Data & Retrieval
| Crate | Lines | Description |
|-------|-------|-------------|
| am-rag | — | Retrieval-augmented generation pipeline |
| am-documents | — | Document parsing and text extraction |
| am-mcp | — | Model Context Protocol client + server |

### User Interface
| Crate | Lines | Description |
|-------|-------|-------------|
| am-cli | — | CLI framework (chat, exec, models commands) |
| am-tui | — | Terminal UI with streaming, spinners, design system (ratatui) |

### Python Bridge
| Crate | Lines | Description |
|-------|-------|-------------|
| am-py | — | PyO3 bindings — same runtime from Python |

## Agent Types

AgentMate provides multiple agent abstractions:

| Agent Type | Use Case | Key Feature |
|-----------|----------|-------------|
| **Base Agent** | General-purpose | Configurable via AgentBuilder |
| **Deep Agent** | Extended reasoning | Multi-step thinking with tool use |
| **Reasoning Agent** | Structured analysis | REASON op with typed output schemas |
| **Supervisor** | Multi-agent coordinator | Routes tasks to specialist agents |
| **Council** | Consensus decisions | Multiple agents debate and vote |

## APXM Integration Points

AgentMate depends on 7 APXM crates:

| APXM Crate | Used For |
|------------|----------|
| apxm-graph | ApxmGraph IR construction and validation |
| apxm-compiler | Graph → MLIR → optimization passes → artifact |
| apxm-runtime | Dataflow scheduler, AAM state, execution engine |
| apxm-core | Shared types (AISOperationType, Plan, constants) |
| apxm-artifact | Compiled artifact format (read/write) |
| apxm-backends | LLM provider adapters (OpenAI, Anthropic, Ollama) |
| apxm-tools | Built-in tool implementations |

## PyTorch Analogy

| PyTorch | AgentMate + A-PXM |
|---------|-------------------|
| `nn.Module` | `FlowModule` (Python) / `WorkflowBuilder` (Rust) |
| `forward()` | `define(g)` graph construction |
| Tensor dataflow edges | Graph edges + dependency types |
| `torch.compile()` | `compile_graph()` + MLIR pipeline |
| Backend kernels | Capabilities + LLM handlers |
| Eager vs. compiled | Direct execution vs. MLIR-optimized artifact |

## Tool Flow

```
ASK node requests tool call
    │
    ├── Built-in tool (bash/read/write/search_web)
    │   └── apxm_tools::register_standard_tools()
    │
    └── Custom tool
        └── ToolCapabilityAdapter(Tool)
            └── Runtime capability registry
```

Built-in tools are native APXM capability executors. Custom tools are bridged via `ToolCapabilityAdapter` and exposed with capability metadata and JSON schema.
