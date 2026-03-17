# AgentMate ↔ A-PXM Integration

How AgentMate depends on and uses A-PXM crates.

## Dependency Graph

```
agentmate (am-agents)
    ├── apxm-graph      ← Graph IR construction
    ├── apxm-compiler   ← MLIR compilation pipeline
    ├── apxm-runtime    ← Dataflow execution engine
    ├── apxm-core       ← Shared types + constants
    ├── apxm-artifact   ← Compiled artifact format
    ├── apxm-backends   ← LLM provider adapters
    └── apxm-tools      ← Built-in tool implementations
```

## Local Development Setup

During development, AgentMate patches APXM git deps with local paths:

```toml
# agentmate/Cargo.toml
[patch."https://github.com/randreshg/apxm.git"]
apxm-artifact = { path = "../apxm/crates/apxm-artifact" }
apxm-backends = { path = "../apxm/crates/apxm-backends" }
apxm-compiler = { path = "../apxm/crates/apxm-compiler" }
apxm-core     = { path = "../apxm/crates/apxm-core" }
apxm-graph    = { path = "../apxm/crates/apxm-graph" }
apxm-runtime  = { path = "../apxm/crates/apxm-runtime" }
apxm-tools    = { path = "../apxm/crates/apxm-tools" }
```

This means changes to A-PXM crates are immediately visible to AgentMate during development.

## Integration Points

### 1. Graph Construction → apxm-graph

AgentMate's `WorkflowBuilder` and Python `FlowModule` both emit `ApxmGraph` JSON:

```rust
// Rust
let graph = WorkflowBuilder::new("research")
    .ask("research", "Research: {0}")
    .ask("critique", "Critique: {0}")
    .wait_all("sync", &["research", "critique"])
    .build();
// graph is an ApxmGraph
```

```python
# Python
class Research(FlowModule):
    def define(self, g):
        r = g.ask("research", "Research: {0}")
        c = g.ask("critique", "Critique: {0}")
        s = g.wait_all("sync", r, c)
        return s
# .to_graph() produces ApxmGraph JSON
```

### 2. Compilation → apxm-compiler

AgentMate calls `compile_graph()` which:
1. Lowers `ApxmGraph` to AIS MLIR dialect
2. Runs optimization passes (Normalize → BuildPrompt → UnconsumedValueWarning → Scheduling → FuseAskOps → Canonicalizer → CSE → SymbolDCE)
3. Emits compiled artifact

```rust
let workflow = WorkflowBuilder::new("research")
    .compile(OptLevel::O2)  // calls apxm-compiler
    .await?;
```

### 3. Execution → apxm-runtime

The compiled artifact executes on A-PXM's dataflow scheduler:
- Token-counting scheduler with O(1) readiness detection
- 7.5μs per-node scheduling overhead
- Automatic parallelism from graph topology

### 4. LLM Calls → apxm-backends

AgentMate configures LLM providers (Ollama, OpenAI, Anthropic, OpenRouter). At runtime, A-PXM's `ASK`/`THINK`/`REASON` handlers call the configured backend.

### 5. Tool Execution → apxm-tools + am-tools

Built-in tools (bash, read, write, search) are registered via `apxm-tools`. Custom tools from AgentMate are bridged via `ToolCapabilityAdapter`.

## AIS Operations Used by AgentMate

The Python `GraphRecorder` exposes all 32 AIS operations. The Rust `WorkflowBuilder` exposes 15 of 32 (the most commonly used subset):

### Rust WorkflowBuilder (15 ops)

| Op | Rust Method | Purpose |
|----|-------------|---------|
| ASK | `.ask()` / `.ask_with()` | LLM call with tool iteration |
| THINK | `.think()` / `.think_with()` | Extended reasoning |
| REASON | `.reason()` / `.reason_with()` | Structured output |
| QMEM | `.query_memory()` | Read memory |
| UMEM | `.update_memory()` | Write memory |
| INV | `.invoke()` | Tool invocation |
| BRANCH_ON_VALUE | `.branch()` | Conditional |
| SWITCH | `.switch_()` | Multi-way dispatch |
| WAIT_ALL | `.wait_all()` | Synchronization barrier |
| MERGE | `.merge()` | Merge token streams |
| FENCE | `.fence()` | Ordering barrier |
| PLAN | `.plan()` / `.plan_with()` | Goal decomposition |
| REFLECT | `.reflect()` / `.reflect_with()` | Self-evaluation |
| VERIFY | `.verify()` / `.verify_with()` | Verification |
| CONST_STR | `.const_()` | Constant value |

### Python GraphRecorder (all 32 ops)

The Python API additionally exposes: `execute` (EXC), `print_` (PRINT), `jump` (JUMP), `loop_start` (LOOP_START), `loop_end` (LOOP_END), `return_` (RETURN), `flow_call` (FLOW_CALL), `try_catch` (TRY_CATCH), `err` (ERR), `communicate` (COMMUNICATE), `update_goal` (UPDATE_GOAL), `guard` (GUARD), `claim` (CLAIM), `pause` (PAUSE), `resume` (RESUME), `agent` (AGENT), `yield_` (YIELD).

### Higher-Level Helpers (both Rust and Python)

Both APIs also provide composite helpers built on top of AIS ops: `handoff`, `handoff_when`, `checkpoint`, `input_guardrail`, `output_guardrail`, and (Rust only) `llm_guardrail`.
