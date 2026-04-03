# Multi-Agent Negotiation → Consensus

**Pattern:** `SPAWN × 2 → COMMUNICATE × 2 (parallel) → WAIT_ALL → cross-counter × 2 → WAIT_ALL → THINK → UMEM → CHECKPOINT`

## What it demonstrates

- Two `SPAWN_AGENT` nodes start Claude and Codex as ACP subagents
- Both receive the same debate topic via parallel `COMMUNICATE` calls
- `WAIT_ALL` syncs their initial positions
- `CHECKPOINT` saves the initial positions durably
- Each agent receives the *other's* argument as a cross-critique prompt (via `MERGE`)
- Second round of `COMMUNICATE` gets each agent's counter-argument
- `WAIT_ALL` syncs the counter-arguments
- `THINK` (extended thinking) synthesizes a final consensus from all four positions
- `UMEM` persists the consensus to LTM
- Final `CHECKPOINT` saves the complete session

## Use case

Structured technical debate: two agents independently propose solutions, cross-critique each other, then a synthesis step extracts the best ideas and trade-offs.

## Running

```bash
apxm agent test claude
apxm agent test codex
apxm validate examples/patterns/multi-agent-negotiate/negotiate-consensus.apxm
apxm execute examples/patterns/multi-agent-negotiate/negotiate-consensus.apxm
```
