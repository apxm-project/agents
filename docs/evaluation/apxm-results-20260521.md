# APXM Results, 2026-05-21

Status: publication-facing evaluation summary. Generated run artifacts remain
under `.apxm/evaluation/`; this file only summarizes and links the evidence.

## Headline

APXM's strongest current result is a graph-aware priority-lane win on
`gpt-oss-120b`: when short user-visible LLM work competes with long background
fanout on the same APXM-vLLM service, APXM-on reduces focus-node finish time
versus flat HTTP.

The best current citation is the combined fresh-service interleaved evidence:

| Metric | APXM-on | flat-HTTP | APXM / flat | 95% bootstrap CI |
|---|---:|---:|---:|---:|
| Paired tenant rows | `320` | `320` | n/a | n/a |
| Failed tenants | `0` | `0` | n/a | n/a |
| Focus-node mean | `49188.3 ms` | `74424.5 ms` | `0.661` | `[0.598, 0.729]` |
| Focus-node p95 | `136390 ms` | `256189 ms` | `0.532` | `[0.320, 0.590]` |

Read directly: APXM cut mean user-visible focus-node latency by about `34%`
and p95 focus-node latency by about `47%` across two pre-registered contention
workload repeats. APXM also honored the expected backend hint path:
`dispatch_ir_v1_internal`, `graph_registration`, `priority`, and
`request_hints`.

## Evaluation Shape

The priority-lane workload is intentionally narrow. Each tenant issues one
short critical LLM node that returns the user-visible answer while many longer
background LLM nodes contend for the same backend queue. APXM-on sends graph
registration and request hints to the APXM-vLLM fork; flat-HTTP uses the same
prompts, model, endpoint, service, and APXM compiler optimization level, but
without APXM request hints.

Locked fresh-service evidence:

- First repeat pre-registration:
  `docs/preregistrations/20260521T140100Z-apxm-priority-lane-c16-bg16-interleaved-iter10.md`
- First repeat run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10/`
- Second repeat pre-registration:
  `docs/preregistrations/20260521T233817Z-priority-lane-repeat2.md`
- Second repeat run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2/`
- Combined analysis:
  `.apxm/evaluation/apxm-priority-lane/combined/20260522T001900Z-fresh-service-repeats/`
- Workload:
  `examples/python/benchmarks/workloads/apxm_priority_lane.py`
- Runner:
  `tools/scripts/run_apxm_priority_lane_interleaved.sh`
- Service and model:
  `vllm-gptoss`, `gpt-oss-120b`
- Shape:
  `ITER=10`, `CONC=16`, `PREFIX_TOK=1024`, `BG_FANOUT=16`,
  `CRITICAL_MAX_TOKENS=64`, `BACKGROUND_MAX_TOKENS=256`,
  `FOCUS_NODE_ID=4`
- Arm order:
  `FIRST_ARM=flat`, `ALTERNATE_ORDER=1`, alternating arm order by iteration

## Result Progression

| Run | Design | Tenant rows | Mean APXM/flat | p95 APXM/flat | Role |
|---|---|---:|---:|---:|---|
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T040617Z-c16-bg16/` | sequential, APXM then flat | `80` | `0.431` CI `[0.413, 0.448]` | `0.517` CI `[0.507, 0.533]` | Strong supporting signal, but arm order is weaker. |
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T124230Z-c16-bg16-interleaved/` | interleaved batches, alternating order | `80` | `0.670` CI `[0.562, 0.798]` | `0.498` CI `[0.483, 0.507]` | Confirms the effect with safer ordering. |
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10/` | fresh-service interleaved repeat | `160` | `0.729` CI `[0.665, 0.797]` | `0.552` CI `[0.538, 0.563]` | First fresh-service confirmation. |
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2/` | second fresh-service interleaved repeat | `160` | `0.513` CI `[0.407, 0.657]` | `0.177` CI `[0.174, 0.181]` | Stronger second confirmation. |
| `.apxm/evaluation/apxm-priority-lane/combined/20260522T001900Z-fresh-service-repeats/` | combined fresh-service repeats | `320` | `0.661` CI `[0.598, 0.729]` | `0.532` CI `[0.320, 0.590]` | Primary public result. |

The result got more interesting after adding conservative sensitivity analysis:
the positive tenant-level signal remained, but the analysis also exposed where
the claim should stop.

## Robustness And Tradeoffs

Both fresh-service repeats passed every pre-registered promotion gate:

- paired rows present;
- zero failed tenants in both arms;
- zero APXM dispatch fallback tenants;
- APXM honored `priority`;
- focus-node timing present in both arms;
- mean-ratio CI upper bound below `0.95`;
- p95-ratio CI upper bound below `0.90`;
- absolute p95 win above `1000 ms`.

