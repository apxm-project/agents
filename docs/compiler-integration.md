# APXM Compiler Integration Guide

*How AIS source files and AgentMate both feed into the same compiler infrastructure.*

---

## The Shared Core

All frontends — whether `.ais` source files or AgentMate's Rust/Python builder APIs —
compile to the same `ApxmGraph` and execute through the same dataflow runtime.

```
┌─────────────────────────────────────────────────────────────────┐
│                        FRONTENDS                                │
│                                                                 │
│  .ais files          AgentMate Rust API    AgentMate Python API │
│  (human-written)     (WorkflowBuilder)     (FlowModule)         │
│       │                     │                     │             │
│       │ GraphGen             │ .build()            │ .to_graph() │
│       │ (C++ DSL parser)     │                     │             │
└───────┼─────────────────────┼─────────────────────┼─────────────┘
        │                     │                     │
        ▼                     ▼                     ▼
┌─────────────────────────────────────────────────────────────────┐
│                    ApxmGraph (in-memory)                        │
│              apxm-graph crate — the common IR                   │
└────────────────────────────┬────────────────────────────────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
              ▼              ▼              ▼
      graph-direct     MLIR pipeline    validate only
      (fast, no opt)   (O1/O2/O3)      (dekk apxm validate)
              │              │
              ▼              ▼
         Artifact         Artifact
       (.apxmobj)       (.apxmobj)
              │              │
              └──────┬───────┘
                     ▼
              Dataflow Runtime
              (apxm-runtime)
                     │
                     ▼
                  Result
```

---

## Integration Points by Frontend

### 1. AIS Source Files (`.ais`)

```
.ais file
    → Linker::compile_graph(path)
    → Compiler::load_graph(path)   [MLIR required]
        → apxm_parse_dsl_to_graph_json()  [GraphGen C++]
        → ApxmGraph::from_json()
    → Linker::compile_from_graph(graph, name)
        → ApxmGraph::to_execution_dag()
        → Artifact
    → Runtime::execute_artifact()
```

**CLI usage:**
```bash
dekk apxm execute examples/workflows/designer/brief_analyzer.ais
dekk apxm compile examples/workflows/designer/brief_analyzer.ais  # → .apxmobj
```

### 2. AgentMate Rust API

```rust
// AgentMate builds the graph in memory — no files written
let graph = WorkflowBuilder::new("research")
    .ask("research", "Research: {topic}")
    .think("synthesize", "Synthesize findings: {0}", budget_tokens: 2000)
    .build();  // → ApxmGraph

// Execute directly via Linker (in-memory, no files)
let result = linker.run_from_graph(graph, args, None, None).await?;

// Or compile to artifact for repeated use
let artifact = linker.compile_from_graph(graph, Some("research".into()))?;
linker.runtime.execute_artifact(artifact, args, None).await?;
```

**Key methods on `Linker`:**
- `compile_from_graph(graph: ApxmGraph, name: Option<String>) -> Artifact` — in-memory compile
- `run_from_graph(graph, args, emitter, session_dir) -> LinkResult` — in-memory execute

**Key method on `Compiler`:**
- `compile_graph(graph: &ApxmGraph) -> Module` — MLIR optimization pipeline

### 3. AgentMate Python API

```python
import agentmate.graph as ag

@ag.compile(opt_level=2)
def research(g: ag.GraphRecorder, topic: str):
    r = g.ask("research", "Research: {0}")
    c = g.think("critique", "Critique: {0}", budget_tokens=1000)
    s = g.wait_all("sync", r, c)
    report = g.ask("report", "Synthesize: {0}")
    s >> report
    return report

# Python produces ApxmGraph JSON, Rust side calls compile_from_graph
result = await research("quantum computing")
```

The PyO3 bridge (`am-py`) calls `Linker::run_from_graph` with the graph
constructed from `FlowModule.to_graph()`.

---

## The Rule: Never Write Files for Programmatic Use

**AgentMate should never serialize a graph to disk just to pass it to the compiler.**

❌ Wrong:
```rust
let json = graph.to_json()?;
std::fs::write("/tmp/graph.apxm", &json)?;
linker.run_graph(Path::new("/tmp/graph.apxm"), args).await?;
```

✅ Right:
```rust
linker.run_from_graph(graph, args, None, None).await?;
```

The `.ais` → file path exists only for **human authoring**. Programmatic frontends
use the in-memory APIs directly.

---

## Adding New Ops to the Compiler

When a new op is added (e.g., `NEGOTIATE`, `DELEGATE`):

1. **AIS DSL**: Add to `GraphGen::classifyCall()` in `GraphGen.cpp`
2. **MLIR**: Add `AIS_NegotiateOp` to `AISOps.td`, implement in `AISOps.cpp`
3. **Runtime**: Add handler in `crates/apxm-runtime/src/executor/handlers/`
4. **Validation**: Add required attributes to `validate_required_attributes()` in `validate.rs`
5. **AgentMate**: Add `g.negotiate(...)` to `WorkflowBuilder` and `GraphRecorder`

All five frontends (AIS, Rust builder, Python builder, MLIR path, validate) share
the same op definitions from `apxm-ais/src/operations/definitions.rs`.

---

## Format Summary

| Format | Who produces it | Who consumes it | Round-trip? |
|--------|----------------|----------------|-------------|
| `.ais` | Humans, AgentMate | `Compiler::load_graph()` | ✅ yes |
| `ApxmGraph` (in-memory) | WorkflowBuilder, FlowModule, GraphGen | `compile_from_graph()` | ✅ yes |
| `.air` | `Compiler::emit_air()` | Humans (debug only) | ❌ output only |
| `.apxmobj` | `compile_from_graph()`, MLIR pipeline | `Runtime::execute_artifact()` | ✅ yes |
