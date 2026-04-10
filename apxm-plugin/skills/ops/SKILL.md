---
name: ops
description: Browse and inspect AIS operations available for graph nodes
user-invocable: true
---

# Ops

Lists and inspects the AIS (Agent Instruction Set) operations — the building blocks of every graph node. Each operation defines what a node does (send a prompt, invoke a tool, synchronize branches, etc.), its latency characteristics, required and optional attributes, and whether it produces output.

This is the reference for graph authoring: when you need to know which operations exist, what attributes they require, or how to use a specific op.

## Commands

```bash
dekk apxm ops list                         # all operations, grouped by category
dekk apxm ops list --category reasoning    # filter to one category
dekk apxm ops list --json                  # machine-readable list
dekk apxm ops show ASK                     # detailed info for one operation
dekk apxm ops show ASK --json              # detailed info as JSON with example node
```

## Operation Categories

- **Reasoning** — LLM interactions: `ASK`, `THINK`, `REASON`
- **Memory** — State read/write: `QMEM`, `UMEM`
- **Planning** — Goal management: `PLAN`, `REFLECT`, `VERIFY`
- **Tools** — External invocation: `INV`, `EXC`, `PRINT`
- **Control Flow** — Branching and looping: `JUMP`, `BRANCH_ON_VALUE`, `LOOP_START`, `LOOP_END`, `RETURN`, `SWITCH`, `FLOW_CALL`
- **Synchronization** — Parallel coordination: `MERGE`, `FENCE`, `WAIT_ALL`
- **Error Handling** — Fault tolerance: `TRY_CATCH`, `ERR`
- **Communication** — Inter-agent messaging: `COMMUNICATE`
- **Coordination** — Multi-agent orchestration: `UPDATE_GOAL`, `GUARD`, `CLAIM`, `PAUSE`, `RESUME`, `DELEGATE`, `NEGOTIATE`
- **Identity** — Passthrough: `NOP`, `IDENTITY`

## Show Output

`dekk apxm ops show ASK` displays:
- Full description and extended explanation
- Category and latency tier (slow/medium/fast/none)
- Whether the operation produces output
- **Required fields** with descriptions (e.g., `template_str` — the prompt template)
- **Optional fields** with descriptions (e.g., `budget_tokens` — max token budget)
- Example JSON node ready to paste into a graph

## When to Use

- When authoring a new graph and you need to know which op to use
- To check required attributes before writing a node (avoid validation errors)
- To discover operations you might not know about (e.g., `DELEGATE`, `NEGOTIATE`)
- As a quick reference instead of reading the full AIS documentation
