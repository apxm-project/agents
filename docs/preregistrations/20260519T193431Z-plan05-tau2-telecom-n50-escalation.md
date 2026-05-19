# Pre-registration

> Commit this file BEFORE the measurement run starts. Post-hoc edits invalidate
> any agentic-accuracy claim derived from the run. Follows
> [Plan 00 — Evaluation Methodology Charter](../../.apxm/docs/plans/00-evaluation-methodology.md)
> and [Plan 05 — Agentic Task Quality](../../.apxm/docs/plans/05-agentic-task-quality.md).

## Run identity

- Run name (slug): `20260519T193431Z-plan05-tau2-telecom-n50-escalation`
- Author: `apxm`
- Date (UTC, ISO 8601): `2026-05-19T19:34:31Z`
- Linked claim file (planned): `.apxm/docs/claims/agentic-accuracy-telecom-n50-escalation.md`

## Workload and rationale

- Benchmark: `tau2-bench` (vendored at `tools/external/tau2-bench/`,
  pin sha `0ed8c0ef0a1f6b024a0fb733e186922411874879`).
- Domain: `telecom`, default task set (`tasks.json`, 2285 sampled
  tasks); same domain and same wire shape as the N=5 + N=20 telecom
  predecessor cells.
- Sub-sample: `--num-tasks 50 --seed 300` (tau2's seeded sampler
  picks 50 from `tasks.json`). Same seed in both arms so identical
  task subsets land in each arm.
- One trial per task (`--num-trials 1`). The pair is therefore 50
  paired observations on the primary metric (continuous wall-time
  per task) and 50 paired binary observations on the secondary
  metric (pass / fail).
- **Note on cross-N nesting:** tau2's seeded sampler does NOT
  produce nested task subsets across `--num-tasks` values (verified
  empirically — N=5 and N=20 share only 1 task ID at the same
  seed=300). The within-cell paired comparison remains valid; the
  cross-N comparison (N=5 / N=20 / N=50 ratios) is informal and
  treated as descriptive in the claim, not inferential.

## Motivation — why this run, why now

The N=20 powered follow-up
(`.apxm/docs/claims/agentic-accuracy-telecom-n20-powered.md`,
pre-reg `cc975479`) shipped on 2026-05-19 as **branch (d)
indeterminate** on the primary metric: paired wall-time ratio
**1.226× flat-http**, 95% bootstrap CI **[0.992, 1.532]** —
CI lower bound (0.992) dips just below the equivalence-low
boundary (0.909), CI upper bound (1.532) is well above the
equivalence-high boundary (1.10). Per the N=20 pre-reg's branch (d)
instruction:

> "**(d) Indeterminate:** CI crosses an equivalence-band boundary
> → ship as 'indeterminate at N=20, equivalence band not excluded,
> regression band not excluded' and queue an N=50 (full
> `tasks.json` first-50) follow-up."

This run **is** that escalation.

The N=20 cell **did** validate the N=5 mechanism reading at 4× power:
4 of 6 paired ratios (steps 1.11, tool_calls 1.19, in_tokens 1.18,
out_tokens 1.19, per-call gen latency 1.12, mean_duration 1.23)
were within 0.02 of the N=5 ratios — ruling in "extra turns + larger
context per call" mechanism and ruling out "pure dispatch overhead".
The open question for this N=50 escalation is therefore narrowly
**"is the wall-time gap real at the ±10% equivalence band, given
the mechanism has already been established"** — not "what is the
mechanism".

## Hypothesis under test

- **Primary metric form (this run):** paired **log-ratio** of
  per-task `mean_duration_s`:
  `delta_i = ln(apxm_on.duration_i / flat_http.duration_i)`
  averaged across the N=50 paired tasks. Log-ratio (not raw diff)
  so the band is scale-invariant — identical choice to the N=20
  pre-reg.
- **Equivalence band:** ±10% (`[ln(0.9091), ln(1.10)] = [-0.0953, +0.0953]`)
  — identical to N=20 pre-reg.
