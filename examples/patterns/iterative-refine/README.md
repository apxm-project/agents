# Iterative Self-Refinement

**Pattern:** `ASK → CHECKPOINT → (REFLECT → ASK → CHECKPOINT) × 3 → VERIFY → CHECKPOINT → UMEM`

## What it demonstrates

- `REFLECT` critiques an existing draft using the execution trace
- `ASK` rewrites based on the critique
- `CHECKPOINT` saves durable state after each version (v0, v1, v2, final)
- `VERIFY` validates the final output against acceptance criteria
- `UMEM` persists the result to long-term memory

## Note on LOOP_START / LOOP_END

The graph format enforces DAG structure (no back-edges), so iterative refinement
is modeled by explicitly unrolling the loop. This example uses 3 rounds because
it keeps the pattern clear while staying in the JSON/Python graph model.

## Running

```bash
apxm validate examples/patterns/iterative-refine/iterative-refine.apxm
apxm execute examples/patterns/iterative-refine/iterative-refine.apxm
```
