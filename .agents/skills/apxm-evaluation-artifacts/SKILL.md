---
name: apxm-evaluation-artifacts
description: Use when creating, moving, reviewing, or documenting APXM benchmark, evaluation, session, compiler-diagnostic, evidence, or vLLM-run artifacts. Enforces that generated artifacts live in the repo-local .apxm workspace rather than examples or docs source trees.
user-invocable: true
repo: apxm-eval
---

# APXM Evaluation Artifacts

Load `_shared/apxm-evaluation-rules.md` and
`_shared/apxm-storage-layout-rules.md` before broad work.

Generated benchmark and evaluation state belongs under the repo-local
APXM workspace. Source examples, workflows, scripts, and docs stay in
the checked-in tree; run outputs do not.

## Canonical locations

- Benchmark harness outputs: `.apxm/benchmarks/results/`
- Evaluation runs: `.apxm/evaluation/<scenario>/runs/<UTC>/`
- vLLM logs/images/service state — via `apxm.contract.RepoLayout`.

Do not write generated CSVs, session directories, `.apxmobj` files,
compiler diagnostics, evidence manifests, or per-run configs under
`examples/` or `docs/`.

## Workflow

1. Before changing artifact paths, inspect the `apxm.contract` module
   and reuse `build_layout()` / `RepoLayout` — never invent path
   strings.
2. Keep benchmark defaults pointed at `.apxm/benchmarks/results/`.
3. For claim-bearing vLLM evaluation, run through the persistent
   service path:
   `dekk apxm vllm service-exec <service-name> -- <command>`.
4. Put run-specific evidence under
   `.apxm/evaluation/<scenario>/runs/<UTC>/` unless an existing
   `RepoLayout` field is more specific.
5. If artifacts accidentally appear under `examples/`, move them
   into the matching `.apxm` location, update docs/scripts that
   produced them, and add a `.gitignore` guard only for the invalid
   generated path.

## Verification

```bash
# Active instructions should not route new artifacts into examples/:
rg "examples/python/benchmarks/results|examples/python/demos/gemma4/runs" \
   docs examples tools --glob '!examples/python/benchmarks/results/**'

# Stray generated trees:
find examples -path '*/results/*' -o -path '*/sessions/*' -o -path '*/runs/*'

# Contract module sanity:
python3 -m py_compile crates/compiler/apxm-frontend/python/apxm/contract.py \
                      examples/python/benchmarks/benchmark_e2e.py
```

The first command should not find active instructions routing
artifacts into `examples/`. The second should not show generated run
trees unless they are historical source fixtures intentionally
documented as such.

## Anti-patterns

- Hardcoding `examples/python/benchmarks/results/...` in a new
  harness.
- Inventing a path string when `RepoLayout` already names one.
- Committing generated `.apxmobj`, CSV, or session-tree files.
- Adding a `.gitignore` guard to mask a wrongly-pathed write rather
  than fixing the write.
