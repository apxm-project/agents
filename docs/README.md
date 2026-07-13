# APXM Documentation

> **Start here for the practical framing:** [VISION.md](../VISION.md) — APXM as a library
> system for agent skills. This documentation set covers the model and the implementation
> behind that framing.

## What APXM is

APXM (**A**gent **P**rogram e**X**ecution **M**odel) treats an agent workflow the
way a programming language treats a function: the author describes *what*, and the
system decides *how* to run it. You author agent operations in Rust, Python,
or TypeScript; each frontend records the compiler-owned `FrontendGraph` and
delegates canonical AIR printing to Rust. Direct `.air` is also a supported
source format. APXM then lowers AIR to MLIR, optimizes it, and executes the
artifact across backends, tools, and sub-agents.

This documentation is the conceptual entry point. For installable, runnable code,
see [`examples/python/`](../examples/python/). For the live API surface, run
`dekk agents ops list`. The crate-level READMEs under `crates/*/apxm-*/README.md`
document the implementation details and stay close to the code.

## The Big Picture

```
        ┌──────────────────────────────────────────────────────┐
        │             Authors: Rust, Python, TypeScript        │
        │ AirModuleBuilder | GraphRecorder | GraphBuilder       │
        └──────────────────────────┬───────────────────────────┘
                                   │  FrontendGraph DTO
                                   ▼
                     ┌──────────────────────────┐
                     │ Rust AIR validator/printer │  canonical owner
                     └─────────────┬────────────┘
                                   │  AIR text (.air)
                                   ▼
                     ┌──────────────────────────┐
                     │   MLIR module            │  ais.* dialect ops
                     └─────────────┬────────────┘
                                   │  resolved optimization pipeline
                                   ▼
                     ┌──────────────────────────┐
                     │   .apxmobj artifact      │  serialized DAG +
                     │   (apxm-artifact)        │  metadata
                     └─────────────┬────────────┘
                                   │  load + dispatch
                                   ▼
        ┌──────────────────────────────────────────────────────┐
        │             Runtime  (apxm-runtime)                  │
        │     scheduler · executor · memory tiers · AAM        │
        └──┬─────────────────┬────────────────┬────────────────┘
           ▼                 ▼                ▼
        ┌──────┐         ┌────────┐       ┌─────────┐
        │ LLMs │         │ Python │       │ Sub-    │
        │      │         │ tools  │       │ agents  │
        └──────┘         └────────┘       └─────────┘
        (apxm-backends)  (apxm-tools)     (HANDOFF /
                                           COMMUNICATE)
```

The same artifact that runs locally can be shipped to a server, inspected
through session traces/timelines for debugging, or dispatched against a
different backend without recompiling. Full scheduler replay requires future
scheduler snapshots.

## Crate Layout

APXM is organized in tiers — each layer depends only on layers above it.

```
machine →  apxm-core, apxm-ais          (contracts in apxm-core; authoring/codegen specs in apxm-ais)
compiler→  apxm-compiler, Python frontend, TypeScript frontend
           (FrontendGraph → AIR → MLIR → .apxmobj)
runtime →  apxm-runtime, apxm-backends, (execution, LLM I/O,
           apxm-backend-registry         backend registry; refs only)
orchestr→  apxm-driver, apxm-acp,       (driver/library glue, ACP protocol,
           apxm-artifact                 artifact load/save)
tools   →  apxm-cli, apxm             (developer CLI incl. generated apxm-server
                                        client as a module, facade)
```

Each crate has a README under `crates/<tier>/<name>/README.md` describing what it
owns and how it composes with its neighbors.

## Agents

- [Create your first agent](agents/first-agent.md) — minimal
  folder contract (`agent.toml`, `prompts/`, `python/`, `capabilities/`) and
  validation workflow

## Core Concepts

The runtime is modeled around an abstract machine and a typed instruction set.
Read in this order:

| # | Doc                                | What you learn                                                     |
|---|------------------------------------|--------------------------------------------------------------------|
| 1 | [aam](pxm/aam.md)                  | Agent Abstract Machine — the formal `(B, G, C)` state model         |
| 2 | [ais](pxm/ais.md)                  | Agent Instruction Set — typed operations, MLIR dialect              |
| 3 | [memory](pxm/memory.md)            | The STM / LTM / Episodic tiers and first-class memory ops           |
| 4 | [processes](pxm/processes.md)      | Agent lifecycle, the process/thread distinction                     |
| 5 | [agent topology boundary](agent-topology-boundary.md) | Hard rule: org topology is policy outside APXM runtime |

## The Compiler

The compiler turns AIR into a runnable artifact through a deterministic optimization
pipeline. It runs as MLIR transforms (the `ais` dialect) plus a few Rust-side passes
that do bookkeeping the MLIR side can't easily express, such as tool binding.
Backend-specific behavior stays in backend adapters; the compiler emits typed
workflow metadata and optimization hints, not vLLM-specific runtime policy.

→ [compiler/pipeline.md](compiler/pipeline.md) — pipeline diagram and pass-by-pass
purpose. The live ordering is in
`crates/compiler/pipeline/src/passes/pipeline.rs`.

## Backend Guides

- [backends/vllm.md](backends/vllm.md) — Dekk-first Docker/image-store setup,
  registration, and metrics guidance for the graph-aware APXM-vLLM fork
- [backends/model-zoo-quickstart.md](backends/model-zoo-quickstart.md) — 15-minute
  walkthrough from shared model storage to a working zoo
- [backends/model-zoo.md](backends/model-zoo.md) — zoo operator reference
- [backends/storage-layout.md](backends/storage-layout.md) — where APXM puts
  large files (HF cache, model roots, image store, evaluation artifacts)
  and the supported relocation procedure

## Trying It Out

```
dekk agents doctor          # verify environment
dekk agents ops list        # browse the live AIS surface
dekk agents execute …       # run an .air workflow end-to-end
```

Runnable demos live in [`examples/python/`](../examples/python/). The
[`README.md`](../README.md) at the repo root has install instructions.

## Server Concurrency

`apxm-server` has two independent concurrency layers:

- `[server.inference].max_concurrent` caps expensive server-wide execution work.
  The default is `16`, and `APXM_SERVER_MAX_INFERENCE` overrides it for the
  process.
- `[server.runtime]` controls the per-execution scheduler, including
  `max_concurrency`, `max_inflight`, `llm_inflight`, and
  `max_parallel_tool_calls`.

For webhook and channel traffic, same-conversation ordering is enforced by the
runtime session lane guard. Calls with the same `session_id` serialize; different
sessions can run in parallel until they hit the server inference limit and the
originating apxm-os agent's `max_concurrency`.

## Key Principle

**Core defines. Everything else consumes.** `apxm-core` is the downstream contract
crate for shared operations, attributes, events, and error codes. `apxm-ais`
remains the authoring/codegen source that feeds those shared contracts and compiler
generation paths. The compiler, runtime, codegen, and bindings should consume the
shared `apxm-core` surface unless they are explicitly participating in authoring or
generation. That invariant is what lets the same `.apxmobj` artifact run in any
APXM environment.

**Topology is policy outside the runtime.** Agent hierarchy and reachability are
authored by higher layers and enforced before execution is lowered into concrete
APXM operations. The runtime executes admitted workflows; it does not interpret
company/org relationships. See [agent topology boundary](agent-topology-boundary.md).
