# APXM Priority-Lane Evaluation

Status: promoted positive APXM-vLLM priority-scheduling result. The current
public citation is the combined two-repeat fresh-service interleaved analysis.
Source workload: `examples/python/benchmarks/workloads/apxm_priority_lane.py`.

## Claim

APXM-on reduces user-visible critical-lane finish time when short critical
LLM work competes with long background LLM fanout on the same fixed
APXM-vLLM service.

This is not a total batch-wall claim and not a prefix-pinning speedup claim.
The measured effect is narrower: APXM graph hints let the vLLM fork honor
priority for the node that returns the user-visible answer, while flat-HTTP
uses the same prompts, model, service, and compiler level without APXM
request hints.

## Primary Public Setup

- First repeat pre-registration:
  `docs/preregistrations/20260521T140100Z-apxm-priority-lane-c16-bg16-interleaved-iter10.md`
- First repeat run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10`
- Second repeat pre-registration:
  `docs/preregistrations/20260521T233817Z-priority-lane-repeat2.md`
- Second repeat run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2`
- Combined analysis:
  `.apxm/evaluation/apxm-priority-lane/combined/20260522T001900Z-fresh-service-repeats`
- Service: `vllm-gptoss`
- Image: `apxm-vllm-runtime:prioritylane-33c1855-490aaad0-dirty`
- APXM commit: `33c1855f` with a dirty worktree carrying the evaluation
  harness and vLLM-priority patches.
- vLLM fork commit: `490aaad0c` with local scheduler changes.
- Model: `gpt-oss-120b`
- Scheduler policy: `priority`
- Optimization level: `-O2` for both arms
- `ITER=10`, `CONC=16`, `PREFIX_TOK=1024`, `BG_FANOUT=16`
- `CRITICAL_MAX_TOKENS=64`, `BACKGROUND_MAX_TOKENS=256`
- Focus node: `4` (`critical_user_answer`)
- `STAGGER_MS=0`, `FIRST_ARM=flat`, `ALTERNATE_ORDER=1`

Generated artifacts are under `.apxm/evaluation/`, not under `examples/`.

## Combined Fresh-Service Result

The two fresh-service repeats are combined as `20` independent batch units and
`320` paired tenant rows:

- Mean focus-node finish ratio: `0.661`, 95% CI `[0.598, 0.729]`
- p95 focus-node finish ratio: `0.532`, 95% CI `[0.320, 0.590]`
- Tenant focus win rate: `222/320`
- Batch-level focus ratio: `0.661`, 95% CI `[0.445, 0.963]`
- Batch-wall ratio: `0.700`, 95% CI `[0.485, 0.985]`
- Sum-tenant-wall ratio: `0.694`, 95% CI `[0.480, 0.982]`

This is the strongest current external citation because it keeps the
interleaved arm-order control and the conservative batch-level intervals now
exclude `1.0`. The supported claim remains narrow: APXM priority hints reduce
user-visible critical-lane latency under background LLM queue contention.

## Original Locked Setup

- Pre-registration:
  `docs/preregistrations/20260521T040539Z-apxm-priority-lane-c16-bg16.md`
- Run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T040617Z-c16-bg16`
- Service: `vllm-gptoss`
- Image: `apxm-vllm-runtime:prioritylane-33c1855-490aaad0-dirty`
- APXM commit: `33c1855f` with a dirty worktree carrying the evaluation
  harness and vLLM-priority patches.
- vLLM fork commit: `490aaad0c` with local scheduler changes.
- Model: `gpt-oss-120b`
- Scheduler policy: `priority`
- Optimization level: `-O2` for both arms
- `ITER=5`, `CONC=16`, `PREFIX_TOK=1024`, `BG_FANOUT=16`
- `CRITICAL_MAX_TOKENS=64`, `BACKGROUND_MAX_TOKENS=256`
- Focus node: `4` (`critical_user_answer`)
- `STAGGER_MS=0`

## Original Sequential Result

Primary metric: per-tenant `focus_node_finish_ms`, paired by
`(iteration, variant)` across APXM-on and flat-HTTP.

| Metric | APXM-on | flat-HTTP | APXM / flat |
|---|---:|---:|---:|
| Paired tenant rows | 80 | 80 | n/a |
| Failed tenants | 0 | 0 | n/a |
| Focus mean | 33058.6 ms | 76639.1 ms | 0.431 |
| Focus p50 | 30911.0 ms | 77067.0 ms | 0.401 |
| Focus p95 | 53819.0 ms | 104002.0 ms | 0.517 |
| Focus max | 54170.0 ms | 104858.0 ms | 0.517 |

Bootstrap gates from `report.json`:

- Mean focus-node finish ratio: `0.431`, 95% CI `[0.413, 0.448]`
- p95 focus-node finish ratio: `0.517`, 95% CI `[0.507, 0.533]`
- Absolute p95 win: `50183 ms`
- Promotion verdict: `APXM priority-lane win; mean ratio=0.431`

APXM-on honored fields included `dispatch_ir_v1_internal`,
`graph_registration`, `priority`, and `request_hints`. The flat-HTTP arm had
no APXM honored fields, as intended.

## Original Promotion Gates

All pre-registered gates passed:

- paired rows are present;
- zero failed tenants in both arms;
- APXM-on has zero dispatch fallback tenants;
- APXM-on honored `priority`;
- focus-node timing is present in both arms;
- bootstrap 95% CI upper bound for mean APXM/flat focus finish ratio is
  below `0.95`;
- bootstrap 95% CI upper bound for p95 APXM/flat focus finish ratio is
  below `0.90`;
- absolute p95 win is at least `1000 ms`.

## Interleaved Confirmation

The follow-up interleaved confirmation alternated one APXM batch and one flat
batch per iteration, with odd iterations flat-first and even iterations
APXM-first:

- Pre-registration:
  `docs/preregistrations/20260521T123000Z-apxm-priority-lane-c16-bg16-interleaved.md`
- Run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T124230Z-c16-bg16-interleaved`
- Paired tenant rows: `80`
- Failed tenants: `0` both arms
- APXM honored fields: `dispatch_ir_v1_internal`, `graph_registration`,
  `priority`, `request_hints`
