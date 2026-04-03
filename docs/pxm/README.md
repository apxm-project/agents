# A-PXM: Agent Program Execution Model

A-PXM is a formal Program Execution Model (PXM) for agentic AI. It treats agent workflows not as opaque scripts but as typed dataflow graphs -- making them visible to compilers, schedulers, and verification tools. A-PXM draws on five decades of PXM research (von Neumann, dataflow, BSP, CSP, MapReduce, actors) to define how compute, memory, and scheduling should work for agents. The result is an execution substrate where optimization, parallelism, and auditability come from the model itself, not from ad-hoc framework code.

---

## Start Here

For newcomers, read these documents in order. Each builds on the previous:

1. [**history.md**](history.md) -- How computing history repeats: the pattern from hardware to agents
2. [**foundations.md**](foundations.md) -- The core problem and the five separations that define A-PXM
3. [**aam.md**](aam.md) -- The Agent Abstract Machine: the formal state model every instruction operates on
4. [**ais.md**](ais.md) -- The Agent Instruction Set: the typed operations that form A-PXM's IR

After these four, the deep dives and case study can be read in any order.

---

## Document Index

### Core Theory

| Document | Description |
|----------|-------------|
| [history.md](history.md) | How the von Neumann bottleneck reappears in every computing era, and why agents are next |
| [foundations.md](foundations.md) | The agentic von Neumann bottleneck, the five separations (Compute, Memory, State, Optimization, Scheduling), and PXM research lineage |
| [aam.md](aam.md) | Agent Abstract Machine -- the (Beliefs, Goals, Capabilities) triple that defines agent state, with transition semantics |
| [ais.md](ais.md) | Agent Instruction Set -- typed operations organized across multiple categories (Reasoning, Memory, Tools, ControlFlow, Synchronization, Communication, Coordination, ErrorHandling, Identity) |

### Deep Dives

| Document | Description |
|----------|-------------|
| [compute.md](compute.md) | How six PXMs define a unit of compute, and how A-PXM introduces typed heterogeneous operations |
| [memory.md](memory.md) | Memory formalization across PXMs, and A-PXM's three-tier hierarchy (STM, LTM, Episodic) |
| [scheduling.md](scheduling.md) | Scheduling and execution models compared, and A-PXM's dataflow token design |
| [processes.md](processes.md) | Process model and multi-agent execution semantics |

### Applied

| Document | Description |
|----------|-------------|
| [vision.md](vision.md) | The LLVM-for-agents vision: what a full compilation stack for agentic AI looks like |
| [Codex case study](../projects/codex/case-study.md) | Case study reconstructing OpenAI Codex as an A-PXM graph |

---

## For Implementers

The theory documented here is realized in the compiler and runtime. For implementation details and TODOs, see [`../implementation/`](../implementation/):

- [architecture.md](../implementation/architecture.md) -- End-to-end system architecture
- [TODO.md](../implementation/TODO.md) -- Master TODO list (AIS gaps, substrate gaps, AAM gaps)
- [compiler/](../implementation/compiler/) -- MLIR pipeline, optimization passes, artifact format
- [runtime/](../implementation/runtime/) -- Dataflow scheduler, memory hierarchy, multi-agent, [hierarchical AAM](../implementation/runtime/hierarchical-aam.md) (TODO)
- [ais/](../implementation/ais/) -- Per-category operation reference with concrete examples
