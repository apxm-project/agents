# APXM Review Council Evaluation

Status: diagnostic mechanism-engagement workflow; current GPT-OSS promotion
run is an honest negative for latency. Source workload:
`examples/python/benchmarks/workloads/apxm_review_council.py`.

## Why this is the right small test

APXM should not be evaluated first as a generic chatbot wrapper. Its strongest
claim is narrower: APXM compiles an agentic workflow into a graph, sends
graph-aware hints to the vLLM fork, and improves execution when many LLM calls
share context and have a critical path.

The Review Council workload is built for that claim:

- one large shared APXM code/docs context,
- parallel specialist reviewers that all read the same prefix,
- explicit `reuse_group` so the APXM-on arm exercises graph registration,
  prefix pinning, cache telemetry, and priority scheduling,
- one synthesis node on the critical path,
- the same runnable graph through APXM-on and flat-HTTP arms.

This is simple enough to run repeatedly, but realistic enough to explain:
"APXM reviews its own vLLM evaluation path faster/cheaper without hiding null
findings."

## Arms

Use the same vLLM service, model, image, cache settings, and scheduler policy
for both arms.

| Arm | Command shape | Meaning |
|---|---|---|
| `apxm-on` | `--opt-levels 2` | Graph hints, `reuse_group`, `/v1/apxm/graphs/register`, priority, pin telemetry |
| `flat-http` | `--opt-levels 2 --no-apxm-hints` | Same graph, compiler level, prompts, and vLLM binary, but no APXM graph hints |

Do not compare against stock vLLM for the first claim. The defensible first
claim is same-fork APXM-on vs flat-HTTP.

## Primary metrics

Report these from the existing CSVs and service probes:

- `batch_wall_ms` ratio, APXM-on / flat-HTTP, paired bootstrap CI.
- `max_tenant_wall_ms` and `sum_tenant_wall_ms`, to separate tail from total
  work.
- `pinned_blocks_peak_max`; if this is zero, no pin-related win can be claimed.
- `prefix_cache_hit_rate`, `prefix_cache_hits_delta`,
  `prefix_cache_queries_delta`.
- `failed_tenants`; any nonzero cell is descriptive only.
- J/req from rocm-smi sidecars for the promotion run.

Secondary metrics:

- service startup state and scheduler policy from `dekk apxm vllm
  service-status vllm-gptoss --probe`,
- APXM SHA, vLLM SHA, dirty-tree state, image tag, model id, and
  `.apxm/deploy/<UTC>/zoo-snapshot.json`.

## Quality guard

The first publishable result can be a systems result, but it still needs a
quality sanity check. For a smoke gate, sample one APXM-on and one flat-HTTP
output and verify:

- each reviewer returns exactly three bullets,
- each bullet cites a concrete file path from the prompt context,
- the final synthesis includes the four requested sections,
- no arm has systematic empty, malformed, or refusal output.

For a stronger follow-up, add a deterministic parser that scores path
citation coverage and section completeness from the captured session outputs.
Do not use an LLM judge for the headline.

## Run recipe

Pre-flight:

```bash
dekk apxm vllm doctor
dekk apxm vllm zoo-apply
dekk apxm vllm service-status vllm-gptoss --probe
```

Preferred paired run:

```bash
tools/scripts/run_apxm_review_council.sh
```

Small smoke:

```bash
SMOKE=1 tools/scripts/run_apxm_review_council.sh
```

The runner captures `service-probe.txt` plus the raw `service-state.json`,
writes APXM-on and flat-HTTP CSVs, produces per-cell matrix reports, and
stitches `combined.csv`, `summary.csv`, and `summary.json` under
`.apxm/evaluation/apxm-review-council/runs/<UTC>/`.
The manual stitch command is:

```bash
python3 examples/python/benchmarks/plan04_cross_workload.py \
  --input apxm-review-council:$OUT/review.apxm-on.csv \
  --input apxm-review-council:$OUT/review.flat-http.csv \
  --output $OUT/combined.csv \
  --summary $OUT/summary.csv \
  --manifest $OUT/summary.json
```

## Decision rules

Promote as a positive APXM-vLLM workflow claim only if:

