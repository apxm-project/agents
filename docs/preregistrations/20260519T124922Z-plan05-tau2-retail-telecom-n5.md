# Pre-registration

> Commit this file BEFORE the measurement runs start. Post-hoc edits invalidate
> any agentic-accuracy claim derived from the runs. Follows
> [Plan 00 — Evaluation Methodology Charter](../../.apxm/docs/plans/00-evaluation-methodology.md)
> and [Plan 05 — Agentic Task Quality](../../.apxm/docs/plans/05-agentic-task-quality.md).

## Run identity

This is a **two-cell cross-domain coverage pre-reg**, registered as one
file because the two cells share workload runner, wire shape, service,
and decision rule — they differ only in `--domain` and seed.

- Cell A slug: `20260519T125000Z-plan05-tau2-retail-n5`
- Cell B slug: `20260519T125000Z-plan05-tau2-telecom-n5`
- Author: `apxm`
- Date (UTC, ISO 8601): `2026-05-19T12:49:22Z`
- Linked claim files (planned, one per cell):
  - `.apxm/docs/claims/agentic-accuracy-retail-n5.md`
  - `.apxm/docs/claims/agentic-accuracy-telecom-n5.md`

## Workloads and rationale

- Benchmark: `tau2-bench` (vendored at `tools/external/tau2-bench/`,
  pin sha `0ed8c0ef0a1f6b024a0fb733e186922411874879`).
- **Cell A — Retail**: 5 tasks from
  `tools/external/tau2-bench/data/tau2/domains/retail/tasks.json`
  (114 tasks total in the vendored suite; tau2 takes the first
  `--num-tasks` deterministically at the given seed).
- **Cell B — Telecom**: 5 tasks from
  `tools/external/tau2-bench/data/tau2/domains/telecom/tasks.json`
  (2285 tasks total; same deterministic-prefix selection).
- One trial per task per cell. Each cell is therefore 5 paired binary
  observations.
- **Rationale**: These two cells extend Plan 05's cross-domain
  coverage beyond airline. The airline domain is now fully resolved
  at N=50 (`agentic-accuracy-airline-n50.md`, equivalence finding).
  Per the Plan 05 charter, defending any "APXM does not change
  τ²-bench task quality" claim beyond a single domain requires
  per-domain replication. Retail + telecom are the remaining two
  domains in the vendored tau2 suite (banking_knowledge + mock are
  out of scope — banking_knowledge is QA-only, mock is for
  integration testing).
- These N=5 cells are **methodology cells**, not powered cells. The
  airline arc demonstrated that N=5 is the minimum viable scale for
  pre-registration discipline + HAL wire-up validation per domain;
  larger-N escalations on retail / telecom are deferred to a
  separate pre-reg if either cell shows a directional signal worth
  resolving.

## Hypothesis under test

For each cell independently, with the same seed in both arms, the
paired difference apxm-on `pass_at_1` − flat-http `pass_at_1` falls
into exactly one of three claim-bearing outcomes:

1. **Equivalence claim**: 95% bootstrap CI on the paired diff is
   **fully inside** the pre-registered ±0.20 equivalence band.
2. **Directional signal**: lower CI bound > 0 (apxm-on favorable) OR
   upper CI bound < 0 (apxm-on unfavorable). At N=5 this only
   triggers a *follow-up escalation pre-reg*, not a final
   directional claim — N=5 is too underpowered to support a final
   directional reading per the airline arc precedent.
3. **Indeterminate-at-N=5**: CI spans 0 AND extends outside ±0.20.
   Ship as an indeterminate methodology cell with explicit
   "requires escalation" flag.

Decision rule is fully enumerated above — no post-hoc choice of
decision boundary is allowed.

## Pre-registered SLOs

- Pass-at-1 SLO: equivalence band ±0.20 (kept identical to the
  airline-arc pre-regs so airline + retail + telecom are directly
  comparable).
- TTFT / TPOT P95 SLOs: not gating these claims.
- DAG critical-path P95 SLO: not used.

## Metric tier

