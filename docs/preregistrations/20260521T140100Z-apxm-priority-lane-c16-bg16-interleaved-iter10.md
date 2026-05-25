# APXM Priority-Lane C16/BG16 Interleaved Fresh-Service Repeat

Date: 2026-05-21 14:01:00 UTC

## Purpose

This run strengthens the accepted APXM priority-lane result by increasing the
number of independent interleaved batches from 5 to 10 on a freshly launched
`vllm-gptoss` service.

The goal is not to broaden the claim. The claim remains:

> Under backend queue contention, APXM-on reduces user-visible critical-lane
> finish time compared with flat-HTTP on the same APXM-vLLM service.

This run additionally reports batch-level sensitivity and tradeoff metrics via
`tools/scripts/analyze_apxm_priority_lane_run.py`.

## Locked Setup

- Workload: `examples/python/benchmarks/workloads/apxm_priority_lane.py`
- Runner: `tools/scripts/run_apxm_priority_lane_interleaved.sh`
- Service: `vllm-gptoss`
- Service job: fresh Dekk zoo allocation started from
  `deploy/vllm/zoo.review-gptoss.toml`
- Image: `apxm-vllm-runtime:prioritylane-33c1855-490aaad0-dirty`
- Model: `gpt-oss-120b`
- Endpoint inside service allocation: `http://127.0.0.1:8916`
- Scheduler policy: `priority`
- Focus node id: `4` (`critical_user_answer`)
- Arms:
  - APXM-on: graph registration and `vllm_xargs.apxm` hints enabled
  - flat-HTTP: same server, `--no-apxm-hints`
- Arm order:
  - `FIRST_ARM=flat`
  - `ALTERNATE_ORDER=1`
  - odd iterations run flat then APXM
  - even iterations run APXM then flat
- Optimization level: `-O2` for both arms
- `ITER=10`
- `CONC=16`
- `PREFIX_TOK=1024`
- `BG_FANOUT=16`
- `CRITICAL_MAX_TOKENS=64`
- `BACKGROUND_MAX_TOKENS=256`
- `STAGGER_MS=0`

## Primary Metric

Primary metric: `focus_node_finish_ms` in the stitched per-tenant CSVs,
paired by `(iteration, variant)`.

## Promotion Gates

The run promotes only if:

- paired rows are present;
- zero failed tenants in both arms;
- APXM-on has zero dispatch fallback tenants;
- APXM-on honored fields include `priority`;
- focus-node timing is present in both arms;
- bootstrap 95% CI upper bound for mean `APXM / flat` focus-node finish
  ratio is `< 0.95`;
- bootstrap 95% CI upper bound for p95 `APXM / flat` focus-node finish ratio
  is `< 0.90`;
- absolute p95 win is at least `1000 ms`.

Secondary publication-strengthening checks:

- batch-level focus-mean APXM/flat ratio and CI;
- batch-wall APXM/flat ratio and CI;
- tenant focus win rate;
- per-iteration tradeoff table.

If the primary gates pass but batch-level CI crosses `1.0`, the result remains
a tenant-level priority-lane confirmation and the batch-level finding is
reported as a publication caveat. If any tenant fails, the service crashes, or
the endpoint refuses requests, reject the run instead of turning it into a
latency claim.

## Command

```bash
SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
PRE_REG=docs/preregistrations/20260521T140100Z-apxm-priority-lane-c16-bg16-interleaved-iter10.md \
ITER=10 \
CONC=16 \
PREFIX_TOK=1024 \
BG_FANOUT=16 \
CRITICAL_MAX_TOKENS=64 \
BACKGROUND_MAX_TOKENS=256 \
FOCUS_NODE_ID=4 \
STAGGER_MS=0 \
FIRST_ARM=flat \
ALTERNATE_ORDER=1 \
TS=20260521T140100Z-c16-bg16-interleaved-iter10 \
tools/scripts/run_apxm_priority_lane_interleaved.sh
```
