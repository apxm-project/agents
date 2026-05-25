# APXM Priority-Lane C16/BG16 Interleaved Confirmation Pre-Registration

Date: 2026-05-21 12:30:00 UTC

## Claim

This confirmation checks whether the promoted APXM priority-lane result
survives arm-order concerns by interleaving APXM-on and flat-HTTP one batch at
a time on the same APXM-vLLM service.

The substantive claim remains narrow: under backend queue contention, APXM-on
should reduce user-visible critical-lane finish time compared with flat-HTTP.

## Locked Setup

- Workload: `examples/python/benchmarks/workloads/apxm_priority_lane.py`
- Runner: `tools/scripts/run_apxm_priority_lane_interleaved.sh`
- Service: `vllm-gptoss`
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
- `ITER=5`
- `CONC=16`
- `PREFIX_TOK=1024`
- `BG_FANOUT=16`
- `CRITICAL_MAX_TOKENS=64`
- `BACKGROUND_MAX_TOKENS=256`
- `STAGGER_MS=0`

## Primary Metric

The primary metric is `focus_node_finish_ms` in the stitched per-tenant CSVs,
paired by `(iteration, variant)`.

## Promotion Gates

Use the same gates as the original C16/BG16 pre-registration:

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

Any service crash, Slurm cancellation, refused endpoint, or nonzero failed
tenant count rejects the confirmation rather than producing a latency claim.

## Command

```bash
SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
PRE_REG=docs/preregistrations/20260521T123000Z-apxm-priority-lane-c16-bg16-interleaved.md \
ITER=5 \
CONC=16 \
PREFIX_TOK=1024 \
BG_FANOUT=16 \
CRITICAL_MAX_TOKENS=64 \
BACKGROUND_MAX_TOKENS=256 \
FOCUS_NODE_ID=4 \
STAGGER_MS=0 \
FIRST_ARM=flat \
ALTERNATE_ORDER=1 \
tools/scripts/run_apxm_priority_lane_interleaved.sh
```