- Mean focus-node finish ratio: `0.670`, 95% CI `[0.562, 0.798]`
- p95 focus-node finish ratio: `0.498`, 95% CI `[0.483, 0.507]`
- Promotion verdict: `APXM priority-lane win; mean ratio=0.670`

Per-iteration focus means show the regime boundary. The first cold/contentious
pair is large (`flat=70385 ms`, `APXM=13190 ms`), while later warm-service
pairs converge and sometimes slightly favor flat. The promoted statement
should therefore emphasize the aggregate pre-registered priority-lane result
and p95 user-visible latency under contention, not claim that every warm batch
is faster.

## Fresh-Service Interleaved Repeat

The first fresh-service 10-iteration repeat was run after the enhanced
analysis showed that the original interleaved confirmation needed more
independent batches:

- Pre-registration:
  `docs/preregistrations/20260521T140100Z-apxm-priority-lane-c16-bg16-interleaved-iter10.md`
- Run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10`
- Paired tenant rows: `160`
- Failed tenants: `0` both arms
- APXM honored fields: `dispatch_ir_v1_internal`, `graph_registration`,
  `priority`, `request_hints`
- Mean focus-node finish ratio: `0.729`, 95% CI `[0.665, 0.797]`
- p95 focus-node finish ratio: `0.552`, 95% CI `[0.538, 0.563]`
- Tenant focus win rate: `111/160`
- Promotion verdict: `APXM priority-lane win; mean ratio=0.729`

The enhanced analysis for this run reports aggregate batch-wall ratio `0.756`
and sum-tenant-wall ratio `0.753`. Batch-level focus ratio is `0.729` with CI
`[0.510, 1.002]`. This run is now one half of the combined fresh-service
evidence rather than the sole primary citation.

## Second Fresh-Service Interleaved Repeat

A second fresh-service 10-iteration repeat used the same locked shape and
pre-registered gates:

- Pre-registration:
  `docs/preregistrations/20260521T233817Z-priority-lane-repeat2.md`
- Run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2`
- Paired tenant rows: `160`
- Failed tenants: `0` both arms
- APXM honored fields: `dispatch_ir_v1_internal`, `graph_registration`,
  `priority`, `request_hints`
- Mean focus-node finish ratio: `0.513`, 95% CI `[0.407, 0.657]`
- p95 focus-node finish ratio: `0.177`, 95% CI `[0.174, 0.181]`
- Tenant focus win rate: `111/160`
- Promotion verdict: `APXM priority-lane win; mean ratio=0.513`

The second repeat materially strengthens the result. In combined form, the
batch-level focus, batch-wall, and sum-tenant-wall intervals all exclude
`1.0`, while the narrower claim boundary remains unchanged.

## Enhanced Sensitivity Analysis

Additional post-hoc analysis artifacts were generated without changing the
promoted `report.json` files:

