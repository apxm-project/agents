---
name: apxm-priority-lane-bench
description: Use when running the APXM priority-lane benchmark (foreground vs background concurrency). Enforces preregistration, paired-arm cache-salt scoping, and artifact placement under .apxm/.
user-invocable: true
repo: apxm-eval
---

# APXM Priority-Lane Benchmark

Load `_shared/apxm-evaluation-rules.md` and
`_shared/apxm-preregistration-rules.md` before broad work.

## What it measures

Tail-latency for a *foreground* lane against a configurable
*background* concurrency, with and without APXM's priority-aware
dispatch. The headline number is foreground p99 latency at c=16,
bg=16 (the current focus per
`docs/preregistrations/20260521T040539Z-apxm-priority-lane-c16-bg16.md`
and follow-ups).

## Pre-flight

1. `apxm-preregistration` — must be committed first.
2. Service running with the priority scheduler enabled:
   `dekk apxm vllm service-status <name> --probe` reports
   `policy = "priority"`. If not, the benchmark cannot back the claim.
3. `_shared/apxm-no-legacy-rules.md` — no resolver fallback, no
   FCFS literal, no `apxm_endpoints_available` flag.
4. Cache-salt scoping is **per `(arm, opt)`**, never per `(iter, row)`.
   See `feedback_paired_arm_cache_salt_scoping`. The harness in
   `examples/python/benchmarks/workloads/apxm_priority_lane.py`
   enforces this; do not override.

## Execution

```bash
# In the service allocation:
dekk apxm vllm service-exec <name> -- \
  bash tools/scripts/run_apxm_priority_lane.sh \
    --c 16 --bg 16 --iters <iters>

# Or the interleaved variant:
dekk apxm vllm service-exec <name> -- \
  bash tools/scripts/run_apxm_priority_lane_interleaved.sh \
    --c 16 --bg 16 --iters <iters>
```

Artifacts land under `.apxm/evaluation/apxm_priority_lane/runs/<UTC>/`.

## Analysis

```bash
python3 tools/scripts/analyze_apxm_priority_lane_run.py \
  .apxm/evaluation/apxm_priority_lane/runs/<UTC>/

python3 tools/scripts/analyze_apxm_priority_lane_combined.py \
  .apxm/evaluation/apxm_priority_lane/runs/<UTC1>/ \
  .apxm/evaluation/apxm_priority_lane/runs/<UTC2>/
```

## Write-up

`apxm-claim-evidence` produces the write-up at
`docs/evaluation/apxm_priority_lane/<UTC>.md` and (if paper-bound) a
claim card.

## Anti-patterns

- Running without confirming `policy = "priority"` — the result is
  unattributable.
- Running on a stock vLLM (no `/v1/apxm/scheduler` route).
- Killing the user's existing Slurm allocation to free GPUs; always
  allocate a fresh service job. See `feedback_no_kill_user_slurm`.
- Skipping the cache-salt scoping rule. The second arm hits the first
  arm's cache and silently invalidates the comparison.

## See also

- Plan 09 preregistration corpus in `docs/preregistrations/`.
- `examples/python/benchmarks/apxm_priority_lane_report.py` — report
  helper.
