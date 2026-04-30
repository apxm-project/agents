# Runtime analysis — `03_vllm_backend_hints`

Sweep: 2026-04-28, local Gemma vLLM fork.
Sources:
- `/tmp/case01-runs/backend-hints/priority-contention/sessions/opt-0/run-1/03_vllm_backend_hints-O0-latency-…/metrics.json`
- `/tmp/case01-runs/backend-hints/priority-contention/sessions/opt-2/run-1/03_vllm_backend_hints-O2-latency-20260428T032331/metrics.json`

## Measured numbers

| Metric | O0 | O2 | Δ |
|---|---|---|---|
| `runtime.execution.duration_ms` | 15572 | **15751** | **+1.1 % (slightly worse)** |
| `runtime.execution.nodes_executed / failed` | 16 / 0 | 16 / 0 | — |
| LLM call count | 7 | 7 | — |
| Total tokens | 22,663 | 22,669 | ≈ 0 |
| `cached_input_tokens` | 0 | 0 | — |
| `pinned_blocks / pinned_handles` | 0 / 0 | 0 / 0 | — |
| `observed_critical_path.duration_ms` | 15424 | 15619 | +1.3 % |
| `observed_critical_path.nodes` | [4, 9, 10, 11, 15, 16] | [4, 9, 10, 11, 15, 16] | identical |
| `scheduler.avg_parallelism` | 2.1875 | **2.4375** | +11 % |
| `scheduler.max_parallelism` | 5 | 5 | — |

O2 wall is **not** measurably faster. This is the most informative runtime story of the three demos.

## Per-LLM-call breakdown at O2

`runtime.token_accounting.per_node`:

| Node | Op | input | output | total | duration_ms |
|---|---|---|---|---|---|
| 5 | `background_security` | 5,491 | 15 | 5,506 | 3,810 |
| 6 | `background_reliability` | 5,492 | 15 | 5,507 | 3,668 |
| 7 | `background_finance` | 5,489 | 15 | 5,504 | 3,961 |
| 8 | `background_rollout` | 5,611 | 16 | 5,627 | 4,120 |
| 9 | `critical_triage` | 68 | 22 | 90 | 3,213 |
| 10 | `critical_plan` | 85 | 25 | 110 | 667 |
| 11 | `critical_summary` | 86 | 239 | 325 | 4,537 |
| **Total** | | **22,322** | **347** | **22,669** | |

Two facts that explain the rest of the analysis:

- **The background audits each push ~5,500 input tokens at vLLM** (the `_pressure_facts(lane, …)` block dominates input). Their decode is tiny (15 tokens out) — these are essentially big prefills with truncated outputs, by design (the workflow uses them as backend pressure, not for their content quality).
- **The critical chain is small** (90 + 110 + 325 = 525 tokens total) but `critical_triage` waits ~3.2 s and `critical_summary` takes ~4.5 s. With only ~108 input tokens between them, the wall time is mostly **vLLM scheduler queue wait** behind the 4 background prefills.

## Critical path

`observed_critical_path.nodes = [4, 9, 10, 11, 15, 16]` in both O0 and O2:
- 4: presumably `incident_dossier` tool dispatch (~600 ms)
- 9 → 10 → 11: critical_triage (3.2 s) → critical_plan (0.7 s) → critical_summary (4.5 s)
- 15 → 16: print → done (~ms)

`observed_critical_path.duration_ms = 15619` at O2 vs `15424` at O0 — within run-to-run noise (a single iteration; the small regression is meaningless statistically).

## Why O2 didn't move the needle

The O2 compiler stamped:
- `priority = high` on the critical chain
- `priority = low` on the 4 background audits
- Full scheduling metadata on all 9 executable ops

For this metadata to **reduce wall time**, the runtime priority scheduler needs:
1. **Contention.** The scheduler can only reorder if there's a queue. Today the runtime sees `avg_parallelism = 2.4`, `max = 5` — well within capacity. It dispatches everything as soon as deps clear; there's no reorder window.
2. **vLLM-side priority.** The `extra_body.apxm_hints` payload includes the priority hint, but vLLM's scheduler treats requests FIFO unless the fork honors the hint. The local fork (submodule HEAD `6dc7a18958`; load-bearing commits `8e2ccc9308`, `fe6d35e45b`, `6dc7a18958`) does pass the hint through, but with only 7 concurrent requests the vLLM batch scheduler has nothing to defer.

Since both sides have ample capacity, the priority stamping is informational only. The wall-time signal you'd want — critical_triage running *before* the 4 background prefills land — would require either:
- A `concurrency_limit` declared on the module so the runtime queues requests, *then* the priority sort matters; or
- Loading vLLM with more in-flight requests until its batch scheduler queues; or
- Explicit `wait_all` deps that put the criticals after the backgrounds (the current graph already has the criticals start at the same fan-out point as the backgrounds).

## Why `cached_input_tokens` and `pinned_blocks` are 0

- `APXM_VLLM_CACHE_SALT=execution` — benchmark harness deliberately defeats cross-execution prefix reuse.
- `pin_policy.mode="prefix"` not declared on this module — so the runtime stamps no pin metadata even though `pinned_handles` would be the natural place to stamp the critical chain's prompt blocks.
- The 4 background audits each have lane-specific `_pressure_facts` content at the top, so they don't share a prefix to cache (this is also why `shared-prefix-analysis` correctly fired=0 in the compiler — see `compiler.md`).

## What the avg_parallelism bump (+11 %) actually means

`avg_parallelism` rose from 2.19 to 2.44 at O2. That is the scheduler dispatching slightly more concurrently — consistent with `assign-priority` letting the scheduler ready criticals in parallel with backgrounds rather than serializing on the merge Think. But because the bottleneck is per-request vLLM latency (3.2-4.5 s on the critical chain regardless of when it's dispatched), the parallelism gain doesn't shorten the critical path.

## Net runtime story

The compiler did everything it could (priority stamping, scheduling metadata). The runtime correctly threaded the priority hint to vLLM. Neither side has the slack needed to convert priority into wall-time wins. To make this demo's O2 numbers compelling you'd need to add a real bottleneck — an explicit `concurrency_limit`, more in-flight requests, or pin policy enabling block-level scheduling.

Today's O2 result for case 03: zero wall-time improvement, +1 % well within noise. The metadata is correct; the workload doesn't exercise it.
