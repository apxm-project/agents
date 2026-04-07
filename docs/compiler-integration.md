# APXM Compiler Integration Guide

*How Python, `.air`, and JSON authoring flow into the same compiler infrastructure.*

---

## The Shared Core

All supported frontends compile to the same `ApxmGraph` and execute through the
same dataflow runtime.

```
┌─────────────────────────────────────────────────────────────────┐
│                        FRONTENDS                                │
│                                                                 │
│  JSON graphs         Rust API              Python API           │
│  (CLI / services)    (WorkflowBuilder)     (`apxm.graph`)       │
│       │                     │                     │             │
│       │ from_json()         │ .build()            │ .to_air()   │
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

### 1. JSON Graph Files (`.json`)

```
.json file
    → Linker::compile_graph(path)
    → Compiler::load_graph(path)
        → ApxmGraph::from_json()
    → Linker::compile_from_graph(graph, name)
        → ApxmGraph::to_execution_dag()
        → Artifact
    → Runtime::execute_artifact()
```

**CLI usage:**
```bash
dekk apxm validate workflow.json
dekk apxm compile workflow.json -o workflow.apxmobj
dekk apxm execute workflow.json
```

JSON remains supported as a compatibility path. The canonical file handoff from the Python
frontend is now `.air`.

### 2. Rust API

```rust
// APXM Frontend builds the graph in memory — no files written
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

### 3. Python API

```python
import apxm.graph as ag

@ag.compile(opt_level=2)
def research(g: ag.GraphRecorder, topic: str):
    r = g.ask("research", "Research: {0}")
    c = g.think("critique", "Critique: {0}", budget_tokens=1000)
    s = g.wait_all("sync", r, c)
    report = g.ask("report", "Synthesize: {0}")
    s >> report
    return report

# Python produces canonical .air text for file-based handoff
print(research._graph.to_air())

# The programmatic bridge still passes the in-memory graph to Rust
result = await research("quantum computing")
```

The Python bridge calls `Linker::run_from_graph` with the graph
constructed from `FlowModule.to_graph()`.

---

## The Rule: Never Write Files for Programmatic Use

**Programmatic frontends should not serialize a graph to disk unless they are
explicitly handing it to a file-based CLI or external tool.**

❌ Wrong:
```rust
let json = graph.to_json()?;
std::fs::write("/tmp/graph.json", &json)?;
dekk_apxm_execute("/tmp/graph.json");
```

✅ Right:
```rust
linker.run_from_graph(graph, args, None, None).await?;
```

---

## Adding New Ops to the Compiler

When a new op is added (e.g., `NEGOTIATE`, `DELEGATE`):

1. **MLIR**: Add `AIS_NegotiateOp` to `AISOps.td`, implement in `AISOps.cpp`
2. **Runtime**: Add handler in `crates/apxm-runtime/src/executor/handlers/`
3. **Validation**: Add required attributes to `validate_required_attributes()` in `validate.rs`
4. **Rust frontend**: Add the builder helper to `WorkflowBuilder`
5. **Python frontend**: Add `g.negotiate(...)` to `GraphRecorder`

All frontends (JSON, Rust builder, Python builder, MLIR path, validate) share
the same op definitions from `apxm-ais/src/operations/definitions.rs`.

---

## Format Summary

| Format | Who produces it | Who consumes it | Round-trip? |
|--------|----------------|----------------|-------------|
| `.air` | Python frontend, humans | Rust `.air` parser / compiler input | ✅ yes |
| `.json` | Services, humans, compatibility tooling | `Compiler::load_graph()` | ✅ yes |
| `ApxmGraph` (in-memory) | WorkflowBuilder, FlowModule | `compile_from_graph()` | ✅ yes |
| `.apxmobj` | `compile_from_graph()`, MLIR pipeline | `Runtime::execute_artifact()` | ✅ yes |
