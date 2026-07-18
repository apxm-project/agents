# Compute in A-PXM

> **Pre-canonical framing — non-normative.** References to AAM and current
> multi-agent operations describe the system being replaced. Target execution
> semantics are fixed by the
> [canonical v1 contract](../agents/agent-program-composition-and-air-contract.md).

Classical PXMs make compute visible to compilers: von Neumann exposes
instruction streams and memory; dataflow exposes dependency edges; actor models
expose message passing. A-PXM combines those lessons for agentic programs.

## What APXM Exposes

| Concern | APXM representation |
|---|---|
| Sequential work | AIS operations in a compiled graph |
| Data dependency | Typed dataflow edges |
| Long-running work | Futures and parked continuations |
| External side effects | Capability invocation with grants |
| Agent state | AAM beliefs, goals, capabilities, and episodes |
| Multi-agent calls | Concrete admitted targets, not organization charts |

## Why This Matters

Opaque agent scripts hide cost, effects, and authority. APXM makes those
properties explicit so the compiler can validate them, the runtime can schedule
them, and operators can audit what happened after a run.

Related docs:

- [ais.md](ais.md)
- [aam.md](aam.md)
- [processes.md](processes.md)
