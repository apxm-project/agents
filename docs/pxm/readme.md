# A-PXM: Agent Program Execution Model

A-PXM is a formal Program Execution Model (PXM) for agentic AI. It treats agent workflows not as opaque scripts but as typed dataflow graphs -- making them visible to compilers, schedulers, and verification tools. A-PXM draws on five decades of PXM research (von Neumann, dataflow, BSP, CSP, MapReduce, actors) to define how compute, memory, and scheduling should work for agents.

---

## Learning Path

Read the core theory in order. Each document builds on the previous.

| Order | Document | What You Learn |
|-------|----------|----------------|
| 1 | [history.md](history.md) | The recurring pattern: new capability, ad-hoc wiring, opacity wall, formal model. Why agents are next. |
| 2 | [foundations.md](foundations.md) | The core thesis: the agentic von Neumann bottleneck, the five separations (Compute, Memory, State, Optimization, Scheduling), and the ISA contract analogy. |
| 3 | [aam.md](aam.md) | The Agent Abstract Machine -- the formal (Beliefs, Goals, Capabilities) state model every instruction operates on. |
| 4 | [ais.md](ais.md) | The Agent Instruction Set -- typed operations organized by category, latency model, and MLIR dialect. |

After these four, the deep dives and applied documents can be read in any order.

---

## Deep Dives

Each document compares A-PXM against six classical PXMs on one axis.

| Document | Focus |
|----------|-------|
| [compute.md](compute.md) | What is a unit of compute? Von Neumann instructions through AIS operations. |
| [memory.md](memory.md) | How memory is organized. A-PXM's three-tier hierarchy (STM / LTM / Episodic) and why each tier exists. |
| [scheduling.md](scheduling.md) | How execution order is determined. Token-counting dataflow with O(1) readiness detection. |
| [processes.md](processes.md) | Agent lifecycle, process/thread distinction, and multi-agent execution semantics. |

## Applied

| Document | Focus |
|----------|-------|
| [vision.md](vision.md) | The LLVM-for-agents vision: what a full compilation stack for agentic AI looks like. |

---

## For Implementers

The theory documented here is realized in the compiler and runtime. For implementation details, see the crate READMEs:

- [apxm-compiler](../../crates/compiler/apxm-compiler/README.md) -- MLIR pipeline, [optimization pipeline](../compiler/pipeline.md), artifact format
- [apxm-runtime](../../crates/runtime/apxm-runtime/README.md) -- dataflow scheduler, memory hierarchy, multi-agent
- [apxm-ais](../../crates/core/apxm-ais/README.md) -- 41 AIS operations, attributes, types
