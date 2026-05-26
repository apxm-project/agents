# A-PXM: Agent Program Execution Model

> This is the **formal model** behind APXM. For the practical "skills as
> libraries" framing aimed at users and contributors, start at
> [VISION.md](../../VISION.md) and the [root README](../../README.md).

A-PXM is a formal Program Execution Model (PXM) for agentic AI. It treats agent
workflows not as opaque scripts but as typed dataflow graphs, making them
visible to compilers, schedulers, and verification tools.

---

## Learning Path

| Order | Document | What You Learn |
|-------|----------|----------------|
| 1 | [aam.md](aam.md) | The Agent Abstract Machine, the formal (Beliefs, Goals, Capabilities) state model every instruction operates on. |
| 2 | [ais.md](ais.md) | The Agent Instruction Set, typed operations organized by category, latency model, and MLIR dialect. |
| 3 | [memory.md](memory.md) | A-PXM's three-tier hierarchy (STM / LTM / Episodic) and why each tier exists. |
| 4 | [processes.md](processes.md) | Agent lifecycle, process/thread distinction, and multi-agent execution semantics. |

---

## For Implementers

The theory documented here is realized in the compiler and runtime. For
implementation details, see the crate READMEs:

- [apxm-compiler](../../crates/compiler/apxm-compiler/README.md): MLIR pipeline, [optimization pipeline](../compiler/pipeline.md), artifact format
- [apxm-runtime](../../crates/runtime/apxm-runtime/README.md): dataflow scheduler, memory hierarchy, multi-agent
- [apxm-ais](../../crates/core/apxm-ais/README.md): 43 AIS operations, attributes, types
