# Architecture

A-PXM is composed of a compiler and a runtime. The compiler transforms AIS
graphs into optimized execution plans via MLIR. The runtime executes them using
a dataflow scheduler that automatically extracts parallelism from dependencies.

## Pipeline

```
 Graph JSON ──▶ Compiler ──▶ .apxmobj Artifact ──▶ Runtime ──▶ Results
                  │                                    │
            lower to MLIR                        dataflow scheduler
            optimize (fuse, CSE, DCE)            token-based firing
            emit binary artifact                 LLM / Tool / Memory executors
```

## Sub-documentation

### Theory

| Document | Covers |
|----------|--------|
| [PXM Foundations](../pxm/foundations.md) | Dataflow semantics, token model, codelet/threaded execution |

### Compiler

| Document | Covers |
|----------|--------|
| [Compiler Overview](compiler/overview.md) | Frontend normalization, MLIR lowering, emit pipeline |
| [Optimization Passes](compiler/optimization-passes.md) | FuseAskOps, CSE, dead-code elimination |
| [Artifact Format](compiler/artifact-format.md) | `.apxmobj` binary layout, versioning, schema |

### Runtime

| Document | Covers |
|----------|--------|
| [Dataflow Scheduler](runtime/dataflow-scheduler.md) | Token counters, O(1) readiness, parallel dispatch |
| [Memory Hierarchy](runtime/memory-hierarchy.md) | Three-tier memory (node / agent / global) |
| [Tasks](runtime/tasks.md) | Task abstraction and grouping |
| [Multi-Agent](runtime/multi-agent.md) | Agent model, FlowRegistry, FLOW_CALL |
| Hierarchical AAM *(partial — ScopeRegistry, WorkspaceManager, GoalTree implemented)* | Nested agent scoping and delegation |
| [Pluggable Sandbox](pluggable-sandbox-design.md) | SandboxBackend trait, IsolationLevel, SecurityManifest |
| [Host Integration](host-integration-guide.md) | Codex & Gemini integration, AAM deep dive |

### AIS Operations

The `ais/` directory documents every operation category:
[LLM](ais/llm-ops.md) |
[Memory](ais/memory-ops.md) |
[Tool](ais/tool-ops.md) |
[Control-flow](ais/control-flow.md) |
[Sync](ais/sync-ops.md) |
[Coordination](ais/coordination-ops.md) |
[Communication](ais/communication.md)

### Contracts & Internals

| Document | Covers |
|----------|--------|
| [Contracts](internals/contracts.md) | Op-kind indices, wire format, sync rules |
| [Graph JSON Contract](internals/graph-json-contract.md) | Canonical graph schema, edge types, parameters |
