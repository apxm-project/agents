# Implementation Architecture

A-PXM is composed of a compiler and a runtime. The compiler transforms AIS graphs into optimized execution plans via MLIR. The runtime executes them using a dataflow scheduler that automatically extracts parallelism from dependencies.

## Pipeline

```
 Graph JSON --> Compiler --> .apxmobj Artifact --> Runtime --> Results
                  |                                    |
            lower to MLIR                        dataflow scheduler
            optimize (fuse, CSE, DCE)            token-based firing
            emit binary artifact                 LLM / Tool / Memory executors
```

## Sub-documentation

### Theory

| Document | Covers |
|----------|--------|
| [PXM Foundations](../pxm/foundations.md) | Dataflow semantics, token model, codelet/threaded execution |
| [Agent Abstract Machine](../pxm/aam.md) | Formal AAM definition, state model, memory hierarchy |
| [Hierarchical AAM](../design/hierarchical-aam.md) | Nested agent scoping diagrams, file-tree-as-AAM design |

### Compiler

| Document | Covers |
|----------|--------|
| [Compiler Overview](compiler/overview.md) | Frontend normalization, MLIR lowering, emit pipeline |
| [Optimization Passes](../optimization/passes.md) | FuseAskOps, CSE, dead-code elimination, and all other passes |
| [Artifact Format](compiler/artifact-format.md) | `.apxmobj` binary layout, versioning, schema |
| [Compiler Integration](compiler-integration.md) | How JSON, Rust, and Python frontends share the compiler |

### Runtime

| Document | Covers |
|----------|--------|
| [Dataflow Scheduler](runtime/dataflow-scheduler.md) | Token counters, O(1) readiness, parallel dispatch |
| [Memory Hierarchy](runtime/memory-hierarchy.md) | Three-tier memory (node / agent / global) |
| [Tasks](runtime/tasks.md) | Task abstraction and grouping |
| [Multi-Agent](runtime/multi-agent.md) | Agent model, FlowRegistry, FLOW_CALL |
| [Observability](runtime/observability.md) | Event system, MetricsCollector, tracing, --emit-metrics, --emit-session |
| [Sessions](runtime/sessions.md) | AAM checkpoints, SessionManager, ProcessTable, ACP lifecycle |
| [Host Integration](host-integration.md) | Codex and Gemini integration, pluggable backend interfaces |

### AIS Operations

The `ais/` directory documents every operation category:
[LLM](ais/llm-ops.md) |
[Memory](ais/memory-ops.md) |
[Tool](ais/tool-ops.md) |
[Control-flow](ais/control-flow.md) |
[Sync](ais/sync-ops.md) |
[Utility and Error](ais/utility-ops.md) |
[Communication](ais/communication.md)

### Contracts & Internals

| Document | Covers |
|----------|--------|
| [Contracts](internals/contracts.md) | Op-kind indices, wire format, sync rules |
| [Graph JSON Format](../reference/graph-format.md) | Canonical graph schema, edge types, parameters |

### Guides

| Document | Covers |
|----------|--------|
| [Debugging](../guides/debugging.md) | Tracing flags, RUST_LOG filters, session replay |
| [Backends](../guides/backends.md) | Backend configuration and management |
| [Multi-Agent](../guides/multi-agent.md) | Multi-agent workflow patterns |
