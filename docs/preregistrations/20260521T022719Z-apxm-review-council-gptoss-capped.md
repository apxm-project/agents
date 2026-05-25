# Pre-Registration: APXM Review Council GPT-OSS Output-Capped Promotion

Authored: 2026-05-21T02:27:19Z.

## Scope

Run the APXM Review Council dogfood workflow as a paired APXM-on vs flat-HTTP
evaluation on the Dekk-managed GPT-OSS APXM-vLLM service. This amends the
2026-05-21T00:15:40Z pre-registration before the promotion rerun by adding
explicit request-level output caps and increasing iterations from 10 to 20.

The amendment is motivated by the Mistral diagnostic run
`.apxm/evaluation/apxm-review-council/runs/20260521T020516Z-mistral7b-fixed`,
where one flat-HTTP tenant generated roughly 29k output tokens and dominated
the mean latency. Output caps are part of the benchmark workload, not a
post-hoc exclusion rule.

## Locked setup

- Service: `vllm-gptoss`
- Endpoint: `http://127.0.0.1:8916`
- Model: `gpt-oss-120b`
- Scheduler policy: `priority`
- Prefix caching: enabled
- Workload: `examples/python/benchmarks/workloads/apxm_review_council.py`
- Shared-context target: `APXM_WORKLOAD_PREFIX_TOK=8192`
- Fanout: `APXM_WORKLOAD_FANOUT=6`
- Reviewer output cap: `APXM_WORKLOAD_REVIEW_MAX_TOKENS=256`
- Synthesis output cap: `APXM_WORKLOAD_VERDICT_MAX_TOKENS=512`
- Concurrency: 16
- Iterations: 20
- Artifact root: `.apxm/evaluation/apxm-review-council/runs/<UTC>/`

The service probe must be recorded before the run. If scheduler policy is not
`priority`, the run is diagnostic only and cannot support a graph-aware APXM
claim.

## Arms

| Arm | Flags | Meaning |
|---|---|---|
| APXM-on | `--opt-levels 2` | graph hints enabled, reuse group emitted, APXM routes active |
| flat-HTTP | `--opt-levels 2 --no-apxm-hints` | same graph, compiler level, prompts, and vLLM binary, no APXM graph hints |

Both arms must run against the same service image, model id, max model length,
max sequences, prefix-cache setting, and APXM/vLLM SHAs.

## Hypotheses

Primary systems hypothesis:

> On this shared-context fanout workflow, APXM-on reduces `batch_wall_ms`
> versus flat-HTTP, with paired bootstrap CI on APXM/flat ratio excluding 1.0
> in APXM's favor.

Mechanism hypothesis:

> The APXM-on arm reports `pinned_blocks_peak_max > 0` in at least 80% of
> measured iterations while flat-HTTP reports zero pin peaks, and both arms have
> non-missing prefix-cache telemetry.

Quality non-regression hypothesis:

> Both arms produce structurally valid review outputs: non-empty output, path
> citations from the prompt context, the requested final sections, and no
> refusals. This is a deterministic smoke gate, not an LLM-judge claim.

## Primary Metrics

- `batch_wall_ms`
- `max_tenant_wall_ms`
- `sum_tenant_wall_ms`
- `pinned_blocks_peak_max`
- `prefix_cache_hit_rate`
- `prefix_cache_queries_delta`
- `prefix_cache_hits_delta`
- `failed_tenants`
- `quality.json` pass/fail and row counts

## Decision Rules

Promote as APXM win if:

- `failed_tenants == 0` in both arms,
- APXM/flat `batch_wall_ms` CI excludes 1.0 with upper bound `< 1.0`,
- leave-one-out paired analysis does not flip the APXM/flat verdict,
- APXM-on `pinned_blocks_peak_max >= 1000` in at least 80% of iterations,
- flat-HTTP `pinned_blocks_peak_max == 0` in every iteration,
- quality smoke gate passes in both arms.

Promote as honest null if:

- quality smoke gate passes,
- latency CI crosses 1.0 or APXM pin engagement is absent.

Promote as APXM regression if:

- quality smoke gate passes,
- APXM/flat `batch_wall_ms` CI excludes 1.0 with lower bound `> 1.0`.

Reject and rerun if:

- any tenant fails,
- service probe is missing or not priority,
- cache/pin telemetry is missing,
- model/image/service flags differ between arms,
- output caps are not present in the emitted AIR or run log.

## Run Command

```bash
dekk apxm vllm zoo-apply deploy/vllm/zoo.review-gptoss.toml --prune
dekk apxm vllm service-status vllm-gptoss --probe

SERVICE=vllm-gptoss \
APXM_ENDPOINT=http://127.0.0.1:8916 \
MODEL=gpt-oss-120b \
ITER=20 \
CONC=16 \
PREFIX_TOK=8192 \
FANOUT=6 \
REVIEW_MAX_TOKENS=256 \
VERDICT_MAX_TOKENS=512 \
PRE_REG=docs/preregistrations/20260521T022719Z-apxm-review-council-gptoss-capped.md \
tools/scripts/run_apxm_review_council.sh
```

