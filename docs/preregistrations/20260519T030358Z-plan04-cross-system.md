# Pre-registration

> Commit this file BEFORE the measurement run starts. Post-hoc edits invalidate
> any goodput / SLO claim derived from the run. Follows
> [Plan 00 — Evaluation Methodology Charter](../../.apxm/docs/plans/00-evaluation-methodology.md).

## Run identity

- Run name (slug): `20260519T030358Z-plan04-cross-system`
- Author: `raherrer`
- Date (UTC, ISO 8601): `2026-05-19T03:03:58Z`
- Linked claim file (planned): `.apxm/docs/claims/plan04-cross-system.md`

## Workload(s) and rationale

- Workloads (three, run as independent per-workload paired arms):
  - **Mooncake** trace replay (`mooncake_replay.py` → `workloads/mooncake_row.py`)
    — production conversation trace, `reuse_group` derived from `hash_ids[0]`
    so the trace's natural prefix-cache sharing cohort is visible to APXM.
  - **ShareGPT** multi-turn (`workloads/sharegpt_row.py`) — sequential ASK
    chain whose rendered prefix grows turn-by-turn, every turn stamped with
    a per-conversation `reuse_group`.
  - **LooGLE** long-shared-context (`workloads/loogle_row.py`) — fan-out of
    N questions sharing one long document, every question stamped with a
    per-document `reuse_group`.
- Why this mix: each workload tests a distinct shared-prefix pattern
  (production-trace cohorts, multi-turn growth, document fan-out). The
  three patterns are complementary — a single-workload claim would not
  generalise, and a single-workload null would not falsify dispatch as a
  whole. Cross-workload coverage is the methodology contract for Plan 04
  EVAL per [`.apxm/docs/plans/04-cross-system-baselines.md`](../../.apxm/docs/plans/04-cross-system-baselines.md).
- Cohort stamping was shipped 2026-05-18 in commits `076b4a40` (Mooncake)
  and `99c14776` (ShareGPT + LooGLE) — necessary precondition for the
  pin-engagement signal to appear in the per-cell metrics.

## Hypothesis under test

- Form: On each of `mooncake | sharegpt | loogle` at c=32, APXM-on reduces
  `batch_wall_ms` (cell-level, paired across iterations) vs flat-HTTP
  (`--no-apxm-hints` arm) on `gpt-oss-120b` running on `vllm-gptoss`,
  with paired bootstrap-CI on the (APXM-on / flat-HTTP) ratio strictly < 1.0
  in **at least one** of the three workloads. The flat-HTTP arm and the
  APXM-on arm run against the same vLLM binary (same image, same
  `--enable-prefix-caching`, same `--scheduling-policy priority`) — only the
  request body (`vllm_xargs.apxm` block + reuse_group) and graph
  registration differ.
- Concrete value: per-workload paired-ratio CI excluding 1.0 in ≥1
  workload, with the magnitude of any win bounded by its own CI. **The
  claim does not commit to any specific percentage in advance** — the
  matrix is the result; percentages are read from the CSVs.

## Pre-registered SLOs

- TTFT P95 SLO: not gating this claim (workloads are batch-style).
- TPOT P95 SLO: not gating this claim.
- DAG critical-path P95 SLO: not gating this claim.
- Joint-SLO definition for goodput: not used.

## Metric tier

- Primary metric (the claim hangs on this one): `batch_wall_ms` per cell,
  paired across iterations within a (workload, arm) pair.
- Secondary metrics (reported but not claim-bearing):
  `pinned_blocks_peak_max`, `prefix_cache_hit_rate`,
  `prefix_cache_queries_delta`, `prefix_cache_hits_delta`,
  `failed_tenants` (must be 0 for a claim-bearing cell per lint rule).
- Decision rule: ship the claim iff the primary metric's paired bootstrap-CI
  on the (APXM-on / flat-HTTP) ratio excludes 1.0 in the favorable
  direction (ratio < 1.0) in **at least one** workload, AND the paired
  Wilcoxon p < 0.05 in that workload. **Honest negatives** (workloads
  where APXM did not win, or where pin did not engage despite cohort
  stamping) must be reported in the claim file alongside the wins,
  citing the regime each null was observed in.
- Cross-workload synthesis: the per-workload verdicts are stitched into a
  combined CSV + summary via
  `examples/python/benchmarks/plan04_cross_workload.py` (shipped
  `99c14776`).

## Pin-engagement gate

- `pinned_blocks_peak_max ≥ 1` is **required** for at least one APXM-on
  cell across the three workloads to substantiate the cohort-stamping
  hypothesis. The expected regime for engagement (per
  `.apxm/docs/claims/rack-scale-int-preliminary.md` three-regime map) is
  warm prefix cache + cohort + queue contention. If no workload engages
  pin, the claim becomes a scoped honest-null finding rather than a
  positive dispatch claim.