Post-hoc sensitivity analysis adds a stricter batch-level view where each
iteration is treated as one independent unit:

| Metric | APXM / flat | 95% bootstrap CI |
|---|---:|---:|
| Focus-node mean across batches | `0.661` | `[0.445, 0.963]` |
| Batch wall | `0.700` | `[0.485, 0.985]` |
| Sum tenant wall | `0.694` | `[0.480, 0.982]` |

This is the main improvement from the second repeat: the conservative
batch-level confidence intervals now exclude `1.0` across `20` independent
batch units. The claim is still scoped to a priority-lane latency result under
contention, not a broad scheduler or throughput result.

Tradeoff summary for the combined fresh-service evidence:

- Tenant focus-node win rate: `222/320`.
- Combined batch-wall ratio: `0.700` with CI `[0.485, 0.985]`.
- Combined sum-tenant-wall ratio: `0.694` with CI `[0.480, 0.982]`.

This supports a priority-lane scheduling claim. It does not yet support a
general batch-wall or fairness claim across workloads.

## Regime Map

The result is credible because the negative and null cells are not hidden.
Current evidence says APXM helps when the graph hint directly matches the
metric being measured, but does not automatically speed up all LLM traffic.

| Regime | Evidence | Interpretation |
|---|---|---|
| Priority lane under background contention | Combined fresh-service repeats mean `0.661`, p95 `0.532`, batch-level focus CI `[0.445, 0.963]`, zero failures, APXM honored `priority` | Positive headline result. |
| Review Council GPT-OSS | `.apxm/evaluation/apxm-review-council/runs/20260521T023541Z-gptoss120b-capped-iter20/summary.csv`: APXM/flat `1.222`, CI `[1.029, 1.556]` | Mechanism engaged, but latency was negative. Do not promote as a speedup. |
| Current-service pin demo | `.apxm/evaluation/pin-demo/runs/20260521T024800Z-gptoss120b-pin-demo-c32-iter10/summary.csv`: APXM/flat `1.064`, CI `[1.014, 1.151]` | Pin engagement alone does not imply a speedup. |
| Plan 04 cross-system | `.apxm/evaluation/cross-system/20260519T075832Z/summary.csv`: Mooncake `1.073`, ShareGPT `1.223`, LooGLE `1.001` | Default single-instance batch-wall workloads are null or negative. |

## What We Can Claim

Safe:

> On a pre-registered APXM-vLLM priority-lane workload, APXM graph-aware
> priority hints reduced user-visible critical-lane latency under background
> LLM queue contention. Across two fresh-service interleaved repeats, the
> combined APXM/flat focus-node mean ratio was `0.661` with 95% CI
> `[0.598, 0.729]`, and the p95 ratio was `0.532` with CI `[0.320, 0.590]`,
> with `320` paired tenant rows, zero failed tenants, APXM honoring
> `priority`, and batch-level focus ratio `0.661` with CI `[0.445, 0.963]`
> across `20` independent batch units.

Do not claim:

- APXM is generally faster.
- APXM reduces total batch wall time across workloads.
- Prefix pinning caused the priority-lane result.
- Pin engagement implies lower latency.
- Review Council is currently a quality-passing or latency-positive workflow.
- The result is proven across more than two fresh services or nodes.

## Reproduce

Launch the APXM-vLLM service through Dekk, then run the pre-registered
fresh-service repeats and combine them:

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

Repeat with
`PRE_REG=docs/preregistrations/20260521T233817Z-priority-lane-repeat2.md`
and a new `TS`, then combine the two run directories with
`tools/scripts/analyze_apxm_priority_lane_combined.py`.

Generated outputs should land under
`.apxm/evaluation/apxm-priority-lane/runs/<UTC>/`.

## Evidence Files

- Combined analysis:
  `.apxm/evaluation/apxm-priority-lane/combined/20260522T001900Z-fresh-service-repeats/combined-priority-lane-analysis.json`
- First repeat report:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10/report.json`
- First repeat summary CSV:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10/summary.csv`
- Second repeat report:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2/report.json`
- Second repeat summary CSV:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2/summary.csv`
- Enhanced analyses:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10/priority-lane-enhanced-analysis.md`
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2/priority-lane-enhanced-analysis.md`
- Batch tradeoffs:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10/priority-lane-batch-tradeoffs.csv`
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2/priority-lane-batch-tradeoffs.csv`
- Evidence manifest:
  `.apxm/evaluation/apxm-priority-lane/priority-lane-evidence.sha256`
- Claim ledger:
  `.apxm/docs/claims/apxm-priority-lane-c16-bg16.md`
- Current synthesis:
  `.apxm/docs/evaluation/EVIDENCE-SYNTHESIS.md`
