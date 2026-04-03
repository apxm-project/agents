# 11 — Worker Pool (CLAIM + Parallel Workers)

**Pattern:** `UPDATE_GOAL → CLAIM × 3 → GUARD × 3 → THINK × 3 → CHECKPOINT → WAIT_ALL → ASK → UMEM → UPDATE_GOAL`

## What it demonstrates

- `UPDATE_GOAL` sets a named goal with priority before work begins
- Three parallel `CLAIM` nodes atomically pull tasks from the shared `analysis_tasks` queue
- `GUARD` with `on_fail: skip` gracefully handles empty queue slots (no crash)
- Three parallel `THINK` workers process their tasks with extended reasoning
- `CHECKPOINT` saves intermediate results durably (on_fail: continue)
- `WAIT_ALL` synchronizes all worker outputs
- `ASK` aggregates results ranked by confidence score
- `UMEM` persists the aggregate to long-term memory
- `UPDATE_GOAL` marks the goal as complete (remove)

## Use case

Distributed work queue processing: multiple agents claim tasks atomically, process in parallel, and produce a ranked aggregate. Graceful degradation when the queue has fewer tasks than workers.

## Running

```bash
apxm validate examples/11-worker-pool/worker-pool.json
apxm execute examples/11-worker-pool/worker-pool.json
```
