# Pre-registration

> Commit this file BEFORE the measurement run starts. Post-hoc edits invalidate
> any agentic-accuracy claim derived from the run. Follows
> [Plan 00 — Evaluation Methodology Charter](../../.apxm/docs/plans/00-evaluation-methodology.md)
> and [Plan 05 — Agentic Task Quality](../../.apxm/docs/plans/05-agentic-task-quality.md).

## Run identity

- Run name (slug): `20260519T185355Z-plan05-tau2-telecom-n20-powered`
- Author: `raherrer`
- Date (UTC, ISO 8601): `2026-05-19T18:53:55Z`
- Linked claim file (planned): `.apxm/docs/claims/agentic-accuracy-telecom-n20.md`

## Workload and rationale

- Benchmark: `tau2-bench` (vendored at `tools/external/tau2-bench/`,
  pin sha `0ed8c0ef0a1f6b024a0fb733e186922411874879`).
- Domain: `telecom` (small-suite N=20; same domain and same wire
  shape as the just-shipped N=5 cell at
  `.apxm/docs/claims/agentic-accuracy-telecom-n5.md`).
- Sub-sample: full `tasks_small.json` (exactly 20 tasks); same seed
  in both arms so identical task subsets land in each arm.
- One trial per task (`--num-trials 1`). The pair is therefore 20
  paired observations on the primary metric (continuous wall-time
  per task) and 20 paired binary observations on the secondary
  metric (pass / fail).

## Motivation — why this run, why now

The N=5 telecom cell (`.apxm/docs/claims/agentic-accuracy-telecom-n5.md`)
shipped on 2026-05-19 as an **exact-equivalence on pass-at-1**
(paired diff = 0.000 on every pair) with a **directional caveat**:
apxm-on mean wall was **+8.11 s/task (~31% slower)** than flat-http.
That is the largest single-cell wall-time directional disagreement
observed across the airline + retail + telecom N=5 cells.

Mechanism dig on existing telecom artifacts (see investigation
notebook in run dir): the +31% gap decomposes into

1. apxm-on agent took **~10% more assistant turns** on most tasks
   (51 vs 46 turns across 5 tasks);
2. apxm-on processed **~16% more prompt tokens** and generated
   **~23% more output tokens** (downstream of extra turns →
   accumulating context);
3. per-call generation latency is only ~13% higher on apxm-on
   (1.96 s vs 1.73 s mean), consistent with larger prompts not
   pure dispatch overhead;
4. per-task delta is heterogeneous — one task (data_saver) is
   actually faster on apxm-on, three are 29-57% slower.

Reading: the N=5 wall-time signal is the noisy tail of a
high-variance distribution. The upstream wire-level cause is
plausibly the `prefix_cohorts` hint (honored on apxm-on only)
shaping vLLM's KV-cache routing → tiny attention-score
differences → token-sample divergence → multi-turn compound. At
N=5 this is within stochastic noise; this run quadruples power
to test whether the signal survives.

## Hypothesis under test

- **Primary metric form (this run):** paired **log-ratio** of
  per-task `mean_duration_s`:
  `delta_i = ln(apxm_on.duration_i / flat_http.duration_i)`
  averaged across the N=20 paired tasks. Use log-ratio (not raw
  diff) so the band is scale-invariant (tasks vary from ~10 s to
  ~50 s wall time; a ±3 s absolute band is meaningless on a 50-s
  task and crushing on a 10-s task).
