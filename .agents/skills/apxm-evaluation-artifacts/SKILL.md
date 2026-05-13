---
name: apxm-evaluation-artifacts
description: Use when creating, moving, reviewing, or documenting APXM benchmark, evaluation, session, compiler-diagnostic, evidence, or vLLM-run artifacts. Enforces that generated artifacts live in the repo-local .apxm workspace rather than examples or docs source trees.
---

# APXM Evaluation Artifacts

Generated benchmark and evaluation state belongs under the repo-local APXM
workspace. Source examples, workflows, scripts, and docs stay in the checked-in
tree; run outputs do not.

## Canonical Locations

- Benchmark harness outputs: `.apxm/benchmarks/results/`
- Gemma4 evaluation runs: `.apxm/evaluation/gemma4/runs/<UTC>/`
- vLLM logs, images, and service state: use the paths exposed by
  `tools/scripts/apxm_vllm_contract.py::RepoLayout`.

Do not write generated CSVs, session directories, `.apxmobj` files,
compiler-diagnostics, evidence manifests, or per-run configs under
`examples/`.

## Workflow

1. Before changing artifact paths, inspect `tools/scripts/apxm_vllm_contract.py`
   and reuse `build_layout()` / `RepoLayout` instead of inventing path strings.
2. Keep benchmark defaults pointed at `.apxm/benchmarks/results/`.
3. For claim-bearing vLLM evaluation, run through the persistent service path:
   `dekk apxm vllm service-exec <service-name> -- <command>`.
4. Put run-specific evidence under `.apxm/evaluation/<scenario>/runs/<UTC>/`
   unless an existing `RepoLayout` field is more specific.
5. If artifacts accidentally appear under `examples/`, move them into the
   matching `.apxm` location, update docs/scripts that produced them, and add a
   `.gitignore` guard only for the invalid generated path.

## Verification

- `rg "examples/python/benchmarks/results|examples/python/demos/gemma4/runs" docs examples tools --glob '!examples/python/benchmarks/results/**'`
- `find examples -path '*/results/*' -o -path '*/sessions/*' -o -path '*/runs/*'`
- `python3 -m py_compile tools/scripts/apxm_vllm_contract.py examples/python/benchmarks/benchmark_e2e.py`

The first command should not find active instructions that route new generated
artifacts into `examples/`. The second command should not show generated run
trees unless they are historical source fixtures intentionally documented as
fixtures.
