# APXM Priority-Lane Brief

## Headline

APXM graph-aware priority hints reduce user-visible critical-lane latency on
`gpt-oss-120b` when short critical LLM work competes with long background LLM
fanout on the same APXM-vLLM service.

This is a scheduler result. It is not a broad APXM latency claim, not a total
batch-wall claim, and not a prefix-pinning claim.

## Evidence

| Run | Design | Mean APXM/flat | p95 APXM/flat | Gate result |
|---|---|---:|---:|---|
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T040617Z-c16-bg16` | sequential, APXM then flat | `0.431` CI `[0.413, 0.448]` | `0.517` CI `[0.507, 0.533]` | pass |
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T124230Z-c16-bg16-interleaved` | interleaved by batch, alternating order | `0.670` CI `[0.562, 0.798]` | `0.498` CI `[0.483, 0.507]` | pass |
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10` | fresh-service interleaved by batch, alternating order | `0.729` CI `[0.665, 0.797]` | `0.552` CI `[0.538, 0.563]` | pass |
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2` | second fresh-service interleaved repeat | `0.513` CI `[0.407, 0.657]` | `0.177` CI `[0.174, 0.181]` | pass |
| `.apxm/evaluation/apxm-priority-lane/combined/20260522T001900Z-fresh-service-repeats` | combined fresh-service repeats | `0.661` CI `[0.598, 0.729]` | `0.532` CI `[0.320, 0.590]` | pass |

All accepted runs used `vllm-gptoss`, `gpt-oss-120b`, APXM-vLLM image
`apxm-vllm-runtime:prioritylane-33c1855-490aaad0-dirty`, scheduler policy
`priority`, `CONC=16`, `BG_FANOUT=16`, `PREFIX_TOK=1024`, focus node id `4`,
and `-O2` for both arms.

The combined fresh-service analysis is the publication-safe confirmation for
the priority-lane claim because it removes the largest arm-order concern from
the sequential run and raises the independent batch count to `20` across two
separate service allocations.

Enhanced post-hoc sensitivity artifacts now live beside accepted runs as
`priority-lane-enhanced-analysis.json`, `priority-lane-enhanced-analysis.md`,
and `priority-lane-batch-tradeoffs.csv`. The sequential run remains strong at
batch level. The combined fresh-service analysis reports tenant focus win
rate `222/320`, batch-wall ratio `0.700` with CI `[0.485, 0.985]`,
sum-tenant-wall ratio `0.694` with CI `[0.480, 0.982]`, and batch-level
focus ratio `0.661` with CI `[0.445, 0.963]`. Lead external copy with the
combined fresh-service result and keep the scope limited to the priority-lane
scheduler claim.

## How To Cite

Safe:

> On a pre-registered priority-lane workload, APXM-on reduced user-visible
> focus-node finish time versus flat-HTTP on the same APXM-vLLM service. The
> two fresh-service interleaved confirmations reported combined mean
> APXM/flat ratio `0.661` with 95% bootstrap CI `[0.598, 0.729]` and p95
> ratio `0.532` with CI `[0.320, 0.590]`, with `320` paired tenant rows,
> zero failed tenants, APXM honoring `priority`, and batch-level focus ratio
> `0.661` with CI `[0.445, 0.963]` across `20` independent batch units.

Do not say:

- APXM is faster in general.
- APXM reduces total batch wall time for all workloads.
- Prefix pinning caused this result.
- Every individual warm-service batch was faster.

## Reproduce

```bash
dekk apxm vllm zoo-apply deploy/vllm/zoo.review-gptoss.toml --prune
dekk apxm vllm service-status vllm-gptoss --probe

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
tools/scripts/run_apxm_priority_lane_interleaved.sh
```

Repeat the same command shape for the second fresh-service repeat with:

```bash
PRE_REG=docs/preregistrations/20260521T233817Z-priority-lane-repeat2.md
TS=20260521T233817Z-c16-bg16-interleaved-repeat2
```

Then combine the two fresh-service runs:

```bash
python3 tools/scripts/analyze_apxm_priority_lane_combined.py \
  --output-dir .apxm/evaluation/apxm-priority-lane/combined/<UTC>-fresh-service-repeats \
  .apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10 \
  .apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2
```

Generated run evidence belongs under `.apxm/evaluation/apxm-priority-lane/runs/`;
combined analyses belong under `.apxm/evaluation/apxm-priority-lane/combined/`.
