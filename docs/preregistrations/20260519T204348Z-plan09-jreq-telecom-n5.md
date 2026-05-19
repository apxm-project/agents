# Pre-Registration: Plan 09 J/req Cell — τ²-Bench Telecom N=5 (2026-05-19)

**Authored: 2026-05-19T20:43:48Z**
**Scope: descriptive single-cell paired J/req measurement — NOT a comparative hypothesis test.**

This is the **second** cell of the Plan 09 INT-pack J/req column,
mechanically replicating the methodology proven by the retail N=5
demo (pre-reg `c1cd69f6`,
[claim](../../.apxm/docs/claims/plan09-jreq-methodology-demo-retail-n5.md)).

## Why this is a mechanical replication, not new methodology

The retail N=5 demo proved the rocm-smi-sidecar + integrate-joules
pipeline produces usable per-arm energy data on a real agentic
workload. This cell ships the next data point on the J/req column
on a different τ²-bench domain (telecom) at the same N=5/seed=300
design point.

The shipped τ²-bench telecom N=5 claim
([`agentic-accuracy-telecom-n5.md`](../../.apxm/docs/claims/agentic-accuracy-telecom-n5.md))
is pass-at-1 0.800 = 0.800 (4/5 tie) with a +31% wall-time
directional caveat on apxm-on that was later **demoted to
within-variance noise** by the N=50 escalation arc
(claim `agentic-accuracy-telecom-n50-escalation.md`). This cell
re-runs the same N=5 design point to add J/req — pass-at-1
reproducibility is a sanity check, not a new claim.

The output of this cell is **two J/req numbers and their ratio**,
reported descriptively. No bootstrap CI, no decision branches, no
equivalence band.

## Wire / image / service (locked before run)

- Service: `vllm-gptoss` (Slurm job 57976, host b05u13)
- vLLM image: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
- Model: `gpt-oss-120b` (TP=8 on 1 × MI300X node, max-num-seqs=64,
  scheduling-policy=priority, --enable-prefix-caching)
- HAL adapter v1: `hal-adapter-v1-2026-05-19`
- Tau2 pin: `0ed8c0ef0a1f6b024a0fb733e186922411874879`
- Domain: telecom, N=5, seed=300, num-trials=1
- Same wire as the existing telecom N=5 cell at
  `.apxm/evaluation/agentic/20260519T125000Z-telecom-n5/`

## What gets measured

1. **Per-arm joules consumed**: rocm-smi sidecar samples
   `total_watts` every 2s across all 8 MI300X cards while the arm
   runs (HAL adapter → tau2 → vLLM upstream). Trapezoidal
   integration → arm-aggregate joules.
2. **Tasks completed per arm**: tau2 reports N=5 paired tasks.
3. **J/req per arm**: joules ÷ 5 tasks.
4. **Wall window per arm**: seconds between sidecar start and stop.

The sidecar window covers HAL adapter startup + tau2 evaluation +
HAL adapter shutdown. We do NOT subtract idle baseline.

## What gets reported

Single CSV row + JSON record per arm:

| arm | n_tasks | total_joules | joules_per_task | mean_watts | sample_count | window_seconds |

Plus the **descriptive ratio** `apxm-on.J / flat-http.J`. The ratio
is reported without significance testing for the same reasons as
the retail N=5 demo.

## What is NOT being claimed

- **Not claiming "APXM is more/less energy-efficient than flat-HTTP".**
  N=5 paired with a single power trace per arm is descriptively
  informative but not inferentially powered.
- **Not claiming "this is the J/req number for production deploys".**
- **Not claiming any quality result.** Pass-at-1 is already shipped
  for telecom N=5; this run reproduces it descriptively only.
- **Not re-opening the telecom regression question.** The N=5 +31%
  wall-time directional signal was closed by the N=50 escalation
  arc; any wall-time drift in this run is within the variance the
  N=50 cell already characterized.

## Honest negatives anticipated

- **Idle power dominates short runs.** Per the retail N=5 demo, a
  ~180s per-arm window at ~3.07 kW under load (~50% above 1.52 kW
  idle baseline) gives ~540 kJ total per arm. Generation-attributable
  energy is a smaller fraction.
- **The two arms run sequentially**, so the second arm may see
  warmer prefix-cache state. For descriptive J/req this is
  acceptable; the retail N=5 demo showed only 0.12% ratio difference,
  well within any plausible cache-warmth bias.
- **rocm-smi --overlap may fail mid-run.** Sidecar handles by
  skipping and retrying. If >50% of samples fail, J/req is
  unreliable.

## Stop conditions

- If the service becomes unhealthy mid-run: kill HAL, stop sidecar,
  report partial result.
- If sidecar samples < 30 per arm (i.e., < 60s of capture): note
  in claim as insufficient sampling, do not promote into INT pack.
- If pass-at-1 deviates from the shipped 0.800 by more than ±0.40
  (i.e., a 2-of-5 swing): note as a service-drift signal but ship
  the J/req number anyway with a transparency caveat. Reproducibility
  expectation is exact 0.800 = 0.800 per the shipped claim.

## Why "cell #2" not "full INT-pack J/req column"

A full INT-pack J/req column needs the remaining 5 cells after this
one: 1× Plan 01 pin_demo + 3× Plan 04 (Mooncake/ShareGPT/LooGLE) +
1× Plan 05 (airline N=5). This cell ships the second data point;
the orchestrator pattern is identical.
