# APXM Priority-Lane C16/BG16 Fresh-Service Repeat 2

Date: 2026-05-21 23:38:17 UTC

## Purpose

This run is a second fresh-service repeat of the accepted APXM priority-lane
result. The goal is to strengthen the paper draft with another independent
service allocation.

The claim remains narrow:

> Under backend queue contention, APXM-on reduces user-visible critical-lane
> finish time compared with flat-HTTP on the same APXM-vLLM service.

Do not use this run to claim broad APXM speedup, total batch-wall generality,
or prefix-pinning benefit.

## Locked Setup

- Workload: `examples/python/benchmarks/workloads/apxm_priority_lane.py`
- Runner: `tools/scripts/run_apxm_priority_lane_interleaved.sh`
- Service: `vllm-gptoss`
- Service job: fresh Dekk zoo allocation from
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

Secondary paper-strengthening checks:

- batch-level focus-mean APXM/flat ratio and CI;
- batch-wall APXM/flat ratio and CI;
- tenant focus win rate;
- per-iteration tradeoff table;
- combined analysis against the earlier fresh-service repeat.

If the primary gates pass but batch-level CI crosses `1.0`, report the result
as tenant-level priority-lane evidence and retain the batch-level caveat. If
the service crashes, tenants fail, or APXM does not honor `priority`, reject
the run for latency claims.

## Command

```bash
SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
PRE_REG=docs/preregistrations/20260521T233817Z-priority-lane-repeat2.md \
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
TS=20260521T233817Z-c16-bg16-interleaved-repeat2 \
tools/scripts/run_apxm_priority_lane_interleaved.sh
```
