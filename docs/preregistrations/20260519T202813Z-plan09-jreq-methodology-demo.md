# Pre-Registration: Plan 09 J/req Methodology Demonstration on τ²-Bench Retail N=5 (2026-05-19)

**Authored: 2026-05-19T20:28:13Z**
**Scope: descriptive single-cell paired J/req measurement — NOT a comparative hypothesis test.**

## Why this is a methodology demonstration, not a benchmark

The retail N=5 pass-at-1 result is already an exact tie at 0.80=0.80
([`agentic-accuracy-retail-n5.md`](../claims/agentic-accuracy-retail-n5.md)).
Re-running the same cell with rocm-smi power capture does not add
quality evidence; it adds **the J/req column the Plan 09 INT pack
needs to close its single remaining load-bearing gap**
([`rack-scale-int-preliminary.md`](../claims/rack-scale-int-preliminary.md)
Path-to-pack item #4).

The output of this cell is **two J/req numbers and their ratio**,
reported descriptively. No bootstrap CI, no decision branches, no
equivalence band. This is a measurement-feasibility claim, not a
comparison-of-arms claim.

## Wire / image / service (locked before run)

- Service: `vllm-gptoss` (Slurm job 57976, host b05u13)
- vLLM image: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
- Model: `gpt-oss-120b` (TP=8 on 1 × MI300X node, max-num-seqs=64,
  scheduling-policy=priority, --enable-prefix-caching)
- HAL adapter v1: `hal-adapter-v1-2026-05-19`
- Tau2 pin: `0ed8c0ef0a1f6b024a0fb733e186922411874879`
- Domain: retail, N=5, seed=300, num-trials=1
- Same wire as the existing retail N=5 cell at
  `.apxm/evaluation/agentic/20260519T125000Z-retail-n5/`

## What gets measured

1. **Per-arm joules consumed**: rocm-smi sidecar samples
   `total_watts` every 2s across all 8 MI300X cards while the arm
   runs (HAL adapter → tau2 → vLLM upstream). Trapezoidal
   integration → arm-aggregate joules.
2. **Tasks completed per arm**: tau2 reports N=5 paired tasks; arm
   completes all 5 (no early termination).
3. **J/req per arm**: joules ÷ 5 tasks.
4. **Wall window per arm**: seconds between sidecar start and stop.

The sidecar window covers HAL adapter startup + tau2 evaluation +
HAL adapter shutdown. We do NOT subtract idle baseline — the J/req
number is the FULL cell cost as observed at the rocm-smi layer.

## What gets reported

Single CSV row + JSON record per arm:

| arm | n_tasks | total_joules | joules_per_task | mean_watts | sample_count | window_seconds |

Plus the **descriptive ratio** `apxm-on.J / flat-http.J`. The ratio
is reported without significance testing because:
- N=5 is too small for paired-bootstrap CI on power
- Single-trial-per-task design means we have 1 power trace per arm,
  not 5
- The point is methodology feasibility, not an arm comparison

## What is NOT being claimed

- **Not claiming "APXM is more/less energy-efficient than flat-HTTP".**
  N=5 paired with a single power trace per arm is descriptively
  informative but not inferentially powered for an energy comparison.
  Any claim of that form needs a separately pre-registered cell
  at higher N with multiple trials.
- **Not claiming "this is the J/req number for production deploys".**
  We are at TP=8 single-node, single-tenant, fixed model. Production
  deploys vary across all of those.
- **Not claiming any quality result.** Pass-at-1 is already shipped
  at this design point; this run does not reproduce the quality
  claim (descriptive only).

## Honest negatives anticipated

- **rocm-smi --overlap may fail mid-run** if the kernel driver
  goes into a low-power state. Sidecar handles this by skipping the
  sample and retrying next interval. If >50% of samples fail, the
  J/req number is unreliable and we report "methodology requires
  driver tuning" instead.
- **Idle power dominates short runs.** If retail N=5 takes ~3 min
  per arm and idle is 1.52 kW, the per-arm joule budget is ~270 kJ
  with maybe 10-20% above idle from actual generation. J/req is
  high-variance for short cells. We report it anyway as descriptive.
- **The two arms run sequentially, not in parallel**, so background
  load (other users on the cluster, kernel updates, etc.) could
  drift between arm 1 and arm 2. Service-state and timestamps are
  captured for both arms.

## Stop conditions

- If the service becomes unhealthy mid-run: kill HAL, stop sidecar,
  report partial result.
- If sidecar samples < 30 per arm (i.e., < 60s of capture): note
  in claim as insufficient sampling, do not promote into INT pack.
- If pass-at-1 < 0.6 on either arm: regression — investigate before
  shipping (existing retail N=5 claim is 0.80).

## Why "demo" not "full INT-pack J/req column"

A real INT-pack J/req column needs:
- Multiple measurement cells (1× Plan 01 + 3× Plan 04 cells + 3× Plan 05 cells)
- Per-cell paired J/req with bootstrap CI
- Equivalence-band decision per cell

This cell ships ONE measurement cell. The INT pack will still
require the remaining 6 cells; this proves the methodology and
gives ONE data point to add to the INT three-regime map.