- Primary metric: `pass_at_1` per arm at N=5 per cell, plus the
  **paired difference** apxm-on − flat-http on identical task IDs
  per cell.
- Secondary metrics: `mean_steps`, `mean_tool_calls`,
  `mean_duration_s`, `db_match_rate`, `termination_reasons`,
  `agent_cost`, `user_cost`. At N=5 these are not claim-bearing.
- Cost note: same as airline cells — locally-served `gpt-oss-120b`
  has no LiteLLM price entry, so `agent_cost` / `user_cost` are
  expected near-zero and not interpreted.

## Pin-engagement gate

Not load-bearing for these claims. HAL probes captured per arm per
cell for audit.

## Power and thermal thresholds

Auto-mode clocks (uniformly across all arms). Reject-run threshold:
"vLLM service unhealthy at arm start". Service is the same long-lived
`vllm-gptoss` (job 54322) used for the airline arc; no engine restart
required between cells.

## GPU-hour budget

- Estimate: ~0.5 GPU-hours total (2 cells × 2 arms × ~3 min/arm at
  N=5 ≈ 12 min wall; airline N=5 cells ran in 6-7 min each).
- Hard cap: 1.0 GPU-hours.
- Cluster reservation reference: Slurm job `54322` for `vllm-gptoss`
  on host `a04u07` (~3h remaining at pre-reg commit; fits trivially).

## Build identity

- APXM SHA: working tree head as of pre-reg commit.
- vLLM-fork SHA: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
  on job 54322 (identical to airline arc cells — image / service /
  wall window all continuous).
- HAL adapter SHA: `hal-adapter-v1-2026-05-19`.

## Matrix design

Per cell, identical wire setup to airline cells; only `--domain` and
`--graph-context` vary:

| Cell | Arm | HAL adapter args | Wire shape |
|---|---|---|---|
| A (retail) | `apxm-on` | `--backend apxm-on --inject-apxm --graph-context tau2-retail-n5` | apxm injected |
| A (retail) | `flat-http` | `--backend flat-http` | apxm stripped |
| B (telecom) | `apxm-on` | `--backend apxm-on --inject-apxm --graph-context tau2-telecom-n5` | apxm injected |
| B (telecom) | `flat-http` | `--backend flat-http` | apxm stripped |

tau2 runner config (identical across arms within each cell):

```
tau2 run \
  --domain {retail|telecom} \
  --agent-llm openai/gpt-oss-120b \
  --user-llm openai/gpt-oss-120b \
  --num-trials 1 \
  --num-tasks 5 \
  --seed 300 \
  --save-to .apxm/evaluation/agentic/<TS>-<domain>-n5/<arm>/results.json
```

Same env prelude as airline cells (unset upstream `OPENAI_BASE_URL` etc.).

## Honest-negative discipline

- Each cell ships an independent claim file — neither cell's outcome
  supersedes the other or the airline cells. The airline-n50
  equivalence finding remains the substantive airline claim; retail
  and telecom cells stand or fall on their own decision-rule
  outcomes.
- If either cell hits the directional-signal branch (CI excludes
  zero), the appropriate response is to author a separate larger-N
  pre-reg for that domain — not to draw a final directional reading
  from N=5.
- If either cell hits indeterminate-at-N=5, ship as such and flag
  the escalation as the next required step for that domain only.

## Linked plans

- [`.apxm/docs/plans/05-agentic-task-quality.md`](../../.apxm/docs/plans/05-agentic-task-quality.md)
- [`.apxm/docs/plans/00-evaluation-methodology.md`](../../.apxm/docs/plans/00-evaluation-methodology.md)
- [`.apxm/docs/claims/agentic-accuracy-airline-n50.md`](../../.apxm/docs/claims/agentic-accuracy-airline-n50.md)
  (the equivalence finding these cells extend cross-domain)
- [`tools/external/_apxm-notes/tau2-bench.APXM-INTEGRATION.md`](../../tools/external/_apxm-notes/tau2-bench.APXM-INTEGRATION.md)
- [`tools/hal_adapter/README.md`](../../tools/hal_adapter/README.md)
