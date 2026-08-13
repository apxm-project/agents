# PXM Foundations

> **Pre-canonical framing — non-normative.** AAM, memory-tier, process, and
> prototype AIS claims below are origin/migration evidence. Canonical v1
> authority is the
> [Agent Program composition and AIR contract](../agents/agent-program-composition-and-air-contract.md).

A Program Execution Model (PXM) defines what work is, how work is represented,
where state lives, and which scheduler rules are valid. APXM applies that idea
to agentic AI: workflows are typed dataflow graphs instead of opaque scripts.

## The Five Separations

1. **Program vs. process.** The compiled APXM graph is the program. A run,
   session, or spawned ACP child is a process executing part of that program.
2. **Instruction set vs. runtime.** AIS defines operations; the runtime chooses
   scheduling, persistence, and backend routes that satisfy those operations.
3. **State model vs. storage.** AAM defines beliefs, goals, capabilities, and
   episodes. Files, SQLite tables, and ledgers are implementations of that
   state model.
4. **Capability authority vs. transport.** Grants and permissions decide whether
   an operation is allowed. HTTP, MCP, ACP, and Link are only transport seams.
5. **Execution topology vs. organization topology.** Agent processes and graph
   nodes are runtime facts. Reporting lines, approvals, and directory
   visibility are host policy; see
   [execution admission contract](../agents/execution-admission-contract.md).

## ISA Contract

The ISA contract is simple: every AIS operation has typed operands, typed
results, declared effects, and deterministic AAM transitions. That is what lets
the compiler validate graphs and lets the runtime schedule independent work
without guessing what an opaque script might do.

Read next:

- [aam.md](aam.md) for the abstract machine state.
- [ais.md](ais.md) for the instruction set.
- [scheduling.md](scheduling.md) for scheduling rules.
