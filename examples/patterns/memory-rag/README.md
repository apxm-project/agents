# Memory-Augmented RAG Pipeline

**Pattern:** `QMEM × 2 → FENCE → MERGE → GUARD → REASON (w/ context) → VERIFY → UMEM × 2 → FENCE → CHECKPOINT`

## What it demonstrates

- `QMEM` recalls from both LTM (long-term memory) and episodic memory in parallel
- `FENCE` ensures all prior reads are visible before proceeding (memory barrier)
- `MERGE` concatenates recalled context
- `GUARD` (on_fail: skip) gracefully handles cold-start (no prior memory)
- `REASON` answers with context (if memory exists) or cold (if guard skips)
- `VERIFY` validates the answer against known facts
- `UMEM` persists new knowledge to LTM + records the query in episodic memory
- `FENCE` ensures writes are ordered before checkpointing
- `CHECKPOINT` saves session state with 24-hour TTL

## Use case

Knowledge-augmented Q&A: the agent recalls what it already knows, reasons with that context, verifies its answer, and updates its memory — all in a single graph execution.

## Running

```bash
apxm validate examples/patterns/memory-rag/memory-rag-pipeline.json
apxm execute examples/patterns/memory-rag/memory-rag-pipeline.json
```