- **Equivalence band:** ±10% (`[ln(0.9091), ln(1.10)] = [-0.0953, +0.0953]`).
- **Decision rule on the primary metric (95% bootstrap CI on the
  mean of paired log-ratios, B=10000, seed=1234):**
  - **(a) Equivalence:** CI **entirely inside** [-0.0953, +0.0953]
    → ship as equivalence claim ("telecom wall-time within ±10%
    band at N=20; the N=5 +31% signal does not survive
    quadrupled power").
  - **(b) Regression confirmed:** CI lower bound **> +0.0953**
    → ship as honest-negative regression claim ("telecom wall-time
    is X% slower on apxm-on at N=20 with CI [lower, upper];
    confirms the N=5 caveat; opens Plan 08 follow-up"). The
    associated retraction notice on the N=5 caveat language is
    written from "directional signal" to "confirmed regression".
  - **(c) Improvement:** CI upper bound **< -0.0953** → ship as a
    superiority claim (extremely unlikely given the N=5 direction).
  - **(d) Indeterminate:** CI crosses an equivalence-band boundary
    → ship as "indeterminate at N=20, equivalence band not
    excluded, regression band not excluded" and queue an N=50 (full
    `tasks.json` first-50) follow-up.

- **Secondary metric (carried, not load-bearing):** paired
  difference apxm-on `pass_at_1` − flat-http `pass_at_1` on
  identical task IDs. Equivalence band ±0.20 (kept for
  back-compat with the N=5 / airline N=20 / airline N=50 arc).

## Pre-registered SLOs

- Wall-time SLO (primary): equivalence band ±10% on paired log-ratio.
- Pass-at-1 SLO (secondary): equivalence band ±0.20.
- TTFT / TPOT P95 SLOs: not gating this claim.
- DAG critical-path P95 SLO: not used.

## Metric tier

- Primary: paired log-ratio of `mean_duration_s` per task; mean
  + 95% bootstrap CI; decision rule as above.
- Secondary: paired diff on `pass_at_1`, `db_match_rate`,
  `mean_steps`, `mean_tool_calls`. Reported for transparency.
- Carried diagnostics: per-task HAL request counts, per-task token
  counts (in / out), per-task assistant-message count, per-task
  generation_time_seconds mean. These let the claim file
  decompose any surviving signal into "extra turns" vs "slower
  per-turn" — the same decomposition the N=5 mechanism dig used.

## Pin-engagement gate

Not load-bearing for this claim. HAL probe on both arms captured
for audit. The honesty-channel header
(`x-apxm-fields-honored`) is captured pre-run on the apxm-on arm
and absent on the flat-http arm; both states are recorded in the
manifest.

## Power and thermal thresholds

- Auto-mode clocks (uniformly across both arms, internally
  consistent for A/B). No `rocm-smi --setperflevel` lock unless
  the user-facing budget allows.
- Reject-run threshold: "vLLM service unhealthy at arm start" or
  "HAL probe fails to round-trip a single chat completion".

## GPU-hour budget

- Estimate: ~0.75 GPU-hours total. N=20 tasks × 2 arms ×
  ~30 s/task wall (≈ midpoint of the N=5 apxm-on 34 s and
  flat-http 26 s) ≈ 20 min wall benchmark; add service warm-up
  (~5 min) + HAL adapter startup (~2 × ~30 s) → ~30 min total.
  Equivalent to ~4 GPU-hours of MI300X compute on TP=8.
- Hard cap: 1.5 GPU-hours (≈12 GPU-hours on 8 GPUs).
- Cluster reservation reference: fresh `vllm-gptoss` allocation
  to be requested in the same wall window as this pre-reg commit;
  previous service (job 54322) has expired.

## Build identity (filled by the harness / captured at run-start)

- APXM SHA: `<filled at run-start>` (working tree head as of
  pre-reg commit will be recorded by the run-manifest).
- vLLM-fork image: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
  (the `mwon-postrebase-ray` image used for the entire airline +
  retail + telecom N=5 arc — unchanged so the run is wire-comparable
  with the predecessor cell).
- HAL adapter SHA: `hal-adapter-v1-2026-05-19` (unchanged).
- tau2 pin SHA: `0ed8c0ef0a1f6b024a0fb733e186922411874879` (unchanged).

## Matrix design

Identical to the N=5 telecom cell except `--num-tasks 20`:

| Arm | HAL adapter args | Wire shape |
|---|---|---|
| `apxm-on` | `--backend apxm-on --inject-apxm --graph-context tau2-telecom-n20` | `extra_body.vllm_xargs.apxm` injected per request |
| `flat-http` | `--backend flat-http` | apxm block stripped per request |

tau2 runner config (identical across arms):

```
tau2 run \
  --domain telecom \
  --agent-llm openai/gpt-oss-120b \
  --user-llm openai/gpt-oss-120b \
  --num-trials 1 \
  --num-tasks 20 \
  --seed 300 \
  --save-to .apxm/evaluation/agentic/<TS>-telecom-n20-powered/<arm>/results.json
```

Same env prelude as the N=5 telecom cell (unset AMD-env defaults
for `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL`; set local
`OPENAI_API_BASE` / `OPENAI_BASE_URL` to the HAL endpoint).

## Honest-negative discipline (Plan 00 §6)

- If the equivalence branch fires, the N=5 caveat language is
  retracted ("directional signal that does not survive quadrupled
  power", same wording the airline arc used for its N=5 +0.20
  retraction).
- If the regression branch fires, the claim is shipped as an
  honest-negative wall-time regression — the **first**
  load-bearing apxm-on regression in the Plan 05 evidence stack —
  and a Plan 08 / dispatch-overhead investigation ticket is
  opened. The mechanism dig from the N=5 cell becomes the
  starting hypothesis (KV-routing nudge → token-sample divergence
  → extra turns → larger context per call → slower per call).
- If the secondary metric (pass-at-1) shifts unfavorably with CI
  excluding 0, that is an additional honest-negative claim
  shipped alongside the primary-metric finding, regardless of
  which primary branch fires.
- The indeterminate branch produces a "preserved-for-audit" cell
  with no shipped claim — only a notice in the master plan that
  the powered follow-up was indeterminate and an N=50 escalation
  is queued.

## Linked plans and predecessors

- [`.apxm/docs/plans/05-agentic-task-quality.md`](../../.apxm/docs/plans/05-agentic-task-quality.md)
- [`.apxm/docs/plans/00-evaluation-methodology.md`](../../.apxm/docs/plans/00-evaluation-methodology.md)
- [`.apxm/docs/claims/agentic-accuracy-telecom-n5.md`](../../.apxm/docs/claims/agentic-accuracy-telecom-n5.md)
  (the N=5 predecessor cell this run is powered to resolve)
- [`.apxm/docs/claims/plan04-cross-system.md`](../../.apxm/docs/claims/plan04-cross-system.md)
  (the ShareGPT regression — same wall-time directional shape;
  if this run's regression branch fires, the two findings become
  a coherent two-workload regression pattern motivating Plan 08
  protocol design)
- [`tools/external/_apxm-notes/tau2-bench.APXM-INTEGRATION.md`](../../tools/external/_apxm-notes/tau2-bench.APXM-INTEGRATION.md)
- [`tools/hal_adapter/README.md`](../../tools/hal_adapter/README.md)
