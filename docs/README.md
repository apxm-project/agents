# APXM Documentation

## What APXM is

APXM (**A**gent **P**rogram e**X**ecution **M**odel) treats an agent graph the
way a programming language treats a function: the author describes *what*, and the
system decides *how* to run it. You write a graph of agent operations in Python; APXM
lowers it to MLIR, optimizes it, and executes the result deterministically across LLM
backends, Python tools, and sub-agents.

This documentation is the conceptual entry point. For installable, runnable code,
see [`examples/python/`](../examples/python/). For the live API surface, run
`dekk apxm ops list`. The crate-level READMEs under `crates/*/apxm-*/README.md`
document the implementation details and stay close to the code.

## The Big Picture

```
        ┌──────────────────────────────────────────────────────┐
        │                Author (Python frontend)              │
        │   @compile, Agent(), @tool, GraphRecorder            │
        └──────────────────────────┬───────────────────────────┘
                                   │  to_air()
                                   ▼
                     ┌──────────────────────────┐
                     │   AIR text  (.air)       │  human-readable
                     └─────────────┬────────────┘  graph IR
                                   │  parse + lower
                                   ▼
                     ┌──────────────────────────┐
                     │   MLIR module            │  ais.* dialect ops
                     │   (apxm-compiler)        │
                     └─────────────┬────────────┘
                                   │  optimization pipeline
                                   ▼   (~12 MLIR passes + 4 Rust)
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

The same artifact that runs locally can be shipped to a server, replayed for
debugging, or dispatched against a different backend without recompiling.

## Crate Layout

APXM is organized in tiers — each layer depends only on layers above it.

```
core    →  apxm-core, apxm-ais          (contracts in apxm-core; authoring/codegen specs in apxm-ais)
compiler→  apxm-compiler, apxm-frontend (AIR → MLIR → .apxmobj)
runtime →  apxm-runtime, apxm-backends, (execution, LLM I/O, secrets)
           apxm-credentials
orchestr→  apxm-driver, apxm-acp,       (CLI/library glue, ACP protocol,
           apxm-artifact                 artifact load/save)
tools   →  apxm-cli, apxm-server,       (binary, HTTP API, browser UI)
           apxm-gui
```

Each crate has a README under `crates/<tier>/<name>/README.md` describing what it
owns and how it composes with its neighbors.

## Core Concepts

The **Process Algebra of Minds** (PAM, the "M" in APXM) is the formal model. These
docs are the order to read them in:

| # | Doc                                | What you learn                                                     |
|---|------------------------------------|--------------------------------------------------------------------|
| 1 | [history](pxm/history.md)          | Why every domain hits the same "ad-hoc wiring → opacity wall" cycle |
| 2 | [foundations](pxm/foundations.md)  | The agentic von-Neumann bottleneck and PAM's five separations       |
| 3 | [aam](pxm/aam.md)                  | Agent Abstract Machine — the formal `(B, G, C)` state model         |
| 4 | [ais](pxm/ais.md)                  | Agent Instruction Set — typed operations, MLIR dialect              |
| 5 | [compute](pxm/compute.md)          | How seven different PXMs treat compute (and what APXM steals)       |
| 6 | [memory](pxm/memory.md)            | The STM / LTM / Episodic tiers and first-class memory ops           |
| 7 | [scheduling](pxm/scheduling.md)    | Token-counting dataflow with O(1) readiness detection               |
| 8 | [processes](pxm/processes.md)      | Agent lifecycle, the process/thread distinction                     |
| 9 | [vision](pxm/vision.md)            | The "LLVM for agents" thesis                                        |

## The Compiler

The compiler turns AIR into a runnable artifact through a deterministic optimization
pipeline. It runs as MLIR transforms (the `ais` dialect) plus a few Rust-side passes
that do bookkeeping the MLIR side can't easily express, such as tool binding.
Backend-specific behavior stays in backend adapters; the compiler emits typed
graph metadata and optimization hints, not vLLM-specific runtime policy.

→ [compiler/pipeline.md](compiler/pipeline.md) — pipeline diagram and pass-by-pass
purpose. The live ordering is in
`crates/compiler/apxm-compiler/src/passes/pipeline.rs`.

## Design Notes

Design notes capture context for areas that are not part of the runnable API
unless the note explicitly says they are implemented.

- [design/guardrails_handoffs.md](design/guardrails_handoffs.md) — input/output
  guardrails and inter-agent handoffs as first-class AIR constructs

## Backend Guides

- [backends/vllm.md](backends/vllm.md) — Dekk-first setup, registration, and
  metrics guidance for the repo-local graph-aware vLLM fork

## Trying It Out

```
dekk apxm doctor          # verify environment
dekk apxm ops list        # browse the live AIS surface
dekk apxm execute …       # run an .air graph end-to-end
```

Runnable demos live in [`examples/python/`](../examples/python/). The
[`README.md`](../README.md) at the repo root has install instructions.

## Key Principle

**Core defines. Everything else consumes.** `apxm-core` is the downstream contract
crate for shared operations, attributes, events, and error codes. `apxm-ais`
remains the authoring/codegen source that feeds those shared contracts and compiler
generation paths. The compiler, runtime, codegen, and bindings should consume the
shared `apxm-core` surface unless they are explicitly participating in authoring or
generation. That invariant is what lets the same `.apxmobj` artifact run in any
APXM environment.