- **Decision rule on the primary metric (95% bootstrap CI on the
  mean of paired log-ratios, B=10000, seed=1234):**
  - **(a) Equivalence:** CI **entirely inside** [-0.0953, +0.0953]
    → ship as equivalence claim ("telecom wall-time within ±10%
    band at N=50; the N=5 +31% and N=20 +22.6% directional
    signals do not survive escalated power; both predecessor
    caveats are retracted").
  - **(b) Regression confirmed:** CI lower bound **> +0.0953**
    → ship as honest-negative regression claim ("telecom wall-time
    is X% slower on apxm-on at N=50 with CI [lower, upper];
    confirms the N=5 + N=20 caveats; opens Plan 08 follow-up").
    The N=20 indeterminate cell is then promoted from
    "indeterminate" to "consistent with the confirmed regression";
    its file is retained for provenance but the substantive claim
    is the N=50 finding.
  - **(c) Improvement:** CI upper bound **< -0.0953** → ship as a
    superiority claim (extremely unlikely given the consistent
    N=5 + N=20 directional signal of apxm-on being slower).
  - **(d) Indeterminate at N=50:** CI still crosses an
    equivalence-band boundary → ship as "indeterminate at N=50"
    AND **stop the telecom escalation arc**. Two consecutive
    indeterminate findings at 1× → 4× → 10× nominal power on the
    same wire signal indicates the underlying per-task wall-time
    variance is too large to resolve with paired-N≤50 designs on
    this workload, and the right next move is a different design
    (e.g. per-task multi-trial averaging within a smaller N, or
    a different workload). No N=100 escalation. The methodology
    decision is locked: the indeterminate finding is itself the
    shipped claim (with the explicit "telecom wall-time is too
    high-variance at single-trial-per-task to resolve within ±10%
    band at this power level" framing).

- **Secondary metric (carried, not load-bearing):** paired
  difference apxm-on `pass_at_1` − flat-http `pass_at_1` on
  identical task IDs. Equivalence band ±0.20 (kept for
  back-compat with the airline / retail / telecom-N=5 / telecom-N=20
  arc). At N=50 the bootstrap CI on this binary diff should be
  meaningfully tighter than the ±0.25 tail seen at N=20.

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
  generation_time_seconds mean. The mechanism decomposition table
  from the N=20 claim is extended to N=50 in the claim file.

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

- Estimate: ~1.5 GPU-hours total (wall). At N=50 tasks × 2 arms ×
  ~45 s/task wall mean (midpoint of N=20's 49.7 s apxm-on +
  40.0 s flat-http) ≈ 75 min benchmark wall; add HAL adapter
  startup (~2 × ~30 s) and probe (~2 × ~5 s) → ~80 min total.
  Equivalent to ~10 GPU-hours of MI300X compute on TP=8.
- Hard cap: 2.5 GPU-hours wall (≈20 GPU-hours on 8 GPUs).
- Cluster reservation reference: live `vllm-gptoss` allocation
  (Slurm job 57976 on host b05u13, started ~21:55 UTC 2026-05-19,
  4-hour wall, ~2.5 h remaining at this pre-reg commit time).

## Build identity (filled by the harness / captured at run-start)

- APXM SHA: `<filled at run-start>` (working tree head as of
  pre-reg commit will be recorded by the run-manifest).
- vLLM-fork image: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
  (same image as the airline + retail + telecom N=5 + telecom N=20
  arc — unchanged so the run is wire-comparable with all
  predecessor cells).
- HAL adapter SHA: `hal-adapter-v1-2026-05-19` (unchanged).
- tau2 pin SHA: `0ed8c0ef0a1f6b024a0fb733e186922411874879` (unchanged).
- Slurm job id: `57976` (same job as the N=20 cell — no service
  restart between N=20 and N=50, so same vLLM process + same
  in-memory KV state; this is intentional for build invariance).

## Matrix design

Identical to the N=20 telecom cell except `--num-tasks 50`:

| Arm | HAL adapter args | Wire shape |
|---|---|---|
| `apxm-on` | `--backend apxm-on --inject-apxm --graph-context tau2-telecom-n50-escalation` | `extra_body.vllm_xargs.apxm` injected per request |
| `flat-http` | `--backend flat-http` | apxm block stripped per request |

tau2 runner config (identical across arms):

```
tau2 run \
  --domain telecom \
  --agent-llm openai/gpt-oss-120b \
  --user-llm openai/gpt-oss-120b \
  --num-trials 1 \
  --num-tasks 50 \
  --seed 300 \
  --save-to .apxm/evaluation/agentic/<TS>-telecom-n50-escalation/<arm>/results.json
```

Same env prelude as the N=20 telecom cell (unset AMD-env defaults
for `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL`; set local
`OPENAI_API_BASE` / `OPENAI_BASE_URL` to the HAL endpoint).

## Honest-negative discipline (Plan 00 §6)

- If the equivalence branch (a) fires, the N=5 and N=20 caveats are
  retracted as a coherent pair ("directional signals that did not
  survive 10× escalated power").
- If the regression branch (b) fires, the claim is shipped as an
  honest-negative wall-time regression — the **first** load-bearing
  apxm-on regression in the Plan 05 evidence stack — and a Plan 08
  ticket is opened. The mechanism reading already validated at
  N=20 (extra turns + larger context per call, not pure dispatch
  overhead) is carried forward as the starting hypothesis.
- If the secondary metric (pass-at-1) shifts unfavorably with CI
  excluding 0, that is an additional honest-negative claim shipped
  alongside the primary-metric finding, regardless of which
  primary branch fires.
- If the indeterminate branch (d) fires at N=50, the methodology
  decision is locked per the decision rule above: **stop the
  telecom escalation arc; the indeterminate finding is itself the
  shipped claim with explicit "too-high-variance-to-resolve"
  framing.** No N=100 escalation.

## Linked plans and predecessors

- [`.apxm/docs/plans/05-agentic-task-quality.md`](../../.apxm/docs/plans/05-agentic-task-quality.md)
- [`.apxm/docs/plans/00-evaluation-methodology.md`](../../.apxm/docs/plans/00-evaluation-methodology.md)
- [`.apxm/docs/claims/agentic-accuracy-telecom-n5.md`](../../.apxm/docs/claims/agentic-accuracy-telecom-n5.md)
  (N=5 predecessor — exact pass-at-1 equivalence with +31%
  directional wall-time caveat)
- [`.apxm/docs/claims/agentic-accuracy-telecom-n20-powered.md`](../../.apxm/docs/claims/agentic-accuracy-telecom-n20-powered.md)
  (N=20 predecessor — indeterminate at 4× power; this run is its
  pre-registered escalation)
- [`.apxm/docs/claims/plan04-cross-system.md`](../../.apxm/docs/claims/plan04-cross-system.md)
  (the ShareGPT regression — same wall-time directional shape; if
  this run's regression branch (b) fires, the two findings become
  a coherent two-workload regression pattern motivating Plan 08
  protocol design)
- [`tools/external/_apxm-notes/tau2-bench.APXM-INTEGRATION.md`](../../tools/external/_apxm-notes/tau2-bench.APXM-INTEGRATION.md)
- [`tools/hal_adapter/README.md`](../../tools/hal_adapter/README.md)
