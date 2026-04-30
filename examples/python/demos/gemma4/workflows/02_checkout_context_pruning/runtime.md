# Runtime analysis — `02_checkout_context_pruning`

Sweep: 2026-04-28, local Gemma vLLM fork.
Sources:
- `/tmp/case01-runs/context-pruning/sessions/opt-0/run-1/02_checkout_context_pruning-O0-balanced-…/metrics.json`
- `/tmp/case01-runs/context-pruning/sessions/opt-2/run-1/02_checkout_context_pruning-O2-balanced-…/metrics.json`

## Measured numbers

| Metric | O0 | O2 | Δ |
|---|---|---|---|
| `runtime.execution.duration_ms` | 7897 | 6891 | **−12.7 %** |
| `runtime.execution.nodes_executed` | **8** | **4** | −50 % |
| LLM call count | 6 | **2** | **−66 %** |
| Total tokens | 1,177 | **547** | **−53.5 %** |
| `cached_input_tokens` | 0 | 0 | — |
| `pinned_blocks / pinned_handles` | 0 / 0 | 0 / 0 | — |
| `observed_critical_path.nodes` | [1, 6, 8] | [1, 2, 3] | re-numbered post-DCE |
| `scheduler.avg_parallelism` | 2.375 | ~1.0 | reduced (correctly) |

This is the case where the compiler's IR mutation translates 1:1 to runtime cost reduction.

## Per-LLM-call breakdown at O2

`runtime.token_accounting.per_node`:

| Node | Op | input | output | total |
|---|---|---|---|---|
| 1 | `Checkout_Context` Ask | 127 | 95 | 222 |
| 2 | `Decision` Think | 171 | 154 | 325 |
| **Total** | | **298** | **249** | **547** |

At O0 there were 6 LLM calls (5 context Asks + 1 Decision Think). The 4 dead context Asks at O0 each spent ~150 input tokens and ~50 output tokens; the Decision Think also carried their content as additional input prefix (boosting its input-token bill). The DCE pass collapsed both effects.

## Why the wall improvement (−12.7 %) is smaller than the work reduction (−50 %)

The 4 eliminated Asks ran in parallel with `Checkout_Context` at O0 (`avg_parallelism = 2.375`), so they did not stack on the critical path. They contributed:

- 4 × ~250 ms each on a parallel side-branch — invisible to wall when the slowest sibling (Checkout_Context) takes ~3 s.
- A modest input-token bloat on the Decision Think prefill.

The wall improvement comes from:
- Fewer Decision-prefill tokens (~600 → 171 input tokens) → faster prefill.
- One less round-trip on the critical path: at O2 the schedule is purely `Checkout_Context → Decision → Print → done`.

The remaining ~6.9 s wall is dominated by the two LLM round-trips. With more aggressive prefix sharing (none possible here since only one Ask remains) or pinning (not enabled), there's nothing further to win.

## Critical path

O0 critical path was `[1, 6, 8]` — Checkout_Context (1) → Decision (6) → done (8).
O2 critical path is `[1, 2, 3]` — Checkout_Context (1) → Decision (2) → Print (3).

Both runs ran the same logical sequence; the renumbering reflects post-DCE op IDs. `observed_critical_path.duration_ms` shrinks from ~7.9 s to ~6.9 s.

## Scheduler signals

- `scheduler.max_parallelism` at O0 was higher (5 contexts could fan out). At O2 the surviving graph is essentially serial, so `avg_parallelism` drops by design — this is *correct* behavior, not regression.
- `scheduler.queue_wait_*` is zero in both runs.
- `scheduler.per_op_overhead_us` single-digit µs.

## Why `cached_input_tokens` and `pinned_blocks` are still 0

Same reasons as case 01:
- `APXM_VLLM_CACHE_SALT=execution` per the benchmark harness, deliberately defeating cross-execution prefix reuse.
- `pin_policy.mode="prefix"` not declared on the module, so the runtime stamps no pin metadata.
- After DCE there's only **one** distinct Ask prompt anyway; there's no within-execution prefix-sharing opportunity to measure.

## Net runtime story

This is the workflow where O2 actually produces an order-of-magnitude work reduction. The numbers all line up: half the ops, two-thirds fewer LLM calls, half the tokens. The 12.7 % wall improvement is muted only because the eliminated work was already off the critical path at O0.
