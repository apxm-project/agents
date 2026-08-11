# PXM documentation

The normative theory is [Program Execution Model theory](theory.md). It defines
the single digest-linked execution spine and the separate authoritative sources
for behavior, semantic legality, publication, authority, admission/spend,
delivery, execution, evidence, and Studio projections.

## Historical theory

- Status: non-normative migration evidence
- Baseline: pre-replacement APXM abstract-machine design
- Target authority: [Program Execution Model theory](theory.md) and [Agent Program composition and AIR contract](../agents/agent-program-composition-and-air-contract.md)

This directory preserves the theory that informed the prototype
implementation: AAM beliefs/goals, memory tiers, the 38-operation AIS,
process/spawn/communicate semantics, cognition op latency classes, session
re-arm, and graph scheduling. It is useful for understanding what must be
migrated, but it is not the accepted target architecture.

The target replaces those assumptions:

| Pre-canonical idea | Canonical v1 target |
| --- | --- |
| AAM beliefs/goals/capabilities as runtime semantic state | Explicit typed Program Context/local values plus admitted Capabilities |
| QMEM/UMEM and memory tiers in AIR | Local values or external durable-memory Capability |
| ASK/THINK/REASON/etc. runtime operations | Frontend patterns over `model.call` |
| spawn/communicate/handoff/delegate/flow/workflow ops | `program.new` and `program.invoke` |
| host/runtime session re-arm loop | Frontend-authored loop and generic program yield/resume |
| runtime Turn | Studio projection of a generic loop-region occurrence |
| direct/raw AIR authoring | Python/TypeScript FrontendGraph v2 through Rust compiler |

Historical pages:

1. [AAM](aam.md)
2. [AIS](ais.md)
3. [Memory](memory.md)
4. [Processes](processes.md)
5. [Foundations](foundations.md)
6. [Compute](compute.md)
7. [Scheduling](scheduling.md)

Every page above is baseline evidence only. The binding dispositions are
[ADR-0008](../adr/0008-agent-programs-compose-through-new-and-invoke.md),
[ADR-0009](../adr/0009-air-has-five-public-semantic-operations.md), and
[ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md).
No target implementation may cite this directory to preserve a v1 operation,
runtime state model, loop, callback, reader, or compatibility path.
