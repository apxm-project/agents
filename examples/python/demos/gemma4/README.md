# Three Skill-Library Proof Points

Three runnable workflows that demonstrate the APXM skill-library model on a
real vLLM-served Gemma model. Each case is a self-contained Python graph that
compiles to an `.apxmobj` artifact and runs on a registered vLLM backend.

| # | Workflow | What it shows |
| --- | --- | --- |
| 1 | [`workflows/01_review_synthesis_skill.py`](workflows/01_review_synthesis_skill.py) | Same compiled skill consumed by multiple agents — Claude (architect) and Codex (reviewer) feed a Gemma synthesis chain through a 6-way aspect fanout that shares a vLLM prefix. |
| 2 | [`workflows/02_checkout_context_pruning.py`](workflows/02_checkout_context_pruning.py) | Compiler optimization (dead-operand elimination at O2) removes unreferenced context branches; the optimization is part of the skill, not the caller. |
| 3 | [`workflows/03_vllm_backend_hints.py`](workflows/03_vllm_backend_hints.py) | Typed runtime hints (priority class, prefix-cache mode) survive the lowering and reach vLLM — one packaging, multiple deployments. |

See the per-workflow [`compiler.md`](workflows/01_review_synthesis_skill/compiler.md)
and [`runtime.md`](workflows/01_review_synthesis_skill/runtime.md) notes (and
their `02_*/`, `03_*/` siblings) for compiler-pass details and runtime evidence.

## Layout

| Path | Purpose |
| --- | --- |
| `workflows/0N_*.py` | One file per case. Each file is the graph definition **and** the entry point. |
| `workflows/0N_*/` | Per-case `compiler.md`, `runtime.md`, and `dspy.md` analysis notes. |
| `shared/` | Cross-case helpers (route aliases, dossier text). |
| `scripts/` | Operator scripts: `run_demo.py` writes the local config, `run_case.py` invokes the shared benchmark harness, `measure_vllm_hints.py` probes the vLLM HTTP boundary directly, `o0_o2_report.py` summarizes O0 vs O2 evidence. |
| `.apxm/evaluation/gemma4/runs/` | Repo-local APXM workspace for generated evidence; never write run outputs under `examples/`. |

## Compile once, run many

Every case is meant to be measured with `dekk apxm compile` (build one
`.apxmobj` per opt level) and `dekk apxm run` (execute the artifact repeatedly).
`dekk apxm execute` is fine for smoke runs but conflates compile + run timing.

```sh
DEMO=examples/python/demos/gemma4
RUN_DIR=".apxm/evaluation/gemma4/runs/$(date -u +%Y%m%dT%H%M%SZ)-three-cases"

python3 "$DEMO/scripts/run_demo.py" \
    --run-dir "$RUN_DIR" \
    --backend vllm-fork \
    --model google/gemma-4-31B-it
CFG="$RUN_DIR/generated/config.toml"
```

Then run any case as a normal APXM graph:

```sh
python3 examples/python/benchmarks/benchmark_e2e.py \
    --graph "$DEMO/workflows/02_checkout_context_pruning.py" \
    --iterations 3 \
    --precompile-artifacts \
    --emit-compiler-diagnostics \
    --artifact-dir       "$RUN_DIR/context-pruning/artifacts" \
    --diagnostics-dir    "$RUN_DIR/context-pruning/compiler-diagnostics" \
    --output             "$RUN_DIR/context-pruning/runtime-o0-o2.csv" \
    --session-base       "$RUN_DIR/context-pruning/sessions" \
    --backend-label gemma4-demo-vllm \
    --target tokens \
    --apxm-config "$CFG" \
    --interleave-opt-levels
```

Or use the `run_case.py` wrapper to run a single case end-to-end:

```sh
python3 "$DEMO/scripts/run_case.py" \
    --case context-pruning \
    --run-dir "$RUN_DIR" \
    --apxm-config "$CFG"
```

## Reading the evidence

After a sweep, summarize O0 vs O2 across all three cases with:

```sh
python3 "$DEMO/scripts/o0_o2_report.py" --run-dir "$RUN_DIR"
```

Each case has its own claim metric documented inline (in the workflow
`.py` docstring and the `runtime.md` next to it). Quote only the metric the
workflow actually measures — for example, case 02 owns the call/token-reduction
claim, case 03 owns the critical-chain priority claim, and so on.
