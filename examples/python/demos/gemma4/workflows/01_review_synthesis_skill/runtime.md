# Runtime analysis — `01_review_synthesis_skill`

Sweep: 2026-04-28, local Gemma vLLM fork.
Sources:
- `/tmp/case01-runs/review-synthesis/sessions/opt-0/run-1/01_review_synthesis_skill-O0-balanced-20260428T032135/metrics.json`
- `/tmp/case01-runs/review-synthesis/sessions/opt-2/run-1/01_review_synthesis_skill-O2-balanced-20260428T032223/metrics.json`
- `/tmp/case01-runs/review-synthesis/runtime-o0-o2.csv`

## Headline numbers

| Metric | O0 | O2 | Δ |
|---|---|---|---|
| `runtime.execution.duration_ms` | 47840 | 40914 | **−14.5%** |
| `runtime.execution.nodes_executed / failed` | 16 / 0 | 16 / 0 | — |
| `wall_ms` (process incl. compile) | 48448 | 41527 | −14.3% |
| `compile_wall_ms` | 809 | 437 | −46% |
| LLM call count | 10 | 10 | — |
| Total tokens | 1,333,401 | 1,333,011 | ≈ 0 |
| `cached_input_tokens` | 0 | 0 | — |
| `pinned_blocks / pinned_handles` | 0 / 0 | 0 / 0 | — |
| `observed_critical_path.duration_ms` | 47529 | 40668 | −14.4% |
| `observed_critical_path.nodes` | [10,12,13,14,16] | [10,12,13,14,16] | identical |
| `scheduler.avg_parallelism` | 3.625 | similar | ≈ |

The critical path is **identical** node-for-node across O0/O2: the chain runs through the slower of the two ACP spawns (`codex_reviewer`, node 10) → synthesis (12) → validation (13) → print (14) → done (16).

## Where the wall time is spent (O0)

Per-node `operation.total_duration_ms` from O0 metrics (top contributors):

| Node | Op | Duration | Tokens (in / out) |
|---|---|---|---|
| 10 | `codex_reviewer` ACP spawn | 28304 ms | 14,775 / 258,400 |
| 9 | `claude_architect` ACP spawn | 12,640 ms | 53,210 / 1,000,000 |
| 12 | `gemma4_synthesis` think | ~3,800 ms | ~1,500 / ~150 |
| 13 | `gemma4_validation_checklist` reason | ~1,300 ms | ~400 / ~80 |
| 4–8 | 6 aspect asks (parallel) | 600–700 ms each | ~605 / 50–77 each |

The two ACP spawns dominate. They are ~95 % of the wall and they run concurrently with the 6 aspect asks (avg parallelism 3.6, max 5). The 6 aspect asks finish well inside the ACP window; they don't extend the critical path.

## Why the critical path is unchanged at O2

The compiler stamped `shared_prefix_group` on the 6 aspect ops and `priority` on most ops, but:

1. The aspect asks **were never on the critical path** — they fan out under the ACP spawns. Speeding them up via prefix-cache reuse would shorten only off-critical-path branches.
2. The two ACP spawns (`codex_reviewer`, `claude_architect`) are external subprocesses; the compiler cannot touch them.
3. With only 16 nodes and the critical path running through a single sub-agent (codex), the priority scheduler has nothing to preempt.

So the −14 % wall improvement at O2 is **not** from prefix-cache hits (`cached_input_tokens = 0` in both runs — vLLM didn't report any). It is consistent with reduced compile overhead (−372 ms) plus modest scheduler gains in dispatch order; the bulk of the speedup is **run-to-run noise on the codex ACP spawn**, which swung from ~28 s to ~21 s between runs. A multi-iteration sweep is needed to claim the speedup is real.

## Why `cached_input_tokens` is 0 in both runs

The runtime exposes vLLM prefix-cache telemetry only when the backend reports `cached_tokens` in its response. Two reasons it shows zero here:

- The benchmark harness sets `APXM_VLLM_CACHE_SALT=execution`, so each `dekk apxm execute` salts the prefix cache by execution_id and **deliberately** prevents cross-execution prefix reuse for clean methodology.
- Even within a single execution, the 6 aspect asks share `ASPECT_PREFIX` (~85 tokens) but the rest of each prompt diverges. The shared prefix is well below the typical block size that vLLM allocates per cache entry, so block-aligned overlap may have been below the reporting threshold.

## Why `pinned_blocks / pinned_handles` are 0

Neither stamping happens unless the workflow opts into `pin_policy.mode = "prefix"` on the module. This workflow doesn't declare it. The 6 shared-prefix aspect ops therefore get the `shared_prefix_group` *hint* (so vLLM can opportunistically reuse cache blocks) but no *pin* (which would force the runtime to keep the block resident across the execution).

## Scheduler signals worth checking

From the O2 `runtime.scheduler` block:
- `max_parallelism = 5`, `avg_parallelism = 3.6` — the runtime is parallelizing well within the available shape.
- `per_op_overhead_us` is single-digit microseconds. Scheduler overhead is negligible vs. LLM latency.

The `observed_graph.queue_wait` block reports zeros across the board — no node spent measurable time waiting on the ready queue. The ACP spawn slots dictated everything.

## What an apples-to-apples speedup story would need

This workflow's critical path is bound by external ACP sub-agents. To see compiler-attributable wall improvements you'd need to:
- Either pin the prefix (`pin_policy.mode="prefix"`) so cache hits become measurable, *and* re-run with `APXM_VLLM_CACHE_SALT` unset so the second iteration of a multi-iteration sweep can hit cache;
- Or replace the ACP spawns with in-process LLM calls so the priority scheduler can actually reorder critical-path work.

Today's numbers are honest about what the compiler did: metadata stamping on an externally-bound graph.
