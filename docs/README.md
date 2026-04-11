# APXM Documentation

## Getting Started

New to APXM? **[Getting Started Guide](getting-started.md)** — install, configure, and run your first workflow.

## Architecture

APXM follows a strict layered architecture where **Core** is the single source of truth.
All downstream components (Compiler, Runtime, Backend) derive from Core definitions.

```
                         CORE
              (operations, attributes, events)
                  ┌──────┼──────┐
                  │      │      │
               Compiler  │   Backend
               (MLIR +   │   (LLMs, credentials,
                Python)  │    storage)
                  │   Runtime     Codegen
                  │   (executor,  (registry.rs →
                  │    scheduler)  Python + TS)
                  │      │
               Artifact  ACP
                  │      │
                Driver
           (orchestration)
               ┌──┴──┐
             CLI    GUI     Server
```

## Crate Documentation

Each crate has its own README as the primary documentation:

| Layer | Crate | README |
|-------|-------|--------|
| Core | `apxm-ais` | [`crates/core/apxm-ais/README.md`](../crates/core/apxm-ais/README.md) |
| Core | `apxm-core` | [`crates/core/apxm-core/README.md`](../crates/core/apxm-core/README.md) |
| Compiler | `apxm-compiler` | [`crates/compiler/apxm-compiler/README.md`](../crates/compiler/apxm-compiler/README.md) |
| Compiler | `apxm-frontend` | [`crates/compiler/apxm-frontend/README.md`](../crates/compiler/apxm-frontend/README.md) |
| Runtime | `apxm-runtime` | [`crates/runtime/apxm-runtime/README.md`](../crates/runtime/apxm-runtime/README.md) |
| Runtime | `apxm-backends` | [`crates/runtime/apxm-backends/README.md`](../crates/runtime/apxm-backends/README.md) |
| Runtime | `apxm-credentials` | [`crates/runtime/apxm-credentials/README.md`](../crates/runtime/apxm-credentials/README.md) |
| Orchestration | `apxm-driver` | [`crates/orchestration/apxm-driver/README.md`](../crates/orchestration/apxm-driver/README.md) |
| Orchestration | `apxm-acp` | [`crates/orchestration/apxm-acp/README.md`](../crates/orchestration/apxm-acp/README.md) |
| Orchestration | `apxm-artifact` | [`crates/orchestration/apxm-artifact/README.md`](../crates/orchestration/apxm-artifact/README.md) |
| Tools | `apxm-cli` | [`crates/tools/apxm-cli/README.md`](../crates/tools/apxm-cli/README.md) |
| Tools | `apxm-gui` | [`crates/tools/apxm-gui/README.md`](../crates/tools/apxm-gui/README.md) |
| Tools | `apxm-server` | [`crates/tools/apxm-server/README.md`](../crates/tools/apxm-server/README.md) |

## Theory

The A-PXM (Agent Program Execution Model) theory — why agent workflows need a formal
execution model, and how A-PXM separates Compute, Memory, State, Optimization, and Scheduling:

| Order | Document | What You Learn |
|-------|----------|----------------|
| 1 | [History](pxm/history.md) | The recurring pattern: ad-hoc wiring → opacity wall → formal model |
| 2 | [Foundations](pxm/foundations.md) | The agentic von Neumann bottleneck and the five separations |
| 3 | [AAM](pxm/aam.md) | Agent Abstract Machine — formal (B, G, C) state model |
| 4 | [AIS](pxm/ais.md) | Agent Instruction Set — typed operations, latency model, MLIR dialect |
| 5 | [Compute](pxm/compute.md) | Comparative analysis of compute across 7 PXMs |
| 6 | [Memory](pxm/memory.md) | Three-tier hierarchy (STM/LTM/Episodic) and first-class memory ops |
| 7 | [Scheduling](pxm/scheduling.md) | Token-counting dataflow with O(1) readiness detection |
| 8 | [Processes](pxm/processes.md) | Agent lifecycle, process/thread distinction |
| 9 | [Vision](pxm/vision.md) | The LLVM-for-agents vision |

## Cross-Cutting Specs

Documents that span multiple crates (not covered by any single crate README):

- [Optimization Passes](compiler/passes.md) — 15 passes, pipeline configuration, optimization levels, benchmark results

## Assessment & Strategy

- [Evaluation](evaluation/) — Audit results and production readiness
- [Strategy](strategy/) — Future integration plans (DSPy, vLLM)

## Key Principle

**Core defines. Everything else consumes.**

- `apxm-ais` defines the 41 AIS operations, 150+ attributes, and fundamental types
- `apxm-core` defines events, error codes, and protocol constants
- Compiler reads Core definitions to build MLIR dialect ops and generate TableGen
- Runtime reads Core definitions to dispatch operations
- Backend reads Core types for LLM request/response contracts
- Codegen pipeline auto-generates Python and TypeScript bindings from Core
- `apxm codegen frontend` → Python; `apxm codegen typescript` → TypeScript