- If pin does not engage at default `gpu-memory-utilization=0.9`, the
  mitigation order is: (a) extend concurrency to drive higher
  contention; (b) constrain KV via `gpu-memory-utilization=0.20` (per the
  INT-03 / `dispatch-pin-latency-win.md` regime) — explicitly recording
  the change in the run manifest.

## Power and thermal thresholds

- GPU clock lock: same caveat as
  `20260517T224507Z-gptoss120b-concurrent-matrix.md` — privileged
  `setperflevel` is not available inside `apxm-vllm-runtime`; auto-mode
  applies uniformly across all cells, internally consistent for A/B.
- Thermal soak between cells: rely on the harness's existing inter-cell
  pause.
- Reject-run threshold: GPU junction temp > 80°C at cell start (must be
  re-verified at pre-flight; record observed temps in the manifest).
- Power sampling interval: not collected this run unless rocm-smi power
  capture lands before the run starts — in which case the harness
  records the sampling interval into the manifest.

## GPU-hour budget

- Estimate: ~2.0 GPU-hours total (3 workloads × 2 arms × ~10 min/cell
  × 1 node × 8 GPUs), allowing 25% headroom for cold-cache warmup and
  cohort tagging overhead.
- Hard cap: 4.0 GPU-hours (abort run if exceeded — fits a single 4-hour
  Slurm wall budget).
- Cluster reservation reference: Slurm job ID `<filled at run-start>`
  for `vllm-gptoss`. Requires a fresh allocation; the prior `vllm-gptoss`
  (job 49149) expired naturally at 2026-05-18T05:44:48 wall.

## Build identity (filled by the harness, do NOT edit before run)

- APXM SHA: `<filled in by manifest>` (working tree head as of
  pre-registration commit will be the basis; the harness records the
  actual checkout SHA into the per-cell manifest).
- vLLM-fork SHA: `<filled in by manifest>` (must be ≥ `490aaad0c` so the
  fork-side `x-apxm-fields-honored` emitter is live).
- Image: `<filled in by manifest>` (must be the post-rebase image so
  `num_computed_tokens` scheduler fix is included).
- Zoo snapshot ref: `<filled in by manifest>`.

## Matrix design

Per-workload paired arms (apxm-on vs flat-http), c=32, single
`vllm-gptoss` service:

| Workload | Arm | Cell |
|---|---|---|
| Mooncake | flat-http (`--no-apxm-hints`) | M-FH |
| Mooncake | apxm-on (graph hints + cohort) | M-ON |
| ShareGPT | flat-http | S-FH |
| ShareGPT | apxm-on | S-ON |
| LooGLE | flat-http | L-FH |
| LooGLE | apxm-on | L-ON |

- Both arms run against the same `vllm-gptoss` binary; the difference is
  the request body (`vllm_xargs.apxm` block + `reuse_group`) and whether
  `/v1/apxm/graphs/register` is called.
- Workload isolation: a `POST /v1/apxm/admin/reset_prefix_cache` is
  issued between workloads so the prefix-cache state from workload N
  does not leak into workload N+1.
- Per-(arm, opt) cache-salt scoping is mandatory — never per-(iter, row)
  (the historical contamination bug recorded in archived
  `mooncake-paired-arms-saltfix.md`).

## Honest-negative discipline

Per Plan 00 §6 and [`.apxm/docs/claims/CLAIMS-INDEX.md`](../../.apxm/docs/claims/CLAIMS-INDEX.md):

- Any workload where APXM-on does not win is a **claim-bearing honest
  negative** — it must be reported in `plan04-cross-system.md` with the
  regime in which it was observed, not silently omitted.
- Any cell where `failed_tenants > 0` is **not claim-bearing**. The
  partial result is preserved on disk for audit (per the
  `mooncake-replay-partial.md` pattern); the claim cites only cells
  where `failed_tenants == 0`.
- Any cell where the pin path did not engage (`pinned_blocks_peak_max ==
  0`) **despite cohort stamping** must be reported with the
  `gpu-memory-utilization` and concurrency at which it was observed —
  this informs the three-regime map and Plan 09 INT synthesis.

## Linked plans

- [`.apxm/docs/plans/04-cross-system-baselines.md`](../../.apxm/docs/plans/04-cross-system-baselines.md)
- [`.apxm/docs/plans/00-evaluation-methodology.md`](../../.apxm/docs/plans/00-evaluation-methodology.md)
- [`.apxm/docs/plans/03-comparable-workloads.md`](../../.apxm/docs/plans/03-comparable-workloads.md)
  (workloads + cohort-stamping precondition)
- [`.apxm/docs/plans/07-dispatch-phase1-closure.md`](../../.apxm/docs/plans/07-dispatch-phase1-closure.md)
  (the honesty channel every reported metric rests on)
