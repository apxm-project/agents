# Codex-on-APXM

Codex-on-APXM is the proof-of-concept that A-PXM's substrate can represent a production coding agent. Like LLVM-GCC proved LLVM IR could represent real C programs, Codex-on-APXM proves AIS graphs can represent real agent execution.

This directory tracks the plan, architecture, and progress toward reconstructing a Codex-class coding agent on top of A-PXM. AgentMate is treated as an optional/native frontend for authoring graphs; the architectural target is the A-PXM substrate itself.

## Why Codex First

Coding agents are the best first target because:

1. **Concrete primitives** — inference loop, tool dispatch, context management, memory, sandboxing, streaming — all exist and are well-understood
2. **Multiple implementations** — Codex, Claude Code, Aider, Cursor all solve the same problems differently, proving the primitives are universal
3. **Measurable** — SWE-Bench, HumanEval, and real codebases provide objective evaluation
4. **High value** — coding agents are the largest commercial agentic AI market today

## Documents

| Document | Description |
|----------|-------------|
| [case-study.md](case-study.md) | Why coding agents map onto A-PXM — the redundancy problem, AIS mappings, LLVM-GCC parallel |
| [plan.md](plan.md) | Step-by-step implementation plan with phases and milestones |
| [architecture.md](architecture.md) | How Codex maps onto AgentMate + A-PXM components |
| [primitives.md](primitives.md) | The 7 runtime primitives every coding agent needs and how AIS provides them |

## Relationship to Other Docs

- **Theory**: [case-study.md](case-study.md) — the high-level "why" (now in this directory)
- **Optimizations**: [optimizations.md](../../advantages/optimizations.md) — what A-PXM provides that Codex doesn't have today
- **AgentMate**: [../agentmate/](../agentmate/) — optional/reference frontend SDK; reusable pieces may be absorbed where they strengthen A-PXM