- `failed_tenants == 0` in both arms,
- APXM-on / flat-HTTP `batch_wall_ms` CI excludes 1.0 in APXM's favor,
- `pinned_blocks_peak_max > 0` in the APXM-on arm,
- quality smoke gate passes in both arms.

Promote as an honest null if:

- quality passes,
- pin engagement is zero or the latency CI crosses 1.0.

Promote as a negative if:

- quality passes,
- APXM-on is slower with CI excluding 1.0.

Reject and rerun if:

- any tenant fails,
- service probe does not report priority scheduling,
- cache/pin telemetry is missing,
- the two arms use different model, image, max model length, max sequences, or
  prefix caching settings.

## Why this can make APXM shine

The workload creates the conditions APXM is designed for: repeated long
context, graph-level knowledge, parallel fanout, a critical merge, and vLLM
prefix-cache pressure. If APXM does not help here, that is useful evidence:
Phase 1 needs a protocol change before broader claims. If it does help, the
result is easy to explain and reproduce.

## 2026-05-21 Results

Two post-fix Mistral runs and one GPT-OSS promotion run were completed after
the runtime/provider fixes that made graph-aware vLLM capabilities and
legacy `reuse_group` cache salting line up.

| Run | Model | Setup | Mean APXM / flat | CI | Pin result | Verdict |
|---|---|---:|---:|---:|---|---|
| `.apxm/evaluation/apxm-review-council/runs/20260521T020516Z-mistral7b-fixed` | Mistral 7B | iter=10, conc=8, prefix=4096, uncapped | 0.555 | [0.283, 1.100] | APXM pin >0, flat pin 0 | diagnostic only; flat had one 157s generation runaway |
| `.apxm/evaluation/apxm-review-council/runs/20260521T021726Z` | Mistral 7B | iter=20, conc=8, prefix=4096, capped | 1.004 | [0.992, 1.015] | APXM pin 17584 in all rows, flat pin 0 | honest null |
| `.apxm/evaluation/apxm-review-council/runs/20260521T023541Z-gptoss120b-capped-iter20` | GPT-OSS 120B | iter=20, conc=16, prefix=8192, capped | 1.222 | [1.029, 1.556] | APXM pin 14208 in all rows, flat pin 0 | flat-HTTP latency win; APXM 22.2% slower |

The GPT-OSS run also failed the deterministic quality gate:
`quality.json` passed parseability, non-empty output, path citations, and
no-refusal checks for both arms, but many rows hit the 2048 combined output
token cap before the final four requested sections were complete. Treat that
run as systems/mechanism evidence, not a quality-passing workflow claim.

The current publishable statement is therefore narrow:

> APXM graph-aware vLLM registration, prefix pinning, and flat-HTTP suppression
> are now observable end-to-end on a real APXM dogfood workflow. On the
> measured Review Council workflow, those mechanisms do not produce a latency
> win under the current batch-wall metric; GPT-OSS is latency-negative.

## 2026-05-21 Pin-Demo Check

A fresh `pin_demo` check was also run on the same GPT-OSS service:
`.apxm/evaluation/pin-demo/runs/20260521T024800Z-gptoss120b-pin-demo-c32-iter10`.
At `conc=32`, `prefix=16384`, and `fanout=8`, APXM pinning engaged
(`pinned_blocks_peak_max` up to 15173) and flat-HTTP stayed at zero pins, but
flat-HTTP was still faster:

| Workload | Mean APXM / flat | CI | Verdict |
|---|---:|---:|---|
| `pin_demo` | 1.064 | [1.014, 1.151] | flat-HTTP win; APXM 6.4% slower |

This means the older positive pin-demo regime is not reproduced on the current
full-memory GPT-OSS service state. Do not cite historical pin wins as the
current promotion result without also citing this regime dependence.

## Next Publishable Step

The next publishable artifact should be an honest mechanism/negative report,
not a speedup claim. If a positive systems claim is still required, define a
new pre-registered workload that measures priority-lane latency directly:
mixed short critical-path requests competing with long background fanout under
controlled queue pressure. Review Council and `pin_demo` both show that
batch-wall speedup is not guaranteed merely because prefix pinning engages.
