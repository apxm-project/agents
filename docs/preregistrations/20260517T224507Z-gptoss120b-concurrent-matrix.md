# Pre-registration

> Commit this file BEFORE the measurement run starts. Post-hoc edits invalidate
> any goodput / SLO claim derived from the run.

## Run identity

- Run name (slug): `20260517T224507Z-gptoss120b-concurrent-matrix`
- Author: `raherrer`
- Date (UTC, ISO 8601): `2026-05-17T22:45:07Z`
- Linked claim file (planned): `docs/claims/gptoss120b-concurrent-matrix.md`

## Workload(s) and rationale

- Workload: `pin_demo` cohort (the workload INT-03 used; produces measurable
  pin behaviour under constrained KV at c=32).
- Why: the matrix's load-bearing axis is APXM-on vs flat-HTTP under prefix-
  cache cold/warm conditions. `pin_demo` is the only checked-in workload
  with a demonstrated pin-engagement signal at this concurrency
  (`pinned_blocks_peak_max ≥ 1` was observed in the INT-03 paired runs at
  the same concurrency). Cross-instance prefix sharing
  is not exercised here — this matrix is single-service single-model.

## Hypothesis under test

- Form: On `pin_demo` at c=32, APXM-on reduces `batch_wall_ms` P95 by
  ≥ 15% vs flat-HTTP (`--no-apxm-hints` arm) on gpt-oss-120b running on
  `vllm-gptoss` (Slurm job 48711, port 8916), with bootstrap-CI on the
  paired ratio strictly < 1.0 in at least one of cells C (cold) or D
  (warm). The flat-HTTP arm and the APXM-on arm run against the same
  vLLM binary (same `apxm-vllm-runtime:a0e42ad3-e4e3a9af-post-rebase`
  image, same `--enable-prefix-caching` setting, same
  `--scheduling-policy priority`) — only the request body and graph
  registration differ.
- Concrete value: ≥ 15% P95 reduction in at least one APXM-on cell.

## Pre-registered SLOs

- TTFT P95 SLO: not gating this claim (workload is batch-style).
- TPOT P95 SLO: not gating this claim.
- DAG critical-path P95 SLO: not gating this claim.
- Joint-SLO definition for goodput: not used (primary metric is
  `batch_wall_ms` directly).

## Metric tier

- Primary metric (the claim hangs on this one): `batch_wall_ms` (cell-
  level, paired across iterations).
- Secondary metrics (reported but not claim-bearing):
  `pinned_blocks_peak_max`, `prefix_cache_hit_rate`,
  `prefix_cache_queries_delta`, `prefix_cache_hits_delta`.
- Decision rule: ship the claim iff the primary metric's paired
  bootstrap-CI on the APXM/baseline ratio excludes 1.0 in the favorable
  direction (ratio < 1.0) in at least one of cells C or D, AND the
  paired Wilcoxon p < 0.05. **Honest negatives** (cells where APXM did
  not win) must be reported in the claim file alongside the wins.

## Power and thermal thresholds

- GPU clock lock script: **CAVEAT — privileged setperflevel access is
  not available inside the apxm-vllm-runtime container on this cluster
  configuration. `rocm-smi --setperflevel high` runs without error but
  silently no-ops; perflevel remains `auto`.** Since the same auto-mode
  applies to ALL cells (within-run uniformity), the A/B comparison is
  internally consistent. Cross-run cluster-wide comparability with
  claims requiring locked clocks is best-effort.
- Thermal soak between cells: rely on the harness's existing inter-cell
  pause; no explicit soak applied (perflevel was not changed, so no
  state-change to settle).
- Reject-run threshold: GPU junction temp > 80°C at cell start. At
  pre-flight (2026-05-17T22:43Z), junctions were 40-43°C, memory
  34-36°C across all 8 GPUs — well under the reject threshold.
- Power sampling interval: not collected this run; J/req rocm-smi sampling is out of scope.

## GPU-hour budget

- Estimate: 0.7 GPU-hours total (4 cells × ~10 min/cell × 1 node ×
  8 GPUs); allowing 25% headroom for cold-cache warmup.
- Hard cap: 1.5 GPU-hours (abort run if exceeded).
- Cluster reservation reference: Slurm job ID `48711` (currently
  R, 2:10:24 remaining of 4:00:00 wall-time limit, host `a04u07`).

## Build identity (filled by the harness, do NOT edit before run)

- APXM SHA: `<filled in by manifest>`
- vLLM-fork SHA: `<filled in by manifest>` (rebased to v0.21.0)
- Image: `apxm-vllm-runtime:a0e42ad3-e4e3a9af-post-rebase`
- Zoo snapshot ref: `.apxm/deploy/20260517T205404Z/zoo-snapshot.json`

## Matrix design (referenced from the plan)

|  | Cold prefix cache | Warm prefix cache |
|---|---|---|
| **APXM-off** (`--no-apxm-hints`, flat-HTTP) | Cell A | Cell B |
| **APXM-on** (graph hints + pin policy) | Cell C | Cell D |

- Concurrency: 32 paired iterations per cell.
- Both arms run against the same vLLM binary; the difference is the
  request body (`vllm_xargs.apxm` block presence) and whether
  `/v1/apxm/graphs/register` is called.
- Cell isolation: cold cells are preceded by
  `POST /v1/apxm/admin/reset_prefix_cache`. The exact mechanism
  (in-harness flag or manual curl) will be recorded in the run
  manifest after the first run.
