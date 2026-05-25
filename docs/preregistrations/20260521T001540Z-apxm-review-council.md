# Pre-Registration: APXM Review Council vLLM Workflow

Authored: 2026-05-21T00:15:40Z.

## Scope

Run a paired APXM-on vs flat-HTTP evaluation of
`examples/python/benchmarks/workloads/apxm_review_council.py` on the same
APXM-vLLM fork service. This is the first dogfood workflow intended to make
APXM's graph-aware vLLM implementation visible on a realistic but simple task:
parallel code/evaluation reviewers over one large shared APXM context, followed
by a critical-path synthesis node.

## Locked setup

- Service: `vllm-gptoss`
- Endpoint: `http://127.0.0.1:8916`
- Model: `gpt-oss-120b`
- Scheduler policy: `priority`
- Prefix caching: enabled
- Workload: `examples/python/benchmarks/workloads/apxm_review_council.py`
- Shared-context target: `APXM_WORKLOAD_PREFIX_TOK=8192`
- Fanout: `APXM_WORKLOAD_FANOUT=6`
- Concurrency: 16
- Iterations: 10
- Artifact root: `.apxm/evaluation/apxm-review-council/runs/<UTC>/`

The service probe must be recorded before the run. If scheduler policy is not
`priority`, the run is diagnostic only and cannot support a graph-aware APXM
claim.

## Arms

| Arm | Flags | Meaning |
|---|---|---|
| APXM-on | `--opt-levels 2` | graph hints enabled, reuse_group emitted, APXM routes active |
| flat-HTTP | `--opt-levels 2 --no-apxm-hints` | same graph, compiler level, prompts, and vLLM binary, no APXM graph hints |

Both arms must run against the same service image, model id, max model length,
max sequences, and prefix-cache setting.

## Hypotheses

Primary systems hypothesis:

> On this shared-context fanout workflow, APXM-on reduces `batch_wall_ms`
> versus flat-HTTP, with paired bootstrap CI on APXM/flat ratio excluding 1.0
> in APXM's favor.

Mechanism hypothesis:

> The APXM-on arm reports `pinned_blocks_peak_max > 0` for at least one
> iteration and has non-missing prefix-cache telemetry.

Quality non-regression hypothesis:

> Both arms produce structurally valid review outputs: reviewer bullets cite
> paths from the prompt context, and the final synthesis contains the requested
> four sections. This is a smoke gate, not an LLM-judge claim.

## Primary metrics

- `batch_wall_ms`
- `max_tenant_wall_ms`
- `sum_tenant_wall_ms`
- `pinned_blocks_peak_max`
- `prefix_cache_hit_rate`
- `prefix_cache_queries_delta`
- `prefix_cache_hits_delta`
- `failed_tenants`

## Decision rules

Promote as APXM win if:

- `failed_tenants == 0` in both arms,
- APXM/flat `batch_wall_ms` CI excludes 1.0 with upper bound `< 1.0`,
- `pinned_blocks_peak_max > 0` in APXM-on,
- quality smoke gate passes in both arms.

Promote as honest null if:

- quality smoke gate passes,
- latency CI crosses 1.0 or pin engagement is zero.

Promote as APXM regression if:

- quality smoke gate passes,
- APXM/flat `batch_wall_ms` CI excludes 1.0 with lower bound `> 1.0`.

Reject and rerun if:

- any tenant fails,
- service probe is missing or not priority,
- cache/pin telemetry is missing,
- model/image/service flags differ between arms.

## Run commands

Preferred run:

```bash
tools/scripts/run_apxm_review_council.sh
```

Small smoke:

```bash
SMOKE=1 tools/scripts/run_apxm_review_council.sh
```

Manual setup, if the runner is not used:

```bash
TS=$(date -u +%Y%m%dT%H%M%SZ)
OUT=.apxm/evaluation/apxm-review-council/runs/$TS
mkdir -p "$OUT"
dekk apxm vllm service-status vllm-gptoss --probe > "$OUT/service-probe.txt"
cp .apxm/vllm-services/vllm-gptoss.json "$OUT/service-state.json"
```

Stitch:

```bash
python3 examples/python/benchmarks/plan04_cross_workload.py \
  --input apxm-review-council:$OUT/review.apxm-on.csv \
  --input apxm-review-council:$OUT/review.flat-http.csv \
  --output "$OUT/combined.csv" \
  --summary "$OUT/summary.csv" \
  --manifest "$OUT/summary.json"
```
