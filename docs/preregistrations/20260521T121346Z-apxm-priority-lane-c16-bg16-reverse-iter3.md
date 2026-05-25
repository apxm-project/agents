# APXM Priority-Lane C16/BG16 Reverse-Order ITER=3 Confirmation Pre-Registration

Date: 2026-05-21 12:13:46 UTC

## Claim

This is a bounded confirmation after the full reverse-order run hit a vLLM
EngineCore/TCPStore service failure during flat-HTTP iteration 4. The goal is
to check whether the APXM priority-lane advantage is visible when flat-HTTP
runs first, while limiting the run to the first three iterations that completed
cleanly in the rejected service-failure artifact.

## Locked Setup

- Workload: `examples/python/benchmarks/workloads/apxm_priority_lane.py`
- Runner: `tools/scripts/run_apxm_priority_lane.sh`
- Service: `vllm-gptoss`
- Image: `apxm-vllm-runtime:prioritylane-33c1855-490aaad0-dirty`
- Model: `gpt-oss-120b`
- Endpoint inside service allocation: `http://127.0.0.1:8916`
- Scheduler policy: `priority`
- Focus node id: `4` (`critical_user_answer`)
- Arm order: `ARM_ORDER=flat,apxm`
- Optimization level: `-O2` for both arms
- `ITER=3`
- `CONC=16`
- `PREFIX_TOK=1024`
- `BG_FANOUT=16`
- `CRITICAL_MAX_TOKENS=64`
- `BACKGROUND_MAX_TOKENS=256`
- `STAGGER_MS=0`

## Primary Metric

The primary metric is `focus_node_finish_ms` in the per-tenant CSVs, paired
by `(iteration, variant)`.

## Promotion Gates

Use the same gates as the original C16/BG16 pre-registration, with the smaller
sample size declared here before the rerun:

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

## Command

```bash
SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
PRE_REG=docs/preregistrations/20260521T121346Z-apxm-priority-lane-c16-bg16-reverse-iter3.md \
ITER=3 \
CONC=16 \
PREFIX_TOK=1024 \
BG_FANOUT=16 \
CRITICAL_MAX_TOKENS=64 \
BACKGROUND_MAX_TOKENS=256 \
FOCUS_NODE_ID=4 \
STAGGER_MS=0 \
ARM_ORDER=flat,apxm \
tools/scripts/run_apxm_priority_lane.sh
```
