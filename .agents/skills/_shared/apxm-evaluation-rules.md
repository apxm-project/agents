# Shared rule — APXM evaluation rules

Load before any session that runs a benchmark, evaluation, or
quality-eval harness; that creates artifacts under `.apxm/`; or that
produces numbers that will back a claim in the paper or in a PR
description.

## Artifact placement

All generated artifacts live under `.apxm/` (repo-local, gitignored).
Never put benchmark CSVs, session directories, `.apxmobj` files,
compiler diagnostics, evidence manifests, vLLM logs, or per-run configs
under `examples/`, `docs/`, or repo root.

Canonical sub-locations:

- Benchmark results: `.apxm/benchmarks/results/`
- Evaluation runs: `.apxm/evaluation/<scenario>/runs/<UTC>/`
- vLLM image store: `.apxm/vllm-images/`
- vLLM service registry: `.apxm/vllm-services/`

Use the helper rather than inventing path strings:

```python
from apxm.contract import RepoLayout, build_layout
layout = build_layout(__file__)
results_dir = layout.benchmarks_results_dir
```

If you find a generated artifact under `examples/` or `docs/`, move it
to the matching `.apxm` location and patch whatever script wrote it
there — do not add an ignore guard to mask the bug.

## Claim-bearing runs

A run is *claim-bearing* if its output backs:

- The APXM paper in `apxm-project/eval`.
- A claim card in the `eval` repo.
- A benchmark number cited in a PR description, README, or `docs/`.
- A "X is faster than Y" / "X matches Y in quality" assertion.
- Any external write-up consuming this harness's evidence.

Claim-bearing runs require **all** of:

1. A committed preregistration in `workspace/eval/preregistrations/` *before*
   the run starts. See `apxm-preregistration` for the template.
2. Execution through `dekk agents vllm service-exec <service> -- ...` so
   the service allocation is the recorded GPU context.
3. Artifacts written under `.apxm/evaluation/<scenario>/runs/<UTC>/`.
4. A write-up in `workspace/eval/evidence/reports/` citing the
   preregistration commit SHA and the artifact path.
5. (If paper-bound) a claim card in the `eval` repo.

`finish` refuses to claim completion of a claim-bearing run if the
preregistration commit isn't present, or if artifacts landed outside
`.apxm/`.

## Paired-arm benchmarks

For A/B benchmarks, cache-salt scoping must be **per (arm, opt)**, never
per (iter, row). The wrong scoping lets the second arm hit the first
arm's cache and silently contaminates the comparison. See the
`feedback_paired_arm_cache_salt_scoping` incident.

## Reproducibility checklist

Before publishing a number:

- Preregistration commit SHA referenced in the write-up.
- Artifact path under `.apxm/evaluation/<scenario>/runs/<UTC>/` with
  config, raw outputs, and a `manifest.json`.
- Service allocation recorded (`dekk agents vllm service-status <name>
  --probe`).
- vLLM commit SHA recorded (the `external/vllm` submodule pointer).
- APXM commit SHA recorded (HEAD at run time).
- For seeded samplers, the exact `--seed`, `--num-tasks`, and (if
  needed) `--task-ids` recorded — `tau2 --num-tasks N` reshuffles per
  N, so cross-N ratio tables are descriptive only, not inferential.

## tau2 / quality-eval specifics

- `tau2 --seed S --num-tasks N` is not nested across N. If you need
  nested arcs across multiple Ns, use `--task-ids` to pin the set.
- `dekk eval quality-eval` is the tier-3 path (rubric + budget +
  judge). `dekk eval test-quality-eval` exercises the harness offline.
