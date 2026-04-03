# A-PXM Documentation

A-PXM is a Program Execution Model. Like von Neumann defined how hardware executes sequential programs, A-PXM defines how systems execute agent programs. It provides the ISA contract (AIS), a compiler (MLIR-based optimization), and a runtime that commits to the Agent Abstract Machine (Beliefs, Goals, Capabilities). This is the "LLVM for agents": shared infrastructure where improvements benefit every agent built on it.

## Reading Path

If you are new to A-PXM, start here:

1. [**history.md**](pxm/history.md) -- How computing history repeats at the agentic scale.
2. [**foundations.md**](pxm/foundations.md) -- The agentic von Neumann bottleneck and the five separations.
3. [**aam.md**](pxm/aam.md) -- The Agent Abstract Machine (Beliefs, Goals, Capabilities).
4. [**ais.md**](pxm/ais.md) -- The typed operations that agents actually execute.
5. [**vision.md**](pxm/vision.md) -- The LLVM-for-agents vision and where this is going.
6. [**optimizations.md**](advantages/optimizations.md) -- What you get: compiler and runtime optimizations.
7. [**getting-started.md**](guides/getting-started.md) -- Install, build, and run your first graph.

---

## Documentation Map

### [`pxm/`](pxm/) -- The Model *(for anyone)*

What A-PXM *is* -- the formal execution model, its foundations, and where it is going.

- [**history.md**](pxm/history.md) -- How computing history repeats: from von Neumann to agents
- [**foundations.md**](pxm/foundations.md) -- The agentic von Neumann bottleneck and the five separations
- [**aam.md**](pxm/aam.md) -- Agent Abstract Machine: Beliefs, Goals, Capabilities
- [**ais.md**](pxm/ais.md) -- Agent Instruction Set: typed operations across multiple categories
- [**compute.md**](pxm/compute.md), [**memory.md**](pxm/memory.md), [**scheduling.md**](pxm/scheduling.md) -- Deep dives on each separation
- [**vision.md**](pxm/vision.md) -- The LLVM-for-agents vision: shared infrastructure for all agents

### [`implementation/`](implementation/) -- The System *(for contributors)*

Compiler, runtime, wire-level contracts, and TODOs toward the vision.

- [**architecture.md**](implementation/architecture.md) -- End-to-end system architecture (index)
- [**TODO.md**](implementation/TODO.md) -- Master TODO: AIS gaps, substrate gaps, AAM gaps
- [`ais/`](implementation/ais/) -- Per-category operation reference (LLM, Memory, Tool, Control, Sync, Communication, Coordination)
- [`compiler/`](implementation/compiler/) -- MLIR pipeline, optimization passes, artifact format, [TODO](implementation/compiler/TODO.md)
- [`runtime/`](implementation/runtime/) -- Dataflow scheduler, memory hierarchy, tasks, multi-agent, hierarchical AAM, [TODO](implementation/runtime/TODO.md)
- [`internals/`](implementation/internals/) -- Wire contracts, graph JSON contract

### [`advantages/`](advantages/) -- Why A-PXM *(for evaluators)*

What you get for free by targeting A-PXM instead of building from scratch.

- [**optimizations.md**](advantages/optimizations.md) -- The optimization catalog: compiler and runtime optimizations with LLVM analogues
- [**llvm-parallel.md**](advantages/llvm-parallel.md) -- Why the LLVM analogy is architecturally precise

### [`projects/`](projects/) -- Applied Projects *(for builders)*

Concrete applications and SDKs being built on A-PXM.

- [`codex/`](projects/codex/) -- Codex-on-APXM: reconstruct a coding agent on the shared substrate ([plan](projects/codex/plan.md), [architecture](projects/codex/architecture.md), [primitives](projects/codex/primitives.md))
- [`agentmate/`](projects/agentmate/) -- AgentMate: the Rust + Python frontend SDK ([architecture](projects/agentmate/architecture.md), [integration](projects/agentmate/integration.md), [status](projects/agentmate/status.md))

### [`guides/`](guides/) -- Getting Started *(for users)*

- [**getting-started.md**](guides/getting-started.md) -- Installation, first build, first run
- [**backends.md**](guides/backends.md) -- Backend/model hierarchy, setup, and routing
- [**first-graph.md**](guides/first-graph.md) -- Writing your first agent graph
- [**debugging.md**](guides/debugging.md) -- Debugging agent workflows
- [**multi-agent.md**](guides/multi-agent.md) -- Multi-agent orchestration

### [`reference/`](reference/) -- Configuration Reference

- [**config.md**](reference/config.md) -- Complete `~/.apxm/config.toml` file reference
- [**backends-quickref.md**](reference/backends-quickref.md) -- Quick reference for backend configuration

### External References

- **A-PXM Research Paper** (v1, v2) -- foundational academic papers on the Agent Program Execution Model. The key arguments and historical context are captured in [history.md](pxm/history.md) and [foundations.md](pxm/foundations.md).
- **Quantum Quill Lyceum** ([skool.com/quantum-quill-lyceum-1116](https://www.skool.com/quantum-quill-lyceum-1116)) -- file-tree agent architecture arguments that informed A-PXM's hierarchical AAM design. Key ideas integrated into [vision.md](pxm/vision.md) and [aam.md](pxm/aam.md).
