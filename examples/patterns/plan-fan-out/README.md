# Plan → Fan-Out → Synthesize

**Pattern:** `PLAN → ASK × 3 (parallel) → WAIT_ALL → THINK → VERIFY → CHECKPOINT`

## What it demonstrates

- `PLAN` decomposes a high-level goal into structured steps with constraints
- Three parallel `ASK` nodes each write an independent section using the plan
- `WAIT_ALL` synchronizes all three outputs
- `THINK` (extended thinking) assembles the sections into a cohesive document
- `VERIFY` checks factual accuracy
- `CHECKPOINT` saves the final artifact with a 1-hour TTL

## Running

```bash
apxm validate examples/patterns/plan-fan-out/plan-fan-out.apxm
apxm execute examples/patterns/plan-fan-out/plan-fan-out.apxm
```
