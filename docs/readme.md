# APXM Documentation

**APXM (Agent Programming eXecution Model)** is a compiler and dataflow runtime for AI agent workflows. Graphs of AIS (Agent Instruction Set) operations compile through MLIR to optimized artifacts, then execute on a parallel scheduler with pluggable LLM backends. Like LLVM provides shared infrastructure for programming languages, APXM provides shared infrastructure for AI agents -- bringing compiler optimizations, runtime efficiency, and reproducible execution to LLM orchestration.

## Quick Start

New to APXM? Begin here:

1. [getting-started/installation.md](getting-started/installation.md) -- Install dependencies and build APXM
2. [getting-started/first-graph.md](getting-started/first-graph.md) -- Write, compile, and execute your first workflow

Then explore [guides/](guides/) for task-oriented howtos.

## Reading Paths

### New users

Set up your environment, build your first graph, then learn the tools.

[getting-started/](getting-started/) -> [guides/](guides/) -> [integrations/](integrations/)

### Optimizing workflows

Understand the compiler passes, tune for your workload, measure the results.

[optimization/](optimization/) -> [integrations/](integrations/) -> [benchmarks/](benchmarks/)

### Understanding the theory

Learn the formal execution model and the design decisions behind it.

[pxm/](pxm/) -> [design/](design/)

### Contributing

Navigate the codebase internals and authoritative reference material.

[implementation/](implementation/) -> [reference/](reference/)

### Planning

Roadmap, investigations, and strategic direction.

[strategy/](strategy/) -> [research/](research/)

## Directory Map

| Path | Contents |
|------|----------|
| [architecture.md](architecture.md) | System overview: five-layer architecture (frontend, compiler, runtime, backend, optimization) |
| [changelog.md](changelog.md) | Release notes and migration guides |
| [getting-started/](getting-started/) | Installation and first-graph onboarding for new users |
| [guides/](guides/) | Task-oriented howtos: backends, multi-agent, multi-model, caching, debugging, self-hosted workflows |
| [optimization/](optimization/) | Compiler optimization levels, targets, decision matrix, and all optimization passes |
| [integrations/](integrations/) | External system integrations (vLLM graph-aware inference, DSPy prompt optimization) |
| [reference/](reference/) | Authoritative look-up: `~/.apxm/config.toml` format, graph JSON contract |
| [pxm/](pxm/) | Theory and foundations: Agent Abstract Machine, Agent Instruction Set, scheduling, memory |
| [implementation/](implementation/) | Developer internals: compiler pipeline, runtime scheduler, per-op reference |
| [design/](design/) | Architecture decision records and prior-art analysis |
| [strategy/](strategy/) | Roadmap, planning, and strategic direction |
| [research/](research/) | Investigations, token estimation, production heuristics, DSPy internals |
| [benchmarks/](benchmarks/) | Measurement methodology and results (vLLM, compiler passes, MemoCache, DSPy) |
| [archive/](archive/) | Historical snapshots of superseded documents |
