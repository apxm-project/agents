# Pre-registration

> Commit this file BEFORE the measurement run starts. Post-hoc edits invalidate
> any goodput / SLO claim derived from the run. See
> `docs/plans/00-evaluation-methodology.md` §5 for the discipline this enforces.

## Run identity

- Run name (slug): `<RUN_NAME>`
- Author: `<HUMAN_NAME>`
- Date (UTC, ISO 8601): `<YYYY-MM-DDTHH:MM:SSZ>`
- Linked claim file (if known): `docs/claims/<NAME>.md`

## Workload(s) and rationale

- Workload(s): `<e.g. mooncake-trace + apxm-pfx-cancel>`
- Why this mix: `<one or two sentences justifying the choice with respect to the
  hypothesis below; reference Plan 00 §6 if the workload exercises a specific
  trap (cross-instance prefix sharing, parallel branches with cancellation, etc.)>`

## Hypothesis under test

- Form: `On <workload>, APXM-on reduces <primary_metric> P95 by ≥ <X>% vs
  flat-HTTP at <RPS> on <model> running on the model zoo, with bootstrap-CI
  lower bound on the ratio strictly < 1.0 (or > 1.0 for cache-reuse metrics).`
- Concrete value: `<fill in>`

## Pre-registered SLOs

- TTFT P95 SLO: `<MS> ms`
- TPOT P95 SLO: `<MS> ms`
- DAG critical-path P95 SLO: `<MS> ms`
- (Optional) Joint-SLO definition for goodput: `<both above must be met>`

## Metric tier

- Primary metric (the claim hangs on this one): `<metric>`
- Secondary metrics (reported but not claim-bearing): `<metric, metric, ...>`
- Decision rule: `<e.g. "ship the claim iff the primary metric's bootstrap-CI
  excludes 1.0 in the favorable direction AND Wilcoxon p < 0.05">`

## Power and thermal thresholds

- GPU clock lock script: `tools/bench/isolate.sh` (planned) — vendor
  CLI sets the highest performance level for the duration of the run.
- Thermal soak between cells: `5 minutes`
- Reject-run threshold: ambient `> <DEG_C>` at cell start
- Power sampling interval: `100 ms` (Wave 2 / Plan 04)

## GPU-hour budget

- Estimate: `<HOURS>` GPU-hours total across all cells × seeds
- Hard cap: `<HOURS>` GPU-hours (abort run if exceeded)
- Cluster reservation reference: `<SLURM_RESERVATION_NAME>` or
  `<JOB_ID>`

## Build identity (filled by the harness, do NOT edit before run)

- APXM SHA: `<filled in by manifest>`
- vLLM-fork SHA: `<filled in by manifest>`
- Image: `<filled in by manifest>`
- Zoo snapshot ref: `<filled in by manifest>`
