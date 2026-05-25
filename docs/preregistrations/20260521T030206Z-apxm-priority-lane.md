# APXM Priority-Lane Contention Pre-Registration

Date: 2026-05-21 03:02:06 UTC

## Claim

APXM-on should reduce user-visible critical-lane finish time under backend queue contention compared with flat-HTTP on the same APXM-vLLM fork, model, service, and workload. The claim is about the declared user-visible lane, not total batch wall time.

## Locked Setup

- Workload: `examples/python/benchmarks/workloads/apxm_priority_lane.py`
- Runner: `tools/scripts/run_apxm_priority_lane.sh`
- Service: `vllm-gptoss`
- Model: `gpt-oss-120b`
- Endpoint inside service allocation: `http://127.0.0.1:8916`
- Arms:
  - APXM-on: graph registration and `vllm_xargs.apxm` hints enabled
  - flat-HTTP: same server, `--no-apxm-hints`
- Optimization level: `-O2` for both arms
- Primary focus node: APXM node id `4`, named `critical_user_answer`
- Promotion run defaults:
  - `ITER=10`
  - `CONC=16`
  - `PREFIX_TOK=2048`
  - `BG_FANOUT=10`
  - `CRITICAL_MAX_TOKENS=96`
  - `BACKGROUND_MAX_TOKENS=384`
  - `STAGGER_MS=0`

## Primary Metric

The primary metric is `focus_node_finish_ms` in `priority.*.tenants.csv`, measured from each tenant execution start until node id `4` finishes. The reporting script pairs rows by `(iteration, variant)` and bootstraps APXM/flat ratios over paired tenant rows.

## Secondary Metrics

- `focus_node_queue_wait_ms`
- `focus_node_duration_ms`
- `dispatch_fallback_triggered`
- `fields_honored`
- `failed_tenants`
- `pinned_blocks_peak`
- `prefix_cache_hit_rate`
- `batch_wall_ms`

Pin and prefix-cache metrics are mechanism context only. They are not promotion gates for this priority-lane claim.

## Promotion Gates

The result is promoted only if all gates pass:

- paired rows are present;
- zero failed tenants in both arms;
- APXM-on has zero dispatch fallback tenants;
- APXM-on honored fields include `priority`;
- focus-node timing is present in both arms;
- bootstrap 95% CI upper bound for mean `APXM focus_node_finish_ms / flat focus_node_finish_ms` is `< 0.95`;
- bootstrap 95% CI upper bound for p95 `APXM focus_node_finish_ms / flat focus_node_finish_ms` is `< 0.90`;
- absolute p95 win is at least `1000 ms`.

If these gates do not pass, publish an honest null or negative and keep the run as diagnostic evidence.

## Command

```bash
dekk apxm vllm zoo-apply deploy/vllm/zoo.review-gptoss.toml --prune
dekk apxm vllm service-status vllm-gptoss --probe

SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
ITER=10 \
CONC=16 \
PREFIX_TOK=2048 \
BG_FANOUT=10 \
CRITICAL_MAX_TOKENS=96 \
BACKGROUND_MAX_TOKENS=384 \
FOCUS_NODE_ID=4 \
STAGGER_MS=0 \
tools/scripts/run_apxm_priority_lane.sh
```

## Interpretation

A positive result supports the narrower claim that APXM graph-aware priority hints improve user-visible lane latency under contention. It does not claim that APXM reduces total batch wall time or energy unless separate gates are pre-registered and passed.