- Sequential run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T040617Z-c16-bg16/priority-lane-enhanced-analysis.json`
- Interleaved run:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T124230Z-c16-bg16-interleaved/priority-lane-enhanced-analysis.json`
- Fresh-service interleaved repeat:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10/priority-lane-enhanced-analysis.json`
- Second fresh-service interleaved repeat:
  `.apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2/priority-lane-enhanced-analysis.json`
- Combined fresh-service analysis:
  `.apxm/evaluation/apxm-priority-lane/combined/20260522T001900Z-fresh-service-repeats/combined-priority-lane-analysis.json`
- Analysis tool:
  `tools/scripts/analyze_apxm_priority_lane_run.py`
- Combined analysis tool:
  `tools/scripts/analyze_apxm_priority_lane_combined.py`

The sequential run is strong even when each iteration is treated as one
independent batch: focus-mean ratio `0.431` with batch-level CI
`[0.353, 0.491]`, batch-wall ratio `0.452` with CI `[0.395, 0.498]`,
and tenant focus win rate `80/80`. This remains supporting evidence because
the arm order was APXM first, then flat.

The first interleaved run is the safer design than the sequential run, but its
batch-level sensitivity is intentionally conservative: focus-mean ratio
`0.670` with batch-level CI `[0.338, 1.135]`, batch-wall ratio `0.776` with CI
`[0.490, 1.129]`, and tenant focus win rate `44/80`.

The combined fresh-service analysis improves that evidence: tenant-level
focus-mean ratio `0.661` with CI `[0.598, 0.729]`, p95 ratio `0.532` with CI
`[0.320, 0.590]`, tenant focus win rate `222/320`, batch-level focus ratio
`0.661` with CI `[0.445, 0.963]`, batch-wall ratio `0.700` with CI
`[0.485, 0.985]`, and sum-tenant-wall ratio `0.694` with CI `[0.480, 0.982]`.

This makes the result more interesting, not weaker: APXM produces a real
priority-lane benefit under queue contention, and the enhanced analysis shows
where the benefit is concentrated and what tradeoffs need to be reported.

## Caveats

- The headline is a priority-lane result under queue contention. It does not
  claim APXM finishes all background work faster.
- The original promoted run was same-service but sequential by arm:
  APXM-on first, flat-HTTP second. The combined fresh-service interleaved
  analysis is the current public citation because it reduces that order
  concern and increases the independent batch count to 20, but the workload
  remains intentionally claim-shaped.
- Runtime `focus_node_queue_wait_ms` is APXM runtime queue wait, not vLLM
  backend queue wait. The backend priority effect is visible in focus-node
  finish time under contention.
- The workload is intentionally claim-shaped. It is a defensible scheduler
  result, but it should be presented alongside the Review Council negative
  and Plan 04 null/regression results to avoid overgeneralizing.

## Reproduce

```bash
SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
PRE_REG=docs/preregistrations/20260521T040539Z-apxm-priority-lane-c16-bg16.md \
ITER=5 \
CONC=16 \
PREFIX_TOK=1024 \
BG_FANOUT=16 \
CRITICAL_MAX_TOKENS=64 \
BACKGROUND_MAX_TOKENS=256 \
FOCUS_NODE_ID=4 \
STAGGER_MS=0 \
tools/scripts/run_apxm_priority_lane.sh
```

For the reverse-order confirmation:

```bash
SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
PRE_REG=docs/preregistrations/20260521T114939Z-apxm-priority-lane-c16-bg16-reverse.md \
ITER=5 \
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

For the stronger interleaved confirmation:

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

For the current fresh-service 10-iteration confirmation:

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
tools/scripts/run_apxm_priority_lane_interleaved.sh
```

For the second fresh-service repeat, use the same command shape with:

```bash
PRE_REG=docs/preregistrations/20260521T233817Z-priority-lane-repeat2.md
TS=20260521T233817Z-c16-bg16-interleaved-repeat2
```

Then combine both fresh-service run directories:

```bash
python3 tools/scripts/analyze_apxm_priority_lane_combined.py \
  --output-dir .apxm/evaluation/apxm-priority-lane/combined/<UTC>-fresh-service-repeats \
  .apxm/evaluation/apxm-priority-lane/runs/20260521T140100Z-c16-bg16-interleaved-iter10 \
  .apxm/evaluation/apxm-priority-lane/runs/20260521T233817Z-c16-bg16-interleaved-repeat2
```

## Confirmation Attempts

Two reverse-order confirmations were attempted after the promoted run. They
are rejected as service-failure artifacts, not as APXM-negative results.

| Run | Setup | Outcome |
|---|---|---|
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T120100Z-c16-bg16-reverse` | `ITER=5`, `ARM_ORDER=flat,apxm`, Slurm job `62151` | flat-HTTP iterations 1-3 completed with zero failures; vLLM EngineCore then died during flat-HTTP iteration 4 with TCPStore heartbeat/broken-pipe errors, so APXM-on only saw refused endpoints. Reject. |
| `.apxm/evaluation/apxm-priority-lane/runs/20260521T122100Z-c16-bg16-reverse-iter3` | `ITER=3`, `ARM_ORDER=flat,apxm`, Slurm job `62161` | flat-HTTP completed three zero-failure iterations; APXM-on completed two zero-failure iterations, then the Slurm service job was terminated before the third APXM-on iteration finished. Reject. |

The combined two-repeat fresh-service analysis is the current follow-up to
cite for the priority-lane claim. The two rejected reverse-order runs remain
useful service-stability diagnostics for GPT-OSS TP=8, but they should not be
used as APXM latency evidence.
